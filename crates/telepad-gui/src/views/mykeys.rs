//! My keys tab of the Authorized keys page (#101): this computer's key
//! pairs, Generate, Convert / Export and Import.

use eframe::egui::{self, RichText};
use telepad_app::mykeys::{MkMsg, MyKeysDraft, PassMode, authorized_on, used_by};
use telepad_app::{Model, Msg};
use telepad_core::local_keys::{GenType, LocalKey, Target, unsupported};

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button, pal};

fn mk(out: &mut Vec<Msg>, m: MkMsg) {
    out.push(Msg::MyKeys(m));
}

fn name(k: &LocalKey) -> String {
    k.path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| k.path.display().to_string())
}

pub fn my_keys(ui: &mut egui::Ui, m: &Model, d: &mut MyKeysDraft, out: &mut Vec<Msg>) {
    let mine = &m.my_keys;
    ui.horizontal_wrapped(|ui| {
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Generate key pair…"))
            .on_hover_text("A new key pair with ssh-keygen (Ed25519 by default) in ~/.ssh")
            .clicked()
        {
            mk(out, MkMsg::AskGenerate);
        }
        if ui
            .button(il(icons::IMPORT, "Import…"))
            .on_hover_text("Convert a key file in another format (PuTTY, PEM, PKCS#8, PKCS#12, JWK, DER) to OpenSSH")
            .clicked()
        {
            mk(out, MkMsg::PickImport);
        }
        if ui
            .button(il(icons::REFRESH, "Refresh"))
            .on_hover_text("Read ~/.ssh and the inventory's IdentityFile paths again")
            .clicked()
        {
            mk(out, MkMsg::Load);
        }
        ui.add(
            egui::TextEdit::singleline(&mut d.filter)
                .hint_text(format!("{} Filter", icons::SEARCH))
                .desired_width(200.0),
        );
        if mine.loading {
            ui.spinner();
        }
        ui.label(
            RichText::new(format!("in {}", mine.dir.display()))
                .weak()
                .small(),
        );
    });
    ui.add_space(4.0);
    let keys: Vec<&LocalKey> = match &mine.keys {
        None => {
            ui.label(RichText::new("Reading your keys…").weak());
            return;
        }
        Some(Err(e)) => {
            ui.colored_label(pal().red, e);
            return;
        }
        Some(Ok(v)) => v
            .iter()
            .filter(|k| {
                telepad_app::matches_query(
                    &format!(
                        "{} {} {} {}",
                        name(k),
                        k.kind,
                        k.comment,
                        k.fingerprint.as_deref().unwrap_or("")
                    ),
                    &d.filter,
                )
            })
            .collect(),
    };
    if keys.is_empty() {
        ui.label(RichText::new("No key pairs found. Generate key pair… makes one (Ed25519, OpenSSH format) in ~/.ssh.").weak());
    }
    egui::Panel::right("mykeys-detail")
        .resizable(true)
        .default_size(360.0)
        .show(ui, |ui| {
            let sel = d
                .selected
                .as_ref()
                .and_then(|p| keys.iter().find(|k| &k.path == p));
            match sel {
                Some(k) => detail(ui, m, k, out),
                None => {
                    ui.label(RichText::new("Pick a key on the left.").weak());
                }
            }
        });
    egui::ScrollArea::vertical()
        .id_salt("mykeys-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("mykeys-grid")
                .striped(true)
                .num_columns(5)
                .show(ui, |ui| {
                    for h in ["File", "Type", "Format", "Passphrase", "Comment"] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for k in keys {
                        let selected = d.selected.as_ref() == Some(&k.path);
                        let mut text = format!("{} {}", icons::KEY, name(k));
                        if k.error.is_some() || k.permission_warning.is_some() {
                            text.push_str(&format!(" {}", icons::WARNING));
                        }
                        if ui
                            .selectable_label(selected, text)
                            .on_hover_text(k.path.display().to_string())
                            .clicked()
                        {
                            mk(out, MkMsg::Select(k.path.clone()));
                        }
                        ui.label(&k.kind);
                        ui.label(k.format.map(|f| f.label()).unwrap_or("—"));
                        ui.label(if k.encrypted { "yes" } else { "no" });
                        ui.label(RichText::new(&k.comment).weak());
                        ui.end_row();
                    }
                });
        });
    generate_dialog(ui.ctx(), d, out);
    convert_dialog(ui.ctx(), d, out);
}

