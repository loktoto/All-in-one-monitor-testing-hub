//! Queue-depth-one sequential and random I/O on a private scratch file.

use crate::stress;
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const ALIGNMENT: usize = 4096;
pub const SIZES_MIB: [usize; 4] = [16, 32, 64, 128];

#[derive(Clone, Copy)]
pub enum Access {
    SequentialRead,
    SequentialWrite,
    RandomRead,
    RandomWrite,
    RandomMixed,
}

impl Access {
    const ALL: [Self; 5] = [
        Self::SequentialWrite,
        Self::SequentialRead,
        Self::RandomRead,
        Self::RandomWrite,
        Self::RandomMixed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SequentialRead => "Sequential 1 MiB read",
            Self::SequentialWrite => "Sequential 1 MiB write",
            Self::RandomRead => "Random 4 KiB read",
            Self::RandomWrite => "Random 4 KiB write",
            Self::RandomMixed => "Random 4 KiB mixed (70% read)",
        }
    }

    fn random(self) -> bool {
        matches!(
            self,
            Self::RandomRead | Self::RandomWrite | Self::RandomMixed
        )
    }

    fn writes(self, operation: usize) -> bool {
        matches!(self, Self::SequentialWrite | Self::RandomWrite)
            || (matches!(self, Self::RandomMixed) && operation % 10 >= 7)
    }
}

#[derive(Clone)]
pub struct ResultRow {
    pub access: Access,
    pub mbps: f64,
    pub iops: f64,
    pub mean_us: f64,
}

#[derive(Clone)]
pub struct Progress {
    pub running: bool,
    pub status: String,
    pub rows: Vec<ResultRow>,
}

pub struct Run {
    pub folder: PathBuf,
    pub mib: usize,
    pub direct: bool,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}

