//! Portainers (#171): the Portainers page (register, edit, delete, status)
//! and the "Portainer - [name]" page of one server: its environments and
//! the stacks of the chosen environment. Read-only; Open in Portainer opens
//! its web UI in the browser.

use eframe::egui::{self, RichText};
use telepad_app::portainer::{
    EditorDraft, EditorId, Origin, PtMsg, PtPageId, PtRow, PtStatus, StackEditor, StackTab,
    editor_dirty, suggestions, var_changes,
};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::portainer::{
    Action, ContainerOp, ReadError, env_host, env_url, stack_locked, stack_url,
};

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
                "Through an SSH tunnel to the host that runs Portainer, with your token (kept in the OS password store); changes only after you confirm them.",
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
    confirm_dialog(ctx, d, out);
    choose_dialog(ctx, d, out);
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
    // Environments on top, stacks below, then the chosen stack's containers.
    let split = if page.stack.is_some() {
        (ui.available_height() * 0.22).clamp(100.0, 260.0)
    } else {
        (ui.available_height() * 0.3).clamp(130.0, 320.0)
    };
    let mut env_click = None;
    let mut button = false;
    ui.label(RichText::new(il(icons::NETWORK, "Environments")).strong());
    ui.allocate_ui(egui::vec2(ui.available_width(), split), |ui| {
        ui.set_max_height(split);
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
    ui.horizontal(|ui| {
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
        if let Some(b) = &page.busy {
            ui.spinner();
            ui.label(RichText::new(b).weak());
        }
    });
    let env_name = env.map_or(String::new(), |e| e.name.clone());
    let env_host_alias = env
        .and_then(|e| env_host(e, &row.p.alias, &hosts))
        .map(|h| h.alias.clone());
    let stack_cells: Vec<Vec<String>> = stacks
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.kind_name().to_string(),
                s.state().to_string(),
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
    let busy = page.busy.is_some();
    let stack_h = ui.available_height();
    let mut stack_click = None;
    let mut stack_open = None;
    let mut button = false;
    ui.allocate_ui(egui::vec2(ui.available_width(), stack_h), |ui| {
        ui.set_max_height(stack_h);
        let state = d.tables.entry(format!("pt-stacks:{id}")).or_default();
        let stack_title = format!("{} stacks", row.p.name);
        let spec = Spec::new(("pt-stacks", id), &stack_title, &stack_cols)
            .kind(Kind::Fill)
            .row_h(24.0)
            .selected(stacks.iter().position(|s| Some(s.id) == page.stack));
        let o = table::show(ui, &spec, state, &stack_cells, |ui, ev| match ev {
            Ev::Cell { row: r, col } => {
                let s = stacks[r];
                match col {
                    0 => {
                        ui.label(RichText::new(&s.name).strong());
                    }
                    2 => match s.state() {
                        "active" => {
                            ui.colored_label(usage_color(10.0), "active");
                        }
                        "deploying" => {
                            ui.colored_label(usage_color(80.0), "deploying");
                        }
                        "error" => {
                            ui.colored_label(usage_color(95.0), "error");
                        }
                        "" => table::weak(ui, "—"),
                        other => table::weak(ui, other),
                    },
                    6 => {
                        if let Some(why) = stack_locked(s, env) {
                            table::weak(ui, why);
                        } else if s.deploying {
                            ui.spinner();
                            table::weak(ui, "Portainer is deploying it");
                        } else {
                            if act_small(ui, !busy, Act::Edit, &il(icons::EDIT, "Edit"))
                                .on_hover_text("The compose file and variables; Update the stack")
                                .clicked()
                            {
                                button = true;
                                pt(
                                    out,
                                    PtMsg::OpenStack {
                                        portainer: row.p.name.clone(),
                                        stack: s.id,
                                        tab: StackTab::Editor,
                                    },
                                );
                            }
                            let (act, what, kind) = if s.active == Some(false) {
                                (
                                    Action::StackStart {
                                        id: s.id,
                                        env: s.env,
                                    },
                                    "Start",
                                    Act::Add,
                                )
                            } else {
                                (
                                    Action::StackStop {
                                        id: s.id,
                                        env: s.env,
                                    },
                                    "Stop",
                                    Act::Caution,
                                )
                            };
                            let icon = if what == "Start" {
                                icons::PLAY
                            } else {
                                icons::STOP
                            };
                            if act_small(ui, !busy, kind, &il(icon, what)).clicked() {
                                button = true;
                                pt(
                                    out,
                                    PtMsg::Ask {
                                        origin: Origin::Page(id),
                                        act,
                                        title: format!("{what} stack {} on {env_name}", s.name),
                                        type_name: String::new(),
                                    },
                                );
                            }
                            if act_small(ui, !busy, Act::Danger, &il(icons::TRASH, "Delete"))
                                .clicked()
                            {
                                button = true;
                                pt(
                                    out,
                                    PtMsg::Ask {
                                        origin: Origin::Page(id),
                                        act: Action::StackDelete {
                                            id: s.id,
                                            env: s.env,
                                        },
                                        title: format!("Delete stack {} on {env_name}", s.name),
                                        type_name: s.name.clone(),
                                    },
                                );
                            }
                        }
                        if let Some(b) = &base
                            && ui
                                .small_button(icons::BROWSERS)
                                .on_hover_text("Open this stack in Portainer")
                                .clicked()
                        {
                            button = true;
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
                                .small_button(icons::DESKTOP)
                                .on_hover_text(format!("The host page of {a} (its Docker table)"))
                                .clicked()
                        {
                            button = true;
                            out.push(Msg::SelectAlias(a.clone()));
                        }
                    }
                    c => table::text(ui, &stack_cells[r][c]),
                }
            }
            Ev::Menu { row: r, .. } => {
                let s = stacks[r];
                if ui
                    .button(il(icons::STACK, "Open stack"))
                    .on_hover_text("Its Stack page: containers and editor (also a double-click)")
                    .clicked()
                {
                    pt(
                        out,
                        PtMsg::OpenStack {
                            portainer: row.p.name.clone(),
                            stack: s.id,
                            tab: StackTab::Containers,
                        },
                    );
                    ui.close();
                }
                if stack_locked(s, env).is_none() && !busy {
                    if ui
                        .button(act_text(Act::Edit, &il(icons::EDIT, "Edit…")))
                        .clicked()
                    {
                        pt(
                            out,
                            PtMsg::OpenStack {
                                portainer: row.p.name.clone(),
                                stack: s.id,
                                tab: StackTab::Editor,
                            },
                        );
                        ui.close();
                    }
                    if ui
                        .button(act_text(Act::Danger, &il(icons::TRASH, "Delete…")))
                        .clicked()
                    {
                        pt(
                            out,
                            PtMsg::Ask {
                                origin: Origin::Page(id),
                                act: Action::StackDelete {
                                    id: s.id,
                                    env: s.env,
                                },
                                title: format!("Delete stack {} on {env_name}", s.name),
                                type_name: s.name.clone(),
                            },
                        );
                        ui.close();
                    }
                }
            }
            Ev::HeaderMenu => {}
        });
        if let Some((r, _)) = o.clicked {
            stack_click = Some(stacks[r].id);
        }
        if let Some((r, _)) = o.double_clicked {
            stack_open = Some(stacks[r].id);
        }
        for (r, resp) in &o.rows {
            resp.clone()
                .on_hover_text(format!("Double-click: Stack page of {}", stacks[*r].name));
        }
        out.extend(o.msgs);
    });
    // A double-click opens the stack's page (#176); a click selects it.
    if let Some(sid) = stack_open.filter(|_| !button) {
        pt(
            out,
            PtMsg::OpenStack {
                portainer: row.p.name.clone(),
                stack: sid,
                tab: StackTab::Containers,
            },
        );
    } else if let Some(sid) = stack_click.filter(|_| !button) {
        pt(
            out,
            PtMsg::SelectStack {
                page: id,
                stack: Some(sid),
            },
        );
    }
}
/// The Stack page's Containers tab (#176; actions #173).
fn containers_tab(ui: &mut egui::Ui, d: &mut Drafts, e: &StackEditor, out: &mut Vec<Msg>) {
    let id = e.id;
    let stack = &e.stack;
    let busy = e.busy.is_some();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(il(
                icons::CONTAINER,
                &format!("Containers of {}", stack.name),
            ))
            .strong(),
        );
        if e.loading_containers {
            ui.spinner();
        }
    });
    let list = match &e.containers {
        Some(Ok(l)) => l,
        Some(Err(err)) => {
            ui.colored_label(usage_color(95.0), err);
            return;
        }
        None => return,
    };
    if list.is_empty() {
        ui.label(RichText::new("No containers (the stack is down, or not deployed yet).").weak());
    }
    // Swarm tasks are managed by their service: no container actions.
    let locked = if stack.kind == 1 {
        Some("Swarm service tasks: scale or update the service in Portainer")
    } else if stack.kind == 3 || matches!(e.env_kind, 4 | 7) {
        Some("changed in Portainer")
    } else {
        None
    };
    let c_cells: Vec<Vec<String>> = list
        .iter()
        .map(|c| {
            vec![
                c.name.clone(),
                c.image.clone(),
                c.state.clone(),
                c.status.clone(),
                c.ports.clone(),
                String::new(),
            ]
        })
        .collect();
    let c_cols = [
        Col::new("Container"),
        Col::new("Image"),
        Col::new("State"),
        Col::new("Status"),
        Col::new("Ports"),
        Col::action(""),
    ];
    let state = d
        .tables
        .entry(format!("pt-stack-containers:{id}"))
        .or_default();
    let c_title = format!("{} containers", stack.name);
    let spec = Spec::new(("pt-stack-containers", id), &c_title, &c_cols)
        .kind(Kind::Fill)
        .row_h(24.0);
    let o = table::show(ui, &spec, state, &c_cells, |ui, ev| {
        if let Ev::Cell { row: r, col } = ev {
            let c = &list[r];
            match col {
                0 => {
                    ui.label(RichText::new(&c.name).strong());
                }
                2 => {
                    let colour = match c.state.as_str() {
                        "running" => usage_color(10.0),
                        "paused" | "restarting" => usage_color(80.0),
                        _ => ui.visuals().weak_text_color(),
                    };
                    ui.colored_label(colour, &c.state);
                }
                5 => {
                    if let Some(why) = locked {
                        table::weak(ui, why);
                        return;
                    }
                    for op in ContainerOp::for_state(&c.state) {
                        let kind = match op {
                            ContainerOp::Remove { .. } | ContainerOp::Kill => Some(Act::Danger),
                            ContainerOp::Stop | ContainerOp::Pause => Some(Act::Caution),
                            ContainerOp::Start | ContainerOp::Resume => Some(Act::Add),
                            ContainerOp::Restart => None,
                        };
                        let label = op.title();
                        let enabled = !busy;
                        let clicked = match kind {
                            Some(k) => act_small(ui, enabled, k, label).clicked(),
                            None => ui
                                .add_enabled(enabled, egui::Button::new(label).small())
                                .clicked(),
                        };
                        if clicked {
                            pt(
                                out,
                                PtMsg::Ask {
                                    origin: Origin::Stack(id),
                                    act: Action::Container {
                                        env: stack.env,
                                        id: c.id.clone(),
                                        op,
                                    },
                                    title: format!("{label} container {}", c.name),
                                    type_name: String::new(),
                                },
                            );
                        }
                    }
                }
                cc => table::text(ui, &c_cells[r][cc]),
            }
        }
    });
    out.extend(o.msgs);
}

