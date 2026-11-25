//! The table component every table is built from (#153).
//!
//! Two kinds of table, one look and one behaviour:
//! - **simple** ([`Kind::Simple`] / [`Kind::Fill`]): every row, no pager;
//! - **paged** ([`Kind::Paged`]): the pager footer under the table.
//!
//! Both sort on a header click (ascending → descending → the table's own
//! order), have a filter row above the titles (wtbapp rules, see
//! [`telepad_app::grid::filter_match`]) and a right-click menu: the table's
//! own items first, then Filter by, Clear All Filters, Export as CSV / Excel
//! / PDF and Copy as JSON (as the wtbapp table menu, #341). Exports take the
//! whole filtered and sorted list, not only the page on screen.
//!
//! [`show`] keeps sort, filters and page in a [`TableState`] and filters /
//! sorts the rows in memory. [`controlled`] draws the same table for views
//! that keep their own order (Hosts, file panes, database grid, Log info):
//! the view gets the header clicks, filter changes and menu picks back in
//! [`Output`].

use eframe::egui::{self, Response, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use telepad_app::Msg;
use telepad_app::grid::{self, Dir, Format, TableData, TableMsg, TableState};

use crate::icons::{self, label as il};

/// One column.
#[derive(Clone)]
pub struct Col {
    pub title: String,
    /// Name in exports and the menu when the title is only an icon.
    pub name: Option<String>,
    pub width: Column,
    /// A header click sorts by it.
    pub sort: bool,
    /// It has a box in the filter row.
    pub filter: bool,
    /// Exports include it (icon / button columns do not).
    pub export: bool,
    /// Header hover text.
    pub tip: Option<String>,
}

impl Col {
    /// Sized to its content, sortable, filterable, exported.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            name: None,
            width: Column::auto().at_least(36.0).clip(true),
            sort: true,
            filter: true,
            export: true,
            tip: None,
        }
    }

    /// Buttons / icons only: no sort, filter or export.
    pub fn action(title: impl Into<String>) -> Self {
        Self {
            sort: false,
            filter: false,
            export: false,
            width: Column::auto(),
            ..Self::new(title)
        }
    }

    pub fn width(mut self, w: Column) -> Self {
        self.width = w;
        self
    }

    pub fn initial(self, w: f32) -> Self {
        self.width(Column::initial(w).at_least(36.0_f32.min(w)).clip(true))
    }

    pub fn remainder(self) -> Self {
        self.width(Column::remainder().at_least(60.0).clip(true))
    }

    pub fn no_sort(mut self) -> Self {
        self.sort = false;
        self
    }

    pub fn no_filter(mut self) -> Self {
        self.filter = false;
        self
    }

    pub fn no_export(mut self) -> Self {
        self.export = false;
        self
    }

    pub fn named(mut self, n: impl Into<String>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// Exports and the menu call it this.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.title)
    }

    pub fn tip(mut self, t: impl Into<String>) -> Self {
        self.tip = Some(t.into());
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// As tall as its rows, inside the page's own scrolling.
    Simple,
    /// Fills the height left and scrolls itself; no pager.
    Fill,
    /// Fills the height left above the pager footer.
    Paged,
}

/// How a table looks; [`Spec::new`] gives the defaults.
pub struct Spec<'a> {
    /// Keeps the table's widths and menu apart from other tables.
    pub id: egui::Id,
    /// Export title (PDF heading, file name).
    pub title: &'a str,
    pub cols: &'a [Col],
    pub kind: Kind,
    pub row_h: f32,
    /// The drawn row with this source index is highlighted.
    pub selected: Option<usize>,
    /// Wider than the window: scrolls sideways (0 = never).
    pub min_width: f32,
    pub filter_row: bool,
    pub resizable: bool,
    /// Rows highlight on hover and report clicks.
    pub sense: bool,
    /// Rows can also be dragged (file panes).
    pub drag: bool,
    /// Every source row in here is highlighted too (multi-select).
    pub selected_set: Option<&'a dyn Fn(usize) -> bool>,
}

impl<'a> Spec<'a> {
    pub fn new(
        id: impl std::hash::Hash + std::fmt::Debug,
        title: &'a str,
        cols: &'a [Col],
    ) -> Self {
        Self {
            id: egui::Id::unique("tp-table").with(id),
            title,
            cols,
            kind: Kind::Simple,
            row_h: 22.0,
            selected: None,
            min_width: 0.0,
            filter_row: true,
            resizable: true,
            sense: true,
            drag: false,
            selected_set: None,
        }
    }

