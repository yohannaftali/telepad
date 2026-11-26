//! History of what Telepad changed (#100): file transfers, file operations
//! and database statements, kept in `history.db` next to `telepad.db`.
//! Purely local: an entry records what already ran. Never holds a
//! password, key material or file content; SQL is stored as it ran, with
//! password literals masked ([`mask_secrets`]).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::store::StoreError;

pub const HISTORY_FILE: &str = "history.db";
const HISTORY_VERSION: i32 = 1;
/// Kept at most this long …
pub const KEEP_DAYS: i64 = 90;
/// … and at most this many entries.
pub const KEEP_ROWS: i64 = 50_000;
/// Entries a window loads (newest).
pub const LOAD_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Transfer,
    FileOp,
    Sql,
    /// A stack / container action or stack update through Portainer (#173).
    Portainer,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Transfer => "transfer",
            Kind::FileOp => "fileop",
            Kind::Sql => "sql",
            Kind::Portainer => "portainer",
        }
    }
    fn parse(s: &str) -> Kind {
        match s {
            "fileop" => Kind::FileOp,
            "sql" => Kind::Sql,
            "portainer" => Kind::Portainer,
            _ => Kind::Transfer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Outcome {
    #[default]
    Done,
    Failed,
    Cancelled,
    /// Database: the transaction was rolled back.
    RolledBack,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Failed => "failed",
            Outcome::Cancelled => "cancelled",
            Outcome::RolledBack => "rolled back",
        }
    }
    fn parse(s: &str) -> Outcome {
        match s {
            "failed" => Outcome::Failed,
            "cancelled" => Outcome::Cancelled,
            "rolled back" => Outcome::RolledBack,
            _ => Outcome::Done,
        }
    }
}

/// One thing Telepad did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    /// Unix seconds; `None` until written (the runtime stamps it).
    pub at: Option<i64>,
    pub kind: Kind,
    /// Host alias.
    pub host: String,
    /// Database connection (`DbTarget::key`); empty for files.
    pub target: String,
    /// One line: "Upload 3 items → /srv/www", "UPDATE `shop`.`items`".
    pub title: String,
    /// Paths (one per line) or the SQL as it ran.
    pub detail: String,
    pub outcome: Outcome,
    /// Error text, or what was skipped.
    pub error: String,
    /// Database: rows affected.
    pub rows: Option<u64>,
    pub bytes: Option<u64>,
    pub files: Option<u64>,
    pub duration_ms: Option<u64>,
    /// SFTP, scp -O, rsync, rsync (dry run); database: engine.
    pub method: String,
    /// Database user.
    pub user: String,
    /// Database / schema.
    pub database: String,
}

/// Which entries a window shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// A host's transfers and file operations.
    Files(String),
    /// A database connection's statements.
    Db(String),
}

pub fn history_path(db: &Path, cli: Option<&str>) -> PathBuf {
    cli.map(PathBuf::from)
        .or_else(|| std::env::var_os("TELEPAD_HISTORY").map(PathBuf::from))
        .unwrap_or_else(|| db.with_file_name(HISTORY_FILE))
}

pub struct History {
    conn: Connection,
    permission_warning: Option<String>,
}

