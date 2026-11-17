//! "Software update" window: release info, install/restart, update source settings.

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Model, Msg, UpdatePhase};
use telepad_core::update::{self, CURRENT_VERSION, DEFAULT_REPO};

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

pub fn update_window(ctx: &egui::Context, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let u = &m.update;
    if !u.window {
        return;
    }
    let mut open = true;
    egui::Window::new(il(icons::IMPORT, "Software update"))
        .id(egui::Id::unique("update-window"))
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("Installed: {CURRENT_VERSION}"));
            match &u.check {
                Some(c) => {
                    let repo = u.config.as_ref().map_or("", |x| x.repo.as_str());
                    ui.label(format!("Latest release of {repo}: {}", c.version));
                    if let Some(n) = &c.notes {
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .show(ui, |ui| ui.label(RichText::new(n).weak()));
                    }
                    if !c.page_url.is_empty() && ui.link("Release page on GitHub").clicked() {
                        out.push(Msg::OpenUrl(c.page_url.clone()));
                    }
                    if c.newer && c.asset.is_none() {
                        ui.colored_label(
                            usage_color(80.0),
                            format!(
                                "This release has no {} download.",
                                update::asset_name(&c.version)
                            ),
                        );
                    }
                }
                None => {
                    ui.label(RichText::new("Not checked yet").weak());
                }
            }
            if let Some(e) = &u.error {
                ui.colored_label(usage_color(95.0), e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| match &u.phase {
                UpdatePhase::Checking => {
                    ui.spinner();
                    ui.label("Checking…");
                }
                UpdatePhase::Installing => {
                    ui.spinner();
                    ui.label("Downloading and installing…");
                }
                UpdatePhase::Installed(v) => {
                    ui.label(format!("{v} is installed."));
                    if ui.button(il(icons::REFRESH, "Restart now")).clicked() {
                        out.push(Msg::Restart);
                    }
                }
                UpdatePhase::Idle => {
                    let can_install = u.can_install();
                    if can_install
                        && ui
                            .button(RichText::new(il(icons::IMPORT, "Update now")).strong())
                            .clicked()
                    {
                        out.push(Msg::UpdateInstall);
                    }
                    if can_install && ui.button("Later").clicked() {
                        out.push(Msg::UpdateLater);
                    }
                    if ui
                        .add_enabled(
                            u.config.is_some(),
                            egui::Button::new(il(icons::REFRESH, "Check now")),
                        )
                        .clicked()
                    {
                        out.push(Msg::UpdateCheck);
                    }
                }
            });
            ui.add_space(6.0);
            egui::CollapsingHeader::new("Update source")
                .default_open(u.error.is_some())
                .show(ui, |ui| {
                    egui::Grid::new("update-settings")
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Check for updates");
                            ui.checkbox(&mut d.update_form.enabled, "at start and every 6 h");
                            ui.end_row();
                            ui.label("GitHub repository");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.update_form.repo)
                                    .hint_text(DEFAULT_REPO),
                            );
                            ui.end_row();
                        });
                    ui.label(
                        RichText::new(
                            "Reads the latest GitHub release over HTTPS (no account needed) \
                             and checks the download against its .sha256 file.",
                        )
                        .weak()
                        .small(),
                    );
                    if let Some(e) = &d.update_form_error {
                        ui.colored_label(usage_color(95.0), e);
                    }
                    if ui.button(il(icons::SAVE, "Save")).clicked() {
                        out.push(Msg::UpdateSaveSettings);
                    }
                });
        });
    if !open {
        out.push(Msg::UpdateWindow(false));
    }
}
