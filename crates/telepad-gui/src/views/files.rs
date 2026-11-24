//! File windows (#62–#64): WinSCP-style, this computer on the left and the
//! host on the right, transfer queue at the bottom; edit / new / rename /
//! delete / permissions / properties from each pane's toolbar, right-click
//! menu and keys (F4, F7, F2, Del). One inner window of the workspace per
//! host (#77), plus its editor window ([`super::text_editor`]).

use std::time::Instant;

use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use telepad_app::editor::EdMsg;
use telepad_app::files::{
    DialogKind, FileDraft, FileSession, FmMsg, Mode, OpKind, Pane, Side, SortCol, Status, TState,
    history_shown, op_blocked, other, parse_mode,
};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::db::human_size;
use telepad_core::files::ops::Measure;
use telepad_core::files::transfer::{Decision, Direction};
use telepad_core::files::{Entry, Kind, perm_text, time_text};
use telepad_core::history::{Entry as HistEntry, Kind as HistKind, Outcome};

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button, act_small, act_text, usage_color};

const ROW: f32 = 22.0;

fn send(out: &mut Vec<Msg>, m: FmMsg) {
    out.push(Msg::Files(m));
}

fn red() -> Color32 {
    usage_color(95.0)
}

fn green() -> Color32 {
    usage_color(10.0)
}

/// Files dragged from one pane.
struct DragFiles {
    window: u64,
    from: Side,
    names: Vec<String>,
}

/// Telepad itself closing: when work would be lost (unsaved editor text,
/// uncommitted database edits, running transfers …) the leave question
/// lists it first (#99); after Yes (or with nothing to lose) it closes.
pub fn close_guard(ctx: &egui::Context, m: &Model, d: &Drafts, out: &mut Vec<Msg>) {
    if m.quit {
        if !ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        return;
    }
    if !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    if d.ws_leave.is_none() {
        out.push(Msg::Wk(telepad_app::workspace::WkMsg::AskQuit));
    }
}

/// One host's file window: an inner window of the workspace. `focused` =
/// it is the top window, so its keys (F2, F4, F5, F7, Del) are live.
pub fn files_page(
    ui: &mut egui::Ui,
    m: &Model,
    d: &mut Drafts,
    id: u64,
    focused: bool,
    out: &mut Vec<Msg>,
    now: Instant,
) {
    let Some(s) = m.files.iter().find(|s| s.id == id) else {
        return;
    };
    let Some(dr) = d.files.get_mut(&s.id) else {
        return;
    };
    window(ui, s, dr, focused, out, now);
    ask(ui, s, dr, out);
    dialog(ui, s, dr, out);
}

fn window(
    ui: &mut egui::Ui,
    s: &FileSession,
    d: &mut FileDraft,
    focused: bool,
    out: &mut Vec<Msg>,
    now: Instant,
) {
    let id = s.id;
    egui::Panel::top(("files-top", id)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(il(icons::FOLDER, &format!("Files — {}", s.host.alias))).strong(),
            );
            ui.label(RichText::new(s.host.target()).monospace().weak());
            match (s.status, s.mode) {
                // A failed connect shows its error and Retry below, not a spinner.
                (Status::Connecting, _) if s.error.is_some() => {}
                (Status::Connecting, _) => {
                    ui.spinner();
                    ui.label("connecting…");
                }
                (Status::Ready, Mode::Sftp) => {
                    ui.label(RichText::new("SFTP").weak());
                }
                (Status::Ready, Mode::Fallback) => {
                    ui.label(
                        RichText::new(format!(
                            "{} no SFTP on the host: copies with scp",
                            icons::WARNING
                        ))
                        .color(usage_color(80.0)),
                    );
                }
            }
            ui.separator();
            ui.checkbox(&mut d.show_hidden, "Hidden files");
            match &s.rsync {
                Some(Ok(())) => {
                    ui.checkbox(&mut d.use_rsync, "Use rsync").on_hover_text(
                        "Copies with rsync over this host's SSH login; shows a dry run first",
                    );
                }
                Some(Err(why)) => {
                    ui.label(RichText::new(format!("rsync: {why}")).weak().small());
                }
                None => {}
            }
        });
    });
    // In a small window the file lists keep most of the height.
    let max_queue = (ui.available_height() * 0.4).max(90.0);
    egui::Panel::bottom(("files-queue", id))
        .resizable(true)
        .default_size(190.0f32.min(max_queue))
        .max_size(max_queue)
        .show(ui, |ui| queue(ui, s, d, out, now));
    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(e) = &s.error {
            ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
            if ui.button(il(icons::REFRESH, "Retry")).clicked() {
                send(out, FmMsg::Retry(id));
            }
            return;
        }
        if s.status != Status::Ready {
            ui.spinner();
            return;
        }
        // Keys act on the pane clicked last, never while typing in a field
        // or while a dialog is open.
        let typing = ui.ctx().egui_wants_keyboard_input();
        if focused
            && !typing
            && s.dialog.is_none()
            && s.ask.is_none()
            && let Some(side) = d.active
        {
            let key = |k| ui.input(|i| i.key_pressed(k));
            let op = if key(egui::Key::F4) {
                Some(OpKind::Edit)
            } else if key(egui::Key::F7) {
                Some(OpKind::NewFolder)
            } else if key(egui::Key::F2) {
                Some(OpKind::Rename)
            } else if key(egui::Key::Delete) {
                Some(OpKind::Delete)
            } else {
                None
            };
            if let Some(op) = op {
                send(out, FmMsg::Ask { id, side, op });
            }
        }
        if let Some(n) = &s.notice {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} {n}", icons::INFO));
                if ui.small_button("OK").clicked() {
                    send(out, FmMsg::DismissErrors(id));
                }
            });
        }
        if !s.op_errors.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    red(),
                    format!(
                        "{} {} item(s) could not be changed:",
                        icons::X_CIRCLE,
                        s.op_errors.len()
                    ),
                );
                if ui.small_button("Dismiss").clicked() {
                    send(out, FmMsg::DismissErrors(id));
                }
            });
            egui::ScrollArea::vertical()
                .id_salt(("files-op-errors", id))
                .max_height(70.0)
                .show(ui, |ui| {
                    for e in &s.op_errors {
                        ui.label(RichText::new(e).monospace().small());
                    }
                });
        }
        if focused
            && ui.input(|i| i.key_pressed(egui::Key::F5))
            && let Some(side) = d.active
        {
            let names: Vec<String> = d.selected(side).iter().cloned().collect();
            send(
                out,
                FmMsg::Copy {
                    id,
                    from: side,
                    names,
                },
            );
        }
        ui.columns(2, |cols| {
            pane(&mut cols[0], s, d, Side::Local, out);
            pane(&mut cols[1], s, d, Side::Remote, out);
        });
    });
}

