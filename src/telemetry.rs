use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use sysinfo::{Components, Disks, System};

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
}
#[derive(Clone)]
pub struct Temperature {
    pub label: String,
    pub celsius: Option<f32>,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub cpu_name: String,
    pub cpu_usage: f32,
    pub cores: Vec<f32>,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_available: u64,
    pub disks: Vec<Disk>,
    pub gpus: Vec<Gpu>,
    pub adapters: Vec<Adapter>,
    pub temperatures: Vec<Temperature>,
    pub updated_at: Option<Instant>,
    pub ready: bool,
}

// A single platform-preferred backend prevents the same GPU appearing once per API.
// Adapter handles are retained and reused for test selection; identical models stay distinct.
fn discover_adapters() -> Vec<Adapter> {
    std::panic::catch_unwind(|| {
        let instance = wgpu::Instance::default();
        #[cfg(target_os = "windows")]
        let preferred = wgpu::Backends::DX12;
        #[cfg(not(target_os = "windows"))]
        let preferred = wgpu::Backends::VULKAN;
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
        }
        merged.push(n);
    }
    merged
}

pub fn spawn(shared: Arc<Mutex<Snapshot>>, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut system = System::new_all();
        let mut disks = Disks::new_with_refreshed_list();
        let adapters = discover_adapters();
        thread::sleep(Duration::from_millis(250));
        let nvml = nvml_wrapper::Nvml::init().ok();
        while !stop.load(Ordering::Relaxed) {
            system.refresh_cpu_all();
            system.refresh_memory();
            disks.refresh(true);
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
                            nvidia.push(Gpu {
                                name,
                                detail: format!("NVML device {i} · {load} · {temp} · {memory}"),
                            });
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
            let snapshot = Snapshot {
                cpu_name: system
                    .cpus()
                    .first()
                    .map(|c| c.brand().to_owned())
                    .unwrap_or_else(|| "Unavailable".into()),
                cpu_usage: system.global_cpu_usage(),
                cores: system.cpus().iter().map(|c| c.cpu_usage()).collect(),
                memory_total: system.total_memory(),
                memory_used: system.used_memory(),
                memory_available: system.available_memory(),
                disks: disks
                    .list()
                    .iter()
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
                gpus: merge_gpu_lists(adapter_gpus, nvidia),
                adapters: adapters.clone(),
                temperatures: components
                    .list()
                    .iter()
                    .map(|c| Temperature {
                        label: c.label().to_owned(),
                        celsius: c.temperature().filter(|v| v.is_finite()),
                    })
                    .collect(),
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
#[cfg(test)]
mod tests {
    use super::*;
    fn g(name: &str, detail: &str) -> Gpu {
        Gpu {
            name: name.into(),
            detail: detail.into(),
        }
    }
    #[test]
    fn unique_names_are_merged() {
        let r = merge_gpu_lists(vec![g("GPU A", "adapter")], vec![g("gpu a", "nvml")]);
        assert_eq!(r.len(), 1);
        assert!(r[0].detail.contains("nvml"));
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
