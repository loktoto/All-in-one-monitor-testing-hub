#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

use crate::ui::{
    shell::{
        expanded_window_size, floating_hud, COMPACT_SIZE, MIN_WINDOW_SIZE, SECONDARY_NAV_MIN_WIDTH,
    },
    theme::*,
    widgets::*,
    Page,
};
use eframe::egui::{self, Color32, RichText};
use hardware_hub::{
    alerts::{self, Severity, Threshold},
    frames::FrameSummary,
    history::History,
    logging::Recorder,
    netcheck::State as NetworkCheckState,
    report::Format,
    settings::{OverviewOpen, Settings},
    smart::SmartReport,
    specs::{self, SystemSpecs},
    stress::{
        CpuWorkload, DiskPattern, GpuIntensity, Kind, MemPattern, RunMode, State,
        MAX_DISK_WRITE_CAP_MIB, MAX_TIMED_SECONDS, MAX_WORKERS,
    },
    telemetry::{self, SamplingControl, Snapshot},
};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

fn main() -> eframe::Result {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == hardware_hub::stress::GPU_WORKER_FLAG)
    {
        return hardware_hub::stress::run_gpu_worker()
            .map_err(|error| eframe::Error::AppCreation(std::io::Error::other(error).into()));
    }
    let settings = Settings::load();
    let compact = env_bool("HH_COMPACT").unwrap_or(settings.compact_mode);
    let initial_size = if compact {
        COMPACT_SIZE
    } else {
        expanded_window_size([settings.window_width, settings.window_height], None)
    };
    // Never force maximized/fullscreen: launch as a sleek, resizable desktop
    // window that fits beside other apps (and shrinks cleanly to ~420x680 in
    // Compact HWMonitor Mode).
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Hardware Hub")
            .with_inner_size(initial_size)
            .with_min_inner_size(MIN_WINDOW_SIZE)
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Hardware Hub",
        opts,
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings, compact)))),
    )
}

fn env_bool(name: &str) -> Option<bool> {
    std::env::var(name)
        .ok()
        .and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
}

/// Generate a 64x64 RGBA window/taskbar icon at startup: an Obsidian Graphite
/// rounded tile with three telemetry bars, requiring zero external asset files.
fn app_icon() -> egui::IconData {
    const N: u32 = 64;
    let mut rgba = vec![0u8; (N * N * 4) as usize];
    let radius = 14.0_f32;
    let edge = N as f32 - radius;
    for y in 0..N {
        for x in 0..N {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let cx = fx.clamp(radius, edge);
            let cy = fy.clamp(radius, edge);
            let dx = fx - cx;
            let dy = fy - cy;
            if dx * dx + dy * dy <= radius * radius {
                let idx = ((y * N + x) * 4) as usize;
                rgba[idx] = 24;
                rgba[idx + 1] = 28;
                rgba[idx + 2] = 36;
                rgba[idx + 3] = 255;
            }
        }
    }
    let mut put = |x0: u32, y0: u32, w: u32, h: u32, color: (u8, u8, u8)| {
        for y in y0..(y0 + h).min(N) {
            for x in x0..(x0 + w).min(N) {
                let idx = ((y * N + x) * 4) as usize;
                rgba[idx] = color.0;
                rgba[idx + 1] = color.1;
                rgba[idx + 2] = color.2;
                rgba[idx + 3] = 255;
            }
        }
    };
    let bottom = 50;
    let bar_w = 9;
    for (i, (height, color)) in [
        (18u32, (48u8, 209u8, 88u8)),
        (30, (34, 211, 238)),
        (22, (47, 129, 247)),
    ]
    .iter()
    .enumerate()
    {
        let x = 14 + i as u32 * 12;
        put(x, bottom - height, bar_w, *height, *color);
    }
    egui::IconData {
        rgba,
        width: N,
        height: N,
    }
}

#[derive(Default)]
pub(crate) struct BluetoothRefresh {
    pub running: bool,
    pub attempted: bool,
    pub value: Option<specs::BluetoothSpec>,
}

