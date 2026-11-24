//! Host Management window (#58): Telepad's hosts next to an OpenSSH config
//! file, with changes collected per side and written only on Apply.

use std::collections::{BTreeMap, BTreeSet};

use telepad_core::config::{Access, HostEntry};
use telepad_core::hostsync::{self, ConfigChange, ConfigFile, Row, RowState};
use telepad_core::store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Telepad,
    Config,
}

impl Side {
    pub fn title(self) -> &'static str {
        match self {
            Side::Telepad => "Telepad",
            Side::Config => "config",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    /// Save this host (for Telepad keyed by the alias it had, so a rename
    /// keeps the database row).
    Upsert(Box<HostEntry>),
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Add missing and update different hosts; keep the target's extra hosts.
    Merge,
    /// The target becomes a copy of the source.
    Replace,
}

/// What Apply will do, shown first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preview {
    /// New config text, `None` when the file does not change.
    pub config_text: Option<String>,
    pub diff: Vec<(char, String)>,
    /// One line per Telepad change.
    pub telepad: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HostSync {
    pub path: String,
    pub file: Option<ConfigFile>,
    pub loading: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// Keyed by the alias the host has in Telepad now (or the new alias).
    pub telepad: BTreeMap<String, Pending>,
    /// Keyed by alias in the config.
    pub config: BTreeMap<String, Pending>,
    /// Sync: different on both sides, waiting for the user to pick a side.
    pub conflicts: BTreeSet<String>,
    /// Comparison of both sides with the pending changes applied.
    pub rows: Vec<Row>,
    pub preview: Option<Preview>,
    pub applying: bool,
}

impl HostSync {
    pub fn pending_count(&self) -> usize {
        self.telepad.len() + self.config.len()
    }
}

/// Edit form for one host on one side.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostForm {
    pub side: Option<Side>,
    /// Alias before editing; `None` for a new host.
    pub original: Option<String>,
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: String,
    pub identity_file: String,
    pub note: String,
    /// One `Keyword value` per line.
    pub options: String,
    pub unmonitored: bool,
    pub rdp: bool,
    /// Group / cluster names (#144, #145); empty = none.
    pub group: String,
    pub cluster: String,
    pub error: Option<String>,
}

impl HostForm {
    fn from_entry(side: Side, h: Option<&HostEntry>) -> Self {
        let h = h.cloned().unwrap_or_else(|| HostEntry::new(""));
        Self {
            side: Some(side),
            original: (!h.alias.is_empty()).then(|| h.alias.clone()),
            alias: h.alias.clone(),
            hostname: h.hostname.clone(),
            user: h.user.clone(),
            port: h.port.to_string(),
            identity_file: h.identity_file.clone().unwrap_or_default(),
            note: h.note.clone(),
            options: store::options_text(&h.options),
            unmonitored: h.unmonitored,
            rdp: h.access == Access::Rdp,
            group: h.group.clone(),
            cluster: h.cluster.clone(),
            error: None,
        }
    }

