//! Application shell: Compact HWMonitor vs Expanded Workspace window modes,
//! Slack-inspired dual-rail navigation (52px Icon Rail + collapsible 192px
//! channel tree), top toolbar (`Ctrl+K` quick jump, `Simple | Pro` density,
//! `LIVE` status), status bar, `Ctrl+K` Command Palette, Help Modal, and
//! sleek Floating Monitor HUD.

use crate::ui::{kind_label, theme::*, widgets::*, Page, Workspace, STRESS_KINDS};
use crate::App;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Vec2};
use hardware_hub::{stress::Kind, telemetry::Snapshot};
use std::{sync::atomic::Ordering, time::Duration};

pub const COMPACT_SIZE: [f32; 2] = [450.0, 680.0];
pub const MIN_WINDOW_SIZE: [f32; 2] = [400.0, 400.0];
pub const SECONDARY_NAV_MIN_WIDTH: f32 = 820.0;
const JUMP_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘K"
} else {
    "Ctrl+K"
};

pub fn expanded_window_size(saved: [f32; 2], monitor: Option<Vec2>) -> [f32; 2] {
    let [min_width, min_height] = MIN_WINDOW_SIZE;
    let size = [
        saved[0].clamp(min_width, 1800.0),
        saved[1].clamp(min_height, 1200.0),
    ];
    if let Some(monitor) = monitor {
        [
            size[0].min((monitor.x - 32.0).max(min_width)),
            size[1].min((monitor.y - 64.0).max(min_height)),
        ]
    } else {
        size
    }
}

