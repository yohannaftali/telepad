//! Add / edit host window. Fields are drafts; saving goes through [`Msg::EditorSave`].

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Msg};
use telepad_core::config::Access;

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

pub fn editor_window(ctx: &egui::Context, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(ed) = &mut d.editor else { return };
    let mut open = true;
    egui::Window::new(ed.title())
        .id(egui::Id::unique("host-editor"))
        .collapsible(false)
        .resizable(true)
        .default_width(460.0)
        .open(&mut open)
        .show(ctx, |ui| {
            if let Some(src) = &ed.cloned_from {
                ui.label(
                    RichText::new(format!(
                        "User, Port and IdentityFile copied from {src}; enter the new alias and HostName."
                    ))
                    .weak(),
                );
                ui.add_space(4.0);
            }
            egui::Grid::new("edit-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Access");
                    ui.horizontal(|ui| {
                        for (access, text) in [
                            (Access::Ssh, "SSH (monitored)"),
                            (Access::Rdp, "RDP only (Windows)"),
                        ] {
                            if ui.radio(ed.access == access, text).clicked() {
                                ed.set_access(access);
                            }
                        }
                    });
                    ui.end_row();
                    if ed.access == Access::Rdp {
                        ui.label("");
                        ui.label(
                            RichText::new(
                                "No SSH: the app only checks that the RDP port answers. \
                                 User and IdentityFile are not used.",
                            )
                            .weak(),
                        );
                        ui.end_row();
                    }
                    ui.label("Alias (Host)");
                    ui.add(egui::TextEdit::singleline(&mut ed.entry.alias).hint_text("e.g. web01"));
                    ui.end_row();
                    ui.label("Description");
                    ui.add(
                        egui::TextEdit::singleline(&mut ed.entry.note)
                            .hint_text("shown under the host name"),
                    );
                    ui.end_row();
                    ui.label("HostName / IP");
                    ui.add(egui::TextEdit::singleline(&mut ed.entry.hostname).hint_text("192.0.2.58"));
                    ui.end_row();
                    ui.label("User");
                    ui.text_edit_singleline(&mut ed.entry.user);
                    ui.end_row();
                    ui.label("Port");
                    ui.add(egui::TextEdit::singleline(&mut ed.port).desired_width(80.0));
                    ui.end_row();
                    ui.label("IdentityFile");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut ed.identity)
                                .hint_text("~/.ssh/id_ed25519"),
                        );
                        if ui.small_button("Browse…").clicked() {
                            out.push(Msg::EditorBrowseKey);
                        }
                    });
                    ui.end_row();
                    ui.label("Monitored");
                    ui.checkbox(&mut ed.monitored, "poll this host");
                    ui.end_row();
                    ui.label("Other ssh options").on_hover_text(
                        "One 'Keyword value' per line, e.g. KexAlgorithms +diffie-hellman-group1-sha1",
                    );
                    ui.add(
                        egui::TextEdit::multiline(&mut ed.options)
                            .desired_rows(3)
                            .code_editor()
                            .hint_text("ProxyJump bastion"),
                    );
                    ui.end_row();
                });
            if let Some(e) = &ed.error {
                ui.colored_label(usage_color(95.0), e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(il(icons::SAVE, "Save")).clicked() {
                    out.push(Msg::EditorSave);
                }
                if ui.button("Cancel").clicked() {
                    out.push(Msg::EditorClose);
                }
                if ed.entry.id.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ed.confirm_delete {
                            if ui
                                .button(RichText::new("Really delete").color(usage_color(95.0)))
                                .clicked()
                            {
                                out.push(Msg::EditorDelete);
                            }
                            if ui.button("Keep").clicked() {
                                ed.confirm_delete = false;
                            }
                        } else if ui.button(il(icons::TRASH, "Delete")).clicked() {
                            ed.confirm_delete = true;
                        }
                    });
                }
            });
        });
    if !open {
        out.push(Msg::EditorClose);
    }
}
