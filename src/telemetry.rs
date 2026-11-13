use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use sysinfo::{Disks, System};

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
    pub ready: bool,
}

pub fn spawn(shared: Arc<Mutex<Snapshot>>, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut system = System::new_all();
        let mut disks = Disks::new_with_refreshed_list();
        // Adapter enumeration is discovery, not a GPU load or a test.
        let instance = wgpu::Instance::default();
        let adapters = instance.enumerate_adapters(wgpu::Backends::all());
        let gpu_names: Vec<String> = adapters
            .iter()
            .map(|a| {
                let info = a.get_info();
                format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend)
            })
            .collect();
        let nvml = nvml_wrapper::Nvml::init().ok();
        while !stop.load(Ordering::Relaxed) {
            system.refresh_cpu_all();
            system.refresh_memory();
            disks.refresh(true);
            let mut gpus: Vec<Gpu> = Vec::new();
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
                            gpus.push(Gpu {
                                name,
                                detail: format!("NVIDIA NVML · {load} · {temp} · {memory}"),
                            });
                        }
                    }
                }
            }
            for name in &gpu_names {
                // Avoid showing the NVML device twice; adapters can have multiple backend entries.
                if !gpus
                    .iter()
                    .any(|g| name.to_lowercase().contains(&g.name.to_lowercase()))
                    && !gpus.iter().any(|g| g.name == *name)
                {
                    gpus.push(Gpu {
                        name: name.clone(),
                        detail: "Adapter inventory · load, temperature and VRAM unavailable".into(),
                    });
                }
            }
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
                gpus,
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
