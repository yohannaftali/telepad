//! SQLite host inventory (`telepad.db`). Single file, no server; editable in the
//! app or with any SQLite tool. Import/export: OpenSSH config text or JSON.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::config::{self, Access, HostEntry, HostSet, Inventory, SetKind};
use crate::update::UpdateConfig;

const SCHEMA_VERSION: i32 = 6;
const JSON_FORMAT: &str = "telepad/hosts";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
    permission_warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// Add new aliases, update existing ones, keep the rest.
    Merge,
    /// Make the inventory exactly the imported file.
    Replace,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
}

/// Colour theme: follow the OS, or always light / dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }
}

/// View settings kept in the database so they survive restarts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiPrefs {
    /// One line per host in the host list (default), or the detailed three-line rows.
    pub compact_host_list: bool,
    /// Host list grouped into Down / Up / Not checked yet.
    pub group_by_status: bool,
    /// Overview as cards instead of the table.
    pub overview_cards: bool,
    pub theme: ThemePref,
    /// Page sidebar shows icons only (#77).
    pub sidebar_collapsed: bool,
    /// Inner windows of the workspace (#77), as written by the app's
    /// `Workspace::save`; empty = default layout.
    pub workspace: String,
    /// Editor colour scheme (#183) by its key; empty = follow the app theme.
    pub editor_scheme: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            compact_host_list: true,
            group_by_status: false,
            overview_cards: false,
            theme: ThemePref::System,
            sidebar_collapsed: false,
            workspace: String::new(),
            editor_scheme: String::new(),
        }
    }
}

/// Saved database connections (Database browser). Never holds a password:
/// that lives in the OS password store (`db::secret`).
const DB_CONNECTIONS: &str = "CREATE TABLE IF NOT EXISTS db_connections (
    key           TEXT PRIMARY KEY,
    user          TEXT    NOT NULL DEFAULT '',
    database      TEXT    NOT NULL DEFAULT '',
    save_password INTEGER NOT NULL DEFAULT 0,
    read_only     INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP
);";

/// Saved Remote Desktop settings per host (#102), as JSON. Follows the
/// host's alias (rename) and goes with the host (delete). No password.
const RDP_SETTINGS: &str = "CREATE TABLE IF NOT EXISTS rdp_settings (
    alias      TEXT PRIMARY KEY REFERENCES hosts(alias) ON UPDATE CASCADE ON DELETE CASCADE,
    settings   TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);";

/// Groups and clusters (#144, #145): name, description and display order.
/// Hosts refer to them by name (`hosts.grp` / `hosts.cluster`).
const HOST_SETS: &str = "CREATE TABLE IF NOT EXISTS host_sets (
    kind        TEXT    NOT NULL,
    name        TEXT    NOT NULL COLLATE NOCASE,
    description TEXT    NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, name)
);";

/// Registered Portainer servers (#171): reached through the tunnel of
/// `alias`, so they follow the host's alias and go with the host. The API
/// token is in the OS password store, never here.
const PORTAINERS: &str = "CREATE TABLE IF NOT EXISTS portainers (
    name        TEXT    NOT NULL PRIMARY KEY COLLATE NOCASE,
    alias       TEXT    NOT NULL REFERENCES hosts(alias) ON UPDATE CASCADE ON DELETE CASCADE,
    port        INTEGER NOT NULL,
    https       INTEGER NOT NULL DEFAULT 1,
    fingerprint TEXT    NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0
);";

/// The `hosts` column holding a host's name of `kind`.
fn set_column(kind: SetKind) -> &'static str {
    match kind {
        SetKind::Group => "grp",
        SetKind::Cluster => "cluster",
    }
}

