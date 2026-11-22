//! Remote Desktop settings per host (#102): kept in `telepad.db`
//! (`rdp_settings`), written as a `.rdp` file for mstsc / Windows App, or as
//! `xfreerdp` arguments on Linux. Passwords are never part of them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    #[default]
    FullScreen,
    /// A window of this size.
    Window { width: u16, height: u16 },
    /// A window that follows its size (dynamic resolution).
    Fit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audio {
    /// Play on this computer.
    #[default]
    Local,
    /// Leave it on the remote computer.
    Remote,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayUse {
    #[default]
    Never,
    Always,
    /// Only when the computer cannot be reached directly.
    Detect,
}

/// What mstsc should do when the server cannot be verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthLevel {
    /// Connect without warning (0).
    Connect,
    /// Do not connect (1).
    Refuse,
    /// Warn me (2, mstsc default).
    #[default]
    Warn,
}

/// Saved Remote Desktop settings of one host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpSettings {
    /// Empty: the host's HostName (or alias).
    pub address: String,
    /// 0: the host's port when it is RDP-only, else 3389.
    pub port: u16,
    pub user: String,
    pub domain: String,
    pub screen: Screen,
    pub multimon: bool,
    /// 15, 16, 24 or 32.
    pub color_depth: u8,
    pub clipboard: bool,
    /// Drives to bring along: empty = none, `*` = all, else `C:;D:`.
    pub drives: String,
    pub printers: bool,
    pub audio: Audio,
    /// `/admin` (console session).
    pub admin: bool,
    pub gateway: String,
    pub gateway_use: GatewayUse,
    pub auth_level: AuthLevel,
    pub autodetect: bool,
    /// `.rdp` lines Telepad does not edit, kept as they were imported.
    pub extra: Vec<String>,
}

impl Default for RdpSettings {
    fn default() -> Self {
        Self {
            address: String::new(),
            port: 0,
            user: String::new(),
            domain: String::new(),
            screen: Screen::FullScreen,
            multimon: false,
            color_depth: 32,
            clipboard: true,
            drives: String::new(),
            printers: false,
            audio: Audio::Local,
            admin: false,
            gateway: String::new(),
            gateway_use: GatewayUse::Never,
            auth_level: AuthLevel::Warn,
            autodetect: true,
            extra: Vec::new(),
        }
    }
}

fn host_ok(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_:[]".contains(c))
}

fn line_ok(s: &str) -> bool {
    !s.chars().any(|c| c.is_control()) && s.len() <= 256
}

/// `name:t:value`, one line, a known type letter.
fn extra_ok(l: &str) -> bool {
    let mut p = l.splitn(3, ':');
    let (Some(name), Some(t), Some(_)) = (p.next(), p.next(), p.next()) else {
        return false;
    };
    line_ok(l)
        && !name.is_empty()
        && matches!(t, "i" | "s" | "b")
        && !name.trim().eq_ignore_ascii_case("password 51")
}

impl RdpSettings {
    /// Checked before saving: everything ends up in a `.rdp` file or on a
    /// command line (as separate arguments).
    pub fn validate(&self) -> Result<(), String> {
        if !self.address.is_empty() && !host_ok(&self.address) {
            return Err(format!(
                "Address '{}' is not a host name or IP",
                self.address
            ));
        }
        if !self.gateway.is_empty() && !host_ok(&self.gateway) {
            return Err(format!(
                "Gateway '{}' is not a host name or IP",
                self.gateway
            ));
        }
        for (what, v) in [("User", &self.user), ("Domain", &self.domain)] {
            if !line_ok(v) || v.contains(';') {
                return Err(format!("{what} must be one line without ';'"));
            }
        }
        if let Screen::Window { width, height } = self.screen
            && !((200..=8192).contains(&width) && (200..=8192).contains(&height))
        {
            return Err("Window size: 200 to 8192 points each way".into());
        }
        if !matches!(self.color_depth, 15 | 16 | 24 | 32) {
            return Err("Colours: 15, 16, 24 or 32 bits".into());
        }
        let d = self.drives.trim();
        if !(d.is_empty()
            || d == "*"
            || d.split(';').filter(|x| !x.trim().is_empty()).all(|x| {
                let x = x.trim().trim_end_matches('\\');
                x.len() == 2 && x.as_bytes()[0].is_ascii_alphabetic() && x.ends_with(':')
            }))
        {
            return Err("Drives: empty, * (all) or letters like C:;D:".into());
        }
        if let Some(bad) = self.extra.iter().find(|l| !extra_ok(l)) {
            return Err(format!("Not a .rdp setting line: {bad}"));
        }
        Ok(())
    }

