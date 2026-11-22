//! Overview as cards: one card per host, wrapping to the window width. Same
//! rows and order as the table (search filter + sort).

use eframe::egui::{self, RichText};
use telepad_app::{HostState, Model, Msg, Sort, fullest_disk, host_flags, hottest, overview_rows};

use super::overview::docker_text;
use crate::icons::{self, label as il};
use crate::widgets::*;

const CARD_WIDTH: f32 = 300.0;
const CARD_MIN_HEIGHT: f32 = 168.0;
const CARD_MARGIN: f32 = 10.0;

pub fn cards(ui: &mut egui::Ui, m: &Model, filter: &str, sort: Sort, out: &mut Vec<Msg>) {
    let rows = overview_rows(m, filter, sort);
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
            for chunk in rows.chunks(per_row) {
                ui.horizontal(|ui| {
                    for &i in chunk {
                        card(
                            ui,
                            i,
                            &m.hosts[i],
                            m.rdp.contains_key(&m.hosts[i].entry.alias),
                            out,
                        );
                    }
                });
            }
        });
}

fn card(ui: &mut egui::Ui, i: usize, h: &HostState, has_rdp: bool, out: &mut Vec<Msg>) {
    // Text must not take the right-click meant for the card's menu (#83).
    ui.style_mut().interaction.selectable_labels = false;
    let card = egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(CARD_MARGIN as i8))
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            ui.set_min_height(CARD_MIN_HEIGHT);
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
        });
    // SSH, Files, databases, edit… on right-click (#83).
    card.response
        .interact(egui::Sense::click())
        .context_menu(|ui| host_menu(ui, h, i, has_rdp, out));
}