/// The Stack page's header (#176): where it runs, its status and the
/// stack actions (#173).
fn stack_header(ui: &mut egui::Ui, m: &Model, e: &StackEditor, out: &mut Vec<Msg>) {
    let id = e.id;
    let s = &e.stack;
    let row = m.pt(&e.portainer);
    let hosts: Vec<telepad_core::config::HostEntry> =
        m.hosts.iter().map(|h| h.entry.clone()).collect();
    let env = row
        .and_then(|r| r.data.as_ref())
        .and_then(|d| d.environments.iter().find(|x| x.id == e.env_id));
    let host_alias = match (row, env) {
        (Some(r), Some(env)) => env_host(env, &r.p.alias, &hosts).map(|h| h.alias.clone()),
        _ => None,
    };
    let base = row.and_then(|r| {
        m.hosts
            .iter()
            .find(|h| h.entry.alias == r.p.alias)
            .map(|h| r.p.web_url(&h.entry))
    });
    let busy = e.busy.is_some();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(il(icons::STACK, &e.name)).strong().size(16.0));
        let (txt, colour) = match s.state() {
            "active" => ("active", usage_color(10.0)),
            "deploying" => ("deploying", usage_color(80.0)),
            "error" => ("error", usage_color(95.0)),
            other => (other, ui.visuals().weak_text_color()),
        };
        ui.colored_label(colour, txt);
        ui.label(
            RichText::new(format!(
                "· {} · {} · Portainer {}",
                s.kind_name(),
                e.env_name,
                e.portainer
            ))
            .weak(),
        );
        if let Some(a) = &host_alias {
            ui.label(RichText::new("· on").weak());
            if ui
                .link(a)
                .on_hover_text(format!("The host page of {a}"))
                .clicked()
            {
                out.push(Msg::SelectAlias(a.clone()));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        let locked = env.map_or_else(|| stack_locked(s, None), |x| stack_locked(s, Some(x)));
        if let Some(why) = locked {
            table::weak(ui, why);
        } else if s.deploying {
            ui.spinner();
            table::weak(ui, "Portainer is deploying it");
        } else {
            let (act, what, kind, icon) = if s.active == Some(false) {
                (
                    Action::StackStart {
                        id: s.id,
                        env: s.env,
                    },
                    "Start",
                    Act::Add,
                    icons::PLAY,
                )
            } else {
                (
                    Action::StackStop {
                        id: s.id,
                        env: s.env,
                    },
                    "Stop",
                    Act::Caution,
                    icons::STOP,
                )
            };
            if act_small(ui, !busy, kind, &il(icon, what)).clicked() {
                pt(
                    out,
                    PtMsg::Ask {
                        origin: Origin::Stack(id),
                        act,
                        title: format!("{what} stack {} on {}", s.name, e.env_name),
                        type_name: String::new(),
                    },
                );
            }
            if act_small(ui, !busy, Act::Danger, &il(icons::TRASH, "Delete…")).clicked() {
                pt(
                    out,
                    PtMsg::Ask {
                        origin: Origin::Stack(id),
                        act: Action::StackDelete {
                            id: s.id,
                            env: s.env,
                        },
                        title: format!("Delete stack {} on {}", s.name, e.env_name),
                        type_name: s.name.clone(),
                    },
                );
            }
        }
        if ui
            .add_enabled(
                !e.loading_containers,
                egui::Button::new(il(icons::REFRESH, "Refresh")).small(),
            )
            .on_hover_text("Read the stack and its containers again")
            .clicked()
        {
            pt(out, PtMsg::RefreshStack(id));
        }
        if let Some(b) = &base
            && ui
                .add(egui::Button::new(il(icons::BROWSERS, "Open in Portainer")).small())
                .clicked()
        {
            out.push(Msg::OpenUrl(stack_url(b, s)));
        }
        if let Some(b) = &e.busy {
            ui.spinner();
            ui.label(RichText::new(b).weak());
        }
    });
}