fn pane(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, side: Side, out: &mut Vec<Msg>) {
    // A narrow window: the table scrolls inside its own half, never over the other.
    let half = ui.max_rect().intersect(ui.clip_rect());
    ui.set_clip_rect(half);
    let id = s.id;
    let p: &Pane = s.pane(side);
    ui.horizontal(|ui| {
        let (icon, title) = match side {
            Side::Local => (icons::DESKTOP, "This computer".to_string()),
            Side::Remote => (icons::DISKS, s.host.alias.clone()),
        };
        ui.label(RichText::new(il(icon, &title)).strong());
        if p.loading {
            ui.spinner();
        }
        let n = d.selected(side).len();
        if n > 0 {
            ui.label(RichText::new(format!("{n} selected")).weak());
        }
    });
    ui.horizontal(|ui| {
        if ui
            .small_button(icons::CARET_UP)
            .on_hover_text("Up")
            .clicked()
        {
            send(out, FmMsg::Up { id, side });
        }
        if ui
            .small_button(icons::OVERVIEW)
            .on_hover_text("Home")
            .clicked()
        {
            send(out, FmMsg::Home { id, side });
        }
        if ui
            .small_button(icons::REFRESH)
            .on_hover_text("Refresh")
            .clicked()
        {
            send(out, FmMsg::Refresh { id, side });
        }
        let path = match side {
            Side::Local => &mut d.local_path,
            Side::Remote => &mut d.remote_path,
        };
        let w = (ui.available_width() - 150.0).max(120.0);
        let r = ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(w)
                .font(egui::TextStyle::Monospace),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            send(
                out,
                FmMsg::Navigate {
                    id,
                    side,
                    path: path.clone(),
                },
            );
        }
        let filter = match side {
            Side::Local => &mut d.local_filter,
            Side::Remote => &mut d.remote_filter,
        };
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text(format!("{} filter", icons::SEARCH))
                .desired_width(130.0),
        );
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        toolbar(ui, s, d, side, out);
    });
    if let Some(e) = &p.error {
        ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
    }
    let filter = match side {
        Side::Local => d.local_filter.clone(),
        Side::Remote => d.remote_filter.clone(),
    };
    let rows: Vec<Entry> = p
        .sorted(d.show_hidden, &filter)
        .into_iter()
        .cloned()
        .collect();
    let has_up = p.path != "/"
        && !(side == Side::Local && telepad_core::files::local_parent(&p.path).is_none());
    // Own fill: the drop zone only highlights while something is dragged.
    let frame = egui::Frame::new()
        .inner_margin(2.0)
        .fill(ui.visuals().extreme_bg_color);
    let (_, dropped) = ui.dnd_drop_zone::<DragFiles, ()>(frame, |ui| {
        table(ui, s, d, side, p, &rows, has_up, out);
    });
    if let Some(drag) = dropped
        && drag.window == id
        && drag.from == other(side)
    {
        send(
            out,
            FmMsg::Copy {
                id,
                from: drag.from,
                names: drag.names.clone(),
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn table(
    ui: &mut egui::Ui,
    s: &FileSession,
    d: &mut FileDraft,
    side: Side,
    p: &Pane,
    rows: &[Entry],
    has_up: bool,
    out: &mut Vec<Msg>,
) {
    let id = s.id;
    let remote = side == Side::Remote;
    let n = rows.len() + usize::from(has_up);
    let height = ui.available_height().max(120.0);
    let mut tb = TableBuilder::new(ui)
        .id_salt(("files-table", id, side))
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .max_scroll_height(height)
        // The whole row selects, opens and drags (#130); cells are plain text.
        .sense(egui::Sense::click_and_drag())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(190.0).at_least(100.0).clip(true))
        .column(Column::initial(70.0).clip(true))
        .column(Column::initial(108.0).clip(true))
        .column(Column::initial(76.0).clip(true));
    if remote {
        tb = tb.column(Column::remainder().at_least(60.0).clip(true));
    }
    let arrow = |col: SortCol| {
        if p.sort == col {
            if p.descending {
                format!(" {}", icons::CARET_DOWN)
            } else {
                format!(" {}", icons::CARET_UP)
            }
        } else {
            String::new()
        }
    };
    let mut cols = vec![
        (SortCol::Name, "Name"),
        (SortCol::Size, "Size"),
        (SortCol::Modified, "Modified (UTC)"),
        (SortCol::Permissions, "Permissions"),
    ];
    if remote {
        cols.push((SortCol::Owner, "Owner"));
    }
    tb.header(ROW, |mut h| {
        for (col, title) in &cols {
            h.col(|ui| {
                if ui
                    .add(
                        egui::Label::new(RichText::new(format!("{title}{}", arrow(*col))).strong())
                            .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    send(
                        out,
                        FmMsg::Sort {
                            id,
                            side,
                            col: *col,
                        },
                    );
                }
            });
        }
    })
    .body(|body| {
        let order: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        body.rows(ROW, n, |mut row| {
            let i = row.index();
            if has_up && i == 0 {
                row.set_selected(d.parent_selected.contains(&side));
                row.col(|ui| {
                    cell(ui, format!("{} ..", icons::FOLDER));
                });
                for _ in 1..cols.len() {
                    row.col(|_| {});
                }
                let r = row.response();
                if r.double_clicked() {
                    send(out, FmMsg::Up { id, side });
                } else if r.clicked() {
                    d.active = Some(side);
                    d.click_parent(side);
                }
                return;
            }
            let e = &rows[i - usize::from(has_up)];
            let selected = d.selected(side).contains(&e.name);
            row.set_selected(selected);
            row.col(|ui| {
                let icon = match (e.is_dir(), e.kind) {
                    (true, _) => icons::FOLDER,
                    (false, Kind::Link) => icons::ARROW_RIGHT,
                    _ => icons::INFO,
                };
                let mut t = RichText::new(format!("{icon} {}", e.name));
                if e.hidden {
                    t = t.weak();
                }
                cell(ui, t);
            });
            row.col(|ui| {
                if !e.is_dir() {
                    cell(ui, human_size(e.size));
                }
            });
            row.col(|ui| {
                cell(ui, RichText::new(time_text(e.mtime)).monospace().small());
            });
            row.col(|ui| {
                cell(ui, RichText::new(perm_text(e.perm)).monospace().small());
            });
            if remote {
                row.col(|ui| {
                    cell(ui, e.owner.as_str());
                });
            }
            // One response for the whole row: click (select), double-click
            // (open), drag (copy), right-click (menu).
            let name_resp = row.response();
            if name_resp.drag_started() {
                let names: Vec<String> = if selected {
                    d.selected(side).iter().cloned().collect()
                } else {
                    vec![e.name.clone()]
                };
                egui::DragAndDrop::set_payload(
                    &name_resp.ctx,
                    DragFiles {
                        window: id,
                        from: side,
                        names,
                    },
                );
            }
            if name_resp.dragged() {
                name_resp.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if name_resp.secondary_clicked() && !selected {
                // Right-click acts on the row under the mouse.
                let sel = d.selected_mut(side);
                sel.clear();
                sel.insert(e.name.clone());
            }
            if name_resp.secondary_clicked() {
                d.active = Some(side);
            }
            name_resp.context_menu(|ui| {
                let (copy, label) = match side {
                    Side::Local => (Side::Local, "Upload"),
                    Side::Remote => (Side::Remote, "Download"),
                };
                if ui
                    .button(il(
                        if copy == Side::Local {
                            icons::ARROW_RIGHT
                        } else {
                            icons::ARROW_LEFT
                        },
                        label,
                    ))
                    .clicked()
                {
                    send(
                        out,
                        FmMsg::Copy {
                            id,
                            from: side,
                            names: d.selected(side).iter().cloned().collect(),
                        },
                    );
                    ui.close();
                }
                ui.separator();
                toolbar_menu(ui, s, d, side, out);
            });
            if name_resp.double_clicked() {
                if e.is_dir() {
                    send(
                        out,
                        FmMsg::Enter {
                            id,
                            side,
                            name: e.name.clone(),
                        },
                    );
                } else if e.kind != Kind::Other {
                    d.active = Some(side);
                    send(
                        out,
                        FmMsg::Editor {
                            id,
                            msg: EdMsg::Open {
                                side,
                                name: e.name.clone(),
                            },
                        },
                    );
                }
            } else if name_resp.clicked() {
                d.active = Some(side);
                let (ctrl, shift) = name_resp
                    .ctx
                    .input(|i| (i.modifiers.command || i.modifiers.ctrl, i.modifiers.shift));
                d.click_row(side, &e.name, &order, ctrl, shift);
            }
        });
    });
}

/// Text in a file-list cell: not selectable and not clickable itself, so the
/// row under it takes the click and the pointer stays an arrow (#130).
fn cell(ui: &mut egui::Ui, t: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(t).truncate().selectable(false));
}

fn queue(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, out: &mut Vec<Msg>, now: Instant) {
    let id = s.id;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let ready = s.status == Status::Ready;
        let up: Vec<String> = d.selected_local.iter().cloned().collect();
        let down: Vec<String> = d.selected_remote.iter().cloned().collect();
        if ui
            .add_enabled(
                ready && !up.is_empty(),
                egui::Button::new(il(icons::ARROW_RIGHT, "Upload")),
            )
            .on_hover_text("Copy the selected local files to the host folder (F5, or drag them)")
            .clicked()
        {
            send(
                out,
                FmMsg::Copy {
                    id,
                    from: Side::Local,
                    names: up,
                },
            );
        }
        if ui
            .add_enabled(
                ready && !down.is_empty(),
                egui::Button::new(il(icons::ARROW_LEFT, "Download")),
            )
            .on_hover_text("Copy the selected host files to the local folder (F5, or drag them)")
            .clicked()
        {
            send(
                out,
                FmMsg::Copy {
                    id,
                    from: Side::Remote,
                    names: down,
                },
            );
        }
        ui.separator();
        if ui
            .selectable_label(!d.show_history, RichText::new("Transfers").strong())
            .clicked()
        {
            d.show_history = false;
        }
        if ui
            .selectable_label(
                d.show_history,
                RichText::new(format!("History ({})", s.history.len())).strong(),
            )
            .on_hover_text(
                "Every transfer and file operation on this host, also from earlier sessions",
            )
            .clicked()
        {
            d.show_history = true;
        }
        if !d.show_history
            && s.transfers.iter().any(|t| t.state.finished())
            && ui.small_button("Clear finished").clicked()
        {
            send(out, FmMsg::ClearFinished(id));
        }
    });
    if d.show_history {
        history_view(ui, s, d, out);
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt(("files-queue-scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if s.transfers.is_empty() {
                ui.label(RichText::new("Select files and click Upload / Download, press F5, or drag them to the other side.").weak());
            }
            for t in s.transfers.iter().rev() {
                ui.horizontal_wrapped(|ui| {
                    let icon = match t.direction {
                        Direction::Upload => icons::ARROW_RIGHT,
                        Direction::Download => icons::ARROW_LEFT,
                    };
                    ui.label(RichText::new(format!("{icon} {}", t.title)).monospace())
                        .on_hover_text(&t.dest);
                    if t.rsync {
                        ui.label(RichText::new("rsync").weak().small());
                    }
                    let state = match &t.state {
                        TState::Planning => "listing…".to_string(),
                        TState::Asking => "waiting for your answer".to_string(),
                        TState::Previewed(_) => "dry run done".to_string(),
                        TState::Running => String::new(),
                        TState::Done { copied, skipped, failed } => {
                            format!("done: {copied} copied, {skipped} skipped, {failed} failed")
                        }
                        TState::Failed(e) => format!("failed: {e}"),
                        TState::Cancelled => "cancelled".to_string(),
                    };
                    let color = match &t.state {
                        TState::Failed(_) => red(),
                        TState::Done { failed, .. } if *failed > 0 => red(),
                        TState::Done { .. } => green(),
                        _ => ui.visuals().text_color(),
                    };
                    if !state.is_empty() {
                        ui.colored_label(color, state);
                    }
                    if matches!(t.state, TState::Planning | TState::Running | TState::Asking | TState::Previewed(_))
                        && ui.small_button(il(icons::STOP, "Cancel")).clicked()
                    {
                        send(out, FmMsg::Cancel { id, transfer: t.id });
                    }
                });
                if t.state == TState::Running {
                    let frac = if t.bytes_total > 0 {
                        (t.bytes_done as f32 / t.bytes_total as f32).min(1.0)
                    } else {
                        0.0
                    };
                    let speed = t
                        .speed(now)
                        .map(|b| format!(" · {}/s", human_size(b as u64)))
                        .unwrap_or_default();
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(ui.available_width().min(700.0))
                            .text(format!(
                                "{} of {}{speed} · {}",
                                human_size(t.bytes_done),
                                human_size(t.bytes_total),
                                t.current
                            )),
                    );
                }
                if let TState::Previewed(list) = &t.state {
                    ui.label(format!("rsync would change {} path(s):", list.len()));
                    for l in list.iter().take(12) {
                        ui.label(RichText::new(format!("  {l}")).monospace().small());
                    }
                    if list.len() > 12 {
                        ui.label(RichText::new(format!("  … and {} more", list.len() - 12)).weak().small());
                    }
                    if ui.button(il(icons::PLAY, "Run rsync")).clicked() {
                        send(out, FmMsg::RsyncRun { id, transfer: t.id });
                    }
                }
                if !t.errors.is_empty() {
                    egui::CollapsingHeader::new(RichText::new(format!("{} error(s)", t.errors.len())).color(red()))
                        .id_salt(("files-errors", id, t.id))
                        .show(ui, |ui| {
                            for e in &t.errors {
                                ui.label(RichText::new(e).monospace().small());
                            }
                        });
                }
                ui.separator();
            }
        });
}

