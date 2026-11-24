//! One open database connection: the SSH tunnel plus the MySQL or PostgreSQL
//! driver behind it. All calls block; the GUI runs them on a worker thread.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mysql::consts::CapabilityFlags;
use mysql::prelude::Queryable;
use mysql::{Conn, OptsBuilder, TxOpts, Value};
use postgres::error::ErrorPosition;
use postgres::types::ToSql;
use postgres::{CancelToken, Client, NoTls, SimpleQueryMessage};

use super::tunnel::Tunnel;
use super::{
    Cell, Column, ColumnDefault, DatabaseInfo, DbTarget, Engine, ForeignKey, Grid, Index,
    IndexKind, Login, MAX_ROWS, ResultSet, Secret, ServerInfo, Statement, TableInfo, TableKind,
    TableRef, TableSummary, sql,
};
use crate::collector::SshSettings;
use crate::config::HostEntry;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Stops the statement that is running on a [`Session`], from another thread.
pub enum Canceller {
    MySql {
        port: u16,
        user: String,
        password: Secret,
        id: u32,
    },
    Postgres(CancelToken),
}

impl Canceller {
    pub fn cancel(&self) -> Result<(), String> {
        match self {
            Canceller::MySql {
                port,
                user,
                password,
                id,
            } => {
                let login = Login {
                    user: user.clone(),
                    password: password.clone(),
                    database: String::new(),
                };
                let mut c = Conn::new(my_opts(*port, &login)).map_err(|e| my_err(&e))?;
                c.query_drop(format!("KILL QUERY {id}"))
                    .map_err(|e| my_err(&e))
            }
            Canceller::Postgres(t) => t.cancel_query(NoTls).map_err(|e| pg_err(&e, None)),
        }
    }
}

/// Shared slot the session fills before each statement, read by Cancel.
pub type CancelSlot = Arc<Mutex<Option<Canceller>>>;

enum Backend {
    MySql {
        conn: Conn,
        mariadb: bool,
    },
    /// PostgreSQL needs one connection per database.
    Postgres(HashMap<String, Client>),
}

pub struct Session {
    /// `None` for this computer's databases (#129) and the tests against a
    /// local server.
    tunnel: Option<Tunnel>,
    /// Local end of the tunnel (or the local server's port).
    port: u16,
    login: Login,
    backend: Backend,
    read_only: bool,
    cancel: CancelSlot,
}

fn my_opts(port: u16, login: &Login) -> OptsBuilder {
    let db = Some(login.database.trim()).filter(|d| !d.is_empty());
    OptsBuilder::new()
        .ip_or_hostname(Some("127.0.0.1"))
        .tcp_port(port)
        .prefer_socket(false)
        .user(Some(login.user.clone()))
        .pass(Some(login.password.expose().to_string()))
        .db_name(db)
        .tcp_connect_timeout(Some(CONNECT_TIMEOUT))
        // UPDATE reports matched rows, so "exactly one row" holds when a value is unchanged.
        .additional_capabilities(CapabilityFlags::CLIENT_FOUND_ROWS)
}

fn pg_connect(port: u16, login: &Login, database: &str) -> Result<Client, postgres::Error> {
    let mut c = postgres::Config::new();
    c.host("127.0.0.1")
        .port(port)
        .user(&login.user)
        .password(login.password.expose())
        .dbname(database)
        .connect_timeout(CONNECT_TIMEOUT)
        .application_name("Telepad");
    c.connect(NoTls)
}

fn my_err(e: &mysql::Error) -> String {
    match e {
        mysql::Error::MySqlError(me) => {
            format!("ERROR {} ({}): {}", me.code, me.state, me.message)
        }
        other => other.to_string(),
    }
}

/// PostgreSQL error with severity, detail, hint and the line in `sql`.
fn pg_err(e: &postgres::Error, sql: Option<&str>) -> String {
    let Some(db) = e.as_db_error() else {
        let mut s = e.to_string();
        let mut src = std::error::Error::source(e);
        while let Some(inner) = src {
            s.push_str(&format!(": {inner}"));
            src = inner.source();
        }
        return s;
    };
    let mut s = format!("{}: {}", db.severity(), db.message());
    if let (Some(ErrorPosition::Original(pos)), Some(sql)) = (db.position(), sql) {
        let upto: String = sql
            .chars()
            .take((*pos as usize).saturating_sub(1))
            .collect();
        s.push_str(&format!(" (line {})", upto.matches('\n').count() + 1));
    }
    if let Some(d) = db.detail() {
        s.push_str(&format!("\nDETAIL: {d}"));
    }
    if let Some(h) = db.hint() {
        s.push_str(&format!("\nHINT: {h}"));
    }
    s
}