/// Several Portainers have the host's stack (#176): pick one.
fn choose_dialog(ctx: &egui::Context, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(list) = &d.pt.choose else { return };
    let modal = egui::Modal::new(egui::Id::unique("pt-choose")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading(il(icons::STACK, "Which stack?"));
        ui.label("More than one registered Portainer has this stack for the host:");
        for r in list {
            if ui
                .button(format!("{} · {} (stack {})", r.portainer, r.env, r.stack))
                .clicked()
            {
                pt(out, PtMsg::ChooseStack(Some(r.clone())));
            }
        }
        if ui.button("Cancel").clicked() {
            pt(out, PtMsg::ChooseStack(None));
        }
    });
    if modal.should_close() {
        pt(out, PtMsg::ChooseStack(None));
    }
}

/// The confirmation of a stack / container action (#173).
fn confirm_dialog(ctx: &egui::Context, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(c) = &mut d.pt.confirm else { return };
    let modal = egui::Modal::new(egui::Id::unique("pt-confirm")).show(ctx, |ui| {
        ui.set_width(480.0);
        let danger = matches!(
            c.act,
            Action::StackDelete { .. }
                | Action::Container {
                    op: ContainerOp::Kill | ContainerOp::Remove { .. },
                    ..
                }
        );
        let kind = if danger { Act::Danger } else { Act::Caution };
        ui.heading(act_text(kind, &format!("{}?", c.title)));
        ui.add_space(4.0);
        ui.label(match &c.act {
            Action::StackStart { .. } => "Portainer deploys the stack again (docker compose up). This may take a while when images are pulled.",
            Action::StackStop { .. } => "Portainer takes the stack down: its containers are stopped and removed. Its files, variables and named volumes stay; Start deploys it again.",
            Action::StackDelete { .. } => "Portainer removes the stack: its containers and networks, and the stack's files in Portainer. Telepad sends no volume option (Portainer's own default for volumes applies). This cannot be undone.",
            Action::Container { op: ContainerOp::Kill, .. } => "The container is killed at once (SIGKILL), without a clean shutdown.",
            Action::Container { op: ContainerOp::Remove { .. }, .. } => "The container is removed (stopped first if it runs). Compose recreates it on the next deploy of the stack.",
            Action::Container { .. } => "",
        });
        ui.label(RichText::new(format!("Portainer: {}", c.portainer)).weak().small());
        let mut volumes = None;
        if let Action::Container {
            op: ContainerOp::Remove { volumes: v },
            ..
        } = c.act
        {
            let mut v = v;
            if ui
                .checkbox(&mut v, "Also remove its anonymous volumes")
                .changed()
            {
                volumes = Some(v);
            }
        }
        if !c.type_name.is_empty() {
            ui.add_space(4.0);
            ui.label(format!("Type the stack name ({}) to delete it:", c.type_name));
            ui.add(egui::TextEdit::singleline(&mut c.typed).desired_width(300.0))
                .request_focus();
        }
        ui.add_space(6.0);
        let ok = c.type_name.is_empty() || c.typed.trim() == c.type_name;
        ui.horizontal(|ui| {
            let label = c.title.split_whitespace().next().unwrap_or("OK").to_string();
            if act_button(ui, ok, kind, &label).clicked() {
                pt(out, PtMsg::ConfirmYes);
            }
            if ui.button("Cancel").clicked() {
                pt(out, PtMsg::ConfirmNo);
            }
        });
        volumes
    });
    if let Some(v) = modal.inner {
        pt(out, PtMsg::ConfirmVolumes(v));
    }
    if modal.should_close() {
        pt(out, PtMsg::ConfirmNo);
    }
}

