//! "Public keys…": add or revoke one SSH public key in `~/.ssh/authorized_keys`
//! on the chosen hosts, after a typed confirmation.

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, KeysDraft, Model, Msg, key_selectable};
use telepad_core::authorized_keys::{KeyAction, PublicKey};
use telepad_core::config::Access;

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button, usage_color};

pub fn keys_window(ctx: &egui::Context, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(k) = &mut d.keys else { return };
    let mut open = true;
    // Key, hosts, confirmation and results can be taller than the app window:
    // keep the window on screen and scroll its content.
    let max_height = (ctx.content_rect().height() - 120.0).max(200.0);
    egui::Window::new("Public keys")
        .collapsible(false)
        .default_width(640.0)
        .open(&mut open)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("keys-window")
                .max_height(max_height)
                .show(ui, |ui| body(ui, m, k, out));
        });
    if !open {
        out.push(Msg::KeysClose);
    }
}

fn body(ui: &mut egui::Ui, m: &Model, k: &mut KeysDraft, out: &mut Vec<Msg>) {
    let (ok, bad) = (usage_color(10.0), usage_color(95.0));
    let running = m.key_run.as_ref().is_some_and(|r| r.running());

    ui.label("Paste one public key — the line from a .pub file, never the private key:");
    ui.add_enabled(
        k.confirm.is_none(),
        egui::TextEdit::multiline(&mut k.text)
            .code_editor()
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text("ssh-ed25519 AAAA… alice@example.com"),
    );
    let key = PublicKey::parse(&k.text);
    match &key {
        Ok(key) => {
            ui.colored_label(
                ok,
                format!(
                    "{} {}  ·  {}  ·  {}",
                    icons::CHECK_CIRCLE,
                    key.kind,
                    key.fingerprint(),
                    if key.comment.is_empty() {
                        "(no comment)"
                    } else {
                        &key.comment
                    }
                ),
            );
        }
        Err(e) if !k.text.trim().is_empty() => {
            ui.colored_label(bad, format!("{} {e}", icons::X_CIRCLE));
        }
        Err(_) => {}
    }

    ui.separator();
    host_picker(ui, m, k, out);
    ui.separator();

    let chosen = k.chosen(m);
    if let Some(action) = k.confirm {
        ui.colored_label(
            bad,
            RichText::new(format!(
                "{} {} on {} host(s)",
                icons::WARNING,
                action.title(),
                chosen.len()
            ))
            .strong(),
        );
        if let Ok(key) = &key {
            ui.label(RichText::new(format!("{}  {}", key.fingerprint(), key.comment)).monospace());
        }
        ui.label("This changes ~/.ssh/authorized_keys of the login user on each of these hosts:");
        let names: Vec<&str> = chosen.iter().map(|h| h.alias.as_str()).collect();
        egui::ScrollArea::vertical()
            .id_salt("key-confirm-hosts")
            .max_height(90.0)
            .show(ui, |ui| ui.label(RichText::new(names.join(", ")).strong()));
        let word = action.word();
        ui.horizontal(|ui| {
            ui.label(format!("Type {word} to confirm:"));
            let field = ui.add(egui::TextEdit::singleline(&mut k.typed).desired_width(90.0));
            if !field.has_focus() && k.typed.is_empty() {
                field.request_focus();
            }
            let ready = k.typed.trim() == word;
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if act_button(
                ui,
                ready,
                key_act(action),
                &il(key_icon(action), action.title()),
            )
            .clicked()
                || (ready && enter)
            {
                out.push(Msg::KeysRun);
            }
            if ui.button("Cancel").clicked() {
                out.push(Msg::KeysConfirmCancel);
            }
        });
    } else {
        let can = key.is_ok() && !chosen.is_empty() && !running;
        ui.horizontal(|ui| {
            for action in [KeyAction::Add, KeyAction::Revoke] {
                if act_button(
                    ui,
                    can,
                    key_act(action),
                    &il(key_icon(action), action.title()),
                )
                .clicked()
                {
                    out.push(Msg::KeysAsk(action));
                }
            }
            if ui.button("Close").clicked() {
                out.push(Msg::KeysClose);
            }
        });
    }

    results(ui, m, ok, bad);
}

