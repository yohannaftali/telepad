//! Delete confirmation dialog.

use eframe::egui::{self, RichText};
use telepad_app::{Model, Msg};

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button};

pub fn delete_confirm(ctx: &egui::Context, m: &Model, out: &mut Vec<Msg>) {
    let Some((_, alias)) = &m.pending_delete else {
        return;
    };
    egui::Window::new("Delete host")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(format!("Delete {alias} from the host database?"));
            ui.label(
                RichText::new(
                    "Its ssh config entry is removed too. Export first if you may need it back.",
                )
                .weak(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
                    out.push(Msg::ConfirmDelete);
                }
                if ui.button("Cancel").clicked() {
                    out.push(Msg::CancelDelete);
                }
            });
        });
}

/// A PDF export of many rows asks first (#153, as wtbapp: above 5000 rows).
pub fn table_export_confirm(ctx: &egui::Context, m: &Model, out: &mut Vec<Msg>) {
    let Some((data, _)) = &m.table_ask else {
        return;
    };
    use telepad_app::grid::TableMsg;
    egui::Window::new("Export as PDF")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(format!(
                "Export {} rows of {} as PDF?",
                data.rows.len(),
                data.title
            ));
            ui.label(
                RichText::new("A PDF this long takes a while and runs to many pages; CSV or Excel may suit better.")
                    .weak(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(il(icons::FILE_PDF, "Export")).clicked() {
                    out.push(Msg::Table(TableMsg::Answer(true)));
                }
                if ui.button("Cancel").clicked() {
                    out.push(Msg::Table(TableMsg::Answer(false)));
                }
            });
        });
}