/// "File exists" question, one file at a time (Apply to all for the rest).
fn ask(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some(a) = &s.ask else {
        return;
    };
    let Some(t) = s.transfer(a.transfer) else {
        return;
    };
    let Some(it) = a.conflicts.get(a.at).and_then(|&i| t.items.get(i)) else {
        return;
    };
    let modal = egui::Modal::new(egui::Id::unique(("files-ask", id))).show(ui.ctx(), |ui| {
        ui.set_width(520.0);
        ui.heading(il(icons::WARNING, "The file exists"));
        ui.label(RichText::new(&it.label).monospace().strong());
        let (ssize, smtime) = (it.size, it.mtime);
        let (dsize, dmtime) = it.exists.unwrap_or_default();
        let (from, to) = match t.direction {
            Direction::Upload => ("this computer", s.host.alias.as_str()),
            Direction::Download => (s.host.alias.as_str(), "this computer"),
        };
        egui::Grid::new(("files-ask-grid", id))
            .num_columns(3)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                ui.strong("Size");
                ui.strong("Modified (UTC)");
                ui.end_row();
                ui.label(format!("New ({from})"));
                ui.label(human_size(ssize));
                ui.label(time_text(smtime));
                ui.end_row();
                ui.label(format!("Existing ({to})"));
                ui.label(human_size(dsize));
                ui.label(time_text(dmtime));
                ui.end_row();
            });
        let left = a.conflicts.len() - a.at;
        ui.checkbox(
            &mut d.apply_all,
            format!("Apply to all {left} existing file(s)"),
        );
        ui.horizontal(|ui| {
            if act_button(ui, true, Act::Danger, "Overwrite").clicked() {
                send(
                    out,
                    FmMsg::Decide {
                        id,
                        decision: Decision::Overwrite,
                    },
                );
            }
            if ui.button("Skip").clicked() {
                send(
                    out,
                    FmMsg::Decide {
                        id,
                        decision: Decision::Skip,
                    },
                );
            }
            if ui
                .button("Newer only")
                .on_hover_text("Overwrite only when the new file is newer")
                .clicked()
            {
                send(
                    out,
                    FmMsg::Decide {
                        id,
                        decision: Decision::NewerOnly,
                    },
                );
            }
            if ui.button("Cancel transfer").clicked() {
                send(out, FmMsg::AskCancel(id));
            }
        });
    });
    if modal.should_close() {
        send(out, FmMsg::AskCancel(id));
    }
}

