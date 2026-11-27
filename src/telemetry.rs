use crate::sensors::{ExternalThermals, Sensor};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use sysinfo::{Components, Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Default)]
pub struct Disk {
    pub name: String,
    pub mount: String,
    pub kind: String,
    pub total: u64,
    pub available: u64,
    pub read: u64,
    pub written: u64,
}
#[derive(Clone, Default)]
pub struct Network {
    pub name: String,
    pub received: u64,
    pub transmitted: u64,
}
#[derive(Clone, Default)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
}
#[derive(Clone, Default)]
pub struct Gpu {
    pub name: String,
    pub detail: String,
}
#[derive(Clone)]
pub struct Adapter {
    pub handle: wgpu::Adapter,
    pub name: String,
    pub backend: String,
    pub software: bool,
    /// Driver and PCI identifiers, shown in the specification sheet.
    pub driver: String,
    pub driver_info: String,
    pub vendor_id: u32,
    pub device_id: u32,
}
#[derive(Clone, Default)]
pub struct Temperature {
    /// Provider key, so an unfamiliar sensor stays traceable.
    pub key: String,
    pub label: String,
    /// Short provenance tag (`SMC`, `ACPI`, `hwmon`, `sysinfo`).
    pub source: &'static str,
    pub celsius: Option<f32>,
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// Arithmetic mean of valid samples since the last reset.
    pub average: Option<f32>,
    pub cpu_core: bool,
}
#[derive(Clone)]
pub struct Fan {
    pub key: String,
    pub label: String,
    /// Short provenance tag (`SMC`, `hwmon`, `sysinfo`).
    pub source: &'static str,
    pub rpm: Option<f32>,
    pub min: Option<f32>,
    pub max: Option<f32>,
}
/// All-time low/high observed since the app started or the last reset.
#[derive(Clone, Default)]
pub struct Range {
    pub min: Option<f32>,
    pub max: Option<f32>,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub cpu_name: String,
    /// Preferred graphics API for this platform, shown in the interface.
    pub gpu_backend: String,
    pub cpu_usage: f32,
    pub cores: Vec<f32>,
    /// Per-logical-core frequency in MHz as reported by the OS (0 when unavailable).
    pub core_mhz: Vec<u64>,
    /// Sensors the OS can associate with a physical core behind each logical CPU.
    pub core_celsius: Vec<Option<f32>>,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_available: u64,
    pub disks: Vec<Disk>,
    pub networks: Vec<Network>,
    pub network_interval_seconds: Option<f64>,
    pub processes: Vec<Process>,
    pub gpus: Vec<Gpu>,
    pub adapters: Vec<Adapter>,
    pub temperatures: Vec<Temperature>,
    pub external_thermals: ExternalThermals,
    pub hwinfo_thermals: ExternalThermals,
    pub fans: Vec<Fan>,
    pub cpu_range: Range,
    pub memory_range: Range,
    pub updated_at: Option<Instant>,
    pub ready: bool,
}

// Each platform has one native graphics API to enumerate. Asking for Vulkan on
// macOS returns nothing (Metal is the only native API there), so the backend is
// selected per target instead of assuming DX12-or-Vulkan.
const fn preferred_backends() -> wgpu::Backends {
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::DX12
    }
    #[cfg(target_os = "macos")]
    {
        wgpu::Backends::METAL
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        wgpu::Backends::VULKAN
    }
}

const fn backend_label() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "DX12"
    }
    #[cfg(target_os = "macos")]
    {
        "Metal"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "Vulkan"
    }
}

// NVML is an NVIDIA driver library. It only exists on Windows and Linux, so other
// platforms skip the dynamic load entirely and report adapter inventory only.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn init_nvml() -> Option<nvml_wrapper::Nvml> {
    nvml_wrapper::Nvml::init().ok()
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn init_nvml() -> Option<nvml_wrapper::Nvml> {
    None
}

// macOS exposes synthetic APFS mounts under /System/Volumes that duplicate the
// root volume; listing them adds noise without new storage information.
#[cfg(target_os = "macos")]
fn is_synthetic_volume(mount: &str) -> bool {
    mount.starts_with("/System/Volumes/")
}
#[cfg(not(target_os = "macos"))]
fn is_synthetic_volume(_mount: &str) -> bool {
    false
}

