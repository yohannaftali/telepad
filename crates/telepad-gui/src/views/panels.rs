//! Top bar and status bar (the page sidebar is in `workspace`).

use std::time::Instant;

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Model, Msg, UpdatePhase};
use telepad_core::store::ThemePref;
use telepad_core::update::CURRENT_VERSION;

use crate::icons::{self, label as il};
use crate::widgets::*;

pub fn top_bar(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    ui.add_space(4.0);
    // The menu keeps the top-right corner; the rest wraps onto more lines in
    // the width left of it on narrow windows.
    const MENU_WIDTH: f32 = 44.0;
    let full = ui.available_rect_before_wrap();
    let menu_rect = egui::Rect::from_min_size(
        egui::pos2(full.right() - MENU_WIDTH, full.top()),
        egui::vec2(MENU_WIDTH, 34.0),
    );
    let mut menu_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(menu_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Min)),
    );
    hosts_menu(&mut menu_ui, d.theme, out);
    ui.expand_to_include_rect(menu_ui.min_rect());
    let row = egui::vec2(
        (full.width() - MENU_WIDTH - 8.0).max(100.0),
        ui.spacing().interact_size.y,
    );
    ui.allocate_ui_with_layout(
        row,
        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
        |ui| top_bar_items(ui, m, d, out),
    );
    ui.add_space(4.0);
}

