#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use hardware_hub::{
    alerts::{self, Severity, Threshold},
    frames::{self, FrameSummary},
    history::{self, History},
    logging::Recorder,
    netcheck::{self, State as NetworkCheckState},
    report::{self, Format},
    smart::{self, SmartReport},
    specs::{self, SystemSpecs},
    stress::{
        self, Config, DiskPattern, ErrorDetail, GpuIntensity, Kind, MemPattern, ReportContext,
        State, Status,
    },
    system_report,
    telemetry::{self, Snapshot},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
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

const KINDS: [(Kind, &str, &str); 5] = [
    (Kind::Cpu, "CPU compute", "Integer rounds, fixed answer"),
    (Kind::Gpu, "GPU check", "Shader dispatch, read back"),
    (Kind::Memory, "Memory patterns", "Alternating byte patterns"),
    (Kind::Disk, "File I/O", "Owned scratch file verified"),
    (Kind::Combined, "Combined load", "CPU + memory together"),
];

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Hardware Hub")
            // Open maximised: on a 1280x720 screen at 150% scaling only ~853x448
            // logical points are available, so a fixed 1180x800 window was
            // clipped and pushed the status bar off-screen.
            .with_maximized(true)
            .with_min_inner_size([720.0, 420.0]),
        renderer: eframe::Renderer::Glow,
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
    Drivers,
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
            Page::Motherboard => "Parts & board",
            Page::Drivers => "Drivers",
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
                "Available sensors, live temperature history, sampling and advisory alerts."
            }
            Page::Motherboard => "Mainboard, BIOS, attached displays and network adapters.",
            Page::Drivers => "Installed hardware drivers as reported by the OS driver store.",
            Page::Network => "Network interfaces and operating-system traffic counters.",
            Page::Bluetooth => "Bluetooth availability and supported diagnostics.",
            Page::Details => "Specification sheet read once from local OS sources.",
            Page::Processes => "Live CPU and memory usage by process, plus network interfaces.",
            Page::Frames => {
                "Inspect a saved PresentMon capture; no live game capture is performed."
            }
            Page::Tests => {
                "One bounded check at a time, with live load and peak temperature. Not benchmarks."
            }
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

fn initial_page() -> Page {
    match std::env::var("HH_START_PAGE").as_deref() {
        Ok("details") => Page::Details,
        Ok("cpu") => Page::Cpu,
        Ok("gpu") => Page::Gpu,
        Ok("memory") => Page::Memory,
        Ok("drivers") => Page::Drivers,
        Ok("thermal") | Ok("thermals") => Page::Thermal,
        Ok("motherboard") | Ok("board") => Page::Motherboard,
        Ok("network") => Page::Network,
        Ok("bluetooth") => Page::Bluetooth,
        Ok("frames") => Page::Frames,
        Ok("tests") => Page::Tests,
        Ok("reports") => Page::Reports,
        Ok("processes") => Page::Processes,
        Ok("overview") => Page::Overview,
        _ => Page::Overview,
    }
}

