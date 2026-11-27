//! Text of the editor (#64): decoding with line endings and BOM kept, find /
//! replace, go to line, indent / outdent. Positions are character indexes
//! (what the GUI's cursor uses) unless a name says bytes.

use std::ops::Range;

use regex::{NoExpand, Regex, RegexBuilder};

pub use crate::files::charset::Charset;

/// Larger files are refused (download them instead).
pub const MAX_EDIT: u64 = 10 * 1024 * 1024;

/// A file as the editor holds it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Doc {
    /// Text with `\n` line endings.
    pub text: String,
    /// Saved with `\r\n`.
    pub crlf: bool,
    /// Both `\r\n` and `\n` were found (saved with the more common one).
    pub mixed: bool,
    /// Starts with a byte order mark (kept when saving).
    pub bom: bool,
    /// What the bytes were read as (#203).
    pub charset: Charset,
    /// Not valid UTF-8 and no byte order mark: read as [`GUESS`], the usual
    /// legacy encoding, until the user picks the right one.
    pub guessed: bool,
}

/// What a file is written as: line ending and encoding (#203).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub crlf: bool,
    pub charset: Charset,
    /// Write a byte order mark (UTF-8 / UTF-16 only).
    pub bom: bool,
}

impl Default for Format {
    /// New files: LF, UTF-8 without a byte order mark.
    fn default() -> Self {
        Format {
            crlf: false,
            charset: Charset::Utf8,
            bom: false,
        }
    }
}

impl Format {
    /// "UTF-8", "UTF-8 with BOM", "Windows-1252", …
    pub fn encoding_label(self) -> String {
        if self.bom && self.charset == Charset::Utf8 {
            "UTF-8 with BOM".into()
        } else {
            self.charset.name()
        }
    }
}

/// The encoding a file that is not UTF-8 is read as first.
pub const GUESS: Charset = Charset::Other(encoding_rs::WINDOWS_1252);

pub fn too_big(size: u64) -> String {
    format!(
        "{} is more than the editor's 10 MiB: download the file instead",
        crate::db::human_size(size)
    )
}

/// Bytes of a file → text. A byte order mark names the encoding (UTF-8,
/// UTF-16); otherwise UTF-8, and text that is not valid UTF-8 is read as
/// [`GUESS`] (`guessed`). Binary files are refused with the reason.
pub fn decode(bytes: &[u8]) -> Result<Doc, String> {
    decode_with(bytes, None)
}

/// Bytes of a file → text in a chosen encoding (Reopen with Encoding, #203),
/// or detected as [`decode`] does when `charset` is `None`. A byte order mark
/// of the chosen encoding is taken off and remembered.
pub fn decode_with(bytes: &[u8], charset: Option<Charset>) -> Result<Doc, String> {
    if bytes.len() as u64 > MAX_EDIT {
        return Err(too_big(bytes.len() as u64));
    }
    let (cs, bom, body) = match (charset, Charset::sniff(bytes)) {
        (None, Some((c, n))) => (c, true, &bytes[n..]),
        (Some(c), Some((s, n))) if s == c => (c, true, &bytes[n..]),
        (Some(c), _) => (c, false, bytes),
        (None, None) => (Charset::Utf8, false, bytes),
    };
    if !cs.wide() && body.iter().take(8192).any(|&b| b == 0) {
        return Err("Binary file: it cannot be edited here (download it instead)".into());
    }
    let (s, cs, guessed) = match cs.decode(body) {
        Ok(s) => (s, cs, false),
        Err(_) if charset.is_none() && cs == Charset::Utf8 && !bom => {
            (GUESS.decode(body)?, GUESS, true)
        }
        Err(e) => return Err(e),
    };
    let s = s.as_str();
    let crlf_n = s.matches("\r\n").count();
    let lf_n = s.matches('\n').count() - crlf_n;
    Ok(Doc {
        text: if crlf_n > 0 {
            s.replace("\r\n", "\n")
        } else {
            s.to_string()
        },
        crlf: crlf_n > 0 && crlf_n >= lf_n,
        mixed: crlf_n > 0 && lf_n > 0,
        bom,
        charset: cs,
        guessed,
    })
}