/// ☰ at the top right: add, import, export, settings, software update.
fn hosts_menu(ui: &mut egui::Ui, theme: ThemePref, out: &mut Vec<Msg>) {
    ui.menu_button(RichText::new(icons::MENU).size(20.0), |ui| {
        for (group, items) in [
            ("Add", vec![(icons::PLUS, "Add one host…", Msg::AddHost)]),
            (
                "Import",
                vec![
                    (icons::IMPORT, "Import from a file…", Msg::Import),
                    (icons::IMPORT, "Import ~/.ssh/config", Msg::ImportSshConfig),
                    (icons::COPY, "Import from pasted text…", Msg::PasteImport),
                ],
            ),
            ("Export", vec![(icons::EXPORT, "Export…", Msg::Export)]),
            (
                "Compare",
                vec![(
                    icons::SYNC,
                    "Host Management… (compare with ~/.ssh/config)",
                    Msg::HostSync(telepad_app::hostsync::HsMsg::Open),
                )],
            ),
            (
                "Access",
                vec![(
                    icons::KEY,
                    "Authorized keys… (list, add, revoke)",
                    Msg::KeysOpen,
                )],
            ),
        ] {
            ui.label(RichText::new(group).weak().small());
            for (icon, text, msg) in items {
                if ui.button(il(icon, text)).clicked() {
                    out.push(msg);
                    ui.close();
                }
            }
            ui.separator();
        }
        // Settings (#92): the theme left the top bar for the window picker.
        ui.label(RichText::new("Settings").weak().small());
        ui.horizontal(|ui| {
            ui.label(il(icons::CIRCLE_HALF, "Theme"));
            for t in ThemePref::ALL {
                if ui
                    .selectable_label(theme == t, t.title())
                    .on_hover_text("System follows your Windows / macOS setting")
                    .clicked()
                {
                    out.push(Msg::SetTheme(t));
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.button(il(icons::IMPORT, "Software update…")).clicked() {
            out.push(Msg::UpdateWindow(true));
            ui.close();
        }
    })
    .response
    .on_hover_text(
        "Hosts: add, import, export, Host Management · Authorized keys · Settings (theme) · Software update",
    );
}

fn top_bar_items(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    ui.heading(il(icons::DATABASE, "Telepad"));
    ui.separator();
    match &m.update.phase {
        UpdatePhase::Installed(v) => {
            if ui
                .button(
                    RichText::new(il(icons::REFRESH, &format!("Restart for {v}")))
                        .color(usage_color(0.0))
                        .strong(),
                )
                .clicked()
            {
                out.push(Msg::Restart);
            }
        }
        UpdatePhase::Installing => {
            ui.spinner();
            ui.label("updating…");
        }
        _ => {
            if let Some(c) = m.update.banner()
                && ui
                    .button(
                        RichText::new(il(
                            icons::IMPORT,
                            &format!("Update {} available", c.version),
                        ))
                        .color(usage_color(0.0))
                        .strong(),
                    )
                    .on_hover_text(format!("Installed: {CURRENT_VERSION}"))
                    .clicked()
            {
                out.push(Msg::UpdateWindow(true));
            }
        }
    }
    if ui
        .add_enabled(
            m.busy == 0,
            egui::Button::new(il(icons::REFRESH, "Refresh all")),
        )
        .clicked()
    {
        out.push(Msg::RefreshAll);
    }
    ui.checkbox(&mut d.auto_refresh, "Auto every");
    ui.add(
        egui::DragValue::new(&mut d.interval_secs)
            .range(15..=3600)
            .suffix(" s"),
    );
    ui.separator();
    window_picker(ui, m, d, out);
    ui.separator();
    ui.label(
        RichText::new(format!("{} {} up", icons::DOT, m.online_count())).color(usage_color(0.0)),
    );
    ui.label(
        RichText::new(format!("{} {} down", icons::DOT, m.offline_count()))
            .color(usage_color(95.0)),
    );
    ui.label(format!("{} hosts", m.hosts.len()));
    if m.busy > 0 {
        ui.spinner();
        ui.label(format!("{} loading", m.busy));
    }
}

/// Every open window, searchable (#92): pick one to bring it to the front
/// (a minimized one is restored).
fn window_picker(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    use super::picker::{Item, Picked, Spec, picker, shorten};
    use telepad_app::workspace::WsMsg;
    let wins = m.ws.open_windows();
    let button = match m.ws.focused.filter(|&w| m.ws.is_open(w)) {
        _ if wins.is_empty() => il(icons::APP_WINDOW, "No windows"),
        Some(w) => {
            let (icon, name) = super::workspace::title(m, w);
            format!("{icon} {} · {}", shorten(&name, 28), wins.len())
        }
        None => il(icons::APP_WINDOW, &format!("Windows · {}", wins.len())),
    };
    let items: Vec<Item> = wins
        .iter()
        .map(|&(w, minimized)| {
            let (icon, label) = super::workspace::title(m, w);
            Item {
                icon,
                label,
                current: m.ws.focused == Some(w),
                note: if minimized { "minimized" } else { "" },
                delete: None,
            }
        })
        .collect();
    let spec = Spec {
        button,
        tip: "Open windows: search and pick one to bring it to the front",
        enabled: !wins.is_empty(),
        placeholder: "Search windows…",
        empty: "No matching windows",
        width: 320.0,
    };
    if let Some(Picked::Item(i)) = picker(ui, spec, &items, &[], &mut d.win_picker) {
        out.push(Msg::Ws(WsMsg::Open(wins[i].0)));
    }
}

pub fn status_bar(ui: &mut egui::Ui, m: &Model, now: Instant) {
    ui.horizontal(|ui| {
        if let Some(db) = &m.db_path {
            let label = ui.label(RichText::new(format!("database: {}", db.display())).weak());
            if let Some(cfg) = &m.ssh_config_path {
                label.on_hover_text(format!("ssh uses the generated file {}", cfg.display()));
            }
        }
        ui.separator();
        let v = ui.label(RichText::new(format!("v{CURRENT_VERSION}")).weak());
        if let Some(e) = &m.update.error {
            let tip = format!("Update check failed: {e}");
            v.on_hover_text(&tip);
            ui.label(RichText::new(icons::WARNING).weak())
                .on_hover_text(tip);
        }
        if let Some(text) = m.message_at(now) {
            ui.separator();
            ui.label(text);
        }
    });
}
