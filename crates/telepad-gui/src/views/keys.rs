//! Authorized keys page (#49, #78): what each host's `~/.ssh/authorized_keys`
//! holds — by server or by public key, with revoke / add shortcuts — and the
//! Add / revoke form (paste one key, pick hosts, type the confirmation word).

use eframe::egui::{self, RichText};
use telepad_app::{AkHost, Drafts, KeysDraft, KeysTab, Model, Msg, key_holders, key_selectable};
use telepad_core::authorized_keys::{AuthKey, KeyAction, PublicKey};
use telepad_core::config::Access;

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button, act_small, usage_color};

/// The Authorized keys page (an inner window of the workspace).
pub fn keys_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(k) = &mut d.keys else { return };
    ui.horizontal_wrapped(|ui| {
        for (tab, icon, name) in [
            (KeysTab::ByServer, icons::DESKTOP, "By server"),
            (KeysTab::ByKey, icons::KEY, "By public key"),
            (KeysTab::AddRevoke, icons::PLUS, "Add / revoke"),
            (KeysTab::MyKeys, icons::KEY, "My keys"),
        ] {
            if ui.selectable_label(k.tab == tab, il(icon, name)).clicked() {
                out.push(Msg::AkTab(tab));
            }
        }
        ui.separator();
        let loading = m
            .auth_keys
            .values()
            .filter(|v| **v == AkHost::Loading)
            .count();
        if ui
            .button(il(icons::REFRESH, "Refresh"))
            .on_hover_text("Read ~/.ssh/authorized_keys on every SSH host again (read-only)")
            .clicked()
        {
            out.push(Msg::AkLoad(None));
        }
        if loading > 0 {
            ui.spinner();
            ui.label(RichText::new(format!("reading {loading} host(s)…")).weak());
        }
    });
    ui.separator();
    match k.tab {
        KeysTab::ByServer => by_server(ui, m, k, out),
        KeysTab::ByKey => by_key(ui, m, k, out),
        KeysTab::MyKeys => super::mykeys::my_keys(ui, m, &mut k.mine, out),
        KeysTab::AddRevoke => {
            egui::ScrollArea::vertical()
                .id_salt("keys-window")
                .auto_shrink(false)
                .show(ui, |ui| body(ui, m, k, out));
        }
    }
}

fn filter_box(ui: &mut egui::Ui, k: &mut KeysDraft, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(&mut k.list_filter)
            .hint_text(format!("{} {hint}", icons::SEARCH))
            .desired_width(220.0),
    );
}

fn hit(filter: &str, texts: &[&str]) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty() || texts.iter().any(|t| t.to_lowercase().contains(&f))
}

