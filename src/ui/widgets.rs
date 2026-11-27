//! Reusable, high-performance UI widgets for Hardware Hub v2.
//!
//! Includes:
//! - **Liquid / Water-Fill CPU Core Grid**: scales cleanly from 4 to 128+ threads
//! - **Scalable Multi-DIMM / Unified Memory Visualizer**: 1–4 sticks, 5–16+ rack, or Apple Silicon unified
//! - **Modern HWMonitor Sensor Table**: Min–Current–Max visual range bar, provenance badge, click-to-plot, HUD pin
//! - **Circular Progress Ring & Mini Arc Gauges**
//! - **Virtualized Process & Driver Tables** (`ScrollArea::show_rows`)

use crate::ui::{theme::*, Page, Workspace};
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Vec2};
use hardware_hub::{
    history::History,
    specs::{DriverSpec, MemoryProfile},
    stress::{Config, ErrorDetail, Kind, RunMode},
    telemetry::{Fan, Process, Snapshot, Temperature},
};
use std::{f32::consts::TAU, sync::Arc};

// ---------------------------------------------------------------- text & buttons
pub fn elided(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id: egui::FontId::proportional(size),
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(12.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts(|fonts| fonts.layout_job(job))
}

pub fn text_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    text_field_enabled(ui, true, text, hint, width)
}

pub fn text_field_enabled(
    ui: &mut egui::Ui,
    enabled: bool,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).size(11.5).color(MUTED))
            .margin(egui::Margin::symmetric(9, 5))
            .background_color(INPUT)
            .desired_width(width),
    )
}

pub fn primary_button(
    ui: &mut egui::Ui,
    text: &str,
    full_width: bool,
    height: f32,
) -> egui::Response {
    let mut button = egui::Button::new(RichText::new(text).size(12.5).strong().color(ACTION_INK))
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(8));
    if full_width {
        button = button.min_size(egui::vec2(ui.available_width(), height));
    } else {
        button = button.min_size(egui::vec2(0.0, height));
    }
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn danger_button(
    ui: &mut egui::Ui,
    text: &str,
    full_width: bool,
    height: f32,
) -> egui::Response {
    let mut button = egui::Button::new(RichText::new(text).size(12.5).strong().color(ACTION_INK))
        .fill(RED)
        .corner_radius(CornerRadius::same(8));
    if full_width {
        button = button.min_size(egui::vec2(ui.available_width(), height));
    } else {
        button = button.min_size(egui::vec2(0.0, height));
    }
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.0).color(TEXT_DIM))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(CornerRadius::same(7)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.fonts(|fonts| {
        fonts.layout_no_wrap(label.to_owned(), font.clone(), ui.visuals().text_color())
    });
    let height: f32 = 22.0;
    let width = ui.available_width().min(galley.size().x + 42.0).max(120.0);
    let (rect, mut response) = ui.allocate_exact_size(
        egui::vec2(width, height.max(galley.size().y)),
        egui::Sense::click(),
    );
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let ctx = ui.ctx();
        let on = animated_bool(ctx, response.id.with("on"), *value, MOTION_FAST);
        let painter = ui.painter();
        // Sleek pill switch (28x16)
        let track = Rect::from_center_size(
            Pos2::new(rect.left() + 14.0, rect.center().y),
            Vec2::new(28.0, 15.0),
        );
        let fill = lerp_color(TRACK, ACCENT, on);
        painter.rect_filled(track, CornerRadius::same(8), fill);
        let knob_x = track.left() + 7.5 + on * (track.width() - 15.0);
        painter.circle_filled(Pos2::new(knob_x, track.center().y), 5.5, TEXT);
        let text_color = if *value || response.hovered() {
            TEXT
        } else {
            TEXT_DIM
        };
        painter.galley(
            Pos2::new(rect.left() + 36.0, rect.center().y - galley.size().y / 2.0),
            galley,
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    current: &mut T,
    options: &[(T, &'static str)],
) -> bool {
    let mut changed = false;
    let font = egui::FontId::proportional(11.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap((*label).to_owned(), font.clone(), TEXT_DIM)
                    .size()
                    .x
            })
        })
        .collect();
    let seg_h = 23.0;
    let pad_x = 18.0;
    let total: f32 = widths.iter().sum::<f32>() + pad_x * options.len() as f32 + 4.0;
    let (rect, group) = ui.allocate_exact_size(Vec2::new(total, seg_h + 4.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return changed;
    }
    ui.painter().rect_filled(rect, CornerRadius::same(8), INPUT);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 2.0;
    for (index, (value, label)) in options.iter().enumerate() {
        let w = widths[index] + pad_x;
        let seg = Rect::from_min_size(Pos2::new(x, rect.top() + 2.0), Vec2::new(w, seg_h));
        let response = ui.interact(seg, group.id.with(index), egui::Sense::click());
        let selected = *current == *value;
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                selected,
                *label,
            )
        });
        if response.clicked() && !selected {
            *current = *value;
            changed = true;
        }
        if selected {
            ui.painter()
                .rect_filled(seg, CornerRadius::same(6), CARD_HI);
            ui.painter().rect_stroke(
                seg,
                CornerRadius::same(6),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 28)),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter().rect_filled(
                seg,
                CornerRadius::same(6),
                Color32::from_rgba_unmultiplied(255, 255, 255, 10),
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                seg,
                CornerRadius::same(6),
                Stroke::new(1.0_f32, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            seg.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if selected { TEXT } else { TEXT_DIM },
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        x += w;
    }
    changed
}

pub fn picker<T: PartialEq + Copy>(
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

// ---------------------------------------------------------------- badges & headers
pub fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    let ink = badge_ink(color);
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(10.5), ink);
    let size = galley.size() + Vec2::new(24.0, 9.0);
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
    painter.rect_stroke(
        rect,
        CornerRadius::same((rect.height() / 2.0) as u8),
        Stroke::new(1.0_f32, tint_alpha(color, 85)),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(rect.left_center() + Vec2::new(9.5, 0.0), 3.0, color);
    painter.galley(
        rect.left_center() + Vec2::new(17.0, -galley.size().y / 2.0),
        galley,
        ink,
    );
}

pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    let ink = badge_ink(color);
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(10.0), ink);
    let size = galley.size() + Vec2::new(14.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(5), tint(color));
    painter.rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(1.0_f32, tint_alpha(color, 72)),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.left_top() + Vec2::new(7.0, 3.0), galley, ink);
}

pub fn mono_badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let ink = badge_ink(color);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::monospace(9.5), ink);
    let size = galley.size() + Vec2::new(10.0, 5.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), INPUT);
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.left_top() + Vec2::new(5.0, 2.5), galley, ink);
}

pub fn legend_dot(ui: &mut egui::Ui, label: &str, color: Color32) {
    let galley = ui.fonts(|fonts| {
        fonts.layout_no_wrap(label.to_owned(), egui::FontId::proportional(10.5), TEXT_DIM)
    });
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 15.0, 16.0),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.circle_filled(Pos2::new(rect.left() + 4.0, rect.center().y), 3.0, color);
    painter.galley(
        Pos2::new(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT_DIM,
    );
}

pub fn keycap(ui: &mut egui::Ui, text: &str) {
    let font = egui::FontId::monospace(10.0);
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(text.to_owned(), font, TEXT_DIM));
    let size = galley.size() + Vec2::new(14.0, 7.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(5), INPUT);
    painter.rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, TEXT_DIM);
}

pub fn chevron(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let c = rect.center();
        let color = if response.hovered() { TEXT } else { MUTED };
        let points = if open {
            vec![
                c + Vec2::new(-4.0, -2.0),
                c + Vec2::new(4.0, -2.0),
                c + Vec2::new(0.0, 2.6),
            ]
        } else {
            vec![
                c + Vec2::new(-2.0, -4.0),
                c + Vec2::new(2.6, 0.0),
                c + Vec2::new(-2.0, 4.0),
            ]
        };
        ui.painter()
            .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn collapsible_header(
    ui: &mut egui::Ui,
    title: &str,
    hint: Option<&str>,
    open: bool,
    right: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut toggled = false;
    ui.horizontal(|ui| {
        toggled |= chevron(ui, open).clicked();
        let title_r = ui.add(
            egui::Label::new(RichText::new(title).size(12.5).strong().color(TEXT))
                .sense(egui::Sense::click()),
        );
        toggled |= title_r.clicked();
        if let Some(hint) = hint {
            let hint_r = ui.add(
                egui::Label::new(RichText::new(hint).size(10.5).color(MUTED))
                    .sense(egui::Sense::click()),
            );
            toggled |= hint_r.clicked();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui);
        });
    });
    toggled
}