/// Text → the file's bytes in `f` (#203); `Err` names the characters the
/// encoding cannot store (they are never replaced).
pub fn encode_as(text: &str, f: Format) -> Result<Vec<u8>, String> {
    let lf = text.replace("\r\n", "\n");
    let body = if f.crlf { lf.replace('\n', "\r\n") } else { lf };
    let bytes = f.charset.encode(&body).map_err(|bad| {
        format!(
            "{} cannot store {}: choose another encoding (Save with Encoding) or change those characters",
            f.charset.name(),
            crate::files::charset::describe_unmappable(&bad)
        )
    })?;
    let mut out = Vec::with_capacity(bytes.len() + 3);
    if f.bom
        && let Some(m) = f.charset.bom()
    {
        out.extend_from_slice(m);
    }
    out.extend_from_slice(&bytes);
    Ok(out)
}

/// Text → bytes with the file's line endings and BOM.
pub fn encode(text: &str, crlf: bool, bom: bool) -> Vec<u8> {
    // Pasted text may bring its own `\r\n`.
    let lf = if text.contains("\r\n") {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    };
    let mut out = Vec::with_capacity(lf.len() + 3);
    if bom {
        out.extend_from_slice(b"\xEF\xBB\xBF");
    }
    if crlf {
        out.extend_from_slice(lf.replace('\n', "\r\n").as_bytes());
    } else {
        out.extend_from_slice(lf.as_bytes());
    }
    out
}

pub fn char_to_byte(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

pub fn byte_to_char(text: &str, bi: usize) -> usize {
    text[..bi.min(text.len())].chars().count()
}

/// 1-based line and column of a character index.
pub fn line_col(text: &str, ci: usize) -> (usize, usize) {
    let (mut line, mut col) = (1, 1);
    for c in text.chars().take(ci) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub fn line_count(text: &str) -> usize {
    text.matches('\n').count() + 1
}

/// Character index where a 1-based line starts (the last line when past the end).
pub fn line_start(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut seen = 1;
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            seen += 1;
            if seen == line {
                return i + 1;
            }
        }
    }
    // Past the end: start of the last line.
    text.rfind('\n').map_or(0, |b| byte_to_char(text, b + 1))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FindOpts {
    pub case: bool,
    pub whole: bool,
    pub regex: bool,
}

fn matcher(query: &str, o: FindOpts) -> Result<Regex, String> {
    let pat = if o.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let pat = if o.whole {
        format!(r"\b(?:{pat})\b")
    } else {
        pat
    };
    RegexBuilder::new(&pat)
        .case_insensitive(!o.case)
        .multi_line(true)
        .size_limit(1 << 22)
        .build()
        .map_err(|e| {
            let last = e.to_string();
            let why = last
                .lines()
                .last()
                .unwrap_or("")
                .trim()
                .trim_start_matches("error: ");
            format!("Invalid regular expression: {why}")
        })
}

/// Byte ranges of every (non-empty) match.
pub fn find_all(text: &str, query: &str, o: FindOpts) -> Result<Vec<Range<usize>>, String> {
    if query.is_empty() {
        return Ok(vec![]);
    }
    let re = matcher(query, o)?;
    Ok(re
        .find_iter(text)
        .filter(|m| !m.is_empty())
        .map(|m| m.range())
        .collect())
}

/// `$1`, `$2`… are group numbers even before a letter or `_` (`$2_$1`), as
/// in VS Code; the regex crate would read `$2_` as a group named `2_`.
fn numbered(with: &str) -> String {
    let mut out = String::with_capacity(with.len() + 4);
    let mut it = with.chars().peekable();
    while let Some(c) = it.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match it.peek() {
            Some('$') => {
                out.push_str("$$");
                it.next();
            }
            Some(d) if d.is_ascii_digit() => {
                out.push_str("${");
                while let Some(d) = it.peek().copied().filter(char::is_ascii_digit) {
                    out.push(d);
                    it.next();
                }
                out.push('}');
            }
            _ => out.push('$'),
        }
    }
    out
}

/// The replacement for the match at byte `at` (`$1`… expand in regex mode).
fn replacement(
    re: &Regex,
    text: &str,
    at: usize,
    with: &str,
    o: FindOpts,
) -> Option<(Range<usize>, String)> {
    let caps = re.captures_at(text, at)?;
    let m = caps.get(0)?;
    if m.start() != at {
        return None;
    }
    let mut out = String::new();
    if o.regex {
        caps.expand(&numbered(with), &mut out);
    } else {
        out.push_str(with);
    }
    Some((m.range(), out))
}

