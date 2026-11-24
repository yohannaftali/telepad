//! Log info page (#126): a host's SSH logins from its systemd journal in one
//! flat table — header click sorts (asc → desc → newest first), one filter
//! box per column, pages — plus Register / Unregister of journal access.

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use telepad_app::logins::{Col, Dir, LgMsg, LogId, LoginDraft, Who, page_range, rows};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::login_log::{DAYS, JournalAccess, LIMIT, LoginLog};

use crate::icons::{self, label as il};
use crate::widgets::*;

const ROW: f32 = 22.0;

pub fn logins_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, id: LogId, out: &mut Vec<Msg>) {
    let Some(s) = m.logins.iter().find(|s| s.id == id) else {
        return;
    };
    let draft = d.logins.entry(id).or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&s.alias).strong().size(16.0));
        if ui
            .add_enabled(!s.loading, egui::Button::new(il(icons::REFRESH, "Refresh")))
            .on_hover_text(format!(
                "Read the logins of the last {DAYS} days again (journalctl, no sudo)"
            ))
            .clicked()
        {
            out.push(Msg::Logins(LgMsg::Refresh(id)));
        }
        if s.loading {
            ui.spinner();
            ui.label(RichText::new("reading the journal…").weak());
        } else if s.read_at.is_some() {
            ui.label(RichText::new(format!("read {}", ago(s.read_at))).weak());
        }
        if let Some(Ok(log)) = &s.log {
            access_buttons(ui, id, log, out);
        }
    });
    let log = match &s.log {
        None => return,
        Some(Err(e)) => {
            ui.colored_label(usage_color(95.0), il(icons::WARNING, e));
            return;
        }
        Some(Ok(log)) => log,
    };
    summary(ui, log);
    match log.access {
        JournalAccess::NoJournal => {
            ui.add_space(8.0);
            ui.label("Journal not available on this host (no systemd): Log info needs journalctl.");
            return;
        }
        JournalAccess::Denied => {
            ui.add_space(8.0);
            ui.colored_label(
                usage_color(80.0),
                il(
                    icons::LOCK,
                    &format!(
                        "{} cannot read the system journal, so SSH logins are not visible. \
                         Register Systemd Journal opens a terminal on the host that runs \
                         sudo usermod -aG systemd-journal {} after you press Enter (sudo asks \
                         for your password there). Then click Refresh.",
                        log.user, log.user
                    ),
                ),
            );
            ui.label(
                RichText::new(
                    "The group can only read logs (no root, logs cannot be changed). Logs may \
                     hold sensitive text, e.g. sudo command lines of other users.",
                )
                .weak(),
            );
            return;
        }
        JournalAccess::Readable => {}
    }
    ui.add_space(4.0);
    let mine: Vec<(String, String)> = m
        .my_keys
        .keys
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .map(|keys| {
            keys.iter()
                .filter_map(|k| {
                    let fp = k.fingerprint.clone()?;
                    let file = k.path.file_name()?.to_string_lossy().to_string();
                    Some((fp, file))
                })
                .collect()
        })
        .unwrap_or_default();
    let all = rows(log, &mine, draft);
    controls(ui, draft, all.len(), log.logins.len());
    let (start, end, _) = page_range(all.len(), draft);
    table(ui, draft, &all[start..end]);
}

fn access_buttons(ui: &mut egui::Ui, id: LogId, log: &LoginLog, out: &mut Vec<Msg>) {
    if log.access == JournalAccess::NoJournal {
        return;
    }
    ui.separator();
    if log.access == JournalAccess::Denied
        && act_button(
            ui,
            true,
            Act::Caution,
            &il(icons::TERMINAL, "Register Systemd Journal"),
        )
        .on_hover_text(format!(
            "Opens an SSH terminal that shows and runs\n  sudo usermod -aG systemd-journal {}\n\
                 after Enter; sudo asks for the password there. Read access to logs only.",
            log.user
        ))
        .clicked()
    {
        out.push(Msg::Logins(LgMsg::Access { id, register: true }));
    }
    if log.in_journal_group()
        && act_button(ui, true, Act::Caution, &il(icons::TERMINAL, "Unregister"))
            .on_hover_text(format!(
                "Opens an SSH terminal that shows and runs\n  sudo gpasswd -d {} systemd-journal\n\
                 after Enter; sudo asks for the password there.",
                log.user
            ))
            .clicked()
    {
        out.push(Msg::Logins(LgMsg::Access {
            id,
            register: false,
        }));
    }
}

fn summary(ui: &mut egui::Ui, log: &LoginLog) {
    let mut parts = vec![format!("login user {}", log.user)];
    let readers: Vec<&str> = ["systemd-journal", "adm", "wheel"]
        .into_iter()
        .filter(|g| log.groups.iter().any(|x| x == g))
        .collect();
    if log.user == "root" {
        parts.push("root".into());
    } else if !readers.is_empty() {
        parts.push(format!("reads the journal as {}", readers.join(", ")));
    }
    if log.access == JournalAccess::Readable {
        if let Some(f) = &log.first {
            parts.push(format!("journal holds entries since {f}"));
        }
        parts.push(format!(
            "{} logins in the last {DAYS} days{}",
            log.logins.len(),
            if log.truncated {
                format!(" (newest {LIMIT} only)")
            } else {
                String::new()
            }
        ));
    }
    ui.label(RichText::new(parts.join(" · ")).weak());
    if let Some(l) = log.this_session() {
        let name = log
            .own
            .iter()
            .find(|(fp, _)| *fp == l.fingerprint)
            .map(|(_, n)| format!(" ({n})"))
            .unwrap_or_default();
        ui.label(
            RichText::new(format!(
                "Telepad logged in as {} from {} with {} {}{name}",
                l.user, l.from, l.key_type, l.fingerprint
            ))
            .small(),
        )
        .on_hover_text(
            "The key sshd accepted for this read (its journal line has this session's process id)",
        );
    }
}

