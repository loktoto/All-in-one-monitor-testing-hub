#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use hardware_hub::{
    frames::{self, FrameSummary},
    history::{self, History},
    logging::Recorder,
    netcheck::{self, State as NetworkCheckState},
    report::{self, Format},
    smart::{self, SmartReport},
    specs::{self, SystemSpecs},
    stress::{self, Config, Kind, ReportContext, State, Status},
    telemetry::{self, Snapshot},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

// ---------------------------------------------------------------- design tokens
// Apple system dark palette — neutral graphite surfaces with high-contrast labels.
const BG: Color32 = Color32::from_rgb(28, 28, 30);
const PANEL: Color32 = Color32::from_rgb(36, 36, 38);
const CARD: Color32 = Color32::from_rgb(44, 44, 46);
const CARD_HI: Color32 = Color32::from_rgb(58, 58, 60);
const BORDER: Color32 = Color32::from_rgb(52, 52, 54);
const TRACK: Color32 = Color32::from_rgb(64, 64, 66);
const TEXT: Color32 = Color32::from_rgb(245, 245, 247);
const TEXT_DIM: Color32 = Color32::from_rgb(174, 174, 178);
const MUTED: Color32 = Color32::from_rgb(134, 134, 139);
const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
const ACCENT_SOFT: Color32 = Color32::from_rgb(36, 62, 96);
const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
const AMBER: Color32 = Color32::from_rgb(255, 159, 10);
const RED: Color32 = Color32::from_rgb(255, 69, 58);
const VIOLET: Color32 = Color32::from_rgb(191, 90, 242);

const KINDS: [(Kind, &str, &str); 4] = [
    (
        Kind::Cpu,
        "CPU compute",
        "Integer rounds against a fixed answer",
    ),
    (
        Kind::Gpu,
        "GPU check",
        "Shader dispatch verified by readback",
    ),
    (
        Kind::Memory,
        "Memory patterns",
        "Alternating bytes written and read back",
    ),
    (
        Kind::Disk,
        "File I/O",
        "Owned scratch file written and verified",
    ),
];

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Hardware Hub")
            .with_inner_size([1180.0, 800.0])
            .with_min_inner_size([880.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Hardware Hub",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Overview,
    Cpu,
    Gpu,
    Memory,
    Storage,
    Thermal,
    Motherboard,
    Network,
    Bluetooth,
    Details,
    Processes,
    Frames,
    Tests,
    Reports,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Cpu => "CPU",
            Page::Gpu => "GPU",
            Page::Memory => "Memory",
            Page::Storage => "Storage",
            Page::Thermal => "Thermals / fans",
            Page::Motherboard => "Motherboard",
            Page::Network => "Network / Wi-Fi",
            Page::Bluetooth => "Bluetooth",
            Page::Details => "Details",
            Page::Processes => "Processes",
            Page::Frames => "Frame analysis",
            Page::Tests => "Stress tests",
            Page::Reports => "Reports",
        }
    }
    fn title(self) -> &'static str {
        self.label()
    }
    fn group(self) -> &'static str {
        match self {
            Page::Details | Page::Processes | Page::Frames | Page::Tests | Page::Reports => {
                "WORKSPACE"
            }
            _ => "HARDWARE",
        }
    }
    fn subtitle(self) -> &'static str {
        match self {
            Page::Overview => "Live local readings, refreshed about once per second.",
            Page::Cpu => "Processor readings, specifications and a bounded CPU check.",
            Page::Gpu => "Graphics sources and a bounded GPU compute check.",
            Page::Memory => "Memory usage, modules and a bounded pattern check.",
            Page::Storage => "Mounted volumes, reported drives and a bounded file check.",
            Page::Thermal => {
                "Available temperature and fan sensors with observed minimum and maximum."
            }
            Page::Motherboard => "System identification and available board-related information.",
            Page::Network => "Network interfaces and operating-system traffic counters.",
            Page::Bluetooth => "Bluetooth availability and supported diagnostics.",
            Page::Details => "Specification sheet read once from local OS sources.",
            Page::Processes => "Live CPU and memory usage by process, plus network interfaces.",
            Page::Frames => {
                "Inspect a saved PresentMon capture; no live game capture is performed."
            }
            Page::Tests => "One bounded check at a time. These are not benchmarks.",
            Page::Reports => "Session results. Nothing leaves this window until you export.",
        }
    }
}

#[derive(Default)]
struct BluetoothRefresh {
    running: bool,
    attempted: bool,
    value: Option<specs::BluetoothSpec>,
}

