//! Overview table: one row per host. Click a header to sort (again to reverse,
//! a third time for inventory order); drag a header border to resize. Cells
//! clip long text with "…" and show it in full on hover, so no value (an
//! error message, a long OS name) can widen its column.

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder, TableRow};
use telepad_app::{
    HostState, Model, Msg, Sort, SortKey, can_reorder, fullest_disk, host_flags, hottest,
    overview_rows,
};
use telepad_core::model::{HostReport, PortainerRole, human_kb};

use crate::icons;
use crate::widgets::*;

const ROW_HEIGHT: f32 = 24.0;

/// (title, sort key, initial width, minimum width)
const COLUMNS: [(&str, Option<SortKey>, f32, f32); 12] = [
    ("", Some(SortKey::Status), 36.0, 36.0),
    ("Host", Some(SortKey::Host), 130.0, 60.0),
    ("Description", Some(SortKey::Description), 170.0, 40.0),
    ("Address", Some(SortKey::Address), 115.0, 50.0),
    ("OS", Some(SortKey::Os), 190.0, 60.0),
    ("Flags", Some(SortKey::Flags), 150.0, 40.0),
    ("CPU", Some(SortKey::Cpu), 80.0, 50.0),
    ("Memory", Some(SortKey::Memory), 175.0, 60.0),
    ("Fullest disk", Some(SortKey::Disk), 150.0, 60.0),
    ("Temp", Some(SortKey::Temp), 60.0, 40.0),
    ("Docker", Some(SortKey::Docker), 160.0, 50.0),
    ("Updated", Some(SortKey::Updated), 75.0, 50.0),
];

/// Column widths after a double-click fit (#83): `round` renames the table so
/// egui_extras starts from these widths.
#[derive(Clone, Default)]
struct Fit {
    round: u32,
    widths: Option<Vec<f32>>,
}

