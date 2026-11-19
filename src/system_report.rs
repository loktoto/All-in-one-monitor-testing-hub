//! CPU-Z/GPU-Z style system report export.
//!
//! Renders the same local inventory shown in Details plus a live sensor
//! snapshot. Text and JSON are written only to a path the user chooses and
//! existing files are never replaced. `redact` omits serials, paths and
//! connected-device names so the result is safer to share.

use crate::{report::Format, specs::SystemSpecs, telemetry::Snapshot};
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

struct Row {
    label: String,
    value: String,
}

fn row(label: impl Into<String>, value: impl Into<String>) -> Row {
    Row {
        label: label.into(),
        value: value.into(),
    }
}

fn text_rows(
    specs: &SystemSpecs,
    snapshot: &Snapshot,
    redact: bool,
) -> Vec<(&'static str, Vec<Row>)> {
    let value = |text: &str| {
        if text.trim().is_empty() {
            "unavailable".to_owned()
        } else {
            text.to_owned()
        }
    };
    let available = |number: Option<u64>| {
        number
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unavailable".into())
    };
    let cpu = &specs.cpu;

    let system = vec![
        row("Model", value(&specs.model_name)),
        row("Model identifier", value(&specs.model_id)),
        row(
            "OS",
            format!("{} {}", specs.os_name, specs.os_version)
                .trim()
                .to_owned(),
        ),
        row("Kernel", value(&specs.kernel)),
        row("Firmware", value(&specs.firmware)),
        row("Board vendor", value(&specs.board_vendor)),
        row("Board model", value(&specs.board_model)),
        row("Board version", value(&specs.board_version)),
        row("Chassis", value(&specs.chassis)),
        row(
            "Board identifier",
            if redact {
                "redacted".into()
            } else {
                value(specs.board_id.as_deref().unwrap_or(""))
            },
        ),
        row("Sources", value(&specs.sources)),
    ];

    let mut processor = vec![
        row("Name", value(&cpu.name)),
        row("Vendor", value(&cpu.vendor)),
        row("Architecture", value(&cpu.architecture)),
        row("Family / description", value(&cpu.description)),
        row("Socket", value(&cpu.socket)),
        row("Stepping", value(&cpu.stepping)),
        row(
            "Cores (physical / logical)",
            format!(
                "{} / {}",
                available(cpu.physical_cores.map(u64::from)),
                available(cpu.logical_cores.map(u64::from))
            ),
        ),
        row(
            "Clock (base / max)",
            format!(
                "{} / {} MHz",
                available(cpu.base_mhz),
                available(cpu.max_mhz)
            ),
        ),
        row(
            "Cache L1I / L1D / L2 / L3",
            [
                cpu.l1_instruction_bytes,
                cpu.l1_data_bytes,
                cpu.l2_bytes,
                cpu.l3_bytes,
            ]
            .iter()
            .map(|size| available(*size))
            .collect::<Vec<_>>()
            .join(" / "),
        ),
        row(
            "Clock at collection",
            format!("{} MHz", available(cpu.current_mhz)),
        ),
        row("Instruction sets", value(&cpu.instructions)),
        row(
            "Virtualization (firmware)",
            cpu.virtualization
                .map(|enabled| if enabled { "enabled" } else { "not enabled" }.to_owned())
                .unwrap_or_else(|| "unavailable".into()),
        ),
        row(
            "Processor ID",
            if redact {
                "redacted".into()
            } else {
                value(&cpu.processor_id)
            },
        ),
        row(
            "Live CPU load",
            if snapshot.cores.is_empty() {
                "unavailable".into()
            } else {
                format!(
                    "{:.0}% across {} logical cores",
                    snapshot.cpu_usage,
                    snapshot.cores.len()
                )
            },
        ),
    ];
    if let (Some(performance), Some(efficiency)) = (cpu.performance_cores, cpu.efficiency_cores) {
        processor.push(row(
            "Performance / efficiency cores",
            format!("{performance} / {efficiency}"),
        ));
    }

    let mut memory = vec![
        row(
            "Installed",
            format!("{} bytes", available(specs.memory.total_bytes)),
        ),
        row(
            "In use",
            if snapshot.memory_total > 0 {
                format!("{} / {} bytes", snapshot.memory_used, snapshot.memory_total)
            } else {
                "unavailable".into()
            },
        ),
        row(
            "Slots (used / total)",
            format!(
                "{} / {}",
                available(specs.memory.slots_used.map(u64::from)),
                available(specs.memory.slots_total.map(u64::from))
            ),
        ),
        row(
            "Maximum capacity",
            format!("{} bytes", available(specs.memory.max_capacity_bytes)),
        ),
        row(
            "ECC / upgradeable",
            format!(
                "{} / {}",
                specs
                    .memory
                    .ecc
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unavailable".into()),
                specs
                    .memory
                    .upgradeable
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unavailable".into())
            ),
        ),
    ];
    for module in &specs.memory.modules {
        let mut detail = format!(
            "{} · {} · {}",
            value(&module.slot),
            value(&module.kind),
            available(module.size_bytes)
        );
        detail.push_str(&format!(" · rated {}", available(module.speed_mhz)));
        if let Some(configured) = module.configured_mhz {
            detail.push_str(&format!(" · running {configured} MT/s"));
        }
        if !module.form_factor.is_empty() {
            detail.push_str(&format!(" · {}", module.form_factor));
        }
        if let Some(mv) = module.configured_voltage_mv {
            detail.push_str(&format!(" · {:.3} V", mv as f64 / 1000.0));
        }
        if let (Some(data), Some(total)) = (module.data_width, module.total_width) {
            detail.push_str(&format!(" · {data}/{total}-bit bus"));
        }
        if !module.manufacturer.is_empty() {
            detail.push_str(&format!(" · {}", module.manufacturer));
        }
        if !module.part_number.is_empty() {
            detail.push_str(&format!(" · part {}", module.part_number));
        }
        if !redact && !module.serial.is_empty() {
            detail.push_str(&format!(" · serial {}", module.serial));
        }
        memory.push(row("Module", detail));
    }

    let mut graphics: Vec<Row> = specs
        .graphics
        .iter()
        .map(|gpu| {
            let mut detail = format!(
                "{} · {} · {} · VRAM {} · bus {}",
                value(&gpu.name),
                value(&gpu.chipset),
                value(&gpu.device_id),
                available(gpu.vram_bytes),
                value(&gpu.bus)
            );
            if !gpu.driver.is_empty() {
                detail.push_str(&format!(" · driver {}", gpu.driver));
            }
            if !gpu.driver_date.is_empty() {
                detail.push_str(&format!(" ({})", gpu.driver_date));
            }
            if !gpu.bios.is_empty() {
                detail.push_str(&format!(" · BIOS {}", gpu.bios));
            }
            if !gpu.status.is_empty() {
                detail.push_str(&format!(" · {}", gpu.status));
            }
            row("Adapter", detail)
        })
        .collect();
    graphics.push(row("OS driver", value(&specs.gpu_driver)));
    graphics.push(row("Driver date", value(&specs.gpu_driver_date)));
    graphics.push(row("Display mode", value(&specs.display)));
    for gpu in &snapshot.gpus {
        graphics.push(row(
            "Sensor source",
            format!("{} · {}", gpu.name, gpu.detail),
        ));
    }

    let mut displays: Vec<Row> = specs
        .monitors
        .iter()
        .map(|monitor| {
            let mut detail = format!(
                "{} · {}",
                value(&monitor.manufacturer),
                value(&monitor.model)
            );
            if !redact && !monitor.serial.is_empty() {
                detail.push_str(&format!(" · serial {}", monitor.serial));
            }
            if let Some(year) = monitor.year {
                detail.push_str(&format!(" · {year}"));
                if let Some(week) = monitor.week {
                    detail.push_str(&format!(" week {week}"));
                }
            }
            row("Display", detail)
        })
        .collect();
    if displays.is_empty() {
        displays.push(row("Display", "unavailable"));
    }

    let mut drivers: Vec<Row> = specs
        .drivers
        .iter()
        .map(|driver| {
            let name = if redact {
                "redacted".to_owned()
            } else {
                value(&driver.name)
            };
            row(
                "Driver",
                format!(
                    "{} · {} · version {} · {} · {}",
                    name,
                    value(&driver.class_name),
                    value(&driver.version),
                    value(&driver.provider),
                    match driver.signed {
                        Some(true) => "signed",
                        Some(false) => "unsigned",
                        None => "signature unknown",
                    }
                ),
            )
        })
        .collect();
    if drivers.is_empty() {
        drivers.push(row("Driver", "unavailable"));
    }

    let mut storage: Vec<Row> = specs
        .drives
        .iter()
        .map(|drive| {
            let mut detail = format!(
                "{} · {} · {} · SMART {}",
                value(&drive.model),
                value(&drive.protocol),
                value(&drive.link),
                value(&drive.smart)
            );
            if !drive.firmware.is_empty() {
                detail.push_str(&format!(" · firmware {}", drive.firmware));
            }
            if !redact && !drive.serial.is_empty() {
                detail.push_str(&format!(" · serial {}", drive.serial));
            }
            row("Drive", detail)
        })
        .collect();
    for volume in &specs.volumes {
        storage.push(row(
            "Volume",
            format!(
                "{} · {} · {} · SMART {}",
                if redact {
                    "redacted".into()
                } else {
                    value(&volume.mount)
                },
                value(&volume.media),
                value(&volume.protocol),
                value(&volume.smart)
            ),
        ));
    }
    for disk in &snapshot.disks {
        storage.push(row(
            "Live volume",
            format!(
                "{} used / {} total bytes · read {} · written {}",
                disk.total.saturating_sub(disk.available),
                disk.total,
                disk.read,
                disk.written
            ),
        ));
    }

    let mut sensors: Vec<Row> = snapshot
        .temperatures
        .iter()
        .map(|t| {
            row(
                format!("{} ({})", t.label, t.key),
                format!(
                    "now {} · min {} · max {} °C",
                    t.celsius
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "unavailable".into()),
                    t.min
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "unavailable".into()),
                    t.max
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "unavailable".into())
                ),
            )
        })
        .collect();
    sensors.extend(snapshot.fans.iter().map(|f| {
        row(
            format!("{} ({})", f.label, f.key),
            format!(
                "now {} · min {} · max {} RPM",
                f.rpm
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "unavailable".into()),
                f.min
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "unavailable".into()),
                f.max
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "unavailable".into())
            ),
        )
    }));

    let mut connectivity = Vec::new();
    if !redact {
        if let Some(wifi) = &specs.wifi_interface {
            connectivity.push(row("Wi-Fi interface", wifi.clone()));
        }
        if let Some(bluetooth) = &specs.bluetooth {
            connectivity.push(row(
                "Bluetooth",
                format!(
                    "state {} · {} · {}",
                    bluetooth
                        .state
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unavailable".into()),
                    value(&bluetooth.transport),
                    value(&bluetooth.chipset)
                ),
            ));
            if let Some(devices) = &bluetooth.connected_devices {
                connectivity.push(row("Connected devices", devices.join(", ")));
            }
        }
    }
    for network in &snapshot.networks {
        connectivity.push(row(
            "Interface",
            format!(
                "{} · received {} · sent {} bytes",
                if redact {
                    "redacted".into()
                } else {
                    network.name.clone()
                },
                network.received,
                network.transmitted
            ),
        ));
    }

    let limitations = specs
        .unavailable
        .iter()
        .map(|item| row("Limit", item.clone()))
        .collect();

    vec![
        ("System", system),
        ("Processor", processor),
        ("Memory", memory),
        ("Graphics", graphics),
        ("Displays", displays),
        ("Drivers", drivers),
        ("Storage", storage),
        ("Sensors", sensors),
        ("Connectivity", connectivity),
        ("Disclosure and limits", limitations),
    ]
}