fn initial_page(settings: &Settings) -> Page {
    if let Ok(value) = std::env::var("HH_START_PAGE") {
        if let Some(page) = Page::from_label(&value) {
            return page;
        }
    }
    Page::from_label(&settings.page).unwrap_or(Page::Overview)
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

fn parse_run_mode(raw: &str) -> RunMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "continuous" => RunMode::Continuous,
        "passes" => RunMode::Passes,
        _ => RunMode::Timed,
    }
}

fn run_mode_key(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Timed => "timed",
        RunMode::Continuous => "continuous",
        RunMode::Passes => "passes",
    }
}

pub(crate) struct App {
    pub page: Page,
    pub compact_mode: bool,
    pub expanded_size: [f32; 2],
    pub sidebar_open: bool,
    pub pro_density: bool,
    pub show_palette: bool,
    pub palette_focus: bool,
    pub palette_query: String,
    pub palette_index: usize,
    pub sensor_filter: String,
    pub core_sensor_filter: String,
    pub focus_temperature_chart: bool,
    pub pinned_sensors: Vec<String>,
    pub dark_titlebar_ticks: u8,

    pub snapshot: Arc<Mutex<Arc<Snapshot>>>,
    pub specs: Arc<Mutex<Option<Arc<SystemSpecs>>>>,
    pub process_wanted: Arc<AtomicBool>,
    pub overview: OverviewOpen,
    pub specs_refreshed: Arc<AtomicBool>,
    pub poll_ms: Arc<AtomicU64>,
    pub sampling_control: Arc<SamplingControl>,
    pub libre_monitor_enabled: bool,
    pub libre_monitor_port: u16,
    pub stop: Arc<AtomicBool>,
    pub stats_reset: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
    pub state: Arc<Mutex<State>>,

    pub run_peak_c: Option<f32>,
    pub last_live_units: u64,
    pub last_live_at: Instant,
    pub rate_ema: f64,
    pub shown_progress: f32,
    pub last_page: Page,
    pub page_enter_at: Instant,

    pub kind: Kind,
    pub stress_mode: RunMode,
    pub seconds: u64,
    pub stress_passes: u64,
    pub workers: usize,
    pub cpu_workload: CpuWorkload,
    pub mib: usize,
    pub disk_write_limit_mib: u64,
    pub directory: String,
    pub mem_pattern: MemPattern,
    pub disk_pattern: DiskPattern,
    pub gpu_intensity: GpuIntensity,
    pub auto_stop_on_heat: bool,
    pub thermal_stop: alerts::ThermalStop,
    pub run_thermal_rule: Option<Threshold>,

    pub report: String,
    pub report_format: Format,
    pub certificate_path: String,
    pub compare_a: usize,
    pub compare_b: usize,
    pub gpu_index: usize,
    pub gpu_initialized: bool,
    pub message: String,

    pub history: History,
    pub history_path: String,
    pub log_path: String,
    pub all_sensor_log_path: String,
    pub recorder: Option<Recorder>,

    pub process_query: String,
    pub process_sort_memory: bool,
    pub driver_query: String,
    pub driver_class: String,
    pub bluetooth_refresh: Arc<Mutex<BluetoothRefresh>>,
    pub network_target: String,
    pub network_message: String,
    pub network_check: Arc<Mutex<NetworkCheckState>>,
    pub network_cancel: Arc<AtomicBool>,
    pub smart_path: String,
    pub smart_report: Option<SmartReport>,
    pub smart_message: String,
    pub frame_path: String,
    pub frame_summary: Option<FrameSummary>,
    pub frame_message: String,

    pub alert: alerts::Monitor,
    pub alert_celsius: f32,
    pub alert_hold_secs: u64,
    pub alert_severity: Severity,
    pub selected_temp_key: String,
    pub sensor_log_path: String,

    pub system_report_path: String,
    pub system_report_format: Format,
    pub system_report_redact: bool,

