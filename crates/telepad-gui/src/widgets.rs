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
    /// Action buttons (#76, #83), indexed by `Act`: danger, edit, add, caution.
    pub act: [ActColors; 4],
    /// Table headers (#151).
    pub head: HeadColors,
}

/// Table header colours (#151): the header band stands apart from the rows,
/// and a sortable title lights up under the mouse.
#[derive(Clone, Copy)]
pub struct HeadColors {
    pub bg: Color32,
    pub text: Color32,
    pub hover_bg: Color32,
    pub hover_text: Color32,
    /// The line under the header.
    pub line: Color32,
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
        act(0xb45309, 0xbd5410, 0x92400e, 0xffffff),
    ],
    head: HeadColors {
        bg: Color32::from_rgb(0x2a, 0x33, 0x3f),
        text: Color32::from_rgb(0xe6, 0xec, 0xf3),
        hover_bg: Color32::from_rgb(0x1c, 0x5a, 0x9c),
        hover_text: Color32::WHITE,
        line: Color32::from_rgb(0x5a, 0xa2, 0xf0),
    },
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
        act(0xc2410c, 0xb23c0b, 0x9a3412, 0xffffff),
    ],
    head: HeadColors {
        bg: Color32::from_rgb(0xd5, 0xde, 0xea),
        text: Color32::from_rgb(0x1f, 0x29, 0x37),
        hover_bg: Color32::from_rgb(0x1d, 0x4e, 0xd8),
        hover_text: Color32::WHITE,
        line: Color32::from_rgb(0x1d, 0x4e, 0xd8),
    },
};

/// Telepad's light theme (#86). egui's own light colours put panels, windows
/// and buttons on nearly the same white (panel and window gray 248, buttons
/// gray 230 without a border), so nothing stood apart. Here: a grey app
/// background, white windows with a border, and buttons / inputs with a
/// visible fill and border.
pub fn light_visuals() -> egui::Visuals {
    let c = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    let mut v = egui::Visuals::light();
    v.panel_fill = c(0xe6e9ef);
    v.window_fill = c(0xffffff);
    v.window_stroke = egui::Stroke::new(1.0, c(0xaeb6c2));
    v.window_shadow.color = Color32::from_black_alpha(40);
    v.faint_bg_color = c(0xf1f3f6);
    v.extreme_bg_color = c(0xffffff);
    v.code_bg_color = c(0xeef1f5);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, c(0xc9cfd8));
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0, c(0x2b3440));
    for (st, fill, border, text) in [
        (&mut w.inactive, 0xe2e6ec, 0xa9b2bf, 0x1f2733),
        (&mut w.hovered, 0xd3d9e2, 0x7c8796, 0x111820),
        (&mut w.active, 0xc3cbd6, 0x5d6878, 0x000000),
        (&mut w.open, 0xd3d9e2, 0x7c8796, 0x111820),
    ] {
        st.weak_bg_fill = c(fill);
        st.bg_fill = c(fill);
        st.bg_stroke = egui::Stroke::new(1.0, c(border));
        st.fg_stroke.color = c(text);
    }
    v
}

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
    match h.shown() {
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
    t.map_or("never".into(), |t| ago_secs(t.elapsed().as_secs()))
}

/// [`ago`] for a time that may lie before this run (#122, saved facts).
pub fn ago_seen(t: Option<telepad_app::Seen>) -> String {
    t.map_or("never".into(), |t| {
        ago_secs(t.age(Instant::now()).as_secs())
    })
}