struct App {
    page: Page,
    snapshot: Arc<Mutex<Arc<Snapshot>>>,
    specs: Arc<Mutex<Option<Arc<SystemSpecs>>>>,
    /// Desired telemetry period in milliseconds; the worker clamps it to 250–5000.
    poll_ms: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    /// Set to ask the telemetry thread to clear its min/max history.
    stats_reset: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
    /// Highest temperature observed during the current stress run, for the
    /// live safety readout. Cleared whenever a new check starts.
    run_peak_c: Option<f32>,
    /// Previous live-progress reading and its timestamp, for a units/s rate.
    last_live_units: u64,
    last_live_at: Instant,
    kind: Kind,
    seconds: u64,
    workers: usize,
    mib: usize,
    disk_write_limit_mib: u64,
    directory: String,
    /// Selected data pattern for memory and disk checks (pro-tool style).
    mem_pattern: hardware_hub::stress::MemPattern,
    disk_pattern: hardware_hub::stress::DiskPattern,
    gpu_intensity: hardware_hub::stress::GpuIntensity,
    /// Opt-in: stop the current check when peak temperature reaches the
    /// advisory alert limit. This stops only the software check, never hardware.
    auto_stop_on_heat: bool,
    auto_stop_fired: bool,
    report: String,
    report_format: Format,
    certificate_path: String,
    /// Indices into the session result list for the side-by-side comparison.
    compare_a: usize,
    compare_b: usize,
    gpu_index: usize,
    gpu_initialized: bool,
    message: String,
    history: History,
    history_path: String,
    log_path: String,
    all_sensor_log_path: String,
    recorder: Option<Recorder>,
    process_query: String,
    process_sort_memory: bool,
    driver_query: String,
    driver_class: String,
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
    // Optional advisory temperature alert (never changes hardware behaviour).
    alert: alerts::Monitor,
    alert_celsius: f32,
    alert_hold_secs: u64,
    alert_severity: Severity,
    // One selectable temperature series in the thermal history graph.
    selected_temp_key: String,
    sensor_log_path: String,
    // CPU-Z style, exportable system inventory.
    system_report_path: String,
    system_report_format: Format,
    system_report_redact: bool,
    show_widget: bool,
    started: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_theme(&cc.egui_ctx);
        let snapshot = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let stop = Arc::new(AtomicBool::new(false));
        let stats_reset = Arc::new(AtomicBool::new(false));
        let poll_ms = Arc::new(AtomicU64::new(1000));
        telemetry::spawn(
            snapshot.clone(),
            stop.clone(),
            stats_reset.clone(),
            poll_ms.clone(),
        );
        // The inventory shells out to local tools once, so it must not block the
        // first frame; the Details page shows a placeholder until it lands.
        let specs = Arc::new(Mutex::new(None));
        let specs_slot = Arc::clone(&specs);
        std::thread::spawn(move || {
            let collected = specs::collect();
            if let Ok(mut slot) = specs_slot.lock() {
                *slot = Some(Arc::new(collected));
            }
        });
        Self {
            page: initial_page(),
            snapshot,
            specs,
            poll_ms,
            stop,
            stats_reset,
            cancel: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(State::default())),
            run_peak_c: None,
            last_live_units: 0,
            last_live_at: Instant::now(),
            kind: Kind::Cpu,
            seconds: 15,
            workers: std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(1),
            mib: 64,
            disk_write_limit_mib: 64,
            directory: std::env::temp_dir().to_string_lossy().into_owned(),
            mem_pattern: MemPattern::Alternating,
            disk_pattern: DiskPattern::Constant,
            gpu_intensity: GpuIntensity::Standard,
            auto_stop_on_heat: false,
            auto_stop_fired: false,
            report: "hardware-hub-report.txt".into(),
            report_format: Format::Text,
            certificate_path: "hardware-hub-run-summary.txt".into(),
            compare_a: 0,
            compare_b: 0,
            gpu_index: 0,
            gpu_initialized: false,
            message: String::new(),
            history: History::default(),
            history_path: "hardware-hub-history.csv".into(),
            log_path: "hardware-hub-session.csv".into(),
            all_sensor_log_path: "hardware-hub-all-sensors.csv".into(),
            recorder: None,
            process_query: String::new(),
            process_sort_memory: false,
            driver_query: String::new(),
            driver_class: "All".into(),
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
            alert: alerts::Monitor::default(),
            alert_celsius: 85.0,
            alert_hold_secs: 5,
            alert_severity: Severity::Warning,
            selected_temp_key: String::new(),
            sensor_log_path: "hardware-hub-sensor.csv".into(),
            system_report_path: "hardware-hub-system.txt".into(),
            system_report_format: Format::Text,
            system_report_redact: true,
            show_widget: false,
            started: Instant::now(),
        }
    }

    // ------------------------------------------------------------- navigation
    fn navigation(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let frame = egui::Frame {
            fill: PANEL,
            stroke: Stroke::NONE,
            inner_margin: egui::Margin {
                left: 14,
                right: 14,
                top: 20,
                bottom: 14,
            },
            ..Default::default()
        };
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(248.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, CornerRadius::same(10), ACCENT);
                    painter.circle_stroke(
                        rect.center(),
                        7.5,
                        Stroke::new(1.5_f32, Color32::from_white_alpha(215)),
                    );
                    painter.circle_filled(rect.center(), 2.4, Color32::from_white_alpha(235));
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Hardware Hub")
                                .size(15.0)
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
                ui.add_space(24.0);
                // Pin the footer and the controls to the bottom first, so the
                // page list receives exactly the remaining height. A fixed
                // reserve previously pushed eight pages below the fold and let
                // the toggle overlap the last visible item at high DPI.
                egui::TopBottomPanel::bottom("nav-footer")
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        ui.add_space(2.0);
                        // One compact line: local-only reassurance plus the
                        // graphics provider, so the footer never crowds the
                        // taskbar when a run adds status rows above it.
                        let backend = if s.adapters.is_empty() {
                            "local readings only".to_string()
                        } else {
                            format!("local only · wgpu · {}", s.gpu_backend)
                        };
                        ui.label(RichText::new(backend).size(10.0).color(MUTED));
                        ui.label(
                            RichText::new("⌘/Ctrl 1–9 pages · ⇧1–6 more · Esc stop")
                                .size(10.0)
                                .color(MUTED),
                        );
                        ui.add_space(6.0);
                    });
                egui::TopBottomPanel::bottom("nav-controls")
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        ui.add_space(8.0);
                        ui.checkbox(&mut self.show_widget, "Floating monitor")
                            .on_hover_text("Desktop window; not an in-game FPS overlay.");
                        if let Some(recorder) = &mut self.recorder {
                            let status = recorder.status();
                            if recorder.active() {
                                ui.colored_label(
                                    GREEN,
                                    format!("Recording · {} batches", status.batches),
                                );
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
                            let stopping = self.cancel.load(Ordering::Relaxed);
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    ACCENT,
                                    if stopping {
                                        "Stopping…"
                                    } else {
                                        "Check running"
                                    },
                                );
                                if !stopping && ui.small_button("Stop").clicked() {
                                    self.cancel.store(true, Ordering::Relaxed);
                                    self.message =
                                        "Stopping after the current work or I/O operation…".into();
                                }
                            });
                        }
                    });
                egui::ScrollArea::vertical()
                    .id_salt("navigation-pages")
                    // Always show the bar: on a 1280x720 screen at 150% scaling
                    // there is not enough height for all pages, and a fading
                    // scrollbar left them looking absent.
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new("HARDWARE").size(10.0).strong().color(MUTED));
                        ui.add_space(7.0);
                        for page in [
                            Page::Overview,
                            Page::Cpu,
                            Page::Gpu,
                            Page::Memory,
                            Page::Storage,
                            Page::Thermal,
                            Page::Motherboard,
                            Page::Drivers,
                            Page::Network,
                            Page::Bluetooth,
                        ] {
                            if nav_item(ui, self.page == page, page, page.label()).clicked() {
                                self.page = page;
                            }
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new("WORKSPACE").size(10.0).strong().color(MUTED));
                        ui.add_space(7.0);
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

        let peak_celsius = s
            .temperatures
            .iter()
            .filter_map(|t| t.celsius)
            .fold(None::<f32>, |peak, value| {
                Some(peak.map_or(value, |previous: f32| previous.max(value)))
            });

        ui.columns(3, |cols| {
            let cpu_footnote = if s.cores.is_empty() {
                "No core data from the operating system".to_owned()
            } else {
                let mut text = format!(
                    "{} · {} cores",
                    truncate(&short_cpu_name(&s.cpu_name), 24),
                    s.cores.len()
                );
                if let (Some(min), Some(max)) = (s.cpu_range.min, s.cpu_range.max) {
                    text.push_str(&format!(" · {min:.0}–{max:.0}%"));
                }
                text
            };
            metric(
                &mut cols[0],
                Page::Cpu,
                "PROCESSOR",
                &if s.cores.is_empty() || !s.cpu_usage.is_finite() {
                    "Unavailable".to_owned()
                } else {
                    format!("{:.0}%", s.cpu_usage)
                },
                &cpu_footnote,
                if s.cores.is_empty() || !s.cpu_usage.is_finite() {
                    None
                } else {
                    Some(s.cpu_usage / 100.0)
                },
                load_color(s.cpu_usage / 100.0),
            );

            let memory_footnote = if s.memory_total == 0 {
                "No memory data from the operating system".to_owned()
            } else {
                let mut text = format!("{:.1} GiB free", gib(s.memory_available));
                if let (Some(min), Some(max)) = (s.memory_range.min, s.memory_range.max) {
                    text.push_str(&format!(" · {min:.0}–{max:.0}%"));
                }
                text
            };
            metric(
                &mut cols[1],
                Page::Memory,
                "MEMORY",
                &if s.memory_total == 0 {
                    "Unavailable".to_string()
                } else {
                    format!("{:.1} / {:.1} GiB", gib(s.memory_used), gib(s.memory_total))
                },
                &memory_footnote,
                (s.memory_total > 0).then_some(memory_fraction),
                VIOLET,
            );

            let graphics_footnote = match s.adapters.first() {
                None => "No adapter reported".to_string(),
                Some(gpu) => {
                    let mut text = truncate(&gpu.name, 30);
                    if hardware_adapters == 0 {
                        text.push_str(" · software");
                    }
                    text
                }
            };
            metric(
                &mut cols[2],
                Page::Gpu,
                "GRAPHICS",
                &if s.adapters.is_empty() {
                    "None".to_string()
                } else {
                    s.adapters.len().to_string()
                },
                &graphics_footnote,
                None,
                AMBER,
            );
        });

        ui.add_space(10.0);
        let smart = specs
            .as_ref()
            .and_then(|known| {
                known
                    .volumes
                    .iter()
                    .map(|volume| volume.smart.as_str())
                    .find(|smart| !smart.is_empty())
            })
            .unwrap_or("");
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "At a glance", Some("session summary"));
            ui.add_space(8.0);
            ui.columns(4, |cols| {
                stat(
                    &mut cols[0],
                    "PEAK TEMPERATURE",
                    &peak_celsius
                        .map(|value| format!("{value:.0} °C"))
                        .unwrap_or_else(|| "—".to_string()),
                    peak_celsius.map(temperature_color).unwrap_or(MUTED),
                    &format!("{} reported sensors", s.temperatures.len()),
                );
                stat(
                    &mut cols[1],
                    "FANS",
                    &if s.fans.is_empty() {
                        "—".to_string()
                    } else {
                        s.fans.len().to_string()
                    },
                    if s.fans.is_empty() { MUTED } else { TEXT },
                    if s.fans.is_empty() {
                        "not exposed by firmware"
                    } else {
                        "reported speeds"
                    },
                );
                stat(
                    &mut cols[2],
                    "STORAGE HEALTH",
                    &if smart.is_empty() {
                        "—".to_string()
                    } else {
                        smart.to_uppercase()
                    },
                    smart_color(smart),
                    if specs.is_some() {
                        "drive-reported state"
                    } else {
                        "reading inventory…"
                    },
                );
                stat(
                    &mut cols[3],
                    "UPTIME",
                    &uptime_text(sysinfo::System::uptime()),
                    TEXT_DIM,
                    "since last boot",
                );
            });
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
                let columns = ((width / 190.0).floor() as usize).clamp(1, 6);
                let cell = ((width - 10.0 * columns.saturating_sub(1) as f32) / columns as f32)
                    .clamp(120.0, 180.0);
                egui::Grid::new("cores")
                    .num_columns(columns)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        for (i, usage) in s.cores.iter().enumerate() {
                            core_meter(ui, i, *usage, cell);
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
                empty_note(ui, sensor_unavailable_note());
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
                    ui.add_space(8.0);
                    if snapshot.cores.is_empty() {
                        empty_note(ui, "This provider does not report per-core usage.");
                    } else {
                        let width = ui.available_width();
                        let columns = ((width / 190.0).floor() as usize).clamp(1, 6);
                        let cell = ((width - 10.0 * columns.saturating_sub(1) as f32)
                            / columns as f32)
                            .clamp(120.0, 180.0);
                        egui::Grid::new("cpu-page-cores")
                            .num_columns(columns)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                for (i, usage) in snapshot.cores.iter().enumerate() {
                                    core_meter(ui, i, *usage, cell);
                                    if (i + 1) % columns == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    }
                    egui::CollapsingHeader::new("Recent CPU activity").show(ui, |ui| {
                        Plot::new("cpu-page-history").height(120.0).include_y(0.0).include_y(100.0).allow_drag(false).allow_zoom(false).allow_scroll(false).show_x(false).show(ui, |plot| {
                            for points in history::segments(&self.history, self.started, |sample| sample.cpu) { plot.line(Line::new(PlotPoints::from(points)).color(ACCENT)); }
                        });
                        ui.weak("Up to 60 batches; gaps mean missing or delayed readings.");
                    });

                    ui.add_space(14.0);
                    section_title(ui, "Specifications", Some("CPU-Z style identity, clocks and caches"));
                    match &specs {
                        Some(specs) => {
                            let cpu = &specs.cpu;
                            let mut rows: Vec<(String, String)> = Vec::new();
                            spec_row(&mut rows, "Name", cpu.name.as_str());
                            spec_row(&mut rows, "Vendor", vendor_text(&cpu.vendor));
                            spec_row(&mut rows, "Architecture", cpu.architecture.as_str());
                            spec_row(&mut rows, "Family / description", cpu.description.as_str());
                            spec_row(&mut rows, "Socket", cpu.socket.as_str());
                            spec_row(&mut rows, "Stepping", cpu.stepping.as_str());
                            match (cpu.physical_cores, cpu.logical_cores) {
                                (Some(physical), Some(logical)) => spec_row(&mut rows, "Cores / threads", format!("{physical} cores · {logical} threads")),
                                (None, Some(logical)) => spec_row(&mut rows, "Threads", logical.to_string()),
                                (Some(physical), None) => spec_row(&mut rows, "Cores", physical.to_string()),
                                (None, None) => {}
                            }
                            if cpu.logical_cores.is_some() && cpu.physical_cores.is_some_and(|physical| cpu.logical_cores.unwrap_or(0) > physical) {
                                spec_row(&mut rows, "Simultaneous threading", "Enabled");
                            }
                            if let Some(cores) = cpu.performance_cores { spec_row(&mut rows, "Performance cores", cores.to_string()); }
                            if let Some(cores) = cpu.efficiency_cores { spec_row(&mut rows, "Efficiency cores", cores.to_string()); }
                            if let Some(mhz) = cpu.base_mhz { spec_row(&mut rows, "Base clock", clock_text(mhz)); }
                            if let Some(mhz) = cpu.max_mhz { spec_row(&mut rows, "Max clock", clock_text(mhz)); }
                            if let Some(mhz) = cpu.current_mhz { spec_row(&mut rows, "Clock at collection", clock_text(mhz)); }
                            if let Some(value) = cpu.l1_data_bytes { spec_row(&mut rows, "L1 data cache", bytes(value)); }
                            if let Some(value) = cpu.l1_instruction_bytes { spec_row(&mut rows, "L1 instruction cache", bytes(value)); }
                            if let Some(value) = cpu.l2_bytes { spec_row(&mut rows, "L2 cache", bytes(value)); }
                            if let Some(value) = cpu.l3_bytes { spec_row(&mut rows, "L3 cache", bytes(value)); }
                            if let Some(value) = cpu.cache_line_bytes { spec_row(&mut rows, "Cache line", bytes(value)); }
                            spec_row(&mut rows, "Instruction sets", cpu.instructions.as_str());
                            if let Some(enabled) = cpu.virtualization {
                                spec_row(&mut rows, "Virtualization", if enabled { "Enabled in firmware" } else { "Not enabled" });
                            }
                            spec_row(&mut rows, "Processor ID", cpu.processor_id.as_str());
                            spec_grid(ui, "cpu-page-specs", &rows);
                            ui.weak("Values are OS-reported. Clocks move with load; cache sizes and identifiers are static.");
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Reading the processor inventory…").size(11.0).color(MUTED));
                            });
                        }
                    }

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

                    match &specs {
                        Some(specs) if !specs.graphics.is_empty() => {
                            for (index, gpu) in specs.graphics.iter().enumerate() {
                                ui.add_space(14.0);
                                section_title(ui, &format!("Adapter {} · {}", index + 1, gpu.name), Some("GPU-Z style · OS-reported"));
                                let mut rows: Vec<(String, String)> = Vec::new();
                                spec_row(&mut rows, "Vendor", gpu.vendor.as_str());
                                spec_row(&mut rows, "Chipset", gpu.chipset.as_str());
                                spec_row(&mut rows, "PCI vendor:device", gpu.device_id.as_str());
                                spec_row(&mut rows, "Bus location", gpu.bus.as_str());
                                if let Some(vram) = gpu.vram_bytes {
                                    spec_row(&mut rows, "Dedicated memory (reported)", decimal_bytes(vram));
                                }
                                spec_row(&mut rows, "Driver", gpu.driver.as_str());
                                spec_row(&mut rows, "Driver date", gpu.driver_date.as_str());
                                spec_row(&mut rows, "Video BIOS", gpu.bios.as_str());
                                spec_row(&mut rows, "BIOS date", gpu.bios_date.as_str());
                                spec_row(&mut rows, "Display mode", gpu.current_mode.as_str());
                                spec_row(&mut rows, "Refresh rate", gpu.refresh.as_str());
                                spec_row(&mut rows, "Status", gpu.status.as_str());
                                spec_grid(ui, &format!("gpu-page-spec-{index}"), &rows);
                            }
                            ui.weak("Video BIOS and dedicated memory come from the installed display driver. PCIe link width and GPU clocks need a vendor library.");
                        }
                        Some(_) => empty_note(ui, "The OS reported no display adapter. Add-in cards need an installed driver to appear here."),
                        None => {
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Reading the display adapter inventory…").size(11.0).color(MUTED));
                            });
                        }
                    }

                    if !snapshot.adapters.is_empty() {
                        ui.add_space(12.0);
                        egui::CollapsingHeader::new("Graphics API capabilities").default_open(true).show(ui, |ui| {
                            section_title(ui, "Adapter capabilities", Some("wgpu device limits"));
                            let mut rows: Vec<(String, String)> = Vec::new();
                            for (index, adapter) in snapshot.adapters.iter().enumerate() {
                                let limits = adapter.handle.limits();
                                spec_row(&mut rows, format!("Adapter {index}"), &adapter.name);
                                spec_row(&mut rows, "Backend", &adapter.backend);
                                spec_row(&mut rows, "Vendor / device ID", &format!("{:#06x} / {:#06x}", adapter.vendor_id, adapter.device_id));
                                spec_row(&mut rows, "Max texture (2D)", &format!("{} px", limits.max_texture_dimension_2d));
                                spec_row(&mut rows, "Max compute workgroup", &format!("{} × {} × {}", limits.max_compute_workgroup_size_x, limits.max_compute_workgroup_size_y, limits.max_compute_workgroup_size_z));
                                spec_row(&mut rows, "Max storage buffer binding", &format!("{} MiB", limits.max_storage_buffer_binding_size / (1024 * 1024)));
                                spec_row(&mut rows, "Optional features", &adapter.handle.features().bits().count_ones().to_string());
                            }
                            spec_grid(ui, "gpu-capabilities", &rows);
                            ui.weak("Limits come from the graphics API and describe capability, not current clocks or temperature.");
                        });
                    }
                    if ui.button("Open frame analysis").clicked() { self.page = Page::Frames; }
                }
                Page::Memory => {
                    section_title(ui, "Memory", Some("OS-reported usage and installed modules"));
                    if snapshot.memory_total == 0 { ui.weak("Memory usage unavailable."); }
                    else { ui.label(format!("{:.1} / {:.1} GiB used · {:.1} GiB available", gib(snapshot.memory_used), gib(snapshot.memory_total), gib(snapshot.memory_available))); }
                    ui.add_space(6.0);
                    bar(ui, if snapshot.memory_total > 0 { snapshot.memory_used as f32 / snapshot.memory_total as f32 } else { 0.0 }, VIOLET, 6.0);
                    egui::CollapsingHeader::new("Recent memory activity").show(ui, |ui| {
                        Plot::new("memory-page-history").height(120.0).include_y(0.0).include_y(100.0).allow_drag(false).allow_zoom(false).allow_scroll(false).show_x(false).show(ui, |plot| {
                            for points in history::segments(&self.history, self.started, |sample| sample.memory) { plot.line(Line::new(PlotPoints::from(points)).color(VIOLET)); }
                        });
                        ui.weak("Up to 60 batches; gaps mean missing or delayed readings.");
                    });

                    ui.add_space(14.0);
                    section_title(ui, "Installed modules", Some("CPU-Z style · per slot"));
                    match &specs {
                        Some(specs) => {
                            let memory = &specs.memory;
                            let mut rows: Vec<(String, String)> = Vec::new();
                            if let Some(total) = memory.total_bytes { spec_row(&mut rows, "Installed memory", decimal_bytes(total)); }
                            if let (Some(used), Some(total)) = (memory.slots_used, memory.slots_total) {
                                spec_row(&mut rows, "Slots", format!("{used} used · {} free", total.saturating_sub(used)));
                            } else if let Some(total) = memory.slots_total {
                                spec_row(&mut rows, "Slots", total.to_string());
                            }
                            if let Some(max) = memory.max_capacity_bytes { spec_row(&mut rows, "Maximum capacity", decimal_bytes(max)); }
                            if let Some(ecc) = memory.ecc { spec_row(&mut rows, "ECC", if ecc { "Enabled" } else { "Not enabled" }); }
                            if let Some(upgradeable) = memory.upgradeable { spec_row(&mut rows, "Upgradeable", if upgradeable { "Yes" } else { "No" }); }
                            spec_grid(ui, "memory-page-summary", &rows);
                            if memory.modules.is_empty() {
                                empty_note(ui, "No memory module records were reported. Unified or soldered memory has no removable modules.");
                            }
                            for (index, module) in memory.modules.iter().enumerate() {
                                ui.add_space(12.0);
                                let heading = if module.slot.is_empty() { format!("Module {}", index + 1) } else { module.slot.clone() };
                                let size = module.size_bytes.map(decimal_bytes).unwrap_or_default();
                                section_title(ui, &heading, if size.is_empty() { None } else { Some(&size) });
                                let mut rows: Vec<(String, String)> = Vec::new();
                                spec_row(&mut rows, "Type", module.kind.as_str());
                                spec_row(&mut rows, "Form factor", module.form_factor.as_str());
                                spec_row(&mut rows, "Bank", module.bank.as_str());
                                if let Some(speed) = module.speed_mhz { spec_row(&mut rows, "Rated speed", format!("{speed} MT/s")); }
                                if let Some(speed) = module.configured_mhz { spec_row(&mut rows, "Running speed", format!("{speed} MT/s")); }
                                if let Some(mv) = module.configured_voltage_mv { spec_row(&mut rows, "Configured voltage", format!("{:.3} V", mv as f64 / 1000.0)); }
                                if let (Some(min), Some(max)) = (module.min_voltage_mv, module.max_voltage_mv) {
                                    if min != max { spec_row(&mut rows, "Voltage range", format!("{:.3}–{:.3} V", min as f64 / 1000.0, max as f64 / 1000.0)); }
                                }
                                if let (Some(data), Some(total)) = (module.data_width, module.total_width) {
                                    spec_row(&mut rows, "Bus width", if total > data { format!("{data}-bit data · {total}-bit total (ECC)") } else { format!("{data}-bit") });
                                }
                                spec_row(&mut rows, "Manufacturer", module.manufacturer.as_str());
                                spec_row(&mut rows, "Part number", module.part_number.as_str());
                                spec_row(&mut rows, "Serial", module.serial.as_str());
                                spec_grid(ui, &format!("memory-page-module-{index}"), &rows);
                            }
                            ui.weak("Timings (CL, tRCD, tRP, tRAS) and XMP/EXPO profiles need raw SPD reads and are not shown.");
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Reading the memory inventory…").size(11.0).color(MUTED));
                            });
                        }
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

                    // A dedicated sensor graph, like the real-time graphs in HWMonitor.
                    ui.add_space(14.0);
                    section_title(ui, "Temperature history", Some("up to 60 recent batches"));
                    let keys = history::temperature_keys(&self.history);
                    if keys.is_empty() {
                        ui.weak("Waiting for temperature samples…");
                    } else {
                        if !keys.iter().any(|key| key == &self.selected_temp_key) {
                            self.selected_temp_key = keys[0].clone();
                        }
                        let selected = self.selected_temp_key.clone();
                        ui.horizontal_wrapped(|ui| {
                            egui::ComboBox::from_id_salt("thermal-series")
                                .selected_text(&selected)
                                .show_ui(ui, |ui| {
                                    for key in &keys {
                                        ui.selectable_value(&mut self.selected_temp_key, key.clone(), key);
                                    }
                                });
                            if let Some((low, high, mean)) = history::range(&self.history, |sample| history::temperature_value(sample, &selected)) {
                                ui.label(RichText::new(format!("{low:.1}–{high:.1} °C · avg {mean:.1}")).size(11.0).color(TEXT_DIM));
                            }
                        });
                        Plot::new("thermal-history")
                            .height(150.0)
                            .include_y(0.0)
                            .auto_bounds([true, true])
                            .allow_drag(false)
                            .allow_zoom(false)
                            .allow_scroll(false)
                            .show_x(false)
                            .show(ui, |plot| {
                                for points in history::segments(&self.history, self.started, |sample| history::temperature_value(sample, &selected)) {
                                    plot.line(Line::new(PlotPoints::from(points)).color(AMBER).width(1.8_f32));
                                }
                            });
                        ui.weak("A gap means the sensor was missing or the sample was delayed; gaps are never drawn as a line.");
                        egui::CollapsingHeader::new("Export sensors to CSV").show(ui, |ui| {
                            ui.label(RichText::new("One sensor").size(10.5).color(MUTED));
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut self.sensor_log_path);
                                if ui.button("Export CSV").clicked() {
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
                            ui.label(RichText::new("All sensors (one column each)").size(10.5).color(MUTED));
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut self.all_sensor_log_path);
                                if ui.button("Export all").clicked() {
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

                    // Advisory threshold: a sustained breach of a fresh reading.
                    ui.add_space(14.0);
                    section_title(ui, "Temperature alert", Some("advisory only · no automatic stop"));
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Warn when any sensor reaches");
                        ui.add(egui::DragValue::new(&mut self.alert_celsius).range(30.0..=125.0).speed(0.5).suffix(" °C"));
                        ui.label("for");
                        ui.add(egui::DragValue::new(&mut self.alert_hold_secs).range(1..=120).speed(1).suffix(" s"));
                        ui.selectable_value(&mut self.alert_severity, Severity::Warning, "Warning");
                        ui.selectable_value(&mut self.alert_severity, Severity::Critical, "Critical");
                    });
                    let rule = Threshold {
                        celsius: self.alert_celsius,
                        hold: Duration::from_secs(self.alert_hold_secs),
                        severity: self.alert_severity,
                    };
                    if !rule.is_valid() {
                        ui.colored_label(AMBER, "Choose a threshold between 30 and 125 °C, held for 1–120 seconds.");
                    } else if let Some(since) = self.alert.active_since() {
                        let color = if rule.severity == Severity::Critical { RED } else { AMBER };
                        ui.horizontal_wrapped(|ui| {
                            pill(ui, rule.severity.label(), color);
                            ui.colored_label(color, format!("Temperature has been at or above {:.0} °C for {:.0}s.", rule.celsius, since.elapsed().as_secs_f32()));
                            if ui.button("Acknowledge").clicked() { self.alert.clear(); }
                        });
                    } else {
                        ui.weak(format!("No active alert. Highest observation this session: {}.", self.alert.peak_celsius().map(|v| format!("{v:.1} °C")).unwrap_or_else(|| "unavailable".into())));
                    }
                    if ui.button("Reset alert state").clicked() { self.alert.clear(); }

                    // Polling cost is user-controlled; slower sampling reduces CPU work.
                    ui.add_space(14.0);
                    section_title(ui, "Sampling", Some("applies to all readings and logs"));
                    let mut period_ms = self.poll_ms.load(Ordering::Relaxed) as f64;
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(egui::Slider::new(&mut period_ms, 250.0..=5000.0).step_by(250.0).suffix(" ms")).changed() {
                            self.poll_ms.store(period_ms.round() as u64, Ordering::Relaxed);
                        }
                        ui.weak("Slower sampling lowers background CPU usage; faster sampling gives a denser chart.");
                    });
                }
                Page::Motherboard => {
                    section_title(ui, "Mainboard and parts", Some("read-only system inventory"));
                    match &specs {
                        Some(specs) => {
                            let mut system_rows: Vec<(String, String)> = Vec::new();
                            spec_row(&mut system_rows, "System", specs.model_name.as_str());
                            spec_row(&mut system_rows, "Model identifier", specs.model_id.as_str());
                            spec_row(&mut system_rows, "Chassis", specs.chassis.as_str());
                            spec_row(&mut system_rows, "System firmware", specs.firmware.as_str());
                            spec_row(&mut system_rows, "Operating system", format!("{} {}", specs.os_name, specs.os_version).trim());
                            spec_row(&mut system_rows, "Kernel", specs.kernel.as_str());
                            if let Some(battery) = &specs.battery { spec_row(&mut system_rows, "Battery", battery.as_str()); }
                            spec_row(&mut system_rows, "Uptime", uptime_text(sysinfo::System::uptime()));
                            spec_grid(ui, "parts-system", &system_rows);

                            ui.add_space(14.0);
                            section_title(ui, "Board", Some("firmware-reported identity"));
                            let mut board_rows: Vec<(String, String)> = Vec::new();
                            spec_row(&mut board_rows, "Vendor", specs.board_vendor.as_str());
                            spec_row(&mut board_rows, "Model", specs.board_model.as_str());
                            spec_row(&mut board_rows, "Version / revision", specs.board_version.as_str());
                            spec_row(&mut board_rows, "Board identifier", specs.board_id.as_deref().unwrap_or(""));
                            spec_grid(ui, "parts-board", &board_rows);
                            ui.weak("Chipset, VRM and per-board sensors need a kernel driver and are not read.");

                            ui.add_space(14.0);
                            section_title(ui, "Displays", Some("decoded from the monitor identity block"));
                            if specs.monitors.is_empty() {
                                empty_note(ui, "No display identity was reported. External monitors show up only when the OS reads their EDID.");
                            } else {
                                for (index, monitor) in specs.monitors.iter().enumerate() {
                                    let mut rows: Vec<(String, String)> = Vec::new();
                                    spec_row(&mut rows, "Manufacturer", monitor.manufacturer.as_str());
                                    spec_row(&mut rows, "Model", monitor.model.as_str());
                                    spec_row(&mut rows, "Serial", monitor.serial.as_str());
                                    if let Some(year) = monitor.year {
                                        let detail = match monitor.week {
                                            Some(week) => format!("{year} · week {week}"),
                                            None => year.to_string(),
                                        };
                                        spec_row(&mut rows, "Manufactured", detail);
                                    }
                                    spec_grid(ui, &format!("parts-monitor-{index}"), &rows);
                                    if index + 1 < specs.monitors.len() { ui.add_space(8.0); }
                                }
                            }

                            ui.add_space(14.0);
                            section_title(ui, "Network adapters", Some("physical parts, not traffic counters"));
                            if specs.network_adapters.is_empty() {
                                empty_note(ui, "No network adapter was reported.");
                            } else {
                                for adapter in &specs.network_adapters {
                                    let mut detail: Vec<String> = Vec::new();
                                    if !adapter.description.is_empty() { detail.push(adapter.description.clone()); }
                                    if !adapter.status.is_empty() { detail.push(adapter.status.clone()); }
                                    if !adapter.speed.is_empty() { detail.push(adapter.speed.clone()); }
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(&adapter.name).size(12.0).color(TEXT));
                                        if adapter.status.eq_ignore_ascii_case("up") { chip(ui, "UP", GREEN); }
                                        ui.label(RichText::new(detail.join(" · ")).size(10.5).color(MUTED));
                                    });
                                }
                            }
                            ui.weak("The OS reports adapter names and link speeds; radio signal quality and antenna info are unavailable.");
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Reading the system inventory…").size(11.0).color(MUTED));
                            });
                        }
                    }
                }
                Page::Drivers => {
                    section_title(ui, "Installed drivers", Some("OS driver store · read-only"));
                    match &specs {
                        Some(specs) if !specs.drivers.is_empty() => {
                            let mut classes: Vec<&str> = specs.drivers.iter().map(|driver| driver.class_name.as_str()).collect();
                            classes.sort_unstable();
                            classes.dedup();
                            if !classes.iter().any(|class| *class == self.driver_class) {
                                self.driver_class = "All".into();
                            }
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Find:");
                                ui.add(egui::TextEdit::singleline(&mut self.driver_query).hint_text("Driver or device name").desired_width(220.0));
                                ui.add_space(6.0);
                                ui.label("Class:");
                                egui::ComboBox::from_id_salt("driver-class")
                                    .selected_text(if self.driver_class == "All" { "All classes".to_owned() } else { driver_class_label(&self.driver_class).to_owned() })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.driver_class, "All".into(), "All classes");
                                        for class in &classes {
                                            ui.selectable_value(&mut self.driver_class, (*class).to_owned(), format!("{} ({})", driver_class_label(class), specs.drivers.iter().filter(|driver| driver.class_name == *class).count()));
                                        }
                                    });
                            });
                            let query = self.driver_query.to_lowercase();
                            let filtered: Vec<_> = specs.drivers.iter().filter(|driver| {
                                (self.driver_class == "All" || driver.class_name == self.driver_class)
                                    && (query.is_empty()
                                        || driver.name.to_lowercase().contains(&query)
                                        || driver.provider.to_lowercase().contains(&query))
                            }).collect();
                            ui.add_space(4.0);
                            ui.weak(format!("{} of {} driver records · {} classes", filtered.len(), specs.drivers.len(), classes.len()));
                            if filtered.is_empty() {
                                empty_note(ui, "No driver matches the current filter.");
                            } else {
                                egui::Grid::new("drivers-grid")
                                    .num_columns(4)
                                    .spacing([22.0, 7.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        for heading in ["Driver", "Class", "Version", "Signature"] {
                                            ui.label(RichText::new(heading).size(10.5).strong().color(MUTED));
                                        }
                                        ui.end_row();
                                        for driver in filtered {
                                            ui.vertical(|ui| {
                                                ui.label(RichText::new(&driver.name).size(11.5).color(TEXT));
                                                let mut meta: Vec<String> = Vec::new();
                                                if !driver.provider.is_empty() { meta.push(driver.provider.clone()); }
                                                if !driver.date.is_empty() { meta.push(driver.date.clone()); }
                                                ui.label(RichText::new(meta.join(" · ")).size(10.0).color(MUTED));
                                            });
                                            ui.label(RichText::new(driver_class_label(&driver.class_name)).size(11.0).color(TEXT_DIM));
                                            ui.label(RichText::new(if driver.version.is_empty() { "—" } else { &driver.version }).size(11.0).color(TEXT_DIM));
                                            match driver.signed {
                                                Some(true) => { chip(ui, "SIGNED", GREEN); }
                                                Some(false) => { chip(ui, "UNSIGNED", AMBER); }
                                                None => { ui.label(RichText::new("—").color(MUTED)); }
                                            }
                                            ui.end_row();
                                        }
                                    });
                            }
                            ui.weak("Signature state is what the OS reports for the installed package. A signed driver is not a guarantee of stability.");
                        }
                        Some(_) => empty_note(ui, "The OS driver store reported no matching hardware drivers."),
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Reading the OS driver store…").size(11.0).color(MUTED));
                            });
                        }
                    }
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
                    // Keep the primary action visible: scroll the configuration,
                    // not the Start button (Apple HIG: consistent action location).
                    let config_height = (ui.available_height() - 62.0).max(150.0);
                    egui::ScrollArea::vertical()
                        .id_salt("stress-config")
                        .max_height(config_height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
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
                        ui.add_space(2.0);
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
                            ui.add_space(6.0);
                            picker(ui, "gpu-intensity", "Intensity", &mut self.gpu_intensity,
                                &GpuIntensity::ALL.map(|v| (v, v.label())));
                        }
                        Kind::Memory => {
                            ui.label(
                                RichText::new(format!(
                                    "Write/read patterns over the allocated region, limited to 256 MiB and one quarter of available RAM ({:.0} MiB).",
                                    s.memory_available as f64 / 4.0 / 1048576.0
                                ))
                                .size(11.0)
                                .color(TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            ui.add(egui::Slider::new(&mut self.mib, 1..=256).text("MiB"));
                            ui.add_space(6.0);
                            picker(ui, "mem-pattern", "Pattern", &mut self.mem_pattern,
                                &MemPattern::ALL.map(|v| (v, v.label())));
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
                            ui.add_space(6.0);
                            picker(ui, "disk-pattern", "Pattern", &mut self.disk_pattern,
                                &DiskPattern::ALL.map(|v| (v, v.label())));
                        }
                        Kind::Combined => {
                            ui.label(
                                RichText::new(
                                    "CPU integer rounds and memory write/read patterns run at the same time, in the style of a combined system load. Both checks stay within their usual limits.",
                                )
                                .size(11.0)
                                .color(TEXT_DIM),
                            );
                            ui.add_space(4.0);
                            ui.add(egui::Slider::new(&mut self.workers, 1..=8).text("CPU workers"));
                            ui.add(egui::Slider::new(&mut self.mib, 1..=256).text("memory MiB"));
                            ui.add_space(6.0);
                            picker(ui, "combined-mem-pattern", "Memory pattern", &mut self.mem_pattern,
                                &MemPattern::ALL.map(|v| (v, v.label())));
                        }
                    }
                    ui.add_space(6.0);
                    duration_presets(ui, &mut self.seconds);
                    ui.add_space(4.0);
                    ui.add(egui::Slider::new(&mut self.seconds, 1..=600).text("max seconds"));
                        });
                    ui.add_space(4.0);
                    let start = ui.add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new(RichText::new("Start test").strong().color(Color32::from_rgb(9, 18, 30)))
                            .fill(ACCENT)
                            .corner_radius(CornerRadius::same(9)),
                    );
                    if start.clicked() {
                        self.run_peak_c = None;
                        self.auto_stop_fired = false;
                        self.last_live_units = 0;
                        self.last_live_at = Instant::now();
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
                    let fraction = (elapsed / self.seconds as f32).clamp(0.0, 1.0);
                    let remaining = (self.seconds as f32 - elapsed).max(0.0);
                    let percent = (fraction * 100.0).round() as u32;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{elapsed:.1}s"))
                                .size(30.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            chip(ui, &format!("{percent}%"), ACCENT);
                        });
                    });
                    ui.label(
                        RichText::new(format!("about {remaining:.0}s left of the {0}s limit", self.seconds))
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.add_space(6.0);
                    // Keep the progress bar moving and let people estimate the
                    // wait — both are Apple HIG guidance for determinate tasks.
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .fill(ACCENT)
                            .corner_radius(CornerRadius::same(6))
                            .desired_height(8.0),
                    );
                    ui.add_space(14.0);
                    ui.label(RichText::new("WORK PROGRESS").size(9.5).strong().color(MUTED));
                    ui.add_space(4.0);
                    {
                        let live = self.state.lock().ok().map(|s| s.live.clone()).unwrap_or_default();
                        let now = Instant::now();
                        let dt = now.duration_since(self.last_live_at).as_secs_f64().max(1e-3);
                        let rate = if self.last_live_units > 0 || live.units > 0 {
                            (live.units.saturating_sub(self.last_live_units)) as f64 / dt
                        } else {
                            0.0
                        };
                        self.last_live_units = live.units;
                        self.last_live_at = now;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Phase").size(11.0).color(MUTED));
                            ui.label(RichText::new(&live.phase).size(11.0).strong().color(TEXT));
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Done").size(11.0).color(MUTED));
                            ui.label(
                                RichText::new(format!("{} {}", live.units, live.unit_label))
                                    .size(12.5)
                                    .strong()
                                    .color(ACCENT),
                            );
                            ui.add_space(12.0);
                            ui.label(RichText::new("Rate").size(11.0).color(MUTED));
                            ui.label(
                                RichText::new(if rate >= 1.0 {
                                    format!("{rate:.0}/s")
                                } else {
                                    format!("{rate:.2}/s")
                                })
                                .size(12.0)
                                .strong()
                                .color(TEXT_DIM),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Errors").size(11.0).color(MUTED));
                            ui.label(
                                RichText::new(live.errors.to_string())
                                    .size(12.5)
                                    .strong()
                                    .color(if live.errors > 0 { RED } else { GREEN }),
                            );
                        });
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("LIVE READINGS").size(9.5).strong().color(MUTED));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("CPU").size(11.0).color(MUTED));
                        ui.label(
                            RichText::new(format!("{:.0}%", s.cpu_usage))
                                .size(12.5)
                                .strong()
                                .color(load_color(s.cpu_usage / 100.0)),
                        );
                        ui.add_space(14.0);
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
                            .size(12.5)
                            .strong()
                            .color(VIOLET),
                        );
                    });
                    ui.add_space(8.0);
                    // Peak temperature with an at-a-glance safety verdict is the
                    // signal mainstream stress tools surface; it belongs beside
                    // the Start/Stop controls, not buried on the Thermal page.
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Peak temp").size(11.0).color(MUTED));
                        match self.run_peak_c {
                            Some(peak) => {
                                ui.label(
                                    RichText::new(format!("{peak:.0} °C"))
                                        .size(12.5)
                                        .strong()
                                        .color(temperature_color(peak)),
                                );
                                let (label, color) = thermal_safety(peak, self.alert_celsius);
                                chip(ui, label, color);
                            }
                            None => {
                                ui.label(RichText::new("no sensor").size(11.0).color(MUTED));
                            }
                        }
                    });
                    ui.add_space(6.0);
                    mini_temperature_graph(ui, &self.history);
                    ui.add_space(12.0);
                    if ui
                        .add_sized(
                            [ui.available_width(), 32.0],
                            egui::Button::new(RichText::new("Stop test · Esc").color(Color32::from_rgb(30, 18, 6)))
                                .fill(AMBER)
                                .corner_radius(CornerRadius::same(9)),
                        )
                        .clicked()
                    {
                        self.cancel.store(true, Ordering::Relaxed);
                        self.message = "Stopping after the current work or I/O operation…".into();
                    }
                    ui.add_space(10.0);
                    if self.auto_stop_on_heat {
                        callout(ui, GREEN, "Auto-stop armed (check only)",
                            "This check will stop itself if peak temperature reaches the advisory limit. No fan, clock or other hardware setting is changed. A blocked driver or disk operation can still delay stopping.");
                    } else {
                        callout(ui, AMBER, "No automatic shutdown",
                            "Watch the peak temperature above. Stop the check yourself if it climbs; a blocked driver or disk operation can also delay stopping.");
                    }
                } else {
                    let last = self
                        .state
                        .lock()
                        .ok()
                        .and_then(|s| s.results.iter().rev().find(|r| fixed.is_none_or(|kind| r.kind == kind)).cloned());
                    match last {
                        Some(r) => {
                            // A plain-language verdict confirms the outcome of a
                            // significant action (Apple HIG: confirm completion).
                            let verdict = match r.status {
                                Status::Completed => "PASSED",
                                Status::Stopped => "STOPPED",
                                Status::Failed => "FAILED",
                                Status::Inconclusive => "INCONCLUSIVE",
                                Status::Unavailable => "UNAVAILABLE",
                            };
                            let tone = status_color(&r.status);
                            ui.horizontal(|ui| {
                                chip(ui, verdict, tone);
                                ui.label(
                                    RichText::new(kind_label(r.kind))
                                        .size(12.5)
                                        .strong()
                                        .color(TEXT),
                                );
                            });
                            ui.add_space(8.0);
                            ui.label(RichText::new(&r.detail).size(11.5).color(TEXT_DIM));
                            ui.add_space(6.0);
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
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    timestamp_text(r.timestamp),
                                    config_summary(&r.config)
                                ))
                                .size(10.0)
                                .color(MUTED),
                            );
                            error_details(ui, &r.errors, r.error_count);
                            ui.add_space(8.0);
                            if ui.button("View all reports").clicked() {
                                self.page = Page::Reports;
                            }
                        }
                        None => {
                            empty_note(
                                ui,
                                "Nothing has run yet. Pick a check on the left, review its limits, then start it.",
                            );
                        }
                    }
                    ui.add_space(12.0);
                    // Safety option sits with the run status: it is a property
                    // of the next run, and the right column has room for it.
                    let auto_stop = ui.checkbox(
                        &mut self.auto_stop_on_heat,
                        format!("Stop check at {:.0} °C (advisory)", self.alert_celsius),
                    );
                    auto_stop.on_hover_text("Stops only this check when the observed peak reaches the alert limit. Never changes fans, clocks or hardware.");
                    ui.add_space(10.0);
                    callout(ui, MUTED, "Read-only checks, never benchmarks",
                        "These bounded checks verify correctness, not raw speed or full burn-in. The machine has no automatic temperature shutdown; supervise any stress run.");
                }
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
        spec_row(&mut cpu_rows, "Socket", specs.cpu.socket.as_str());
        spec_row(&mut cpu_rows, "Stepping", specs.cpu.stepping.as_str());
        spec_row(
            &mut cpu_rows,
            "Family / description",
            specs.cpu.description.as_str(),
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
        if let Some(mhz) = specs.cpu.current_mhz {
            spec_row(&mut cpu_rows, "Clock at collection", clock_text(mhz));
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
        if let Some(enabled) = specs.cpu.virtualization {
            spec_row(
                &mut cpu_rows,
                "Virtualization (firmware)",
                if enabled { "Enabled" } else { "Not enabled" },
            );
        }
        spec_row(
            &mut cpu_rows,
            "Instruction sets",
            specs.cpu.instructions.as_str(),
        );
        spec_row(
            &mut cpu_rows,
            "Processor ID",
            specs.cpu.processor_id.as_str(),
        );

        let mut memory_rows: Vec<(String, String)> = Vec::new();
        if let Some(total) = specs.memory.total_bytes {
            spec_row(&mut memory_rows, "Installed", decimal_bytes(total));
        }
        if let (Some(used), Some(total)) = (specs.memory.slots_used, specs.memory.slots_total) {
            spec_row(
                &mut memory_rows,
                "Slots",
                format!("{used} used · {} free", total.saturating_sub(used)),
            );
        }
        if let Some(max) = specs.memory.max_capacity_bytes {
            spec_row(&mut memory_rows, "Maximum capacity", decimal_bytes(max));
        }
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
            if let Some(configured) = module.configured_mhz {
                if Some(configured) != module.speed_mhz {
                    detail.push_str(&format!(" (running {configured} MHz)"));
                }
            }
            if !module.form_factor.is_empty() {
                detail.push_str(&format!(" · {}", module.form_factor));
            }
            if !module.manufacturer.is_empty() {
                detail.push_str(&format!(" · {}", module.manufacturer));
            }
            if !module.part_number.is_empty() {
                detail.push_str(&format!(" · {}", module.part_number));
            }
            if !module.serial.is_empty() {
                detail.push_str(&format!(" · serial {}", module.serial));
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
            spec_row(&mut graphics_rows, "Adapter driver", gpu.driver.as_str());
            spec_row(&mut graphics_rows, "Video BIOS", gpu.bios.as_str());
        }
        if !specs.gpu_driver.is_empty() {
            let detail = if specs.gpu_driver_date.is_empty() {
                specs.gpu_driver.clone()
            } else {
                format!("{} ({})", specs.gpu_driver, specs.gpu_driver_date)
            };
            spec_row(&mut graphics_rows, "OS driver", detail);
        }
        spec_row(&mut graphics_rows, "Display", specs.display.as_str());

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
            if !drive.serial.is_empty() {
                detail.push_str(&format!(" · serial {}", drive.serial));
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

    fn reports(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let records = self
            .state
            .lock()
            .map(|s| s.results.clone())
            .unwrap_or_default();

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "System report", Some("CPU-Z / GPU-Z style inventory"));
            ui.weak("Exports the Details inventory and current sensor readings to a new text or JSON file. Nothing is uploaded.");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.system_report_format, Format::Text, "Text");
                ui.selectable_value(&mut self.system_report_format, Format::Json, "JSON");
                ui.checkbox(&mut self.system_report_redact, "Redact identifiers");
            });
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.system_report_path);
                if ui.button("Export system report").clicked() {
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
            ui.add_space(4.0);
            ui.label(RichText::new("Redaction removes serial numbers, volume paths, interface names and connected-device names. Review the file before sharing.").size(10.0).color(MUTED));
        });

        ui.add_space(12.0);

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

        // Side-by-side result comparison, in the style of a pro diagnostic
        // report tool. Uses only results already collected this session.
        if records.len() >= 2 {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                section_title(ui, "Compare runs", Some("this session only"));
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
                    for (slot, selected) in [(&mut self.compare_a, "A"), (&mut self.compare_b, "B")]
                        .into_iter()
                        .enumerate()
                    {
                        let (value, label) = selected;
                        let width = cols[slot].available_width() - 24.0;
                        cols[slot].horizontal(|ui| {
                            ui.label(RichText::new(label).size(11.0).strong().color(MUTED));
                            egui::ComboBox::from_id_salt(format!("compare-{label}"))
                                .width(width)
                                .selected_text(RichText::new(labels[*value].clone()).size(11.0))
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
                ui.add_space(8.0);
                comparison_row(ui, "Result", &a.status.to_string(), &b.status.to_string());
                comparison_row(
                    ui,
                    "Verified",
                    &format!(
                        "{} {}",
                        a.verified_units
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        a.verified_unit
                    ),
                    &format!(
                        "{} {}",
                        b.verified_units
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "unknown".into()),
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
                comparison_row(ui, "Settings", &config_summary(&a.config), &config_summary(&b.config));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Different settings or durations make raw unit counts non-comparable. No score is implied.")
                        .size(10.0)
                        .color(MUTED),
                );
            });
            ui.add_space(12.0);
        }

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
                error_details(ui, &r.errors, r.error_count);
                ui.push_id(format!("certificate-{record_index}"), |ui| {
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.certificate_path);
                        if ui.button("Save run summary").clicked() {
                            self.message = match report::export_certificate(
                                std::path::Path::new(&self.certificate_path),
                                r,
                                self.run_peak_c,
                            ) {
                                Ok(()) => format!("Saved {}", self.certificate_path),
                                Err(error) => format!("Export failed: {error}"),
                            };
                        }
                    });
                    ui.label(
                        RichText::new("One-page, shareable summary with no paths or hardware identifiers. Not a benchmark score or certification.")
                            .size(10.0)
                            .color(MUTED),
                    );
                });
                ui.add_space(4.0);
                ui.push_id(record_index, |ui| {
                    egui::CollapsingHeader::new("Evidence and limits").show(ui, |ui| {
                        ui.strong("Test settings");
                        ui.label(format!("Time limit: {} seconds · {}", r.config.seconds, config_summary(&r.config)));
                        if r.kind == Kind::Disk { ui.label(format!("Directory: {} · write cap: {} MiB", r.config.directory.display(), r.config.disk_write_limit_mib)); }
                        if r.kind == Kind::Memory || r.kind == Kind::Combined { ui.label(format!("Pattern: {}", r.config.mem_pattern.label())); }
                        if r.kind == Kind::Disk { ui.label(format!("Pattern: {}", r.config.disk_pattern.label())); }
                        if r.kind == Kind::Gpu { ui.label(format!("Intensity: {}", r.config.gpu_intensity.label())); }
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
                            Kind::Combined => "CPU and memory checks run together to add combined load, but this is not a PSU/voltage certification or a full-RAM test.",
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
                    if self.alert.active_since().is_some() {
                        pill(ui, "TEMP ALERT", RED);
                        ui.add_space(8.0);
                    }
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
                        ui.add_space(10.0);
                        if s.ready {
                            let memory = if s.memory_total > 0 {
                                format!(
                                    "RAM {:.0}%",
                                    s.memory_used as f32 * 100.0 / s.memory_total as f32
                                )
                            } else {
                                "RAM —".to_string()
                            };
                            let cpu = if s.cpu_usage.is_finite() && !s.cores.is_empty() {
                                format!("CPU {:.0}%", s.cpu_usage)
                            } else {
                                "CPU —".to_string()
                            };
                            ui.label(
                                RichText::new(format!("{cpu}   {memory}"))
                                    .size(10.0)
                                    .color(MUTED),
                            );
                        }
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
        let snapshot = self
            .snapshot
            .lock()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_default();
        let now = Instant::now();
        // The alert only sees a fresh provider sample. A stale snapshot pauses
        // the streak instead of pretending the machine has cooled down.
        let fresh = snapshot
            .updated_at
            .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(3));
        let hottest = fresh
            .then(|| {
                snapshot
                    .temperatures
                    .iter()
                    .filter_map(|t| t.celsius.filter(|v| v.is_finite()))
                    .fold(None::<f32>, |peak, value| {
                        Some(peak.map_or(value, |previous: f32| previous.max(value)))
                    })
            })
            .flatten();
        // Track the hottest reading seen while a check runs, for the live
        // safety readout. A stale sample never raises the recorded peak.
        if running {
            if let Some(value) = hottest {
                self.run_peak_c = Some(self.run_peak_c.map_or(value, |peak| peak.max(value)));
            }
            // Opt-in safety: stop only the running check when the peak reaches
            // the advisory limit. Never alters fans, clocks or other hardware.
            if self.auto_stop_on_heat
                && !self.auto_stop_fired
                && self
                    .run_peak_c
                    .is_some_and(|peak| peak >= self.alert_celsius)
            {
                self.auto_stop_fired = true;
                self.cancel.store(true, Ordering::Relaxed);
                self.message = format!(
                    "Check stopped: peak temperature reached {:.0} °C. No hardware setting was changed.",
                    self.alert_celsius
                );
            }
        }
        let rule = Threshold {
            celsius: self.alert_celsius,
            hold: Duration::from_secs(self.alert_hold_secs),
            severity: self.alert_severity,
        };
        if self.alert.observe(hottest, now, rule) {
            self.message = format!(
                "{} temperature alert: at or above {:.0} °C for {}s. Advisory only.",
                rule.severity.label(),
                rule.celsius,
                self.alert_hold_secs
            );
        }
        // Apple-like keyboard navigation: Cmd/Ctrl+1…9 jumps between the main
        // pages, Cmd/Ctrl+Shift+1…6 reaches the remaining six, Escape requests
        // cancellation of the current bounded check.
        let jump = ctx.input(|input| {
            if !input.modifiers.command && !input.modifiers.ctrl {
                return None;
            }
            let keys = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ];
            let index = keys.iter().position(|key| input.key_pressed(*key))?;
            if input.modifiers.shift {
                Some(9 + index)
            } else {
                Some(index)
            }
        });
        if let Some(index) = jump {
            self.page = match index {
                0 => Page::Overview,
                1 => Page::Cpu,
                2 => Page::Gpu,
                3 => Page::Memory,
                4 => Page::Storage,
                5 => Page::Thermal,
                6 => Page::Details,
                7 => Page::Processes,
                8 => Page::Reports,
                // Shift+1…6 cover the pages that had no keyboard access.
                9 => Page::Motherboard,
                10 => Page::Drivers,
                11 => Page::Network,
                12 => Page::Bluetooth,
                13 => Page::Frames,
                14 => Page::Tests,
                _ => self.page,
            };
        }
        if running
            && !self.cancel.load(Ordering::Relaxed)
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.cancel.store(true, Ordering::Relaxed);
            self.message = "Stopping after the current work or I/O operation…".into();
        }
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
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new(self.page.title())
                            .size(28.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.add_space(3.0);
                    ui.label(RichText::new(self.page.subtitle()).size(12.5).color(MUTED));
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
                    | Page::Drivers
                    | Page::Network
                    | Page::Bluetooth => self.hardware_page(ui, &snapshot),
                    Page::Details => self.details(ui, &snapshot),
                    Page::Processes => self.processes(ui, &snapshot),
                    Page::Frames => self.frames(ui),
                    Page::Tests => self.tests(ui, &snapshot, None),
                    Page::Reports => self.reports(ui, &snapshot),
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
        // While the window is focused the monitor needs a few redraws per second
        // for fresh readings. In the background telemetry keeps sampling but the
        // interface redraws once a second, which keeps idle CPU cost low.
        let period = if ctx.input(|input| input.viewport().focused.unwrap_or(true)) {
            Duration::from_millis(250)
        } else {
            Duration::from_millis(1000)
        };
        ctx.request_repaint_after(period);
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
    visuals.faint_bg_color = Color32::from_rgb(50, 50, 52);
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
    // Apple-style type ramp: a 13 pt body, compact captions and one heading level.
    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::proportional(21.0)),
        (egui::TextStyle::Body, egui::FontId::proportional(13.0)),
        (egui::TextStyle::Button, egui::FontId::proportional(12.5)),
        (egui::TextStyle::Small, egui::FontId::proportional(11.0)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(11.5)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(13.0, 6.5);
    style.spacing.slider_width = 180.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.bar_inner_margin = 3.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.spacing.menu_margin = egui::Margin::same(8);
    style.animation_time = 0.12;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(70),
    };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.widgets.open.corner_radius = CornerRadius::same(8);
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
        .spacing([28.0, 8.0])
        .striped(true)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(RichText::new(label).size(11.5).color(MUTED));
                ui.label(RichText::new(value).size(12.0).color(TEXT_DIM));
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
    ui.add_space(4.0);
    ui.push_id(id, |ui| {
        let width = ui.available_width().max(260.0);
        let (header, _) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
        let right = header.right();
        {
            let painter = ui.painter();
            let font = egui::FontId::proportional(9.5);
            painter.text(
                egui::pos2(header.left() + 6.0, header.center().y),
                egui::Align2::LEFT_CENTER,
                "SENSOR",
                font.clone(),
                MUTED,
            );
            for (offset, text) in [(112.0, "CURRENT"), (60.0, "MIN"), (6.0, "MAX")] {
                painter.text(
                    egui::pos2(right - offset, header.center().y),
                    egui::Align2::RIGHT_CENTER,
                    text,
                    font.clone(),
                    MUTED,
                );
            }
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, row) in rows.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(
                    rect,
                    CornerRadius::same(6),
                    Color32::from_rgba_unmultiplied(255, 255, 255, 9),
                );
            }
            painter.text(
                egui::pos2(rect.left() + 6.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                truncate(row.label, 36),
                egui::FontId::proportional(11.5),
                TEXT_DIM,
            );
            let current = row
                .current
                .map(|value| format!("{value:.0} {}", row.unit))
                .unwrap_or_else(|| "—".to_string());
            let current_color = match row.current {
                Some(value) if row.unit == "°C" => temperature_color(value),
                Some(_) => TEXT,
                None => MUTED,
            };
            painter.text(
                egui::pos2(right - 112.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                current,
                egui::FontId::monospace(11.5),
                current_color,
            );
            for (offset, value) in [(60.0, row.min), (6.0, row.max)] {
                painter.text(
                    egui::pos2(right - offset, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    value
                        .map(|value| format!("{value:.0}"))
                        .unwrap_or_else(|| "—".to_string()),
                    egui::FontId::monospace(11.0),
                    MUTED,
                );
            }
            if index + 1 < rows.len() {
                painter.hline(
                    egui::Rangef::new(rect.left() + 6.0, rect.right() - 6.0),
                    rect.bottom(),
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 11)),
                );
            }
            response.on_hover_text(row.key);
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

/// Vendor prefixes and clock suffixes add noise to a one-line caption.
fn short_cpu_name(name: &str) -> String {
    let mut text = name.trim().to_string();
    for prefix in ["Intel(R) Core(TM) ", "Intel(R) ", "AMD ", "Apple "] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.to_string();
            break;
        }
    }
    if let Some((head, _)) = text.split_once(" CPU @") {
        text = head.to_string();
    }
    text
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

/// Why the sensor table has no rows, stated as the platform actually behaves.
fn sensor_unavailable_note() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "This firmware reported no ACPI thermal zone. Windows exposes temperature only through ACPI thermal zones (the Thermal Zone Information counters, or the WMI class when elevated), and states no per-core die temperature without a kernel driver."
    }
    #[cfg(not(target_os = "windows"))]
    {
        "No sensor provider is available here. The Details page lists what this platform exposes."
    }
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
        Page::Drivers => {
            // A driver chip: a small board with contact pins.
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(9.0, 9.0)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for dy in [-4.0, 0.0, 4.0] {
                line(
                    center + egui::vec2(-7.0, dy),
                    center + egui::vec2(-4.8, dy),
                    faint,
                );
                line(
                    center + egui::vec2(4.8, dy),
                    center + egui::vec2(7.0, dy),
                    faint,
                );
            }
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
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 33.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if active {
            // macOS sidebars use a translucent neutral selection, not a filled accent row.
            painter.rect_filled(
                rect,
                CornerRadius::same(9),
                Color32::from_rgba_unmultiplied(255, 255, 255, 20),
            );
            let accent = egui::Rect::from_min_size(
                rect.left_center() + egui::vec2(0.0, -8.0),
                egui::vec2(3.0, 16.0),
            );
            painter.rect_filled(accent, CornerRadius::same(2), ACCENT);
        } else if response.hovered() {
            painter.rect_filled(
                rect,
                CornerRadius::same(9),
                Color32::from_rgba_unmultiplied(255, 255, 255, 10),
            );
        }
        draw_nav_icon(
            painter,
            rect.left_center() + egui::vec2(20.0, 0.0),
            page,
            if active { ACCENT } else { MUTED },
        );
        painter.text(
            rect.left_center() + egui::vec2(40.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            if active { TEXT } else { TEXT_DIM },
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Generic labelled combo box for a bounded set of enum choices.
fn picker<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    current: &mut T,
    choices: &[(T, &'static str)],
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(MUTED));
        egui::ComboBox::from_id_salt(id)
            .selected_text(
                RichText::new(
                    choices
                        .iter()
                        .find(|(v, _)| *v == *current)
                        .map(|(_, s)| *s)
                        .unwrap_or("—"),
                )
                .size(11.0),
            )
            .show_ui(ui, |ui| {
                for (value, text) in choices {
                    ui.selectable_value(current, *value, *text);
                }
            });
    });
}

/// Apple-style segmented control for common test durations. The fine-grained
/// slider remains below; presets just make the usual choices one tap away.
fn duration_presets(ui: &mut egui::Ui, seconds: &mut u64) {
    ui.label(RichText::new("Quick duration").size(10.0).color(MUTED));
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (value, label) in [(15u64, "15s"), (60, "1 min"), (300, "5 min")] {
            let selected = *seconds == value;
            let fill = if selected {
                ACCENT
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 12)
            };
            let text_color = if selected {
                Color32::from_rgb(9, 18, 30)
            } else {
                TEXT_DIM
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).size(11.0).strong().color(text_color))
                        .fill(fill)
                        .corner_radius(CornerRadius::same(8)),
                )
                .clicked()
            {
                *seconds = value;
            }
        }
    });
}

