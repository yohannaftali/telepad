//! Pager of a table (#138, #140), as in the Senar / wtbapp `code2` tables
//! (`y.panel.js` `pager_*`, `y.app.js` `y_record_info`, `y_page_size_ask`):
//! `Record [1 - 20] of 279`, first / previous / `Page [n] of N` / next /
//! last, and Rows per page typed by the user (empty = 100, more than 100
//! asks first).
//!
//! Continuous mode (#152, as wtbapp `y.form.js` `lazy` / Senar #1104): no
//! page buttons, the rows grow by "Rows per load" when the table is
//! scrolled to its bottom, and fill the view by themselves up to
//! [`FILL_MAX`] rows. Each table has its own mode (not saved); pagination
//! by default, continuous when the table first shows in a narrow window.

/// Rows per page when the box is emptied.
pub const EMPTY_SIZE: usize = 100;
/// More than this asks first (slow on big lists).
pub const ASK_ABOVE: usize = 100;
/// The most rows one page may hold.
pub const MAX_SIZE: usize = 5000;
/// Continuous mode loads by itself (rows do not fill the view yet) up to
/// this many rows, as wtbapp / Senar.
pub const FILL_MAX: usize = 500;
/// A table first drawn narrower than this starts continuous (Senar: 768 px).
pub const NARROW: f32 = 768.0;

/// Where a pager button goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Go {
    First,
    Prev,
    Next,
    Last,
}

/// A pager's state: what the view binds to (typed texts) and the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pager {
    /// 0-based.
    pub page: usize,
    pub per_page: usize,
    /// The Rows per page box as typed.
    pub per_page_text: String,
    /// The page number box as typed (1-based).
    pub page_text: String,
    /// A typed size above [`ASK_ABOVE`] waiting for Yes / No.
    pub ask: Option<usize>,
    /// Continuous mode (#152) instead of pages.
    pub continuous: bool,
    /// Continuous mode: rows shown so far (from the first).
    pub loaded: usize,
    /// The mode was chosen (first draw or the toggle).
    pub mode_set: bool,
}

impl Pager {
    pub fn new(per_page: usize) -> Self {
        Self {
            page: 0,
            per_page,
            per_page_text: per_page.to_string(),
            page_text: "1".into(),
            ask: None,
            continuous: false,
            loaded: per_page,
            mode_set: false,
        }
    }

    /// The first draw: a narrow table starts continuous (Senar), once.
    pub fn start_mode(&mut self, width: f32) {
        if !self.mode_set {
            self.mode_set = true;
            if width < NARROW {
                self.continuous = true;
                self.reset();
            }
        }
    }

    /// The pager's mode button: pages ↔ continuous, back to the first row.
    pub fn toggle(&mut self) {
        self.mode_set = true;
        self.continuous = !self.continuous;
        self.reset();
    }

    /// Continuous mode: the next rows; `true` when more are shown.
    pub fn more(&mut self, total: usize) -> bool {
        if !self.continuous || self.loaded >= total {
            return false;
        }
        self.loaded = (self.loaded + self.per_page.max(1)).min(total);
        true
    }

    /// Continuous mode: load more now? The table is scrolled within reach of
    /// its bottom (`near_bottom`) and there are more rows; while the rows do
    /// not fill the view (`fits`), only up to [`FILL_MAX`].
    pub fn wants_more(&self, total: usize, near_bottom: bool, fits: bool) -> bool {
        self.continuous && near_bottom && self.loaded < total && (!fits || self.loaded < FILL_MAX)
    }

    /// (first index, end index, page count) for `total` rows; the page is
    /// clamped to the last one.
    pub fn range(&self, total: usize) -> (usize, usize, usize) {
        let per = self.per_page.max(1);
        let pages = total.div_ceil(per).max(1);
        if self.continuous {
            return (0, self.loaded.max(per).min(total), pages);
        }
        let page = self.page.min(pages - 1);
        let start = page * per;
        (start.min(total), (start + per).min(total), pages)
    }

    /// The pages for `total` rows.
    pub fn pages(&self, total: usize) -> usize {
        self.range(total).2
    }

    /// Back to the first page (new filter, new sort).
    pub fn reset(&mut self) {
        self.page = 0;
        self.page_text = "1".into();
        self.loaded = self.per_page;
    }

    fn set_page(&mut self, page: usize) {
        self.page = page;
        self.page_text = (page + 1).to_string();
    }

    /// A button: `true` when the page changed.
    pub fn go(&mut self, to: Go, total: usize) -> bool {
        let pages = self.pages(total);
        let cur = self.page.min(pages - 1);
        let target = match to {
            Go::First => 0,
            Go::Prev => cur.saturating_sub(1),
            Go::Next => (cur + 1).min(pages - 1),
            Go::Last => pages - 1,
        };
        let changed = target != self.page;
        self.set_page(target);
        changed
    }

    /// The typed page number (Enter): clamped to 1 ..= pages; `true` when
    /// the page changed.
    pub fn apply_page(&mut self, total: usize) -> bool {
        let pages = self.pages(total);
        let n = digits(&self.page_text).unwrap_or(1).clamp(1, pages);
        let changed = n - 1 != self.page;
        self.set_page(n - 1);
        changed
    }

    /// The typed Rows per page: empty / not a number = 100, capped at
    /// [`MAX_SIZE`]; above [`ASK_ABOVE`] it waits for [`Pager::answer`].
    /// `true` when the size changed now.
    pub fn apply_per_page(&mut self) -> bool {
        let n = digits(&self.per_page_text)
            .filter(|n| *n > 0)
            .map_or(EMPTY_SIZE, |n| n.min(MAX_SIZE));
        if n == self.per_page {
            self.per_page_text = n.to_string();
            return false;
        }
        if n > ASK_ABOVE {
            self.ask = Some(n);
            return false;
        }
        self.set_size(n);
        true
    }