/// Filter, header checkbox (all / none) and one checkbox per host.
fn host_picker(ui: &mut egui::Ui, m: &Model, k: &mut KeysDraft, out: &mut Vec<Msg>) {
    let listed = k.listed(m);
    let selectable: Vec<i64> = listed
        .iter()
        .map(|&i| &m.hosts[i].entry)
        .filter(|e| key_selectable(e))
        .filter_map(|e| e.id)
        .collect();
    let on = selectable
        .iter()
        .filter(|id| k.selected.contains(id))
        .count();
    let all = !selectable.is_empty() && on == selectable.len();
    let locked = k.confirm.is_some();

    ui.horizontal(|ui| {
        let mut header = all;
        let resp = ui.add_enabled(
            !locked && !selectable.is_empty(),
            egui::Checkbox::new(&mut header, "Select all").indeterminate(on > 0 && !all),
        );
        if resp.clicked() {
            out.push(Msg::KeysSelectAll(!all));
        }
        ui.add_enabled(
            !locked,
            egui::TextEdit::singleline(&mut k.filter)
                .hint_text(format!("{} filter hosts", icons::SEARCH))
                .desired_width(200.0),
        );
        ui.label(RichText::new(format!("{} selected", k.chosen(m).len())).strong());
    });
    egui::ScrollArea::vertical()
        .id_salt("key-hosts")
        .max_height(240.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if listed.is_empty() {
                ui.label(RichText::new("No host matches the filter").weak());
            }
            for i in listed {
                let e = &m.hosts[i].entry;
                let Some(id) = e.id else { continue };
                let mut on = k.selected.contains(&id) && key_selectable(e);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !locked && key_selectable(e),
                            egui::Checkbox::new(&mut on, &e.alias),
                        )
                        .clicked()
                    {
                        out.push(Msg::KeysToggle(id));
                    }
                    let target = if e.hostname.is_empty() {
                        &e.alias
                    } else {
                        &e.hostname
                    };
                    let who = if e.user.is_empty() {
                        target.clone()
                    } else {
                        format!("{}@{target}", e.user)
                    };
                    ui.label(RichText::new(who).weak().small());
                    if e.access == Access::Rdp {
                        ui.label(RichText::new("RDP only — no SSH").weak().small());
                    }
                });
            }
        });
}

fn results(ui: &mut egui::Ui, m: &Model, ok: egui::Color32, bad: egui::Color32) {
    let Some(run) = &m.key_run else { return };
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} {}  ·  {}/{} done",
                run.action.title(),
                run.fingerprint,
                run.finished(),
                run.results.len()
            ))
            .strong(),
        );
        if run.running() {
            ui.spinner();
        }
    });
    // No own scroll area: the window's scroll area holds the whole list.
    egui::Grid::new("key-results-grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for (alias, r) in &run.results {
                ui.label(alias);
                match r {
                    None => ui.label(RichText::new("running…").weak()),
                    Some(r) if r.ok() => {
                        ui.colored_label(ok, format!("{} {}", icons::CHECK_CIRCLE, r.text()))
                    }
                    Some(r) => ui.colored_label(bad, format!("{} {}", icons::X_CIRCLE, r.text())),
                };
                ui.end_row();
            }
        });
}

/// Add is green, Revoke red (#76).
fn key_act(action: KeyAction) -> Act {
    match action {
        KeyAction::Add => Act::Add,
        KeyAction::Revoke => Act::Danger,
    }
}

fn key_icon(action: KeyAction) -> &'static str {
    match action {
        KeyAction::Add => icons::PLUS,
        KeyAction::Revoke => icons::TRASH,
    }
}