// A single platform-preferred backend prevents the same GPU appearing once per API.
// Adapter handles are retained and reused for test selection; identical models stay distinct.
pub(crate) fn discover_adapters() -> Vec<Adapter> {
    std::panic::catch_unwind(|| {
        let preferred = preferred_backends();
        let enumerate = |backend| {
            wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: backend,
                ..Default::default()
            })
            .enumerate_adapters(backend)
        };
        let mut found = enumerate(preferred);
        if wgpu::Instance::enabled_backend_features().contains(wgpu::Backends::GL)
            && found
                .iter()
                .all(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
        {
            let fallback = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                enumerate(wgpu::Backends::GL)
            }))
            .unwrap_or_default();
            if found.is_empty()
                || fallback
                    .iter()
                    .any(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
            {
                found = fallback;
            }
        }
        found
            .into_iter()
            .map(|handle| {
                let info = handle.get_info();
                Adapter {
                    name: info.name,
                    backend: format!("{:?}", info.backend),
                    software: info.device_type == wgpu::DeviceType::Cpu,
                    driver: info.driver,
                    driver_info: info.driver_info,
                    vendor_id: info.vendor,
                    device_id: info.device,
                    handle,
                }
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Native Dx12 adapter identity and reported video memory limits.
#[cfg(target_os = "windows")]
pub(crate) fn dxgi_adapter_details(adapter: &wgpu::Adapter) -> Option<(u64, u64, u64)> {
    // Borrow the existing HAL handle and read its description without changing it.
    unsafe {
        adapter.as_hal::<wgpu::hal::api::Dx12, _, _>(|adapter| {
            let desc = adapter?.raw_adapter().GetDesc1().ok()?;
            let luid = (u64::from(desc.AdapterLuid.HighPart as u32) << 32)
                | u64::from(desc.AdapterLuid.LowPart);
            Some((
                luid,
                desc.DedicatedVideoMemory as u64,
                desc.SharedSystemMemory as u64,
            ))
        })
    }
}

fn describe_adapters(adapters: &[Adapter]) -> Vec<Gpu> {
    adapters
        .iter()
        .enumerate()
        .map(|(index, adapter)| {
            let detail = format!(
                "Adapter {index} · {}{} · wgpu inventory",
                adapter.backend,
                if adapter.software { " · software" } else { "" }
            );
            #[cfg(target_os = "windows")]
            let detail = if let Some((_, dedicated, shared)) = dxgi_adapter_details(&adapter.handle)
            {
                let gib = (1024 * 1024 * 1024) as f64;
                format!(
                    "{detail} · DXGI {:.2} GiB dedicated / {:.2} GiB shared",
                    dedicated as f64 / gib,
                    shared as f64 / gib
                )
            } else {
                detail
            };
            Gpu {
                name: adapter.name.clone(),
                detail,
            }
        })
        .collect()
}

// Match NVML to an adapter only when the name is unique on both sides. Ambiguous
// same-model devices remain separate rather than attributing telemetry incorrectly.
fn merge_gpu_lists(adapters: Vec<Gpu>, nvml: Vec<Gpu>) -> Vec<Gpu> {
    let mut a_counts = HashMap::<String, usize>::new();
    let mut n_counts = HashMap::<String, usize>::new();
    for g in &adapters {
        *a_counts.entry(g.name.to_lowercase()).or_default() += 1;
    }
    for g in &nvml {
        *n_counts.entry(g.name.to_lowercase()).or_default() += 1;
    }
    let mut merged = Vec::new();
    for mut g in adapters {
        let key = g.name.to_lowercase();
        if a_counts.get(&key) == Some(&1) && n_counts.get(&key) == Some(&1) {
            if let Some(n) = nvml.iter().find(|n| n.name.eq_ignore_ascii_case(&g.name)) {
                g.detail = format!("{} · {} · name match only (unverified)", g.detail, n.detail);
            }
        } else if n_counts.contains_key(&key) {
            g.detail
                .push_str(" · NVML mapping ambiguous; see separate sensors");
        }
        merged.push(g);
    }
    for mut n in nvml {
        let key = n.name.to_lowercase();
        if a_counts.get(&key) == Some(&1) && n_counts.get(&key) == Some(&1) {
            continue;
        }
        if a_counts.contains_key(&key) {
            n.detail.push_str(" · adapter mapping ambiguous");
        } else if !a_counts.is_empty() {
            n.detail
                .push_str(" · not matched by name; may overlap an adapter");
        }
        n.name = format!("NVML sensor source: {}", n.name);
        merged.push(n);
    }
    merged
}

// IORegistry exposes vendor-specific raw counters. Keep these separate from wgpu
// adapters: neither names nor ordering prove that two records identify the same GPU.
#[cfg(any(target_os = "macos", test))]
fn parse_ioreg_gpu(text: &str) -> Vec<Gpu> {
    let mut name = String::new();
    let mut result = Vec::new();
    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("+-o ") {
            name = rest
                .split_whitespace()
                .next()
                .unwrap_or("accelerator")
                .to_owned();
        }
        let line = line.trim();
        if !line.starts_with("\"PerformanceStatistics\" = {") {
            continue;
        }
        let mut values = Vec::new();
        for key in [
            "Device Utilization %",
            "Device Utilization % at cur p-state",
        ] {
            let prefix = format!("\"{key}\"=");
            if let Some(value) = line
                .split_once(&prefix)
                .and_then(|(_, tail)| tail.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|v| *v <= 100)
            {
                values.push(format!("{key}: {value}%"));
            }
        }
        if !values.is_empty() {
            result.push(Gpu {
                name: format!(
                    "IORegistry source: {}",
                    if name.is_empty() {
                        "accelerator"
                    } else {
                        &name
                    }
                ),
                detail: format!(
                    "{} · raw driver-reported keys; adapter mapping unverified",
                    values.join(" · ")
                ),
            });
        }
    }
    result
}
#[cfg(target_os = "macos")]
fn mac_gpu_sources() -> Vec<Gpu> {
    let args = ["-r", "-c", "IOAccelerator", "-d", "1"];
    let output = std::process::Command::new("ioreg")
        .args(args)
        .output()
        .ok()
        .or_else(|| {
            std::process::Command::new("/usr/sbin/ioreg")
                .args(args)
                .output()
                .ok()
        })
        .filter(|o| o.status.success());
    output
        .map(|o| parse_ioreg_gpu(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}
#[cfg(not(target_os = "macos"))]
fn mac_gpu_sources() -> Vec<Gpu> {
    Vec::new()
}

/// Publish through an `Arc` so the UI can read a batch with one atomic refcount
/// increment instead of rebuilding a full snapshot (processes, adapters, disks)
/// on every rendered frame.
pub fn spawn(
    shared: Arc<Mutex<Arc<Snapshot>>>,
    stop: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
    interval_ms: Arc<AtomicU64>,
    // Set while a page that lists processes is visible. Enumerating every
    // process is the most expensive part of a sample, so it only runs on
    // demand (with a slow background refresh to keep the list warm).
    process_wanted: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    spawn_with_notification(shared, stop, reset, interval_ms, process_wanted, || {})
}

/// Like `spawn`, with a notification on the sampling thread after each
/// snapshot becomes available to readers.
pub fn spawn_with_notification(
    shared: Arc<Mutex<Arc<Snapshot>>>,
    stop: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
    interval_ms: Arc<AtomicU64>,
    process_wanted: Arc<AtomicBool>,
    notify: impl Fn() + Send + 'static,
) -> thread::JoinHandle<()> {
    spawn_inner(
        shared,
        stop,
        reset,
        interval_ms,
        process_wanted,
        None,
        notify,
    )
}

#[derive(Default)]
pub struct SamplingControl {
    pub paused: AtomicBool,
    /// Zero disables the local LibreHardwareMonitor feed.
    pub lhm_port: AtomicU16,
    pub hwinfo_gadget_enabled: AtomicBool,
    sampler: OnceLock<thread::Thread>,
}

impl SamplingControl {
    pub fn wake(&self) {
        if let Some(sampler) = self.sampler.get() {
            sampler.unpark();
        }
    }
}

pub fn spawn_with_control(
    shared: Arc<Mutex<Arc<Snapshot>>>,
    stop: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
    interval_ms: Arc<AtomicU64>,
    process_wanted: Arc<AtomicBool>,
    control: Arc<SamplingControl>,
    notify: impl Fn() + Send + 'static,
) -> thread::JoinHandle<()> {
    spawn_inner(
        shared,
        stop,
        reset,
        interval_ms,
        process_wanted,
        Some(control),
        notify,
    )
}

fn spawn_inner(
    shared: Arc<Mutex<Arc<Snapshot>>>,
    stop: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
    interval_ms: Arc<AtomicU64>,
    process_wanted: Arc<AtomicBool>,
    control: Option<Arc<SamplingControl>>,
    notify: impl Fn() + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Some(control) = &control {
            let _ = control.sampler.set(thread::current());
        }
        let mut system = System::new();
        system.refresh_cpu_all();
        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();
        let mut network_refreshed_at = Instant::now();
        let mut network_interval_seconds = 0.0;
        let mut medium_at = None;
        let mut mac_gpu_sample = Vec::new();
        let adapters = discover_adapters();
        let adapter_inventory = describe_adapters(&adapters);
        // The SMC exposes many more sensors than sysinfo does; keep the reader
        // open so the enumeration cost is paid once, and fall back to sysinfo
        // when the platform has no provider.
        let mut sensors = crate::sensors::open();
        let mut temp_stats = HashMap::new();
        #[cfg(target_os = "windows")]
        let mut external_feed = crate::sensors::lhm::Feed::new();
        let mut fan_bounds: HashMap<String, (f32, f32)> = HashMap::new();
        let mut cpu_bounds: Option<(f32, f32)> = None;
        let mut memory_bounds: Option<(f32, f32)> = None;
        thread::sleep(Duration::from_millis(250));
        let nvml = init_nvml();
        let mut process_cache: Vec<Process> = Vec::new();
        let mut last_processes = None;
        let mut was_process_wanted = false;
        let mut last_frequency = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if control
                .as_ref()
                .is_some_and(|c| c.paused.load(Ordering::Relaxed))
            {
                thread::park_timeout(Duration::from_secs(5));
                medium_at = None;
                continue;
            }
            let batch_started = Instant::now();
            // Min/max survive for the session until the user clears them.
            if reset.swap(false, Ordering::Relaxed) {
                temp_stats.clear();
                fan_bounds.clear();
                cpu_bounds = None;
                memory_bounds = None;
            }
            if sample_due(Some(last_frequency), batch_started, Duration::from_secs(2)) {
                system.refresh_cpu_all();
                last_frequency = batch_started;
            } else {
                system.refresh_cpu_usage();
            }
            // Process enumeration walks every PID and opens handles; on Windows
            // with 300+ processes it was the most expensive part of an idle
            // sample. Walk the table once at startup (so the first visit is
            // instant) and afterwards only while the process page is open.
            let wants_processes = process_wanted.load(Ordering::Relaxed);
            if last_processes.is_none()
                || (wants_processes
                    && (!was_process_wanted
                        || sample_due(last_processes, batch_started, Duration::from_secs(1))))
            {
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing().with_cpu().with_memory(),
                );
                process_cache = system
                    .processes()
                    .iter()
                    .map(|(pid, p)| Process {
                        pid: pid.as_u32(),
                        name: p.name().to_string_lossy().into_owned(),
                        cpu_usage: p.cpu_usage(),
                        memory: p.memory(),
                    })
                    .collect();
                last_processes = Some(batch_started);
            }
            was_process_wanted = wants_processes;
            system.refresh_memory();
            if sample_due(medium_at, batch_started, Duration::from_secs(2)) {
                disks.refresh(true);
                networks.refresh(true);
                let network_now = Instant::now();
                network_interval_seconds = network_now
                    .duration_since(network_refreshed_at)
                    .as_secs_f64();
                network_refreshed_at = network_now;
                mac_gpu_sample = mac_gpu_sources();
                medium_at = Some(batch_started);
            }
            // Built only when the platform sensor provider is missing: on
            // Windows this list costs a per-second WMI query and never supplies
            // a reading anyway.
            let mut components: Option<Components> = None;
            let mut nvidia = Vec::new();
            if let Some(nvml) = &nvml {
                if let Ok(count) = nvml.device_count() {
                    for i in 0..count {
                        if let Ok(device) = nvml.device_by_index(i) {
                            let name = device.name().unwrap_or_else(|_| format!("NVIDIA GPU {i}"));
                            let load = device
                                .utilization_rates()
                                .map(|v| format!("{}% load", v.gpu))
                                .unwrap_or_else(|_| "load unavailable".into());
                            let temp = device
                                .temperature(
                                    nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu,
                                )
                                .map(|v| format!("{v} °C"))
                                .unwrap_or_else(|_| "temperature unavailable".into());
                            let memory = device
                                .memory_info()
                                .map(|v| {
                                    format!(
                                        "{:.1} / {:.1} GiB VRAM",
                                        v.used as f64 / 1073741824.0,
                                        v.total as f64 / 1073741824.0
                                    )
                                })
                                .unwrap_or_else(|_| "VRAM unavailable".into());
                            let clock = device
                                .clock_info(nvml_wrapper::enum_wrappers::device::Clock::Graphics)
                                .map(|v| format!("{v} MHz graphics clock"))
                                .unwrap_or_else(|_| "graphics clock unavailable".into());
                            let power = device
                                .power_usage()
                                .map(|v| format!("{:.1} W", v as f64 / 1000.0))
                                .unwrap_or_else(|_| "power unavailable".into());
                            let fan = device
                                .fan_speed(0)
                                .map(|v| format!("{v}% intended fan 0 speed"))
                                .unwrap_or_else(|_| "fan 0 speed unavailable".into());
                            let driver = nvml
                                .sys_driver_version()
                                .map(|v| format!("driver {v}"))
                                .unwrap_or_else(|_| "driver version unavailable".into());
                            nvidia.push(Gpu { name, detail: format!("NVML device {i} · {load} · {temp} · {memory} · {clock} · {power} · {fan} · {driver}") });
                        }
                    }
                }
            }
            let cpu_usage = system.global_cpu_usage();
            let memory_total = system.total_memory();
            let memory_used = system.used_memory();
            let cores: Vec<f32> = system.cpus().iter().map(|c| c.cpu_usage()).collect();
            let core_mhz: Vec<u64> = system.cpus().iter().map(|c| c.frequency()).collect();
            let cpu_range = if cores.is_empty() {
                Range::default()
            } else {
                observe(&mut cpu_bounds, cpu_usage)
            };
            let memory_range = if memory_total == 0 {
                Range::default()
            } else {
                observe(
                    &mut memory_bounds,
                    memory_used as f32 * 100.0 / memory_total as f32,
                )
            };
            let readings = sensors.as_mut().map(|reader| reader.readings());
            let core_celsius = sensors
                .as_ref()
                .zip(readings.as_ref())
                .map_or_else(Vec::new, |(reader, readings)| {
                    reader.core_temperatures(readings, system.cpus())
                });
            let mut temperatures: Vec<Temperature> = Vec::new();
            let mut fans: Vec<Fan> = Vec::new();
            match readings {
                Some(readings) if !readings.temps.is_empty() || !readings.fans.is_empty() => {
                    for sensor in readings.temps {
                        temperatures.push(track_temperature(&mut temp_stats, sensor));
                    }
                    for sensor in readings.fans {
                        fans.push(track_fan(&mut fan_bounds, sensor));
                    }
                }
                _ => {
                    // sysinfo reports at most a comparison temperature; a sensor
                    // reporting 0 is treated as unavailable rather than cold.
                    let fallback =
                        components.get_or_insert_with(Components::new_with_refreshed_list);
                    let mut counts = HashMap::<String, usize>::new();
                    for component in fallback.list() {
                        *counts.entry(component.label().to_owned()).or_default() += 1;
                    }
                    for (index, component) in fallback.list().iter().enumerate() {
                        let celsius = component
                            .temperature()
                            .filter(|value| value.is_finite() && *value > 0.0);
                        let label = component.label().to_owned();
                        let duplicate = counts.get(&label).copied().unwrap_or(0) > 1;
                        let stats = if duplicate {
                            // Name-only keys cannot safely track multiple identical labels.
                            temp_stats.remove(&label);
                            None
                        } else {
                            temperature_stats(&mut temp_stats, &label, celsius)
                        };
                        temperatures.push(Temperature {
                            key: if duplicate {
                                format!("{label} · ambiguous #{index}")
                            } else {
                                label.clone()
                            },
                            label,
                            source: "sysinfo",
                            celsius,
                            min: stats.map(|value| value.min),
                            max: stats.map(|value| value.max),
                            average: stats.map(|value| value.mean as f32),
                            cpu_core: false,
                        });
                    }
                }
            }
            #[cfg(target_os = "windows")]
            let external_thermals = {
                let port = control
                    .as_ref()
                    .map_or(0, |c| c.lhm_port.load(Ordering::Relaxed));
                let (status, readings) = external_feed.sample(port);
                temperatures.extend(
                    readings
                        .into_iter()
                        .map(|sensor| track_temperature(&mut temp_stats, sensor)),
                );
                status
            };
            #[cfg(not(target_os = "windows"))]
            let external_thermals = ExternalThermals::Disabled;
            #[cfg(target_os = "windows")]
            let hwinfo_thermals = {
                let enabled = control
                    .as_ref()
                    .is_some_and(|c| c.hwinfo_gadget_enabled.load(Ordering::Relaxed));
                let (status, readings) = crate::sensors::hwinfo::sample(enabled);
                for sensor in readings {
                    match sensor.unit {
                        crate::sensors::Unit::Celsius => {
                            temperatures.push(track_temperature(&mut temp_stats, sensor))
                        }
                        crate::sensors::Unit::Rpm => fans.push(track_fan(&mut fan_bounds, sensor)),
                    }
                }
                status
            };
            #[cfg(not(target_os = "windows"))]
            let hwinfo_thermals = ExternalThermals::Disabled;
            let mut network_list: Vec<Network> = networks
                .iter()
                .map(|(name, data)| Network {
                    name: name.clone(),
                    received: data.received(),
                    transmitted: data.transmitted(),
                })
                .collect();
            network_list.sort_by(|a, b| a.name.cmp(&b.name));
            let mut gpus = merge_gpu_lists(adapter_inventory.clone(), nvidia);
            gpus.extend(mac_gpu_sample.iter().cloned());
            let snapshot = Snapshot {
                cpu_name: system
                    .cpus()
                    .first()
                    .map(|c| c.brand().to_owned())
                    .unwrap_or_else(|| "Unavailable".into()),
                gpu_backend: backend_label().to_owned(),
                cpu_usage,
                cores,
                core_mhz,
                core_celsius,
                memory_total,
                memory_used,
                memory_available: system.available_memory(),
                disks: disks
                    .list()
                    .iter()
                    .filter(|d| !is_synthetic_volume(&d.mount_point().to_string_lossy()))
                    .map(|d| {
                        let usage = d.usage();
                        Disk {
                            name: d.name().to_string_lossy().into_owned(),
                            mount: d.mount_point().to_string_lossy().into_owned(),
                            kind: format!("{:?}", d.kind()),
                            total: d.total_space(),
                            available: d.available_space(),
                            read: usage.read_bytes,
                            written: usage.written_bytes,
                        }
                    })
                    .collect(),
                networks: network_list,
                network_interval_seconds: (network_interval_seconds > 0.0)
                    .then_some(network_interval_seconds),
                processes: process_cache.clone(),
                gpus,
                adapters: adapters.clone(),
                temperatures,
                external_thermals,
                hwinfo_thermals,
                fans,
                cpu_range,
                memory_range,
                updated_at: Some(Instant::now()),
                ready: true,
            };
            publish(&shared, snapshot, &notify);
            // Measure the whole collection and sleep only for the remainder, so
            // a slow provider cannot turn into two stacked one-second polls.
            let period =
                Duration::from_millis(interval_ms.load(Ordering::Relaxed).clamp(250, 5000));
            if control.is_some() {
                thread::park_timeout(period.saturating_sub(batch_started.elapsed()));
            } else {
                while !stop.load(Ordering::Relaxed) {
                    let remaining = period.saturating_sub(batch_started.elapsed());
                    if remaining.is_zero() {
                        break;
                    }
                    thread::sleep(remaining.min(Duration::from_millis(25)));
                }
            }
        }
    })
}