/// The operations of one pane: (op, icon, label, hint).
/// Colour of an operation's button (#76); `None` = neutral.
fn op_act(op: OpKind) -> Option<Act> {
    match op {
        OpKind::NewFolder | OpKind::NewFile => Some(Act::Add),
        OpKind::Edit | OpKind::Rename => Some(Act::Edit),
        OpKind::Delete => Some(Act::Danger),
        _ => None,
    }
}

fn ops_for(side: Side) -> Vec<(OpKind, &'static str, &'static str, &'static str)> {
    let mut v = vec![
        (
            OpKind::Edit,
            icons::CODE,
            "Edit",
            "Open in the editor (F4, or double-click)",
        ),
        (OpKind::NewFolder, icons::PLUS, "Folder", "New folder (F7)"),
        (OpKind::NewFile, icons::PLUS, "File", "New empty file"),
        (OpKind::Rename, icons::EDIT, "Rename", "Rename (F2)"),
        (
            OpKind::Delete,
            icons::TRASH,
            "Delete",
            "Delete the selected items (Del)",
        ),
        (
            OpKind::Properties,
            icons::INFO,
            "Properties",
            "Size, dates, permissions, owner",
        ),
    ];
    if side == Side::Remote {
        v.push((
            OpKind::Permissions,
            icons::LOCK,
            "Permissions",
            "Change permissions (chmod)",
        ));
    }
    v
}

