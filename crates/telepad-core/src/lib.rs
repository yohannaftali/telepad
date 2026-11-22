pub mod authorized_keys;
pub mod cluster;
pub mod collector;
pub mod config;
pub mod db;
pub mod docker_actions;
pub mod eol;
pub mod files;
pub mod history;
pub mod hostsync;
pub mod local_keys;
pub mod model;
pub mod parse;
pub mod platform;
pub mod prefs;
pub mod rdp;
pub mod store;
pub mod update;

use std::path::{Path, PathBuf};

/// File next to the exe that marks a portable copy (shipped in the zips):
/// data stays next to the program instead of the per-user data folder.
pub const PORTABLE_MARKER: &str = "portable";

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
}

/// Where `telepad.db` lives when neither `--db`, `TELEPAD_DB` nor a `config`
/// decides it: next to the exe for a portable copy (marker file, or a
/// `telepad.db` already there), otherwise the per-user data folder.
pub fn data_dir() -> PathBuf {
    let exe = exe_dir();
    choose_data_dir(exe.as_deref(), platform::user_data_dir(), |d| {
        d.join(PORTABLE_MARKER).is_file() || d.join(DB_FILE).is_file()
    })
}

fn choose_data_dir(
    exe_dir: Option<&Path>,
    user_dir: Option<PathBuf>,
    is_portable: impl Fn(&Path) -> bool,
) -> PathBuf {
    match (exe_dir, user_dir) {
        (Some(e), _) if is_portable(e) => e.to_path_buf(),
        (_, Some(u)) => u,
        (Some(e), None) => e.to_path_buf(),
        (None, None) => PathBuf::from("."),
    }
}

/// Config path: `--config <path>` > `TELEPAD_CONFIG` > `./config` > `config` next
/// to the exe > `config` in the data folder.
pub fn resolve_config(cli: Option<&str>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = cli {
        candidates.push(p.into());
    }
    if let Some(p) = std::env::var_os("TELEPAD_CONFIG") {
        candidates.push(p.into());
    }
    candidates.push("config".into());
    if let Some(dir) = exe_dir() {
        candidates.push(dir.join("config"));
    }
    candidates.push(data_dir().join("config"));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| std::fs::canonicalize(&p).map(strip_verbatim).unwrap_or(p))
}

/// `canonicalize` on Windows returns `\\?\C:\...`, which ssh cannot open.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.display().to_string();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

/// Read one `KEY=value` from a `.env` next to the config (no other values are kept).
pub fn dotenv_value(dir: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".env")).ok()?;
    text.lines()
        .find_map(|l| {
            let (k, v) = l.trim().split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
        })
        .filter(|v| !v.is_empty())
}

pub fn expand_home(p: &str) -> PathBuf {
    let rest = if p == "~" {
        Some("")
    } else {
        p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\"))
    };
    if let Some(rest) = rest {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
        if let Some(h) = home {
            let h = PathBuf::from(h);
            return if rest.is_empty() { h } else { h.join(rest) };
        }
    }
    PathBuf::from(p)
}

/// Private keys in `~/.ssh`, as `~/.ssh/<name>`, sorted. Only the first line
/// of each file is read, to tell keys from `.pub`, `known_hosts`, `config`, …
pub fn find_ssh_keys() -> Vec<String> {
    use std::io::Read;
    let dir = expand_home("~/.ssh");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let mut head = [0u8; 64];
            let n = std::fs::File::open(e.path())
                .and_then(|mut f| f.read(&mut head))
                .ok()?;
            looks_like_private_key(&name, &String::from_utf8_lossy(&head[..n]))
                .then(|| format!("~/.ssh/{name}"))
        })
        .collect();
    keys.sort();
    keys
}

/// A private key file: PEM / OpenSSH header, and not a public key or ssh's other files.
pub fn looks_like_private_key(name: &str, head: &str) -> bool {
    let skip = name.ends_with(".pub")
        || name.starts_with('.')
        || matches!(
            name,
            "config" | "known_hosts" | "known_hosts.old" | "authorized_keys" | "authorized_keys2"
        );
    let first = head.lines().next().unwrap_or_default();
    !skip && first.starts_with("-----BEGIN") && first.contains("PRIVATE KEY")
}

/// SSH settings for a config file, including the optional `.env` fallback key.
pub fn settings_for(config_path: &Path) -> collector::SshSettings {
    let mut s = collector::SshSettings::new(config_path.to_path_buf());
    let dir = config_path.parent().unwrap_or(Path::new("."));
    s.fallback_identity = dotenv_value(dir, "SSH_IDENTITY_FILE").map(|v| expand_home(&v));
    s
}