pub fn section_title(ui: &mut egui::Ui, title: &str, hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(12.5).strong().color(TEXT));
        if let Some(hint) = hint {
            ui.label(RichText::new(hint).size(10.5).color(MUTED));
        }
    });
    ui.add_space(2.0);
}

pub fn empty_note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(MUTED).italics());
}

pub fn page_rule(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, HAIRLINE);
}

pub fn callout(ui: &mut egui::Ui, tone: Color32, title: &str, body: &str) {
    let frame = egui::Frame {
        fill: tint_alpha(tone, 20),
        stroke: Stroke::new(1.0_f32, tint_alpha(tone, 72)),
        corner_radius: CornerRadius::same(9),
        inner_margin: egui::Margin {
            left: 14,
            right: 12,
            top: 9,
            bottom: 9,
        },
        ..Default::default()
    };
    let response = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(11.5).strong().color(TEXT));
        ui.label(RichText::new(body).size(10.5).color(TEXT_DIM));
    });
    let bar = Rect::from_min_size(
        response.response.rect.left_top() + Vec2::new(4.0, 9.0),
        Vec2::new(2.5, (response.response.rect.height() - 18.0).max(8.0)),
    );
    ui.painter().rect_filled(bar, CornerRadius::same(2), tone);
}

pub fn bar(ui: &mut egui::Ui, id: &str, fraction: f32, color: Color32, height: f32) {
    let width = ui.available_width().max(24.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let radius = CornerRadius::same((height / 2.0) as u8);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, TRACK);
    let fraction = animated_value(
        ui.ctx(),
        egui::Id::new(("bar", id)),
        fraction.clamp(0.0, 1.0),
        MOTION_MEDIUM,
    );
    if fraction > 0.0 {
        let filled = Rect::from_min_size(
            rect.min,
            Vec2::new((rect.width() * fraction).max(height), rect.height()),
        );
        painter.rect_filled(filled, radius, color);
    }
}

pub fn scroll_fade(ui: &egui::Ui, viewport: Rect, color: Color32) {
    let height = 32.0_f32.min(viewport.height() * 0.3);
    if height <= 1.0 {
        return;
    }
    let bottom = viewport.bottom().min(ui.clip_rect().bottom());
    let solid = 10.0_f32.min(height * 0.35);
    ui.painter().rect_filled(
        Rect::from_min_max(
            Pos2::new(viewport.left(), bottom - solid),
            Pos2::new(viewport.right(), bottom),
        ),
        0.0,
        color,
    );
    let bottom = bottom - solid;
    let top = bottom - (height - solid);
    let clear = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    let half = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 145);
    let left = viewport.left();
    let right = viewport.right();
    let mid = top + height * 0.45;
    let mut mesh = egui::Mesh::default();
    for (y, t) in [(top, clear), (mid, half), (bottom, color)] {
        mesh.colored_vertex(Pos2::new(left, y), t);
        mesh.colored_vertex(Pos2::new(right, y), t);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh.add_triangle(2, 3, 4);
    mesh.add_triangle(2, 4, 5);
    ui.painter().add(egui::Shape::mesh(mesh));
}

// ---------------------------------------------------------------- gauges
/// Draw a clean circular arc gauge (e.g. for KPI cards or Stress Lab cockpit).
pub fn circular_progress(
    ui: &mut egui::Ui,
    id: &str,
    fraction: Option<f32>,
    center_text: &str,
    sub_text: &str,
    diameter: f32,
    color: Color32,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(diameter), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let center = rect.center();
    let radius = diameter * 0.43;
    let thickness = (diameter * 0.085).clamp(4.0, 8.0);

    // Background ring
    painter.circle_stroke(center, radius, Stroke::new(thickness, TRACK));

    // Active arc (animated when determinate; rotating sweep when continuous/None)
    let (start_angle, sweep) = match fraction {
        Some(f) => {
            let eased = animated_value(
                ui.ctx(),
                egui::Id::new(("ring", id)),
                f.clamp(0.0, 1.0),
                MOTION_MEDIUM,
            );
            (-TAU * 0.25, eased * TAU)
        }
        None => {
            let t = ui.ctx().input(|i| i.time) as f32;
            if motion_enabled(ui.ctx()) {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(80));
            }
            (-TAU * 0.25 + (t * 0.85) % TAU, TAU * 0.35)
        }
    };

    if sweep > 0.01 {
        let steps = ((sweep / TAU) * 48.0).ceil().clamp(6.0, 48.0) as usize;
        let pts: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let a = start_angle + sweep * (i as f32 / steps as f32);
                Pos2::new(center.x + radius * a.cos(), center.y + radius * a.sin())
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(thickness, color)));
    }

    let main_size = (diameter * 0.22).clamp(12.0, 22.0);
    painter.text(
        Pos2::new(
            center.x,
            center.y - if sub_text.is_empty() { 0.0 } else { 6.0 },
        ),
        egui::Align2::CENTER_CENTER,
        center_text,
        egui::FontId::monospace(main_size),
        TEXT,
    );
    if !sub_text.is_empty() {
        painter.text(
            Pos2::new(center.x, center.y + main_size * 0.65),
            egui::Align2::CENTER_CENTER,
            sub_text,
            egui::FontId::proportional((diameter * 0.11).clamp(9.0, 11.0)),
            MUTED,
        );
    }
}

// ---------------------------------------------------------------- liquid water-fill CPU grid
/// Adaptive CPU Core Liquid / Water-Fill Grid.
///
/// Renders each core tile with a smooth rising liquid water-fill level (`0..100%`)
/// and scales gracefully across 4-core laptops, 16–32 thread hybrid CPUs, and
/// 64–128+ thread workstations without overflowing the screen.
pub fn liquid_core_grid(ui: &mut egui::Ui, id_prefix: &str, snapshot: &Snapshot, compact: bool) {
    let cores = &snapshot.cores;
    if cores.is_empty() {
        empty_note(ui, "Waiting for logical processor usage.");
        return;
    }
    let has_core_temperature = snapshot.core_celsius.iter().any(Option::is_some);
    let description = if has_core_temperature {
        "Load by logical processor. Threads on the same physical core share its temperature."
    } else if snapshot.temperatures.iter().any(|sensor| sensor.cpu_core) {
        "Load by logical processor. Measured CPU core temperatures are listed in the sensor table."
    } else if snapshot
        .temperatures
        .iter()
        .any(|sensor| sensor.source == "ACPI")
    {
        "Load by logical processor. Windows reports ACPI zone temperatures under Thermals / fans."
    } else {
        "Load by logical processor. Per-core temperatures are unavailable from this provider."
    };
    ui.label(RichText::new(description).size(10.5).color(TEXT_DIM));
    ui.add_space(6.0);
    let count = cores.len();
    let width = ui.available_width().max(180.0);

    // For high-core-count workstations (>32 threads), show a Top-4 Busiest strip first
    // so hot threads jump out immediately even when the micro-tile grid is dense.
    if count > 32 {
        let mut ranked: Vec<(usize, f32)> = cores
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, load)| load.is_finite())
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("BUSIEST THREADS")
                    .size(9.5)
                    .strong()
                    .color(MUTED),
            );
            for &(idx, pct) in ranked.iter().take(4) {
                let tone = load_color(pct / 100.0);
                mono_badge(ui, &format!("LP {idx} {}", thread_load_text(pct)), tone);
            }
        });
        ui.add_space(6.0);
    }

    let (columns, tile_h, micro) = core_grid_layout(count, width, compact);

    let gap = if micro { 4.0 } else { 7.0 };
    let tile_w = ((width - gap * columns.saturating_sub(1) as f32) / columns as f32).max(28.0);
    let rows = count.div_ceil(columns);
    let render_row = |ui: &mut egui::Ui, row_idx: usize| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let start = row_idx * columns;
            let end = (start + columns).min(count);
            for (core_idx, raw_usage) in cores.iter().enumerate().take(end).skip(start) {
                let usage = raw_usage.clamp(0.0, 100.0);
                let mhz = snapshot.core_mhz.get(core_idx).copied().filter(|m| *m > 0);
                let temperature = snapshot.core_celsius.get(core_idx).copied().flatten();
                liquid_core_tile(
                    ui,
                    id_prefix,
                    core_idx,
                    usage,
                    mhz,
                    temperature,
                    Vec2::new(tile_w, tile_h),
                    micro,
                );
            }
        });
    };

    if rows > 8 {
        // Virtualize huge 64–256 thread arrays so frame time stays <1ms.
        egui::ScrollArea::vertical()
            .id_salt((id_prefix, "liquid-cores-scroll"))
            .max_height(260.0)
            .auto_shrink([false, true])
            .show_rows(ui, tile_h + gap, rows, |ui, range| {
                ui.spacing_mut().item_spacing.y = gap;
                for r in range {
                    render_row(ui, r);
                }
            });
    } else {
        ui.spacing_mut().item_spacing.y = gap;
        for r in 0..rows {
            render_row(ui, r);
        }
    }
}

