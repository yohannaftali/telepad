//! `preference.db` (#94): view state kept apart from the host inventory —
//! the named workspaces (WinBox style) with their window layouts, and which
//! one is current. A small SQLite file next to `telepad.db`.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::store::StoreError;

pub const PREFS_FILE: &str = "preference.db";
const PREFS_VERSION: i32 = 1;
pub const NONE_LABEL: &str = "<none>";
pub const OWN_LABEL: &str = "<own>";
pub const MAX_LABEL: usize = 40;

/// Which workspace is shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Current {
    /// No windows; nothing is saved while it is current.
    None,
    /// The default workspace.
    #[default]
    Own,
    Custom(String),
}

impl Current {
    pub fn label(&self) -> &str {
        match self {
            Current::None => NONE_LABEL,
            Current::Own => OWN_LABEL,
            Current::Custom(l) => l,
        }
    }

    fn parse(s: &str) -> Current {
        match s {
            NONE_LABEL => Current::None,
            OWN_LABEL | "" => Current::Own,
            l => Current::Custom(l.to_string()),
        }
    }
}

/// Everything `preference.db` holds about workspaces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspacePrefs {
    /// Layout of `<own>` (`workspace::Workspace::save` text).
    pub own: String,
    /// Custom workspaces: (label, layout), sorted by label ignoring case.
    pub custom: Vec<(String, String)>,
    pub current: Current,
}

impl WorkspacePrefs {
    pub fn layout(&self, c: &Current) -> Option<&str> {
        match c {
            Current::None => None,
            Current::Own => Some(&self.own),
            Current::Custom(l) => self
                .custom
                .iter()
                .find(|(x, _)| x == l)
                .map(|(_, v)| v.as_str()),
        }
    }

    pub fn layout_mut(&mut self, c: &Current) -> Option<&mut String> {
        match c {
            Current::None => None,
            Current::Own => Some(&mut self.own),
            Current::Custom(l) => self.custom.iter_mut().find(|(x, _)| x == l).map(|(_, v)| v),
        }
    }

    pub fn sort(&mut self) {
        self.custom.sort_by_key(|a| a.0.to_lowercase());
    }

    /// A new label: trimmed, 1–40 characters, one line, not `<none>` /
    /// `<own>`, not used yet (case does not matter). Returns the trimmed label.
    pub fn check_label(&self, label: &str) -> Result<String, String> {
        let l = label.trim();
        if l.is_empty() {
            return Err("Type a name for the workspace.".into());
        }
        if l.chars().count() > MAX_LABEL {
            return Err(format!("At most {MAX_LABEL} characters."));
        }
        if l.chars().any(char::is_control) {
            return Err("One line, without tabs or control characters.".into());
        }
        if l.eq_ignore_ascii_case(NONE_LABEL) || l.eq_ignore_ascii_case(OWN_LABEL) {
            return Err(format!("{NONE_LABEL} and {OWN_LABEL} are built in."));
        }
        if self
            .custom
            .iter()
            .any(|(x, _)| x.to_lowercase() == l.to_lowercase())
        {
            return Err(format!("A workspace named \"{l}\" exists already."));
        }
        Ok(l.to_string())
    }
}

/// `--prefs` > `TELEPAD_PREFS` > `preference.db` next to `telepad.db`.
pub fn prefs_path(db: &Path, cli: Option<&str>) -> PathBuf {
    cli.map(PathBuf::from)
        .or_else(|| std::env::var_os("TELEPAD_PREFS").map(PathBuf::from))
        .unwrap_or_else(|| db.with_file_name(PREFS_FILE))
}

pub struct Prefs {
    conn: Connection,
    path: PathBuf,
}