impl Run {
    pub fn start(
        folder: PathBuf,
        mib: usize,
        direct: bool,
        notify: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        if !SIZES_MIB.contains(&mib) || (direct && !cfg!(windows)) {
            return Err("Unsupported test size or I/O mode.".into());
        }
        let progress = Arc::new(Mutex::new(Progress {
            running: true,
            status: "Preparing benchmark…".into(),
            rows: Vec::new(),
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_folder = folder.clone();
        let worker_cancel = Arc::clone(&cancel);
        let worker_progress = Arc::clone(&progress);
        thread::Builder::new()
            .name("storage-benchmark".into())
            .spawn(move || {
                let result = exercise(
                    &worker_folder,
                    mib,
                    direct,
                    &worker_cancel,
                    |access, row| {
                        if let Ok(mut state) = worker_progress.lock() {
                            state.status = access.label().into();
                            if let Some(row) = row {
                                state.rows.push(row);
                            }
                        }
                        notify();
                    },
                );
                if let Ok(mut state) = worker_progress.lock() {
                    state.running = false;
                    state.status = match result {
                        Ok(true) => "Finished".into(),
                        Ok(false) => "Stopped".into(),
                        Err(error) => format!("Benchmark could not finish: {error}"),
                    };
                }
                notify();
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            folder,
            mib,
            direct,
            cancel,
            progress,
        })
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn progress(&self) -> Progress {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.stop();
    }
}

fn exercise(
    folder: &std::path::Path,
    mib: usize,
    direct: bool,
    cancel: &AtomicBool,
    mut publish: impl FnMut(Access, Option<ResultRow>),
) -> Result<bool, String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const DELETE_ON_CLOSE: u32 = 0x0400_0000;
        const NO_BUFFERING: u32 = 0x2000_0000;
        options.custom_flags(DELETE_ON_CLOSE | if direct { NO_BUFFERING } else { 0 });
    }
    let _ = direct;
    let (scratch, mut file) = stress::scratch_with_options(folder, &options)?;
    #[cfg(unix)]
    scratch
        .unlink()
        .map_err(|e| format!("Cannot unlink the benchmark scratch file: {e}"))?;
    let mut allocation = vec![0_u8; 1024 * 1024 + ALIGNMENT];
    let offset = allocation.as_ptr().align_offset(ALIGNMENT);
    let buffer = &mut allocation[offset..offset + 1024 * 1024];
    let mut seed = 0x4d59_5df4_d0f3_3173_u64;
    for word in buffer.chunks_exact_mut(8) {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        word.copy_from_slice(&seed.to_ne_bytes());
    }
    for access in Access::ALL {
        publish(access, None);
        let size = if access.random() {
            ALIGNMENT
        } else {
            buffer.len()
        };
        let blocks = mib * 1024 * 1024 / size;
        let start = Instant::now();
        let mut service = Duration::ZERO;
        for operation in 0..blocks {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let operation_start = Instant::now();
            if operation == 0 || access.random() {
                let block = if access.random() {
                    random_block(operation, blocks)
                } else {
                    0
                };
                file.seek(SeekFrom::Start((block * size) as u64))
                    .map_err(|e| e.to_string())?;
            }
            if access.writes(operation) {
                file.write_all(&buffer[..size]).map_err(|e| e.to_string())?;
            } else {
                file.read_exact(&mut buffer[..size])
                    .map_err(|e| e.to_string())?;
            }
            service += operation_start.elapsed();
        }
        if !matches!(access, Access::SequentialRead | Access::RandomRead) {
            file.sync_data().map_err(|e| e.to_string())?;
        }
        let seconds = start.elapsed().as_secs_f64().max(f64::EPSILON);
        publish(
            access,
            Some(ResultRow {
                access,
                mbps: (blocks * size) as f64 / (seconds * 1_000_000.0),
                iops: blocks as f64 / seconds,
                mean_us: service.as_secs_f64() * 1_000_000.0 / blocks as f64,
            }),
        );
    }
    drop(file);
    drop(scratch);
    Ok(true)
}

fn random_block(operation: usize, blocks: usize) -> usize {
    // XOR shifts and odd multiplications permute the low bits of each pass.
    let n = (operation ^ (operation >> 5)).wrapping_mul(0x9e37_79b1) % blocks;
    ((n ^ (n >> 7)).wrapping_mul(0x5bd1_e995).wrapping_add(7417)) % blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_pass_visits_each_requested_block_exactly_once() {
        for mib in SIZES_MIB {
            let blocks = mib * 1024 * 1024 / ALIGNMENT;
            let mut results: Vec<_> = (0..blocks).map(|n| random_block(n, blocks)).collect();
            results.sort_unstable();
            assert!(results.into_iter().eq(0..blocks));
        }
    }

    fn run_test(direct: bool) {
        let dir = std::env::temp_dir().join(format!(
            "hardware-hub-bench-test-{}-{direct}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut rows = Vec::new();
        let cancel = AtomicBool::new(false);
        let result = exercise(&dir, 16, direct, &cancel, |_, row| rows.extend(row));
        std::fs::remove_dir(&dir).unwrap();
        assert!(result.unwrap());
        assert_eq!(rows.len(), 5);
        assert!(rows
            .iter()
            .all(|row| row.mbps.is_finite() && row.mbps > 0.0 && row.iops > 0.0));
    }

    #[test]
    fn measures_all_passes_and_removes_the_private_file() {
        run_test(false);
    }

    #[test]
    fn stops_and_removes_scratch_when_cancelled_between_passes() {
        let dir =
            std::env::temp_dir().join(format!("hardware-hub-bench-stop-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let cancel = AtomicBool::new(false);
        let run = exercise(&dir, 16, false, &cancel, |_, row| {
            if row.is_some() {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        std::fs::remove_dir(&dir).unwrap();
        assert!(!run.unwrap());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the Windows temporary volume to support unbuffered I/O"]
    fn measures_direct_io_with_aligned_blocks() {
        run_test(true);
    }
}