    pub fn drag(mut self) -> Self {
        self.drag = true;
        self
    }

    pub fn selected_set(mut self, f: &'a dyn Fn(usize) -> bool) -> Self {
        self.selected_set = Some(f);
        self
    }

    pub fn kind(mut self, k: Kind) -> Self {
        self.kind = k;
        self
    }

    pub fn row_h(mut self, h: f32) -> Self {
        self.row_h = h;
        self
    }

    pub fn selected(mut self, r: Option<usize>) -> Self {
        self.selected = r;
        self
    }

    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
}

/// What the view draws for the component.
pub enum Ev {
    /// The content of a cell (source row, column).
    Cell { row: usize, col: usize },
    /// The table's own menu items for a right-click on this cell.
    Menu { row: usize, col: usize },
    /// The table's own items of the header menu.
    HeaderMenu,
}

/// What happened this frame. Row numbers are source rows.
#[derive(Default)]
pub struct Output {
    pub clicked: Option<(usize, usize)>,
    pub double_clicked: Option<(usize, usize)>,
    /// Every drawn row's response (drag and drop, hover).
    pub rows: Vec<(usize, Response)>,
    /// Header cells as drawn.
    pub header: Vec<egui::Rect>,
    /// A header click on this column ([`controlled`] only; [`show`] sorts).
    pub sort: Option<usize>,
    /// Typing in a filter box.
    pub filter_changed: bool,
    /// Enter in a box, Filter by or Clear All Filters: apply now.
    pub filter_enter: bool,
    pub cleared: bool,
    /// Export picked ([`controlled`] only; [`show`] puts it in `msgs`).
    pub export: Option<Format>,
    /// Messages for the update loop (exports).
    pub msgs: Vec<Msg>,
}

/// The view's own order for [`controlled`].
pub struct Controlled<'a> {
    pub sort: Option<(usize, Dir)>,
    /// One per column (at least as many as columns); `None`: no filter row.
    pub filters: Option<&'a mut [String]>,
    /// Source row of each drawn row, in order.
    pub rows: &'a [usize],
    /// A cell's text (Filter by): (source row, column).
    pub text: &'a dyn Fn(usize, usize) -> String,
}

/// A menu pick.
enum Pick {
    FilterBy(usize, String),
    Clear,
    Export(Format),
}

const HINT: &str = "Filter: text = contains · =x equal · !x not equal · >x / <x (numbers by value) · NULL empty · !NULL has a value";

/// Shortened for the menu label, as wtbapp (30 characters).
fn short(s: &str) -> String {
    let s = s.replace(['\n', '\r', '\t'], " ");
    if s.chars().count() > 30 {
        format!("{}...", s.chars().take(30).collect::<String>())
    } else {
        s
    }
}

/// The default items of every table menu.
fn default_items(
    ui: &mut Ui,
    filter_by: Option<(&str, String, usize)>,
    can_filter: bool,
    filtered: bool,
) -> Option<Pick> {
    let mut pick = None;
    if can_filter {
        let (text, on) = match &filter_by {
            Some((title, v, _)) if !v.trim().is_empty() => {
                let t = if title.is_empty() {
                    "this column"
                } else {
                    title
                };
                (format!("Filter by {t}: {}", short(v)), true)
            }
            _ => ("Filter by".to_string(), false),
        };
        if ui
            .add_enabled(on, egui::Button::new(il(icons::FUNNEL, &text)))
            .clicked()
            && let Some((_, v, c)) = filter_by
        {
            pick = Some(Pick::FilterBy(c, v));
            ui.close();
        }
        if ui
            .add_enabled(
                filtered,
                egui::Button::new(il(icons::ERASER, "Clear All Filters")),
            )
            .clicked()
        {
            pick = Some(Pick::Clear);
            ui.close();
        }
        ui.separator();
    }
    for (f, icon, text) in [
        (Format::Csv, icons::FILE_CSV, "Export as CSV…"),
        (Format::Xlsx, icons::FILE_XLS, "Export as Excel (.xlsx)…"),
        (Format::Pdf, icons::FILE_PDF, "Export as PDF…"),
    ] {
        if ui.button(il(icon, text)).clicked() {
            pick = Some(Pick::Export(f));
            ui.close();
        }
    }
    ui.separator();
    if ui
        .button(il(icons::BRACES, "Copy as JSON"))
        .on_hover_text(
            "Every row of the table (all pages, as filtered and sorted) to the clipboard",
        )
        .clicked()
    {
        pick = Some(Pick::Export(Format::Json));
        ui.close();
    }
    pick
}