pub fn render(specs: &SystemSpecs, snapshot: &Snapshot, format: Format, redact: bool) -> Vec<u8> {
    let sections = text_rows(specs, snapshot, redact);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".into());
    match format {
        Format::Text => {
            let mut out = format!(
                "Hardware Hub system report · v{} · Unix {stamp}\n\n",
                env!("CARGO_PKG_VERSION")
            );
            for (heading, rows) in &sections {
                out.push_str(&format!("[{heading}]\n"));
                for item in rows {
                    out.push_str(&format!("  {}: {}\n", item.label, item.value));
                }
                out.push('\n');
            }
            out.into_bytes()
        }
        Format::Json => {
            let mut map = serde_json::Map::new();
            map.insert("schema_version".into(), serde_json::json!(1));
            map.insert(
                "app_version".into(),
                serde_json::json!(env!("CARGO_PKG_VERSION")),
            );
            map.insert("unix_time".into(), serde_json::json!(stamp));
            for (heading, rows) in &sections {
                let values: serde_json::Map<String, serde_json::Value> = rows
                    .iter()
                    .map(|item| (item.label.clone(), serde_json::json!(item.value)))
                    .collect();
                map.insert(heading.to_string(), serde_json::json!(values));
            }
            serde_json::to_vec_pretty(&serde_json::Value::Object(map))
                .unwrap_or_else(|_| b"{\"error\":\"serialisation failed\"}".to_vec())
        }
    }
}

