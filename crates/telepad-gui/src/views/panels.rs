//! Top bar, status bar and the host list on the left.

use std::time::Instant;

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Model, Msg, UpdatePhase, matches_filter};
use telepad_core::update::CURRENT_VERSION;

use crate::icons::{self, label as il};
use crate::widgets::*;

pub fn top_bar(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading(il(icons::DATABASE, "Telepad"));
        ui.separator();
        ui.menu_button(il(icons::MENU, "Hosts"), |ui| {
            for (icon, text, msg) in [
                (icons::PLUS, "Add host…", Msg::AddHost),
                (icons::IMPORT, "Import…", Msg::Import),
                (icons::EXPORT, "Export…", Msg::Export),
            ] {
                if ui.button(il(icon, text)).clicked() {
                    out.push(msg);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(il(icons::IMPORT, "Software update…")).clicked() {
                out.push(Msg::UpdateWindow(true));
                ui.close();
            }
        });
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
        ui.label(RichText::new(icons::SEARCH).size(16.0));
        ui.add(
            egui::TextEdit::singleline(&mut d.filter)
                .desired_width(180.0)
                .hint_text("host, IP, OS, container"),
        );
        ui.separator();
        ui.label(
            RichText::new(format!("{} {} up", icons::DOT, m.online_count()))
                .color(usage_color(0.0)),
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
    });
    ui.add_space(4.0);
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

pub fn host_list(ui: &mut egui::Ui, m: &Model, filter: &str, out: &mut Vec<Msg>) {
    ui.add_space(4.0);
    if ui
        .selectable_label(
            m.selected.is_none(),
            RichText::new(il(icons::OVERVIEW, "Overview — all hosts")).strong(),
        )
        .clicked()
    {
        out.push(Msg::Select(None));
    }
    if ui.button(il(icons::PLUS, "Add host")).clicked() {
        out.push(Msg::AddHost);
    }
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, h) in m.hosts.iter().enumerate() {
                if !matches_filter(filter, h) {
                    continue;
                }
                let resp = ui
                    .horizontal(|ui| {
                        status_dot(ui, h);
                        let mut name = RichText::new(&h.entry.alias).strong();
                        if h.entry.unmonitored {
                            name = name.weak();
                        }
                        icons::os_logo(ui, host_os(h), 16.0);
                        let label = ui.selectable_label(m.selected == Some(i), name);
                        ui.label(RichText::new(&h.entry.hostname).weak().small());
                        label
                    })
                    .inner;
                if resp.clicked() {
                    out.push(Msg::Select(Some(i)));
                }
                resp.context_menu(|ui| {
                    for (icon, text, msg) in [
                        (icons::EDIT, "Edit", Msg::Edit(i)),
                        (
                            icons::CLONE,
                            "Clone (User, Port, IdentityFile)",
                            Msg::Clone(i),
                        ),
                        (icons::TRASH, "Delete", Msg::Delete(i)),
                    ] {
                        if ui.button(il(icon, text)).clicked() {
                            out.push(msg);
                            ui.close();
                        }
                    }
                });
                if !h.entry.note.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(RichText::new(&h.entry.note).small().italics());
                    });
                }
                if let Some(r) = &h.report {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        if let Some(c) = r.cpu_usage_pct {
                            bar(ui, c, format!("CPU {c:.0}%"), 80.0);
                        }
                        if let Some(mem) = &r.memory {
                            let p = mem.used_pct();
                            bar(ui, p, format!("RAM {p:.0}%"), 80.0);
                        }
                        if let Some(disk) = r
                            .disks
                            .iter()
                            .max_by(|a, b| a.used_pct().total_cmp(&b.used_pct()))
                        {
                            let p = disk.used_pct();
                            bar(ui, p, format!("Disk {p:.0}%"), 80.0);
                        }
                    });
                } else if let Some(e) = &h.error {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(RichText::new(e).small().color(usage_color(95.0)));
                    });
                }
                ui.add_space(2.0);
            }
        });
}
