//! OpenSSH client config reader/writer. The host inventory lives in SQLite
//! (`store`); this module imports from and exports to the OpenSSH format, and
//! renders the file that `ssh -F` uses, so ssh stays the authority on every option.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Marker comment that keeps a host in the file but out of monitoring.
const UNMONITORED_TAG: &str = "telepad: not monitored";
/// Marker comment for hosts reached by Remote Desktop only (no SSH).
const RDP_TAG: &str = "telepad: rdp only";

/// How the app reaches a host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Polled over SSH with the remote script.
    #[default]
    Ssh,
    /// Windows without SSH: only a TCP check on `port` (RDP) and an Open RDP button.
    Rdp,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Ssh => "ssh",
            Access::Rdp => "rdp",
        }
    }
    pub fn from_db(s: &str) -> Self {
        if s.eq_ignore_ascii_case("rdp") {
            Access::Rdp
        } else {
            Access::Ssh
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            Access::Ssh => 22,
            Access::Rdp => 3389,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostEntry {
    /// Database row id; `None` for hosts not saved yet.
    #[serde(skip)]
    pub id: Option<i64>,
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub identity_file: Option<String>,
    /// Description, e.g. "Mail Server Wima - Witec - WTB - Arsanu".
    #[serde(rename = "description")]
    pub note: String,
    /// Other ssh options for this host, in order (`KexAlgorithms +x`, ...).
    pub options: Vec<(String, String)>,
    /// Kept in the inventory and ssh config, but not polled.
    pub unmonitored: bool,
    /// SSH (default) or RDP-only Windows host; for RDP, `port` is the RDP port.
    pub access: Access,
}

impl HostEntry {
    pub fn new(alias: &str) -> Self {
        Self {
            alias: alias.to_string(),
            port: 22,
            ..Default::default()
        }
    }

    pub fn target(&self) -> String {
        let host = if self.hostname.is_empty() {
            &self.alias
        } else {
            &self.hostname
        };
        if self.user.is_empty() {
            host.clone()
        } else {
            format!("{}@{}", self.user, host)
        }
    }

    /// Problems that would make this entry unusable or unsafe to write out.
    pub fn validate(&self) -> Result<(), String> {
        let ok_alias = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        };
        if !ok_alias(&self.alias) {
            return Err("Alias must be letters, digits, '.', '_' or '-' (no spaces)".into());
        }
        if self.access == Access::Rdp && self.hostname.trim().is_empty() {
            return Err("An RDP host needs a HostName / IP".into());
        }
        if self.port == 0 {
            return Err("Port must be 1-65535".into());
        }
        let single_line = |label: &str, v: &str| {
            if v.contains(['\n', '\r']) {
                Err(format!("{label} must be a single line"))
            } else {
                Ok(())
            }
        };
        single_line("HostName", &self.hostname)?;
        single_line("User", &self.user)?;
        single_line("Description", &self.note)?;
        if let Some(f) = &self.identity_file {
            single_line("IdentityFile", f)?;
        }
        for (k, v) in &self.options {
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(format!("Option name '{k}' is not a valid ssh keyword"));
            }
            if ["host", "match", "include"].contains(&k.to_ascii_lowercase().as_str()) {
                return Err(format!("'{k}' cannot be used as a per-host option"));
            }
            single_line(k, v)?;
        }
        Ok(())
    }
}

/// Hosts plus everything that is not a plain host block (`Host *`, `Match`, top-level options).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inventory {
    pub hosts: Vec<HostEntry>,
    /// Raw OpenSSH text appended after the host blocks.
    pub global: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn load(path: &Path) -> Result<Vec<HostEntry>, ConfigError> {
    Ok(load_inventory(path)?.hosts)
}

pub fn load_inventory(path: &Path) -> Result<Inventory, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(parse_inventory(&text))
}

pub fn parse(text: &str) -> Vec<HostEntry> {
    parse_inventory(text).hosts
}