impl Prefs {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v > PREFS_VERSION {
            return Err(StoreError::Invalid(format!(
                "{} was written by a newer Telepad (version {v})",
                path.display()
            )));
        }
        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS workspaces (
                 id       INTEGER PRIMARY KEY,
                 label    TEXT    NOT NULL,
                 -- 'own' (exactly one row) or 'custom'
                 kind     TEXT    NOT NULL DEFAULT 'custom',
                 layout   TEXT    NOT NULL DEFAULT '',
                 position INTEGER NOT NULL DEFAULT 0,
                 updated  TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE UNIQUE INDEX IF NOT EXISTS workspaces_label
                 ON workspaces(lower(label));
             CREATE TABLE IF NOT EXISTS prefs (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             PRAGMA user_version = {PREFS_VERSION};"
        ))?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn pref(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM prefs WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Once per `preference.db`: the layout an older Telepad kept in
    /// `telepad.db` (`ui_workspace`) becomes `<own>`. `true` when it did.
    pub fn import_legacy(&self, layout: &str) -> Result<bool, StoreError> {
        if self.pref("legacy_imported")?.is_some() {
            return Ok(false);
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO workspaces(label, kind, layout) VALUES(?1, 'own', ?2)
             ON CONFLICT DO NOTHING",
            params![OWN_LABEL, layout],
        )?;
        tx.execute(
            "UPDATE workspaces SET layout = ?1 WHERE kind = 'own' AND layout = ''",
            [layout],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO prefs(key, value) VALUES('legacy_imported', '1')",
            [],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn load(&self) -> Result<WorkspacePrefs, StoreError> {
        let mut p = WorkspacePrefs::default();
        let mut st = self
            .conn
            .prepare("SELECT label, kind, layout FROM workspaces ORDER BY position, id")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (label, kind, layout) = row?;
            if kind == "own" {
                p.own = layout;
            } else {
                p.custom.push((label, layout));
            }
        }
        p.sort();
        p.current = Current::parse(&self.pref("current")?.unwrap_or_default());
        if let Current::Custom(l) = &p.current
            && !p.custom.iter().any(|(x, _)| x == l)
        {
            p.current = Current::Own;
        }
        Ok(p)
    }

    /// Write `p` as it is: changed layouts get a new `updated` time, labels
    /// that are gone are deleted. One transaction.
    pub fn save(&self, p: &WorkspacePrefs) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO workspaces(label, kind, layout) VALUES(?1, 'own', ?2)
             ON CONFLICT DO NOTHING",
            params![OWN_LABEL, p.own],
        )?;
        tx.execute(
            "UPDATE workspaces SET layout = ?1, updated = CURRENT_TIMESTAMP
             WHERE kind = 'own' AND layout <> ?1",
            [&p.own],
        )?;
        let keep: Vec<String> = p.custom.iter().map(|(l, _)| l.clone()).collect();
        let existing: Vec<String> = {
            let mut st = tx.prepare("SELECT label FROM workspaces WHERE kind = 'custom'")?;
            st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        for l in existing.iter().filter(|l| !keep.contains(l)) {
            tx.execute(
                "DELETE FROM workspaces WHERE kind = 'custom' AND label = ?1",
                [l],
            )?;
        }
        for (i, (label, layout)) in p.custom.iter().enumerate() {
            tx.execute(
                "INSERT INTO workspaces(label, kind, layout, position) VALUES(?1, 'custom', ?2, ?3)
                 ON CONFLICT DO UPDATE SET
                     updated = CASE WHEN layout <> excluded.layout
                                    THEN CURRENT_TIMESTAMP ELSE updated END,
                     layout = excluded.layout,
                     position = excluded.position",
                params![label, layout, i as i64 + 1],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO prefs(key, value) VALUES('current', ?1)",
            [p.current.label()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("telepad-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", p.display()));
        }
        p
    }

    #[test]
    fn round_trip_delete_and_current() {
        let path = temp("rt.db");
        let db = Prefs::open(&path).unwrap();
        assert_eq!(db.load().unwrap(), WorkspacePrefs::default());
        let mut p = WorkspacePrefs {
            own: "|0|1|overview|overview".into(),
            custom: vec![
                ("ops".into(), "|0|0|docker|docker".into()),
                ("Backups".into(), String::new()),
            ],
            current: Current::Custom("ops".into()),
        };
        p.sort();
        db.save(&p).unwrap();
        drop(db);
        let db = Prefs::open(&path).unwrap();
        let back = db.load().unwrap();
        assert_eq!(back, p);
        assert_eq!(back.custom[0].0, "Backups", "sorted by label");
        // Deleting the current one: it is gone, and a missing current reads as <own>.
        p.custom.retain(|(l, _)| l != "ops");
        db.save(&p).unwrap();
        let back = db.load().unwrap();
        assert_eq!(back.custom.len(), 1);
        assert_eq!(back.current, Current::Own);
        p.current = Current::None;
        db.save(&p).unwrap();
        assert_eq!(db.load().unwrap().current, Current::None);
    }

    #[test]
    fn legacy_layout_becomes_own_once() {
        let path = temp("legacy.db");
        let db = Prefs::open(&path).unwrap();
        assert!(db.import_legacy("|0|1|docker|docker").unwrap());
        assert_eq!(db.load().unwrap().own, "|0|1|docker|docker");
        let mut p = db.load().unwrap();
        p.own = "|0|0|kube|kube".into();
        db.save(&p).unwrap();
        assert!(!db.import_legacy("|0|1|docker|docker").unwrap());
        assert_eq!(db.load().unwrap().own, "|0|0|kube|kube");
    }

    #[test]
    fn newer_file_is_refused() {
        let path = temp("newer.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("PRAGMA user_version = 99;").unwrap();
        }
        let e = Prefs::open(&path).err().unwrap().to_string();
        assert!(e.contains("newer Telepad"), "{e}");
    }

    #[test]
    fn labels_are_checked() {
        let p = WorkspacePrefs {
            custom: vec![("Ops".into(), String::new())],
            ..Default::default()
        };
        assert_eq!(p.check_label("  web team  ").unwrap(), "web team");
        assert!(p.check_label("   ").is_err());
        assert!(p.check_label("ops").is_err(), "case-insensitive duplicate");
        assert!(p.check_label("<own>").is_err());
        assert!(p.check_label("<NONE>").is_err());
        assert!(p.check_label("a\tb").is_err());
        assert!(p.check_label(&"x".repeat(41)).is_err());
        assert!(p.check_label(&"é".repeat(40)).is_ok());
    }

    #[test]
    fn path_prefers_cli_then_next_to_the_db() {
        let db = Path::new("data").join("telepad.db");
        assert_eq!(prefs_path(&db, Some("x/p.db")), PathBuf::from("x/p.db"));
        if std::env::var_os("TELEPAD_PREFS").is_none() {
            assert_eq!(prefs_path(&db, None), Path::new("data").join(PREFS_FILE));
        }
    }
}
