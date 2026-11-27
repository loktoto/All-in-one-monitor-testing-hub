//! Hardware Dive workspace: CPU-Z, GPU-Z, CrystalDiskInfo, Memory DIMMs,
//! Motherboard/EDID Displays, Network, Bluetooth, Drivers, and System Summary.

use crate::ui::{kind_label, theme::*, widgets::*, Page};
use crate::App;
use eframe::egui::{self, CornerRadius, RichText, Stroke};
use egui_plot::{Line, Plot, PlotPoints};
use hardware_hub::{
    diskbench, history, netcheck, smart,
    specs::{self, SystemSpecs},
    stress::Kind,
    telemetry::Snapshot,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

impl App {
    pub fn view_hardware_page(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let specs: Option<Arc<SystemSpecs>> = self.specs.lock().ok().and_then(|held| held.clone());
        let page = self.page;
        let narrow = self.compact_mode || ui.available_width() < 740.0;

        match page {
            Page::Cpu => {
                self.view_core_temperatures(ui, snapshot);
                ui.add_space(10.0);
                self.view_cpu_dive(ui, snapshot, specs.as_deref(), narrow);
            }
            Page::Gpu => self.view_gpu_dive(ui, snapshot, specs.as_deref(), narrow),
            Page::Memory => self.view_memory_dive(ui, snapshot, specs.as_deref(), narrow),
            Page::Storage => {
                self.view_storage_dive(ui, snapshot, specs.as_deref());
                ui.add_space(8.0);
                self.storage_benchmark(ui, snapshot);
            }
            Page::Motherboard => self.view_motherboard_dive(ui, specs.as_deref()),
            Page::Drivers => self.view_drivers_dive(ui, specs.as_deref()),
            Page::Network => self.view_network_dive(ui, snapshot, specs.as_deref()),
            Page::Bluetooth => self.view_bluetooth_dive(ui, specs.as_deref()),
            _ => {}
        }

        let check = match page {
            Page::Cpu => Some(Kind::Cpu),
            Page::Gpu => Some(Kind::Gpu),
            Page::Memory => Some(Kind::Memory),
            Page::Storage => Some(Kind::Disk),
            _ => None,
        };
        if let Some(kind) = check {
            ui.add_space(10.0);
            let running = self.state.lock().map(|s| s.running).unwrap_or(false);
            if running && self.kind != kind {
                card_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "A {} check is already running. Only one check can run at a time.",
                            kind_label(self.kind)
                        ));
                        if ghost_button(ui, "Open Stress Lab").clicked() {
                            self.page = Page::Tests;
                        }
                    });
                });
            } else {
                if !running && self.kind != kind {
                    if kind == Kind::Disk {
                        self.mib = self.mib.min(64);
                        self.disk_write_limit_mib = self.disk_write_limit_mib.max(self.mib as u64);
                    }
                    self.kind = kind;
                }
                self.view_stress_lab(ui, snapshot, Some(kind));
            }
            if page == Page::Storage {
                ui.add_space(10.0);
                egui::CollapsingHeader::new("Import S.M.A.R.T. report (smartctl -a -j)")
                    .show(ui, |ui| self.smart_report_ui(ui));
            }
            self.recent_device_checks(ui, kind);
        }
    }

    fn view_cpu_dive(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        specs: Option<&SystemSpecs>,
        narrow: bool,
    ) {
        // Header identity pills (CPU-Z style)
        compact_card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let name = specs
                    .map(|sp| sp.cpu.name.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&snapshot.cpu_name);
                ui.label(RichText::new(name).size(14.5).strong().color(TEXT));
                if let Some(sp) = specs {
                    if !sp.cpu.architecture.is_empty() {
                        mono_badge(ui, &sp.cpu.architecture, CYAN);
                    }
                    if sp.cpu.logical_cores.is_some()
                        && sp
                            .cpu
                            .physical_cores
                            .is_some_and(|p| sp.cpu.logical_cores.unwrap_or(0) > p)
                    {
                        chip(ui, "SMT ENABLED", GREEN);
                    }
                    if let Some(true) = sp.cpu.virtualization {
                        chip(ui, "VT / VIRT", ACCENT);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(ui, "Export CPU-Z Report").clicked() {
                        self.page = Page::Reports;
                    }
                });
            });
        });
        ui.add_space(10.0);

        // Top Bento: Identity & Caches (left) + Liquid Core Topology (right)
        let draw_identity = |ui: &mut egui::Ui| {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Identity & Clocks", Some("CPU-Z style"));
                ui.add_space(6.0);
                match specs {
                    Some(sp) => {
                        let cpu = &sp.cpu;
                        let mut rows = Vec::new();
                        spec_row(&mut rows, "Vendor", vendor_text(&cpu.vendor));
                        spec_row(&mut rows, "Family / description", cpu.description.as_str());
                        spec_row(&mut rows, "Socket", cpu.socket.as_str());
                        spec_row(&mut rows, "Stepping", cpu.stepping.as_str());
                        match (cpu.physical_cores, cpu.logical_cores) {
                            (Some(p), Some(l)) => {
                                spec_row(&mut rows, "Cores / threads", format!("{p}P · {l}T"))
                            }
                            (None, Some(l)) => spec_row(&mut rows, "Threads", l.to_string()),
                            (Some(p), None) => spec_row(&mut rows, "Cores", p.to_string()),
                            (None, None) => {}
                        }
                        if let Some(p) = cpu.performance_cores {
                            spec_row(&mut rows, "Performance cores", p.to_string());
                        }
                        if let Some(e) = cpu.efficiency_cores {
                            spec_row(&mut rows, "Efficiency cores", e.to_string());
                        }
                        if let Some(mhz) = cpu.base_mhz {
                            spec_row(&mut rows, "Base clock", clock_text(mhz));
                        }
                        if let Some(mhz) = cpu.max_mhz {
                            spec_row(&mut rows, "Max clock", clock_text(mhz));
                        }
                        if let Some(mhz) = cpu.current_mhz {
                            spec_row(&mut rows, "Clock at collection", clock_text(mhz));
                        }
                        spec_row(
                            &mut rows,
                            "Live utilization",
                            if snapshot.cores.is_empty() {
                                "Unavailable".into()
                            } else {
                                format!("{:.1}%", snapshot.cpu_usage)
                            },
                        );
                        spec_row(&mut rows, "Processor ID", cpu.processor_id.as_str());
                        spec_grid(ui, "cpu-dive-id", &rows);

                        // Cache blocks row (L1 / L2 / L3)
                        let caches: Vec<(&str, u64)> = [
                            ("L1D", cpu.l1_data_bytes),
                            ("L1I", cpu.l1_instruction_bytes),
                            ("L2", cpu.l2_bytes),
                            ("L3", cpu.l3_bytes),
                        ]
                        .into_iter()
                        .filter_map(|(k, v)| v.map(|sz| (k, sz)))
                        .collect();
                        if !caches.is_empty() {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("CACHE HIERARCHY")
                                    .size(9.5)
                                    .strong()
                                    .color(MUTED),
                            );
                            ui.add_space(4.0);
                            ui.columns(caches.len(), |cols| {
                                for (i, (label, sz)) in caches.iter().enumerate() {
                                    let f = egui::Frame::NONE
                                        .fill(INPUT)
                                        .stroke(Stroke::new(1.0_f32, BORDER))
                                        .corner_radius(CornerRadius::same(7))
                                        .inner_margin(egui::Margin::same(8));
                                    f.show(&mut cols[i], |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new(*label).size(9.5).strong().color(MUTED),
                                        );
                                        ui.label(
                                            RichText::new(bytes(*sz))
                                                .monospace()
                                                .size(13.0)
                                                .strong()
                                                .color(TEXT),
                                        );
                                    });
                                }
                            });
                        }
                        if !cpu.instructions.is_empty() {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("INSTRUCTION SETS")
                                    .size(9.5)
                                    .strong()
                                    .color(MUTED),
                            );
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for flag in cpu
                                    .instructions
                                    .split([',', ' '])
                                    .filter(|s| !s.is_empty())
                                    .take(16)
                                {
                                    mono_badge(ui, flag.trim(), TEXT_DIM);
                                }
                            });
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("Reading processor inventory…")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        });
                    }
                }
            });
        };

        let draw_topology = |ui: &mut egui::Ui| {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                let hint = format!("{} logical processors", snapshot.cores.len());
                section_title(ui, "CPU Thread Activity", Some(&hint));
                ui.add_space(6.0);
                liquid_core_grid(ui, "cpu-dive-cores", snapshot, narrow);
                ui.add_space(10.0);
                egui::CollapsingHeader::new("Recent CPU history")
                    .default_open(!narrow)
                    .show(ui, |ui| {
                        Plot::new("cpu-dive-hist")
                            .height(105.0)
                            .include_y(0.0)
                            .include_y(100.0)
                            .allow_drag(false)
                            .allow_zoom(false)
                            .allow_scroll(false)
                            .show_x(false)
                            .show(ui, |plot| {
                                for pts in history::segments_with_gap(
                                    &self.history,
                                    self.started,
                                    self.history_gap(),
                                    |s| s.cpu,
                                ) {
                                    plot.line(
                                        Line::new(PlotPoints::from(pts))
                                            .color(ACCENT)
                                            .width(1.9_f32),
                                    );
                                }
                            });
                    });
            });
        };

        if narrow {
            draw_topology(ui);
            ui.add_space(10.0);
            draw_identity(ui);
        } else {
            ui.columns(2, |cols| {
                draw_identity(&mut cols[0]);
                draw_topology(&mut cols[1]);
            });
        }

        // Quick preview of Memory Slots & Storage right below on Desktop (Nano Banana layout)
        if let Some(sp) = specs {
            if !narrow && !sp.memory.modules.is_empty() {
                ui.add_space(10.0);
                card_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        section_title(
                            ui,
                            "Memory Slots Preview",
                            Some("click Memory for full DIMM specs"),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .link(RichText::new("Open Memory >").size(10.5).color(VIOLET))
                                .clicked()
                            {
                                self.page = Page::Memory;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    memory_slots_view(ui, &sp.memory, false);
                });
            }
        }
    }

    fn view_gpu_dive(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        specs: Option<&SystemSpecs>,
        _narrow: bool,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section_title(
                    ui,
                    "Graphics Adapters",
                    Some("GPU-Z style · wgpu + OS profile + NVML"),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    mono_badge(ui, &snapshot.gpu_backend, CYAN);
                });
            });
            ui.add_space(8.0);
            if snapshot.gpus.is_empty() {
                empty_note(ui, "No graphics adapter reported.");
            } else {
                for gpu in &snapshot.gpus {
                    let f = egui::Frame::NONE
                        .fill(INPUT)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .corner_radius(CornerRadius::same(9))
                        .inner_margin(egui::Margin::same(10));
                    f.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&gpu.name).size(12.5).strong().color(TEXT));
                            if gpu.detail.contains("software") {
                                chip(ui, "SOFTWARE", AMBER);
                            } else {
                                chip(ui, "HARDWARE", GREEN);
                            }
                        });
                        ui.label(
                            RichText::new(&gpu.detail)
                                .monospace()
                                .size(10.5)
                                .color(MUTED),
                        );
                    });
                    ui.add_space(6.0);
                }
            }

            if let Some(sp) = specs {
                for (index, gpu) in sp.graphics.iter().enumerate() {
                    ui.add_space(10.0);
                    section_title(
                        ui,
                        &format!("Adapter {} · {}", index + 1, gpu.name),
                        Some("OS driver & BIOS"),
                    );
                    let mut rows = Vec::new();
                    spec_row(&mut rows, "Vendor", gpu.vendor.as_str());
                    spec_row(&mut rows, "Chipset", gpu.chipset.as_str());
                    spec_row(&mut rows, "PCI vendor:device", gpu.device_id.as_str());
                    spec_row(&mut rows, "Bus location", gpu.bus.as_str());
                    if let Some(vram) = gpu.vram_bytes {
                        spec_row(&mut rows, "Dedicated memory", decimal_bytes(vram));
                    }
                    spec_row(&mut rows, "Driver", gpu.driver.as_str());
                    spec_row(&mut rows, "Driver date", gpu.driver_date.as_str());
                    spec_row(&mut rows, "Video BIOS", gpu.bios.as_str());
                    spec_row(&mut rows, "BIOS date", gpu.bios_date.as_str());
                    spec_row(&mut rows, "Display mode", gpu.current_mode.as_str());
                    spec_row(&mut rows, "Refresh rate", gpu.refresh.as_str());
                    spec_row(&mut rows, "Status", gpu.status.as_str());
                    spec_grid(ui, &format!("gpu-spec-{index}"), &rows);
                }
            }

            if !snapshot.adapters.is_empty() {
                ui.add_space(10.0);
                egui::CollapsingHeader::new("wgpu API Capabilities & Limits")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut rows = Vec::new();
                        for (index, adapter) in snapshot.adapters.iter().enumerate() {
                            let limits = adapter.handle.limits();
                            spec_row(&mut rows, format!("Adapter {index}"), &adapter.name);
                            spec_row(&mut rows, "Backend", &adapter.backend);
                            spec_row(
                                &mut rows,
                                "PCI Vendor / Device",
                                format!("{:#06x} / {:#06x}", adapter.vendor_id, adapter.device_id),
                            );
                            spec_row(
                                &mut rows,
                                "Max 2D texture",
                                format!("{} px", limits.max_texture_dimension_2d),
                            );
                            spec_row(
                                &mut rows,
                                "Max compute workgroup",
                                format!(
                                    "{} × {} × {}",
                                    limits.max_compute_workgroup_size_x,
                                    limits.max_compute_workgroup_size_y,
                                    limits.max_compute_workgroup_size_z
                                ),
                            );
                            spec_row(
                                &mut rows,
                                "Max storage buffer",
                                format!(
                                    "{} MiB",
                                    limits.max_storage_buffer_binding_size / (1024 * 1024)
                                ),
                            );
                            spec_row(
                                &mut rows,
                                "Optional features",
                                adapter.handle.features().bits().count_ones().to_string(),
                            );
                        }
                        spec_grid(ui, "gpu-caps", &rows);
                    });
            }
            ui.add_space(6.0);
            if ghost_button(ui, "Open Frame Analysis").clicked() {
                self.page = Page::Frames;
            }
        });
    }

    fn view_memory_dive(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        specs: Option<&SystemSpecs>,
        narrow: bool,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Memory Usage & DIMM Topology",
                Some("scalable 1–16+ slots & unified memory"),
            );
            ui.add_space(6.0);
            if snapshot.memory_total == 0 {
                empty_note(ui, "Memory usage unavailable.");
            } else {
                let frac = snapshot.memory_used as f32 / snapshot.memory_total as f32;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{:.1} / {:.1} GiB used ({:.0}%)",
                            gib(snapshot.memory_used),
                            gib(snapshot.memory_total),
                            frac * 100.0
                        ))
                        .monospace()
                        .size(14.0)
                        .strong()
                        .color(TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{:.1} GiB available",
                                gib(snapshot.memory_available)
                            ))
                            .monospace()
                            .size(11.5)
                            .color(MUTED),
                        );
                    });
                });
                ui.add_space(4.0);
                bar(ui, "mem-dive", frac, VIOLET, 7.0);
            }
            ui.add_space(10.0);
            match specs {
                Some(sp) => {
                    memory_slots_view(ui, &sp.memory, narrow);
                    if !sp.memory.modules.is_empty() {
                        ui.add_space(10.0);
                        egui::CollapsingHeader::new("Detailed Per-Slot Specifications")
                            .default_open(sp.memory.modules.len() <= 4)
                            .show(ui, |ui| {
                                for (index, module) in sp.memory.modules.iter().enumerate() {
                                    let heading = if module.slot.is_empty() {
                                        format!("Module {}", index + 1)
                                    } else {
                                        module.slot.clone()
                                    };
                                    let size =
                                        module.size_bytes.map(decimal_bytes).unwrap_or_default();
                                    section_title(
                                        ui,
                                        &heading,
                                        (!size.is_empty()).then_some(size.as_str()),
                                    );
                                    let mut rows = Vec::new();
                                    spec_row(&mut rows, "Type", module.kind.as_str());
                                    spec_row(&mut rows, "Form factor", module.form_factor.as_str());
                                    spec_row(&mut rows, "Bank", module.bank.as_str());
                                    if let Some(speed) = module.speed_mhz {
                                        spec_row(&mut rows, "Rated speed", format!("{speed} MT/s"));
                                    }
                                    if let Some(speed) = module.configured_mhz {
                                        spec_row(
                                            &mut rows,
                                            "Configured speed",
                                            format!("{speed} MT/s"),
                                        );
                                    }
                                    if let Some(mv) = module.configured_voltage_mv {
                                        spec_row(
                                            &mut rows,
                                            "Voltage",
                                            format!("{:.3} V", mv as f64 / 1000.0),
                                        );
                                    }
                                    if let (Some(data), Some(total)) =
                                        (module.data_width, module.total_width)
                                    {
                                        spec_row(
                                            &mut rows,
                                            "Bus width",
                                            if total > data {
                                                format!("{data}-bit data · {total}-bit total (ECC)")
                                            } else {
                                                format!("{data}-bit")
                                            },
                                        );
                                    }
                                    spec_row(
                                        &mut rows,
                                        "Manufacturer",
                                        module.manufacturer.as_str(),
                                    );
                                    spec_row(&mut rows, "Part number", module.part_number.as_str());
                                    spec_row(&mut rows, "Serial", module.serial.as_str());
                                    spec_grid(ui, &format!("mem-mod-{index}"), &rows);
                                    ui.add_space(6.0);
                                }
                            });
                    }
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Reading memory modules…")
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                }
            }
        });
    }

    fn storage_benchmark(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        compact_card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Drive benchmark", Some("Queue depth 1"));
            let progress = self.disk_benchmark.as_ref().map(diskbench::Run::progress);
            let running = progress.as_ref().is_some_and(|p| p.running);
            let stress_running = self.state.lock().is_ok_and(|s| s.running);
            let mounts: Vec<_> = snapshot.disks.iter().filter(|d| d.total > 0).collect();
            let temp = std::env::temp_dir();
            let temp_volume = mounts
                .iter()
                .filter(|d| temp.starts_with(&d.mount))
                .max_by_key(|d| d.mount.len());
            // An empty choice refers to the temporary folder on the current user's volume.
            if !self.disk_bench_mount.is_empty()
                && !mounts.iter().any(|d| d.mount == self.disk_bench_mount)
            {
                self.disk_bench_mount.clear();
            }
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("bench-volume")
                        .selected_text(if self.disk_bench_mount.is_empty() {
                            "System temp folder"
                        } else {
                            &self.disk_bench_mount
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.disk_bench_mount,
                                String::new(),
                                "System temp folder",
                            );
                            for volume in &mounts {
                                ui.selectable_value(
                                    &mut self.disk_bench_mount,
                                    volume.mount.clone(),
                                    &volume.mount,
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt("bench-size")
                        .selected_text(format!("{} MiB", self.disk_bench_mib))
                        .show_ui(ui, |ui| {
                            for size in diskbench::SIZES_MIB {
                                ui.selectable_value(
                                    &mut self.disk_bench_mib,
                                    size,
                                    format!("{size} MiB"),
                                );
                            }
                        });
                    #[cfg(windows)]
                    ui.checkbox(&mut self.disk_bench_direct, "Bypass OS file cache");
                });
            });
            if !self.disk_bench_direct {
                ui.label(
                    RichText::new("Buffered I/O. Cached reads can exceed device speed.")
                        .size(10.5)
                        .color(MUTED),
                );
            }
            let target = if self.disk_bench_mount.is_empty() {
                temp_volume
            } else {
                mounts.iter().find(|d| d.mount == self.disk_bench_mount)
            };
            let has_space = target
                .is_some_and(|d| d.available >= (self.disk_bench_mib as u64 + 64) * 1024 * 1024);
            ui.horizontal_wrapped(|ui| {
                if running {
                    if danger_button(ui, "Stop benchmark", false, 28.0).clicked() {
                        if let Some(bench) = &self.disk_benchmark {
                            bench.stop();
                        }
                    }
                } else if ui
                    .add_enabled(
                        !stress_running && has_space,
                        egui::Button::new("Run benchmark"),
                    )
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    let folder = if self.disk_bench_mount.is_empty() {
                        temp
                    } else {
                        self.disk_bench_mount.clone().into()
                    };
                    match diskbench::Run::start(
                        folder,
                        self.disk_bench_mib,
                        self.disk_bench_direct,
                        move || ctx.request_repaint(),
                    ) {
                        Ok(bench) => {
                            self.disk_benchmark = Some(bench);
                            self.disk_bench_message.clear();
                        }
                        Err(error) => self.disk_bench_message = error,
                    }
                }
                if stress_running {
                    ui.colored_label(AMBER, "A stability check is running.");
                } else if !has_space && !running {
                    ui.label("Choose a mounted volume with enough free space.");
                }
            });
            if !self.disk_bench_message.is_empty() {
                ui.colored_label(AMBER, &self.disk_bench_message);
            }
            if let (Some(bench), Some(progress)) = (&self.disk_benchmark, progress) {
                let mode = if bench.direct { "Direct" } else { "Buffered" };
                ui.label(format!(
                    "{} · {} MiB · {} · {}",
                    bench.folder.display(),
                    bench.mib,
                    mode,
                    progress.status
                ));
                let mut clipboard = format!(
                    "{}\t{} MiB\t{mode} QD1\t{}\nTest\tMB/s\tIOPS\tMean µs\n",
                    bench.folder.display(),
                    bench.mib,
                    progress.status
                );
                for row in &progress.rows {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(row.access.label());
                        ui.monospace(format!(
                            "{:.1} MB/s  ·  {:.0} IOPS  ·  {:.1} µs",
                            row.mbps, row.iops, row.mean_us
                        ));
                    });
                    use std::fmt::Write;
                    let _ = writeln!(
                        clipboard,
                        "{}\t{:.1}\t{:.0}\t{:.1}",
                        row.access.label(),
                        row.mbps,
                        row.iops,
                        row.mean_us
                    );
                }
                if !progress.rows.is_empty() && ghost_button(ui, "Copy benchmark results").clicked()
                {
                    ui.ctx().copy_text(clipboard);
                }
            }
        });
    }

    fn view_storage_dive(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        specs: Option<&SystemSpecs>,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Storage & Drive Health",
                Some("CrystalDiskInfo-style · volumes & physical drives"),
            );
            ui.add_space(8.0);
            if let Some(sp) = specs {
                if !sp.drives.is_empty() {
                    for (idx, drive) in sp.drives.iter().enumerate() {
                        let f = egui::Frame::NONE
                            .fill(INPUT)
                            .stroke(Stroke::new(1.0_f32, BORDER))
                            .corner_radius(CornerRadius::same(9))
                            .inner_margin(egui::Margin::same(10));
                        f.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(&drive.model).size(12.5).strong().color(TEXT),
                                );
                                if !drive.protocol.is_empty() {
                                    mono_badge(ui, &drive.protocol, CYAN);
                                }
                                if let Some(cap) = drive.capacity_bytes {
                                    mono_badge(ui, &decimal_bytes(cap), TEXT_DIM);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if !drive.smart.is_empty() {
                                            let col = smart_color(&drive.smart);
                                            chip(ui, &drive.smart.to_uppercase(), col);
                                        }
                                    },
                                );
                            });
                            let mut meta = Vec::new();
                            if !drive.firmware.is_empty() {
                                meta.push(format!("FW {}", drive.firmware));
                            }
                            if !drive.link.is_empty() {
                                meta.push(drive.link.clone());
                            }
                            if let Some(true) = drive.trim {
                                meta.push("TRIM".into());
                            }
                            if !meta.is_empty() {
                                ui.label(
                                    RichText::new(meta.join(" · "))
                                        .monospace()
                                        .size(10.5)
                                        .color(MUTED),
                                );
                            }
                        });
                        if idx + 1 < sp.drives.len() {
                            ui.add_space(6.0);
                        }
                    }
                    ui.add_space(10.0);
                }
            }

            section_title(ui, "Mounted Volumes", Some("OS capacity & live I/O"));
            if snapshot.disks.is_empty() {
                empty_note(ui, "No mounted volumes reported.");
            }
            for d in &snapshot.disks {
                let used = d.total.saturating_sub(d.available);
                let frac = if d.total > 0 {
                    used as f32 / d.total as f32
                } else {
                    0.0
                };
                let health = specs.and_then(|s| s.volume_health(&d.mount));
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(&d.mount)
                            .monospace()
                            .strong()
                            .size(12.0)
                            .color(TEXT),
                    );
                    mono_badge(ui, &d.kind.to_uppercase(), MUTED);
                    if let Some(h) = health {
                        if !h.smart.is_empty() {
                            chip(ui, &h.smart.to_uppercase(), smart_color(&h.smart));
                        }
                    }
                    ui.label(
                        RichText::new(format!(
                            "{:.1} / {:.1} GiB free · R {} · W {}",
                            gib(d.available),
                            gib(d.total),
                            bytes(d.read),
                            bytes(d.written)
                        ))
                        .monospace()
                        .size(10.5)
                        .color(TEXT_DIM),
                    );
                });
                bar(
                    ui,
                    &format!("sto-{}", d.mount),
                    frac,
                    if frac > 0.9 { AMBER } else { ACCENT },
                    5.0,
                );
                ui.add_space(6.0);
            }
        });
    }

    fn view_motherboard_dive(&mut self, ui: &mut egui::Ui, specs: Option<&SystemSpecs>) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Mainboard, BIOS & Displays",
                Some("read-only firmware & EDID"),
            );
            match specs {
                Some(specs) => {
                    let mut board_rows = Vec::new();
                    spec_row(&mut board_rows, "System", specs.model_name.as_str());
                    spec_row(&mut board_rows, "Model ID", specs.model_id.as_str());
                    spec_row(&mut board_rows, "Chassis", specs.chassis.as_str());
                    spec_row(&mut board_rows, "BIOS / UEFI", specs.firmware.as_str());
                    spec_row(&mut board_rows, "Board vendor", specs.board_vendor.as_str());
                    spec_row(&mut board_rows, "Board model", specs.board_model.as_str());
                    spec_row(
                        &mut board_rows,
                        "Board version",
                        specs.board_version.as_str(),
                    );
                    spec_row(
                        &mut board_rows,
                        "Board ID",
                        specs.board_id.as_deref().unwrap_or(""),
                    );
                    if let Some(bat) = &specs.battery {
                        spec_row(&mut board_rows, "Battery", bat.as_str());
                    }
                    spec_grid(ui, "mb-board", &board_rows);

                    ui.add_space(12.0);
                    section_title(ui, "Attached Displays (EDID)", None);
                    if specs.monitors.is_empty() {
                        empty_note(ui, "No EDID display identity reported by the OS.");
                    } else {
                        for (idx, m) in specs.monitors.iter().enumerate() {
                            let mut rows = Vec::new();
                            spec_row(&mut rows, "Manufacturer", m.manufacturer.as_str());
                            spec_row(&mut rows, "Model", m.model.as_str());
                            spec_row(&mut rows, "Serial", m.serial.as_str());
                            if let Some(year) = m.year {
                                let built = match m.week {
                                    Some(w) => format!("{year} · week {w}"),
                                    None => year.to_string(),
                                };
                                spec_row(&mut rows, "Manufactured", built);
                            }
                            spec_grid(ui, &format!("mb-mon-{idx}"), &rows);
                            ui.add_space(6.0);
                        }
                    }

                    ui.add_space(10.0);
                    section_title(ui, "Physical Network Adapters", None);
                    if specs.network_adapters.is_empty() {
                        empty_note(ui, "No physical network adapter reported.");
                    } else {
                        for a in &specs.network_adapters {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&a.name).strong().size(11.5).color(TEXT));
                                if a.status.eq_ignore_ascii_case("up") {
                                    chip(ui, "UP", GREEN);
                                }
                                ui.label(
                                    RichText::new(format!("{} · {}", a.description, a.speed))
                                        .size(10.5)
                                        .color(MUTED),
                                );
                            });
                        }
                    }
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Reading mainboard inventory…")
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                }
            }
        });
    }

    fn view_drivers_dive(&mut self, ui: &mut egui::Ui, specs: Option<&SystemSpecs>) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Installed Hardware Drivers",
                Some("virtualized · read-only OS driver store"),
            );
            match specs {
                Some(sp) if !sp.drivers.is_empty() => {
                    let mut classes: Vec<&str> =
                        sp.drivers.iter().map(|d| d.class_name.as_str()).collect();
                    classes.sort_unstable();
                    classes.dedup();
                    if !classes.iter().any(|c| *c == self.driver_class) {
                        self.driver_class = "All".into();
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        text_field(
                            ui,
                            &mut self.driver_query,
                            "Filter driver or provider…",
                            220.0,
                        );
                        egui::ComboBox::from_id_salt("driver-class")
                            .selected_text(if self.driver_class == "All" {
                                "All classes".to_owned()
                            } else {
                                driver_class_label(&self.driver_class).to_owned()
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.driver_class,
                                    "All".into(),
                                    "All classes",
                                );
                                for class in &classes {
                                    let cnt = sp
                                        .drivers
                                        .iter()
                                        .filter(|d| d.class_name == *class)
                                        .count();
                                    ui.selectable_value(
                                        &mut self.driver_class,
                                        (*class).to_owned(),
                                        format!("{} ({cnt})", driver_class_label(class)),
                                    );
                                }
                            });
                    });
                    let query = self.driver_query.to_lowercase();
                    let filtered: Vec<_> = sp
                        .drivers
                        .iter()
                        .filter(|d| {
                            (self.driver_class == "All" || d.class_name == self.driver_class)
                                && (query.is_empty()
                                    || d.name.to_lowercase().contains(&query)
                                    || d.provider.to_lowercase().contains(&query))
                        })
                        .collect();
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{} of {} drivers · {} classes",
                            filtered.len(),
                            sp.drivers.len(),
                            classes.len()
                        ))
                        .monospace()
                        .size(10.5)
                        .color(MUTED),
                    );
                    ui.add_space(6.0);
                    if filtered.is_empty() {
                        empty_note(ui, "No driver matches the current filter.");
                    } else {
                        driver_table(ui, &filtered);
                    }
                }
                Some(_) => empty_note(ui, "No driver records reported on this platform."),
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Reading OS driver store…")
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                }
            }
        });
    }

    fn view_network_dive(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        specs: Option<&SystemSpecs>,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Network Interfaces",
                Some("live OS throughput counters"),
            );
            if let Some(wifi) = specs.and_then(|s| s.wifi_interface.as_ref()) {
                chip(ui, &format!("WI-FI: {wifi}"), CYAN);
                ui.add_space(6.0);
            }
            if snapshot.networks.is_empty() {
                empty_note(ui, "No network interfaces reported.");
            }
            for n in &snapshot.networks {
                let rate = |b: u64| match (b, snapshot.network_interval_seconds) {
                    (0, _) => "0 B/s".into(),
                    (_, Some(sec)) if sec > 0.0 => format!("{:.1} KiB/s", b as f64 / sec / 1024.0),
                    _ => "—".into(),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&n.name).strong().size(11.5).color(TEXT));
                    ui.label(
                        RichText::new(format!(
                            "↓ {}   ↑ {}",
                            rate(n.received),
                            rate(n.transmitted)
                        ))
                        .monospace()
                        .size(11.0)
                        .color(TEXT_DIM),
                    );
                });
            }
        });
        ui.add_space(10.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "TCP Reachability Check",
                Some("optional · up to 4 connects · no payload"),
            );
            let state = self
                .network_check
                .lock()
                .map(|s| s.clone())
                .unwrap_or_default();
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                text_field_enabled(
                    ui,
                    !state.running,
                    &mut self.network_target,
                    "e.g. 127.0.0.1:80",
                    240.0,
                );
                let start = ui.add_enabled_ui(!state.running, |ui| {
                    primary_button(ui, "Start check", false, 28.0)
                });
                if start.inner.clicked() {
                    self.network_message = match netcheck::start(
                        &self.network_target,
                        self.network_check.clone(),
                        self.network_cancel.clone(),
                    ) {
                        Ok(()) => {
                            ui.ctx().request_repaint();
                            String::new()
                        }
                        Err(e) => e,
                    };
                }
                if state.running && danger_button(ui, "Stop", false, 28.0).clicked() {
                    self.network_cancel.store(true, Ordering::Relaxed);
                }
            });
            if !self.network_message.is_empty() {
                ui.colored_label(AMBER, &self.network_message);
            }
            if state.running {
                ui.ctx().request_repaint_after(Duration::from_millis(250));
                ui.label(format!("Checking… {} attempts", state.attempted));
            } else if state.attempted > 0 {
                ui.label(
                    RichText::new(format!(
                        "{}/{} succeeded{}",
                        state.succeeded,
                        state.attempted,
                        if state.stopped { " · stopped" } else { "" }
                    ))
                    .monospace()
                    .color(TEXT),
                );
                if state.succeeded > 0 {
                    ui.label(
                        RichText::new(format!(
                            "Mean TCP connect: {:.1} ms",
                            state.total_ms / state.succeeded as f64
                        ))
                        .monospace()
                        .color(CYAN),
                    );
                }
                if let Some(err) = state.last_error {
                    ui.weak(format!("Last error: {err}"));
                }
            }
        });
    }

    fn view_bluetooth_dive(&mut self, ui: &mut egui::Ui, specs: Option<&SystemSpecs>) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Bluetooth Controller", Some("read-only OS snapshot"));
            let (running, attempted, refreshed) = self
                .bluetooth_refresh
                .lock()
                .map(|s| (s.running, s.attempted, s.value.clone()))
                .unwrap_or_default();
            let info = if attempted {
                refreshed.as_ref()
            } else {
                specs.and_then(|s| s.bluetooth.as_ref())
            };
            ui.add_space(6.0);
            if let Some(info) = info {
                let mut rows = Vec::new();
                rows.push((
                    "Controller state".into(),
                    match info.state {
                        Some(true) => "On".into(),
                        Some(false) => "Off".into(),
                        None => "Unavailable".into(),
                    },
                ));
                spec_row(&mut rows, "Transport", info.transport.as_str());
                spec_row(&mut rows, "Chipset", info.chipset.as_str());
                spec_grid(ui, "bt-grid", &rows);
                if let Some(devices) = &info.connected_devices {
                    if !devices.is_empty() {
                        ui.add_space(6.0);
                        for dev in devices {
                            ui.label(RichText::new(format!("· {dev}")).size(11.5).color(TEXT_DIM));
                        }
                    }
                }
            } else {
                empty_note(ui, "Bluetooth controller not reported by the OS.");
            }
            ui.add_space(8.0);
            if ui
                .add_enabled(
                    !running,
                    egui::Button::new(if running {
                        "Checking…"
                    } else {
                        "Refresh Bluetooth"
                    }),
                )
                .clicked()
            {
                if let Ok(mut st) = self.bluetooth_refresh.lock() {
                    st.running = true;
                }
                let refresh = self.bluetooth_refresh.clone();
                let ctx = ui.ctx().clone();
                ctx.request_repaint();
                std::thread::spawn(move || {
                    let value = specs::bluetooth_status();
                    if let Ok(mut st) = refresh.lock() {
                        st.value = value;
                        st.attempted = true;
                        st.running = false;
                    }
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                });
            }
        });
    }

    // ---------------------------------------------------------------- System Summary (Details)
    pub fn view_details(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let specs: Option<Arc<SystemSpecs>> = self.specs.lock().ok().and_then(|held| held.clone());
        let Some(specs) = specs else {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Reading local system inventory…").color(TEXT_DIM));
                });
            });
            return;
        };

        let narrow = self.compact_mode || ui.available_width() < 740.0;
        let mut sys_rows = Vec::new();
        spec_row(&mut sys_rows, "Model", specs.model_name.as_str());
        spec_row(&mut sys_rows, "Model ID", specs.model_id.as_str());
        spec_row(
            &mut sys_rows,
            "OS",
            format!("{} {}", specs.os_name, specs.os_version),
        );
        spec_row(&mut sys_rows, "Kernel", specs.kernel.as_str());
        spec_row(&mut sys_rows, "Firmware", specs.firmware.as_str());
        spec_row(
            &mut sys_rows,
            "Uptime",
            uptime_text(sysinfo::System::uptime()),
        );
        if let Some(total) = specs.memory.total_bytes {
            spec_row(&mut sys_rows, "Memory", decimal_bytes(total));
        }
        if let Some(bat) = &specs.battery {
            spec_row(&mut sys_rows, "Battery", bat.as_str());
        }

        let mut cpu_rows = Vec::new();
        spec_row(&mut cpu_rows, "Processor", specs.cpu.name.as_str());
        spec_row(&mut cpu_rows, "Vendor", vendor_text(&specs.cpu.vendor));
        spec_row(
            &mut cpu_rows,
            "Architecture",
            specs.cpu.architecture.as_str(),
        );
        spec_row(&mut cpu_rows, "Socket", specs.cpu.socket.as_str());
        match (specs.cpu.physical_cores, specs.cpu.logical_cores) {
            (Some(p), Some(l)) => spec_row(
                &mut cpu_rows,
                "Cores",
                format!("{p} physical · {l} logical"),
            ),
            (None, Some(l)) => spec_row(&mut cpu_rows, "Threads", l.to_string()),
            _ => {}
        }
        if let Some(mhz) = specs.cpu.base_mhz {
            spec_row(&mut cpu_rows, "Base clock", clock_text(mhz));
        }
        if let Some(v) = specs.cpu.l2_bytes {
            spec_row(&mut cpu_rows, "L2 cache", bytes(v));
        }
        if let Some(v) = specs.cpu.l3_bytes {
            spec_row(&mut cpu_rows, "L3 cache", bytes(v));
        }

        if narrow {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "System", None);
                spec_grid(ui, "det-sys", &sys_rows);
            });
            ui.add_space(10.0);
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Processor", None);
                spec_grid(ui, "det-cpu", &cpu_rows);
            });
        } else {
            ui.columns(2, |cols| {
                card_frame().show(&mut cols[0], |ui| {
                    ui.set_width(ui.available_width());
                    section_title(ui, "System", Some("OS & firmware"));
                    spec_grid(ui, "det-sys", &sys_rows);
                });
                card_frame().show(&mut cols[1], |ui| {
                    ui.set_width(ui.available_width());
                    section_title(ui, "Processor", Some("CPU-Z summary"));
                    spec_grid(ui, "det-cpu", &cpu_rows);
                });
            });
        }

        ui.add_space(10.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Memory Modules", Some("installed slots"));
            ui.add_space(6.0);
            memory_slots_view(ui, &specs.memory, narrow);
        });

        ui.add_space(10.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Graphics & Storage", None);
            let mut rows = Vec::new();
            for (i, a) in s.adapters.iter().enumerate() {
                spec_row(
                    &mut rows,
                    format!("GPU {i}"),
                    format!("{} ({})", a.name, a.backend),
                );
            }
            for (i, d) in specs.drives.iter().enumerate() {
                spec_row(
                    &mut rows,
                    format!("Drive {}", i + 1),
                    format!(
                        "{} · {}",
                        d.model,
                        if d.smart.is_empty() { "—" } else { &d.smart }
                    ),
                );
            }
            spec_grid(ui, "det-gs", &rows);
        });

        ui.add_space(10.0);
        self.smart_report_ui(ui);

        if !specs.unavailable.is_empty() {
            ui.add_space(10.0);
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::CollapsingHeader::new(format!(
                    "Platform Limits ({} notes)",
                    specs.unavailable.len()
                ))
                .show(ui, |ui| {
                    for note in &specs.unavailable {
                        ui.label(RichText::new(format!("• {note}")).size(10.5).color(MUTED));
                    }
                });
            });
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("Sources: {}", specs.sources))
                .size(10.0)
                .color(MUTED),
        );
    }

    pub fn smart_report_ui(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Import S.M.A.R.T. Report", Some("smartctl -a -j JSON"));
            ui.horizontal_wrapped(|ui| {
                text_field(ui, &mut self.smart_path, "Path to smartctl JSON", 340.0);
                if ghost_button(ui, "Open report").clicked() {
                    match smart::read(std::path::Path::new(&self.smart_path)) {
                        Ok(report) => {
                            self.smart_report = Some(report);
                            self.smart_message.clear();
                        }
                        Err(error) => {
                            self.smart_report = None;
                            self.smart_message = error;
                        }
                    }
                }
                if self.smart_report.is_some() && ghost_button(ui, "Clear").clicked() {
                    self.smart_report = None;
                }
            });
            if !self.smart_message.is_empty() {
                ui.colored_label(AMBER, &self.smart_message);
            }
            if let Some(report) = &self.smart_report {
                ui.add_space(6.0);
                ui.strong(format!("Imported · {}", report.model));
                if !report.nvme.is_empty() {
                    egui::Grid::new("smart_nvme").striped(true).show(ui, |ui| {
                        for (k, v) in &report.nvme {
                            ui.label(k);
                            ui.label(RichText::new(v).monospace());
                            ui.end_row();
                        }
                    });
                }
                if !report.attributes.is_empty() {
                    egui::ScrollArea::horizontal()
                        .id_salt("smart_ata_h")
                        .show(ui, |ui| {
                            egui::Grid::new("smart_ata").striped(true).show(ui, |ui| {
                                for h in ["ID", "Name", "Current", "Worst", "Thresh", "Raw"] {
                                    ui.strong(h);
                                }
                                ui.end_row();
                                for a in &report.attributes {
                                    for v in
                                        [&a.id, &a.name, &a.value, &a.worst, &a.threshold, &a.raw]
                                    {
                                        ui.label(RichText::new(v).monospace().size(10.5));
                                    }
                                    ui.end_row();
                                }
                            });
                        });
                }
            }
        });
    }

    fn recent_device_checks(&mut self, ui: &mut egui::Ui, kind: Kind) {
        ui.add_space(10.0);
        let records = self
            .state
            .lock()
            .map(|s| {
                s.results
                    .iter()
                    .filter(|r| r.kind == kind)
                    .rev()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section_title(ui, "Recent Checks", Some("this session"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(ui, "All Reports").clicked() {
                        self.page = Page::Reports;
                    }
                });
            });
            if records.is_empty() {
                empty_note(ui, "No checks run for this component in this session.");
            }
            for r in records {
                ui.horizontal_wrapped(|ui| {
                    chip(
                        ui,
                        &r.status.to_string().to_uppercase(),
                        status_color(&r.status),
                    );
                    ui.label(
                        RichText::new(format!("{:.1}s · {}", r.elapsed, r.detail))
                            .size(11.0)
                            .color(TEXT_DIM),
                    );
                });
            }
        });
    }
}
