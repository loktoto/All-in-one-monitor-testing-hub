//! Small, dependency-free settings store.
//!
//! The app writes a single JSON file next to the platform's per-user config
//! directory so a restart resumes the same page, sampling cadence, worker
//! count and alert thresholds. Every field is optional on read: a missing or
//! corrupt file silently falls back to the built-in defaults, so a bad config
//! can never stop the app from starting.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Everything worth remembering between runs. Kept deliberately small and
/// scalar so it can never hold a value that would be unsafe to restore.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Page label, e.g. "CPU" or "Thermal". Unknown labels fall back to Overview.
    pub page: String,
    /// Telemetry sampling interval in milliseconds.
    pub poll_ms: u64,
    /// Worker threads for the software checks.
    pub workers: usize,
    /// Default duration for a check, in seconds.
    pub seconds: u64,
    /// Default memory working set, in MiB.
    pub mib: usize,
    /// Disk scratch-file write cap, in MiB.
    pub disk_write_limit_mib: u64,
    /// Scratch directory used by the file I/O check.
    pub directory: String,
    /// Advisory alert threshold in Celsius.
    pub alert_celsius: f32,
    /// Seconds a reading must stay above the threshold before it fires.
    pub alert_hold_secs: u64,
    /// Opt-in: stop the software check when the peak reaches the threshold.
    pub auto_stop_on_heat: bool,
    /// Default export paths.
    pub history_path: String,
    pub log_path: String,
    pub sensor_log_path: String,
    pub report: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            page: "Overview".into(),
            poll_ms: 1000,
            workers: std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(1),
            seconds: 15,
            mib: 64,
            disk_write_limit_mib: 64,
            directory: std::env::temp_dir().to_string_lossy().into_owned(),
            alert_celsius: 85.0,
            alert_hold_secs: 5,
            auto_stop_on_heat: false,
            history_path: "hardware-hub-history.csv".into(),
            log_path: "hardware-hub-session.csv".into(),
            sensor_log_path: "hardware-hub-sensor.csv".into(),
            report: "hardware-hub-report.txt".into(),
        }
    }
}

impl Settings {
    /// Directory that holds the settings file for the current user.
    pub fn config_dir() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("hardware-hub"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
                if !xdg.is_empty() {
                    return Some(PathBuf::from(xdg).join("hardware-hub"));
                }
            }
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config").join("hardware-hub"))
        }
    }

    /// Full path of the JSON settings file, if a config directory is known.
    pub fn path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("settings.json"))
    }

    /// Read settings from disk. Any failure (missing file, bad JSON, no config
    /// directory) yields the defaults so startup is never blocked.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Write settings to disk, creating the config directory if needed. The
    /// write goes through a temporary file and an atomic rename so a crash
    /// mid-write cannot leave a truncated file behind.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "no config directory available".to_owned())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("serialise: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("replace {}: {e}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert!(s.poll_ms >= 250);
        assert!(s.workers >= 1);
        assert!(s.alert_celsius > 0.0);
        assert!(!s.page.is_empty());
    }

    #[test]
    fn missing_fields_fall_back() {
        // An older or hand-edited file that only carries a few keys still loads.
        let partial = r#"{"page":"CPU","poll_ms":500}"#;
        let s: Settings = serde_json::from_str(partial).unwrap();
        assert_eq!(s.page, "CPU");
        assert_eq!(s.poll_ms, 500);
        assert_eq!(s.workers, Settings::default().workers);
    }

    #[test]
    fn roundtrips_through_json() {
        let s = Settings {
            page: "Reports".into(),
            poll_ms: 750,
            alert_celsius: 90.0,
            ..Settings::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(s, back);
    }
}
