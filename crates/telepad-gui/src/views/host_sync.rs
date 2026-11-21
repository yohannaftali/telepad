//! Host Management (#58): Telepad's hosts (left) next to an OpenSSH config
//! file (right), copy / edit / delete per host or all at once, then Apply.

use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use telepad_app::hostsync::{HostSync, HostSyncDraft, HsMsg, Mode, Pending, Side};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::config::{Access, HostEntry};
use telepad_core::hostsync::{Row, RowState};

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button, act_small, act_text, usage_color};

const ROW: f32 = 26.0;

fn send(out: &mut Vec<Msg>, m: HsMsg) {
    out.push(Msg::HostSync(m));
}

fn red() -> Color32 {
    usage_color(95.0)
}

fn green() -> Color32 {
    usage_color(10.0)
}

fn amber() -> Color32 {
    usage_color(80.0)
}

fn summary(h: &HostEntry) -> String {
    let mut s = h.target();
    if h.port != h.access.default_port() {
        s.push_str(&format!(":{}", h.port));
    }
    if h.access == Access::Rdp {
        s.push_str(" (RDP)");
    }
    s
}

/// The Host Management page (an inner window of the workspace).
pub fn host_sync_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(hs) = &m.host_sync else {
        return;
    };
    body(ui, m, hs, &mut d.host_sync, out);
}

/// Its host form and apply preview, over the workspace.
pub fn host_sync_dialogs(ctx: &egui::Context, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(hs) = &m.host_sync else {
        return;
    };
    form_window(ctx, &mut d.host_sync, out);
    preview_window(ctx, hs, out);
}

