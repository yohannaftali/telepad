//! Overview as cards: one card per host, wrapping to the window width. Same
//! rows and order as the table (search filter + sort).

use eframe::egui::{self, RichText};
use telepad_app::{Model, Msg, fullest_disk, host_flags, hottest};
use telepad_core::config::SetKind;

use super::overview::docker_text;
use crate::icons::{self, label as il};
use crate::widgets::*;

const CARD_WIDTH: f32 = 300.0;
const CARD_MIN_HEIGHT: f32 = 168.0;
const CARD_MARGIN: f32 = 10.0;

/// Cards of `rows` (see `overview::view_rows`).
pub fn cards(ui: &mut egui::Ui, m: &Model, rows: &[usize], out: &mut Vec<Msg>) {
    if rows.is_empty() {
        ui.label(RichText::new("No hosts match the search.").weak());
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // horizontal_wrapped cannot wrap a Frame (its size is only known after
            // drawing), so cards are laid out row by row for the available width.
            let gap = 10.0;
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            let outer = CARD_WIDTH + 2.0 * CARD_MARGIN + 2.0;
            let per_row = (((ui.available_width() + gap) / (outer + gap)).floor() as usize).max(1);
            // Every card as tall as the tallest one (#157), measured last frame.
            let height_id = ui.scope_id().with("card-height");
            let height: f32 = ui
                .data(|d| d.get_temp(height_id))
                .unwrap_or(CARD_MIN_HEIGHT)
                .max(CARD_MIN_HEIGHT);
            let mut tallest = CARD_MIN_HEIGHT;
            for chunk in rows.chunks(per_row) {
                ui.horizontal(|ui| {
                    for &i in chunk {
                        tallest = tallest.max(card(ui, m, i, height, out));
                    }
                });
            }
            if (tallest - height).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(height_id, tallest));
                ui.ctx().request_repaint();
            }
        });
}

/// One card, `height` tall at least; returns the height its content needs.
fn card(ui: &mut egui::Ui, m: &Model, i: usize, height: f32, out: &mut Vec<Msg>) -> f32 {
    let h = &m.hosts[i];
    // Text must not take the right-click meant for the card's menu (#83).
    ui.style_mut().interaction.selectable_labels = false;
    let card = egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(CARD_MARGIN as i8))
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            let top = ui.min_rect().top();
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    status_dot(ui, h);
                    icons::os_logo(ui, host_os(h), 20.0);
                    if ui
                        .add(egui::Link::new(
                            RichText::new(&h.entry.alias).strong().size(16.0),
                        ))
                        .on_hover_text("Open details")
                        .clicked()
                    {
                        out.push(Msg::Select(Some(i)));
                    }
                });
                let mut sub = h.entry.hostname.clone();
                if !h.entry.note.is_empty() {
                    if !sub.is_empty() {
                        sub.push_str(" · ");
                    }
                    sub.push_str(&h.entry.note);
                }
                if !sub.is_empty() {
                    ui.add(egui::Label::new(RichText::new(sub).weak().small()).truncate());
                }
                // Its group and cluster (#144, #145), when it has them.
                if !h.entry.group.is_empty() || !h.entry.cluster.is_empty() {
                    ui.horizontal(|ui| {
                        for (kind, icon) in [
                            (SetKind::Group, icons::FOLDERS),
                            (SetKind::Cluster, icons::CIRCLES_THREE),
                        ] {
                            let name = h.entry.set(kind);
                            if !name.is_empty() {
                                ui.add(
                                    egui::Label::new(RichText::new(il(icon, name)).small())
                                        .truncate(),
                                )
                                .on_hover_text(format!("{}: {name}", kind.title()));
                            }
                        }
                    });
                }
                ui.add_space(4.0);
                match &h.report {
                    Some(r) => {
                        ui.add(egui::Label::new(RichText::new(&r.os_pretty).small()).truncate());
                        let flags = host_flags(r);
                        if !flags.is_empty() {
                            ui.horizontal_wrapped(|ui| flag_chips(ui, &flags));
                        }
                        let w = ui.available_width();
                        if let Some(c) = r.cpu_usage_pct {
                            bar(ui, c, format!("CPU {c:.0}%"), w);
                        }
                        if let Some(mem) = &r.memory {
                            let p = mem.used_pct();
                            bar(ui, p, format!("RAM {p:.0}%"), w);
                        }
                        if let Some(d) = fullest_disk(r) {
                            let p = d.used_pct();
                            bar(ui, p, format!("Disk {p:.0}%  {}", d.mount), w);
                        }
                        ui.horizontal(|ui| {
                            if let Some(t) = hottest(r) {
                                ui.colored_label(temp_color(t), format!("{t:.0}°C"));
                                ui.label(RichText::new("·").weak());
                            }
                            match docker_text(r) {
                                Some(d) => {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(il(icons::CONTAINER, &d)).small(),
                                        )
                                        .truncate(),
                                    );
                                }
                                None => {
                                    ui.label(RichText::new("no Docker").weak().small());
                                }
                            }
                        });
                    }
                    None => {
                        let msg = if let Some(e) = &h.error {
                            RichText::new(e).color(usage_color(95.0))
                        } else if h.loading {
                            RichText::new("loading…").weak()
                        } else {
                            RichText::new("queued").weak()
                        };
                        ui.add(egui::Label::new(msg).wrap());
                    }
                }
                ui.label(
                    RichText::new(format!("updated {}", ago(h.updated)))
                        .weak()
                        .small(),
                );
            });
            let natural = ui.min_rect().bottom() - top;
            ui.set_min_height(height);
            natural
        });
    // A click anywhere opens the host (#157); SSH, Files, databases, edit… on
    // right-click (#83).
    let resp = card
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        out.push(Msg::Select(Some(i)));
    }
    resp.context_menu(|ui| host_menu(ui, m, i, out));
    card.inner
}
