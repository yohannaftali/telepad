//! Groups and Clusters pages (#144, #145): the list (add, edit, reorder,
//! delete; a fixed entry for hosts without one) and the "Hosts - [name]"
//! page of one group / cluster (same table / cards as the Hosts page).

use eframe::egui::{self, RichText};
use telepad_app::sets::{SetPageId, StMsg, list, shown_name, stats};
use telepad_app::{Drafts, Model, Msg, matches_query};
use telepad_core::config::SetKind;

use super::cards;
use super::overview::{self, TableView};
use crate::icons::{self, label as il};
use crate::widgets::*;

fn icon(kind: SetKind) -> &'static str {
    match kind {
        SetKind::Group => icons::FOLDERS,
        SetKind::Cluster => icons::CIRCLES_THREE,
    }
}

fn st(out: &mut Vec<Msg>, msg: StMsg) {
    out.push(Msg::Sets(msg));
}

/// The Groups or Clusters page.
pub fn sets_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, kind: SetKind, out: &mut Vec<Msg>) {
    let plural = format!("{}s", kind.as_str());
    ui.horizontal_wrapped(|ui| {
        let filter = d.sets.filter.entry(kind).or_default();
        ui.add(
            egui::TextEdit::singleline(filter)
                .desired_width(200.0)
                .hint_text(format!("{} {plural}", icons::SEARCH)),
        );
        ui.separator();
        if act_button(
            ui,
            true,
            Act::Add,
            &il(icons::PLUS, &format!("Add {}…", kind.as_str())),
        )
        .clicked()
        {
            st(out, StMsg::Add(kind));
        }
        ui.label(
            RichText::new(match kind {
                SetKind::Group => "A host is in at most one group; groups are optional.",
                SetKind::Cluster => "A host is in at most one cluster, separate from its group.",
            })
            .weak()
            .small(),
        );
    });
    ui.add_space(4.0);
    let query = d.sets.filter.get(&kind).cloned().unwrap_or_default();
    let sets = list(m, kind);
    let shown: Vec<usize> = (0..sets.len())
        .filter(|&i| {
            matches_query(&sets[i].name, &query) || matches_query(&sets[i].description, &query)
        })
        .collect();
    let none_shown = matches_query(kind.none_name(), &query);
    // Source rows: the sets shown, then the fixed entry ("Ungrouped" / "No cluster").
    let names: Vec<(String, String, bool)> = shown
        .iter()
        .map(|&i| (sets[i].name.clone(), sets[i].description.clone(), false))
        .chain(none_shown.then(|| (String::new(), String::new(), true)))
        .collect();
    let stats_of: Vec<_> = names
        .iter()
        .map(|(n, _, _)| stats(&m.hosts, kind, n))
        .collect();
    let cells: Vec<Vec<String>> = names
        .iter()
        .zip(&stats_of)
        .map(|((n, desc, _), s)| {
            vec![
                shown_name(kind, n).to_string(),
                desc.clone(),
                s.hosts.to_string(),
                s.down.to_string(),
                s.flagged.to_string(),
                String::new(),
            ]
        })
        .collect();
    use crate::table::{self, Col, Ev, Kind, Spec};
    let cols = [
        Col::new(kind.title()),
        Col::new("Description"),
        Col::new("Hosts"),
        Col::new("Down"),
        Col::new("Flags"),
        Col::action(""),
    ];
    let key = format!("sets:{}", kind.as_str());
    let state = d.tables.entry(key).or_default();
    // Moving up / down follows the saved order: only while the list is not
    // searched, filtered or sorted.
    let can_move = query.is_empty() && state.sort.is_none() && !state.filtered();
    let title = format!("{}s", kind.title());
    let spec = Spec::new(("sets", kind.as_str()), &title, &cols)
        .kind(Kind::Fill)
        .row_h(26.0);
    let mut button = false;
    let o = table::show(ui, &spec, state, &cells, |ui, ev| match ev {
        Ev::Cell { row, col } => {
            let (name, _, fixed) = &names[row];
            let s = &stats_of[row];
            match col {
                0 => {
                    let text = RichText::new(il(icon(kind), shown_name(kind, name))).strong();
                    ui.label(if *fixed { text.italics() } else { text });
                }
                1 => table::weak(ui, &cells[row][1]),
                2 => table::text(ui, &cells[row][2]),
                3 => {
                    if s.down > 0 {
                        ui.colored_label(usage_color(95.0), s.down.to_string());
                    } else {
                        table::weak(ui, "0");
                    }
                }
                4 => {
                    if s.flagged > 0 {
                        ui.colored_label(usage_color(75.0), s.flagged.to_string());
                    } else {
                        table::weak(ui, "0");
                    }
                }
                _ => {
                    if *fixed {
                        ui.label(RichText::new("fixed").weak().small())
                            .on_hover_text("Hosts without one; cannot be renamed or deleted");
                        return;
                    }
                    let pos = shown[row];
                    if ui
                        .add_enabled(
                            pos > 0 && can_move,
                            egui::Button::new(icons::CARET_UP).small(),
                        )
                        .on_hover_text("Move up")
                        .clicked()
                    {
                        button = true;
                        st(
                            out,
                            StMsg::Move {
                                kind,
                                name: name.clone(),
                                by: -1,
                            },
                        );
                    }
                    if ui
                        .add_enabled(
                            pos + 1 < sets.len() && can_move,
                            egui::Button::new(icons::CARET_DOWN).small(),
                        )
                        .on_hover_text("Move down")
                        .clicked()
                    {
                        button = true;
                        st(
                            out,
                            StMsg::Move {
                                kind,
                                name: name.clone(),
                                by: 1,
                            },
                        );
                    }
                    if act_small(ui, true, Act::Edit, &il(icons::EDIT, "Edit")).clicked() {
                        button = true;
                        st(
                            out,
                            StMsg::Edit {
                                kind,
                                name: name.clone(),
                            },
                        );
                    }
                    if act_small(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
                        button = true;
                        st(
                            out,
                            StMsg::Delete {
                                kind,
                                name: name.clone(),
                            },
                        );
                    }
                }
            }
        }
        Ev::Menu { row, .. } => {
            let (name, _, fixed) = &names[row];
            if ui
                .button(il(
                    icons::DESKTOP,
                    &format!("Open Hosts - {}", shown_name(kind, name)),
                ))
                .clicked()
            {
                st(
                    out,
                    StMsg::OpenHosts {
                        kind,
                        name: name.clone(),
                    },
                );
                ui.close();
            }
            if !*fixed {
                if ui
                    .button(act_text(Act::Edit, &il(icons::EDIT, "Edit…")))
                    .clicked()
                {
                    st(
                        out,
                        StMsg::Edit {
                            kind,
                            name: name.clone(),
                        },
                    );
                    ui.close();
                }
                if ui
                    .button(act_text(Act::Danger, &il(icons::TRASH, "Delete…")))
                    .clicked()
                {
                    st(
                        out,
                        StMsg::Delete {
                            kind,
                            name: name.clone(),
                        },
                    );
                    ui.close();
                }
            }
        }
        Ev::HeaderMenu => {}
    });
    if let Some((r, _)) = o.clicked
        && !button
    {
        st(
            out,
            StMsg::OpenHosts {
                kind,
                name: names[r].0.clone(),
            },
        );
    }
    for (r, resp) in &o.rows {
        resp.clone().on_hover_text(format!(
            "Click to open Hosts - {}",
            shown_name(kind, &names[*r].0)
        ));
    }
    out.extend(o.msgs);
    // Each dialog only on its own page: with Groups and Clusters both open
    // it would otherwise be drawn (and typed into) twice.
    form_dialog(ui, d, kind, out);
    delete_dialog(ui, m, d, kind, out);
}

