//! Optional temperature alerts, in the spirit of HWiNFO/HWMonitor thresholds.
//!
//! The alert is advisory only: it never changes fan speed, clocks or test
//! behaviour. It requires a fresh, trusted reading and a sustained breach so a
//! single sensor spike cannot flip the state. Missing readings are not zeros.

use std::time::{Duration, Instant};

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

#[derive(Clone, Copy, Debug)]
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

#[derive(Default)]
pub struct Monitor {
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

    /// Feed the highest currently available temperature. `None` means no fresh
    /// reading; it pauses the streak rather than treating the sensor as cold.
    /// Returns true on the frame the alert first becomes active.
    pub fn observe(&mut self, temperature: Option<f32>, now: Instant, rule: Threshold) -> bool {
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
                    if now.saturating_duration_since(recovered) >= Duration::from_secs(2) {
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
    fn short_spikes_do_not_trigger_but_sustained_heat_does() {
        let now = Instant::now();
        let mut monitor = Monitor::default();
        assert!(!monitor.observe(Some(95.0), now, rule()));
        assert!(!monitor.observe(Some(20.0), now + Duration::from_secs(1), rule()));
        assert!(!monitor.observe(Some(95.0), now + Duration::from_secs(2), rule()));
        assert!(!monitor.observe(Some(95.0), now + Duration::from_secs(3), rule()));
        assert!(!monitor.observe(Some(95.0), now + Duration::from_secs(4), rule()));
        assert!(monitor.observe(Some(95.0), now + Duration::from_secs(5), rule()));
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
    fn cold_samples_clear_after_a_recovery_delay() {
        let now = Instant::now();
        let mut monitor = Monitor::default();
        for second in 0..4 {
            monitor.observe(Some(90.0), now + Duration::from_secs(second), rule());
        }
        monitor.observe(Some(70.0), now + Duration::from_secs(4), rule());
        monitor.observe(Some(70.0), now + Duration::from_secs(5), rule());
        assert!(monitor.active_since().is_some());
        monitor.observe(Some(70.0), now + Duration::from_secs(6), rule());
        assert!(monitor.active_since().is_none());
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
