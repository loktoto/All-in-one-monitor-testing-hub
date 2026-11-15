#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use hardware_hub::{
    report::{self, Format},
    stress::{self, Config, Kind, ReportContext, State},
    telemetry::{self, Snapshot},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([760.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Hardware Hub",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
#[derive(PartialEq)]
enum Page {
    Overview,
    Tests,
    Reports,
}
struct App {
    page: Page,
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
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
    history: VecDeque<[f64; 3]>,
    last_sample: Instant,
    started: Instant,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(15, 20, 31);
        style.visuals.window_fill = Color32::from_rgb(20, 27, 40);
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        cc.egui_ctx.set_style(style);
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        telemetry::spawn(snapshot.clone(), stop.clone());
        Self {
            page: Page::Overview,
            snapshot,
            stop,
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
            history: VecDeque::new(),
            last_sample: Instant::now(),
            started: Instant::now(),
        }
    }
    fn overview(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        ui.heading("System overview");
        ui.label(
            RichText::new("Live local readings · refreshed approximately every second")
                .color(Color32::GRAY),
        );
        ui.add_space(12.0);
        if let Some(updated) = s.updated_at {
            let age = updated.elapsed().as_secs_f32();
            let label = if age > 3.0 {
                format!(
                    "STALE · collection completed {:.1}s ago · sources: sysinfo / wgpu / optional NVML",
                    age
                )
            } else {
                format!(
                    "Collection completed {:.1}s ago · sources: sysinfo / wgpu / optional NVML",
                    age
                )
            };
            ui.colored_label(
                if age > 3.0 {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GRAY
                },
                label,
            );
        }
        if !s.ready {
            ui.label("Collecting initial readings…");
            return;
        }
        ui.columns(2, |cols| {
            card(
                &mut cols[0],
                "PROCESSOR",
                &if s.cores.is_empty() {
                    "Unavailable".into()
                } else {
                    format!("{:.0}%", s.cpu_usage)
                },
                &format!("{} · {} logical cores", s.cpu_name, s.cores.len()),
            );
            card(
                &mut cols[1],
                "MEMORY",
                &if s.memory_total == 0 {
                    "Unavailable".into()
                } else {
                    format!("{:.1} / {:.1} GiB", gib(s.memory_used), gib(s.memory_total))
                },
                &if s.memory_total == 0 {
                    "sysinfo returned no memory data".into()
                } else {
                    format!("{:.1} GiB available", gib(s.memory_available))
                },
            );
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Recent activity · up to 60 samples").strong());
            if ui.button("Reset history").clicked() {
                self.history.clear();
            }
        });
        if !self.history.is_empty() {
            let (min, max) = self
                .history
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(v[1]), hi.max(v[1]))
                });
            let (mem_min, mem_max) = self
                .history
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(v[2]), hi.max(v[2]))
                });
            ui.weak(format!(
                "Shown samples · CPU {:.0}–{:.0}% · memory {:.0}–{:.0}%",
                min, max, mem_min, mem_max
            ));
        }
        let cpu: PlotPoints = self.history.iter().map(|v| [v[0], v[1]]).collect();
        let mem: PlotPoints = self.history.iter().map(|v| [v[0], v[2]]).collect();
        Plot::new("history")
            .height(150.0)
            .legend(Legend::default())
            .include_y(0.0)
            .include_y(100.0)
            .show_x(false)
            .show(ui, |p| {
                p.line(
                    Line::new(cpu)
                        .name("CPU %")
                        .color(Color32::from_rgb(90, 190, 255)),
                );
                p.line(
                    Line::new(mem)
                        .name("Memory %")
                        .color(Color32::from_rgb(110, 220, 166)),
                );
            });
        ui.label(RichText::new("Logical CPU cores").strong());
        ui.horizontal_wrapped(|ui| {
            for (i, v) in s.cores.iter().enumerate() {
                ui.label(format!("{}: {:.0}%", i, v));
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Graphics adapters").strong());
        if s.gpus.is_empty() {
            ui.label("No adapter detected, or graphics discovery is unavailable.");
        }
        for g in &s.gpus {
            ui.group(|ui| {
                ui.strong(&g.name);
                ui.label(&g.detail);
            });
        }
        ui.add_space(10.0);
        ui.label(RichText::new("System temperature sensors · sysinfo").strong());
        if s.temperatures.is_empty() {
            ui.label("No system temperature sensors available from this provider.");
        }
        for temp in &s.temperatures {
            ui.label(format!(
                "{}: {}",
                temp.label,
                temp.celsius
                    .map(|v| format!("{v:.1} °C"))
                    .unwrap_or_else(|| "unavailable".into())
            ));
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Storage volumes · not physical-drive identification").strong());
        if s.disks.is_empty() {
            ui.label("No volumes available.");
        }
        for d in &s.disks {
            ui.group(|ui| { ui.strong(format!("{}  ·  {}",d.mount,d.name)); ui.label(format!("{} · {:.1} / {:.1} GiB free · read {} / write {} bytes since last refresh (when supported)",d.kind,gib(d.available),gib(d.total),if d.read == 0 { "0 or unavailable".to_string() } else { d.read.to_string() },if d.written == 0 { "0 or unavailable".to_string() } else { d.written.to_string() })); });
        }
        ui.add_space(10.0);
        ui.weak("System sensor labels and device associations are supplied by sysinfo and may be ambiguous. SMART health, fan speed and power are unavailable. No sensor is used for automatic shutdown.");
    }
    fn tests(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        ui.heading("Manual checks");
        ui.label("Run one bounded check at a time. These are not performance benchmarks.");
        let running = self.state.lock().map(|s| s.running).unwrap_or(false);
        ui.add_enabled_ui(!running, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.kind,Kind::Cpu,"CPU compute");
                ui.selectable_value(&mut self.kind,Kind::Gpu,"GPU check");
                ui.selectable_value(&mut self.kind,Kind::Memory,"Memory patterns");
                ui.selectable_value(&mut self.kind,Kind::Disk,"File I/O");
            });
            ui.add_space(10.0);
            match self.kind {
                Kind::Cpu => {ui.label("Repeated integer calculations compared against a fixed known answer; not a full hardware certification.");ui.add(egui::Slider::new(&mut self.workers,1..=8).text("workers"));}
                Kind::Gpu => {
                    ui.label("Light compute validation with readback, not a full GPU or VRAM stress test. Software adapters cannot be tested.");
                    if s.adapters.is_empty() { ui.label("No GPU adapter available."); }
                    else {
                        self.gpu_index=self.gpu_index.min(s.adapters.len()-1);
                        egui::ComboBox::from_label("GPU adapter").selected_text(format!("{} · {}{}",s.adapters[self.gpu_index].name,s.adapters[self.gpu_index].backend,if s.adapters[self.gpu_index].software {" · software"} else {""})).show_ui(ui,|ui| {
                            for (i,a) in s.adapters.iter().enumerate() {ui.selectable_value(&mut self.gpu_index,i,format!("{i}: {} · {}{}",a.name,a.backend,if a.software {" · software"} else {""}));}
                        });
                    }
                }
                Kind::Memory => {ui.label(format!("Alternating patterns, limited to 256 MiB and one quarter of available RAM (currently {:.0} MiB).",s.memory_available as f64/4.0/1048576.0));ui.add(egui::Slider::new(&mut self.mib,1..=256).text("MiB"));}
                Kind::Disk => {ui.colored_label(Color32::from_rgb(255,190,100),"Creates and verifies a new temporary file. Writes cause SSD wear and use space; maximum 1 GiB written per run. Readback may come from cache.");ui.label("Temporary file directory:");ui.text_edit_singleline(&mut self.directory);ui.add(egui::Slider::new(&mut self.mib,1..=256).text("file size MiB"));self.disk_write_limit_mib=self.disk_write_limit_mib.max(self.mib as u64);ui.add(egui::Slider::new(&mut self.disk_write_limit_mib,self.mib as u64..=1024).text("total write limit MiB"));}
            }
            ui.add(egui::Slider::new(&mut self.seconds,1..=600).text("maximum seconds"));
            if ui.button("Start test").clicked() {
                let adapter=if self.kind==Kind::Gpu {s.adapters.get(self.gpu_index)} else {None};
                let gpu_name=adapter.map(|a|format!("{} · {}",a.name,a.backend));
                let cfg=Config{kind:self.kind,seconds:self.seconds,workers:self.workers,mib:self.mib,directory:PathBuf::from(&self.directory),gpu_adapter:gpu_name,disk_write_limit_mib:self.disk_write_limit_mib};
                let context=ReportContext{cpu:s.cpu_name.clone(),logical_cpus:s.cores.len(),memory_total:s.memory_total,adapters:s.adapters.iter().map(|a|format!("{} · {}",a.name,a.backend)).collect(),os:sysinfo::System::long_os_version().unwrap_or_else(||"unavailable".into()),snapshot_age_seconds:s.updated_at.map(|t|t.elapsed().as_secs_f64())};
                self.message=match stress::start(cfg,self.state.clone(),self.cancel.clone(),adapter.map(|a|a.handle.clone()),context) {Ok(())=>"Test started".into(),Err(e)=>e};
            }
        });
        if running {
            let elapsed = self
                .state
                .lock()
                .ok()
                .and_then(|s| s.started)
                .map(|t| t.elapsed().as_secs_f32())
                .unwrap_or_default();
            ui.label(format!(
                "Test running · {:.1} / {} seconds",
                elapsed, self.seconds
            ));
            ui.add(
                egui::ProgressBar::new((elapsed / self.seconds as f32).min(1.0)).show_percentage(),
            );
            if ui.button("Stop test").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
                self.message = "Stopping after the current work or I/O operation…".into();
            }
        }
        ui.add_space(14.0);
        ui.label(RichText::new("There is no automatic temperature-based shutdown. Monitor temperatures with a compatible tool. A blocked driver or disk operation may delay stopping.").color(Color32::LIGHT_GRAY));
        ui.colored_label(Color32::LIGHT_BLUE, &self.message);
    }
    fn reports(&mut self, ui: &mut egui::Ui) {
        ui.heading("Test reports");
        ui.label("Results stay in this session until exported. Reports include the chosen path and system details.");
        let records = self
            .state
            .lock()
            .map(|s| s.results.clone())
            .unwrap_or_default();
        if records.is_empty() {
            ui.weak("No tests run yet.");
        }
        for r in records.iter().rev() {
            ui.group(|ui| {
                ui.strong(format!("{:?} · {} · {:.1}s", r.kind, r.status, r.elapsed));
                ui.label(RichText::new(&r.detail).color(Color32::LIGHT_GRAY));
                ui.label(
                    RichText::new(format!(
                        "{} verified {} · Unix time {} · configured {}s",
                        r.verified_units
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        r.verified_unit,
                        r.timestamp,
                        r.config.seconds
                    ))
                    .color(Color32::from_rgb(175, 190, 205)),
                );
            });
        }
        ui.add_space(10.0);
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
        });
        ui.label("Export path (new file only):");
        ui.text_edit_singleline(&mut self.report);
        if ui.button("Export report").clicked() {
            self.message = match report::export(
                std::path::Path::new(&self.report),
                self.report_format,
                &records,
            ) {
                Ok(()) => format!("Saved {}", self.report),
                Err(e) => format!("Export failed: {e}"),
            };
        }
        ui.label(&self.message);
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
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
        if snapshot.ready
            && !snapshot.cores.is_empty()
            && snapshot.memory_total > 0
            && snapshot
                .updated_at
                .map(|t| t.elapsed() < Duration::from_secs(3))
                .unwrap_or(false)
            && self.last_sample.elapsed() >= Duration::from_secs(1)
        {
            self.history.push_back([
                self.started.elapsed().as_secs_f64(),
                snapshot.cpu_usage as f64,
                if snapshot.memory_total > 0 {
                    snapshot.memory_used as f64 * 100.0 / snapshot.memory_total as f64
                } else {
                    0.0
                },
            ]);
            while self.history.len() > 60 {
                self.history.pop_front();
            }
            self.last_sample = Instant::now();
        }
        egui::SidePanel::left("nav")
            .exact_width(180.0)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                ui.heading(RichText::new("HARDWARE").color(Color32::from_rgb(100, 200, 255)));
                ui.label(RichText::new("HUB  /  01").color(Color32::GRAY));
                ui.add_space(30.0);
                ui.selectable_value(&mut self.page, Page::Overview, "  Overview");
                ui.selectable_value(&mut self.page, Page::Tests, "  Stress tests");
                ui.selectable_value(&mut self.page, Page::Reports, "  Reports");
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.weak("LOCAL  ·  v0.1");
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(12.0);
                match self.page {
                    Page::Overview => self.overview(ui, &snapshot),
                    Page::Tests => self.tests(ui, &snapshot),
                    Page::Reports => self.reports(ui),
                }
            });
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
fn gib(v: u64) -> f64 {
    v as f64 / 1073741824.0
}
fn card(ui: &mut egui::Ui, title: &str, value: &str, subtitle: &str) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(25, 34, 49))
        .corner_radius(8.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_min_height(100.0);
            ui.label(
                RichText::new(title)
                    .small()
                    .color(Color32::from_rgb(120, 182, 210)),
            );
            ui.label(RichText::new(value).size(27.0).strong());
            ui.label(RichText::new(subtitle).color(Color32::LIGHT_GRAY));
        });
}