impl History {
    /// Open (or create) the file and trim old entries (`now`: unix seconds).
    pub fn open(path: &Path, now: i64) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        let permission_warning = crate::platform::restrict_db_files(path).err();
        let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v > HISTORY_VERSION {
            return Err(StoreError::Invalid(format!(
                "{} was written by a newer Telepad (version {v})",
                path.display()
            )));
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 at INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 host TEXT NOT NULL,
                 target TEXT NOT NULL,
                 title TEXT NOT NULL,
                 detail TEXT NOT NULL,
                 outcome TEXT NOT NULL,
                 error TEXT NOT NULL,
                 rows INTEGER,
                 bytes INTEGER,
                 files INTEGER,
                 duration_ms INTEGER,
                 method TEXT NOT NULL,
                 user TEXT NOT NULL,
                 database TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS entries_host ON entries(kind, host);
             CREATE INDEX IF NOT EXISTS entries_target ON entries(target);",
        )?;
        conn.pragma_update(None, "user_version", HISTORY_VERSION)?;
        let h = History {
            conn,
            permission_warning,
        };
        h.trim(now)?;
        Ok(h)
    }

    /// Why the file stays readable by others (#110), if it does.
    pub fn permission_warning(&self) -> Option<&str> {
        self.permission_warning.as_deref()
    }

    /// Drop entries older than [`KEEP_DAYS`] and beyond [`KEEP_ROWS`].
    pub fn trim(&self, now: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM entries WHERE at < ?1",
            params![now - KEEP_DAYS * 86_400],
        )?;
        self.conn.execute(
            "DELETE FROM entries WHERE id <= (SELECT id FROM entries ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            params![KEEP_ROWS],
        )?;
        Ok(())
    }

    /// Write one entry; `at` is set to `now` when missing. Text that looks
    /// like a password in SQL is masked again here, whoever built the entry.
    pub fn add(&self, e: &Entry, now: i64) -> Result<(), StoreError> {
        let opt = |v: Option<u64>| v.map(|v| v.min(i64::MAX as u64) as i64);
        self.conn.execute(
            "INSERT INTO entries (at, kind, host, target, title, detail, outcome, error, rows, bytes,
                 files, duration_ms, method, user, database)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                e.at.unwrap_or(now),
                e.kind.as_str(),
                e.host,
                e.target,
                mask_secrets(&e.title),
                mask_secrets(&e.detail),
                e.outcome.as_str(),
                e.error,
                opt(e.rows),
                opt(e.bytes),
                opt(e.files),
                opt(e.duration_ms),
                e.method,
                e.user,
                e.database,
            ],
        )?;
        Ok(())
    }

    /// The newest `limit` entries of `scope`, oldest first.
    pub fn list(&self, scope: &Scope, limit: usize) -> Result<Vec<Entry>, StoreError> {
        let (cond, key) = match scope {
            Scope::Files(h) => ("kind IN ('transfer', 'fileop') AND host = ?1", h),
            Scope::Db(k) => ("kind = 'sql' AND target = ?1", k),
        };
        let sql = format!(
            "SELECT at, kind, host, target, title, detail, outcome, error, rows, bytes, files,
                 duration_ms, method, user, database
             FROM entries WHERE {cond} ORDER BY id DESC LIMIT ?2"
        );
        let mut st = self.conn.prepare(&sql)?;
        let u = |v: Option<i64>| v.map(|v| v.max(0) as u64);
        let rows = st.query_map(params![key, limit as i64], |r| {
            Ok(Entry {
                at: Some(r.get(0)?),
                kind: Kind::parse(&r.get::<_, String>(1)?),
                host: r.get(2)?,
                target: r.get(3)?,
                title: r.get(4)?,
                detail: r.get(5)?,
                outcome: Outcome::parse(&r.get::<_, String>(6)?),
                error: r.get(7)?,
                rows: u(r.get(8)?),
                bytes: u(r.get(9)?),
                files: u(r.get(10)?),
                duration_ms: u(r.get(11)?),
                method: r.get(12)?,
                user: r.get(13)?,
                database: r.get(14)?,
            })
        })?;
        let mut v = rows.collect::<Result<Vec<_>, _>>()?;
        v.reverse();
        Ok(v)
    }

    /// Delete every entry of `scope`.
    pub fn clear(&self, scope: &Scope) -> Result<(), StoreError> {
        match scope {
            Scope::Files(h) => self.conn.execute(
                "DELETE FROM entries WHERE kind IN ('transfer', 'fileop') AND host = ?1",
                params![h],
            )?,
            Scope::Db(k) => self.conn.execute(
                "DELETE FROM entries WHERE kind = 'sql' AND target = ?1",
                params![k],
            )?,
        };
        Ok(())
    }
}

