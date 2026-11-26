//! Editor colour schemes (#183): background, text, line numbers, current
//! line, selection and the syntax colours of every [`Tok`].
//!
//! The colours follow the published palettes of the VS Code themes of the
//! same names (MIT-licensed, github.com/microsoft/vscode `extensions/theme-*`)
//! and of Solarized (Ethan Schoonover, MIT). A few are darkened or lightened
//! where the original falls below the contrast the test asks for (noted).

use std::sync::atomic::{AtomicUsize, Ordering};

use eframe::egui::{self, Color32};
use telepad_core::files::syntax::Tok;

#[derive(Debug, Clone, Copy)]
pub struct Scheme {
    /// Saved in `settings` (`ui_editor_scheme`).
    pub key: &'static str,
    pub name: &'static str,
    pub dark: bool,
    pub bg: Color32,
    pub text: Color32,
    pub line_no: Color32,
    pub line_no_current: Color32,
    pub current_line: Color32,
    pub selection: Color32,
    pub comment: Color32,
    pub string: Color32,
    pub number: Color32,
    pub keyword: Color32,
    pub key_name: Color32,
    pub heading: Color32,
    pub variable: Color32,
}

impl Scheme {
    pub fn colour(&self, t: Tok) -> Color32 {
        match t {
            Tok::Plain => self.text,
            Tok::Comment => self.comment,
            Tok::Str => self.string,
            Tok::Num => self.number,
            Tok::Keyword => self.keyword,
            Tok::Key => self.key_name,
            Tok::Heading => self.heading,
            Tok::Var => self.variable,
        }
    }

    /// The editor's visuals: background, text, selection, cursor, line
    /// numbers (weak / strong text) and light or dark for the diff colours.
    pub fn apply(&self, v: &mut egui::Visuals) {
        v.dark_mode = self.dark;
        v.extreme_bg_color = self.bg;
        v.override_text_color = Some(self.text);
        v.weak_text_color = Some(self.line_no);
        v.widgets.active.fg_stroke.color = self.line_no_current;
        v.selection.bg_fill = self.selection;
        v.selection.stroke.color = self.text;
        v.text_cursor.stroke.color = self.text;
    }
}

