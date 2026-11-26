use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod gpu_worker;
pub use gpu_worker::{run_gpu_worker, GPU_WORKER_FLAG};

/// Maximum duration for timed runs (24 hours). Continuous mode has no timer cutoff.
pub const MAX_TIMED_SECONDS: u64 = 86_400;
/// Maximum supported CPU worker threads.
pub const MAX_WORKERS: usize = 256;
/// Hard ceiling for memory stress working set (16 GiB); further guarded by OS reserve.
pub const MAX_MEMORY_MIB: usize = 16_384;
/// Minimum memory left untouched for the OS so stress checks stay responsive.
pub const OS_MEMORY_RESERVE_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum scratch file size for disk verification (1 GiB).
pub const MAX_DISK_FILE_MIB: usize = 1_024;
/// Maximum cumulative disk writes per run (64 GiB).
pub const MAX_DISK_WRITE_CAP_MIB: u64 = 65_536;
const CPU_ROUNDS_PER_PASS: u64 = 256;
const GPU_DISPATCHES_PER_PASS: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Kind {
    Cpu,
    Gpu,
    Memory,
    Disk,
    /// CPU and memory checks running at the same time, in the style of a
    /// combined system stability load.
    Combined,
}
impl Kind {
    pub fn unit(self) -> &'static str {
        match self {
            Self::Cpu => "compute rounds",
            Self::Gpu => "compute dispatches",
            Self::Memory => "memory passes",
            Self::Disk => "file passes",
            Self::Combined => "work units",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU compute",
            Self::Gpu => "GPU check",
            Self::Memory => "Memory patterns",
            Self::Disk => "File I/O",
            Self::Combined => "Combined CPU + memory",
        }
    }
}

/// How a stress run decides when to finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum RunMode {
    /// Stop after `Config::seconds` elapses (up to 24 hours).
    #[default]
    Timed,
    /// Run continuously until the user clicks Stop, presses Esc, or thermal auto-stop fires.
    Continuous,
    /// Run until `Config::target_passes` verified passes/cycles complete.
    Passes,
}

impl RunMode {
    pub const ALL: [Self; 3] = [Self::Timed, Self::Continuous, Self::Passes];
    pub fn label(self) -> &'static str {
        match self {
            Self::Timed => "Timed",
            Self::Continuous => "Continuous",
            Self::Passes => "Pass count",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timed => "timed",
            Self::Continuous => "continuous",
            Self::Passes => "passes",
        }
    }
    pub fn from_str_lossy(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "continuous" | "infinite" | "until_stopped" => Self::Continuous,
            "passes" | "pass" => Self::Passes,
            _ => Self::Timed,
        }
    }
}

/// Selectable CPU stability workloads inspired by Prime95 / OCCT.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize)]
pub enum CpuWorkload {
    /// Deterministic 64-bit integer LCG verified against a fixed known answer.
    #[default]
    IntegerVerify,
    /// Heavy deterministic FP64 multiply-add loop verifying floating-point units.
    FloatFma,
    /// Blended integer + L2/L3 cache stride verification.
    Blend,
}