fn sample_due(last: Option<Instant>, now: Instant, period: Duration) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= period)
}

fn publish(shared: &Mutex<Arc<Snapshot>>, snapshot: Snapshot, notify: impl FnOnce()) {
    if let Ok(mut out) = shared.lock() {
        *out = Arc::new(snapshot);
        drop(out);
        notify();
    }
}

#[derive(Clone, Copy)]
struct TemperatureStats {
    min: f32,
    max: f32,
    mean: f64,
    count: u64,
}

fn temperature_stats(
    session: &mut HashMap<String, TemperatureStats>,
    key: &str,
    current: Option<f32>,
) -> Option<TemperatureStats> {
    if let Some(value) = current.filter(|v| v.is_finite()) {
        let stats = session.entry(key.to_owned()).or_insert(TemperatureStats {
            min: value,
            max: value,
            mean: 0.0,
            count: 0,
        });
        stats.min = stats.min.min(value);
        stats.max = stats.max.max(value);
        stats.count = stats.count.saturating_add(1);
        stats.mean += (f64::from(value) - stats.mean) / stats.count as f64;
        Some(*stats)
    } else {
        session.get(key).copied()
    }
}

fn track_temperature(
    session: &mut HashMap<String, TemperatureStats>,
    sensor: Sensor,
) -> Temperature {
    let celsius = sensor.value.filter(|value| value.is_finite());
    let stats = temperature_stats(session, &sensor.key, celsius);
    Temperature {
        key: sensor.key,
        label: sensor.label,
        source: sensor.source,
        celsius,
        min: stats.map(|value| value.min),
        max: stats.map(|value| value.max),
        average: stats.map(|value| value.mean as f32),
        cpu_core: sensor.cpu_core,
    }
}