fn my_text(v: &Value) -> Option<String> {
    match v {
        Value::NULL => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
        Value::Int(i) => Some(i.to_string()),
        Value::UInt(u) => Some(u.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Double(d) => Some(d.to_string()),
        Value::Date(y, mo, d, h, mi, s, us) => {
            let mut t = format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
            if *us > 0 {
                t.push_str(&format!(".{us:06}"));
            }
            Some(t)
        }
        Value::Time(neg, days, h, m, s, us) => {
            let hours = u64::from(*days) * 24 + u64::from(*h);
            let mut t = format!("{}{hours:02}:{m:02}:{s:02}", if *neg { "-" } else { "" });
            if *us > 0 {
                t.push_str(&format!(".{us:06}"));
            }
            Some(t)
        }
    }
}

fn my_cell(v: &Value) -> Cell {
    match v {
        Value::Bytes(b) => Cell::from_bytes(b),
        other => my_text(other).map_or(Cell::Null, Cell::Text),
    }
}

type TextRow = Vec<Option<String>>;

/// Prepared statement with parameters, every value as text.
fn my_rows(conn: &mut Conn, q: &str, params: Vec<Value>) -> Result<Vec<TextRow>, String> {
    let rows: Vec<mysql::Row> = conn.exec(q, params).map_err(|e| my_err(&e))?;
    Ok(rows
        .into_iter()
        .map(|r| r.unwrap().iter().map(my_text).collect())
        .collect())
}

fn pg_rows(
    c: &mut Client,
    q: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<TextRow>, String> {
    let rows = c.query(q, params).map_err(|e| pg_err(&e, None))?;
    Ok(rows
        .iter()
        .map(|r| {
            (0..r.len())
                .map(|i| r.get::<_, Option<String>>(i))
                .collect()
        })
        .collect())
}

fn at(r: &TextRow, i: usize) -> String {
    r.get(i).cloned().flatten().unwrap_or_default()
}

fn num(r: &TextRow, i: usize) -> Option<u64> {
    r.get(i)
        .cloned()
        .flatten()
        .and_then(|s| s.split('.').next().and_then(|n| n.parse().ok()))
}

/// MySQL rows of every result set of one query (text protocol).
fn my_query(
    conn: &mut Conn,
    q: &str,
    out: &mut Vec<(Option<Grid>, u64)>,
) -> Result<(), mysql::Error> {
    let mut result = conn.query_iter(q)?;
    while let Some(set) = result.iter() {
        let columns: Vec<String> = set
            .columns()
            .as_ref()
            .iter()
            .map(|c| c.name_str().into_owned())
            .collect();
        let affected = set.affected_rows();
        if columns.is_empty() {
            for _ in set {}
            out.push((None, affected));
            continue;
        }
        let mut grid = Grid {
            columns,
            ..Default::default()
        };
        for row in set {
            let row = row?;
            if grid.rows.len() >= MAX_ROWS {
                grid.truncated = true;
                continue;
            }
            grid.rows.push(row.unwrap().iter().map(my_cell).collect());
        }
        out.push((Some(grid), affected));
    }
    Ok(())
}

/// PostgreSQL results of one simple query: (grid, affected) per statement.
fn pg_messages(msgs: Vec<SimpleQueryMessage>) -> Vec<(Option<Grid>, u64)> {
    let mut out = Vec::new();
    let mut grid: Option<Grid> = None;
    for m in msgs {
        match m {
            SimpleQueryMessage::RowDescription(cols) => {
                grid = Some(Grid {
                    columns: cols.iter().map(|c| c.name().to_string()).collect(),
                    ..Default::default()
                });
            }
            SimpleQueryMessage::Row(row) => {
                let g = grid.get_or_insert_with(|| Grid {
                    columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                    ..Default::default()
                });
                if g.rows.len() >= MAX_ROWS {
                    g.truncated = true;
                    continue;
                }
                g.rows.push(
                    (0..row.len())
                        .map(|i| row.get(i).map_or(Cell::Null, Cell::from_text))
                        .collect(),
                );
            }
            SimpleQueryMessage::CommandComplete(n) => out.push((grid.take(), n)),
            _ => {}
        }
    }
    if let Some(g) = grid {
        out.push((Some(g), 0));
    }
    out
}

/// Split `varchar(255)` / `numeric(10,2)` / `timestamp(3) with time zone`
/// into the type name and what is between the parentheses.
pub fn split_type(full: &str) -> (String, String) {
    let full = full.trim();
    let (Some(open), Some(close)) = (full.find('('), full.rfind(')')) else {
        return (full.to_string(), String::new());
    };
    if close < open {
        return (full.to_string(), String::new());
    }
    let (before, inside, after) = (&full[..open], &full[open + 1..close], &full[close + 1..]);
    if after.trim().is_empty() {
        (before.trim().to_string(), inside.to_string())
    } else if after.starts_with(" with") && (before == "timestamp" || before == "time") {
        (format!("{before}{after}"), inside.to_string())
    } else {
        (full.to_string(), String::new())
    }
}

/// MySQL / MariaDB `COLUMN_DEFAULT` as the structure editor shows it.
pub fn my_default(raw: Option<&str>, extra: &str, nullable: bool, mariadb: bool) -> ColumnDefault {
    let Some(v) = raw else {
        return if nullable {
            ColumnDefault::Null
        } else {
            ColumnDefault::None
        };
    };
    let expression = |v: &str| {
        let u = v.to_ascii_uppercase();
        v.contains('(')
            || u.starts_with("CURRENT_TIMESTAMP")
            || u.starts_with("NOW")
            || u.starts_with("CURRENT_DATE")
    };
    if mariadb {
        if v == "NULL" {
            return ColumnDefault::Null;
        }
        if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
            let inner = &v[1..v.len() - 1];
            return ColumnDefault::Text(inner.replace("''", "'").replace("\\\\", "\\"));
        }
        if v.parse::<f64>().is_ok() {
            return ColumnDefault::Text(v.to_string());
        }
        return ColumnDefault::Expression(v.to_string());
    }
    if extra.contains("DEFAULT_GENERATED") || expression(v) {
        ColumnDefault::Expression(v.to_string())
    } else {
        ColumnDefault::Text(v.to_string())
    }
}

fn pg_action(code: &str) -> String {
    match code {
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "NO ACTION",
    }
    .to_string()
}

fn list(s: &str) -> Vec<String> {
    if s.is_empty() {
        vec![]
    } else {
        s.split('\u{1f}').map(str::to_string).collect()
    }
}

impl Session {
    /// Open the tunnel, log in, apply Read-only, and list the databases.
    pub fn connect(
        settings: &SshSettings,
        host: &HostEntry,
        target: &DbTarget,
        login: Login,
        read_only: bool,
    ) -> Result<(Self, ServerInfo), String> {
        if host.is_local() {
            return Self::connect_local(target, login, read_only);
        }
        let tunnel = Tunnel::open(settings, host, &target.address, target.port)?;
        let port = tunnel.local_port;
        Self::login(Some(tunnel), port, target.engine, login, read_only)
    }

    /// A database of this computer (#129, owner decision): straight to its
    /// port on the loopback address, no SSH; any other address is refused.
    pub fn connect_local(
        target: &DbTarget,
        login: Login,
        read_only: bool,
    ) -> Result<(Self, ServerInfo), String> {
        if target.address != "127.0.0.1" {
            return Err(format!(
                "{} is not this computer's loopback address",
                target.address
            ));
        }
        Self::login(None, target.port, target.engine, login, read_only)
    }

    fn login(
        tunnel: Option<Tunnel>,
        port: u16,
        engine: Engine,
        login: Login,
        read_only: bool,
    ) -> Result<(Self, ServerInfo), String> {
        let explain = |e: String| match &tunnel {
            Some(t) => t.explain(&e),
            None => e,
        };
        let backend = match engine {
            Engine::MySql => {
                let conn = Conn::new(my_opts(port, &login)).map_err(|e| explain(my_err(&e)))?;
                Backend::MySql {
                    conn,
                    mariadb: false,
                }
            }
            Engine::Postgres => {
                let db = pg_default_db(&login);
                let client =
                    pg_connect(port, &login, &db).map_err(|e| explain(pg_err(&e, None)))?;
                Backend::Postgres(HashMap::from([(db, client)]))
            }
        };
        let mut s = Session {
            tunnel,
            port,
            login,
            backend,
            read_only: false,
            cancel: Arc::new(Mutex::new(None)),
        };
        let version = s.version()?;
        if let Backend::MySql { mariadb, .. } = &mut s.backend {
            *mariadb = version.contains("MariaDB");
        }
        s.set_read_only(read_only)?;
        let databases = s.databases()?;
        Ok((s, ServerInfo { version, databases }))
    }

    pub fn engine(&self) -> Engine {
        match self.backend {
            Backend::MySql { .. } => Engine::MySql,
            Backend::Postgres(_) => Engine::Postgres,
        }
    }

    pub fn cancel_slot(&self) -> CancelSlot {
        self.cancel.clone()
    }

    /// Use a slot the caller already holds (the GUI keeps it for Cancel).
    pub fn with_cancel_slot(mut self, slot: CancelSlot) -> Self {
        if let (Ok(mut new), Ok(mut old)) = (slot.lock(), self.cancel.lock()) {
            *new = old.take();
        }
        self.cancel = slot;
        self
    }

    /// The tunnel is still up.
    pub fn alive(&mut self) -> bool {
        self.tunnel.as_mut().is_none_or(Tunnel::alive)
    }

    fn explain(&self, e: &str) -> String {
        match &self.tunnel {
            Some(t) => t.explain(e),
            None => e.to_string(),
        }
    }

    fn fail(&mut self, e: String) -> String {
        if !self.alive() {
            format!("{e} — the SSH tunnel has closed; reconnect")
        } else {
            self.explain(&e)
        }
    }

    fn arm(&self, c: Canceller) {
        if let Ok(mut slot) = self.cancel.lock() {
            *slot = Some(c);
        }
    }

    fn my(&mut self) -> Option<&mut Conn> {
        match &mut self.backend {
            Backend::MySql { conn, .. } => Some(conn),
            Backend::Postgres(_) => None,
        }
    }

    /// The PostgreSQL connection for `database` (opened on first use).
    fn pg(&mut self, database: &str) -> Result<&mut Client, String> {
        let db = if database.is_empty() {
            pg_default_db(&self.login)
        } else {
            database.to_string()
        };
        let port = self.port;
        let read_only = self.read_only;
        let Backend::Postgres(clients) = &mut self.backend else {
            return Err("not a PostgreSQL connection".into());
        };
        if clients.get(&db).is_none_or(Client::is_closed) {
            let mut c = pg_connect(port, &self.login, &db).map_err(|e| {
                let e = pg_err(&e, None);
                match &self.tunnel {
                    Some(t) => t.explain(&e),
                    None => e,
                }
            })?;
            if read_only {
                c.batch_execute("SET default_transaction_read_only = on")
                    .map_err(|e| pg_err(&e, None))?;
            }
            clients.insert(db.clone(), c);
        }
        let client = clients.get_mut(&db).ok_or("connection lost")?;
        if let Ok(mut slot) = self.cancel.lock() {
            *slot = Some(Canceller::Postgres(client.cancel_token()));
        }
        Ok(client)
    }

    fn arm_mysql(&mut self) {
        if let Some(c) = self.my() {
            let id = c.connection_id();
            let canceller = Canceller::MySql {
                port: self.port,
                user: self.login.user.clone(),
                password: self.login.password.clone(),
                id,
            };
            self.arm(canceller);
        }
    }

    pub fn version(&mut self) -> Result<String, String> {
        let r = match &mut self.backend {
            Backend::MySql { conn, .. } => conn
                .query_first::<String, _>("SELECT VERSION()")
                .map_err(|e| my_err(&e))
                .map(Option::unwrap_or_default),
            Backend::Postgres(_) => self
                .pg("")
                .and_then(|c| {
                    pg_rows(
                        c,
                        "SELECT 'PostgreSQL ' || current_setting('server_version')",
                        &[],
                    )
                })
                .map(|rows| rows.first().map(|r| at(r, 0)).unwrap_or_default()),
        };
        r.map_err(|e| self.fail(e))
    }

    /// Read-only on: the server refuses writes for this session.
    pub fn set_read_only(&mut self, on: bool) -> Result<(), String> {
        let r = match &mut self.backend {
            Backend::MySql { conn, .. } => conn
                .query_drop(if on {
                    "SET SESSION TRANSACTION READ ONLY"
                } else {
                    "SET SESSION TRANSACTION READ WRITE"
                })
                .map_err(|e| my_err(&e)),
            Backend::Postgres(clients) => clients.values_mut().try_for_each(|c| {
                c.batch_execute(if on {
                    "SET default_transaction_read_only = on"
                } else {
                    "SET default_transaction_read_only = off"
                })
                .map_err(|e| pg_err(&e, None))
            }),
        };
        r.map_err(|e| self.fail(e))?;
        self.read_only = on;
        Ok(())
    }

    pub fn databases(&mut self) -> Result<Vec<DatabaseInfo>, String> {
        let rows = match &mut self.backend {
            Backend::MySql { conn, .. } => my_rows(
                conn,
                "SELECT s.SCHEMA_NAME, SUM(t.DATA_LENGTH + t.INDEX_LENGTH) \
                 FROM information_schema.SCHEMATA s \
                 LEFT JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
                 GROUP BY s.SCHEMA_NAME ORDER BY s.SCHEMA_NAME",
                vec![],
            ),
            Backend::Postgres(_) => self.pg("").and_then(|c| {
                pg_rows(
                    c,
                    "SELECT datname::text, CASE WHEN has_database_privilege(datname, 'CONNECT') \
                     THEN pg_database_size(datname)::text END \
                     FROM pg_database WHERE NOT datistemplate AND datallowconn ORDER BY 1",
                    &[],
                )
            }),
        };
        let rows = rows.map_err(|e| self.fail(e))?;
        Ok(rows
            .iter()
            .map(|r| DatabaseInfo {
                name: at(r, 0),
                size: num(r, 1),
            })
            .collect())
    }

    /// PostgreSQL schemas of `database` (user schemas first, `public` on top).
    pub fn schemas(&mut self, database: &str) -> Result<Vec<String>, String> {
        let r = self.pg(database).and_then(|c| {
            pg_rows(
                c,
                "SELECT nspname::text FROM pg_namespace \
                 WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' \
                 ORDER BY nspname <> 'public', 1",
                &[],
            )
        });
        let rows = r.map_err(|e| self.fail(e))?;
        Ok(rows.iter().map(|r| at(r, 0)).collect())
    }

    pub fn tables(&mut self, database: &str, schema: &str) -> Result<Vec<TableSummary>, String> {
        let engine = self.engine();
        let rows = match &mut self.backend {
            Backend::MySql { conn, .. } => my_rows(
                conn,
                "SELECT TABLE_NAME, TABLE_TYPE, ENGINE, TABLE_ROWS, DATA_LENGTH + INDEX_LENGTH, TABLE_COMMENT \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME",
                vec![Value::from(database)],
            ),
            Backend::Postgres(_) => self.pg(database).and_then(|c| {
                pg_rows(
                    c,
                    "SELECT c.relname::text, c.relkind::text, NULL::text, \
                     CASE WHEN c.reltuples >= 0 THEN c.reltuples::bigint::text END, \
                     pg_total_relation_size(c.oid)::text, obj_description(c.oid, 'pg_class') \
                     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') ORDER BY 1",
                    &[&schema],
                )
            }),
        };
        let rows = rows.map_err(|e| self.fail(e))?;
        Ok(rows
            .iter()
            .map(|r| {
                let kind_raw = at(r, 1);
                let (kind, engine_text) = match engine {
                    Engine::MySql => (
                        if kind_raw.contains("VIEW") {
                            TableKind::View
                        } else {
                            TableKind::Table
                        },
                        at(r, 2),
                    ),
                    Engine::Postgres => match kind_raw.as_str() {
                        "v" => (TableKind::View, "view".into()),
                        "m" => (TableKind::View, "materialized view".into()),
                        "p" => (TableKind::Table, "partitioned".into()),
                        "f" => (TableKind::Table, "foreign".into()),
                        _ => (TableKind::Table, "table".into()),
                    },
                };
                TableSummary {
                    name: at(r, 0),
                    kind,
                    engine: engine_text,
                    rows: num(r, 3),
                    size: num(r, 4),
                    // MariaDB / MySQL report the comment "VIEW" for views.
                    comment: if kind == TableKind::View && at(r, 5) == "VIEW" {
                        String::new()
                    } else {
                        at(r, 5)
                    },
                }
            })
            .collect())
    }

    /// Columns, indexes, foreign keys (and MySQL's CREATE statement).
    pub fn table_info(&mut self, t: &TableRef) -> Result<TableInfo, String> {
        let r = match self.engine() {
            Engine::MySql => self.my_table_info(t),
            Engine::Postgres => self.pg_table_info(t),
        };
        r.map_err(|e| self.fail(e))
    }

    fn my_table_info(&mut self, t: &TableRef) -> Result<TableInfo, String> {
        let Backend::MySql { conn, mariadb } = &mut self.backend else {
            return Err("not a MySQL connection".into());
        };
        let mariadb = *mariadb;
        let p = || vec![Value::from(&t.database), Value::from(&t.table)];
        let kind = my_rows(
            conn,
            "SELECT TABLE_TYPE FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
            p(),
        )?;
        let kind = if kind.first().is_some_and(|r| at(r, 0).contains("VIEW")) {
            TableKind::View
        } else {
            TableKind::Table
        };
        let columns = my_rows(
            conn,
            "SELECT COLUMN_NAME, COLUMN_TYPE, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT, EXTRA, COLUMN_COMMENT, COLLATION_NAME \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
            p(),
        )?
        .iter()
        .map(|r| {
            let full = at(r, 1);
            let nullable = at(r, 3) == "YES";
            let extra = at(r, 5);
            let (_, length) = split_type(full.split(" unsigned").next().unwrap_or(&full));
            Column {
                name: at(r, 0),
                type_name: at(r, 2).to_ascii_uppercase(),
                length,
                unsigned: full.contains(" unsigned"),
                nullable,
                default: my_default(r.get(4).cloned().flatten().as_deref(), &extra, nullable, mariadb),
                auto_increment: extra.to_ascii_lowercase().contains("auto_increment"),
                comment: at(r, 6),
                collation: at(r, 7),
                full_type: full,
            }
        })
        .collect();
        let mut indexes: Vec<Index> = Vec::new();
        for r in my_rows(
            conn,
            "SELECT INDEX_NAME, NON_UNIQUE, INDEX_TYPE, COLUMN_NAME FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX",
            p(),
        )? {
            let name = at(&r, 0);
            if indexes.last().is_none_or(|i| i.name != name) {
                let kind = if name == "PRIMARY" {
                    IndexKind::Primary
                } else if at(&r, 2) == "FULLTEXT" {
                    IndexKind::Fulltext
                } else if at(&r, 1) == "0" {
                    IndexKind::Unique
                } else {
                    IndexKind::Key
                };
                indexes.push(Index {
                    name: name.clone(),
                    kind,
                    columns: vec![],
                    constraint: false,
                });
            }
            if let Some(i) = indexes.last_mut() {
                i.columns.push(at(&r, 3));
            }
        }
        let mut foreign_keys: Vec<ForeignKey> = Vec::new();
        for r in my_rows(
            conn,
            "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, \
             k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
               ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
            p(),
        )? {
            let name = at(&r, 0);
            if foreign_keys.last().is_none_or(|f| f.name != name) {
                foreign_keys.push(ForeignKey {
                    name: name.clone(),
                    ref_schema: at(&r, 2),
                    ref_table: at(&r, 3),
                    on_update: at(&r, 5),
                    on_delete: at(&r, 6),
                    ..Default::default()
                });
            }
            if let Some(f) = foreign_keys.last_mut() {
                f.columns.push(at(&r, 1));
                f.ref_columns.push(at(&r, 4));
            }
        }
        let create = format!("SHOW CREATE TABLE {}", sql::table_path(Engine::MySql, t));
        let create_sql = conn
            .query_first::<mysql::Row, _>(create)
            .map_err(|e| my_err(&e))?
            .and_then(|r| r.as_ref(1).and_then(my_text))
            .unwrap_or_default();
        Ok(TableInfo {
            table: t.clone(),
            kind,
            columns,
            indexes,
            foreign_keys,
            create_sql,
        })
    }

    fn pg_table_info(&mut self, t: &TableRef) -> Result<TableInfo, String> {
        let path = sql::table_path(Engine::Postgres, t);
        let c = self.pg(&t.database)?;
        let p: [&(dyn ToSql + Sync); 1] = [&path];
        let kind = pg_rows(
            c,
            "SELECT relkind::text FROM pg_class WHERE oid = ($1::text)::regclass",
            &p,
        )?;
        let kind = match kind.first().map(|r| at(r, 0)).as_deref() {
            Some("v" | "m") => TableKind::View,
            _ => TableKind::Table,
        };
        let columns = pg_rows(
            c,
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), (NOT a.attnotnull)::text, \
             pg_get_expr(d.adbin, d.adrelid), a.attidentity::text, col_description(a.attrelid, a.attnum) \
             FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = ($1::text)::regclass AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
            &p,
        )?
        .iter()
        .map(|r| {
            let full = at(r, 1);
            let (type_name, length) = split_type(&full);
            let identity = matches!(at(r, 4).as_str(), "a" | "d");
            Column {
                name: at(r, 0),
                type_name,
                length,
                unsigned: false,
                nullable: at(r, 2) == "true",
                default: match r.get(3).cloned().flatten() {
                    Some(e) if !identity => ColumnDefault::Expression(e),
                    _ => ColumnDefault::None,
                },
                auto_increment: identity,
                comment: at(r, 5),
                collation: String::new(),
                full_type: full,
            }
        })
        .collect();
        let indexes = pg_rows(
            c,
            "SELECT i.relname::text, ix.indisprimary::text, ix.indisunique::text, \
             (SELECT string_agg(a.attname::text, chr(31) ORDER BY k.n) \
                FROM unnest(ix.indkey::int2[]) WITH ORDINALITY AS k(attnum, n) \
                JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = k.attnum), \
             (EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = ix.indexrelid AND c.contype IN ('p', 'u')))::text \
             FROM pg_index ix JOIN pg_class i ON i.oid = ix.indexrelid \
             WHERE ix.indrelid = ($1::text)::regclass ORDER BY ix.indisprimary DESC, 1",
            &p,
        )?
        .iter()
        .map(|r| Index {
            name: at(r, 0),
            kind: if at(r, 1) == "true" {
                IndexKind::Primary
            } else if at(r, 2) == "true" {
                IndexKind::Unique
            } else {
                IndexKind::Key
            },
            columns: list(&at(r, 3)),
            constraint: at(r, 4) == "true",
        })
        .collect();
        let cols = |key: &str, rel: &str| {
            format!(
                "(SELECT string_agg(a.attname::text, chr(31) ORDER BY k.n) FROM unnest(c.{key}) WITH ORDINALITY AS k(attnum, n) \
                 JOIN pg_attribute a ON a.attrelid = c.{rel} AND a.attnum = k.attnum)"
            )
        };
        let foreign_keys = pg_rows(
            c,
            &format!(
                "SELECT c.conname::text, {}, rn.nspname::text, rc.relname::text, {}, c.confupdtype::text, c.confdeltype::text \
                 FROM pg_constraint c JOIN pg_class rc ON rc.oid = c.confrelid JOIN pg_namespace rn ON rn.oid = rc.relnamespace \
                 WHERE c.contype = 'f' AND c.conrelid = ($1::text)::regclass ORDER BY 1",
                cols("conkey", "conrelid"),
                cols("confkey", "confrelid")
            ),
            &p,
        )?
        .iter()
        .map(|r| ForeignKey {
            name: at(r, 0),
            columns: list(&at(r, 1)),
            ref_schema: at(r, 2),
            ref_table: at(r, 3),
            ref_columns: list(&at(r, 4)),
            on_update: pg_action(&at(r, 5)),
            on_delete: pg_action(&at(r, 6)),
        })
        .collect();
        Ok(TableInfo {
            table: t.clone(),
            kind,
            columns,
            indexes,
            foreign_keys,
            create_sql: String::new(),
        })
    }

    /// One page of the Data tab.
    pub fn page(
        &mut self,
        t: &TableRef,
        filter: &str,
        sort: &sql::Sort,
        offset: u64,
        limit: u64,
    ) -> Result<Grid, String> {
        let q = sql::select_page(self.engine(), t, filter, sort, offset, limit);
        let r = match self.engine() {
            Engine::MySql => {
                self.arm_mysql();
                let conn = self.my().ok_or("no connection")?;
                let mut out = Vec::new();
                my_query(conn, &q, &mut out)
                    .map_err(|e| my_err(&e))
                    .map(|()| out)
            }
            Engine::Postgres => self.pg(&t.database).and_then(|c| {
                c.simple_query(&q)
                    .map(pg_messages)
                    .map_err(|e| pg_err(&e, Some(&q)))
            }),
        };
        let sets = r.map_err(|e| self.fail(e))?;
        Ok(sets.into_iter().find_map(|(g, _)| g).unwrap_or_default())
    }

    /// Query tab: run statements in order in `database` / `schema`; stops at
    /// the first error.
    pub fn run(&mut self, database: &str, schema: &str, statements: &[String]) -> Vec<ResultSet> {
        let mut out = Vec::new();
        if let Err(e) = self.use_context(database, schema) {
            out.push(ResultSet {
                statement: String::new(),
                error: Some(e),
                ..Default::default()
            });
            return out;
        }
        for st in statements {
            let started = Instant::now();
            let r = match self.engine() {
                Engine::MySql => {
                    self.arm_mysql();
                    match self.my() {
                        Some(conn) => {
                            let mut sets = Vec::new();
                            my_query(conn, st, &mut sets)
                                .map_err(|e| my_err(&e))
                                .map(|()| sets)
                        }
                        None => Err("no connection".into()),
                    }
                }
                Engine::Postgres => self.pg(database).and_then(|c| {
                    c.simple_query(st)
                        .map(pg_messages)
                        .map_err(|e| pg_err(&e, Some(st)))
                }),
            };
            let millis = started.elapsed().as_millis() as u64;
            match r {
                Ok(sets) if sets.is_empty() => out.push(ResultSet {
                    statement: st.clone(),
                    millis,
                    affected: Some(0),
                    ..Default::default()
                }),
                Ok(sets) => {
                    for (grid, n) in sets {
                        out.push(ResultSet {
                            statement: st.clone(),
                            affected: if grid.is_none() { Some(n) } else { None },
                            grid,
                            millis,
                            error: None,
                        });
                    }
                }
                Err(e) => {
                    let e = self.fail(e);
                    out.push(ResultSet {
                        statement: st.clone(),
                        millis,
                        error: Some(e),
                        ..Default::default()
                    });
                    break;
                }
            }
        }
        out
    }

    fn use_context(&mut self, database: &str, schema: &str) -> Result<(), String> {
        match self.engine() {
            Engine::MySql => {
                if database.is_empty() {
                    return Ok(());
                }
                let conn = self.my().ok_or("no connection")?;
                conn.select_db(database).map_err(|e| my_err(&e))
            }
            Engine::Postgres => {
                let c = self.pg(database)?;
                if schema.is_empty() {
                    return Ok(());
                }
                let q = format!(
                    "SET search_path TO {}, public",
                    sql::quote_ident(Engine::Postgres, schema)
                );
                c.batch_execute(&q).map_err(|e| pg_err(&e, None))
            }
        }
    }

    /// Grid changes in one transaction; each statement must touch exactly one
    /// row, else everything is rolled back. Returns the rows changed.
    pub fn apply(&mut self, database: &str, statements: &[Statement]) -> Result<u64, String> {
        let engine = self.engine();
        let one = |n: u64, st: &Statement| {
            if n == 1 {
                Ok(())
            } else {
                Err(format!(
                    "{n} rows matched instead of 1 — nothing was saved:\n{}",
                    st.display(engine)
                ))
            }
        };
        let r = match engine {
            Engine::MySql => {
                let conn = self.my().ok_or("no connection")?;
                (|| {
                    let mut tx = conn
                        .start_transaction(TxOpts::default())
                        .map_err(|e| my_err(&e))?;
                    let mut total = 0;
                    for st in statements {
                        let params: Vec<Value> = st
                            .params
                            .iter()
                            .map(|p| p.as_ref().map_or(Value::NULL, |v| Value::from(v.as_str())))
                            .collect();
                        tx.exec_drop(&st.sql, params).map_err(|e| my_err(&e))?;
                        let n = tx.affected_rows();
                        one(n, st)?;
                        total += n;
                    }
                    tx.commit().map_err(|e| my_err(&e))?;
                    Ok(total)
                })()
            }
            Engine::Postgres => self.pg(database).and_then(|c| {
                let mut tx = c.transaction().map_err(|e| pg_err(&e, None))?;
                let mut total = 0;
                for st in statements {
                    let params: Vec<&(dyn ToSql + Sync)> =
                        st.params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
                    let n = tx
                        .execute(st.sql.as_str(), &params)
                        .map_err(|e| pg_err(&e, Some(&st.sql)))?;
                    one(n, st)?;
                    total += n;
                }
                tx.commit().map_err(|e| pg_err(&e, None))?;
                Ok(total)
            }),
        };
        r.map_err(|e| self.fail(e))
    }

    /// Structure changes. PostgreSQL runs them in one transaction; MySQL
    /// commits each DDL statement on its own (the server does that anyway).
    pub fn ddl(&mut self, database: &str, statements: &[String]) -> Result<(), String> {
        let r = match self.engine() {
            Engine::MySql => {
                let conn = self.my().ok_or("no connection")?;
                (|| {
                    if !database.is_empty() {
                        conn.select_db(database).map_err(|e| my_err(&e))?;
                    }
                    for st in statements {
                        conn.query_drop(st)
                            .map_err(|e| format!("{}\n{st}", my_err(&e)))?;
                    }
                    Ok(())
                })()
            }
            Engine::Postgres => self.pg(database).and_then(|c| {
                let mut tx = c.transaction().map_err(|e| pg_err(&e, None))?;
                for st in statements {
                    tx.batch_execute(st)
                        .map_err(|e| format!("{}\n{st}", pg_err(&e, Some(st))))?;
                }
                tx.commit().map_err(|e| pg_err(&e, None))
            }),
        };
        r.map_err(|e| self.fail(e))
    }
}

