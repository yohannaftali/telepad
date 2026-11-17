//! Import preview dialog (Merge / Replace).

use eframe::egui::{self, RichText};
use telepad_app::{Model, Msg};
use telepad_core::store::ImportMode;

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

pub fn import_window(ctx: &egui::Context, m: &Model, out: &mut Vec<Msg>) {
    let Some(d) = &m.import else { return };
    let mut open = true;
    egui::Window::new("Import hosts")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(RichText::new(d.path.display().to_string()).monospace());
            ui.label(format!("{} hosts in file", d.inventory.hosts.len()));
            if !d.inventory.global.is_empty() {
                ui.label(RichText::new("Includes global options (Host * / Match)").weak());
            }
            ui.add_space(6.0);
            for (mode, text) in [
                (
                    ImportMode::Merge,
                    "Merge — add new, update same alias, keep others",
                ),
                (
                    ImportMode::Replace,
                    "Replace — hosts become exactly this file",
                ),
            ] {
                if ui.radio(d.mode == mode, text).clicked() {
                    out.push(Msg::ImportMode(mode));
                }
            }
            ui.add_space(6.0);
            match &d.preview {
                Ok(p) => {
                    ui.label(format!(
                        "{} added · {} updated · {} unchanged",
                        p.added, p.updated, p.unchanged
                    ));
                    if p.removed > 0 {
                        ui.colored_label(
                            usage_color(95.0),
                            format!("{} host(s) not in the file will be deleted", p.removed),
                        );
                    }
                }
                Err(e) => {
                    ui.colored_label(usage_color(95.0), e);
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        d.preview.is_ok(),
                        egui::Button::new(il(icons::IMPORT, "Import")),
                    )
                    .clicked()
                {
                    out.push(Msg::ImportApply);
                }
                if ui.button("Cancel").clicked() {
                    out.push(Msg::ImportCancel);
                }
            });
        });
    if !open {
        out.push(Msg::ImportCancel);
    }
}