fn controls(ui: &mut egui::Ui, d: &mut LoginDraft, shown: usize, total: usize) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .checkbox(&mut d.show_own, "Show this PC's logins")
            .on_hover_text(
                "Logins from this PC's address with a key Telepad logs in with: Telepad's \
                 refreshes (one per round) and your own ssh with that key. Hidden by default.",
            )
            .changed()
        {
            d.page = 0;
        }
        if d.filters.iter().any(|f| !f.is_empty()) && ui.button("Clear filters").clicked() {
            d.filters = Default::default();
            d.page = 0;
        }
        ui.separator();
        let (start, end, pages) = page_range(shown, d);
        let page = d.page.min(pages - 1);
        if ui
            .add_enabled(page > 0, egui::Button::new(icons::CARET_LEFT))
            .on_hover_text("Previous page")
            .clicked()
        {
            d.page = page - 1;
        }
        ui.label(format!("Page {} of {pages}", page + 1));
        if ui
            .add_enabled(page + 1 < pages, egui::Button::new(icons::CARET_RIGHT))
            .on_hover_text("Next page")
            .clicked()
        {
            d.page = page + 1;
        }
        ui.label("Rows per page");
        let r = ui.add(
            egui::TextEdit::singleline(&mut d.per_page_text)
                .desired_width(48.0)
                .hint_text("20"),
        );
        r.clone()
            .on_hover_text("Type a number (20, 50, 100 …) and press Enter");
        if r.lost_focus() {
            d.apply_per_page();
        }
        let info = if shown == 0 {
            "no rows".to_string()
        } else {
            format!("Rows {}–{} of {shown}", start + 1, end)
        };
        let hidden = total - shown;
        ui.label(
            RichText::new(if hidden > 0 {
                format!("{info} ({hidden} hidden by filters / this PC)")
            } else {
                info
            })
            .weak(),
        );
    });
}

fn width(c: Col) -> Column {
    match c {
        Col::Time => Column::initial(150.0).at_least(120.0),
        Col::User => Column::initial(100.0).at_least(60.0).clip(true),
        Col::From => Column::initial(130.0).at_least(80.0).clip(true),
        Col::Method => Column::initial(95.0).at_least(60.0).clip(true),
        Col::KeyType => Column::initial(80.0).at_least(50.0).clip(true),
        Col::Fingerprint => Column::initial(330.0).at_least(120.0).clip(true),
        Col::KeyName => Column::initial(200.0).at_least(80.0).clip(true),
        Col::Who => Column::remainder().at_least(70.0).clip(true),
    }
}

fn table(ui: &mut egui::Ui, d: &mut LoginDraft, page: &[telepad_app::logins::Row]) {
    let mut b = TableBuilder::new(ui)
        .id_salt("login-log-table")
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for c in Col::ALL {
        b = b.column(width(c));
    }
    let mut changed = false;
    b.header(48.0, |mut h| {
        for c in Col::ALL {
            h.col(|ui| {
                ui.vertical(|ui| {
                    let arrow = match d.sort {
                        Some((s, Dir::Asc)) if s == c => format!(" {}", icons::CARET_UP),
                        Some((s, Dir::Desc)) if s == c => format!(" {}", icons::CARET_DOWN),
                        None if c == Col::Time => format!(" {}", icons::CARET_DOWN),
                        _ => String::new(),
                    };
                    if ui
                        .add(
                            egui::Label::new(RichText::new(format!("{}{arrow}", c.title())).strong())
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Click: ascending → descending → newest first")
                        .clicked()
                    {
                        d.click(c);
                    }
                    let f = ui.add(
                        egui::TextEdit::singleline(d.filter_mut(c))
                            .hint_text(icons::FUNNEL)
                            .desired_width(f32::INFINITY),
                    );
                    if f.changed() {
                        changed = true;
                    }
                    f.on_hover_text(
                        "Filter: text = contains · =x equal · !x not equal · >x / <x \
                         (numbers by value, times as text, e.g. >2026-11-20) · NULL empty · !NULL not empty",
                    );
                });
            });
        }
    })
    .body(|mut body| {
        for r in page {
            body.row(ROW, |mut row| {
                for c in Col::ALL {
                    row.col(|ui| {
                        let text = r.cell(c);
                        let rt = match (c, r.who) {
                            (Col::Who, Who::ThisRead) => RichText::new(text).strong(),
                            (Col::Fingerprint, _) => RichText::new(text).monospace().small(),
                            (Col::KeyName, _) if text.is_empty() && !r.login.fingerprint.is_empty() => {
                                RichText::new("unknown key").weak().italics()
                            }
                            _ => RichText::new(text),
                        };
                        let resp = ui.add(egui::Label::new(rt).truncate());
                        if c == Col::From && r.login.port != 0 {
                            resp.on_hover_text(format!("port {}", r.login.port));
                        }
                    });
                }
            });
        }
    });
    if changed {
        d.page = 0;
    }
}