    pub reduce_motion: bool,
    pub show_widget: bool,
    pub show_help: bool,
    pub started: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, settings: Settings, compact_mode: bool) -> Self {
        install_theme(&cc.egui_ctx);
        set_reduce_motion(&cc.egui_ctx, settings.reduce_motion);
        let app = Self::from_settings(settings, compact_mode);
        let telemetry_ctx = cc.egui_ctx.clone();
        telemetry::spawn_with_control(
            Arc::clone(&app.snapshot),
            Arc::clone(&app.stop),
            Arc::clone(&app.stats_reset),
            Arc::clone(&app.poll_ms),
            Arc::clone(&app.process_wanted),
            Arc::clone(&app.sampling_control),
            // A short delay keeps the wake within a frame and avoids egui's
            // extra settling repaint for immediate requests.
            move || {
                telemetry_ctx
                    .request_repaint_after_for(Duration::from_millis(8), egui::ViewportId::ROOT);
            },
        );

        let cache_path = Settings::inventory_path();
        let cached = cache_path
            .as_deref()
            .and_then(specs::load_cache)
            .filter(|(specs, _)| !specs.cpu.name.is_empty() || !specs.model_name.is_empty());
        if let Ok(mut slot) = app.specs.lock() {
            *slot = cached.map(|(specs, _)| Arc::new(specs));
        }
        let specs_slot = Arc::clone(&app.specs);
        let refreshed_flag = Arc::clone(&app.specs_refreshed);
        let inventory_ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            let collected = specs::collect();
            if !collected.cpu.name.is_empty() || !collected.model_name.is_empty() {
                if let Some(path) = cache_path.as_deref() {
                    let _ = specs::save_cache(path, &collected);
                }
            }
            if let Ok(mut slot) = specs_slot.lock() {
                *slot = Some(Arc::new(collected));
            }
            refreshed_flag.store(true, Ordering::Relaxed);
            inventory_ctx.request_repaint_of(egui::ViewportId::ROOT);
        });

        app
    }

    fn from_settings(settings: Settings, compact_mode: bool) -> Self {
        let start_page = initial_page(&settings);
        Self {
            page: start_page,
            compact_mode,
            expanded_size: expanded_window_size(
                [settings.window_width, settings.window_height],
                None,
            ),
            sidebar_open: true,
            pro_density: settings.pro_density,
            show_palette: false,
            palette_focus: false,
            palette_query: String::new(),
            palette_index: 0,
            sensor_filter: String::new(),
            core_sensor_filter: String::new(),
            focus_temperature_chart: false,
            pinned_sensors: settings.pinned_sensors.clone(),
            dark_titlebar_ticks: 0,

            snapshot: Arc::new(Mutex::new(Arc::new(Snapshot::default()))),
            overview: settings.overview.clone(),
            process_wanted: Arc::new(AtomicBool::new(false)),
            specs: Arc::new(Mutex::new(None)),
            specs_refreshed: Arc::new(AtomicBool::new(false)),
            poll_ms: Arc::new(AtomicU64::new(settings.poll_ms.clamp(250, 5000))),
            sampling_control: Arc::new(SamplingControl::default()),
            libre_monitor_enabled: settings.libre_monitor_enabled,
            libre_monitor_port: if settings.libre_monitor_port == 0 {
                8085
            } else {
                settings.libre_monitor_port
            },
            stop: Arc::new(AtomicBool::new(false)),
            stats_reset: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(State::default())),
            run_peak_c: None,
            last_live_units: 0,
            last_live_at: Instant::now(),
            rate_ema: 0.0,
            shown_progress: 0.0,
            last_page: start_page,
            page_enter_at: Instant::now(),

            kind: Kind::Cpu,
            stress_mode: parse_run_mode(&settings.stress_mode),
            seconds: settings.seconds.clamp(1, MAX_TIMED_SECONDS),
            stress_passes: settings.stress_passes.clamp(1, 10_000),
            workers: settings.workers.clamp(1, MAX_WORKERS),
            cpu_workload: CpuWorkload::Blend,
            mib: settings.mib.clamp(1, 16_384),
            disk_write_limit_mib: settings
                .disk_write_limit_mib
                .clamp(1, MAX_DISK_WRITE_CAP_MIB),
            directory: if settings.directory.trim().is_empty() {
                std::env::temp_dir().to_string_lossy().into_owned()
            } else {
                settings.directory.clone()
            },
            mem_pattern: MemPattern::Alternating,
            disk_pattern: DiskPattern::Constant,
            gpu_intensity: GpuIntensity::Standard,
            auto_stop_on_heat: settings.auto_stop_on_heat,
            thermal_stop: alerts::ThermalStop::default(),
            run_thermal_rule: None,

            report: non_empty(&settings.report, "hardware-hub-report.txt"),
            report_format: Format::Text,
            certificate_path: "hardware-hub-run-summary.txt".into(),
            compare_a: 0,
            compare_b: 0,
            gpu_index: 0,
            gpu_initialized: false,
            message: String::new(),
            history: History::default(),
            history_path: non_empty(&settings.history_path, "hardware-hub-history.csv"),
            log_path: non_empty(&settings.log_path, "hardware-hub-session.csv"),
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
            alert_celsius: if settings.alert_celsius.is_finite() {
                settings.alert_celsius.clamp(30.0, 125.0)
            } else {
                85.0
            },
            alert_hold_secs: settings.alert_hold_secs.clamp(1, 120),
            alert_severity: Severity::Warning,
            selected_temp_key: String::new(),
            sensor_log_path: non_empty(&settings.sensor_log_path, "hardware-hub-sensor.csv"),
            system_report_path: "hardware-hub-system.txt".into(),
            system_report_format: Format::Text,
            system_report_redact: true,
            reduce_motion: settings.reduce_motion,
            show_widget: env_bool("HH_SHOW_HUD").unwrap_or(false),
            show_help: false,
            started: Instant::now(),
        }
    }

    pub(crate) fn history_gap(&self) -> Duration {
        Duration::from_millis(
            self.poll_ms
                .load(Ordering::Relaxed)
                .saturating_mul(2)
                .max(500),
        )
    }

    pub(crate) fn stale_secs(&self) -> f32 {
        self.poll_ms
            .load(Ordering::Relaxed)
            .saturating_mul(2)
            .max(1000) as f32
            / 1000.0
    }

    fn persist_settings(&mut self) {
        let settings = Settings {
            page: self.page.label().to_owned(),
            compact_mode: self.compact_mode,
            window_width: self.expanded_size[0],
            window_height: self.expanded_size[1],
            pro_density: self.pro_density,
            reduce_motion: self.reduce_motion,
            poll_ms: self.poll_ms.load(Ordering::Relaxed),
            libre_monitor_enabled: self.libre_monitor_enabled,
            libre_monitor_port: self.libre_monitor_port,
            workers: self.workers,
            seconds: self.seconds,
            stress_mode: run_mode_key(self.stress_mode).to_owned(),
            stress_passes: self.stress_passes,
            mib: self.mib,
            disk_write_limit_mib: self.disk_write_limit_mib,
            directory: self.directory.clone(),
            alert_celsius: self.alert_celsius,
            alert_hold_secs: self.alert_hold_secs,
            auto_stop_on_heat: self.auto_stop_on_heat,
            pinned_sensors: self.pinned_sensors.clone(),
            history_path: self.history_path.clone(),
            log_path: self.log_path.clone(),
            sensor_log_path: self.sensor_log_path.clone(),
            report: self.report.clone(),
            overview: self.overview.clone(),
        };
        if let Err(error) = settings.save() {
            self.message = format!("Could not save settings: {error}");
        }
    }
    fn render(&mut self, ctx: &egui::Context) {
        if self.dark_titlebar_ticks == 0 {
            let (window, monitor) =
                ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
            if let (Some(window), Some(monitor)) = (window, monitor) {
                let current: [f32; 2] = window.size().into();
                let size = expanded_window_size(current, Some(monitor));
                if size != current {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
                }
            }
        }
        // Apply native dark titlebar on Windows once the HWND is mapped.
        if self.dark_titlebar_ticks < 3 {
            apply_dark_titlebar();
            self.dark_titlebar_ticks += 1;
            if self.dark_titlebar_ticks < 3 {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }

        // Remember non-compact window geometry while resized.
        if !self.compact_mode {
            if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                if rect.width() >= SECONDARY_NAV_MIN_WIDTH
                    && rect.height() >= MIN_WINDOW_SIZE[1]
                    && !ctx.input(|i| {
                        i.viewport().maximized.unwrap_or(false)
                            || i.viewport().fullscreen.unwrap_or(false)
                    })
                {
                    self.expanded_size = [rect.width(), rect.height()];
                }
            }
        }

        if self.last_page != self.page {
            self.last_page = self.page;
            self.page_enter_at = Instant::now();
            ctx.request_repaint();
        }

        let (running, run_started) = self
            .state
            .lock()
            .map(|s| (s.running, s.started))
            .unwrap_or_default();
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

        // Graceful close guard when recording or running a stress check.
        if ctx.input(|i| i.viewport().close_requested()) {
            let mut notices: Vec<&str> = Vec::new();
            if let Some(recorder) = &mut self.recorder {
                if !recorder.status().closed {
                    recorder.stop();
                    notices.push("Finishing the recording");
                }
            }
            if running {
                self.cancel.store(true, Ordering::Relaxed);
                self.page = Page::Tests;
                notices.push("Stopping the test");
            }
            if !notices.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.message = format!(
                    "{}. Close again when it has stopped.",
                    notices.join(" and ")
                );
            }
        }

        let snapshot = self
            .snapshot
            .lock()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_default();
        let now = Instant::now();
        let fresh_for = Duration::from_secs_f32(self.stale_secs());
        let fresh_until = snapshot
            .updated_at
            .map(|at| at + fresh_for)
            .filter(|deadline| *deadline > now);
        let hottest = fresh_until
            .is_some()
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

        if running
            && fresh_until.is_some()
            && snapshot
                .updated_at
                .zip(run_started)
                .is_some_and(|(sample_at, started)| sample_at >= started)
        {
            if let Some(value) = hottest {
                self.run_peak_c = Some(self.run_peak_c.map_or(value, |peak| peak.max(value)));
            }
            if let Some((sample_at, rule)) = snapshot.updated_at.zip(self.run_thermal_rule) {
                if !self.cancel.load(Ordering::Relaxed)
                    && self
                        .thermal_stop
                        .observe(sample_at, hottest, rule, fresh_for)
                {
                    let reason = format!(
                        "Thermal stop: readings stayed at or above {:.0} °C for {} s.",
                        rule.celsius,
                        rule.hold.as_secs()
                    );
                    if let Ok(mut state) = self.state.lock() {
                        state.request_thermal_stop(&self.cancel, reason.clone());
                    }
                    self.message = reason;
                }
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
        if let Some(stale_at) = fresh_until {
            // Wake once to show STALE if sampling stalls, or earlier if the
            // current temperature streak is due to change the alert.
            let deadline = self
                .alert
                .next_transition(rule)
                .map_or(stale_at, |alert_at| stale_at.min(alert_at));
            ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }

        // Global shortcuts: Ctrl/Cmd+K quick jump, Ctrl/Cmd+1..9 & Shift+1..6, ?/F1, Esc.
        if ctx.input(|i| (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(egui::Key::K)) {
            if self.show_palette {
                self.show_palette = false;
            } else {
                self.open_palette();
            }
        }
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
                9 => Page::Motherboard,
                10 => Page::Drivers,
                11 => Page::Network,
                12 => Page::Bluetooth,
                13 => Page::Frames,
                14 => Page::Tests,
                _ => self.page,
            };
        }
        if !self.show_palette
            && ctx.input(|i| i.key_pressed(egui::Key::Questionmark) || i.key_pressed(egui::Key::F1))
        {
            self.show_help = !self.show_help;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_palette {
                self.show_palette = false;
            } else if self.show_help {
                self.show_help = false;
            } else if running && !self.cancel.load(Ordering::Relaxed) {
                self.cancel.store(true, Ordering::Relaxed);
                self.message = "Stopping after the current work or I/O operation…".into();
            }
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

        // Chrome: Top Toolbar -> Status Bar -> Dual-Rail Navigation -> Central Canvas
        self.top_toolbar(ctx, &snapshot);
        self.status_bar(ctx, &snapshot);
        self.navigation(ctx, &snapshot);

        let narrow = self.compact_mode || ctx.screen_rect().width() < SECONDARY_NAV_MIN_WIDTH;
        let pad_x = if narrow { 12 } else { 20 };
        let pad_top = if narrow { 12 } else { 16 };
        let frame = egui::Frame {
            fill: BG,
            inner_margin: egui::Margin {
                left: pad_x,
                right: pad_x,
                top: pad_top,
                bottom: 0,
            },
            ..Default::default()
        };

        let enter = if motion_enabled(ctx) {
            (self.page_enter_at.elapsed().as_secs_f32() / MOTION_SLOW).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if enter < 1.0 {
            ctx.request_repaint();
        }
        let eased = 1.0 - (1.0 - enter).powi(3);
        let slide = (1.0 - eased) * 8.0;
        let fade = |c: Color32| {
            let a = (c.a() as f32 * (0.35 + 0.65 * eased))
                .round()
                .clamp(0.0, 255.0) as u8;
            Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), a)
        };

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            // Compact header when secondary tree sidebar is hidden
            if narrow || !self.sidebar_open {
                self.compact_subtabs(ui);
            }
            ui.horizontal(|ui| {
                ui.add_space(slide);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(self.page.group())
                            .size(TYPE_OVERLINE)
                            .strong()
                            .color(fade(ACCENT)),
                    );
                    ui.label(
                        RichText::new(self.page.title())
                            .size(if narrow { 18.0 } else { TYPE_TITLE })
                            .strong()
                            .color(fade(TEXT)),
                    );
                    if !narrow {
                        ui.label(
                            RichText::new(self.page.subtitle())
                                .size(TYPE_SUBTITLE)
                                .color(fade(MUTED)),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if matches!(
                        self.page,
                        Page::Details | Page::Motherboard | Page::Bluetooth
                    ) {
                        let ready = self.specs.lock().map(|s| s.is_some()).unwrap_or(false);
                        let refreshed = self.specs_refreshed.load(Ordering::Relaxed);
                        let label = if !ready {
                            "WAITING"
                        } else if !refreshed {
                            "CACHED"
                        } else if self.page == Page::Bluetooth {
                            "SNAPSHOT"
                        } else {
                            "AT START"
                        };
                        pill(ui, label, if label == "CACHED" { AMBER } else { MUTED });
                    }
                });
            });
            ui.add_space(if narrow { 8.0 } else { 12.0 });
            page_rule(ui);
            ui.add_space(if narrow { 8.0 } else { 12.0 });

            if self.page == Page::Tests {
                self.stress_action(ui, &snapshot, running, narrow);
                ui.add_space(8.0);
            }

            let page_scroll = egui::ScrollArea::vertical()
                .id_salt(self.page.label())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(2.0);
                    match self.page {
                        Page::Overview => self.view_overview(ui, &snapshot),
                        Page::Thermal => self.view_thermal(ui, &snapshot),
                        Page::Cpu
                        | Page::Gpu
                        | Page::Memory
                        | Page::Storage
                        | Page::Motherboard
                        | Page::Drivers
                        | Page::Network
                        | Page::Bluetooth => self.view_hardware_page(ui, &snapshot),
                        Page::Details => self.view_details(ui, &snapshot),
                        Page::Processes => self.view_processes(ui, &snapshot),
                        Page::Frames => self.view_frames(ui),
                        Page::Tests => self.view_stress_lab(ui, &snapshot, None),
                        Page::Reports => self.view_reports(ui, &snapshot),
                    }
                    ui.add_space(22.0);
                });
            scroll_fade(ui, page_scroll.inner_rect, BG);
        });

        // Translucent Floating Monitor HUD (separate always-on-top viewport)
        if self.show_widget {
            let monitor_stale = Duration::from_secs_f32(self.stale_secs());
            let hud_height = (118.0 + self.pinned_sensors.len() as f32 * 22.0).clamp(118.0, 260.0);
            let mut keep_open = true;
            let pinned = self.pinned_sensors.clone();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("floating-monitor"),
                egui::ViewportBuilder::default()
                    .with_title("Hardware Hub · HUD")
                    .with_inner_size([276.0, hud_height])
                    .with_min_inner_size([240.0, 110.0])
                    .with_always_on_top(),
                |ctx, class| {
                    let hud_frame = egui::Frame::NONE
                        .fill(PANEL)
                        .inner_margin(egui::Margin::same(12));
                    if class == egui::ViewportClass::Embedded {
                        egui::Window::new("Hardware Hub · HUD")
                            .open(&mut keep_open)
                            .frame(hud_frame)
                            .show(ctx, |ui| {
                                floating_hud(ui, &snapshot, &pinned, monitor_stale)
                            });
                    } else {
                        apply_dark_titlebar();
                        egui::CentralPanel::default()
                            .frame(hud_frame)
                            .show(ctx, |ui| {
                                floating_hud(ui, &snapshot, &pinned, monitor_stale)
                            });
                        if ctx.input(|input| input.viewport().close_requested()) {
                            keep_open = false;
                        }
                    }
                },
            );
            self.show_widget = keep_open;
        }

        self.quick_jump_palette(ctx, &snapshot);
        self.help_overlay(ctx);

        // Switch collection on the frame that handles navigation.
        self.process_wanted.store(
            self.page == Page::Processes
                || (self.show_palette && !self.palette_query.trim().is_empty()),
            Ordering::Relaxed,
        );
        if self.last_page != self.page {
            ctx.request_repaint();
        }

        // Stress progress and the CSV writer can change between telemetry
        // batches. The idle dashboard wakes on input, animation or new data.
        let recording = self
            .recorder
            .as_ref()
            .is_some_and(|recorder| !recorder.status().closed);
        let minimized = ctx.input(|input| input.viewport().minimized.unwrap_or(false));
        let paused = minimized && !running && !recording && !self.show_widget;
        let port = if self.libre_monitor_enabled {
            self.libre_monitor_port
        } else {
            0
        };
        let previous_pause = self.sampling_control.paused.swap(paused, Ordering::Relaxed);
        let previous_port = self.sampling_control.lhm_port.swap(port, Ordering::Relaxed);
        if paused != previous_pause || port != previous_port {
            self.sampling_control.wake();
        }
        if running || recording {
            let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
            ctx.request_repaint_after(if focused {
                Duration::from_millis(250)
            } else {
                Duration::from_secs(1)
            });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.render(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop.store(true, Ordering::Relaxed);
        self.sampling_control.wake();
        self.persist_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Workspace;

    #[test]
    fn every_page_renders_at_compact_and_expanded_widths() {
        use hardware_hub::telemetry::{Process, Temperature};

        let snapshot = Arc::new(Snapshot {
            updated_at: Some(Instant::now()),
            ready: true,
            cpu_name: "128-thread fixture CPU".into(),
            cpu_usage: 42.0,
            cores: (0..128).map(|n| (n % 101) as f32).collect(),
            core_mhz: vec![3_200; 128],
            core_celsius: vec![Some(54.0); 128],
            memory_total: 32 << 30,
            memory_used: 8 << 30,
            memory_available: 24 << 30,
            processes: (0..500)
                .map(|pid| Process {
                    name: format!("process-{pid}"),
                    pid,
                    cpu_usage: 0.5,
                    memory: 32 << 20,
                })
                .collect(),
            temperatures: (0..100)
                .map(|n| Temperature {
                    key: format!("TEMP{n}"),
                    label: format!("Test temperature {n}"),
                    celsius: Some(37.0),
                    min: Some(32.0),
                    max: Some(48.0),
                    average: Some(42.0),
                    cpu_core: n < 64,
                    source: "fixture",
                })
                .collect(),
            ..Snapshot::default()
        });
        let inventory = Arc::new(SystemSpecs {
            drivers: (0..350)
                .map(|n| specs::DriverSpec {
                    name: format!("Driver {n}"),
                    class_name: "Display".into(),
                    ..Default::default()
                })
                .collect(),
            memory: specs::MemoryProfile {
                modules: (0..16)
                    .map(|n| specs::MemoryModule {
                        slot: format!("DIMM{n}"),
                        kind: "DDR5".into(),
                        size_bytes: Some(2 << 30),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
            ..SystemSpecs::default()
        });
        for (width, height, compact) in [
            (400.0, 400.0, true),
            (853.0, 416.0, false),
            (1120.0, 720.0, false),
        ] {
            let ctx = egui::Context::default();
            install_theme(&ctx);
            set_reduce_motion(&ctx, true);
            let mut app = App::from_settings(Settings::default(), compact);
            app.show_widget = false;
            app.dark_titlebar_ticks = 3;
            *app.snapshot.lock().unwrap() = Arc::clone(&snapshot);
            *app.specs.lock().unwrap() = Some(Arc::clone(&inventory));
            for page in Page::ALL {
                app.page = page;
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                };
                let output = ctx.run(input, |ctx| app.render(ctx));
                assert!(
                    !output.shapes.is_empty(),
                    "{page:?} did not render at {width}×{height}"
                );
                assert!(
                    output
                        .shapes
                        .iter()
                        .all(|clipped| !has_duplicate_widget(&clipped.shape)),
                    "{page:?} reused a widget ID at {width}×{height}"
                );
            }
        }
    }

    fn has_duplicate_widget(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(has_duplicate_widget),
            egui::Shape::Text(text) => text.galley.job.text.contains("use of widget ID"),
            _ => false,
        }
    }

    #[test]
    fn quick_jump_opens_a_temperature_sensor_with_the_keyboard() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut app = App::from_settings(Settings::default(), true);
        app.dark_titlebar_ticks = 3;
        app.show_widget = false;
        *app.snapshot.lock().unwrap() = Arc::new(Snapshot {
            ready: true,
            updated_at: Some(Instant::now()),
            temperatures: vec![telemetry::Temperature {
                key: "TZ01".into(),
                label: "ACPI zone TZ01".into(),
                source: "ACPI",
                celsius: Some(42.0),
                min: None,
                max: None,
                ..telemetry::Temperature::default()
            }],
            ..Snapshot::default()
        });
        app.open_palette();
        for events in [
            vec![],
            vec![egui::Event::Text("tz01".into())],
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 520.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.render(ctx),
            );
        }
        assert_eq!(app.page, Page::Thermal);
        assert_eq!(app.selected_temp_key, "TZ01");
        assert!(!app.show_palette);
    }

    #[test]
    fn minimized_window_keeps_sampling_for_an_active_check() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut app = App::from_settings(Settings::default(), true);
        app.dark_titlebar_ticks = 3;
        app.show_widget = false;
        for (minimized, running, expected_pause) in [
            (false, false, false),
            (true, false, true),
            (true, true, false),
            (false, false, false),
        ] {
            app.state.lock().unwrap().running = running;
            let mut input = egui::RawInput::default();
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .minimized = Some(minimized);
            let _ = ctx.run(input, |ctx| app.render(ctx));
            assert_eq!(
                app.sampling_control.paused.load(Ordering::Relaxed),
                expected_pause
            );
        }
    }

    #[test]
    fn every_page_label_roundtrips_through_from_label() {
        for page in Page::ALL {
            assert_eq!(
                Page::from_label(page.label()),
                Some(page),
                "label {:?} did not round-trip",
                page.label()
            );
            assert_eq!(
                Page::from_label(page.nav_label()),
                Some(page),
                "nav_label {:?} did not round-trip",
                page.nav_label()
            );
        }
        assert_eq!(Page::from_label("nonsense"), None);
    }

    #[test]
    fn workspaces_cover_every_page_without_duplication() {
        let mut seen = Vec::new();
        for ws in Workspace::ALL {
            for &p in ws.pages() {
                assert_eq!(p.workspace(), ws);
                assert!(!seen.contains(&p), "duplicate page {p:?} across workspaces");
                seen.push(p);
            }
        }
        assert_eq!(seen.len(), Page::ALL.len());
    }

    #[test]
    fn run_mode_settings_roundtrip() {
        for mode in [RunMode::Timed, RunMode::Continuous, RunMode::Passes] {
            assert_eq!(parse_run_mode(run_mode_key(mode)), mode);
        }
        assert_eq!(parse_run_mode("unknown"), RunMode::Timed);
    }

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
