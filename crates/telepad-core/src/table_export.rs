//! Table exports (#153), as the wtbapp table menu (`y_table_menu`, #341):
//! the rows a table shows (whole filtered and sorted list, every page) as
//! CSV, Excel, PDF or JSON. Pure: bytes in memory, written by [`write`]
//! through `<name>.telepad-part` and a rename.

use std::path::Path;

/// A table as text: what the user sees in each exported column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableData {
    /// Shown on top of the PDF and used for the file name.
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Xlsx,
    Pdf,
    /// To the clipboard, not a file.
    Json,
}

/// A PDF with more rows than this asks first (wtbapp: 5000).
pub const PDF_ASK_ABOVE: usize = 5000;

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Xlsx => "xlsx",
            Format::Pdf => "pdf",
            Format::Json => "json",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Xlsx => "Excel workbook",
            Format::Pdf => "PDF",
            Format::Json => "JSON",
        }
    }
}

/// `<title>-YYYYMMDD-HHMM.<ext>` (UTC), the title cut down to letters,
/// digits and `-` so it is a safe file name everywhere.
pub fn file_name(title: &str, unix_secs: i64, f: Format) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(60).collect();
    let slug = if slug.is_empty() {
        "table".into()
    } else {
        slug
    };
    let t = crate::history::format_utc(unix_secs);
    // "2026-11-25 14:30:05" → "20261125-1430"
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    let stamp = if digits.len() >= 12 {
        format!("{}-{}", &digits[..8], &digits[8..12])
    } else {
        digits
    };
    format!("{slug}-{stamp}.{}", f.ext())
}

/// A text cell a spreadsheet would run as a formula (#111) gets a leading
/// `'`; numbers (also negative ones) stay as they are.
fn defuse(s: &str) -> std::borrow::Cow<'_, str> {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) && s.trim().parse::<f64>().is_err() {
        format!("'{s}").into()
    } else {
        s.into()
    }
}

/// CSV: UTF-8 with BOM (Excel reads the characters right), RFC 4180
/// quoting, formulas defused.
pub fn to_csv(t: &TableData) -> String {
    let q = |s: &str| {
        let s = defuse(s);
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.into_owned()
        }
    };
    let line = |cells: &[String]| cells.iter().map(|c| q(c)).collect::<Vec<_>>().join(",");
    let mut out = String::from('\u{feff}');
    out.push_str(&line(&t.columns));
    out.push_str("\r\n");
    for r in &t.rows {
        out.push_str(&line(r));
        out.push_str("\r\n");
    }
    out
}

/// Column names as JSON keys: a repeated name gets " (2)", " (3)", …
fn keys(columns: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for c in columns {
        let base = if c.trim().is_empty() {
            format!("column {}", seen.len() + 1)
        } else {
            c.clone()
        };
        let mut k = base.clone();
        let mut n = 2;
        while seen.contains(&k) {
            k = format!("{base} ({n})");
            n += 1;
        }
        seen.push(k);
    }
    seen
}

/// JSON: an array of objects keyed by column name, in column order, every
/// value a string as shown in the table.
pub fn to_json(t: &TableData) -> String {
    let keys = keys(&t.columns);
    let s = |v: &str| serde_json::Value::String(v.to_string()).to_string();
    let mut out = String::from("[");
    for (i, r) in t.rows.iter().enumerate() {
        out.push_str(if i == 0 { "\n  {" } else { ",\n  {" });
        for (c, k) in keys.iter().enumerate() {
            if c > 0 {
                out.push_str(", ");
            }
            out.push_str(&s(k));
            out.push_str(": ");
            out.push_str(&s(r.get(c).map_or("", String::as_str)));
        }
        out.push('}');
    }
    out.push_str(if t.rows.is_empty() { "]" } else { "\n]" });
    out
}

/// A cell Excel may store as a number: plain decimal numbers only, so
/// versions, addresses, ports with leading zeros and ids stay text.
fn as_number(s: &str) -> Option<f64> {
    let t = s.trim();
    let ok = !t.is_empty()
        && t.len() < 16
        && t.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        && t.matches('.').count() <= 1
        && !t[1..].contains('-')
        && !(t.len() > 1 && t.starts_with('0') && !t.starts_with("0."))
        && !(t.len() > 2 && t.starts_with("-0") && !t.starts_with("-0."));
    if ok { t.parse().ok() } else { None }
}

