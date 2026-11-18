//! Bounded local smoke run: CPU, 1 MiB RAM, 1 MiB file, unavailable GPU, JSON export.
use hardware_hub::{
    report::{self, Format},
    stress::{self, Config, Kind, ReportContext, State, Status},
    telemetry::{self, Snapshot},
};
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!(
        "hub-smoke-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&root)?;
    let snapshot = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
    let stop = Arc::new(AtomicBool::new(false));
    let reset = Arc::new(AtomicBool::new(false));
    let interval = Arc::new(AtomicU64::new(1000));
    let telemetry = telemetry::spawn(
        snapshot.clone(),
        stop.clone(),
        reset.clone(),
        interval.clone(),
    );
    let until = Instant::now() + Duration::from_secs(20);
    while !snapshot.lock().unwrap().ready && Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }
    let sample = snapshot.lock().unwrap().clone();
    if !sample.ready {
        return Err("No telemetry sample within 20 seconds".into());
    }
    println!("Telemetry: CPU {}, {} logical CPUs, {} bytes RAM, {} volumes, {} adapters, {} temperature sensors, {} fans",sample.cpu_name,sample.cores.len(),sample.memory_total,sample.disks.len(),sample.adapters.len(),sample.temperatures.len(),sample.fans.len());
    println!(
        "Graphics sources: {} total, {} IORegistry utilization sources",
        sample.gpus.len(),
        sample
            .gpus
            .iter()
            .filter(|g| g.name.starts_with("IORegistry source:"))
            .count()
    );
    if !sample.processes.iter().any(|p| p.pid == std::process::id()) {
        return Err("The telemetry process list did not include the smoke process".into());
    }
    println!(
        "Process telemetry: own PID found among {} reported processes",
        sample.processes.len()
    );
    println!(
        "CPU range: {:?}-{:?}% · memory range: {:?}-{:?}%",
        sample.cpu_range.min,
        sample.cpu_range.max,
        sample.memory_range.min,
        sample.memory_range.max
    );
    let state = Arc::new(Mutex::new(State::default()));
    let cancel = Arc::new(AtomicBool::new(false));
    let context = ReportContext {
        cpu: sample.cpu_name.clone(),
        logical_cpus: sample.cores.len(),
        memory_total: sample.memory_total,
        adapters: sample
            .adapters
            .iter()
            .map(|a| format!("{} · {}", a.name, a.backend))
            .collect(),
        os: sysinfo::System::long_os_version().unwrap_or_default(),
        snapshot_age_seconds: sample.updated_at.map(|x| x.elapsed().as_secs_f64()),
    };
    // Exercise the public cancellation path and restart before the short checks.
    let cancel_config = Config {
        kind: Kind::Cpu,
        seconds: 2,
        workers: 1,
        mib: 1,
        directory: root.clone(),
        gpu_adapter: None,
        disk_write_limit_mib: 1,
    };
    stress::start(
        cancel_config,
        state.clone(),
        cancel.clone(),
        None,
        context.clone(),
    )?;
    thread::sleep(Duration::from_millis(50));
    cancel.store(true, Ordering::Relaxed);
    let until = Instant::now() + Duration::from_secs(20);
    while state.lock().unwrap().running && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    {
        let guard = state.lock().unwrap();
        if guard.running || guard.results.last().map(|r| &r.status) != Some(&Status::Stopped) {
            return Err("CPU cancellation did not reach Stopped".into());
        }
    }
    println!("CPU cancellation: Stopped; restarting");
    for kind in [Kind::Cpu, Kind::Memory, Kind::Disk, Kind::Gpu] {
        let cfg = Config {
            kind,
            seconds: 1,
            workers: 1,
            mib: 1,
            directory: root.clone(),
            gpu_adapter: None,
            disk_write_limit_mib: 1,
        };
        // No physical GPU workload is launched by this smoke test.
        stress::start(cfg, state.clone(), cancel.clone(), None, context.clone())?;
        let until = Instant::now() + Duration::from_secs(20);
        while state.lock().unwrap().running && Instant::now() < until {
            thread::sleep(Duration::from_millis(20));
        }
        let guard = state.lock().unwrap();
        if guard.running {
            cancel.store(true, Ordering::Relaxed);
            return Err(format!("{kind:?} did not stop").into());
        }
        let result = guard.results.last().ok_or("missing result")?;
        let expected = if kind == Kind::Gpu {
            Status::Unavailable
        } else {
            Status::Completed
        };
        println!("{kind:?}: {} — {}", result.status, result.detail);
        if result.status != expected {
            return Err(format!("{kind:?}: expected {expected}, got {}", result.status).into());
        }
    }
    if fs::read_dir(&root)?.next().is_some() {
        return Err("Scratch file was not removed".into());
    }
    let report_path = root.join("report.json");
    let records = state.lock().unwrap().results.clone();
    report::export(&report_path, Format::Json, &records)?;
    let bytes = fs::read(&report_path)?;
    let report_json: serde_json::Value = serde_json::from_slice(&bytes)?;
    if report_json["results"].as_array().map(Vec::len) != Some(5) {
        return Err("report did not contain five results".into());
    }
    if report::export(&report_path, Format::Json, &records).is_ok()
        || fs::read(&report_path)? != bytes
    {
        return Err("report overwrite protection failed".into());
    }
    println!("JSON report, overwrite protection and scratch cleanup: OK");
    fs::remove_file(report_path)?;
    fs::remove_dir(&root)?;
    stop.store(true, Ordering::Relaxed);
    telemetry.join().map_err(|_| "telemetry thread panicked")?;
    Ok(())
}