/// Draws `f`'s own items; a separator follows when it drew any.
fn own_items(ui: &mut Ui, f: &mut dyn FnMut(&mut Ui, Ev), ev: Ev) {
    let before = ui.next_widget_position();
    f(ui, ev);
    if ui.next_widget_position() != before {
        ui.separator();
    }
}

/// The title line of a header cell (#151): the title on the left (cut with
/// "…" when narrow), the sort caret on the right edge, the whole line lit
/// up under the mouse when it sorts. Returns its click response.
fn title_line(ui: &mut Ui, c: &Col, dir: Option<Dir>, h: f32, id: egui::Id) -> Response {
    let colors = crate::widgets::pal().head;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
    let resp = ui.interact(rect, id, egui::Sense::click());
    let lit = c.sort && resp.hovered();
    // The highlight covers the whole cell width, gaps included.
    let half = ui.spacing().item_spacing.x / 2.0;
    if lit {
        ui.painter()
            .rect_filled(rect.expand2(egui::vec2(half, 1.0)), 2.0, colors.hover_bg);
    }
    let color = if lit { colors.hover_text } else { colors.text };
    let caret = match dir {
        Some(Dir::Asc) => Some(icons::CARET_UP),
        Some(Dir::Desc) => Some(icons::CARET_DOWN),
        None => None,
    };
    let font = egui::TextStyle::Body.resolve(ui.style());
    let icon_w = if caret.is_some() {
        font.size + 4.0
    } else {
        0.0
    };
    if let Some(caret) = caret {
        ui.painter().text(
            egui::pos2(rect.right() - 2.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            caret,
            font.clone(),
            color,
        );
    }
    let galley = egui::WidgetText::from(RichText::new(&c.title).strong().color(color)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() - icon_w - 2.0).max(8.0),
        egui::TextStyle::Body,
    );
    let pos = egui::pos2(rect.left() + 1.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    resp
}

/// A table whose order the view keeps (see the module docs).
pub fn controlled(
    ui: &mut Ui,
    spec: &Spec,
    ctl: Controlled,
    f: &mut dyn FnMut(&mut Ui, Ev),
) -> Output {
    let mut out = Output::default();
    let Controlled {
        sort,
        mut filters,
        rows,
        text,
    } = ctl;
    let n = spec.cols.len();
    if filters.as_deref().is_some_and(|fs| fs.len() < n) {
        filters = None;
    }
    let filter_row = spec.filter_row && filters.is_some() && spec.cols.iter().any(|c| c.filter);
    let filtered = filters
        .as_deref()
        .is_some_and(|fs| fs.iter().any(|f| !f.trim().is_empty()));
    let row_h = spec.row_h;
    let head_h = if filter_row { 2.0 * row_h + 6.0 } else { row_h };
    let target_id = spec.id.with("menu-target");
    let mut pick: Option<Pick> = None;
    let mut draw = |ui: &mut Ui| {
        ui.style_mut().interaction.selectable_labels = false;
        let fill_h = (ui.available_height() - head_h - 4.0).max(row_h * 2.0);
        let mut b = TableBuilder::new(ui)
            .id_salt(spec.id)
            .striped(true)
            .resizable(spec.resizable)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        if spec.drag {
            b = b.sense(egui::Sense::click_and_drag());
        } else if spec.sense {
            b = b.sense(egui::Sense::click());
        }
        b = match spec.kind {
            Kind::Simple => b.vscroll(false).auto_shrink([false, true]),
            Kind::Fill | Kind::Paged => b.auto_shrink([false, false]).max_scroll_height(fill_h),
        };
        for c in spec.cols {
            b = b.column(c.width);
        }
        let table = b.header(head_h, |mut h| {
            for (ci, c) in spec.cols.iter().enumerate() {
                let (rect, _) = h.col(|ui| {
                    // The header band (#151): its own colour, a line under it.
                    let colors = crate::widgets::pal().head;
                    let half = ui.spacing().item_spacing.x / 2.0;
                    let band = ui.max_rect().expand2(egui::vec2(half, 0.0));
                    ui.painter().rect_filled(band, 0.0, colors.bg);
                    ui.painter().hline(
                        band.x_range(),
                        band.bottom(),
                        egui::Stroke::new(1.5, colors.line),
                    );
                    ui.vertical(|ui| {
                        if filter_row {
                            match filters.as_deref_mut() {
                                Some(fs) if c.filter => {
                                    let w = ui.available_width().max(30.0);
                                    let r = ui.add(
                                        egui::TextEdit::singleline(&mut fs[ci])
                                            .hint_text(icons::FUNNEL)
                                            .desired_width(w),
                                    );
                                    if r.changed() {
                                        out.filter_changed = true;
                                    }
                                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                        out.filter_enter = true;
                                    }
                                    r.on_hover_text(HINT);
                                }
                                _ => {
                                    ui.add_space(row_h + 2.0);
                                }
                            }
                        }
                        let dir = sort.filter(|(s, _)| *s == ci).map(|(_, d)| d);
                        let resp = title_line(ui, c, dir, row_h, ui.scope_id().with(("head", ci)));
                        let resp = if c.sort {
                            let tip = c.tip.clone().unwrap_or_else(|| {
                                "Click to sort (again to reverse, a third time for the original order)"
                                    .into()
                            });
                            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(tip)
                        } else if let Some(t) = &c.tip {
                            resp.on_hover_text(t)
                        } else {
                            resp
                        };
                        // Near the right edge is the column border (resize).
                        let edge = ui.max_rect().right() - 8.0;
                        let on_border = resp.interact_pointer_pos().is_some_and(|p| p.x >= edge);
                        if c.sort && resp.clicked() && !on_border {
                            out.sort = Some(ci);
                        }
                        resp.context_menu(|ui| {
                            own_items(ui, f, Ev::HeaderMenu);
                            if let Some(p) = default_items(ui, None, filter_row, filtered) {
                                pick = Some(p);
                            }
                        });
                    });
                });
                out.header.push(rect);
            }
        });
        table.body(|body| {
            body.rows(row_h, rows.len(), |mut row| {
                let src = rows[row.index()];
                row.set_selected(
                    spec.selected == Some(src) || spec.selected_set.is_some_and(|f| f(src)),
                );
                let mut rects = Vec::with_capacity(n);
                for ci in 0..n {
                    let (r, _) = row.col(|ui| f(ui, Ev::Cell { row: src, col: ci }));
                    rects.push(r);
                }
                let resp = row.response();
                let col_at = |p: Option<egui::Pos2>| {
                    p.and_then(|p| {
                        rects
                            .iter()
                            .position(|r| p.x >= r.left() && p.x < r.right() + 4.0)
                    })
                    .unwrap_or(0)
                };
                if resp.clicked() {
                    out.clicked = Some((src, col_at(resp.interact_pointer_pos())));
                }
                if resp.double_clicked() {
                    out.double_clicked = Some((src, col_at(resp.interact_pointer_pos())));
                }
                if resp.secondary_clicked() {
                    let c = col_at(resp.interact_pointer_pos());
                    resp.ctx.data_mut(|d| d.insert_temp(target_id, (src, c)));
                }
                resp.context_menu(|ui| {
                    let (r, c) = ui
                        .ctx()
                        .data(|d| d.get_temp::<(usize, usize)>(target_id))
                        .filter(|(r, _)| *r == src)
                        .unwrap_or((src, 0));
                    own_items(ui, f, Ev::Menu { row: r, col: c });
                    let col = &spec.cols[c.min(n.saturating_sub(1))];
                    let by = (filter_row && col.filter).then(|| (col.label(), text(r, c), c));
                    if let Some(p) = default_items(ui, by, filter_row, filtered) {
                        pick = Some(p);
                    }
                });
                out.rows.push((src, resp));
            });
        });
    };
    if spec.min_width > 0.0 {
        egui::ScrollArea::horizontal()
            .id_salt(spec.id.with("h"))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_min_width(spec.min_width);
                draw(ui);
            });
    } else {
        draw(ui);
    }
    match pick {
        Some(Pick::FilterBy(c, v)) => {
            if let Some(fs) = filters {
                fs[c] = grid::filter_value(&v);
                out.filter_changed = true;
                out.filter_enter = true;
            }
        }
        Some(Pick::Clear) => {
            if let Some(fs) = filters {
                fs.iter_mut().for_each(String::clear);
            }
            out.cleared = true;
            out.filter_changed = true;
            out.filter_enter = true;
        }
        Some(Pick::Export(fmt)) => out.export = Some(fmt),
        None => {}
    }
    out
}