struct App {
    page: Page,
    snapshot: Arc<Mutex<Snapshot>>,
    specs: Arc<Mutex<Option<SystemSpecs>>>,
    stop: Arc<AtomicBool>,
    /// Set to ask the telemetry thread to clear its min/max history.
    stats_reset: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
    kind: Kind,
    seconds: u64,
    workers: usize,
    mib: usize,
    disk_write_limit_mib: u64,
    directory: String,
    report: String,
    report_format: Format,
    gpu_index: usize,
    gpu_initialized: bool,
    message: String,
    history: History,
    history_path: String,
    log_path: String,
    recorder: Option<Recorder>,
    process_query: String,
    process_sort_memory: bool,
    bluetooth_refresh: Arc<Mutex<BluetoothRefresh>>,
    network_target: String,
    network_message: String,
    network_check: Arc<Mutex<NetworkCheckState>>,
    network_cancel: Arc<AtomicBool>,
    smart_path: String,
    smart_report: Option<SmartReport>,
    smart_message: String,
    frame_path: String,
    frame_summary: Option<FrameSummary>,
    frame_message: String,
    show_widget: bool,
    started: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_theme(&cc.egui_ctx);
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let stats_reset = Arc::new(AtomicBool::new(false));
        telemetry::spawn(snapshot.clone(), stop.clone(), stats_reset.clone());
        // The inventory shells out to local tools once, so it must not block the
        // first frame; the Details page shows a placeholder until it lands.
        let specs = Arc::new(Mutex::new(None));
        let specs_slot = Arc::clone(&specs);
        std::thread::spawn(move || {
            let collected = specs::collect();
            if let Ok(mut slot) = specs_slot.lock() {
                *slot = Some(collected);
            }
        });
        Self {
            page: Page::Overview,
            snapshot,
            specs,
            stop,
            stats_reset,
            cancel: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(State::default())),
            kind: Kind::Cpu,
            seconds: 15,
            workers: std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(1),
            mib: 64,
            disk_write_limit_mib: 64,
            directory: std::env::temp_dir().to_string_lossy().into_owned(),
            report: "hardware-hub-report.txt".into(),
            report_format: Format::Text,
            gpu_index: 0,
            gpu_initialized: false,
            message: String::new(),
            history: History::default(),
            history_path: "hardware-hub-history.csv".into(),
            log_path: "hardware-hub-session.csv".into(),
            recorder: None,
            process_query: String::new(),
            process_sort_memory: false,
            bluetooth_refresh: Arc::new(Mutex::new(BluetoothRefresh::default())),
            network_target: String::new(),
            network_message: String::new(),
            network_check: Arc::new(Mutex::new(NetworkCheckState::default())),
            network_cancel: Arc::new(AtomicBool::new(false)),
            smart_path: String::new(),
            smart_report: None,
            smart_message: String::new(),
            frame_path: String::new(),
            frame_summary: None,
            frame_message: String::new(),
            show_widget: false,
            started: Instant::now(),
        }
    }

    // ------------------------------------------------------------- navigation
    fn navigation(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let frame = egui::Frame {
            fill: PANEL,
            stroke: Stroke::new(1.0_f32, BORDER),
            inner_margin: egui::Margin {
                left: 14,
                right: 14,
                top: 18,
                bottom: 14,
            },
            ..Default::default()
        };
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(236.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, CornerRadius::same(9), ACCENT);
                    painter.circle_stroke(
                        rect.center(),
                        6.5,
                        Stroke::new(1.5_f32, Color32::from_white_alpha(210)),
                    );
                    painter.circle_filled(rect.center(), 2.0, Color32::from_white_alpha(230));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Hardware Hub")
                                .size(14.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new(format!("Local · v{}", env!("CARGO_PKG_VERSION")))
                                .size(10.5)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(20.0);
                let check_active = self.state.lock().map(|s| s.running).unwrap_or(false);
                let log_visible = self.recorder.as_ref().is_some_and(|r| {
                    r.active() || !r.status().closed || r.status().error.is_some()
                });
                let reserved = 150.0
                    + if check_active { 55.0 } else { 0.0 }
                    + if log_visible { 55.0 } else { 0.0 };
                let max_nav_height = (ui.available_height() - reserved).max(120.0);
                egui::ScrollArea::vertical()
                    .id_salt("navigation-pages")
                    .max_height(max_nav_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.label(RichText::new("HARDWARE").size(10.5).strong().color(MUTED));
                        ui.add_space(6.0);
                        for page in [
                            Page::Overview,
                            Page::Cpu,
                            Page::Gpu,
                            Page::Memory,
                            Page::Storage,
                            Page::Thermal,
                            Page::Motherboard,
                            Page::Network,
                            Page::Bluetooth,
                        ] {
                            if nav_item(ui, self.page == page, page, page.label()).clicked() {
                                self.page = page;
                            }
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new("WORKSPACE").size(10.5).strong().color(MUTED));
                        ui.add_space(6.0);
                        for page in [
                            Page::Details,
                            Page::Processes,
                            Page::Frames,
                            Page::Tests,
                            Page::Reports,
                        ] {
                            if nav_item(ui, self.page == page, page, page.label()).clicked() {
                                self.page = page;
                            }
                        }
                    });
                ui.add_space(12.0);
                ui.checkbox(&mut self.show_widget, "Floating monitor")
                    .on_hover_text("Desktop window; not an in-game FPS overlay.");
                if let Some(recorder) = &mut self.recorder {
                    let status = recorder.status();
                    if recorder.active() {
                        ui.colored_label(GREEN, format!("Recording · {} batches", status.batches));
                        if ui.button("Stop recording").clicked() {
                            recorder.stop();
                        }
                    } else if !status.closed {
                        ui.colored_label(AMBER, "Finishing recording…");
                    }
                    if let Some(error) = &status.error {
                        ui.colored_label(AMBER, format!("Log error: {error}"));
                    }
                }
                let running = self
                    .state
                    .lock()
                    .map(|state| state.running)
                    .unwrap_or(false);
                if running {
                    ui.colored_label(
                        ACCENT,
                        if self.cancel.load(Ordering::Relaxed) {
                            "Stopping check…"
                        } else {
                            "Check running"
                        },
                    );
                    if !self.cancel.load(Ordering::Relaxed) && ui.button("Stop check").clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                        self.message = "Stopping after the current work or I/O operation…".into();
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("Readings are best effort and never leave this computer.")
                            .size(10.5)
                            .color(MUTED),
                    );
                    ui.add_space(10.0);
                    if !s.adapters.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "wgpu · {}  ·  {} adapter(s)",
                                s.gpu_backend,
                                s.adapters.len()
                            ))
                            .size(10.5)
                            .color(MUTED),
                        );
                    }
                });
            });
    }

    // ------------------------------------------------------------- overview
    fn overview(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let specs = self.specs.lock().ok().and_then(|held| (*held).clone());
        if !s.ready {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add_space(6.0);
                    ui.label(RichText::new("Collecting initial readings…").color(TEXT_DIM));
                });
                ui.label(
                    RichText::new(
                        "The first sample covers GPU discovery, which can take a moment.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            });
            return;
        }

        let memory_fraction = if s.memory_total > 0 {
            s.memory_used as f32 / s.memory_total as f32
        } else {
            0.0
        };
        let hardware_adapters = s.adapters.iter().filter(|a| !a.software).count();

        ui.columns(3, |cols| {
            metric(
                &mut cols[0],
                "PROCESSOR",
                &if s.cores.is_empty() || !s.cpu_usage.is_finite() {
                    "Unavailable".to_owned()
                } else {
                    format!("{:.0}%", s.cpu_usage)
                },
                if s.cores.is_empty() {
                    "No core data from the operating system"
                } else {
                    ""
                },
                if s.cores.is_empty() || !s.cpu_usage.is_finite() {
                    None
                } else {
                    Some(s.cpu_usage / 100.0)
                },
                load_color(s.cpu_usage / 100.0),
            );
            cols[0]
                .label(
                    RichText::new(format!(
                        "{} · {} logical cores",
                        truncate(&s.cpu_name, 28),
                        s.cores.len()
                    ))
                    .size(10.5)
                    .color(MUTED),
                )
                .on_hover_text(if s.cpu_name.is_empty() {
                    "CPU model unavailable".to_string()
                } else {
                    s.cpu_name.clone()
                });
            if let (Some(min), Some(max)) = (s.cpu_range.min, s.cpu_range.max) {
                cols[0].label(
                    RichText::new(format!("min {min:.0}% · max {max:.0}%"))
                        .size(10.0)
                        .color(MUTED),
                );
            }

            metric(
                &mut cols[1],
                "MEMORY",
                &if s.memory_total == 0 {
                    "Unavailable".to_string()
                } else {
                    format!("{:.1} / {:.1} GiB", gib(s.memory_used), gib(s.memory_total))
                },
                if s.memory_total == 0 {
                    "No memory data from the operating system"
                } else {
                    ""
                },
                (s.memory_total > 0).then_some(memory_fraction),
                VIOLET,
            );
            cols[1].label(
                RichText::new(if s.memory_total == 0 {
                    "Available memory unknown".to_owned()
                } else {
                    format!("{:.1} GiB available", gib(s.memory_available))
                })
                .size(10.5)
                .color(MUTED),
            );
            if let (Some(min), Some(max)) = (s.memory_range.min, s.memory_range.max) {
                cols[1].label(
                    RichText::new(format!("min {min:.0}% · max {max:.0}%"))
                        .size(10.0)
                        .color(MUTED),
                );
            }

            metric(
                &mut cols[2],
                "GRAPHICS",
                &if s.adapters.is_empty() {
                    "None".to_string()
                } else {
                    s.adapters.len().to_string()
                },
                if s.adapters.is_empty() {
                    "No adapter reported"
                } else if hardware_adapters == 0 {
                    "software adapters only"
                } else {
                    ""
                },
                None,
                AMBER,
            );
            cols[2].label(
                RichText::new(match s.adapters.first() {
                    Some(g) => truncate(&g.name, 34),
                    None => "no adapter reported".to_string(),
                })
                .size(10.5)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);

        // ---------------------------------------------------------- live chart
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section_title(ui, "Recent activity", Some("up to 60 samples"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Reset").size(11.5))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .corner_radius(CornerRadius::same(7)),
                        )
                        .clicked()
                    {
                        self.history.clear();
                    }
                    if let (Some((lo, hi, _)), Some((mem_lo, mem_hi, _))) = (history::range(&self.history, |v| v.cpu), history::range(&self.history, |v| v.memory)) {
                        ui.label(RichText::new(format!("CPU {lo:.0}–{hi:.0}%  ·  MEM {mem_lo:.0}–{mem_hi:.0}%")).size(10.5).color(MUTED));
                    }
                });
            });
            ui.add_space(6.0);
            if self.history.is_empty() {
                ui.allocate_ui(egui::vec2(ui.available_width(), 150.0), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Waiting for the first samples…")
                                .size(12.0)
                                .color(MUTED),
                        );
                    });
                });
            } else {
                Plot::new("history")
                    .height(158.0)
                    .legend(Legend::default())
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
                        for (i, points) in history::segments(&self.history, self.started, |v| v.cpu).into_iter().enumerate() {
                            p.line(Line::new(PlotPoints::from(points)).name(if i == 0 { "CPU %" } else { "" }).color(ACCENT).width(1.8_f32));
                        }
                        for (i, points) in history::segments(&self.history, self.started, |v| v.memory).into_iter().enumerate() {
                            p.line(Line::new(PlotPoints::from(points)).name(if i == 0 { "Memory %" } else { "" }).color(VIOLET).width(1.8_f32));
                        }
                    });
                ui.weak("Gaps indicate missing or delayed samples. Statistics cover the retained window.");
                egui::CollapsingHeader::new("Record session to CSV").show(ui, |ui| {
                    ui.weak("Optional local log: CPU, memory, temperature and fan readings, with sensor keys. No process names are recorded. A new file is required; logging stops at 1,000,000 rows.");
                    let active = self.recorder.as_ref().is_some_and(|r| r.active());
                    let finishing = self.recorder.as_ref().is_some_and(|r| !r.active() && !r.status().closed);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_enabled(!active && !finishing, egui::TextEdit::singleline(&mut self.log_path).desired_width(330.0));
                        if ui.add_enabled(!active && !finishing, egui::Button::new("Start recording")).clicked() {
                            match Recorder::start(std::path::Path::new(&self.log_path)) {
                                Ok(recorder) => { self.recorder = Some(recorder); self.message = format!("Recording to {}", self.log_path); }
                                Err(error) => self.message = format!("Recording failed: {error}"),
                            }
                        }
                        if active && ui.button("Stop recording").clicked() { if let Some(r) = &mut self.recorder { r.stop(); } }
                    });
                    if let Some(recorder) = &self.recorder {
                        let status = recorder.status();
                        let state = if recorder.active() { "Recording" } else if status.closed { "Stopped" } else { "Finishing" };
                        ui.label(format!("{state} · {} batches written · {} dropped", status.batches, status.dropped));
                        if let Some(error) = status.error { ui.colored_label(AMBER, format!("Recording error: {error}")); }
                    }
                    ui.weak("Times are when the app recorded each batch, not individual sensor timestamps. Disk errors or forced termination may leave a partial file.");
                });
                egui::CollapsingHeader::new("Export recent history").show(ui, |ui| {
                    ui.weak("CSV contains up to 60 samples, with elapsed seconds since launch. Missing values are blank; existing files are never overwritten.");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.history_path);
                        if ui.button("Export CSV").clicked() {
                            self.message = match history::export_csv(std::path::Path::new(&self.history_path), &self.history, self.started) {
                                Ok(()) => format!("Saved {}", self.history_path),
                                Err(e) => format!("Export failed: {e}"),
                            };
                        }
                    });
                });
            }
        });

        ui.add_space(12.0);

        // ------------------------------------------------------------ core grid
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Per-core load",
                Some(&format!("{} logical cores", s.cores.len())),
            );
            ui.add_space(4.0);
            if s.cores.is_empty() {
                empty_note(ui, "This provider does not report per-core usage.");
            } else {
                let width = ui.available_width();
                let columns = ((width / 168.0).floor() as usize).clamp(1, 6);
                egui::Grid::new("cores")
                    .num_columns(columns)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        for (i, usage) in s.cores.iter().enumerate() {
                            core_meter(ui, i, *usage);
                            if (i + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            }
        });

        ui.add_space(12.0);

        // ------------------------------------------------------------- sensors
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section_title(
                    ui,
                    "Sensors",
                    Some(&format!(
                        "{} temperatures · {} fans · min/max since launch",
                        s.temperatures.len(),
                        s.fans.len()
                    )),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Reset min/max").size(11.5))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .corner_radius(CornerRadius::same(7)),
                        )
                        .clicked()
                    {
                        self.stats_reset.store(true, Ordering::Relaxed);
                    }
                });
            });
            ui.add_space(8.0);
            if s.temperatures.is_empty() && s.fans.is_empty() {
                empty_note(
                    ui,
                    "No sensor provider is available here. The Details page lists what this platform exposes.",
                );
            }
            let temp_rows: Vec<RowView<'_>> = s
                .temperatures
                .iter()
                .map(|t| RowView {
                    label: &t.label,
                    key: &t.key,
                    current: t.celsius,
                    min: t.min,
                    max: t.max,
                    unit: "°C",
                })
                .collect();
            if !temp_rows.is_empty() {
                sensor_table(ui, "temperature-sensors", "Temperature", temp_rows);
            }
            let fan_rows: Vec<RowView<'_>> = s
                .fans
                .iter()
                .map(|f| RowView {
                    label: &f.label,
                    key: &f.key,
                    current: f.rpm,
                    min: f.min,
                    max: f.max,
                    unit: "RPM",
                })
                .collect();
            if !fan_rows.is_empty() {
                ui.add_space(10.0);
                sensor_table(ui, "fan-sensors", "Fan speed", fan_rows);
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Hover a sensor for the raw provider key. Min/max are the observed extremes since launch or the last reset, not a specification.",
                )
                .size(10.0)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);

        // -------------------------------------------------------------- devices
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Graphics adapters and sensor sources",
                Some("Sources may overlap; mappings are not verified"),
            );
            ui.add_space(6.0);
            if s.gpus.is_empty() {
                empty_note(
                    ui,
                    "No adapter detected, or graphics discovery is unavailable.",
                );
            }
            for g in &s.gpus {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&g.name).strong().color(TEXT).size(12.5));
                    if g.detail.contains("software") {
                        chip(ui, "SOFTWARE", AMBER);
                    }
                });
                ui.label(RichText::new(&g.detail).size(11.0).color(MUTED));
                ui.add_space(6.0);
            }
        });

        ui.add_space(12.0);

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Storage volumes", Some("volume capacity, not physical drives"));
            ui.add_space(6.0);
            if s.disks.is_empty() {
                empty_note(ui, "No volumes available.");
            }
            for d in &s.disks {
                let used = d.total.saturating_sub(d.available);
                let fraction = if d.total > 0 {
                    used as f32 / d.total as f32
                } else {
                    0.0
                };
                let health = specs
                    .as_ref()
                    .and_then(|known| known.volume_health(&d.mount));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&d.mount).strong().color(TEXT).size(12.5));
                    chip(ui, &d.kind.to_uppercase(), MUTED);
                    if let Some(health) = health {
                        if !health.smart.is_empty() {
                            chip(
                                ui,
                                &format!("SMART {}", health.smart.to_uppercase()),
                                smart_color(&health.smart),
                            );
                        }
                        if let Some(solid) = health.solid_state {
                            chip(ui, if solid { "SSD" } else { "HDD" }, MUTED);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.0}% used", fraction * 100.0))
                                .size(11.0)
                                .color(if fraction > 0.9 { AMBER } else { TEXT_DIM }),
                        );
                    });
                });
                bar(
                    ui,
                    fraction,
                    if fraction > 0.9 { AMBER } else { ACCENT },
                    6.0,
                );
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if d.name.is_empty() {
                            format!(
                                "{:.1} GiB free of {:.1} GiB",
                                gib(d.available),
                                gib(d.total)
                            )
                        } else {
                            format!(
                                "{} · {:.1} GiB free of {:.1} GiB",
                                d.name,
                                gib(d.available),
                                gib(d.total)
                            )
                        })
                        .size(10.5)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "read {}  ·  write {}",
                                bytes(d.read),
                                bytes(d.written)
                            ))
                            .size(10.5)
                            .color(MUTED),
                        )
                        .on_hover_text(
                            "Bytes since the last refresh (about one second) when the OS provides it; zero can mean unsupported.",
                        );
                    });
                });
                ui.add_space(8.0);
            }
        });

        ui.add_space(12.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(
                    "Sensor labels and device associations may be ambiguous. Availability varies by source and device; no reading triggers an automatic stop.",
                )
                .size(10.5)
                .color(MUTED),
            );
        });
        ui.add_space(8.0);
    }

    fn hardware_page(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let specs = self.specs.lock().ok().and_then(|held| (*held).clone());
        let page = self.page;
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            match page {
                Page::Cpu => {
                    section_title(ui, "Processor", Some("live usage · OS-reported specifications"));
                    if snapshot.cores.is_empty() { ui.weak("CPU usage unavailable."); }
                    else { ui.label(format!("Current: {:.0}% · {} logical cores", snapshot.cpu_usage, snapshot.cores.len())); }
                    if let (Some(min), Some(max)) = (snapshot.cpu_range.min, snapshot.cpu_range.max) { ui.weak(format!("Observed since reset: {min:.0}% min · {max:.0}% max")); }
                    ui.label(&snapshot.cpu_name);
                    if let Some(specs) = &specs {
                        ui.weak(format!("{} · {} · {} physical / {} logical cores", specs.cpu.vendor, specs.cpu.architecture, specs.cpu.physical_cores.map(|n| n.to_string()).unwrap_or_else(|| "unknown".into()), specs.cpu.logical_cores.map(|n| n.to_string()).unwrap_or_else(|| "unknown".into())));
                        if let Some(cache) = specs.cpu.l3_bytes { ui.weak(format!("L3 cache: {}", bytes(cache))); }
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| { for (i, v) in snapshot.cores.iter().enumerate() { ui.label(format!("Core {i}: {v:.0}%")); } });
                    egui::CollapsingHeader::new("Recent CPU activity").show(ui, |ui| {
                        Plot::new("cpu-page-history").height(120.0).include_y(0.0).include_y(100.0).allow_drag(false).allow_zoom(false).allow_scroll(false).show_x(false).show(ui, |plot| {
                            for points in history::segments(&self.history, self.started, |sample| sample.cpu) { plot.line(Line::new(PlotPoints::from(points)).color(ACCENT)); }
                        });
                        ui.weak("Up to 60 batches; gaps mean missing or delayed readings.");
                    });
                    let sensors: Vec<_> = snapshot.temperatures.iter().filter(|t| t.label.to_lowercase().contains("cpu")).collect();
                    if !sensors.is_empty() {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(format!("CPU-labelled temperature sensors ({})", sensors.len())).show(ui, |ui| {
                            ui.weak("Temperature labels are provider-supplied; device association is not verified.");
                            let rows = sensors.iter().map(|t| RowView { label: &t.label, key: &t.key, current: t.celsius, min: t.min, max: t.max, unit: "°C" }).collect();
                            sensor_table(ui, "cpu-page-sensors", "CPU-labelled temperatures", rows);
                        });
                    }
                }
                Page::Gpu => {
                    section_title(ui, "Graphics", Some("adapter inventory and independent sensor sources"));
                    if snapshot.gpus.is_empty() { ui.weak("No graphics source reported."); }
                    for gpu in &snapshot.gpus { ui.strong(&gpu.name); ui.label(&gpu.detail); ui.add_space(4.0); }
                    ui.weak("Sources may overlap; IORegistry and name-based matches do not prove adapter identity.");
                    if ui.button("Open frame analysis").clicked() { self.page = Page::Frames; }
                }
                Page::Memory => {
                    section_title(ui, "Memory", Some("OS-reported"));
                    if snapshot.memory_total == 0 { ui.weak("Memory usage unavailable."); }
                    else { ui.label(format!("{:.1} / {:.1} GiB used · {:.1} GiB available", gib(snapshot.memory_used), gib(snapshot.memory_total), gib(snapshot.memory_available))); }
                    egui::CollapsingHeader::new("Recent memory activity").show(ui, |ui| {
                        Plot::new("memory-page-history").height(120.0).include_y(0.0).include_y(100.0).allow_drag(false).allow_zoom(false).allow_scroll(false).show_x(false).show(ui, |plot| {
                            for points in history::segments(&self.history, self.started, |sample| sample.memory) { plot.line(Line::new(PlotPoints::from(points)).color(VIOLET)); }
                        });
                        ui.weak("Up to 60 batches; gaps mean missing or delayed readings.");
                    });
                    if let Some(specs) = &specs {
                        if specs.memory.modules.is_empty() { ui.weak("Module details not reported."); }
                        for module in &specs.memory.modules { ui.label(format!("{} · {} · {} · {}", module.slot, module.kind, module.speed_mhz.map(|n| format!("{n} MHz")).unwrap_or_else(|| "speed unavailable".into()), module.size_bytes.map(decimal_bytes).unwrap_or_else(|| "size unavailable".into()))); }
                    }
                }
                Page::Storage => {
                    section_title(ui, "Storage", Some("mounted volumes and OS-reported drives"));
                    if snapshot.disks.is_empty() { ui.weak("No mounted volumes reported."); }
                    for disk in &snapshot.disks { ui.label(format!("{} · {} · {:.1} / {:.1} GiB free", disk.mount, disk.name, gib(disk.available), gib(disk.total))); }
                    if let Some(specs) = &specs { for drive in &specs.drives { ui.weak(format!("{} · SMART: {}", drive.model, if drive.smart.is_empty() { "unavailable" } else { &drive.smart })); } }
                    ui.weak("A volume is not necessarily a physical drive. Imported SMART reports are not live or automatically associated.");
                    if ui.button("Open detailed specifications").clicked() { self.page = Page::Details; }
                }
                Page::Thermal => {
                    section_title(ui, "Thermal sensors and fans", Some("observed min/max since launch or reset"));
                    if ui.button("Reset min/max").clicked() { self.stats_reset.store(true, Ordering::Relaxed); }
                    if snapshot.temperatures.is_empty() && snapshot.fans.is_empty() { ui.weak("No temperature or fan sensors were reported."); }
                    let temps = snapshot.temperatures.iter().map(|t| RowView { label: &t.label, key: &t.key, current: t.celsius, min: t.min, max: t.max, unit: "°C" }).collect::<Vec<_>>();
                    if !temps.is_empty() { sensor_table(ui, "thermal-page-temperatures", "Temperatures", temps); }
                    let fans = snapshot.fans.iter().map(|f| RowView { label: &f.label, key: &f.key, current: f.rpm, min: f.min, max: f.max, unit: "RPM" }).collect::<Vec<_>>();
                    if !fans.is_empty() { ui.add_space(8.0); sensor_table(ui, "thermal-page-fans", "Fans", fans); }
                    ui.weak("Sensor labels and device mappings may be ambiguous. No automatic thermal shutdown or fan control is provided; a thermal stress test is unavailable.");
                }
                Page::Motherboard => {
                    section_title(ui, "Motherboard", Some("availability depends on the platform"));
                    if let Some(specs) = &specs { ui.label(format!("System model: {} · {}", specs.model_name, specs.model_id)); ui.weak(format!("System firmware: {}", if specs.firmware.is_empty() { "unavailable" } else { &specs.firmware })); ui.label(format!("OS-reported board ID: {}", specs.board_id.as_deref().unwrap_or("unavailable"))); }
                    else { ui.weak("Collecting system information…"); }
                    ui.weak("Motherboard model, chipset, voltage and board-specific stress tests are unavailable in this build. The board ID is an OS registry identifier, not a model name.");
                }
                Page::Network => {
                    section_title(ui, "Network interfaces", Some("OS counters · rates since the previous refresh"));
                    if let Some(wifi) = specs.as_ref().and_then(|s| s.wifi_interface.as_ref()) { ui.label(format!("Wi-Fi interface at startup: {wifi}")); }
                    else { ui.weak("Wi-Fi interface not identified by the OS at startup."); }
                    if snapshot.networks.is_empty() { ui.weak("No interfaces reported."); }
                    for n in &snapshot.networks {
                        let rate = |bytes: u64| match (bytes, snapshot.network_interval_seconds) { (0, _) => "0 or unavailable".into(), (_, Some(seconds)) if seconds > 0.0 => format!("{:.1} KiB/s", bytes as f64 / seconds / 1024.0), _ => "unavailable".into() };
                        ui.label(format!("{} · received {} · sent {}", n.name, rate(n.received), rate(n.transmitted)));
                    }
                    ui.weak("The OS may include virtual interfaces. Wi-Fi mapping is collected once at startup; link quality and network stress testing are unavailable.");
                    if ui.button("Open process usage").clicked() { self.page = Page::Processes; }
                }
                Page::Bluetooth => {
                    section_title(ui, "Bluetooth", Some("read-only controller diagnostic"));
                    let (running, attempted, refreshed) = self.bluetooth_refresh.lock().map(|s| (s.running, s.attempted, s.value.clone())).unwrap_or_default();
                    let info = if attempted { refreshed.as_ref() } else { specs.as_ref().and_then(|s| s.bluetooth.as_ref()) };
                    if let Some(info) = info {
                        ui.label(format!("Controller state: {}", match info.state { Some(true) => "on", Some(false) => "off", None => "unavailable" }));
                        if !info.transport.is_empty() { ui.label(format!("Transport: {}", info.transport)); }
                        if !info.chipset.is_empty() { ui.label(format!("Chipset (OS-reported): {}", info.chipset)); }
                        match &info.connected_devices {
                            Some(devices) if devices.is_empty() => { ui.weak("No connected devices were listed in this snapshot."); }
                            Some(devices) => { ui.weak("Connected devices reported by the OS (up to 32):"); for device in devices { ui.label(device); } }
                            None => { ui.weak("Connected-device list not reported."); }
                        }
                    } else { ui.weak("Bluetooth controller status is unavailable or not reported by the OS."); }
                    if ui.add_enabled(!running, egui::Button::new(if running { "Checking…" } else { "Check controller again" })).clicked() {
                        if let Ok(mut state) = self.bluetooth_refresh.lock() { state.running = true; }
                        let refresh = self.bluetooth_refresh.clone();
                        std::thread::spawn(move || {
                            let value = specs::bluetooth_status();
                            if let Ok(mut state) = refresh.lock() { state.value = value; state.attempted = true; state.running = false; }
                        });
                    }
                    ui.weak(if attempted { "Last controller check is a snapshot, not live status." } else { "Initial controller report was collected at launch." });
                    ui.weak("This reads OS-reported status only. Signal quality and Bluetooth stress testing are unavailable.");
                }
                _ => {}
            }
        });
        if page == Page::Network {
            ui.add_space(12.0);
            self.network_diagnostic(ui);
        }
        let check = match page {
            Page::Cpu => Some(Kind::Cpu),
            Page::Gpu => Some(Kind::Gpu),
            Page::Memory => Some(Kind::Memory),
            Page::Storage => Some(Kind::Disk),
            _ => None,
        };
        if let Some(kind) = check {
            ui.add_space(12.0);
            let running = self.state.lock().map(|s| s.running).unwrap_or(false);
            if running && self.kind != kind {
                card_frame().show(ui, |ui| {
                    ui.label(format!(
                        "A {} check is already running. Only one check can run at a time.",
                        kind_label(self.kind)
                    ));
                    if ui.button("View current check").clicked() {
                        self.page = Page::Tests;
                    }
                });
            } else {
                if !running && self.kind != kind {
                    if kind == Kind::Disk {
                        self.mib = self.mib.min(64);
                        self.disk_write_limit_mib = self.disk_write_limit_mib.max(self.mib as u64);
                    }
                    self.kind = kind;
                }
                self.tests(ui, snapshot, Some(kind));
            }
            if page == Page::Storage {
                ui.add_space(12.0);
                egui::CollapsingHeader::new("Import S.M.A.R.T. report")
                    .show(ui, |ui| self.smart_report_ui(ui));
            }
            ui.add_space(12.0);
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
                section_title(ui, "Recent checks", Some("this session"));
                if records.is_empty() {
                    ui.weak("No checks for this device in this session.");
                }
                for r in records {
                    ui.label(format!(
                        "{} · {} · {:.1}s · {}",
                        r.status,
                        timestamp_text(r.timestamp),
                        r.elapsed,
                        r.detail
                    ));
                }
                if ui.button("View all reports").clicked() {
                    self.page = Page::Reports;
                }
            });
        }
    }

    fn network_diagnostic(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "TCP connection check", Some("optional · up to four attempts"));
            ui.weak("Enter a numeric IP address and port. Each attempt opens and closes a TCP connection without sending an application payload.");
            let state = self.network_check.lock().map(|s| s.clone()).unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled(!state.running, egui::TextEdit::singleline(&mut self.network_target).desired_width(260.0).hint_text("e.g. 127.0.0.1:80"));
                if ui.add_enabled(!state.running, egui::Button::new("Start check")).clicked() {
                    self.network_message = match netcheck::start(&self.network_target, self.network_check.clone(), self.network_cancel.clone()) { Ok(()) => String::new(), Err(e) => e };
                }
                if state.running && ui.button("Stop check").clicked() { self.network_cancel.store(true, Ordering::Relaxed); }
            });
            if !self.network_message.is_empty() { ui.colored_label(AMBER, &self.network_message); }
            if state.running { ui.label(format!("Checking… {} attempts completed", state.attempted)); }
            else if state.attempted > 0 {
                ui.label(format!("{} of {} connections succeeded{}", state.succeeded, state.attempted, if state.stopped { " · stopped" } else { "" }));
                if state.succeeded > 0 { ui.label(format!("Mean successful TCP connect time: {:.1} ms", state.total_ms / state.succeeded as f64)); }
                if let Some(error) = state.last_error { ui.weak(format!("Most recent error: {error}")); }
            }
            ui.weak("This checks reachability to the chosen endpoint. It is not a Wi-Fi stress, throughput, or radio test; a refused connection does not prove a hardware fault.");
        });
    }

    // ---------------------------------------------------------------- tests
    fn tests(&mut self, ui: &mut egui::Ui, s: &Snapshot, fixed: Option<Kind>) {
        let running = self.state.lock().map(|s| s.running).unwrap_or(false);
        ui.columns(2, |cols| {
            // ------------------------------------------------------- configuration
            card_frame().show(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Check", Some(if running { "locked while running" } else if fixed.is_some() { "selected for this device" } else { "choose one" }));
                ui.add_space(8.0);
                ui.add_enabled_ui(!running, |ui| {
                    if fixed.is_none() {
                    for pair in KINDS.chunks(2) {
                        ui.columns(2, |tiles| {
                            for (slot, (kind, label, hint)) in pair.iter().enumerate() {
                                if kind_tile(&mut tiles[slot], self.kind == *kind, label, hint)
                                    .clicked()
                                {
                                    self.kind = *kind;
                                    if *kind == Kind::Disk {
                                        self.mib = self.mib.min(64);
                                        self.disk_write_limit_mib =
                                            self.disk_write_limit_mib.max(self.mib as u64);
                                    }
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }
                    } else {
                        ui.label(RichText::new(kind_label(self.kind)).strong().color(TEXT));
                    }
                    ui.add_space(6.0);
                    match self.kind {
                        Kind::Cpu => {
                            ui.label(
                                RichText::new(
                                    "Repeated integer calculations compared against a fixed known answer. Not a hardware certification.",
                                )
                                .size(11.0)
                                .color(TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            ui.add(egui::Slider::new(&mut self.workers, 1..=8).text("workers"));
                        }
                        Kind::Gpu => {
                            ui.label(
                                RichText::new(
                                    "Light compute validation with readback, not a full GPU or VRAM stress test. Software adapters cannot be tested.",
                                )
                                .size(11.0)
                                .color(TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            if s.adapters.is_empty() {
                                empty_note(ui, "No GPU adapter is available.");
                            } else {
                                self.gpu_index = self.gpu_index.min(s.adapters.len() - 1);
                                let selected = &s.adapters[self.gpu_index];
                                let label = format!(
                                    "{} · {}{}",
                                    selected.name,
                                    selected.backend,
                                    if selected.software { " · software" } else { "" }
                                );
                                egui::ComboBox::from_id_salt("gpu-adapter")
                                    .width(ui.available_width() - 8.0)
                                    .selected_text(RichText::new(label).size(11.5))
                                    .show_ui(ui, |ui| {
                                        for (i, a) in s.adapters.iter().enumerate() {
                                            let text = format!(
                                                "{i}: {} · {}{}",
                                                a.name,
                                                a.backend,
                                                if a.software { " · software" } else { "" }
                                            );
                                            ui.selectable_value(&mut self.gpu_index, i, text);
                                        }
                                    });
                            }
                        }
                        Kind::Memory => {
                            ui.label(
                                RichText::new(format!(
                                    "Alternating patterns, limited to 256 MiB and one quarter of available RAM ({:.0} MiB).",
                                    s.memory_available as f64 / 4.0 / 1048576.0
                                ))
                                .size(11.0)
                                .color(TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            ui.add(egui::Slider::new(&mut self.mib, 1..=256).text("MiB"));
                        }
                        Kind::Disk => {
                            ui.label(
                                RichText::new(
                                    "Creates and verifies a new temporary file. Writes cause SSD wear; maximum 1 GiB per run.",
                                )
                                .size(11.0)
                                .color(AMBER),
                            );
                            ui.add_space(4.0);
                            ui.label(RichText::new("Temporary directory").size(11.0).color(MUTED));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.directory)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.add_space(4.0);
                            ui.add(egui::Slider::new(&mut self.mib, 1..=256).text("file MiB"));
                            self.disk_write_limit_mib =
                                self.disk_write_limit_mib.max(self.mib as u64);
                            ui.add(
                                egui::Slider::new(
                                    &mut self.disk_write_limit_mib,
                                    self.mib as u64..=1024,
                                )
                                .text("write cap MiB"),
                            );
                        }
                    }
                    ui.add_space(6.0);
                    ui.add(egui::Slider::new(&mut self.seconds, 1..=600).text("max seconds"));
                    ui.add_space(10.0);
                    let start = ui.add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new(RichText::new("Start test").strong().color(Color32::from_rgb(9, 18, 30)))
                            .fill(ACCENT)
                            .corner_radius(CornerRadius::same(9)),
                    );
                    if start.clicked() {
                        let adapter = if self.kind == Kind::Gpu {
                            s.adapters.get(self.gpu_index)
                        } else {
                            None
                        };
                        let gpu_name =
                            adapter.map(|a| format!("{} · {}", a.name, a.backend));
                        let cfg = Config {
                            kind: self.kind,
                            seconds: self.seconds,
                            workers: self.workers,
                            mib: self.mib,
                            directory: PathBuf::from(&self.directory),
                            gpu_adapter: gpu_name,
                            disk_write_limit_mib: self.disk_write_limit_mib,
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
                            os: sysinfo::System::long_os_version()
                                .unwrap_or_else(|| "unavailable".into()),
                            snapshot_age_seconds: s
                                .updated_at
                                .map(|t| t.elapsed().as_secs_f64()),
                        };
                        self.message = match stress::start(
                            cfg,
                            self.state.clone(),
                            self.cancel.clone(),
                            adapter.map(|a| a.handle.clone()),
                            context,
                        ) {
                            Ok(()) => "Test started".into(),
                            Err(e) => e,
                        };
                    }
                });
            });

            // ------------------------------------------------------------ progress
            card_frame().show(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Current run", Some(if running { "in progress" } else { "idle" }));
                ui.add_space(10.0);
                if running {
                    let elapsed = self
                        .state
                        .lock()
                        .ok()
                        .and_then(|s| s.started)
                        .map(|t| t.elapsed().as_secs_f32())
                        .unwrap_or_default();
                    let fraction = (elapsed / self.seconds as f32).min(1.0);
                    ui.label(
                        RichText::new(format!("{elapsed:.1}s"))
                            .size(30.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(format!("of the {}s limit", self.seconds))
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.add_space(6.0);
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .fill(ACCENT)
                            .corner_radius(CornerRadius::same(6))
                            .desired_height(8.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Live CPU").size(11.0).color(MUTED));
                        ui.label(
                            RichText::new(format!("{:.0}%", s.cpu_usage))
                                .size(12.0)
                                .strong()
                                .color(load_color(s.cpu_usage / 100.0)),
                        );
                        ui.add_space(12.0);
                        ui.label(RichText::new("Memory").size(11.0).color(MUTED));
                        ui.label(
                            RichText::new(if s.memory_total > 0 {
                                format!(
                                    "{:.0}%",
                                    s.memory_used as f32 * 100.0 / s.memory_total as f32
                                )
                            } else {
                                "—".into()
                            })
                            .size(12.0)
                            .strong()
                            .color(VIOLET),
                        );
                    });
                    ui.add_space(12.0);
                    if ui
                        .add_sized(
                            [ui.available_width(), 32.0],
                            egui::Button::new(RichText::new("Stop test").color(Color32::from_rgb(30, 18, 6)))
                                .fill(AMBER)
                                .corner_radius(CornerRadius::same(9)),
                        )
                        .clicked()
                    {
                        self.cancel.store(true, Ordering::Relaxed);
                        self.message = "Stopping after the current work or I/O operation…".into();
                    }
                } else {
                    let last = self
                        .state
                        .lock()
                        .ok()
                        .and_then(|s| s.results.iter().rev().find(|r| fixed.is_none_or(|kind| r.kind == kind)).cloned());
                    match last {
                        Some(r) => {
                            ui.horizontal(|ui| {
                                chip(ui, &r.status.to_string().to_uppercase(), status_color(&r.status));
                                ui.label(
                                    RichText::new(kind_label(r.kind))
                                        .size(12.0)
                                        .color(TEXT),
                                );
                            });
                            ui.add_space(8.0);
                            ui.label(RichText::new(&r.detail).size(11.5).color(TEXT_DIM));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!(
                                    "{} verified {} · {:.1}s",
                                    r.verified_units
                                        .map(|n| n.to_string())
                                        .unwrap_or_else(|| "unknown".into()),
                                    r.verified_unit,
                                    r.elapsed
                                ))
                                .size(10.5)
                                .color(MUTED),
                            );
                        }
                        None => {
                            empty_note(
                                ui,
                                "Nothing has run yet. Pick a check on the left, review its limits, then start it.",
                            );
                        }
                    }
                }
                ui.add_space(12.0);
                ui.label(
                    RichText::new(
                        "There is no automatic temperature-based shutdown. A blocked driver or disk operation can delay stopping.",
                    )
                    .size(10.5)
                    .color(MUTED),
                );
            });
        });
        ui.add_space(8.0);
    }

    // -------------------------------------------------------------- details
    // CPU-Z-style specification sheet: identity, caches, memory modules, adapter
    // driver details and CrystalDiskInfo-style drive health, from local sources.
    fn details(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let specs = self.specs.lock().ok().and_then(|held| (*held).clone());
        let Some(specs) = specs else {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add_space(6.0);
                    ui.label(RichText::new("Reading the system inventory…").color(TEXT_DIM));
                });
                ui.label(
                    RichText::new(
                        "One read-only pass over sysctl, system_profiler, diskutil and pmset. Nothing is uploaded.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            });
            return;
        };

        let mut system_rows: Vec<(String, String)> = Vec::new();
        spec_row(&mut system_rows, "Model", specs.model_name.as_str());
        spec_row(
            &mut system_rows,
            "Model identifier",
            specs.model_id.as_str(),
        );
        spec_row(
            &mut system_rows,
            "Operating system",
            format!("{} {}", specs.os_name, specs.os_version),
        );
        spec_row(&mut system_rows, "Kernel", specs.kernel.as_str());
        spec_row(&mut system_rows, "System firmware", specs.firmware.as_str());
        spec_row(
            &mut system_rows,
            "Uptime",
            uptime_text(sysinfo::System::uptime()),
        );
        if let Some(total) = specs.memory.total_bytes {
            spec_row(&mut system_rows, "Installed memory", decimal_bytes(total));
        }
        if let Some(battery) = &specs.battery {
            spec_row(&mut system_rows, "Battery", battery.as_str());
        }

        let mut cpu_rows: Vec<(String, String)> = Vec::new();
        spec_row(&mut cpu_rows, "Processor", specs.cpu.name.as_str());
        spec_row(&mut cpu_rows, "Vendor", vendor_text(&specs.cpu.vendor));
        spec_row(
            &mut cpu_rows,
            "Architecture",
            specs.cpu.architecture.as_str(),
        );
        match (specs.cpu.physical_cores, specs.cpu.logical_cores) {
            (Some(physical), Some(logical)) => spec_row(
                &mut cpu_rows,
                "Cores",
                format!("{physical} physical · {logical} logical"),
            ),
            (None, Some(logical)) => spec_row(&mut cpu_rows, "Cores", format!("{logical} logical")),
            (Some(physical), None) => {
                spec_row(&mut cpu_rows, "Cores", format!("{physical} physical"))
            }
            (None, None) => {}
        }
        if let Some(cores) = specs.cpu.performance_cores {
            spec_row(&mut cpu_rows, "Performance cores", cores.to_string());
        }
        if let Some(cores) = specs.cpu.efficiency_cores {
            spec_row(&mut cpu_rows, "Efficiency cores", cores.to_string());
        }
        if let Some(mhz) = specs.cpu.base_mhz {
            spec_row(&mut cpu_rows, "Base clock", clock_text(mhz));
        }
        if let Some(mhz) = specs.cpu.max_mhz {
            spec_row(&mut cpu_rows, "Max clock", clock_text(mhz));
        }
        if let Some(value) = specs.cpu.l1_instruction_bytes {
            spec_row(&mut cpu_rows, "L1 instruction cache", bytes(value));
        }
        if let Some(value) = specs.cpu.l1_data_bytes {
            spec_row(&mut cpu_rows, "L1 data cache", bytes(value));
        }
        if let Some(value) = specs.cpu.l2_bytes {
            spec_row(&mut cpu_rows, "L2 cache", bytes(value));
        }
        if let Some(value) = specs.cpu.l3_bytes {
            spec_row(&mut cpu_rows, "L3 cache", bytes(value));
        }
        if let Some(value) = specs.cpu.cache_line_bytes {
            spec_row(&mut cpu_rows, "Cache line", bytes(value));
        }
        match (specs.cpu.physical_cores, specs.cpu.logical_cores) {
            (Some(physical), Some(logical)) if logical > physical => {
                spec_row(&mut cpu_rows, "Simultaneous threads", "Enabled")
            }
            (Some(_), Some(_)) => spec_row(&mut cpu_rows, "Simultaneous threads", "Not enabled"),
            _ => {}
        }

        let mut memory_rows: Vec<(String, String)> = Vec::new();
        if let Some(ecc) = specs.memory.ecc {
            spec_row(
                &mut memory_rows,
                "ECC",
                if ecc { "Enabled" } else { "Disabled" },
            );
        }
        if let Some(upgradeable) = specs.memory.upgradeable {
            spec_row(
                &mut memory_rows,
                "Upgradeable",
                if upgradeable { "Yes" } else { "No" },
            );
        }
        if specs.memory.modules.is_empty() {
            spec_row(
                &mut memory_rows,
                "Modules",
                "Not reported; unified or soldered memory has no module records.",
            );
        }
        for module in &specs.memory.modules {
            let mut detail = String::new();
            if let Some(size) = module.size_bytes {
                detail.push_str(&decimal_bytes(size));
            }
            if !module.kind.is_empty() {
                detail.push_str(&format!(" {}", module.kind));
            }
            if let Some(speed) = module.speed_mhz {
                detail.push_str(&format!(" · {speed} MHz"));
            }
            if !module.manufacturer.is_empty() {
                detail.push_str(&format!(" · {}", module.manufacturer));
            }
            if !module.part_number.is_empty() {
                detail.push_str(&format!(" · {}", module.part_number));
            }
            let slot = if module.slot.is_empty() {
                "Module".to_string()
            } else {
                module.slot.clone()
            };
            spec_row(&mut memory_rows, slot, detail);
        }

        let mut graphics_rows: Vec<(String, String)> = Vec::new();
        for (index, adapter) in s.adapters.iter().enumerate() {
            spec_row(
                &mut graphics_rows,
                format!("Adapter {index}"),
                format!("{} · {}", adapter.name, adapter.backend),
            );
            spec_row(
                &mut graphics_rows,
                "Driver",
                format!("{} {}", adapter.driver, adapter.driver_info),
            );
            spec_row(
                &mut graphics_rows,
                "Vendor / device",
                format!("0x{:04x} / 0x{:04x}", adapter.vendor_id, adapter.device_id),
            );
            spec_row(
                &mut graphics_rows,
                "Adapter type",
                if adapter.software {
                    "Software"
                } else {
                    "Hardware"
                },
            );
        }
        for gpu in &specs.graphics {
            spec_row(&mut graphics_rows, "System profile", gpu.name.as_str());
            if let Some(vram) = gpu.vram_bytes {
                spec_row(&mut graphics_rows, "VRAM (reported)", decimal_bytes(vram));
            }
            spec_row(&mut graphics_rows, "API", gpu.api.as_str());
            spec_row(&mut graphics_rows, "Bus", gpu.bus.as_str());
            spec_row(&mut graphics_rows, "Device ID", gpu.device_id.as_str());
        }

        let mut storage_rows: Vec<(String, String)> = Vec::new();
        for volume in &specs.volumes {
            let mut detail = String::new();
            if !volume.protocol.is_empty() {
                detail.push_str(&volume.protocol);
            }
            if let Some(solid) = volume.solid_state {
                if !detail.is_empty() {
                    detail.push_str(" · ");
                }
                detail.push_str(if solid { "Solid state" } else { "Rotational" });
            }
            if !volume.smart.is_empty() {
                if !detail.is_empty() {
                    detail.push_str(" · ");
                }
                detail.push_str(&format!("S.M.A.R.T. {}", volume.smart));
            }
            spec_row(&mut storage_rows, volume.mount.clone(), detail);
        }
        for (index, drive) in specs.drives.iter().enumerate() {
            let mut detail = drive.model.clone();
            if !drive.firmware.is_empty() {
                detail.push_str(&format!(" · firmware {}", drive.firmware));
            }
            if !drive.link.is_empty() {
                detail.push_str(&format!(" · link {}", drive.link));
            }
            if !drive.smart.is_empty() {
                detail.push_str(&format!(" · S.M.A.R.T. {}", drive.smart));
            }
            match drive.trim {
                Some(true) => detail.push_str(" · TRIM"),
                Some(false) => detail.push_str(" · no TRIM"),
                None => {}
            }
            spec_row(&mut storage_rows, format!("Drive {}", index + 1), detail);
        }

        ui.columns(2, |cols| {
            card_frame().show(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "System", Some("operating-system report"));
                ui.add_space(6.0);
                spec_grid(ui, "spec-system", &system_rows);
            });
            card_frame().show(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Processor", Some("identification and caches"));
                ui.add_space(6.0);
                spec_grid(ui, "spec-cpu", &cpu_rows);
            });
        });

        ui.add_space(12.0);

        ui.columns(2, |cols| {
            card_frame().show(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Memory", Some("installed modules"));
                ui.add_space(6.0);
                spec_grid(ui, "spec-memory", &memory_rows);
            });
            card_frame().show(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Graphics", Some("wgpu adapter and system profile"));
                ui.add_space(6.0);
                if graphics_rows.is_empty() {
                    empty_note(ui, "No adapter was reported by wgpu or the system profile.");
                }
                spec_grid(ui, "spec-graphics", &graphics_rows);
            });
        });

        ui.add_space(12.0);

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Storage health",
                Some("volume state reported by the OS; not a drive diagnosis"),
            );
            ui.add_space(6.0);
            if storage_rows.is_empty() {
                empty_note(ui, "No volume or drive health was reported.");
            }
            spec_grid(ui, "spec-storage", &storage_rows);
        });

        ui.add_space(12.0);
        self.smart_report_ui(ui);

        if !specs.unavailable.is_empty() {
            ui.add_space(12.0);
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Not available", Some("left blank rather than guessed"));
                ui.add_space(4.0);
                for note in &specs.unavailable {
                    ui.label(RichText::new(format!("• {note}")).size(10.5).color(MUTED));
                }
            });
        }

        ui.add_space(10.0);
        ui.label(
            RichText::new(format!(
                "Sources: {}. Collected once at startup; re-open the app to re-read it.",
                specs.sources
            ))
            .size(10.0)
            .color(MUTED),
        );
        ui.add_space(8.0);
    }

    fn smart_report_ui(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Import S.M.A.R.T. report", Some("optional · smartctl JSON"));
            ui.weak("Open a locally saved smartctl -a -j report to inspect ATA attributes or NVMe health fields. This app does not run smartctl or access a drive for this view.");
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.smart_path).desired_width(380.0).hint_text("Path to smartctl JSON report"));
                if ui.button("Open report").clicked() {
                    match smart::read(std::path::Path::new(&self.smart_path)) {
                        Ok(report) => { self.smart_report = Some(report); self.smart_message.clear(); }
                        Err(error) => { self.smart_report = None; self.smart_message = error; }
                    }
                }
                if self.smart_report.is_some() && ui.button("Clear").clicked() { self.smart_report = None; }
            });
            if !self.smart_message.is_empty() { ui.colored_label(AMBER, &self.smart_message); }
            if let Some(report) = &self.smart_report {
                ui.add_space(6.0);
                ui.strong(format!("Imported report · {}", report.model));
                ui.weak("Source and device association are not verified. Raw values and status are as reported by smartctl; a passed status does not guarantee drive health.");
                ui.label(format!("SMART status: {} · smartctl exit status: {}", match report.health { Some(true) => "passed", Some(false) => "failed", None => "not reported" }, report.exit_status.map(|v| v.to_string()).unwrap_or_else(|| "not reported".into())));
                if report.exit_status.is_some_and(|v| v != 0) { ui.colored_label(AMBER, "smartctl reported a nonzero exit status; the report may contain errors or drive warnings."); }
                for message in &report.messages { ui.colored_label(AMBER, message); }
                if !report.nvme.is_empty() {
                    ui.strong("NVMe health fields");
                    egui::Grid::new("smart_nvme").striped(true).show(ui, |ui| { for (name, value) in &report.nvme { ui.label(name); ui.label(value); ui.end_row(); } });
                }
                if !report.attributes.is_empty() {
                    ui.strong("ATA attributes");
                    egui::ScrollArea::horizontal().id_salt("smart_ata_horizontal").show(ui, |ui| {
                        egui::Grid::new("smart_ata").striped(true).show(ui, |ui| {
                            for heading in ["ID", "Name", "Current", "Worst", "Threshold", "Raw"] { ui.strong(heading); } ui.end_row();
                            for attr in &report.attributes { for value in [&attr.id, &attr.name, &attr.value, &attr.worst, &attr.threshold, &attr.raw] { ui.label(value); } ui.end_row(); }
                        });
                    });
                    if report.truncated { ui.colored_label(AMBER, "Only the first 512 attributes are displayed."); }
                }
                if report.attributes.is_empty() && report.nvme.is_empty() { ui.weak("No supported ATA attribute or NVMe health fields were present."); }
            }
        });
    }

    // -------------------------------------------------------------- reports
    fn processes(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Network interfaces",
                Some("average since previous refresh"),
            );
            ui.add_space(8.0);
            if s.networks.is_empty() {
                ui.weak("No interfaces reported.");
            }
            for n in &s.networks {
                let rate = |bytes: u64| match (bytes, s.network_interval_seconds) {
                    (0, _) => "0 or unavailable".to_owned(),
                    (_, Some(seconds)) if seconds > 0.0 => {
                        format!("{:.1} KiB/s", bytes as f64 / seconds / 1024.0)
                    }
                    _ => "unavailable".into(),
                };
                ui.label(format!(
                    "{}  ·  received {}  ·  sent {}",
                    n.name,
                    rate(n.received),
                    rate(n.transmitted)
                ));
            }
            ui.weak("OS counters may include virtual interfaces; zero can also mean unavailable.");
        });
        ui.add_space(12.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Processes", Some("OS-reported"));
            ui.horizontal_wrapped(|ui| {
                ui.label("Find:");
                ui.add(egui::TextEdit::singleline(&mut self.process_query).hint_text("Name or PID"));
                ui.label("Sort:");
                ui.selectable_value(&mut self.process_sort_memory, false, "CPU");
                ui.selectable_value(&mut self.process_sort_memory, true, "Memory");
            });
            ui.weak("CPU is relative to one logical core and can exceed 100%. OS permissions may limit results.");
            let query = self.process_query.to_lowercase();
            let mut processes: Vec<_> = s.processes.iter().filter(|p| p.name.to_lowercase().contains(&query) || p.pid.to_string().contains(&query)).collect();
            if self.process_sort_memory { processes.sort_by(|a, b| b.memory.cmp(&a.memory).then_with(|| a.pid.cmp(&b.pid))); }
            else { processes.sort_by(|a, b| b.cpu_usage.total_cmp(&a.cpu_usage).then_with(|| a.pid.cmp(&b.pid))); }
            ui.weak(format!("Showing up to 100 of {} matching processes", processes.len()));
            if processes.is_empty() { ui.label(if s.ready { "No matching processes reported." } else { "Collecting process data…" }); }
            egui::Grid::new("process_table").striped(true).min_col_width(70.0).show(ui, |ui| {
                ui.strong("Process"); ui.strong("PID"); ui.strong("CPU"); ui.strong("Memory"); ui.end_row();
                for p in processes.iter().take(100) {
                    ui.label(truncate(&p.name, 38)); ui.label(p.pid.to_string());
                    ui.label(if p.cpu_usage.is_finite() { format!("{:.1}%", p.cpu_usage) } else { "unavailable".into() });
                    ui.label(format!("{:.1} MiB", p.memory as f64 / 1_048_576.0)); ui.end_row();
                }
            });
        });
    }

    fn frames(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "PresentMon capture", Some("local CSV · display metrics"));
            ui.weak("Open a previously captured PresentMon CSV for one process and one swap chain. This app does not capture game frames.");
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.frame_path).desired_width(400.0).hint_text("Path to PresentMon CSV"));
                if ui.button("Analyze capture").clicked() {
                    match frames::read(std::path::Path::new(&self.frame_path)) {
                        Ok(summary) => { self.frame_summary = Some(summary); self.frame_message.clear(); }
                        Err(error) => { self.frame_summary = None; self.frame_message = error; }
                    }
                }
                if self.frame_summary.is_some() && ui.button("Clear").clicked() { self.frame_summary = None; }
            });
            if !self.frame_message.is_empty() { ui.colored_label(AMBER, &self.frame_message); }
            if let Some(r) = &self.frame_summary {
                ui.add_space(12.0);
                ui.strong(format!("{} · PID {} · swap chain {}", r.application, r.process_id, r.swap_chain));
                ui.weak(format!("{} valid displayed-frame intervals · {} missing or invalid rows · source: {}", r.valid, r.skipped, r.column));
                ui.add_space(8.0);
                egui::Grid::new("frame_summary").striped(true).show(ui, |ui| {
                    ui.label("Average displayed FPS"); ui.strong(format!("{:.1}", r.average_fps)); ui.end_row();
                    ui.label("Slowest 1% FPS"); ui.strong(format!("{:.1}", r.one_percent_low_fps)); ui.end_row();
                    ui.label("99th percentile frame time"); ui.strong(format!("{:.2} ms", r.p99_ms)); ui.end_row();
                    if let Some(v) = r.average_cpu_ms { ui.label("Mean CPU busy (available rows)"); ui.label(format!("{v:.2} ms")); ui.end_row(); }
                    if let Some(v) = r.average_gpu_ms { ui.label("Mean GPU busy (available rows)"); ui.label(format!("{v:.2} ms")); ui.end_row(); }
                });
                ui.weak("Average FPS = valid intervals ÷ their total time. Slowest 1% FPS = reciprocal of the mean of the longest 1% of intervals (rounded up). Values describe this imported capture, not current performance.");
            }
        });
    }

    fn reports(&mut self, ui: &mut egui::Ui) {
        let records = self
            .state
            .lock()
            .map(|s| s.results.clone())
            .unwrap_or_default();

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(
                ui,
                "Export",
                Some(if records.is_empty() {
                    "no results yet"
                } else {
                    "never overwrites"
                }),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .selectable_value(&mut self.report_format, Format::Text, "Text")
                    .clicked()
                    && self.report == "hardware-hub-report.json"
                {
                    self.report = "hardware-hub-report.txt".into();
                }
                if ui
                    .selectable_value(&mut self.report_format, Format::Json, "JSON")
                    .clicked()
                    && self.report == "hardware-hub-report.txt"
                {
                    self.report = "hardware-hub-report.json".into();
                }
                ui.add_space(6.0);
                let export = ui.add_enabled(
                    !records.is_empty(),
                    egui::Button::new(RichText::new("Export report").strong())
                        .fill(ACCENT)
                        .corner_radius(CornerRadius::same(8)),
                );
                if export.clicked() {
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
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.report)
                    .desired_width(f32::INFINITY)
                    .hint_text("Path for a new report file"),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Reports include the chosen path and hardware details. Review before sharing.",
                )
                .size(10.5)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);

        if records.is_empty() {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(18.0);
                    ui.label(
                        RichText::new("No tests run in this session")
                            .size(14.0)
                            .strong()
                            .color(TEXT_DIM),
                    );
                    ui.label(
                        RichText::new("Results stay in memory until you export them.")
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.add_space(18.0);
                });
            });
            return;
        }

        for (record_index, r) in records.iter().enumerate().rev() {
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
                            .size(13.0)
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
                            .size(10.5)
                            .color(MUTED),
                        );
                    });
                });
                ui.add_space(6.0);
                ui.label(RichText::new(&r.detail).size(12.0).color(TEXT_DIM));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "verified {} {}  ·  {}  ·  {}",
                        r.verified_units
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        r.verified_unit,
                        config_summary(&r.config),
                        format!("{}s limit", r.config.seconds)
                    ))
                    .size(10.5)
                    .color(MUTED),
                );
                ui.push_id(record_index, |ui| {
                    egui::CollapsingHeader::new("Evidence and limits").show(ui, |ui| {
                        ui.strong("Test settings");
                        ui.label(format!("Time limit: {} seconds · {}", r.config.seconds, config_summary(&r.config)));
                        if r.kind == Kind::Disk { ui.label(format!("Directory: {} · write cap: {} MiB", r.config.directory.display(), r.config.disk_write_limit_mib)); }
                        ui.strong("System context at start");
                        ui.label(format!("{} · {} logical cores · {:.1} GiB RAM", r.context.cpu, r.context.logical_cpus, gib(r.context.memory_total)));
                        ui.label(format!("OS: {}", r.context.os));
                        ui.label(if r.context.adapters.is_empty() { "Adapters: none reported".into() } else { format!("Adapters: {}", r.context.adapters.join(", ")) });
                        ui.label(format!("Collection age at start: {}", r.context.snapshot_age_seconds.map(|v| format!("{v:.1} seconds")).unwrap_or_else(|| "unavailable".into())));
                        ui.strong("Scope and limitations");
                        ui.weak(match r.kind {
                            Kind::Cpu => "A fixed integer calculation is checked; it does not certify every core or instruction.",
                            Kind::Gpu => "A small compute readback is checked; it is not a full GPU or VRAM stress test.",
                            Kind::Memory => "Only the allocated region is checked; this is not a full-RAM or ECC test.",
                            Kind::Disk => "Only an owned temporary file is checked; readback may use cache and is not a physical-drive health test.",
                        });
                        ui.weak("A completed check does not guarantee hardware health. No automatic thermal shutdown is available.");
                    });
                });
            });
            ui.add_space(8.0);
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let frame = egui::Frame {
            fill: PANEL,
            stroke: Stroke::new(1.0_f32, BORDER),
            inner_margin: egui::Margin::symmetric(20, 10),
            ..Default::default()
        };
        egui::TopBottomPanel::bottom("status")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.message.is_empty() {
                        let (dot, text) = if !s.ready {
                            (MUTED, "Collecting readings…".to_string())
                        } else {
                            match s.updated_at.map(|t| t.elapsed().as_secs_f32()) {
                                Some(age) if age > 3.0 => {
                                    (AMBER, format!("Readings are stale ({age:.0}s old)"))
                                }
                                Some(age) => (GREEN, format!("Live · updated {age:.1}s ago")),
                                None => (MUTED, "Waiting for data".to_string()),
                            }
                        };
                        ui.painter().circle_filled(
                            ui.cursor().left_center() + egui::vec2(4.0, 0.0),
                            3.2,
                            dot,
                        );
                        ui.add_space(14.0);
                        ui.label(RichText::new(text).size(11.0).color(TEXT_DIM));
                    } else {
                        let (color, icon) = if self.message.contains("failed")
                            || self.message.contains("Failed")
                            || self.message.contains("exceeds")
                            || self.message.contains("Limits")
                        {
                            (RED, "!")
                        } else {
                            (ACCENT, "i")
                        };
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(14.0, 12.0), egui::Sense::hover());
                        if ui.is_rect_visible(rect) {
                            ui.painter().circle_filled(rect.center(), 5.5, tint(color));
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                icon,
                                egui::FontId::proportional(9.0),
                                color,
                            );
                        }
                        ui.label(RichText::new(&self.message).size(11.0).color(TEXT_DIM));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(std::env::consts::OS.to_uppercase())
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                });
            });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.network_cancel.store(true, Ordering::Relaxed);
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let running = self.state.lock().map(|s| s.running).unwrap_or(false);
        if !running && (self.message.starts_with("Stopping") || self.message == "Test started") {
            if let Ok(state) = self.state.lock() {
                if let Some(result) = state.results.last() {
                    self.message = format!(
                        "Test {}. See Reports for details.",
                        result.status.to_string().to_lowercase()
                    );
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(recorder) = &mut self.recorder {
                if !recorder.status().closed {
                    recorder.stop();
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.message =
                        "Finishing the recording. Close again when it has stopped.".into();
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && running {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.cancel.store(true, Ordering::Relaxed);
            self.page = Page::Tests;
            self.message = "Stopping test before closing. Close again when it has stopped.".into();
        }
        let snapshot = self.snapshot.lock().map(|s| s.clone()).unwrap_or_default();
        if snapshot.ready && !self.gpu_initialized {
            self.gpu_index = snapshot
                .adapters
                .iter()
                .position(|a| !a.software)
                .unwrap_or(0);
            self.gpu_initialized = true;
        }
        self.history.record(&snapshot, Instant::now());
        if let Some(recorder) = &mut self.recorder {
            recorder.record(&snapshot, Instant::now());
        }

        self.navigation(ctx, &snapshot);
        self.status_bar(ctx, &snapshot);

        let frame = egui::Frame {
            fill: BG,
            inner_margin: egui::Margin {
                left: 28,
                right: 28,
                top: 24,
                bottom: 20,
            },
            ..Default::default()
        };
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(self.page.group())
                            .size(10.5)
                            .strong()
                            .color(ACCENT),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(self.page.title())
                            .size(26.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.add_space(2.0);
                    ui.label(RichText::new(self.page.subtitle()).size(12.0).color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if matches!(
                        self.page,
                        Page::Details | Page::Motherboard | Page::Bluetooth
                    ) {
                        let ready = self.specs.lock().map(|s| s.is_some()).unwrap_or(false);
                        let label = if !ready {
                            "WAITING"
                        } else if self.page == Page::Bluetooth {
                            "SNAPSHOT"
                        } else {
                            "AT START"
                        };
                        pill(ui, label, MUTED);
                    } else if snapshot.ready {
                        let (color, text) =
                            match snapshot.updated_at.map(|t| t.elapsed().as_secs_f32()) {
                                Some(age) if age >= 3.0 => (AMBER, "STALE"),
                                Some(_) => (GREEN, "LIVE"),
                                None => (MUTED, "WAITING"),
                            };
                        pill(ui, text, color);
                    }
                });
            });
            ui.add_space(18.0);
            page_rule(ui);
            ui.add_space(18.0);
            egui::ScrollArea::vertical()
                .id_salt(self.page.label())
                .auto_shrink([false, false])
                .show(ui, |ui| match self.page {
                    Page::Overview => self.overview(ui, &snapshot),
                    Page::Cpu
                    | Page::Gpu
                    | Page::Memory
                    | Page::Storage
                    | Page::Thermal
                    | Page::Motherboard
                    | Page::Network
                    | Page::Bluetooth => self.hardware_page(ui, &snapshot),
                    Page::Details => self.details(ui, &snapshot),
                    Page::Processes => self.processes(ui, &snapshot),
                    Page::Frames => self.frames(ui),
                    Page::Tests => self.tests(ui, &snapshot, None),
                    Page::Reports => self.reports(ui),
                });
        });
        if self.show_widget {
            let mut keep_open = true;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("floating-monitor"),
                egui::ViewportBuilder::default()
                    .with_title("Hardware Hub · Monitor")
                    .with_inner_size([300.0, 150.0])
                    .with_min_inner_size([260.0, 130.0])
                    .with_always_on_top(),
                |ctx, class| {
                    if class == egui::ViewportClass::Embedded {
                        egui::Window::new("Floating monitor")
                            .open(&mut keep_open)
                            .show(ctx, |ui| floating_readings(ui, &snapshot));
                    } else {
                        egui::CentralPanel::default()
                            .show(ctx, |ui| floating_readings(ui, &snapshot));
                        if ctx.input(|input| input.viewport().close_requested()) {
                            keep_open = false;
                        }
                    }
                },
            );
            self.show_widget = keep_open;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn floating_readings(ui: &mut egui::Ui, s: &Snapshot) {
    ui.strong("LIVE MONITOR");
    match s.updated_at {
        Some(at) if at.elapsed() >= Duration::from_secs(3) => {
            ui.colored_label(
                AMBER,
                format!("Stale · {:.1}s ago", at.elapsed().as_secs_f32()),
            );
        }
        Some(_) if s.ready => {
            ui.weak("Local readings · sysinfo");
        }
        _ => {
            ui.weak("Collecting readings…");
        }
    }
    ui.horizontal(|ui| {
        ui.label(
            if s.ready && !s.cores.is_empty() && s.cpu_usage.is_finite() {
                format!("CPU  {:.0}%", s.cpu_usage)
            } else {
                "CPU  unavailable".into()
            },
        );
        ui.separator();
        ui.label(if s.ready && s.memory_total > 0 {
            format!(
                "RAM  {:.0}%",
                s.memory_used as f64 * 100.0 / s.memory_total as f64
            )
        } else {
            "RAM  unavailable".into()
        });
    });
    ui.weak("Desktop window · game FPS unavailable");
}

// ------------------------------------------------------------------- helpers
fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = Color32::from_rgb(20, 20, 22);
    visuals.faint_bg_color = CARD_HI;
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.weak_bg_fill = CARD_HI;
    visuals.widgets.inactive.bg_fill = TRACK;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.weak_bg_fill = CARD_HI;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(72, 72, 74);
    visuals.widgets.hovered.bg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 42));
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.weak_bg_fill = ACCENT_SOFT;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.slider_width = 180.0;
    style.animation_time = 0.12;
    ctx.set_style(style);
}