fn kind_tile(ui: &mut egui::Ui, selected: bool, title: &str, hint: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 50.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        // Clip to the tile: a long hint used to spill over the card edge and
        // overlap the next tile and the description below.
        let painter = ui.painter().with_clip_rect(rect.shrink(1.0));
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
    icon: Page,
    label: &str,
    value: &str,
    footnote: &str,
    fraction: Option<f32>,
    accent: Color32,
) {
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(142.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                painter.rect_filled(rect, CornerRadius::same(8), tint(accent));
                draw_nav_icon(painter, rect.center(), icon, accent);
            }
            ui.add_space(2.0);
            ui.label(RichText::new(label).size(10.5).strong().color(MUTED));
        });
        ui.add_space(10.0);
        ui.label(RichText::new(value).size(27.0).strong().color(TEXT));
        ui.add_space(9.0);
        match fraction {
            Some(f) => bar(ui, f, accent, 6.0),
            // No meaningful ratio: keep the caption baseline without faking 0%.
            None => ui.add_space(6.0),
        }
        ui.add_space(9.0);
        ui.add(egui::Label::new(RichText::new(footnote).size(10.5).color(MUTED)).truncate());
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

fn core_meter(ui: &mut egui::Ui, index: usize, usage: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::hover());
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
    let value_width = 34.0;
    let track = egui::Rect::from_min_size(
        rect.left_center() + egui::vec2(24.0, -3.0),
        egui::vec2((width - 24.0 - value_width).max(24.0), 6.0),
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

/// Compact label/value pair used by the overview status band.
fn stat(ui: &mut egui::Ui, label: &str, value: &str, color: Color32, hint: &str) {
    ui.label(RichText::new(label).size(9.5).strong().color(MUTED));
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(value).size(15.0).strong().color(color)).truncate());
    ui.add_space(2.0);
    ui.label(RichText::new(hint).size(9.5).color(MUTED));
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