/// Replace the match starting at byte `at`; `None` when nothing matches there.
pub fn replace_at(
    text: &str,
    at: usize,
    query: &str,
    with: &str,
    o: FindOpts,
) -> Result<Option<(String, Range<usize>)>, String> {
    if query.is_empty() {
        return Ok(None);
    }
    let re = matcher(query, o)?;
    Ok(replacement(&re, text, at, with, o).map(|(r, rep)| {
        let s = format!("{}{rep}{}", &text[..r.start], &text[r.end..]);
        (s, r.start..r.start + rep.len())
    }))
}

/// Replace every match; returns the new text and how many were replaced.
pub fn replace_all(
    text: &str,
    query: &str,
    with: &str,
    o: FindOpts,
) -> Result<(String, usize), String> {
    let n = find_all(text, query, o)?.len();
    if n == 0 {
        return Ok((text.to_string(), 0));
    }
    let re = matcher(query, o)?;
    let s = if o.regex {
        re.replace_all(text, numbered(with).as_str())
    } else {
        re.replace_all(text, NoExpand(with))
    };
    Ok((s.into_owned(), n))
}

/// The file's indentation: a tab, or the smallest run of leading spaces (2–8; default 4).
pub fn indent_unit(text: &str) -> String {
    let (mut tabs, mut spaces, mut min) = (0, 0, usize::MAX);
    for l in text.lines().take(5000) {
        if l.starts_with('\t') {
            tabs += 1;
        } else {
            let n = l.len() - l.trim_start_matches(' ').len();
            if n >= 2 && n < l.len() {
                spaces += 1;
                min = min.min(n);
            }
        }
    }
    if tabs > spaces {
        "\t".into()
    } else if spaces > 0 {
        " ".repeat(min.clamp(2, 8))
    } else {
        "    ".into()
    }
}

/// How a tab indents (#182): tabs or spaces, and the width of one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indent {
    pub tabs: bool,
    /// Columns per level (2, 4 or 8; also a tab's width).
    pub size: usize,
}

impl Default for Indent {
    fn default() -> Self {
        Indent {
            tabs: false,
            size: 4,
        }
    }
}

impl Indent {
    pub const SIZES: [usize; 3] = [2, 4, 8];

    /// What Tab inserts.
    pub fn unit(self) -> String {
        if self.tabs {
            "\t".into()
        } else {
            " ".repeat(self.size)
        }
    }

    pub fn label(self) -> String {
        if self.tabs {
            format!("Tab size: {}", self.size)
        } else {
            format!("Spaces: {}", self.size)
        }
    }

    /// The file's indentation, as [`indent_unit`] finds it (tabs: width 4).
    pub fn detect(text: &str) -> Indent {
        let u = indent_unit(text);
        if u == "\t" {
            Indent {
                tabs: true,
                size: 4,
            }
        } else {
            Indent {
                tabs: false,
                size: u.len(),
            }
        }
    }
}

/// Rewrite the leading whitespace of each line from `from` to `to` (#182):
/// tabs ↔ spaces and re-indent between sizes. A line's indentation in
/// columns (tabs to the next multiple of `from.size`) is split into whole
/// levels and a rest; the levels are rescaled to `to.size`, the rest kept as
/// spaces. Only the lines touched by `sel` (character indexes) change when
/// given; text after the first other character is never changed.
pub fn convert_indent(text: &str, sel: Option<Range<usize>>, from: Indent, to: Indent) -> String {
    let (first, last) = match sel {
        Some(r) if r.start != r.end => {
            let a = char_to_byte(text, r.start.min(r.end));
            let b = char_to_byte(text, r.start.max(r.end));
            let end = if text[..b].ends_with('\n') && b > a {
                b - 1
            } else {
                b
            };
            (
                text[..a].matches('\n').count(),
                text[..end].matches('\n').count(),
            )
        }
        _ => (0, usize::MAX),
    };
    let from_size = from.size.max(1);
    let to_size = to.size.max(1);
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if i < first || i > last {
            out.push_str(line);
            continue;
        }
        let lead = line.len() - line.trim_start_matches([' ', '\t']).len();
        let mut cols = 0;
        for c in line[..lead].chars() {
            cols = if c == '\t' {
                (cols / from_size + 1) * from_size
            } else {
                cols + 1
            };
        }
        let (levels, rest) = (cols / from_size, cols % from_size);
        if to.tabs {
            out.push_str(&"\t".repeat(levels));
        } else {
            out.push_str(&" ".repeat(levels * to_size));
        }
        out.push_str(&" ".repeat(rest));
        out.push_str(&line[lead..]);
    }
    out
}

