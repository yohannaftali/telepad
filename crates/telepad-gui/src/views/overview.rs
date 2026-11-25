//! Overview table: one row per host. Click a header to sort (again to reverse,
//! a third time for inventory order); drag a header border to resize. Cells
//! clip long text with "…" and show it in full on hover, so no value (an
//! error message, a long OS name) can widen its column.

use eframe::egui::{self, RichText};
use egui_extras::Column;
use telepad_app::{
    HostState, Model, Msg, Sort, SortKey, can_reorder, fullest_disk, host_flags, hottest,
    overview_rows,
};
use telepad_core::config::SetKind;
use telepad_core::model::{HostReport, PortainerRole, human_kb};

use crate::icons;
use crate::widgets::*;

const ROW_HEIGHT: f32 = 24.0;

/// (title, sort key, initial width, minimum width)
const COLUMNS: [(&str, Option<SortKey>, f32, f32); 12] = [
    ("", Some(SortKey::Status), 36.0, 36.0),
    ("Host", Some(SortKey::Host), 130.0, 60.0),
    ("Description", Some(SortKey::Description), 170.0, 100.0),
    ("Address", Some(SortKey::Address), 115.0, 90.0),
    ("OS", Some(SortKey::Os), 190.0, 110.0),
    ("Flags", Some(SortKey::Flags), 150.0, 40.0),
    ("CPU", Some(SortKey::Cpu), 80.0, 50.0),
    ("Memory", Some(SortKey::Memory), 175.0, 60.0),
    ("Fullest disk", Some(SortKey::Disk), 150.0, 60.0),
    ("Temp", Some(SortKey::Temp), 60.0, 40.0),
    ("Docker", Some(SortKey::Docker), 160.0, 50.0),
    ("Updated", Some(SortKey::Updated), 75.0, 50.0),
];

/// Columns that share the spare width (#143): Description, OS, Flags, Docker.
const STRETCH: [usize; 4] = [2, 4, 5, 10];
/// Columns that give way when the page is too narrow (#143), in this order:
/// long text that cuts well, then the bars; Host, Flags and Updated last.
const SHRINK_ORDER: [&[usize]; 2] = [&[2, 10, 4, 3], &[6, 7, 8, 9]];
/// Room around a cell's content.
const CELL_PAD: f32 = 12.0;

type Col = (&'static str, Option<SortKey>, f32, f32);

/// What a hosts table shows (#144, #145): every host (the Hosts page) or
/// the hosts of one group / cluster, extra Group / Cluster columns, and on
/// the Hosts page optionally grouped by group.
#[derive(Clone, Copy, Default)]
pub struct TableView<'a> {
    /// Only the hosts of this group / cluster ("" = the fixed entry).
    pub only: Option<(SetKind, &'a str)>,
    /// Group / Cluster columns, before Updated.
    pub extra: &'a [SetKind],
    /// Hosts in group order (ungrouped last), the current sort inside.
    pub by_group: bool,
    /// Keeps each page's column layout apart.
    pub salt: u64,
}

/// The columns of a view: the fixed ones with the extra Group / Cluster
/// columns before Updated.
fn columns(extra: &[SetKind]) -> Vec<Col> {
    let mut c: Vec<Col> = COLUMNS[..COLUMNS.len() - 1].to_vec();
    for k in extra {
        c.push((k.title(), None, 120.0, 60.0));
    }
    c.push(COLUMNS[COLUMNS.len() - 1]);
    c
}

/// Hosts of a view in display order: search and sort as on the Hosts page,
/// then only the hosts of the group / cluster; with `by_group` the groups
/// in their order, ungrouped hosts last.
pub fn view_rows(m: &Model, filter: &str, sort: Sort, view: &TableView) -> Vec<usize> {
    let mut rows = overview_rows(m, filter, sort);
    if let Some((kind, name)) = view.only {
        rows.retain(|&i| m.hosts[i].entry.set(kind).eq_ignore_ascii_case(name));
    }
    if view.by_group {
        let order = telepad_app::sets::list(m, SetKind::Group);
        let rank = |i: usize| {
            let g = &m.hosts[i].entry.group;
            if g.is_empty() {
                return usize::MAX;
            }
            order
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(g))
                .unwrap_or(usize::MAX - 1)
        };
        rows.sort_by_key(|&i| rank(i));
    }
    rows
}