    /// Yes / No to a size above [`ASK_ABOVE`]; `true` when it changed.
    pub fn answer(&mut self, yes: bool) -> bool {
        let Some(n) = self.ask.take() else {
            return false;
        };
        if yes {
            self.set_size(n);
            true
        } else {
            self.per_page_text = self.per_page.to_string();
            false
        }
    }

    fn set_size(&mut self, n: usize) {
        self.per_page = n;
        self.per_page_text = n.to_string();
        self.reset();
    }

    /// "Rows per page", or "Rows per load" in continuous mode.
    pub fn size_label(&self) -> &'static str {
        if self.continuous {
            "Rows per load"
        } else {
            "Rows per page"
        }
    }

    /// `Record [21 - 40] of 279`; empty when there are no rows.
    pub fn record_info(&self, total: usize) -> String {
        // Continuous: from the first row to the last one shown.
        let (start, end, _) = self.range(total);
        if total == 0 {
            String::new()
        } else {
            format!("Record [{} - {end}] of {total}", start + 1)
        }
    }
}

impl Default for Pager {
    fn default() -> Self {
        Self::new(EMPTY_SIZE)
    }
}

fn digits(s: &str) -> Option<usize> {
    let d: String = s.chars().filter(char::is_ascii_digit).collect();
    d.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_mode_grows_by_rows_per_load() {
        let mut p = Pager::new(20);
        p.go(Go::Next, 970);
        p.toggle();
        assert!(p.continuous && p.mode_set);
        assert_eq!(p.page, 0, "back to the first row");
        assert_eq!(p.range(970), (0, 20, 49));
        assert_eq!(p.record_info(970), "Record [1 - 20] of 970");
        assert_eq!(p.size_label(), "Rows per load");
        assert!(p.more(970));
        assert_eq!(p.record_info(970), "Record [1 - 40] of 970");
        // Near the bottom: more; not near: nothing.
        assert!(p.wants_more(970, true, false));
        assert!(!p.wants_more(970, false, false));
        // Filling a view that has no scroll bar yet stops at 500 rows.
        p.loaded = 500;
        assert!(!p.wants_more(970, true, true));
        assert!(p.wants_more(970, true, false), "scrolling still loads");
        p.loaded = 960;
        assert!(p.more(970));
        assert_eq!(p.loaded, 970);
        assert!(!p.more(970), "nothing left");
        assert!(!p.wants_more(970, true, false));
        // A new filter / sort / size starts again from the first row.
        p.reset();
        assert_eq!(p.loaded, 20);
        p.per_page_text = "50".into();
        assert!(p.apply_per_page());
        assert_eq!(p.range(970), (0, 50, 20));
        p.toggle();
        assert!(!p.continuous);
        assert_eq!(p.size_label(), "Rows per page");
        assert!(!p.wants_more(970, true, true), "pages never load more");
    }

    #[test]
    fn a_narrow_table_starts_continuous_once() {
        let mut p = Pager::new(20);
        p.start_mode(500.0);
        assert!(p.continuous);
        p.toggle();
        p.start_mode(500.0);
        assert!(!p.continuous, "the user's choice stays");
        let mut wide = Pager::new(20);
        wide.start_mode(1200.0);
        assert!(!wide.continuous && wide.mode_set);
    }

    #[test]
    fn pages_buttons_and_record_info() {
        let mut p = Pager::new(20);
        assert_eq!(p.range(279), (0, 20, 14));
        assert_eq!(p.record_info(279), "Record [1 - 20] of 279");
        assert!(!p.go(Go::Prev, 279), "nothing before the first page");
        assert!(p.go(Go::Next, 279));
        assert_eq!(p.record_info(279), "Record [21 - 40] of 279");
        assert!(p.go(Go::Last, 279));
        assert_eq!((p.page, p.page_text.as_str()), (13, "14"));
        assert_eq!(p.record_info(279), "Record [261 - 279] of 279");
        assert!(!p.go(Go::Next, 279), "nothing after the last page");
        assert!(p.go(Go::First, 279));
        p.page_text = "7".into();
        assert!(p.apply_page(279));
        assert_eq!(p.page, 6);
        p.page_text = "999".into();
        p.apply_page(279);
        assert_eq!(p.page_text, "14", "clamped to the last page");
        assert_eq!(p.record_info(0), "");
        assert_eq!(p.range(0), (0, 0, 1));
    }

    #[test]
    fn rows_per_page_as_typed_asks_above_100() {
        let mut p = Pager::new(20);
        p.go(Go::Next, 100);
        p.per_page_text = "50 rows".into();
        assert!(p.apply_per_page());
        assert_eq!(
            (p.per_page, p.page, p.per_page_text.as_str()),
            (50, 0, "50")
        );
        p.per_page_text = "".into();
        assert!(p.apply_per_page());
        assert_eq!(p.per_page, 100, "empty = 100");
        p.per_page_text = "500".into();
        assert!(!p.apply_per_page());
        assert_eq!((p.ask, p.per_page), (Some(500), 100), "asks first");
        assert!(!p.answer(false));
        assert_eq!((p.per_page, p.per_page_text.as_str()), (100, "100"));
        p.per_page_text = "999999".into();
        p.apply_per_page();
        assert!(p.answer(true));
        assert_eq!(p.per_page, MAX_SIZE);
    }
}
