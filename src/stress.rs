use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Cpu,
    Gpu,
    Memory,
    Disk,
}
#[derive(Clone, Debug)]
pub struct Config {
    pub kind: Kind,
    pub seconds: u64,
    pub workers: usize,
    pub mib: usize,
    pub directory: PathBuf,
}
#[derive(Clone, Debug)]
pub struct ResultRecord {
    pub kind: Kind,
    pub status: String,
    pub detail: String,
    pub elapsed: f64,
    pub timestamp: u64,
}
#[derive(Default)]
pub struct State {
    pub running: bool,
    pub progress: String,
    pub started: Option<Instant>,
    pub results: Vec<ResultRecord>,
}

pub fn start(
    config: Config,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    if !(1..=600).contains(&config.seconds)
        || !(1..=8).contains(&config.workers)
        || !(1..=256).contains(&config.mib)
    {
        return Err("Limits: 1–600 seconds, 1–8 workers, 1–256 MiB.".into());
    }
    {
        let mut s = state.lock().map_err(|_| "Test state unavailable")?;
        if s.running {
            return Err("A test is already running.".into());
        }
        s.running = true;
        s.progress = "Starting…".into();
        s.started = Some(Instant::now());
    }
    cancel.store(false, Ordering::Relaxed);
    thread::spawn(move || {
        let started = Instant::now();
        let deadline = started + Duration::from_secs(config.seconds);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match config.kind {
            Kind::Cpu => cpu(&config, &cancel, deadline),
            Kind::Gpu => gpu(&cancel, deadline),
            Kind::Memory => memory(&config, &cancel, deadline),
            Kind::Disk => disk(&config, &cancel, deadline),
        }))
        .unwrap_or_else(|_| Err("Test worker panicked; no result was verified".into()));
        let (status, detail) = match result {
            Ok(detail) if cancel.load(Ordering::Relaxed) => ("Stopped".into(), detail),
            Ok(detail) if detail.starts_with("0 ") => ("Inconclusive".into(), detail),
            Ok(detail) => ("Completed".into(), detail),
            Err(e) => ("Failed".into(), e),
        };
        let record = ResultRecord {
            kind: config.kind,
            status,
            detail,
            elapsed: started.elapsed().as_secs_f64(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        if let Ok(mut s) = state.lock() {
            s.running = false;
            s.progress = "Idle".into();
            s.started = None;
            s.results.push(record);
        }
    });
    Ok(())
}