#[derive(Serialize)]
struct JsonFile {
    format: String,
    version: i32,
    #[serde(flatten)]
    inventory: Inventory,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let permission_warning = crate::platform::restrict_db_files(path).err();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS hosts (
                 id            INTEGER PRIMARY KEY,
                 alias         TEXT    NOT NULL UNIQUE,
                 hostname      TEXT    NOT NULL DEFAULT '',
                 user          TEXT    NOT NULL DEFAULT '',
                 port          INTEGER NOT NULL DEFAULT 22,
                 identity_file TEXT,
                 description   TEXT    NOT NULL DEFAULT '',
                 -- one 'Keyword value' per line
                 options       TEXT    NOT NULL DEFAULT '',
                 monitored     INTEGER NOT NULL DEFAULT 1,
                 access        TEXT    NOT NULL DEFAULT 'ssh',
                 sort_order    INTEGER NOT NULL DEFAULT 0,
                 grp           TEXT    NOT NULL DEFAULT '',
                 cluster       TEXT    NOT NULL DEFAULT '',
                 updated_at    TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        let mut v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v == 1 {
            // v2: hosts.access ('ssh' | 'rdp')
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE hosts ADD COLUMN access TEXT NOT NULL DEFAULT 'ssh';
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
            v = 2;
        }
        if v == 2 {
            // v3: saved database connections (#52); passwords are not stored here.
            conn.execute_batch(&format!(
                "BEGIN;
{DB_CONNECTIONS}
PRAGMA user_version = 3;
COMMIT;"
            ))?;
            v = 3;
        }
        if v == 3 {
            // v4: saved Remote Desktop settings (#102).
            conn.execute_batch(&format!(
                "BEGIN;
{RDP_SETTINGS}
PRAGMA user_version = 4;
COMMIT;"
            ))?;
            v = 4;
        }
        if v == 4 {
            // v5: host groups and clusters (#144, #145).
            let has = |col: &str| -> Result<bool, rusqlite::Error> {
                conn.query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('hosts') WHERE name = ?1",
                    params![col],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n > 0)
            };
            let mut add = String::new();
            for col in ["grp", "cluster"] {
                if !has(col)? {
                    add.push_str(&format!(
                        "ALTER TABLE hosts ADD COLUMN {col} TEXT NOT NULL DEFAULT '';\n"
                    ));
                }
            }
            conn.execute_batch(&format!(
                "BEGIN;
{add}{HOST_SETS}
PRAGMA user_version = 5;
COMMIT;"
            ))?;
            v = 5;
        }
        if v == 5 {
            // v6: registered Portainer servers (#171).
            conn.execute_batch(&format!(
                "BEGIN;
{PORTAINERS}
PRAGMA user_version = 6;
COMMIT;"
            ))?;
        } else if v == 0 {
            conn.execute_batch(&format!(
                "{DB_CONNECTIONS}
{RDP_SETTINGS}
{HOST_SETS}
{PORTAINERS}
PRAGMA user_version = {SCHEMA_VERSION};"
            ))?;
        } else if v > SCHEMA_VERSION {
            return Err(StoreError::Invalid(format!(
                "{} was written by a newer Telepad (schema {v})",
                path.display()
            )));
        }
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            permission_warning,
        })
    }

    /// Why the database file stays readable by others (#110), if it does.
    pub fn permission_warning(&self) -> Option<&str> {
        self.permission_warning.as_deref()
    }

    /// Saved database connection for a [`crate::db::DbTarget::key`].
    pub fn db_profile(&self, key: &str) -> Result<Option<crate::db::Profile>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT key, user, database, save_password, read_only FROM db_connections WHERE key = ?1",
        )?;
        let mut rows = st.query([key])?;
        Ok(match rows.next()? {
            Some(r) => Some(crate::db::Profile {
                key: r.get(0)?,
                user: r.get(1)?,
                database: r.get(2)?,
                save_password: r.get::<_, i64>(3)? != 0,
                read_only: r.get::<_, i64>(4)? != 0,
            }),
            None => None,
        })
    }

    pub fn save_db_profile(&self, p: &crate::db::Profile) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO db_connections (key, user, database, save_password, read_only, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET user = ?2, database = ?3, save_password = ?4,
                 read_only = ?5, updated_at = CURRENT_TIMESTAMP",
            rusqlite::params![
                p.key,
                p.user,
                p.database,
                p.save_password as i64,
                p.read_only as i64
            ],
        )?;
        Ok(())
    }

    pub fn delete_db_profile(&self, key: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM db_connections WHERE key = ?1", [key])?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_empty(&self) -> Result<bool, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM hosts", [], |r| r.get(0))?;
        Ok(n == 0)
    }

    pub fn hosts(&self) -> Result<Vec<HostEntry>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT id, alias, hostname, user, port, identity_file, description, options, monitored,
                    access, grp, cluster
             FROM hosts ORDER BY sort_order, id",
        )?;
        let rows = st.query_map([], |r| {
            let options: String = r.get(7)?;
            Ok(HostEntry {
                id: Some(r.get(0)?),
                alias: r.get(1)?,
                hostname: r.get(2)?,
                user: r.get(3)?,
                port: r.get::<_, i64>(4)?.clamp(1, 65535) as u16,
                identity_file: r.get(5)?,
                note: r.get(6)?,
                options: decode_options(&options),
                unmonitored: r.get::<_, i64>(8)? == 0,
                access: Access::from_db(&r.get::<_, String>(9)?),
                group: r.get(10)?,
                cluster: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn global(&self) -> Result<String, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'global_ssh'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn set_global(&self, text: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES('global_ssh', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![text],
        )?;
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Self-update settings (`update_*` keys); missing keys use the defaults.
    pub fn update_config(&self) -> Result<UpdateConfig, StoreError> {
        let d = UpdateConfig::default();
        Ok(UpdateConfig {
            enabled: self
                .setting("update_enabled")?
                .map_or(d.enabled, |v| v != "0"),
            repo: self
                .setting("update_repo")?
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(d.repo),
            auto_install: self
                .setting("update_auto_install")?
                .map_or(d.auto_install, |v| v == "1"),
        })
    }

    pub fn set_update_config(&self, c: &UpdateConfig) -> Result<(), StoreError> {
        c.validate().map_err(StoreError::Invalid)?;
        let rows = [
            (
                "update_enabled",
                if c.enabled { "1" } else { "0" }.to_string(),
            ),
            ("update_repo", c.repo.clone()),
            (
                "update_auto_install",
                if c.auto_install { "1" } else { "0" }.to_string(),
            ),
        ];
        let tx = self.conn.unchecked_transaction()?;
        for (k, v) in rows {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remembered view settings (`ui_*` keys); missing keys use the defaults.
    pub fn ui_prefs(&self) -> Result<UiPrefs, StoreError> {
        let d = UiPrefs::default();
        let flag = |key: &str, default: bool| -> Result<bool, StoreError> {
            Ok(self.setting(key)?.map_or(default, |v| v != "0"))
        };
        Ok(UiPrefs {
            compact_host_list: flag("ui_compact_host_list", d.compact_host_list)?,
            group_by_status: flag("ui_group_by_status", d.group_by_status)?,
            overview_cards: flag("ui_overview_cards", d.overview_cards)?,
            theme: self
                .setting("ui_theme")?
                .map_or(d.theme, |v| ThemePref::parse(&v)),
            sidebar_collapsed: flag("ui_sidebar_collapsed", d.sidebar_collapsed)?,
            workspace: self.setting("ui_workspace")?.unwrap_or_default(),
            editor_scheme: self.setting("ui_editor_scheme")?.unwrap_or_default(),
        })
    }

    pub fn set_ui_prefs(&self, p: &UiPrefs) -> Result<(), StoreError> {
        let flag = |v: bool| if v { "1" } else { "0" };
        let rows = [
            ("ui_compact_host_list", flag(p.compact_host_list)),
            ("ui_group_by_status", flag(p.group_by_status)),
            ("ui_overview_cards", flag(p.overview_cards)),
            ("ui_theme", p.theme.as_str()),
            ("ui_sidebar_collapsed", flag(p.sidebar_collapsed)),
            ("ui_workspace", p.workspace.as_str()),
            ("ui_editor_scheme", p.editor_scheme.as_str()),
        ];
        let tx = self.conn.unchecked_transaction()?;
        for (k, v) in rows {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn inventory(&self) -> Result<Inventory, StoreError> {
        Ok(Inventory {
            hosts: self.hosts()?,
            global: self.global()?,
            rdp: self.rdp_all()?,
            sets: SetKind::ALL
                .iter()
                .map(|&k| Ok((k, self.sets(k)?)))
                .collect::<Result<_, StoreError>>()?,
            portainers: self.portainers()?,
        })
    }

    /// Registered Portainer servers (#171), in display order.
    pub fn portainers(&self) -> Result<Vec<crate::portainer::Portainer>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT name, alias, port, https, fingerprint FROM portainers ORDER BY position, name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(crate::portainer::Portainer {
                name: r.get(0)?,
                alias: r.get(1)?,
                port: r.get::<_, i64>(2)?.clamp(1, 65535) as u16,
                https: r.get::<_, i64>(3)? != 0,
                fingerprint: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Add a Portainer (`old` = None, last in the list) or change one
    /// (rename too); the fingerprint is kept unless the address changed.
    pub fn save_portainer(
        &mut self,
        old: Option<&str>,
        p: &crate::portainer::Portainer,
    ) -> Result<(), StoreError> {
        crate::portainer::check_name(&p.name).map_err(StoreError::Invalid)?;
        if p.port == 0 {
            return Err(StoreError::Invalid("A port is needed".into()));
        }
        let name = p.name.trim();
        let tx = self.conn.transaction()?;
        let taken: i64 = tx.query_row(
            "SELECT COUNT(*) FROM portainers WHERE name = ?1 AND (?2 IS NULL OR name <> ?2)",
            params![name, old],
            |r| r.get(0),
        )?;
        if taken > 0 {
            return Err(StoreError::Invalid(format!(
                "There is already a Portainer named '{name}'"
            )));
        }
        match old {
            Some(o) => {
                let n = tx.execute(
                    "UPDATE portainers SET name = ?1, alias = ?2, port = ?3, https = ?4,
                       fingerprint = CASE WHEN alias = ?2 AND port = ?3 AND https = ?4
                                     THEN fingerprint ELSE '' END
                     WHERE name = ?5",
                    params![name, p.alias, p.port, p.https, o],
                )?;
                if n == 0 {
                    return Err(StoreError::Invalid(format!(
                        "Portainer '{o}' no longer exists"
                    )));
                }
            }
            None => {
                tx.execute(
                    "INSERT INTO portainers(name, alias, port, https, fingerprint, position)
                     VALUES(?1, ?2, ?3, ?4, ?5,
                       (SELECT COALESCE(MAX(position), 0) + 1 FROM portainers))",
                    params![name, p.alias, p.port, p.https, p.fingerprint],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_portainer(&mut self, name: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM portainers WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Pin the certificate seen on the first read, or the one accepted.
    pub fn pin_portainer(&mut self, name: &str, fingerprint: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE portainers SET fingerprint = ?2 WHERE name = ?1",
            params![name, fingerprint],
        )?;
        Ok(())
    }

    /// Saved Remote Desktop settings of every host (#102), by alias. A row
    /// that does not read (hand-edited) is left out.
    pub fn rdp_all(
        &self,
    ) -> Result<std::collections::BTreeMap<String, crate::rdp::RdpSettings>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT alias, settings FROM rdp_settings")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = std::collections::BTreeMap::new();
        for row in rows {
            let (alias, json) = row?;
            if let Ok(s) = serde_json::from_str(&json) {
                out.insert(alias, s);
            }
        }
        Ok(out)
    }

    /// Save (`Some`) or forget (`None`) a host's Remote Desktop settings.
    pub fn set_rdp(
        &self,
        alias: &str,
        settings: Option<&crate::rdp::RdpSettings>,
    ) -> Result<(), StoreError> {
        match settings {
            Some(s) => {
                s.validate().map_err(StoreError::Invalid)?;
                self.conn.execute(
                    "INSERT INTO rdp_settings(alias, settings) VALUES(?1, ?2)
                     ON CONFLICT(alias) DO UPDATE SET settings = excluded.settings,
                       updated_at = CURRENT_TIMESTAMP",
                    params![alias, serde_json::to_string(s)?],
                )?;
            }
            None => {
                self.conn
                    .execute("DELETE FROM rdp_settings WHERE alias = ?1", params![alias])?;
            }
        }
        Ok(())
    }

    /// Insert (no id) or update (id) one host. Returns its id.
    pub fn save_host(&self, h: &HostEntry) -> Result<i64, StoreError> {
        h.validate().map_err(StoreError::Invalid)?;
        let clash: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM hosts WHERE alias = ?1",
                params![h.alias],
                |r| r.get(0),
            )
            .optional()?;
        if clash.is_some() && clash != h.id {
            return Err(StoreError::Invalid(format!(
                "Another host already uses the alias '{}'",
                h.alias
            )));
        }
        let opts = encode_options(&h.options);
        match h.id {
            Some(id) => {
                let n = self.conn.execute(
                    "UPDATE hosts SET alias=?2, hostname=?3, user=?4, port=?5, identity_file=?6,
                     description=?7, options=?8, monitored=?9, access=?10,
                     grp=?11, cluster=?12, updated_at=CURRENT_TIMESTAMP
                     WHERE id=?1",
                    params![
                        id,
                        h.alias,
                        h.hostname,
                        h.user,
                        h.port,
                        h.identity_file,
                        h.note,
                        opts,
                        !h.unmonitored,
                        h.access.as_str(),
                        h.group,
                        h.cluster
                    ],
                )?;
                ensure_sets(&self.conn, h)?;
                if n == 0 {
                    return Err(StoreError::Invalid(format!("host #{id} no longer exists")));
                }
                Ok(id)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO hosts(alias, hostname, user, port, identity_file, description,
                     options, monitored, access, grp, cluster, sort_order)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                            (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM hosts))",
                    params![
                        h.alias,
                        h.hostname,
                        h.user,
                        h.port,
                        h.identity_file,
                        h.note,
                        opts,
                        !h.unmonitored,
                        h.access.as_str(),
                        h.group,
                        h.cluster
                    ],
                )?;
                let id = self.conn.last_insert_rowid();
                ensure_sets(&self.conn, h)?;
                Ok(id)
            }
        }
    }

    /// Groups or clusters in display order (#144, #145).
    pub fn sets(&self, kind: SetKind) -> Result<Vec<HostSet>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT name, description FROM host_sets WHERE kind = ?1 ORDER BY position, name",
        )?;
        let rows = st.query_map(params![kind.as_str()], |r| {
            Ok(HostSet {
                name: r.get(0)?,
                description: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Add a group / cluster (`old` = None) or rename / describe one; a
    /// rename moves its hosts along, in one transaction.
    pub fn save_set(
        &mut self,
        kind: SetKind,
        old: Option<&str>,
        set: &HostSet,
    ) -> Result<(), StoreError> {
        config::check_set_name(kind, &set.name).map_err(StoreError::Invalid)?;
        if set.description.contains(['\n', '\r']) {
            return Err(StoreError::Invalid(
                "Description must be a single line".into(),
            ));
        }
        let tx = self.conn.transaction()?;
        let clash: Option<String> = tx
            .query_row(
                "SELECT name FROM host_sets WHERE kind = ?1 AND name = ?2",
                params![kind.as_str(), set.name],
                |r| r.get(0),
            )
            .optional()?;
        if clash.is_some_and(|c| old.is_none_or(|o| !c.eq_ignore_ascii_case(o))) {
            return Err(StoreError::Invalid(format!(
                "There is already a {} named '{}'",
                kind.as_str(),
                set.name
            )));
        }
        match old {
            Some(o) => {
                let n = tx.execute(
                    "UPDATE host_sets SET name = ?3, description = ?4 WHERE kind = ?1 AND name = ?2",
                    params![kind.as_str(), o, set.name, set.description],
                )?;
                if n == 0 {
                    return Err(StoreError::Invalid(format!("'{o}' no longer exists")));
                }
                tx.execute(
                    &format!("UPDATE hosts SET {0} = ?2 WHERE {0} = ?1", set_column(kind)),
                    params![o, set.name],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO host_sets(kind, name, description, position)
                     VALUES(?1, ?2, ?3, (SELECT COALESCE(MAX(position), 0) + 1 FROM host_sets
                                         WHERE kind = ?1))",
                    params![kind.as_str(), set.name, set.description],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete a group / cluster; its hosts stay, without one.
    pub fn delete_set(&mut self, kind: SetKind, name: &str) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM host_sets WHERE kind = ?1 AND name = ?2",
            params![kind.as_str(), name],
        )?;
        tx.execute(
            &format!("UPDATE hosts SET {0} = '' WHERE {0} = ?1", set_column(kind)),
            params![name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Display order of the groups / clusters: `names` first to last.
    pub fn order_sets(&mut self, kind: SetKind, names: &[String]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for (i, n) in names.iter().enumerate() {
            tx.execute(
                "UPDATE host_sets SET position = ?3 WHERE kind = ?1 AND name = ?2",
                params![kind.as_str(), n, i as i64 + 1],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Put hosts `ids` into `name` (empty = take them out), in one transaction.
    pub fn assign_set(&mut self, kind: SetKind, ids: &[i64], name: &str) -> Result<(), StoreError> {
        if !name.is_empty() {
            config::check_set_name(kind, name).map_err(StoreError::Invalid)?;
        }
        let tx = self.conn.transaction()?;
        let probe = HostEntry {
            group: if kind == SetKind::Group {
                name.into()
            } else {
                String::new()
            },
            cluster: if kind == SetKind::Cluster {
                name.into()
            } else {
                String::new()
            },
            ..HostEntry::default()
        };
        ensure_sets(&tx, &probe)?;
        for id in ids {
            tx.execute(
                &format!(
                    "UPDATE hosts SET {} = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    set_column(kind)
                ),
                params![id, name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Store the display order: `ids` first to last, in one transaction.
    pub fn save_order(&mut self, ids: &[i64]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE hosts SET sort_order = ?1 WHERE id = ?2",
                params![i as i64 + 1, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Host Management: delete `deletes` (ids), then save `upserts` (new or
    /// updated), all in one transaction.
    pub fn apply_host_changes(
        &self,
        upserts: &[HostEntry],
        deletes: &[i64],
    ) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        for id in deletes {
            self.delete_host(*id)?;
        }
        for h in upserts {
            self.save_host(h)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// File window: the local and remote folders last open for a host.
    pub fn files_paths(&self, alias: &str) -> Result<Option<(String, String)>, StoreError> {
        Ok(self
            .setting(&format!("files_paths:{alias}"))?
            .and_then(|v| {
                v.split_once('\n')
                    .map(|(l, r)| (l.to_string(), r.to_string()))
            }))
    }

    pub fn set_files_paths(
        &self,
        alias: &str,
        local: &str,
        remote: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("files_paths:{alias}"), format!("{local}\n{remote}")],
        )?;
        Ok(())
    }

    /// Hidden Data grid columns of one table (#138), one per line; the
    /// key comes from `telepad_app::db::hidden_key`.
    pub fn db_hidden(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.setting(key)
    }

    pub fn set_db_hidden(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Transfer settings of a host's file window (#131), as
    /// `files::transfer::Options::to_setting` writes them.
    pub fn files_options(&self, alias: &str) -> Result<Option<String>, StoreError> {
        self.setting(&format!("files_opts:{alias}"))
    }

    pub fn set_files_options(&self, alias: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("files_opts:{alias}"), value],
        )?;
        Ok(())
    }

    /// The config file Host Management compares with (`None`: `~/.ssh/config`).
    pub fn host_sync_path(&self) -> Result<Option<String>, StoreError> {
        self.setting("hostsync_config_path")
    }

    pub fn set_host_sync_path(&self, path: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES('hostsync_config_path', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![path],
        )?;
        Ok(())
    }

    pub fn delete_host(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM hosts WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// What an import would do, without changing anything.
    pub fn preview_import(
        &self,
        inv: &Inventory,
        mode: ImportMode,
    ) -> Result<ImportSummary, StoreError> {
        let existing = self.hosts()?;
        let mut s = ImportSummary::default();
        for h in &inv.hosts {
            match existing.iter().find(|e| e.alias == h.alias) {
                None => s.added += 1,
                Some(e) if same_fields(e, h) => s.unchanged += 1,
                Some(_) => s.updated += 1,
            }
        }
        if mode == ImportMode::Replace {
            s.removed = existing
                .iter()
                .filter(|e| !inv.hosts.iter().any(|h| h.alias == e.alias))
                .count();
        }
        Ok(s)
    }

    /// Apply an import in one transaction: all of it or nothing.
    pub fn import(
        &mut self,
        inv: &Inventory,
        mode: ImportMode,
    ) -> Result<ImportSummary, StoreError> {
        for h in &inv.hosts {
            h.validate()
                .map_err(|e| StoreError::Invalid(format!("{}: {e}", h.alias)))?;
        }
        for (alias, r) in &inv.rdp {
            r.validate()
                .map_err(|e| StoreError::Invalid(format!("{alias} (Remote Desktop): {e}")))?;
        }
        let summary = self.preview_import(inv, mode)?;
        let tx = self.conn.transaction()?;
        if mode == ImportMode::Replace {
            tx.execute("DELETE FROM hosts", [])?;
        }
        // New hosts go after the existing ones, in imported order; an updated
        // host keeps its place (the conflict update leaves sort_order alone).
        let last: i64 =
            tx.query_row("SELECT COALESCE(MAX(sort_order), 0) FROM hosts", [], |r| {
                r.get(0)
            })?;
        for (i, h) in inv.hosts.iter().enumerate() {
            tx.execute(
                "INSERT INTO hosts(alias, hostname, user, port, identity_file, description, options,
                 monitored, sort_order, access, grp, cluster)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(alias) DO UPDATE SET hostname=excluded.hostname, user=excluded.user,
                   port=excluded.port, identity_file=excluded.identity_file,
                   description=excluded.description, options=excluded.options,
                   monitored=excluded.monitored, access=excluded.access,
                   grp=excluded.grp, cluster=excluded.cluster, updated_at=CURRENT_TIMESTAMP",
                params![
                    h.alias,
                    h.hostname,
                    h.user,
                    h.port,
                    h.identity_file,
                    h.note,
                    encode_options(&h.options),
                    !h.unmonitored,
                    last + i as i64 + 1,
                    h.access.as_str(),
                    h.group,
                    h.cluster
                ],
            )?;
        }
        // Descriptions and order of imported groups / clusters, then any name
        // a host uses that has no entry yet.
        for (&kind, sets) in &inv.sets {
            for s in sets {
                tx.execute(
                    "INSERT INTO host_sets(kind, name, description, position)
                     VALUES(?1, ?2, ?3, (SELECT COALESCE(MAX(position), 0) + 1 FROM host_sets
                                         WHERE kind = ?1))
                     ON CONFLICT(kind, name) DO UPDATE SET description = excluded.description",
                    params![kind.as_str(), s.name, s.description],
                )?;
            }
        }
        for h in &inv.hosts {
            ensure_sets(&tx, h)?;
        }
        if mode == ImportMode::Replace || !inv.global.trim().is_empty() {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES('global_ssh', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![inv.global],
            )?;
        }
        for (alias, r) in &inv.rdp {
            if inv.hosts.iter().any(|h| &h.alias == alias) {
                tx.execute(
                    "INSERT INTO rdp_settings(alias, settings) VALUES(?1, ?2)
                     ON CONFLICT(alias) DO UPDATE SET settings = excluded.settings,
                       updated_at = CURRENT_TIMESTAMP",
                    params![alias, serde_json::to_string(r)?],
                )?;
            }
        }
        // Portainers of imported hosts (#171; never a token).
        for p in &inv.portainers {
            if crate::portainer::check_name(&p.name).is_err()
                || p.port == 0
                || !inv.hosts.iter().any(|h| h.alias == p.alias)
            {
                continue;
            }
            tx.execute(
                "INSERT INTO portainers(name, alias, port, https, fingerprint, position)
                 VALUES(?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(position), 0) + 1 FROM portainers))
                 ON CONFLICT(name) DO UPDATE SET alias = excluded.alias, port = excluded.port,
                   https = excluded.https, fingerprint = excluded.fingerprint",
                params![p.name.trim(), p.alias, p.port, p.https, p.fingerprint],
            )?;
        }
        tx.commit()?;
        Ok(summary)
    }

    /// Render the file `ssh -F` reads. Written atomically so a running ssh never
    /// sees a half-written file.
    pub fn write_ssh_config(&self, path: &Path) -> Result<(), StoreError> {
        let text = config::to_ssh_config(&self.inventory()?);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

fn same_fields(a: &HostEntry, b: &HostEntry) -> bool {
    HostEntry {
        id: None,
        ..a.clone()
    } == HostEntry {
        id: None,
        ..b.clone()
    }
}

fn encode_options(o: &[(String, String)]) -> String {
    o.iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn decode_options(s: &str) -> Vec<(String, String)> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once([' ', '\t', '=']) {
            Some((k, v)) => (k.to_string(), v.trim().to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect()
}

pub fn options_text(o: &[(String, String)]) -> String {
    encode_options(o)
}

/// Parse an import file. JSON is anything with a list of hosts: a Telepad
/// export of any version or app name, `{"hosts": [...]}`, or a bare `[...]`,
/// with common field names (`alias`/`name`/`host`, `hostname`/`ip`/`address`,
/// `user`/`username`, `identity_file`/`key`, `description`/`note`, ...).
/// Everything else is read as an OpenSSH client config (e.g. `~/.ssh/config`).
/// Hosts are validated later, before anything is saved.
pub fn parse_import(text: &str) -> Result<Inventory, StoreError> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        parse_json_import(t)
    } else {
        Ok(config::parse_inventory(t))
    }
}

/// Hosts pasted as text. JSON and OpenSSH config text (anything with a `Host`
/// line) are read as by [`parse_import`]; otherwise one host per line,
/// `[alias] [user@]host[:port]`, with blank lines and `#` comments ignored.
/// Unreadable lines and invalid hosts are left out and described in the
/// returned list, so the valid ones can still be imported.
pub fn parse_pasted(text: &str) -> (Inventory, Vec<String>) {
    let t = text.trim_start_matches('\u{feff}').trim();
    let has_host_block = t.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next().is_some_and(|k| k.eq_ignore_ascii_case("host")) && w.next().is_some()
    });
    let (mut inv, mut problems) = if t.starts_with('{') || t.starts_with('[') || has_host_block {
        match parse_import(t) {
            Ok(inv) => (inv, Vec::new()),
            Err(e) => (Inventory::default(), vec![e.to_string()]),
        }
    } else {
        parse_host_lines(t)
    };
    inv.hosts.retain(|h| match h.validate() {
        Ok(()) => true,
        Err(e) => {
            problems.push(format!("{}: {e}", h.alias));
            false
        }
    });
    (inv, problems)
}

/// `[alias] [user@]host[:port]` per line.
fn parse_host_lines(t: &str) -> (Inventory, Vec<String>) {
    let mut inv = Inventory::default();
    let mut problems = Vec::new();
    for (n, line) in t.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let n = n + 1;
        let words: Vec<&str> = line.split_whitespace().collect();
        let (alias, target) = match words.as_slice() {
            [target] => (None, *target),
            [alias, target] => (Some(*alias), *target),
            _ => {
                problems.push(format!("line {n}: expected `[alias] [user@]host[:port]`"));
                continue;
            }
        };
        let (user, rest) = target.rsplit_once('@').unwrap_or(("", target));
        let (host, port) = match rest.rsplit_once(':') {
            // One colon only: `host:port` (an IPv6 address has several).
            Some((h, p)) if !h.contains(':') => match p.parse::<u16>() {
                Ok(p) if p > 0 => (h, p),
                _ => {
                    problems.push(format!("line {n}: port '{p}' is not 1-65535"));
                    continue;
                }
            },
            _ => (rest, 22),
        };
        if host.is_empty() {
            problems.push(format!("line {n}: no host name or address"));
            continue;
        }
        let alias = alias.unwrap_or(host);
        if inv.hosts.iter().any(|h| h.alias == alias) {
            problems.push(format!("line {n}: alias {alias} appears twice"));
            continue;
        }
        inv.hosts.push(HostEntry {
            hostname: host.to_string(),
            user: user.to_string(),
            port,
            ..HostEntry::new(alias)
        });
    }
    (inv, problems)
}

fn parse_json_import(t: &str) -> Result<Inventory, StoreError> {
    let v: serde_json::Value = serde_json::from_str(t)?;
    let (list, global) = match &v {
        serde_json::Value::Array(a) => (a, String::new()),
        serde_json::Value::Object(o) => {
            let hosts = field(o, &["hosts", "servers", "machines", "items"])
                .and_then(|h| h.as_array())
                .ok_or_else(|| StoreError::Invalid("no list of hosts in this JSON".into()))?;
            let global = field(o, &["global"])
                .and_then(|g| g.as_str())
                .unwrap_or_default()
                .to_string();
            (hosts, global)
        }
        _ => return Err(StoreError::Invalid("no list of hosts in this JSON".into())),
    };
    let hosts = list
        .iter()
        .enumerate()
        .map(|(i, h)| {
            host_from_json(h).ok_or_else(|| {
                StoreError::Invalid(format!("host #{}: needs at least a name or address", i + 1))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Telepad's own export keeps Remote Desktop settings by alias (#102).
    let rdp = match &v {
        serde_json::Value::Object(o) => o
            .get("rdp")
            .and_then(|r| serde_json::from_value(r.clone()).ok())
            .unwrap_or_default(),
        _ => Default::default(),
    };
    // Telepad's own export keeps groups / clusters with descriptions (#144).
    let sets = match &v {
        serde_json::Value::Object(o) => o
            .get("sets")
            .and_then(|r| serde_json::from_value(r.clone()).ok())
            .unwrap_or_default(),
        _ => Default::default(),
    };
    // ... and the registered Portainers (#171).
    let portainers = match &v {
        serde_json::Value::Object(o) => o
            .get("portainers")
            .and_then(|r| serde_json::from_value(r.clone()).ok())
            .unwrap_or_default(),
        _ => Default::default(),
    };
    Ok(Inventory {
        hosts,
        global,
        rdp,
        sets,
        portainers,
    })
}

/// Look up a key ignoring case, `_` and `-` (`IdentityFile` = `identity_file`).
fn field<'a>(
    o: &'a serde_json::Map<String, serde_json::Value>,
    names: &[&str],
) -> Option<&'a serde_json::Value> {
    let norm = |k: &str| {
        k.chars()
            .filter(|c| *c != '_' && *c != '-')
            .collect::<String>()
            .to_ascii_lowercase()
    };
    names.iter().find_map(|n| {
        let n = norm(n);
        o.iter().find(|(k, _)| norm(k) == n).map(|(_, v)| v)
    })
}

fn host_from_json(v: &serde_json::Value) -> Option<HostEntry> {
    let o = v.as_object()?;
    let text = |names: &[&str]| -> Option<String> {
        field(o, names).and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };
    let alias = text(&["alias", "name", "hostalias"]);
    let address = text(&[
        "hostname",
        "hostaddress",
        "ip",
        "ipaddress",
        "address",
        "addr",
    ]);
    // "host" is the alias in some exports and the address in others.
    let host = text(&["host"]);
    let (alias, hostname) = match (alias, address, host) {
        (Some(a), Some(h), _) => (a, h),
        (Some(a), None, h) => (a, h.unwrap_or_default()),
        (None, Some(h), Some(a)) => (a, h),
        (None, Some(h), None) => (h.clone(), h),
        (None, None, Some(h)) => (h.clone(), h),
        (None, None, None) => return None,
    };
    let access = text(&["access", "type", "protocol"])
        .map(|a| Access::from_db(&a))
        .unwrap_or_default();
    let port = text(&["port"])
        .and_then(|p| p.parse().ok())
        .filter(|p| *p != 0)
        .unwrap_or_else(|| access.default_port());
    let flag = |names: &[&str]| field(o, names).and_then(|v| v.as_bool());
    let unmonitored = flag(&["unmonitored", "disabled"])
        .or_else(|| flag(&["monitored", "enabled"]).map(|m| !m))
        .unwrap_or(false);
    let options = match field(o, &["options", "sshoptions"]) {
        // [["Keyword", "value"], ...] as Telepad exports them
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|kv| {
                let kv = kv.as_array()?;
                Some((
                    kv.first()?.as_str()?.to_string(),
                    kv.get(1)?.as_str()?.to_string(),
                ))
            })
            .collect(),
        // {"Keyword": "value", ...}
        Some(serde_json::Value::Object(m)) => m
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect(),
        _ => Vec::new(),
    };
    Some(HostEntry {
        id: None,
        alias,
        hostname,
        user: text(&["user", "username", "login"]).unwrap_or_default(),
        port,
        identity_file: text(&[
            "identityfile",
            "identity",
            "key",
            "keyfile",
            "privatekey",
            "sshkey",
        ]),
        note: text(&["description", "note", "comment", "desc"]).unwrap_or_default(),
        options,
        unmonitored,
        access,
        group: text(&["group"]).unwrap_or_default(),
        cluster: text(&["cluster"]).unwrap_or_default(),
    })
}

/// Make sure the group / cluster names `h` uses have an entry (#144).
fn ensure_sets(conn: &Connection, h: &HostEntry) -> Result<(), StoreError> {
    for kind in SetKind::ALL {
        let name = h.set(kind);
        if !name.is_empty() {
            conn.execute(
                "INSERT OR IGNORE INTO host_sets(kind, name, position)
                 VALUES(?1, ?2, (SELECT COALESCE(MAX(position), 0) + 1 FROM host_sets
                                 WHERE kind = ?1))",
                params![kind.as_str(), name],
            )?;
        }
    }
    Ok(())
}

pub fn export_json(inv: &Inventory) -> Result<String, StoreError> {
    Ok(serde_json::to_string_pretty(&JsonFile {
        format: JSON_FORMAT.into(),
        version: SCHEMA_VERSION,
        inventory: inv.clone(),
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Store {
        Store::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn pasted_host_lists_and_ssh_config_text() {
        let (inv, problems) = parse_pasted(
            "# office\nweb01 admin@192.0.2.10\n198.51.100.7:2222\n\nnas 192.0.2.20 extra\n\
             db01 root@192.0.2.30:0\nweb01 192.0.2.11\nbad/alias 192.0.2.40\n",
        );
        let got: Vec<(&str, &str, &str, u16)> = inv
            .hosts
            .iter()
            .map(|h| {
                (
                    h.alias.as_str(),
                    h.hostname.as_str(),
                    h.user.as_str(),
                    h.port,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("web01", "192.0.2.10", "admin", 22),
                ("198.51.100.7", "198.51.100.7", "", 2222)
            ]
        );
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems[0].starts_with("line 5:"), "{problems:?}");
        assert!(problems[1].starts_with("line 6: port"), "{problems:?}");
        assert!(problems[2].contains("appears twice"), "{problems:?}");
        assert!(problems[3].starts_with("bad/alias:"), "{problems:?}");

        // OpenSSH config text goes through the config parser.
        let (inv, problems) =
            parse_pasted("Host web02\n  HostName 192.0.2.12\n  User deploy\n  Port 2200\n");
        assert!(problems.is_empty());
        assert_eq!(inv.hosts[0].alias, "web02");
        assert_eq!(
            (inv.hosts[0].user.as_str(), inv.hosts[0].port),
            ("deploy", 2200)
        );
    }

    fn sample() -> Inventory {
        config::parse_inventory(
            "# Mail\nHost mail\n  HostName 10.0.0.1\n  User root\n  KexAlgorithms +x\n\nHost web\n  HostName 10.0.0.2\n\nHost *\n  ServerAliveInterval 30\n",
        )
    }

    #[test]
    fn import_merge_replace_and_preview() {
        let mut s = mem();
        assert!(s.is_empty().unwrap());
        let r = s.import(&sample(), ImportMode::Merge).unwrap();
        assert_eq!(r.added, 2);
        let inv = s.inventory().unwrap();
        assert_eq!(inv.hosts[0].note, "Mail");
        assert_eq!(
            inv.hosts[0].options,
            vec![("KexAlgorithms".into(), "+x".into())]
        );
        assert_eq!(inv.global, "Host *\n  ServerAliveInterval 30");

        let mut next = sample();
        next.hosts[1].hostname = "10.0.0.20".into();
        next.hosts.truncate(2);
        next.hosts.push(HostEntry::new("db"));
        let p = s.preview_import(&next, ImportMode::Merge).unwrap();
        assert_eq!((p.added, p.updated, p.unchanged), (1, 1, 1));
        s.import(&next, ImportMode::Merge).unwrap();
        assert_eq!(s.hosts().unwrap().len(), 3);

        let only = Inventory {
            hosts: vec![HostEntry::new("db")],
            global: String::new(),
            ..Default::default()
        };
        let r = s.import(&only, ImportMode::Replace).unwrap();
        assert_eq!(r.removed, 2);
        assert_eq!(s.hosts().unwrap().len(), 1);
        assert_eq!(s.global().unwrap(), "");
    }

    #[test]
    fn saved_order_is_the_order_hosts_are_read_in() {
        let mut s = mem();
        let ids: Vec<i64> = ["a", "b", "c"]
            .iter()
            .map(|a| s.save_host(&HostEntry::new(a)).unwrap())
            .collect();
        s.save_order(&[ids[2], ids[0], ids[1]]).unwrap();
        let order: Vec<String> = s.hosts().unwrap().into_iter().map(|h| h.alias).collect();
        assert_eq!(order, ["c", "a", "b"]);
        let cfg = config::to_ssh_config(&s.inventory().unwrap());
        assert!(cfg.find("Host c").unwrap() < cfg.find("Host a").unwrap());
    }

    #[test]
    fn save_validates_and_rejects_duplicate_alias() {
        let s = mem();
        let id = s.save_host(&HostEntry::new("a")).unwrap();
        s.save_host(&HostEntry::new("b")).unwrap();
        let mut a = s.hosts().unwrap().remove(0);
        assert_eq!(a.id, Some(id));
        a.alias = "b".into();
        assert!(s.save_host(&a).is_err());
        a.alias = "a2".into();
        a.unmonitored = true;
        s.save_host(&a).unwrap();
        let got = s.hosts().unwrap();
        assert_eq!(got[0].alias, "a2");
        assert!(got[0].unmonitored);
        assert!(s.save_host(&HostEntry::new("has space")).is_err());
        s.delete_host(id).unwrap();
        assert_eq!(s.hosts().unwrap().len(), 1);
    }

    #[test]
    fn host_changes_apply_in_one_transaction() {
        let s = mem();
        let a = s.save_host(&HostEntry::new("alpha")).unwrap();
        s.save_host(&HostEntry::new("bravo")).unwrap();
        let mut a2 = HostEntry::new("alpha");
        a2.id = Some(a);
        a2.user = "ops".into();
        // A failing upsert (duplicate alias) rolls the delete back too.
        let dup = HostEntry::new("bravo");
        assert!(s.apply_host_changes(&[a2.clone(), dup], &[a]).is_err());
        assert_eq!(s.hosts().unwrap().len(), 2);
        s.apply_host_changes(&[a2, HostEntry::new("charlie")], &[])
            .unwrap();
        let h = s.hosts().unwrap();
        assert_eq!(
            h.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "bravo", "charlie"]
        );
        assert_eq!(h[0].user, "ops");
        assert_eq!(s.host_sync_path().unwrap(), None);
        s.set_host_sync_path("/tmp/x/config").unwrap();
        assert_eq!(s.files_paths("web1").unwrap(), None);
        s.set_files_paths("web1", "/home/me/my site", "/srv/my app")
            .unwrap();
        assert_eq!(
            s.files_paths("web1").unwrap(),
            Some(("/home/me/my site".into(), "/srv/my app".into()))
        );
        assert_eq!(
            s.host_sync_path().unwrap().as_deref(),
            Some("/tmp/x/config")
        );
    }

    #[test]
    fn saves_database_connections_without_passwords() {
        let s = mem();
        let key = "dock1 / container shop-db / mysql";
        assert_eq!(s.db_profile(key).unwrap(), None);
        let mut p = crate::db::Profile {
            key: key.into(),
            user: "shop".into(),
            database: "shop".into(),
            save_password: true,
            read_only: false,
        };
        s.save_db_profile(&p).unwrap();
        p.read_only = true;
        s.save_db_profile(&p).unwrap();
        assert_eq!(s.db_profile(key).unwrap(), Some(p));
        let cols: Vec<String> = s
            .conn
            .prepare("SELECT name FROM pragma_table_info('db_connections')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            !cols
                .iter()
                .any(|c| c.contains("pass") && c != "save_password"),
            "{cols:?}"
        );
        s.delete_db_profile(key).unwrap();
        assert_eq!(s.db_profile(key).unwrap(), None);
    }

    #[test]
    fn migrates_v1_database_and_keeps_rows() {
        let dir = std::env::temp_dir().join(format!("telepad-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v1.db");
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE hosts (id INTEGER PRIMARY KEY, alias TEXT NOT NULL UNIQUE,
                   hostname TEXT NOT NULL DEFAULT '', user TEXT NOT NULL DEFAULT '',
                   port INTEGER NOT NULL DEFAULT 22, identity_file TEXT,
                   description TEXT NOT NULL DEFAULT '', options TEXT NOT NULL DEFAULT '',
                   monitored INTEGER NOT NULL DEFAULT 1, sort_order INTEGER NOT NULL DEFAULT 0,
                   updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                 INSERT INTO hosts(alias, hostname) VALUES('old', '10.0.0.9');
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let h = s.hosts().unwrap();
        assert_eq!(h[0].alias, "old");
        assert_eq!(h[0].access, Access::Ssh);
        let mut w = HostEntry::new("win5");
        w.hostname = "198.51.100.5".into();
        w.access = Access::Rdp;
        w.port = 3389;
        s.save_host(&w).unwrap();
        assert_eq!(s.hosts().unwrap()[1].access, Access::Rdp);
        let v: i32 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION, "v1 -> v2 -> v3 -> v4 -> v5 -> v6");
        assert_eq!(s.hosts().unwrap()[0].group, "", "old hosts are ungrouped");
        assert!(s.rdp_all().unwrap().is_empty(), "rdp_settings exists");
        assert!(
            s.portainers().unwrap().is_empty(),
            "portainers exists (v6, #171)"
        );
        assert_eq!(s.db_profile("x").unwrap(), None, "db_connections exists");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn groups_and_clusters_rename_delete_assign_and_order() {
        let mut s = mem();
        let mut a = HostEntry::new("web1");
        a.hostname = "192.0.2.10".into();
        a.group = "Office".into();
        a.cluster = "Web".into();
        let a = s.save_host(&a).unwrap();
        let mut b = HostEntry::new("web2");
        b.group = "Lab".into();
        b.cluster = "Web".into();
        let b = s.save_host(&b).unwrap();
        // Names a host uses get an entry; group and cluster are separate.
        let names = |s: &Store, k| {
            s.sets(k)
                .unwrap()
                .into_iter()
                .map(|x| x.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&s, SetKind::Group), ["Office", "Lab"]);
        assert_eq!(names(&s, SetKind::Cluster), ["Web"]);
        // Rename moves the hosts along; the description is kept.
        let set = HostSet {
            name: "Head office".into(),
            description: "Floor 2".into(),
        };
        s.save_set(SetKind::Group, Some("Office"), &set).unwrap();
        let h = s.hosts().unwrap();
        assert_eq!(h[0].group, "Head office");
        assert_eq!(h[0].cluster, "Web", "cluster untouched");
        assert_eq!(s.sets(SetKind::Group).unwrap()[0], set);
        // Names are unique ignoring case, and the fixed entry is reserved.
        let dup = HostSet {
            name: "lab".into(),
            ..Default::default()
        };
        assert!(s.save_set(SetKind::Group, None, &dup).is_err());
        let fixed = HostSet {
            name: "Ungrouped".into(),
            ..Default::default()
        };
        assert!(s.save_set(SetKind::Group, None, &fixed).is_err());
        // Changing the case of its own name is fine.
        let lab = HostSet {
            name: "LAB".into(),
            ..Default::default()
        };
        s.save_set(SetKind::Group, Some("Lab"), &lab).unwrap();
        // Order, assign, delete (hosts stay, without the group).
        s.order_sets(SetKind::Group, &["LAB".into(), "Head office".into()])
            .unwrap();
        assert_eq!(names(&s, SetKind::Group), ["LAB", "Head office"]);
        s.assign_set(SetKind::Group, &[a, b], "New").unwrap();
        assert_eq!(names(&s, SetKind::Group), ["LAB", "Head office", "New"]);
        s.delete_set(SetKind::Group, "New").unwrap();
        let h = s.hosts().unwrap();
        assert_eq!(h.len(), 2);
        assert!(h.iter().all(|h| h.group.is_empty() && h.cluster == "Web"));
        s.assign_set(SetKind::Cluster, &[b], "").unwrap();
        assert_eq!(s.hosts().unwrap()[1].cluster, "");
    }

    #[test]
    fn groups_and_clusters_survive_json_and_openssh_export() {
        let mut s = mem();
        let mut a = HostEntry::new("web1");
        a.group = "Office".into();
        a.cluster = "Ceph 1".into();
        s.save_host(&a).unwrap();
        s.save_set(
            SetKind::Group,
            Some("Office"),
            &HostSet {
                name: "Office".into(),
                description: "Head office".into(),
            },
        )
        .unwrap();
        let inv = s.inventory().unwrap();
        let json = export_json(&inv).unwrap();
        let back = parse_json_import(&json).unwrap();
        assert_eq!(back.hosts[0].group, "Office");
        assert_eq!(back.hosts[0].cluster, "Ceph 1");
        assert_eq!(back.sets[&SetKind::Group][0].description, "Head office");
        let mut t = mem();
        t.import(&back, ImportMode::Merge).unwrap();
        assert_eq!(
            t.sets(SetKind::Group).unwrap()[0].description,
            "Head office"
        );
        assert_eq!(t.hosts().unwrap()[0].cluster, "Ceph 1");
        // OpenSSH: names travel in marker comments.
        let text = config::to_ssh_config(&inv);
        assert!(text.contains("# telepad: group Office\n"), "{text}");
        assert!(text.contains("# telepad: cluster Ceph 1\n"), "{text}");
        let parsed = config::parse(&text);
        assert_eq!(parsed[0].group, "Office");
        assert_eq!(parsed[0].cluster, "Ceph 1");
        assert_eq!(parsed[0].note, "", "tags are not a description");
    }

    #[test]
    fn update_settings_default_round_trip_and_validate() {
        let s = mem();
        assert_eq!(s.update_config().unwrap(), UpdateConfig::default());
        let c = UpdateConfig {
            enabled: false,
            repo: "someone/telepad-fork".into(),
            auto_install: true,
        };
        s.set_update_config(&c).unwrap();
        assert_eq!(s.update_config().unwrap(), c);
        let bad = UpdateConfig {
            repo: "../../evil".into(),
            ..UpdateConfig::default()
        };
        assert!(s.set_update_config(&bad).is_err());
        assert_eq!(
            s.update_config().unwrap(),
            c,
            "rejected save changes nothing"
        );
    }

    #[test]
    fn ui_prefs_default_and_round_trip() {
        let s = mem();
        assert_eq!(s.ui_prefs().unwrap(), UiPrefs::default());
        assert!(UiPrefs::default().compact_host_list);
        let p = UiPrefs {
            compact_host_list: false,
            group_by_status: true,
            overview_cards: true,
            theme: ThemePref::Light,
            sidebar_collapsed: true,
            workspace: "0,0,700,500|0|0|overview|overview;docker".into(),
            editor_scheme: "solarized-dark".into(),
        };
        s.set_ui_prefs(&p).unwrap();
        assert_eq!(s.ui_prefs().unwrap(), p);
    }

    #[test]
    fn merge_import_appends_new_hosts_and_keeps_the_saved_order() {
        let mut s = mem();
        let ids: Vec<i64> = ["alpha", "bravo", "charlie", "delta"]
            .iter()
            .map(|a| s.save_host(&HostEntry::new(a)).unwrap())
            .collect();
        s.save_order(&[ids[3], ids[1], ids[2], ids[0]]).unwrap();
        let names = |s: &Store| -> Vec<String> {
            s.hosts().unwrap().into_iter().map(|h| h.alias).collect()
        };

        let mut bravo = HostEntry::new("bravo");
        bravo.hostname = "192.0.2.21".into();
        let inv = Inventory {
            hosts: vec![HostEntry::new("echo01"), bravo, HostEntry::new("foxtrot")],
            global: String::new(),
            ..Default::default()
        };
        s.import(&inv, ImportMode::Merge).unwrap();
        assert_eq!(
            names(&s),
            ["delta", "bravo", "charlie", "alpha", "echo01", "foxtrot"]
        );
        assert_eq!(s.hosts().unwrap()[1].hostname, "192.0.2.21");

        // Replace takes the file order.
        s.import(&inv, ImportMode::Replace).unwrap();
        assert_eq!(names(&s), ["echo01", "bravo", "foxtrot"]);
    }

    #[test]
    fn bad_import_changes_nothing() {
        let mut s = mem();
        s.import(&sample(), ImportMode::Merge).unwrap();
        let mut bad = sample();
        bad.hosts.push(HostEntry::new("bad alias"));
        assert!(s.import(&bad, ImportMode::Replace).is_err());
        assert_eq!(s.hosts().unwrap().len(), 2);
    }

    #[test]
    fn json_round_trip_and_format_check() {
        let inv = sample();
        let json = export_json(&inv).unwrap();
        assert_eq!(parse_import(&json).unwrap(), inv);
        assert!(parse_import("{\"format\":\"other\",\"version\":1}").is_err());
        assert_eq!(parse_import(&config::to_ssh_config(&inv)).unwrap(), inv);

        // Labels do not matter: older versions or the app's previous name import too.
        let relabelled = json
            .replace("\"telepad/hosts\"", "\"old-monitor/hosts\"")
            .replace(&format!("\"version\": {SCHEMA_VERSION}"), "\"version\": 99");
        assert_ne!(relabelled, json);
        assert_eq!(parse_import(&relabelled).unwrap(), inv);
    }

    #[test]
    fn imports_foreign_json_with_common_field_names() {
        let inv = parse_import(
            r#"[
              {"name": "web01", "ip": "192.0.2.10", "username": "admin", "Port": "2222",
               "IdentityFile": "~/.ssh/id_web", "comment": "Web server"},
              {"host": "192.0.2.20"},
              {"host": "db01", "address": "192.0.2.30", "monitored": false,
               "options": {"ProxyJump": "bastion"}},
              {"alias": "win01", "hostname": "192.0.2.40", "access": "RDP"}
            ]"#,
        )
        .unwrap();
        let h = &inv.hosts;
        assert_eq!(h.len(), 4);
        assert_eq!(
            (
                h[0].alias.as_str(),
                h[0].hostname.as_str(),
                h[0].user.as_str(),
                h[0].port
            ),
            ("web01", "192.0.2.10", "admin", 2222)
        );
        assert_eq!(h[0].identity_file.as_deref(), Some("~/.ssh/id_web"));
        assert_eq!(h[0].note, "Web server");
        assert_eq!(
            (h[1].alias.as_str(), h[1].hostname.as_str(), h[1].port),
            ("192.0.2.20", "192.0.2.20", 22)
        );
        assert!(h[2].unmonitored);
        assert_eq!(
            h[2].options,
            vec![("ProxyJump".to_string(), "bastion".to_string())]
        );
        assert_eq!((h[3].access, h[3].port), (Access::Rdp, 3389));
        assert!(h.iter().all(|h| h.validate().is_ok()));

        let wrapped = parse_import(r#"{"servers": [{"name": "a", "ip": "192.0.2.1"}]}"#).unwrap();
        assert_eq!(wrapped.hosts[0].alias, "a");
        assert!(parse_import(r#"{"something": 1}"#).is_err());
        assert!(
            parse_import(r#"[{"port": 22}]"#).is_err(),
            "no name or address"
        );
        assert!(parse_import("[1, 2]").is_err());
    }

    #[test]
    fn rdp_settings_follow_their_host_and_travel_in_json() {
        use crate::rdp::{RdpSettings, Screen};
        let s = mem();
        let mut w = HostEntry::new("win1");
        w.hostname = "198.51.100.7".into();
        w.access = Access::Rdp;
        w.port = 3389;
        let id = s.save_host(&w).unwrap();
        let r = RdpSettings {
            user: "ops".into(),
            screen: Screen::Window {
                width: 1600,
                height: 900,
            },
            ..Default::default()
        };
        s.set_rdp("win1", Some(&r)).unwrap();
        assert_eq!(s.rdp_all().unwrap().get("win1"), Some(&r));
        assert!(
            s.set_rdp(
                "win1",
                Some(&RdpSettings {
                    color_depth: 9,
                    ..Default::default()
                })
            )
            .is_err(),
            "validated"
        );
        // Rename: the settings follow.
        w.id = Some(id);
        w.alias = "win2".into();
        s.save_host(&w).unwrap();
        assert_eq!(s.rdp_all().unwrap().get("win2"), Some(&r));
        // JSON export / import keeps them.
        let json = export_json(&s.inventory().unwrap()).unwrap();
        assert!(json.contains("\"rdp\""), "{json}");
        let back = parse_import(&json).unwrap();
        assert_eq!(back.rdp.get("win2"), Some(&r));
        let mut other = mem();
        other.import(&back, ImportMode::Merge).unwrap();
        assert_eq!(other.rdp_all().unwrap().get("win2"), Some(&r));
        // Delete: they go with the host.
        s.delete_host(id).unwrap();
        assert!(s.rdp_all().unwrap().is_empty());
        // Forget.
        other.set_rdp("win2", None).unwrap();
        assert!(other.rdp_all().unwrap().is_empty());
    }

    #[test]
    fn a_v3_database_gets_the_rdp_table() {
        let path = std::env::temp_dir().join(format!("telepad-v3-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let s = Store::open(&path).unwrap();
            s.save_host(&HostEntry::new("a")).unwrap();
            s.conn
                .execute_batch("DROP TABLE rdp_settings; PRAGMA user_version = 3;")
                .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let v: i32 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        s.set_rdp("a", Some(&crate::rdp::RdpSettings::default()))
            .unwrap();
        assert_eq!(s.hosts().unwrap().len(), 1);
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn portainers_follow_their_host_and_travel_in_json_without_a_token() {
        // #171
        use crate::portainer::Portainer;
        let mut s = mem();
        s.import(&sample(), ImportMode::Merge).unwrap();
        let p = Portainer {
            name: "Main".into(),
            alias: "web".into(),
            port: 9443,
            https: true,
            fingerprint: String::new(),
        };
        s.save_portainer(None, &p).unwrap();
        let other = Portainer {
            name: "Lab".into(),
            alias: "mail".into(),
            port: 9000,
            https: false,
            ..Default::default()
        };
        s.save_portainer(None, &other).unwrap();
        let names = |s: &Store| -> Vec<String> {
            s.portainers()
                .unwrap()
                .into_iter()
                .map(|p| p.name)
                .collect()
        };
        assert_eq!(names(&s), ["Main", "Lab"], "added last");
        // Unique ignoring case.
        let dup = Portainer {
            name: "main".into(),
            ..other.clone()
        };
        assert!(s.save_portainer(None, &dup).is_err());
        // Pinned; kept on a rename, dropped when the address changes.
        s.pin_portainer("Main", "AB:CD").unwrap();
        let renamed = Portainer {
            name: "Prod".into(),
            ..p.clone()
        };
        s.save_portainer(Some("Main"), &renamed).unwrap();
        assert_eq!(s.portainers().unwrap()[0].fingerprint, "AB:CD");
        let moved = Portainer {
            port: 9444,
            ..renamed.clone()
        };
        s.save_portainer(Some("Prod"), &moved).unwrap();
        assert_eq!(s.portainers().unwrap()[0].fingerprint, "");
        // Follows a host rename.
        let mut web = s
            .hosts()
            .unwrap()
            .into_iter()
            .find(|h| h.alias == "web")
            .unwrap();
        web.alias = "web1".into();
        s.save_host(&web).unwrap();
        assert_eq!(s.portainers().unwrap()[0].alias, "web1");
        // JSON export / import: no token field, ever.
        let json = export_json(&s.inventory().unwrap()).unwrap();
        assert!(json.contains("\"portainers\""), "{json}");
        assert!(!json.to_lowercase().contains("token"), "{json}");
        let back = parse_import(&json).unwrap();
        assert_eq!(back.portainers.len(), 2);
        let mut other_db = mem();
        other_db.import(&back, ImportMode::Merge).unwrap();
        assert_eq!(names(&other_db), ["Prod", "Lab"]);
        // Deleting a host deletes its Portainers.
        s.delete_portainer("Lab").unwrap();
        assert_eq!(names(&s), ["Prod"]);
        let id = s
            .hosts()
            .unwrap()
            .into_iter()
            .find(|h| h.alias == "web1")
            .unwrap()
            .id
            .unwrap();
        s.delete_host(id).unwrap();
        assert!(s.portainers().unwrap().is_empty());
    }
}
