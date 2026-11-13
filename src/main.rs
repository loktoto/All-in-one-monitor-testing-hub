#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use hardware_hub::{
    stress::{self, Config, Kind, State},
    telemetry::{self, Snapshot},
};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
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
    directory: String,
    report: String,
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
            directory: std::env::temp_dir().to_string_lossy().into_owned(),
            report: "hardware-hub-report.txt".into(),
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
        if !s.ready {
            ui.label("Collecting initial readings…");
            return;
        }
        ui.columns(2, |cols| {
            card(
                &mut cols[0],
                "PROCESSOR",
                &format!("{:.0}%", s.cpu_usage),
                &format!("{} · {} logical cores", s.cpu_name, s.cores.len()),
            );
            card(
                &mut cols[1],
                "MEMORY",
                &format!("{:.1} / {:.1} GiB", gib(s.memory_used), gib(s.memory_total)),
                &format!("{:.1} GiB available", gib(s.memory_available)),
            );
        });
        ui.add_space(10.0);
        ui.label(RichText::new("60-second activity").strong());
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
        ui.label(RichText::new("Storage volumes").strong());
        if s.disks.is_empty() {
            ui.label("No volumes available.");
        }
        for d in &s.disks {
            ui.group(|ui| { ui.strong(format!("{}  ·  {}",d.mount,d.name)); ui.label(format!("{} · {:.1} / {:.1} GiB free · read {} / write {} bytes since last refresh (when supported)",d.kind,gib(d.available),gib(d.total),if d.read == 0 { "0 or unavailable".to_string() } else { d.read.to_string() },if d.written == 0 { "0 or unavailable".to_string() } else { d.written.to_string() })); });
        }
        ui.add_space(10.0);
        ui.weak("CPU, RAM and disk temperature, SMART health, fan speed and power are unavailable in this version. A missing sensor is never shown as zero.");
    }
    fn tests(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        ui.heading("Stress tests");
        ui.label("Run one manual test at a time. Nothing starts automatically.");
        ui.add_space(12.0);
        let running = self.state.lock().map(|s| s.running).unwrap_or(false);
        ui.add_enabled_ui(!running,|ui| {
      ui.horizontal(|ui| {ui.selectable_value(&mut self.kind,Kind::Cpu,"CPU compute");ui.selectable_value(&mut self.kind,Kind::Gpu,"GPU check");ui.selectable_value(&mut self.kind,Kind::Memory,"Memory patterns");ui.selectable_value(&mut self.kind,Kind::Disk,"File I/O");});
      ui.add_space(10.0);
      match self.kind {
       Kind::Cpu=>{ui.label("Repeated integer computations checked against a reference result. This is a basic stability check, not a certification.");ui.add(egui::Slider::new(&mut self.workers,1..=8).text("workers"));}
       Kind::Gpu=>{ui.label("Light compute validation on the default high-performance GPU with readback. It is not a full GPU stress test. Software adapters are rejected.");}
       Kind::Memory=>{ui.label(format!("Write and verify alternating patterns. Limited to 256 MiB and one quarter of available RAM (currently {:.0} MiB).",s.memory_available as f64 / 4.0 / 1048576.0));ui.add(egui::Slider::new(&mut self.mib,1..=256).text("MiB"));}
       Kind::Disk=>{ui.colored_label(Color32::from_rgb(255,190,100),"Writes and verifies a new temporary file. Repeated writes cause SSD wear and consume space. Up to 1 GiB is written per run.");ui.label("Directory for temporary test file:");ui.text_edit_singleline(&mut self.directory);ui.add(egui::Slider::new(&mut self.mib,1..=256).text("file size MiB"));}
      }
      ui.add(egui::Slider::new(&mut self.seconds,1..=600).text("maximum seconds"));
      if ui.button("Start test").clicked() { let cfg=Config{kind:self.kind,seconds:self.seconds,workers:self.workers,mib:self.mib,directory:PathBuf::from(&self.directory)}; self.message=match stress::start(cfg,self.state.clone(),self.cancel.clone()){Ok(())=>"Test started".into(),Err(e)=>e}; }
    });
        if running {
            ui.add_space(10.0);
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
        ui.label(RichText::new("There is no automatic temperature-based shutdown. Monitor temperatures with a compatible tool and stop the test if necessary. Stop may wait for the current disk operation.").color(Color32::LIGHT_GRAY));
        ui.colored_label(Color32::LIGHT_BLUE, &self.message);
    }
    fn reports(&mut self, ui: &mut egui::Ui) {
        ui.heading("Test reports");
        ui.label("Results are kept in this session. Export them to keep a record.");
        ui.add_space(12.0);
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
                ui.label(&r.detail);
                ui.weak(format!("Unix time: {}", r.timestamp));
            });
        }
        ui.add_space(10.0);
        ui.label("Export path (a new file will be created):");
        ui.text_edit_singleline(&mut self.report);
        if ui.button("Export report").clicked() {
            let output = records
                .iter()
                .map(|r| {
                    format!(
                        "{:?} | {} | {:.1}s | Unix {}\n{}\n",
                        r.kind, r.status, r.elapsed, r.timestamp, r.detail
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.message = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.report)
                .and_then(|mut f| {
                    f.write_all(format!("Hardware Hub report\n\n{output}").as_bytes())
                }) {
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
                        result.status.to_lowercase()
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
        if snapshot.ready && self.last_sample.elapsed() >= Duration::from_secs(1) {
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
