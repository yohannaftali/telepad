//! The main window (#77): a page sidebar on the left and a workspace of inner
//! windows on the right. Inner windows are moved by their title bar, resized
//! by their edges, minimized to the strip at the bottom, maximized to fill the
//! workspace and grouped as tabs. Their state lives in `telepad_app::workspace`;
//! this module only draws it and reports what the user did.

use std::time::Instant;

use eframe::egui::{self, Id, LayerId, Order, Rect, RichText, Sense, Vec2};
use telepad_app::workspace::{
    Group, SNAP_LAYOUTS, Win, WsMsg, ZONE_MAX, Zone, edge_zone, zone_rect,
};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::config::SetKind;

use crate::icons::{self, label as il};
use crate::widgets::nav_button;

const TITLE_H: f32 = 28.0;
const STRIP_H: f32 = 32.0;

fn ws(out: &mut Vec<Msg>, m: WsMsg) {
    out.push(Msg::Ws(m));
}

/// Icon and title of a page's window.
pub fn title(m: &Model, w: Win) -> (&'static str, String) {
    let alias = |id: u64| {
        m.files
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.host.alias.clone())
            .unwrap_or_default()
    };
    match w {
        Win::MyComputer => (icons::DESKTOP, "My Computer".into()),
        Win::Overview => (icons::OVERVIEW, "Hosts".into()),
        Win::Docker => (icons::CONTAINER, "Docker / Podman".into()),
        Win::Kube => (icons::NETWORK, "Kubernetes".into()),
        Win::Swarm => (icons::STACK, "Docker Swarm".into()),
        Win::Host => (
            icons::DESKTOP,
            m.selected
                .and_then(|i| m.hosts.get(i))
                .map_or("Host".into(), |h| format!("Host — {}", h.entry.alias)),
        ),
        Win::HostSync => (icons::SYNC, "Host Management".into()),
        Win::Keys => (icons::KEY, "Authorized keys".into()),
        Win::Strategy => (icons::CLOCK, "Update Strategy".into()),
        Win::Files(id) => (icons::FOLDER, format!("Files — {}", alias(id))),
        Win::Db(id) => (
            icons::DATABASE,
            m.db.iter()
                .find(|s| s.id == id)
                .map_or("Database".into(), |s| s.target.title()),
        ),
        Win::Editor(id) => (icons::CODE, format!("Editor — {}", alias(id))),
        Win::Logins(id) => (
            icons::ROWS,
            m.logins
                .iter()
                .find(|s| s.id == id)
                .map_or("Log info".into(), |s| format!("Log info — {}", s.alias)),
        ),
        Win::Sets(SetKind::Group) => (icons::FOLDERS, "Groups".into()),
        Win::Sets(SetKind::Cluster) => (icons::CIRCLES_THREE, "Clusters".into()),
        Win::SetHosts(id) => {
            let p = m.set_pages.iter().find(|p| p.id == id);
            let icon = match p.map(|p| p.kind) {
                Some(SetKind::Cluster) => icons::CIRCLES_THREE,
                _ => icons::FOLDERS,
            };
            (icon, p.map_or("Hosts".into(), |p| p.title()))
        }
        Win::Portainers => (super::portainer::ICON, "Portainers".into()),
        Win::Portainer(id) => (
            super::portainer::ICON,
            m.pt_pages
                .iter()
                .find(|p| p.id == id)
                .map_or("Portainer".into(), |p| p.title()),
        ),
        Win::PtStack(id) => (
            icons::STACK,
            m.pt_editors
                .iter()
                .find(|e| e.id == id)
                .map_or("Stack".into(), |e| e.title()),
        ),
    }
}