fn thread_load_text(usage: f32) -> String {
    match usage {
        v if !v.is_finite() => "—".into(),
        v if v > 0.0 && v < 1.0 => "<1%".into(),
        v => format!("{v:.0}%"),
    }
}

#[allow(clippy::too_many_arguments)]
fn liquid_core_tile(
    ui: &mut egui::Ui,
    id_prefix: &str,
    index: usize,
    usage: f32,
    mhz: Option<u64>,
    temperature: Option<f32>,
    size: Vec2,
    micro: bool,
) {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let target = if usage.is_finite() {
        usage / 100.0
    } else {
        0.0
    };
    let frac = animated_value(
        ui.ctx(),
        egui::Id::new((id_prefix, "liq", index)),
        target,
        MOTION_MEDIUM,
    );
    let radius = CornerRadius::same(if micro { 5 } else { 8 });
    let painter = ui.painter();

    painter.rect_filled(rect, radius, INPUT);

    // Reserve a dry row above the gauge so the load readout keeps its contrast.
    let well = if micro {
        rect.shrink(2.0)
    } else {
        Rect::from_min_max(
            rect.left_top() + Vec2::new(5.0, 22.0),
            rect.right_bottom() - Vec2::splat(5.0),
        )
    };
    painter.rect_filled(well, CornerRadius::same(3), HAIRLINE);
    if frac > 0.005 {
        let surface = well.bottom() - well.height() * frac;
        let amplitude = ((surface - well.top()).min(well.bottom() - surface) * 0.68)
            .min(if micro { 1.4 } else { 3.4 });
        let phase = if motion_enabled(ui.ctx()) {
            frac * TAU * 1.6
        } else {
            0.0
        };
        let surface_color = load_color(frac);
        let deep_color = lerp_color(surface_color, INPUT, 0.55);
        let segments = if micro { 6 } else { 12 };
        let mut mesh = egui::Mesh::default();
        let mut crest = Vec::with_capacity(segments + 1);
        for segment in 0..=segments {
            let x = segment as f32 / segments as f32;
            let wave =
                ((TAU * x + phase).sin() * 0.74 + (2.0 * TAU * x - phase).sin() * 0.26) * amplitude;
            let point = Pos2::new(well.left() + well.width() * x, surface + wave);
            crest.push(point);
            mesh.colored_vertex(point, surface_color);
            mesh.colored_vertex(Pos2::new(point.x, well.bottom()), deep_color);
            if segment > 0 {
                let base = (segment as u32 - 1) * 2;
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base + 1, base + 2, base + 3);
            }
        }
        let clipped = painter.with_clip_rect(well);
        clipped.add(egui::Shape::mesh(mesh));
        clipped.add(egui::Shape::line(
            crest,
            Stroke::new(1.2_f32, lerp_color(surface_color, TEXT, 0.38)),
        ));
    }

    if micro {
        let load_rect = Rect::from_center_size(rect.center(), Vec2::new(27.0, 13.0));
        painter.rect_filled(load_rect, CornerRadius::same(4), tint_alpha(INPUT, 232));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            thread_load_text(usage),
            egui::FontId::monospace(9.5),
            TEXT,
        );
    } else {
        painter.text(
            rect.left_top() + Vec2::new(6.0, 5.0),
            egui::Align2::LEFT_TOP,
            format!("LP {index}"),
            egui::FontId::monospace(10.0),
            TEXT_DIM,
        );
        painter.text(
            rect.right_top() + Vec2::new(-6.0, 4.0),
            egui::Align2::RIGHT_TOP,
            thread_load_text(usage),
            egui::FontId::monospace(12.0),
            TEXT,
        );
        if let Some(temp) = temperature {
            let badge =
                Rect::from_min_size(well.left_top() + Vec2::new(2.0, 2.0), Vec2::new(36.0, 13.0));
            painter.rect_filled(badge, CornerRadius::same(3), INPUT);
            painter.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                format!("{temp:.0} °C"),
                egui::FontId::monospace(9.0),
                TEXT,
            );
        }
    }

    // Crisp outer border (highlights on hover)
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0_f32,
            if response.hovered() {
                Color32::from_rgba_unmultiplied(255, 255, 255, 72)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );

    response.on_hover_text(format!(
        "Logical processor {index} · {} load\nReported clock: {}\nPhysical core temperature: {}",
        if usage.is_finite() {
            format!("{usage:.1}%")
        } else {
            "unavailable".into()
        },
        mhz.map(clock_text).unwrap_or_else(|| "unavailable".into()),
        temperature
            .map(|temp| format!("{temp:.1} °C · hwmon/coretemp"))
            .unwrap_or_else(|| "unavailable".into()),
    ));
}

// ---------------------------------------------------------------- scalable multi-DIMM visualizer
/// Renders installed RAM slots cleanly for 0 (Unified), 1–4 (Visual DIMMs), or
/// 5–16+ (Channel Rack / Dense Table) memory configurations.
pub fn memory_slots_view(ui: &mut egui::Ui, memory: &MemoryProfile, compact: bool) {
    // Summary bar
    ui.horizontal_wrapped(|ui| {
        if let Some(total) = memory.total_bytes {
            chip(ui, &decimal_bytes(total), VIOLET);
        }
        if let (Some(used), Some(total)) = (memory.slots_used, memory.slots_total) {
            chip(
                ui,
                &format!("{used} / {total} SLOTS POPULATED"),
                if used < total { ACCENT } else { GREEN },
            );
        } else if !memory.modules.is_empty() {
            chip(ui, &format!("{} MODULES", memory.modules.len()), ACCENT);
        }
        if let Some(ecc) = memory.ecc {
            chip(
                ui,
                if ecc { "ECC ON" } else { "NON-ECC" },
                if ecc { GREEN } else { MUTED },
            );
        }
        if let Some(max) = memory.max_capacity_bytes {
            mono_badge(ui, &format!("Max {}", decimal_bytes(max)), MUTED);
        }
    });
    ui.add_space(8.0);

    if memory.modules.is_empty() {
        // Unified / soldered memory card (Apple Silicon / thin laptops / Linux standard user)
        let frame = egui::Frame::NONE
            .fill(INPUT)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::same(12));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                chip(ui, "UNIFIED / SOLDERED", VIOLET);
                ui.label(
                    RichText::new("No removable DIMM records exposed by the OS.")
                        .size(11.5)
                        .color(TEXT_DIM),
                );
            });
        });
        return;
    }

    let total_slots = memory
        .slots_total
        .map(|s| (s as usize).max(memory.modules.len()))
        .unwrap_or(memory.modules.len());

    if total_slots <= 4 && !compact {
        // Visual DIMM stick cards (including ghosted empty slots!)
        let cols = total_slots.clamp(1, 4);
        ui.columns(cols, |columns| {
            for (slot_idx, col) in columns.iter_mut().enumerate() {
                if let Some(m) = memory.modules.get(slot_idx) {
                    draw_dimm_card(col, m, slot_idx);
                } else {
                    draw_empty_dimm_card(col, slot_idx);
                }
            }
        });
    } else {
        // Scalable multi-DIMM rack table for 5–16+ slot workstations & servers (or compact mode)
        let frame = egui::Frame::NONE
            .fill(INPUT)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::same(8));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("dimm-rack-table")
                .max_height(210.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (idx, m) in memory.modules.iter().enumerate() {
                        let slot = if m.slot.is_empty() {
                            format!("DIMM {}", idx + 1)
                        } else {
                            m.slot.clone()
                        };
                        let cap = m
                            .size_bytes
                            .map(decimal_bytes)
                            .unwrap_or_else(|| "—".into());
                        let speed = m
                            .configured_mhz
                            .or(m.speed_mhz)
                            .map(|s| format!("{s} MT/s"))
                            .unwrap_or_else(|| "—".into());
                        ui.horizontal(|ui| {
                            mono_badge(ui, &truncate(&slot, 14), VIOLET);
                            ui.label(
                                RichText::new(cap)
                                    .monospace()
                                    .strong()
                                    .size(11.5)
                                    .color(TEXT),
                            );
                            if !m.kind.is_empty() {
                                ui.label(RichText::new(&m.kind).size(11.0).color(TEXT_DIM));
                            }
                            ui.label(RichText::new(speed).monospace().size(11.0).color(CYAN));
                            if !m.manufacturer.is_empty() {
                                ui.label(
                                    RichText::new(truncate(&m.manufacturer, 18))
                                        .size(10.5)
                                        .color(MUTED),
                                );
                            }
                        });
                        if idx + 1 < memory.modules.len() {
                            page_rule(ui);
                        }
                    }
                    let empty_count = total_slots.saturating_sub(memory.modules.len());
                    if empty_count > 0 {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("+ {empty_count} empty slot(s) available"))
                                .size(10.5)
                                .color(MUTED),
                        );
                    }
                });
        });
    }
}