fn body(ui: &mut egui::Ui, m: &Model, hs: &HostSync, d: &mut HostSyncDraft, out: &mut Vec<Msg>) {
    ui.columns(2, |cols| {
        cols[0].horizontal(|ui| {
            ui.label(RichText::new(il(icons::DATABASE, "Telepad database")).strong());
            ui.label(RichText::new(format!("{} hosts", m.hosts.len())).weak());
        });
        cols[0].label(
            RichText::new(
                m.db_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            )
            .weak()
            .small(),
        );
        // Add / import / export (#93; they left the ☰ menu): straight into
        // Telepad's database, not pending like "Add to Telepad…" below.
        cols[0].horizontal_wrapped(|ui| {
            if act_small(ui, true, Act::Add, &il(icons::PLUS, "Add host…"))
                .on_hover_text("Add one host to Telepad now (the host editor)")
                .clicked()
            {
                out.push(Msg::AddHost);
            }
            ui.menu_button(il(icons::IMPORT, "Import"), |ui| {
                for (icon, text, tip, msg) in [
                    (
                        icons::IMPORT,
                        "From a file…",
                        "An OpenSSH config or a Telepad JSON export",
                        Msg::Import,
                    ),
                    (
                        icons::IMPORT,
                        "From ~/.ssh/config",
                        "Your own OpenSSH client config, in one click",
                        Msg::ImportSshConfig,
                    ),
                    (
                        icons::COPY,
                        "From pasted text…",
                        "Paste a config, JSON or one host per line",
                        Msg::PasteImport,
                    ),
                ] {
                    if ui.button(il(icon, text)).on_hover_text(tip).clicked() {
                        out.push(msg);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Import hosts into Telepad");
            if ui
                .small_button(il(icons::EXPORT, "Export…"))
                .on_hover_text("Telepad's hosts as JSON (re-importable) or an OpenSSH config")
                .clicked()
            {
                out.push(Msg::Export);
            }
        });
        cols[1].horizontal(|ui| {
            ui.label(RichText::new(il(icons::DESKTOP, "OpenSSH config")).strong());
            if let Some(f) = &hs.file {
                ui.label(RichText::new(format!("{} hosts", f.hosts().len())).weak());
            }
            if hs.loading {
                ui.spinner();
            }
        });
        cols[1].horizontal_wrapped(|ui| {
            ui.label(RichText::new(&hs.path).monospace().small());
            if ui.small_button(il(icons::IMPORT, "File…")).clicked() {
                send(out, HsMsg::Browse { folder: false });
            }
            if ui
                .small_button(il(icons::IMPORT, "Folder…"))
                .on_hover_text("Its `config` file")
                .clicked()
            {
                send(out, HsMsg::Browse { folder: true });
            }
            if ui.small_button(il(icons::REFRESH, "Reload")).clicked() {
                send(out, HsMsg::Reload);
            }
        });
    });
    ui.separator();

    // Toolbar: filter, bulk actions, selection actions.
    let selected: Vec<String> = d.selected.iter().cloned().collect();
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut d.filter)
                .hint_text(format!("{} filter", icons::SEARCH))
                .desired_width(160.0),
        );
        ui.checkbox(&mut d.only_differences, "Only differences");
        ui.separator();
        ui.menu_button(il(icons::ARROW_RIGHT, "Telepad → config"), |ui| {
            if ui
                .button("Merge")
                .on_hover_text("Add missing hosts and update different ones; keep hosts only the config has")
                .clicked()
            {
                send(out, HsMsg::Bulk { from: Side::Telepad, mode: Mode::Merge });
                ui.close();
            }
            if ui
                .button(act_text(Act::Danger, "Replace"))
                .on_hover_text("The config's hosts become a copy of Telepad's; hosts only the config has are removed (Host *, Match and shared lines stay)")
                .clicked()
            {
                send(out, HsMsg::Bulk { from: Side::Telepad, mode: Mode::Replace });
                ui.close();
            }
        });
        ui.menu_button(il(icons::ARROW_LEFT, "Config → Telepad"), |ui| {
            if ui
                .button("Merge")
                .on_hover_text("Add missing hosts and update different ones; keep hosts only Telepad has")
                .clicked()
            {
                send(out, HsMsg::Bulk { from: Side::Config, mode: Mode::Merge });
                ui.close();
            }
            if ui
                .button(act_text(Act::Danger, "Replace"))
                .on_hover_text("Telepad's hosts become a copy of the config's; hosts only Telepad has are removed")
                .clicked()
            {
                send(out, HsMsg::Bulk { from: Side::Config, mode: Mode::Replace });
                ui.close();
            }
        });
        if ui
            .button(il(icons::SYNC, "Sync"))
            .on_hover_text("Both sides get every host; for hosts that differ you pick a side with → or ←")
            .clicked()
        {
            send(out, HsMsg::Sync);
        }
        ui.separator();
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add to Telepad…")).clicked() {
            send(out, HsMsg::Add(Side::Telepad));
        }
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add to config…")).clicked() {
            send(out, HsMsg::Add(Side::Config));
        }
    });
    if !selected.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} selected:", selected.len())).strong());
            if ui
                .button(il(icons::ARROW_RIGHT, "copy to config"))
                .clicked()
            {
                send(out, HsMsg::ToConfig(selected.clone()));
            }
            if ui
                .button(il(icons::ARROW_LEFT, "copy to Telepad"))
                .clicked()
            {
                send(out, HsMsg::ToTelepad(selected.clone()));
            }
            if act_button(
                ui,
                true,
                Act::Danger,
                &il(icons::TRASH, "delete in Telepad"),
            )
            .clicked()
            {
                send(
                    out,
                    HsMsg::Delete {
                        side: Side::Telepad,
                        aliases: selected.clone(),
                    },
                );
            }
            if act_button(ui, true, Act::Danger, &il(icons::TRASH, "delete in config")).clicked() {
                send(
                    out,
                    HsMsg::Delete {
                        side: Side::Config,
                        aliases: selected.clone(),
                    },
                );
            }
            if ui.button(il(icons::UNDO, "undo")).clicked() {
                send(out, HsMsg::Undo(selected.clone()));
            }
            if ui.button("clear selection").clicked() {
                d.selected.clear();
            }
        });
    }
    if let Some(e) = &hs.error {
        ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
    } else if let Some(n) = &hs.notice {
        ui.colored_label(green(), format!("{} {n}", icons::CHECK_CIRCLE));
    }

    // Footer first so the table can take the rest of the height.
    egui::Panel::bottom(egui::Id::unique("host-sync-footer")).show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let n = hs.pending_count();
            ui.label(if n == 0 {
                RichText::new("No pending changes").weak()
            } else {
                RichText::new(format!(
                    "{n} pending change(s): {} in Telepad, {} in the config",
                    hs.telepad.len(),
                    hs.config.len()
                ))
                .strong()
            });
            if !hs.conflicts.is_empty() {
                ui.colored_label(
                    amber(),
                    format!(
                        "{} {} host(s) differ: choose → or ←",
                        icons::WARNING,
                        hs.conflicts.len()
                    ),
                );
            }
            if ui
                .add_enabled(
                    n > 0 && !hs.applying,
                    egui::Button::new(il(icons::SAVE, "Preview and apply…")),
                )
                .on_hover_text("Shows the changes first; nothing is written before Apply")
                .clicked()
            {
                send(out, HsMsg::Preview);
            }
            if ui
                .add_enabled(n > 0, egui::Button::new(il(icons::UNDO, "Discard")))
                .clicked()
            {
                send(out, HsMsg::Discard);
            }
            if hs.applying {
                ui.spinner();
            }
        });
    });

    let filter = d.filter.trim().to_lowercase();
    let rows: Vec<&Row> = hs
        .rows
        .iter()
        .filter(|r| !d.only_differences || r.state != RowState::Same || pending(hs, &r.alias))
        .filter(|r| {
            filter.is_empty()
                || r.alias.to_lowercase().contains(&filter)
                || [&r.db, &r.cfg].iter().filter_map(|h| h.as_ref()).any(|h| {
                    summary(h).to_lowercase().contains(&filter)
                        || h.note.to_lowercase().contains(&filter)
                })
        })
        .collect();
    let shared = hs
        .file
        .as_ref()
        .map(|f| f.shared_aliases())
        .unwrap_or_default();
    TableBuilder::new(ui)
        .id_salt("host-sync-table")
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .column(Column::exact(28.0))
        .column(Column::initial(330.0).at_least(160.0).clip(true))
        .column(Column::exact(150.0))
        .column(Column::initial(330.0).at_least(160.0).clip(true))
        .column(Column::remainder().at_least(120.0).clip(true))
        .header(ROW, |mut h| {
            h.col(|ui| {
                let all = !rows.is_empty() && rows.iter().all(|r| d.selected.contains(&r.alias));
                let mut on = all;
                if ui
                    .checkbox(&mut on, "")
                    .on_hover_text("Select all shown")
                    .changed()
                {
                    for r in &rows {
                        if on {
                            d.selected.insert(r.alias.clone());
                        } else {
                            d.selected.remove(&r.alias);
                        }
                    }
                }
            });
            h.col(|ui| {
                ui.strong(il(icons::DATABASE, "Telepad"));
            });
            h.col(|ui| {
                ui.strong("");
            });
            h.col(|ui| {
                ui.strong(il(icons::DESKTOP, "Config"));
            });
            h.col(|ui| {
                ui.strong("Differences");
            });
        })
        .body(|body| {
            body.rows(ROW, rows.len(), |mut row| {
                let r = rows[row.index()];
                let conflict = hs.conflicts.contains(&r.alias);
                row.col(|ui| {
                    let mut on = d.selected.contains(&r.alias);
                    if ui.checkbox(&mut on, "").changed() {
                        if on {
                            d.selected.insert(r.alias.clone());
                        } else {
                            d.selected.remove(&r.alias);
                        }
                    }
                });
                row.col(|ui| side_cell(ui, hs, r, Side::Telepad, false, out));
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        let (icon, color, tip) = match r.state {
                            RowState::Same => (icons::EQUALS, green(), "Same on both sides"),
                            RowState::Different => (icons::NOT_EQUALS, amber(), "Different"),
                            RowState::OnlyTelepad => (
                                icons::DATABASE,
                                ui.visuals().weak_text_color(),
                                "Only in Telepad",
                            ),
                            RowState::OnlyConfig => (
                                icons::DESKTOP,
                                ui.visuals().weak_text_color(),
                                "Only in the config",
                            ),
                        };
                        ui.colored_label(if conflict { red() } else { color }, icon)
                            .on_hover_text(tip);
                        let to_cfg = r.db.is_some()
                            && r.state != RowState::Same
                            && !r.cfg_read_only
                            && !shared.contains(&r.alias);
                        if ui
                            .add_enabled(to_cfg, egui::Button::new(icons::ARROW_RIGHT).small())
                            .on_hover_text("Copy Telepad's host to the config")
                            .clicked()
                        {
                            send(out, HsMsg::ToConfig(vec![r.alias.clone()]));
                        }
                        let to_tp = r.cfg.is_some() && r.state != RowState::Same;
                        if ui
                            .add_enabled(to_tp, egui::Button::new(icons::ARROW_LEFT).small())
                            .on_hover_text("Copy the config's host to Telepad")
                            .clicked()
                        {
                            send(out, HsMsg::ToTelepad(vec![r.alias.clone()]));
                        }
                        if pending(hs, &r.alias)
                            && ui
                                .add(egui::Button::new(icons::UNDO).small())
                                .on_hover_text("Undo the pending changes of this host")
                                .clicked()
                        {
                            send(out, HsMsg::Undo(vec![r.alias.clone()]));
                        }
                    });
                });
                row.col(|ui| side_cell(ui, hs, r, Side::Config, r.cfg_read_only, out));
                row.col(|ui| {
                    let text = if r.diff.is_empty() {
                        String::new()
                    } else {
                        r.diff.join(", ")
                    };
                    ui.label(RichText::new(text).color(amber()));
                });
            });
        });
}