fn card_frame() -> egui::Frame {
    egui::Frame {
        fill: CARD,
        stroke: Stroke::new(1.0_f32, BORDER),
        corner_radius: CornerRadius::same(14),
        inner_margin: egui::Margin {
            left: 18,
            right: 18,
            top: 16,
            bottom: 16,
        },
        shadow: egui::epaint::Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(28),
        },
        ..Default::default()
    }
}

fn spec_row(rows: &mut Vec<(String, String)>, label: impl Into<String>, value: impl Into<String>) {
    let value = value.into();
    if !value.trim().is_empty() {
        rows.push((label.into(), value));
    }
}

fn spec_grid(ui: &mut egui::Ui, id: &str, rows: &[(String, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([20.0, 7.0])
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(RichText::new(label).size(10.5).color(MUTED));
                ui.label(RichText::new(value).size(11.5).color(TEXT_DIM));
                ui.end_row();
            }
        });
}

/// One row of the HWMonitor-style sensor table.
struct RowView<'a> {
    label: &'a str,
    key: &'a str,
    current: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
    unit: &'a str,
}

fn sensor_table(ui: &mut egui::Ui, id: &str, heading: &str, rows: Vec<RowView<'_>>) {
    ui.label(RichText::new(heading).size(11.0).strong().color(TEXT_DIM));
    ui.add_space(3.0);
    egui::Grid::new(id)
        .num_columns(4)
        .spacing([22.0, 5.0])
        .striped(true)
        .show(ui, |ui| {
            for header in ["Sensor", "Current", "Min", "Max"] {
                ui.label(RichText::new(header).size(9.5).color(MUTED));
            }
            ui.end_row();
            for row in rows {
                ui.label(RichText::new(row.label).size(11.5).color(TEXT_DIM))
                    .on_hover_text(row.key);
                let current = row
                    .current
                    .map(|value| format!("{value:.0} {}", row.unit))
                    .unwrap_or_else(|| "—".to_string());
                let current_color = match row.current {
                    Some(value) if row.unit == "°C" => temperature_color(value),
                    Some(_) => TEXT,
                    None => MUTED,
                };
                ui.label(
                    RichText::new(current)
                        .size(11.5)
                        .strong()
                        .color(current_color),
                );
                for value in [row.min, row.max] {
                    let text = value
                        .map(|value| format!("{value:.0}"))
                        .unwrap_or_else(|| "—".to_string());
                    ui.label(RichText::new(text).size(11.0).color(MUTED));
                }
                ui.end_row();
            }
        });
}