fn toolbar(ui: &mut egui::Ui, s: &FileSession, d: &FileDraft, side: Side, out: &mut Vec<Msg>) {
    let id = s.id;
    let n = d.selected(side).len();
    for (op, icon, label, hint) in ops_for(side) {
        let why = op_blocked(s, n, side, op);
        let r = match op_act(op) {
            Some(k) => act_small(ui, why.is_none(), k, &il(icon, label)),
            None => ui.add_enabled(why.is_none(), egui::Button::new(il(icon, label)).small()),
        };
        let r = match why {
            Some(w) => r.on_disabled_hover_text(w),
            None => r.on_hover_text(hint),
        };
        if r.clicked() {
            send(out, FmMsg::Ask { id, side, op });
        }
    }
    if ui
        .small_button(il(icons::COPY, "Path"))
        .on_hover_text("Copy the full path of the selection (or of this folder)")
        .clicked()
    {
        send(out, FmMsg::CopyPath { id, side });
    }
}

fn toolbar_menu(ui: &mut egui::Ui, s: &FileSession, d: &FileDraft, side: Side, out: &mut Vec<Msg>) {
    let id = s.id;
    let n = d.selected(side).len();
    for (op, icon, _, hint) in ops_for(side) {
        let why = op_blocked(s, n, side, op);
        let text = match op_act(op) {
            Some(k) => act_text(k, &il(icon, hint)),
            None => RichText::new(il(icon, hint)),
        };
        let r = ui.add_enabled(why.is_none(), egui::Button::new(text));
        let r = match why {
            Some(w) => r.on_disabled_hover_text(w),
            None => r,
        };
        if r.clicked() {
            send(out, FmMsg::Ask { id, side, op });
            ui.close();
        }
    }
    if ui.button(il(icons::COPY, "Copy path")).clicked() {
        send(out, FmMsg::CopyPath { id, side });
        ui.close();
    }
}

