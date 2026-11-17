use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
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
#[derive(Clone)]
pub struct Temperature {
    /// Provider key, so an unfamiliar sensor stays traceable.
    pub key: String,
    pub label: String,
    pub celsius: Option<f32>,
    pub min: Option<f32>,
    pub max: Option<f32>,
}
#[derive(Clone)]
pub struct Fan {
    pub key: String,
    pub label: String,
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
fn discover_adapters() -> Vec<Adapter> {
    std::panic::catch_unwind(|| {
        let instance = wgpu::Instance::default();
        let preferred = preferred_backends();
        let mut found = instance.enumerate_adapters(preferred);
        if found.is_empty()
            || found
                .iter()
                .all(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
        {
            let fallback = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                instance.enumerate_adapters(wgpu::Backends::GL)
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
    std::process::Command::new("/usr/sbin/ioreg")
        .args(["-r", "-c", "IOAccelerator", "-d", "1"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_ioreg_gpu(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}
#[cfg(not(target_os = "macos"))]
fn mac_gpu_sources() -> Vec<Gpu> {
    Vec::new()
}

pub fn spawn(
    shared: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut system = System::new_all();
        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();
        let mut network_refreshed_at = Instant::now();
        let adapters = discover_adapters();
        // The SMC exposes many more sensors than sysinfo does; keep the reader
        // open so the enumeration cost is paid once, and fall back to sysinfo
        // when the platform has no provider.
        let mut sensors = crate::sensors::open();
        let mut temp_bounds: HashMap<String, (f32, f32)> = HashMap::new();
        let mut fan_bounds: HashMap<String, (f32, f32)> = HashMap::new();
        let mut cpu_bounds: Option<(f32, f32)> = None;
        let mut memory_bounds: Option<(f32, f32)> = None;
        thread::sleep(Duration::from_millis(250));
        let nvml = init_nvml();
        while !stop.load(Ordering::Relaxed) {
            // Min/max survive for the session until the user clears them.
            if reset.swap(false, Ordering::Relaxed) {
                temp_bounds.clear();
                fan_bounds.clear();
                cpu_bounds = None;
                memory_bounds = None;
            }
            system.refresh_cpu_all();
            system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
            system.refresh_memory();
            disks.refresh(true);
            networks.refresh(true);
            let network_now = Instant::now();
            let network_interval_seconds = network_now
                .duration_since(network_refreshed_at)
                .as_secs_f64();
            network_refreshed_at = network_now;
            // Recreate the list so a failed read cannot retain a prior value.
            let components = Components::new_with_refreshed_list();
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
            let adapter_gpus = adapters
                .iter()
                .enumerate()
                .map(|(i, a)| Gpu {
                    name: a.name.clone(),
                    detail: format!(
                        "Adapter {i} · {}{} · wgpu inventory",
                        a.backend,
                        if a.software { " · software" } else { "" }
                    ),
                })
                .collect();
            let cpu_usage = system.global_cpu_usage();
            if std::env::var("HH_CPU_LOG").is_ok() {
                let cores: Vec<String> = system
                    .cpus()
                    .iter()
                    .map(|c| format!("{:.0}", c.cpu_usage()))
                    .collect();
                eprintln!(
                    "[telemetry] global {:.1}% | cores {}",
                    cpu_usage,
                    cores.join(" ")
                );
            }
            let memory_total = system.total_memory();
            let memory_used = system.used_memory();
            let cores: Vec<f32> = system.cpus().iter().map(|c| c.cpu_usage()).collect();
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
            let mut temperatures: Vec<Temperature> = Vec::new();
            let mut fans: Vec<Fan> = Vec::new();
            match readings {
                Some(readings) if !readings.temps.is_empty() => {
                    for sensor in readings.temps {
                        let (min, max) = note(&mut temp_bounds, &sensor.key, sensor.value);
                        temperatures.push(Temperature {
                            key: sensor.key,
                            label: sensor.label,
                            celsius: sensor.value,
                            min,
                            max,
                        });
                    }
                    for sensor in readings.fans {
                        let (min, max) = note(&mut fan_bounds, &sensor.key, sensor.value);
                        fans.push(Fan {
                            key: sensor.key,
                            label: sensor.label,
                            rpm: sensor.value,
                            min,
                            max,
                        });
                    }
                }
                _ => {
                    // sysinfo reports at most a comparison temperature; a sensor
                    // reporting 0 is treated as unavailable rather than cold.
                    let mut counts = HashMap::<String, usize>::new();
                    for component in components.list() {
                        *counts.entry(component.label().to_owned()).or_default() += 1;
                    }
                    for (index, component) in components.list().iter().enumerate() {
                        let celsius = component
                            .temperature()
                            .filter(|value| value.is_finite() && *value > 0.0);
                        let label = component.label().to_owned();
                        let duplicate = counts.get(&label).copied().unwrap_or(0) > 1;
                        let (key, min, max) = if duplicate {
                            // Name-only keys cannot safely track multiple identical labels.
                            temp_bounds.remove(&label);
                            (format!("{label} · ambiguous #{index}"), None, None)
                        } else {
                            let (min, max) = note(&mut temp_bounds, &label, celsius);
                            (label.clone(), min, max)
                        };
                        temperatures.push(Temperature {
                            key,
                            label,
                            celsius,
                            min,
                            max,
                        });
                    }
                }
            }
            let mut network_list: Vec<Network> = networks
                .iter()
                .map(|(name, data)| Network {
                    name: name.clone(),
                    received: data.received(),
                    transmitted: data.transmitted(),
                })
                .collect();
            network_list.sort_by(|a, b| a.name.cmp(&b.name));
            let mut gpus = merge_gpu_lists(adapter_gpus, nvidia);
            gpus.extend(mac_gpu_sources());
            let snapshot = Snapshot {
                cpu_name: system
                    .cpus()
                    .first()
                    .map(|c| c.brand().to_owned())
                    .unwrap_or_else(|| "Unavailable".into()),
                gpu_backend: backend_label().to_owned(),
                cpu_usage,
                cores,
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
                processes: system
                    .processes()
                    .iter()
                    .map(|(pid, p)| Process {
                        pid: pid.as_u32(),
                        name: p.name().to_string_lossy().into_owned(),
                        cpu_usage: p.cpu_usage(),
                        memory: p.memory(),
                    })
                    .collect(),
                gpus,
                adapters: adapters.clone(),
                temperatures,
                fans,
                cpu_range,
                memory_range,
                updated_at: Some(Instant::now()),
                ready: true,
            };
            if let Ok(mut out) = shared.lock() {
                *out = snapshot;
            }
            for _ in 0..10 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    })
}
/// Tracks an all-time low/high for a named sensor. A missing sample never
/// becomes a zero, and never clears what was already observed.
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
    use super::*;

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
