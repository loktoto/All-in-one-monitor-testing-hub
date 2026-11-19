use crate::stress::ResultRecord;
use std::{fs::OpenOptions, io::Write, path::Path};

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Text,
    Json,
}

pub fn export(path: &Path, format: Format, records: &[ResultRecord]) -> Result<(), String> {
    if records.is_empty() {
        return Err("No test results to export.".into());
    }
    let data = match format {
        Format::Json => serde_json::to_vec_pretty(&serde_json::json!({"schema_version":1,"app_version":env!("CARGO_PKG_VERSION"),"results":records})).map_err(|e| e.to_string())?,
        Format::Text => {
            let mut out=format!("Hardware Hub report · version {}\n\n",env!("CARGO_PKG_VERSION"));
            for r in records {
                let settings=match r.kind {
                    crate::stress::Kind::Cpu=>format!("workers {}",r.config.workers),
                    crate::stress::Kind::Gpu=>format!("GPU {}",r.config.gpu_adapter.as_deref().unwrap_or("not selected")),
                    crate::stress::Kind::Memory=>format!("memory {} MiB",r.config.mib),
                    crate::stress::Kind::Disk=>format!("file {} MiB, write cap {} MiB, directory {}",r.config.mib,r.config.disk_write_limit_mib,r.config.directory.display()),
                    crate::stress::Kind::Combined=>format!("workers {}, memory {} MiB, pattern {}",r.config.workers,r.config.mib,r.config.mem_pattern.label()),
                };
                let age=r.context.snapshot_age_seconds.map(|v|format!("{v:.1} s")).unwrap_or_else(||"unavailable".into());
                out.push_str(&format!("{:?} | {} | {:.1}s | Unix {}\n{}\nVerified {}: {}\nSettings: duration {}s, {}\nSystem: {}, {} logical CPUs, {} bytes RAM, OS {}, adapters: {}; snapshot age at start: {}\n\n",r.kind,r.status,r.elapsed,r.timestamp,r.detail,r.verified_unit,r.verified_units.map(|n|n.to_string()).unwrap_or_else(||"unknown".into()),r.config.seconds,settings,r.context.cpu,r.context.logical_cpus,r.context.memory_total,r.context.os,r.context.adapters.join(", "),age));
            }
            out.into_bytes()
        }
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create report: {e}"))?;
    file.write_all(&data)
        .map_err(|e| format!("Report write failed; a partial file may remain: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stress::{
        Config, DiskPattern, GpuIntensity, Kind, MemPattern, ReportContext, ResultRecord, Status,
    };
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    #[test]
    fn exports_json_with_relevant_configuration_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!(
            "hub-report-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("report.json");
        let record = ResultRecord {
            kind: Kind::Cpu,
            status: Status::Completed,
            detail: "verified".into(),
            verified_units: Some(1),
            verified_unit: "compute rounds",
            elapsed: 1.0,
            timestamp: 0,
            config: Config {
                kind: Kind::Cpu,
                seconds: 1,
                workers: 2,
                mib: 1,
                directory: dir.clone(),
                gpu_adapter: None,
                disk_write_limit_mib: 1,
                mem_pattern: MemPattern::Alternating,
                disk_pattern: DiskPattern::Constant,
                gpu_intensity: GpuIntensity::Standard,
            },
            context: ReportContext::default(),
            error_count: 0,
            errors: Vec::new(),
        };
        export(&path, Format::Json, std::slice::from_ref(&record)).unwrap();
        let original = fs::read(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(json["results"][0]["status"], "Completed");
        assert_eq!(json["results"][0]["config"]["workers"], 2);
        assert!(json["results"][0]["config"].get("directory").is_none());
        assert!(export(&path, Format::Text, &[record]).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