pub fn export(
    path: &Path,
    specs: &SystemSpecs,
    snapshot: &Snapshot,
    format: Format,
    redact: bool,
) -> Result<(), String> {
    let data = render(specs, snapshot, format, redact);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create system report: {e}"))?;
    file.write_all(&data)
        .map_err(|e| format!("System report write failed; a partial file may remain: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_exports_specs_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!(
            "hub-system-report-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut specs = SystemSpecs::default();
        specs.cpu.name = "Test CPU".into();
        specs.cpu.socket = "LGA1700".into();
        let path = dir.join("system.json");
        export(&path, &specs, &Snapshot::default(), Format::Json, false).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["Processor"]["Name"], "Test CPU");
        assert_eq!(json["Processor"]["Socket"], "LGA1700");
        assert!(export(&path, &specs, &Snapshot::default(), Format::Text, false).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn redaction_removes_identifiers() {
        let mut specs = SystemSpecs::default();
        specs.volumes.push(crate::specs::VolumeHealth {
            mount: "C:\\Users\\loktoto".into(),
            smart: "Healthy".into(),
            ..Default::default()
        });
        specs.memory.modules.push(crate::specs::MemoryModule {
            serial: "ABCD-1234".into(),
            ..Default::default()
        });
        let text =
            String::from_utf8(render(&specs, &Snapshot::default(), Format::Text, true)).unwrap();
        assert!(!text.contains("ABCD-1234"));
        assert!(!text.contains("C:\\Users"));
        let plain =
            String::from_utf8(render(&specs, &Snapshot::default(), Format::Text, false)).unwrap();
        assert!(plain.contains("ABCD-1234"));
        assert!(plain.contains("C:\\Users"));
    }
}