/// Column layout of the Hosts table (#143), kept between frames.
#[derive(Clone, Default)]
struct Layout {
    /// Part of the table's id: a new value makes egui_extras start from `widths`.
    key: u64,
    widths: Vec<f32>,
    /// Widths the user dragged a column to; `None` = automatic.
    manual: Vec<Option<f32>>,
    /// The header row: a press there may be a column resize.
    header: Option<egui::Rect>,
    /// Left edge and width of each column as last drawn.
    left: Vec<f32>,
    seen: Vec<f32>,
    /// The primary button went down in the header and is still down.
    pressed: bool,
    /// Bumped to make egui_extras forget widths it changed itself (its own
    /// double-click auto-size) or a drag that has been taken over.
    round: u32,
}

/// Column widths for `avail` points (#143). Each column gets what its content
/// wants (at least its minimum); a column the user sized keeps that width.
/// Spare width goes to the `stretch` columns in proportion to what they want;
/// when there is too little, the automatic columns shrink toward their
/// minimum in proportion to the room they have above it (then the table
/// scrolls).
pub fn fit_widths(
    want: &[f32],
    min: &[f32],
    stretch: &[usize],
    manual: &[Option<f32>],
    avail: f32,
) -> Vec<f32> {
    let n = want.len();
    let fixed = |i: usize| manual.get(i).copied().flatten();
    let mut w: Vec<f32> = (0..n)
        .map(|i| fixed(i).unwrap_or(want[i]).max(min[i]))
        .collect();
    let total: f32 = w.iter().sum();
    if total < avail {
        let grow: Vec<usize> = stretch
            .iter()
            .copied()
            .filter(|&i| i < n && fixed(i).is_none())
            .collect();
        let weight: f32 = grow.iter().map(|&i| w[i]).sum();
        if weight > 0.0 {
            let spare = avail - total;
            for &i in &grow {
                w[i] += spare * w[i] / weight;
            }
        }
    } else if total > avail {
        // Each tier gives way only when the ones before it are at their minimum.
        let mut over = total - avail;
        let mut tiers: Vec<Vec<usize>> = SHRINK_ORDER
            .iter()
            .map(|t| t.iter().copied().filter(|&i| i < n).collect())
            .collect();
        // Group / Cluster columns (between Docker and Updated) are long
        // text too: they give way before Host, Flags and Updated.
        if n > COLUMNS.len() {
            tiers[0].extend(COLUMNS.len() - 1..n - 1);
        }
        let listed: Vec<usize> = tiers.concat();
        tiers.push((0..n).filter(|i| !listed.contains(i)).collect());
        for tier in tiers {
            let auto: Vec<usize> = tier.into_iter().filter(|&i| fixed(i).is_none()).collect();
            let room: f32 = auto.iter().map(|&i| w[i] - min[i]).sum();
            if over <= 0.0 || room <= 0.0 {
                continue;
            }
            let k = (over / room).min(1.0);
            for &i in &auto {
                w[i] -= (w[i] - min[i]) * k;
            }
            over -= room * k;
        }
    }
    w
}

/// Width of one line of `text`.
fn text_w(ui: &egui::Ui, text: impl Into<egui::WidgetText>) -> f32 {
    text.into()
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::FontSelection::Default,
        )
        .size()
        .x
}

fn header_text(title: &str, key: Option<SortKey>, sort: Sort) -> String {
    let arrow = match key.and_then(|k| sort.direction(k)) {
        Some(true) => format!(" {}", icons::CARET_DOWN),
        Some(false) => format!(" {}", icons::CARET_UP),
        None => String::new(),
    };
    if title.is_empty() {
        format!("{}{arrow}", icons::DOT)
    } else {
        format!("{title}{arrow}")
    }
}