/// Where the host inventory lives and the files derived from it.
pub struct Workspace {
    pub store: store::Store,
    /// Generated from the database; what `ssh -F` reads.
    pub ssh_config: PathBuf,
    /// Set when this run created the database from an OpenSSH config file.
    pub imported_from: Option<PathBuf>,
}

impl Workspace {
    pub fn settings(&self) -> collector::SshSettings {
        settings_for(&self.ssh_config)
    }

    /// Regenerate the ssh config after any change to the database.
    pub fn sync_ssh_config(&self) -> Result<(), String> {
        self.store
            .write_ssh_config(&self.ssh_config)
            .map_err(|e| e.to_string())
    }
}

pub const DB_FILE: &str = "telepad.db";
pub const GENERATED_SSH_CONFIG: &str = "telepad.ssh_config";

/// Database path: `--db` > `TELEPAD_DB` > `telepad.db` next to the OpenSSH
/// `config` (if one is found) > next to the executable. On first run (empty database)
/// the hosts are imported from that `config`.
pub fn open_workspace(cli_config: Option<&str>, cli_db: Option<&str>) -> Result<Workspace, String> {
    let config = resolve_config(cli_config);
    let db = cli_db
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TELEPAD_DB").map(PathBuf::from))
        .unwrap_or_else(|| {
            config
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(data_dir)
                .join(DB_FILE)
        });
    let db = std::path::absolute(&db).map(strip_verbatim).unwrap_or(db);
    if let Some(dir) = db.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut store = store::Store::open(&db).map_err(|e| format!("{}: {e}", db.display()))?;

    let mut imported_from = None;
    if store.is_empty().map_err(|e| e.to_string())?
        && let Some(cfg) = &config
    {
        let inv = config::load_inventory(cfg).map_err(|e| e.to_string())?;
        store
            .import(&inv, store::ImportMode::Merge)
            .map_err(|e| format!("importing {}: {e}", cfg.display()))?;
        imported_from = Some(cfg.clone());
    }
    let ssh_config = db.with_file_name(GENERATED_SSH_CONFIG);
    let ws = Workspace {
        store,
        ssh_config,
        imported_from,
    };
    ws.sync_ssh_config()?;
    Ok(ws)
}

/// Open an interactive SSH terminal for `alias` (used by `--open-ssh`).
pub fn open_ssh_alias(
    cli_config: Option<&str>,
    cli_db: Option<&str>,
    alias: &str,
) -> Result<(), String> {
    let ws = open_workspace(cli_config, cli_db)?;
    let hosts = ws.store.hosts().map_err(|e| e.to_string())?;
    let host = hosts
        .iter()
        .find(|h| h.alias == alias)
        .ok_or_else(|| format!("unknown host alias: {alias}"))?;
    let s = ws.settings();
    platform::open_terminal(&s.ssh_program, &s.args(host, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_copies_keep_data_next_to_the_exe() {
        let exe = Path::new("/opt/telepad");
        let user = PathBuf::from("/home/a/.local/share/telepad");
        assert_eq!(
            choose_data_dir(Some(exe), Some(user.clone()), |_| true),
            exe
        );
        assert_eq!(
            choose_data_dir(Some(exe), Some(user.clone()), |_| false),
            user,
            "installed copy uses the per-user folder"
        );
        assert_eq!(choose_data_dir(Some(exe), None, |_| false), exe);
        assert_eq!(choose_data_dir(None, None, |_| false), PathBuf::from("."));
        assert_eq!(choose_data_dir(None, Some(user.clone()), |_| true), user);
    }

    #[test]
    fn recognises_private_keys_only() {
        let openssh = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbn";
        let rsa = "-----BEGIN RSA PRIVATE KEY-----\nMIIE";
        assert!(looks_like_private_key("id_ed25519", openssh));
        assert!(looks_like_private_key("work-key", rsa));
        assert!(!looks_like_private_key(
            "id_ed25519.pub",
            "ssh-ed25519 AAAA user@pc"
        ));
        assert!(!looks_like_private_key(
            "known_hosts",
            "host ssh-ed25519 AAAA"
        ));
        assert!(!looks_like_private_key("config", "Host *"));
        assert!(!looks_like_private_key("notes.txt", "hello"));
        assert!(
            !looks_like_private_key("id_ed25519.pub", openssh),
            "a .pub name is never a key"
        );
    }

    #[test]
    fn user_data_dir_is_named_after_the_app() {
        if let Some(d) = platform::user_data_dir() {
            let name = d.file_name().unwrap().to_string_lossy().to_lowercase();
            assert_eq!(name, "telepad");
        }
    }
}
