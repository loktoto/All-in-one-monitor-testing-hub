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

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub enum Kind {
    Cpu,
    Gpu,
    Memory,
    Disk,
}
impl Kind {
    pub fn unit(self) -> &'static str {
        match self {
            Self::Cpu => "compute rounds",
            Self::Gpu => "compute dispatches",
            Self::Memory => "memory passes",
            Self::Disk => "file passes",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Config {
    pub kind: Kind,
    pub seconds: u64,
    pub workers: usize,
    pub mib: usize,
    pub directory: PathBuf,
    pub gpu_adapter: Option<String>,
    pub disk_write_limit_mib: u64,
}
// Export only fields that actually influence this test kind.
impl serde::Serialize for Config {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut out = serializer.serialize_struct("Config", 4)?;
        out.serialize_field("kind", &self.kind)?;
        out.serialize_field("duration_seconds", &self.seconds)?;
        match self.kind {
            Kind::Cpu => out.serialize_field("workers", &self.workers)?,
            Kind::Gpu => out.serialize_field("gpu_adapter", &self.gpu_adapter)?,
            Kind::Memory => out.serialize_field("memory_mib", &self.mib)?,
            Kind::Disk => {
                out.serialize_field("file_mib", &self.mib)?;
                out.serialize_field("directory", &self.directory)?;
                out.serialize_field("write_limit_mib", &self.disk_write_limit_mib)?;
            }
        }
        out.end()
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Status {
    Completed,
    Stopped,
    Failed,
    Inconclusive,
    Unavailable,
}
impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ReportContext {
    pub cpu: String,
    pub logical_cpus: usize,
    pub memory_total: u64,
    pub adapters: Vec<String>,
    pub os: String,
    pub snapshot_age_seconds: Option<f64>,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct ResultRecord {
    pub kind: Kind,
    pub status: Status,
    pub detail: String,
    pub verified_units: Option<u64>,
    pub verified_unit: &'static str,
    pub elapsed: f64,
    pub timestamp: u64,
    pub config: Config,
    pub context: ReportContext,
}
#[derive(Debug)]
struct Outcome {
    verified: u64,
    detail: String,
}
impl Outcome {
    fn new(verified: u64, detail: String) -> Self {
        Self { verified, detail }
    }
}
#[derive(Debug)]
enum TestError {
    Failed(String),
    Unavailable(String),
}
impl From<String> for TestError {
    fn from(s: String) -> Self {
        Self::Failed(s)
    }
}
impl From<&str> for TestError {
    fn from(s: &str) -> Self {
        Self::Failed(s.into())
    }
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
    adapter: Option<wgpu::Adapter>,
    context: ReportContext,
) -> Result<(), String> {
    let valid = (1..=600).contains(&config.seconds)
        && match config.kind {
            Kind::Cpu => (1..=8).contains(&config.workers),
            Kind::Gpu => true,
            Kind::Memory => (1..=256).contains(&config.mib),
            Kind::Disk => {
                (1..=256).contains(&config.mib)
                    && (1..=1024).contains(&config.disk_write_limit_mib)
                    && config.disk_write_limit_mib >= config.mib as u64
            }
        };
    if !valid {
        return Err("Limits: 1–600 seconds; CPU 1–8 workers; memory/file 1–256 MiB; disk write limit must be at least file size and at most 1024 MiB.".into());
    }
    {
        let mut s = state.lock().map_err(|_| "Test state unavailable")?;
        if s.running {
            return Err("A test is already running.".into());
        }
        cancel.store(false, Ordering::Relaxed);
        s.running = true;
        s.progress = "Starting…".into();
        s.started = Some(Instant::now());
    }
    thread::spawn(move || {
        let started = Instant::now();
        let deadline = started + Duration::from_secs(config.seconds);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match config.kind {
            Kind::Cpu => cpu(&config, &cancel, deadline),
            Kind::Gpu => gpu(adapter.as_ref(), &cancel, deadline),
            Kind::Memory => memory(&config, &cancel, deadline),
            Kind::Disk => disk(&config, &cancel, deadline),
        }))
        .unwrap_or_else(|_| {
            Err(TestError::Failed(
                "Test worker panicked; no result was verified".into(),
            ))
        });
        let (status, verified_units, detail) = classify(result, cancel.load(Ordering::Relaxed));
        let record = ResultRecord {
            kind: config.kind,
            status,
            detail,
            verified_units,
            verified_unit: config.kind.unit(),
            elapsed: started.elapsed().as_secs_f64(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            config,
            context,
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

fn classify(result: Result<Outcome, TestError>, cancelled: bool) -> (Status, Option<u64>, String) {
    match result {
        Ok(outcome) if cancelled => (Status::Stopped, Some(outcome.verified), outcome.detail),
        Ok(outcome) if outcome.verified == 0 => (Status::Inconclusive, Some(0), outcome.detail),
        Ok(outcome) => (Status::Completed, Some(outcome.verified), outcome.detail),
        Err(TestError::Failed(message)) => (Status::Failed, None, message),
        Err(TestError::Unavailable(message)) => (Status::Unavailable, Some(0), message),
    }
}

const CPU_EXPECTED: u64 = 0x42e1_bf14_e0f1_2920;
fn cpu_result_is_correct(actual: u64) -> bool {
    actual == CPU_EXPECTED
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
fn cpu(c: &Config, cancel: &AtomicBool, deadline: Instant) -> Result<Outcome, TestError> {
    let mut rounds = 0u64;
    let mut failures = 0u64;
    thread::scope(|scope| {
        let mut tasks = Vec::new();
        for _worker in 0..c.workers {
            tasks.push(scope.spawn(move || {
                // Expected value is fixed, not derived from the tested execution.
                let mut n = 0;
                let mut bad = 0;
                while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                    if !cpu_result_is_correct(calculation(0)) {
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
        )
        .into())
    } else {
        Ok(Outcome::new(
            rounds,
            format!(
                "{rounds} checked compute rounds across {} workers; no mismatches detected",
                c.workers
            ),
        ))
    }
}
fn memory(c: &Config, cancel: &AtomicBool, deadline: Instant) -> Result<Outcome, TestError> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let requested = c.mib as u64 * 1024 * 1024;
    let allowed = (sys.available_memory() / 4).min(256 * 1024 * 1024);
    if requested > allowed {
        return Err(format!(
            "Requested {} MiB exceeds one quarter of currently available memory ({} MiB).",
            c.mib,
            allowed / 1024 / 1024
        )
        .into());
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
                return Ok(Outcome::new(
                    passes,
                    format!("{passes} complete write/read passes over {} MiB", c.mib),
                ));
            }
            chunk.fill(pattern);
        }
        std::hint::black_box(&mut bytes);
        for chunk in bytes.chunks(1024 * 1024) {
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(Outcome::new(
                    passes,
                    format!("{passes} complete write/read passes over {} MiB", c.mib),
                ));
            }
            if chunk.iter().any(|&b| b != pattern) {
                return Err(format!("Memory pattern mismatch in pass {}", passes + 1).into());
            }
        }
        passes += 1;
    }
    Ok(Outcome::new(
        passes,
        format!(
            "{passes} complete write/read passes over {} MiB; no mismatches detected",
            c.mib
        ),
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
fn disk(c: &Config, cancel: &AtomicBool, deadline: Instant) -> Result<Outcome, TestError> {
    disk_with_cap(c, cancel, deadline, c.disk_write_limit_mib)
}
fn disk_with_cap(
    c: &Config,
    cancel: &AtomicBool,
    deadline: Instant,
    cap_mib: u64,
) -> Result<Outcome, TestError> {
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
                return Ok(Outcome::new(
                    passes,
                    format!("{passes} verified passes; {written} MiB written"),
                ));
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
                return Ok(Outcome::new(
                    passes,
                    format!("{passes} verified passes; {written} MiB written"),
                ));
            }
            file.read_exact(&mut read)
                .map_err(|e| format!("Read failed: {e}"))?;
            if read != block {
                return Err("File readback mismatch".into());
            }
        }
        passes += 1;
    }
    Ok(Outcome::new(
        passes,
        format!("{passes} verified passes; {written} MiB written"),
    ))
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
            gpu_adapter: None,
            disk_write_limit_mib: 1,
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
        assert!(result.detail.starts_with("1 verified passes"), "{result:?}");
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
        assert!(!result.detail.starts_with("0 "), "{result:?}");
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
        assert!(result.detail.contains("0 MiB written"));
    }
    #[test]
    fn status_reflects_verified_work_and_unavailable_devices() {
        assert_eq!(
            classify(Ok(Outcome::new(0, "nothing".into())), false).0,
            Status::Inconclusive
        );
        assert_eq!(
            classify(Ok(Outcome::new(1, "ok".into())), false).0,
            Status::Completed
        );
        assert_eq!(
            classify(Ok(Outcome::new(1, "ok".into())), true).0,
            Status::Stopped
        );
        assert_eq!(
            classify(Err(TestError::Unavailable("no device".into())), false).0,
            Status::Unavailable
        );
        assert_eq!(
            classify(Err(TestError::Failed("error".into())), true).0,
            Status::Failed
        );
        assert!(matches!(
            gpu(
                None,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(1)
            ),
            Err(TestError::Unavailable(_))
        ));
    }
    #[test]
    fn worker_cancellation_and_restart_clear_running_state() {
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut config = test_config(Kind::Cpu);
        config.seconds = 1;
        start(
            config.clone(),
            state.clone(),
            stop.clone(),
            None,
            ReportContext::default(),
        )
        .unwrap();
        assert!(start(
            config.clone(),
            state.clone(),
            stop.clone(),
            None,
            ReportContext::default()
        )
        .is_err());
        stop.store(true, Ordering::Relaxed);
        let until = Instant::now() + Duration::from_secs(15);
        while state.lock().unwrap().running && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!state.lock().unwrap().running);
        assert_eq!(state.lock().unwrap().results[0].status, Status::Stopped);
        start(
            config,
            state.clone(),
            stop.clone(),
            None,
            ReportContext::default(),
        )
        .unwrap();
        let until = Instant::now() + Duration::from_secs(15);
        while state.lock().unwrap().running && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        let state = state.lock().unwrap();
        assert!(!state.running);
        assert_eq!(state.results.len(), 2);
        assert_eq!(state.results[1].status, Status::Completed);
        assert!(state.results[1].verified_units.unwrap_or(0) > 0);
    }
    #[test]
    fn disk_invalid_directory_is_an_error() {
        let mut c = test_config(Kind::Disk);
        c.directory = std::env::temp_dir().join(format!(
            "hardware-hub-missing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(matches!(
            disk(
                &c,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(1)
            ),
            Err(TestError::Failed(_))
        ));
    }
    #[test]
    fn deterministic_cpu() {
        // Independent geometric-series exponentiation: sum a^i in Z/(2^64).
        let (mut base_pow, mut base_sum) = (6364136223846793005u64, 1u64);
        let (mut accum_pow, mut accum_sum) = (1u64, 0u64);
        let mut count = 100_000u64;
        while count > 0 {
            if count & 1 != 0 {
                accum_sum = accum_sum.wrapping_add(accum_pow.wrapping_mul(base_sum));
                accum_pow = accum_pow.wrapping_mul(base_pow);
            }
            base_sum = base_sum.wrapping_mul(1u64.wrapping_add(base_pow));
            base_pow = base_pow.wrapping_mul(base_pow);
            count >>= 1;
        }
        let independent = 1442695040888963407u64.wrapping_mul(accum_sum);
        assert_eq!(independent, CPU_EXPECTED);
        assert!(cpu_result_is_correct(calculation(0)));
        assert!(!cpu_result_is_correct(calculation(0) ^ 1));
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
            gpu_adapter: None,
            disk_write_limit_mib: 1,
        };
        assert!(start(
            c,
            s.clone(),
            Arc::new(AtomicBool::new(false)),
            None,
            ReportContext::default()
        )
        .is_err());
        assert!(!s.lock().unwrap().running);
    }
}

fn gpu(
    adapter: Option<&wgpu::Adapter>,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Outcome, TestError> {
    use wgpu::util::DeviceExt;
    let adapter = adapter.ok_or_else(|| {
        TestError::Unavailable("No GPU adapter was selected or discovered".into())
    })?;
    let info = adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu {
        return Err(TestError::Unavailable(
            "Only a software adapter was selected; GPU test unavailable".into(),
        ));
    }
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .map_err(|e| TestError::Unavailable(format!("GPU device request failed: {e}")))?;
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
        return Err(format!("GPU setup failed: {e}").into());
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
                return Err(format!("GPU error: {e}").into());
            }
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Ok(Outcome::new(
                    rounds,
                    format!(
                    "{rounds} verified compute dispatches on {}; pending dispatch was not verified",
                    info.name
                ),
                ));
            }
            if dispatch_started.elapsed() >= Duration::from_secs(15) {
                return Err(
                    format!("GPU readback timed out after {rounds} verified dispatches").into(),
                );
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
                    )
                    .into());
                }
            }
        }
        staging.unmap();
        if let Some(e) = errors
            .lock()
            .map_err(|_| "GPU error state unavailable")?
            .take()
        {
            return Err(format!("GPU error: {e}").into());
        }
        rounds += 1;
    }
    Ok(Outcome::new(
        rounds,
        format!(
            "{rounds} verified compute dispatches on {} ({:?}); no mismatches detected",
            info.name, info.backend
        ),
    ))
}