fn detail(ui: &mut egui::Ui, m: &Model, k: &LocalKey, out: &mut Vec<Msg>) {
    ui.label(RichText::new(name(k)).strong().size(16.0));
    ui.label(RichText::new(k.path.display().to_string()).weak().small());
    ui.add_space(4.0);
    egui::Grid::new("mykeys-detail-grid")
        .num_columns(2)
        .show(ui, |ui| {
            let row = |ui: &mut egui::Ui, a: &str, b: &str| {
                ui.label(RichText::new(a).weak());
                ui.label(b);
                ui.end_row();
            };
            row(ui, "Type", &k.kind);
            row(
                ui,
                "Format",
                k.format.map(|f| f.label()).unwrap_or("not a key"),
            );
            row(
                ui,
                "Passphrase",
                if k.encrypted {
                    "yes"
                } else {
                    "no (anyone with the file can use it)"
                },
            );
            row(ui, "Fingerprint", k.fingerprint.as_deref().unwrap_or("—"));
            row(ui, "Comment", &k.comment);
            row(
                ui,
                ".pub file",
                if k.has_pub_file {
                    "next to it"
                } else {
                    "missing"
                },
            );
            if let Some(t) = k.modified {
                row(
                    ui,
                    "Changed",
                    &format!("{} UTC", telepad_core::history::format_utc(t)),
                );
            }
            let used = used_by(m, k);
            row(
                ui,
                "Logs in to",
                &if used.is_empty() {
                    "— (no IdentityFile points here)".into()
                } else {
                    used.join(", ")
                },
            );
            let on = authorized_on(m, k);
            row(
                ui,
                "Authorized on",
                &if on.is_empty() {
                    "— (in the lists read so far)".into()
                } else {
                    on.join(", ")
                },
            );
        });
    if let Some(w) = &k.permission_warning {
        ui.colored_label(pal().amber, format!("{} {w}", icons::WARNING));
    }
    if let Some(e) = &k.error {
        ui.colored_label(pal().red, format!("{} {e}", icons::X_CIRCLE));
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        let has_pub = k.public_line.is_some();
        if ui
            .add_enabled(has_pub, egui::Button::new(il(icons::COPY, "Copy public key")))
            .clicked()
        {
            mk(out, MkMsg::CopyPublic(k.path.clone()));
        }
        if ui.button(il(icons::FOLDER, "Open folder")).clicked() {
            mk(out, MkMsg::OpenFolder(k.path.clone()));
        }
        if act_button(ui, has_pub, Act::Add, &il(icons::PLUS, "Add to hosts…"))
            .on_hover_text("Add / revoke with this public key filled in (you pick the hosts and confirm there)")
            .clicked()
        {
            mk(out, MkMsg::AddToHosts(k.path.clone()));
        }
        if ui
            .add_enabled(
                k.format.is_some() && k.error.is_none(),
                egui::Button::new(il(icons::EXPORT, "Convert / export…")),
            )
            .on_hover_text("Write this key as PuTTY PPK, PEM, PKCS#8, PKCS#12, JWK, DER or OpenSSH (a new file)")
            .clicked()
        {
            mk(out, MkMsg::AskConvert(k.path.clone()));
        }
    });
}

fn pass_field(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(text)
            .password(true)
            .hint_text(hint)
            .desired_width(260.0),
    );
}