impl CpuWorkload {
    pub const ALL: [Self; 3] = [Self::IntegerVerify, Self::FloatFma, Self::Blend];
    pub fn label(self) -> &'static str {
        match self {
            Self::IntegerVerify => "Integer verify (ALU)",
            Self::FloatFma => "Floating-point FMA",
            Self::Blend => "Blend (CPU + cache)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum MemPattern {
    /// 0x55 / 0xAA alternating, one pattern per 1 MiB block.
    Alternating,
    /// 0x0F / 0xF0 checkerboard, one pattern per 1 MiB block.
    Checkerboard,
    /// A single zero bit walks across each byte. Catches adjacent-bit faults.
    WalkingZero,
    /// Deterministic per-byte hash. Exercises address decoding and data lines.
    PseudoRandom,
}
impl MemPattern {
    pub const ALL: [Self; 4] = [
        Self::Alternating,
        Self::Checkerboard,
        Self::WalkingZero,
        Self::PseudoRandom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Alternating => "Alternating 55/AA",
            Self::Checkerboard => "Checkerboard 0F/F0",
            Self::WalkingZero => "Walking zero bit",
            Self::PseudoRandom => "Pseudo-random bytes",
        }
    }
    /// Byte written at a given absolute offset. Kept deterministic so the
    /// verifier can regenerate it without storing a full copy of the buffer.
    pub fn byte(self, index: usize) -> u8 {
        match self {
            Self::Alternating => {
                if (index / (1024 * 1024)) % 2 == 0 {
                    0x55
                } else {
                    0xAA
                }
            }
            Self::Checkerboard => {
                if (index / (1024 * 1024)) % 2 == 0 {
                    0x0F
                } else {
                    0xF0
                }
            }
            Self::WalkingZero => !(1u8 << (index % 8)),
            Self::PseudoRandom => {
                let mut x = (index as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
                x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
                (x >> 56) as u8
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum DiskPattern {
    /// Single repeated byte, the historical default.
    Constant,
    /// Alternating bytes, closer to a worst-case write pattern.
    Alternating,
    /// Deterministic per-byte hash.
    PseudoRandom,
}
impl DiskPattern {
    pub const ALL: [Self; 3] = [Self::Constant, Self::Alternating, Self::PseudoRandom];
    pub fn label(self) -> &'static str {
        match self {
            Self::Constant => "Constant A5",
            Self::Alternating => "Alternating 55/AA",
            Self::PseudoRandom => "Pseudo-random bytes",
        }
    }
    pub fn byte(self, index: usize) -> u8 {
        match self {
            Self::Constant => 0xA5,
            Self::Alternating => {
                if (index / 4096) % 2 == 0 {
                    0x55
                } else {
                    0xAA
                }
            }
            Self::PseudoRandom => MemPattern::PseudoRandom.byte(index),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GpuIntensity {
    /// 1024 words, 64 LCG steps — a quick sanity check.
    Light,
    /// 4096 words, 256 steps — the historical default.
    Standard,
    /// 8192 words, 1024 steps — a heavier shader load.
    Heavy,
    /// 16384 words, 2048 steps — sustained compute stress.
    Extreme,
}
impl GpuIntensity {
    pub const ALL: [Self; 4] = [Self::Light, Self::Standard, Self::Heavy, Self::Extreme];
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light · 1K × 64",
            Self::Standard => "Standard · 4K × 256",
            Self::Heavy => "Heavy · 8K × 1024",
            Self::Extreme => "Extreme · 16K × 2048",
        }
    }
    pub fn params(self) -> (usize, u32) {
        match self {
            Self::Light => (1024, 64),
            Self::Standard => (4096, 256),
            Self::Heavy => (8192, 1024),
            Self::Extreme => (16384, 2048),
        }
    }
}

/// Safe maximum memory (in MiB) that can be allocated for a memory check
/// without starving the OS.
pub fn safe_max_memory_mib(available_bytes: u64) -> usize {
    let usable = available_bytes.saturating_sub(OS_MEMORY_RESERVE_BYTES);
    let seventy_five_pct = available_bytes.saturating_mul(3) / 4;
    let cap = usable.min(seventy_five_pct) / (1024 * 1024);
    (cap as usize).clamp(1, MAX_MEMORY_MIB)
}

/// Format a duration in seconds into a readable label (e.g., `15s`, `30m`, `4h`, `1h 30m`).
pub fn format_duration(seconds: u64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 && m > 0 {
        format!("{h}h {m}m")
    } else if h > 0 {
        format!("{h}h")
    } else if m > 0 && s > 0 {
        format!("{m}m {s}s")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub kind: Kind,
    pub run_mode: RunMode,
    pub seconds: u64,
    pub target_passes: u64,
    pub workers: usize,
    pub cpu_workload: CpuWorkload,
    pub mib: usize,
    pub directory: PathBuf,
    pub gpu_adapter: Option<String>,
    pub disk_write_limit_mib: u64,
    pub mem_pattern: MemPattern,
    pub disk_pattern: DiskPattern,
    pub gpu_intensity: GpuIntensity,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kind: Kind::Cpu,
            run_mode: RunMode::Timed,
            seconds: 1800,
            target_passes: 4,
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            cpu_workload: CpuWorkload::IntegerVerify,
            mib: 256,
            directory: std::env::temp_dir(),
            gpu_adapter: None,
            disk_write_limit_mib: 256,
            mem_pattern: MemPattern::Alternating,
            disk_pattern: DiskPattern::Constant,
            gpu_intensity: GpuIntensity::Standard,
        }
    }
}

impl Config {
    pub fn pass_target_units(&self) -> Option<u64> {
        (self.run_mode == RunMode::Passes).then(|| {
            let cpu = CPU_ROUNDS_PER_PASS.saturating_mul(self.workers as u64);
            let per_pass = match self.kind {
                Kind::Cpu => cpu,
                Kind::Gpu => GPU_DISPATCHES_PER_PASS,
                Kind::Memory | Kind::Disk => 1,
                Kind::Combined => cpu.saturating_add(1),
            };
            self.target_passes.saturating_mul(per_pass)
        })
    }
}

// Export only fields that actually influence this test kind.
impl serde::Serialize for Config {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut out = serializer.serialize_struct("Config", 6)?;
        out.serialize_field("kind", &self.kind)?;
        out.serialize_field("run_mode", &self.run_mode)?;
        match self.run_mode {
            RunMode::Timed => out.serialize_field("duration_seconds", &self.seconds)?,
            RunMode::Continuous => {}
            RunMode::Passes => out.serialize_field("target_passes", &self.target_passes)?,
        }
        match self.kind {
            Kind::Cpu => {
                out.serialize_field("workers", &self.workers)?;
                out.serialize_field("cpu_workload", &self.cpu_workload)?;
            }
            Kind::Gpu => {
                out.serialize_field("gpu_adapter", &self.gpu_adapter)?;
                out.serialize_field("intensity", &self.gpu_intensity)?;
            }
            Kind::Memory => {
                out.serialize_field("memory_mib", &self.mib)?;
                out.serialize_field("pattern", &self.mem_pattern)?;
            }
            Kind::Disk => {
                out.serialize_field("file_mib", &self.mib)?;
                out.serialize_field("directory", &self.directory)?;
                out.serialize_field("write_limit_mib", &self.disk_write_limit_mib)?;
                out.serialize_field("pattern", &self.disk_pattern)?;
            }
            Kind::Combined => {
                out.serialize_field("workers", &self.workers)?;
                out.serialize_field("cpu_workload", &self.cpu_workload)?;
                out.serialize_field("memory_mib", &self.mib)?;
                out.serialize_field("pattern", &self.mem_pattern)?;
            }
        }
        out.end()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
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
/// One recorded mismatch, in the style of memory-diagnostic tools: where it
/// happened, what was expected and what was read back. Bounded per run.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErrorDetail {
    pub location: String,
    pub expected: String,
    pub actual: String,
}
impl ErrorDetail {
    fn new(
        location: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            location: location.into(),
            expected: expected.into(),
            actual: actual.into(),
        }
    }
}

/// Work-in-progress counters published by the worker thread for the live UI.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub units: u64,
    pub unit_label: &'static str,
    pub errors: u64,
    pub phase: String,
    /// Per-worker health status (true = healthy, false = mismatch observed).
    pub worker_ok: Vec<bool>,
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
    /// Total mismatches detected (may exceed the number stored in `errors`).
    pub error_count: u64,
    /// First few mismatches, kept for diagnosis.
    pub errors: Vec<ErrorDetail>,
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Outcome {
    verified: u64,
    detail: String,
    errors: Vec<ErrorDetail>,
    error_count: u64,
}
impl Outcome {
    fn new(verified: u64, detail: String) -> Self {
        Self {
            verified,
            detail,
            errors: Vec::new(),
            error_count: 0,
        }
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
    pub active_config: Option<Config>,
    pub results: Vec<ResultRecord>,
    /// Live work counters, updated by the worker and read by the UI each frame.
    pub live: Progress,
    stop_reason: Option<String>,
}

impl State {
    /// Associate the reason with the active run before signalling cancellation.
    pub fn request_thermal_stop(&mut self, cancel: &AtomicBool, reason: String) {
        if self.running && !cancel.load(Ordering::Relaxed) {
            self.stop_reason = Some(reason);
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

const MAX_STORED_ERRORS: usize = 8;

/// Record a mismatch, keeping only the first few details but counting all.
fn note_error(errors: &mut Vec<ErrorDetail>, count: &mut u64, detail: ErrorDetail) {
    *count += 1;
    if errors.len() < MAX_STORED_ERRORS {
        errors.push(detail);
    }
}

/// Publish a progress snapshot from the worker at a throttled cadence.
fn publish(live: &Mutex<State>, units: u64, errors: u64, phase: &str) {
    if let Ok(mut s) = live.lock() {
        s.live.units = units;
        s.live.errors = errors;
        s.live.phase = phase.to_owned();
    }
}

fn publish_workers(
    live: &Mutex<State>,
    units: u64,
    errors: u64,
    phase: &str,
    worker_ok: Vec<bool>,
) {
    if let Ok(mut s) = live.lock() {
        s.live.units = units;
        s.live.errors = errors;
        s.live.phase = phase.to_owned();
        s.live.worker_ok = worker_ok;
    }
}

fn reached_deadline(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| Instant::now() >= d)
}

pub fn start(
    config: Config,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    adapter: Option<wgpu::Adapter>,
    context: ReportContext,
) -> Result<(), String> {
    start_with(
        config,
        state,
        cancel,
        context,
        move |config, cancel, deadline, state| {
            gpu(config, adapter.as_ref(), cancel, deadline, state)
        },
    )
}

pub fn start_gpu_in_worker(
    config: Config,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    adapter: Option<wgpu::Adapter>,
    context: ReportContext,
    executable: PathBuf,
) -> Result<(), String> {
    if config.kind != Kind::Gpu {
        return Err("The GPU worker can only run a GPU check.".into());
    }
    let adapter = adapter.as_ref().map(gpu_worker::AdapterKey::from);
    start_with(
        config,
        state,
        cancel,
        context,
        move |config, cancel, deadline, state| {
            gpu_worker::supervise(&executable, config, adapter, cancel, deadline, state)
        },
    )
}

fn start_with(
    config: Config,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    context: ReportContext,
    run_gpu: impl FnOnce(&Config, &AtomicBool, Option<Instant>, &Mutex<State>) -> Result<Outcome, TestError>
        + Send
        + 'static,
) -> Result<(), String> {
    let mode_valid = match config.run_mode {
        RunMode::Timed => (1..=MAX_TIMED_SECONDS).contains(&config.seconds),
        RunMode::Continuous => true,
        RunMode::Passes => (1..=10_000).contains(&config.target_passes),
    };
    let kind_valid = match config.kind {
        Kind::Cpu => (1..=MAX_WORKERS).contains(&config.workers),
        Kind::Gpu => true,
        Kind::Memory => (1..=MAX_MEMORY_MIB).contains(&config.mib),
        Kind::Disk => {
            (1..=MAX_DISK_FILE_MIB).contains(&config.mib)
                && (1..=MAX_DISK_WRITE_CAP_MIB).contains(&config.disk_write_limit_mib)
                && config.disk_write_limit_mib >= config.mib as u64
        }
        Kind::Combined => {
            (1..=MAX_WORKERS).contains(&config.workers)
                && (1..=MAX_MEMORY_MIB).contains(&config.mib)
        }
    };
    if !(mode_valid && kind_valid) {
        return Err(format!(
            "Limits: timed 1–{MAX_TIMED_SECONDS}s (24h) or Continuous/Passes; CPU 1–{MAX_WORKERS} workers; memory 1–{MAX_MEMORY_MIB} MiB; disk file 1–{MAX_DISK_FILE_MIB} MiB (cap >= file size, max {MAX_DISK_WRITE_CAP_MIB} MiB)."
        ));
    }
    if config.kind == Kind::Disk
        && config.run_mode == RunMode::Passes
        && config.target_passes * config.mib as u64 > config.disk_write_limit_mib
    {
        return Err(format!(
            "{} passes of {} MiB require a write cap of at least {} MiB.",
            config.target_passes,
            config.mib,
            config.target_passes * config.mib as u64,
        ));
    }
    {
        let mut s = state.lock().map_err(|_| "Test state unavailable")?;
        if s.running {
            return Err("A test is already running.".into());
        }
        cancel.store(false, Ordering::Relaxed);
        s.running = true;
        s.stop_reason = None;
        s.progress = "Starting…".into();
        s.started = Some(Instant::now());
        s.active_config = Some(config.clone());
        let worker_count = match config.kind {
            Kind::Cpu | Kind::Combined => config.workers,
            _ => 1,
        };
        s.live = Progress {
            units: 0,
            unit_label: config.kind.unit(),
            errors: 0,
            phase: "Preparing…".into(),
            worker_ok: vec![true; worker_count],
        };
    }
    thread::spawn(move || {
        let started = Instant::now();
        let deadline = match config.run_mode {
            RunMode::Timed => Some(started + Duration::from_secs(config.seconds)),
            RunMode::Continuous | RunMode::Passes => None,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match config.kind {
            Kind::Cpu => cpu(&config, &cancel, deadline, &state),
            Kind::Gpu => run_gpu(&config, &cancel, deadline, &state),
            Kind::Memory => memory(&config, &cancel, deadline, &state),
            Kind::Disk => disk(&config, &cancel, deadline, &state),
            Kind::Combined => combined(&config, &cancel, deadline, &state),
        }))
        .unwrap_or_else(|_| {
            Err(TestError::Failed(
                "Test worker panicked; no result was verified".into(),
            ))
        });
        let (status, outcome) = classify(result, cancel.load(Ordering::Relaxed));
        let mut record = ResultRecord {
            kind: config.kind,
            status,
            detail: outcome.detail,
            verified_units: Some(outcome.verified),
            verified_unit: config.kind.unit(),
            elapsed: started.elapsed().as_secs_f64(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            config,
            context,
            error_count: outcome.error_count,
            errors: outcome.errors,
        };
        if let Ok(mut s) = state.lock() {
            if let Some(reason) = s.stop_reason.take() {
                if record.status == Status::Stopped {
                    record.detail = format!("{reason} {}", record.detail);
                }
            }
            s.running = false;
            s.progress = "Idle".into();
            s.started = None;
            s.active_config = None;
            s.results.push(record);
        }
    });
    Ok(())
}

fn classify(result: Result<Outcome, TestError>, cancelled: bool) -> (Status, Outcome) {
    fn empty(detail: String) -> Outcome {
        Outcome::new(0, detail)
    }
    match result {
        Ok(outcome) if outcome.error_count > 0 => (Status::Failed, outcome),
        Ok(outcome) if cancelled => (Status::Stopped, outcome),
        Ok(outcome) if outcome.verified == 0 => (Status::Inconclusive, outcome),
        Ok(outcome) => (Status::Completed, outcome),
        Err(TestError::Failed(message)) => (Status::Failed, empty(message)),
        Err(TestError::Unavailable(message)) => (Status::Unavailable, empty(message)),
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

/// Deterministic FP64 multiply-add workload verified against a golden reference
/// computed once on a clean path (exercises FPU/SIMD pipelines).
fn fma_calculation() -> u64 {
    let mut a = 1.000_001_f64;
    let mut b = 0.999_997_f64;
    let mut c = 0.5_f64;
    for i in 0..50_000u32 {
        let k = ((i & 255) as f64) * 0.000_125 + 0.5;
        a = a.mul_add(0.999_999, k * 1e-6);
        b = b.mul_add(1.000_001, -k * 1e-6);
        c = (a + b) * 0.5;
        c = std::hint::black_box(c);
    }
    c.to_bits()
}

/// Blended CPU + L2/L3 cache stripe verification (64 KiB working buffer per worker).
fn blend_calculation(buf: &mut [u64]) -> bool {
    let mut acc = 0x9e37_79b9_7f4a_7c15u64;
    for (i, slot) in buf.iter_mut().enumerate() {
        acc = acc
            .wrapping_mul(6364136223846793005)
            .wrapping_add((i as u64).wrapping_mul(1442695040888963407));
        *slot = acc;
    }
    std::hint::black_box(&mut *buf);
    let mut verify = 0x9e37_79b9_7f4a_7c15u64;
    for (i, slot) in buf.iter().enumerate() {
        verify = verify
            .wrapping_mul(6364136223846793005)
            .wrapping_add((i as u64).wrapping_mul(1442695040888963407));
        if *slot != verify {
            return false;
        }
    }
    cpu_result_is_correct(calculation(0))
}

fn cpu(
    c: &Config,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    live: &Mutex<State>,
) -> Result<Outcome, TestError> {
    let mut rounds = 0u64;
    let mut failures = 0u64;
    let next_publish = Mutex::new(Instant::now());
    let published_rounds = AtomicU64::new(0);
    let published_errors = AtomicU64::new(0);
    let worker_health: Vec<AtomicBool> = (0..c.workers).map(|_| AtomicBool::new(true)).collect();
    let fma_expected = if c.cpu_workload == CpuWorkload::FloatFma {
        fma_calculation()
    } else {
        0
    };
    // In Pass-count mode, each pass equals 256 verified rounds per worker.
    let target_per_worker = if c.run_mode == RunMode::Passes {
        Some(c.target_passes.saturating_mul(CPU_ROUNDS_PER_PASS))
    } else {
        None
    };
    let phase_label = c.cpu_workload.label();
    thread::scope(|scope| {
        let mut tasks = Vec::new();
        for worker_idx in 0..c.workers {
            let next_publish = &next_publish;
            let live = &live;
            let published_rounds = &published_rounds;
            let published_errors = &published_errors;
            let worker_health = &worker_health;
            let workload = c.cpu_workload;
            tasks.push(scope.spawn(move || {
                let mut blend_buf = if workload == CpuWorkload::Blend {
                    vec![0u64; 8192]
                } else {
                    Vec::new()
                };
                let mut n = 0u64;
                let mut bad = 0u64;
                while !reached_deadline(deadline)
                    && !cancel.load(Ordering::Relaxed)
                    && target_per_worker.is_none_or(|t| n < t)
                {
                    let ok = match workload {
                        CpuWorkload::IntegerVerify => cpu_result_is_correct(calculation(0)),
                        CpuWorkload::FloatFma => fma_calculation() == fma_expected,
                        CpuWorkload::Blend => blend_calculation(&mut blend_buf),
                    };
                    if !ok {
                        bad += 1;
                        worker_health[worker_idx].store(false, Ordering::Relaxed);
                        published_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    n += 1;
                    if n % 64 == 0 {
                        published_rounds.fetch_add(64, Ordering::Relaxed);
                        // Publish at most ~5 times a second across all workers.
                        if let Ok(mut next) = next_publish.lock() {
                            if Instant::now() >= *next {
                                *next = Instant::now() + Duration::from_millis(200);
                                drop(next);
                                let health: Vec<bool> = worker_health
                                    .iter()
                                    .map(|f| f.load(Ordering::Relaxed))
                                    .collect();
                                publish_workers(
                                    live,
                                    published_rounds.load(Ordering::Relaxed),
                                    published_errors.load(Ordering::Relaxed),
                                    phase_label,
                                    health,
                                );
                            }
                        }
                    }
                }
                (n, bad)
            }));
        }
        for task in tasks {
            match task.join() {
                Ok((n, bad)) => {
                    rounds += n;
                    failures += bad;
                    published_rounds.fetch_add(n % 64, Ordering::Relaxed);
                }
                Err(_) => failures += 1,
            }
        }
    });
    let health: Vec<bool> = worker_health
        .iter()
        .map(|f| f.load(Ordering::Relaxed))
        .collect();
    publish_workers(live, rounds, failures, phase_label, health);
    if failures > 0 {
        Ok(Outcome {
            verified: rounds.saturating_sub(failures),
            detail: format!("{failures} computation mismatches or worker failures after {rounds} checked rounds"),
            errors: Vec::new(),
            error_count: failures,
        })
    } else {
        Ok(Outcome::new(
            rounds,
            format!(
                "{rounds} checked {} rounds across {} workers; no mismatches detected",
                phase_label, c.workers
            ),
        ))
    }
}

fn memory(
    c: &Config,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    live: &Mutex<State>,
) -> Result<Outcome, TestError> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let requested = c.mib as u64 * 1024 * 1024;
    let allowed_mib = safe_max_memory_mib(sys.available_memory());
    if c.mib > allowed_mib {
        return Err(format!(
            "Requested {} MiB exceeds safe available memory limit ({allowed_mib} MiB, preserving OS headroom).",
            c.mib,
        )
        .into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(requested as usize)
        .map_err(|e| format!("Allocation failed: {e}"))?;
    bytes.resize(requested as usize, 0u8);
    let pattern = c.mem_pattern;
    let target_passes = (c.run_mode == RunMode::Passes).then_some(c.target_passes);
    let mut passes = 0u64;
    let mut errors: Vec<ErrorDetail> = Vec::new();
    let mut error_count = 0u64;
    let mut last_publish = Instant::now();
    while !reached_deadline(deadline)
        && !cancel.load(Ordering::Relaxed)
        && target_passes.is_none_or(|t| passes < t)
    {
        // Fill with the deterministic per-byte pattern for this pass. Check
        // cancellation per 1 MiB block so Stop stays responsive on large regions.
        for (block, chunk) in bytes.chunks_mut(1024 * 1024).enumerate() {
            if cancel.load(Ordering::Relaxed) || reached_deadline(deadline) {
                publish(live, passes, error_count, pattern.label());
                return Ok(Outcome {
                    verified: passes,
                    detail: format!(
                        "{passes} complete write/read passes over {} MiB with {}",
                        c.mib,
                        pattern.label()
                    ),
                    errors,
                    error_count,
                });
            }
            let base = block * 1024 * 1024;
            for (offset, byte) in chunk.iter_mut().enumerate() {
                *byte = pattern.byte(base + offset);
            }
        }
        std::hint::black_box(&mut bytes);
        // Verify; on mismatch keep an address-style record like a memory diagnostic.
        'verify: for (block, chunk) in bytes.chunks(1024 * 1024).enumerate() {
            if cancel.load(Ordering::Relaxed) || reached_deadline(deadline) {
                publish(live, passes, error_count, pattern.label());
                return Ok(Outcome::new(
                    passes,
                    format!(
                        "{passes} complete write/read passes over {} MiB with {}",
                        c.mib,
                        pattern.label()
                    ),
                ));
            }
            let base = block * 1024 * 1024;
            for (offset, byte) in chunk.iter().enumerate() {
                let index = base + offset;
                let expected = pattern.byte(index);
                if *byte != expected {
                    note_error(
                        &mut errors,
                        &mut error_count,
                        ErrorDetail::new(
                            format!("pass {} · offset {index} ({:#x})", passes + 1, index),
                            format!("{expected:#04x}"),
                            format!("{:#04x}", byte),
                        ),
                    );
                    if error_count >= MAX_STORED_ERRORS as u64 {
                        break 'verify;
                    }
                }
            }
        }
        if error_count > 0 {
            let detail = format!(
                "{error_count} byte mismatch(es) in pass {} with {}",
                passes + 1,
                pattern.label()
            );
            publish(live, passes, error_count, "Memory patterns");
            return Ok(Outcome {
                verified: passes,
                detail,
                errors,
                error_count,
            });
        }
        passes += 1;
        if last_publish.elapsed() >= Duration::from_millis(200) {
            last_publish = Instant::now();
            publish(live, passes, 0, pattern.label());
        }
    }
    publish(live, passes, 0, pattern.label());
    Ok(Outcome::new(
        passes,
        format!(
            "{passes} complete write/read passes over {} MiB with {}; no mismatches detected",
            c.mib,
            pattern.label()
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
fn disk(
    c: &Config,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    live: &Mutex<State>,
) -> Result<Outcome, TestError> {
    disk_with_cap(c, cancel, deadline, c.disk_write_limit_mib, live)
}
fn disk_with_cap(
    c: &Config,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    cap_mib: u64,
    live: &Mutex<State>,
) -> Result<Outcome, TestError> {
    let (_scratch, mut file) = scratch(&c.directory)?;
    let pattern = c.disk_pattern;
    let block: Vec<u8> = (0..1024 * 1024).map(|i| pattern.byte(i)).collect();
    let target_passes = (c.run_mode == RunMode::Passes).then_some(c.target_passes);
    let mut passes = 0u64;
    let mut written = 0u64;
    let mut errors: Vec<ErrorDetail> = Vec::new();
    let mut error_count = 0u64;
    while !reached_deadline(deadline)
        && !cancel.load(Ordering::Relaxed)
        && written + c.mib as u64 <= cap_mib
        && target_passes.is_none_or(|t| passes < t)
    {
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        for _ in 0..c.mib {
            if cancel.load(Ordering::Relaxed) || reached_deadline(deadline) {
                publish(live, passes, error_count, pattern.label());
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
        for chunk_index in 0..c.mib {
            if cancel.load(Ordering::Relaxed) || reached_deadline(deadline) {
                publish(live, passes, error_count, pattern.label());
                return Ok(Outcome {
                    verified: passes,
                    detail: format!("{passes} verified passes; {written} MiB written"),
                    errors,
                    error_count,
                });
            }
            file.read_exact(&mut read)
                .map_err(|e| format!("Read failed: {e}"))?;
            if read != block {
                let offset = read
                    .iter()
                    .zip(block.iter())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                note_error(
                    &mut errors,
                    &mut error_count,
                    ErrorDetail::new(
                        format!(
                            "pass {} · chunk {chunk_index} · offset {offset}",
                            passes + 1
                        ),
                        format!("{:#04x}", block[offset]),
                        format!("{:#04x}", read[offset]),
                    ),
                );
            }
        }
        if error_count > 0 {
            let detail = format!(
                "{error_count} readback mismatch(es) with {}",
                pattern.label()
            );
            publish(live, passes, error_count, pattern.label());
            return Ok(Outcome {
                verified: passes,
                detail,
                errors,
                error_count,
            });
        }
        passes += 1;
        publish(live, passes, 0, pattern.label());
    }
    publish(live, passes, 0, pattern.label());
    Ok(Outcome::new(
        passes,
        format!(
            "{passes} verified passes; {written} MiB written with {}",
            pattern.label()
        ),
    ))
}

/// Run the CPU and memory correctness checks at the same time and present one
/// combined result. Each subsystem keeps its own safety limits and publishes
/// into a private state; this function merges their counters for the live UI
/// and keeps the first mismatches from either worker.
fn combined(
    c: &Config,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    live: &Mutex<State>,
) -> Result<Outcome, TestError> {
    let mut cpu_config = c.clone();
    cpu_config.kind = Kind::Cpu;
    let mut mem_config = c.clone();
    mem_config.kind = Kind::Memory;

    let cpu_live = Mutex::new(State::default());
    let mem_live = Mutex::new(State::default());
    let done = AtomicBool::new(false);
    let merge = || {
        let cpu = cpu_live.lock().map(|s| s.live.clone()).unwrap_or_default();
        let mem = mem_live.lock().map(|s| s.live.clone()).unwrap_or_default();
        publish_workers(
            live,
            cpu.units + mem.units,
            cpu.errors + mem.errors,
            "CPU + memory",
            cpu.worker_ok,
        );
    };

    let (cpu_result, mem_result) = thread::scope(|scope| {
        let cpu_handle = scope.spawn(|| cpu(&cpu_config, cancel, deadline, &cpu_live));
        let mem_handle = scope.spawn(|| memory(&mem_config, cancel, deadline, &mem_live));
        let merger = scope.spawn(|| {
            while !done.load(Ordering::Relaxed) && !reached_deadline(deadline) {
                merge();
                thread::sleep(Duration::from_millis(200));
            }
        });
        let cpu_result = cpu_handle.join().unwrap_or_else(|_| {
            Err(TestError::Failed(
                "CPU worker panicked during combined check".into(),
            ))
        });
        let mem_result = mem_handle.join().unwrap_or_else(|_| {
            Err(TestError::Failed(
                "Memory worker panicked during combined check".into(),
            ))
        });
        done.store(true, Ordering::Relaxed);
        let _ = merger.join();
        (cpu_result, mem_result)
    });
    merge();

    let mut verified = 0u64;
    let mut errors: Vec<ErrorDetail> = Vec::new();
    let mut error_count = 0u64;
    let mut details: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for (name, result) in [("CPU", cpu_result), ("Memory", mem_result)] {
        match result {
            Ok(outcome) => {
                verified += outcome.verified;
                error_count += outcome.error_count;
                for detail in outcome.errors {
                    if errors.len() < MAX_STORED_ERRORS {
                        errors.push(detail);
                    }
                }
                details.push(format!("{name}: {}", outcome.detail));
            }
            Err(TestError::Failed(message)) => failures.push(format!("{name}: {message}")),
            Err(TestError::Unavailable(message)) => {
                return Err(TestError::Unavailable(format!("{name}: {message}")));
            }
        }
    }
    publish(live, verified, error_count, "CPU + memory");
    if !failures.is_empty() {
        return Err(TestError::Failed(failures.join(" · ")));
    }
    Ok(Outcome {
        verified,
        detail: format!(
            "Combined CPU + memory load; {} · {} mismatch(es)",
            details.join(" · "),
            error_count
        ),
        errors,
        error_count,
    })
}

fn gpu(
    config: &Config,
    adapter: Option<&wgpu::Adapter>,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    live: &Mutex<State>,
) -> Result<Outcome, TestError> {
    use wgpu::util::DeviceExt;
    let intensity = config.gpu_intensity;
    let (words, steps) = intensity.params();
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
    let shader_source = format!(
        r#"
@group(0) @binding(0) var<storage, read_write> output: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    var x = id.x;
    for (var i = 0u; i < {steps}u; i = i + 1u) {{ x = x * 1664525u + 1013904223u; }}
    output[id.x] = x;
}}
"#
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("verified compute"),
        source: wgpu::ShaderSource::Wgsl(shader_source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("verified compute"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bytes = vec![0u8; words * 4];
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
    let mut expected = Vec::with_capacity(words);
    for i in 0..words {
        let mut x = i as u32;
        for _ in 0..steps {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        }
        expected.push(x);
    }
    let target_dispatches = config.pass_target_units();
    let mut rounds = 0u64;
    let mut last_publish = Instant::now();
    while !reached_deadline(deadline)
        && !cancel.load(Ordering::Relaxed)
        && target_dispatches.is_none_or(|t| rounds < t)
    {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((words / 64) as u32, 1, 1);
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
            if cancel.load(Ordering::Relaxed) || reached_deadline(deadline) {
                publish(live, rounds, 0, intensity.label());
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
                    publish(live, rounds, 1, intensity.label());
                    return Ok(Outcome {
                        verified: rounds,
                        detail: format!("GPU result mismatch at word {index} after {rounds} verified dispatches"),
                        errors: vec![ErrorDetail::new(
                            format!("word {index}"),
                            format!("{:#010x}", expected[index]),
                            format!("{value:#010x}"),
                        )],
                        error_count: 1,
                    });
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
        if last_publish.elapsed() >= Duration::from_millis(200) {
            last_publish = Instant::now();
            publish(live, rounds, 0, intensity.label());
        }
    }
    publish(live, rounds, 0, intensity.label());
    Ok(Outcome::new(
        rounds,
        format!(
            "{rounds} verified compute dispatches on {} ({:?}) with {}; no mismatches detected",
            info.name,
            info.backend,
            intensity.label()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dummy_live() -> Mutex<State> {
        Mutex::new(State::default())
    }
    fn test_config(kind: Kind) -> Config {
        Config {
            kind,
            run_mode: RunMode::Timed,
            seconds: 2,
            target_passes: 1,
            workers: 1,
            cpu_workload: CpuWorkload::IntegerVerify,
            mib: 1,
            directory: std::env::temp_dir(),
            gpu_adapter: None,
            disk_write_limit_mib: 1,
            mem_pattern: MemPattern::Alternating,
            disk_pattern: DiskPattern::Constant,
            gpu_intensity: GpuIntensity::Standard,
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
            Some(Instant::now() + Duration::from_secs(10)),
            1,
            &dummy_live(),
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
            Some(Instant::now() + Duration::from_secs(2)),
            &dummy_live(),
        )
        .unwrap();
        assert!(!result.detail.starts_with("0 "), "{result:?}");
    }
    #[test]
    fn memory_pass_count_mode_stops_after_exact_passes() {
        let mut cfg = test_config(Kind::Memory);
        cfg.run_mode = RunMode::Passes;
        cfg.target_passes = 2;
        cfg.mib = 1;
        let result = memory(&cfg, &AtomicBool::new(false), None, &dummy_live()).unwrap();
        assert_eq!(result.verified, 2);
    }
    #[test]
    fn cpu_supports_fma_and_blend_workloads_and_continuous_cancel() {
        for workload in CpuWorkload::ALL {
            let mut cfg = test_config(Kind::Cpu);
            cfg.cpu_workload = workload;
            cfg.workers = 2;
            let result = cpu(
                &cfg,
                &AtomicBool::new(false),
                Some(Instant::now() + Duration::from_millis(250)),
                &dummy_live(),
            )
            .unwrap();
            assert!(result.verified > 0, "{workload:?} verified 0 rounds");
        }
        // Continuous mode runs until cancelled.
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut cfg = test_config(Kind::Cpu);
        cfg.run_mode = RunMode::Continuous;
        cfg.workers = 2;
        start(
            cfg,
            state.clone(),
            stop.clone(),
            None,
            ReportContext::default(),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(120));
        stop.store(true, Ordering::Relaxed);
        let until = Instant::now() + Duration::from_secs(10);
        while state.lock().unwrap().running && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        let guard = state.lock().unwrap();
        assert!(!guard.running);
        assert_eq!(guard.results[0].status, Status::Stopped);
        assert!(guard.results[0].verified_units.unwrap_or(0) > 0);
    }
    #[test]
    fn cancelled_disk_test_does_not_write() {
        let result = disk_with_cap(
            &test_config(Kind::Disk),
            &AtomicBool::new(true),
            Some(Instant::now() + Duration::from_secs(1)),
            1,
            &dummy_live(),
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
                &test_config(Kind::Gpu),
                None,
                &AtomicBool::new(false),
                Some(Instant::now() + Duration::from_secs(1)),
                &dummy_live(),
            ),
            Err(TestError::Unavailable(_))
        ));
    }

    #[test]
    fn mismatch_evidence_survives_cancellation_and_report_classification() {
        let evidence = ErrorDetail::new("offset 7", "0x55", "0x54");
        let (status, result) = classify(
            Ok(Outcome {
                verified: 3,
                detail: "Readback mismatch".into(),
                errors: vec![evidence.clone()],
                error_count: 1,
            }),
            true,
        );
        assert_eq!(status, Status::Failed);
        assert_eq!(result.verified, 3);
        assert_eq!(result.error_count, 1);
        assert_eq!(result.errors, vec![evidence]);
    }

    #[test]
    fn disk_pass_count_must_fit_the_write_cap() {
        let mut config = test_config(Kind::Disk);
        config.run_mode = RunMode::Passes;
        config.target_passes = 2;
        config.mib = 1;
        config.disk_write_limit_mib = 1;
        let state = Arc::new(Mutex::new(State::default()));
        let error = start(
            config,
            Arc::clone(&state),
            Arc::new(AtomicBool::new(false)),
            None,
            ReportContext::default(),
        )
        .unwrap_err();
        assert!(error.contains("at least 2 MiB"));
        assert!(!state.lock().unwrap().running);
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
                Some(Instant::now() + Duration::from_secs(1)),
                &dummy_live(),
            ),
            Err(TestError::Failed(_))
        ));
    }
    #[test]
    fn patterns_are_deterministic_and_distinct() {
        use std::collections::HashSet;
        for pattern in MemPattern::ALL {
            let first: Vec<u8> = (0..4096).map(|i| pattern.byte(i)).collect();
            let second: Vec<u8> = (0..4096).map(|i| pattern.byte(i)).collect();
            assert_eq!(first, second, "{} must be deterministic", pattern.label());
        }
        for pattern in DiskPattern::ALL {
            let first: Vec<u8> = (0..4096).map(|i| pattern.byte(i)).collect();
            let second: Vec<u8> = (0..4096).map(|i| pattern.byte(i)).collect();
            assert_eq!(first, second, "{} must be deterministic", pattern.label());
        }
        let bits: HashSet<u8> = (0..8).map(|i| MemPattern::WalkingZero.byte(i)).collect();
        assert_eq!(bits.len(), 8);
        let randoms: HashSet<u8> = (0..256).map(|i| MemPattern::PseudoRandom.byte(i)).collect();
        assert!(randoms.len() > 32);
        let (lw, ls) = GpuIntensity::Light.params();
        let (sw, ss) = GpuIntensity::Standard.params();
        let (hw, hs) = GpuIntensity::Heavy.params();
        let (ew, es) = GpuIntensity::Extreme.params();
        assert!(lw < sw && sw < hw && hw < ew);
        assert!(ls < ss && ss < hs && hs < es);
    }
    #[test]
    fn memory_all_patterns_complete_a_pass() {
        for pattern in MemPattern::ALL {
            let mut config = test_config(Kind::Memory);
            config.mem_pattern = pattern;
            config.mib = 1;
            let result = memory(
                &config,
                &AtomicBool::new(false),
                Some(Instant::now() + Duration::from_millis(400)),
                &dummy_live(),
            )
            .unwrap_or_else(|e| panic!("{} failed: {e:?}", pattern.label()));
            assert!(!result.detail.starts_with("0 "), "{result:?}");
            assert!(result.detail.contains(pattern.label()));
        }
    }
    #[test]
    fn note_error_caps_stored_details_but_counts_all() {
        let mut errors = Vec::new();
        let mut count = 0u64;
        for i in 0..20 {
            note_error(
                &mut errors,
                &mut count,
                ErrorDetail::new(format!("offset {i}"), "0x55", "0xaa"),
            );
        }
        assert_eq!(count, 20);
        assert_eq!(errors.len(), MAX_STORED_ERRORS);
        assert_eq!(errors[0].location, "offset 0");
        assert_eq!(errors[7].location, "offset 7");
    }
    #[test]
    fn live_progress_is_published() {
        let live = Arc::new(Mutex::new(State::default()));
        let mut config = test_config(Kind::Disk);
        config.mib = 1;
        config.directory = std::env::temp_dir();
        let result = disk_with_cap(
            &config,
            &AtomicBool::new(false),
            Some(Instant::now() + Duration::from_secs(10)),
            1,
            &live,
        )
        .unwrap();
        let guard = live.lock().unwrap();
        assert_eq!(guard.live.units, 1);
        assert_eq!(guard.live.errors, 0);
        assert!(!guard.live.phase.is_empty());
        assert!(result.error_count == 0);
    }
    #[test]
    fn combined_runs_cpu_and_memory_with_merged_progress() {
        let live = Arc::new(Mutex::new(State::default()));
        let mut config = test_config(Kind::Combined);
        config.workers = 2;
        config.mib = 1;
        let result = combined(
            &config,
            &AtomicBool::new(false),
            Some(Instant::now() + Duration::from_millis(400)),
            &live,
        )
        .unwrap();
        assert!(result.detail.contains("CPU:"), "{result:?}");
        assert!(result.detail.contains("Memory:"), "{result:?}");
        assert!(result.verified > 0, "{result:?}");
        assert_eq!(result.error_count, 0);
        let guard = live.lock().unwrap();
        assert_eq!(guard.live.units, result.verified);
        assert_eq!(guard.live.phase, "CPU + memory");
    }
    #[test]
    fn combined_stops_when_cancelled() {
        let cancel = Arc::new(AtomicBool::new(false));
        cancel.store(true, Ordering::Relaxed);
        let result = combined(
            &test_config(Kind::Combined),
            &cancel,
            Some(Instant::now() + Duration::from_secs(1)),
            &dummy_live(),
        )
        .unwrap();
        assert!(result.verified < 1000, "{result:?}");
    }
    #[test]
    fn deterministic_cpu() {
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
            run_mode: RunMode::Timed,
            seconds: 1,
            target_passes: 1,
            workers: 1,
            cpu_workload: CpuWorkload::IntegerVerify,
            mib: MAX_DISK_FILE_MIB + 1,
            directory: std::env::temp_dir(),
            gpu_adapter: None,
            disk_write_limit_mib: 1,
            mem_pattern: MemPattern::Alternating,
            disk_pattern: DiskPattern::Constant,
            gpu_intensity: GpuIntensity::Standard,
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
    #[test]
    fn safe_memory_limit_preserves_os_headroom() {
        // 16 GiB available -> 75% is 12 GiB, minus 512 MiB reserve -> 12288 MiB
        let sixteen_gib = 16 * 1024 * 1024 * 1024u64;
        assert_eq!(safe_max_memory_mib(sixteen_gib), 12_288);
        // Low memory -> clamped to at least 1 MiB
        assert_eq!(safe_max_memory_mib(128 * 1024 * 1024), 1);
        assert_eq!(format_duration(15), "15s");
        assert_eq!(format_duration(1800), "30m");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(5400), "1h 30m");
        assert_eq!(format_duration(86400), "24h");
    }
}
