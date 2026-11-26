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
                    crate::stress::Kind::Cpu=>format!("workers {}, workload {}",r.config.workers,r.config.cpu_workload.label()),
                    crate::stress::Kind::Gpu=>format!("GPU {}",r.config.gpu_adapter.as_deref().unwrap_or("not selected")),
                    crate::stress::Kind::Memory=>format!("memory {} MiB",r.config.mib),
                    crate::stress::Kind::Disk=>format!("file {} MiB, write cap {} MiB, directory {}",r.config.mib,r.config.disk_write_limit_mib,r.config.directory.display()),
                    crate::stress::Kind::Combined=>format!("workers {}, memory {} MiB, pattern {}",r.config.workers,r.config.mib,r.config.mem_pattern.label()),
                };
                let run_limit = match r.config.run_mode {
                    crate::stress::RunMode::Timed => format!("duration {}", crate::stress::format_duration(r.config.seconds)),
                    crate::stress::RunMode::Continuous => "continuous (until stopped)".into(),
                    crate::stress::RunMode::Passes => format!("{} target pass(es)", r.config.target_passes),
                };
                let age=r.context.snapshot_age_seconds.map(|v|format!("{v:.1} s")).unwrap_or_else(||"unavailable".into());
                out.push_str(&format!("{:?} | {} | {:.1}s | Unix {}\n{}\nVerified {}: {}\nSettings: {}, {}\nSystem: {}, {} logical CPUs, {} bytes RAM, OS {}, adapters: {}; snapshot age at start: {}\n\n",r.kind,r.status,r.elapsed,r.timestamp,r.detail,r.verified_unit,r.verified_units.map(|n|n.to_string()).unwrap_or_else(||"unknown".into()),run_limit,settings,r.context.cpu,r.context.logical_cpus,r.context.memory_total,r.context.os,r.context.adapters.join(", "),age));
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

/// Render a single-run, printable summary in the style of a stability
/// certificate. Deliberately honest: it records what was checked and observed,
/// and states that no benchmark score or hardware certification is implied.
/// Paths, adapter names and other identifiers are omitted so it can be shared
/// without exposing the machine.
pub fn render_certificate(record: &ResultRecord, peak_c: Option<f32>) -> String {
    let verdict = match record.status {
        crate::stress::Status::Completed => "PASSED (bounded check)",
        crate::stress::Status::Stopped => "STOPPED BY USER",
        crate::stress::Status::Failed => "FAILED",
        crate::stress::Status::Inconclusive => "INCONCLUSIVE",
        crate::stress::Status::Unavailable => "UNAVAILABLE",
    };
    let settings = match record.kind {
        crate::stress::Kind::Cpu => format!("CPU workers: {}", record.config.workers),
        crate::stress::Kind::Gpu => "GPU compute dispatch check".to_string(),
        crate::stress::Kind::Memory => format!(
            "Memory: {} MiB, pattern {}",
            record.config.mib,
            record.config.mem_pattern.label()
        ),
        crate::stress::Kind::Disk => format!(
            "File: {} MiB, write cap {} MiB (directory omitted)",
            record.config.mib, record.config.disk_write_limit_mib
        ),
        crate::stress::Kind::Combined => format!(
            "CPU workers: {}, memory {} MiB, pattern {}",
            record.config.workers,
            record.config.mib,
            record.config.mem_pattern.label()
        ),
    };
    let verified = record
        .verified_units
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unknown".into());
    let peak = peak_c
        .map(|v| format!("{v:.0} °C observed during this run"))
        .unwrap_or_else(|| "not recorded".into());
    let timestamp = format!("Unix {}", record.timestamp);
    let check = format!("{:?}", record.kind);
    let gib = record.context.memory_total as f64 / 1073741824.0;
    let limit_desc = match record.config.run_mode {
        crate::stress::RunMode::Timed => {
            format!(
                "limit {}",
                crate::stress::format_duration(record.config.seconds)
            )
        }
        crate::stress::RunMode::Continuous => "continuous".into(),
        crate::stress::RunMode::Passes => format!("{} pass(es)", record.config.target_passes),
    };
    format!(
        "HARDWARE HUB · RUN SUMMARY\n\
         version {version} · generated {timestamp}\n\
         \n\
         Result:      {verdict}\n\
         Check:       {check}\n\
         Run at:      {timestamp}\n\
         Duration:    {elapsed:.1}s ({limit_desc})\n\
         \n\
         Verified:    {verified} {unit}\n\
         Detail:      {detail}\n\
         Mismatches:  {errors} total\n\
         Peak temp:   {peak}\n\
         \n\
         Settings:    {settings}\n\
         System:      {cpu} · {logical} logical CPUs · {gib:.1} GiB RAM\n\
         OS:          {os}\n\
         \n\
         This is a bounded correctness check, not a benchmark score, a full\n\
         burn-in, or a hardware health certification. No fan, clock or other\n\
         hardware setting was changed. A supervised, physical inspection is\n\
         still required before relying on any component.\n",
        version = env!("CARGO_PKG_VERSION"),
        elapsed = record.elapsed,
        unit = record.verified_unit,
        detail = record.detail,
        errors = record.error_count,
        cpu = record.context.cpu,
        logical = record.context.logical_cpus,
        os = record.context.os,
    )
}

pub fn export_certificate(
    path: &Path,
    record: &ResultRecord,
    peak_c: Option<f32>,
) -> Result<(), String> {
    let data = render_certificate(record, peak_c);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create certificate: {e}"))?;
    file.write_all(data.as_bytes())
        .map_err(|e| format!("Certificate write failed; a partial file may remain: {e}"))
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
                ..Config::default()
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

    #[test]
    fn certificate_is_shareable_and_honest_about_scope() {
        let dir = std::env::temp_dir().join(format!(
            "hub-cert-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let record = ResultRecord {
            kind: Kind::Disk,
            status: Status::Completed,
            detail: "1 verified pass".into(),
            verified_units: Some(1),
            verified_unit: "file passes",
            elapsed: 1.0,
            timestamp: 0,
            config: Config {
                kind: Kind::Disk,
                seconds: 1,
                workers: 1,
                mib: 1,
                directory: dir.clone(),
                gpu_adapter: None,
                disk_write_limit_mib: 1,
                mem_pattern: MemPattern::Alternating,
                disk_pattern: DiskPattern::Constant,
                gpu_intensity: GpuIntensity::Standard,
                ..Config::default()
            },
            context: ReportContext::default(),
            error_count: 0,
            errors: Vec::new(),
        };
        let text = render_certificate(&record, Some(55.0));
        assert!(text.contains("PASSED (bounded check)"));
        assert!(text.contains("55 °C"));
        assert!(text.contains("not a benchmark score"));
        assert!(!text.contains(dir.to_string_lossy().as_ref()));
        let path = dir.join("certificate.txt");
        export_certificate(&path, &record, Some(55.0)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(export_certificate(&path, &record, Some(55.0)).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