fn ago_secs(s: u64) -> String {
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 2 * 86400 {
        format!("{}h {}m ago", s / 3600, s % 3600 / 60)
    } else {
        format!("{}d {}h ago", s / 86400, s % 86400 / 3600)
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
        // Icon only (folded sidebar, #83): a square button in the middle.
        let label = if text.is_empty() {
            RichText::new(icon).strong().size(18.0)
        } else {
            RichText::new(icons::label(icon, text)).strong().size(16.0)
        };
        let button = if shown {
            egui::Button::new(label.color(on_accent)).fill(accent)
        } else {
            egui::Button::new(label.color(strong))
        };
        if text.is_empty() {
            ui.vertical_centered(|ui| ui.add(button.min_size(egui::vec2(40.0, 36.0))))
                .inner
        } else {
            ui.add(button.min_size(egui::vec2(ui.available_width(), 36.0)))
        }
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
    /// Opens a session that can change things (SSH, RDP): amber / orange.
    Caution,
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
    // Colours set on this Ui and put back right after: a child Ui (scope)
    // would keep a wrapping row from moving the button to the next line.
    let c = kind.colors();
    let saved = ui.visuals().widgets.clone();
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
    let mut b =
        egui::Button::new(RichText::new(text).color(c.text)).wrap_mode(egui::TextWrapMode::Extend);
    if small {
        b = b.small();
    }
    let r = ui.add_enabled(enabled, b);
    ui.visuals_mut().widgets = saved;
    r
}

/// Right-click menu of a host row or card (#83): SSH / RDP, Files, the
/// databases Telepad detected on it (host and containers), then open, edit,
/// clone and delete. Coloured like the buttons they replace.
pub fn host_menu(ui: &mut egui::Ui, m: &telepad_app::Model, i: usize, out: &mut Vec<Msg>) {
    let Some(h) = m.hosts.get(i) else { return };
    let has_rdp = m.rdp.contains_key(&h.entry.alias);
    let paused = m.host_paused(h);
    let item = |ui: &mut egui::Ui, text: RichText, msg: Msg, out: &mut Vec<Msg>| {
        if ui.button(text).clicked() {
            out.push(msg);
            ui.close();
        }
    };
    ui.label(RichText::new(&h.entry.alias).strong());
    ui.separator();
    let open = match h.entry.access {
        Access::Ssh => "Open SSH",
        Access::Rdp => "Open RDP",
    };
    item(
        ui,
        act_text(Act::Caution, &icons::label(connect_icon(&h.entry), open)),
        Msg::Ssh(i),
        out,
    );
    // An SSH host with saved Remote Desktop settings (#102).
    if h.entry.access == Access::Ssh && has_rdp {
        item(
            ui,
            act_text(Act::Caution, &icons::label(icons::MONITOR, "Open RDP")),
            Msg::OpenRdp(i),
            out,
        );
    }
    if h.entry.access == Access::Ssh {
        item(
            ui,
            act_text(Act::Edit, &icons::label(icons::FOLDER, "Files")),
            Msg::Files(telepad_app::files::FmMsg::Open(h.entry.alias.clone())),
            out,
        );
    }
    if let Some(r) = &h.report {
        let mut dbs = telepad_core::db::detect::host_targets(&h.entry.alias, r);
        dbs.extend(
            r.docker
                .containers
                .iter()
                .filter_map(|c| telepad_core::db::detect::container_target(&h.entry.alias, c)),
        );
        if !dbs.is_empty() {
            ui.separator();
            ui.label(RichText::new("Databases").weak().small());
            for t in dbs {
                let label = icons::label(icons::DATABASE, &t.title());
                item(
                    ui,
                    RichText::new(label),
                    Msg::Db(telepad_app::db::DbMsg::Open(t)),
                    out,
                );
            }
        }
    }
    ui.separator();
    // Pause this host's automatic checks (#122).
    if !h.entry.unmonitored {
        let (icon, text) = if paused {
            (icons::PLAY, "Resume updates")
        } else {
            (icons::PAUSE, "Pause updates")
        };
        item(
            ui,
            RichText::new(icons::label(icon, text)),
            Msg::PauseHosts {
                aliases: vec![h.entry.alias.clone()],
                paused: !paused,
            },
            out,
        );
    }
    // Group / cluster (#144, #145): move the host, or take it out.
    for kind in telepad_core::config::SetKind::ALL {
        let current = h.entry.set(kind);
        let icon = match kind {
            telepad_core::config::SetKind::Group => icons::FOLDERS,
            telepad_core::config::SetKind::Cluster => icons::CIRCLES_THREE,
        };
        let shown = telepad_app::sets::shown_name(kind, current);
        ui.menu_button(
            icons::label(icon, &format!("{}: {shown}", kind.title())),
            |ui| {
                let sets = telepad_app::sets::list(m, kind);
                if sets.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "No {}s yet: add them on the {}s page",
                            kind.as_str(),
                            kind.title()
                        ))
                        .weak(),
                    );
                }
                let assign = |name: &str| {
                    Msg::Sets(telepad_app::sets::StMsg::Assign {
                        kind,
                        aliases: vec![h.entry.alias.clone()],
                        name: name.to_string(),
                    })
                };
                for s in sets {
                    if ui.radio(current == s.name, &s.name).clicked() {
                        out.push(assign(&s.name));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.radio(current.is_empty(), kind.none_name()).clicked() {
                    out.push(assign(""));
                    ui.close();
                }
            },
        );
    }
    ui.separator();
    item(
        ui,
        RichText::new(icons::label(icons::DESKTOP, "Open host window")),
        Msg::Select(Some(i)),
        out,
    );
    item(
        ui,
        act_text(Act::Edit, &icons::label(icons::EDIT, "Edit")),
        Msg::Edit(i),
        out,
    );
    item(
        ui,
        RichText::new(icons::label(
            icons::CLONE,
            "Clone (User, Port, IdentityFile)",
        )),
        Msg::Clone(i),
        out,
    );
    item(
        ui,
        act_text(Act::Danger, &icons::label(icons::TRASH, "Delete…")),
        Msg::Delete(i),
        out,
    );
}