fn uptime_text(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3600;
    let minutes = (seconds % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn clock_text(mhz: u64) -> String {
    if mhz >= 1000 {
        format!("{:.2} GHz", mhz as f64 / 1000.0)
    } else {
        format!("{mhz} MHz")
    }
}

fn vendor_text(vendor: &str) -> &str {
    match vendor {
        "GenuineIntel" => "Intel (GenuineIntel)",
        "AuthenticAMD" => "AMD (AuthenticAMD)",
        other => other,
    }
}

fn smart_color(smart: &str) -> Color32 {
    match smart.to_lowercase().as_str() {
        "verified" | "good" | "ok" => GREEN,
        "" => MUTED,
        "failing" | "bad" => RED,
        _ => AMBER,
    }
}

/// Decimal units, matching how vendors label memory and drive capacity.
fn decimal_bytes(v: u64) -> String {
    const BASE: f64 = 1000.0;
    let value = v as f64;
    if value >= BASE * BASE * BASE {
        format!("{:.1} GB", value / (BASE * BASE * BASE))
    } else if value >= BASE * BASE {
        format!("{:.0} MB", value / (BASE * BASE))
    } else if value >= BASE {
        format!("{:.0} KB", value / BASE)
    } else {
        format!("{v} B")
    }
}

fn section_title(ui: &mut egui::Ui, title: &str, hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(13.0).strong().color(TEXT));
        if let Some(hint) = hint {
            ui.label(RichText::new(hint).size(10.5).color(MUTED));
        }
    });
    ui.add_space(2.0);
}

