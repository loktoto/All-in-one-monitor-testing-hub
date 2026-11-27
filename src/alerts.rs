//! Optional temperature alerts, in the spirit of HWiNFO/HWMonitor thresholds.
//!
//! The alert is advisory only: it never changes fan speed, clocks or test
//! behaviour. It requires a fresh, trusted reading and a sustained breach so a
//! single sensor spike cannot flip the state. Missing readings are not zeros.

use std::time::{Duration, Instant};

const RECOVERY_HOLD: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Critical,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Warning => "WARNING",
            Severity::Critical => "CRITICAL",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threshold {
    pub celsius: f32,
    pub hold: Duration,
    pub severity: Severity,
}

impl Threshold {
    /// Rejects values that cannot represent a useful, sustained temperature limit.
    pub fn is_valid(self) -> bool {
        self.celsius.is_finite()
            && (30.0..=125.0).contains(&self.celsius)
            && self.hold >= Duration::from_secs(1)
            && self.hold <= Duration::from_secs(120)
    }
}

/// Tracks a run's consecutive hot telemetry samples independently of rendering.
#[derive(Default)]
pub struct ThermalStop {
    latest_sample: Option<Instant>,
    hot_since: Option<Instant>,
}

impl ThermalStop {
    pub fn observe(
        &mut self,
        sample_at: Instant,
        temperature: Option<f32>,
        rule: Threshold,
        fresh_for: Duration,
    ) -> bool {
        if self.latest_sample.is_some_and(|at| sample_at <= at) {
            return false;
        }
        if self
            .latest_sample
            .is_some_and(|at| sample_at.duration_since(at) >= fresh_for)
        {
            self.hot_since = None;
        }
        self.latest_sample = Some(sample_at);
        if !rule.is_valid() || !temperature.is_some_and(|v| v.is_finite() && v >= rule.celsius) {
            self.hot_since = None;
            return false;
        }
        let since = *self.hot_since.get_or_insert(sample_at);
        sample_at.duration_since(since) >= rule.hold
    }
}

#[derive(Default)]
pub struct Monitor {
    rule: Option<Threshold>,
    breached_since: Option<Instant>,
    active_since: Option<Instant>,
    recovered_since: Option<Instant>,
    peak_celsius: Option<f32>,
}

impl Monitor {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn active_since(&self) -> Option<Instant> {
        self.active_since
    }

    pub fn peak_celsius(&self) -> Option<f32> {
        self.peak_celsius
    }

    /// When the current streak can next change the alert state.
    pub fn next_transition(&self, rule: Threshold) -> Option<Instant> {
        if self.active_since.is_some() {
            self.recovered_since.map(|since| since + RECOVERY_HOLD)
        } else {
            self.breached_since.map(|since| since + rule.hold)
        }
    }