/// Literals after `IDENTIFIED BY`, `PASSWORD` (also `PASSWORD(...)`) and
/// `IDENTIFIED WITH … BY` replaced by `'***'`, so a `CREATE USER` / `ALTER
/// USER` / `SET PASSWORD` never keeps its password in the history.
pub fn mask_secrets(sql: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    let bytes = sql.as_bytes();
    while i < sql.len() {
        let rest = &lower[i..];
        let key = ["identified by", "password"]
            .iter()
            .find(|k| rest.starts_with(**k) && word_start(&lower, i))
            .map(|k| k.len());
        if let Some(n) = key {
            out.push_str(&sql[i..i + n]);
            i += n;
            // Spaces, `=`, `(` before the literal.
            while i < sql.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'(') {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < sql.len() && matches!(bytes[i], b'\'' | b'"') {
                let q = bytes[i];
                let mut j = i + 1;
                while j < sql.len() {
                    if bytes[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if bytes[j] == q {
                        if j + 1 < sql.len() && bytes[j + 1] == q {
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    j += 1;
                }
                out.push_str("'***'");
                i = (j + 1).min(sql.len());
            }
            continue;
        }
        let ch = sql[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn word_start(s: &str, i: usize) -> bool {
    i == 0
        || !s[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `YYYY-MM-DD HH:MM:SS` in UTC for unix seconds.
pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Entries as CSV (header line, RFC 4180 quoting), for Export.
/// Text cells that a spreadsheet would read as a formula (#111) get a
/// leading `'`: file names and error text come from hosts, SQL from users.
fn defuse(s: &str) -> std::borrow::Cow<'_, str> {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}").into()
    } else {
        s.into()
    }
}

pub fn to_csv(entries: &[Entry]) -> String {
    let q = |s: &str| {
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let t = |s: &str| q(&defuse(s));
    let n = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_default();
    let mut out = String::from(
        "time_utc,kind,host,target,title,detail,outcome,error,rows,bytes,files,duration_ms,method,user,database\n",
    );
    for e in entries {
        // Times, kinds, outcomes and numbers are Telepad's own; the rest is text.
        let fields = [
            q(&e.at.map(format_utc).unwrap_or_default()),
            q(e.kind.as_str()),
            t(&e.host),
            t(&e.target),
            t(&e.title),
            t(&e.detail),
            q(e.outcome.as_str()),
            t(&e.error),
            q(&n(e.rows)),
            q(&n(e.bytes)),
            q(&n(e.files)),
            q(&n(e.duration_ms)),
            t(&e.method),
            t(&e.user),
            t(&e.database),
        ];
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("telepad-history-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn transfer(host: &str, title: &str) -> Entry {
        Entry {
            kind: Kind::Transfer,
            host: host.into(),
            title: title.into(),
            detail: "/home/me/a.txt\n→ /srv/www".into(),
            bytes: Some(10),
            files: Some(1),
            method: "SFTP".into(),
            ..Default::default()
        }
    }

    #[test]
    fn writes_and_reads_by_host_and_connection() {
        let p = temp("rw");
        let h = History::open(&p, 1_000_000).unwrap();
        h.add(&transfer("web1", "Upload 1 item"), 1_000_000)
            .unwrap();
        h.add(&transfer("web2", "Download"), 1_000_001).unwrap();
        h.add(
            &Entry {
                kind: Kind::Sql,
                host: "dock1".into(),
                target: "dock1 / container shop-db / mysql".into(),
                title: "UPDATE".into(),
                detail: "UPDATE items SET n = 1".into(),
                rows: Some(1),
                ..Default::default()
            },
            1_000_002,
        )
        .unwrap();
        let v = h.list(&Scope::Files("web1".into()), 10).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].title, "Upload 1 item");
        assert_eq!(v[0].at, Some(1_000_000));
        assert_eq!(v[0].bytes, Some(10));
        let v = h
            .list(&Scope::Db("dock1 / container shop-db / mysql".into()), 10)
            .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rows, Some(1));
        h.clear(&Scope::Files("web1".into())).unwrap();
        assert!(h.list(&Scope::Files("web1".into()), 10).unwrap().is_empty());
        assert_eq!(h.list(&Scope::Files("web2".into()), 10).unwrap().len(), 1);
        drop(h);
        let _ = std::fs::remove_file(&p);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let p = temp("mode");
        let h = History::open(&p, 1).unwrap();
        h.add(&transfer("web1", "x"), 1).unwrap();
        for suffix in ["", "-wal", "-shm"] {
            let mut f = p.as_os_str().to_os_string();
            f.push(suffix);
            if let Ok(m) = std::fs::metadata(&f) {
                assert_eq!(m.permissions().mode() & 0o777, 0o600, "{suffix}");
            }
        }
        assert!(h.permission_warning().is_none());
        drop(h);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn old_entries_are_trimmed_on_open() {
        let p = temp("trim");
        let day = 86_400;
        let h = History::open(&p, 100 * day).unwrap();
        h.add(&transfer("web1", "old"), 100 * day).unwrap();
        h.add(&transfer("web1", "new"), 180 * day).unwrap();
        drop(h);
        let h = History::open(&p, 195 * day).unwrap();
        let v = h.list(&Scope::Files("web1".into()), 10).unwrap();
        assert_eq!(
            v.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            ["new"]
        );
        drop(h);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn newest_first_limit_returns_oldest_first() {
        let p = temp("limit");
        let h = History::open(&p, 1).unwrap();
        for i in 0..5 {
            h.add(&transfer("web1", &format!("t{i}")), 1 + i).unwrap();
        }
        let v = h.list(&Scope::Files("web1".into()), 2).unwrap();
        assert_eq!(
            v.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            ["t3", "t4"]
        );
        drop(h);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_newer_file_is_refused() {
        let p = temp("newer");
        {
            let c = Connection::open(&p).unwrap();
            c.pragma_update(None, "user_version", 99).unwrap();
        }
        assert!(History::open(&p, 1).is_err());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn passwords_in_sql_are_masked() {
        assert_eq!(
            mask_secrets("CREATE USER 'a'@'%' IDENTIFIED BY 's3cr,et';"),
            "CREATE USER 'a'@'%' IDENTIFIED BY '***';"
        );
        assert_eq!(
            mask_secrets("ALTER ROLE app WITH PASSWORD 'x''y' VALID UNTIL 'infinity'"),
            "ALTER ROLE app WITH PASSWORD '***' VALID UNTIL 'infinity'"
        );
        assert_eq!(
            mask_secrets("SET PASSWORD FOR bob = PASSWORD('pw')"),
            "SET PASSWORD FOR bob = PASSWORD('***')"
        );
        assert_eq!(
            mask_secrets("UPDATE users SET password_hash = 'abc'"),
            "UPDATE users SET password_hash = 'abc'",
            "only the keyword itself"
        );
        assert_eq!(mask_secrets("SELECT 'é' -- ü"), "SELECT 'é' -- ü");
    }

    #[test]
    fn csv_cells_that_look_like_formulas_are_defused() {
        let csv = to_csv(&[Entry {
            at: Some(0),
            title: "=HYPERLINK(\"http://example.invalid\",\"x\")".into(),
            detail: "-- cleanup\nDELETE FROM t".into(),
            error: "@SUM(1)".into(),
            user: "+ops".into(),
            rows: Some(3),
            ..Default::default()
        }]);
        let row = csv.lines().nth(1).unwrap();
        assert!(
            row.contains("\"'=HYPERLINK(\"\"http://example.invalid\"\""),
            "{row}"
        );
        assert!(csv.contains("\"'-- cleanup"), "{csv}");
        assert!(csv.contains(",'@SUM(1),"), "{csv}");
        assert!(csv.contains(",'+ops,"), "{csv}");
        assert!(csv.contains(",3,"), "numbers unchanged: {csv}");
        assert!(row.starts_with("1970-01-01 00:00:00,"), "times unchanged");
    }

    #[test]
    fn utc_times_and_csv() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1_795_000_000), "2026-11-18 11:06:40");
        let csv = to_csv(&[Entry {
            at: Some(0),
            title: "a, \"b\"".into(),
            ..transfer("web1", "")
        }]);
        assert!(csv.starts_with("time_utc,kind,"));
        assert!(csv.contains("\"a, \"\"b\"\"\""));
        assert!(csv.contains("\"/home/me/a.txt\n→ /srv/www\""));
    }
}