fn where_text(s: &FileSession, side: Side) -> String {
    match side {
        Side::Local => "this computer".into(),
        Side::Remote => s.host.alias.clone(),
    }
}

fn measure_text(m: &Measure) -> String {
    let more = if m.partial { "at least " } else { "" };
    // `dirs` counts the folder itself.
    match m.dirs {
        0 => human_size(m.bytes),
        1 => format!("{more}{} in {} file(s)", human_size(m.bytes), m.files),
        n => format!(
            "{more}{} in {} file(s) and {} subfolder(s)",
            human_size(m.bytes),
            m.files,
            n - 1
        ),
    }
}

/// Mode checkboxes (owner / group / others × read / write / execute).
fn mode_grid(ui: &mut egui::Ui, id: u64, d: &mut FileDraft) {
    let mut mode = parse_mode(&d.mode_text).unwrap_or(d.mode);
    let before = mode;
    egui::Grid::new(("files-mode", id))
        .num_columns(4)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            ui.label("");
            ui.strong("Read");
            ui.strong("Write");
            ui.strong("Execute");
            ui.end_row();
            for (who, shift) in [("Owner", 6), ("Group", 3), ("Others", 0)] {
                ui.label(who);
                for bit in [4u32, 2, 1] {
                    let b = bit << shift;
                    let mut on = mode & b != 0;
                    if ui.checkbox(&mut on, "").changed() {
                        mode = if on { mode | b } else { mode & !b };
                    }
                }
                ui.end_row();
            }
        });
    if mode != before {
        d.mode = mode;
        d.mode_text = format!("{mode:o}");
    }
}