const fn c(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const fn a(v: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_premultiplied(
        ((v >> 16) as u8 as u32 * alpha as u32 / 255) as u8,
        ((v >> 8) as u8 as u32 * alpha as u32 / 255) as u8,
        (v as u8 as u32 * alpha as u32 / 255) as u8,
        alpha,
    )
}

/// Dark+ token colours (Dark and Dark Modern).
#[allow(clippy::too_many_arguments)]
const fn scheme(
    key: &'static str,
    name: &'static str,
    dark: bool,
    bg: u32,
    text: u32,
    lines: (u32, u32),
    current_line: Color32,
    selection: u32,
    tokens: [u32; 7],
) -> Scheme {
    Scheme {
        key,
        name,
        dark,
        bg: c(bg),
        text: c(text),
        line_no: c(lines.0),
        line_no_current: c(lines.1),
        current_line,
        selection: c(selection),
        comment: c(tokens[0]),
        string: c(tokens[1]),
        number: c(tokens[2]),
        keyword: c(tokens[3]),
        key_name: c(tokens[4]),
        heading: c(tokens[5]),
        variable: c(tokens[6]),
    }
}

const DARK_PLUS: [u32; 7] = [
    0x6A9955, 0xCE9178, 0xB5CEA8, 0x569CD6, 0x9CDCFE, 0x4EC9B0, 0x4FC1FF,
];
const LIGHT_PLUS: [u32; 7] = [
    0x008000, 0xA31515, 0x098658, 0x0000FF, 0x0451A5, 0x800000, 0x001080,
];

pub const SCHEMES: [Scheme; 11] = [
    scheme(
        "dark-modern",
        "Dark Modern",
        true,
        0x1F1F1F,
        0xCCCCCC,
        (0x8B949E, 0xCCCCCC),
        a(0xFFFFFF, 12),
        0x264F78,
        DARK_PLUS,
    ),
    // Line numbers 0x858585 → 0x8E8E8E for 4.5 : 1.
    scheme(
        "dark",
        "Dark (Visual Studio)",
        true,
        0x1E1E1E,
        0xD4D4D4,
        (0x8E8E8E, 0xC6C6C6),
        a(0xFFFFFF, 12),
        0x264F78,
        DARK_PLUS,
    ),
    scheme(
        "light-modern",
        "Light Modern",
        false,
        0xFFFFFF,
        0x3B3B3B,
        (0x6E7681, 0x171184),
        a(0x000000, 14),
        0xADD6FF,
        LIGHT_PLUS,
    ),
    // Line numbers 0x237893 kept (4.9 : 1).
    scheme(
        "light",
        "Light (Visual Studio)",
        false,
        0xFFFFFF,
        0x000000,
        (0x237893, 0x0B216F),
        a(0x000000, 14),
        0xADD6FF,
        LIGHT_PLUS,
    ),
    // Comments 0x75715E → 0x8F8A72 and line numbers for 4.5 : 1.
    scheme(
        "monokai",
        "Monokai",
        true,
        0x272822,
        0xF8F8F2,
        (0x95958F, 0xC2C2BF),
        a(0xFFFFFF, 14),
        0x49483E,
        [
            0x8F8A72, 0xE6DB74, 0xAE81FF, 0xF92672, 0xA6E22E, 0x66D9EF, 0xFD971F,
        ],
    ),
    // base0 text; comments / line numbers base01 → lightened for 3 : 1 / 4.5 : 1.
    scheme(
        "solarized-dark",
        "Solarized Dark",
        true,
        0x002B36,
        0x93A1A1,
        (0x839496, 0xEEE8D5),
        a(0xFFFFFF, 12),
        0x274642,
        [
            0x6C8A93, 0x2AA198, 0xE0559F, 0x859900, 0x4A9FE0, 0xE06A2B, 0xB58900,
        ],
    ),
    // Text base01 (base00 is 4.1 : 1); colours darkened for 3 : 1.
    scheme(
        "solarized-light",
        "Solarized Light",
        false,
        0xFDF6E3,
        0x586E75,
        (0x5F7278, 0x073642),
        a(0x000000, 12),
        0xD5DEDF,
        [
            0x7A8A8C, 0x1F7A73, 0xC0306F, 0x6A7A00, 0x1F6FB0, 0xB3410D, 0x8A6900,
        ],
    ),
    scheme(
        "red",
        "Red",
        true,
        0x390000,
        0xF8F8F8,
        (0xD9A0A0, 0xFFFFFF),
        a(0xFFFFFF, 16),
        0x750000,
        [
            0xE7C0C0, 0xFFC8A0, 0xFFB3B3, 0xF12727, 0xFB9A4B, 0xFFD479, 0xFF6464,
        ],
    ),
    // Comments 0x7285B7 kept (4.3 : 1 is above 3 : 1).
    scheme(
        "tomorrow-night-blue",
        "Tomorrow Night Blue",
        true,
        0x002451,
        0xFFFFFF,
        (0x8FA0C8, 0xFFFFFF),
        a(0xFFFFFF, 14),
        0x003F8E,
        [
            0x7285B7, 0xD1F1A9, 0xFFC58F, 0xEBBBFF, 0xFF9DA4, 0xFFEEAD, 0x99FFFF,
        ],
    ),
    // Comments 0xAAAAAA → 0x8C8C8C and keyword / key darkened for 3 : 1.
    scheme(
        "quiet-light",
        "Quiet Light",
        false,
        0xF5F5F5,
        0x333333,
        (0x6D705B, 0x0B216F),
        a(0x000000, 12),
        0xC9D0D9,
        [
            0x8C8C8C, 0x448C27, 0xAB6526, 0x3F70B0, 0x7A3E9D, 0x9C5D27, 0x7A3E9D,
        ],
    ),
    scheme(
        "high-contrast",
        "High Contrast",
        true,
        0x000000,
        0xFFFFFF,
        (0xD0D0D0, 0xFFFFFF),
        a(0xFFFFFF, 30),
        0x264F78,
        [
            0x7CA668, 0xCE9178, 0xB5CEA8, 0x569CD6, 0x9CDCFE, 0x4EC9B0, 0x9CDCFE,
        ],
    ),
];

/// Empty key: Dark Modern in the dark theme, Light Modern in the light one.
pub const FOLLOW: &str = "";

static CHOSEN: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Set once per frame from the saved choice (#183).
pub fn set(key: &str) {
    let i = SCHEMES
        .iter()
        .position(|s| s.key == key)
        .unwrap_or(usize::MAX);
    CHOSEN.store(i, Ordering::Relaxed);
}

/// The scheme to draw with: the chosen one, else the app theme's.
pub fn current(app_dark: bool) -> &'static Scheme {
    match CHOSEN.load(Ordering::Relaxed) {
        i if i < SCHEMES.len() => &SCHEMES[i],
        _ if app_dark => &SCHEMES[0],
        _ => &SCHEMES[2],
    }
}

/// The saved choice's key; empty = follow the app theme.
pub fn chosen_key() -> &'static str {
    SCHEMES
        .get(CHOSEN.load(Ordering::Relaxed))
        .map_or(FOLLOW, |s| s.key)
}

pub fn name_of(key: &str) -> &'static str {
    SCHEMES
        .iter()
        .find(|s| s.key == key)
        .map_or("Follow app theme", |s| s.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Color32) -> f64 {
        let f = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_scheme_is_readable_and_complete() {
        let mut keys = std::collections::HashSet::new();
        for s in &SCHEMES {
            assert!(keys.insert(s.key), "{} twice", s.key);
            assert!(!s.key.is_empty() && !s.name.is_empty());
            let r = contrast(s.text, s.bg);
            assert!(r >= 4.5, "{}: text {r:.2}", s.name);
            let r = contrast(s.line_no, s.bg);
            assert!(r >= 4.5, "{}: line numbers {r:.2}", s.name);
            for t in [
                Tok::Comment,
                Tok::Str,
                Tok::Num,
                Tok::Keyword,
                Tok::Key,
                Tok::Heading,
                Tok::Var,
            ] {
                let r = contrast(s.colour(t), s.bg);
                assert!(r >= 3.0, "{}: {t:?} {r:.2}", s.name);
            }
            // Dark schemes are dark, light ones light (the diff colours follow).
            assert_eq!(lum(s.bg) < 0.2, s.dark, "{}", s.name);
            assert!(contrast(s.selection, s.bg) > 1.2, "{}: selection", s.name);
        }
        for want in [
            "dark-modern",
            "dark",
            "light-modern",
            "light",
            "monokai",
            "solarized-dark",
            "solarized-light",
            "red",
        ] {
            assert!(keys.contains(want), "{want} missing");
        }
    }

    #[test]
    fn follows_the_app_theme_unless_chosen() {
        set("");
        assert_eq!(current(true).key, "dark-modern");
        assert_eq!(current(false).key, "light-modern");
        set("monokai");
        assert_eq!(current(false).key, "monokai");
        set("gone");
        assert_eq!(current(true).key, "dark-modern");
        assert_eq!(name_of(""), "Follow app theme");
        assert_eq!(name_of("red"), "Red");
        set("");
    }
}