/// Left: SSH hosts with their key count; right: the chosen host's keys.
fn by_server(ui: &mut egui::Ui, m: &Model, k: &mut KeysDraft, out: &mut Vec<Msg>) {
    let (ok, bad) = (usage_color(10.0), usage_color(95.0));
    let running = m.key_run.as_ref().is_some_and(|r| r.running());
    filter_box(ui, k, "server");
    ui.add_space(4.0);
    let hosts: Vec<&str> = m
        .hosts
        .iter()
        .filter(|h| key_selectable(&h.entry))
        .map(|h| h.entry.alias.as_str())
        .filter(|a| hit(&k.list_filter, &[a]))
        .collect();
    if k.server.is_none() {
        k.server = hosts.first().map(|a| a.to_string());
    }
    ui.columns(2, |cols| {
        egui::ScrollArea::vertical()
            .id_salt("ak-servers")
            .auto_shrink(false)
            .show(&mut cols[0], |ui| {
                for a in &hosts {
                    let state = match m.auth_keys.get(*a) {
                        Some(AkHost::Ready(h)) if h.missing => "no file".to_string(),
                        Some(AkHost::Ready(h)) => format!("{} key(s)", h.keys.len()),
                        Some(AkHost::Loading) => "reading…".into(),
                        Some(AkHost::Failed(_)) => format!("{} failed", icons::WARNING),
                        None => "—".into(),
                    };
                    let sel = k.server.as_deref() == Some(*a);
                    let r = ui.selectable_label(sel, format!("{}  {a}   {state}", icons::DESKTOP));
                    if r.clicked() {
                        k.server = Some(a.to_string());
                    }
                }
            });
        let ui = &mut cols[1];
        let Some(alias) = k.server.clone() else {
            ui.label(RichText::new("No SSH host.").weak());
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(il(icons::DESKTOP, &alias)).strong());
            if act_button(ui, !running, Act::Add, &il(icons::PLUS, "Add key"))
                .on_hover_text("Paste a key to add on this server")
                .clicked()
            {
                out.push(Msg::AkPrepare {
                    action: KeyAction::Add,
                    line: String::new(),
                    aliases: vec![alias.clone()],
                });
            }
            if act_button(ui, true, Act::Edit, &il(icons::EDIT, "Edit file"))
                .on_hover_text(
                    "Open ~/.ssh/authorized_keys of this server in the editor (saved only on Save)",
                )
                .clicked()
            {
                out.push(Msg::Files(telepad_app::files::FmMsg::OpenFile {
                    alias: alias.clone(),
                    rel: ".ssh/authorized_keys".into(),
                }));
            }
            if ui
                .small_button(icons::REFRESH)
                .on_hover_text("Read this server again")
                .clicked()
            {
                out.push(Msg::AkLoad(Some(vec![alias.clone()])));
            }
        });
        match m.auth_keys.get(&alias) {
            None | Some(AkHost::Loading) => {
                ui.spinner();
            }
            Some(AkHost::Failed(e)) => {
                ui.colored_label(bad, format!("{} {e}", icons::X_CIRCLE));
            }
            Some(AkHost::Ready(h)) => {
                if h.missing {
                    ui.label(
                        RichText::new(
                            "~/.ssh/authorized_keys does not exist: no key is authorized.",
                        )
                        .weak(),
                    );
                }
                if h.unreadable > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} line(s) that are not keys (left alone)",
                            h.unreadable
                        ))
                        .weak()
                        .small(),
                    );
                }
                egui::ScrollArea::vertical()
                    .id_salt(("ak-keys", &alias))
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        // Keys under their group heading (the `#` line above
                        // them, #85), groups in file order, "No group" last.
                        let mut groups: Vec<Option<&str>> = Vec::new();
                        for ak in &h.keys {
                            let g = ak.group.as_deref();
                            if g.is_some() && !groups.contains(&g) {
                                groups.push(g);
                            }
                        }
                        if h.keys.iter().any(|k| k.group.is_none()) {
                            groups.push(None);
                        }
                        let grouped = groups.iter().any(Option::is_some);
                        for g in groups {
                            if grouped {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(il(icons::KEY, g.unwrap_or("No group"))).strong(),
                                );
                            }
                            for ak in h.keys.iter().filter(|k| k.group.as_deref() == g) {
                                key_row(
                                    ui,
                                    ak,
                                    h.login.contains(&ak.key.fingerprint()),
                                    &alias,
                                    running,
                                    ok,
                                    out,
                                );
                            }
                        }
                    });
            }
        }
    });
}

/// One key of one server, with its Revoke shortcut.
fn key_row(
    ui: &mut egui::Ui,
    ak: &AuthKey,
    login: bool,
    alias: &str,
    running: bool,
    ok: egui::Color32,
    out: &mut Vec<Msg>,
) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&ak.key.kind).strong());
            ui.label(RichText::new(if ak.key.comment.is_empty() {
                "(no comment)"
            } else {
                &ak.key.comment
            }));
            if login {
                ui.colored_label(ok, format!("{} Telepad logs in with this key", icons::LOCK));
            }
        });
        ui.label(RichText::new(ak.key.fingerprint()).monospace().small());
        if !ak.options.is_empty() {
            ui.label(
                RichText::new(format!("options: {}", ak.options))
                    .monospace()
                    .small()
                    .weak(),
            );
        }
        let why = if login {
            Some("Telepad logs in to this host with this key: revoking it would lock Telepad out")
        } else if !ak.supported {
            Some("Telepad does not manage this key type")
        } else if running {
            Some("A key run is in progress")
        } else {
            None
        };
        let r = ui
            .horizontal(|ui| act_small(ui, why.is_none(), Act::Danger, &il(icons::TRASH, "Revoke")))
            .inner;
        let r = match why {
            Some(w) => r.on_disabled_hover_text(w),
            None => r.on_hover_text(format!(
                "Remove this key from {alias} (asks for \"revoke\")"
            )),
        };
        if r.clicked() {
            out.push(Msg::AkPrepare {
                action: KeyAction::Revoke,
                line: ak.key.line(),
                aliases: vec![alias.to_string()],
            });
        }
    });
}