fn empty_note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(MUTED).italics());
}

fn page_rule(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, BORDER);
}

/// Small line-art icons keep the sidebar scannable without external assets.
fn draw_nav_icon(painter: &egui::Painter, center: egui::Pos2, page: Page, color: Color32) {
    let stroke = Stroke::new(1.35_f32, color);
    let faint = Stroke::new(
        1.0_f32,
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 130),
    );
    let line = |a: egui::Pos2, b: egui::Pos2, stroke: Stroke| {
        painter.line_segment([a, b], stroke);
    };
    let box_rect = egui::Rect::from_center_size(center, egui::vec2(13.0, 13.0));
    match page {
        Page::Overview => {
            for (dx, dy) in [(-3.2, -3.2), (3.2, -3.2), (-3.2, 3.2), (3.2, 3.2)] {
                painter.rect_filled(
                    egui::Rect::from_center_size(center + egui::vec2(dx, dy), egui::vec2(4.6, 4.6)),
                    CornerRadius::same(2),
                    color,
                );
            }
        }
        Page::Cpu => {
            painter.rect_stroke(
                box_rect.shrink(0.5),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(5.5, 5.5)),
                CornerRadius::same(1),
                faint,
                egui::StrokeKind::Inside,
            );
            for offset in [-4.0, 0.0, 4.0] {
                line(
                    center + egui::vec2(offset, -8.0),
                    center + egui::vec2(offset, -7.0),
                    faint,
                );
                line(
                    center + egui::vec2(offset, 7.0),
                    center + egui::vec2(offset, 8.0),
                    faint,
                );
            }
        }
        Page::Gpu => {
            painter.rect_stroke(
                box_rect,
                CornerRadius::same(3),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(center, 3.4, faint);
            for end in [
                egui::vec2(2.8, -1.4),
                egui::vec2(-1.4, 2.8),
                egui::vec2(-2.4, -2.4),
            ] {
                line(center, center + end, faint);
            }
        }
        Page::Memory => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(15.0, 9.0)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for dx in [-4.0, 0.0, 4.0] {
                line(
                    center + egui::vec2(dx, 4.5),
                    center + egui::vec2(dx, 7.0),
                    faint,
                );
            }
        }
        Page::Storage => {
            for dy in [-4.0, 0.0, 4.0] {
                painter.rect_stroke(
                    egui::Rect::from_center_size(
                        center + egui::vec2(0.0, dy),
                        egui::vec2(14.0, 3.0),
                    ),
                    CornerRadius::same(2),
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        Page::Thermal => {
            line(
                center + egui::vec2(-2.0, -7.0),
                center + egui::vec2(-2.0, 1.5),
                stroke,
            );
            painter.circle_filled(center + egui::vec2(-2.0, 4.0), 3.3, color);
            for dy in [-4.0, 0.0] {
                line(
                    center + egui::vec2(2.0, dy),
                    center + egui::vec2(5.0, dy),
                    faint,
                );
            }
        }
        Page::Motherboard => {
            painter.rect_stroke(
                box_rect,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for (dx, dy) in [(-3.0, -3.0), (3.0, -3.0), (-3.0, 3.0)] {
                painter.rect_filled(
                    egui::Rect::from_center_size(center + egui::vec2(dx, dy), egui::vec2(3.0, 3.0)),
                    CornerRadius::same(1),
                    color,
                );
            }
            line(
                center + egui::vec2(1.0, 3.0),
                center + egui::vec2(4.0, 3.0),
                faint,
            );
        }
        Page::Network => {
            painter.circle_filled(center, 1.8, color);
            for radius in [4.5, 7.5] {
                painter.circle_stroke(center, radius, faint);
            }
        }
        Page::Bluetooth => {
            for (a, b) in [
                (egui::vec2(-2.0, -6.0), egui::vec2(-2.0, 6.0)),
                (egui::vec2(-2.0, -6.0), egui::vec2(3.0, -1.0)),
                (egui::vec2(-2.0, 0.0), egui::vec2(3.0, -5.0)),
                (egui::vec2(-2.0, 0.0), egui::vec2(3.0, 5.0)),
                (egui::vec2(-2.0, 6.0), egui::vec2(3.0, 1.0)),
            ] {
                line(center + a, center + b, stroke);
            }
        }
        Page::Details => {
            painter.circle_stroke(center, 6.5, stroke);
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "i",
                egui::FontId::proportional(10.0),
                color,
            );
        }
        Page::Processes => {
            for dy in [-4.0, 0.0, 4.0] {
                painter.circle_filled(center + egui::vec2(-4.0, dy), 1.4, color);
                line(
                    center + egui::vec2(-1.0, dy),
                    center + egui::vec2(6.0, dy),
                    faint,
                );
            }
        }
        Page::Frames => {
            for (dx, height) in [(-4.0, 5.0), (0.0, 8.0), (4.0, 11.0)] {
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        center + egui::vec2(dx, 5.5 - height / 2.0),
                        egui::vec2(3.0, height),
                    ),
                    CornerRadius::same(1),
                    color,
                );
            }
        }
        Page::Tests => {
            line(
                center + egui::vec2(-3.0, -7.0),
                center + egui::vec2(-3.0, -2.0),
                stroke,
            );
            line(
                center + egui::vec2(3.0, -7.0),
                center + egui::vec2(3.0, -2.0),
                stroke,
            );
            line(
                center + egui::vec2(-4.0, -7.0),
                center + egui::vec2(4.0, -7.0),
                stroke,
            );
            painter.rect_stroke(
                egui::Rect::from_center_size(center + egui::vec2(0.0, 3.0), egui::vec2(12.0, 9.0)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for (dx, dy) in [(-2.0, 2.0), (2.0, 4.0), (0.0, 6.0)] {
                painter.circle_filled(center + egui::vec2(dx, dy), 1.1, color);
            }
        }
        Page::Reports => {
            painter.rect_stroke(
                box_rect,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for dy in [-3.0, 0.0, 3.0] {
                line(
                    center + egui::vec2(-3.0, dy),
                    center + egui::vec2(4.0, dy),
                    faint,
                );
            }
        }
    }
}

fn nav_item(ui: &mut egui::Ui, active: bool, page: Page, label: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 36.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, CornerRadius::same(10), ACCENT_SOFT);
            let accent = egui::Rect::from_min_size(
                rect.left_center() + egui::vec2(0.0, -9.0),
                egui::vec2(3.0, 18.0),
            );
            painter.rect_filled(accent, CornerRadius::same(2), ACCENT);
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(10), CARD_HI);
        }
        draw_nav_icon(
            painter,
            rect.left_center() + egui::vec2(19.0, 0.0),
            page,
            if active { ACCENT } else { MUTED },
        );
        painter.text(
            rect.left_center() + egui::vec2(39.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.5),
            if active { TEXT } else { TEXT_DIM },
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn kind_tile(ui: &mut egui::Ui, selected: bool, title: &str, hint: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 56.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if selected {
            ACCENT_SOFT
        } else if response.hovered() {
            CARD_HI
        } else {
            CARD
        };
        painter.rect_filled(rect, CornerRadius::same(12), fill);
        painter.rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(
                if selected { 1.4 } else { 1.0_f32 },
                if selected { ACCENT } else { BORDER },
            ),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.left_top() + egui::vec2(13.0, 11.0),
            egui::Align2::LEFT_TOP,
            title,
            egui::FontId::proportional(12.5),
            if selected { TEXT } else { TEXT_DIM },
        );
        painter.text(
            rect.left_top() + egui::vec2(13.0, 30.0),
            egui::Align2::LEFT_TOP,
            hint,
            egui::FontId::proportional(10.0),
            MUTED,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn metric(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    empty_hint: &str,
    fraction: Option<f32>,
    accent: Color32,
) {
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(104.0);
        ui.label(RichText::new(label).size(10.0).strong().color(MUTED));
        ui.add_space(6.0);
        ui.label(RichText::new(value).size(26.0).strong().color(TEXT));
        if let Some(f) = fraction {
            ui.add_space(10.0);
            bar(ui, f, accent, 6.0);
        } else if !empty_hint.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(empty_hint).size(10.5).color(MUTED));
        }
    });
}

