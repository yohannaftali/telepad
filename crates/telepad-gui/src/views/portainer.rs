//! Portainers (#171): the Portainers page (register, edit, delete, status)
//! and the "Portainer - [name]" page of one server: its environments and
//! the stacks of the chosen environment. Read-only; Open in Portainer opens
//! its web UI in the browser.

use eframe::egui::{self, RichText};
use telepad_app::portainer::{PtMsg, PtPageId, PtRow, PtStatus, suggestions};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::portainer::{ReadError, env_host, env_url, stack_url};

use crate::icons::{self, label as il};
use crate::table::{self, Col, Ev, Kind, Spec};
use crate::widgets::*;

pub const ICON: &str = icons::BROWSERS;

fn pt(out: &mut Vec<Msg>, msg: PtMsg) {
    out.push(Msg::Portainer(msg));
}

fn status_text(ui: &mut egui::Ui, r: &PtRow) {
    if r.reading {
        ui.spinner();
        table::weak(ui, "reading…");
        return;
    }
    let text = r.status.text();
    let resp = match &r.status {
        PtStatus::Ok => ui.colored_label(usage_color(10.0), text),
        PtStatus::Unknown => ui.label(RichText::new(text).weak()),
        PtStatus::Error(e) => ui
            .colored_label(usage_color(95.0), text)
            .on_hover_text(e.to_string()),
    };
    let _ = resp;
}

fn unix_time(t: Option<i64>) -> String {
    t.map(telepad_core::history::format_utc).unwrap_or_default()
}

/// The Portainers page.
pub fn portainers_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    ui.horizontal_wrapped(|ui| {
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add Portainer…")).clicked() {
            pt(out, PtMsg::Add);
        }
        if ui
            .button(il(icons::REFRESH, "Refresh all"))
            .on_hover_text("Read every Portainer again")
            .clicked()
        {
            pt(out, PtMsg::RefreshAll);
        }
        ui.label(
            RichText::new(
                "Read-only, through an SSH tunnel to the host that runs Portainer; tokens stay in the OS password store.",
            )
            .weak()
            .small(),
        );
    });
    ui.add_space(4.0);
    let rows = &m.portainers;
    if rows.is_empty() {
        ui.label(
            RichText::new(
                "No Portainer yet. Add one: pick the host it runs on and paste an API access token (Portainer: My account ▸ Access tokens).",
            )
            .weak(),
        );
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let data = r.data.as_ref();
            vec![
                r.p.name.clone(),
                r.p.alias.clone(),
                r.p.port.to_string(),
                data.map(|s| s.version.clone()).unwrap_or_default(),
                data.map(|s| s.environments.len().to_string())
                    .unwrap_or_default(),
                data.map(|s| s.stacks.len().to_string()).unwrap_or_default(),
                r.status.text(),
                ago(r.checked),
                String::new(),
            ]
        })
        .collect();
    let cols = [
        Col::new("Name"),
        Col::new("Host"),
        Col::new("Port"),
        Col::new("Version"),
        Col::new("Environments"),
        Col::new("Stacks"),
        Col::new("Status"),
        Col::new("Last checked"),
        Col::action(""),
    ];
    let state = d.tables.entry("portainers".into()).or_default();
    let spec = Spec::new("portainers", "Portainers", &cols)
        .kind(Kind::Fill)
        .row_h(26.0);
    let mut button = false;
    let o = table::show(ui, &spec, state, &cells, |ui, ev| match ev {
        Ev::Cell { row, col } => {
            let r = &rows[row];
            match col {
                0 => {
                    ui.label(RichText::new(il(ICON, &r.p.name)).strong());
                }
                1 => table::text(ui, &r.p.alias),
                2 => {
                    let scheme = if r.p.https { "https" } else { "http" };
                    table::text(ui, &format!("{} ({scheme})", r.p.port));
                }
                6 => status_text(ui, r),
                7 => table::weak(ui, &cells[row][7]),
                8 => {
                    if ui
                        .small_button(icons::REFRESH)
                        .on_hover_text("Read again")
                        .clicked()
                    {
                        button = true;
                        pt(out, PtMsg::Refresh(r.p.name.clone()));
                    }
                    if act_small(ui, true, Act::Edit, &il(icons::EDIT, "Edit")).clicked() {
                        button = true;
                        pt(out, PtMsg::Edit(r.p.name.clone()));
                    }
                    if act_small(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
                        button = true;
                        pt(out, PtMsg::Delete(r.p.name.clone()));
                    }
                }
                c => table::text(ui, &cells[row][c]),
            }
        }
        Ev::Menu { row, .. } => {
            let name = &rows[row].p.name;
            if ui
                .button(il(ICON, &format!("Open Portainer - {name}")))
                .clicked()
            {
                pt(out, PtMsg::Open(name.clone()));
                ui.close();
            }
            if let Some(h) = m.hosts.iter().find(|h| h.entry.alias == rows[row].p.alias)
                && ui
                    .button(il(icons::BROWSERS, "Open in Portainer"))
                    .clicked()
            {
                out.push(Msg::OpenUrl(rows[row].p.web_url(&h.entry)));
                ui.close();
            }
            if ui.button(il(icons::REFRESH, "Refresh")).clicked() {
                pt(out, PtMsg::Refresh(name.clone()));
                ui.close();
            }
            if ui
                .button(act_text(Act::Edit, &il(icons::EDIT, "Edit…")))
                .clicked()
            {
                pt(out, PtMsg::Edit(name.clone()));
                ui.close();
            }
            if ui
                .button(act_text(Act::Danger, &il(icons::TRASH, "Delete…")))
                .clicked()
            {
                pt(out, PtMsg::Delete(name.clone()));
                ui.close();
            }
        }
        Ev::HeaderMenu => {}
    });
    if let Some((r, _)) = o.clicked
        && !button
    {
        pt(out, PtMsg::Open(rows[r].p.name.clone()));
    }
    for (r, resp) in &o.rows {
        resp.clone()
            .on_hover_text(format!("Click to open Portainer - {}", rows[*r].p.name));
    }
    out.extend(o.msgs);
}