/// The stack editor (#173): compose file, variables, Update the stack.
pub fn stack_editor_page(
    ui: &mut egui::Ui,
    m: &Model,
    d: &mut Drafts,
    id: EditorId,
    focused: bool,
    out: &mut Vec<Msg>,
) {
    let Some(e) = m.pt_editors.iter().find(|e| e.id == id) else {
        return;
    };
    let Some(dr) = d.pt.editors.get_mut(&id) else {
        return;
    };
    let git = e.detail.as_ref().and_then(|x| x.git.clone());
    let dirty = editor_dirty(e, dr);
    stack_header(ui, m, e, out);
    ui.horizontal(|ui| {
        for (tab, label) in [
            (StackTab::Containers, il(icons::CONTAINER, "Containers")),
            (
                StackTab::Editor,
                il(icons::CODE, if dirty { "Editor ●" } else { "Editor" }),
            ),
        ] {
            if ui.selectable_label(e.tab == tab, label).clicked() && e.tab != tab {
                pt(out, PtMsg::SetTab { editor: id, tab });
            }
        }
    });
    ui.separator();
    if e.tab == StackTab::Containers {
        containers_tab(ui, d, e, out);
        let Some(dr) = d.pt.editors.get_mut(&id) else {
            return;
        };
        close_question(ui, e, dr, id, out);
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(il(icons::STACK, &e.name)).strong());
        ui.label(
            RichText::new(format!(
                "· Portainer {} · stack {}",
                e.portainer, e.stack_id
            ))
            .weak(),
        );
        if let Some(x) = &e.detail {
            ui.label(RichText::new(format!("· updated {}", unix_time(x.updated))).weak());
        }
        if dirty {
            ui.label(RichText::new("· edited").color(usage_color(80.0)));
        }
        if e.loading {
            ui.spinner();
        } else if ui
            .button(il(icons::REFRESH, "Reload"))
            .on_hover_text("Load the stack from Portainer again (drops your edits)")
            .clicked()
        {
            pt(out, PtMsg::EditorReload(id));
        }
    });
    if let Some(g) = &git {
        ui.colored_label(
            usage_color(80.0),
            il(icons::INFO, &format!("Git-based stack ({g}): its file comes from the repository. Edit and redeploy it in Portainer; Telepad shows it read-only.")),
        );
    }
    if e.conflict {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                usage_color(95.0),
                il(icons::WARNING, &ReadError::Changed.to_string()),
            );
            if ui.button("Reload").clicked() {
                pt(out, PtMsg::EditorReload(id));
            }
            if act_small(ui, !e.saving, Act::Danger, "Overwrite").clicked() {
                pt(
                    out,
                    PtMsg::EditorUpdate {
                        editor: id,
                        force: true,
                    },
                );
            }
        });
    }
    if let Some(err) = &e.error {
        ui.colored_label(usage_color(95.0), il(icons::WARNING, err));
    }
    if e.detail.is_none() {
        if e.loading {
            ui.label(RichText::new("Loading the stack…").weak());
        }
        return;
    }
    let editable = git.is_none() && !e.saving;
    if dr.review {
        review(ui, e, dr, id, out);
        return;
    }
    // The variables are on their own page; the compose file gets the window.
    ui.horizontal(|ui| {
        let n = dr.vars.len();
        if ui
            .button(il(icons::ROWS, &format!("Variables ({n})…")))
            .on_hover_text("The stack's environment variables, on their own page: edit, import a .env file, paste")
            .clicked()
        {
            pt(out, PtMsg::OpenVars(id));
        }
        ui.separator();
        ui.add_enabled(
            editable,
            egui::Checkbox::new(&mut dr.repull, "Re-pull image and redeploy"),
        );
        ui.add_enabled(editable, egui::Checkbox::new(&mut dr.prune, "Prune services"))
            .on_hover_text("Remove services that are no longer in the compose file");
        if e.saving {
            ui.spinner();
            ui.label(RichText::new("Updating the stack…").weak());
        } else if act_button(ui, editable, Act::Edit, &il(icons::SAVE, "Update the stack…")).clicked() {
            pt(out, PtMsg::EditorReview(id));
        }
    });
    let saved = e
        .detail
        .as_ref()
        .map(|x| x.file.clone())
        .unwrap_or_default();
    super::text_editor::embedded(
        ui,
        0x5741_0000_0000 + id,
        focused,
        "docker-compose.yml",
        telepad_core::files::syntax::Lang::Yaml,
        &mut dr.file,
        &saved,
        editable,
    );
    close_question(ui, e, dr, id, out);
}

