//! Text encodings of the editor (#203): which bytes a file's text is read
//! from and written as. UTF-8, UTF-16 and ISO-8859-1 are done here (the
//! WHATWG tables of `encoding_rs` treat ISO-8859-1 as Windows-1252 and have
//! no UTF-16 encoder); every other encoding goes through `encoding_rs`.

use std::collections::HashMap;

use encoding_rs::Encoding;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Charset {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
    /// ISO-8859-1 proper: byte n is U+00n.
    Latin1,
    Other(&'static Encoding),
}

/// The other encodings offered, grouped roughly by region.
const OTHERS: [&Encoding; 36] = [
    encoding_rs::WINDOWS_1252,
    encoding_rs::ISO_8859_15,
    encoding_rs::WINDOWS_1250,
    encoding_rs::ISO_8859_2,
    encoding_rs::WINDOWS_1251,
    encoding_rs::ISO_8859_5,
    encoding_rs::KOI8_R,
    encoding_rs::KOI8_U,
    encoding_rs::IBM866,
    encoding_rs::X_MAC_CYRILLIC,
    encoding_rs::WINDOWS_1253,
    encoding_rs::ISO_8859_7,
    encoding_rs::WINDOWS_1254,
    encoding_rs::ISO_8859_3,
    encoding_rs::ISO_8859_4,
    encoding_rs::ISO_8859_10,
    encoding_rs::ISO_8859_13,
    encoding_rs::ISO_8859_14,
    encoding_rs::ISO_8859_16,
    encoding_rs::WINDOWS_1257,
    encoding_rs::WINDOWS_1258,
    encoding_rs::WINDOWS_1255,
    encoding_rs::ISO_8859_8,
    encoding_rs::ISO_8859_8_I,
    encoding_rs::WINDOWS_1256,
    encoding_rs::ISO_8859_6,
    encoding_rs::WINDOWS_874,
    encoding_rs::MACINTOSH,
    encoding_rs::SHIFT_JIS,
    encoding_rs::EUC_JP,
    encoding_rs::ISO_2022_JP,
    encoding_rs::GBK,
    encoding_rs::GB18030,
    encoding_rs::BIG5,
    encoding_rs::EUC_KR,
    encoding_rs::X_USER_DEFINED,
];

/// At most this many places are named when characters cannot be stored.
pub const MAX_UNMAPPABLE: usize = 20;

impl Charset {
    /// Every encoding the editor offers, most common first.
    pub fn all() -> Vec<Charset> {
        let mut v = vec![
            Charset::Utf8,
            Charset::Utf16Le,
            Charset::Utf16Be,
            Charset::Latin1,
        ];
        v.extend(OTHERS.iter().map(|e| Charset::Other(e)));
        v
    }

    /// The name shown in the status bar and the lists.
    pub fn name(self) -> String {
        match self {
            Charset::Utf8 => "UTF-8".into(),
            Charset::Utf16Le => "UTF-16 LE".into(),
            Charset::Utf16Be => "UTF-16 BE".into(),
            Charset::Latin1 => "ISO-8859-1".into(),
            Charset::Other(e) => {
                let n = e.name();
                match n.strip_prefix("windows-") {
                    Some(rest) => format!("Windows-{rest}"),
                    None => n.to_string(),
                }
            }
        }
    }

    /// Its byte order mark, if it has one.
    pub fn bom(self) -> Option<&'static [u8]> {
        match self {
            Charset::Utf8 => Some(b"\xEF\xBB\xBF"),
            Charset::Utf16Le => Some(b"\xFF\xFE"),
            Charset::Utf16Be => Some(b"\xFE\xFF"),
            _ => None,
        }
    }

    /// Two bytes per unit: text files of these hold NUL bytes.
    pub fn wide(self) -> bool {
        matches!(self, Charset::Utf16Le | Charset::Utf16Be)
    }

    /// Bytes (without a BOM) → text; `Err` names the encoding when the bytes
    /// are not valid in it.
    pub fn decode(self, bytes: &[u8]) -> Result<String, String> {
        let bad = || format!("Not valid {} text", self.name());
        match self {
            Charset::Utf8 => std::str::from_utf8(bytes)
                .map(str::to_string)
                .map_err(|_| bad()),
            Charset::Utf16Le | Charset::Utf16Be => {
                let (pairs, rest) = bytes.as_chunks::<2>();
                if !rest.is_empty() {
                    return Err(bad());
                }
                let units = pairs.iter().map(|b| match self {
                    Charset::Utf16Le => u16::from_le_bytes([b[0], b[1]]),
                    _ => u16::from_be_bytes([b[0], b[1]]),
                });
                char::decode_utf16(units)
                    .collect::<Result<String, _>>()
                    .map_err(|_| bad())
            }
            Charset::Latin1 => Ok(bytes.iter().map(|&b| char::from(b)).collect()),
            Charset::Other(e) => e
                .decode_without_bom_handling_and_without_replacement(bytes)
                .map(|c| c.into_owned())
                .ok_or_else(bad),
        }
    }

    /// Text → bytes (without a BOM). `Err` lists the (line, column) of
    /// characters this encoding cannot store (1-based, at most
    /// [`MAX_UNMAPPABLE`]) — they are never replaced.
    pub fn encode(self, text: &str) -> Result<Vec<u8>, Vec<(usize, usize, char)>> {
        match self {
            Charset::Utf8 => return Ok(text.as_bytes().to_vec()),
            Charset::Utf16Le => {
                return Ok(text.encode_utf16().flat_map(u16::to_le_bytes).collect());
            }
            Charset::Utf16Be => {
                return Ok(text.encode_utf16().flat_map(u16::to_be_bytes).collect());
            }
            _ => {}
        }
        let mut ok: HashMap<char, bool> = HashMap::new();
        let mut fits = |c: char| -> bool {
            if c.is_ascii() && self != Charset::Other(encoding_rs::ISO_2022_JP) {
                return true;
            }
            *ok.entry(c).or_insert_with(|| match self {
                Charset::Latin1 => (c as u32) < 256,
                Charset::Other(e) => {
                    let mut buf = [0u8; 4];
                    let (_, _, bad) = e.encode(c.encode_utf8(&mut buf));
                    !bad
                }
                _ => true,
            })
        };
        let mut bad = Vec::new();
        for (li, line) in text.split('\n').enumerate() {
            for (ci, c) in line.chars().enumerate() {
                if !fits(c) && bad.len() < MAX_UNMAPPABLE {
                    bad.push((li + 1, ci + 1, c));
                }
            }
        }
        if !bad.is_empty() {
            return Err(bad);
        }
        Ok(match self {
            Charset::Latin1 => text.chars().map(|c| c as u8).collect(),
            Charset::Other(e) => e.encode(text).0.into_owned(),
            _ => unreachable!("handled above"),
        })
    }

    /// The encoding a byte order mark at the start of `bytes` names, and the
    /// mark's length.
    pub fn sniff(bytes: &[u8]) -> Option<(Charset, usize)> {
        [Charset::Utf8, Charset::Utf16Le, Charset::Utf16Be]
            .into_iter()
            .find_map(|c| {
                let m = c.bom()?;
                bytes.starts_with(m).then_some((c, m.len()))
            })
    }
}