/// What closing a page means: Overview / Docker just close, the others end
/// their session (which may ask first, e.g. unsaved editor tabs).
fn close_msg(w: Win) -> Msg {
    use telepad_app::files::FmMsg;
    match w {
        Win::MyComputer
        | Win::Overview
        | Win::Docker
        | Win::Kube
        | Win::Swarm
        | Win::Strategy
        | Win::Sets(_)
        | Win::Portainers => Msg::Ws(WsMsg::Close(w)),
        Win::Portainer(id) => Msg::Portainer(telepad_app::portainer::PtMsg::ClosePage(id)),
        Win::PtStack(id) => Msg::Portainer(telepad_app::portainer::PtMsg::EditorClose(id)),
        Win::SetHosts(id) => Msg::Sets(telepad_app::sets::StMsg::ClosePage(id)),
        Win::Host => Msg::Select(None),
        Win::HostSync => Msg::HostSync(telepad_app::hostsync::HsMsg::Close),
        Win::Keys => Msg::KeysClose,
        Win::Files(id) => Msg::Files(FmMsg::Close(id)),
        Win::Db(id) => Msg::Db(telepad_app::db::DbMsg::Close(id)),
        Win::Logins(id) => Msg::Logins(telepad_app::logins::LgMsg::Close(id)),
        Win::Editor(id) => Msg::Files(FmMsg::Editor {
            id,
            msg: telepad_app::editor::EdMsg::CloseWindow,
        }),
    }
}

/// Default size of a page's window (clamped to the workspace).
fn default_size(w: Win) -> Vec2 {
    match w {
        Win::MyComputer => egui::vec2(900.0, 640.0),
        Win::Overview => egui::vec2(1000.0, 600.0),
        Win::Docker | Win::Kube | Win::Swarm => egui::vec2(1000.0, 600.0),
        Win::Host => egui::vec2(900.0, 640.0),
        Win::HostSync => egui::vec2(1150.0, 680.0),
        Win::Keys => egui::vec2(660.0, 560.0),
        Win::Strategy => egui::vec2(960.0, 470.0),
        Win::Files(_) => egui::vec2(1200.0, 720.0),
        Win::Db(_) => egui::vec2(1150.0, 700.0),
        Win::Editor(_) => egui::vec2(1050.0, 720.0),
        Win::Logins(_) => egui::vec2(1200.0, 640.0),
        Win::Sets(_) => egui::vec2(820.0, 520.0),
        Win::SetHosts(_) => egui::vec2(1000.0, 600.0),
        Win::Portainers => egui::vec2(1000.0, 480.0),
        Win::Portainer(_) => egui::vec2(1100.0, 700.0),
        Win::PtStack(_) => egui::vec2(1150.0, 720.0),
    }
}

// ---------------------------------------------------------------- sidebar