fn draw_dimm_card(ui: &mut egui::Ui, m: &hardware_hub::specs::MemoryModule, index: usize) {
    let frame = egui::Frame::NONE
        .fill(INPUT)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // Stylized DIMM PCB illustration (dark emerald/slate strip with memory IC blocks)
        let (pcb, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), egui::Sense::hover());
        if ui.is_rect_visible(pcb) {
            let p = ui.painter();
            p.rect_filled(pcb, CornerRadius::same(4), Color32::from_rgb(26, 40, 36));
            p.rect_stroke(
                pcb,
                CornerRadius::same(4),
                Stroke::new(1.0_f32, Color32::from_rgb(44, 78, 66)),
                egui::StrokeKind::Inside,
            );
            // Draw 6 IC packages along the stick
            let chips = 6usize;
            let pad = 6.0;
            let gap = 4.0;
            let cw = ((pcb.width() - pad * 2.0 - gap * (chips - 1) as f32) / chips as f32).max(6.0);
            for c in 0..chips {
                let x = pcb.left() + pad + c as f32 * (cw + gap);
                let r = Rect::from_min_size(Pos2::new(x, pcb.top() + 5.0), Vec2::new(cw, 11.0));
                p.rect_filled(r, CornerRadius::same(2), Color32::from_rgb(46, 52, 62));
            }
            // Gold contact pins strip along bottom edge
            p.hline(
                egui::Rangef::new(pcb.left() + 6.0, pcb.right() - 6.0),
                pcb.bottom() - 2.0,
                Stroke::new(2.0_f32, Color32::from_rgb(196, 160, 72)),
            );
        }
        ui.add_space(6.0);
        let slot = if m.slot.is_empty() {
            format!("DIMM {}", index + 1)
        } else {
            truncate(&m.slot, 20)
        };
        let cap = m
            .size_bytes
            .map(decimal_bytes)
            .unwrap_or_else(|| "—".into());
        ui.horizontal(|ui| {
            ui.label(RichText::new(slot).size(11.0).strong().color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(cap)
                        .monospace()
                        .strong()
                        .size(11.5)
                        .color(VIOLET),
                );
            });
        });
        let mut line = Vec::new();
        if !m.kind.is_empty() {
            line.push(m.kind.clone());
        }
        if let Some(spd) = m.configured_mhz.or(m.speed_mhz) {
            line.push(format!("{spd} MT/s"));
        }
        if !m.manufacturer.is_empty() {
            line.push(truncate(&m.manufacturer, 14));
        }
        ui.label(
            RichText::new(if line.is_empty() {
                "Installed".into()
            } else {
                line.join(" · ")
            })
            .size(10.0)
            .color(TEXT_DIM),
        );
        if !m.part_number.is_empty() {
            ui.label(
                RichText::new(truncate(&m.part_number, 24))
                    .monospace()
                    .size(9.5)
                    .color(MUTED),
            );
        }
    });
}

fn draw_empty_dimm_card(ui: &mut egui::Ui, index: usize) {
    let frame = egui::Frame::NONE
        .fill(Color32::from_rgba_unmultiplied(20, 22, 27, 160))
        .stroke(Stroke::new(1.0_f32, HAIRLINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (pcb, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), egui::Sense::hover());
        if ui.is_rect_visible(pcb) {
            ui.painter().rect_stroke(
                pcb,
                CornerRadius::same(4),
                Stroke::new(1.0_f32, BORDER),
                egui::StrokeKind::Inside,
            );
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("Slot {}", index + 1))
                .size(11.0)
                .color(MUTED),
        );
        ui.label(
            RichText::new("Empty slot")
                .size(10.0)
                .italics()
                .color(MUTED),
        );
        ui.add_space(12.0);
    });
}

// ---------------------------------------------------------------- HWMonitor sensor table
pub struct RowView<'a> {
    pub label: &'a str,
    pub key: &'a str,
    pub source: &'a str,
    pub current: Option<f32>,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub average: Option<f32>,
    pub unit: &'a str,
}

impl<'a> From<&'a Temperature> for RowView<'a> {
    fn from(sensor: &'a Temperature) -> Self {
        Self {
            label: &sensor.label,
            key: &sensor.key,
            source: sensor.source,
            current: sensor.celsius,
            min: sensor.min,
            max: sensor.max,
            average: sensor.average,
            unit: "°C",
        }
    }
}

impl<'a> From<&'a Fan> for RowView<'a> {
    fn from(sensor: &'a Fan) -> Self {
        Self {
            label: &sensor.label,
            key: &sensor.key,
            source: sensor.source,
            current: sensor.rpm,
            min: sensor.min,
            max: sensor.max,
            average: None,
            unit: "RPM",
        }
    }
}

pub struct SensorTableOutcome {
    pub clicked_key: Option<String>,
    pub toggled_pin: Option<String>,
}

