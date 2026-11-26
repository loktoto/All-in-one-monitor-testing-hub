//! Real-world Stress Lab: Prime95 / FurMark / MemTest / OCCT-inspired
//! 2-column live supervision cockpit with Timed (5m–24h / Custom), Continuous
//! (Until Stopped), and Pass-Count modes, full logical-core scaling, safe
//! percentage-based RAM sizing, and an in-place crimson Stop Check (Esc) CTA.

use crate::ui::{kind_label, theme::*, widgets::*, Page, STRESS_KINDS};
use crate::App;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Vec2};
use hardware_hub::{
    stress::{
        self, Config, CpuWorkload, DiskPattern, GpuIntensity, Kind, MemPattern, ReportContext,
        RunMode, Status, MAX_DISK_FILE_MIB, MAX_DISK_WRITE_CAP_MIB, MAX_TIMED_SECONDS,
    },
    telemetry::Snapshot,
};
use std::{path::PathBuf, sync::atomic::Ordering, time::Instant};

impl App {
    pub fn view_stress_lab(&mut self, ui: &mut egui::Ui, s: &Snapshot, fixed: Option<Kind>) {
        let running = self.state.lock().map(|st| st.running).unwrap_or(false);
        let logical_cores = s.cores.len().max(1);
        let safe_mem_max = stress::safe_max_memory_mib(s.memory_available);
        let narrow = self.compact_mode || ui.available_width() < 780.0;

        // Horizontal 5-mode segmented selector bar (standalone Stress Lab only)
        if fixed.is_none() {
            self.stress_mode_bar(ui, running);
            ui.add_space(10.0);
        }

        if narrow {
            self.stress_cockpit_panel(ui, s, fixed, running);
            ui.add_space(10.0);
            self.stress_config_panel(ui, s, fixed, running, logical_cores, safe_mem_max);
        } else {
            let avail = ui.available_width();
            let left_w = (avail * 0.39).clamp(300.0, 440.0);
            let gap = 12.0;
            let right_w = (avail - left_w - gap).max(320.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.stress_config_panel(
                            ui,
                            s,
                            fixed,
                            running,
                            logical_cores,
                            safe_mem_max,
                        );
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.stress_cockpit_panel(ui, s, fixed, running);
                    },
                );
            });
        }
        ui.add_space(8.0);
    }

    fn stress_mode_bar(&mut self, ui: &mut egui::Ui, running: bool) {
        compact_card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (kind, label, hint) in STRESS_KINDS {
                        let selected = self.kind == kind;
                        let btn = egui::Button::new(
                            RichText::new(label).size(11.5).strong().color(if selected {
                                ACCENT_INK
                            } else {
                                TEXT_DIM
                            }),
                        )
                        .fill(if selected { ACCENT } else { INPUT })
                        .stroke(Stroke::new(1.0_f32, if selected { ACCENT } else { BORDER }))
                        .corner_radius(CornerRadius::same(8))
                        .min_size(Vec2::new(0.0, 30.0));
                        let r = ui
                            .add(btn)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(hint);
                        if r.clicked() && !selected {
                            self.kind = kind;
                            if kind == Kind::Disk {
                                self.mib = self.mib.min(256);
                                self.disk_write_limit_mib =
                                    self.disk_write_limit_mib.max(self.mib as u64);
                            }
                        }
                    }
                });
            });
        });
    }

    fn stress_config_panel(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        fixed: Option<Kind>,
        running: bool,
        logical_cores: usize,
        safe_mem_max: usize,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section_title(
                    ui,
                    "Test Configuration",
                    Some(if fixed.is_some() { "this component" } else { "workload & duration" }),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if running {
                        chip(ui, "LOCKED", AMBER);
                    }
                });
            });
            ui.add_space(6.0);

            ui.add_enabled_ui(!running, |ui| {
                // Kind-specific controls
                match self.kind {
                    Kind::Cpu => {
                        self.cpu_worker_controls(ui, logical_cores);
                        ui.add_space(6.0);
                        picker(
                            ui,
                            "cpu-workload",
                            "Workload",
                            &mut self.cpu_workload,
                            &CpuWorkload::ALL.map(|w| (w, w.label())),
                        );
                    }
                    Kind::Gpu => {
                        if s.adapters.is_empty() {
                            empty_note(ui, "No GPU adapter discovered.");
                        } else {
                            self.gpu_index = self.gpu_index.min(s.adapters.len() - 1);
                            let selected = &s.adapters[self.gpu_index];
                            let label = format!(
                                "{} · {}{}",
                                truncate(&selected.name, 26),
                                selected.backend,
                                if selected.software { " · software" } else { "" }
                            );
                            egui::ComboBox::from_id_salt("gpu-adapter-lab")
                                .width(ui.available_width() - 4.0)
                                .selected_text(RichText::new(label).size(11.0))
                                .show_ui(ui, |ui| {
                                    for (i, a) in s.adapters.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.gpu_index,
                                            i,
                                            format!("{i}: {} · {}", a.name, a.backend),
                                        );
                                    }
                                });
                        }
                        ui.add_space(6.0);
                        picker(
                            ui,
                            "gpu-intensity-lab",
                            "Intensity",
                            &mut self.gpu_intensity,
                            &GpuIntensity::ALL.map(|v| (v, v.label())),
                        );
                    }
                    Kind::Memory => {
                        self.memory_size_controls(ui, s.memory_available, safe_mem_max);
                        ui.add_space(6.0);
                        picker(
                            ui,
                            "mem-pattern-lab",
                            "Pattern",
                            &mut self.mem_pattern,
                            &MemPattern::ALL.map(|v| (v, v.label())),
                        );
                    }
                    Kind::Disk => {
                        ui.label(
                            RichText::new("Creates & verifies an owned scratch file (causes SSD wear).")
                                .size(10.5)
                                .color(AMBER),
                        );
                        ui.add_space(4.0);
                        text_field(ui, &mut self.directory, "Scratch directory", f32::INFINITY);
                        ui.add_space(4.0);
                        let max_file = MAX_DISK_FILE_MIB;
                        self.mib = self.mib.clamp(1, max_file);
                        ui.add(egui::Slider::new(&mut self.mib, 1..=max_file).text("file MiB"));
                        self.disk_write_limit_mib =
                            self.disk_write_limit_mib.clamp(self.mib as u64, MAX_DISK_WRITE_CAP_MIB);
                        ui.add(
                            egui::Slider::new(
                                &mut self.disk_write_limit_mib,
                                self.mib as u64..=MAX_DISK_WRITE_CAP_MIB,
                            )
                            .logarithmic(true)
                            .text("write cap MiB"),
                        );
                        ui.add_space(4.0);
                        picker(
                            ui,
                            "disk-pattern-lab",
                            "Pattern",
                            &mut self.disk_pattern,
                            &DiskPattern::ALL.map(|v| (v, v.label())),
                        );
                    }
                    Kind::Combined => {
                        self.cpu_worker_controls(ui, logical_cores);
                        ui.add_space(4.0);
                        picker(
                            ui,
                            "comb-cpu-wl",
                            "CPU mode",
                            &mut self.cpu_workload,
                            &CpuWorkload::ALL.map(|w| (w, w.label())),
                        );
                        ui.add_space(6.0);
                        self.memory_size_controls(ui, s.memory_available, safe_mem_max);
                        ui.add_space(4.0);
                        picker(
                            ui,
                            "comb-mem-pat",
                            "RAM pattern",
                            &mut self.mem_pattern,
                            &MemPattern::ALL.map(|v| (v, v.label())),
                        );
                    }
                }

                ui.add_space(10.0);
                page_rule(ui);
                ui.add_space(8.0);

                // Run Mode selector: Timed | Continuous | Pass Count
                ui.label(RichText::new("RUN MODE").size(9.5).strong().color(MUTED));
                ui.add_space(4.0);
                segmented(
                    ui,
                    &mut self.stress_mode,
                    &[
                        (RunMode::Timed, "Timed"),
                        (RunMode::Continuous, "Continuous"),
                        (RunMode::Passes, "Pass count"),
                    ],
                );
                ui.add_space(6.0);

                match self.stress_mode {
                    RunMode::Timed => {
                        ui.label(RichText::new("REAL-WORLD DURATION PRESETS").size(9.5).strong().color(MUTED));
                        ui.add_space(3.0);
                        ui.horizontal_wrapped(|ui| {
                            for &(secs, lbl) in &[
                                (300u64, "5m"),
                                (900, "15m"),
                                (1800, "30m"),
                                (3600, "1h"),
                                (14_400, "4h"),
                                (28_800, "8h"),
                                (43_200, "12h"),
                                (86_400, "24h"),
                            ] {
                                let active = self.seconds == secs;
                                let btn = egui::Button::new(
                                    RichText::new(lbl)
                                        .monospace()
                                        .size(10.5)
                                        .color(if active { TEXT } else { TEXT_DIM }),
                                )
                                .fill(if active { ACCENT_SOFT } else { INPUT })
                                .stroke(Stroke::new(1.0_f32, if active { ACCENT } else { BORDER }))
                                .corner_radius(CornerRadius::same(6));
                                if ui.add(btn).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    self.seconds = secs;
                                }
                            }
                        });
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Custom:").size(10.5).color(MUTED));
                            ui.add(
                                egui::DragValue::new(&mut self.seconds)
                                    .range(1..=MAX_TIMED_SECONDS)
                                    .speed(30)
                                    .suffix(" s"),
                            );
                            ui.label(
                                RichText::new(format!("({})", stress::format_duration(self.seconds)))
                                    .monospace()
                                    .size(10.5)
                                    .color(CYAN),
                            );
                        });
                    }
                    RunMode::Continuous => {
                        ui.label(
                            RichText::new("Runs continuously with live error verification until you click Stop or press Esc.")
                                .size(11.0)
                                .color(TEXT_DIM),
                        );
                    }
                    RunMode::Passes => {
                        ui.horizontal_wrapped(|ui| {
                            for &p in &[1u64, 4, 10, 25, 100] {
                                let active = self.stress_passes == p;
                                let btn = egui::Button::new(
                                    RichText::new(format!("{p}x"))
                                        .monospace()
                                        .size(10.5)
                                        .color(if active { TEXT } else { TEXT_DIM }),
                                )
                                .fill(if active { ACCENT_SOFT } else { INPUT })
                                .stroke(Stroke::new(1.0_f32, if active { ACCENT } else { BORDER }))
                                .corner_radius(CornerRadius::same(6));
                                if ui.add(btn).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    self.stress_passes = p;
                                }
                            }
                        });
                        ui.add_space(4.0);
                        ui.add(
                            egui::Slider::new(&mut self.stress_passes, 1..=1000)
                                .logarithmic(true)
                                .text("target passes"),
                        );
                    }
                }
            });

            ui.add_space(8.0);
            ui.add_enabled_ui(!running, |ui| {
                toggle(ui, &mut self.auto_stop_on_heat, "Thermal auto-stop")
                    .on_hover_text("Stops this check after consecutive fresh readings above the set limit.");
                if self.auto_stop_on_heat {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Limit");
                        ui.add(egui::DragValue::new(&mut self.alert_celsius).range(30.0..=125.0).suffix(" °C"));
                        ui.label("Hold");
                        ui.add(egui::DragValue::new(&mut self.alert_hold_secs).range(1..=120).suffix(" s"));
                    });
                    if !s.temperatures.iter().any(|sensor| sensor.celsius.is_some()) {
                        ui.colored_label(AMBER, "Waiting for a temperature reading.");
                    }
                }
            });

            if fixed.is_some() {
                ui.add_space(10.0);
                self.stress_action(ui, s, running, true);
            }
        });
    }

    pub fn stress_action(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        running: bool,
        full_width: bool,
    ) {
        if running {
            let stopping = self.cancel.load(Ordering::Relaxed);
            let label = if stopping {
                "Stopping check…"
            } else {
                "■ Stop Check (Esc)"
            };
            if ui
                .add_enabled_ui(!stopping, |ui| danger_button(ui, label, full_width, 32.0))
                .inner
                .clicked()
            {
                self.cancel.store(true, Ordering::Relaxed);
                self.message = "Stopping after the current work or I/O operation…".into();
            }
        } else if primary_button(ui, "Start stability check", full_width, 32.0).clicked() {
            self.launch_stress_check(s);
            ui.ctx().request_repaint();
        }
    }

    fn cpu_worker_controls(&mut self, ui: &mut egui::Ui, logical_cores: usize) {
        self.workers = self.workers.clamp(1, logical_cores.max(1));
        let physical = (logical_cores / 2).max(1);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Threads:").size(11.0).color(MUTED));
            for &(n, lbl) in &[
                (1usize, "1T"),
                (physical, "Physical"),
                (logical_cores, "All Logical"),
            ] {
                let active = self.workers == n;
                let btn = egui::Button::new(RichText::new(lbl).size(10.5).color(if active {
                    TEXT
                } else {
                    TEXT_DIM
                }))
                .fill(if active { ACCENT_SOFT } else { INPUT })
                .stroke(Stroke::new(1.0_f32, if active { ACCENT } else { BORDER }))
                .corner_radius(CornerRadius::same(6));
                if ui
                    .add(btn)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.workers = n;
                }
            }
        });
        ui.add_space(3.0);
        ui.add(
            egui::Slider::new(&mut self.workers, 1..=logical_cores.max(1))
                .text(format!("/ {logical_cores} workers")),
        );
    }

    fn memory_size_controls(&mut self, ui: &mut egui::Ui, available_bytes: u64, safe_max: usize) {
        self.mib = self.mib.clamp(1, safe_max.max(1));
        let avail_mib = (available_bytes / (1024 * 1024)) as usize;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("RAM target:").size(11.0).color(MUTED));
            for &(pct, lbl) in &[(25usize, "25%"), (50, "50%"), (75, "75% Max")] {
                let target = ((avail_mib * pct) / 100).clamp(1, safe_max);
                let active = self.mib == target;
                let btn =
                    egui::Button::new(RichText::new(lbl).monospace().size(10.5).color(if active {
                        TEXT
                    } else {
                        TEXT_DIM
                    }))
                    .fill(if active { ACCENT_SOFT } else { INPUT })
                    .stroke(Stroke::new(1.0_f32, if active { ACCENT } else { BORDER }))
                    .corner_radius(CornerRadius::same(6));
                if ui
                    .add(btn)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.mib = target;
                }
            }
        });
        ui.add_space(3.0);
        ui.add(
            egui::Slider::new(&mut self.mib, 1..=safe_max.max(1))
                .logarithmic(safe_max > 512)
                .text(format!("MiB (safe max {safe_max})")),
        );
    }

    fn launch_stress_check(&mut self, s: &Snapshot) {
        self.run_peak_c = None;
        self.last_live_units = 0;
        self.last_live_at = Instant::now();
        self.rate_ema = 0.0;
        self.shown_progress = 0.0;
        let adapter = if self.kind == Kind::Gpu {
            s.adapters.get(self.gpu_index)
        } else {
            None
        };
        let gpu_name = adapter.map(|a| format!("{} · {}", a.name, a.backend));
        let cfg = Config {
            kind: self.kind,
            run_mode: self.stress_mode,
            seconds: self.seconds,
            target_passes: self.stress_passes,
            workers: self.workers,
            cpu_workload: self.cpu_workload,
            mib: self.mib,
            directory: PathBuf::from(&self.directory),
            gpu_adapter: gpu_name,
            disk_write_limit_mib: self.disk_write_limit_mib,
            mem_pattern: self.mem_pattern,
            disk_pattern: self.disk_pattern,
            gpu_intensity: self.gpu_intensity,
        };
        let context = ReportContext {
            cpu: s.cpu_name.clone(),
            logical_cpus: s.cores.len(),
            memory_total: s.memory_total,
            adapters: s
                .adapters
                .iter()
                .map(|a| format!("{} · {}", a.name, a.backend))
                .collect(),
            os: sysinfo::System::long_os_version().unwrap_or_else(|| "unavailable".into()),
            snapshot_age_seconds: s.updated_at.map(|t| t.elapsed().as_secs_f64()),
        };
        let result = if cfg.kind == Kind::Gpu {
            std::env::current_exe()
                .map_err(|e| format!("GPU worker path is unavailable: {e}"))
                .and_then(|executable| {
                    stress::start_gpu_in_worker(
                        cfg,
                        self.state.clone(),
                        self.cancel.clone(),
                        adapter.map(|a| a.handle.clone()),
                        context,
                        executable,
                    )
                })
        } else {
            stress::start(cfg, self.state.clone(), self.cancel.clone(), None, context)
        };
        self.message = match result {
            Ok(()) => {
                self.thermal_stop = Default::default();
                self.run_thermal_rule =
                    self.auto_stop_on_heat
                        .then_some(hardware_hub::alerts::Threshold {
                            celsius: self.alert_celsius,
                            hold: std::time::Duration::from_secs(self.alert_hold_secs),
                            severity: self.alert_severity,
                        });
                "Test started".into()
            }
            Err(e) => e,
        };
    }

    fn stress_cockpit_panel(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        fixed: Option<Kind>,
        running: bool,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if running {
                let heat_limit = self.run_thermal_rule.map_or(self.alert_celsius, |rule| rule.celsius);
                let (elapsed, active_cfg, live) = self
                    .state
                    .lock()
                    .ok()
                    .map(|st| {
                        (
                            st.started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0),
                            st.active_config.clone(),
                            st.live.clone(),
                        )
                    })
                    .unwrap_or_default();

                // Update EMA throughput
                let now = Instant::now();
                if live.units != self.last_live_units {
                    if self.last_live_units > 0 {
                        let dt = now.duration_since(self.last_live_at).as_secs_f64().max(1e-3);
                        let inst = (live.units.saturating_sub(self.last_live_units)) as f64 / dt;
                        self.rate_ema = if self.rate_ema <= 0.0 {
                            inst
                        } else {
                            self.rate_ema * 0.65 + inst * 0.35
                        };
                    }
                    self.last_live_units = live.units;
                    self.last_live_at = now;
                }
                let rate = self.rate_ema;

                // Header with unmistakable 0 ERRORS · VERIFIED badge
                ui.horizontal(|ui| {
                    section_title(ui, "Live Supervision Cockpit", Some(&live.phase));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match (live.errors, live.units) {
                            (0, 0) => pill(ui, "VERIFYING", MUTED),
                            (0, _) => pill(ui, "0 ERRORS", GREEN),
                            (errors, _) => pill(ui, &format!("{errors} ERRORS DETECTED"), RED),
                        }
                    });
                });
                ui.add_space(8.0);

                // Upper cockpit row: Circular Progress Gauge + 3 Tabular KPI Tiles
                let run_mode = active_cfg.as_ref().map(|c| c.run_mode).unwrap_or(self.stress_mode);
                let limit_secs = active_cfg.as_ref().map(|c| c.seconds).unwrap_or(self.seconds).max(1);
                let (ring_frac, ring_main, ring_sub) = match run_mode {
                    RunMode::Timed => {
                        let f = (elapsed / limit_secs as f32).clamp(0.0, 1.0);
                        let rem = (limit_secs as f32 - elapsed).max(0.0) as u64;
                        (Some(f), format!("{:.0}%", f * 100.0), format!("-{}", stress::format_duration(rem)))
                    }
                    RunMode::Continuous => {
                        (None, stress::format_duration(elapsed as u64), "CONT".into())
                    }
                    RunMode::Passes => {
                        let denom = active_cfg.as_ref().and_then(Config::pass_target_units).unwrap_or(1).max(1);
                        let f = (live.units as f32 / denom as f32).clamp(0.0, 1.0);
                        (Some(f), format!("{:.0}%", f * 100.0), format!("{}/{}", live.units, denom))
                    }
                };

                ui.horizontal(|ui| {
                    circular_progress(
                        ui,
                        "stress-ring",
                        ring_frac,
                        &ring_main,
                        &ring_sub,
                        86.0,
                        if live.errors > 0 { RED } else { ACCENT },
                    );
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.columns(3, |kpis| {
                            draw_kpi_box(
                                &mut kpis[0],
                                "WORK",
                                &live.units.to_string(),
                                live.unit_label,
                                ACCENT,
                            );
                            let rate_str = if rate >= 10.0 {
                                format!("{rate:.0}/s")
                            } else {
                                format!("{rate:.2}/s")
                            };
                            draw_kpi_box(&mut kpis[1], "RATE", &rate_str, &format!("{:.1}s elapsed", elapsed), CYAN);
                            let (peak_val, peak_sub, peak_col) = match self.run_peak_c {
                                Some(p) => {
                                    let (lbl, col) = thermal_safety(p, heat_limit);
                                    (format!("{p:.0} °C"), lbl.to_owned(), col)
                                }
                                None if s.temperatures.is_empty() => ("—".into(), "no sensor".into(), MUTED),
                                None => ("—".into(), "awaiting sample".into(), MUTED),
                            };
                            draw_kpi_box(&mut kpis[2], "PEAK TEMP", &peak_val, &peak_sub, peak_col);
                        });
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            mono_badge(ui, &format!("CPU {:.0}%", s.cpu_usage), load_color(s.cpu_usage / 100.0));
                            if s.memory_total > 0 {
                                mono_badge(
                                    ui,
                                    &format!("RAM {:.0}%", s.memory_used as f32 * 100.0 / s.memory_total as f32),
                                    VIOLET,
                                );
                            }
                            if let Some(rule) = self.run_thermal_rule {
                                let (text, color) = if self.run_peak_c.is_some() {
                                    (format!("AUTO-STOP {:.0} °C · {} s", rule.celsius, rule.hold.as_secs()), GREEN)
                                } else {
                                    ("AUTO-STOP · AWAITING SENSOR".into(), AMBER)
                                };
                                pill(ui, &text, color);
                            }
                        });
                    });
                });

                // Prime95-style per-worker health matrix when >1 worker
                if live.worker_ok.len() > 1 {
                    ui.add_space(8.0);
                    draw_worker_dots(ui, &live.worker_ok);
                }

                // Unclipped live temperature chart with dashed 85°C advisory limit
                ui.add_space(10.0);
                mini_temperature_graph(ui, &self.history, Some(heat_limit), 96.0);
            } else {
                // Idle state: show last run verdict or clean ready card
                let last = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|st| {
                        st.results
                            .iter()
                            .rev()
                            .find(|r| fixed.is_none_or(|kind| r.kind == kind))
                            .cloned()
                    });
                ui.horizontal(|ui| {
                    section_title(ui, "Live Supervision Cockpit", Some("ready"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        chip(ui, "IDLE", MUTED);
                    });
                });
                ui.add_space(8.0);
                match last {
                    Some(r) => {
                        let verdict = match r.status {
                            Status::Completed => "✓ PASSED",
                            Status::Stopped => "■ STOPPED",
                            Status::Failed => "■ FAILED",
                            Status::Inconclusive => "▲ INCONCLUSIVE",
                            Status::Unavailable => "— UNAVAILABLE",
                        };
                        ui.horizontal_wrapped(|ui| {
                            chip(ui, verdict, status_color(&r.status));
                            ui.label(RichText::new(kind_label(r.kind)).size(13.0).strong().color(TEXT));
                            ui.label(
                                RichText::new(format!("{:.1}s", r.elapsed))
                                    .monospace()
                                    .size(11.5)
                                    .color(MUTED),
                            );
                        });
                        ui.add_space(6.0);
                        ui.label(RichText::new(&r.detail).size(11.5).color(TEXT_DIM));
                        ui.label(
                            RichText::new(format!("{} · {}", timestamp_text(r.timestamp), config_summary(&r.config)))
                                .monospace()
                                .size(10.0)
                                .color(MUTED),
                        );
                        error_details(ui, &r.errors, r.error_count);
                        ui.add_space(8.0);
                        if ghost_button(ui, "View & Compare Reports →").clicked() {
                            self.page = Page::Reports;
                        }
                    }
                    None => {
                        empty_note(
                            ui,
                            "Choose a workload, thread count, and duration (5m–24h, Continuous, or Pass Count) on the left, then click Start.",
                        );
                    }
                }
                ui.add_space(10.0);
                mini_temperature_graph(ui, &self.history, Some(self.alert_celsius), 84.0);
                ui.add_space(8.0);
                callout(
                    ui,
                    ACCENT,
                    "Verified correctness stress bench",
                    "Supports full logical-core saturation, multi-hour or continuous runs, and exact mismatch capture. Advisory thermal auto-stop halts the software workload without modifying hardware fan curves or voltages.",
                );
            }
        });
    }
}

