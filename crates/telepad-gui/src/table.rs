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
    /// Width from its text (title and cells), not from `width`.
    pub fit: bool,
    /// Its cells are monospace (addresses, ports): measured that way.
    pub mono: bool,
    /// Sized by its content (buttons, icons): the header takes no more room
    /// than its title, so the column can shrink to its buttons.
    pub auto: bool,
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
            fit: true,
            mono: false,
            auto: false,
            tip: None,
        }
    }

    /// Buttons / icons only: no sort, filter or export.
    pub fn action(title: impl Into<String>) -> Self {
        Self {
            sort: false,
            filter: false,
            export: false,
            fit: false,
            auto: true,
            width: Column::auto(),
            ..Self::new(title)
        }
    }

    pub fn width(mut self, w: Column) -> Self {
        self.fit = false;
        self.width = w;
        self
    }

    pub fn initial(self, w: f32) -> Self {
        self.width(Column::initial(w).at_least(36.0_f32.min(w)).clip(true))
    }

    pub fn remainder(self) -> Self {
        self.width(Column::remainder().at_least(60.0).clip(true))
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
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
    /// Ids of the filter boxes and the menu: stable even when `id` changes
    /// (the Hosts table renews `id` when its widths are fitted again, which
    /// must not take the focus out of a box being typed in).
    pub stable: egui::Id,
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
            id: egui::Id::unique("tp-table").with(&id),
            stable: egui::Id::unique("tp-table-stable").with(id),
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

    /// Keeps filter boxes and the menu across `id` changes.
    pub fn stable(mut self, id: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.stable = egui::Id::unique("tp-table-stable").with(id);
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
    /// The rows are scrolled within reach of their bottom (#152).
    pub near_bottom: bool,
    /// Every row fits: no vertical scrolling yet (#152).
    pub fits: bool,
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

/// Where the pointer is for a row's click: a right-click reports no
/// interact position, so the latest pointer position stands in.
fn pointer(resp: &Response) -> Option<egui::Pos2> {
    resp.interact_pointer_pos()
        .or_else(|| resp.ctx.input(|i| i.pointer.latest_pos()))
}

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
/// One line of `t` in the body font (strong for titles).
fn text_width(ui: &Ui, t: &str, strong: bool) -> f32 {
    measure(ui, t, strong, false)
}

fn measure(ui: &Ui, t: &str, strong: bool, mono: bool) -> f32 {
    let mut rt = RichText::new(t);
    if strong {
        rt = rt.strong();
    }
    if mono {
        rt = rt.monospace();
    }
    egui::WidgetText::from(rt)
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
        .size()
        .x
}

/// Widths of the text-fitted columns: the title with room for the sort
/// caret, or the longest value of the rows on screen (a sample of 300),
/// whichever is wider, within 40 ..= 420 points.
fn fit_widths(
    ui: &Ui,
    spec: &Spec,
    rows: &[usize],
    text: &dyn Fn(usize, usize) -> String,
) -> Vec<Option<(f32, f32)>> {
    let caret = egui::TextStyle::Body.resolve(ui.style()).size + 8.0;
    spec.cols
        .iter()
        .enumerate()
        .map(|(ci, c)| {
            if !c.fit {
                return None;
            }
            let head = text_width(ui, &c.title, true) + if c.sort { caret } else { 4.0 } + 6.0;
            let body = rows
                .iter()
                .take(300)
                .map(|&r| measure(ui, &text(r, ci), false, c.mono))
                .fold(0.0, f32::max)
                // Monospace and strong cells run a little wider.
                * 1.12
                + 10.0;
            let want = head.max(body).clamp(40.0, 420.0).round();
            Some((want, head.clamp(30.0, want)))
        })
        .collect()
}

/// Widths of the text-fitted columns for `room` points (#156, as the Hosts
/// table #143): each wants its text, a column the user sized keeps that
/// width; spare room goes to the long text columns (natural width of 100
/// points or more, else all), in proportion to their text; too little room
/// shrinks them toward their titles in proportion to the room they have
/// above them (then the table scrolls sideways).
pub fn fill_widths(want: &[f32], least: &[f32], manual: &[Option<f32>], room: f32) -> Vec<f32> {
    let n = want.len();
    let fixed = |i: usize| manual.get(i).copied().flatten();
    let mut w: Vec<f32> = (0..n)
        .map(|i| fixed(i).unwrap_or(want[i]).max(least[i]))
        .collect();
    let total: f32 = w.iter().sum();
    let free: Vec<usize> = (0..n).filter(|&i| fixed(i).is_none()).collect();
    if total < room {
        let long: Vec<usize> = free.iter().copied().filter(|&i| want[i] >= 100.0).collect();
        let grow = if long.is_empty() { free } else { long };
        let weight: f32 = grow.iter().map(|&i| w[i]).sum();
        if weight > 0.0 {
            let spare = room - total;
            for &i in &grow {
                w[i] += spare * w[i] / weight;
            }
        }
    } else if total > room {
        let give: f32 = free.iter().map(|&i| w[i] - least[i]).sum();
        if give > 0.0 {
            let k = ((total - room) / give).min(1.0);
            for &i in &free {
                w[i] -= (w[i] - least[i]) * k;
            }
        }
    }
    w
}

/// Width a column that is not text-fitted needs: what it was drawn at, or
/// its widest content when that was more (squeezed buttons), 60 at first.
fn other_width(lay: &Layout, c: usize) -> f32 {
    let w = lay.seen[c].max(lay.content[c]);
    if w > 0.0 { w } else { 60.0 }
}

/// Column layout of a table with text-fitted columns, kept between frames
/// (the Hosts table's #143 rules for every table, #156).
#[derive(Clone, Default)]
struct Layout {
    /// Widths the user dragged a column to; `None` = automatic.
    manual: Vec<Option<f32>>,
    /// Widths given to egui_extras this frame.
    widths: Vec<f32>,
    /// Each column's width as last drawn.
    seen: Vec<f32>,
    /// Widest cell content of the columns sized by their content (buttons),
    /// which the page may have squeezed: their real need.
    content: Vec<f32>,
    left: Vec<f32>,
    header: Option<egui::Rect>,
    pressed: bool,
    round: u32,
}

fn title_line(ui: &mut Ui, c: &Col, dir: Option<Dir>, h: f32, id: egui::Id) -> Response {
    let colors = crate::widgets::pal().head;
    // A content-sized column: claim only the title, so it can shrink.
    let w = if c.auto {
        text_width(ui, &c.title, true).min(ui.available_width())
    } else {
        ui.available_width()
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
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
    // Filter box and title line have the same height (#156).
    // egui draws a text box's frame a little inside the space it gets, so
    // the filter row takes BOX_INSET more for the box to look one row tall
    // (#164).
    const BOX_INSET: f32 = 3.0;
    let box_h = row_h + BOX_INSET;
    let head_h = if filter_row {
        box_h + row_h + ui.spacing().item_spacing.y
    } else {
        row_h
    };
    // The filter box is exactly one row tall (#164): its own size, not one
    // that follows from the text height and a rounded margin.
    let font_h = ui.text_style_height(&egui::TextStyle::Body);
    let box_margin = ((box_h - font_h) / 2.0).floor().clamp(0.0, 8.0);
    let target_id = spec.stable.with("menu-target");
    let mut pick: Option<Pick> = None;
    let mut scrolled: Option<(f32, f32, f32)> = None;
    let mut used = vec![0.0_f32; n];
    let natural = fit_widths(ui, spec, rows, text);
    let any_fit = natural.iter().any(Option::is_some);
    let gap = ui.spacing().item_spacing.x;
    let layout_id = spec.stable.with(("layout", n));
    let mut lay: Layout = ui.data(|d| d.get_temp(layout_id)).unwrap_or_default();
    lay.manual.resize(n, None);
    lay.seen.resize(n, 0.0);
    lay.content.resize(n, 0.0);
    // A press in the header row may be a border drag: the widths stay while
    // it lasts, and a dragged column keeps its width when it ends.
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
    if let Some(p) = dbl {
        // Double-click on a border: that column goes back to automatic.
        let near = (0..n.saturating_sub(1))
            .filter_map(|c| {
                lay.left
                    .get(c + 1)
                    .map(|l| (c, (p.x - (l - gap / 2.0)).abs()))
            })
            .filter(|&(_, d)| d <= 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((c, _)) = near {
            lay.manual[c] = None;
        }
        lay.round = lay.round.wrapping_add(1);
    } else if released && lay.widths.len() == n {
        for (c, nat) in natural.iter().enumerate() {
            if nat.is_some() && (lay.seen[c] - lay.widths[c]).abs() > 1.5 {
                lay.manual[c] = Some(lay.seen[c]);
                lay.round = lay.round.wrapping_add(1);
            }
        }
    }
    lay.pressed = resizing;
    let fits: Vec<Option<f32>> = if resizing && lay.widths.len() == n {
        (0..n).map(|c| natural[c].map(|_| lay.widths[c])).collect()
    } else {
        // Room for the fitted columns: the page less the other columns (as
        // last drawn) and the gaps, and the scroll bar of a scrolling table.
        let bar = if spec.kind == Kind::Simple { 0.0 } else { 14.0 };
        let others: f32 = (0..n)
            .filter(|&c| natural[c].is_none())
            .map(|c| other_width(&lay, c))
            .sum();
        let room = ui.available_width() - gap * n as f32 - others - bar;
        let idx: Vec<usize> = (0..n).filter(|&c| natural[c].is_some()).collect();
        let want: Vec<f32> = idx
            .iter()
            .map(|&c| natural[c].map_or(0.0, |w| w.0))
            .collect();
        let least: Vec<f32> = idx
            .iter()
            .map(|&c| natural[c].map_or(0.0, |w| w.1))
            .collect();
        let manual: Vec<Option<f32>> = idx.iter().map(|&c| lay.manual[c]).collect();
        let w = fill_widths(&want, &least, &manual, room);
        let mut out_w = vec![None; n];
        for (k, &c) in idx.iter().enumerate() {
            out_w[c] = Some(w[k].floor());
        }
        out_w
    };
    lay.widths = (0..n).map(|c| fits[c].unwrap_or(0.0)).collect();
    // New widths start the columns again (egui_extras keeps the widths of a
    // table id); a drag on a border lasts until double-click / Auto-fit.
    let fit_key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        lay.round.hash(&mut h);
        for w in &fits {
            w.map(|w| w as i32).hash(&mut h);
        }
        h.finish()
    };
    let last_fit = (0..n).rev().find(|&c| fits[c].is_some());
    let mut auto_fit = false;
    let mut draw = |ui: &mut Ui| {
        ui.style_mut().interaction.selectable_labels = false;
        let fill_h = (ui.available_height() - head_h - 4.0).max(row_h * 2.0);
        let mut b = TableBuilder::new(ui)
            .id_salt(spec.id.with(fit_key))
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
        for (ci, (c, fit)) in spec.cols.iter().zip(&fits).enumerate() {
            let least = natural[ci].map_or(30.0, |w| w.1.min(30.0));
            b = b.column(match fit {
                // The last fitted column takes what is left, so the table
                // ends at the page edge.
                Some(_) if Some(ci) == last_fit && ci + 1 == n => {
                    Column::remainder().at_least(least).clip(true)
                }
                Some(w) => Column::initial(*w).at_least(least).clip(true),
                None => c.width,
            });
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
                                            .id(spec.stable.with(("filter", ci)))
                                            .margin(egui::Margin::symmetric(4, box_margin as i8))
                                            .min_size(egui::vec2(w, box_h))
                                            .align(egui::Align2::LEFT_CENTER)
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
                                    // A slot as tall as the box, so every title is level.
                                    ui.allocate_exact_size(
                                        egui::vec2(ui.available_width().min(1.0), box_h),
                                        egui::Sense::hover(),
                                    );
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
                            if any_fit && ui.button("Auto-fit columns").clicked() {
                                auto_fit = true;
                                ui.close();
                            }
                            if let Some(p) = default_items(ui, None, filter_row, filtered) {
                                pick = Some(p);
                            }
                        });
                    });
                });
                out.header.push(rect);
            }
        });
        let scroll = table.body(|body| {
            body.rows(row_h, rows.len(), |mut row| {
                let src = rows[row.index()];
                row.set_selected(
                    spec.selected == Some(src) || spec.selected_set.is_some_and(|f| f(src)),
                );
                let mut rects = Vec::with_capacity(n);
                for (ci, need) in used.iter_mut().enumerate() {
                    let (r, _) = row.col(|ui| {
                        let start = ui.cursor().min.x;
                        f(ui, Ev::Cell { row: src, col: ci });
                        *need = need.max(ui.min_rect().right() - start);
                    });
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
                    out.clicked = Some((src, col_at(pointer(&resp))));
                }
                if resp.double_clicked() {
                    out.double_clicked = Some((src, col_at(pointer(&resp))));
                }
                if resp.secondary_clicked() {
                    let c = col_at(pointer(&resp));
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
        scrolled = Some((
            scroll.state.clamped_offset().y,
            scroll.inner_rect.height(),
            scroll.content_size.y,
        ));
    };
    // Wider than the room: scroll sideways rather than cut the last columns.
    let total: f32 = (0..n)
        .map(|c| fits[c].unwrap_or_else(|| other_width(&lay, c)) + gap)
        .sum();
    let min_width = if spec.min_width > 0.0 {
        spec.min_width
    } else if total > ui.available_width() + 1.0 {
        total
    } else {
        0.0
    };
    if min_width > 0.0 {
        egui::ScrollArea::horizontal()
            .id_salt(spec.stable.with("h"))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_min_width(min_width);
                draw(ui);
            });
    } else {
        draw(ui);
    }
    for (c, nat) in natural.iter().enumerate() {
        if nat.is_none() && used[c] > 0.0 {
            lay.content[c] = used[c] + 6.0;
        }
    }
    lay.left = out.header.iter().map(|r| r.left()).collect();
    lay.seen = (0..n)
        .map(|c| match (out.header.get(c), out.header.get(c + 1)) {
            (Some(r), Some(next)) => next.left() - r.left() - gap,
            (Some(r), None) => r.width(),
            _ => 0.0,
        })
        .collect();
    lay.header = Some(
        out.header
            .iter()
            .fold(egui::Rect::NOTHING, |a, r| a.union(*r)),
    );
    if auto_fit {
        lay.manual = vec![None; n];
        lay.round = lay.round.wrapping_add(1);
    }
    ui.data_mut(|d| d.insert_temp(layout_id, lay));
    // Continuous mode (#152): within 80 points of the bottom loads more
    // (wtbapp); a view the rows do not fill yet is "at the bottom" too.
    if let Some((offset, view, content)) = scrolled {
        out.fits = content <= view + 2.0;
        out.near_bottom = offset + view >= content - 80.0;
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
    if spec.kind == Kind::Paged {
        st.pager.start_mode(ui.available_width());
    }
    let mut out = if spec.kind == Kind::Paged {
        let h = (ui.available_height() - crate::widgets::pager_height(ui.available_width()) - 6.0)
            .max(120.0);
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
    if spec.kind == Kind::Paged && st.pager.wants_more(total, out.near_bottom, out.fits) {
        st.pager.more(total);
        ui.ctx().request_repaint();
    }
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

#[cfg(test)]
mod tests {
    use super::fill_widths;

    #[test]
    fn spare_room_goes_to_long_text_columns() {
        // A long text column (300) and two short ones (50, 60).
        let w = fill_widths(&[300.0, 50.0, 60.0], &[60.0, 40.0, 40.0], &[None; 3], 610.0);
        assert_eq!(w, vec![500.0, 50.0, 60.0]);
        // No long column: all share in proportion.
        let w = fill_widths(&[50.0, 50.0], &[40.0, 40.0], &[None; 2], 200.0);
        assert_eq!(w, vec![100.0, 100.0]);
    }

    #[test]
    fn too_little_room_shrinks_toward_the_titles() {
        let want = [300.0, 50.0];
        let least = [100.0, 40.0];
        let w = fill_widths(&want, &least, &[None; 2], 290.0);
        assert!((w.iter().sum::<f32>() - 290.0).abs() < 0.01, "{w:?}");
        assert!(w[0] < 300.0 && w[0] > 100.0 && w[1] < 50.0 && w[1] >= 40.0);
        // Far too narrow: every column at its title width (the table scrolls).
        assert_eq!(fill_widths(&want, &least, &[None; 2], 10.0), least.to_vec());
    }

    #[test]
    fn a_column_sized_by_hand_keeps_its_width() {
        let manual = [Some(120.0), None];
        let w = fill_widths(&[300.0, 50.0], &[60.0, 40.0], &manual, 1000.0);
        assert_eq!(w[0], 120.0);
        assert_eq!(w[1], 880.0);
    }
}