fn generate_dialog(ctx: &egui::Context, d: &mut MyKeysDraft, out: &mut Vec<Msg>) {
    let Some(g) = d.generate.as_mut() else { return };
    egui::Window::new("Generate key pair")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            egui::Grid::new("gen-grid").num_columns(2).show(ui, |ui| {
                ui.label("Type");
                let mut kind = g.kind;
                egui::ComboBox::from_id_salt("gen-type")
                    .selected_text(match kind {
                        GenType::Rsa(_) => "RSA".into(),
                        k => k.label(),
                    })
                    .show_ui(ui, |ui| {
                        for t in [
                            GenType::Ed25519,
                            GenType::Ecdsa256,
                            GenType::Ecdsa384,
                            GenType::Ecdsa521,
                            GenType::Rsa(g.rsa_bits),
                        ] {
                            let label = match t {
                                GenType::Rsa(_) => "RSA".into(),
                                t => t.label(),
                            };
                            ui.selectable_value(&mut kind, t, label);
                        }
                    });
                if kind != g.kind {
                    mk(out, MkMsg::GenKind(kind));
                }
                ui.end_row();
                if matches!(g.kind, GenType::Rsa(_)) {
                    ui.label("Bits");
                    egui::ComboBox::from_id_salt("gen-bits")
                        .selected_text(g.rsa_bits.to_string())
                        .show_ui(ui, |ui| {
                            for b in [2048, 3072, 4096, 8192] {
                                ui.selectable_value(&mut g.rsa_bits, b, b.to_string());
                            }
                        });
                    ui.end_row();
                }
                ui.label("File");
                ui.add(egui::TextEdit::singleline(&mut g.path).desired_width(320.0));
                ui.end_row();
                ui.label("Comment");
                ui.add(egui::TextEdit::singleline(&mut g.comment).desired_width(320.0));
                ui.end_row();
                ui.label("Passphrase");
                pass_field(ui, g.pass.expose_mut(), "recommended");
                ui.end_row();
                ui.label("Repeat");
                pass_field(ui, g.pass2.expose_mut(), "");
                ui.end_row();
            });
            if g.pass.is_empty() {
                ui.colored_label(
                    pal().amber,
                    format!("{} Without a passphrase anyone who gets the file can log in with it.", icons::WARNING),
                );
            }
            ui.label(
                RichText::new("Runs ssh-keygen on this computer; an existing file is never overwritten. The private key is readable by your user only.")
                    .weak()
                    .small(),
            );
            if let Some(e) = &g.error {
                ui.colored_label(pal().red, e);
            }
            ui.horizontal(|ui| {
                if act_button(ui, !g.busy, Act::Add, &il(icons::KEY, "Generate")).clicked() {
                    mk(out, MkMsg::Generate);
                }
                if ui.button("Cancel").clicked() {
                    mk(out, MkMsg::CancelGenerate);
                }
                if g.busy {
                    ui.spinner();
                }
            });
        });
}

fn convert_dialog(ctx: &egui::Context, d: &mut MyKeysDraft, out: &mut Vec<Msg>) {
    let Some(c) = d.convert.as_mut() else { return };
    let title = if c.import {
        "Import key"
    } else {
        "Convert / export key"
    };
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(RichText::new(c.source.display().to_string()).strong());
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    c.format.map(|f| f.label()).unwrap_or("unknown format"),
                    if c.algorithm.is_empty() { "type known after the passphrase" } else { &c.algorithm }
                ))
                .weak(),
            );
            ui.add_space(4.0);
            if c.encrypted {
                ui.horizontal(|ui| {
                    ui.label("Passphrase of this key");
                    pass_field(ui, c.pass_in.expose_mut(), "");
                });
            }
            let alg = telepad_core::local_keys::algorithm_of(&c.algorithm);
            ui.label("Write as");
            for t in Target::ALL {
                let why = alg.as_ref().and_then(|a| unsupported(a, t));
                ui.add_enabled_ui(why.is_none(), |ui| {
                    let r = ui.radio_value(&mut c.target, t, t.label());
                    if let Some(w) = why {
                        r.on_disabled_hover_text(w);
                    }
                });
            }
            if c.target.can_encrypt() {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut c.mode, PassMode::Keep, "Same passphrase");
                    ui.radio_value(&mut c.mode, PassMode::New, "New passphrase");
                    ui.radio_value(&mut c.mode, PassMode::Remove, "No passphrase");
                });
                if c.mode == PassMode::New {
                    ui.horizontal(|ui| {
                        pass_field(ui, c.pass_out.expose_mut(), "new passphrase");
                        pass_field(ui, c.pass_out2.expose_mut(), "repeat");
                    });
                }
                if c.mode == PassMode::Remove || (c.mode == PassMode::Keep && !c.encrypted) {
                    ui.colored_label(
                        pal().amber,
                        format!("{} The new file will have no passphrase.", icons::WARNING),
                    );
                }
            } else if !c.target.is_public() {
                ui.colored_label(
                    pal().amber,
                    format!("{} This format has no passphrase: the private key is written in clear.", icons::WARNING),
                );
            }
            ui.label(
                RichText::new("A new file, chosen in the next dialog; the source is never changed. Private keys are written readable by your user only.")
                    .weak()
                    .small(),
            );
            if let Some(e) = &c.error {
                ui.colored_label(pal().red, e);
            }
            if let Some(done) = &c.done {
                ui.colored_label(pal().green, format!("{} {done}", icons::CHECK_CIRCLE));
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!c.busy, egui::Button::new(il(icons::EXPORT, "Save as…")))
                    .clicked()
                {
                    mk(out, MkMsg::Convert);
                }
                if ui.button(if c.done.is_some() { "Close" } else { "Cancel" }).clicked() {
                    mk(out, MkMsg::CancelConvert);
                }
                if c.busy {
                    ui.spinner();
                }
            });
        });
}