/// A table filtered and sorted in memory: `cells` holds each row's text per
/// column (what the filter boxes match, the sort compares and exports
/// write); `f` draws the cells and the table's own menu items.
pub fn show(
    ui: &mut Ui,
    spec: &Spec,
    st: &mut TableState,
    cells: &[Vec<String>],
    mut f: impl FnMut(&mut Ui, Ev),
) -> Output {
    let order = grid::view(cells, st);
    let total = order.len();
    let (start, end) = match spec.kind {
        Kind::Paged => {
            let (s, e, _) = st.pager.range(total);
            (s, e)
        }
        _ => (0, total),
    };
    let text = |r: usize, c: usize| {
        cells
            .get(r)
            .and_then(|x| x.get(c))
            .cloned()
            .unwrap_or_default()
    };
    let mut filters = std::mem::take(&mut st.filters);
    if filters.len() < spec.cols.len() {
        filters.resize(spec.cols.len(), String::new());
    }
    let mut out = if spec.kind == Kind::Paged {
        let h = (ui.available_height() - crate::widgets::PAGER_H - 6.0).max(120.0);
        let o = ui
            .allocate_ui(egui::vec2(ui.available_width(), h), |ui| {
                ui.set_min_height(h);
                controlled(
                    ui,
                    spec,
                    Controlled {
                        sort: st.sort,
                        filters: Some(&mut filters[..]),
                        rows: &order[start..end],
                        text: &text,
                    },
                    &mut f,
                )
            })
            .inner;
        ui.separator();
        crate::widgets::pager_bar(ui, &mut st.pager, Some(total), spec.id.with("pager"));
        o
    } else {
        controlled(
            ui,
            spec,
            Controlled {
                sort: st.sort,
                filters: Some(&mut filters[..]),
                rows: &order[start..end],
                text: &text,
            },
            &mut f,
        )
    };
    st.filters = filters;
    if let Some(c) = out.sort.take() {
        st.click(c);
    }
    if out.filter_changed {
        st.pager.reset();
    }
    if let Some(fmt) = out.export.take() {
        out.msgs
            .push(export_msg(spec.title, spec.cols, cells, &order, fmt));
    }
    out
}