fn form_dialog(ui: &mut egui::Ui, d: &mut Drafts, page: SetKind, out: &mut Vec<Msg>) {
    let Some(form) = &mut d.sets.form else { return };
    let Some(kind) = form.kind.filter(|k| *k == page) else {
        return;
    };
    let title = match &form.original {
        Some(o) => format!("Edit {} '{o}'", kind.as_str()),
        None => format!("Add {}", kind.as_str()),
    };
    let modal = egui::Modal::new(egui::Id::unique("set-form")).show(ui.ctx(), |ui| {
        ui.set_width(420.0);
        ui.heading(il(icon(kind), &title));
        ui.add_space(6.0);
        egui::Grid::new("set-form-grid")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Name");
                let r = ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(280.0));
                if form.original.is_none() && form.name.is_empty() {
                    r.request_focus();
                }
                ui.end_row();
                ui.label("Description");
                ui.add(
                    egui::TextEdit::singleline(&mut form.description)
                        .desired_width(280.0)
                        .hint_text("optional"),
                );
                ui.end_row();
            });
        if form.original.is_some() {
            ui.label(
                RichText::new("Renaming moves its hosts along.")
                    .weak()
                    .small(),
            );
        }
        if let Some(e) = &form.error {
            ui.colored_label(usage_color(95.0), e);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let kind_act = if form.original.is_some() {
                Act::Edit
            } else {
                Act::Add
            };
            if act_button(ui, true, kind_act, &il(icons::SAVE, "Save")).clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                st(out, StMsg::FormSave);
            }
            if ui.button("Cancel").clicked() {
                st(out, StMsg::FormCancel);
            }
        });
    });
    if modal.should_close() {
        st(out, StMsg::FormCancel);
    }
}