fn bar(ui: &mut egui::Ui, fraction: f32, color: Color32, height: f32) {
    let width = ui.available_width().max(24.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let radius = CornerRadius::same((height / 2.0) as u8);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, TRACK);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let filled = egui::Rect::from_min_size(
            rect.min,
            egui::vec2((rect.width() * fraction).max(height), rect.height()),
        );
        painter.rect_filled(filled, radius, color);
    }
}

fn core_meter(ui: &mut egui::Ui, index: usize, usage: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(150.0, 20.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let fraction = (usage / 100.0).clamp(0.0, 1.0);
    let painter = ui.painter();
    painter.text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        format!("{index:02}"),
        egui::FontId::proportional(10.0),
        MUTED,
    );
    let track = egui::Rect::from_min_size(
        rect.left_center() + egui::vec2(24.0, -3.0),
        egui::vec2(84.0, 6.0),
    );
    painter.rect_filled(track, CornerRadius::same(3), TRACK);
    if fraction > 0.0 {
        painter.rect_filled(
            egui::Rect::from_min_size(
                track.min,
                egui::vec2((track.width() * fraction).max(4.0), track.height()),
            ),
            CornerRadius::same(3),
            load_color(fraction),
        );
    }
    painter.text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        format!("{usage:.0}%"),
        egui::FontId::proportional(10.0),
        TEXT_DIM,
    );
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(10.5), color);
    let size = galley.size() + egui::vec2(28.0, 10.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same((rect.height() / 2.0) as u8),
        tint(color),
    );
    painter.circle_filled(rect.left_center() + egui::vec2(11.0, 0.0), 3.0, color);
    painter.galley(
        rect.left_center() + egui::vec2(19.0, -galley.size().y / 2.0),
        galley,
        color,
    );
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(10.0), color);
    let size = galley.size() + egui::vec2(14.0, 7.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), tint(color));
    painter.galley(rect.left_top() + egui::vec2(7.0, 3.5), galley, color);
}