/// The row (or a renamed Telepad host's old alias) has pending changes.
fn pending(hs: &HostSync, alias: &str) -> bool {
    hs.config.contains_key(alias)
        || hs.telepad.contains_key(alias)
        || hs
            .telepad
            .values()
            .any(|p| matches!(p, Pending::Upsert(h) if h.alias == alias))
}

fn side_cell(
    ui: &mut egui::Ui,
    hs: &HostSync,
    r: &Row,
    side: Side,
    read_only: bool,
    out: &mut Vec<Msg>,
) {
    let host = match side {
        Side::Telepad => &r.db,
        Side::Config => &r.cfg,
    };
    let changed = match side {
        Side::Telepad => {
            hs.telepad.contains_key(&r.alias)
                || hs
                    .telepad
                    .values()
                    .any(|p| matches!(p, Pending::Upsert(h) if h.alias == r.alias))
        }
        Side::Config => hs.config.contains_key(&r.alias),
    };
    let deleted = match side {
        Side::Telepad => matches!(hs.telepad.get(&r.alias), Some(Pending::Delete)),
        Side::Config => matches!(hs.config.get(&r.alias), Some(Pending::Delete)),
    };
    ui.horizontal(|ui| match host {
        Some(h) => {
            let mut name = RichText::new(&h.alias).strong();
            if changed {
                name = name.color(ui.visuals().selection.stroke.color);
            }
            ui.label(name);
            ui.label(RichText::new(summary(h)).monospace().weak())
                .on_hover_text(if h.note.is_empty() {
                    "-".to_string()
                } else {
                    h.note.clone()
                });
            if read_only {
                ui.label(RichText::new("shared Host line").weak().small())
                    .on_hover_text(
                        "Defined together with other names; change it in the file by hand",
                    );
                return;
            }
            if act_small(ui, true, Act::Edit, icons::EDIT)
                .on_hover_text("Edit")
                .clicked()
            {
                send(
                    out,
                    HsMsg::Edit {
                        side,
                        alias: h.alias.clone(),
                    },
                );
            }
            if act_small(ui, true, Act::Danger, icons::TRASH)
                .on_hover_text(format!("Delete in {}", side.title()))
                .clicked()
            {
                send(
                    out,
                    HsMsg::Delete {
                        side,
                        aliases: vec![h.alias.clone()],
                    },
                );
            }
            if changed {
                ui.label(
                    RichText::new("pending")
                        .small()
                        .color(ui.visuals().selection.stroke.color),
                );
            }
        }
        None if deleted => {
            ui.label(
                RichText::new(format!("{} deleted", r.alias))
                    .strikethrough()
                    .color(red()),
            );
        }
        None => {
            ui.label(RichText::new("—").weak());
        }
    });
}

