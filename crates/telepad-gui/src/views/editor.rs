//! Add / edit host window. Fields are drafts; saving goes through [`Msg::EditorSave`].

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Msg};
use telepad_core::config::Access;

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button, usage_color};

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
                        if !ed.key_choices.is_empty() {
                            ui.menu_button(icons::CARET_DOWN, |ui| {
                                ui.label(RichText::new("Keys in ~/.ssh").weak().small());
                                for k in &ed.key_choices {
                                    if ui.button(k).clicked() {
                                        ed.identity = k.clone();
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Choose a key found in ~/.ssh");
                        }
                        if ui
                            .small_button("Browse…")
                            .on_hover_text(
                                "Opens in ~/.ssh. On macOS press Cmd+Shift+. in the dialog to \
                                 show hidden folders; you can also type a path such as \
                                 ~/.ssh/id_ed25519.",
                            )
                            .clicked()
                        {
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
            rdp_section(ui, ed, out);
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
                            if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Really delete"))
                                .clicked()
                            {
                                out.push(Msg::EditorDelete);
                            }
                            if ui.button("Keep").clicked() {
                                ed.confirm_delete = false;
                            }
                        } else if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
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

/// Saved Remote Desktop settings (#102): open for RDP-only hosts, optional
/// for SSH hosts. No password field: mstsc asks for it (and can remember it).
fn rdp_section(ui: &mut egui::Ui, ed: &mut telepad_app::Editor, out: &mut Vec<Msg>) {
    use telepad_core::rdp::{Audio, AuthLevel, GatewayUse, Screen};
    egui::CollapsingHeader::new(il(icons::MONITOR, "Remote Desktop settings"))
        .id_salt("host-editor-rdp")
        .default_open(ed.access == Access::Rdp || ed.rdp_on)
        .show(ui, |ui| {
            ui.checkbox(
                &mut ed.rdp_on,
                "Save Remote Desktop settings for this host (used by Open RDP)",
            );
            ui.horizontal(|ui| {
                if ui
                    .button(il(icons::IMPORT, "Import .rdp…"))
                    .on_hover_text("Fill these settings from a .rdp file saved by Remote Desktop Connection (a saved password in it is ignored)")
                    .clicked()
                {
                    out.push(Msg::EditorRdpImport);
                }
                if ui
                    .add_enabled(ed.rdp_on, egui::Button::new(il(icons::EXPORT, "Export .rdp…")))
                    .on_hover_text("Save these settings as a .rdp file")
                    .clicked()
                {
                    out.push(Msg::EditorRdpExport);
                }
            });
            if !ed.rdp_on {
                ui.label(
                    RichText::new("Off: Open RDP connects to HostName:Port with the client's defaults.")
                        .weak()
                        .small(),
                );
                return;
            }
            let r = &mut ed.rdp;
            egui::Grid::new("rdp-grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Address");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut r.address)
                                .hint_text("empty: HostName")
                                .desired_width(200.0),
                        );
                        ui.label("Port");
                        ui.add(
                            egui::TextEdit::singleline(&mut ed.rdp_port)
                                .hint_text("3389")
                                .desired_width(60.0),
                        );
                    });
                    ui.end_row();
                    ui.label("User / domain");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut r.user)
                                .hint_text("user")
                                .desired_width(140.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut r.domain)
                                .hint_text("domain")
                                .desired_width(110.0),
                        );
                    });
                    ui.end_row();
                    ui.label("Display");
                    ui.horizontal(|ui| {
                        let window = matches!(r.screen, Screen::Window { .. });
                        if ui.radio(r.screen == Screen::FullScreen, "Full screen").clicked() {
                            r.screen = Screen::FullScreen;
                        }
                        if ui.radio(window, "Window").clicked() && !window {
                            r.screen = Screen::Window {
                                width: 1600,
                                height: 900,
                            };
                        }
                        if ui.radio(r.screen == Screen::Fit, "Fit the window").clicked() {
                            r.screen = Screen::Fit;
                        }
                    });
                    ui.end_row();
                    if matches!(r.screen, Screen::Window { .. }) {
                        ui.label("Size");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.rdp_width).desired_width(60.0));
                            ui.label("×");
                            ui.add(egui::TextEdit::singleline(&mut ed.rdp_height).desired_width(60.0));
                        });
                        ui.end_row();
                    }
                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut r.multimon, "All monitors");
                        if r.screen != Screen::Fit {
                            ui.checkbox(&mut r.scale, "Scale to the window").on_hover_text(
                                "smart sizing: the remote desktop keeps its size and is shrunk or stretched to the window",
                            );
                        }
                        egui::ComboBox::from_id_salt("rdp-bpp")
                            .selected_text(format!("{} bit colour", r.color_depth))
                            .show_ui(ui, |ui| {
                                for b in [32u8, 24, 16, 15] {
                                    ui.selectable_value(&mut r.color_depth, b, format!("{b} bit"));
                                }
                            });
                    });
                    ui.end_row();
                    ui.label("Bring along");
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut r.clipboard, "Clipboard");
                        ui.checkbox(&mut r.printers, "Printers");
                        ui.label("Drives");
                        ui.add(
                            egui::TextEdit::singleline(&mut r.drives)
                                .hint_text("none, * or C:;D:")
                                .desired_width(90.0),
                        );
                    });
                    ui.end_row();
                    ui.label("Sound");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut r.audio, Audio::Local, "Here");
                        ui.radio_value(&mut r.audio, Audio::Remote, "On the remote computer");
                        ui.radio_value(&mut r.audio, Audio::Off, "Off");
                    });
                    ui.end_row();
                    ui.label("Session");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut r.admin, "Admin session (/admin)");
                        ui.checkbox(&mut r.autodetect, "Detect the network");
                    });
                    ui.end_row();
                    ui.label("RD Gateway");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut r.gateway)
                                .hint_text("gateway host")
                                .desired_width(160.0),
                        );
                        egui::ComboBox::from_id_salt("rdp-gw")
                            .selected_text(match r.gateway_use {
                                GatewayUse::Never => "not used",
                                GatewayUse::Always => "always",
                                GatewayUse::Detect => "when needed",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut r.gateway_use, GatewayUse::Never, "not used");
                                ui.selectable_value(&mut r.gateway_use, GatewayUse::Always, "always");
                                ui.selectable_value(&mut r.gateway_use, GatewayUse::Detect, "when needed");
                            });
                    });
                    ui.end_row();
                    ui.label("Server check");
                    egui::ComboBox::from_id_salt("rdp-auth")
                        .selected_text(match r.auth_level {
                            AuthLevel::Warn => "warn me",
                            AuthLevel::Refuse => "do not connect",
                            AuthLevel::Connect => "connect anyway",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut r.auth_level, AuthLevel::Warn, "warn me");
                            ui.selectable_value(&mut r.auth_level, AuthLevel::Refuse, "do not connect");
                            ui.selectable_value(&mut r.auth_level, AuthLevel::Connect, "connect anyway");
                        });
                    ui.end_row();
                });
            if !r.extra.is_empty() {
                egui::CollapsingHeader::new(format!(
                    "{} more line(s) from the imported file",
                    r.extra.len()
                ))
                .id_salt("rdp-extra")
                .show(ui, |ui| {
                    for l in &r.extra {
                        ui.label(RichText::new(l).monospace().small());
                    }
                    if ui.small_button("Drop them").clicked() {
                        r.extra.clear();
                    }
                });
            }
            rdp_password(ui, ed, out);
        });
}

