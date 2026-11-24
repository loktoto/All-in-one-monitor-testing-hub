//! Opt-in bounded asynchronous CSV logging of telemetry batches.
use crate::telemetry::Snapshot;
use std::{
    fs::OpenOptions,
    path::Path,
    sync::{
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const MAX_ROWS: usize = 1_000_000;
#[derive(Default, Clone)]
pub struct LogStatus {
    pub batches: u64,
    pub dropped: u64,
    pub error: Option<String>,
    pub closed: bool,
}
struct Batch {
    unix_ms: u128,
    rows: Vec<(String, String, String, Option<f64>, String)>,
}
pub struct Recorder {
    sender: Option<SyncSender<Batch>>,
    status: Arc<Mutex<LogStatus>>,
    last: Option<Instant>,
}
impl Recorder {
    pub fn start(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("Cannot create log: {e}"))?;
        let (sender, receiver) = mpsc::sync_channel::<Batch>(32);
        let status = Arc::new(Mutex::new(LogStatus::default()));
        let worker_status = status.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let mut writer = csv::Writer::from_writer(file);
                writer
                    .write_record([
                        "recorded_unix_ms",
                        "category",
                        "sensor_key",
                        "label",
                        "value",
                        "unit",
                    ])
                    .map_err(|e| e.to_string())?;
                let mut count = 0;
                for batch in receiver {
                    if count + batch.rows.len() > MAX_ROWS {
                        return Err("Stopped at the 1,000,000-row limit.".into());
                    }
                    for (category, key, label, value, unit) in batch.rows {
                        writer
                            .write_record([
                                batch.unix_ms.to_string(),
                                category,
                                key,
                                label,
                                value.map(|v| v.to_string()).unwrap_or_default(),
                                unit,
                            ])
                            .map_err(|e| e.to_string())?;
                        count += 1;
                    }
                    writer.flush().map_err(|e| e.to_string())?;
                    if let Ok(mut s) = worker_status.lock() {
                        s.batches += 1;
                    }
                }
                writer.flush().map_err(|e| e.to_string())?;
                Ok(())
            })();
            if let Ok(mut s) = worker_status.lock() {
                if let Err(error) = result {
                    s.error = Some(error);
                }
                s.closed = true;
            }
        });
        Ok(Self {
            sender: Some(sender),
            status,
            last: None,
        })
    }
    pub fn record(&mut self, snapshot: &Snapshot, now: Instant) {
        let Some(at) = snapshot.updated_at else {
            return;
        };
        if !snapshot.ready
            || self.last.is_some_and(|last| at <= last)
            || now
                .checked_duration_since(at)
                .is_none_or(|age| age >= Duration::from_secs(3))
        {
            return;
        }
        self.last = Some(at);
        let Some(sender) = &self.sender else { return };
        let unix_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(t) => t.as_millis(),
            Err(_) => {
                if let Ok(mut s) = self.status.lock() {
                    s.error = Some("System clock is before Unix epoch.".into());
                }
                self.sender = None;
                return;
            }
        };
        let mut rows = Vec::new();
        if !snapshot.cores.is_empty() {
            rows.push((
                "cpu".into(),
                "total".into(),
                "CPU usage".into(),
                snapshot
                    .cpu_usage
                    .is_finite()
                    .then_some(snapshot.cpu_usage as f64),
                "%".into(),
            ));
        }
        if snapshot.memory_total > 0 {
            rows.push((
                "memory".into(),
                "used".into(),
                "Memory usage".into(),
                Some(snapshot.memory_used as f64 * 100.0 / snapshot.memory_total as f64),
                "%".into(),
            ));
        }
        for t in &snapshot.temperatures {
            rows.push((
                "temperature".into(),
                t.key.clone(),
                t.label.clone(),
                t.celsius.filter(|v| v.is_finite()).map(f64::from),
                "°C".into(),
            ));
        }
        for f in &snapshot.fans {
            rows.push((
                "fan".into(),
                f.key.clone(),
                f.label.clone(),
                f.rpm.filter(|v| v.is_finite()).map(f64::from),
                "RPM".into(),
            ));
        }
        match sender.try_send(Batch { unix_ms, rows }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                if let Ok(mut s) = self.status.lock() {
                    s.dropped += 1;
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                self.sender = None;
            }
        }
    }
    pub fn stop(&mut self) {
        self.sender = None;
    }
    pub fn status(&self) -> LogStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| LogStatus {
                error: Some("Cannot read logging status.".into()),
                closed: true,
                ..Default::default()
            })
    }
    pub fn active(&self) -> bool {
        self.sender.is_some() && !self.status().closed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_unique_batches_and_preserves_existing_file() {
        let path = std::env::temp_dir().join(format!(
            "hub-log-{}-{}.csv",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let now = Instant::now();
        let snapshot = Snapshot {
            ready: true,
            updated_at: Some(now),
            cpu_usage: 12.0,
            cores: vec![12.0],
            memory_total: 100,
            memory_used: 50,
            ..Default::default()
        };
        let mut log = Recorder::start(&path).unwrap();
        log.record(&snapshot, now);
        log.record(&snapshot, now);
        log.stop();
        for _ in 0..100 {
            if log.status().closed {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(log.status().closed);
        assert_eq!(log.status().batches, 1);
        assert!(log.status().error.is_none());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 3);
        assert!(Recorder::start(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