/// What each column's content needs, over every listed host (not only the
/// rows on screen, so scrolling does not move the columns).
fn wanted(
    ui: &egui::Ui,
    m: &Model,
    rows: &[usize],
    sort: Sort,
    cols: &[Col],
    extra: &[SetKind],
) -> Vec<f32> {
    let mut want: Vec<f32> = cols
        .iter()
        .map(|(t, k, _, _)| text_w(ui, RichText::new(header_text(t, *k, sort)).strong()))
        .collect();
    let gap = ui.spacing().item_spacing.x;
    let last = cols.len() - 1;
    let mut take = |c: usize, w: f32| want[c] = want[c].max(w);
    take(0, COLUMNS[0].2);
    for &i in rows {
        let h = &m.hosts[i];
        take(1, text_w(ui, RichText::new(&h.entry.alias).strong()));
        take(
            2,
            text_w(ui, RichText::new(&h.entry.note).italics()).min(260.0),
        );
        take(3, text_w(ui, RichText::new(&h.entry.hostname)));
        for (k, kind) in extra.iter().enumerate() {
            take(11 + k, text_w(ui, h.entry.set(*kind)).min(180.0));
        }
        take(
            last,
            text_w(ui, ago(h.updated)) + if m.host_paused(h) { 20.0 } else { 0.0 },
        );
        let Some(r) = &h.report else {
            // The message of a host without a report fills the OS cell onward.
            take(4, 220.0);
            continue;
        };
        take(
            4,
            (20.0 + gap + text_w(ui, r.os_pretty.as_str())).min(220.0),
        );
        let flags = host_flags(r);
        let chips: f32 = flags.iter().map(|f| text_w(ui, chip_text(f)) + gap).sum();
        take(5, chips);
        take(6, 70.0);
        if let Some(mem) = &r.memory {
            let text = format!("{} / {}", human_kb(mem.used_kb()), human_kb(mem.total_kb));
            take(7, (text_w(ui, text) + 8.0).min(170.0));
        }
        if let Some(d) = fullest_disk(r) {
            take(
                8,
                (text_w(ui, format!("{} {:.0}%", d.mount, d.used_pct())) + 8.0).min(180.0),
            );
        }
        take(9, text_w(ui, "100°C"));
        if let Some(t) = docker_text(r) {
            take(10, text_w(ui, t).min(200.0));
        }
    }
    want.iter().map(|w| w + CELL_PAD).collect()
}