/// Modern HWMonitor-style Sensor Table with visual Min–Current–Max thermal range
/// bars, provenance badges (`SMC`/`ACPI`/`hwmon`), click-to-inspect selection,
/// and Pin-to-HUD toggles.
pub fn sensor_table(
    ui: &mut egui::Ui,
    id: &str,
    heading: &str,
    rows: &[RowView<'_>],
    selected_key: Option<&str>,
    pinned: &[String],
    pro_density: bool,
) -> SensorTableOutcome {
    let mut outcome = SensorTableOutcome {
        clicked_key: None,
        toggled_pin: None,
    };
    if !heading.is_empty() {
        ui.label(RichText::new(heading).size(11.0).strong().color(TEXT_DIM));
        ui.add_space(4.0);
    }
    ui.push_id(id, |ui| {
        let large_table = rows.len() > 18;
        let width = (ui.available_width() - if large_table { 12.0 } else { 0.0 }).max(260.0);
        let show_range_bar = width >= 460.0;
        let show_minmax = width >= 270.0;
        let show_average = pro_density && width >= 286.0 && rows.iter().any(|row| row.unit == "°C");
        let pin_w = 24.0;
        let stat_w = if width >= 430.0 { 44.0 } else { 32.0 };
        let minmax_w = if show_minmax { stat_w * 2.0 } else { 0.0 };
        let average_w = if show_average { stat_w } else { 0.0 };
        let stats_w = minmax_w + average_w;
        let bar_w = if show_range_bar { 98.0 } else { 0.0 };
        let cur_w = 68.0;

        let (header, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), egui::Sense::hover());
        let right = header.right() - pin_w;
        if ui.is_rect_visible(header) {
            let painter = ui.painter();
            let font = egui::FontId::proportional(9.5);
            painter.text(
                Pos2::new(header.left() + 6.0, header.center().y),
                egui::Align2::LEFT_CENTER,
                "SENSOR",
                font.clone(),
                MUTED,
            );
            if show_minmax {
                painter.text(
                    Pos2::new(right - average_w - 4.0, header.center().y),
                    egui::Align2::RIGHT_CENTER,
                    "MAX",
                    font.clone(),
                    MUTED,
                );
                painter.text(
                    Pos2::new(right - average_w - stat_w - 4.0, header.center().y),
                    egui::Align2::RIGHT_CENTER,
                    "MIN",
                    font.clone(),
                    MUTED,
                );
            }
            if show_average {
                painter.text(
                    Pos2::new(right - 4.0, header.center().y),
                    egui::Align2::RIGHT_CENTER,
                    "AVG",
                    font.clone(),
                    MUTED,
                );
            }
            if show_range_bar {
                painter.text(
                    Pos2::new(right - stats_w - bar_w * 0.5 - 6.0, header.center().y),
                    egui::Align2::CENTER_CENTER,
                    "RANGE",
                    font.clone(),
                    MUTED,
                );
            }
            painter.text(
                Pos2::new(right - stats_w - bar_w - 10.0, header.center().y),
                egui::Align2::RIGHT_CENTER,
                "CURRENT",
                font,
                MUTED,
            );
        }

        let row_h = if pro_density { 25.0 } else { 30.0 };
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut draw_row = |ui: &mut egui::Ui, index: usize| {
            let row = &rows[index];
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, row_h), egui::Sense::hover());
            if !ui.is_rect_visible(rect) {
                return;
            }
            let is_selected = selected_key == Some(row.key);
            let is_pinned = pinned.iter().any(|k| k == row.key);

            // Pin hit region on far right
            let pin_rect =
                Rect::from_min_max(Pos2::new(rect.right() - pin_w, rect.top()), rect.max);
            let label_rect =
                Rect::from_min_max(rect.min, Pos2::new(pin_rect.left(), rect.bottom()));
            let response = ui.interact(
                label_rect,
                ui.make_persistent_id((row.key, "plot")),
                egui::Sense::click(),
            );
            let pin = ui.interact(
                pin_rect,
                ui.make_persistent_id((row.key, "pin")),
                egui::Sense::click(),
            );
            let pin_label = if is_pinned {
                "Unpin from HUD"
            } else {
                "Pin to HUD"
            };
            pin.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Checkbox,
                    ui.is_enabled(),
                    is_pinned,
                    format!("{pin_label}: {}", row.label),
                )
            });
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), row.label)
            });
            if pin.clicked() {
                outcome.toggled_pin = Some(row.key.to_owned());
            } else if response.clicked() {
                outcome.clicked_key = Some(row.key.to_owned());
            }

            let painter = ui.painter();
            if is_selected {
                painter.rect_filled(rect, CornerRadius::same(6), tint_alpha(ACCENT, 34));
                painter.rect_filled(
                    Rect::from_min_size(
                        rect.left_top() + Vec2::new(1.0, 4.0),
                        Vec2::new(2.5, rect.height() - 8.0),
                    ),
                    CornerRadius::same(2),
                    ACCENT,
                );
            } else if response.hovered() || response.has_focus() {
                painter.rect_filled(
                    rect,
                    CornerRadius::same(6),
                    Color32::from_rgba_unmultiplied(255, 255, 255, 11),
                );
            }

            let label_max_w = (width - pin_w - stats_w - bar_w - cur_w - 20.0).max(52.0);
            let label_galley = elided(
                ui,
                row.label,
                11.5,
                if is_selected { TEXT } else { TEXT_DIM },
                if pro_density && !row.source.is_empty() {
                    (label_max_w - 46.0).max(56.0)
                } else {
                    label_max_w
                },
            );
            let lx = rect.left() + 8.0;
            painter.galley(
                Pos2::new(lx, rect.center().y - label_galley.size().y / 2.0),
                Arc::clone(&label_galley),
                if is_selected { TEXT } else { TEXT_DIM },
            );
            if pro_density && !row.source.is_empty() && label_max_w > 110.0 {
                let badge_x = lx + label_galley.size().x + 6.0;
                let src_galley = painter.layout_no_wrap(
                    row.source.to_owned(),
                    egui::FontId::monospace(8.5),
                    MUTED,
                );
                let br = Rect::from_min_size(
                    Pos2::new(badge_x, rect.center().y - 7.0),
                    Vec2::new(src_galley.size().x + 8.0, 14.0),
                );
                if br.right() < rect.left() + label_max_w {
                    painter.rect_filled(br, CornerRadius::same(3), INPUT);
                    painter.rect_stroke(
                        br,
                        CornerRadius::same(3),
                        Stroke::new(1.0_f32, BORDER),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(
                        Pos2::new(br.left() + 4.0, br.center().y - src_galley.size().y / 2.0),
                        src_galley,
                        MUTED,
                    );
                }
            }

            let current_text = row
                .current
                .map(|v| format!("{v:.0} {}", row.unit))
                .unwrap_or_else(|| "—".into());
            let current_color = match row.current {
                Some(v) if row.unit == "°C" => temperature_color(v),
                Some(_) => TEXT,
                None => MUTED,
            };
            painter.text(
                Pos2::new(right - stats_w - bar_w - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                current_text,
                egui::FontId::monospace(11.5),
                current_color,
            );

            // Visual Min–Current–Max thermal/fan range bar
            if show_range_bar {
                let bar_rect = Rect::from_min_size(
                    Pos2::new(right - stats_w - bar_w - 4.0, rect.center().y - 3.0),
                    Vec2::new(bar_w - 8.0, 6.0),
                );
                draw_thermal_range_bar(painter, bar_rect, row.current, row.min, row.max, row.unit);
            }

            for (visible, offset, val) in [
                (show_minmax, average_w + stat_w + 4.0, row.min),
                (show_minmax, average_w + 4.0, row.max),
                (show_average, 4.0, row.average),
            ] {
                if visible {
                    painter.text(
                        Pos2::new(right - offset, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        val.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into()),
                        egui::FontId::monospace(10.5),
                        MUTED,
                    );
                }
            }

            // Pin icon (dot / ring) on far right
            let pin_c = pin_rect.center();
            if is_pinned {
                painter.circle_filled(pin_c, 3.6, ACCENT);
                painter.circle_stroke(pin_c, 5.5, Stroke::new(1.0_f32, tint_alpha(ACCENT, 130)));
            } else if pin.hovered() || pin.has_focus() || response.hovered() {
                painter.circle_stroke(pin_c, 3.6, Stroke::new(1.2_f32, MUTED));
            }
            pin.on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(pin_label);

            if index + 1 < rows.len() {
                painter.hline(
                    egui::Rangef::new(rect.left() + 6.0, rect.right() - 6.0),
                    rect.bottom(),
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 10)),
                );
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            response.on_hover_ui(|ui| {
                let value = |v: Option<f32>| {
                    v.map(|v| format!("{v:.1} {}", row.unit))
                        .unwrap_or_else(|| "Unavailable".into())
                };
                ui.label(format!(
                    "{} ({})\n{}\nCurrent {} · Min {} · Max {} · Avg {}",
                    row.label,
                    row.source,
                    row.key,
                    value(row.current),
                    value(row.min),
                    value(row.max),
                    value(row.average)
                ));
            });
        };
        if large_table {
            egui::ScrollArea::vertical()
                .id_salt("rows")
                .max_height(row_h * 12.0)
                .show_rows(ui, row_h, rows.len(), |ui, visible| {
                    for i in visible {
                        draw_row(ui, i);
                    }
                });
        } else {
            for i in 0..rows.len() {
                draw_row(ui, i);
            }
        }
    });
    outcome
}

