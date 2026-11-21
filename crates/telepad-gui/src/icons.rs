//! Phosphor icon font (MIT, assets/fonts) and OS logos (Simple Icons, assets/os).

use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, ImageSource};

// Phosphor "regular" codepoints (@phosphor-icons/web 2.1.2).
pub const TERMINAL: &str = "\u{eae8}";
pub const MONITOR: &str = "\u{e32e}";
pub const EDIT: &str = "\u{e3b4}";
pub const CLONE: &str = "\u{e1ca}";
pub const COPY: &str = "\u{e198}";
pub const TRASH: &str = "\u{e4a6}";
pub const PLUS: &str = "\u{e3d4}";
pub const REFRESH: &str = "\u{e094}";
pub const IMPORT: &str = "\u{e20c}";
pub const EXPORT: &str = "\u{e4c0}";
pub const MENU: &str = "\u{e2f0}";
/// dots-six-vertical: drag handle.
pub const GRIP: &str = "\u{eae2}";
/// key: public keys window.
pub const KEY: &str = "\u{e2d6}";
pub const CHECK_CIRCLE: &str = "\u{e184}";
pub const X_CIRCLE: &str = "\u{e4f8}";
pub const SEARCH: &str = "\u{e30c}";
pub const CONTAINER: &str = "\u{e1da}";
pub const STACK: &str = "\u{e466}";
pub const THERMOMETER: &str = "\u{e5c6}";
pub const FAN: &str = "\u{e9f2}";
pub const WARNING: &str = "\u{e4e0}";
pub const DISKS: &str = "\u{e2a0}";
pub const NETWORK: &str = "\u{edde}";
pub const GAUGE: &str = "\u{e628}";
pub const OVERVIEW: &str = "\u{e464}";
pub const SAVE: &str = "\u{e248}";
pub const INFO: &str = "\u{e2ce}";
pub const PLUGS: &str = "\u{eb5a}";
pub const DATABASE: &str = "\u{e1de}";
pub const CARET_DOWN: &str = "\u{e136}";
pub const CARET_UP: &str = "\u{e13c}";
pub const CARET_RIGHT: &str = "\u{e13a}";
pub const DOT: &str = "\u{e18a}";
pub const WINDOWS: &str = "\u{e692}";
// Host Management.
pub const SYNC: &str = "\u{e0a0}";
pub const ARROW_RIGHT: &str = "\u{e06c}";
pub const ARROW_LEFT: &str = "\u{e058}";
pub const DESKTOP: &str = "\u{e560}";
pub const EQUALS: &str = "\u{e21c}";
pub const NOT_EQUALS: &str = "\u{eda6}";
// Database window.
pub const TABLE: &str = "\u{e476}";
pub const PLAY: &str = "\u{e3d0}";
pub const STOP: &str = "\u{e46c}";
pub const LOCK: &str = "\u{e2fa}";
pub const LOCK_OPEN: &str = "\u{e306}";
pub const CARET_LEFT: &str = "\u{e138}";
pub const EYE: &str = "\u{e220}";
pub const UNDO: &str = "\u{e038}";
pub const CODE: &str = "\u{e1bc}";
pub const ROWS: &str = "\u{e5a2}";
pub const FUNNEL: &str = "\u{e266}";
pub const FOLDER: &str = "\u{e24a}";
pub const CHECK: &str = "\u{e182}";
pub const MINUS: &str = "\u{e32a}";
pub const ROW_PLUS: &str = "\u{e59e}";
/// Workspace windows (#77): close, maximize, sidebar toggle, window list.
pub const X: &str = "\u{e4f6}";
pub const SQUARE: &str = "\u{e45e}";
pub const SIDEBAR: &str = "\u{ec24}";
pub const APP_WINDOW: &str = "\u{e5da}";
pub const CIRCLE_HALF: &str = "\u{e18c}";

/// Add Phosphor as a fallback font so icon codepoints render in any text.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

/// egui's fonts + Phosphor icons. Proportional text also falls back to the
/// built-in monospace font (Hack), which has arrows and other symbols the
/// UI font lacks; without it they draw as an empty box.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "phosphor".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Phosphor.ttf"
        ))),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("phosphor".into());
        }
    }
    if fonts.font_data.contains_key("Hack")
        && let Some(list) = fonts.families.get_mut(&FontFamily::Proportional)
        && !list.iter().any(|f| f == "Hack")
    {
        list.push("Hack".into());
    }
    fonts
}