/// The text of a host's cells (filter row, Filter by, exports), in column
/// order.
pub fn host_cells(m: &Model, i: usize, extra: &[SetKind]) -> Vec<String> {
    let h = &m.hosts[i];
    let paused = m.host_paused(h);
    let status = if h.error.is_some() {
        "down"
    } else if h.report.is_some() {
        "up"
    } else if paused {
        "paused"
    } else {
        "not checked"
    };
    let mut v = vec![
        status.to_string(),
        h.entry.alias.clone(),
        h.entry.note.clone(),
        h.entry.hostname.clone(),
    ];
    match &h.report {
        Some(r) => {
            let flags = host_flags(r);
            v.push(r.os_pretty.clone());
            v.push(
                flags
                    .iter()
                    .map(|f| f.short.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            v.push(
                r.cpu_usage_pct
                    .map(|c| format!("{c:.0}%"))
                    .unwrap_or_default(),
            );
            v.push(
                r.memory
                    .as_ref()
                    .map(|mem| format!("{} / {}", human_kb(mem.used_kb()), human_kb(mem.total_kb)))
                    .unwrap_or_default(),
            );
            v.push(
                fullest_disk(r)
                    .map(|d| format!("{} {:.0}%", d.mount, d.used_pct()))
                    .unwrap_or_default(),
            );
            v.push(hottest(r).map(|t| format!("{t:.0}°C")).unwrap_or_default());
            v.push(docker_text(r).unwrap_or_default());
        }
        None => {
            let msg = if let Some(e) = &h.error {
                e.clone()
            } else if h.loading {
                "loading…".into()
            } else if paused {
                "paused".into()
            } else {
                "queued".into()
            };
            v.push(msg);
            v.extend(std::iter::repeat_n(String::new(), 6));
        }
    }
    for kind in extra {
        v.push(h.entry.set(*kind).to_string());
    }
    v.push(ago(h.updated));
    v
}

/// A hosts table: the Hosts page, or a group / cluster page (#144, #145).
/// Built on the table component (#153): the sort stays the Hosts page's own
/// (`Msg::SortBy`), the filter row and menu come from the component.
pub fn table(
    ui: &mut egui::Ui,
    m: &Model,
    filter: &str,
    sort: Sort,
    view: &TableView,
    tables: &mut std::collections::BTreeMap<String, telepad_app::grid::TableState>,
    out: &mut Vec<Msg>,
) {
    use crate::table::{self as tc, Col as TCol, Controlled, Ev, Kind, Spec};
    let all = view_rows(m, filter, sort, view);
    let st = tables.entry(format!("hosts:{}", view.salt)).or_default();
    let cols = columns(view.extra);
    let n = cols.len();
    if st.filters.len() < n {
        st.filters.resize(n, String::new());
    }
    let cells: Vec<(usize, Vec<String>)> = all
        .iter()
        .map(|&i| (i, host_cells(m, i, view.extra)))
        .collect();
    let rows: Vec<usize> = cells
        .iter()
        .filter(|(_, c)| telepad_app::grid::row_matches(c, &st.filters))
        .map(|(i, _)| *i)
        .collect();
    let text_of = |i: usize, c: usize| {
        cells
            .iter()
            .find(|(h, _)| *h == i)
            .and_then(|(_, v)| v.get(c).cloned())
            .unwrap_or_default()
    };
    // Dragging rows changes the inventory order: only on the full list.
    let reorder =
        view.only.is_none() && !view.by_group && can_reorder(filter, false, sort) && !st.filtered();
    let accent = ui.visuals().selection.bg_fill;
    let layout_id = egui::Id::unique("overview-layout").with((view.salt, cols.len()));
    let mut lay: Layout = ui.data(|d| d.get_temp(layout_id)).unwrap_or_default();
    lay.manual.resize(n, None);
    let gap = ui.spacing().item_spacing.x;
    let in_header = |p: egui::Pos2| lay.header.is_some_and(|h| h.expand(6.0).contains(p));
    let (resizing, released, dbl) = ui.input(|i| {
        let from_header = i.pointer.press_origin().is_some_and(in_header);
        let dbl = i
            .pointer
            .button_double_clicked(egui::PointerButton::Primary)
            .then(|| i.pointer.latest_pos())
            .flatten()
            .filter(|p| in_header(*p));
        (
            i.pointer.primary_down() && from_header,
            i.pointer.primary_released() && (from_header || lay.pressed),
            dbl,
        )
    });
    // Column borders sit half a gap before the next column's left edge.
    let border = |c: usize| lay.left.get(c + 1).map(|l| l - gap / 2.0);
    if let Some(p) = dbl {
        // Double-click on a border: that column goes back to automatic.
        let near = (1..n - 1)
            .filter_map(|c| border(c).map(|b| (c, (p.x - b).abs())))
            .filter(|&(_, d)| d <= 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((c, _)) = near {
            lay.manual[c] = None;
        }
        lay.round = lay.round.wrapping_add(1);
    } else if released && lay.seen.len() == n && lay.widths.len() == n {
        // A drag on a border ended: that column keeps its new width.
        for c in 1..n - 1 {
            if (lay.seen[c] - lay.widths[c]).abs() > 1.5 {
                lay.manual[c] = Some(lay.seen[c]);
                lay.round = lay.round.wrapping_add(1);
            }
        }
    }
    lay.pressed = resizing;
    // While a border is dragged the table stays as it is.
    let widths = if resizing && lay.widths.len() == n {
        lay.widths.clone()
    } else {
        let min: Vec<f32> = cols.iter().map(|c| c.3).collect();
        // Gaps between columns and the vertical scroll bar.
        let avail = ui.available_width() - gap * (n as f32) - 14.0;
        let want = wanted(ui, m, &rows, sort, &cols, view.extra);
        let w: Vec<f32> = fit_widths(&want, &min, &STRETCH, &lay.manual, avail)
            .iter()
            .map(|w| w.round())
            .collect();
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        lay.round.hash(&mut h);
        for x in &w {
            (*x as i32).hash(&mut h);
        }
        lay.key = h.finish();
        lay.widths = w;
        lay.widths.clone()
    };
    let last = n - 1;
    let tcols: Vec<TCol> = cols
        .iter()
        .enumerate()
        .map(|(i, &(title, key, _, min))| {
            let width = if i == 0 {
                Column::exact(widths[i].max(min))
            } else if i == last {
                // Takes what is left, so rounding never leaves a gap.
                Column::remainder().at_least(min).clip(true)
            } else {
                Column::initial(widths[i]).at_least(min).clip(true)
            };
            let (shown, name) = if title.is_empty() {
                (icons::DOT, "status")
            } else {
                (title, title)
            };
            let c = TCol::new(shown).named(if title.is_empty() { "Status" } else { title }).width(width);
            if key.is_some() {
                c.tip(format!(
                    "Click to sort by {name} (again to reverse, a third time for inventory order). Columns fit the page by themselves; drag a border to set a width, double-click it to go back to automatic."
                ))
            } else {
                // Group / Cluster: not sortable.
                c.no_sort()
            }
        })
        .collect();
    let dir = cols.iter().enumerate().find_map(|(c, (_, k, _, _))| {
        k.and_then(|k| sort.direction(k)).map(|desc| {
            (
                c,
                if desc {
                    telepad_app::grid::Dir::Desc
                } else {
                    telepad_app::grid::Dir::Asc
                },
            )
        })
    });
    let title = match view.only {
        Some((kind, "")) => format!("Hosts - {}", kind.none_name()),
        Some((_, name)) => format!("Hosts - {name}"),
        None => "Hosts".to_string(),
    };
    let min_w: f32 = widths.iter().sum::<f32>() + gap * n as f32;
    let spec = Spec::new(("overview", view.salt, lay.key), &title, &tcols)
        .kind(Kind::Fill)
        .row_h(ROW_HEIGHT)
        .min_width(min_w);
    let mut auto_fit = false;
    let mut filters = std::mem::take(&mut st.filters);
    let o = tc::controlled(
        ui,
        &spec,
        Controlled {
            sort: dir,
            filters: Some(&mut filters[..]),
            rows: &rows,
            text: &text_of,
        },
        &mut |ui, ev| match ev {
            Ev::Cell { row: i, col } => {
                host_cell(ui, m, i, reorder, view.extra, col);
            }
            // SSH, Files, databases, edit… on right-click (#83).
            Ev::Menu { row: i, .. } => host_menu(ui, m, i, out),
            Ev::HeaderMenu => {
                if ui.button("Auto-fit columns").clicked() {
                    auto_fit = true;
                    ui.close();
                }
            }
        },
    );
    st.filters = filters;
    if let Some(c) = o.sort
        && let Some(key) = cols[c].1
    {
        out.push(Msg::SortBy(key));
    }
    if let Some((i, _)) = o.clicked {
        out.push(Msg::Select(Some(i)));
    }
    if reorder {
        for (i, resp) in &o.rows {
            drop_target(resp, *i, accent, out);
        }
    }
    if let Some(fmt) = o.export {
        let data: Vec<Vec<String>> = rows.iter().map(|&i| text_row(&cells, i)).collect();
        out.push(tc::export_rows(&title, &tcols, data, fmt));
    }
    out.extend(o.msgs);
    let header_rects = o.header;
    lay.left = header_rects.iter().map(|r| r.left()).collect();
    lay.seen = (0..n)
        .map(|c| match lay.left.get(c + 1) {
            Some(next) => next - lay.left[c] - gap,
            None => widths[c],
        })
        .collect();
    if auto_fit {
        lay.manual = vec![None; n];
        lay.round = lay.round.wrapping_add(1);
    }
    lay.header = Some(
        header_rects
            .iter()
            .fold(egui::Rect::NOTHING, |a, r| a.union(*r)),
    );
    ui.data_mut(|d| d.insert_temp(layout_id, lay));
}

fn text_row(cells: &[(usize, Vec<String>)], i: usize) -> Vec<String> {
    cells
        .iter()
        .find(|(h, _)| *h == i)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// One cell of a host row.
fn host_cell(ui: &mut egui::Ui, m: &Model, i: usize, grip: bool, extra: &[SetKind], col: usize) {
    let h = &m.hosts[i];
    let paused = m.host_paused(h);
    let last = COLUMNS.len() + extra.len() - 1;
    match col {
        0 => {
            if grip {
                grip_handle(ui, i);
            }
            status_dot(ui, h);
        }
        // Plain text: the whole row opens the host on click and its menu on
        // right-click, also over the name.
        1 => {
            ui.label(RichText::new(&h.entry.alias).strong());
        }
        2 => clipped(ui, RichText::new(&h.entry.note).italics()),
        3 => clipped(ui, RichText::new(&h.entry.hostname)),
        c if c == last => {
            if paused {
                ui.label(
                    RichText::new(icons::label(icons::PAUSE, &ago(h.updated)))
                        .color(usage_color(75.0)),
                )
                .on_hover_text("Updates paused for this host (right-click → Resume updates)");
            } else {
                ui.label(RichText::new(ago(h.updated)).weak());
            }
        }
        c if c >= COLUMNS.len() - 1 => {
            if let Some(kind) = extra.get(c - (COLUMNS.len() - 1)) {
                clipped(ui, RichText::new(h.entry.set(*kind)));
            }
        }
        c => match &h.report {
            Some(r) => report_cell(ui, h, r, c),
            None if c == 4 => {
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
            }
            None => {}
        },
    }
}

fn report_cell(ui: &mut egui::Ui, h: &HostState, r: &HostReport, col: usize) {
    match col {
        4 => {
            icons::os_logo(ui, host_os(h), 16.0);
            clipped(ui, RichText::new(&r.os_pretty));
        }
        5 => {
            let flags = host_flags(r);
            let gap = ui.spacing().item_spacing.x;
            let full: f32 = flags.iter().map(|f| text_w(ui, chip_text(f)) + gap).sum();
            if full <= ui.available_width() + 1.0 {
                flag_chips(ui, &flags);
            } else {
                // Not enough room for the labels: the coloured icons only, so no
                // flag is cut off; all details on hover.
                let all = flags
                    .iter()
                    .map(|f| f.detail.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let r = ui
                    .horizontal(|ui| {
                        for f in &flags {
                            ui.label(
                                RichText::new(flag_icon(f.kind)).color(severity_color(f.severity)),
                            );
                        }
                    })
                    .response;
                r.on_hover_text(all);
            }
        }
        6 => match r.cpu_usage_pct {
            Some(c) => bar(ui, c, format!("{c:.0}%"), ui.available_width()),
            None => dash(ui),
        },
        7 => match &r.memory {
            Some(mem) => {
                let text = format!("{} / {}", human_kb(mem.used_kb()), human_kb(mem.total_kb));
                bar(ui, mem.used_pct(), text, ui.available_width());
            }
            None => dash(ui),
        },
        8 => match fullest_disk(r) {
            Some(d) => {
                let text = format!("{} {:.0}%", d.mount, d.used_pct());
                bar(ui, d.used_pct(), text, ui.available_width());
            }
            None => dash(ui),
        },
        9 => {
            match hottest(r) {
                Some(t) => ui.colored_label(temp_color(t), format!("{t:.0}°C")),
                None => ui.label(RichText::new("n/a").weak()),
            };
        }
        10 => match docker_text(r) {
            Some(t) => clipped(ui, RichText::new(t)),
            None => dash(ui),
        },
        _ => {}
    }
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
    use super::{docker_text, fit_widths};

    #[test]
    fn spare_width_goes_to_the_stretch_columns() {
        let want = [36.0, 100.0, 200.0, 80.0];
        let min = [36.0, 60.0, 40.0, 50.0];
        let w = fit_widths(&want, &min, &[2], &[None; 4], 616.0);
        assert_eq!(w, vec![36.0, 100.0, 400.0, 80.0]);
        // Two stretch columns share in proportion to what they want.
        let w = fit_widths(&want, &min, &[1, 2], &[None; 4], 716.0);
        assert_eq!(w, vec![36.0, 200.0, 400.0, 80.0]);
    }

    #[test]
    fn too_little_room_shrinks_toward_the_minimums() {
        let want = [36.0, 100.0, 200.0, 80.0];
        let min = [36.0, 60.0, 40.0, 50.0];
        // 416 wanted, 316 there: 100 taken from 230 of room above the minimums.
        let w = fit_widths(&want, &min, &[2], &[None; 4], 316.0);
        assert!((w.iter().sum::<f32>() - 316.0).abs() < 0.01, "{w:?}");
        assert!(w[2] < 200.0 && w[2] > min[2]);
        // Far too narrow: every column at its minimum (the table scrolls).
        assert_eq!(
            fit_widths(&want, &min, &[2], &[None; 4], 10.0),
            min.to_vec()
        );
    }

    #[test]
    fn group_and_cluster_columns_give_way_before_host_and_flags() {
        // 12 fixed columns + Group + Cluster before Updated.
        let mut want = vec![40.0; 14];
        want[5] = 300.0; // Flags
        want[11] = 200.0; // Group
        want[12] = 200.0; // Cluster
        let min = vec![30.0; 14];
        let total: f32 = want.iter().sum();
        // 300 too little: Description … Temp go to their minimums (10 each),
        // the rest comes from Group / Cluster, Flags keeps its width.
        let w = fit_widths(&want, &min, &[2, 4, 5, 10], &[None; 14], total - 300.0);
        assert_eq!(w[5], 300.0, "{w:?}");
        assert!(w[11] < 200.0 && w[12] < 200.0, "{w:?}");
    }

    #[test]
    fn a_column_sized_by_hand_keeps_its_width() {
        let want = [36.0, 100.0, 200.0, 80.0];
        let min = [36.0, 60.0, 40.0, 50.0];
        let manual = [None, None, Some(120.0), None];
        // The stretch column is manual: nothing else grows.
        assert_eq!(
            fit_widths(&want, &min, &[2], &manual, 1000.0),
            vec![36.0, 100.0, 120.0, 80.0]
        );
        let w = fit_widths(&want, &min, &[2], &manual, 300.0);
        assert_eq!(w[2], 120.0);
    }
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