/// "Line 3, column 5 'ü'; line 9, column 1 '€'" for a refused save.
pub fn describe_unmappable(bad: &[(usize, usize, char)]) -> String {
    let mut s: Vec<String> = bad
        .iter()
        .map(|(l, c, ch)| format!("line {l}, column {c} '{ch}'"))
        .collect();
    if bad.len() >= MAX_UNMAPPABLE {
        s.push("…".into());
    }
    s.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let text = "Grüße — 1 €\nzweite Zeile\n";
        for c in [Charset::Utf8, Charset::Utf16Le, Charset::Utf16Be] {
            let b = c.encode(text).unwrap();
            assert_eq!(c.decode(&b).unwrap(), text, "{}", c.name());
        }
        let w = Charset::Other(encoding_rs::WINDOWS_1252);
        let b = w.encode(text).unwrap();
        assert!(b.contains(&0x80), "€ is 0x80 in Windows-1252");
        assert_eq!(w.decode(&b).unwrap(), text);
        assert_eq!(w.name(), "Windows-1252");
        // Every byte of the single-byte encodings comes back unchanged.
        let all: Vec<u8> = (0..=255).collect();
        for c in [Charset::Latin1, w] {
            let t = c.decode(&all).unwrap();
            assert_eq!(c.encode(&t).unwrap(), all, "{}", c.name());
        }
        assert_eq!(Charset::Latin1.decode(b"\x80").unwrap(), "\u{80}");
    }

    #[test]
    fn unmappable_characters_are_listed_not_replaced() {
        let latin1 = Charset::Latin1.encode("ok\nprice: 5 € and ✓");
        assert_eq!(latin1, Err(vec![(2, 10, '€'), (2, 16, '✓')]));
        let w = Charset::Other(encoding_rs::WINDOWS_1252).encode("a\nb ✓");
        assert_eq!(w, Err(vec![(2, 3, '✓')]));
        assert_eq!(describe_unmappable(&[(2, 3, '✓')]), "line 2, column 3 '✓'");
        let many = "✓".repeat(50);
        assert_eq!(
            Charset::Latin1.encode(&many).unwrap_err().len(),
            MAX_UNMAPPABLE
        );
    }

    #[test]
    fn invalid_bytes_and_byte_order_marks() {
        assert!(Charset::Utf8.decode(b"\xFF").is_err());
        assert!(Charset::Utf16Le.decode(b"\x41").is_err(), "odd length");
        assert_eq!(Charset::sniff(b"\xFF\xFEA\0"), Some((Charset::Utf16Le, 2)));
        assert_eq!(Charset::sniff(b"\xEF\xBB\xBFx"), Some((Charset::Utf8, 3)));
        assert_eq!(Charset::sniff(b"plain"), None);
        let names: Vec<String> = Charset::all().iter().map(|c| c.name()).collect();
        assert_eq!(
            names[..4],
            ["UTF-8", "UTF-16 LE", "UTF-16 BE", "ISO-8859-1"]
        );
        assert!(!names.iter().any(|n| n == "replacement"));
    }
}