/// Text in the action colour for menu entries and headings, where a filled
/// button would look out of place (readable on the panel background).
pub fn act_text(kind: Act, text: &str) -> RichText {
    let p = pal();
    RichText::new(text).color(match kind {
        Act::Danger => p.red,
        Act::Edit => p.amber,
        Act::Add => p.green,
        Act::Caution => p.amber,
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
/// One flag chip's text, in the normal text size (#143: small was hard to read).
pub fn chip_text(f: &Flag) -> RichText {
    RichText::new(format!("{} {}", flag_icon(f.kind), f.short)).color(severity_color(f.severity))
}

pub fn flag_chips(ui: &mut egui::Ui, flags: &[Flag]) {
    for f in flags {
        ui.label(chip_text(f)).on_hover_text(&f.detail);
    }
}

/// Text of a table cell: plain, cut with "…", so the row takes the click
/// (#137, #153).
pub fn cell_text(ui: &mut egui::Ui, t: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Label::new(t).truncate().selectable(false))
}

/// Height of a [`pager_bar`] footer.
pub const PAGER_H: f32 = 34.0;
/// Below this width the pager's controls go on a line of their own (#152).
pub const PAGER_WIDE: f32 = 680.0;

/// Height the pager footer takes at this width.
pub fn pager_height(width: f32) -> f32 {
    if width >= PAGER_WIDE {
        PAGER_H
    } else {
        2.0 * PAGER_H + 6.0
    }
}

/// The footer pager of a table (#138, #140, #152), as in the Senar / wtbapp
/// tables: `Record [1 - 20] of 279` on the left; on the right Rows per page
/// (typed; above 100 asks first), first / previous / `Page [n] of N` / next /
/// last and the continuous-mode toggle (∞ / numbered list). `total`:
/// `None` while the count is not known (shown as "of many"). Returns
/// `true` when the page or the size changed.
pub fn pager_bar(
    ui: &mut egui::Ui,
    p: &mut telepad_app::pager::Pager,
    total: Option<usize>,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
) -> bool {
    let mut changed = false;
    let known = total.unwrap_or(usize::MAX / 2);
    let info = match total {
        Some(t) => p.record_info(t),
        None => {
            let (s, e, _) = p.range(known);
            format!("Record [{} - {e}] of many", s + 1)
        }
    };
    if ui.available_width() >= PAGER_WIDE {
        ui.horizontal(|ui| {
            ui.set_min_height(PAGER_H - 8.0);
            ui.label(RichText::new(&info).weak());
            ask_size(ui, p, &mut changed);
            // Right-aligned: drawn from the right edge, so in reverse.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pager_controls(ui, p, total, known, salt, true, &mut changed);
            });
        });
    } else {
        ui.label(RichText::new(&info).weak());
        ask_size(ui, p, &mut changed);
        ui.horizontal_wrapped(|ui| {
            pager_controls(ui, p, total, known, salt, false, &mut changed);
        });
    }
    changed
}

/// "N rows per page makes the list slow. Continue?" Yes / No.
fn ask_size(ui: &mut egui::Ui, p: &mut telepad_app::pager::Pager, changed: &mut bool) {
    if let Some(n) = p.ask {
        ui.horizontal(|ui| {
            ui.colored_label(
                usage_color(80.0),
                format!("{n} rows per page makes the list slow. Continue?"),
            );
            if ui.small_button("Yes").clicked() {
                *changed |= p.answer(true);
            }
            if ui.small_button("No").clicked() {
                p.answer(false);
            }
        });
    }
}