fn resize_window(ctx: &egui::Context, size: [f32; 2]) {
    let (inner, outer) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().outer_rect));
    if let (Some(inner), Some(outer)) = (inner, outer) {
        let decorations = (outer.size() - inner.size()).max(Vec2::ZERO);
        let new_outer_size = Vec2::from(size) + decorations;
        let mut position = outer.center() - new_outer_size * 0.5;
        if let Some(work_area) = monitor_work_area(ctx.pixels_per_point()) {
            let limit = (work_area.max - new_outer_size).max(work_area.min);
            position = position.clamp(work_area.min, limit);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
}

#[cfg(target_os = "windows")]
fn monitor_work_area(pixels_per_point: f32) -> Option<Rect> {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct NativeRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct MonitorInfo {
        size: u32,
        monitor: NativeRect,
        work: NativeRect,
        flags: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetActiveWindow() -> *mut c_void;
        fn MonitorFromWindow(window: *mut c_void, flags: u32) -> *mut c_void;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
    }

    let mut info = MonitorInfo {
        size: std::mem::size_of::<MonitorInfo>() as u32,
        monitor: NativeRect::default(),
        work: NativeRect::default(),
        flags: 0,
    };
    // The mode button runs on the window thread. If there is no active
    // window, keep the ordinary viewport-centred placement.
    unsafe {
        let window = GetActiveWindow();
        if window.is_null() {
            return None;
        }
        let monitor = MonitorFromWindow(window, 2); // MONITOR_DEFAULTTONEAREST
        if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
    }
    let point = |x, y| Pos2::new(x as f32 / pixels_per_point, y as f32 / pixels_per_point);
    Some(Rect::from_min_max(
        point(info.work.left, info.work.top),
        point(info.work.right, info.work.bottom),
    ))
}

#[cfg(not(target_os = "windows"))]
fn monitor_work_area(_: f32) -> Option<Rect> {
    None
}

enum JumpTarget<'a> {
    Page(Page),
    Stress(Kind),
    Temperature(&'a str),
    Process(&'a str),
}

fn search_rank(text: &str, query: &str) -> Option<u8> {
    let text = text.to_ascii_lowercase();
    if text == query {
        Some(0)
    } else if text.starts_with(query) {
        Some(1)
    } else if text.contains(query) {
        Some(2)
    } else if query.len() >= 3 && !query.contains(char::is_whitespace) {
        let mut chars = text.chars();
        query
            .chars()
            .all(|needle| chars.any(|ch| ch == needle))
            .then_some(3)
    } else {
        None
    }
}

impl App {
    pub fn open_palette(&mut self) {
        self.show_palette = true;
        self.palette_focus = true;
        self.palette_query.clear();
        self.palette_index = 0;
    }

    /// Top sleek workspace toolbar: Compact/Expanded toggle, `Ctrl+K` quick jump,
    /// `Simple | Pro` density switch, Floating HUD toggle, and `LIVE` badge.
    pub fn top_toolbar(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let frame = egui::Frame {
            fill: PANEL,
            stroke: Stroke::NONE,
            inner_margin: egui::Margin {
                left: 12,
                right: 12,
                top: 7,
                bottom: 7,
            },
            ..Default::default()
        };
        egui::TopBottomPanel::top("top-toolbar")
            .frame(frame)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter()
                    .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0_f32, BORDER));
                let narrow = self.compact_mode || ui.available_width() < 640.0;
                ui.horizontal(|ui| {
                    // App brand + dynamic version from Cargo.toml
                    if narrow {
                        ui.label(
                            RichText::new("Hardware Hub")
                                .size(12.5)
                                .strong()
                                .color(TEXT),
                        );
                        mono_badge(ui, concat!("v", env!("CARGO_PKG_VERSION")), MUTED);
                    } else {
                        // Toggle secondary tree sidebar
                        let tree_btn = egui::Button::new(
                            RichText::new(if self.sidebar_open { "◧" } else { "☰" })
                                .size(13.0)
                                .color(TEXT_DIM),
                        )
                        .fill(INPUT)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(26.0, 24.0));
                        if ctx.screen_rect().width() >= SECONDARY_NAV_MIN_WIDTH {
                            if ui
                                .add(tree_btn)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Toggle channel sidebar")
                                .clicked()
                            {
                                self.sidebar_open = !self.sidebar_open;
                            }
                            ui.add_space(4.0);
                        }
                        ui.label(
                            RichText::new("Hardware Hub")
                                .size(13.0)
                                .strong()
                                .color(TEXT),
                        );
                        mono_badge(ui, concat!("v", env!("CARGO_PKG_VERSION")), MUTED);
                    }

                    if !narrow {
                        ui.add_space(8.0);
                    }
                    // Ctrl+K Quick-Jump pill button
                    let jump_w = if narrow { 52.0 } else { 220.0 };
                    let jump_text = if narrow {
                        "Jump…"
                    } else {
                        "Jump to page or sensor…"
                    };
                    let jump_btn =
                        egui::Button::new(RichText::new(jump_text).size(11.0).color(MUTED))
                            .fill(INPUT)
                            .stroke(Stroke::new(1.0_f32, BORDER))
                            .corner_radius(CornerRadius::same(7))
                            .min_size(Vec2::new(jump_w, 24.0));
                    if ui
                        .add(jump_btn)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(format!("Open Quick Jump ({JUMP_SHORTCUT})"))
                        .clicked()
                    {
                        self.open_palette();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Live status pill
                        if s.ready {
                            let stale_after = self.stale_secs();
                            let (color, text) =
                                match s.updated_at.map(|t| t.elapsed().as_secs_f32()) {
                                    Some(age) if age >= stale_after => (AMBER, "STALE"),
                                    Some(_) => (GREEN, "LIVE"),
                                    None => (MUTED, "WAIT"),
                                };
                            pill(ui, text, color);
                        } else {
                            pill(ui, "INIT", MUTED);
                        }

                        // Compact HWMonitor Mode <-> Expanded Workspace toggle
                        let mode_lbl = if self.compact_mode {
                            "Expand"
                        } else {
                            "Compact"
                        };
                        let mode_btn =
                            egui::Button::new(RichText::new(mode_lbl).size(11.0).color(TEXT_DIM))
                                .fill(if self.compact_mode {
                                    ACCENT_SOFT
                                } else {
                                    INPUT
                                })
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    if self.compact_mode { ACCENT } else { BORDER },
                                ))
                                .corner_radius(CornerRadius::same(7))
                                .min_size(Vec2::new(0.0, 24.0));
                        if ui
                            .add(mode_btn)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Switch between Compact and Expanded window modes")
                            .clicked()
                        {
                            self.toggle_compact_mode(ctx);
                        }

                        if !narrow {
                            // Floating HUD toggle
                            let hud_btn = egui::Button::new(
                                RichText::new(if self.show_widget { "● HUD" } else { "HUD" })
                                    .size(11.0)
                                    .color(if self.show_widget { GREEN } else { TEXT_DIM }),
                            )
                            .fill(if self.show_widget {
                                tint_alpha(GREEN, 30)
                            } else {
                                INPUT
                            })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if self.show_widget { GREEN } else { BORDER },
                            ))
                            .corner_radius(CornerRadius::same(7))
                            .min_size(Vec2::new(0.0, 24.0));
                            if ui
                                .add(hud_btn)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Toggle always-on-top Floating Monitor HUD")
                                .clicked()
                            {
                                self.show_widget = !self.show_widget;
                            }

                            // Simple | Pro density toggle
                            segmented(
                                ui,
                                &mut self.pro_density,
                                &[(false, "Simple"), (true, "Pro")],
                            );
                        } else {
                            ui.menu_button("Menu", |ui| {
                                ui.checkbox(&mut self.show_widget, "Floating HUD");
                                ui.checkbox(&mut self.pro_density, "Pro sensor table");
                                if ui
                                    .checkbox(&mut self.reduce_motion, "Reduce motion")
                                    .changed()
                                {
                                    set_reduce_motion(ui.ctx(), self.reduce_motion);
                                }
                            })
                            .response
                            .on_hover_text("Monitor options");
                        }
                    });
                });
            });
    }

    /// Switch cleanly between narrow HWMonitor Compact Mode (~440×700) and
    /// Expanded Workspace (remembered size) without ever forcing fullscreen.
    pub fn toggle_compact_mode(&mut self, ctx: &egui::Context) {
        let monitor = ctx.input(|i| i.viewport().monitor_size);
        if !self.compact_mode {
            self.compact_mode = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            resize_window(ctx, expanded_window_size(COMPACT_SIZE, monitor));
        } else {
            self.compact_mode = false;
            resize_window(ctx, expanded_window_size(self.expanded_size, monitor));
        }
    }

    /// Slack-inspired Dual-Rail Navigation:
    /// 1. Permanent 54px Icon Rail with 4 workspaces (Monitor / Hardware / Stress / Diagnostics)
    /// 2. Collapsible 190px Channel Tree showing only the active workspace's sub-pages
    pub fn navigation(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let running = self.state.lock().map(|st| st.running).unwrap_or(false);
        let screen_w = ctx.screen_rect().width();

        // Rail 1: 54px Primary Workspace Icon Rail (always visible, never clips)
        let rail_frame = egui::Frame {
            fill: RAIL,
            stroke: Stroke::NONE,
            inner_margin: egui::Margin {
                left: 6,
                right: 6,
                top: 10,
                bottom: 10,
            },
            ..Default::default()
        };
        egui::SidePanel::left("workspace-rail")
            .resizable(false)
            .exact_width(54.0)
            .frame(rail_frame)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter()
                    .vline(rect.right(), rect.y_range(), Stroke::new(1.0_f32, BORDER));
                let active_ws = self.page.workspace();
                ui.vertical_centered(|ui| {
                    for ws in Workspace::ALL {
                        let is_active = active_ws == ws;
                        let (btn_rect, resp) =
                            ui.allocate_exact_size(Vec2::new(40.0, 44.0), egui::Sense::click());
                        if ui.is_rect_visible(btn_rect) {
                            let p = ui.painter();
                            if is_active {
                                p.rect_filled(btn_rect, CornerRadius::same(10), ACCENT_SOFT);
                                p.rect_filled(
                                    Rect::from_min_size(
                                        Pos2::new(btn_rect.left() - 5.0, btn_rect.top() + 8.0),
                                        Vec2::new(3.0, btn_rect.height() - 16.0),
                                    ),
                                    CornerRadius::same(2),
                                    ACCENT,
                                );
                            } else if resp.hovered() {
                                p.rect_filled(
                                    btn_rect,
                                    CornerRadius::same(10),
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 14),
                                );
                            }
                            let icon_col = if is_active {
                                ACCENT
                            } else if resp.hovered() {
                                TEXT
                            } else {
                                MUTED
                            };
                            draw_workspace_icon(
                                p,
                                Pos2::new(btn_rect.center().x, btn_rect.top() + 16.0),
                                ws,
                                icon_col,
                            );
                            p.text(
                                Pos2::new(btn_rect.center().x, btn_rect.bottom() - 8.0),
                                egui::Align2::CENTER_CENTER,
                                ws.short_label(),
                                egui::FontId::proportional(8.5),
                                if is_active { TEXT } else { MUTED },
                            );
                            // Running stress indicator dot
                            if ws == Workspace::Stress && running {
                                p.circle_filled(
                                    Pos2::new(btn_rect.right() - 6.0, btn_rect.top() + 7.0),
                                    3.5,
                                    RED,
                                );
                            }
                        }
                        if resp.clicked() && active_ws != ws {
                            self.page = ws.default_page();
                        }
                        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(ws.label());
                        ui.add_space(4.0);
                    }
                });

                // Bottom utility icons on the rail
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    let help_r = ui
                        .add(
                            egui::Button::new(RichText::new("?").monospace().strong().color(MUTED))
                                .fill(INPUT)
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .corner_radius(CornerRadius::same(7))
                                .min_size(Vec2::new(32.0, 26.0)),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Keyboard shortcuts (? or F1)");
                    if help_r.clicked() {
                        self.show_help = !self.show_help;
                    }
                });
            });

        // Rail 2: Collapsible Slack Channel Tree (hidden in Compact Mode or < 760px)
        if self.compact_mode || !self.sidebar_open || screen_w < SECONDARY_NAV_MIN_WIDTH {
            return;
        }
        let active_ws = self.page.workspace();
        let tree_frame = egui::Frame {
            fill: PANEL,
            stroke: Stroke::NONE,
            inner_margin: egui::Margin {
                left: 10,
                right: 10,
                top: 12,
                bottom: 10,
            },
            ..Default::default()
        };
        egui::SidePanel::left("channel-tree")
            .resizable(false)
            .exact_width(190.0)
            .frame(tree_frame)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter()
                    .vline(rect.right(), rect.y_range(), Stroke::new(1.0_f32, BORDER));
                ui.label(
                    RichText::new(active_ws.label().to_uppercase())
                        .size(10.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.add_space(8.0);

                egui::TopBottomPanel::bottom("tree-footer")
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        page_rule(ui);
                        ui.add_space(8.0);
                        if toggle(ui, &mut self.reduce_motion, "Reduce motion").changed() {
                            set_reduce_motion(ui.ctx(), self.reduce_motion);
                        }
                        if running {
                            ui.add_space(4.0);
                            let stopping = self.cancel.load(Ordering::Relaxed);
                            if ui
                                .add_enabled_ui(!stopping, |ui| {
                                    danger_button(
                                        ui,
                                        if stopping {
                                            "Stopping…"
                                        } else {
                                            "■ Stop Check"
                                        },
                                        true,
                                        26.0,
                                    )
                                })
                                .inner
                                .clicked()
                            {
                                self.cancel.store(true, Ordering::Relaxed);
                                self.message = "Stopping check…".into();
                            }
                        }
                        ui.add_space(6.0);
                        let backend = if s.adapters.is_empty() {
                            "Local · read-only".into()
                        } else {
                            format!("Local · wgpu · {}", s.gpu_backend)
                        };
                        ui.label(RichText::new(backend).monospace().size(9.5).color(MUTED));
                    });

                egui::ScrollArea::vertical()
                    .id_salt("channel-pages")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for &page in active_ws.pages() {
                            if channel_item(ui, self.page == page, page).clicked() {
                                self.page = page;
                            }
                        }
                    });
            });
    }

    /// Compact horizontal sub-tabs when the secondary channel tree is hidden
    /// (Compact HWMonitor mode or narrow widths), so every sub-page stays 1 click away.
    pub fn compact_subtabs(&mut self, ui: &mut egui::Ui) {
        let pages = self.page.workspace().pages();
        if pages.len() <= 1 {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for &p in pages {
                let active = self.page == p;
                let btn = egui::Button::new(RichText::new(p.label()).size(10.5).color(if active {
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
                    self.page = p;
                }
            }
        });
        ui.add_space(8.0);
    }

    pub fn status_bar(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let frame = egui::Frame::NONE.fill(PANEL);
        egui::TopBottomPanel::bottom("status")
            .frame(frame)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top() + 0.5,
                    Stroke::new(1.0_f32, BORDER),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    if self.alert.active_since().is_some() {
                        pill(ui, "TEMP ALERT", RED);
                        ui.add_space(6.0);
                    }
                    if self.message.is_empty() {
                        let (dot, text) = if !s.ready {
                            (MUTED, "Collecting readings…".to_string())
                        } else {
                            let stale_after = self.stale_secs();
                            match s.updated_at.map(|t| t.elapsed().as_secs_f32()) {
                                Some(age) if age > stale_after => {
                                    (AMBER, format!("Stale ({age:.0}s)"))
                                }
                                Some(age) => (GREEN, format!("Live · {age:.1}s ago")),
                                None => (MUTED, "Waiting".to_string()),
                            }
                        };
                        ui.painter().circle_filled(
                            ui.cursor().left_center() + Vec2::new(4.0, 0.0),
                            3.0,
                            dot,
                        );
                        ui.add_space(12.0);
                        ui.label(RichText::new(text).size(10.5).color(TEXT_DIM));
                    } else {
                        let is_err = self.message.contains("failed")
                            || self.message.contains("Failed")
                            || self.message.contains("exceeds")
                            || self.message.contains("Limits");
                        ui.colored_label(if is_err { RED } else { ACCENT }, &self.message);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(std::env::consts::OS.to_uppercase())
                                .monospace()
                                .size(9.5)
                                .color(MUTED),
                        );
                        if s.ready {
                            ui.add_space(10.0);
                            let ram = if s.memory_total > 0 {
                                format!(
                                    "RAM {:.0}%",
                                    s.memory_used as f32 * 100.0 / s.memory_total as f32
                                )
                            } else {
                                "RAM —".into()
                            };
                            let cpu = if s.cpu_usage.is_finite() && !s.cores.is_empty() {
                                format!("CPU {:.0}%", s.cpu_usage)
                            } else {
                                "CPU —".into()
                            };
                            ui.label(
                                RichText::new(format!("{cpu}  {ram}"))
                                    .monospace()
                                    .size(10.0)
                                    .color(TEXT_DIM),
                            );
                        }
                    });
                });
                ui.add_space(6.0);
            });
    }

    /// Global `Ctrl+K` / `Cmd+K` Quick-Jump Command Palette.
    pub fn quick_jump_palette(&mut self, ctx: &egui::Context, s: &Snapshot) {
        if !self.show_palette {
            return;
        }
        let (up, down, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let frame = egui::Frame::NONE
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(14))
            .shadow(egui::epaint::Shadow {
                offset: [0, 16],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha(140),
            });
        let modal = egui::Modal::new(egui::Id::new("quick-jump-palette"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 64.0).clamp(220.0, 440.0));
                let edit = text_field(
                    ui,
                    &mut self.palette_query,
                    "Search pages, sensors, checks or processes…",
                    ui.available_width(),
                );
                if self.palette_focus {
                    edit.request_focus();
                    self.palette_focus = false;
                }
                if edit.changed() {
                    self.palette_index = 0;
                }
                ui.add_space(8.0);

                let q = self.palette_query.trim().to_ascii_lowercase();
                let mut results = Vec::new();
                for page in Page::ALL {
                    let rank = [page.nav_label(), page.label(), page.keywords()]
                        .into_iter()
                        .filter_map(|name| search_rank(name, &q))
                        .min();
                    if let Some(rank) = rank {
                        results.push((
                            rank,
                            JumpTarget::Page(page),
                            format!("{} · {}", page.nav_label(), page.workspace().label()),
                        ));
                    }
                }
                if !q.is_empty() {
                    for sensor in &s.temperatures {
                        let rank = [sensor.key.as_str(), sensor.label.as_str()]
                            .into_iter()
                            .filter_map(|name| search_rank(name, &q))
                            .min();
                        if let Some(rank) = rank {
                            results.push((
                                rank,
                                JumpTarget::Temperature(&sensor.key),
                                format!("{} · {}", sensor.label, sensor.key),
                            ));
                        }
                    }
                    for (kind, label, _) in STRESS_KINDS {
                        let match_text = format!("{label} check");
                        if let Some(rank) = search_rank(&match_text, &q) {
                            results.push((
                                rank,
                                JumpTarget::Stress(kind),
                                format!("{} check · Stress Lab", kind_label(kind)),
                            ));
                        }
                    }
                    for process in &s.processes {
                        if let Some(rank) = search_rank(&process.name, &q) {
                            results.push((
                                rank + 1,
                                JumpTarget::Process(&process.name),
                                format!("{} · PID {}", process.name, process.pid),
                            ));
                        }
                    }
                }
                results.sort_by(|a, b| (a.0, &a.2).cmp(&(b.0, &b.2)));
                results.truncate(9);
                if results.is_empty() {
                    empty_note(ui, "No matches.");
                }
                self.palette_index = self.palette_index.min(results.len().saturating_sub(1));
                if up {
                    self.palette_index = self.palette_index.saturating_sub(1);
                } else if down {
                    self.palette_index =
                        (self.palette_index + 1).min(results.len().saturating_sub(1));
                }
                egui::ScrollArea::vertical()
                    .id_salt("quick-jump-results")
                    .max_height((ctx.screen_rect().height() - 138.0).max(112.0))
                    .show(ui, |ui| {
                        for (i, (_, target, label)) in results.into_iter().enumerate() {
                            let selected = self.palette_index == i;
                            let button =
                                egui::Button::new(RichText::new(label).size(12.0).color(TEXT))
                                    .truncate()
                                    .fill(if selected {
                                        ACCENT_SOFT
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .min_size(Vec2::new(ui.available_width(), 28.0));
                            let response = ui
                                .add(button)
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if selected && (up || down) {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() || (selected && enter) {
                                self.page = match target {
                                    JumpTarget::Page(page) => page,
                                    JumpTarget::Stress(kind) => {
                                        if !self.state.lock().is_ok_and(|state| state.running) {
                                            self.kind = kind;
                                        }
                                        Page::Tests
                                    }
                                    JumpTarget::Temperature(key) => {
                                        self.selected_temp_key = key.to_owned();
                                        Page::Thermal
                                    }
                                    JumpTarget::Process(name) => {
                                        self.process_query = name.to_owned();
                                        Page::Processes
                                    }
                                };
                                self.show_palette = false;
                            }
                        }
                    });
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Up/Down to select · Enter to open · Esc to close")
                        .size(10.0)
                        .color(MUTED),
                );
            });
        if modal.should_close() {
            self.show_palette = false;
        }
    }

    /// Keyboard shortcut cheat-sheet modal (`?` or `F1`).
    pub fn help_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let frame = egui::Frame::NONE
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(18));
        let modal = egui::Modal::new(egui::Id::new("help-modal"))
            .backdrop_color(Color32::from_black_alpha(150))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 72.0).clamp(220.0, 450.0));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Keyboard Shortcuts").size(14.5).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ghost_button(ui, "Close").clicked() {
                            self.show_help = false;
                        }
                    });
                });
                ui.add_space(8.0);
                page_rule(ui);
                ui.add_space(10.0);
                let rows = [
                    ("Ctrl/⌘ K", "Open Quick Jump command palette"),
                    ("Ctrl/⌘ 1–9", "Overview, CPU, GPU, Memory, Storage, Thermals, Details, Processes, Reports"),
                    ("Ctrl/⌘ Shift 1–6", "Board, Drivers, Network, Bluetooth, Frames, Stress Lab"),
                    ("Esc", "Stop running check, or close overlay"),
                    ("?  /  F1", "Toggle shortcuts help"),
                ];
                egui::Grid::new("help_grid")
                    .num_columns(2)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        for (k, desc) in rows {
                            keycap(ui, k);
                            ui.label(RichText::new(desc).size(11.5).color(TEXT_DIM));
                            ui.end_row();
                        }
                    });
            });
        if modal.should_close() {
            self.show_help = false;
        }
    }
}