/// Closing a Stack page with unsaved edits asks first.
fn close_question(
    ui: &mut egui::Ui,
    e: &StackEditor,
    dr: &mut EditorDraft,
    id: EditorId,
    out: &mut Vec<Msg>,
) {
    if dr.ask_close {
        let modal = egui::Modal::new(egui::Id::unique(("pt-ed-close", id))).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            ui.heading(format!("Close stack {} with unsaved edits?", e.name));
            ui.label(
                "Your changes to the compose file or the variables are not sent to Portainer.",
            );
            ui.horizontal(|ui| {
                if act_button(ui, true, Act::Danger, "Discard").clicked() {
                    pt(out, PtMsg::EditorDiscard(id));
                }
                if ui.button("Keep editing").clicked() {
                    dr.ask_close = false;
                }
            });
        });
        if modal.should_close() {
            dr.ask_close = false;
        }
    }
}

/// A stack editor's Variables page (#173): name / value rows (values hidden
/// unless shown), add / remove, import a `.env` file, paste `.env` text.
/// Sent with the stack editor's Update the stack.
pub fn vars_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, id: EditorId, out: &mut Vec<Msg>) {
    let Some(e) = m.pt_editors.iter().find(|e| e.id == id) else {
        return;
    };
    let Some(dr) = d.pt.editors.get_mut(&id) else {
        return;
    };
    let git = e.detail.as_ref().is_some_and(|x| x.git.is_some());
    let editable = e.detail.is_some() && !git && !e.saving;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(il(icons::STACK, &e.name)).strong());
        ui.label(RichText::new(format!("· {} variable(s)", dr.vars.len())).weak());
        if editor_dirty(e, dr) {
            ui.label(RichText::new("· edited").color(usage_color(80.0)));
        }
        ui.separator();
        ui.checkbox(&mut dr.reveal, "Show values");
        if act_small(ui, editable, Act::Add, &il(icons::PLUS, "Add variable")).clicked() {
            dr.vars.push(Default::default());
        }
        if ui
            .add_enabled(
                editable,
                egui::Button::new(il(icons::IMPORT, "Import .env file…")).small(),
            )
            .on_hover_text("Read NAME=value lines from a file; same names get the new value")
            .clicked()
        {
            pt(out, PtMsg::VarsFromFile(id));
        }
        if ui
            .add_enabled(
                editable,
                egui::Button::new(il(icons::COPY, "Paste…")).small(),
            )
            .on_hover_text("Paste NAME=value lines (a .env file's text)")
            .clicked()
        {
            pt(out, PtMsg::VarsPasteOpen(id));
        }
        ui.separator();
        if ui
            .button(il(icons::CODE, "Compose file"))
            .on_hover_text("Back to the stack editor")
            .clicked()
        {
            out.push(Msg::Ws(telepad_app::workspace::WsMsg::Open(
                telepad_app::workspace::Win::PtStack(id),
            )));
        }
    });
    if git {
        ui.colored_label(
            usage_color(80.0),
            il(
                icons::INFO,
                "Git-based stack: variables are changed in Portainer.",
            ),
        );
    }
    match &e.notice {
        Some(Ok(t)) => {
            ui.colored_label(usage_color(10.0), il(icons::CHECK, t));
        }
        Some(Err(t)) => {
            ui.colored_label(usage_color(95.0), il(icons::WARNING, t));
        }
        None => {}
    }
    ui.label(
        RichText::new(
            "Changes are sent with Update the stack in the stack editor; Portainer redeploys the stack then.",
        )
        .weak()
        .small(),
    );
    ui.add_space(4.0);
    let w = ui.available_width();
    let row_h = ui.spacing().interact_size.y;
    let (nw, vw) = ((w * 0.34).clamp(140.0, 360.0), (w * 0.58).max(160.0));
    egui::ScrollArea::vertical()
        .id_salt(("pt-vars", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut remove = None;
            ui.horizontal(|ui| {
                ui.add_sized(
                    [nw, row_h],
                    egui::Label::new(RichText::new("Name").strong()),
                );
                ui.add_sized(
                    [vw, row_h],
                    egui::Label::new(RichText::new("Value").strong()),
                );
            });
            for (i, v) in dr.vars.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.add_sized(
                            [nw, row_h],
                            egui::TextEdit::singleline(&mut v.name)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("NAME"),
                        );
                        ui.add_sized(
                            [vw, row_h],
                            egui::TextEdit::singleline(&mut v.value)
                                .font(egui::TextStyle::Monospace)
                                .password(!dr.reveal)
                                .hint_text("value"),
                        );
                        if ui
                            .add(egui::Button::new(icons::X).small())
                            .on_hover_text("Remove this variable")
                            .clicked()
                        {
                            remove = Some(i);
                        }
                    });
                });
            }
            if dr.vars.is_empty() {
                ui.label(
                    RichText::new(
                        "No variables. Add one, import a .env file or paste NAME=value lines.",
                    )
                    .weak(),
                );
            }
            if let Some(i) = remove {
                dr.vars.remove(i);
            }
        });
    if let Some(text) = &mut dr.paste {
        let modal = egui::Modal::new(egui::Id::unique(("pt-vars-paste", id))).show(ui.ctx(), |ui| {
            ui.set_width(560.0);
            ui.heading(il(icons::COPY, &format!("Paste variables into {}", e.name)));
            ui.label(
                RichText::new(
                    "One NAME=value per line, as in a .env file (# comments, export and quotes are fine). Same names get the new value.",
                )
                .weak()
                .small(),
            );
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(12)
                        .desired_width(f32::INFINITY)
                        .hint_text("APP_ENV=dev\nDB_HOST=db"),
                )
                .request_focus();
            });
            if let Some(Err(t)) = &e.notice {
                ui.colored_label(usage_color(95.0), t);
            }
            ui.horizontal(|ui| {
                if act_button(
                    ui,
                    !text.trim().is_empty(),
                    Act::Add,
                    &il(icons::PLUS, "Add variables"),
                )
                .clicked()
                {
                    pt(out, PtMsg::VarsPaste(id));
                }
                if ui.button("Cancel").clicked() {
                    pt(out, PtMsg::VarsPasteCancel(id));
                }
            });
        });
        if modal.should_close() {
            pt(out, PtMsg::VarsPasteCancel(id));
        }
    }
}