    fn target(&self, host: &str, default_port: u16) -> String {
        let addr = if self.address.is_empty() {
            host
        } else {
            &self.address
        };
        let port = if self.port == 0 {
            default_port
        } else {
            self.port
        };
        if port == 3389 {
            addr.to_string()
        } else {
            format!("{addr}:{port}")
        }
    }

    /// The `.rdp` file (CRLF lines, as mstsc writes them). `host` /
    /// `default_port`: the host's address when the settings have none.
    pub fn render(&self, host: &str, default_port: u16) -> String {
        let mut l: Vec<String> = Vec::new();
        l.push(format!(
            "full address:s:{}",
            self.target(host, default_port)
        ));
        let user = match (self.domain.trim(), self.user.trim()) {
            (_, "") => String::new(),
            ("", u) => u.to_string(),
            (d, u) => format!("{d}\\{u}"),
        };
        if !user.is_empty() {
            l.push(format!("username:s:{user}"));
        }
        if !self.domain.trim().is_empty() {
            l.push(format!("domain:s:{}", self.domain.trim()));
        }
        match self.screen {
            Screen::FullScreen => l.push("screen mode id:i:2".into()),
            Screen::Window { width, height } => {
                l.push("screen mode id:i:1".into());
                l.push(format!("desktopwidth:i:{width}"));
                l.push(format!("desktopheight:i:{height}"));
            }
            Screen::Fit => {
                l.push("screen mode id:i:1".into());
                l.push("dynamic resolution:i:1".into());
                l.push("smart sizing:i:1".into());
            }
        }
        l.push(format!("use multimon:i:{}", u8::from(self.multimon)));
        l.push(format!("session bpp:i:{}", self.color_depth));
        l.push(format!("redirectclipboard:i:{}", u8::from(self.clipboard)));
        let drives = self.drives.trim();
        if !drives.is_empty() {
            l.push(format!("drivestoredirect:s:{drives}"));
        }
        l.push(format!("redirectprinters:i:{}", u8::from(self.printers)));
        l.push(format!(
            "audiomode:i:{}",
            match self.audio {
                Audio::Local => 0,
                Audio::Remote => 1,
                Audio::Off => 2,
            }
        ));
        if self.admin {
            l.push("administrative session:i:1".into());
        }
        if !self.gateway.is_empty() && self.gateway_use != GatewayUse::Never {
            l.push(format!("gatewayhostname:s:{}", self.gateway));
            l.push(format!(
                "gatewayusagemethod:i:{}",
                match self.gateway_use {
                    GatewayUse::Always => 1,
                    _ => 2,
                }
            ));
            l.push("gatewayprofileusagemethod:i:1".into());
        } else {
            l.push("gatewayusagemethod:i:0".into());
        }
        l.push(format!(
            "authentication level:i:{}",
            match self.auth_level {
                AuthLevel::Connect => 0,
                AuthLevel::Refuse => 1,
                AuthLevel::Warn => 2,
            }
        ));
        l.push(format!("networkautodetect:i:{}", u8::from(self.autodetect)));
        for e in &self.extra {
            if extra_ok(e) {
                l.push(e.clone());
            }
        }
        let mut s = l.join("\r\n");
        s.push_str("\r\n");
        s
    }