/// Thermal safety label relative to the person's advisory alert limit. Used
/// during a stress run so peak temperature can be read at a glance and the
/// decision to stop stays with the user (the app never changes hardware state).
fn thermal_safety(celsius: f32, limit_celsius: f32) -> (&'static str, Color32) {
    if celsius >= limit_celsius {
        ("Above limit", RED)
    } else if celsius >= limit_celsius - 12.0 {
        ("Warming", AMBER)
    } else {
        ("Normal", GREEN)
    }
}

/// Compact temperature sparkline from the bounded session history. Draws the
/// hottest sensor per sample; gaps (missing readings) break the line like the
/// full thermal graph does.
fn mini_temperature_graph(ui: &mut egui::Ui, history: &History) {
    let values: Vec<f64> = history
        .samples()
        .map(|sample| {
            sample
                .temperatures
                .iter()
                .filter_map(|(_, v)| *v)
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .filter(|v| v.is_finite())
        .collect();
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) || values.len() < 2 {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "temperature graph needs a sensor",
            egui::FontId::proportional(10.0),
            MUTED,
        );
        return;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1.0);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(8),
        Color32::from_rgba_unmultiplied(255, 255, 255, 8),
    );
    let points: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x =
                rect.left() + 6.0 + (rect.width() - 12.0) * i as f32 / (values.len() - 1) as f32;
            let y = rect.bottom() - 6.0 - (rect.height() - 12.0) * ((*v - min) / span) as f32;
            egui::pos2(x, y)
        })
        .collect();
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], Stroke::new(1.6_f32, ACCENT));
    }
    painter.text(
        rect.left_top() + egui::vec2(8.0, 4.0),
        egui::Align2::LEFT_TOP,
        format!("{max:.0}° peak"),
        egui::FontId::proportional(9.0),
        TEXT_DIM,
    );
}