/// The export message for rows `order` of `cells` (exported columns only).
pub fn export_msg(
    title: &str,
    cols: &[Col],
    cells: &[Vec<String>],
    order: &[usize],
    format: Format,
) -> Msg {
    let columns: Vec<(String, bool)> = cols
        .iter()
        .map(|c| (c.label().to_string(), c.export))
        .collect();
    Msg::Table(TableMsg::Export {
        data: grid::data(title, &columns, cells, order),
        format,
    })
}

/// The export message for rows already as text (one entry per column of
/// `cols`, in display order).
pub fn export_rows(title: &str, cols: &[Col], rows: Vec<Vec<String>>, format: Format) -> Msg {
    let keep: Vec<usize> = (0..cols.len()).filter(|&c| cols[c].export).collect();
    Msg::Table(TableMsg::Export {
        data: TableData {
            title: title.to_string(),
            columns: keep.iter().map(|&c| cols[c].label().to_string()).collect(),
            rows: rows
                .into_iter()
                .map(|r| {
                    keep.iter()
                        .map(|&c| r.get(c).cloned().unwrap_or_default())
                        .collect()
                })
                .collect(),
        },
        format,
    })
}

/// The state of table `key` in a table map (views that get only part of the
/// drafts).
pub fn state_in(
    t: &mut std::collections::BTreeMap<String, TableState>,
    key: impl Into<String>,
) -> &mut TableState {
    t.entry(key.into()).or_default()
}

/// The state of table `key` in the drafts.
pub fn state(d: &mut telepad_app::Drafts, key: impl Into<String>) -> &mut TableState {
    d.tables.entry(key.into()).or_default()
}

/// A plain cell: one line, cut with "…", the full text on hover when cut.
pub fn text(ui: &mut Ui, t: &str) {
    ui.add(egui::Label::new(t).truncate().selectable(false));
}

/// A cell of weak (grey) text.
pub fn weak(ui: &mut Ui, t: &str) {
    ui.add(
        egui::Label::new(RichText::new(t).weak())
            .truncate()
            .selectable(false),
    );
}

/// A cell of monospace text.
pub fn mono(ui: &mut Ui, t: &str) {
    ui.add(
        egui::Label::new(RichText::new(t).monospace())
            .truncate()
            .selectable(false),
    );
}