    /// Read a `.rdp` file's text. Known lines become settings; others are
    /// kept in `extra`; a saved password (`password 51:b:`) is dropped.
    /// Returns the settings and the port of `full address` (0 if none).
    pub fn parse(text: &str) -> RdpSettings {
        let mut s = RdpSettings {
            screen: Screen::FullScreen,
            ..Default::default()
        };
        let (mut w, mut h, mut mode, mut dynamic) = (None, None, 2, false);
        let mut gw_use = 0;
        for raw in text.lines() {
            let line = raw.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            let mut p = line.splitn(3, ':');
            let (Some(name), Some(_t), Some(value)) = (p.next(), p.next(), p.next()) else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let num = || value.trim().parse::<i64>().unwrap_or(0);
            match name.as_str() {
                "full address" => {
                    let v = value.trim();
                    // host, host:port, [v6]:port
                    let (host, port) = match v.rsplit_once(':') {
                        Some((h, p))
                            if !h.ends_with(':')
                                && p.parse::<u16>().is_ok()
                                && (!h.contains(':') || h.ends_with(']')) =>
                        {
                            (h.to_string(), p.parse().unwrap_or(0))
                        }
                        _ => (v.to_string(), 0),
                    };
                    s.address = host;
                    s.port = port;
                }
                "username" => {
                    let v = value.trim();
                    match v.split_once('\\') {
                        Some((d, u)) => {
                            s.domain = d.to_string();
                            s.user = u.to_string();
                        }
                        None => s.user = v.to_string(),
                    }
                }
                "domain" => s.domain = value.trim().to_string(),
                "screen mode id" => mode = num(),
                "desktopwidth" => w = Some(num()),
                "desktopheight" => h = Some(num()),
                "dynamic resolution" | "smart sizing" => dynamic |= num() == 1,
                "use multimon" => s.multimon = num() == 1,
                "session bpp" => s.color_depth = (num() as u8).max(15),
                "redirectclipboard" => s.clipboard = num() == 1,
                "drivestoredirect" => s.drives = value.trim().to_string(),
                "redirectprinters" => s.printers = num() == 1,
                "audiomode" => {
                    s.audio = match num() {
                        1 => Audio::Remote,
                        2 => Audio::Off,
                        _ => Audio::Local,
                    }
                }
                "administrative session" => s.admin = num() == 1,
                "gatewayhostname" => s.gateway = value.trim().to_string(),
                "gatewayusagemethod" => gw_use = num(),
                "authentication level" => {
                    s.auth_level = match num() {
                        0 => AuthLevel::Connect,
                        1 => AuthLevel::Refuse,
                        _ => AuthLevel::Warn,
                    }
                }
                "networkautodetect" => s.autodetect = num() == 1,
                "password 51" => {}
                // Written by Telepad from the settings above.
                "gatewayprofileusagemethod" => {}
                _ => {
                    if extra_ok(line) {
                        s.extra.push(line.to_string());
                    }
                }
            }
        }
        s.screen = match (mode, w, h) {
            (1, _, _) if dynamic => Screen::Fit,
            (1, Some(w), Some(h)) => Screen::Window {
                width: w.clamp(200, 8192) as u16,
                height: h.clamp(200, 8192) as u16,
            },
            (1, _, _) => Screen::Fit,
            _ => Screen::FullScreen,
        };
        if !matches!(s.color_depth, 15 | 16 | 24 | 32) {
            s.color_depth = 32;
        }
        s.gateway_use = match (s.gateway.is_empty(), gw_use) {
            (true, _) | (_, 0) => GatewayUse::Never,
            (_, 1) => GatewayUse::Always,
            _ => GatewayUse::Detect,
        };
        s
    }

