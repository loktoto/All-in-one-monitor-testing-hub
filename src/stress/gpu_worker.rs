//! Isolate graphics-driver calls from the desktop process.

use super::{
    gpu, publish, reached_deadline, Config, GpuIntensity, Kind, Outcome, RunMode, State, TestError,
    MAX_TIMED_SECONDS,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const GPU_WORKER_FLAG: &str = "--hh-gpu-worker";
const MAX_MESSAGE: u64 = 32 * 1024;
const POLL: Duration = Duration::from_millis(100);
const STALL: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct AdapterKey {
    name: String,
    vendor: u32,
    device: u32,
    driver: String,
    driver_info: String,
    backend: String,
    native_id: Option<u64>,
}

impl From<&wgpu::Adapter> for AdapterKey {
    fn from(adapter: &wgpu::Adapter) -> Self {
        let info = adapter.get_info();
        #[cfg(target_os = "windows")]
        let native_id = crate::telemetry::dxgi_adapter_details(adapter).map(|(luid, _, _)| luid);
        #[cfg(not(target_os = "windows"))]
        let native_id = None;
        Self {
            name: info.name,
            vendor: info.vendor,
            device: info.device,
            driver: info.driver,
            driver_info: info.driver_info,
            backend: format!("{:?}", info.backend),
            native_id,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Job {
    adapter: AdapterKey,
    mode: RunMode,
    seconds: u64,
    passes: u64,
    intensity: GpuIntensity,
}

#[derive(Serialize, Deserialize)]
enum Event {
    Progress {
        verified: u64,
        errors: u64,
        phase: String,
    },
    Finished(Result<Outcome, TestError>),
}

fn read_message<T: DeserializeOwned>(from: &mut impl BufRead) -> io::Result<T> {
    let mut line = Vec::new();
    from.take(MAX_MESSAGE).read_until(b'\n', &mut line)?;
    if line.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Incomplete GPU worker message",
        ));
    }
    serde_json::from_slice(&line).map_err(io::Error::other)
}

fn write_message(to: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *to, value).map_err(io::Error::other)?;
    to.write_all(b"\n")?;
    to.flush()
}

struct Worker {
    child: Child,
    // While this pipe stays open the worker knows its supervisor is alive.
    input: Option<ChildStdin>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn supervise(
    executable: &Path,
    config: &Config,
    adapter: Option<AdapterKey>,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    state: &Mutex<State>,
) -> Result<Outcome, TestError> {
    let adapter =
        adapter.ok_or_else(|| TestError::Unavailable("No GPU adapter was selected.".into()))?;
    let job = Job {
        adapter,
        mode: config.run_mode,
        seconds: config.seconds,
        passes: config.target_passes,
        intensity: config.gpu_intensity,
    };
    let mut command = Command::new(executable);
    command
        .arg(GPU_WORKER_FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start GPU worker: {e}"))?;
    let stdout = child.stdout.take();
    let input = child.stdin.take();
    let mut worker = Worker { child, input };
    let writer = worker
        .input
        .as_mut()
        .ok_or("GPU worker input is unavailable")?;
    write_message(writer, &job).map_err(|e| format!("Could not send the GPU check: {e}"))?;
    let mut reader = BufReader::new(stdout.ok_or("GPU worker output is unavailable")?);
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(event) = read_message::<Event>(&mut reader) {
            let finished = matches!(event, Event::Finished(_));
            if send.send(event).is_err() || finished {
                break;
            }
        }
    });

    publish(state, 0, 0, "Starting isolated GPU worker");
    monitor_worker(receive, cancel, deadline, state, STALL)
}

