//! Delete confirmation dialog.

use eframe::egui::{self, RichText};
use telepad_app::{Model, Msg};

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

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
                if ui
                    .button(RichText::new(il(icons::TRASH, "Delete")).color(usage_color(95.0)))
                    .clicked()
                {
                    out.push(Msg::ConfirmDelete);
                }
                if ui.button("Cancel").clicked() {
                    out.push(Msg::CancelDelete);
                }
            });
        });
}