fn draw_thermal_range_bar(
    painter: &egui::Painter,
    rect: Rect,
    current: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
    unit: &str,
) {
    painter.rect_filled(rect, CornerRadius::same(3), TRACK);
    let is_temp = unit == "°C";
    let is_pct = unit == "%";
    let (scale_lo, scale_hi) = if is_temp {
        (20.0_f32, 105.0_f32)
    } else if is_pct {
        (0.0_f32, 100.0_f32)
    } else {
        (0.0_f32, 5000.0_f32)
    };
    let norm = |v: f32| ((v - scale_lo) / (scale_hi - scale_lo)).clamp(0.0, 1.0);

    // Draw observed session min..max band
    if let (Some(lo), Some(hi)) = (min, max) {
        let x0 = rect.left() + rect.width() * norm(lo);
        let x1 = (rect.left() + rect.width() * norm(hi)).max(x0 + 4.0);
        let span = Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, rect.bottom()));
        let c_lo = if is_temp {
            temperature_color(lo)
        } else if is_pct {
            load_color(lo / 100.0)
        } else {
            ACCENT
        };
        let c_hi = if is_temp {
            temperature_color(hi)
        } else if is_pct {
            load_color(hi / 100.0)
        } else {
            CYAN
        };
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(span.left_top(), c_lo);
        mesh.colored_vertex(span.left_bottom(), c_lo);
        mesh.colored_vertex(span.right_top(), c_hi);
        mesh.colored_vertex(span.right_bottom(), c_hi);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(2, 1, 3);
        painter.add(egui::Shape::mesh(mesh));
    }
    // Crisp white current-value marker dot
    if let Some(cur) = current {
        let cx = rect.left() + rect.width() * norm(cur);
        painter.circle_filled(Pos2::new(cx, rect.center().y), 3.4, TEXT);
    }
}

// ---------------------------------------------------------------- spec & virtualized tables
pub fn spec_row(
    rows: &mut Vec<(String, String)>,
    label: impl Into<String>,
    value: impl Into<String>,
) {
    let value = value.into();
    if !value.trim().is_empty() {
        rows.push((label.into(), value));
    }
}

pub fn spec_grid(ui: &mut egui::Ui, id: &str, rows: &[(String, String)]) {
    if rows.is_empty() {
        return;
    }
    ui.push_id(id, |ui| {
        let width = ui.available_width();
        let label_w = rows
            .iter()
            .map(|(label, _)| {
                ui.fonts(|fonts| {
                    fonts
                        .layout_no_wrap(label.clone(), egui::FontId::proportional(11.5), MUTED)
                        .size()
                        .x
                })
            })
            .fold(0.0_f32, f32::max)
            .max(120.0)
            .min(width * 0.46)
            + 20.0;
        let row_h = 23.0;
        for (index, (label, value)) in rows.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, row_h), egui::Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            if index % 2 == 1 {
                painter.rect_filled(
                    rect.expand2(Vec2::new(6.0, 0.5)),
                    CornerRadius::same(6),
                    STRIPE,
                );
            }
            let label_galley = elided(ui, label, 11.5, MUTED, label_w - 22.0);
            painter.galley(
                Pos2::new(
                    rect.left() + 6.0,
                    rect.center().y - label_galley.size().y / 2.0,
                ),
                label_galley,
                MUTED,
            );
            let value_galley = elided(ui, value, 11.5, TEXT_DIM, width - label_w - 6.0);
            painter.galley(
                Pos2::new(
                    rect.left() + label_w,
                    rect.center().y - value_galley.size().y / 2.0,
                ),
                value_galley,
                TEXT_DIM,
            );
        }
    });
}

/// Virtualized driver store table (`ScrollArea::show_rows`) for instant 60fps scrolling.
pub fn driver_table(ui: &mut egui::Ui, drivers: &[&DriverSpec]) {
    let width = ui.available_width();
    let class_w = 114.0;
    let version_w = 112.0;
    let sign_w = 82.0;
    let name_w = (width - class_w - version_w - sign_w).max(140.0);
    let (header, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), egui::Sense::hover());
    if ui.is_rect_visible(header) {
        let painter = ui.painter();
        let font = egui::FontId::proportional(9.5);
        for (x, text) in [
            (header.left() + 8.0, "DRIVER"),
            (header.left() + name_w, "CLASS"),
            (header.left() + name_w + class_w, "VERSION"),
            (header.right() - sign_w + 8.0, "SIGNATURE"),
        ] {
            painter.text(
                Pos2::new(x, header.center().y),
                egui::Align2::LEFT_CENTER,
                text,
                font.clone(),
                MUTED,
            );
        }
        painter.hline(
            header.x_range(),
            header.bottom() - 0.5,
            Stroke::new(1.0_f32, HAIRLINE),
        );
    }
    egui::ScrollArea::vertical()
        .id_salt("virtual-driver-table")
        .max_height(420.0)
        .auto_shrink([false, true])
        .show_rows(ui, 40.0, drivers.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                let driver = drivers[index];
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, 40.0), egui::Sense::hover());
                let painter = ui.painter();
                if response.hovered() {
                    painter.rect_filled(
                        rect,
                        CornerRadius::same(6),
                        Color32::from_rgba_unmultiplied(255, 255, 255, 12),
                    );
                } else if index % 2 == 1 {
                    painter.rect_filled(rect, CornerRadius::same(6), STRIPE);
                }
                let name = elided(ui, &driver.name, 11.5, TEXT, name_w - 18.0);
                painter.galley(Pos2::new(rect.left() + 8.0, rect.top() + 6.0), name, TEXT);
                let mut meta = Vec::new();
                if !driver.provider.is_empty() {
                    meta.push(driver.provider.as_str());
                }
                if !driver.date.is_empty() {
                    meta.push(driver.date.as_str());
                }
                let meta_g = elided(ui, &meta.join(" · "), 10.0, MUTED, name_w - 18.0);
                painter.galley(
                    Pos2::new(rect.left() + 8.0, rect.top() + 23.0),
                    meta_g,
                    MUTED,
                );
                painter.text(
                    Pos2::new(rect.left() + name_w, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    driver_class_label(&driver.class_name),
                    egui::FontId::proportional(11.0),
                    TEXT_DIM,
                );
                painter.text(
                    Pos2::new(rect.left() + name_w + class_w, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    if driver.version.is_empty() {
                        "—"
                    } else {
                        &driver.version
                    },
                    egui::FontId::monospace(10.5),
                    TEXT_DIM,
                );
                if let Some(signed) = driver.signed {
                    let (lbl, col) = if signed {
                        ("SIGNED", GREEN)
                    } else {
                        ("UNSIGNED", AMBER)
                    };
                    let g = ui.fonts(|f| {
                        f.layout_no_wrap(lbl.to_owned(), egui::FontId::proportional(9.5), col)
                    });
                    let cr = Rect::from_min_size(
                        Pos2::new(
                            rect.left() + name_w + class_w + version_w,
                            rect.center().y - 8.0,
                        ),
                        g.size() + Vec2::new(12.0, 6.0),
                    );
                    painter.rect_filled(cr, CornerRadius::same(5), tint(col));
                    painter.galley(cr.min + Vec2::new(6.0, 3.0), g, col);
                }
            }
        });
}

/// Virtualized live process table (`ScrollArea::show_rows`) handling 500+ processes effortlessly.
pub fn process_table(ui: &mut egui::Ui, processes: &[&Process]) {
    let width = ui.available_width();
    let pid_w = 72.0;
    let cpu_w = 72.0;
    let mem_w = 90.0;
    let name_w = (width - pid_w - cpu_w - mem_w).max(110.0);
    let (header, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), egui::Sense::hover());
    if ui.is_rect_visible(header) {
        let painter = ui.painter();
        let font = egui::FontId::proportional(9.5);
        painter.text(
            Pos2::new(header.left() + 8.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            "PROCESS",
            font.clone(),
            MUTED,
        );
        painter.text(
            Pos2::new(header.left() + name_w + pid_w - 8.0, header.center().y),
            egui::Align2::RIGHT_CENTER,
            "PID",
            font.clone(),
            MUTED,
        );
        painter.text(
            Pos2::new(
                header.left() + name_w + pid_w + cpu_w - 8.0,
                header.center().y,
            ),
            egui::Align2::RIGHT_CENTER,
            "CPU",
            font.clone(),
            MUTED,
        );
        painter.text(
            Pos2::new(header.right() - 8.0, header.center().y),
            egui::Align2::RIGHT_CENTER,
            "MEMORY",
            font,
            MUTED,
        );
        painter.hline(
            header.x_range(),
            header.bottom() - 0.5,
            Stroke::new(1.0_f32, HAIRLINE),
        );
    }
    egui::ScrollArea::vertical()
        .id_salt("virtual-process-table")
        .max_height(440.0)
        .auto_shrink([false, true])
        .show_rows(ui, 26.0, processes.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                let process = processes[index];
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, 26.0), egui::Sense::hover());
                let painter = ui.painter();
                if response.hovered() {
                    painter.rect_filled(
                        rect,
                        CornerRadius::same(6),
                        Color32::from_rgba_unmultiplied(255, 255, 255, 12),
                    );
                } else if index % 2 == 1 {
                    painter.rect_filled(rect, CornerRadius::same(6), STRIPE);
                }
                let name = elided(ui, &process.name, 11.5, TEXT_DIM, name_w - 14.0);
                painter.galley(
                    Pos2::new(rect.left() + 8.0, rect.center().y - name.size().y / 2.0),
                    name,
                    TEXT_DIM,
                );
                painter.text(
                    Pos2::new(rect.left() + name_w + pid_w - 8.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    process.pid.to_string(),
                    egui::FontId::monospace(10.5),
                    MUTED,
                );
                let cpu_text = if process.cpu_usage.is_finite() {
                    format!("{:.1}%", process.cpu_usage)
                } else {
                    "—".into()
                };
                painter.text(
                    Pos2::new(rect.left() + name_w + pid_w + cpu_w - 8.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    cpu_text,
                    egui::FontId::monospace(10.5),
                    if process.cpu_usage > 50.0 {
                        AMBER
                    } else {
                        TEXT_DIM
                    },
                );
                painter.text(
                    Pos2::new(rect.right() - 8.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{:.1} MiB", process.memory as f64 / 1_048_576.0),
                    egui::FontId::monospace(10.5),
                    MUTED,
                );
            }
        });
}