/// Tab / Shift+Tab on the selection `sel` (character indexes, either order).
/// Several lines: each line (not empty ones) is indented or outdented by one
/// unit. One line: Tab inserts the unit at the cursor, Shift+Tab outdents it.
/// Returns the new text and selection.
pub fn indent(text: &str, sel: Range<usize>, unit: &str, outdent: bool) -> (String, Range<usize>) {
    let a = char_to_byte(text, sel.start.min(sel.end));
    let b = char_to_byte(text, sel.start.max(sel.end));
    let multi = text[a..b].contains('\n');
    if !outdent && !multi {
        let s = format!("{}{unit}{}", &text[..a], &text[b..]);
        let c = byte_to_char(&s, a + unit.len());
        return (s, c..c);
    }
    let first = text[..a].rfind('\n').map_or(0, |i| i + 1);
    // A selection that ends at the start of a line does not touch that line.
    let end = if multi && text[..b].ends_with('\n') {
        b - 1
    } else {
        b
    };
    let last = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let width = if unit == "\t" { 4 } else { unit.len() };
    let (mut out, mut first_delta, mut total) = (String::new(), 0isize, 0isize);
    for (i, line) in text[first..last].split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let delta = if outdent {
            let n = if line.starts_with('\t') {
                1
            } else {
                line.bytes().take(width).take_while(|c| *c == b' ').count()
            };
            out.push_str(&line[n..]);
            -(n as isize)
        } else if line.is_empty() {
            0
        } else {
            out.push_str(unit);
            out.push_str(line);
            unit.len() as isize
        };
        if i == 0 {
            first_delta = delta;
        }
        total += delta;
    }
    let s = format!("{}{out}{}", &text[..first], &text[last..]);
    let na = (a as isize + first_delta).max(first as isize) as usize;
    let nb = (b as isize + total).max(na as isize) as usize;
    let r = byte_to_char(&s, na)..byte_to_char(&s, nb);
    (s, r)
}

#[cfg(test)]
mod tests {
    #[test]
    fn indentation_converts_and_reindents() {
        // #182
        let sp = |size| Indent { tabs: false, size };
        let tab = |size| Indent { tabs: true, size };
        let four = "a:\n    b: 1\n        c: 2\n      d\n";
        assert_eq!(Indent::detect(four), sp(4));
        assert_eq!(Indent::detect("x\n\ty\n"), tab(4));
        // 4 → 2 and back; 6 columns = 1 level + 2 kept.
        let two = convert_indent(four, None, sp(4), sp(2));
        assert_eq!(two, "a:\n  b: 1\n    c: 2\n    d\n");
        assert_eq!(
            convert_indent("  b\n    c\n", None, sp(2), sp(4)),
            "    b\n        c\n"
        );
        // Spaces → tabs (a partial run stays spaces) and back.
        let t = convert_indent(four, None, sp(4), tab(4));
        assert_eq!(t, "a:\n\tb: 1\n\t\tc: 2\n\t  d\n");
        assert_eq!(convert_indent(&t, None, tab(4), sp(4)), four);
        // Mixed indentation, CRLF kept, text after the indent untouched.
        assert_eq!(
            convert_indent("\t  x\t y\r\n  \tz\r\n", None, tab(4), sp(2)),
            "    x\t y\r\n  z\r\n"
        );
        // Only the selected lines (the line after a trailing newline is not touched).
        let sel = 3..12; // "    b: 1\n"
        assert_eq!(
            convert_indent(four, Some(sel), sp(4), sp(2)),
            "a:\n  b: 1\n        c: 2\n      d\n"
        );
        // Empty selection = whole text; labels and units.
        assert_eq!(convert_indent(four, Some(5..5), sp(4), sp(2)), two);
        assert_eq!(sp(2).label(), "Spaces: 2");
        assert_eq!(tab(4).label(), "Tab size: 4");
        assert_eq!(tab(8).unit(), "\t");
        assert_eq!(sp(2).unit(), "  ");
    }

