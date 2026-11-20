//! Small drawing helpers and colours shared by the views.

use std::time::Instant;

use eframe::egui::{self, Color32, RichText};
use telepad_app::{Flag, FlagKind, HostState, Msg, Severity};
use telepad_core::config::{Access, HostEntry};

use crate::icons;

pub const TEMP_WARM: f32 = 70.0;
pub const TEMP_HOT: f32 = 85.0;
/// Telepad's own colours, one set per theme so text stays readable (WCAG AA,
/// 4.5:1 against the panel background; checked by a test).
pub struct Palette {
    pub red: Color32,
    pub amber: Color32,
    pub green: Color32,
    pub blue: Color32,
    pub cyan: Color32,
    pub purple: Color32,
    pub grey: Color32,
    /// Usage-bar fills (red, amber, green, blue) under the theme's own bar text colour.
    pub bar: [Color32; 4],
    /// Action buttons (#76), indexed by `Act`: danger, edit, add.
    pub act: [ActColors; 3],
}

/// Filled action-button colours: rest / hover / pressed fill and the text on them.
#[derive(Clone, Copy)]
pub struct ActColors {
    pub fill: Color32,
    pub hover: Color32,
    pub press: Color32,
    pub text: Color32,
}

const fn act(fill: u32, hover: u32, press: u32, text: u32) -> ActColors {
    const fn c(v: u32) -> Color32 {
        Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
    ActColors {
        fill: c(fill),
        hover: c(hover),
        press: c(press),
        text: c(text),
    }
}

pub const DARK: Palette = Palette {
    red: Color32::from_rgb(0xf0, 0x5a, 0x5a),
    amber: Color32::from_rgb(0xe8, 0xa3, 0x26),
    green: Color32::from_rgb(0x3f, 0xb9, 0x6e),
    blue: Color32::from_rgb(0x5a, 0xa2, 0xf0),
    cyan: Color32::from_rgb(0x2c, 0xc0, 0xe8),
    purple: Color32::from_rgb(0xb3, 0x92, 0xec),
    grey: Color32::from_rgb(0x9a, 0x9a, 0x9a),
    bar: [
        Color32::from_rgb(0xa8, 0x2a, 0x2a),
        Color32::from_rgb(0x8a, 0x50, 0x00),
        Color32::from_rgb(0x1b, 0x6b, 0x3c),
        Color32::from_rgb(0x1c, 0x5a, 0x9c),
    ],
    // Deep, muted fills with light text: they sit calmly on the dark panels.
    act: [
        act(0x8f1d1d, 0xa82424, 0x731616, 0xffe4e4),
        act(0xca8a04, 0xdb9a10, 0xa87203, 0x1a1000),
        act(0x166534, 0x1b7a3f, 0x11522a, 0xe6ffee),
    ],
};

pub const LIGHT: Palette = Palette {
    red: Color32::from_rgb(0xb9, 0x1c, 0x1c),
    amber: Color32::from_rgb(0x8a, 0x52, 0x00),
    green: Color32::from_rgb(0x15, 0x6b, 0x34),
    blue: Color32::from_rgb(0x1d, 0x4e, 0xd8),
    cyan: Color32::from_rgb(0x0e, 0x60, 0x7a),
    purple: Color32::from_rgb(0x6d, 0x28, 0xd9),
    grey: Color32::from_rgb(0x5f, 0x5f, 0x5f),
    bar: [
        Color32::from_rgb(0xfe, 0xca, 0xca),
        Color32::from_rgb(0xfc, 0xd3, 0x4d),
        Color32::from_rgb(0x86, 0xef, 0xac),
        Color32::from_rgb(0x93, 0xc5, 0xfd),
    ],
    // Brighter fills for the light panels (a clear red, not the dark theme's maroon).
    act: [
        act(0xdc2626, 0xc81e1e, 0xb21a1a, 0xffffff),
        act(0xfacc15, 0xf0bf0a, 0xd9ab06, 0x422006),
        act(0x15803d, 0x13703a, 0x0f5e30, 0xffffff),
    ],
};

static DARK_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Set once per frame from the active egui theme.
pub fn set_dark_mode(dark: bool) {
    DARK_MODE.store(dark, std::sync::atomic::Ordering::Relaxed);
}

pub fn pal() -> &'static Palette {
    if DARK_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

/// Fill for a usage bar: the text on it must stay readable in both themes.
pub fn usage_fill(pct: f32) -> Color32 {
    let [red, amber, green, _] = pal().bar;
    if pct >= 90.0 {
        red
    } else if pct >= 75.0 {
        amber
    } else {
        green
    }
}

pub fn info_fill() -> Color32 {
    pal().bar[3]
}

pub fn nginx_color() -> Color32 {
    pal().blue
}

pub fn portainer_color() -> Color32 {
    pal().cyan
}

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
        pal().red
    } else if pct >= 75.0 {
        pal().amber
    } else {
        pal().green
    }
}