    fn entry(&self) -> Result<HostEntry, String> {
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| "Port must be 1-65535".to_string())?;
        let id = self.identity_file.trim();
        let h = HostEntry {
            id: None,
            alias: self.alias.trim().to_string(),
            hostname: self.hostname.trim().to_string(),
            user: self.user.trim().to_string(),
            port,
            identity_file: (!id.is_empty()).then(|| id.to_string()),
            note: self.note.trim().to_string(),
            options: store::decode_options(&self.options),
            unmonitored: self.unmonitored,
            access: if self.rdp { Access::Rdp } else { Access::Ssh },
            group: self.group.trim().to_string(),
            cluster: self.cluster.trim().to_string(),
        };
        h.validate()?;
        Ok(h)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HostSyncDraft {
    pub filter: String,
    pub only_differences: bool,
    pub selected: BTreeSet<String>,
    pub form: Option<HostForm>,
}

#[derive(Debug, Clone)]
pub enum HsMsg {
    Open,
    Close,
    /// File (false) or folder (true) dialog.
    Browse {
        folder: bool,
    },
    Picked(String),
    Reload,
    Loaded {
        path: String,
        result: Result<String, String>,
    },
    /// Copy hosts from Telepad to the config.
    ToConfig(Vec<String>),
    /// Copy hosts from the config to Telepad.
    ToTelepad(Vec<String>),
    Delete {
        side: Side,
        aliases: Vec<String>,
    },
    /// All hosts from one side to the other.
    Bulk {
        from: Side,
        mode: Mode,
    },
    /// Both sides get every host; differences wait for a choice.
    Sync,
    /// Drop the pending changes of these aliases.
    Undo(Vec<String>),
    Discard,
    Edit {
        side: Side,
        alias: String,
    },
    Add(Side),
    FormSave,
    FormCancel,
    Preview,
    PreviewClose,
    Apply,
    Applied(Result<String, String>),
}

#[derive(Debug, Clone)]
pub enum HsEffect {
    /// Read the file (`None`: the saved path, else `~/.ssh/config`; a folder
    /// means its `config`); `remember` saves the path → [`HsMsg::Loaded`].
    Load {
        path: Option<String>,
        remember: bool,
    },
    /// File / folder dialog → [`HsMsg::Picked`] (nothing if cancelled).
    Pick { folder: bool },
    /// Write the config (if `config_text`), then Telepad changes in one
    /// transaction → [`HsMsg::Applied`].
    Apply {
        path: String,
        expected: String,
        config_text: Option<String>,
        upserts: Vec<HostEntry>,
        deletes: Vec<i64>,
    },
}

/// Telepad's hosts with the pending changes applied.
fn effective_telepad(db: &[HostEntry], hs: &HostSync) -> Vec<HostEntry> {
    let mut out: Vec<HostEntry> = Vec::new();
    for h in db {
        match hs.telepad.get(&h.alias) {
            Some(Pending::Delete) => {}
            Some(Pending::Upsert(n)) => out.push(HostEntry {
                id: h.id,
                ..(**n).clone()
            }),
            None => out.push(h.clone()),
        }
    }
    for (k, p) in &hs.telepad {
        if let Pending::Upsert(n) = p
            && !db.iter().any(|h| &h.alias == k)
        {
            out.push((**n).clone());
        }
    }
    out
}

fn config_changes(hs: &HostSync) -> Vec<ConfigChange> {
    hs.config
        .iter()
        .map(|(k, p)| match p {
            Pending::Upsert(h) => ConfigChange::Upsert((**h).clone()),
            Pending::Delete => ConfigChange::Delete(k.clone()),
        })
        .collect()
}

/// The config text with the pending changes.
fn config_text(hs: &HostSync) -> Result<Option<String>, String> {
    let Some(file) = &hs.file else {
        return Ok(None);
    };
    if hs.config.is_empty() {
        return Ok(None);
    }
    hostsync::apply(file, &config_changes(hs)).map(Some)
}

/// Recompute the rows; called after anything that changes either side.
pub fn refresh(hs: &mut HostSync, db: &[HostEntry]) {
    let Some(file) = &hs.file else {
        hs.rows.clear();
        return;
    };
    let shown = match config_text(hs) {
        Ok(Some(t)) => ConfigFile::parse(&t),
        Ok(None) => file.clone(),
        Err(e) => {
            hs.error = Some(e);
            file.clone()
        }
    };
    hs.rows = hostsync::compare(&effective_telepad(db, hs), &shown);
    let still: BTreeSet<String> = hs
        .rows
        .iter()
        .filter(|r| r.state == RowState::Different)
        .map(|r| r.alias.clone())
        .collect();
    hs.conflicts.retain(|a| still.contains(a));
}

fn db_entry<'a>(db: &'a [HostEntry], hs: &'a HostSync, alias: &str) -> Option<HostEntry> {
    effective_telepad(db, hs)
        .into_iter()
        .find(|h| h.alias == alias)
}