    use super::*;

    #[test]
    fn line_endings_bom_and_final_newline_round_trip() {
        for bytes in [
            &b"a\nb\n"[..],
            b"a\r\nb\r\n",
            b"a\r\nb",
            b"\xEF\xBB\xBFkey=v\r\n",
            b"no newline",
            b"",
            "ünïcode 'x'\n".as_bytes(),
        ] {
            let d = decode(bytes).unwrap();
            assert!(!d.text.contains('\r'), "{d:?}");
            assert_eq!(encode(&d.text, d.crlf, d.bom), bytes, "{d:?}");
        }
        let d = decode(b"\xEF\xBB\xBFx\r\n").unwrap();
        assert!(d.bom && d.crlf && !d.mixed);
        assert_eq!(d.text, "x\n");
    }

    #[test]
    fn mixed_endings_use_the_more_common_one() {
        let d = decode(b"a\r\nb\r\nc\n").unwrap();
        assert!(d.crlf && d.mixed);
        assert_eq!(encode(&d.text, d.crlf, d.bom), b"a\r\nb\r\nc\r\n");
        let d = decode(b"a\nb\nc\r\n").unwrap();
        assert!(!d.crlf && d.mixed);
    }

    #[test]
    fn pasted_crlf_is_saved_with_the_files_endings() {
        assert_eq!(encode("a\r\nb\n", false, false), b"a\nb\n");
        assert_eq!(encode("a\r\nb\n", true, false), b"a\r\nb\r\n");
    }

    #[test]
    fn binary_and_big_files_are_refused() {
        assert!(
            decode(b"PK\x03\x04\x00\x00")
                .unwrap_err()
                .contains("Binary")
        );
        let big = vec![b'a'; MAX_EDIT as usize + 1];
        assert!(decode(&big).unwrap_err().contains("10 MiB"));
    }

    #[test]
    fn encodings_are_detected_guessed_and_chosen() {
        // Not UTF-8: opened as Windows-1252, marked as a guess (#203).
        let d = decode(b"caf\xe9\r\n").unwrap();
        assert_eq!(
            (d.text.as_str(), d.charset, d.guessed, d.crlf),
            ("café\n", GUESS, true, true)
        );
        // UTF-16 with a byte order mark: NUL bytes are not "binary".
        let d = decode(b"\xFF\xFEh\0i\0\n\0").unwrap();
        assert_eq!(
            (d.text.as_str(), d.charset, d.bom),
            ("hi\n", Charset::Utf16Le, true)
        );
        // Reopen with a chosen encoding.
        let d = decode_with(b"caf\xe9", Some(Charset::Latin1)).unwrap();
        assert_eq!((d.text.as_str(), d.guessed), ("café", false));
        assert!(
            decode_with(b"caf\xe9", Some(Charset::Utf8))
                .unwrap_err()
                .contains("UTF-8")
        );
        let d = decode_with(b"\xEF\xBB\xBFok", Some(Charset::Utf8)).unwrap();
        assert!(d.bom && d.text == "ok");
    }

    #[test]
    fn encode_as_writes_line_endings_encoding_and_bom() {
        let f = |crlf, charset, bom| Format { crlf, charset, bom };
        assert_eq!(encode_as("a\nb", Format::default()).unwrap(), b"a\nb");
        assert_eq!(
            encode_as("a\r\nb", f(false, Charset::Utf8, false)).unwrap(),
            b"a\nb"
        );
        assert_eq!(
            encode_as("a\nb", f(true, Charset::Utf8, true)).unwrap(),
            b"\xEF\xBB\xBFa\r\nb"
        );
        assert_eq!(
            encode_as("é\n", f(false, Charset::Utf16Be, true)).unwrap(),
            b"\xFE\xFF\x00\xE9\x00\n"
        );
        assert_eq!(encode_as("é", f(false, GUESS, false)).unwrap(), b"\xE9");
        let err = encode_as("ok\n€ ✓", f(false, Charset::Latin1, false)).unwrap_err();
        assert!(
            err.contains("ISO-8859-1 cannot store line 2, column 1 '€'; line 2, column 3 '✓'"),
            "{err}"
        );
        assert_eq!(Format::default().encoding_label(), "UTF-8");
        assert_eq!(
            f(false, Charset::Utf8, true).encoding_label(),
            "UTF-8 with BOM"
        );
    }

