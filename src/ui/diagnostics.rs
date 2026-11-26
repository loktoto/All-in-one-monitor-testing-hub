//! Diagnostics & Reports workspace: virtualized Processes table, PresentMon
//! Frame Analysis, CPU-Z/GPU-Z System Report export, Run Comparison, and
//! shareable redacted Run Summaries.

use crate::ui::{kind_label, theme::*, widgets::*};
use crate::App;
use eframe::egui::{self, RichText};
use hardware_hub::{
    frames,
    report::{self, Format},
    stress::Kind,
    system_report,
    telemetry::Snapshot,
};

impl App {
    pub fn view_processes(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Live Processes", Some("virtualized · OS-reported"));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                text_field(
                    ui,
                    &mut self.process_query,
                    "Filter by process name or PID…",
                    240.0,
                );
                ui.add_space(6.0);
                let mut by_memory = self.process_sort_memory;
                if segmented(
                    ui,
                    &mut by_memory,
                    &[(false, "Sort: CPU"), (true, "Sort: Memory")],
                ) {
                    self.process_sort_memory = by_memory;
                }
            });
            ui.add_space(4.0);
            let query = self.process_query.to_lowercase();
            let mut processes: Vec<_> = s
                .processes
                .iter()
                .filter(|p| {
                    query.is_empty()
                        || p.name.to_lowercase().contains(&query)
                        || p.pid.to_string().contains(&query)
                })
                .collect();
            if self.process_sort_memory {
                processes.sort_by(|a, b| b.memory.cmp(&a.memory).then_with(|| a.pid.cmp(&b.pid)));
            } else {
                processes.sort_by(|a, b| {
                    b.cpu_usage
                        .total_cmp(&a.cpu_usage)
                        .then_with(|| a.pid.cmp(&b.pid))
                });
            }
            ui.label(
                RichText::new(format!(
                    "{} matching process(es) · CPU % is relative to one logical core",
                    processes.len()
                ))
                .monospace()
                .size(10.0)
                .color(MUTED),
            );
            ui.add_space(6.0);
            if processes.is_empty() {
                empty_note(
                    ui,
                    if s.ready {
                        "No matching processes reported."
                    } else {
                        "Collecting process list…"
                    },
                );
            } else {
                process_table(ui, &processes);
            }
        });
    }

    pub fn view_frames(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "PresentMon Frame Analysis", Some("offline CSV · Avg FPS / 1% Low / P99"));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Load a saved PresentMon CSV capture (one process, one swap chain). Hardware Hub does not inject into games.")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.add_space(8.0);
            text_field(ui, &mut self.frame_path, "Path to PresentMon CSV", f32::INFINITY);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Analyze capture", false, 30.0).clicked() {
                    match frames::read(std::path::Path::new(&self.frame_path)) {
                        Ok(summary) => {
                            self.frame_summary = Some(summary);
                            self.frame_message.clear();
                        }
                        Err(error) => {
                            self.frame_summary = None;
                            self.frame_message = error;
                        }
                    }
                }
                if self.frame_summary.is_some() && ghost_button(ui, "Clear").clicked() {
                    self.frame_summary = None;
                }
            });
            if !self.frame_message.is_empty() {
                ui.add_space(6.0);
                ui.colored_label(AMBER, &self.frame_message);
            }
            if let Some(r) = &self.frame_summary {
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!("{} · PID {} · swap chain {}", r.application, r.process_id, r.swap_chain))
                        .size(12.5)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new(format!("{} valid intervals · {} skipped · {}", r.valid, r.skipped, r.column))
                        .monospace()
                        .size(10.0)
                        .color(MUTED),
                );
                ui.add_space(8.0);
                let mut rows = vec![
                    ("Average displayed FPS".into(), format!("{:.1} FPS", r.average_fps)),
                    ("Slowest 1% FPS".into(), format!("{:.1} FPS", r.one_percent_low_fps)),
                    ("99th percentile frame time".into(), format!("{:.2} ms", r.p99_ms)),
                ];
                if let Some(v) = r.average_cpu_ms {
                    rows.push(("Mean CPU busy".into(), format!("{v:.2} ms")));
                }
                if let Some(v) = r.average_gpu_ms {
                    rows.push(("Mean GPU busy".into(), format!("{v:.2} ms")));
                }
                spec_grid(ui, "frame-summary-grid", &rows);
            }
        });
    }

    pub fn view_reports(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let records = self
            .state
            .lock()
            .map(|s| s.results.clone())
            .unwrap_or_default();

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "System Inventory Report",
                Some("CPU-Z / GPU-Z style · local only"),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                let before = self.system_report_format;
                segmented(
                    ui,
                    &mut self.system_report_format,
                    &[(Format::Text, "Text"), (Format::Json, "JSON")],
                );
                if self.system_report_format != before {
                    match self.system_report_format {
                        Format::Text if self.system_report_path.ends_with(".json") => {
                            self.system_report_path =
                                self.system_report_path.replace(".json", ".txt");
                        }
                        Format::Json if self.system_report_path.ends_with(".txt") => {
                            self.system_report_path =
                                self.system_report_path.replace(".txt", ".json");
                        }
                        _ => {}
                    }
                }
                ui.add_space(8.0);
                toggle(ui, &mut self.system_report_redact, "Redact serials & paths");
            });
            ui.add_space(8.0);
            text_field(
                ui,
                &mut self.system_report_path,
                "Output path (never overwrites)",
                f32::INFINITY,
            );
            ui.add_space(8.0);
            if primary_button(ui, "Export System Report", false, 30.0).clicked() {
                let specs = self.specs.lock().ok().and_then(|held| held.clone());
                self.message = match specs {
                    Some(specs) => match system_report::export(
                        std::path::Path::new(&self.system_report_path),
                        &specs,
                        snapshot,
                        self.system_report_format,
                        self.system_report_redact,
                    ) {
                        Ok(()) => format!("Saved {}", self.system_report_path),
                        Err(error) => format!("Export failed: {error}"),
                    },
                    None => "The specification sheet is still being collected.".into(),
                };
            }
        });

        ui.add_space(10.0);

        if records.is_empty() {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new("No stress checks recorded this session")
                            .size(13.0)
                            .strong()
                            .color(TEXT_DIM),
                    );
                    ui.label(
                        RichText::new(
                            "Run any check in Stress Lab to export or compare results here.",
                        )
                        .size(11.0)
                        .color(MUTED),
                    );
                    ui.add_space(14.0);
                });
            });
            return;
        }

        // Session test results export
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Export Session Stress Results",
                Some("never overwrites"),
            );
            ui.add_space(6.0);
            let before = self.report_format;
            segmented(
                ui,
                &mut self.report_format,
                &[(Format::Text, "Text"), (Format::Json, "JSON")],
            );
            if self.report_format != before {
                match self.report_format {
                    Format::Text if self.report == "hardware-hub-report.json" => {
                        self.report = "hardware-hub-report.txt".into();
                    }
                    Format::Json if self.report == "hardware-hub-report.txt" => {
                        self.report = "hardware-hub-report.json".into();
                    }
                    _ => {}
                }
            }
            ui.add_space(6.0);
            text_field(ui, &mut self.report, "Report output path", f32::INFINITY);
            ui.add_space(6.0);
            if primary_button(ui, "Export Results", false, 30.0).clicked() {
                self.message = match report::export(
                    std::path::Path::new(&self.report),
                    self.report_format,
                    &records,
                ) {
                    Ok(()) => format!("Saved {}", self.report),
                    Err(e) => format!("Export failed: {e}"),
                };
            }
        });

        if records.len() >= 2 {
            ui.add_space(10.0);
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Compare Runs", Some("side by side · this session"));
                ui.add_space(6.0);
                let labels: Vec<String> = records
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        format!(
                            "{} · {} · {}",
                            i + 1,
                            kind_label(r.kind),
                            timestamp_text(r.timestamp)
                        )
                    })
                    .collect();
                self.compare_a = self.compare_a.min(records.len() - 1);
                self.compare_b = self.compare_b.min(records.len() - 1);
                ui.columns(2, |cols| {
                    for (slot, (value, label)) in
                        [(&mut self.compare_a, "A"), (&mut self.compare_b, "B")]
                            .into_iter()
                            .enumerate()
                    {
                        let w = cols[slot].available_width() - 24.0;
                        cols[slot].horizontal(|ui| {
                            ui.label(RichText::new(label).strong().color(MUTED));
                            egui::ComboBox::from_id_salt(format!("cmp-{label}"))
                                .width(w)
                                .selected_text(RichText::new(&labels[*value]).size(10.5))
                                .show_ui(ui, |ui| {
                                    for (i, text) in labels.iter().enumerate() {
                                        ui.selectable_value(value, i, text);
                                    }
                                });
                        });
                    }
                });
                let a = &records[self.compare_a];
                let b = &records[self.compare_b];
                ui.add_space(6.0);
                comparison_row(ui, "Result", &a.status.to_string(), &b.status.to_string());
                comparison_row(
                    ui,
                    "Verified",
                    &format!(
                        "{} {}",
                        a.verified_units
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".into()),
                        a.verified_unit
                    ),
                    &format!(
                        "{} {}",
                        b.verified_units
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".into()),
                        b.verified_unit
                    ),
                );
                comparison_row(
                    ui,
                    "Errors",
                    &a.error_count.to_string(),
                    &b.error_count.to_string(),
                );
                comparison_row(
                    ui,
                    "Elapsed",
                    &format!("{:.1}s", a.elapsed),
                    &format!("{:.1}s", b.elapsed),
                );
                comparison_row(
                    ui,
                    "Settings",
                    &config_summary(&a.config),
                    &config_summary(&b.config),
                );
            });
        }

        ui.add_space(10.0);
        for (idx, r) in records.iter().enumerate().rev() {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    chip(
                        ui,
                        &r.status.to_string().to_uppercase(),
                        status_color(&r.status),
                    );
                    ui.label(
                        RichText::new(kind_label(r.kind))
                            .size(12.5)
                            .strong()
                            .color(TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{:.1}s · {}",
                                r.elapsed,
                                timestamp_text(r.timestamp)
                            ))
                            .monospace()
                            .size(10.5)
                            .color(MUTED),
                        );
                    });
                });
                ui.add_space(4.0);
                ui.label(RichText::new(&r.detail).size(11.5).color(TEXT_DIM));
                ui.label(
                    RichText::new(config_summary(&r.config))
                        .monospace()
                        .size(10.0)
                        .color(MUTED),
                );
                error_details(ui, &r.errors, r.error_count);
                ui.add_space(4.0);
                ui.push_id(format!("cert-{idx}"), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        text_field(ui, &mut self.certificate_path, "Run summary path", 320.0);
                        if ghost_button(ui, "Save Run Summary").clicked() {
                            self.message = match report::export_certificate(
                                std::path::Path::new(&self.certificate_path),
                                r,
                                self.run_peak_c,
                            ) {
                                Ok(()) => format!("Saved {}", self.certificate_path),
                                Err(e) => format!("Export failed: {e}"),
                            };
                        }
                    });
                });
                ui.push_id(idx, |ui| {
                    egui::CollapsingHeader::new("Evidence & system context").show(ui, |ui| {
                        ui.label(format!(
                            "{} · {} logical cores · {:.1} GiB RAM",
                            r.context.cpu,
                            r.context.logical_cpus,
                            gib(r.context.memory_total)
                        ));
                        ui.label(format!("OS: {}", r.context.os));
                        if r.kind == Kind::Disk {
                            ui.label(format!(
                                "Directory: {} · cap {} MiB",
                                r.config.directory.display(),
                                r.config.disk_write_limit_mib
                            ));
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
    }
}