/// Show the first few recorded mismatches in a compact table, in the style of
/// memory-diagnostic tools: location, expected value and actual value.
fn error_details(ui: &mut egui::Ui, errors: &[ErrorDetail], total: u64) {
    if errors.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!("MISMATCHES ({total} total)"))
            .size(9.5)
            .strong()
            .color(RED),
    );
    ui.add_space(3.0);
    for error in errors {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&error.location).size(10.0).color(TEXT_DIM));
            ui.label(
                RichText::new(format!("exp {} / got {}", error.expected, error.actual))
                    .size(10.0)
                    .color(MUTED),
            );
        });
    }
    if total > errors.len() as u64 {
        ui.weak(format!("+ {} more not shown", total - errors.len() as u64));
    }
}

/// Apple-style inline callout: a lightly tinted, rounded panel with a short
/// title and supporting text. Replaces faint footnotes when guidance matters
/// enough that a person should notice it before starting a test.
fn callout(ui: &mut egui::Ui, tone: Color32, title: &str, body: &str) {
    let frame = egui::Frame {
        fill: Color32::from_rgba_unmultiplied(tone.r(), tone.g(), tone.b(), 22),
        stroke: Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(tone.r(), tone.g(), tone.b(), 78),
        ),
        corner_radius: CornerRadius::same(10),
        inner_margin: egui::Margin {
            left: 16,
            right: 12,
            top: 10,
            bottom: 10,
        },
        ..Default::default()
    };
    // Draw a slim accent bar in the panel's tone; the label stays high-contrast
    // so even a muted callout reads clearly against the tinted background.
    let response = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(11.5).strong().color(TEXT));
        ui.label(RichText::new(body).size(10.5).color(TEXT_DIM));
    });
    let bar = egui::Rect::from_min_size(
        response.response.rect.left_top() + egui::vec2(4.0, 10.0),
        egui::vec2(2.5, (response.response.rect.height() - 20.0).max(8.0)),
    );
    ui.painter().rect_filled(bar, CornerRadius::same(2), tone);
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

