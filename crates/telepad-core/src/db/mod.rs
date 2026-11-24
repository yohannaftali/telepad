//! Database browser backend (#52–#54): find MariaDB / MySQL / PostgreSQL in
//! containers and on hosts, reach them through an SSH tunnel, read their
//! structure and data, run queries, and change rows / tables only when the
//! user asked for it.
//!
//! The data types here are shared with the app crate; the I/O lives in
//! [`client`] (drivers), [`tunnel`] (ssh -L) and [`secret`] (OS password store).

pub mod client;
pub mod ddl;
pub mod detect;
pub mod io;
pub mod secret;
pub mod sql;
pub mod tunnel;

use std::fmt;

pub use detect::DbTarget;

/// Database engine family; MariaDB, MySQL and Percona speak the same protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Engine {
    #[default]
    MySql,
    Postgres,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::MySql => "mysql",
            Engine::Postgres => "postgres",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mysql" => Some(Engine::MySql),
            "postgres" => Some(Engine::Postgres),
            _ => None,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Engine::MySql => "MariaDB / MySQL",
            Engine::Postgres => "PostgreSQL",
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            Engine::MySql => 3306,
            Engine::Postgres => 5432,
        }
    }
    pub fn default_user(self) -> &'static str {
        match self {
            Engine::MySql => "root",
            Engine::Postgres => "postgres",
        }
    }
    /// PostgreSQL has schemas inside each database; MySQL "databases" are schemas.
    pub fn has_schemas(self) -> bool {
        self == Engine::Postgres
    }
}

/// A password that never shows up in `Debug` output or logs.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// For a password field to edit in place.
    pub fn expose_mut(&mut self) -> &mut String {
        &mut self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "\"\"" } else { "\"***\"" })
    }
}

/// What the connect dialog sends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Login {
    pub user: String,
    pub password: Secret,
    /// Empty: none (MySQL) / `postgres` (PostgreSQL).
    pub database: String,
}

/// A saved connection (like HeidiSQL's session manager). The password is
/// never part of it: it lives in the OS password store, if anywhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// [`DbTarget::key`].
    pub key: String,
    pub user: String,
    pub database: String,
    pub save_password: bool,
    pub read_only: bool,
}

/// Database, PostgreSQL schema and table / view name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TableRef {
    pub database: String,
    /// PostgreSQL schema; empty for MySQL.
    pub schema: String,
    pub table: String,
}

/// One value of a result grid. Long values and binary data are cut so a page
/// of rows stays small; cut values cannot be edited in the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Null,
    Text(String),
    /// First [`MAX_CELL`] bytes of a longer text, and its full length in bytes.
    Long(String, usize),
    /// Non-UTF-8 data as `0x…` hex (cut), and its length in bytes.
    Bytes(String, usize),
}

/// Bytes of one value kept for display.
pub const MAX_CELL: usize = 64 * 1024;
/// Rows kept per query result.
pub const MAX_ROWS: usize = 10_000;

impl Cell {
    pub fn from_text(s: &str) -> Self {
        if s.len() <= MAX_CELL {
            return Cell::Text(s.to_string());
        }
        let mut end = MAX_CELL;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        Cell::Long(s[..end].to_string(), s.len())
    }
    pub fn from_bytes(b: &[u8]) -> Self {
        match std::str::from_utf8(b) {
            Ok(s) => Self::from_text(s),
            Err(_) => {
                let take = b.len().min(MAX_CELL / 2);
                let mut hex = String::with_capacity(2 + take * 2);
                hex.push_str("0x");
                for byte in &b[..take] {
                    hex.push_str(&format!("{byte:02X}"));
                }
                Cell::Bytes(hex, b.len())
            }
        }
    }
    /// The value as text, `None` for NULL.
    pub fn as_value(&self) -> Option<&str> {
        match self {
            Cell::Null => None,
            Cell::Text(s) | Cell::Long(s, _) | Cell::Bytes(s, _) => Some(s),
        }
    }
    /// Complete text value (not cut, not binary): can be edited and used as a key.
    pub fn editable(&self) -> bool {
        matches!(self, Cell::Null | Cell::Text(_))
    }
}

