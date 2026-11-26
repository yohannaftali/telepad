//! Table state shared by every table (#153): sort by header click, one
//! filter box per column (wtbapp rules), the pager of paged tables, and the
//! rows an export takes (the whole filtered and sorted list, every page).

use std::cmp::Ordering;

use crate::pager::Pager;
pub use telepad_core::table_export::{Format, PDF_ASK_ABOVE, TableData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

/// Sort, filters and page of one table, kept in [`crate::Drafts::tables`]
/// under the table's key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableState {
    /// Column and direction; `None` = the table's own order.
    pub sort: Option<(usize, Dir)>,
    /// One box per column (missing = empty).
    pub filters: Vec<String>,
    pub pager: Pager,
}

impl TableState {
    /// A header click: ascending → descending → the table's own order.
    pub fn click(&mut self, col: usize) {
        self.sort = match self.sort {
            Some((c, Dir::Asc)) if c == col => Some((col, Dir::Desc)),
            Some((c, Dir::Desc)) if c == col => None,
            _ => Some((col, Dir::Asc)),
        };
        self.pager.reset();
    }

    pub fn filter_mut(&mut self, col: usize) -> &mut String {
        if self.filters.len() <= col {
            self.filters.resize(col + 1, String::new());
        }
        &mut self.filters[col]
    }

    /// Right-click ▸ Filter by: the cell's value into its column's box.
    pub fn filter_by(&mut self, col: usize, value: &str) {
        *self.filter_mut(col) = filter_value(value);
        self.pager.reset();
    }

    /// Right-click ▸ Clear All Filters; `true` when a box had text.
    pub fn clear_filters(&mut self) -> bool {
        let had = self.filtered();
        self.filters.iter_mut().for_each(String::clear);
        self.pager.reset();
        had
    }

    pub fn filtered(&self) -> bool {
        self.filters.iter().any(|f| !f.trim().is_empty())
    }
}

/// The box text that finds `value`: the value itself (contains, as wtbapp's
/// Filter by), or `=value` when it would read as an operator (`>5`, `!x`,
/// `NULL`).
pub fn filter_value(value: &str) -> String {
    let v = value.trim();
    let lower = v.to_lowercase();
    if v.starts_with(['=', '>', '<', '!']) || lower == "null" {
        format!("={v}")
    } else {
        v.to_string()
    }
}

/// The filter box text that matches exactly `value` (#170, the value
/// dropdown): `=value`, or `NULL` for an empty cell.
pub fn exact_filter(value: &str) -> String {
    let v = value.trim();
    if v.is_empty() {
        "NULL".into()
    } else {
        format!("={v}")
    }
}

/// Most distinct values a filter dropdown lists (#170).
pub const VALUES_CAP: usize = 500;

/// The distinct values of a column for its filter dropdown (#170): trimmed,
/// once each (same text in any case counts once, first spelling kept), in
/// the table's natural order with empty last; at most `cap`, taken in row
/// order so a huge table stays fast.
pub fn distinct_values<'a>(cells: impl IntoIterator<Item = &'a str>, cap: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for c in cells {
        if out.len() >= cap {
            break;
        }
        let v = c.trim();
        if seen.insert(v.to_lowercase()) {
            out.push(v.to_string());
        }
    }
    out.sort_by(|a, b| compare(a, b));
    out
}

/// The values whose text contains `query` (any case), for the dropdown's
/// search field.
pub fn search_values<'a>(values: &'a [String], query: &str) -> Vec<&'a str> {
    let q = query.trim().to_lowercase();
    values
        .iter()
        .filter(|v| q.is_empty() || v.to_lowercase().contains(&q))
        .map(String::as_str)
        .collect()
}

/// A row that passes every filter box except column `skip`'s: the rows
/// whose values the dropdown of `skip` lists (#170).
pub fn row_matches_except(cell: &dyn Fn(usize) -> String, filters: &[String], skip: usize) -> bool {
    filters
        .iter()
        .enumerate()
        .all(|(c, f)| c == skip || f.trim().is_empty() || filter_match(&cell(c), f))
}

/// One filter box against one cell, as in the wtbapp tables: contains (any
/// case), `=x` equal, `>x` / `<x` (numbers by value, else text order, so
/// `>2026-11-20` works on times; units as [`value`]), `!x` not equal, `NULL`
/// empty, `!NULL` not empty.
pub fn filter_match(cell: &str, crit: &str) -> bool {
    let c = cell.trim().to_lowercase();
    let d = crit.trim().to_lowercase();
    match d.as_str() {
        "null" => return c.is_empty(),
        "!null" => return !c.is_empty(),
        _ => {}
    }
    // Numbers by value, also with a unit (`>50` finds `75%`, `>1 GiB` finds
    // `1.5 GiB`); else text order.
    let cmp = |v: &str| match (value(&c), value(v)) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => Some(c.as_str().cmp(v.trim())),
    };
    if let Some(v) = d.strip_prefix('=') {
        c == v.trim()
    } else if let Some(v) = d.strip_prefix('>') {
        cmp(v) == Some(Ordering::Greater)
    } else if let Some(v) = d.strip_prefix('<') {
        cmp(v) == Some(Ordering::Less)
    } else if let Some(v) = d.strip_prefix('!') {
        c != v.trim()
    } else {
        c.contains(&d)
    }
}