/// Every key (by fingerprint) and the servers that have it.
fn by_key(ui: &mut egui::Ui, m: &Model, k: &mut KeysDraft, out: &mut Vec<Msg>) {
    let ok = usage_color(10.0);
    let running = m.key_run.as_ref().is_some_and(|r| r.running());
    filter_box(ui, k, "comment, fingerprint or server");
    ui.add_space(4.0);
    let all = key_holders(m);
    if all.is_empty() {
        ui.label(RichText::new("No keys read yet (Refresh).").weak());
        return;
    }
    let ssh: Vec<String> = m
        .hosts
        .iter()
        .filter(|h| key_selectable(&h.entry))
        .map(|h| h.entry.alias.clone())
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("ak-by-key")
        .auto_shrink(false)
        .show(ui, |ui| {
            for kh in all {
                let mut texts: Vec<&str> = vec![&kh.fingerprint, &kh.key.kind];
                texts.extend(kh.comments.iter().map(String::as_str));
                texts.extend(kh.groups.iter().map(String::as_str));
                texts.extend(kh.hosts.iter().map(|(a, _)| a.as_str()));
                if !hit(&k.list_filter, &texts) {
                    continue;
                }
                let open = k.open_key.as_deref() == Some(kh.fingerprint.as_str());
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        let caret = if open {
                            icons::CARET_DOWN
                        } else {
                            icons::CARET_RIGHT
                        };
                        if ui
                            .selectable_label(
                                open,
                                format!(
                                    "{caret} {} {}",
                                    kh.key.kind,
                                    if kh.comments.is_empty() {
                                        "(no comment)".to_string()
                                    } else {
                                        kh.comments.join(" · ")
                                    }
                                ),
                            )
                            .clicked()
                        {
                            k.open_key = (!open).then(|| kh.fingerprint.clone());
                        }
                        ui.label(RichText::new(format!("{} server(s)", kh.hosts.len())).weak());
                        let groups = if kh.groups.is_empty() {
                            "No group".to_string()
                        } else {
                            kh.groups.join(", ")
                        };
                        ui.label(RichText::new(il(icons::KEY, &groups)).small());
                        let missing: Vec<String> = ssh
                            .iter()
                            .filter(|a| !kh.hosts.iter().any(|(h, _)| h == *a))
                            .cloned()
                            .collect();
                        let can = kh.supported && !running && !missing.is_empty();
                        if act_small(ui, can, Act::Add, &il(icons::PLUS, "Add to server…"))
                            .on_hover_text("Choose more servers for this key (asks for \"add\")")
                            .clicked()
                        {
                            // Pick the servers in Add / revoke; none chosen yet.
                            out.push(Msg::AkPrepare {
                                action: KeyAction::Add,
                                line: kh.key.line(),
                                aliases: Vec::new(),
                            });
                        }
                    });
                    ui.label(RichText::new(&kh.fingerprint).monospace().small());
                    if !open {
                        return;
                    }
                    for (alias, login) in &kh.hosts {
                        ui.horizontal(|ui| {
                            ui.label(il(icons::DESKTOP, alias));
                            if *login {
                                ui.colored_label(ok, format!("{} login key", icons::LOCK));
                            }
                            let why = if *login {
                                Some("Telepad logs in to this host with this key")
                            } else if !kh.supported {
                                Some("Telepad does not manage this key type")
                            } else if running {
                                Some("A key run is in progress")
                            } else {
                                None
                            };
                            let r = act_small(
                                ui,
                                why.is_none(),
                                Act::Danger,
                                &il(icons::TRASH, "Revoke"),
                            );
                            let r = match why {
                                Some(w) => r.on_disabled_hover_text(w),
                                None => r,
                            };
                            if r.clicked() {
                                out.push(Msg::AkPrepare {
                                    action: KeyAction::Revoke,
                                    line: kh.key.line(),
                                    aliases: vec![alias.clone()],
                                });
                            }
                        });
                    }
                    let others: Vec<String> = kh
                        .hosts
                        .iter()
                        .filter(|(_, l)| !l)
                        .map(|(a, _)| a.clone())
                        .collect();
                    if kh.supported
                        && others.len() > 1
                        && act_small(
                            ui,
                            !running,
                            Act::Danger,
                            &il(
                                icons::TRASH,
                                &format!("Revoke on all {} servers", others.len()),
                            ),
                        )
                        .on_hover_text("Not on servers Telepad logs in to with it")
                        .clicked()
                    {
                        out.push(Msg::AkPrepare {
                            action: KeyAction::Revoke,
                            line: kh.key.line(),
                            aliases: others,
                        });
                    }
                });
            }
        });
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

    // Group for Add (#85): an existing one or a new name; empty = no group.
    ui.horizontal_wrapped(|ui| {
        ui.label("Group (for Add):");
        ui.add(
            egui::TextEdit::singleline(&mut k.group)
                .desired_width(160.0)
                .hint_text("no group"),
        );
        let known = telepad_app::key_groups(m);
        if !known.is_empty() {
            egui::ComboBox::from_id_salt("ak-group-pick")
                .selected_text("pick…")
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(k.group.is_empty(), "No group")
                        .clicked()
                    {
                        k.group.clear();
                    }
                    for g in known {
                        if ui.selectable_label(k.group == g, &g).clicked() {
                            k.group = g;
                        }
                    }
                });
        }
        ui.label(
            RichText::new(
                "the key goes under the # line with this name (created at the end if new)",
            )
            .weak()
            .small(),
        );
    });
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