/// Hot = red, warm = yellow, normal = blue.
pub fn temp_color(c: f32) -> Color32 {
    if c >= TEMP_HOT {
        pal().red
    } else if c >= TEMP_WARM {
        pal().amber
    } else {
        pal().blue
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
        (_, Some(false)) => pal().red,
        (_, Some(true)) => pal().green,
        (true, None) => pal().blue,
        (false, None) => pal().grey,
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

pub fn bar(ui: &mut egui::Ui, pct: f32, text: String, width: f32) {
    ui.add(
        egui::ProgressBar::new((pct / 100.0).clamp(0.0, 1.0))
            .desired_width(width)
            .fill(usage_fill(pct))
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
            } else if s < 3600 {
                format!("{}m ago", s / 60)
            } else {
                format!("{}h {}m ago", s / 3600, s % 3600 / 60)
            }
        }
    }
}

/// Drag-and-drop payload: index of the host being dragged.
#[derive(Debug, Clone, Copy)]
pub struct HostDrag(pub usize);

/// The ⋮⋮ handle a host is dragged by.
pub fn grip_handle(ui: &mut egui::Ui, i: usize) {
    ui.dnd_drag_source(ui.make_persistent_id(("host-drag", i)), HostDrag(i), |ui| {
        ui.label(RichText::new(icons::GRIP).weak())
    })
    .response
    .on_hover_cursor(egui::CursorIcon::Grab)
    .on_hover_text("Drag to reorder");
}

/// A host row (`resp`) that a dragged host can be dropped on: a line marks the
/// upper or lower edge under the pointer, and the release becomes a move.
pub fn drop_target(resp: &egui::Response, onto: usize, accent: Color32, out: &mut Vec<Msg>) {
    let Some(pos) = resp.ctx.pointer_latest_pos() else {
        return;
    };
    let below = pos.y > resp.rect.center().y;
    if resp.dnd_hover_payload::<HostDrag>().is_some() {
        let y = if below {
            resp.rect.bottom()
        } else {
            resp.rect.top()
        };
        resp.ctx.layer_painter(resp.layer_id).hline(
            resp.rect.x_range(),
            y,
            egui::Stroke::new(2.5, accent),
        );
    }
    if let Some(drag) = resp.dnd_release_payload::<HostDrag>() {
        out.push(Msg::MoveHost {
            from: drag.0,
            to: telepad_app::drop_index(drag.0, onto, below),
        });
    }
}

/// Full-width page button that reads as a button in both themes: filled with
/// the accent colour while its page is shown, otherwise a raised fill with an
/// accent outline that brightens on hover.
pub fn nav_button(ui: &mut egui::Ui, shown: bool, icon: &str, text: &str) -> egui::Response {
    ui.scope(|ui| {
        let accent = ui.visuals().selection.bg_fill;
        let on_accent = ui.visuals().selection.stroke.color;
        // The light theme's accent fill is too pale for a 1.5 px outline.
        let outline = if ui.visuals().dark_mode {
            accent
        } else {
            on_accent
        };
        let strong = ui.visuals().strong_text_color();
        let w = &mut ui.visuals_mut().widgets;
        for (state, width) in [
            (&mut w.inactive, 1.5),
            (&mut w.hovered, 2.0),
            (&mut w.active, 2.0),
        ] {
            state.bg_stroke = egui::Stroke::new(width, outline);
            state.corner_radius = egui::CornerRadius::same(6);
        }
        w.hovered.weak_bg_fill = accent.gamma_multiply(0.35);
        w.active.weak_bg_fill = accent.gamma_multiply(0.6);
        let label = RichText::new(icons::label(icon, text)).strong().size(16.0);
        let button = if shown {
            egui::Button::new(label.color(on_accent)).fill(accent)
        } else {
            egui::Button::new(label.color(strong))
        };
        ui.add(button.min_size(egui::vec2(ui.available_width(), 36.0)))
    })
    .inner
}

/// What an action button does, which sets its colour (#76).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// Deletes, removes, drops or revokes: red.
    Danger,
    /// Changes something in place: yellow.
    Edit,
    /// Adds or creates: green.
    Add,
}

impl Act {
    pub fn colors(self) -> ActColors {
        pal().act[self as usize]
    }
}

/// A filled action button (red / yellow / green by `kind`), with hover and
/// pressed shades; disabled it is drawn faded like any disabled widget.
pub fn act_button(ui: &mut egui::Ui, enabled: bool, kind: Act, text: &str) -> egui::Response {
    act_widget(ui, enabled, kind, text, false)
}