    #[test]
    fn lines_and_columns() {
        let t = "ab\nçd\n\nlast";
        assert_eq!(line_col(t, 0), (1, 1));
        assert_eq!(line_col(t, 4), (2, 2));
        assert_eq!(line_col(t, 7), (4, 1));
        assert_eq!(line_count(t), 4);
        assert_eq!(line_start(t, 2), 3);
        assert_eq!(line_start(t, 4), 7);
        assert_eq!(line_start(t, 99), 7);
        assert_eq!(line_start(t, 0), 0);
    }

    #[test]
    fn find_with_case_whole_word_and_regex() {
        let t = "Port 80\nport 8080\nexport=1";
        let plain = FindOpts::default();
        assert_eq!(find_all(t, "port", plain).unwrap().len(), 3);
        let case = FindOpts {
            case: true,
            ..plain
        };
        assert_eq!(find_all(t, "port", case).unwrap(), vec![8..12, 20..24]);
        let whole = FindOpts {
            whole: true,
            ..plain
        };
        assert_eq!(find_all(t, "port", whole).unwrap().len(), 2);
        let re = FindOpts {
            regex: true,
            ..plain
        };
        assert_eq!(find_all(t, r"^\w+ \d+$", re).unwrap().len(), 2);
        assert!(
            find_all(t, "(", re)
                .unwrap_err()
                .contains("Invalid regular expression")
        );
        assert!(find_all(t, "a.c", plain).unwrap().is_empty(), "not a regex");
        assert!(find_all(t, "", plain).unwrap().is_empty());
    }

    #[test]
    fn replace_one_and_all() {
        let t = "listen 80;\nlisten 8080;";
        let re = FindOpts {
            regex: true,
            ..Default::default()
        };
        let (s, n) = replace_all(t, r"listen (\d+)", "listen [::]:$1", re).unwrap();
        assert_eq!((s.as_str(), n), ("listen [::]:80;\nlisten [::]:8080;", 2));
        let (s, _) = replace_all("APP_ENV: x", r"([A-Z]+)_([A-Z]+):", "$2_$1:", re).unwrap();
        assert_eq!(s, "ENV_APP: x", "$2_ is group 2, then _");
        let (s, _) = replace_all("a1", r"(\w)(\d)", "$$${1}-$2", re).unwrap();
        assert_eq!(s, "$a-1");
        let lit = FindOpts::default();
        let (s, n) = replace_all("a.b a.b", "a.b", "$1", lit).unwrap();
        assert_eq!((s.as_str(), n), ("$1 $1", 2), "plain mode never expands");
        let (s, r) = replace_at(t, 11, "listen", "port", lit).unwrap().unwrap();
        assert_eq!(s, "listen 80;\nport 8080;");
        assert_eq!(r, 11..15);
        assert!(replace_at(t, 3, "listen", "x", lit).unwrap().is_none());
    }

    #[test]
    fn indent_and_outdent() {
        assert_eq!(indent_unit("a:\n  b: 1\n    c: 2\n"), "  ");
        assert_eq!(indent_unit("fn x() {\n\tlet a;\n}\n"), "\t");
        assert_eq!(indent_unit("plain\n"), "    ");
        // Tab without a selection inserts at the cursor.
        assert_eq!(indent("ab", 1..1, "  ", false), ("a  b".into(), 3..3));
        // Several lines: every non-empty line, the selection follows.
        let (s, r) = indent("a\n\nb\nc", 0..4, "  ", false);
        assert_eq!(s, "  a\n\n  b\nc");
        assert_eq!(r, 2..8);
        // Shift+Tab removes one unit (or a tab) from each line.
        let (s, _) = indent("    a\n\tb\n c", 0..10, "    ", true);
        assert_eq!(s, "a\nb\nc");
        // Outdent on one line works with just a cursor.
        let (s, r) = indent("x\n    y", 4..4, "    ", true);
        assert_eq!((s.as_str(), r), ("x\ny", 2..2));
        // A selection ending at a line start leaves that line alone.
        let (s, _) = indent("a\nb\n", 0..2, "\t", false);
        assert_eq!(s, "\ta\nb\n");
    }
}
