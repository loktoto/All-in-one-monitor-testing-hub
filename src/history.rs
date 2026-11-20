//! Bounded, session-only history of completed telemetry batches.
use crate::telemetry::Snapshot;
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

const LIMIT: usize = 60;
const MAX_AGE: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct Sample {
    pub at: Instant,
    pub cpu: Option<f64>,
    pub memory: Option<f64>,
    pub cores: Vec<Option<f64>>,
    /// Provider key and value for each temperature seen in this batch.
    pub temperatures: Vec<(String, Option<f64>)>,
}

#[derive(Default)]
pub struct History {
    samples: VecDeque<Sample>,
    last_batch: Option<Instant>,
}

impl History {
    pub fn record(&mut self, snapshot: &Snapshot, now: Instant) {
        let Some(at) = snapshot.updated_at else {
            return;
        };
        if !snapshot.ready
            || self.last_batch.is_some_and(|last| at <= last)
            || now
                .checked_duration_since(at)
                .is_none_or(|age| age >= MAX_AGE)
        {
            return;
        }
        self.last_batch = Some(at);
        self.samples.push_back(Sample {
            at,
            cpu: (!snapshot.cores.is_empty() && snapshot.cpu_usage.is_finite())
                .then_some(snapshot.cpu_usage as f64),
            memory: (snapshot.memory_total > 0)
                .then(|| snapshot.memory_used as f64 * 100.0 / snapshot.memory_total as f64),
            cores: snapshot
                .cores
                .iter()
                .map(|v| v.is_finite().then_some(*v as f64))
                .collect(),
            temperatures: snapshot
                .temperatures
                .iter()
                .map(|t| {
                    (
                        t.key.clone(),
                        t.celsius.filter(|v| v.is_finite()).map(f64::from),
                    )
                })
                .collect(),
        });
        if self.samples.len() > LIMIT {
            self.samples.pop_front();
        }
    }
    pub fn clear(&mut self) {
        self.samples.clear();
    }
    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
    pub fn samples(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter()
    }
}