fn form_window(ctx: &egui::Context, d: &mut HostSyncDraft, out: &mut Vec<Msg>) {
    let Some(f) = &mut d.form else {
        return;
    };
    let side = f.side.unwrap_or(Side::Telepad);
    let title = match (&f.original, side) {
        (None, Side::Telepad) => "Add host to Telepad".to_string(),
        (None, Side::Config) => "Add host to the config".to_string(),
        (Some(a), s) => format!("Edit {a} in {}", s.title()),
    };
    let mut open = true;
    egui::Window::new(title)
        .id(egui::Id::unique("host-sync-form"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("host-sync-form-grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    for (label, value) in [
                        ("Alias (Host)", &mut f.alias),
                        ("HostName", &mut f.hostname),
                        ("User", &mut f.user),
                        ("Port", &mut f.port),
                        ("IdentityFile", &mut f.identity_file),
                        ("Description", &mut f.note),
                    ] {
                        ui.label(label);
                        ui.add_sized([320.0, 22.0], egui::TextEdit::singleline(value));
                        ui.end_row();
                    }
                    ui.label("Other options");
                    ui.add_sized(
                        [320.0, 70.0],
                        egui::TextEdit::multiline(&mut f.options)
                            .hint_text("one per line, e.g. ProxyJump bastion"),
                    );
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut f.unmonitored, "Not monitored");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut f.rdp, "RDP only (Windows, no SSH)");
                    ui.end_row();
                });
            if let Some(e) = &f.error {
                ui.colored_label(red(), e);
            }
            ui.label(
                RichText::new("Saved as a pending change; nothing is written before Apply")
                    .weak()
                    .small(),
            );
            ui.horizontal(|ui| {
                if ui.button(il(icons::CHECK, "OK")).clicked() {
                    send(out, HsMsg::FormSave);
                }
                if ui.button("Cancel").clicked() {
                    send(out, HsMsg::FormCancel);
                }
            });
        });
    if !open {
        send(out, HsMsg::FormCancel);
    }
}