/// Text order with digit runs by value (`192.0.2.9` before `192.0.2.10`).
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(&c) = it.peek().filter(|c| c.is_ascii_digit()) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(p), Some(q)) => {
                if p != q {
                    return p.cmp(&q);
                }
                x.next();
                y.next();
            }
        }
    }
}

/// A cell as a number with its unit: `12%`, `-3.5`, `45°C`, `1.2 GiB`,
/// `980 MB`, `3d`, `2h 5m ago` … (sizes in bytes, times in seconds), else
/// `None`.
pub fn value(s: &str) -> Option<f64> {
    let t = s.trim();
    let t = t.strip_suffix(" ago").unwrap_or(t).trim();
    let parts: Vec<&str> = t.split_whitespace().collect();
    if parts.len() > 1
        && parts
            .iter()
            .all(|p| p.starts_with(|c: char| c.is_ascii_digit()))
    {
        return parts.iter().map(|p| one(p)).sum();
    }
    one(t)
}

fn one(t: &str) -> Option<f64> {
    let end = t
        .char_indices()
        .take_while(|(i, c)| {
            c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))
        })
        .map(|(i, c)| i + c.len_utf8())
        .last()?;
    let n: f64 = t[..end].parse().ok()?;
    let unit = t[end..].trim().to_lowercase();
    let k = match unit.as_str() {
        "" | "%" | "°c" | "°f" | "c" => 1.0,
        "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        // "m" alone is minutes ("5m ago"), as Telepad writes times.
        "mb" | "mib" => 1024f64.powi(2),
        "g" | "gb" | "gib" => 1024f64.powi(3),
        "t" | "tb" | "tib" => 1024f64.powi(4),
        "p" | "pb" | "pib" => 1024f64.powi(5),
        "s" | "sec" => 1.0,
        "m" | "min" => 60.0,
        "h" => 3600.0,
        "d" => 86_400.0,
        "w" => 604_800.0,
        _ => return None,
    };
    Some(n * k)
}

/// Sort order of two cells: empty last, numbers (with units) by value,
/// else text in natural order, any case.
pub fn compare(a: &str, b: &str) -> Ordering {
    match (a.trim().is_empty(), b.trim().is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    match (value(a), value(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => natural(&a.to_lowercase(), &b.to_lowercase()),
    }
}

/// `true` when `row` passes every box.
pub fn row_matches(row: &[String], filters: &[String]) -> bool {
    filters
        .iter()
        .enumerate()
        .all(|(c, f)| f.trim().is_empty() || filter_match(row.get(c).map_or("", String::as_str), f))
}

/// The rows in display order: every box applied, then the sort (stable, so
/// equal cells keep the table's own order; empty cells last both ways).
pub fn view(cells: &[Vec<String>], st: &TableState) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..cells.len())
        .filter(|&i| row_matches(&cells[i], &st.filters))
        .collect();
    if let Some((c, dir)) = st.sort {
        let get = |i: usize| cells[i].get(c).map_or("", String::as_str);
        rows.sort_by(|&x, &y| {
            let (a, b) = (get(x), get(y));
            let o = compare(a, b);
            match dir {
                Dir::Desc if !a.trim().is_empty() && !b.trim().is_empty() => o.reverse(),
                _ => o,
            }
        });
    }
    rows
}

/// What an export takes: the exported columns of `rows` (display order).
pub fn data(
    title: &str,
    columns: &[(String, bool)],
    cells: &[Vec<String>],
    rows: &[usize],
) -> TableData {
    let keep: Vec<usize> = (0..columns.len()).filter(|&c| columns[c].1).collect();
    TableData {
        title: title.to_string(),
        columns: keep.iter().map(|&c| columns[c].0.clone()).collect(),
        rows: rows
            .iter()
            .map(|&r| {
                keep.iter()
                    .map(|&c| cells[r].get(c).cloned().unwrap_or_default())
                    .collect()
            })
            .collect(),
    }
}

/// Messages of the table menu (#153).
#[derive(Debug, Clone)]
pub enum TableMsg {
    /// Export as CSV / Excel / PDF (save dialog) or copy as JSON.
    Export { data: TableData, format: Format },
    /// Yes / No to a PDF above [`PDF_ASK_ABOVE`] rows.
    Answer(bool),
    /// The file was written: Ok((rows, path)).
    Exported(Result<(usize, std::path::PathBuf), String>),
}