fn tint(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 44)
}

fn load_color(fraction: f32) -> Color32 {
    if fraction > 0.9 {
        RED
    } else if fraction > 0.7 {
        AMBER
    } else if fraction > 0.45 {
        ACCENT
    } else {
        GREEN
    }
}

fn temperature_color(celsius: f32) -> Color32 {
    if celsius >= 90.0 {
        RED
    } else if celsius >= 75.0 {
        AMBER
    } else {
        GREEN
    }
}

fn status_color(status: &Status) -> Color32 {
    match status {
        Status::Completed => GREEN,
        Status::Stopped => AMBER,
        Status::Failed => RED,
        Status::Inconclusive => VIOLET,
        Status::Unavailable => MUTED,
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Cpu => "CPU compute",
        Kind::Gpu => "GPU check",
        Kind::Memory => "Memory patterns",
        Kind::Disk => "File I/O",
    }
}

fn config_summary(config: &Config) -> String {
    match config.kind {
        Kind::Cpu => format!("{} workers", config.workers),
        Kind::Gpu => format!(
            "GPU {}",
            config.gpu_adapter.as_deref().unwrap_or("not selected")
        ),
        Kind::Memory => format!("{} MiB memory", config.mib),
        Kind::Disk => format!(
            "{} MiB file, {} MiB cap",
            config.mib, config.disk_write_limit_mib
        ),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn gib(v: u64) -> f64 {
    v as f64 / 1073741824.0
}

fn bytes(v: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = v as f64;
    if value >= KIB * KIB * KIB {
        format!("{:.1} GiB", value / (KIB * KIB * KIB))
    } else if value >= KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else if value >= KIB {
        format!("{:.0} KiB", value / KIB)
    } else {
        format!("{v} B")
    }
}

// Unix seconds formatted as UTC; reports stay readable without a date library.
fn timestamp_text(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let seconds = (unix % 86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_render_as_utc_dates() {
        assert_eq!(timestamp_text(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(timestamp_text(1_700_000_000), "2023-11-14 22:13:20 UTC");
        assert_eq!(timestamp_text(1_709_164_800), "2024-02-29 00:00:00 UTC");
    }

    #[test]
    fn byte_sizes_stay_human_readable() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2048), "2 KiB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn long_names_are_truncated_to_the_limit() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a very long adapter name", 10).chars().count(), 10);
    }
}