/// The Add / Edit and Delete dialogs (drawn once, over every window: Edit
/// also comes from a Portainer page).
pub fn dialogs(ctx: &egui::Context, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    form_dialog(ctx, m, d, out);
    delete_dialog(ctx, d, out);
}

fn form_dialog(ctx: &egui::Context, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(form) = &mut d.pt.form else { return };
    let title = match &form.original {
        Some(o) => format!("Edit Portainer '{o}'"),
        None => "Add Portainer".to_string(),
    };
    let found = suggestions(m);
    let modal = egui::Modal::new(egui::Id::unique("pt-form")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.heading(il(ICON, &title));
        ui.add_space(6.0);
        egui::Grid::new("pt-form-grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Name");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .desired_width(330.0)
                        .hint_text("e.g. Main"),
                );
                if form.original.is_none() && form.name.is_empty() {
                    r.request_focus();
                }
                ui.end_row();
                ui.label("Host");
                egui::ComboBox::from_id_salt("pt-form-host")
                    .width(330.0)
                    .selected_text(if form.alias.is_empty() {
                        "pick the host Portainer runs on".to_string()
                    } else {
                        form.alias.clone()
                    })
                    .show_ui(ui, |ui| {
                        // Hosts that run a Portainer server first, with their port.
                        for (alias, port, https) in &found {
                            let scheme = if *https { "https" } else { "http" };
                            if ui
                                .selectable_label(
                                    form.alias == *alias,
                                    format!("{alias} · Portainer on {port} ({scheme})"),
                                )
                                .clicked()
                            {
                                form.alias = alias.clone();
                                form.port = port.to_string();
                                form.https = *https;
                            }
                        }
                        if !found.is_empty() {
                            ui.separator();
                        }
                        for h in &m.hosts {
                            if found.iter().any(|(a, _, _)| *a == h.entry.alias) {
                                continue;
                            }
                            if ui
                                .selectable_label(form.alias == h.entry.alias, &h.entry.alias)
                                .clicked()
                            {
                                form.alias = h.entry.alias.clone();
                            }
                        }
                    });
                ui.end_row();
                ui.label("Port");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.port).desired_width(80.0));
                    ui.checkbox(&mut form.https, "HTTPS");
                    ui.label(
                        RichText::new("on the host (reached as 127.0.0.1 there)")
                            .weak()
                            .small(),
                    );
                });
                ui.end_row();
                ui.label("API token");
                ui.add(
                    egui::TextEdit::singleline(form.token.expose_mut())
                        .password(true)
                        .desired_width(330.0)
                        .hint_text(if form.original.is_some() {
                            "leave empty to keep the saved token"
                        } else {
                            "ptr_…"
                        }),
                );
                ui.end_row();
            });
        ui.label(
            RichText::new(
                "Create the token in Portainer: My account ▸ Access tokens ▸ Add access token. It is kept in the OS password store, never in telepad.db or an export.",
            )
            .weak()
            .small(),
        );
        if form.original.is_some() {
            ui.label(
                RichText::new("Changing the host, port or HTTPS forgets the pinned certificate.")
                    .weak()
                    .small(),
            );
        }
        if let Some(e) = &form.error {
            ui.colored_label(usage_color(95.0), e);
        }
        match &form.test {
            Some(Ok(t)) => {
                ui.colored_label(usage_color(10.0), il(icons::CHECK, t));
            }
            Some(Err(e)) => {
                ui.colored_label(usage_color(95.0), e);
            }
            None => {}
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let kind_act = if form.original.is_some() {
                Act::Edit
            } else {
                Act::Add
            };
            if act_button(ui, true, kind_act, &il(icons::SAVE, "Save")).clicked() {
                pt(out, PtMsg::FormSave);
            }
            if form.testing {
                ui.spinner();
            } else if ui
                .button(il(icons::PLUGS, "Test"))
                .on_hover_text("Read the Portainer with these settings, without saving")
                .clicked()
            {
                pt(out, PtMsg::FormTest);
            }
            if ui.button("Cancel").clicked() {
                pt(out, PtMsg::FormCancel);
            }
        });
    });
    if modal.should_close() {
        pt(out, PtMsg::FormCancel);
    }
}

