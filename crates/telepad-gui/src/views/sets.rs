//! Groups and Clusters pages (#144, #145): the list (add, edit, reorder,
//! delete; a fixed entry for hosts without one) and the "Hosts - [name]"
//! page of one group / cluster (same table / cards as the Hosts page).

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
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
    let rows = shown.len() + usize::from(none_shown);
    ui.style_mut().interaction.selectable_labels = false;
    TableBuilder::new(ui)
        .id_salt(("sets", kind.as_str()))
        .striped(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(200.0).at_least(100.0).clip(true))
        .column(Column::remainder().at_least(120.0).clip(true))
        .column(Column::exact(60.0))
        .column(Column::exact(60.0))
        .column(Column::exact(60.0))
        .column(Column::exact(250.0))
        .auto_shrink([false, false])
        .header(24.0, |mut h| {
            for t in [kind.title(), "Description", "Hosts", "Down", "Flags", ""] {
                h.col(|ui| {
                    cell_text(ui, RichText::new(t).strong());
                });
            }
        })
        .body(|body| {
            body.rows(26.0, rows, |mut row| {
                let at = row.index();
                // The fixed entry ("Ungrouped" / "No cluster") is always last.
                let (name, description, fixed) = match shown.get(at) {
                    Some(&i) => (sets[i].name.clone(), sets[i].description.clone(), false),
                    None => (String::new(), String::new(), true),
                };
                let s = stats(&m.hosts, kind, &name);
                row.col(|ui| {
                    let text = RichText::new(il(icon(kind), shown_name(kind, &name))).strong();
                    ui.label(if fixed { text.italics() } else { text });
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(&description).weak()).truncate());
                });
                row.col(|ui| {
                    ui.label(s.hosts.to_string());
                });
                row.col(|ui| {
                    if s.down > 0 {
                        ui.colored_label(usage_color(95.0), s.down.to_string());
                    } else {
                        ui.label(RichText::new("0").weak());
                    }
                });
                row.col(|ui| {
                    if s.flagged > 0 {
                        ui.colored_label(usage_color(75.0), s.flagged.to_string());
                    } else {
                        ui.label(RichText::new("0").weak());
                    }
                });
                let mut button = false;
                row.col(|ui| {
                    if fixed {
                        ui.label(RichText::new("fixed").weak().small())
                            .on_hover_text("Hosts without one; cannot be renamed or deleted");
                        return;
                    }
                    let pos = shown[at];
                    if ui
                        .add_enabled(
                            pos > 0 && query.is_empty(),
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
                            pos + 1 < sets.len() && query.is_empty(),
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
                });
                let resp = row.response();
                if resp.clicked() && !button {
                    st(
                        out,
                        StMsg::OpenHosts {
                            kind,
                            name: name.clone(),
                        },
                    );
                }
                resp.on_hover_text(format!("Click to open Hosts - {}", shown_name(kind, &name)));
            });
        });
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
        by_group: false,
        salt: id,
    };
    if cards {
        let rows = overview::view_rows(m, &filter, d.sort, &view);
        cards::cards(ui, m, &rows, out);
    } else {
        overview::table(ui, m, &filter, d.sort, &view, out);
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