    /// `xfreerdp` arguments (each one separate) and what it cannot do.
    pub fn xfreerdp_args(&self, host: &str, default_port: u16) -> (Vec<String>, Vec<&'static str>) {
        let mut a = vec![format!("/v:{}", self.target(host, default_port))];
        let mut not = Vec::new();
        if !self.user.trim().is_empty() {
            a.push(format!("/u:{}", self.user.trim()));
        }
        if !self.domain.trim().is_empty() {
            a.push(format!("/d:{}", self.domain.trim()));
        }
        match self.screen {
            Screen::FullScreen => a.push("/f".into()),
            Screen::Window { width, height } => a.push(format!("/size:{width}x{height}")),
            Screen::Fit => a.push("/dynamic-resolution".into()),
        }
        if self.multimon {
            a.push("/multimon".into());
        }
        a.push(format!("/bpp:{}", self.color_depth));
        if self.clipboard {
            a.push("+clipboard".into());
        }
        if !self.drives.trim().is_empty() {
            a.push("+home-drive".into());
            if self.drives.trim() != "*" {
                not.push("single drive letters (the home folder is shared instead)");
            }
        }
        if self.printers {
            a.push("/printer".into());
        }
        match self.audio {
            Audio::Local => a.push("/sound".into()),
            Audio::Remote => a.push("/audio-mode:1".into()),
            Audio::Off => a.push("/audio-mode:2".into()),
        }
        if self.admin {
            a.push("/admin".into());
        }
        if !self.gateway.is_empty() && self.gateway_use != GatewayUse::Never {
            a.push(format!("/g:{}", self.gateway));
        }
        if self.auth_level != AuthLevel::Warn {
            not.push("authentication level (xfreerdp asks about unknown certificates)");
        }
        if !self.extra.is_empty() {
            not.push("extra .rdp lines");
        }
        (a, not)
    }
}

/// Text of a `.rdp` file: UTF-16 LE with BOM (as mstsc saves it) or UTF-8.
pub fn decode_file(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0xfe {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes)
        .trim_start_matches('\u{feff}')
        .to_string()
}