/// Export the retained window only. Empty fields represent unavailable readings.
pub fn export_csv(path: &Path, history: &History, origin: Instant) -> Result<(), String> {
    if history.is_empty() {
        return Err("No history samples to export.".into());
    }
    let mut data = String::from("elapsed_seconds,cpu_percent,memory_percent\n");
    for sample in history.samples() {
        let cpu = sample.cpu.map(|v| format!("{v:.3}")).unwrap_or_default();
        let memory = sample.memory.map(|v| format!("{v:.3}")).unwrap_or_default();
        data.push_str(&format!(
            "{:.3},{cpu},{memory}\n",
            sample.at.saturating_duration_since(origin).as_secs_f64()
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create history file: {e}"))?;
    file.write_all(data.as_bytes())
        .map_err(|e| format!("History write failed; a partial file may remain: {e}"))
}

/// Split lines on missing samples or long collection gaps rather than inventing readings.
/// Uses the default 3 s gap (kept for internal callers and tests).
pub fn segments(
    history: &History,
    origin: Instant,
    value: impl Fn(&Sample) -> Option<f64>,
) -> Vec<Vec<[f64; 2]>> {
    segments_with_gap(history, origin, MAX_AGE, value)
}

/// Like [`segments`] but with an explicit gap. The caller passes roughly twice
/// its configured sampling interval, so a slow 5 s poll still draws a continuous
/// line instead of splitting every pair of samples into a one-point segment.
pub fn segments_with_gap(
    history: &History,
    origin: Instant,
    gap: Duration,
    value: impl Fn(&Sample) -> Option<f64>,
) -> Vec<Vec<[f64; 2]>> {
    let mut result: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut current = Vec::new();
    let mut previous = None;
    for sample in history.samples() {
        if previous.is_some_and(|at: Instant| sample.at.duration_since(at) >= gap)
            || value(sample).is_none()
        {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
        }
        if let Some(v) = value(sample) {
            current.push([sample.at.saturating_duration_since(origin).as_secs_f64(), v]);
        }
        previous = Some(sample.at);
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub fn range(history: &History, value: impl Fn(&Sample) -> Option<f64>) -> Option<(f64, f64, f64)> {
    let values: Vec<_> = history.samples().filter_map(value).collect();
    if values.is_empty() {
        return None;
    }
    Some((
        values.iter().copied().fold(f64::INFINITY, f64::min),
        values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        values.iter().sum::<f64>() / values.len() as f64,
    ))
}

/// Sensor keys in first-seen order, so a graph appears without configuration.
pub fn temperature_keys(history: &History) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for sample in history.samples() {
        for (key, _) in &sample.temperatures {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

pub fn temperature_value<'a>(sample: &'a Sample, key: &str) -> Option<f64> {
    sample
        .temperatures
        .iter()
        .find(|(candidate, _)| candidate == key)
        .and_then(|(_, value)| *value)
}

/// CSV for one selected sensor. Sensor keys are quoted when required; a gap is
/// left blank rather than connected or filled with zero.
pub fn export_sensor_csv(
    path: &Path,
    history: &History,
    key: &str,
    origin: Instant,
) -> Result<(), String> {
    if history.is_empty() {
        return Err("No history samples to export.".into());
    }
    let mut data = String::from("elapsed_seconds,sensor,celsius\n");
    for sample in history.samples() {
        let value = temperature_value(sample, key)
            .map(|v| format!("{v:.3}"))
            .unwrap_or_default();
        data.push_str(&format!(
            "{:.3},{},{value}\n",
            sample.at.saturating_duration_since(origin).as_secs_f64(),
            csv_field(key),
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create sensor history file: {e}"))?;
    file.write_all(data.as_bytes())
        .map_err(|e| format!("Sensor history write failed; a partial file may remain: {e}"))
}

/// Wide CSV: one column per temperature sensor, so all series can be compared
/// in a spreadsheet. Keys are quoted when needed; a missing sample is blank,
/// never zero or interpolated.
pub fn export_all_sensors_csv(
    path: &Path,
    history: &History,
    origin: Instant,
) -> Result<(), String> {
    if history.is_empty() {
        return Err("No history samples to export.".into());
    }
    let keys = temperature_keys(history);
    if keys.is_empty() {
        return Err("No temperature sensor was recorded yet.".into());
    }
    let mut data = String::from("elapsed_seconds");
    for key in &keys {
        data.push(',');
        data.push_str(&csv_field(key));
    }
    data.push('\n');
    for sample in history.samples() {
        data.push_str(&format!(
            "{:.3}",
            sample.at.saturating_duration_since(origin).as_secs_f64()
        ));
        for key in &keys {
            let value = temperature_value(sample, key)
                .map(|v| format!("{v:.3}"))
                .unwrap_or_default();
            data.push(',');
            data.push_str(&value);
        }
        data.push('\n');
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create sensor history file: {e}"))?;
    file.write_all(data.as_bytes())
        .map_err(|e| format!("Sensor history write failed; a partial file may remain: {e}"))
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::Temperature;
    fn snapshot(at: Instant) -> Snapshot {
        Snapshot {
            ready: true,
            updated_at: Some(at),
            cores: vec![1.0],
            cpu_usage: 0.0,
            memory_total: 100,
            memory_used: 50,
            ..Default::default()
        }
    }
    #[test]
    fn records_batches_once_and_does_not_readd_after_reset() {
        let now = Instant::now();
        let mut h = History::default();
        let s = snapshot(now);
        h.record(&s, now);
        h.record(&s, now);
        assert_eq!(h.len(), 1);
        h.clear();
        h.record(&s, now);
        assert!(h.is_empty());
        h.record(
            &snapshot(now + Duration::from_secs(1)),
            now + Duration::from_secs(1),
        );
        assert_eq!(h.len(), 1);
    }
    #[test]
    fn missing_and_stale_data_are_not_zero_or_connected() {
        let now = Instant::now();
        let mut h = History::default();
        h.record(&snapshot(now), now);
        let mut missing = snapshot(now + Duration::from_secs(1));
        missing.cores.clear();
        h.record(&missing, now + Duration::from_secs(1));
        h.record(
            &snapshot(now + Duration::from_secs(5)),
            now + Duration::from_secs(5),
        );
        h.record(
            &snapshot(now + Duration::from_secs(6)),
            now + Duration::from_secs(10),
        );
        assert_eq!(h.len(), 3);
        assert_eq!(segments(&h, now, |s| s.cpu).len(), 2);
        assert_eq!(range(&h, |s| s.cpu), Some((0.0, 0.0, 0.0)));
    }
    #[test]
    fn csv_preserves_missing_values_and_never_overwrites() {
        let now = Instant::now();
        let mut history = History::default();
        let mut sample = snapshot(now);
        sample.cores.clear();
        history.record(&sample, now);
        let path = std::env::temp_dir().join(format!(
            "hardware-hub-history-{}-{}.csv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        export_csv(&path, &history, now).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            "elapsed_seconds,cpu_percent,memory_percent\n0.000,,50.000\n"
        );
        assert!(export_csv(&path, &history, now).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn sensor_history_keeps_missing_values_and_exports_one_sensor() {
        let now = Instant::now();
        let mut history = History::default();
        let mut sample = snapshot(now);
        sample.temperatures.push(Temperature {
            key: "TC0P".into(),
            label: "CPU".into(),
            celsius: Some(61.5),
            min: None,
            max: None,
        });
        sample.temperatures.push(Temperature {
            key: "TCSF".into(),
            label: "Skin".into(),
            celsius: Some(40.0),
            min: None,
            max: None,
        });
        history.record(&sample, now);
        let mut later = snapshot(now + Duration::from_secs(1));
        later.temperatures.push(Temperature {
            key: "TC0P".into(),
            label: "CPU".into(),
            celsius: None,
            min: None,
            max: None,
        });
        history.record(&later, now + Duration::from_secs(1));
        assert_eq!(temperature_keys(&history), vec!["TC0P", "TCSF"]);
        assert_eq!(
            segments(&history, now, |s| temperature_value(s, "TC0P")).len(),
            1
        );
        assert_eq!(
            range(&history, |s| temperature_value(s, "TC0P")),
            Some((61.5, 61.5, 61.5))
        );
        let path = std::env::temp_dir().join(format!(
            "hardware-hub-sensor-history-{}-{}.csv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        export_sensor_csv(&path, &history, "TC0P", now).unwrap();
        assert_eq!(
            std::str::from_utf8(&std::fs::read(&path).unwrap()).unwrap(),
            "elapsed_seconds,sensor,celsius\n0.000,TC0P,61.500\n1.000,TC0P,\n"
        );
        assert!(export_sensor_csv(&path, &history, "TC0P", now).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn all_sensor_csv_has_one_column_each_and_keeps_gaps_blank() {
        let now = Instant::now();
        let mut history = History::default();
        let mut sample = snapshot(now);
        sample.temperatures.push(Temperature {
            key: "TC0P".into(),
            label: "CPU".into(),
            celsius: Some(61.5),
            min: None,
            max: None,
        });
        sample.temperatures.push(Temperature {
            key: "TCSF".into(),
            label: "Skin".into(),
            celsius: Some(40.0),
            min: None,
            max: None,
        });
        history.record(&sample, now);
        let mut later = snapshot(now + Duration::from_secs(1));
        later.temperatures.push(Temperature {
            key: "TC0P".into(),
            label: "CPU".into(),
            celsius: Some(63.0),
            min: None,
            max: None,
        });
        // TCSF missing in the second sample: blank, never zero.
        history.record(&later, now + Duration::from_secs(1));
        let path = std::env::temp_dir().join(format!(
            "hardware-hub-all-sensors-{}-{}.csv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        export_all_sensors_csv(&path, &history, now).unwrap();
        assert_eq!(
            std::str::from_utf8(&std::fs::read(&path).unwrap()).unwrap(),
            "elapsed_seconds,TC0P,TCSF\n0.000,61.500,40.000\n1.000,63.000,\n"
        );
        assert!(export_all_sensors_csv(&path, &history, now).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn history_is_bounded() {
        let now = Instant::now();
        let mut h = History::default();
        for n in 0..100 {
            let at = now + Duration::from_secs(n);
            h.record(&snapshot(at), at);
        }
        assert_eq!(h.len(), 60);
    }
}
