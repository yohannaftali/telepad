pub mod collector;
pub mod config;
pub mod model;
pub mod parse;
pub mod platform;
pub mod store;
pub mod update;

use std::path::{Path, PathBuf};

/// Config path: `--config <path>` > `TELEPAD_CONFIG` > `./config` > `config` next to the exe.
pub fn resolve_config(cli: Option<&str>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = cli {
        candidates.push(p.into());
    }
    if let Some(p) = std::env::var_os("TELEPAD_CONFIG") {
        candidates.push(p.into());
    }
    candidates.push("config".into());
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        candidates.push(dir.join("config"));
    }
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
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
        if let Some(h) = home {
            return PathBuf::from(h).join(rest);
        }
    }
    PathBuf::from(p)
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
            // Portable layout: with no `config` around, keep the data next to the exe.
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(Path::to_path_buf));
            config
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .or(exe_dir)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(DB_FILE)
        });
    let db = std::path::absolute(&db).map(strip_verbatim).unwrap_or(db);
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
