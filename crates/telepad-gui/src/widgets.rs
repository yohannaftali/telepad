//! Small drawing helpers and colours shared by the views.

use std::time::Instant;

use eframe::egui::{self, Color32, RichText};
use telepad_app::HostState;
use telepad_core::config::{Access, HostEntry};

use crate::icons;

pub const TEMP_WARM: f32 = 70.0;
pub const TEMP_HOT: f32 = 85.0;
pub const NGINX_COLOR: Color32 = Color32::from_rgb(0x2b, 0x8a, 0xd6);
pub const PORTAINER_COLOR: Color32 = Color32::from_rgb(0x13, 0xb5, 0xea);

pub fn connect_icon(e: &HostEntry) -> &'static str {
    match e.access {
        Access::Ssh => icons::TERMINAL,
        Access::Rdp => icons::MONITOR,
    }
}

pub fn host_os(h: &HostState) -> icons::OsKind {
    if h.entry.access == Access::Rdp {
        return icons::OsKind::Windows;
    }
    match &h.report {
        Some(r) => icons::detect(&r.os_name, &r.os_pretty, &r.os_type, &r.kernel),
        None => icons::OsKind::Unknown,
    }
}

pub fn usage_color(pct: f32) -> Color32 {
    if pct >= 90.0 {
        Color32::from_rgb(0xd9, 0x3f, 0x3f)
    } else if pct >= 75.0 {
        Color32::from_rgb(0xe0, 0x9b, 0x1a)
    } else {
        Color32::from_rgb(0x2f, 0x9e, 0x5b)
    }
}

/// Hot = red, warm = yellow, normal = blue.
pub fn temp_color(c: f32) -> Color32 {
    if c >= TEMP_HOT {
        Color32::from_rgb(0xe0, 0x3e, 0x3e)
    } else if c >= TEMP_WARM {
        Color32::from_rgb(0xf0, 0xc0, 0x20)
    } else {
        Color32::from_rgb(0x3a, 0x8e, 0xe6)
    }
}

pub fn temp_level(c: f32) -> &'static str {
    if c >= TEMP_HOT {
        "Hot (85 °C or more)"
    } else if c >= TEMP_WARM {
        "Warm (70–85 °C)"
    } else {
        "Normal (below 70 °C)"
    }
}

/// Rotations per second for a fan at `frac` of the host's fastest fan:
/// a slow idle turn at 0, about 2.5 turns/s at full speed.
pub fn fan_turns_per_sec(frac: f32) -> f32 {
    0.25 + 2.25 * frac.clamp(0.0, 1.0)
}

/// A painted four-blade fan that spins faster the higher `frac` (0..=1) is.
pub fn spinning_fan(ui: &mut egui::Ui, size: f32, frac: f32) -> egui::Response {
    use std::f32::consts::TAU;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let time = ui.input(|i| i.time) as f32;
        // Wrap time so the angle keeps float precision in long sessions.
        let angle = (time % 3600.0) * fan_turns_per_sec(frac) * TAU;
        let c = rect.center();
        let r = size * 0.5;
        let color = Color32::from_rgb(0x2b, 0x8a, 0xd6);
        let p = ui.painter();
        p.circle_stroke(
            c,
            r - 0.5,
            egui::Stroke::new(1.0, color.gamma_multiply(0.6)),
        );
        let at = |a: f32, d: f32| c + egui::vec2(a.cos(), a.sin()) * d;
        for k in 0..4 {
            let a = angle + k as f32 * TAU / 4.0;
            let blade = vec![
                at(a - 0.30, r * 0.22),
                at(a - 0.10, r * 0.85),
                at(a + 0.45, r * 0.80),
                at(a + 0.20, r * 0.22),
            ];
            p.add(egui::Shape::convex_polygon(
                blade,
                color,
                egui::Stroke::NONE,
            ));
        }
        p.circle_filled(c, r * 0.22, color);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    resp.on_hover_text(format!("{:.0}% of the fastest fan", frac * 100.0))
}

pub fn status_dot(ui: &mut egui::Ui, h: &HostState) {
    let color = match (h.loading, h.online()) {
        (_, Some(false)) => Color32::from_rgb(0xd9, 0x3f, 0x3f),
        (_, Some(true)) => Color32::from_rgb(0x2f, 0x9e, 0x5b),
        (true, None) => Color32::from_rgb(0x4a, 0x90, 0xd9),
        (false, None) => Color32::GRAY,
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

pub fn bar(ui: &mut egui::Ui, pct: f32, text: String, width: f32) {
    ui.add(
        egui::ProgressBar::new((pct / 100.0).clamp(0.0, 1.0))
            .desired_width(width)
            .fill(usage_color(pct))
            .text(text),
    );
}

pub fn ago(t: Option<Instant>) -> String {
    match t {
        None => "never".into(),
        Some(t) => {
            let s = t.elapsed().as_secs();
            if s < 60 {
                format!("{s}s ago")
            } else {
                format!("{}m ago", s / 60)
            }
        }
    }
}

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).heading().size(17.0));
    ui.separator();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_colours_and_fan_speed() {
        let blue = temp_color(45.0);
        let yellow = temp_color(75.0);
        let red = temp_color(90.0);
        assert!(blue.b() > blue.r() && red.r() > red.b() && yellow.g() > yellow.b());
        assert_eq!(temp_color(TEMP_WARM), yellow);
        assert_eq!(temp_color(TEMP_HOT), red);
        assert!(fan_turns_per_sec(1.0) > fan_turns_per_sec(0.5));
        assert!(fan_turns_per_sec(0.5) > fan_turns_per_sec(0.0));
        assert!(fan_turns_per_sec(0.0) > 0.0);
    }
}
