//! Named workspaces (#94, WinBox style) in the top bar: the workspace
//! dropdown (`<none>`, `<own>`, custom ones with a delete icon; Save and
//! Add… at its foot), the Arrange menu and their dialogs.

use eframe::egui::{self, RichText};
use telepad_app::workspace::{WkMsg, WsMsg};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::prefs::{Current, MAX_LABEL, NONE_LABEL, OWN_LABEL};

use super::picker::{Footer, Item, Picked, Spec, picker, shorten};
use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button};

fn wk(out: &mut Vec<Msg>, m: WkMsg) {
    out.push(Msg::Wk(m));
}

pub fn workspace_picker(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    let current = &m.wss.current;
    let mut choices = vec![Current::None, Current::Own];
    choices.extend(m.wss.custom.iter().map(|(l, _)| Current::Custom(l.clone())));
    let items: Vec<Item> = choices
        .iter()
        .map(|c| Item {
            icon: match c {
                Current::None => icons::X_CIRCLE,
                _ => icons::LAYOUT,
            },
            label: c.label().to_string(),
            current: c == current,
            note: match c {
                Current::None => "no windows",
                Current::Own => "default",
                Current::Custom(_) => "",
            },
            delete: matches!(c, Current::Custom(_))
                .then(|| format!("Delete the workspace {}…", c.label())),
        })
        .collect();
    let footer = [
        Footer {
            icon: icons::SAVE,
            label: "Save",
            enabled: *current != Current::None && m.prefs_ok,
            tip: "Save the windows of this workspace now (changes are saved automatically too)",
        },
        Footer {
            icon: icons::PLUS,
            label: "Add…",
            enabled: m.prefs_ok,
            tip: "A new workspace with the windows that are open now",
        },
    ];
    let spec = Spec {
        button: il(icons::LAYOUT, &shorten(current.label(), 24)),
        tip: "Workspace: a saved set of windows with their places. <none> closes every window",
        enabled: true,
        placeholder: "Search workspaces…",
        empty: "No matching workspace",
        width: 280.0,
    };
    match picker(ui, spec, &items, &footer, &mut d.ws_picker) {
        Some(Picked::Item(i)) => wk(out, WkMsg::Switch(choices[i].clone())),
        Some(Picked::Delete(i)) => wk(out, WkMsg::AskDelete(choices[i].label().to_string())),
        Some(Picked::Footer(0)) => wk(out, WkMsg::Save),
        Some(Picked::Footer(_)) => wk(out, WkMsg::AskAdd),
        None => {}
    }
}

/// Tile, Stack, Minimize all, Restore all (#94).
pub fn arrange_menu(ui: &mut egui::Ui, m: &Model, d: &Drafts, out: &mut Vec<Msg>) {
    let any = !m.ws.groups.is_empty();
    let shown = m.ws.groups.iter().any(|g| !g.minimized);
    let minimized = m.ws.groups.iter().any(|g| g.minimized);
    let [w, h] = d.ws_area;
    ui.add_enabled_ui(any, |ui| {
        ui.menu_button(il(icons::GRID_FOUR, "Arrange"), |ui| {
            for (icon, text, tip, on, msg) in [
                (
                    icons::GRID_FOUR,
                    "Tile",
                    "The shown windows side by side, filling the workspace",
                    shown,
                    WsMsg::Tile { w, h },
                ),
                (
                    icons::BROWSERS,
                    "Stack",
                    "The shown windows cascading from the top left",
                    shown,
                    WsMsg::Stack { w, h },
                ),
                (
                    icons::ARROW_LINE_DOWN,
                    "Minimize all",
                    "Every window to the strip at the bottom",
                    shown,
                    WsMsg::MinimizeAll,
                ),
                (
                    icons::ARROW_LINE_UP,
                    "Restore all",
                    "Every minimized window back",
                    minimized,
                    WsMsg::RestoreAll,
                ),
            ] {
                if ui
                    .add_enabled(on, egui::Button::new(il(icon, text)))
                    .on_hover_text(tip)
                    .clicked()
                {
                    out.push(Msg::Ws(msg));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Arrange the windows: tile, stack, minimize or restore all");
    });
}

/// Add workspace and delete confirmation, over the workspace.
pub fn dialogs(ctx: &egui::Context, d: &mut Drafts, out: &mut Vec<Msg>) {
    if let Some(add) = d.ws_add.as_mut() {
        egui::Window::new("Add workspace")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("A new workspace with the windows that are open now.");
                ui.add_space(4.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut add.label)
                        .hint_text("Name, e.g. Backups")
                        .char_limit(MAX_LABEL + 10)
                        .desired_width(260.0),
                );
                if !r.has_focus() && add.label.is_empty() {
                    r.request_focus();
                }
                if r.changed() {
                    add.error = None;
                }
                if let Some(e) = &add.error {
                    ui.colored_label(crate::widgets::pal().red, e);
                }
                ui.add_space(6.0);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add")).clicked() || enter {
                        wk(out, WkMsg::Add(add.label.clone()));
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        wk(out, WkMsg::CancelAdd);
                    }
                });
                ui.label(
                    RichText::new(format!(
                        "1–{MAX_LABEL} characters; not {NONE_LABEL} or {OWN_LABEL}"
                    ))
                    .weak()
                    .small(),
                );
            });
    }
    if let Some(ask) = d.ws_leave.clone() {
        let title = match &ask.to {
            Some(_) => "Leave this workspace?",
            None => "Close Telepad?",
        };
        // Modal (#99): the rest of Telepad waits until Yes or Back.
        egui::Modal::new(egui::Id::unique("leave-workspace")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(title);
            ui.add_space(4.0);
            ui.label(match &ask.to {
                Some(c) => format!(
                    "Going to {} resets these windows; what is listed below is lost:",
                    c.label()
                ),
                None => "Closing Telepad loses what is listed below:".to_string(),
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    let mut last = "";
                    for (win, what) in &ask.items {
                        if win != last {
                            ui.label(RichText::new(win).strong());
                            last = win;
                        }
                        ui.label(format!("   {} {what}", icons::WARNING));
                    }
                });
            if ask.background > 0 {
                ui.add_space(4.0);
                ui.label(format!(
                    "{} {} file transfer(s) keep running in the background.",
                    icons::INFO,
                    ask.background
                ));
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("To keep them, choose Back, then save or apply them first.").weak(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if act_button(ui, true, Act::Danger, &il(icons::CHECK, "Yes")).clicked() {
                    wk(out, WkMsg::LeaveYes);
                }
                if ui.button(il(icons::ARROW_LEFT, "Back")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    wk(out, WkMsg::LeaveBack);
                }
            });
        });
    }
    if let Some(label) = d.ws_delete.clone() {
        egui::Window::new("Delete workspace")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("Delete the workspace {label}?"));
                ui.label(
                    RichText::new(
                        "Only the saved set of windows goes; hosts and files are not touched.",
                    )
                    .weak(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
                        wk(out, WkMsg::Delete(label.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        wk(out, WkMsg::CancelDelete);
                    }
                });
            });
    }
}
