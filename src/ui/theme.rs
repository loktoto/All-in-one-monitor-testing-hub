//! Obsidian Graphite design tokens, theme installation, formatting helpers, and
//! native Windows dark titlebar integration.

use eframe::egui::{self, Color32, CornerRadius, Stroke};
use hardware_hub::stress::Status;

// ---------------------------------------------------------------- surface tokens
// Obsidian Graphite dark hierarchy: canvas < rail/chrome < card < elevated hover.
pub const BG: Color32 = Color32::from_rgb(17, 18, 21);
pub const RAIL: Color32 = Color32::from_rgb(18, 20, 25);
pub const PANEL: Color32 = Color32::from_rgb(22, 24, 29);
pub const CARD: Color32 = Color32::from_rgb(28, 31, 38);
pub const CARD_HI: Color32 = Color32::from_rgb(37, 42, 52);
pub const BORDER: Color32 = Color32::from_rgb(42, 46, 57);
pub const HAIRLINE: Color32 = Color32::from_rgb(35, 39, 49);
pub const INPUT: Color32 = Color32::from_rgb(20, 22, 27);
pub const TRACK: Color32 = Color32::from_rgb(48, 53, 66);

// ---------------------------------------------------------------- text tokens
pub const TEXT: Color32 = Color32::from_rgb(243, 244, 246);
pub const TEXT_DIM: Color32 = Color32::from_rgb(180, 186, 198);
pub const MUTED: Color32 = Color32::from_rgb(134, 142, 158);

// ---------------------------------------------------------------- semantic palette
pub const ACCENT: Color32 = Color32::from_rgb(47, 129, 247);
pub const CYAN: Color32 = Color32::from_rgb(34, 211, 238);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(24, 52, 92);
pub const ACCENT_INK: Color32 = Color32::from_rgb(245, 249, 255);
pub const ACTION_INK: Color32 = Color32::from_rgb(11, 14, 20);
pub const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
pub const AMBER: Color32 = Color32::from_rgb(255, 159, 10);
pub const RED: Color32 = Color32::from_rgb(255, 69, 58);
pub const VIOLET: Color32 = Color32::from_rgb(163, 113, 247);

/// Subtle rounded row tint.
pub const STRIPE: Color32 = Color32::from_rgba_premultiplied(8, 9, 12, 10);

// ---------------------------------------------------------------- motion tokens
pub const MOTION_FAST: f32 = 0.11;
pub const MOTION_MEDIUM: f32 = 0.18;
pub const MOTION_SLOW: f32 = 0.24;

// ---------------------------------------------------------------- typography ramp
pub const TYPE_OVERLINE: f32 = 10.0;
pub const TYPE_TITLE: f32 = 22.0;
pub const TYPE_SUBTITLE: f32 = 12.0;
pub const TYPE_BODY: f32 = 12.5;

pub fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    let visuals = &mut style.visuals;
    *visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = INPUT;
    visuals.faint_bg_color = HAIRLINE;
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, HAIRLINE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    visuals.widgets.inactive.weak_bg_fill = CARD_HI;
    visuals.widgets.inactive.bg_fill = TRACK;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(7);
    visuals.widgets.hovered.weak_bg_fill = CARD_HI;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(56, 62, 76);
    visuals.widgets.hovered.bg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 58));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(7);
    visuals.widgets.active.weak_bg_fill = ACCENT_SOFT;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT_INK);
    visuals.widgets.active.corner_radius = CornerRadius::same(7);
    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::proportional(19.0)),
        (egui::TextStyle::Body, egui::FontId::proportional(TYPE_BODY)),
        (egui::TextStyle::Button, egui::FontId::proportional(12.0)),
        (egui::TextStyle::Small, egui::FontId::proportional(10.5)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(11.5)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(11.0, 5.5);
    style.spacing.slider_width = 160.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.interact_size.y = 25.0;
    style.spacing.icon_width = 15.0;
    style.spacing.icon_width_inner = 8.0;
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.bar_inner_margin = 2.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.spacing.menu_margin = egui::Margin::same(8);
    style.animation_time = MOTION_FAST;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(9);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    visuals.widgets.open.corner_radius = CornerRadius::same(7);
    ctx.set_style(style);
}

pub fn motion_enabled(ctx: &egui::Context) -> bool {
    ctx.style().animation_time > 0.0
}

pub fn set_reduce_motion(ctx: &egui::Context, reduce: bool) {
    let mut style = (*ctx.style()).clone();
    style.animation_time = if reduce { 0.0 } else { MOTION_FAST };
    ctx.set_style(style);
}

pub fn animated_bool(ctx: &egui::Context, id: egui::Id, target: bool, seconds: f32) -> f32 {
    if motion_enabled(ctx) {
        ctx.animate_bool_with_time(id, target, seconds)
    } else if target {
        1.0
    } else {
        0.0
    }
}

pub fn animated_value(ctx: &egui::Context, id: egui::Id, target: f32, seconds: f32) -> f32 {
    if motion_enabled(ctx) {
        ctx.animate_value_with_time(id, target, seconds)
    } else {
        target
    }
}

pub fn card_frame() -> egui::Frame {
    egui::Frame {
        fill: CARD,
        stroke: Stroke::new(1.0_f32, BORDER),
        corner_radius: CornerRadius::same(12),
        inner_margin: egui::Margin {
            left: 16,
            right: 16,
            top: 14,
            bottom: 14,
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

pub fn compact_card_frame() -> egui::Frame {
    egui::Frame {
        fill: CARD,
        stroke: Stroke::new(1.0_f32, BORDER),
        corner_radius: CornerRadius::same(10),
        inner_margin: egui::Margin {
            left: 12,
            right: 12,
            top: 10,
            bottom: 10,
        },
        ..Default::default()
    }
}

pub fn tint(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38)
}

pub fn tint_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t).round() as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t).round() as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t).round() as u8,
    )
}