    /// Feed the currently watched temperature. `None` means no fresh reading
    /// and clears the streak.
    /// Returns true on the frame the alert first becomes active.
    pub fn observe(&mut self, temperature: Option<f32>, now: Instant, rule: Threshold) -> bool {
        if self.rule != Some(rule) {
            self.rule = Some(rule);
            self.breached_since = None;
            self.active_since = None;
            self.recovered_since = None;
        }
        if let Some(value) = temperature.filter(|v| v.is_finite() && *v > 0.0) {
            self.peak_celsius = Some(self.peak_celsius.map_or(value, |peak| peak.max(value)));
        }
        let was_active = self.active_since.is_some();
        match (temperature, rule.is_valid()) {
            (Some(value), true) if value.is_finite() && value >= rule.celsius => {
                self.recovered_since = None;
                // A breach starts with the first hot sample; never backdate it,
                // otherwise the first frame could fire an "already sustained" alert.
                self.breached_since.get_or_insert(now);
                if !was_active
                    && now.saturating_duration_since(self.breached_since.unwrap_or(now))
                        >= rule.hold
                {
                    self.active_since = Some(now);
                }
            }
            (Some(_), true) => {
                self.breached_since = None;
                if was_active {
                    let recovered = *self.recovered_since.get_or_insert(now);
                    if now.saturating_duration_since(recovered) >= RECOVERY_HOLD {
                        self.active_since = None;
                        self.recovered_since = None;
                    }
                }
            }
            _ => {
                // No trustworthy sample: pause both streaks and hide the alert.
                self.breached_since = None;
                self.recovered_since = None;
                self.active_since = None;
            }
        }
        !was_active && self.active_since.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> Threshold {
        Threshold {
            celsius: 80.0,
            hold: Duration::from_secs(3),
            severity: Severity::Warning,
        }
    }

    #[test]
    fn thermal_stop_requires_distinct_hot_samples_without_a_gap() {
        let start = Instant::now();
        let mut stop = ThermalStop::default();
        let fresh_for = Duration::from_secs(2);
        let mut observe = |second, temperature| {
            stop.observe(
                start + Duration::from_secs(second),
                temperature,
                rule(),
                fresh_for,
            )
        };
        assert!(!observe(0, Some(90.0)));
        assert!(!observe(0, Some(90.0)));
        assert!(!observe(1, Some(90.0)));
        assert!(!observe(3, Some(90.0)));
        assert!(!observe(4, None));
        assert!(!observe(5, Some(90.0)));
        assert!(!observe(6, Some(70.0)));
        assert!(!observe(7, Some(90.0)));
        assert!(!observe(8, Some(90.0)));
        assert!(!observe(9, Some(90.0)));
        assert!(observe(10, Some(90.0)));
    }

    #[test]
    fn short_spikes_do_not_trigger_but_sustained_heat_does() {
        let now = Instant::now();
        let mut monitor = Monitor::default();
        assert_eq!(monitor.next_transition(rule()), None);
        assert!(!monitor.observe(Some(95.0), now, rule()));
        assert_eq!(monitor.next_transition(rule()), Some(now + rule().hold));
        assert!(!monitor.observe(Some(20.0), now + Duration::from_secs(1), rule()));
        assert_eq!(monitor.next_transition(rule()), None);
        assert!(!monitor.observe(Some(95.0), now + Duration::from_secs(2), rule()));
        assert!(!monitor.observe(Some(95.0), now + Duration::from_secs(3), rule()));
        assert!(!monitor.observe(Some(95.0), now + Duration::from_secs(4), rule()));
        assert!(monitor.observe(Some(95.0), now + Duration::from_secs(5), rule()));
        assert_eq!(monitor.next_transition(rule()), None);
        assert_eq!(monitor.peak_celsius(), Some(95.0));
    }

    #[test]
    fn missing_samples_pause_the_streak_and_clear_an_active_alert() {
        let now = Instant::now();
        let mut monitor = Monitor::default();
        for second in 0..4 {
            monitor.observe(Some(90.0), now + Duration::from_secs(second), rule());
        }
        assert!(monitor.active_since().is_some());
        assert!(!monitor.observe(None, now + Duration::from_secs(5), rule()));
        assert!(monitor.active_since().is_none());
    }

    #[test]
    fn changing_the_limit_restarts_an_existing_streak() {
        let now = Instant::now();
        let mut monitor = Monitor::default();
        monitor.observe(Some(95.0), now, rule());
        let changed = Threshold {
            celsius: 90.0,
            ..rule()
        };
        assert!(!monitor.observe(Some(95.0), now + rule().hold, changed));
        assert!(monitor.observe(Some(95.0), now + rule().hold * 2, changed));
    }

    #[test]
    fn cold_samples_clear_after_a_recovery_delay() {
        let now = Instant::now();
        let mut monitor = Monitor::default();
        for second in 0..4 {
            monitor.observe(Some(90.0), now + Duration::from_secs(second), rule());
        }
        monitor.observe(Some(70.0), now + Duration::from_secs(4), rule());
        assert_eq!(
            monitor.next_transition(rule()),
            Some(now + Duration::from_secs(6))
        );
        monitor.observe(Some(70.0), now + Duration::from_secs(5), rule());
        assert!(monitor.active_since().is_some());
        monitor.observe(Some(70.0), now + Duration::from_secs(6), rule());
        assert!(monitor.active_since().is_none());
        assert_eq!(monitor.next_transition(rule()), None);
    }

    #[test]
    fn invalid_rules_cannot_fire() {
        let now = Instant::now();
        let mut monitor = Monitor::default();
        let mut bad = rule();
        bad.celsius = f32::NAN;
        assert!(!monitor.observe(Some(200.0), now, bad));
        assert!(monitor.active_since().is_none());
    }
}