pub fn sidebar(ui: &mut egui::Ui, m: &Model, out: &mut Vec<Msg>) {
    let collapsed = m.ws.sidebar_collapsed;
    ui.add_space(4.0);
    let (icon, tip) = if collapsed {
        (icons::CARET_RIGHT, "Show page names")
    } else {
        (icons::CARET_LEFT, "Icons only")
    };
    let toggle = |ui: &mut egui::Ui, out: &mut Vec<Msg>| {
        if ui
            .button(format!("{} {icon}", icons::SIDEBAR))
            .on_hover_text(tip)
            .clicked()
        {
            ws(out, WsMsg::SetSidebarCollapsed(!collapsed));
        }
    };
    if collapsed {
        ui.vertical_centered(|ui| toggle(ui, out));
    } else {
        ui.horizontal(|ui| {
            toggle(ui, out);
            ui.label(RichText::new("Pages").weak());
        });
    }
    ui.add_space(6.0);
    let text = |t: &'static str| if collapsed { "" } else { t };
    let focused = m.ws.focused;
    let shown = |w: Win| focused == Some(w) && m.ws.is_open(w);

    for (w, icon, name, tip) in [
        (
            Win::MyComputer,
            icons::DESKTOP,
            "My Computer",
            "This computer: system, usage, disks, network, Docker, ports; Terminal and Files",
        ),
        (
            Win::Sets(SetKind::Group),
            icons::FOLDERS,
            "Groups",
            "Your host groups (and Ungrouped); open one for its hosts",
        ),
        (
            Win::Sets(SetKind::Cluster),
            icons::CIRCLES_THREE,
            "Clusters",
            "Your host clusters (separate from groups); open one for its hosts",
        ),
        (
            Win::Overview,
            icons::OVERVIEW,
            "Hosts",
            "All hosts as a table or cards; SSH and Files per host",
        ),
        (
            Win::Docker,
            icons::CONTAINER,
            "Docker / Podman",
            "Containers of all hosts, grouped by stack",
        ),
        (
            Win::Kube,
            icons::NETWORK,
            "Kubernetes",
            "Kubernetes clusters found on your hosts",
        ),
        (
            Win::Swarm,
            icons::STACK,
            "Docker Swarm",
            "Docker Swarm clusters found on your hosts",
        ),
    ] {
        if nav_button(ui, shown(w), icon, text(name))
            .on_hover_text(format!("{name}: {tip}"))
            .clicked()
        {
            ws(out, WsMsg::Open(w));
        }
        ui.add_space(3.0);
    }
    if nav_button(
        ui,
        shown(Win::Portainers),
        super::portainer::ICON,
        text("Portainers"),
    )
    .on_hover_text("Portainers: your Portainer servers, their environments and stacks (read-only)")
    .clicked()
    {
        out.push(Msg::Portainer(telepad_app::portainer::PtMsg::OpenList));
    }
    ui.add_space(3.0);
    if nav_button(ui, shown(Win::Keys), icons::KEY, text("Authorized keys"))
        .on_hover_text(
            "Authorized keys: the keys on each server, by server or by key; add or revoke",
        )
        .clicked()
    {
        if m.ws.is_open(Win::Keys) {
            ws(out, WsMsg::Open(Win::Keys));
        } else {
            out.push(Msg::KeysOpen);
        }
    }
    ui.add_space(3.0);
    if nav_button(
        ui,
        shown(Win::HostSync),
        icons::SYNC,
        text("Host Management"),
    )
    .on_hover_text("Host Management: compare and sync with an OpenSSH config")
    .clicked()
    {
        if m.ws.is_open(Win::HostSync) {
            ws(out, WsMsg::Open(Win::HostSync));
        } else {
            out.push(Msg::HostSync(telepad_app::hostsync::HsMsg::Open));
        }
    }
}

// ---------------------------------------------------------------- workspace

/// Content of one page; `focused` = its window is the top one (keys go there).
pub type Content<'a> = dyn FnMut(&mut egui::Ui, &Model, &mut Drafts, Win, bool, &mut Vec<Msg>) + 'a;

