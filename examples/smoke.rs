//! Bounded local smoke run: CPU, 1 MiB RAM, 1 MiB file, unavailable GPU, JSON export.
use hardware_hub::{
    report::{self, Format},
    stress::{
        self, Config, DiskPattern, GpuIntensity, Kind, MemPattern, ReportContext, State, Status,
    },
    telemetry::{self, Snapshot},
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut test_gpu = false;
    let mut worker_executable = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--gpu" {
            test_gpu = true;
        } else if arg == "--gpu-worker-exe" {
            test_gpu = true;
            worker_executable = Some(PathBuf::from(
                args.next().ok_or("Missing GPU worker executable path")?,
            ));
        } else {
            return Err(format!("Unknown smoke option: {}", arg.to_string_lossy()).into());
        }
    }
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
        Arc::new(AtomicBool::new(true)),
    );
    let until = Instant::now() + Duration::from_secs(20);
    while !snapshot.lock().unwrap().ready && Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }
    let sample = snapshot.lock().unwrap().clone();
    if !sample.ready {
        return Err("No telemetry sample within 20 seconds".into());
    }
    let physical_gpu = sample.adapters.iter().find(|adapter| !adapter.software);
    if test_gpu && physical_gpu.is_none() {
        return Err("--gpu requested, but the platform reported no hardware adapter".into());
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
        mem_pattern: MemPattern::Alternating,
        disk_pattern: DiskPattern::Constant,
        gpu_intensity: GpuIntensity::Standard,
        ..Config::default()
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
    for kind in [
        Kind::Cpu,
        Kind::Memory,
        Kind::Disk,
        Kind::Gpu,
        Kind::Combined,
    ] {
        let adapter = (test_gpu && kind == Kind::Gpu)
            .then_some(physical_gpu)
            .flatten();
        let cfg = Config {
            kind,
            seconds: if adapter.is_some() { 3 } else { 1 },
            workers: 1,
            mib: 1,
            directory: root.clone(),
            gpu_adapter: adapter.map(|a| format!("{} · {}", a.name, a.backend)),
            disk_write_limit_mib: 1,
            mem_pattern: MemPattern::Alternating,
            disk_pattern: DiskPattern::Constant,
            gpu_intensity: if adapter.is_some() {
                GpuIntensity::Light
            } else {
                GpuIntensity::Standard
            },
            ..Config::default()
        };
        if let (Some(adapter), Some(executable)) = (adapter, worker_executable.as_ref()) {
            stress::start_gpu_in_worker(
                cfg,
                state.clone(),
                cancel.clone(),
                Some(adapter.handle.clone()),
                context.clone(),
                executable.clone(),
            )?;
        } else {
            stress::start(
                cfg,
                state.clone(),
                cancel.clone(),
                adapter.map(|a| a.handle.clone()),
                context.clone(),
            )?;
        }
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
        let expected = if kind == Kind::Gpu && adapter.is_none() {
            Status::Unavailable
        } else {
            Status::Completed
        };
        println!("{kind:?}: {} — {}", result.status, result.detail);
        if result.status != expected {
            return Err(format!("{kind:?}: expected {expected}, got {}", result.status).into());
        }
    }
    if let (Some(executable), Some(adapter)) = (worker_executable, physical_gpu) {
        stress::start_gpu_in_worker(
            Config {
                kind: Kind::Gpu,
                run_mode: stress::RunMode::Continuous,
                gpu_intensity: GpuIntensity::Light,
                ..Config::default()
            },
            state.clone(),
            cancel.clone(),
            Some(adapter.handle.clone()),
            context.clone(),
            executable,
        )?;
        let deadline = Instant::now() + Duration::from_secs(12);
        while state.lock().unwrap().live.units == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        let made_progress = state.lock().unwrap().live.units > 0;
        cancel.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.lock().unwrap().running && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        let guard = state.lock().unwrap();
        if !made_progress
            || guard.running
            || !guard
                .results
                .last()
                .is_some_and(|result| result.status == Status::Stopped)
        {
            return Err("Isolated GPU worker did not stop after verified work".into());
        }
        println!("Isolated GPU cancellation: Stopped after verified work");
    }
    if fs::read_dir(&root)?.next().is_some() {
        return Err("Scratch file was not removed".into());
    }
    let report_path = root.join("report.json");
    let records = state.lock().unwrap().results.clone();
    report::export(&report_path, Format::Json, &records)?;
    let bytes = fs::read(&report_path)?;
    let report_json: serde_json::Value = serde_json::from_slice(&bytes)?;
    if report_json["results"].as_array().map(Vec::len) != Some(records.len()) {
        return Err("report did not contain all results".into());
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
