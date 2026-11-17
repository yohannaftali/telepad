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
pub const HIDDEN: &str = "\u{e224}";
pub const CARET_DOWN: &str = "\u{e136}";
pub const CARET_RIGHT: &str = "\u{e13a}";
pub const DOT: &str = "\u{e18a}";
pub const WINDOWS: &str = "\u{e692}";
pub const LINUX: &str = "\u{eb02}";

/// Add Phosphor as a fallback font so icon codepoints render in any text.
pub fn install(ctx: &egui::Context) {
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
    ctx.set_fonts(fonts);
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
}
