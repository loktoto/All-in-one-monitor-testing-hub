//! Live Monitor workspace: modern HWMonitor-style sensor tree, liquid CPU core
//! tiles, slide-in live sensor graph inspector, and Thermals / Alerts view.

use crate::ui::{theme::*, widgets::*, Page};
use crate::App;
use eframe::egui::{self, RichText, Vec2};
use egui_plot::{HLine, Line, Plot, PlotPoints};
use hardware_hub::{
    alerts::{Severity, Threshold},
    history,
    logging::Recorder,
    specs::SystemSpecs,
    telemetry::Snapshot,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

impl App {
    pub fn view_overview(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let specs: Option<Arc<SystemSpecs>> = self.specs.lock().ok().and_then(|held| held.clone());
        if !s.ready {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Collecting initial hardware readings…").color(TEXT_DIM),
                    );
                });
                ui.label(
                    RichText::new(
                        "First sample enumerates platform sensors and graphics adapters.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            });
            return;
        }

        let avail_w = ui.available_width();
        let narrow = self.compact_mode || avail_w < 680.0;

        let memory_fraction = if s.memory_total > 0 {
            s.memory_used as f32 / s.memory_total as f32
        } else {
            0.0
        };
        let peak_celsius = s
            .temperatures
            .iter()
            .filter_map(|t| t.celsius)
            .fold(None::<f32>, |peak, value| {
                Some(peak.map_or(value, |previous: f32| previous.max(value)))
            });
        let smart = specs
            .as_ref()
            .and_then(|known| {
                known
                    .volumes
                    .iter()
                    .map(|v| v.smart.as_str())
                    .find(|v| !v.is_empty())
                    .or_else(|| {
                        known
                            .drives
                            .iter()
                            .map(|d| d.smart.as_str())
                            .find(|v| !v.is_empty())
                    })
            })
            .unwrap_or("");

        // -------------------------------------------------------- top KPI glance strip
        if narrow {
            self.compact_glance_strip(ui, s, memory_fraction, peak_celsius, smart);
        } else {
            self.expanded_kpi_row(ui, s, memory_fraction, peak_celsius, smart);
        }
        ui.add_space(10.0);

        // -------------------------------------------------------- main HWMonitor split
        // In expanded mode (>= 940px), show HWMonitor Sensor Tree on the left (62%)
        // and Live Trend Inspector + Recent Activity on the right (38%).
        if !narrow && avail_w >= 920.0 {
            let left_w = (avail_w * 0.60).floor();
            let gap = 12.0;
            let right_w = (avail_w - left_w - gap).max(260.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.hwmonitor_tree_card(ui, s, specs.as_deref(), false);
                        ui.add_space(10.0);
                        self.liquid_cores_card(ui, s, specs.as_deref(), false);
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.sensor_inspector_card(ui, s);
                        ui.add_space(10.0);
                        self.recent_activity_card(ui, true);
                        ui.add_space(10.0);
                        self.storage_snapshot_card(ui, s, specs.as_deref());
                    },
                );
            });
        } else {
            // Compact HWMonitor vertical flow: sensor tree first, inline plot on click,
            // then liquid core tiles and recent activity.
            self.hwmonitor_tree_card(ui, s, specs.as_deref(), narrow);
            if !self.selected_temp_key.is_empty() {
                ui.add_space(8.0);
                self.sensor_inspector_card(ui, s);
            }
            ui.add_space(8.0);
            self.liquid_cores_card(ui, s, specs.as_deref(), narrow);
            ui.add_space(8.0);
            self.recent_activity_card(ui, narrow);
            ui.add_space(8.0);
            self.storage_snapshot_card(ui, s, specs.as_deref());
        }

        ui.add_space(10.0);
        self.session_log_card(ui);
        ui.add_space(8.0);
    }

    fn compact_glance_strip(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        memory_fraction: f32,
        peak_celsius: Option<f32>,
        smart: &str,
    ) {
        compact_card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.columns(2, |cols| {
                // CPU + RAM
                let cpu_txt = if s.cores.is_empty() || !s.cpu_usage.is_finite() {
                    "—".into()
                } else {
                    format!("{:.0}%", s.cpu_usage)
                };
                cols[0].horizontal(|ui| {
                    ui.label(RichText::new("CPU").size(10.0).strong().color(MUTED));
                    ui.label(
                        RichText::new(cpu_txt)
                            .monospace()
                            .size(14.0)
                            .strong()
                            .color(load_color(s.cpu_usage / 100.0)),
                    );
                    ui.add_space(8.0);
                    ui.label(RichText::new("RAM").size(10.0).strong().color(MUTED));
                    ui.label(
                        RichText::new(format!("{:.0}%", memory_fraction * 100.0))
                            .monospace()
                            .size(14.0)
                            .strong()
                            .color(VIOLET),
                    );
                });
                cols[0].add_space(4.0);
                bar(
                    &mut cols[0],
                    "compact-cpu",
                    s.cpu_usage / 100.0,
                    load_color(s.cpu_usage / 100.0),
                    4.0,
                );

                // Peak Temp + Health
                cols[1].horizontal(|ui| {
                    ui.label(RichText::new("PEAK").size(10.0).strong().color(MUTED));
                    let peak_str = peak_celsius
                        .map(|v| format!("{v:.0}°C"))
                        .unwrap_or_else(|| "—".into());
                    ui.label(
                        RichText::new(peak_str)
                            .monospace()
                            .size(14.0)
                            .strong()
                            .color(peak_celsius.map(temperature_color).unwrap_or(MUTED)),
                    );
                    if !smart.is_empty() {
                        ui.add_space(6.0);
                        let short = smart.split('·').next().unwrap_or(smart).trim();
                        chip(ui, &short.to_uppercase(), smart_color(smart));
                    }
                });
                cols[1].add_space(4.0);
                bar(&mut cols[1], "compact-ram", memory_fraction, VIOLET, 4.0);
            });
        });
    }

    fn expanded_kpi_row(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        memory_fraction: f32,
        peak_celsius: Option<f32>,
        smart: &str,
    ) {
        let hardware_adapters = s.adapters.iter().filter(|a| !a.software).count();
        ui.columns(4, |cols| {
            // 1. Processor KPI with arc gauge
            compact_card_frame().show(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(92.0);
                ui.horizontal(|ui| {
                    let pct = if s.cores.is_empty() || !s.cpu_usage.is_finite() {
                        "—".into()
                    } else {
                        format!("{:.0}%", s.cpu_usage)
                    };
                    circular_progress(
                        ui,
                        "kpi-cpu",
                        Some((s.cpu_usage / 100.0).clamp(0.0, 1.0)),
                        &pct,
                        "CPU",
                        62.0,
                        load_color(s.cpu_usage / 100.0),
                    );
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("PROCESSOR").size(9.5).strong().color(MUTED));
                        ui.add(
                            egui::Label::new(
                                RichText::new(truncate(&short_cpu_name(&s.cpu_name), 20))
                                    .size(12.0)
                                    .strong()
                                    .color(TEXT),
                            )
                            .truncate(),
                        );
                        let range_txt = match (s.cpu_range.min, s.cpu_range.max) {
                            (Some(lo), Some(hi)) => {
                                format!("{}T · {lo:.0}–{hi:.0}%", s.cores.len())
                            }
                            _ => format!("{} threads", s.cores.len()),
                        };
                        ui.label(RichText::new(range_txt).monospace().size(10.0).color(MUTED));
                        if ui
                            .link(RichText::new("Inspect >").size(10.5).color(ACCENT))
                            .clicked()
                        {
                            self.page = Page::Cpu;
                        }
                    });
                });
            });

            // 2. Memory KPI with arc gauge
            compact_card_frame().show(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(92.0);
                ui.horizontal(|ui| {
                    let pct = if s.memory_total > 0 {
                        format!("{:.0}%", memory_fraction * 100.0)
                    } else {
                        "—".into()
                    };
                    circular_progress(
                        ui,
                        "kpi-mem",
                        Some(memory_fraction),
                        &pct,
                        "RAM",
                        62.0,
                        VIOLET,
                    );
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("MEMORY").size(9.5).strong().color(MUTED));
                        ui.label(
                            RichText::new(if s.memory_total > 0 {
                                format!(
                                    "{:.1} / {:.1} GiB",
                                    gib(s.memory_used),
                                    gib(s.memory_total)
                                )
                            } else {
                                "Unavailable".into()
                            })
                            .monospace()
                            .size(12.0)
                            .strong()
                            .color(TEXT),
                        );
                        ui.label(
                            RichText::new(format!("{:.1} GiB free", gib(s.memory_available)))
                                .monospace()
                                .size(10.0)
                                .color(MUTED),
                        );
                        if ui
                            .link(RichText::new("DIMMs >").size(10.5).color(VIOLET))
                            .clicked()
                        {
                            self.page = Page::Memory;
                        }
                    });
                });
            });

            // 3. Graphics KPI
            compact_card_frame().show(&mut cols[2], |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(92.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("GRAPHICS").size(9.5).strong().color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        mono_badge(ui, &s.gpu_backend, CYAN);
                    });
                });
                ui.add_space(4.0);
                let gpu_name = s
                    .adapters
                    .first()
                    .map(|g| truncate(&g.name, 26))
                    .unwrap_or_else(|| "No adapter".into());
                ui.add(
                    egui::Label::new(RichText::new(gpu_name).size(12.5).strong().color(TEXT))
                        .truncate(),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!(
                        "{} adapter(s) · {} hardware",
                        s.adapters.len(),
                        hardware_adapters
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
                ui.add_space(4.0);
                if ui
                    .link(RichText::new("GPU-Z view >").size(10.5).color(ACCENT))
                    .clicked()
                {
                    self.page = Page::Gpu;
                }
            });

            // 4. System Health & Thermals KPI (strict emerald HEALTHY semantics)
            compact_card_frame().show(&mut cols[3], |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(92.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SYSTEM HEALTH")
                            .size(9.5)
                            .strong()
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !smart.is_empty() {
                            let short = smart.split('·').next().unwrap_or(smart).trim();
                            let col = smart_color(smart);
                            pill(ui, &short.to_uppercase(), col);
                        } else {
                            pill(ui, "LOCAL", GREEN);
                        }
                    });
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let peak_str = peak_celsius
                        .map(|v| format!("{v:.0} °C"))
                        .unwrap_or_else(|| "—".into());
                    ui.label(
                        RichText::new(peak_str)
                            .monospace()
                            .size(18.0)
                            .strong()
                            .color(peak_celsius.map(temperature_color).unwrap_or(MUTED)),
                    );
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} sensors · {} fans",
                                s.temperatures.len(),
                                s.fans.len()
                            ))
                            .size(10.0)
                            .color(TEXT_DIM),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Uptime {}",
                                uptime_text(sysinfo::System::uptime())
                            ))
                            .monospace()
                            .size(10.0)
                            .color(MUTED),
                        );
                    });
                });
            });
        });
    }

    /// Modern HWMonitor-style collapsible sensor tree with instant filter, visual
    /// Min–Current–Max range bars, click-to-plot, and Pin-to-HUD.
    fn hwmonitor_tree_card(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        _specs: Option<&SystemSpecs>,
        narrow: bool,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section_title(
                    ui,
                    "Live Sensor Tree",
                    if narrow {
                        None
                    } else {
                        Some("HWMonitor view · click row to graph")
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(ui, "Reset statistics").clicked() {
                        self.stats_reset.store(true, Ordering::Relaxed);
                    }
                });
            });
            ui.add_space(6.0);
            text_field(
                ui,
                &mut self.sensor_filter,
                "Filter sensors (e.g. CPU, TZ01, GPU, Fan)…",
                f32::INFINITY,
            );
            ui.add_space(8.0);

            let query = self.sensor_filter.trim().to_ascii_lowercase();

            // Group 1: Temperatures
            let temp_rows: Vec<RowView<'_>> = s
                .temperatures
                .iter()
                .filter(|t| {
                    query.is_empty()
                        || t.label.to_ascii_lowercase().contains(&query)
                        || t.key.to_ascii_lowercase().contains(&query)
                        || t.source.to_ascii_lowercase().contains(&query)
                })
                .map(RowView::from)
                .collect();

            let mut open = self.overview.sensors;
            let hint = format!("{} temps · {} fans", temp_rows.len(), s.fans.len());
            if collapsible_header(ui, "Thermals & Fans", Some(&hint), open, |_| {}) {
                open = !open;
                self.overview.sensors = open;
            }
            if open {
                ui.add_space(4.0);
                if temp_rows.is_empty() && s.fans.is_empty() {
                    empty_note(
                        ui,
                        if query.is_empty() {
                            sensor_unavailable_note()
                        } else {
                            "No thermal sensor matches the current filter."
                        },
                    );
                } else {
                    if !temp_rows.is_empty() {
                        let out = sensor_table(
                            ui,
                            "hwmon-temps",
                            "",
                            &temp_rows,
                            (!self.selected_temp_key.is_empty())
                                .then_some(self.selected_temp_key.as_str()),
                            &self.pinned_sensors,
                            self.pro_density,
                        );
                        if let Some(key) = out.clicked_key {
                            self.selected_temp_key = key;
                        }
                        if let Some(pin_key) = out.toggled_pin {
                            self.toggle_pin_sensor(&pin_key);
                        }
                    }
                    let fan_rows: Vec<RowView<'_>> = s
                        .fans
                        .iter()
                        .filter(|f| {
                            query.is_empty()
                                || f.label.to_ascii_lowercase().contains(&query)
                                || f.key.to_ascii_lowercase().contains(&query)
                        })
                        .map(RowView::from)
                        .collect();
                    if !fan_rows.is_empty() {
                        ui.add_space(8.0);
                        let out = sensor_table(
                            ui,
                            "hwmon-fans",
                            "Fans",
                            &fan_rows,
                            None,
                            &self.pinned_sensors,
                            self.pro_density,
                        );
                        if let Some(pin_key) = out.toggled_pin {
                            self.toggle_pin_sensor(&pin_key);
                        }
                    }
                }
            }

            // Group 2: Utilization (CPU & Memory min/max summary rows in HWMonitor style)
            ui.add_space(8.0);
            let mem_pct =
                (s.memory_total > 0).then(|| s.memory_used as f32 * 100.0 / s.memory_total as f32);
            let util_rows = [
                RowView {
                    label: "CPU Total Load",
                    key: "cpu.total",
                    source: "OS",
                    current: (!s.cores.is_empty() && s.cpu_usage.is_finite())
                        .then_some(s.cpu_usage),
                    min: s.cpu_range.min,
                    max: s.cpu_range.max,
                    average: None,
                    unit: "%",
                },
                RowView {
                    label: "Memory Utilization",
                    key: "mem.used",
                    source: "OS",
                    current: mem_pct,
                    min: s.memory_range.min,
                    max: s.memory_range.max,
                    average: None,
                    unit: "%",
                },
            ];
            let filtered_util: Vec<RowView<'_>> = util_rows
                .into_iter()
                .filter(|r| {
                    query.is_empty()
                        || r.label.to_ascii_lowercase().contains(&query)
                        || r.key.contains(&query)
                })
                .collect();
            if !filtered_util.is_empty() {
                sensor_table(
                    ui,
                    "hwmon-util",
                    "System Utilization",
                    &filtered_util,
                    None,
                    &self.pinned_sensors,
                    self.pro_density,
                );
            }

            // Group 3: Graphics adapters (collapsible)
            ui.add_space(8.0);
            let mut g_open = self.overview.graphics;
            let g_hint = format!("{} source(s)", s.gpus.len());
            if collapsible_header(ui, "Graphics Sources", Some(&g_hint), g_open, |_| {}) {
                g_open = !g_open;
                self.overview.graphics = g_open;
            }
            if g_open {
                ui.add_space(4.0);
                if s.gpus.is_empty() {
                    empty_note(ui, "No graphics adapter reported.");
                }
                for g in &s.gpus {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&g.name).strong().size(11.5).color(TEXT));
                        if g.detail.contains("software") {
                            chip(ui, "SOFTWARE", AMBER);
                        }
                    });
                    ui.label(RichText::new(&g.detail).size(10.5).color(MUTED));
                    ui.add_space(4.0);
                }
            }
        });
    }

    /// Liquid water-fill CPU core grid card (scales 4T -> 128T+).
    fn liquid_cores_card(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        specs: Option<&SystemSpecs>,
        narrow: bool,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let p_cores = specs.and_then(|sp| sp.cpu.performance_cores);
            let e_cores = specs.and_then(|sp| sp.cpu.efficiency_cores);
            let hint = match (p_cores, e_cores) {
                (Some(p), Some(e)) => format!("{p}P + {e}E · {} threads", s.cores.len()),
                _ => format!("{} logical threads", s.cores.len()),
            };
            let mut open = self.overview.cores;
            if collapsible_header(ui, "CPU Thread Activity", Some(&hint), open, |ui| {
                if !s.cores.is_empty() {
                    ui.label(
                        RichText::new(format!("{:.0}% avg", s.cpu_usage))
                            .monospace()
                            .size(10.5)
                            .color(load_color(s.cpu_usage / 100.0)),
                    );
                }
            }) {
                open = !open;
                self.overview.cores = open;
            }
            if open {
                ui.add_space(8.0);
                liquid_core_grid(ui, "overview-cores", s, narrow);
            }
        });
    }

    /// Right-hand (or inline) Live Sensor Inspector that plots whichever thermal
    /// sensor is clicked in the HWMonitor table.
    fn sensor_inspector_card(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let keys = history::temperature_keys(&self.history);
            if !keys.is_empty() && !keys.iter().any(|k| k == &self.selected_temp_key) {
                self.selected_temp_key = keys[0].clone();
            }
            let selected = self.selected_temp_key.clone();
            let live_temp = s.temperatures.iter().find(|t| t.key == selected);

            ui.horizontal(|ui| {
                let title = live_temp
                    .map(|t| t.label.as_str())
                    .unwrap_or("Sensor Inspector");
                section_title(ui, title, None);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(t) = live_temp {
                        if let Some(c) = t.celsius {
                            ui.label(
                                RichText::new(format!("{c:.1} °C"))
                                    .monospace()
                                    .size(16.0)
                                    .strong()
                                    .color(temperature_color(c)),
                            );
                        }
                    }
                });
            });
            if let Some(t) = live_temp {
                ui.horizontal_wrapped(|ui| {
                    mono_badge(ui, &t.key, MUTED);
                    mono_badge(ui, t.source, CYAN);
                    if let Some((lo, hi, mean)) = history::range(&self.history, |sample| {
                        history::temperature_value(sample, &selected)
                    }) {
                        ui.label(
                            RichText::new(format!("Min {lo:.0}° · Avg {mean:.0}° · Max {hi:.0}°"))
                                .monospace()
                                .size(10.0)
                                .color(TEXT_DIM),
                        );
                    }
                });
                ui.add_space(6.0);
            }
            if keys.is_empty() {
                empty_note(
                    ui,
                    "Click any temperature row or wait for thermal samples to plot.",
                );
            } else {
                let limit = self.alert_celsius as f64;
                Plot::new("inspector-temp-plot")
                    .height(126.0)
                    .include_y(25.0)
                    .include_y((limit + 5.0).max(90.0))
                    .auto_bounds([true, true])
                    .show_x(false)
                    .show_axes(egui::Vec2b::new(false, true))
                    .show_grid(egui::Vec2b::new(false, true))
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .show(ui, |plot| {
                        plot.hline(
                            HLine::new(limit)
                                .color(tint_alpha(AMBER, 160))
                                .style(egui_plot::LineStyle::Dashed { length: 6.0 })
                                .width(1.2_f32),
                        );
                        for (i, pts) in history::segments_with_gap(
                            &self.history,
                            self.started,
                            self.history_gap(),
                            |sample| history::temperature_value(sample, &selected),
                        )
                        .into_iter()
                        .enumerate()
                        {
                            plot.line(
                                Line::new(PlotPoints::from(pts))
                                    .name(if i == 0 { &selected } else { "" })
                                    .color(CYAN)
                                    .fill(20.0_f32)
                                    .width(2.0_f32),
                            );
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Dashed line: {:.0} °C advisory limit",
                            self.alert_celsius
                        ))
                        .size(10.0)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .link(RichText::new("Alerts & CSV >").size(10.5).color(ACCENT))
                            .clicked()
                        {
                            self.page = Page::Thermal;
                        }
                    });
                });
            }
        });
    }

    fn recent_activity_card(&mut self, ui: &mut egui::Ui, compact: bool) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut open = self.overview.activity;
            if collapsible_header(ui, "Recent Activity", Some("60 samples"), open, |ui| {
                if ghost_button(ui, "Reset").clicked() {
                    self.history.clear();
                }
                legend_dot(ui, "RAM", VIOLET);
                legend_dot(ui, "CPU", ACCENT);
            }) {
                open = !open;
                self.overview.activity = open;
            }
            if open {
                ui.add_space(6.0);
                if self.history.is_empty() {
                    empty_note(ui, "Waiting for the first telemetry samples…");
                } else {
                    Plot::new("overview-history")
                        .height(if compact { 112.0 } else { 132.0 })
                        .include_y(0.0)
                        .include_y(100.0)
                        .auto_bounds([true, true])
                        .show_x(false)
                        .show_axes(egui::Vec2b::new(false, true))
                        .show_grid(egui::Vec2b::new(false, true))
                        .allow_drag(false)
                        .allow_zoom(false)
                        .allow_scroll(false)
                        .show(ui, |p| {
                            for (i, points) in history::segments_with_gap(
                                &self.history,
                                self.started,
                                self.history_gap(),
                                |v| v.cpu,
                            )
                            .into_iter()
                            .enumerate()
                            {
                                p.line(
                                    Line::new(PlotPoints::from(points))
                                        .name(if i == 0 { "CPU %" } else { "" })
                                        .color(ACCENT)
                                        .width(1.9_f32),
                                );
                            }
                            for (i, points) in history::segments_with_gap(
                                &self.history,
                                self.started,
                                self.history_gap(),
                                |v| v.memory,
                            )
                            .into_iter()
                            .enumerate()
                            {
                                p.line(
                                    Line::new(PlotPoints::from(points))
                                        .name(if i == 0 { "Memory %" } else { "" })
                                        .color(VIOLET)
                                        .width(1.9_f32),
                                );
                            }
                        });
                }
            }
        });
    }

    fn storage_snapshot_card(
        &mut self,
        ui: &mut egui::Ui,
        s: &Snapshot,
        specs: Option<&SystemSpecs>,
    ) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut open = self.overview.storage;
            let hint = format!("{} volume(s)", s.disks.len());
            if collapsible_header(ui, "Storage Snapshot", Some(&hint), open, |_| {}) {
                open = !open;
                self.overview.storage = open;
            }
            if open {
                ui.add_space(6.0);
                if s.disks.is_empty() {
                    empty_note(ui, "No volumes reported.");
                }
                for d in &s.disks {
                    let used = d.total.saturating_sub(d.available);
                    let fraction = if d.total > 0 {
                        used as f32 / d.total as f32
                    } else {
                        0.0
                    };
                    let health = specs.and_then(|known| known.volume_health(&d.mount));
                    ui.horizontal(|ui| {
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
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", fraction * 100.0))
                                    .monospace()
                                    .size(11.0)
                                    .color(if fraction > 0.9 { AMBER } else { TEXT_DIM }),
                            );
                        });
                    });
                    bar(
                        ui,
                        &format!("disk-{}", d.mount),
                        fraction,
                        if fraction > 0.9 { AMBER } else { ACCENT },
                        5.0,
                    );
                    ui.label(
                        RichText::new(format!(
                            "{:.1} GiB free of {:.1} GiB · R {} / W {}",
                            gib(d.available),
                            gib(d.total),
                            bytes(d.read),
                            bytes(d.written)
                        ))
                        .monospace()
                        .size(10.0)
                        .color(MUTED),
                    );
                    ui.add_space(6.0);
                }
            }
        });
    }

    fn session_log_card(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut open = self.overview.log;
            if collapsible_header(
                ui,
                "Session CSV Logging & Export",
                Some("local files · never overwrites"),
                open,
                |_| {},
            ) {
                open = !open;
                self.overview.log = open;
            }
            if open {
                ui.add_space(8.0);
                let active = self.recorder.as_ref().is_some_and(|r| r.active());
                let finishing = self
                    .recorder
                    .as_ref()
                    .is_some_and(|r| !r.active() && !r.status().closed);
                ui.horizontal_wrapped(|ui| {
                    text_field_enabled(
                        ui,
                        !active && !finishing,
                        &mut self.log_path,
                        "New session CSV path",
                        280.0,
                    );
                    if ui
                        .add_enabled(!active && !finishing, egui::Button::new("Record CSV"))
                        .clicked()
                    {
                        match Recorder::start(std::path::Path::new(&self.log_path)) {
                            Ok(recorder) => {
                                self.recorder = Some(recorder);
                                self.message = format!("Recording to {}", self.log_path);
                            }
                            Err(error) => self.message = format!("Recording failed: {error}"),
                        }
                    }
                    if active && danger_button(ui, "Stop recording", false, 26.0).clicked() {
                        if let Some(r) = &mut self.recorder {
                            r.stop();
                        }
                    }
                });
                if let Some(recorder) = &self.recorder {
                    let status = recorder.status();
                    let state = if recorder.active() {
                        "Recording"
                    } else if status.closed {
                        "Stopped"
                    } else {
                        "Finishing"
                    };
                    ui.label(
                        RichText::new(format!(
                            "{state} · {} batches written · {} dropped",
                            status.batches, status.dropped
                        ))
                        .monospace()
                        .size(10.5)
                        .color(TEXT_DIM),
                    );
                    if let Some(error) = status.error {
                        ui.colored_label(AMBER, format!("Recording error: {error}"));
                    }
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    text_field(ui, &mut self.history_path, "60-sample history CSV", 280.0);
                    if ghost_button(ui, "Export 60-sample CSV").clicked() {
                        self.message = match history::export_csv(
                            std::path::Path::new(&self.history_path),
                            &self.history,
                            self.started,
                        ) {
                            Ok(()) => format!("Saved {}", self.history_path),
                            Err(e) => format!("Export failed: {e}"),
                        };
                    }
                });
            }
        });
    }

    pub fn view_core_temperatures(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                section_title(ui, "CPU core temperatures", None);
                if ghost_button(ui, "Reset statistics").clicked() {
                    self.stats_reset.store(true, Ordering::Relaxed);
                }
            });
            ui.add_space(6.0);
            #[cfg(target_os = "windows")]
            {
                use hardware_hub::sensors::ExternalThermals;
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.libre_monitor_enabled, "Read LibreHardwareMonitor");
                    if self.libre_monitor_enabled {
                        ui.add(
                            egui::DragValue::new(&mut self.libre_monitor_port)
                                .range(1..=u16::MAX)
                                .prefix("Port "),
                        );
                    }
                });
                if self.libre_monitor_enabled {
                    let (status, color) = match snapshot.external_thermals {
                        ExternalThermals::Connected => ("Local feed connected", GREEN),
                        ExternalThermals::Unavailable => ("Waiting for local feed", AMBER),
                        ExternalThermals::Disabled => ("Connecting to local feed", MUTED),
                    };
                    pill(ui, status, color);
                    if snapshot.external_thermals != ExternalThermals::Connected {
                        ui.label(format!(
                            "Run LibreHardwareMonitor’s web server on 127.0.0.1:{}.",
                            self.libre_monitor_port,
                        ));
                    }
                    ui.add_space(5.0);
                }
            }
            let count = snapshot.temperatures.iter().filter(|sensor| sensor.cpu_core).count();
            if count == 0 {
                empty_note(ui, if snapshot.ready {
                    "No per-core temperature channels reported. Package and zone readings are in Thermals / fans."
                } else {
                    "Waiting for the first temperature sample."
                });
                return;
            }
            let sampled = snapshot.temperatures.iter()
                .filter(|sensor| sensor.cpu_core && sensor.celsius.is_some()).count();
            ui.label(RichText::new(format!("{sampled} of {count} channels reporting · session statistics"))
                .size(10.5).color(TEXT_DIM));
            text_field(ui, &mut self.core_sensor_filter, "Filter by core name or sensor key…", f32::INFINITY);
            let query = self.core_sensor_filter.trim().to_ascii_lowercase();
            let rows: Vec<_> = snapshot.temperatures.iter().filter(|sensor| sensor.cpu_core)
                .filter(|sensor| query.is_empty()
                    || sensor.label.to_ascii_lowercase().contains(&query)
                    || sensor.key.to_ascii_lowercase().contains(&query)
                    || sensor.source.to_ascii_lowercase().contains(&query))
                .map(RowView::from).collect();
            if rows.is_empty() {
                empty_note(ui, "No core temperatures match this filter.");
                return;
            }
            let action = sensor_table(ui, "physical-core-sensors", "", &rows, Some(&self.selected_temp_key), &self.pinned_sensors, true);
            if let Some(key) = action.clicked_key {
                self.selected_temp_key = key;
                self.focus_temperature_chart = true;
                self.page = Page::Thermal;
            }
            if let Some(key) = action.toggled_pin {
                self.toggle_pin_sensor(&key);
            }
        });
    }

    // ---------------------------------------------------------------- Thermals & Alerts page
    pub fn view_thermal(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let peak_c = snapshot
            .temperatures
            .iter()
            .filter_map(|t| t.celsius)
            .fold(None::<f32>, |acc, v| Some(acc.map_or(v, |p| p.max(v))));

        // Top telemetry control strip
        compact_card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let peak_str = peak_c
                    .map(|v| format!("Peak {v:.0}°C"))
                    .unwrap_or_else(|| "Peak —".into());
                chip(
                    ui,
                    &peak_str,
                    peak_c.map(temperature_color).unwrap_or(MUTED),
                );
                if let Some(p) = peak_c {
                    let headroom = self.alert_celsius - p;
                    chip(
                        ui,
                        &format!("Headroom {headroom:+.0}°C"),
                        if headroom <= 0.0 {
                            RED
                        } else if headroom <= 12.0 {
                            AMBER
                        } else {
                            CYAN
                        },
                    );
                }
                chip(ui, &format!("Alert {:.0}°C", self.alert_celsius), AMBER);
                ui.add_space(8.0);
                let mut poll = self.poll_ms.load(Ordering::Relaxed);
                if segmented(
                    ui,
                    &mut poll,
                    &[(250, "250ms"), (500, "500ms"), (1000, "1s"), (2000, "2s")],
                ) {
                    self.poll_ms.store(poll, Ordering::Relaxed);
                }
            });
        });

        ui.add_space(10.0);

        self.view_core_temperatures(ui, snapshot);
        ui.add_space(10.0);

        let chart = card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Temperature History",
                Some("gap-aware · up to 60 batches"),
            );
            let keys = history::temperature_keys(&self.history);
            if keys.is_empty() {
                empty_note(ui, "Waiting for temperature samples…");
            } else {
                if !keys.iter().any(|k| k == &self.selected_temp_key) {
                    self.selected_temp_key = keys[0].clone();
                }
                let selected = self.selected_temp_key.clone();
                ui.horizontal_wrapped(|ui| {
                    let display_name = |key: &str| -> String {
                        snapshot
                            .temperatures
                            .iter()
                            .find(|sensor| sensor.key == key)
                            .map(|sensor| format!("{} · {}", sensor.label, sensor.source))
                            .unwrap_or_else(|| key.to_owned())
                    };
                    egui::ComboBox::from_id_salt("thermal-series")
                        .selected_text(display_name(&selected))
                        .show_ui(ui, |ui| {
                            for key in &keys {
                                ui.selectable_value(
                                    &mut self.selected_temp_key,
                                    key.clone(),
                                    display_name(key),
                                )
                                .on_hover_text(key);
                            }
                        });
                    if let Some((low, high, mean)) = history::range(&self.history, |sample| {
                        history::temperature_value(sample, &selected)
                    }) {
                        ui.label(
                            RichText::new(format!("{low:.1}–{high:.1} °C · avg {mean:.1} °C"))
                                .monospace()
                                .size(11.0)
                                .color(TEXT_DIM),
                        );
                    }
                });
                ui.add_space(6.0);
                let limit = self.alert_celsius as f64;
                Plot::new("thermal-history-main")
                    .height(160.0)
                    .include_y(20.0)
                    .include_y((limit + 5.0).max(90.0))
                    .auto_bounds([true, true])
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .show_x(false)
                    .show(ui, |plot| {
                        plot.hline(
                            HLine::new(limit)
                                .color(tint_alpha(AMBER, 175))
                                .style(egui_plot::LineStyle::Dashed { length: 6.0 })
                                .width(1.3_f32),
                        );
                        for points in history::segments_with_gap(
                            &self.history,
                            self.started,
                            self.history_gap(),
                            |sample| history::temperature_value(sample, &selected),
                        ) {
                            plot.line(
                                Line::new(PlotPoints::from(points))
                                    .color(CYAN)
                                    .fill(20.0_f32)
                                    .width(2.0_f32),
                            );
                        }
                    });
                ui.add_space(6.0);
                egui::CollapsingHeader::new("Export sensors to CSV").show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        text_field(ui, &mut self.sensor_log_path, "One-sensor CSV path", 300.0);
                        if ghost_button(ui, "Export selected").clicked() {
                            self.message = match history::export_sensor_csv(
                                std::path::Path::new(&self.sensor_log_path),
                                &self.history,
                                &selected,
                                self.started,
                            ) {
                                Ok(()) => format!("Saved {}", self.sensor_log_path),
                                Err(error) => format!("Export failed: {error}"),
                            };
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        text_field(
                            ui,
                            &mut self.all_sensor_log_path,
                            "All-sensors wide CSV",
                            300.0,
                        );
                        if ghost_button(ui, "Export all sensors").clicked() {
                            self.message = match history::export_all_sensors_csv(
                                std::path::Path::new(&self.all_sensor_log_path),
                                &self.history,
                                self.started,
                            ) {
                                Ok(()) => format!("Saved {}", self.all_sensor_log_path),
                                Err(error) => format!("Export failed: {error}"),
                            };
                        }
                    });
                });
            }
        });
        if std::mem::take(&mut self.focus_temperature_chart) {
            chart.response.scroll_to_me(Some(egui::Align::Min));
        }

        ui.add_space(10.0);

        // Full HWMonitor sensor table
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Other temperatures & fans", None);
            ui.add_space(6.0);
            if snapshot.temperatures.is_empty() && snapshot.fans.is_empty() {
                empty_note(ui, sensor_unavailable_note());
            } else {
                let temps: Vec<RowView<'_>> = snapshot
                    .temperatures
                    .iter()
                    .filter(|sensor| !sensor.cpu_core)
                    .map(RowView::from)
                    .collect();
                if !temps.is_empty() {
                    let out = sensor_table(
                        ui,
                        "thermal-page-temps",
                        "",
                        &temps,
                        (!self.selected_temp_key.is_empty())
                            .then_some(self.selected_temp_key.as_str()),
                        &self.pinned_sensors,
                        true,
                    );
                    if let Some(k) = out.clicked_key {
                        self.selected_temp_key = k;
                        self.focus_temperature_chart = true;
                        ui.ctx().request_repaint();
                    }
                    if let Some(k) = out.toggled_pin {
                        self.toggle_pin_sensor(&k);
                    }
                }
                let fans: Vec<RowView<'_>> = snapshot.fans.iter().map(RowView::from).collect();
                if !fans.is_empty() {
                    ui.add_space(8.0);
                    let out = sensor_table(
                        ui,
                        "thermal-page-fans",
                        "Fans",
                        &fans,
                        None,
                        &self.pinned_sensors,
                        true,
                    );
                    if let Some(k) = out.toggled_pin {
                        self.toggle_pin_sensor(&k);
                    }
                }
            }
        });

        ui.add_space(10.0);

        // Advisory alert configuration + fine sampling slider
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Advisory Temperature Alert & Polling",
                Some("never alters fans or clocks"),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Alert at").size(11.5).color(TEXT_DIM));
                ui.add(
                    egui::DragValue::new(&mut self.alert_celsius)
                        .range(30.0..=125.0)
                        .speed(0.5)
                        .suffix(" °C"),
                );
                ui.label(RichText::new("held for").size(11.5).color(TEXT_DIM));
                ui.add(
                    egui::DragValue::new(&mut self.alert_hold_secs)
                        .range(1..=120)
                        .speed(1)
                        .suffix(" s"),
                );
                segmented(
                    ui,
                    &mut self.alert_severity,
                    &[
                        (Severity::Warning, "Warning"),
                        (Severity::Critical, "Critical"),
                    ],
                );
            });
            ui.add_space(6.0);
            let rule = Threshold {
                celsius: self.alert_celsius,
                hold: Duration::from_secs(self.alert_hold_secs),
                severity: self.alert_severity,
            };
            if !rule.is_valid() {
                ui.colored_label(AMBER, "Choose 30–125 °C held for 1–120 seconds.");
            } else if let Some(since) = self.alert.active_since() {
                let color = if rule.severity == Severity::Critical {
                    RED
                } else {
                    AMBER
                };
                ui.horizontal_wrapped(|ui| {
                    pill(ui, rule.severity.label(), color);
                    ui.colored_label(
                        color,
                        format!(
                            "At or above {:.0} °C for {:.0}s",
                            rule.celsius,
                            since.elapsed().as_secs_f32()
                        ),
                    );
                    if ghost_button(ui, "Acknowledge").clicked() {
                        self.alert.clear();
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "No active alert · Session peak: {}",
                            self.alert
                                .peak_celsius()
                                .map(|v| format!("{v:.1} °C"))
                                .unwrap_or_else(|| "unavailable".into())
                        ))
                        .monospace()
                        .size(11.0)
                        .color(MUTED),
                    );
                    if ghost_button(ui, "Reset alert").clicked() {
                        self.alert.clear();
                    }
                });
            }
            ui.add_space(8.0);
            let mut period_ms = self.poll_ms.load(Ordering::Relaxed) as f64;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Custom polling").size(11.0).color(TEXT_DIM));
                if ui
                    .add(
                        egui::Slider::new(&mut period_ms, 250.0..=5000.0)
                            .step_by(250.0)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.poll_ms
                        .store(period_ms.round() as u64, Ordering::Relaxed);
                }
            });
        });
    }

    pub fn toggle_pin_sensor(&mut self, key: &str) {
        if let Some(pos) = self.pinned_sensors.iter().position(|k| k == key) {
            self.pinned_sensors.remove(pos);
            self.message = format!("Unpinned {key} from Floating HUD");
        } else {
            if self.pinned_sensors.len() >= 6 {
                self.pinned_sensors.remove(0);
            }
            self.pinned_sensors.push(key.to_owned());
            self.message = format!("Pinned {key} to Floating HUD");
        }
    }
}