fn driver_class_label(class: &str) -> &str {
    match class {
        "DISPLAY" => "Display",
        "NET" => "Network",
        "HDC" => "Storage controller",
        "SCSIADAPTER" => "Storage adapter",
        "USB" => "USB",
        "BLUETOOTH" => "Bluetooth",
        "BATTERY" => "Battery",
        "MONITOR" => "Monitor",
        "MEDIA" => "Audio",
        "SYSTEM" => "System device",
        "MOUSE" => "Mouse",
        "KEYBOARD" => "Keyboard",
        "FIRMWARE" => "Firmware",
        "CAMERA" => "Camera",
        "" => "Other",
        other => other,
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Cpu => "CPU compute",
        Kind::Gpu => "GPU check",
        Kind::Memory => "Memory patterns",
        Kind::Disk => "File I/O",
        Kind::Combined => "Combined CPU + memory",
    }
}

fn comparison_row(ui: &mut egui::Ui, label: &str, a: &str, b: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.5).color(MUTED));
        ui.add_space(4.0);
        ui.label(RichText::new(a).size(11.0).strong().color(TEXT));
        ui.add_space(8.0);
        ui.label(RichText::new("vs").size(9.5).color(MUTED));
        ui.add_space(8.0);
        ui.label(RichText::new(b).size(11.0).strong().color(TEXT));
    });
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
        Kind::Combined => format!("{} workers + {} MiB memory", config.workers, config.mib),
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
    fn cpu_captions_drop_vendor_noise() {
        assert_eq!(
            short_cpu_name("Intel(R) Core(TM) i7-8565U CPU @ 1.80GHz"),
            "i7-8565U"
        );
        assert_eq!(short_cpu_name("Apple M2 Pro"), "M2 Pro");
        assert_eq!(short_cpu_name("Unknown CPU"), "Unknown CPU");
    }

    #[test]
    fn long_names_are_truncated_to_the_limit() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a very long adapter name", 10).chars().count(), 10);
    }
}