fn channel_item(ui: &mut egui::Ui, active: bool, page: Page) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if active {
            p.rect_filled(rect, CornerRadius::same(7), ACCENT_SOFT);
            p.rect_filled(
                Rect::from_min_size(
                    rect.left_top() + Vec2::new(1.0, 6.0),
                    Vec2::new(2.5, rect.height() - 12.0),
                ),
                CornerRadius::same(2),
                ACCENT,
            );
        } else if response.hovered() {
            p.rect_filled(
                rect,
                CornerRadius::same(7),
                Color32::from_rgba_unmultiplied(255, 255, 255, 12),
            );
        }
        let icon_c = if active {
            ACCENT
        } else if response.hovered() {
            TEXT
        } else {
            MUTED
        };
        let text_c = if active || response.hovered() {
            TEXT
        } else {
            TEXT_DIM
        };
        draw_nav_icon(p, rect.left_center() + Vec2::new(16.0, 0.0), page, icon_c);
        p.text(
            rect.left_center() + Vec2::new(32.0, 0.0),
            egui::Align2::LEFT_CENTER,
            page.nav_label(),
            egui::FontId::proportional(12.0),
            text_c,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("{} · Ctrl+{}", page.nav_label(), page.shortcut()))
}

/// Sleek compact Floating Monitor HUD showing CPU, RAM, Peak Temp, and any
/// user-pinned sensors.
pub fn floating_hud(ui: &mut egui::Ui, s: &Snapshot, pinned: &[String], stale_after: Duration) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("HARDWARE HUB")
                .size(9.5)
                .strong()
                .color(MUTED),
        );
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| match s.updated_at {
                Some(at) if at.elapsed() >= stale_after => chip(ui, "STALE", AMBER),
                Some(_) if s.ready => chip(ui, "● LIVE", GREEN),
                _ => chip(ui, "INIT", MUTED),
            },
        );
    });
    ui.add_space(6.0);
    let peak_c = s
        .temperatures
        .iter()
        .filter_map(|t| t.celsius)
        .fold(None::<f32>, |acc, v| Some(acc.map_or(v, |p| p.max(v))));
    ui.columns(3, |cols| {
        cols[0].label(RichText::new("CPU").size(9.0).strong().color(MUTED));
        let cpu_s = if s.ready && !s.cores.is_empty() && s.cpu_usage.is_finite() {
            format!("{:.0}%", s.cpu_usage)
        } else {
            "—".into()
        };
        cols[0].label(
            RichText::new(cpu_s)
                .monospace()
                .size(16.0)
                .strong()
                .color(load_color(s.cpu_usage / 100.0)),
        );

        cols[1].label(RichText::new("RAM").size(9.0).strong().color(MUTED));
        let ram_s = if s.ready && s.memory_total > 0 {
            format!(
                "{:.0}%",
                s.memory_used as f64 * 100.0 / s.memory_total as f64
            )
        } else {
            "—".into()
        };
        cols[1].label(
            RichText::new(ram_s)
                .monospace()
                .size(16.0)
                .strong()
                .color(VIOLET),
        );

        cols[2].label(RichText::new("PEAK").size(9.0).strong().color(MUTED));
        let pk_s = peak_c
            .map(|v| format!("{v:.0}°C"))
            .unwrap_or_else(|| "—".into());
        cols[2].label(
            RichText::new(pk_s)
                .monospace()
                .size(16.0)
                .strong()
                .color(peak_c.map(temperature_color).unwrap_or(MUTED)),
        );
    });
    if !pinned.is_empty() {
        ui.add_space(6.0);
        page_rule(ui);
        ui.add_space(4.0);
        for key in pinned {
            if let Some(t) = s.temperatures.iter().find(|t| &t.key == key) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(truncate(&t.label, 20))
                            .size(10.5)
                            .color(TEXT_DIM),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let val = t
                            .celsius
                            .map(|v| format!("{v:.0} °C"))
                            .unwrap_or_else(|| "—".into());
                        ui.label(
                            RichText::new(val)
                                .monospace()
                                .size(11.0)
                                .strong()
                                .color(t.celsius.map(temperature_color).unwrap_or(MUTED)),
                        );
                    });
                });
            } else if let Some(f) = s.fans.iter().find(|f| &f.key == key) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(truncate(&f.label, 20))
                            .size(10.5)
                            .color(TEXT_DIM),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let val = f
                            .rpm
                            .map(|v| format!("{v:.0} RPM"))
                            .unwrap_or_else(|| "—".into());
                        ui.label(RichText::new(val).monospace().size(11.0).color(TEXT));
                    });
                });
            }
        }
    }
}