/// The pager's controls, left to right: Rows per page [n] | « ‹ Page [n] of
/// N › » | mode toggle. `reverse`: drawn in a right-to-left layout.
fn pager_controls(
    ui: &mut egui::Ui,
    p: &mut telepad_app::pager::Pager,
    total: Option<usize>,
    known: usize,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    reverse: bool,
    changed: &mut bool,
) {
    use telepad_app::pager::Go;
    #[derive(Clone, Copy)]
    enum Item {
        SizeLabel,
        SizeBox,
        Sep,
        Button(Go),
        PageLabel,
        PageBox,
        OfPages,
        Toggle,
    }
    let mut items = vec![Item::SizeLabel, Item::SizeBox, Item::Sep];
    // Continuous mode has no page buttons (rows load on scroll).
    if !p.continuous {
        items.extend([
            Item::Button(Go::First),
            Item::Button(Go::Prev),
            Item::PageLabel,
            Item::PageBox,
            Item::OfPages,
            Item::Button(Go::Next),
            Item::Button(Go::Last),
            Item::Sep,
        ]);
    }
    items.push(Item::Toggle);
    if reverse {
        items.reverse();
    }
    let pages = p.pages(known);
    let first = p.page.min(pages - 1) == 0;
    let last = total.is_some() && p.page + 1 >= pages;
    let caret = |a: &str, two: bool| {
        if two {
            format!("{a}{a}")
        } else {
            a.to_string()
        }
    };
    for it in items {
        match it {
            Item::SizeLabel => {
                ui.label(p.size_label());
            }
            Item::SizeBox => {
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut p.per_page_text)
                            .id_salt(("pager-size", salt))
                            .desired_width(48.0)
                            .hint_text("100"),
                    )
                    .on_hover_text("Type a number (20, 50, 100 …) and press Enter; empty = 100");
                if r.lost_focus() {
                    *changed |= p.apply_per_page();
                }
            }
            Item::Sep => {
                ui.separator();
            }
            Item::Button(go) => {
                let (text, tip, on) = match go {
                    Go::First => (caret(crate::icons::CARET_LEFT, true), "First page", !first),
                    Go::Prev => (
                        caret(crate::icons::CARET_LEFT, false),
                        "Previous page",
                        !first,
                    ),
                    Go::Next => (caret(crate::icons::CARET_RIGHT, false), "Next page", !last),
                    Go::Last => (
                        caret(crate::icons::CARET_RIGHT, true),
                        "Last page",
                        !last && total.is_some(),
                    ),
                };
                if ui
                    .add_enabled(on, egui::Button::new(text).small())
                    .on_hover_text(tip)
                    .clicked()
                {
                    *changed |= p.go(go, known);
                }
            }
            Item::PageLabel => {
                ui.label("Page");
            }
            Item::PageBox => {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut p.page_text)
                        .id_salt(("pager-page", salt))
                        .desired_width(40.0),
                );
                if r.lost_focus() {
                    *changed |= p.apply_page(known);
                }
            }
            Item::OfPages => {
                ui.label(match total {
                    Some(_) => format!("of {pages}"),
                    None => "of many".into(),
                });
            }
            Item::Toggle => {
                // The mode toggle, last (wtbapp / Senar #1104).
                let (icon, tip) = if p.continuous {
                    (crate::icons::LIST_NUMBERS, "Switch to pages")
                } else {
                    (
                        crate::icons::INFINITY,
                        "Switch to continuous (load more rows on scroll)",
                    )
                };
                if ui.button(icon).on_hover_text(tip).clicked() {
                    p.toggle();
                    *changed = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #151: header text is readable on the header band and on its hover
    /// colour, and the band stands apart from the rows in both themes.
    #[test]
    fn table_header_colours_are_readable_and_stand_out() {
        for (p, rows) in [(&DARK, Color32::from_gray(27)), (&LIGHT, Color32::WHITE)] {
            let h = p.head;
            assert!(contrast(h.text, h.bg) >= 7.0, "header text");
            assert!(contrast(h.hover_text, h.hover_bg) >= 4.5, "hover text");
            assert!(contrast(h.bg, rows) >= 1.3, "band vs rows");
            assert!(contrast(h.hover_bg, h.bg) >= 1.5, "hover vs band");
        }
    }

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
        let light = light_visuals();
        for (theme, p, bg) in [
            ("dark", &DARK, dark_bg),
            ("light panel", &LIGHT, light.panel_fill),
            ("light window", &LIGHT, light.window_fill),
        ] {
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
            ("light", &LIGHT, light_visuals()),
        ] {
            for (i, fill) in p.bar.iter().enumerate() {
                let r = contrast(v.selection.stroke.color, *fill);
                assert!(r >= 4.5, "{theme} bar {i}: contrast {r:.2} < 4.5");
            }
        }
    }

    #[test]
    fn light_theme_layers_stand_apart() {
        let v = light_visuals();
        // Window vs workspace background, button vs window, text on buttons.
        assert!(contrast(v.window_fill, v.panel_fill) > 1.15);
        assert!(contrast(v.widgets.inactive.bg_stroke.color, v.window_fill) >= 2.0);
        assert!(contrast(v.window_stroke.color, v.panel_fill) >= 1.6);
        for st in [&v.widgets.inactive, &v.widgets.hovered, &v.widgets.active] {
            let r = contrast(st.fg_stroke.color, st.weak_bg_fill);
            assert!(r >= 4.5, "button text contrast {r:.2}");
        }
        assert!(contrast(v.widgets.noninteractive.fg_stroke.color, v.panel_fill) >= 4.5);
        assert!(contrast(v.weak_text_color(), v.window_fill) >= 3.0);
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
        for i in 0..4 {
            assert_ne!(DARK.act[i].fill, LIGHT.act[i].fill);
        }
        let d = DARK.act[Act::Danger as usize].fill;
        assert!(d.r() > d.g() && d.r() > d.b());
        let g = LIGHT.act[Act::Add as usize].fill;
        assert!(g.g() > g.r() && g.g() > g.b());
    }
}