fn calculation(seed: u64) -> u64 {
    let mut x = seed;
    for _ in 0..100_000 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        x = std::hint::black_box(x);
    }
    x
}
fn cpu(c: &Config, cancel: &AtomicBool, deadline: Instant) -> Result<String, String> {
    let mut rounds = 0u64;
    let mut failures = 0u64;
    thread::scope(|scope| {
        let mut tasks = Vec::new();
        for worker in 0..c.workers {
            tasks.push(scope.spawn(move || {
                let expected = calculation(worker as u64);
                let mut n = 0;
                let mut bad = 0;
                while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                    if calculation(worker as u64) != expected {
                        bad += 1;
                    }
                    n += 1;
                }
                (n, bad)
            }));
        }
        for task in tasks {
            match task.join() {
                Ok((n, bad)) => {
                    rounds += n;
                    failures += bad;
                }
                Err(_) => failures += 1,
            }
        }
    });
    if failures > 0 {
        Err(format!(
            "{failures} computation mismatches or worker failures after {rounds} checked rounds"
        ))
    } else {
        Ok(format!(
            "{rounds} checked compute rounds across {} workers; no mismatches detected",
            c.workers
        ))
    }
}
fn memory(c: &Config, cancel: &AtomicBool, deadline: Instant) -> Result<String, String> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let requested = c.mib as u64 * 1024 * 1024;
    let allowed = (sys.available_memory() / 4).min(256 * 1024 * 1024);
    if requested > allowed {
        return Err(format!(
            "Requested {} MiB exceeds one quarter of currently available memory ({} MiB).",
            c.mib,
            allowed / 1024 / 1024
        ));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(requested as usize)
        .map_err(|e| format!("Allocation failed: {e}"))?;
    bytes.resize(requested as usize, 0u8);
    let mut passes = 0u64;
    while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
        let pattern = if passes % 2 == 0 { 0x55 } else { 0xaa };
        for chunk in bytes.chunks_mut(1024 * 1024) {
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(format!(
                    "{passes} complete write/read passes over {} MiB",
                    c.mib
                ));
            }
            chunk.fill(pattern);
        }
        std::hint::black_box(&mut bytes);
        for chunk in bytes.chunks(1024 * 1024) {
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(format!(
                    "{passes} complete write/read passes over {} MiB",
                    c.mib
                ));
            }
            if chunk.iter().any(|&b| b != pattern) {
                return Err(format!("Memory pattern mismatch in pass {}", passes + 1));
            }
        }
        passes += 1;
    }
    Ok(format!(
        "{passes} complete write/read passes over {} MiB; no mismatches detected",
        c.mib
    ))
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn scratch(dir: &Path) -> Result<(Scratch, fs::File), String> {
    for attempt in 0..100 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!(
            "hardware-hub-{}-{nonce}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((Scratch(path), file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot create test file: {e}")),
        }
    }
    Err("Could not create a unique test file".into())
}
fn disk(c: &Config, cancel: &AtomicBool, deadline: Instant) -> Result<String, String> {
    disk_with_cap(c, cancel, deadline, 1024)
}
fn disk_with_cap(
    c: &Config,
    cancel: &AtomicBool,
    deadline: Instant,
    cap_mib: u64,
) -> Result<String, String> {
    let (_scratch, mut file) = scratch(&c.directory)?;
    let block = vec![0xa5u8; 1024 * 1024];
    let mut passes = 0u64;
    let mut written = 0u64;
    while Instant::now() < deadline
        && !cancel.load(Ordering::Relaxed)
        && written + c.mib as u64 <= cap_mib
    {
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        for _ in 0..c.mib {
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(format!("{passes} verified passes; {written} MiB written"));
            }
            file.write_all(&block)
                .map_err(|e| format!("Write failed: {e}"))?;
            written += 1;
        }
        file.sync_all().map_err(|e| format!("Flush failed: {e}"))?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut read = vec![0; block.len()];
        for _ in 0..c.mib {
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(format!("{passes} verified passes; {written} MiB written"));
            }
            file.read_exact(&mut read)
                .map_err(|e| format!("Read failed: {e}"))?;
            if read != block {
                return Err("File readback mismatch".into());
            }
        }
        passes += 1;
    }
    Ok(format!("{passes} verified passes; {written} MiB written"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_config(kind: Kind) -> Config {
        Config {
            kind,
            seconds: 2,
            workers: 1,
            mib: 1,
            directory: std::env::temp_dir(),
        }
    }
    #[test]
    fn disk_writes_verifies_and_cleans_up_without_touching_existing_files() {
        let dir = std::env::temp_dir().join(format!(
            "hardware-hub-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let sentinel = dir.join("keep.txt");
        fs::write(&sentinel, b"keep").unwrap();
        let mut config = test_config(Kind::Disk);
        config.directory = dir.clone();
        let result = disk_with_cap(
            &config,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10),
            1,
        )
        .unwrap();
        assert!(result.starts_with("1 verified passes"), "{result}");
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn memory_completes_a_pattern_pass() {
        let result = memory(
            &test_config(Kind::Memory),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_millis(200),
        )
        .unwrap();
        assert!(!result.starts_with("0 "), "{result}");
    }
    #[test]
    fn cancelled_disk_test_does_not_write() {
        let result = disk_with_cap(
            &test_config(Kind::Disk),
            &AtomicBool::new(true),
            Instant::now() + Duration::from_secs(1),
            1,
        )
        .unwrap();
        assert!(result.contains("0 MiB written"));
    }
    #[test]
    fn deterministic_cpu() {
        assert_eq!(calculation(42), calculation(42));
        assert_ne!(calculation(42), calculation(43));
    }
    #[test]
    fn disk_owns_and_removes_file() {
        let d = std::env::temp_dir();
        let (scratch, mut file) = scratch(&d).unwrap();
        let p = scratch.0.clone();
        file.write_all(b"test").unwrap();
        assert!(p.exists());
        drop(file);
        drop(scratch);
        assert!(!p.exists());
    }
    #[test]
    fn rejects_out_of_bounds_before_launch() {
        let s = Arc::new(Mutex::new(State::default()));
        let c = Config {
            kind: Kind::Disk,
            seconds: 1,
            workers: 1,
            mib: 257,
            directory: std::env::temp_dir(),
        };
        assert!(start(c, s.clone(), Arc::new(AtomicBool::new(false))).is_err());
        assert!(!s.lock().unwrap().running);
    }
}

fn gpu(cancel: &AtomicBool, deadline: Instant) -> Result<String, String> {
    use wgpu::util::DeviceExt;
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok_or("No compatible GPU compute adapter found")?;
    let info = adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu {
        return Err("Only a software adapter was found; GPU test unavailable".into());
    }
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .map_err(|e| format!("GPU device request failed: {e}"))?;
    let errors = Arc::new(Mutex::new(None::<String>));
    let error_out = errors.clone();
    device.on_uncaptured_error(Box::new(move |e| {
        if let Ok(mut value) = error_out.lock() {
            *value = Some(e.to_string());
        }
    }));
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    const WORDS: usize = 4096;
    const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read_write> output: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var x = id.x;
    for (var i = 0u; i < 256u; i = i + 1u) { x = x * 1664525u + 1013904223u; }
    output[id.x] = x;
}
"#;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("verified compute"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("verified compute"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bytes = vec![0u8; WORDS * 4];
    let output = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("compute output"),
        contents: &bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });
    if let Some(e) = pollster::block_on(device.pop_error_scope()) {
        return Err(format!("GPU setup failed: {e}"));
    }
    let mut expected = Vec::with_capacity(WORDS);
    for i in 0..WORDS {
        let mut x = i as u32;
        for _ in 0..256 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        }
        expected.push(x);
    }
    let mut rounds = 0u64;
    while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((WORDS / 64) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, bytes.len() as u64);
        queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let dispatch_started = Instant::now();
        loop {
            device.poll(wgpu::Maintain::Poll);
            match rx.try_recv() {
                Ok(result) => {
                    result.map_err(|e| format!("GPU readback failed: {e}"))?;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("GPU readback callback disconnected".into())
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            if let Some(e) = errors
                .lock()
                .map_err(|_| "GPU error state unavailable")?
                .take()
            {
                return Err(format!("GPU error: {e}"));
            }
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(format!(
                    "{rounds} verified compute dispatches on {}; pending dispatch was not verified",
                    info.name
                ));
            }
            if dispatch_started.elapsed() >= Duration::from_secs(15) {
                return Err(format!(
                    "GPU readback timed out after {rounds} verified dispatches"
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        {
            let data = slice.get_mapped_range();
            for (index, chunk) in data.chunks_exact(4).enumerate() {
                let value = u32::from_le_bytes(chunk.try_into().unwrap());
                if value != expected[index] {
                    return Err(format!(
                        "GPU result mismatch at word {index} after {rounds} verified dispatches"
                    ));
                }
            }
        }
        staging.unmap();
        if let Some(e) = errors
            .lock()
            .map_err(|_| "GPU error state unavailable")?
            .take()
        {
            return Err(format!("GPU error: {e}"));
        }
        rounds += 1;
    }
    if rounds == 0 && !cancel.load(Ordering::Relaxed) {
        return Err(
            "No GPU dispatch completed before the time limit; increase the duration".into(),
        );
    }
    Ok(format!(
        "{rounds} verified compute dispatches on {} ({:?}); no mismatches detected",
        info.name, info.backend
    ))
}