fn monitor_worker(
    receive: mpsc::Receiver<Event>,
    cancel: &AtomicBool,
    deadline: Option<Instant>,
    state: &Mutex<State>,
    stall: Duration,
) -> Result<Outcome, TestError> {
    let mut last_progress = Instant::now();
    let mut verified = 0;
    let mut error_count = 0;
    loop {
        if cancel.load(Ordering::Relaxed) || reached_deadline(deadline) {
            return Ok(Outcome {
                verified,
                detail: format!("{verified} verified GPU dispatches. Pending work was stopped."),
                errors: Vec::new(),
                error_count,
            });
        }
        match receive.recv_timeout(POLL) {
            Ok(Event::Progress {
                verified: count,
                errors,
                phase,
            }) => {
                if count > verified {
                    last_progress = Instant::now();
                    verified = count;
                }
                error_count = errors;
                publish(state, verified, errors, &phase);
            }
            Ok(Event::Finished(result)) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(format!(
                    "GPU worker exited without a result after {verified} verified dispatches."
                )
                .into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if last_progress.elapsed() >= stall {
            return Err(format!(
                "GPU worker stalled for {} s after {verified} verified dispatches.",
                stall.as_secs()
            )
            .into());
        }
    }
}

/// Entry point used by the desktop executable when launched as its own GPU worker.
pub fn run_gpu_worker() -> Result<(), String> {
    let job: Job = read_message(&mut io::stdin().lock()).map_err(|e| e.to_string())?;
    let valid = match job.mode {
        RunMode::Timed => (1..=MAX_TIMED_SECONDS).contains(&job.seconds),
        RunMode::Continuous => true,
        RunMode::Passes => (1..=10_000).contains(&job.passes),
    };
    if !valid {
        return Err("Invalid GPU worker limits.".into());
    }
    thread::spawn(|| {
        // EOF means the parent exited or cancelled. Exit even if a driver call
        // is blocked in the worker thread.
        let _ = io::stdin().read(&mut [0]);
        std::process::exit(0);
    });
    let config = Config {
        kind: Kind::Gpu,
        run_mode: job.mode,
        seconds: job.seconds,
        target_passes: job.passes,
        gpu_intensity: job.intensity,
        ..Config::default()
    };
    let live = Mutex::new(State::default());
    let result = thread::scope(|scope| {
        let task = scope.spawn(|| {
            let mut matches = crate::telemetry::discover_adapters()
                .into_iter()
                .filter(|a| !a.software && AdapterKey::from(&a.handle) == job.adapter);
            let first = matches.next().ok_or_else(|| {
                TestError::Unavailable("Selected GPU was not available to the worker.".into())
            })?;
            if job.adapter.native_id.is_none() && matches.next().is_some() {
                return Err(TestError::Unavailable(
                    "Multiple identical adapters matched the selected GPU.".into(),
                ));
            }
            let deadline = (config.run_mode == RunMode::Timed)
                .then(|| Instant::now() + Duration::from_secs(config.seconds));
            gpu(
                &config,
                Some(&first.handle),
                &AtomicBool::new(false),
                deadline,
                &live,
            )
        });
        let mut output = io::stdout().lock();
        let mut last = (0, 0);
        while !task.is_finished() {
            let progress = live
                .lock()
                .map(|state| state.live.clone())
                .unwrap_or_default();
            if (progress.units, progress.errors) != last {
                last = (progress.units, progress.errors);
                if write_message(
                    &mut output,
                    &Event::Progress {
                        verified: progress.units,
                        errors: progress.errors,
                        phase: progress.phase,
                    },
                )
                .is_err()
                {
                    std::process::exit(1);
                }
            }
            thread::sleep(POLL);
        }
        task.join()
            .unwrap_or_else(|_| Err(TestError::Failed("GPU worker panicked.".into())))
    });
    write_message(&mut io::stdout().lock(), &Event::Finished(result)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_reads_consecutive_bounded_messages() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &Event::Progress {
                verified: 7,
                errors: 0,
                phase: "test".into(),
            },
        )
        .unwrap();
        write_message(
            &mut bytes,
            &Event::Finished(Ok(Outcome::new(8, "checked".into()))),
        )
        .unwrap();
        let mut reader = bytes.as_slice();
        assert!(matches!(
            read_message::<Event>(&mut reader).unwrap(),
            Event::Progress { verified: 7, .. }
        ));
        assert!(matches!(
            read_message::<Event>(&mut reader).unwrap(),
            Event::Finished(Ok(Outcome { verified: 8, .. }))
        ));
        assert!(read_message::<Event>(&mut reader).is_err());
        assert!(
            read_message::<Event>(&mut vec![b'a'; MAX_MESSAGE as usize + 1].as_slice()).is_err()
        );
    }

    #[test]
    fn stopping_and_stalling_are_distinct_outcomes() {
        let (_send, receive) = mpsc::channel();
        let state = Mutex::new(State::default());
        let cancel = AtomicBool::new(true);
        let stopped = monitor_worker(receive, &cancel, None, &state, STALL).unwrap();
        assert_eq!(stopped.verified, 0);

        let (_send, receive) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let stalled = monitor_worker(receive, &cancel, None, &state, Duration::from_millis(10));
        assert!(
            matches!(stalled, Err(TestError::Failed(reason)) if reason.contains("GPU worker stalled"))
        );
    }
}