/// `"<icon>  <text>"` for buttons and labels.
pub fn label(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsKind {
    Ubuntu,
    Debian,
    RedHat,
    CentOS,
    Rocky,
    Alma,
    Alpine,
    Arch,
    OpenSuse,
    Synology,
    Qnap,
    Proxmox,
    Raspbian,
    FreeBsd,
    MacOs,
    Windows,
    Linux,
    Unknown,
}

/// Recognise the distribution from os-release NAME/PRETTY_NAME, uname -s and the kernel.
pub fn detect(os_name: &str, os_pretty: &str, os_type: &str, kernel: &str) -> OsKind {
    let s = format!("{os_name} {os_pretty}").to_ascii_lowercase();
    let t = os_type.to_ascii_lowercase();
    let has = |k: &str| s.contains(k);
    if t.contains("windows")
        || t.starts_with("mingw")
        || t.starts_with("msys")
        || t.starts_with("cygwin")
        || has("windows")
    {
        OsKind::Windows
    } else if t == "darwin" || has("macos") {
        OsKind::MacOs
    } else if t == "freebsd" || has("freebsd") {
        OsKind::FreeBsd
    } else if kernel.contains("-pve") || has("proxmox") {
        OsKind::Proxmox
    } else if has("synology") {
        OsKind::Synology
    } else if has("qnap") || has("qts") {
        OsKind::Qnap
    } else if has("raspbian") || has("raspberry") {
        OsKind::Raspbian
    } else if has("ubuntu") {
        OsKind::Ubuntu
    } else if has("debian") {
        OsKind::Debian
    } else if has("centos") {
        OsKind::CentOS
    } else if has("rocky") {
        OsKind::Rocky
    } else if has("almalinux") {
        OsKind::Alma
    } else if has("red hat") || has("rhel") {
        OsKind::RedHat
    } else if has("alpine") {
        OsKind::Alpine
    } else if has("arch linux") {
        OsKind::Arch
    } else if has("suse") {
        OsKind::OpenSuse
    } else if t == "linux" || !s.trim().is_empty() {
        OsKind::Linux
    } else {
        OsKind::Unknown
    }
}

impl OsKind {
    /// SVG logo and brand colour; `None` colour means "use the text colour"
    /// (black brand marks would vanish on the dark theme).
    fn logo(self) -> Option<(ImageSource<'static>, Option<Color32>)> {
        let rgb = |h: u32| Some(Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8));
        Some(match self {
            Self::Ubuntu => (
                egui::include_image!("../assets/os/ubuntu.svg"),
                rgb(0xE95420),
            ),
            Self::Debian => (
                egui::include_image!("../assets/os/debian.svg"),
                rgb(0xD70A53),
            ),
            Self::RedHat => (
                egui::include_image!("../assets/os/redhat.svg"),
                rgb(0xEE0000),
            ),
            Self::CentOS => (
                egui::include_image!("../assets/os/centos.svg"),
                rgb(0x9CCD2A),
            ),
            Self::Rocky => (
                egui::include_image!("../assets/os/rockylinux.svg"),
                rgb(0x10B981),
            ),
            Self::Alma => (egui::include_image!("../assets/os/almalinux.svg"), None),
            Self::Alpine => (
                egui::include_image!("../assets/os/alpinelinux.svg"),
                rgb(0x0D597F),
            ),
            Self::Arch => (
                egui::include_image!("../assets/os/archlinux.svg"),
                rgb(0x1793D1),
            ),
            Self::OpenSuse => (
                egui::include_image!("../assets/os/opensuse.svg"),
                rgb(0x73BA25),
            ),
            Self::Synology => (
                egui::include_image!("../assets/os/synology.svg"),
                rgb(0xB5B5B6),
            ),
            Self::Qnap => (egui::include_image!("../assets/os/qnap.svg"), rgb(0x3F7BD8)),
            Self::Proxmox => (
                egui::include_image!("../assets/os/proxmox.svg"),
                rgb(0xE57000),
            ),
            Self::Raspbian => (
                egui::include_image!("../assets/os/raspberrypi.svg"),
                rgb(0xA22846),
            ),
            Self::FreeBsd => (
                egui::include_image!("../assets/os/freebsd.svg"),
                rgb(0xAB2B28),
            ),
            Self::MacOs => (egui::include_image!("../assets/os/macos.svg"), None),
            Self::Linux => (
                egui::include_image!("../assets/os/linux.svg"),
                rgb(0xFCC624),
            ),
            Self::Windows | Self::Unknown => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ubuntu => "Ubuntu",
            Self::Debian => "Debian",
            Self::RedHat => "Red Hat Enterprise Linux",
            Self::CentOS => "CentOS",
            Self::Rocky => "Rocky Linux",
            Self::Alma => "AlmaLinux",
            Self::Alpine => "Alpine Linux",
            Self::Arch => "Arch Linux",
            Self::OpenSuse => "openSUSE / SUSE",
            Self::Synology => "Synology DSM",
            Self::Qnap => "QNAP",
            Self::Proxmox => "Proxmox VE",
            Self::Raspbian => "Raspberry Pi OS",
            Self::FreeBsd => "FreeBSD",
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Unknown => "Unknown OS",
        }
    }
}

/// Draw the OS logo at `size` points. Unknown hosts get an empty slot so columns line up.
pub fn os_logo(ui: &mut egui::Ui, kind: OsKind, size: f32) -> egui::Response {
    let text_color = ui.visuals().text_color();
    match kind.logo() {
        Some((src, color)) => ui
            .add(
                egui::Image::new(src)
                    .tint(color.unwrap_or(text_color))
                    .fit_to_exact_size(egui::vec2(size, size)),
            )
            .on_hover_text(kind.name()),
        None if kind == OsKind::Windows => ui
            .label(
                egui::RichText::new(WINDOWS)
                    .size(size)
                    .color(Color32::from_rgb(0x00, 0x78, 0xD4)),
            )
            .on_hover_text(kind.name()),
        None => {
            ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover())
                .1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_fleet() {
        let d = |n: &str, p: &str, t: &str, k: &str| detect(n, p, t, k);
        assert_eq!(
            d("Ubuntu", "Ubuntu 22.04.5 LTS", "Linux", "5.15.0"),
            OsKind::Ubuntu
        );
        assert_eq!(
            d(
                "Debian GNU/Linux",
                "Debian GNU/Linux 12 (bookworm)",
                "Linux",
                "6.1.0-35-amd64"
            ),
            OsKind::Debian
        );
        assert_eq!(
            d("CentOS Linux", "CentOS Linux 7 (Core)", "Linux", "3.10.0"),
            OsKind::CentOS
        );
        assert_eq!(
            d(
                "Red Hat Enterprise Linux",
                "Red Hat Enterprise Linux 8.10 (Ootpa)",
                "Linux",
                "4.18.0"
            ),
            OsKind::RedHat
        );
        assert_eq!(
            d("Synology DSM", "Synology DSM 7.2", "Linux", "4.4.302+"),
            OsKind::Synology
        );
        assert_eq!(
            d(
                "Debian GNU/Linux",
                "Debian GNU/Linux 12",
                "Linux",
                "6.8.12-4-pve"
            ),
            OsKind::Proxmox
        );
        assert_eq!(d("", "", "MINGW64_NT-10.0", ""), OsKind::Windows);
        assert_eq!(d("", "", "Linux", "5.4"), OsKind::Linux);
        assert_eq!(d("", "", "", ""), OsKind::Unknown);
    }

    /// Every non-ASCII character in the GUI's string literals, plus every icon
    /// constant, must have a glyph in both font families (no empty boxes).
    #[test]
    fn every_character_in_the_ui_has_a_glyph() {
        use egui::{FontId, RawInput};

        fn chars_in(dir: &std::path::Path, out: &mut std::collections::BTreeMap<char, String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    chars_in(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    for (n, line) in text.lines().enumerate() {
                        let code = line.trim_start();
                        if code.starts_with("//") {
                            continue;
                        }
                        let mut in_str = false;
                        let mut prev = ' ';
                        for c in line.chars() {
                            if c == '"' && prev != '\\' {
                                in_str = !in_str;
                            } else if in_str && !c.is_ascii() {
                                out.entry(c)
                                    .or_insert_with(|| format!("{}:{}", p.display(), n + 1));
                            }
                            prev = c;
                        }
                    }
                }
            }
        }
        let mut used = std::collections::BTreeMap::new();
        chars_in(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut used,
        );
        for icon in [
            TERMINAL,
            MONITOR,
            EDIT,
            CLONE,
            COPY,
            TRASH,
            PLUS,
            REFRESH,
            IMPORT,
            EXPORT,
            MENU,
            GRIP,
            KEY,
            CHECK_CIRCLE,
            X_CIRCLE,
            SEARCH,
            CONTAINER,
            STACK,
            THERMOMETER,
            FAN,
            WARNING,
            DISKS,
            NETWORK,
            GAUGE,
            OVERVIEW,
            SAVE,
            INFO,
            PLUGS,
            DATABASE,
            CARET_DOWN,
            CARET_UP,
            CARET_RIGHT,
            DOT,
            WINDOWS,
            SYNC,
            ARROW_RIGHT,
            ARROW_LEFT,
            DESKTOP,
            EQUALS,
            NOT_EQUALS,
            TABLE,
            PLAY,
            STOP,
            LOCK,
            LOCK_OPEN,
            CARET_LEFT,
            EYE,
            UNDO,
            CODE,
            ROWS,
            FUNNEL,
            FOLDER,
            CHECK,
            MINUS,
            ROW_PLUS,
            X,
            SQUARE,
            SIDEBAR,
            APP_WINDOW,
            CIRCLE_HALF,
        ] {
            for c in icon.chars() {
                used.entry(c).or_insert_with(|| "icons.rs".to_string());
            }
        }
        assert!(
            used.contains_key(&'\u{2192}'),
            "the arrow is still used somewhere"
        );

        let ctx = egui::Context::default();
        ctx.set_fonts(definitions());
        ctx.begin_pass(RawInput::default());
        let missing: Vec<String> = ctx.fonts_mut(|f| {
            used.iter()
                .filter(|(c, _)| {
                    !f.has_glyph(&FontId::proportional(14.0), **c)
                        || !f.has_glyph(&FontId::monospace(14.0), **c)
                })
                .map(|(c, at)| format!("{c:?} U+{:04X} at {at}", *c as u32))
                .collect()
        });
        // No renderer in a test: drop the font-atlas upload on purpose.
        ctx.end_pass().textures_delta.clear();
        assert!(missing.is_empty(), "no glyph for:\n{}", missing.join("\n"));
    }
}