/// `.rdp` bytes as mstsc writes them: UTF-16 LE with BOM.
pub fn encode_file(text: &str) -> Vec<u8> {
    let mut out = vec![0xff, 0xfe];
    for u in text.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As mstsc 10 saves it (made-up host, documentation address).
    const MSTSC: &str = "screen mode id:i:1\r\nuse multimon:i:0\r\ndesktopwidth:i:1600\r\ndesktopheight:i:900\r\nsession bpp:i:24\r\nwinposstr:s:0,1,0,0,1600,900\r\ncompression:i:1\r\nkeyboardhook:i:2\r\naudiocapturemode:i:0\r\nvideoplaybackmode:i:1\r\nconnection type:i:7\r\nnetworkautodetect:i:1\r\nbandwidthautodetect:i:1\r\ndisplayconnectionbar:i:1\r\nusername:s:LAB\\ops\r\nenableworkspacereconnect:i:0\r\ndisable wallpaper:i:0\r\nallow font smoothing:i:0\r\nallow desktop composition:i:0\r\ndisable full window drag:i:1\r\ndisable menu anims:i:1\r\ndisable themes:i:0\r\ndisable cursor setting:i:0\r\nbitmapcachepersistenable:i:1\r\nfull address:s:192.0.2.40:3390\r\naudiomode:i:0\r\nredirectprinters:i:1\r\nredirectcomports:i:0\r\nredirectsmartcards:i:1\r\nredirectclipboard:i:1\r\nredirectposdevices:i:0\r\ndrivestoredirect:s:C:\\;D:\\\r\nautoreconnection enabled:i:1\r\nauthentication level:i:2\r\nprompt for credentials:i:0\r\nnegotiate security layer:i:1\r\nremoteapplicationmode:i:0\r\nalternate shell:s:\r\nshell working directory:s:\r\ngatewayhostname:s:gw.example.net\r\ngatewayusagemethod:i:2\r\ngatewaycredentialssource:i:4\r\ngatewayprofileusagemethod:i:1\r\npromptcredentialonce:i:0\r\ngatewaybrokeringtype:i:0\r\nuse redirection server name:i:0\r\nrdgiskdcproxy:i:0\r\nkdcproxyname:s:\r\npassword 51:b:01000000D08C9DDF0115D1118C7A00C04FC297EB\r\n";

    #[test]
    fn reads_an_mstsc_file_and_drops_the_saved_password() {
        let bytes = encode_file(MSTSC);
        let s = RdpSettings::parse(&decode_file(&bytes));
        assert_eq!(s.address, "192.0.2.40");
        assert_eq!(s.port, 3390);
        assert_eq!((s.domain.as_str(), s.user.as_str()), ("LAB", "ops"));
        assert_eq!(
            s.screen,
            Screen::Window {
                width: 1600,
                height: 900
            }
        );
        assert_eq!(s.color_depth, 24);
        assert!(s.clipboard && s.printers);
        assert_eq!(s.drives, "C:\\;D:\\");
        assert_eq!(s.gateway, "gw.example.net");
        assert_eq!(s.gateway_use, GatewayUse::Detect);
        assert!(s.extra.iter().any(|l| l == "redirectsmartcards:i:1"));
        assert!(!s.extra.iter().any(|l| l.starts_with("password")));
        assert!(s.validate().is_ok(), "{:?}", s.validate());
        // Round trip.
        let again = RdpSettings::parse(&s.render("ignored", 3389));
        assert_eq!(again, s);
        assert!(!s.render("x", 3389).contains("password"));
    }

    #[test]
    fn rendering_uses_the_hosts_address_and_port_when_empty() {
        let s = RdpSettings::default();
        let text = s.render("win1.example", 3389);
        assert!(text.starts_with("full address:s:win1.example\r\n"));
        assert!(text.contains("screen mode id:i:2"));
        assert!(text.contains("gatewayusagemethod:i:0"));
        let text = s.render("192.0.2.41", 3390);
        assert!(text.starts_with("full address:s:192.0.2.41:3390\r\n"));
        let fit = RdpSettings {
            screen: Screen::Fit,
            user: "ops".into(),
            ..Default::default()
        };
        let text = fit.render("h", 3389);
        assert!(text.contains("dynamic resolution:i:1") && text.contains("username:s:ops"));
    }

    #[test]
    fn validation_keeps_the_file_sane() {
        let ok = RdpSettings::default();
        assert!(ok.validate().is_ok());
        for bad in [
            RdpSettings {
                address: "a b".into(),
                ..Default::default()
            },
            RdpSettings {
                user: "x\ny".into(),
                ..Default::default()
            },
            RdpSettings {
                screen: Screen::Window {
                    width: 10,
                    height: 900,
                },
                ..Default::default()
            },
            RdpSettings {
                color_depth: 8,
                ..Default::default()
            },
            RdpSettings {
                drives: "C:\\Windows".into(),
                ..Default::default()
            },
            RdpSettings {
                extra: vec!["full screen please".into()],
                ..Default::default()
            },
            RdpSettings {
                extra: vec!["password 51:b:0100".into()],
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn xfreerdp_arguments_are_separate() {
        let s = RdpSettings {
            user: "ops".into(),
            domain: "LAB".into(),
            screen: Screen::Window {
                width: 1280,
                height: 800,
            },
            drives: "C:".into(),
            gateway: "gw.example.net".into(),
            gateway_use: GatewayUse::Always,
            ..Default::default()
        };
        let (a, not) = s.xfreerdp_args("192.0.2.40", 3389);
        assert_eq!(a[0], "/v:192.0.2.40");
        assert!(a.contains(&"/u:ops".to_string()) && a.contains(&"/d:LAB".to_string()));
        assert!(a.contains(&"/size:1280x800".to_string()));
        assert!(a.contains(&"+clipboard".to_string()));
        assert!(a.contains(&"/g:gw.example.net".to_string()));
        assert!(!not.is_empty(), "single drives are named as not supported");
    }

    #[test]
    fn utf8_files_and_json() {
        let s =
            RdpSettings::parse("\u{feff}full address:s:[2001:db8::5]:3390\nscreen mode id:i:2\n");
        assert_eq!(s.address, "[2001:db8::5]");
        assert_eq!(s.port, 3390);
        let json = serde_json::to_string(&s).unwrap();
        let back: RdpSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        let partial: RdpSettings = serde_json::from_str("{\"user\":\"ops\"}").unwrap();
        assert_eq!(partial.color_depth, 32, "missing fields get defaults");
    }
}