fn delete_dialog(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, page: SetKind, out: &mut Vec<Msg>) {
    let Some((kind, name)) = d.sets.delete.as_ref().filter(|(k, _)| *k == page) else {
        return;
    };
    let n = stats(&m.hosts, *kind, name).hosts;
    let modal = egui::Modal::new(egui::Id::unique("set-delete")).show(ui.ctx(), |ui| {
        ui.set_width(420.0);
        ui.heading(act_text(
            Act::Danger,
            &format!("Delete {} '{name}'?", kind.as_str()),
        ));
        ui.add_space(4.0);
        ui.label(format!(
            "Its {n} host(s) stay in Telepad and move to {}. No host is deleted.",
            kind.none_name()
        ));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
                st(out, StMsg::DeleteYes);
            }
            if ui.button("Cancel").clicked() {
                st(out, StMsg::DeleteNo);
            }
        });
    });
    if modal.should_close() {
        st(out, StMsg::DeleteNo);
    }
}

/// "Hosts - [name]": the Hosts page for one group / cluster.
pub fn set_hosts_page(
    ui: &mut egui::Ui,
    m: &Model,
    d: &mut Drafts,
    id: SetPageId,
    out: &mut Vec<Msg>,
) {
    let Some(p) = m.set_pages.iter().find(|p| p.id == id) else {
        return;
    };
    let kind = p.kind;
    let cards = d.sets.page_cards.contains(&id);
    ui.horizontal_wrapped(|ui| {
        let filter = d.sets.page_filter.entry(id).or_default();
        ui.add(
            egui::TextEdit::singleline(filter)
                .desired_width(200.0)
                .hint_text(format!("{} host, IP, OS, container", icons::SEARCH)),
        );
        ui.separator();
        ui.label(RichText::new("View").weak());
        if ui
            .selectable_label(!cards, il(icons::OVERVIEW, "Table"))
            .clicked()
        {
            d.sets.page_cards.remove(&id);
        }
        if ui
            .selectable_label(cards, il(icons::GAUGE, "Cards"))
            .clicked()
        {
            d.sets.page_cards.insert(id);
        }
        ui.separator();
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add hosts…")).clicked() {
            st(out, StMsg::AddHostsOpen(id));
        }
        let desc = list(m, kind)
            .iter()
            .find(|s| s.name == p.name)
            .map(|s| s.description.as_str())
            .unwrap_or("");
        if !desc.is_empty() {
            ui.label(RichText::new(desc).weak().italics());
        }
    });
    ui.add_space(4.0);
    let filter = d.sets.page_filter.get(&id).cloned().unwrap_or_default();
    // A cluster page shows each host's group, a group page its cluster.
    let other = match kind {
        SetKind::Group => [SetKind::Cluster],
        SetKind::Cluster => [SetKind::Group],
    };
    let view = TableView {
        only: Some((kind, &p.name)),
        extra: &other,
        salt: id,
    };
    if cards {
        let rows = overview::view_rows(m, &filter, d.sort, &view);
        cards::cards(ui, m, &rows, out);
    } else {
        overview::table(ui, m, &filter, d.sort, &view, &mut d.tables, out);
    }
    if d.sets.add_hosts.as_ref().is_some_and(|a| a.page == id) {
        add_hosts_dialog(ui, m, d, out);
    }
}

fn add_hosts_dialog(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(a) = &mut d.sets.add_hosts else {
        return;
    };
    let Some(p) = m.set_pages.iter().find(|p| p.id == a.page) else {
        return;
    };
    let modal = egui::Modal::new(egui::Id::unique("set-add-hosts")).show(ui.ctx(), |ui| {
        ui.set_width(460.0);
        ui.heading(il(
            icon(p.kind),
            &format!("Add hosts to {}", shown_name(p.kind, &p.name)),
        ));
        ui.label(
            RichText::new(format!(
                "A host can be in one {} only: checked hosts move here from their current one.",
                p.kind.as_str()
            ))
            .weak()
            .small(),
        );
        ui.add(
            egui::TextEdit::singleline(&mut a.filter)
                .desired_width(f32::INFINITY)
                .hint_text(format!("{} host, address", icons::SEARCH)),
        );
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for h in &m.hosts {
                    let e = &h.entry;
                    if e.set(p.kind).eq_ignore_ascii_case(&p.name) {
                        continue;
                    }
                    if !(matches_query(&e.alias, &a.filter)
                        || matches_query(&e.hostname, &a.filter))
                    {
                        continue;
                    }
                    let mut on = a.chosen.contains(&e.alias);
                    let now = shown_name(p.kind, e.set(p.kind));
                    if ui
                        .checkbox(&mut on, format!("{}  ·  {}  ({now})", e.alias, e.hostname))
                        .changed()
                    {
                        if on {
                            a.chosen.insert(e.alias.clone());
                        } else {
                            a.chosen.remove(&e.alias);
                        }
                    }
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let n = a.chosen.len();
            if act_button(ui, n > 0, Act::Add, &il(icons::PLUS, &format!("Add {n}"))).clicked() {
                st(out, StMsg::AddHostsRun);
            }
            if ui.button("Cancel").clicked() {
                st(out, StMsg::AddHostsCancel);
            }
        });
    });
    if modal.should_close() {
        st(out, StMsg::AddHostsCancel);
    }
}