pub fn parse_inventory(text: &str) -> Inventory {
    let mut inv = Inventory::default();
    let mut current: Vec<HostEntry> = Vec::new();
    let mut pending_note: Vec<String> = Vec::new();
    let mut unmonitored = false;
    let mut rdp = false;
    // Inside a `Host *` / `Match` block (or before any Host line): copy lines verbatim.
    let mut raw_block = true;
    let mut global: Vec<String> = Vec::new();

    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        let line = raw.trim();
        if line.is_empty() {
            pending_note.clear();
            unmonitored = false;
            rdp = false;
            if raw_block && !global.is_empty() {
                global.push(String::new());
            }
            continue;
        }
        if let Some(c) = line.strip_prefix('#') {
            let c = c.trim();
            if c == UNMONITORED_TAG {
                unmonitored = true;
            } else if c == RDP_TAG {
                rdp = true;
            } else if !c.is_empty() && current_is_closed(&current, raw) {
                // Commented-out options (e.g. an indented "#IdentityFile ...") are not notes.
                pending_note.push(c.to_string());
            }
            continue;
        }
        let (key, value) = split_kv(line);
        match key.to_ascii_lowercase().as_str() {
            "host" => {
                inv.hosts.append(&mut current);
                let note = pending_note.join(" · ");
                pending_note.clear();
                current = value
                    .split_whitespace()
                    .filter(|a| !a.contains(['*', '?', '!']))
                    .map(|a| HostEntry {
                        note: note.clone(),
                        unmonitored,
                        access: if rdp { Access::Rdp } else { Access::Ssh },
                        ..HostEntry::new(a)
                    })
                    .collect();
                unmonitored = false;
                rdp = false;
                raw_block = current.is_empty();
                if raw_block {
                    if !note.is_empty() {
                        global.push(format!("# {note}"));
                    }
                    global.push(line.to_string());
                }
            }
            "match" => {
                inv.hosts.append(&mut current);
                pending_note.clear();
                raw_block = true;
                global.push(line.to_string());
            }
            _ if raw_block => global.push(if global.is_empty() {
                // Top-level options apply to every host; keep that meaning after the host blocks.
                format!(
                    "Host *
  {line}"
                )
            } else if global.last().is_some_and(|l| l.is_empty()) {
                line.to_string()
            } else {
                format!("  {line}")
            }),
            k => {
                for h in current.iter_mut() {
                    match k {
                        "hostname" => h.hostname = value.to_string(),
                        "user" => h.user = value.to_string(),
                        "port" => h.port = value.parse().unwrap_or(22),
                        "identityfile" => h.identity_file = Some(value.to_string()),
                        _ => h.options.push((key.to_string(), value.to_string())),
                    }
                }
            }
        }
    }
    inv.hosts.append(&mut current);
    inv.global = global.join("\n").trim().to_string();
    inv
}

/// A comment counts as a note for the next host only when it is not indented
/// inside the current block.
fn current_is_closed(current: &[HostEntry], raw: &str) -> bool {
    current.is_empty() || !raw.starts_with([' ', '\t'])
}

fn split_kv(line: &str) -> (&str, &str) {
    let idx = line.find([' ', '\t', '=']).unwrap_or(line.len());
    let key = &line[..idx];
    let value = line[idx..].trim_start_matches([' ', '\t', '=']).trim();
    (key, value.trim_matches('"'))
}

fn quote(v: &str) -> String {
    if v.contains([' ', '\t']) {
        format!("\"{v}\"")
    } else {
        v.to_string()
    }
}