/// New / Rename / Delete / Properties / Permissions dialog.
fn dialog(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some(dl) = &s.dialog else {
        return;
    };
    let place = where_text(s, dl.side);
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let modal = egui::Modal::new(egui::Id::unique(("files-dialog", id))).show(ui.ctx(), |ui| {
        ui.set_width(480.0);
        let mut ok = None;
        match &dl.kind {
            DialogKind::NewFolder | DialogKind::NewFile | DialogKind::Rename { .. } => {
                let (title, button) = match &dl.kind {
                    DialogKind::NewFolder => ("New folder", "Create"),
                    DialogKind::NewFile => ("New file", "Create"),
                    _ => ("Rename", "Rename"),
                };
                ui.heading(title);
                ui.label(
                    RichText::new(format!("in {} on {place}", dl.dir))
                        .monospace()
                        .weak(),
                );
                if let DialogKind::Rename { from } = &dl.kind {
                    ui.label(format!("Old name: {from}"));
                }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut d.name)
                        .hint_text("name")
                        .desired_width(f32::INFINITY),
                );
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if r.lost_focus() && enter {
                    ok = Some(());
                }
                ok = ok.or(buttons(ui, dl.busy, button, false));
            }
            DialogKind::Delete { items, measure } => {
                ui.heading(il(icons::TRASH, &format!("Delete {} item(s)?", items.len())));
                ui.label(format!("in {} on {place}", dl.dir));
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    egui::Grid::new(("files-del", id)).num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                        for (i, e) in items.iter().enumerate() {
                            let icon = if e.kind == Kind::Dir { icons::FOLDER } else { icons::INFO };
                            ui.label(RichText::new(format!("{icon} {}", e.name)).monospace());
                            match measure {
                                Some(Ok(m)) => ui.label(m.get(i).map(measure_text).unwrap_or_default()),
                                Some(Err(e)) => ui.colored_label(red(), e),
                                None => ui.spinner(),
                            };
                            ui.end_row();
                        }
                    });
                });
                if let Some(Ok(m)) = measure {
                    let files: u64 = m.iter().map(|x| x.files).sum();
                    let bytes: u64 = m.iter().map(|x| x.bytes).sum();
                    let more = if m.iter().any(|x| x.partial) { "at least " } else { "" };
                    ui.strong(format!("Total: {more}{files} file(s), {}", human_size(bytes)));
                }
                ui.colored_label(
                    red(),
                    match dl.side {
                        Side::Remote => format!(
                            "{} There is no recycle bin on {place}: this cannot be undone.",
                            icons::WARNING
                        ),
                        Side::Local => format!(
                            "{} The files are deleted, not moved to the recycle bin: this cannot be undone.",
                            icons::WARNING
                        ),
                    },
                );
                ok = buttons(ui, dl.busy, "Delete", true);
            }
            DialogKind::Properties { item, path, measure } => {
                ui.heading(il(icons::INFO, &item.name));
                ui.add(egui::Label::new(RichText::new(path).monospace()).wrap());
                egui::Grid::new(("files-props", id)).num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                    ui.label("Where");
                    ui.label(&place);
                    ui.end_row();
                    ui.label("Type");
                    ui.label(match (item.kind, item.link_dir) {
                        (Kind::Dir, _) => "Folder",
                        (Kind::Link, true) => "Link to a folder",
                        (Kind::Link, false) => "Link",
                        (Kind::File, _) => "File",
                        (Kind::Other, _) => "Other (device, socket, pipe)",
                    });
                    ui.end_row();
                    ui.label("Size");
                    match measure {
                        Some(Ok(m)) => ui.label(measure_text(m)),
                        Some(Err(e)) => ui.colored_label(red(), e),
                        None => ui.spinner(),
                    };
                    ui.end_row();
                    ui.label("Modified (UTC)");
                    ui.label(time_text(item.mtime));
                    ui.end_row();
                    if let Some(p) = item.perm {
                        ui.label("Permissions");
                        ui.label(RichText::new(format!("{} ({:o})", perm_text(Some(p)), p & 0o7777)).monospace());
                        ui.end_row();
                    }
                    if !item.owner.is_empty() {
                        ui.label("Owner");
                        ui.label(&item.owner);
                        ui.end_row();
                    }
                });
                if ui.button("Close").clicked() {
                    send(out, FmMsg::CloseDialog(id));
                }
            }
            DialogKind::Permissions { items } => {
                ui.heading(il(icons::LOCK, "Permissions"));
                let names: Vec<&str> = items.iter().take(4).map(|e| e.name.as_str()).collect();
                let more = items.len().saturating_sub(4);
                ui.label(RichText::new(format!(
                    "{}{} on {place}",
                    names.join(", "),
                    if more > 0 { format!(" and {more} more") } else { String::new() }
                )).monospace());
                mode_grid(ui, id, d);
                ui.horizontal(|ui| {
                    ui.label("Octal");
                    ui.add(egui::TextEdit::singleline(&mut d.mode_text).desired_width(60.0).font(egui::TextStyle::Monospace));
                    if let Some(m) = parse_mode(&d.mode_text) {
                        ui.label(RichText::new(perm_text(Some(m))).monospace().weak());
                    }
                });
                if items.iter().any(|e| e.kind == Kind::Dir) {
                    ui.checkbox(&mut d.recursive, "Also everything inside the folders")
                        .on_hover_text("Folders inside get this mode plus x wherever r is set, so they stay openable. Files inside get its read / write bits; x only where they already had it; never setuid / setgid / sticky. Links are not changed.");
                }
                ui.label(RichText::new("Only what the login user owns can be changed (no sudo).").weak().small());
                ok = buttons(ui, dl.busy, "Apply", false);
            }
        }
        if let Some(e) = &dl.error {
            ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
        }
        if ok.is_some() {
            send(out, FmMsg::Confirm(id));
        }
    });
    if modal.should_close() && !dl.busy {
        send(out, FmMsg::CloseDialog(id));
    }
}