/// The workspace: every non-minimized window, then the strip of minimized ones.
pub fn workspace(
    ui: &mut egui::Ui,
    m: &Model,
    d: &mut Drafts,
    out: &mut Vec<Msg>,
    _now: Instant,
    content: &mut Content<'_>,
) {
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();
    let minimized: Vec<&Group> = m.ws.groups.iter().filter(|g| g.minimized).collect();
    let area = if minimized.is_empty() {
        full
    } else {
        Rect::from_min_max(full.min, egui::pos2(full.max.x, full.max.y - STRIP_H))
    };
    d.ws_area = [area.width(), area.height()];
    if m.ws.groups.is_empty() {
        let text = if m.wss.current == telepad_core::prefs::Current::None {
            "Workspace <none>: no windows. Pick a page on the left, or another workspace at the top."
        } else {
            "Pick a page on the left."
        };
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(text).weak());
        });
    }
    if !minimized.is_empty() {
        let strip = Rect::from_min_max(egui::pos2(full.min.x, area.max.y), full.max);
        let mut sui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(strip.shrink2(egui::vec2(4.0, 3.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        for g in &minimized {
            let w = g.active_win();
            let (icon, name) = title(m, w);
            let extra = if g.tabs.len() > 1 {
                format!(" +{}", g.tabs.len() - 1)
            } else {
                String::new()
            };
            if sui
                .button(il(icon, &format!("{name}{extra}")))
                .on_hover_text("Restore")
                .clicked()
            {
                ws(out, WsMsg::Open(w));
            }
        }
    }

    let top = ctx.top_layer_id();
    for (placed, g) in m.ws.groups.iter().filter(|g| !g.minimized).enumerate() {
        let win_id = Id::unique(("ws-win", g.id));
        let layer = LayerId::new(Order::Middle, win_id);
        let active = g.active_win();
        let focused = top == Some(layer);
        // Bring to the front when asked (opened, restored, picked in the sidebar).
        let seen = Id::unique(("ws-raise", g.id));
        if ctx.data(|mem| mem.get_temp::<u64>(seen)) != Some(g.raise) {
            ctx.data_mut(|mem| mem.insert_temp(seen, g.raise));
            ctx.move_to_top(layer);
        }

        // Where the window is: the model's rectangle, or while the title bar
        // or an edge is being dragged, the live one kept in temp data.
        let live_id = Id::unique(("ws-live", g.id));
        let saved = g
            .rect
            .map(|[x, y, w, h]| Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h)));
        let start = saved.unwrap_or_else(|| {
            let step = 28.0 * (placed % 8) as f32;
            let size = default_size(active).min(area.size() - egui::vec2(20.0 + step, 20.0 + step));
            Rect::from_min_size(area.min + egui::vec2(12.0 + step, 12.0 + step), size)
        });
        let live: Option<Rect> = ctx.data(|mem| mem.get_temp(live_id));
        let rect = if g.maximized {
            area.shrink(1.0)
        } else {
            fit(live.unwrap_or(start), area)
        };

        // A plain area with a window frame, exactly `rect`, clipped to the
        // workspace (never over the sidebar or the status bar). Pressing in it
        // brings it to the front (egui does that for areas).
        let style = ctx.global_style();
        let stroke = style.visuals.window_stroke.width;
        let margin = 6.0;
        let inner = rect.shrink(margin + stroke);
        let shown = egui::Area::new(win_id)
            .order(Order::Middle)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(&ctx, |ui| {
                ui.set_clip_rect(area);
                egui::Frame::window(&style)
                    .inner_margin(margin)
                    .shadow(if focused {
                        style.visuals.window_shadow
                    } else {
                        egui::epaint::Shadow::NONE
                    })
                    .show(ui, |ui| {
                        ui.set_clip_rect(inner.intersect(area));
                        ui.set_min_size(inner.size());
                        ui.set_max_size(inner.size());
                        title_bar(ui, m, g, focused, live_id, rect, area, out);
                        ui.separator();
                        // The page gets the rest of the window as its own area,
                        // like a central panel (its panels and tables fit in it).
                        let body = Rect::from_min_max(ui.cursor().min, inner.max);
                        let mut page = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt(("ws-page", g.id))
                                .max_rect(body)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        page.set_clip_rect(body.intersect(area));
                        content(&mut page, m, d, active, focused, out);
                        if !g.maximized {
                            resize_handles(ui, live_id, rect, inner, area, out, active);
                        }
                    });
            });
        let outer = shown.response.rect;
        // Pressing inside a window makes it the one keys go to.
        if ctx.input(|i| i.pointer.any_pressed())
            && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
            && outer.contains(p)
            && ctx.layer_id_at(p) == Some(layer)
            && m.ws.focused != Some(active)
        {
            ws(out, WsMsg::Focus(active));
        }
        if focused && m.ws.focused != Some(active) {
            ws(out, WsMsg::Focus(active));
        }
        // After a move or resize: report it once the mouse is released (a
        // window never moved keeps its cascade place and is not saved).
        // Read again: a snap on release (#168) has just placed it.
        let live: Option<Rect> = ctx.data(|mem| mem.get_temp(live_id));
        if let Some(l) = live
            && !g.maximized
            && !ctx.input(|i| i.pointer.any_down())
        {
            let r = rel(fit(l, area), area);
            let same = g
                .rect
                .is_some_and(|o| o.iter().zip(r).all(|(a, b)| (a - b).abs() <= 0.5));
            if same {
                ctx.data_mut(|mem| mem.remove::<Rect>(live_id));
            } else {
                ws(
                    out,
                    WsMsg::Geometry {
                        win: active,
                        rect: r,
                    },
                );
            }
        }
    }
}