/// Render an OpenSSH client config. Host blocks first (ssh uses the first
/// value it finds), then the global `Host *` / `Match` text.
pub fn to_ssh_config(inv: &Inventory) -> String {
    let mut out = String::from("# Generated by Telepad\n");
    for h in &inv.hosts {
        out.push('\n');
        if !h.note.is_empty() {
            out.push_str(&format!("# {}\n", h.note));
        }
        if h.unmonitored {
            out.push_str(&format!("# {UNMONITORED_TAG}\n"));
        }
        if h.access == Access::Rdp {
            out.push_str(&format!("# {RDP_TAG}\n"));
        }
        out.push_str(&format!("Host {}\n", h.alias));
        if !h.hostname.is_empty() {
            out.push_str(&format!("  HostName {}\n", quote(&h.hostname)));
        }
        if !h.user.is_empty() {
            out.push_str(&format!("  User {}\n", quote(&h.user)));
        }
        out.push_str(&format!("  Port {}\n", h.port));
        if let Some(f) = &h.identity_file {
            out.push_str(&format!("  IdentityFile {}\n", quote(f)));
        }
        for (k, v) in &h.options {
            out.push_str(&format!("  {k} {}\n", quote(v)));
        }
    }
    if !inv.global.trim().is_empty() {
        out.push('\n');
        out.push_str(inv.global.trim());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "# VM03\n# Git server\nHost git\n  HostName 192.0.2.24\n  User admin\n  IdentityFile ~/.ssh/id_ed25519\n\nHost app\n  Hostname 192.0.2.17\n  user admin\n  Port 2222\n\nHost legacy\n  HostName 198.51.100.2\n  #KexAlgorithms +x\n  KexAlgorithms +diffie-hellman-group1-sha1\n  User olduser\n\n# defaults\nHost *\n  ServerAliveInterval 30\n";

    #[test]
    fn parses_blocks_notes_and_case() {
        let h = parse(CFG);
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].alias, "git");
        assert_eq!(h[0].note, "VM03 · Git server");
        assert_eq!(h[0].identity_file.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(h[1].hostname, "192.0.2.17");
        assert_eq!(h[1].user, "admin");
        assert_eq!(h[1].port, 2222);
        assert_eq!(h[2].user, "olduser");
        assert_eq!(h[2].note, "");
        assert_eq!(h[2].target(), "olduser@198.51.100.2");
        assert_eq!(
            h[2].options,
            vec![(
                "KexAlgorithms".to_string(),
                "+diffie-hellman-group1-sha1".to_string()
            )]
        );
    }

    #[test]
    fn keeps_wildcard_blocks_as_global() {
        let inv = parse_inventory(CFG);
        assert_eq!(inv.global, "# defaults\nHost *\n  ServerAliveInterval 30");
    }

    #[test]
    fn top_level_options_stay_global() {
        let inv = parse_inventory(
            "ServerAliveInterval 15

Host a
  HostName 1.2.3.4
",
        );
        assert_eq!(
            inv.global,
            "Host *
  ServerAliveInterval 15"
        );
        assert!(inv.hosts[0].options.is_empty());
    }

    #[test]
    fn round_trips_through_ssh_config() {
        let mut inv = parse_inventory(CFG);
        inv.hosts[1].unmonitored = true;
        inv.hosts[2].access = Access::Rdp;
        inv.hosts[2].port = 3389;
        let text = to_ssh_config(&inv);
        let back = parse_inventory(&text);
        assert_eq!(back, inv);
    }

    #[test]
    fn rejects_injection_in_fields() {
        let mut h = HostEntry::new("ok");
        assert!(h.validate().is_ok());
        h.hostname = "10.0.0.1\nProxyCommand calc".into();
        assert!(h.validate().is_err());
        let mut h = HostEntry::new("bad alias");
        assert!(h.validate().is_err());
        h.alias = "ok".into();
        h.options = vec![("Match".into(), "all".into())];
        assert!(h.validate().is_err());
    }

    #[test]
    fn reads_repo_config_if_present() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
        let Ok(inv) = load_inventory(&path) else {
            return;
        };
        assert!(!inv.hosts.is_empty());
        assert_eq!(parse_inventory(&to_ssh_config(&inv)), inv);
    }
}