// ---------------------------------------------------------------- stress & telemetry charts
pub fn mini_temperature_graph(
    ui: &mut egui::Ui,
    history: &History,
    limit_c: Option<f32>,
    height: f32,
) {
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
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(8), INPUT);
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    if values.len() < 2 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Waiting for temperature telemetry…",
            egui::FontId::proportional(10.5),
            MUTED,
        );
        return;
    }
    let mut min = values
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min)
        .min(30.0);
    let mut max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if let Some(lim) = limit_c {
        max = max.max(lim as f64 + 4.0);
    }
    if (max - min) < 10.0 {
        min -= 5.0;
        max += 5.0;
    }
    let span = (max - min).max(1.0);
    let pad_x = 10.0;
    let pad_top = 22.0;
    let pad_bottom = 8.0;
    let inner_w = (rect.width() - pad_x * 2.0).max(10.0);
    let inner_h = (rect.height() - pad_top - pad_bottom).max(10.0);
    let y_for = |val: f64| rect.bottom() - pad_bottom - inner_h * ((val - min) / span) as f32;

    // Dashed amber advisory threshold line
    if let Some(lim) = limit_c {
        let ly = y_for(lim as f64).clamp(rect.top() + pad_top, rect.bottom() - pad_bottom);
        let mut x = rect.left() + pad_x;
        while x < rect.right() - pad_x {
            let x2 = (x + 6.0).min(rect.right() - pad_x);
            painter.line_segment(
                [Pos2::new(x, ly), Pos2::new(x2, ly)],
                Stroke::new(1.1_f32, tint_alpha(AMBER, 175)),
            );
            x += 11.0;
        }
        painter.text(
            Pos2::new(rect.right() - 8.0, ly - 3.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{lim:.0}°C limit"),
            egui::FontId::monospace(9.0),
            AMBER,
        );
    }

    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + pad_x + inner_w * i as f32 / (values.len() - 1) as f32;
            Pos2::new(x, y_for(*v))
        })
        .collect();
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], Stroke::new(2.0_f32, CYAN));
    }
    let cur_max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    painter.text(
        rect.left_top() + Vec2::new(9.0, 5.0),
        egui::Align2::LEFT_TOP,
        format!("Peak {cur_max:.0} °C"),
        egui::FontId::monospace(9.5),
        TEXT_DIM,
    );
}

pub fn error_details(ui: &mut egui::Ui, errors: &[ErrorDetail], total: u64) {
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
            ui.label(
                RichText::new(&error.location)
                    .monospace()
                    .size(10.0)
                    .color(TEXT_DIM),
            );
            ui.label(
                RichText::new(format!("exp {} / got {}", error.expected, error.actual))
                    .monospace()
                    .size(10.0)
                    .color(MUTED),
            );
        });
    }
    if total > errors.len() as u64 {
        ui.weak(format!("+ {} more not shown", total - errors.len() as u64));
    }
}

pub fn comparison_row(ui: &mut egui::Ui, label: &str, a: &str, b: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.5).color(MUTED));
        ui.add_space(4.0);
        ui.label(RichText::new(a).monospace().size(11.0).strong().color(TEXT));
        ui.add_space(8.0);
        ui.label(RichText::new("vs").size(9.5).color(MUTED));
        ui.add_space(8.0);
        ui.label(RichText::new(b).monospace().size(11.0).strong().color(TEXT));
    });
}

pub fn config_summary(config: &Config) -> String {
    let mode = match config.run_mode {
        RunMode::Timed => hardware_hub::stress::format_duration(config.seconds),
        RunMode::Continuous => "continuous".into(),
        RunMode::Passes => format!("{} passes", config.target_passes),
    };
    match config.kind {
        Kind::Cpu => format!(
            "{} workers · {} · {mode}",
            config.workers,
            config.cpu_workload.label()
        ),
        Kind::Gpu => format!(
            "GPU {} · {} · {mode}",
            config.gpu_adapter.as_deref().unwrap_or("not selected"),
            config.gpu_intensity.label()
        ),
        Kind::Memory => format!(
            "{} MiB · {} · {mode}",
            config.mib,
            config.mem_pattern.label()
        ),
        Kind::Disk => format!(
            "{} MiB file ({} MiB cap) · {mode}",
            config.mib, config.disk_write_limit_mib
        ),
        Kind::Combined => format!("{} workers + {} MiB · {mode}", config.workers, config.mib),
    }
}

pub fn sensor_unavailable_note() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "This firmware reported no ACPI thermal zone. Windows exposes temperature only through ACPI thermal zones without a kernel driver."
    }
    #[cfg(target_os = "macos")]
    {
        "No AppleSMC sensor provider responded on this Mac. Usage, memory, storage and inventory still work."
    }
    #[cfg(target_os = "linux")]
    {
        "No /sys/class/hwmon temperature sensor was exposed by the kernel on this host."
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        "No sensor provider is available on this platform."
    }
}

// ---------------------------------------------------------------- vector icons
pub fn draw_workspace_icon(painter: &egui::Painter, center: Pos2, ws: Workspace, color: Color32) {
    let stroke = Stroke::new(1.5_f32, color);
    let faint = Stroke::new(1.1_f32, tint_alpha(color, 145));
    match ws {
        Workspace::Monitor => {
            // Pulse / telemetry waveform inside a clean frame
            let r = Rect::from_center_size(center, Vec2::new(16.0, 14.0));
            painter.rect_stroke(r, CornerRadius::same(3), stroke, egui::StrokeKind::Inside);
            let pts = vec![
                center + Vec2::new(-5.5, 1.0),
                center + Vec2::new(-2.5, 1.0),
                center + Vec2::new(-1.0, -3.5),
                center + Vec2::new(1.2, 3.5),
                center + Vec2::new(2.8, 0.0),
                center + Vec2::new(5.5, 0.0),
            ];
            painter.add(egui::Shape::line(pts, stroke));
        }
        Workspace::Hardware => {
            // Processor chip icon
            let r = Rect::from_center_size(center, Vec2::new(12.0, 12.0));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            painter.rect_filled(
                Rect::from_center_size(center, Vec2::new(4.5, 4.5)),
                CornerRadius::same(1),
                color,
            );
            for d in [-3.5, 0.0, 3.5] {
                painter.line_segment(
                    [center + Vec2::new(d, -8.5), center + Vec2::new(d, -6.2)],
                    faint,
                );
                painter.line_segment(
                    [center + Vec2::new(d, 6.2), center + Vec2::new(d, 8.5)],
                    faint,
                );
                painter.line_segment(
                    [center + Vec2::new(-8.5, d), center + Vec2::new(-6.2, d)],
                    faint,
                );
                painter.line_segment(
                    [center + Vec2::new(6.2, d), center + Vec2::new(8.5, d)],
                    faint,
                );
            }
        }
        Workspace::Stress => {
            // Speedometer / stability gauge icon
            painter.circle_stroke(center, 7.5, stroke);
            painter.line_segment([center, center + Vec2::new(4.2, -3.8)], stroke);
            painter.circle_filled(center, 1.8, color);
        }
        Workspace::Diagnostics => {
            // Document / report lines icon
            let r = Rect::from_center_size(center, Vec2::new(13.0, 15.0));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            for dy in [-3.2, 0.0, 3.2] {
                painter.line_segment(
                    [center + Vec2::new(-3.5, dy), center + Vec2::new(3.5, dy)],
                    faint,
                );
            }
        }
    }
}