fn cfg_entry(hs: &HostSync, alias: &str) -> Option<HostEntry> {
    hs.rows
        .iter()
        .find(|r| r.alias == alias)
        .and_then(|r| r.cfg.clone())
}

fn to_config(hs: &mut HostSync, db: &[HostEntry], alias: &str) -> Result<(), String> {
    let Some(h) = db_entry(db, hs, alias) else {
        return Ok(());
    };
    if hs
        .file
        .as_ref()
        .is_some_and(|f| f.shared_aliases().contains(alias))
    {
        return Err(format!(
            "{alias} is on a shared Host line in the config; change it there by hand"
        ));
    }
    hs.config.insert(
        alias.to_string(),
        Pending::Upsert(Box::new(HostEntry { id: None, ..h })),
    );
    hs.conflicts.remove(alias);
    Ok(())
}

fn to_telepad(hs: &mut HostSync, alias: &str) {
    let Some(h) = cfg_entry(hs, alias) else {
        return;
    };
    hs.telepad
        .insert(alias.to_string(), Pending::Upsert(Box::new(h)));
    hs.conflicts.remove(alias);
}

/// Statements of the preview and the Apply effect.
fn plan(hs: &HostSync, db: &[HostEntry]) -> Result<(Preview, Vec<HostEntry>, Vec<i64>), String> {
    let text = config_text(hs)?;
    let diff = match (&text, &hs.file) {
        (Some(t), Some(f)) => hostsync::line_diff(&f.text, t),
        _ => vec![],
    };
    let mut upserts = Vec::new();
    let mut deletes = Vec::new();
    let mut lines = Vec::new();
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    for h in effective_telepad(db, hs) {
        if !aliases.insert(h.alias.clone()) {
            return Err(format!("Two Telepad hosts would be called {}", h.alias));
        }
    }
    for (k, p) in &hs.telepad {
        let id = db.iter().find(|h| &h.alias == k).and_then(|h| h.id);
        match p {
            Pending::Delete => {
                if let Some(id) = id {
                    deletes.push(id);
                    lines.push(format!("delete {k}"));
                }
            }
            Pending::Upsert(h) => {
                h.validate().map_err(|e| format!("{}: {e}", h.alias))?;
                lines.push(match (id, h.alias == *k) {
                    (Some(_), true) => format!("update {k} ({})", h.target()),
                    (Some(_), false) => format!("rename {k} → {} ({})", h.alias, h.target()),
                    (None, _) => format!("add {} ({})", h.alias, h.target()),
                });
                upserts.push(HostEntry {
                    id,
                    ..(**h).clone()
                });
            }
        }
    }
    Ok((
        Preview {
            config_text: text,
            diff,
            telepad: lines,
        },
        upserts,
        deletes,
    ))
}