pub fn overview(ui: &mut egui::Ui, m: &Model, filter: &str, sort: Sort, out: &mut Vec<Msg>) {
    let rows = overview_rows(m, filter, sort);
    let reorder = can_reorder(filter, false, sort);
    let accent = ui.visuals().selection.bg_fill;
    let fit_id = egui::Id::unique("overview-fit");
    let fit: Fit = ui.data(|d| d.get_temp(fit_id)).unwrap_or_default();
    let mut header_rects = vec![egui::Rect::NOTHING; COLUMNS.len()];
    let mut used = vec![0.0f32; COLUMNS.len()];
    // The table scrolls vertically itself; this adds horizontal scrolling
    // when the columns are wider than the window.
    // Labels in the table must not take clicks: the whole row opens the host
    // (left) and its menu (right), and highlights on hover (#83).
    ui.style_mut().interaction.selectable_labels = false;
    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut table = TableBuilder::new(ui)
                .id_salt(("overview", fit.round))
                .striped(true)
                .resizable(true)
                // Rows highlight on hover and open the host on click (#83).
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .auto_shrink([false, false]);
            let last = COLUMNS.len() - 1;
            for (i, (_, _, width, min)) in COLUMNS.iter().enumerate() {
                let width = fit
                    .widths
                    .as_ref()
                    .and_then(|w| w.get(i).copied())
                    .unwrap_or(*width);
                let col = if i == 0 {
                    Column::exact(width.max(*min))
                } else if i == last {
                    // The last column takes the width the others leave, so
                    // the table always fills the window (#83).
                    Column::remainder().at_least(*min).clip(true)
                } else {
                    Column::initial(width).at_least(*min).clip(true)
                };
                table = table.column(col);
            }
            table
                .header(ROW_HEIGHT, |mut header| {
                    for (c, (title, key, _, _)) in COLUMNS.into_iter().enumerate() {
                        let (u, r) = header.col(|ui| {
                            let Some(key) = key else { return };
                            let arrow = match sort.direction(key) {
                                Some(true) => format!(" {}", icons::CARET_DOWN),
                                Some(false) => format!(" {}", icons::CARET_UP),
                                None => String::new(),
                            };
                            let text = if title.is_empty() {
                                format!("{}{arrow}", icons::DOT)
                            } else {
                                format!("{title}{arrow}")
                            };
                            let name = if title.is_empty() { "status" } else { title };
                            if ui
                                .add(egui::Button::new(RichText::new(text).strong()).frame(false))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(format!(
                                    "Click to sort by {name} (again to reverse, a third time for inventory order). Double-click a column border to fit it."
                                ))
                                .clicked()
                            {
                                out.push(Msg::SortBy(key));
                            }
                        });
                        header_rects[c] = r.rect;
                        used[c] = used[c].max(u.width());
                    }
                })
                .body(|body| {
                    body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                        let i = rows[row.index()];
                        let mut cells = Cells {
                            row: &mut row,
                            n: 0,
                            used: &mut used,
                        };
                        host_row(&mut cells, i, &m.hosts[i], m.host_paused(&m.hosts[i]), reorder);
                        let resp = row.response();
                        if resp.clicked() {
                            out.push(Msg::Select(Some(i)));
                        }
                        // SSH, Files, databases, edit… on right-click (#83).
                        resp.context_menu(|ui| {
                            host_menu(
                                ui,
                                &m.hosts[i],
                                i,
                                m.rdp.contains_key(&m.hosts[i].entry.alias),
                                m.host_paused(&m.hosts[i]),
                                out,
                            )
                        });
                        if reorder {
                            drop_target(&row.response(), i, accent, out);
                        }
                    });
                });
        });
    // Double-click on a header border: fit that column to its longest value.
    let dbl = ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
            .then(|| i.pointer.latest_pos())
            .flatten()
    });
    if let Some(p) = dbl {
        // A column's border sits half a gap before the next column's left edge
        // (the header responses are wider than their columns).
        let gap = ui.spacing().item_spacing.x;
        let left: Vec<f32> = header_rects.iter().map(|r| r.left()).collect();
        let (top, bottom) = (header_rects[0].top(), header_rects[0].bottom());
        let n = COLUMNS.len();
        let hit = (1..n - 1).find(|&c| {
            (top..=bottom).contains(&p.y) && (p.x - (left[c + 1] - gap / 2.0)).abs() <= 6.0
        });
        if let Some(c) = hit {
            let mut widths: Vec<f32> = (0..n)
                .map(|k| match left.get(k + 1) {
                    Some(next) => next - left[k] - gap,
                    None => COLUMNS[k].2,
                })
                .collect();
            widths[c] = (used[c] + 8.0).max(COLUMNS[c].3);
            let next = Fit {
                round: fit.round.wrapping_add(1),
                widths: Some(widths),
            };
            ui.data_mut(|d| d.insert_temp(fit_id, next));
        }
    }
}

/// One row's cells, recording how wide each one's content really is.
struct Cells<'r, 'a, 'b> {
    row: &'r mut TableRow<'a, 'b>,
    n: usize,
    used: &'r mut [f32],
}

impl Cells<'_, '_, '_> {
    fn col(&mut self, add: impl FnOnce(&mut egui::Ui)) {
        let (r, _) = self.row.col(add);
        if let Some(u) = self.used.get_mut(self.n) {
            *u = u.max(r.width());
        }
        self.n += 1;
    }
}

fn host_row(row: &mut Cells<'_, '_, '_>, i: usize, h: &HostState, paused: bool, grip: bool) {
    row.col(|ui| {
        if grip {
            grip_handle(ui, i);
        }
        status_dot(ui, h);
    });
    // Plain text: the whole row opens the host on click and its menu on
    // right-click, also over the name.
    row.col(|ui| {
        ui.label(RichText::new(&h.entry.alias).strong());
    });
    row.col(|ui| clipped(ui, RichText::new(&h.entry.note).italics()));
    row.col(|ui| clipped(ui, RichText::new(&h.entry.hostname)));
    match &h.report {
        Some(r) => report_cells(row, h, r),
        None => {
            row.col(|ui| {
                let known = h
                    .known
                    .as_ref()
                    .map(|k| k.os_pretty.as_str())
                    .filter(|s| !s.is_empty())
                    .map_or(String::new(), |os| format!(" · last known: {os}"));
                let msg = if let Some(e) = &h.error {
                    RichText::new(format!("{e}{known}")).color(usage_color(95.0))
                } else if h.loading {
                    RichText::new(format!("loading…{known}")).weak()
                } else if paused {
                    RichText::new(format!("paused{known}")).color(usage_color(75.0))
                } else {
                    RichText::new(format!("queued{known}")).weak()
                };
                clipped(ui, msg);
            });
            for _ in 0..6 {
                row.col(|_| {});
            }
        }
    }
    row.col(|ui| {
        if paused {
            ui.label(
                RichText::new(icons::label(icons::PAUSE, &ago(h.updated))).color(usage_color(75.0)),
            )
            .on_hover_text("Updates paused for this host (right-click → Resume updates)");
        } else {
            ui.label(RichText::new(ago(h.updated)).weak());
        }
    });
}