/// OK / Cancel; `danger` paints OK red. `Some` when OK was clicked.
fn buttons(ui: &mut egui::Ui, busy: bool, ok: &str, danger: bool) -> Option<()> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        let r = if danger {
            act_button(ui, !busy, Act::Danger, &il(icons::TRASH, ok))
        } else {
            ui.add_enabled(!busy, egui::Button::new(ok))
        };
        if r.clicked() {
            clicked = Some(());
        }
        if busy {
            ui.spinner();
        }
        if ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked() {
            ui.close();
        }
    });
    clicked
}

/// When a history line was written: UTC time, or "this session" for lines
/// not read back from history.db yet (they get their time when the window
/// opens again).
pub fn when(at: Option<i64>) -> String {
    at.map(|t| format!("{} UTC", telepad_core::history::format_utc(t)))
        .unwrap_or_else(|| "this session".into())
}

pub fn outcome_text(o: Outcome) -> (&'static str, Color32) {
    match o {
        Outcome::Done => ("done", green()),
        Outcome::Failed => ("failed", red()),
        Outcome::Cancelled => ("cancelled", crate::widgets::pal().amber),
        Outcome::RolledBack => ("rolled back", red()),
    }
}

/// The local folder of a transfer line (download target / upload source).
fn local_folder(e: &HistEntry) -> Option<String> {
    if e.kind != HistKind::Transfer {
        return None;
    }
    let lines: Vec<&str> = e.detail.lines().collect();
    if e.title.starts_with("Download") {
        lines
            .iter()
            .find_map(|l| l.strip_prefix("→ "))
            .map(str::to_string)
    } else {
        lines.first().map(|s| s.to_string())
    }
}

/// History tab of the transfer panel (#100).
fn history_view(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut d.history_filter)
                .hint_text(format!("{} Filter", icons::SEARCH))
                .desired_width(180.0),
        );
        egui::ComboBox::from_id_salt(("files-history-dir", id))
            .selected_text(match d.history_dir {
                None => "All",
                Some(Direction::Upload) => "Uploads",
                Some(Direction::Download) => "Downloads",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut d.history_dir, None, "All");
                ui.selectable_value(&mut d.history_dir, Some(Direction::Upload), "Uploads");
                ui.selectable_value(&mut d.history_dir, Some(Direction::Download), "Downloads");
            });
        ui.checkbox(&mut d.history_failed, "Not done only")
            .on_hover_text("Failed and cancelled lines only");
        ui.separator();
        if ui
            .button(il(icons::EXPORT, "Export…"))
            .on_hover_text("The lines shown, as CSV")
            .clicked()
        {
            send(out, FmMsg::HistoryExport(id));
        }
        if d.history_clear {
            ui.label("Clear this host's history?");
            if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Clear")).clicked() {
                send(out, FmMsg::HistoryClear(id));
            }
            if ui.button("Cancel").clicked() {
                d.history_clear = false;
            }
        } else if act_button(
            ui,
            !s.history.is_empty(),
            Act::Danger,
            &il(icons::TRASH, "Clear…"),
        )
        .clicked()
        {
            d.history_clear = true;
        }
    });
    let shown = history_shown(s, d);
    egui::ScrollArea::vertical()
        .id_salt(("files-history-scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if shown.is_empty() {
                ui.label(RichText::new("Nothing here yet: transfers, new / rename / delete / permissions and editor saves are listed as they happen.").weak());
            }
            for e in shown.iter().rev() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(when(e.at)).weak().small().monospace());
                    let icon = match e.kind {
                        HistKind::Transfer if e.title.starts_with("Upload") => icons::ARROW_RIGHT,
                        HistKind::Transfer => icons::ARROW_LEFT,
                        _ => icons::FOLDER,
                    };
                    ui.label(format!("{icon} {}", e.title)).on_hover_text(&e.detail);
                    let (text, color) = outcome_text(e.outcome);
                    ui.colored_label(color, text);
                    let mut info = Vec::new();
                    if let Some(b) = e.bytes.filter(|b| *b > 0) {
                        info.push(human_size(b));
                    }
                    if let Some(ms) = e.duration_ms {
                        info.push(format!("{:.1} s", ms as f64 / 1000.0));
                    }
                    if !e.method.is_empty() {
                        info.push(e.method.clone());
                    }
                    ui.label(RichText::new(info.join(" · ")).weak().small());
                    if !e.error.is_empty() {
                        ui.label(RichText::new(first_line(&e.error)).color(red()).small())
                            .on_hover_text(&e.error);
                    }
                    if let Some(dir) = local_folder(e)
                        && ui
                            .small_button(icons::FOLDER)
                            .on_hover_text(format!("Show {dir} in the file manager"))
                            .clicked()
                    {
                        send(out, FmMsg::HistoryOpenFolder { id, path: dir });
                    }
                });
            }
        });
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}