/// Rows of a page or query result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grid {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    /// More rows existed than were kept ([`MAX_ROWS`]).
    pub truncated: bool,
}

/// Result of one statement in the Query tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultSet {
    pub statement: String,
    pub grid: Option<Grid>,
    pub affected: Option<u64>,
    pub millis: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerInfo {
    pub version: String,
    pub databases: Vec<DatabaseInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseInfo {
    pub name: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableKind {
    #[default]
    Table,
    View,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableSummary {
    pub name: String,
    pub kind: TableKind,
    pub engine: String,
    /// Estimate from the server's statistics.
    pub rows: Option<u64>,
    pub size: Option<u64>,
    pub comment: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ColumnDefault {
    /// No DEFAULT clause.
    #[default]
    None,
    Null,
    /// A literal, quoted when written.
    Text(String),
    /// SQL written as is (`CURRENT_TIMESTAMP`, `nextval('…')`, `now()`).
    Expression(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// `varchar`, `int`, `character varying`, `timestamp with time zone`…
    pub type_name: String,
    /// What goes between the parentheses: `255`, `10,2`, `'a','b'`.
    pub length: String,
    /// MySQL only.
    pub unsigned: bool,
    pub nullable: bool,
    pub default: ColumnDefault,
    /// MySQL `AUTO_INCREMENT`, PostgreSQL identity column.
    pub auto_increment: bool,
    pub comment: String,
    /// MySQL only; empty = table default.
    pub collation: String,
    /// The type as the server reports it (PostgreSQL casts use it).
    pub full_type: String,
}

impl Column {
    /// Type written in DDL: `varchar(255)`, `int`.
    pub fn sql_type(&self) -> String {
        let l = self.length.trim();
        if l.is_empty() {
            self.type_name.trim().to_string()
        } else {
            format!("{}({l})", self.type_name.trim())
        }
    }
    /// Binary, spatial and bit columns are shown, not edited, in the grid.
    pub fn grid_editable(&self) -> bool {
        let t = self.type_name.to_ascii_lowercase();
        !(t.contains("blob")
            || t.contains("binary")
            || t == "bit"
            || t == "geometry"
            || t == "point"
            || t == "linestring"
            || t == "polygon"
            || t.starts_with("multi")
            || t == "geometrycollection")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IndexKind {
    Primary,
    Unique,
    #[default]
    Key,
    /// MySQL only.
    Fulltext,
}

impl IndexKind {
    pub const ALL: [IndexKind; 4] = [
        IndexKind::Primary,
        IndexKind::Unique,
        IndexKind::Key,
        IndexKind::Fulltext,
    ];
    pub fn title(self) -> &'static str {
        match self {
            IndexKind::Primary => "PRIMARY",
            IndexKind::Unique => "UNIQUE",
            IndexKind::Key => "KEY",
            IndexKind::Fulltext => "FULLTEXT",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub kind: IndexKind,
    pub columns: Vec<String>,
    /// PostgreSQL: the index belongs to a PRIMARY KEY / UNIQUE constraint.
    pub constraint: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    /// MySQL database / PostgreSQL schema of the referenced table.
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

/// Actions offered for ON UPDATE / ON DELETE.
pub const FK_ACTIONS: [&str; 5] = [
    "RESTRICT",
    "CASCADE",
    "SET NULL",
    "NO ACTION",
    "SET DEFAULT",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableInfo {
    pub table: TableRef,
    pub kind: TableKind,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub foreign_keys: Vec<ForeignKey>,
    /// MySQL `SHOW CREATE TABLE` (or view); PostgreSQL: empty.
    pub create_sql: String,
}

impl TableInfo {
    /// Columns that identify one row: the primary key, else the first unique
    /// index whose columns are all NOT NULL. `None`: rows cannot be edited.
    pub fn row_key(&self) -> Option<Vec<String>> {
        if self.kind == TableKind::View {
            return None;
        }
        let not_null = |c: &String| {
            self.columns
                .iter()
                .any(|col| &col.name == c && !col.nullable)
        };
        self.indexes
            .iter()
            .find(|i| i.kind == IndexKind::Primary && !i.columns.is_empty())
            .or_else(|| {
                self.indexes.iter().find(|i| {
                    i.kind == IndexKind::Unique
                        && !i.columns.is_empty()
                        && i.columns.iter().all(not_null)
                })
            })
            .map(|i| i.columns.clone())
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// A statement with bound parameters (`?` for MySQL, `$n` for PostgreSQL).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Option<String>>,
}

impl Statement {
    /// SQL with the parameters written in, for showing the user what will run.
    pub fn display(&self, engine: Engine) -> String {
        let lit = |p: &Option<String>| match p {
            None => "NULL".to_string(),
            Some(v) => sql::quote_str(engine, v),
        };
        match engine {
            Engine::MySql => {
                let mut out = String::new();
                let mut params = self.params.iter();
                for ch in self.sql.chars() {
                    match (ch, ch == '?') {
                        (_, true) => match params.next() {
                            Some(p) => out.push_str(&lit(p)),
                            None => out.push('?'),
                        },
                        (c, false) => out.push(c),
                    }
                }
                out
            }
            Engine::Postgres => {
                let mut out = self.sql.clone();
                // Highest number first so $1 does not eat the start of $12.
                for (i, p) in self.params.iter().enumerate().rev() {
                    out = out.replace(&format!("${}", i + 1), &lit(p));
                }
                out
            }
        }
    }
}

/// Human size: 512 B, 1.5 KiB, 12.0 MiB.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_do_not_print() {
        let l = Login {
            user: "alice".into(),
            password: Secret::new("hunter2"),
            database: String::new(),
        };
        let shown = format!("{l:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("***"));
    }

    #[test]
    fn cells_cut_long_and_binary_values() {
        assert_eq!(Cell::from_bytes(b"abc"), Cell::Text("abc".into()));
        let long = "é".repeat(MAX_CELL);
        match Cell::from_text(&long) {
            Cell::Long(head, n) => {
                assert!(head.len() <= MAX_CELL);
                assert_eq!(n, long.len());
            }
            other => panic!("{other:?}"),
        }
        match Cell::from_bytes(&[0xff, 0x00, 0x10]) {
            Cell::Bytes(hex, 3) => assert_eq!(hex, "0xFF0010"),
            other => panic!("{other:?}"),
        }
        assert!(Cell::Null.editable() && !Cell::Bytes("0x".into(), 1).editable());
    }

    fn col(name: &str, nullable: bool) -> Column {
        Column {
            name: name.into(),
            type_name: "int".into(),
            nullable,
            ..Default::default()
        }
    }

    #[test]
    fn row_key_prefers_primary_then_not_null_unique() {
        let mut t = TableInfo {
            columns: vec![col("id", false), col("code", false), col("opt", true)],
            indexes: vec![
                Index {
                    name: "u_opt".into(),
                    kind: IndexKind::Unique,
                    columns: vec!["opt".into()],
                    constraint: false,
                },
                Index {
                    name: "u_code".into(),
                    kind: IndexKind::Unique,
                    columns: vec!["code".into()],
                    constraint: false,
                },
            ],
            ..Default::default()
        };
        assert_eq!(t.row_key(), Some(vec!["code".to_string()]));
        t.indexes.push(Index {
            name: "PRIMARY".into(),
            kind: IndexKind::Primary,
            columns: vec!["id".into()],
            constraint: true,
        });
        assert_eq!(t.row_key(), Some(vec!["id".to_string()]));
        t.indexes.clear();
        assert_eq!(t.row_key(), None);
    }

    #[test]
    fn statement_display_fills_in_parameters() {
        let s = Statement {
            sql: "UPDATE `t` SET `a` = ? WHERE `id` = ?".into(),
            params: vec![Some("it's".into()), None],
        };
        assert_eq!(
            s.display(Engine::MySql),
            "UPDATE `t` SET `a` = 'it''s' WHERE `id` = NULL"
        );
        let p = Statement {
            sql: "UPDATE t SET a = $1, b = $12".into(),
            params: (1..=12).map(|i| Some(i.to_string())).collect(),
        };
        assert_eq!(
            p.display(Engine::Postgres),
            "UPDATE t SET a = '1', b = '12'"
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
    }
}
