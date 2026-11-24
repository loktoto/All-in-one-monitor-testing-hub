//! Bounded, offline analysis of a single PresentMon process and swap chain.
use std::{fs::File, io::Read, path::Path};
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ROWS: usize = 500_000;
#[derive(Clone, Debug)]
pub struct FrameSummary {
    pub application: String,
    pub process_id: String,
    pub swap_chain: String,
    pub column: String,
    pub valid: usize,
    pub skipped: usize,
    pub average_fps: f64,
    pub one_percent_low_fps: f64,
    pub p99_ms: f64,
    pub average_cpu_ms: Option<f64>,
    pub average_gpu_ms: Option<f64>,
}
pub fn read(path: &Path) -> Result<FrameSummary, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("Cannot inspect capture: {e}"))?;
    if !metadata.is_file() {
        return Err("Choose a regular CSV file.".into());
    }
    let file = File::open(path).map_err(|e| format!("Cannot open capture: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read capture: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Capture exceeds the 32 MiB limit.".into());
    }
    parse(&bytes)
}
fn parse(bytes: &[u8]) -> Result<FrameSummary, String> {
    let mut reader = csv::Reader::from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|e| format!("Invalid CSV header: {e}"))?
        .clone();
    let index = |name: &str| {
        headers
            .iter()
            .position(|h| h.trim_start_matches('\u{feff}').eq_ignore_ascii_case(name))
    };
    let application = index("Application").ok_or("Missing Application column.")?;
    let pid = index("ProcessID").ok_or("Missing ProcessID column.")?;
    let chain = index("SwapChainAddress").ok_or("Missing SwapChainAddress column.")?;
    let (time, column) = index("DisplayedTime")
        .map(|i| (i, "DisplayedTime"))
        .or_else(|| index("MsBetweenDisplayChange").map(|i| (i, "MsBetweenDisplayChange")))
        .ok_or("Missing displayed-frame time column; capture must include display metrics.")?;
    let cpu = index("MsCPUBusy");
    let gpu = index("MsGPUBusy");
    let mut identity: Option<(String, String, String)> = None;
    let mut times = Vec::new();
    let mut skipped = 0;
    let mut cpu_values = Vec::new();
    let mut gpu_values = Vec::new();
    for (n, row) in reader.records().enumerate() {
        if n >= MAX_ROWS {
            return Err("Capture exceeds the 500,000-row limit.".into());
        }
        let row = row.map_err(|e| format!("Invalid CSV row: {e}"))?;
        let id = (
            row.get(application).unwrap_or("").to_owned(),
            row.get(pid).unwrap_or("").to_owned(),
            row.get(chain).unwrap_or("").to_owned(),
        );
        if id.0.is_empty() || id.1.is_empty() || id.2.is_empty() {
            return Err("A row has incomplete process or swap-chain identity.".into());
        }
        if let Some(previous) = &identity {
            if previous != &id {
                return Err("Capture contains multiple processes or swap chains. Export or filter to one before importing.".into());
            }
        } else {
            identity = Some(id);
        }
        let value = |index: usize| {
            row.get(index)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
        };
        match value(time).filter(|v| *v > 0.0) {
            Some(ms) => times.push(ms),
            None => {
                skipped += 1;
                continue;
            }
        }
        if let Some(v) = cpu.and_then(value) {
            cpu_values.push(v);
        }
        if let Some(v) = gpu.and_then(value) {
            gpu_values.push(v);
        }
    }
    if times.is_empty() {
        return Err("No positive displayed-frame times found.".into());
    }
    times.sort_by(f64::total_cmp);
    let count = times.len();
    let slow_count = count.div_ceil(100);
    let average_fps = count as f64 * 1000.0 / times.iter().sum::<f64>();
    let one_percent_low_fps =
        slow_count as f64 * 1000.0 / times[count - slow_count..].iter().sum::<f64>();
    if !average_fps.is_finite()
        || average_fps <= 0.0
        || !one_percent_low_fps.is_finite()
        || one_percent_low_fps <= 0.0
    {
        return Err("Displayed-frame times are outside the supported numeric range.".into());
    }
    let p99_ms = times[(count * 99).div_ceil(100).saturating_sub(1)];
    let average = |values: Vec<f64>| {
        if values.is_empty() {
            None
        } else {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            mean.is_finite().then_some(mean)
        }
    };
    let (application, process_id, swap_chain) =
        identity.ok_or("No frame samples carried a process identity.")?;
    Ok(FrameSummary {
        application,
        process_id,
        swap_chain,
        column: column.into(),
        valid: count,
        skipped,
        average_fps,
        one_percent_low_fps,
        p99_ms,
        average_cpu_ms: average(cpu_values),
        average_gpu_ms: average(gpu_values),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn computes_display_intervals_and_excludes_missing_rows() {
        let r = parse(b"Application,ProcessID,SwapChainAddress,MsBetweenDisplayChange,MsCPUBusy\nGame,1,0x1,10,2\nGame,1,0x1,20,4\nGame,1,0x1,NA,5\n").unwrap();
        assert_eq!(r.valid, 2);
        assert_eq!(r.skipped, 1);
        assert!((r.average_fps - 66.6667).abs() < 0.01);
        assert_eq!(r.one_percent_low_fps, 50.0);
        assert_eq!(r.p99_ms, 20.0);
        assert_eq!(r.average_cpu_ms, Some(3.0));
    }
    #[test]
    fn rejects_numeric_overflow() {
        assert!(parse(b"Application,ProcessID,SwapChainAddress,DisplayedTime\nGame,1,0x1,1e308\nGame,1,0x1,1e308\n").is_err());
    }
    #[test]
    fn refuses_ambiguous_or_missing_display_data() {
        assert!(parse(
            b"Application,ProcessID,SwapChainAddress,MsBetweenPresents\nGame,1,0x1,10\n"
        )
        .is_err());
        assert!(parse(
            b"Application,ProcessID,SwapChainAddress,DisplayedTime\nGame,1,0x1,10\nGame,1,0x2,10\n"
        )
        .is_err());
    }
}
