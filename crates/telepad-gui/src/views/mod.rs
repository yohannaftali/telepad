//! Views. Each one reads the [`Model`], may edit [`Drafts`] (form text,
//! filter, toggles) and pushes [`Msg`]s; none of them does I/O or changes the
//! model directly.

mod cards;
mod cluster;
mod confirm;
mod database;
mod detail;
mod docker;
mod docker_overview;
mod editor;
mod files;
mod host_sync;
mod import;
mod keys;
mod overview;
mod panels;
mod ports;
mod run_info;
mod sensors;
mod text_editor;
mod update_window;
mod workspace;

use std::time::Instant;

use eframe::egui;
use telepad_app::workspace::Win;
use telepad_app::{Drafts, Model, Msg};

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

/// Draw one frame and return what the user asked for.
pub fn draw(root: &mut egui::Ui, m: &Model, d: &mut Drafts, now: Instant) -> Vec<Msg> {
    let ctx = root.ctx().clone();
    let mut out = Vec::new();

    egui::Panel::top("top").show(root, |ui| panels::top_bar(ui, m, d, &mut out));
    egui::Panel::bottom("status").show(root, |ui| panels::status_bar(ui, m, now));

    if let Some(f) = &m.fatal {
        egui::CentralPanel::default().show(root, |ui| {
            ui.colored_label(usage_color(95.0), f);
        });
        return out;
    }

    let width = if m.ws.sidebar_collapsed { 64.0 } else { 236.0 };
    egui::Panel::left("pages")
        .resizable(false)
        .exact_size(width)
        .show(root, |ui| workspace::sidebar(ui, m, &mut out));

    egui::CentralPanel::default()
        .frame(egui::Frame::central_panel(&ctx.global_style()).inner_margin(0.0))
        .show(root, |ui| {
            workspace::workspace(
                ui,
                m,
                d,
                &mut out,
                now,
                &mut |ui, m, d, win, focused, out| page(ui, m, d, win, focused, out, now),
            );
        });

    files::close_guard(&ctx, m, d, &mut out);
    text_editor::raise_on_question(&ctx, m, &mut out);
    editor::editor_window(&ctx, d, &mut out);
    import::import_window(&ctx, m, &mut out);
    import::paste_window(&ctx, d, &mut out);
    run_info::run_info_window(&ctx, d, &mut out);
    confirm::delete_confirm(&ctx, m, &mut out);
    update_window::update_window(&ctx, m, d, &mut out);
    host_sync::host_sync_dialogs(&ctx, m, d, &mut out);
    out
}

/// The content of one page's inner window.
fn page(
    ui: &mut egui::Ui,
    m: &Model,
    d: &mut Drafts,
    win: Win,
    focused: bool,
    out: &mut Vec<Msg>,
    now: Instant,
) {
    match win {
        Win::Overview => {
            overview_toolbar(ui, d, out);
            if d.overview_cards {
                cards::cards(ui, m, &d.filter, d.sort, out);
            } else {
                overview::overview(ui, m, &d.filter, d.sort, out);
            }
        }
        Win::Docker => docker_overview::docker_overview(ui, m, &d.filter, out),
        Win::Host => {
            let Some(i) = m.selected else { return };
            let Some(h) = m.hosts.get(i) else { return };
            let shown_by = telepad_app::cluster_shown_by(m, i)
                .and_then(|j| m.hosts.get(j))
                .map(|o| o.entry.alias.clone());
            detail::detail(ui, h, i, shown_by.as_deref(), &mut d.ports_nginx_only, out);
        }
        Win::HostSync => host_sync::host_sync_page(ui, m, d, out),
        Win::Keys => keys::keys_page(ui, m, d, out),
        Win::Files(id) => files::files_page(ui, m, d, id, focused, out, now),
        Win::Db(id) => database::db_page(ui, m, d, id, out),
        Win::Editor(id) => text_editor::editor_page(ui, m, d, id, focused, out),
    }
}

/// Table | Cards switch; in card view also "Sort by" (the table sorts by its headers).
fn overview_toolbar(ui: &mut egui::Ui, d: &Drafts, out: &mut Vec<Msg>) {
    use telepad_app::{Sort, SortKey};
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("View").weak());
        if ui
            .selectable_label(!d.overview_cards, il(icons::OVERVIEW, "Table"))
            .clicked()
        {
            out.push(Msg::SetOverviewCards(false));
        }
        if ui
            .selectable_label(d.overview_cards, il(icons::GAUGE, "Cards"))
            .clicked()
        {
            out.push(Msg::SetOverviewCards(true));
        }
        if !d.overview_cards {
            let why = match (d.filter.trim().is_empty(), d.sort.key.is_none()) {
                (true, true) => None,
                (false, true) => Some("clear the search"),
                (true, false) => Some("click the sorted header until its arrow is gone"),
                (false, false) => {
                    Some("clear the search and click the sorted header until its arrow is gone")
                }
            };
            if let Some(why) = why {
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("{} To reorder hosts, {why}", icons::GRIP))
                        .weak()
                        .small(),
                );
            }
            return;
        }
        ui.separator();
        ui.label(egui::RichText::new("Sort by").weak());
        let current = d.sort.key.map_or("Inventory order", SortKey::title);
        egui::ComboBox::from_id_salt("card-sort")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(d.sort.key.is_none(), "Inventory order")
                    .clicked()
                {
                    out.push(Msg::SetSort(Sort::default()));
                }
                for key in SortKey::ALL {
                    if ui
                        .selectable_label(d.sort.key == Some(key), key.title())
                        .clicked()
                    {
                        out.push(Msg::SetSort(Sort {
                            key: Some(key),
                            descending: key.starts_descending(),
                        }));
                    }
                }
            });
        if let Some(key) = d.sort.key {
            let (icon, tip) = if d.sort.descending {
                (icons::CARET_DOWN, "Highest / Z first — click to reverse")
            } else {
                (icons::CARET_UP, "Lowest / A first — click to reverse")
            };
            if ui.button(icon).on_hover_text(tip).clicked() {
                out.push(Msg::SetSort(Sort {
                    key: Some(key),
                    descending: !d.sort.descending,
                }));
            }
        }
    });
    ui.add_space(4.0);
}