/// Apply one Host Management message. `db` are Telepad's hosts as loaded.
pub fn update(
    hs_slot: &mut Option<HostSync>,
    d: &mut HostSyncDraft,
    db: &[HostEntry],
    msg: HsMsg,
) -> (Vec<HsEffect>, Vec<String>) {
    // (effects, follow-ups: "reload-hosts" after Apply)
    let mut fx = Vec::new();
    let mut follow = Vec::new();
    if let HsMsg::Open = msg {
        if hs_slot.is_none() {
            *hs_slot = Some(HostSync {
                loading: true,
                ..Default::default()
            });
            *d = HostSyncDraft::default();
            fx.push(HsEffect::Load {
                path: None,
                remember: false,
            });
        }
        return (fx, follow);
    }
    let Some(hs) = hs_slot.as_mut() else {
        return (fx, follow);
    };
    match msg {
        HsMsg::Open => {}
        HsMsg::Close => {
            *hs_slot = None;
            *d = HostSyncDraft::default();
            return (fx, follow);
        }
        HsMsg::Browse { folder } => fx.push(HsEffect::Pick { folder }),
        HsMsg::Picked(path) => {
            if hs.pending_count() > 0 {
                hs.error = Some(
                    "Apply or discard the pending changes before choosing another file".into(),
                );
            } else {
                hs.loading = true;
                fx.push(HsEffect::Load {
                    path: Some(path),
                    remember: true,
                });
            }
        }
        HsMsg::Reload => {
            // Pending changes are per alias, so they still apply to the new text.
            hs.loading = true;
            hs.preview = None;
            fx.push(HsEffect::Load {
                path: Some(hs.path.clone()),
                remember: false,
            });
        }
        HsMsg::Loaded { path, result } => {
            hs.loading = false;
            hs.path = path;
            match result {
                Ok(text) => {
                    hs.file = Some(ConfigFile::parse(&text));
                    hs.error = None;
                }
                Err(e) => {
                    hs.file = None;
                    hs.error = Some(e);
                }
            }
        }
        HsMsg::ToConfig(aliases) => {
            for a in &aliases {
                if let Err(e) = to_config(hs, db, a) {
                    hs.error = Some(e);
                }
            }
        }
        HsMsg::ToTelepad(aliases) => {
            for a in &aliases {
                to_telepad(hs, a);
            }
        }
        HsMsg::Delete { side, aliases } => {
            for a in aliases {
                match side {
                    Side::Telepad => {
                        if db.iter().any(|h| h.alias == a) {
                            hs.telepad.insert(a, Pending::Delete);
                        } else {
                            hs.telepad.remove(&a);
                        }
                    }
                    Side::Config => {
                        if hs
                            .file
                            .as_ref()
                            .is_some_and(|f| f.hosts().iter().any(|h| h.alias == a))
                        {
                            hs.config.insert(a, Pending::Delete);
                        } else {
                            hs.config.remove(&a);
                        }
                    }
                }
            }
        }
        HsMsg::Bulk { from, mode } => {
            let rows = hs.rows.clone();
            for r in &rows {
                let (src, dst) = match from {
                    Side::Telepad => (&r.db, &r.cfg),
                    Side::Config => (&r.cfg, &r.db),
                };
                match (src, dst) {
                    (Some(_), _) if r.state != RowState::Same => match from {
                        Side::Telepad => {
                            if !r.cfg_read_only {
                                let _ = to_config(hs, db, &r.alias);
                            }
                        }
                        Side::Config => to_telepad(hs, &r.alias),
                    },
                    (None, Some(_)) if mode == Mode::Replace && !r.cfg_read_only => match from {
                        Side::Telepad => {
                            hs.config.insert(r.alias.clone(), Pending::Delete);
                        }
                        Side::Config => {
                            hs.telepad.insert(r.alias.clone(), Pending::Delete);
                        }
                    },
                    _ => {}
                }
            }
            hs.conflicts.clear();
        }
        HsMsg::Sync => {
            let rows = hs.rows.clone();
            for r in &rows {
                match r.state {
                    RowState::OnlyTelepad if !r.cfg_read_only => {
                        let _ = to_config(hs, db, &r.alias);
                    }
                    RowState::OnlyConfig => to_telepad(hs, &r.alias),
                    RowState::Different => {
                        hs.conflicts.insert(r.alias.clone());
                    }
                    _ => {}
                }
            }
        }
        HsMsg::Undo(aliases) => {
            for a in &aliases {
                hs.telepad.remove(a);
                hs.config.remove(a);
                hs.conflicts.remove(a);
                // A renamed Telepad host is pending under its old alias.
                hs.telepad
                    .retain(|_, p| !matches!(p, Pending::Upsert(h) if &h.alias == a));
            }
        }
        HsMsg::Discard => {
            hs.telepad.clear();
            hs.config.clear();
            hs.conflicts.clear();
            hs.preview = None;
            hs.error = None;
        }
        HsMsg::Edit { side, alias } => {
            let h = match side {
                Side::Telepad => db_entry(db, hs, &alias),
                Side::Config => cfg_entry(hs, &alias),
            };
            d.form = Some(HostForm::from_entry(side, h.as_ref()));
        }
        HsMsg::Add(side) => {
            let mut f = HostForm::from_entry(side, None);
            f.port = "22".into();
            d.form = Some(f);
        }
        HsMsg::FormCancel => d.form = None,
        HsMsg::FormSave => {
            let Some(form) = d.form.as_mut() else {
                return (fx, follow);
            };
            let entry = match form.entry() {
                Ok(e) => e,
                Err(e) => {
                    form.error = Some(e);
                    return (fx, follow);
                }
            };
            let side = form.side.unwrap_or(Side::Telepad);
            let exists = match side {
                Side::Telepad => db_entry(db, hs, &entry.alias).is_some(),
                Side::Config => cfg_entry(hs, &entry.alias).is_some(),
            };
            if exists && form.original.as_deref() != Some(entry.alias.as_str()) {
                form.error = Some(format!(
                    "{} already exists in {}",
                    entry.alias,
                    side.title()
                ));
                return (fx, follow);
            }
            match side {
                Side::Telepad => {
                    // Key by the alias the host has in the database, so a rename
                    // updates the same row.
                    let key = form
                        .original
                        .as_ref()
                        .and_then(|o| {
                            hs.telepad
                                .iter()
                                .find(|(_, p)| matches!(p, Pending::Upsert(h) if &h.alias == o))
                                .map(|(k, _)| k.clone())
                                .or_else(|| Some(o.clone()))
                        })
                        .unwrap_or_else(|| entry.alias.clone());
                    hs.telepad.insert(key, Pending::Upsert(Box::new(entry)));
                }
                Side::Config => {
                    if hs
                        .file
                        .as_ref()
                        .is_some_and(|f| f.shared_aliases().contains(&entry.alias))
                    {
                        form.error = Some(format!(
                            "{} is on a shared Host line in the config; change it there by hand",
                            entry.alias
                        ));
                        return (fx, follow);
                    }
                    if let Some(o) = form.original.as_ref().filter(|o| **o != entry.alias) {
                        if hs
                            .file
                            .as_ref()
                            .is_some_and(|f| f.hosts().iter().any(|h| &h.alias == o))
                        {
                            hs.config.insert(o.clone(), Pending::Delete);
                        } else {
                            hs.config.remove(o);
                        }
                    }
                    hs.config
                        .insert(entry.alias.clone(), Pending::Upsert(Box::new(entry)));
                }
            }
            d.form = None;
        }
        HsMsg::Preview => {
            if !hs.conflicts.is_empty() {
                hs.error = Some(format!(
                    "Choose a side for {} host(s) that differ: click → or ← in their rows",
                    hs.conflicts.len()
                ));
            } else {
                match plan(hs, db) {
                    Ok((p, _, _)) => {
                        hs.error = None;
                        hs.preview = Some(p);
                    }
                    Err(e) => hs.error = Some(e),
                }
            }
        }
        HsMsg::PreviewClose => hs.preview = None,
        HsMsg::Apply => {
            if hs.applying || !hs.conflicts.is_empty() {
                return (fx, follow);
            }
            match (plan(hs, db), &hs.file) {
                (Ok((p, upserts, deletes)), Some(file)) => {
                    hs.applying = true;
                    fx.push(HsEffect::Apply {
                        path: hs.path.clone(),
                        expected: file.text.clone(),
                        config_text: p.config_text,
                        upserts,
                        deletes,
                    });
                }
                (Err(e), _) => hs.error = Some(e),
                (_, None) => hs.error = Some("The config file is not loaded".into()),
            }
        }
        HsMsg::Applied(result) => {
            hs.applying = false;
            match result {
                Ok(summary) => {
                    hs.telepad.clear();
                    hs.config.clear();
                    hs.conflicts.clear();
                    hs.preview = None;
                    hs.error = None;
                    hs.notice = Some(summary);
                    d.selected.clear();
                    hs.loading = true;
                    fx.push(HsEffect::Load {
                        path: Some(hs.path.clone()),
                        remember: false,
                    });
                    follow.push("reload-hosts".into());
                }
                Err(e) => {
                    hs.preview = None;
                    hs.error = Some(e);
                }
            }
        }
    }
    refresh(hs, db);
    (fx, follow)
}
