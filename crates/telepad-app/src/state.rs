//! Application state: [`Model`] (changed only by [`crate::update`]) and
//! [`Drafts`] (what views may edit directly: form text, filter, toggles).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use telepad_core::config::{Access, HostEntry, Inventory};
use telepad_core::model::HostReport;
use telepad_core::store::{self, ImportMode, ImportSummary};
use telepad_core::update::{Check, UpdateConfig};

/// How long a status-bar message stays visible.
pub const MESSAGE_TTL: Duration = Duration::from_secs(6);

#[derive(Debug, Default)]
pub struct State {
    pub model: Model,
    pub drafts: Drafts,
}

#[derive(Debug, Default)]
pub struct Model {
    pub db_path: Option<PathBuf>,
    /// The generated file ssh reads.
    pub ssh_config_path: Option<PathBuf>,
    pub hosts: Vec<HostState>,
    pub selected: Option<usize>,
    pub last_refresh: Option<Instant>,
    /// Hosts queued or being collected right now (from the runtime).
    pub busy: usize,
    pub message: Option<(String, Instant)>,
    pub fatal: Option<String>,
    /// Host id + alias waiting for delete confirmation.
    pub pending_delete: Option<(i64, String)>,
    /// Select this host id once the host list is reloaded (after a save).
    pub select_after_load: Option<i64>,
    pub import: Option<ImportDialog>,
    pub update: UpdateState,
}

/// Values views bind to widgets directly. Changing them never does I/O.
#[derive(Debug)]
pub struct Drafts {
    pub filter: String,
    pub auto_refresh: bool,
    pub interval_secs: u32,
    pub ports_nginx_only: bool,
    pub editor: Option<Editor>,
    pub update_form: UpdateConfig,
    pub update_form_error: Option<String>,
}

impl Default for Drafts {
    fn default() -> Self {
        Self {
            filter: String::new(),
            auto_refresh: true,
            interval_secs: 60,
            ports_nginx_only: false,
            editor: None,
            update_form: UpdateConfig::default(),
            update_form_error: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct HostState {
    pub entry: HostEntry,
    pub report: Option<HostReport>,
    pub error: Option<String>,
    pub loading: bool,
    pub updated: Option<Instant>,
    pub elapsed: Option<Duration>,
}

impl HostState {
    pub fn new(entry: HostEntry) -> Self {
        Self {
            entry,
            ..Default::default()
        }
    }

    pub fn online(&self) -> Option<bool> {
        match (&self.error, &self.report) {
            (Some(_), _) => Some(false),
            (None, Some(_)) => Some(true),
            _ => None,
        }
    }
}

/// Add/edit form. Text fields are kept as typed and validated on save.
#[derive(Debug, Clone)]
pub struct Editor {
    pub entry: HostEntry,
    pub port: String,
    pub identity: String,
    pub options: String,
    pub monitored: bool,
    pub access: Access,
    pub error: Option<String>,
    pub confirm_delete: bool,
    pub cloned_from: Option<String>,
}

impl Editor {
    pub fn new(entry: HostEntry) -> Self {
        Self {
            cloned_from: None,
            port: entry.port.to_string(),
            identity: entry.identity_file.clone().unwrap_or_default(),
            options: store::options_text(&entry.options),
            monitored: !entry.unmonitored,
            access: entry.access,
            entry,
            error: None,
            confirm_delete: false,
        }
    }

    /// New host with the connection settings (User, Port, IdentityFile) of `src`.
    pub fn clone_of(src: &HostEntry) -> Self {
        let entry = HostEntry {
            user: src.user.clone(),
            port: src.port,
            identity_file: src.identity_file.clone(),
            access: src.access,
            ..HostEntry::new("")
        };
        Self {
            cloned_from: Some(src.alias.clone()),
            ..Self::new(entry)
        }
    }

    pub fn title(&self) -> String {
        match (&self.entry.id, &self.cloned_from) {
            (Some(_), _) => format!("Edit host — {}", self.entry.alias),
            (None, Some(src)) => format!("Add host — cloned from {src}"),
            (None, None) => "Add host".to_string(),
        }
    }

    /// Switch Access; a port still at the old default follows to the new one.
    pub fn set_access(&mut self, access: Access) {
        if self.port.trim() == self.access.default_port().to_string() {
            self.port = access.default_port().to_string();
        }
        self.access = access;
    }

    pub fn build(&self) -> Result<HostEntry, String> {
        let mut e = self.entry.clone();
        e.alias = e.alias.trim().to_string();
        e.hostname = e.hostname.trim().to_string();
        e.user = e.user.trim().to_string();
        e.note = e.note.trim().to_string();
        e.port = self
            .port
            .trim()
            .parse()
            .map_err(|_| "Port must be a number 1-65535".to_string())?;
        let id = self.identity.trim();
        e.identity_file = (!id.is_empty()).then(|| id.to_string());
        e.options = store::decode_options(&self.options);
        e.unmonitored = !self.monitored;
        e.access = self.access;
        e.validate()?;
        Ok(e)
    }
}

#[derive(Debug, Clone)]
pub struct ImportDialog {
    pub path: PathBuf,
    pub inventory: Inventory,
    pub mode: ImportMode,
    pub preview: Result<ImportSummary, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    Installing,
    /// New exes are in place; a restart starts them.
    Installed(String),
}

#[derive(Debug, Clone, Default)]
pub struct UpdateState {
    /// None until the settings were read from the database.
    pub config: Option<UpdateConfig>,
    pub phase: UpdatePhase,
    pub last_check: Option<Instant>,
    /// Result of the last successful check.
    pub check: Option<Check>,
    pub error: Option<String>,
    /// "Later" hides the banner until the next start.
    pub later: bool,
    pub window: bool,
    /// The last check was started by the user: report "up to date" too.
    pub manual: bool,
}

impl UpdateState {
    pub fn enabled(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c.enabled)
    }

    /// The release to advertise in the top bar, if any.
    pub fn banner(&self) -> Option<&Check> {
        self.check
            .as_ref()
            .filter(|c| c.newer && !self.later && self.phase == UpdatePhase::Idle)
    }

    pub fn can_install(&self) -> bool {
        self.phase == UpdatePhase::Idle
            && self
                .check
                .as_ref()
                .is_some_and(|c| c.newer && c.asset.is_some())
    }
}

impl Model {
    pub fn online_count(&self) -> usize {
        self.hosts
            .iter()
            .filter(|h| h.online() == Some(true))
            .count()
    }

    pub fn offline_count(&self) -> usize {
        self.hosts
            .iter()
            .filter(|h| h.online() == Some(false))
            .count()
    }

    pub fn message_at(&self, now: Instant) -> Option<&str> {
        self.message
            .as_ref()
            .filter(|(_, t)| now.saturating_duration_since(*t) < MESSAGE_TTL)
            .map(|(m, _)| m.as_str())
    }
}

/// Search box match: alias, address, description, OS, reported hostname or a container name.
pub fn matches_filter(filter: &str, h: &HostState) -> bool {
    if filter.is_empty() {
        return true;
    }
    let f = filter.to_lowercase();
    let r = h.report.as_ref();
    [
        h.entry.alias.as_str(),
        h.entry.hostname.as_str(),
        h.entry.note.as_str(),
        r.map_or("", |r| r.os_pretty.as_str()),
        r.map_or("", |r| r.hostname.as_str()),
    ]
    .iter()
    .any(|s| s.to_lowercase().contains(&f))
        || r.is_some_and(|r| {
            r.docker
                .containers
                .iter()
                .any(|c| c.name.to_lowercase().contains(&f))
        })
}