fn delete_dialog(ctx: &egui::Context, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(name) = &d.pt.delete else { return };
    let modal = egui::Modal::new(egui::Id::unique("pt-delete")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading(act_text(
            Act::Danger,
            &format!("Delete Portainer '{name}'?"),
        ));
        ui.add_space(4.0);
        ui.label(
            "Telepad forgets it and its saved token. Nothing changes in Portainer or on the host.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
                pt(out, PtMsg::DeleteYes);
            }
            if ui.button("Cancel").clicked() {
                pt(out, PtMsg::DeleteNo);
            }
        });
    });
    if modal.should_close() {
        pt(out, PtMsg::DeleteNo);
    }
}

/// "Portainer - [name]": environments and the stacks of one of them.
pub fn portainer_page(
    ui: &mut egui::Ui,
    m: &Model,
    d: &mut Drafts,
    id: PtPageId,
    out: &mut Vec<Msg>,
) {
    let Some(page) = m.pt_pages.iter().find(|p| p.id == id) else {
        return;
    };
    let Some(row) = m.pt(&page.name) else {
        return;
    };
    let host = m.hosts.iter().find(|h| h.entry.alias == row.p.alias);
    let base = host.map(|h| row.p.web_url(&h.entry));
    ui.horizontal_wrapped(|ui| {
        if row.reading {
            ui.spinner();
        } else if ui
            .button(il(icons::REFRESH, "Refresh"))
            .on_hover_text("Read environments and stacks again")
            .clicked()
        {
            pt(out, PtMsg::Refresh(row.p.name.clone()));
        }
        if let Some(b) = &base
            && ui
                .button(il(icons::BROWSERS, "Open in Portainer"))
                .on_hover_text(b.as_str())
                .clicked()
        {
            out.push(Msg::OpenUrl(b.clone()));
        }
        ui.separator();
        let scheme = if row.p.https { "https" } else { "http" };
        ui.label(
            RichText::new(format!("on {} port {} ({scheme})", row.p.alias, row.p.port)).weak(),
        );
        if let Some(s) = &row.data {
            ui.label(RichText::new(format!("· Portainer {}", s.version)).weak());
        }
        ui.label(RichText::new(format!("· last read {}", ago(row.checked))).weak());
    });
    if let PtStatus::Error(e) = &row.status {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(usage_color(95.0), il(icons::WARNING, &e.to_string()));
            if matches!(e, ReadError::CertChanged { .. })
                && act_small(ui, true, Act::Caution, "Accept new certificate")
                    .on_hover_text("Trust the certificate Portainer shows now and read again")
                    .clicked()
            {
                pt(out, PtMsg::AcceptCert(row.p.name.clone()));
            }
            if matches!(e, ReadError::NoToken | ReadError::Rejected)
                && act_small(ui, true, Act::Edit, &il(icons::EDIT, "Edit…")).clicked()
            {
                pt(out, PtMsg::Edit(row.p.name.clone()));
            }
        });
        if row.data.is_some() {
            ui.label(RichText::new("Showing the last good read.").weak().small());
        }
    }
    ui.add_space(4.0);
    let Some(snap) = &row.data else {
        if row.reading {
            ui.label(RichText::new("Reading…").weak());
        }
        return;
    };
    let hosts: Vec<telepad_core::config::HostEntry> =
        m.hosts.iter().map(|h| h.entry.clone()).collect();
    let env_rows = &snap.environments;
    let chosen = page
        .env
        .filter(|e| env_rows.iter().any(|x| x.id == *e))
        .or_else(|| env_rows.first().map(|e| e.id));
    let count = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_default();
    let env_cells: Vec<Vec<String>> = env_rows
        .iter()
        .map(|e| {
            vec![
                e.name.clone(),
                e.kind_name().to_string(),
                e.url.clone(),
                match e.up {
                    Some(true) => "up".into(),
                    Some(false) => "down".into(),
                    None => String::new(),
                },
                snap.stacks
                    .iter()
                    .filter(|s| s.env == e.id)
                    .count()
                    .to_string(),
                match (e.running, e.total) {
                    (Some(r), Some(t)) => format!("{r} / {t}"),
                    (r, _) => count(r),
                },
                env_host(e, &row.p.alias, &hosts)
                    .map(|h| h.alias.clone())
                    .unwrap_or_default(),
                String::new(),
            ]
        })
        .collect();
    let env_cols = [
        Col::new("Environment"),
        Col::new("Type"),
        Col::new("URL"),
        Col::new("Status"),
        Col::new("Stacks"),
        Col::new("Containers"),
        Col::new("Telepad host"),
        Col::action(""),
    ];
    // Environments on top (about a third), stacks below.
    let split = (ui.available_height() * 0.38).clamp(140.0, 360.0);
    let mut env_click = None;
    let mut button = false;
    ui.label(RichText::new(il(icons::NETWORK, "Environments")).strong());
    ui.allocate_ui(egui::vec2(ui.available_width(), split), |ui| {
        let state = d.tables.entry(format!("pt-env:{id}")).or_default();
        let env_title = format!("{} environments", row.p.name);
        let spec = Spec::new(("pt-env", id), &env_title, &env_cols)
            .kind(Kind::Fill)
            .row_h(24.0)
            .selected(env_rows.iter().position(|e| Some(e.id) == chosen));
        let o = table::show(ui, &spec, state, &env_cells, |ui, ev| {
            if let Ev::Cell { row: r, col } = ev {
                let e = &env_rows[r];
                match col {
                    0 => {
                        ui.label(RichText::new(&e.name).strong());
                    }
                    3 => match e.up {
                        Some(true) => {
                            ui.colored_label(usage_color(10.0), "up");
                        }
                        Some(false) => {
                            ui.colored_label(usage_color(95.0), "down");
                        }
                        None => table::weak(ui, "—"),
                    },
                    6 => {
                        let alias = &env_cells[r][6];
                        if alias.is_empty() {
                            table::weak(ui, "—");
                        } else if ui
                            .link(alias)
                            .on_hover_text(format!("Open the host page of {alias}"))
                            .clicked()
                        {
                            button = true;
                            out.push(Msg::SelectAlias(alias.clone()));
                        }
                    }
                    7 => {
                        if let Some(b) = &base
                            && ui
                                .small_button(icons::BROWSERS)
                                .on_hover_text("Open this environment in Portainer")
                                .clicked()
                        {
                            button = true;
                            out.push(Msg::OpenUrl(env_url(b, e)));
                        }
                    }
                    c => table::text(ui, &env_cells[r][c]),
                }
            }
        });
        if let Some((r, _)) = o.clicked {
            env_click = Some(env_rows[r].id);
        }
        out.extend(o.msgs);
    });
    if let Some(e) = env_click.filter(|_| !button) {
        pt(
            out,
            PtMsg::SelectEnv {
                page: id,
                env: Some(e),
            },
        );
    }
    ui.add_space(6.0);
    let env = env_rows.iter().find(|e| Some(e.id) == chosen);
    let stacks: Vec<&telepad_core::portainer::Stack> = snap
        .stacks
        .iter()
        .filter(|s| Some(s.env) == chosen)
        .collect();
    ui.label(
        RichText::new(il(
            icons::STACK,
            &match env {
                Some(e) => format!("Stacks in {} ({})", e.name, stacks.len()),
                None => "Stacks".into(),
            },
        ))
        .strong(),
    );
    let env_host_alias = env
        .and_then(|e| env_host(e, &row.p.alias, &hosts))
        .map(|h| h.alias.clone());
    let stack_cells: Vec<Vec<String>> = stacks
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.kind_name().to_string(),
                match s.active {
                    Some(true) => "active".into(),
                    Some(false) => "inactive".into(),
                    None => String::new(),
                },
                unix_time(s.created),
                unix_time(s.updated),
                s.created_by.clone(),
                String::new(),
            ]
        })
        .collect();
    let stack_cols = [
        Col::new("Stack"),
        Col::new("Type"),
        Col::new("Status"),
        Col::new("Created (UTC)"),
        Col::new("Updated (UTC)"),
        Col::new("Created by"),
        Col::action(""),
    ];
    let state = d.tables.entry(format!("pt-stacks:{id}")).or_default();
    let stack_title = format!("{} stacks", row.p.name);
    let spec = Spec::new(("pt-stacks", id), &stack_title, &stack_cols)
        .kind(Kind::Fill)
        .row_h(24.0);
    let o = table::show(ui, &spec, state, &stack_cells, |ui, ev| {
        if let Ev::Cell { row: r, col } = ev {
            let s = stacks[r];
            match col {
                0 => {
                    ui.label(RichText::new(&s.name).strong());
                }
                2 => match s.active {
                    Some(true) => {
                        ui.colored_label(usage_color(10.0), "active");
                    }
                    Some(false) => table::weak(ui, "inactive"),
                    None => table::weak(ui, "—"),
                },
                6 => {
                    if let Some(b) = &base
                        && ui
                            .small_button(icons::BROWSERS)
                            .on_hover_text("Open this stack in Portainer (changes are made there)")
                            .clicked()
                    {
                        out.push(Msg::OpenUrl(stack_url(b, s)));
                    }
                    // Its containers on the Telepad host's page (Docker table).
                    if let Some(a) = &env_host_alias
                        && m.hosts.iter().any(|h| {
                            h.entry.alias == *a
                                && h.shown().is_some_and(|r| {
                                    r.docker
                                        .containers
                                        .iter()
                                        .any(|c| c.stack.eq_ignore_ascii_case(&s.name))
                                })
                        })
                        && ui
                            .small_button(icons::CONTAINER)
                            .on_hover_text(format!("Show its containers on the host page of {a}"))
                            .clicked()
                    {
                        out.push(Msg::SelectAlias(a.clone()));
                    }
                }
                c => table::text(ui, &stack_cells[r][c]),
            }
        }
    });
    out.extend(o.msgs);
}