/// Windows Credential Manager (#108): the password goes to Windows, never
/// into Telepad's files.
fn rdp_password(ui: &mut egui::Ui, ed: &mut telepad_app::Editor, out: &mut Vec<Msg>) {
    if !telepad_core::platform::RDP_CREDENTIALS {
        ui.label(
            RichText::new("No password is stored: Remote Desktop asks for it and keeps it in its own keychain.")
                .weak()
                .small(),
        );
        return;
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Password").strong());
        let state = match ed.rdp_cred_saved {
            None => "checking…".to_string(),
            Some(true) => format!(
                "saved in Windows Credential Manager for {}",
                ed.rdp_address()
            ),
            Some(false) => "not saved: Remote Desktop asks for it".to_string(),
        };
        ui.label(RichText::new(state).weak());
    });
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(ed.rdp_cred_pass.expose_mut())
                .password(true)
                .hint_text("password")
                .desired_width(140.0),
        );
        ui.add(
            egui::TextEdit::singleline(ed.rdp_cred_pass2.expose_mut())
                .password(true)
                .hint_text("repeat")
                .desired_width(140.0),
        );
        if act_button(
            ui,
            true,
            Act::Add,
            &il(icons::LOCK, "Save in Credential Manager"),
        )
        .on_hover_text(format!(
            "Windows keeps it as TERMSRV/{} for {}; Telepad never stores it",
            ed.rdp_address(),
            ed.rdp_login()
        ))
        .clicked()
        {
            out.push(Msg::EditorRdpCredSave);
        }
        if ed.rdp_cred_saved == Some(true)
            && act_button(ui, true, Act::Danger, &il(icons::TRASH, "Forget")).clicked()
        {
            out.push(Msg::EditorRdpCredForget);
        }
    });
    match &ed.rdp_cred_note {
        Some(Ok(t)) => {
            ui.colored_label(crate::widgets::pal().green, t);
        }
        Some(Err(e)) => {
            ui.colored_label(crate::widgets::pal().red, e);
        }
        None => {}
    }
}