pub fn draw_nav_icon(painter: &egui::Painter, center: Pos2, page: Page, color: Color32) {
    let stroke = Stroke::new(1.3_f32, color);
    let faint = Stroke::new(1.0_f32, tint_alpha(color, 130));
    let line = |a: Pos2, b: Pos2, s: Stroke| painter.line_segment([a, b], s);
    let box_rect = Rect::from_center_size(center, Vec2::new(12.0, 12.0));
    match page {
        Page::Overview => {
            for (dx, dy) in [(-2.8, -2.8), (2.8, -2.8), (-2.8, 2.8), (2.8, 2.8)] {
                painter.rect_filled(
                    Rect::from_center_size(center + Vec2::new(dx, dy), Vec2::new(4.0, 4.0)),
                    CornerRadius::same(1),
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
                Rect::from_center_size(center, Vec2::new(5.0, 5.0)),
                CornerRadius::same(1),
                faint,
                egui::StrokeKind::Inside,
            );
        }
        Page::Gpu => {
            painter.rect_stroke(
                box_rect,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(center, 3.0, faint);
        }
        Page::Memory => {
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::new(14.0, 8.0)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for dx in [-3.5, 0.0, 3.5] {
                line(
                    center + Vec2::new(dx, 4.0),
                    center + Vec2::new(dx, 6.2),
                    faint,
                );
            }
        }
        Page::Storage => {
            for dy in [-3.5, 0.0, 3.5] {
                painter.rect_stroke(
                    Rect::from_center_size(center + Vec2::new(0.0, dy), Vec2::new(13.0, 2.8)),
                    CornerRadius::same(1),
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        Page::Thermal => {
            line(
                center + Vec2::new(-1.5, -6.5),
                center + Vec2::new(-1.5, 1.5),
                stroke,
            );
            painter.circle_filled(center + Vec2::new(-1.5, 3.5), 2.8, color);
            line(
                center + Vec2::new(2.0, -3.5),
                center + Vec2::new(4.5, -3.5),
                faint,
            );
            line(
                center + Vec2::new(2.0, 0.0),
                center + Vec2::new(4.5, 0.0),
                faint,
            );
        }
        Page::Motherboard => {
            painter.rect_stroke(
                box_rect,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(
                Rect::from_center_size(center + Vec2::new(-2.5, -2.5), Vec2::new(3.0, 3.0)),
                CornerRadius::same(1),
                color,
            );
            painter.rect_filled(
                Rect::from_center_size(center + Vec2::new(2.5, 2.5), Vec2::new(3.0, 3.0)),
                CornerRadius::same(1),
                color,
            );
        }
        Page::Drivers => {
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::new(8.5, 8.5)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for dy in [-3.0, 3.0] {
                line(
                    center + Vec2::new(-6.5, dy),
                    center + Vec2::new(-4.5, dy),
                    faint,
                );
                line(
                    center + Vec2::new(4.5, dy),
                    center + Vec2::new(6.5, dy),
                    faint,
                );
            }
        }
        Page::Network => {
            painter.circle_filled(center, 1.6, color);
            painter.circle_stroke(center, 4.2, faint);
            painter.circle_stroke(center, 6.8, faint);
        }
        Page::Bluetooth => {
            for (a, b) in [
                (Vec2::new(-1.5, -5.5), Vec2::new(-1.5, 5.5)),
                (Vec2::new(-1.5, -5.5), Vec2::new(3.0, -1.0)),
                (Vec2::new(-1.5, 0.0), Vec2::new(3.0, -4.5)),
                (Vec2::new(-1.5, 0.0), Vec2::new(3.0, 4.5)),
                (Vec2::new(-1.5, 5.5), Vec2::new(3.0, 1.0)),
            ] {
                line(center + a, center + b, stroke);
            }
        }
        Page::Details => {
            painter.circle_stroke(center, 6.0, stroke);
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "i",
                egui::FontId::proportional(9.5),
                color,
            );
        }
        Page::Processes => {
            for dy in [-3.5, 0.0, 3.5] {
                painter.circle_filled(center + Vec2::new(-3.8, dy), 1.3, color);
                line(
                    center + Vec2::new(-1.0, dy),
                    center + Vec2::new(5.5, dy),
                    faint,
                );
            }
        }
        Page::Frames => {
            for (dx, h) in [(-3.6, 4.5), (0.0, 7.5), (3.6, 10.5)] {
                painter.rect_filled(
                    Rect::from_center_size(
                        center + Vec2::new(dx, 5.0 - h / 2.0),
                        Vec2::new(2.6, h),
                    ),
                    CornerRadius::same(1),
                    color,
                );
            }
        }
        Page::Tests => {
            painter.circle_stroke(center, 6.0, stroke);
            line(center, center + Vec2::new(3.2, -2.8), stroke);
        }
        Page::Reports => {
            painter.rect_stroke(
                box_rect,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            for dy in [-2.8, 0.0, 2.8] {
                line(
                    center + Vec2::new(-3.0, dy),
                    center + Vec2::new(3.5, dy),
                    faint,
                );
            }
        }
    }
}

/// Compute adaptive `(columns, tile_height, micro_mode)` for the liquid CPU
/// core grid so 4-core laptops, 24-thread hybrid CPUs, and 128-thread
/// workstations all stay crisp and readable in both Compact and Expanded modes.
pub fn core_grid_layout(count: usize, width: f32, compact: bool) -> (usize, f32, bool) {
    if compact {
        if count <= 16 {
            let columns = ((width + 7.0) / 80.0).floor() as usize;
            (columns.clamp(1, 4).min(count.max(1)), 52.0, false)
        } else {
            (8, 30.0, true)
        }
    } else if count <= 16 {
        let cols = ((width / 118.0).floor() as usize)
            .clamp(2, 8)
            .min(count.max(1));
        (cols, 58.0, false)
    } else if count <= 32 {
        let cols = ((width / 82.0).floor() as usize).clamp(4, 8);
        (cols, 46.0, false)
    } else {
        let cols = ((width / 46.0).floor() as usize).clamp(8, 16);
        (cols, 30.0, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liquid_core_grid_scales_from_4_to_128_threads() {
        // 4T laptop in expanded mode: 4 full liquid tiles
        let (c4, h4, m4) = core_grid_layout(4, 600.0, false);
        assert_eq!((c4, h4, m4), (4, 58.0, false));

        // 16T desktop: full liquid tiles
        let (c16, h16, m16) = core_grid_layout(16, 800.0, false);
        assert!((4..=8).contains(&c16));
        assert_eq!(h16, 58.0);
        assert!(!m16);

        // 24T hybrid CPU: medium tiles, non-micro
        let (c24, h24, m24) = core_grid_layout(24, 700.0, false);
        assert!((4..=8).contains(&c24));
        assert_eq!(h24, 46.0);
        assert!(!m24);

        // 128T Threadripper/EPYC: high-density micro-tile heatmap
        let (c128, h128, m128) = core_grid_layout(128, 800.0, false);
        assert!((8..=16).contains(&c128));
        assert_eq!(h128, 30.0);
        assert!(m128);

        // Compact HWMonitor mode (~380px): 4 cols for <=16T, 8 micro cols for >16T
        assert_eq!(core_grid_layout(8, 360.0, true), (4, 52.0, false));
        assert_eq!(core_grid_layout(16, 270.0, true), (3, 52.0, false));
        assert_eq!(core_grid_layout(32, 360.0, true), (8, 30.0, true));
    }

    #[test]
    fn thread_load_distinguishes_idle_from_small_or_missing_values() {
        assert_eq!(thread_load_text(0.0), "0%");
        assert_eq!(thread_load_text(0.2), "<1%");
        assert_eq!(thread_load_text(100.0), "100%");
        assert_eq!(thread_load_text(f32::NAN), "—");
    }
}