fn draw_kpi_box(ui: &mut egui::Ui, label: &str, value: &str, sub: &str, color: Color32) {
    let f = egui::Frame::NONE
        .fill(INPUT)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8));
    f.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(label).size(9.0).strong().color(MUTED));
        ui.add(
            egui::Label::new(
                RichText::new(value)
                    .monospace()
                    .size(15.0)
                    .strong()
                    .color(color),
            )
            .truncate(),
        );
        ui.add(egui::Label::new(RichText::new(sub).size(9.5).color(TEXT_DIM)).truncate());
    });
}

fn draw_worker_dots(ui: &mut egui::Ui, health: &[bool]) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("WORKERS ({})", health.len()))
                .size(9.5)
                .strong()
                .color(MUTED),
        );
        let shown = health.len().min(32);
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(shown as f32 * 11.0, 14.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            for (i, &ok) in health.iter().take(shown).enumerate() {
                let c = Pos2::new(rect.left() + i as f32 * 11.0 + 4.5, rect.center().y);
                let r = Rect::from_center_size(c, Vec2::splat(7.5));
                p.rect_filled(r, CornerRadius::same(2), if ok { GREEN } else { RED });
            }
        }
        if health.len() > shown {
            ui.label(
                RichText::new(format!("+{}", health.len() - shown))
                    .monospace()
                    .size(9.5)
                    .color(MUTED),
            );
        }
        resp.on_hover_text("Green = worker healthy · Red = computation mismatch detected");
    });
}