/// Tracks an all-time low/high for a named sensor. A missing sample never
/// becomes a zero, and never clears what was already observed.
fn track_fan(bounds: &mut HashMap<String, (f32, f32)>, sensor: Sensor) -> Fan {
    let (min, max) = note(bounds, &sensor.key, sensor.value);
    Fan {
        key: sensor.key,
        label: sensor.label,
        source: sensor.source,
        rpm: sensor.value,
        min,
        max,
    }
}

fn note(
    bounds: &mut HashMap<String, (f32, f32)>,
    key: &str,
    value: Option<f32>,
) -> (Option<f32>, Option<f32>) {
    match value {
        Some(value) => {
            let entry = bounds.entry(key.to_owned()).or_insert((value, value));
            entry.0 = entry.0.min(value);
            entry.1 = entry.1.max(value);
            (Some(entry.0), Some(entry.1))
        }
        None => (None, None),
    }
}

fn observe(bounds: &mut Option<(f32, f32)>, value: f32) -> Range {
    let entry = bounds.get_or_insert((value, value));
    entry.0 = entry.0.min(value);
    entry.1 = entry.1.max(value);
    Range {
        min: Some(entry.0),
        max: Some(entry.1),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires a hardware adapter and a working graphics driver"]
    fn discovered_hardware_adapter_opens_a_device() {
        let adapter = super::discover_adapters()
            .into_iter()
            .find(|adapter| !adapter.software)
            .expect("a hardware graphics adapter");
        println!("{} ({})", adapter.name, adapter.backend);
        pollster::block_on(
            adapter
                .handle
                .request_device(&wgpu::DeviceDescriptor::default(), None),
        )
        .expect("adapter remains usable after enumeration");
    }

    use super::*;

    #[test]
    fn core_channels_keep_separate_session_stats_through_gaps_and_reset() {
        let mut session = HashMap::new();
        let sample = |session: &mut _, suffix, value| {
            track_temperature(
                session,
                Sensor {
                    key: format!("lhm:/intelcpu/{suffix}/temperature/2"),
                    label: "Core #1".into(),
                    source: "LHM",
                    value,
                    unit: crate::sensors::Unit::Celsius,
                    cpu_core: true,
                },
            )
        };
        let _ = sample(&mut session, 0, Some(40.0));
        let _ = sample(&mut session, 0, Some(60.0));
        let _ = sample(&mut session, 1, Some(70.0));
        let first = sample(&mut session, 0, None);
        assert_eq!(first.celsius, None);
        assert_eq!(
            (first.min, first.max, first.average),
            (Some(40.0), Some(60.0), Some(50.0))
        );
        assert!(first.cpu_core);
        let second = sample(&mut session, 1, Some(f32::NAN));
        assert_eq!((second.celsius, second.average), (None, Some(70.0)));
        session.clear();
        let reset = sample(&mut session, 0, Some(42.0));
        assert_eq!(
            (reset.min, reset.max, reset.average),
            (Some(42.0), Some(42.0), Some(42.0))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "runs a bounded single-thread workload on each of the first two logical CPUs"]
    fn first_two_logical_processors_respond_under_affinity() {
        use std::{ffi::c_void, sync::mpsc};

        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentThread() -> *mut c_void;
            fn SetThreadAffinityMask(thread: *mut c_void, mask: usize) -> usize;
        }

        let mut system = System::new();
        system.refresh_cpu_all();
        assert!(system.cpus().len() >= 2);
        for index in 0..2 {
            let stop = Arc::new(AtomicBool::new(false));
            let (ready, wait) = mpsc::sync_channel(0);
            let worker_stop = Arc::clone(&stop);
            let worker = thread::spawn(move || {
                let pinned = unsafe { SetThreadAffinityMask(GetCurrentThread(), 1 << index) };
                if ready.send(pinned != 0).is_err() || pinned == 0 {
                    return;
                }
                let until = Instant::now() + Duration::from_secs(4);
                let mut value = 1_u64;
                while !worker_stop.load(Ordering::Relaxed) && Instant::now() < until {
                    value = std::hint::black_box(value.wrapping_mul(2_654_435_761).wrapping_add(1));
                }
            });
            assert!(wait.recv_timeout(Duration::from_secs(2)).unwrap());
            thread::sleep(Duration::from_millis(300));
            system.refresh_cpu_all();
            thread::sleep(Duration::from_secs(1));
            system.refresh_cpu_all();
            let load = system.cpus()[index].cpu_usage();
            stop.store(true, Ordering::Relaxed);
            worker.join().unwrap();
            println!("Logical processor {index} with one pinned worker: {load:.1}%");
            assert!(load >= 50.0, "logical processor {index} read {load:.1}%");
        }
    }

    #[test]
    fn publish_notifies_after_readers_can_access_the_new_snapshot() {
        let shared = Mutex::new(Arc::new(Snapshot::default()));
        let updated_at = Instant::now();
        let mut notified = false;
        publish(
            &shared,
            Snapshot {
                updated_at: Some(updated_at),
                ready: true,
                ..Snapshot::default()
            },
            || {
                let snapshot = shared.try_lock().expect("snapshot lock must be released");
                assert!(snapshot.ready);
                assert_eq!(snapshot.updated_at, Some(updated_at));
                notified = true;
            },
        );
        assert!(notified);
    }

    #[test]
    fn min_and_max_follow_samples_and_ignore_gaps() {
        let mut bounds = HashMap::new();
        assert_eq!(
            note(&mut bounds, "TC0P", Some(70.0)),
            (Some(70.0), Some(70.0))
        );
        assert_eq!(
            note(&mut bounds, "TC0P", Some(55.0)),
            (Some(55.0), Some(70.0))
        );
        assert_eq!(
            note(&mut bounds, "TC0P", Some(92.5)),
            (Some(55.0), Some(92.5))
        );
        // An unavailable sample keeps the previous range instead of resetting it.
        assert_eq!(note(&mut bounds, "TC0P", None), (None, None));
        assert_eq!(
            note(&mut bounds, "TC0P", Some(60.0)),
            (Some(55.0), Some(92.5))
        );
        // Clearing the map is what a user reset does.
        bounds.clear();
        assert_eq!(
            note(&mut bounds, "TC0P", Some(61.0)),
            (Some(61.0), Some(61.0))
        );
    }

    #[test]
    fn usage_range_accumulates_for_the_session() {
        let mut bounds = None;
        assert_eq!(observe(&mut bounds, 12.0).min, Some(12.0));
        let range = observe(&mut bounds, 4.0);
        assert_eq!((range.min, range.max), (Some(4.0), Some(12.0)));
        let range = observe(&mut bounds, 99.0);
        assert_eq!((range.min, range.max), (Some(4.0), Some(99.0)));
    }

    fn g(name: &str, detail: &str) -> Gpu {
        Gpu {
            name: name.into(),
            detail: detail.into(),
        }
    }
    #[test]
    fn ioreg_counters_are_unmapped_and_invalid_values_are_omitted() {
        let output = r#"+-o IntelAccelerator <class X>
  \"PerformanceStatistics\" = {\"Device Utilization %\"=11,\"Device Utilization % at cur p-state\"=43}
+-o Other <class X>
  \"PerformanceStatistics\" = {\"Device Utilization %\"=101}
"#.replace("\\\"", "\"");
        let sources = parse_ioreg_gpu(&output);
        assert_eq!(sources.len(), 1);
        assert!(sources[0].detail.contains("Device Utilization %: 11%"));
        assert!(sources[0].detail.contains("cur p-state: 43%"));
        assert!(sources[0].detail.contains("mapping unverified"));
    }
    #[test]
    fn unique_names_are_merged() {
        let r = merge_gpu_lists(vec![g("GPU A", "adapter")], vec![g("gpu a", "nvml")]);
        assert_eq!(r.len(), 1);
        assert!(r[0].detail.contains("nvml"));
    }
    #[test]
    fn unmatched_names_disclose_possible_overlap() {
        let result = merge_gpu_lists(
            vec![g("GPU A", "adapter")],
            vec![g("GPU A variant", "nvml")],
        );
        assert_eq!(result.len(), 2);
        assert!(result[1].detail.contains("may overlap"));
        assert!(result[1].name.starts_with("NVML sensor source:"));
    }
    #[test]
    fn identical_devices_are_not_collapsed_or_misattributed() {
        let r = merge_gpu_lists(
            vec![g("GPU A", "adapter 1"), g("GPU A", "adapter 2")],
            vec![g("GPU A", "nvml 1"), g("GPU A", "nvml 2")],
        );
        assert_eq!(r.len(), 4);
        assert!(r.iter().all(|v| v.detail.contains("ambiguous")));
    }
}