pub fn badge_ink(color: Color32) -> Color32 {
    if [ACCENT, RED, VIOLET].contains(&color) {
        lerp_color(color, TEXT, 0.28)
    } else {
        color
    }
}

// ---------------------------------------------------------------- semantic color helpers
pub fn load_color(fraction: f32) -> Color32 {
    if fraction > 0.90 {
        RED
    } else if fraction > 0.72 {
        AMBER
    } else if fraction > 0.45 {
        CYAN
    } else {
        ACCENT
    }
}

pub fn temperature_color(celsius: f32) -> Color32 {
    if celsius >= 90.0 {
        RED
    } else if celsius >= 75.0 {
        AMBER
    } else {
        GREEN
    }
}

pub fn thermal_safety(celsius: f32, limit_celsius: f32) -> (&'static str, Color32) {
    if celsius >= limit_celsius {
        ("Above limit", RED)
    } else if celsius >= limit_celsius - 12.0 {
        ("Warming", AMBER)
    } else {
        ("Normal", GREEN)
    }
}

pub fn status_color(status: &Status) -> Color32 {
    match status {
        Status::Completed => GREEN,
        Status::Stopped => AMBER,
        Status::Failed => RED,
        Status::Inconclusive => VIOLET,
        Status::Unavailable => MUTED,
    }
}

/// Strict semantic SMART color mapping. Handles both bare words (`Healthy`,
/// `Verified`) and enriched summaries (`Healthy · 41 °C · wear 3%`).
pub fn smart_color(smart: &str) -> Color32 {
    let lower = smart.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return MUTED;
    }
    if lower.contains("unhealthy")
        || lower.contains("fail")
        || lower.contains("bad")
        || lower.contains("error")
    {
        return RED;
    }
    if lower.contains("warn") || lower.contains("caution") || lower.contains("degrad") {
        return AMBER;
    }
    if lower.starts_with("verified")
        || lower.starts_with("healthy")
        || lower.starts_with("good")
        || lower.starts_with("ok")
        || lower.starts_with("passed")
    {
        return GREEN;
    }
    AMBER
}

// ---------------------------------------------------------------- formatting helpers
pub fn uptime_text(seconds: u64) -> String {
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

pub fn clock_text(mhz: u64) -> String {
    if mhz >= 1000 {
        format!("{:.2} GHz", mhz as f64 / 1000.0)
    } else {
        format!("{mhz} MHz")
    }
}

pub fn short_cpu_name(name: &str) -> String {
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

pub fn vendor_text(vendor: &str) -> &str {
    match vendor {
        "GenuineIntel" => "Intel (GenuineIntel)",
        "AuthenticAMD" => "AMD (AuthenticAMD)",
        other => other,
    }
}

pub fn decimal_bytes(v: u64) -> String {
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

pub fn gib(v: u64) -> f64 {
    v as f64 / 1_073_741_824.0
}

pub fn bytes(v: u64) -> String {
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

pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn driver_class_label(class: &str) -> &str {
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

pub fn timestamp_text(unix: u64) -> String {
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

// ---------------------------------------------------------------- native Windows dark titlebar
/// Enable native dark titlebar (`DWMWA_USE_IMMERSIVE_DARK_MODE`) on every
/// top-level window belonging to this process so Windows 10/11 does not render
/// a bright white caption bar above the dark Obsidian Graphite workspace.
#[cfg(target_os = "windows")]
pub fn apply_dark_titlebar() {
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    type Bool = i32;
    type Dword = u32;
    type Hresult = i32;

    const DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1: Dword = 19;
    const DWMWA_USE_IMMERSIVE_DARK_MODE: Dword = 20;
    const DWMWA_CAPTION_COLOR: Dword = 35;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> Bool, lparam: isize) -> Bool;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut Dword) -> Dword;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> Dword;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: Hwnd,
            attr: Dword,
            value: *const c_void,
            size: Dword,
        ) -> Hresult;
    }

    extern "system" fn callback(hwnd: Hwnd, target_pid: isize) -> Bool {
        let mut pid: Dword = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid as isize == target_pid {
                let enabled: Bool = 1;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    (&enabled as *const Bool).cast::<c_void>(),
                    std::mem::size_of::<Bool>() as Dword,
                );
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1,
                    (&enabled as *const Bool).cast::<c_void>(),
                    std::mem::size_of::<Bool>() as Dword,
                );
                // COLORREF is 0x00BBGGRR — matches PANEL rgb(22, 24, 29).
                let caption: Dword = (29 << 16) | (24 << 8) | 22;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_CAPTION_COLOR,
                    (&caption as *const Dword).cast::<c_void>(),
                    std::mem::size_of::<Dword>() as Dword,
                );
            }
        }
        1
    }

    unsafe {
        let pid = GetCurrentProcessId() as isize;
        let _ = EnumWindows(callback, pid);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_dark_titlebar() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_color_handles_enriched_healthy_strings() {
        assert_eq!(smart_color("Healthy"), GREEN);
        assert_eq!(smart_color("Healthy · 41 °C · wear 3%"), GREEN);
        assert_eq!(smart_color("Verified"), GREEN);
        assert_eq!(smart_color("Good"), GREEN);
        assert_eq!(smart_color("Unhealthy"), RED);
        assert_eq!(smart_color("Failing"), RED);
        assert_eq!(smart_color("Warning"), AMBER);
        assert_eq!(smart_color(""), MUTED);
    }
}
