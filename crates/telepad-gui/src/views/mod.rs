//! Views. Each one reads the [`Model`], may edit [`Drafts`] (form text,
//! filter, toggles) and pushes [`Msg`]s; none of them does I/O or changes the
//! model directly.

mod confirm;
mod detail;
mod docker;
mod editor;
mod import;
mod overview;
mod panels;
mod ports;
mod sensors;
mod update_window;

use std::time::Instant;

use eframe::egui;
use telepad_app::{Drafts, Model, Msg};

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

    egui::Panel::left("hosts")
        .resizable(true)
        .default_size(290.0)
        .show(root, |ui| panels::host_list(ui, m, &d.filter, &mut out));

    egui::CentralPanel::default().show(root, |ui| match m.selected {
        None => overview::overview(ui, m, &d.filter, &mut out),
        Some(i) => {
            if let Some(h) = m.hosts.get(i) {
                detail::detail(ui, h, i, &mut d.ports_nginx_only, &mut out);
            }
        }
    });

    editor::editor_window(&ctx, d, &mut out);
    import::import_window(&ctx, m, &mut out);
    confirm::delete_confirm(&ctx, m, &mut out);
    update_window::update_window(&ctx, m, d, &mut out);
    out
}