fn preview_window(ctx: &egui::Context, hs: &HostSync, out: &mut Vec<Msg>) {
    let Some(p) = &hs.preview else {
        return;
    };
    let modal = egui::Modal::new(egui::Id::unique("host-sync-preview")).show(ctx, |ui| {
        let screen = ui.ctx().content_rect();
        ui.set_width((screen.width() - 80.0).clamp(300.0, 760.0));
        ui.heading(il(icons::SAVE, "Apply these changes?"));
        // Keep Apply / Back on screen in a small window.
        egui::ScrollArea::vertical()
            .max_height((screen.height() - 160.0).max(120.0))
            .show(ui, |ui| {
                ui.label(RichText::new(il(icons::DATABASE, "Telepad")).strong());
                if p.telepad.is_empty() {
                    ui.label(RichText::new("no change").weak());
                }
                for l in &p.telepad {
                    ui.label(RichText::new(l).monospace());
                }
                ui.add_space(8.0);
                ui.label(RichText::new(il(icons::DESKTOP, &hs.path)).strong());
                match &p.config_text {
                    None => {
                        ui.label(RichText::new("no change").weak());
                    }
                    Some(_) => {
                        ui.label(
                            RichText::new(
                                "A timestamped .bak copy is saved next to the file first.",
                            )
                            .weak()
                            .small(),
                        );
                        for (c, l) in &p.diff {
                            let text = RichText::new(format!("{c} {l}")).monospace();
                            ui.label(match c {
                                '+' => text.color(green()),
                                '-' => text.color(red()).strikethrough(),
                                _ => text.weak(),
                            });
                        }
                    }
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!hs.applying, egui::Button::new(il(icons::CHECK, "Apply")))
                .clicked()
            {
                send(out, HsMsg::Apply);
            }
            if ui.button("Back").clicked() {
                send(out, HsMsg::PreviewClose);
            }
        });
    });
    if modal.should_close() {
        send(out, HsMsg::PreviewClose);
    }
}