/// The review before Update the stack: the file diff and the variables
/// added / changed / removed (names only).
fn review(
    ui: &mut egui::Ui,
    e: &StackEditor,
    dr: &mut EditorDraft,
    id: EditorId,
    out: &mut Vec<Msg>,
) {
    let Some(x) = &e.detail else { return };
    ui.label(RichText::new(format!("Update stack {}: review", e.name)).strong());
    let changes = var_changes(&x.env, &dr.vars);
    let file_diff = telepad_core::hostsync::line_diff(&x.file, &dr.file);
    let changed_lines = file_diff.iter().filter(|(c, _)| *c != ' ').count();
    ui.label(format!(
        "Compose file: {} · Variables: {}",
        if changed_lines == 0 {
            "unchanged".to_string()
        } else {
            format!("{changed_lines} line(s) changed")
        },
        if changes.is_empty() {
            "unchanged".to_string()
        } else {
            changes.join(", ")
        }
    ));
    let opts = [
        dr.repull.then_some("re-pull images and redeploy"),
        dr.prune.then_some("prune services"),
    ];
    let opts: Vec<&str> = opts.into_iter().flatten().collect();
    if !opts.is_empty() {
        ui.label(RichText::new(format!("Options: {}", opts.join(", "))).weak());
    }
    ui.label(
        RichText::new(
            "Portainer redeploys the stack (docker compose up): containers whose settings changed are recreated, and a service that waits for an unhealthy one may stay Created.",
        )
        .color(usage_color(80.0))
        .small(),
    );
    let h = ui.available_height() - 44.0;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        egui::ScrollArea::both()
            .id_salt(("pt-ed-diff", id))
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let v = ui.visuals().clone();
                let mono = egui::TextStyle::Monospace.resolve(ui.style());
                for (c, line) in &file_diff {
                    let (colour, sign) = match c {
                        '+' => (usage_color(10.0), "+"),
                        '-' => (usage_color(95.0), "-"),
                        _ => (v.weak_text_color(), " "),
                    };
                    ui.label(
                        RichText::new(format!("{sign} {line}"))
                            .font(mono.clone())
                            .color(colour),
                    );
                }
            });
    });
    ui.horizontal(|ui| {
        if act_button(
            ui,
            !e.saving,
            Act::Edit,
            &il(icons::SAVE, "Update the stack"),
        )
        .clicked()
        {
            pt(
                out,
                PtMsg::EditorUpdate {
                    editor: id,
                    force: false,
                },
            );
        }
        if ui.button("Back to editing").clicked() {
            pt(out, PtMsg::EditorBack(id));
        }
        if e.saving {
            ui.spinner();
        }
    });
}