/// Small icon-only (or short) variant of `act_button` for table rows.
pub fn act_small(ui: &mut egui::Ui, enabled: bool, kind: Act, text: &str) -> egui::Response {
    act_widget(ui, enabled, kind, text, true)
}

fn act_widget(
    ui: &mut egui::Ui,
    enabled: bool,
    kind: Act,
    text: &str,
    small: bool,
) -> egui::Response {
    ui.scope(|ui| {
        let c = kind.colors();
        let w = &mut ui.visuals_mut().widgets;
        for (state, fill) in [
            (&mut w.inactive, c.fill),
            (&mut w.hovered, c.hover),
            (&mut w.active, c.press),
        ] {
            state.weak_bg_fill = fill;
            state.bg_fill = fill;
            state.fg_stroke.color = c.text;
            state.bg_stroke = egui::Stroke::new(1.0, fill);
            state.corner_radius = egui::CornerRadius::same(5);
        }
        let mut b = egui::Button::new(RichText::new(text).color(c.text))
            .wrap_mode(egui::TextWrapMode::Extend);
        if small {
            b = b.small();
        }
        ui.add_enabled(enabled, b)
    })
    .inner
}

/// Text in the action colour for menu entries and headings, where a filled
/// button would look out of place (readable on the panel background).
pub fn act_text(kind: Act, text: &str) -> RichText {
    let p = pal();
    RichText::new(text).color(match kind {
        Act::Danger => p.red,
        Act::Edit => p.amber,
        Act::Add => p.green,
    })
}

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).heading().size(17.0));
    ui.separator();
}

pub fn severity_color(s: Severity) -> Color32 {
    match s {
        Severity::Danger => usage_color(95.0),
        Severity::Warn => usage_color(80.0),
        Severity::Info => pal().blue,
    }
}

pub fn flag_icon(k: FlagKind) -> &'static str {
    match k {
        FlagKind::EndOfSupport => icons::WARNING,
        FlagKind::Reboot => icons::REFRESH,
        FlagKind::Updates => icons::IMPORT,
        FlagKind::Cluster => icons::STACK,
    }
}

/// Maintenance flags as coloured icon + short label, details on hover.
pub fn flag_chips(ui: &mut egui::Ui, flags: &[Flag]) {
    for f in flags {
        ui.label(
            RichText::new(format!("{} {}", flag_icon(f.kind), f.short))
                .color(severity_color(f.severity))
                .small(),
        )
        .on_hover_text(&f.detail);
    }
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

    fn luminance(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn palette_text_is_readable_on_both_themes() {
        let dark_bg = egui::Visuals::dark().panel_fill;
        let light_bg = egui::Visuals::light().panel_fill;
        for (theme, p, bg) in [("dark", &DARK, dark_bg), ("light", &LIGHT, light_bg)] {
            for (name, c) in [
                ("red", p.red),
                ("amber", p.amber),
                ("green", p.green),
                ("blue", p.blue),
                ("cyan", p.cyan),
                ("purple", p.purple),
                ("grey", p.grey),
            ] {
                let r = contrast(c, bg);
                assert!(r >= 4.5, "{theme} {name}: contrast {r:.2} < 4.5");
            }
        }
        // Bar text (egui draws it in the selection stroke colour) on each bar fill.
        for (theme, p, v) in [
            ("dark", &DARK, egui::Visuals::dark()),
            ("light", &LIGHT, egui::Visuals::light()),
        ] {
            for (i, fill) in p.bar.iter().enumerate() {
                let r = contrast(v.selection.stroke.color, *fill);
                assert!(r >= 4.5, "{theme} bar {i}: contrast {r:.2} < 4.5");
            }
        }
    }

    #[test]
    fn action_buttons_are_readable_and_differ_per_theme() {
        for (theme, p) in [("dark", &DARK), ("light", &LIGHT)] {
            for (i, a) in p.act.iter().enumerate() {
                for (state, fill) in [("rest", a.fill), ("hover", a.hover), ("pressed", a.press)] {
                    let r = contrast(a.text, fill);
                    assert!(r >= 4.5, "{theme} act {i} {state}: contrast {r:.2} < 4.5");
                }
            }
        }
        // A different red (and fills) per theme, not one shared colour.
        for i in 0..3 {
            assert_ne!(DARK.act[i].fill, LIGHT.act[i].fill);
        }
        let d = DARK.act[Act::Danger as usize].fill;
        assert!(d.r() > d.g() && d.r() > d.b());
        let g = LIGHT.act[Act::Add as usize].fill;
        assert!(g.g() > g.r() && g.g() > g.b());
    }
}