fn pg_default_db(login: &Login) -> String {
    let d = login.database.trim();
    if d.is_empty() {
        "postgres".into()
    } else {
        d.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_types() {
        let s = |t: &str| {
            let (a, b) = split_type(t);
            (a, b)
        };
        assert_eq!(s("varchar(255)"), ("varchar".into(), "255".into()));
        assert_eq!(s("numeric(10,2)"), ("numeric".into(), "10,2".into()));
        assert_eq!(s("enum('a','b(c)')"), ("enum".into(), "'a','b(c)'".into()));
        assert_eq!(s("integer"), ("integer".into(), String::new()));
        assert_eq!(
            s("timestamp(3) with time zone"),
            ("timestamp with time zone".into(), "3".into())
        );
        assert_eq!(
            s("character varying(20)[]"),
            ("character varying(20)[]".into(), String::new())
        );
    }

    #[test]
    fn reads_mysql_and_mariadb_defaults() {
        use ColumnDefault::{Expression, Null, Text};
        assert_eq!(my_default(None, "", true, false), Null);
        assert_eq!(my_default(None, "", false, false), ColumnDefault::None);
        assert_eq!(
            my_default(Some("abc"), "", false, false),
            Text("abc".into())
        );
        assert_eq!(
            my_default(Some("CURRENT_TIMESTAMP"), "DEFAULT_GENERATED", false, false),
            Expression("CURRENT_TIMESTAMP".into())
        );
        assert_eq!(my_default(Some("NULL"), "", true, true), Null);
        assert_eq!(
            my_default(Some("'it''s'"), "", false, true),
            Text("it's".into())
        );
        assert_eq!(my_default(Some("0"), "", false, true), Text("0".into()));
        assert_eq!(
            my_default(Some("current_timestamp()"), "", false, true),
            Expression("current_timestamp()".into())
        );
    }

    #[test]
    fn mysql_values_as_text() {
        assert_eq!(my_text(&Value::NULL), None);
        assert_eq!(my_text(&Value::Int(-5)).as_deref(), Some("-5"));
        assert_eq!(
            my_text(&Value::Date(2026, 1, 2, 3, 4, 5, 0)).as_deref(),
            Some("2026-01-02 03:04:05")
        );
        assert_eq!(
            my_text(&Value::Time(true, 1, 2, 3, 4, 0)).as_deref(),
            Some("-26:03:04")
        );
        assert_eq!(
            my_cell(&Value::Bytes(vec![0xff])),
            Cell::Bytes("0xFF".into(), 1)
        );
    }
}

/// Against real servers without SSH: set `TELEPAD_TEST_MYSQL` and / or
/// `TELEPAD_TEST_PG` to `<port> <user> <password>` of a throwaway server on
/// 127.0.0.1 (CI starts MariaDB and PostgreSQL service containers). Skipped
/// when unset. The tests create and drop a database `telepad_test`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::db::sql::{delete_row, insert_row, update_row};

    pub(crate) fn server(var: &str) -> Option<(u16, Login)> {
        let v = std::env::var(var).ok()?;
        let mut it = v.split_whitespace();
        let port = it.next()?.parse().ok()?;
        let user = it.next()?.to_string();
        let password = Secret::new(it.next().unwrap_or(""));
        Some((
            port,
            Login {
                user,
                password,
                database: String::new(),
            },
        ))
    }

    fn ok(results: &[ResultSet]) {
        for r in results {
            assert!(r.error.is_none(), "{}: {:?}", r.statement, r.error);
        }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn text(c: &Cell) -> &str {
        c.as_value().unwrap_or("NULL")
    }

    #[test]
    fn mysql_browse_edit_read_only_and_cancel() {
        let Some((port, login)) = server("TELEPAD_TEST_MYSQL") else {
            eprintln!("TELEPAD_TEST_MYSQL not set: skipped");
            return;
        };
        let (mut s, info) = Session::login(None, port, Engine::MySql, login, false).unwrap();
        assert!(!info.version.is_empty());
        ok(&s.run("", "", &strings(&[
            "DROP DATABASE IF EXISTS telepad_test",
            "CREATE DATABASE telepad_test",
            "CREATE TABLE telepad_test.items (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(50) NOT NULL DEFAULT 'x', \
             note TEXT NULL, price DECIMAL(10,2) UNSIGNED, created TIMESTAMP NULL DEFAULT CURRENT_TIMESTAMP, \
             UNIQUE KEY u_name (name), KEY k_price (price)) ENGINE=InnoDB COMMENT='made-up rows'",
            "CREATE TABLE telepad_test.parts (id INT PRIMARY KEY, item INT, CONSTRAINT fk_item FOREIGN KEY (item) REFERENCES telepad_test.items (id) ON DELETE CASCADE)",
            "INSERT INTO telepad_test.items (name, note, price) VALUES ('alpha', NULL, 1.50), ('it''s', 'back\\\\slash', 2)",
        ])));
        assert!(
            s.databases()
                .unwrap()
                .iter()
                .any(|d| d.name == "telepad_test")
        );
        let tables = s.tables("telepad_test", "").unwrap();
        let items = tables.iter().find(|t| t.name == "items").unwrap();
        assert_eq!(
            (items.kind, items.engine.as_str(), items.comment.as_str()),
            (TableKind::Table, "InnoDB", "made-up rows")
        );

        let t = TableRef {
            database: "telepad_test".into(),
            schema: String::new(),
            table: "items".into(),
        };
        let info = s.table_info(&t).unwrap();
        assert_eq!(info.row_key(), Some(vec!["id".to_string()]));
        let col = |n: &str| info.column(n).unwrap().clone();
        assert!(col("id").auto_increment);
        assert_eq!(col("name").default, ColumnDefault::Text("x".into()));
        assert!(
            matches!(col("created").default, ColumnDefault::Expression(_)),
            "{:?}",
            col("created").default
        );
        assert_eq!(
            (
                col("price").type_name.as_str(),
                col("price").length.as_str(),
                col("price").unsigned
            ),
            ("DECIMAL", "10,2", true)
        );
        assert!(
            info.indexes
                .iter()
                .any(|i| i.name == "u_name" && i.kind == IndexKind::Unique)
        );
        assert!(info.create_sql.contains("CREATE TABLE `items`"));
        let parts = s
            .table_info(&TableRef {
                table: "parts".into(),
                ..t.clone()
            })
            .unwrap();
        assert_eq!(parts.foreign_keys[0].ref_table, "items");
        assert_eq!(parts.foreign_keys[0].on_delete, "CASCADE");

        let page = s
            .page(&t, "price > 1", &Some(("name".into(), true)), 0, 1000)
            .unwrap();
        assert_eq!(page.columns[1], "name");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(text(&page.rows[0][1]), "it's");
        assert_eq!(text(&page.rows[0][2]), "back\\slash");
        assert_eq!(page.rows[1][2], Cell::Null);

        let key = |id: &str| vec![("id".to_string(), Some(id.to_string()))];
        let up = update_row(
            Engine::MySql,
            &t,
            &info.columns,
            &key("1"),
            &[("note".into(), Some("new 'note'".into()))],
        );
        let ins = insert_row(
            Engine::MySql,
            &t,
            &info.columns,
            &[("name".into(), Some("gamma".into()))],
        );
        assert_eq!(s.apply("telepad_test", &[up.clone(), ins]).unwrap(), 2);
        // Unchanged value still counts as one matched row.
        assert_eq!(s.apply("telepad_test", &[up]).unwrap(), 1);
        let missing = update_row(
            Engine::MySql,
            &t,
            &info.columns,
            &key("999"),
            &[("note".into(), None)],
        );
        let del = delete_row(Engine::MySql, &t, &info.columns, &key("2"));
        let err = s.apply("telepad_test", &[del, missing]).unwrap_err();
        assert!(err.contains("0 rows matched"), "{err}");
        let count = s.run(
            "telepad_test",
            "",
            &strings(&["SELECT COUNT(*) FROM items"]),
        );
        assert_eq!(
            text(&count[0].grid.as_ref().unwrap().rows[0][0]),
            "3",
            "delete was rolled back"
        );

        s.set_read_only(true).unwrap();
        let r = s.run(
            "telepad_test",
            "",
            &strings(&["INSERT INTO items (name) VALUES ('ro')"]),
        );
        assert!(r[0].error.is_some(), "read-only session refused the insert");
        s.set_read_only(false).unwrap();

        let r = s.run(
            "telepad_test",
            "",
            &strings(&["SELECT 1 AS one", "SELEC nonsense", "SELECT 2"]),
        );
        assert_eq!(r.len(), 2, "stops at the first error");
        assert!(
            r[1].error.as_deref().unwrap_or("").contains("1064"),
            "{:?}",
            r[1].error
        );

        let slot = s.cancel_slot();
        let killer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            slot.lock().unwrap().as_ref().map(Canceller::cancel)
        });
        let started = Instant::now();
        let _ = s.run("", "", &strings(&["SELECT SLEEP(20)"]));
        assert!(started.elapsed() < Duration::from_secs(10), "cancelled");
        assert!(matches!(killer.join().unwrap(), Some(Ok(()))));

        // Structure editor round trip (#54).
        let info = s.table_info(&t).unwrap();
        let mut e = crate::db::ddl::TableEdit::from_info(&info);
        e.columns[2].column.name = "memo".into();
        e.columns
            .push(crate::db::ddl::TableEdit::blank_column(Engine::MySql, 9));
        e.indexes.retain(|i| i.index.name != "k_price");
        let stmts = crate::db::ddl::alter(Engine::MySql, &info, &e).unwrap();
        s.ddl("telepad_test", &stmts).unwrap();
        let after = s.table_info(&t).unwrap();
        assert!(after.column("memo").is_some() && after.column("column9").is_some());
        assert!(!after.indexes.iter().any(|i| i.name == "k_price"));
        assert!(
            crate::db::ddl::alter(
                Engine::MySql,
                &after,
                &crate::db::ddl::TableEdit::from_info(&after)
            )
            .unwrap()
            .is_empty()
        );
        let nt = TableRef {
            table: "fresh".into(),
            ..t.clone()
        };
        let create = crate::db::ddl::create(
            Engine::MySql,
            &nt,
            &crate::db::ddl::TableEdit::new_table(Engine::MySql),
        )
        .unwrap();
        s.ddl("telepad_test", &create).unwrap();
        s.ddl(
            "telepad_test",
            &[crate::db::ddl::rename(Engine::MySql, &nt, "fresh2").unwrap()],
        )
        .unwrap();
        let nt2 = TableRef {
            table: "fresh2".into(),
            ..t.clone()
        };
        s.ddl(
            "telepad_test",
            &[crate::db::ddl::drop(
                Engine::MySql,
                &nt2,
                TableKind::Table,
                "InnoDB",
            )],
        )
        .unwrap();
        assert!(
            !s.tables("telepad_test", "")
                .unwrap()
                .iter()
                .any(|x| x.name.starts_with("fresh"))
        );

        ok(&s.run("", "", &strings(&["DROP DATABASE telepad_test"])));
    }

    #[test]
    fn postgres_browse_edit_read_only_and_cancel() {
        let Some((port, login)) = server("TELEPAD_TEST_PG") else {
            eprintln!("TELEPAD_TEST_PG not set: skipped");
            return;
        };
        let (mut s, info) = Session::login(None, port, Engine::Postgres, login, false).unwrap();
        assert!(info.version.contains("PostgreSQL"));
        ok(&s.run(
            "postgres",
            "",
            &strings(&[
                "DROP DATABASE IF EXISTS telepad_test",
                "CREATE DATABASE telepad_test",
            ]),
        ));
        ok(&s.run("telepad_test", "", &strings(&[
            "CREATE SCHEMA shop",
            "CREATE TABLE shop.items (id integer GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, name varchar(50) NOT NULL DEFAULT 'x', \
             note text, price numeric(10,2), created timestamp(3) with time zone DEFAULT now(), CONSTRAINT u_name UNIQUE (name))",
            "CREATE INDEX k_price ON shop.items (price)",
            "CREATE TABLE shop.parts (id int PRIMARY KEY, item int REFERENCES shop.items (id) ON DELETE CASCADE)",
            "COMMENT ON TABLE shop.items IS 'made-up rows'",
            "INSERT INTO shop.items (name, note, price) VALUES ('alpha', NULL, 1.5), ('it''s', 'back\\slash', 2)",
        ])));
        assert!(
            s.databases()
                .unwrap()
                .iter()
                .any(|d| d.name == "telepad_test")
        );
        assert_eq!(
            s.schemas("telepad_test").unwrap(),
            vec!["public".to_string(), "shop".to_string()]
        );
        let tables = s.tables("telepad_test", "shop").unwrap();
        assert_eq!(
            tables.iter().find(|t| t.name == "items").unwrap().comment,
            "made-up rows"
        );

        let t = TableRef {
            database: "telepad_test".into(),
            schema: "shop".into(),
            table: "items".into(),
        };
        let info = s.table_info(&t).unwrap();
        assert_eq!(info.row_key(), Some(vec!["id".to_string()]));
        let col = |n: &str| info.column(n).unwrap().clone();
        assert!(col("id").auto_increment);
        assert_eq!(
            (col("name").type_name.as_str(), col("name").length.as_str()),
            ("character varying", "50")
        );
        assert_eq!(
            (
                col("created").type_name.as_str(),
                col("created").length.as_str()
            ),
            ("timestamp with time zone", "3")
        );
        assert!(
            info.indexes
                .iter()
                .any(|i| i.name == "u_name" && i.kind == IndexKind::Unique && i.constraint)
        );
        assert!(
            info.indexes
                .iter()
                .any(|i| i.name == "k_price" && i.kind == IndexKind::Key && !i.constraint)
        );
        let parts = s
            .table_info(&TableRef {
                table: "parts".into(),
                ..t.clone()
            })
            .unwrap();
        assert_eq!(parts.foreign_keys[0].ref_table, "items");
        assert_eq!(parts.foreign_keys[0].on_delete, "CASCADE");

        let page = s
            .page(&t, "price > 1", &Some(("name".into(), true)), 0, 1000)
            .unwrap();
        assert_eq!(page.rows.len(), 2);
        assert_eq!(text(&page.rows[0][1]), "it's");
        assert_eq!(text(&page.rows[0][2]), "back\\slash");
        assert_eq!(page.rows[1][2], Cell::Null);

        let key = |id: &str| vec![("id".to_string(), Some(id.to_string()))];
        let up = update_row(
            Engine::Postgres,
            &t,
            &info.columns,
            &key("1"),
            &[("price".into(), Some("9.25".into())), ("note".into(), None)],
        );
        let ins = insert_row(
            Engine::Postgres,
            &t,
            &info.columns,
            &[
                ("name".into(), Some("gamma".into())),
                ("price".into(), Some("3".into())),
            ],
        );
        assert_eq!(s.apply("telepad_test", &[up, ins]).unwrap(), 2);
        let missing = update_row(
            Engine::Postgres,
            &t,
            &info.columns,
            &key("999"),
            &[("note".into(), None)],
        );
        let del = delete_row(Engine::Postgres, &t, &info.columns, &key("2"));
        assert!(
            s.apply("telepad_test", &[del, missing])
                .unwrap_err()
                .contains("0 rows matched")
        );
        let count = s.run(
            "telepad_test",
            "shop",
            &strings(&["SELECT count(*) FROM items"]),
        );
        assert_eq!(
            text(&count[0].grid.as_ref().unwrap().rows[0][0]),
            "3",
            "search_path + rollback"
        );

        s.set_read_only(true).unwrap();
        let r = s.run(
            "telepad_test",
            "shop",
            &strings(&["INSERT INTO items (name) VALUES ('ro')"]),
        );
        assert!(
            r[0].error.as_deref().unwrap_or("").contains("read-only"),
            "{:?}",
            r[0].error
        );
        s.set_read_only(false).unwrap();

        let r = s.run(
            "telepad_test",
            "",
            &strings(&["SELECT 1", "SELECT\n nope FROM nowhere"]),
        );
        assert!(
            r[1].error.as_deref().unwrap_or("").contains("(line 2)"),
            "{:?}",
            r[1].error
        );

        let slot = s.cancel_slot();
        let killer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            slot.lock().unwrap().as_ref().map(Canceller::cancel)
        });
        let started = Instant::now();
        let r = s.run("telepad_test", "", &strings(&["SELECT pg_sleep(20)"]));
        assert!(started.elapsed() < Duration::from_secs(10), "cancelled");
        assert!(r[0].error.is_some());
        assert!(matches!(killer.join().unwrap(), Some(Ok(()))));

        // Structure editor round trip (#54), in one transaction.
        let info = s.table_info(&t).unwrap();
        let mut e = crate::db::ddl::TableEdit::from_info(&info);
        e.columns[2].column.name = "memo".into();
        e.columns[2].column.comment = "made up".into();
        e.columns[3].column.length = "12,3".into();
        e.columns
            .push(crate::db::ddl::TableEdit::blank_column(Engine::Postgres, 9));
        e.indexes.retain(|i| i.index.name != "k_price");
        let stmts = crate::db::ddl::alter(Engine::Postgres, &info, &e).unwrap();
        s.ddl("telepad_test", &stmts).unwrap();
        let after = s.table_info(&t).unwrap();
        assert_eq!(
            after.column("memo").map(|c| c.comment.as_str()),
            Some("made up")
        );
        assert_eq!(
            after.column("price").map(|c| c.length.as_str()),
            Some("12,3")
        );
        assert!(!after.indexes.iter().any(|i| i.name == "k_price"));
        assert!(
            crate::db::ddl::alter(
                Engine::Postgres,
                &after,
                &crate::db::ddl::TableEdit::from_info(&after)
            )
            .unwrap()
            .is_empty()
        );
        // A failing statement rolls the whole change back.
        let mut bad = crate::db::ddl::TableEdit::from_info(&after);
        bad.columns.push(crate::db::ddl::TableEdit::blank_column(
            Engine::Postgres,
            10,
        ));
        bad.columns[1].column.type_name = "no_such_type".into();
        bad.columns[1].column.length = String::new();
        let stmts = crate::db::ddl::alter(Engine::Postgres, &after, &bad).unwrap();
        assert!(s.ddl("telepad_test", &stmts).is_err());
        assert!(
            s.table_info(&t).unwrap().column("column10").is_none(),
            "rolled back"
        );
        let nt = TableRef {
            table: "fresh".into(),
            ..t.clone()
        };
        let create = crate::db::ddl::create(
            Engine::Postgres,
            &nt,
            &crate::db::ddl::TableEdit::new_table(Engine::Postgres),
        )
        .unwrap();
        s.ddl("telepad_test", &create).unwrap();
        s.ddl(
            "telepad_test",
            &[
                crate::db::ddl::truncate(Engine::Postgres, &nt),
                crate::db::ddl::drop(Engine::Postgres, &nt, TableKind::Table, "table"),
            ],
        )
        .unwrap();

        // Leave the test database before dropping it.
        if let Backend::Postgres(clients) = &mut s.backend {
            clients.remove("telepad_test");
        }
        ok(&s.run("postgres", "", &strings(&["DROP DATABASE telepad_test"])));
    }
}