/// One tile of the snap picker (#168): its rectangle and its zones'.
fn layout_tiles(origin: egui::Pos2) -> Vec<(Rect, Vec<(Rect, Zone)>)> {
    const TILE: egui::Vec2 = egui::vec2(84.0, 54.0);
    const PAD: f32 = 8.0;
    SNAP_LAYOUTS
        .iter()
        .enumerate()
        .map(|(k, zones)| {
            let tile = Rect::from_min_size(
                origin + egui::vec2(PAD + k as f32 * (TILE.x + PAD), PAD),
                TILE,
            );
            let inner = tile.shrink(4.0);
            let parts = zones
                .iter()
                .map(|z| {
                    let r = Rect::from_min_size(
                        inner.min + egui::vec2(z[0] * inner.width(), z[1] * inner.height()),
                        egui::vec2(z[2] * inner.width(), z[3] * inner.height()),
                    )
                    .shrink(1.5);
                    (r, *z)
                })
                .collect();
            (tile, parts)
        })
        .collect()
}

/// Size of the picker holding every layout tile.
fn picker_size() -> egui::Vec2 {
    let n = SNAP_LAYOUTS.len() as f32;
    egui::vec2(8.0 + n * (84.0 + 8.0), 54.0 + 16.0)
}

/// Draws the layout tiles at `origin`; returns the zone under `pointer`.
fn paint_tiles(
    painter: &egui::Painter,
    v: &egui::Visuals,
    origin: egui::Pos2,
    pointer: Option<egui::Pos2>,
) -> Option<Zone> {
    let mut hit = None;
    for (tile, parts) in layout_tiles(origin) {
        painter.rect_filled(tile, 4.0, v.extreme_bg_color);
        painter.rect_stroke(
            tile,
            4.0,
            v.widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        for (r, z) in parts {
            let over = pointer.is_some_and(|p| r.contains(p));
            if over {
                hit = Some(z);
            }
            let fill = if over {
                v.selection.bg_fill
            } else {
                v.widgets.inactive.weak_bg_fill
            };
            painter.rect_filled(r, 2.0, fill);
            painter.rect_stroke(
                r,
                2.0,
                v.widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
    hit
}

/// While a window is dragged (#168): the layout picker at the top centre
/// of the workspace once the pointer comes near the top, else the edge
/// zones. Returns the zone the window would snap to.
fn drag_zone(ctx: &egui::Context, id: Id, area: Rect, p: egui::Pos2) -> Option<Zone> {
    let size = picker_size();
    let origin = egui::pos2(area.center().x - size.x / 2.0, area.top() + 6.0);
    let picker = Rect::from_min_size(origin, size);
    let near_top =
        p.y - area.top() <= 140.0 && p.x >= picker.left() - 60.0 && p.x <= picker.right() + 60.0;
    let mut hit = None;
    if near_top {
        egui::Area::new(id.with("snap-picker"))
            .order(Order::Tooltip)
            .fixed_pos(origin)
            .interactable(false)
            .show(ctx, |ui| {
                let v = ui.visuals().clone();
                let painter = ui.painter();
                painter.rect_filled(picker, 8.0, v.window_fill);
                painter.rect_stroke(picker, 8.0, v.window_stroke, egui::StrokeKind::Inside);
                hit = paint_tiles(painter, &v, origin, Some(p));
                ui.allocate_rect(picker, Sense::hover());
            });
    }
    hit.or_else(|| {
        if picker.contains(p) {
            return None;
        }
        let r = p - area.min;
        edge_zone(r.x, r.y, area.width(), area.height())
    })
}

/// The translucent rectangle where the dragged window will land (#168).
fn snap_preview(ctx: &egui::Context, area: Rect, z: Zone) {
    let r = zone_rect(z, area.width(), area.height());
    let rect = Rect::from_min_size(area.min + egui::vec2(r[0], r[1]), egui::vec2(r[2], r[3]));
    let v = ctx.global_style().visuals.clone();
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("snap-preview")));
    painter.rect_filled(
        rect.shrink(2.0),
        8.0,
        v.selection.bg_fill.gamma_multiply(0.35),
    );
    painter.rect_stroke(
        rect.shrink(2.0),
        8.0,
        egui::Stroke::new(2.0, v.selection.bg_fill),
        egui::StrokeKind::Inside,
    );
}

/// The snap layouts under the maximize button (#168): shown while the
/// pointer is on the button or on them; a click on a zone picks it.
fn layout_flyout(ctx: &egui::Context, id: Id, button: &egui::Response) -> Option<Zone> {
    let open_id = id.with("flyout");
    let open = ctx
        .data(|mem| mem.get_temp::<bool>(open_id))
        .unwrap_or(false);
    if !open && !button.hovered() {
        return None;
    }
    let size = picker_size();
    let origin = egui::pos2(button.rect.right() - size.x, button.rect.bottom() + 2.0);
    let rect = Rect::from_min_size(origin, size);
    let pointer = ctx.pointer_latest_pos();
    let keep = pointer.is_some_and(|p| rect.expand(4.0).contains(p) || button.rect.expand(4.0).contains(p))
        // Room between the button and the flyout.
        || pointer.is_some_and(|p| Rect::from_min_max(egui::pos2(rect.left(), button.rect.top()), rect.min + egui::vec2(size.x, 0.0)).contains(p));
    if !keep {
        ctx.data_mut(|mem| mem.remove::<bool>(open_id));
        return None;
    }
    ctx.data_mut(|mem| mem.insert_temp(open_id, true));
    let mut picked = None;
    egui::Area::new(id.with("flyout-area"))
        .order(Order::Tooltip)
        .fixed_pos(origin)
        .show(ctx, |ui| {
            let v = ui.visuals().clone();
            let resp = ui.allocate_rect(rect, Sense::click());
            let painter = ui.painter();
            painter.rect_filled(rect, 8.0, v.window_fill);
            painter.rect_stroke(rect, 8.0, v.window_stroke, egui::StrokeKind::Inside);
            let hit = paint_tiles(painter, &v, origin, pointer);
            if hit.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                picked = hit;
            }
        });
    if picked.is_some() {
        ctx.data_mut(|mem| mem.remove::<bool>(open_id));
    }
    picked
}

/// Window rectangle relative to the workspace, as the model keeps it.
fn rel(r: Rect, area: Rect) -> [f32; 4] {
    [
        (r.min.x - area.min.x).round(),
        (r.min.y - area.min.y).round(),
        r.width().round(),
        r.height().round(),
    ]
}

/// At least 320 × 200, no bigger than the workspace, never over the sidebar
/// and the title bar always reachable (the main window may have become smaller).
fn fit(r: Rect, area: Rect) -> Rect {
    let size = r
        .size()
        .max(egui::vec2(320.0, 200.0))
        .min(area.size().max(egui::vec2(320.0, 200.0)));
    let x = r
        .min
        .x
        .clamp(area.min.x, (area.max.x - 120.0).max(area.min.x));
    let y = r
        .min
        .y
        .clamp(area.min.y, (area.max.y - TITLE_H - 8.0).max(area.min.y));
    Rect::from_min_size(egui::pos2(x, y), size)
}

/// Right edge, bottom edge and the bottom-right corner resize the window.
fn resize_handles(
    ui: &mut egui::Ui,
    live_id: Id,
    rect: Rect,
    inner: Rect,
    area: Rect,
    out: &mut Vec<Msg>,
    active: Win,
) {
    const GRIP: f32 = 7.0;
    ui.set_clip_rect(rect.intersect(area));
    let inner = inner.expand(4.0);
    let edges = [
        (
            "r",
            Rect::from_min_max(
                egui::pos2(inner.max.x - GRIP, inner.min.y + TITLE_H),
                egui::pos2(inner.max.x, inner.max.y - GRIP),
            ),
            egui::vec2(1.0, 0.0),
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            "b",
            Rect::from_min_max(
                egui::pos2(inner.min.x, inner.max.y - GRIP),
                egui::pos2(inner.max.x - GRIP * 2.0, inner.max.y),
            ),
            egui::vec2(0.0, 1.0),
            egui::CursorIcon::ResizeVertical,
        ),
        (
            "rb",
            Rect::from_min_max(
                egui::pos2(inner.max.x - GRIP * 2.0, inner.max.y - GRIP * 2.0),
                inner.max,
            ),
            egui::vec2(1.0, 1.0),
            egui::CursorIcon::ResizeNwSe,
        ),
    ];
    for (name, r, dir, cursor) in edges {
        let resp = ui.interact(r, live_id.with(name), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ws(out, WsMsg::Focus(active));
        }
        if resp.dragged() {
            let d = resp.drag_delta() * dir;
            let base: Rect = ui.ctx().data(|mem| mem.get_temp(live_id)).unwrap_or(rect);
            let next = Rect::from_min_size(base.min, base.size() + d);
            ui.ctx()
                .data_mut(|mem| mem.insert_temp(live_id, fit(next, area)));
        }
    }
    if ui.is_rect_visible(inner) {
        let c = inner.max - egui::vec2(3.0, 3.0);
        let stroke = egui::Stroke::new(1.0, ui.visuals().weak_text_color());
        for k in [4.0, 8.0] {
            ui.painter()
                .line_segment([c - egui::vec2(k, 0.0), c - egui::vec2(0.0, k)], stroke);
        }
    }
}

/// Tabs (or the title), drag handle and the window buttons.
#[allow(clippy::too_many_arguments)]
fn title_bar(
    ui: &mut egui::Ui,
    m: &Model,
    g: &Group,
    focused: bool,
    live_id: Id,
    win_rect: Rect,
    area: Rect,
    out: &mut Vec<Msg>,
) {
    let active = g.active_win();
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), TITLE_H), Sense::hover());
    let [on, off] = crate::widgets::pal().title;
    let bg = if focused { on } else { off };
    ui.painter().rect_filled(rect, 4.0, bg);
    // Drag anywhere on the bar that is not a tab or button; double-click maximizes.
    let handle = ui.interact(rect, live_id.with("handle"), Sense::click_and_drag());
    let ctx = ui.ctx().clone();
    let snap_id = live_id.with("snap");
    let cancel_id = live_id.with("snap-cancel");
    if handle.double_clicked() {
        ws(out, WsMsg::ToggleMaximize(active));
    } else if !g.maximized && handle.dragged() {
        let live: Option<Rect> = ctx.data(|mem| mem.get_temp(live_id));
        let base = match (live, g.unsnap, ctx.pointer_latest_pos()) {
            // A snapped window dragged out of its zone gets its old size
            // back, under the pointer at the same place across (#168).
            (None, Some([_, _, w, h]), Some(p)) => {
                let across = ((p.x - win_rect.left()) / win_rect.width()).clamp(0.0, 1.0);
                Rect::from_min_size(
                    egui::pos2(p.x - across * w, win_rect.top()),
                    egui::vec2(w, h),
                )
            }
            _ => live.unwrap_or(win_rect),
        };
        let next = fit(base.translate(handle.drag_delta()), area);
        ctx.data_mut(|mem| mem.insert_temp(live_id, next));
        // Snap layouts (#168): a zone under the pointer, Esc cancels.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.data_mut(|mem| mem.insert_temp(cancel_id, true));
        }
        let cancelled = ctx
            .data(|mem| mem.get_temp::<bool>(cancel_id))
            .unwrap_or(false);
        let zone = match ctx.pointer_latest_pos() {
            Some(p) if !cancelled => drag_zone(&ctx, live_id, area, p),
            _ => None,
        };
        ctx.data_mut(|mem| match zone {
            Some(z) => {
                mem.insert_temp(snap_id, z);
            }
            None => mem.remove::<Zone>(snap_id),
        });
        if let Some(z) = zone {
            snap_preview(&ctx, area, z);
        }
    }
    if handle.drag_started() {
        ws(out, WsMsg::Focus(active));
    }
    if handle.drag_stopped() {
        let zone: Option<Zone> = ctx.data(|mem| mem.get_temp(snap_id));
        ctx.data_mut(|mem| {
            mem.remove::<Zone>(snap_id);
            mem.remove::<bool>(cancel_id);
        });
        if let Some(z) = zone {
            let from = g.rect.unwrap_or_else(|| rel(win_rect, area));
            if z == ZONE_MAX {
                ctx.data_mut(|mem| mem.remove::<Rect>(live_id));
                ws(out, WsMsg::ToggleMaximize(active));
            } else {
                let r = zone_rect(z, area.width(), area.height());
                // The release report then finds the window already there.
                let abs =
                    Rect::from_min_size(area.min + egui::vec2(r[0], r[1]), egui::vec2(r[2], r[3]));
                ctx.data_mut(|mem| mem.insert_temp(live_id, abs));
                ws(
                    out,
                    WsMsg::Snap {
                        win: active,
                        rect: r,
                        from,
                    },
                );
            }
        }
    }

    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(6.0, 2.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let ui = &mut bar;
    if ui
        .add(egui::Button::new(icons::X).frame(false))
        .on_hover_text("Close")
        .clicked()
    {
        out.push(close_msg(active));
    }
    let max_icon = if g.maximized {
        icons::CORNERS_IN
    } else {
        icons::CORNERS_OUT
    };
    // No tooltip: the snap layouts show under it on hover (#168).
    let max = ui.add(egui::Button::new(max_icon).frame(false));
    // Snap layouts on the maximize button (#168, Windows 11 style).
    if let Some(z) = layout_flyout(&ctx, live_id, &max) {
        let from = g.rect.unwrap_or_else(|| rel(win_rect, area));
        let r = zone_rect(z, area.width(), area.height());
        ws(
            out,
            WsMsg::Snap {
                win: active,
                rect: r,
                from,
            },
        );
    }
    if max.clicked() {
        ws(out, WsMsg::ToggleMaximize(active));
    }
    if ui
        .add(egui::Button::new(icons::MINUS).frame(false))
        .on_hover_text("Minimize")
        .clicked()
    {
        ws(out, WsMsg::Minimize(active));
    }
    // Tab into another window / take this tab out.
    let others: Vec<Win> =
        m.ws.groups
            .iter()
            .filter(|o| o.id != g.id)
            .map(Group::active_win)
            .collect();
    if !others.is_empty() || g.tabs.len() > 1 {
        ui.menu_button(icons::STACK, |ui| {
            if g.tabs.len() > 1 && ui.button(il(icons::APP_WINDOW, "Own window")).clicked() {
                ws(out, WsMsg::Detach(active));
                ui.close();
            }
            if !others.is_empty() {
                ui.label(RichText::new("Show as a tab in").weak().small());
            }
            for o in others {
                let (icon, name) = title(m, o);
                if ui.button(il(icon, &name)).clicked() {
                    ws(
                        out,
                        WsMsg::Merge {
                            win: active,
                            into: o,
                        },
                    );
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Tabs: put this page in another window, or in its own");
    }
    ui.add_space(6.0);
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        // "Hosts - [name]": ← and the title go back to its parent page (#169).
        let back = match active {
            Win::SetHosts(id) => telepad_app::sets::back_target(m, id).map(|w| (id, w)),
            _ => None,
        };
        if let Some((id, parent)) = back {
            let tip = format!("Back to {}", title(m, parent).1);
            if ui
                .add(egui::Button::new(icons::ARROW_LEFT).frame(false))
                .on_hover_text(&tip)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                out.push(Msg::Sets(telepad_app::sets::StMsg::Back(id)));
            }
            if g.tabs.len() == 1 {
                let (icon, name) = title(m, active);
                if ui
                    .add(
                        egui::Label::new(RichText::new(il(icon, &name)).strong())
                            .sense(Sense::click()),
                    )
                    .on_hover_text(&tip)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    out.push(Msg::Sets(telepad_app::sets::StMsg::Back(id)));
                }
                return;
            }
        }
        if g.tabs.len() == 1 {
            let (icon, name) = title(m, active);
            ui.label(RichText::new(il(icon, &name)).strong());
            return;
        }
        for &t in &g.tabs {
            let (icon, name) = title(m, t);
            let is = t == active;
            let label = RichText::new(il(icon, &name));
            let r = ui.selectable_label(is, if is { label.strong() } else { label });
            if r.clicked() && !is {
                ws(out, WsMsg::ActivateTab(t));
            }
            if r.middle_clicked() {
                out.push(close_msg(t));
            }
            r.on_hover_text("Middle-click closes this tab");
        }
    });
}