fn report_cells(row: &mut Cells<'_, '_, '_>, h: &HostState, r: &HostReport) {
    row.col(|ui| {
        icons::os_logo(ui, host_os(h), 16.0);
        clipped(ui, RichText::new(&r.os_pretty));
    });
    row.col(|ui| flag_chips(ui, &host_flags(r)));
    row.col(|ui| match r.cpu_usage_pct {
        Some(c) => bar(ui, c, format!("{c:.0}%"), ui.available_width()),
        None => dash(ui),
    });
    row.col(|ui| match &r.memory {
        Some(mem) => {
            let text = format!("{} / {}", human_kb(mem.used_kb()), human_kb(mem.total_kb));
            bar(ui, mem.used_pct(), text, ui.available_width());
        }
        None => dash(ui),
    });
    row.col(|ui| match fullest_disk(r) {
        Some(d) => {
            let text = format!("{} {:.0}%", d.mount, d.used_pct());
            bar(ui, d.used_pct(), text, ui.available_width());
        }
        None => dash(ui),
    });
    row.col(|ui| {
        match hottest(r) {
            Some(t) => ui.colored_label(temp_color(t), format!("{t:.0}°C")),
            None => ui.label(RichText::new("n/a").weak()),
        };
    });
    row.col(|ui| match docker_text(r) {
        Some(t) => clipped(ui, RichText::new(t)),
        None => dash(ui),
    });
}

/// "12/14 running · agent" (with "(podman)"), or None when Docker is not available.
pub fn docker_text(r: &HostReport) -> Option<String> {
    if r.docker.unavailable.is_some() {
        return None;
    }
    let run = r.docker.containers.iter().filter(|c| c.running()).count();
    let role = match r.docker.portainer().first() {
        Some((PortainerRole::Server, _)) => " · Portainer",
        Some((PortainerRole::Agent, _)) => " · agent",
        None => "",
    };
    let rt = if r.docker.is_podman() {
        " (podman)"
    } else {
        ""
    };
    let badge = r
        .cluster
        .badge()
        .map(|b| format!(" · {b}"))
        .unwrap_or_default();
    Some(format!(
        "{run}/{} running{rt}{role}{badge}",
        r.docker.containers.len()
    ))
}

/// One line, cut with "…" at the cell edge; the full text shows on hover.
/// The cell still reports the text's natural width, so a double-click on the
/// column border fits the column to its longest value (#83).
fn clipped(ui: &mut egui::Ui, text: RichText) {
    let natural = egui::WidgetText::from(text.clone())
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::FontSelection::Default,
        )
        .size()
        .x;
    let start = ui.cursor().min.x;
    ui.add(egui::Label::new(text).truncate());
    let used = ui.min_rect().max.x - start;
    if natural > used {
        let r = egui::Rect::from_min_size(
            ui.min_rect().min,
            egui::vec2(natural + (start - ui.min_rect().min.x), 1.0),
        );
        ui.expand_to_include_rect(r);
    }
}

fn dash(ui: &mut egui::Ui) {
    ui.label(RichText::new("—").weak());
}

#[cfg(test)]
mod tests {
    use super::docker_text;
    use telepad_core::cluster::{Cluster, Swarm};
    use telepad_core::model::HostReport;

    #[test]
    fn docker_cell_shows_the_cluster_badge() {
        let mut r = HostReport::default();
        assert_eq!(docker_text(&r).as_deref(), Some("0/0 running"));
        r.cluster = Cluster {
            swarm: Some(Swarm {
                manager: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            docker_text(&r).as_deref(),
            Some("0/0 running · Swarm manager")
        );
    }
}