/// Excel workbook: bold, frozen header with an autofilter; plain numbers
/// as numbers, the rest as text (formulas never: text is written as text).
pub fn to_xlsx(t: &TableData) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{Format as XFormat, Workbook};
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    let name: String = t
        .title
        .chars()
        .filter(|c| !"[]:*?/\\".contains(*c))
        .take(31)
        .collect();
    if !name.trim().is_empty() {
        ws.set_name(name.trim()).map_err(|e| e.to_string())?;
    }
    let bold = XFormat::new().set_bold();
    let e = |r: Result<&mut rust_xlsxwriter::Worksheet, rust_xlsxwriter::XlsxError>| {
        r.map(|_| ()).map_err(|e| e.to_string())
    };
    let mut widths: Vec<usize> = t.columns.iter().map(|c| c.chars().count()).collect();
    for (c, h) in t.columns.iter().enumerate() {
        e(ws.write_string_with_format(0, c as u16, h, &bold))?;
    }
    for (r, row) in t.rows.iter().enumerate() {
        let r = r as u32 + 1;
        for (c, v) in row.iter().enumerate().take(t.columns.len()) {
            if let Some(w) = widths.get_mut(c) {
                *w = (*w).max(v.chars().count());
            }
            match as_number(v) {
                Some(n) => e(ws.write_number(r, c as u16, n))?,
                None if v.is_empty() => {}
                None => e(ws.write_string(r, c as u16, v))?,
            }
        }
    }
    for (c, w) in widths.iter().enumerate() {
        ws.set_column_width(c as u16, (*w as f64 + 2.0).clamp(6.0, 60.0))
            .map_err(|e| e.to_string())?;
    }
    if !t.columns.is_empty() {
        ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
        ws.autofilter(0, 0, t.rows.len() as u32, t.columns.len() as u16 - 1)
            .map_err(|e| e.to_string())?;
    }
    wb.save_to_buffer().map_err(|e| e.to_string())
}

/// Writes `bytes` to `<path>.telepad-part`, then renames it over `path`, so a
/// failed write never leaves half a file under the chosen name.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut part = path.as_os_str().to_owned();
    part.push(".telepad-part");
    let part = std::path::PathBuf::from(part);
    std::fs::write(&part, bytes)
        .and_then(|()| std::fs::rename(&part, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&part);
            format!("Cannot write {}: {e}", path.display())
        })
}

/// The bytes of a file export (not [`Format::Json`]). `fonts`: TrueType
/// fonts for the PDF, tried in order for each character.
pub fn render(
    t: &TableData,
    f: Format,
    fonts: &[&[u8]],
    generated: &str,
) -> Result<Vec<u8>, String> {
    match f {
        Format::Csv => Ok(to_csv(t).into_bytes()),
        Format::Xlsx => to_xlsx(t),
        Format::Pdf => pdf::to_pdf(t, fonts, generated),
        Format::Json => Ok(to_json(t).into_bytes()),
    }
}

pub mod pdf {
    //! A table as PDF: A4 (landscape when wide), the header on every page,
    //! striped rows, `Page n of N`. Fonts are embedded whole (TrueType,
    //! Identity-H) with a ToUnicode map, so text can be copied and searched;
    //! each character takes the first font that has it.

    use super::TableData;
    use pdf_writer::types::{CidFontType, FontFlags, SystemInfo, UnicodeCmap};
    use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str};
    use std::collections::BTreeMap;

    const MARGIN: f32 = 32.0;
    const BODY: f32 = 7.5;
    const HEAD: f32 = 8.0;
    const TITLE: f32 = 12.0;
    const ROW_H: f32 = 11.0;
    const PAD: f32 = 3.0;
    /// A column is never wider than this (points) before scaling.
    const COL_MAX: f32 = 220.0;
    const COL_MIN: f32 = 24.0;

    struct Font<'a> {
        face: ttf_parser::Face<'a>,
        data: &'a [u8],
        scale: f32,
        used: BTreeMap<u16, char>,
    }

    impl Font<'_> {
        fn advance(&self, g: u16) -> f32 {
            self.face
                .glyph_hor_advance(ttf_parser::GlyphId(g))
                .unwrap_or(0) as f32
                * self.scale
        }
    }

    /// Text runs: (font index, glyph ids) — a character no font has is
    /// dropped.
    fn shape(fonts: &[Font], s: &str) -> Vec<(usize, Vec<u16>, Vec<char>)> {
        let mut runs: Vec<(usize, Vec<u16>, Vec<char>)> = Vec::new();
        for c in s.chars() {
            let c = if c.is_control() { ' ' } else { c };
            let Some((fi, g)) = fonts
                .iter()
                .enumerate()
                .find_map(|(i, f)| f.face.glyph_index(c).map(|g| (i, g.0)))
            else {
                continue;
            };
            match runs.last_mut() {
                Some((f, gs, cs)) if *f == fi => {
                    gs.push(g);
                    cs.push(c);
                }
                _ => runs.push((fi, vec![g], vec![c])),
            }
        }
        runs
    }

    fn width(fonts: &[Font], s: &str, size: f32) -> f32 {
        shape(fonts, s)
            .iter()
            .map(|(f, gs, _)| gs.iter().map(|g| fonts[*f].advance(*g)).sum::<f32>())
            .sum::<f32>()
            * size
            / 1000.0
    }

    /// `s` cut with "…" to fit `max` points.
    fn fit(fonts: &[Font], s: &str, size: f32, max: f32) -> String {
        if width(fonts, s, size) <= max {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let t: String = chars.iter().collect::<String>() + "…";
            if width(fonts, &t, size) <= max {
                return t;
            }
        }
        String::new()
    }

    struct Writer<'f, 'a> {
        fonts: &'f mut [Font<'a>],
        content: Content,
    }

    impl Writer<'_, '_> {
        fn text(&mut self, x: f32, y: f32, size: f32, s: &str) {
            let runs = shape(self.fonts, s);
            let mut x = x;
            for (fi, gs, cs) in runs {
                let f = &mut self.fonts[fi];
                let mut bytes = Vec::with_capacity(gs.len() * 2);
                let mut w = 0.0;
                for (g, c) in gs.iter().zip(&cs) {
                    bytes.extend(g.to_be_bytes());
                    f.used.entry(*g).or_insert(*c);
                    w += f.advance(*g);
                }
                let name = format!("F{fi}");
                self.content.begin_text();
                self.content.set_font(Name(name.as_bytes()), size);
                self.content.next_line(x, y);
                self.content.show(Str(&bytes));
                self.content.end_text();
                x += w * size / 1000.0;
            }
        }

        fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, gray: f32) {
            self.content.set_fill_gray(gray);
            self.content.rect(x, y, w, h);
            self.content.fill_nonzero();
            self.content.set_fill_gray(0.0);
        }
    }

    fn deflate(data: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
    }

    /// Widths that fit `avail`: the room above each column's least width
    /// shrinks in proportion (long text gives way, short columns keep their
    /// titles); when even the least widths do not fit, all scale down.
    pub fn fit_columns(want: &[f32], least: &[f32], avail: f32) -> Vec<f32> {
        let total: f32 = want.iter().sum();
        if total <= avail {
            return want.to_vec();
        }
        let floor: f32 = least.iter().sum();
        if floor >= avail {
            let k = avail / floor.max(1.0);
            return least.iter().map(|w| w * k).collect();
        }
        let room = total - floor;
        let k = (avail - floor) / room;
        want.iter()
            .zip(least)
            .map(|(w, l)| l + (w - l) * k)
            .collect()
    }

    pub fn to_pdf(t: &TableData, fonts: &[&[u8]], generated: &str) -> Result<Vec<u8>, String> {
        let mut fonts: Vec<Font> = fonts
            .iter()
            .filter_map(|data| {
                let face = ttf_parser::Face::parse(data, 0).ok()?;
                // Embedded as FontFile2: TrueType outlines only.
                face.tables().glyf?;
                let scale = 1000.0 / face.units_per_em() as f32;
                Some(Font {
                    face,
                    data,
                    scale,
                    used: BTreeMap::new(),
                })
            })
            .collect();
        if fonts.is_empty() {
            return Err("no font for the PDF".into());
        }
        let n = t.columns.len().max(1);
        // Natural widths: the header or the widest value, capped.
        // Natural widths (the header or the widest value, capped) and the
        // least each column keeps: its header, so no title is cut to "…".
        let mut want = Vec::with_capacity(n);
        let mut least = Vec::with_capacity(n);
        for c in 0..n {
            let h = t.columns.get(c).map_or(0.0, |s| width(&fonts, s, HEAD)) + 2.0 * PAD;
            let body = t
                .rows
                .iter()
                .take(2000)
                .map(|r| r.get(c).map_or(0.0, |s| width(&fonts, s, BODY)))
                .fold(0.0, f32::max)
                + 2.0 * PAD;
            let w = h.max(body).clamp(COL_MIN, COL_MAX);
            want.push(w);
            least.push(h.clamp(COL_MIN, w));
        }
        let total: f32 = want.iter().sum();
        let (pw, ph) = if total > 595.0 - 2.0 * MARGIN {
            (842.0, 595.0)
        } else {
            (595.0, 842.0)
        };
        let avail = pw - 2.0 * MARGIN;
        want = fit_columns(&want, &least, avail);
        let top = ph - MARGIN;
        let table_top = top - TITLE - 10.0;
        let bottom = MARGIN + 14.0;
        let per_page = (((table_top - ROW_H) - bottom) / ROW_H).floor().max(1.0) as usize;
        let pages = t.rows.len().div_ceil(per_page).max(1);

        let mut pdf = Pdf::new();
        let mut next = 1;
        let mut new_ref = || {
            next += 1;
            Ref::new(next)
        };
        let catalog = new_ref();
        let tree = new_ref();
        let font_refs: Vec<(Ref, Ref, Ref, Ref, Ref)> = fonts
            .iter()
            .map(|_| (new_ref(), new_ref(), new_ref(), new_ref(), new_ref()))
            .collect();
        let page_refs: Vec<(Ref, Ref)> = (0..pages).map(|_| (new_ref(), new_ref())).collect();
        let info = new_ref();

        let mut streams: Vec<Vec<u8>> = Vec::with_capacity(pages);
        for p in 0..pages {
            let mut w = Writer {
                fonts: &mut fonts,
                content: Content::new(),
            };
            w.text(MARGIN, top - TITLE, TITLE, &t.title);
            let mut y = table_top;
            let row = |w: &mut Writer, y: f32, cells: &[String], size: f32| {
                let mut x = MARGIN;
                for (c, cw) in want.iter().enumerate() {
                    let s = cells.get(c).map_or("", String::as_str);
                    let s = fit(w.fonts, s, size, cw - 2.0 * PAD);
                    w.text(x + PAD, y + 3.0, size, &s);
                    x += cw;
                }
            };
            w.fill(MARGIN, y - ROW_H, avail.min(want.iter().sum()), ROW_H, 0.85);
            row(&mut w, y - ROW_H, &t.columns, HEAD);
            y -= ROW_H;
            let start = p * per_page;
            for (i, r) in t.rows.iter().skip(start).take(per_page).enumerate() {
                if i % 2 == 1 {
                    w.fill(MARGIN, y - ROW_H, avail.min(want.iter().sum()), ROW_H, 0.95);
                }
                row(&mut w, y - ROW_H, r, BODY);
                y -= ROW_H;
            }
            let foot = format!("{} rows · {generated}", t.rows.len());
            w.text(MARGIN, MARGIN, BODY, &foot);
            let page = format!("Page {} of {pages}", p + 1);
            let pwid = width(w.fonts, &page, BODY);
            w.text(pw - MARGIN - pwid, MARGIN, BODY, &page);
            streams.push(w.content.finish().into_vec());
        }

        pdf.catalog(catalog).pages(tree);
        pdf.pages(tree)
            .kids(page_refs.iter().map(|(p, _)| *p))
            .count(pages as i32);
        for (i, (page_ref, content_ref)) in page_refs.iter().enumerate() {
            let mut page = pdf.page(*page_ref);
            page.media_box(Rect::new(0.0, 0.0, pw, ph));
            page.parent(tree);
            page.contents(*content_ref);
            let mut res = page.resources();
            let mut fd = res.fonts();
            for (fi, r) in font_refs.iter().enumerate() {
                fd.pair(Name(format!("F{fi}").as_bytes()), r.0);
            }
            fd.finish();
            res.finish();
            page.finish();
            let z = deflate(&streams[i]);
            pdf.stream(*content_ref, &z).filter(Filter::FlateDecode);
        }
        let sys = SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"Identity"),
            supplement: 0,
        };
        for (fi, f) in fonts.iter().enumerate() {
            let (type0, cid, desc, file, cmap) = font_refs[fi];
            let base = format!("TelepadFont{fi}");
            pdf.type0_font(type0)
                .base_font(Name(base.as_bytes()))
                .encoding_predefined(Name(b"Identity-H"))
                .descendant_font(cid)
                .to_unicode(cmap);
            let mut c = pdf.cid_font(cid);
            c.subtype(CidFontType::Type2);
            c.base_font(Name(base.as_bytes()));
            c.system_info(sys);
            c.font_descriptor(desc);
            c.default_width(0.0);
            c.cid_to_gid_map_predefined(Name(b"Identity"));
            {
                let mut ws = c.widths();
                for g in f.used.keys() {
                    ws.consecutive(*g, [f.advance(*g)]);
                }
            }
            c.finish();
            let bb = f.face.global_bounding_box();
            let s = f.scale;
            pdf.font_descriptor(desc)
                .name(Name(base.as_bytes()))
                .flags(FontFlags::NON_SYMBOLIC)
                .bbox(Rect::new(
                    bb.x_min as f32 * s,
                    bb.y_min as f32 * s,
                    bb.x_max as f32 * s,
                    bb.y_max as f32 * s,
                ))
                .italic_angle(0.0)
                .ascent(f.face.ascender() as f32 * s)
                .descent(f.face.descender() as f32 * s)
                .cap_height(f.face.capital_height().unwrap_or(f.face.ascender()) as f32 * s)
                .stem_v(80.0)
                .font_file2(file);
            let z = deflate(f.data);
            pdf.stream(file, &z)
                .filter(Filter::FlateDecode)
                .pair(Name(b"Length1"), f.data.len() as i32);
            let mut map = UnicodeCmap::new(Name(b"Custom"), sys);
            for (g, ch) in &f.used {
                map.pair(*g, *ch);
            }
            let map = map.finish();
            pdf.cmap(cmap, &map);
        }
        pdf.document_info(info)
            .title(pdf_writer::TextStr(&t.title))
            .producer(pdf_writer::TextStr("Telepad"));
        Ok(pdf.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TableData {
        TableData {
            title: "Ports on web-01".into(),
            columns: vec![
                "Port".into(),
                "Address".into(),
                "Note".into(),
                "Note".into(),
            ],
            rows: vec![
                vec![
                    "22".into(),
                    "192.0.2.10".into(),
                    "=1+1".into(),
                    "a, \"b\"".into(),
                ],
                vec![
                    "0443".into(),
                    "198.51.100.7".into(),
                    "-5".into(),
                    "→ ok".into(),
                ],
            ],
        }
    }

    #[test]
    fn csv_has_a_bom_quotes_and_defused_formulas() {
        let s = to_csv(&sample());
        assert!(s.starts_with('\u{feff}'));
        let lines: Vec<&str> = s.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "Port,Address,Note,Note");
        assert_eq!(lines[1], "22,192.0.2.10,'=1+1,\"a, \"\"b\"\"\"");
        // A negative number stays a number.
        assert_eq!(lines[2], "0443,198.51.100.7,-5,→ ok");
    }

    #[test]
    fn json_keeps_column_order_and_names_repeats() {
        let j = to_json(&sample());
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v[0]["Port"], "22");
        assert_eq!(v[0]["Note (2)"], "a, \"b\"");
        assert_eq!(v[1]["Note"], "-5");
        assert!(j.find("\"Port\"").unwrap() < j.find("\"Address\"").unwrap());
        assert_eq!(to_json(&TableData::default()), "[]");
    }

    #[test]
    fn numbers_for_excel_are_plain_decimals_only() {
        assert_eq!(as_number("22"), Some(22.0));
        assert_eq!(as_number("-1.5"), Some(-1.5));
        assert_eq!(as_number("0.25"), Some(0.25));
        for text in [
            "0443",
            "192.0.2.10",
            "1-2",
            "",
            "12 GiB",
            "1e5",
            "12345678901234567",
        ] {
            assert_eq!(as_number(text), None, "{text}");
        }
    }

    #[test]
    fn xlsx_is_a_zip_workbook() {
        let b = to_xlsx(&sample()).unwrap();
        assert_eq!(&b[..2], b"PK");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(b)).unwrap();
        let mut sheet = String::new();
        std::io::Read::read_to_string(
            &mut z.by_name("xl/worksheets/sheet1.xml").unwrap(),
            &mut sheet,
        )
        .unwrap();
        assert!(sheet.contains("autoFilter"), "{sheet}");
        // "=1+1" is text, never a formula.
        assert!(!sheet.contains("<f>"), "{sheet}");
    }

    #[test]
    fn file_names_are_safe_and_dated() {
        // 2026-11-25 14:30:05 UTC
        let t = 1_795_617_005;
        assert_eq!(
            file_name("Ports on web-01", t, Format::Csv),
            format!("ports-on-web-01-{}.csv", {
                let d: String = crate::history::format_utc(t)
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                format!("{}-{}", &d[..8], &d[8..12])
            })
        );
        assert!(file_name("../../x", t, Format::Pdf).starts_with("x-"));
        assert!(file_name("", t, Format::Xlsx).starts_with("table-"));
    }

    #[test]
    fn write_replaces_through_a_part_file() {
        let dir = std::env::temp_dir().join(format!("telepad-te-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("out.csv");
        write(&p, b"a").unwrap();
        write(&p, b"b").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"b");
        assert!(!dir.join("out.csv.telepad-part").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A small TrueType font from the system or the test fonts, if any.
    fn test_font() -> Option<Vec<u8>> {
        let candidates = [
            "C:\\Windows\\Fonts\\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial.ttf",
        ];
        candidates.iter().find_map(|p| std::fs::read(p).ok())
    }

    #[test]
    fn pdf_has_pages_fonts_and_text() {
        let Some(font) = test_font() else {
            eprintln!("no system TrueType font: PDF test skipped");
            return;
        };
        let mut t = sample();
        for i in 0..150 {
            t.rows.push(vec![
                i.to_string(),
                "192.0.2.1".into(),
                "x".into(),
                "y".into(),
            ]);
        }
        let b = render(&t, Format::Pdf, &[&font], "2026-11-25 14:30 UTC").unwrap();
        assert!(b.starts_with(b"%PDF-"));
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("/Identity-H"));
        assert!(s.contains("/ToUnicode"));
        assert!(s.contains("/Count 3"), "152 rows on 3 pages");
        assert!(pdf::to_pdf(&t, &[b"not a font"], "").is_err());
    }

    #[test]
    fn pdf_columns_shrink_long_text_first() {
        // Two short columns keep their titles; the long one gives way.
        let w = pdf::fit_columns(&[40.0, 400.0, 50.0], &[40.0, 60.0, 50.0], 300.0);
        assert_eq!((w[0], w[2]), (40.0, 50.0));
        assert!((w.iter().sum::<f32>() - 300.0).abs() < 0.01);
        // Fits already: unchanged. Far too narrow: everything scales.
        assert_eq!(
            pdf::fit_columns(&[10.0, 20.0], &[10.0, 20.0], 100.0),
            vec![10.0, 20.0]
        );
        let w = pdf::fit_columns(&[100.0, 100.0], &[80.0, 120.0], 100.0);
        assert!((w.iter().sum::<f32>() - 100.0).abs() < 0.01 && w[0] < w[1]);
    }
}
