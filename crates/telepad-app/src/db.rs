//! Database browser windows (#52–#54): one [`DbSession`] per open window,
//! its messages ([`DbMsg`]), the work it asks for ([`DbEffect`]) and the pure
//! update logic. Writes happen only after the user confirmed the exact SQL.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use telepad_core::config::HostEntry;
use telepad_core::db::ddl::{self, TableEdit};
use telepad_core::db::{
    Cell, DatabaseInfo, DbTarget, Engine, Grid, Login, Profile, ResultSet, Secret, ServerInfo,
    Statement, TableInfo, TableKind, TableRef, TableSummary, detect, sql,
};
use telepad_core::history::{Entry as LogEntry, Kind as LogKind, Outcome};
use telepad_core::model::HostReport;

pub type DbId = u64;

/// Rows per Data page offered in the toolbar.
pub const PAGE_SIZES: [u64; 4] = [100, 500, 1000, 5000];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DbTab {
    #[default]
    Host,
    Database,
    Table,
    Data,
    Query,
}

impl DbTab {
    pub const ALL: [DbTab; 5] = [
        DbTab::Host,
        DbTab::Database,
        DbTab::Table,
        DbTab::Data,
        DbTab::Query,
    ];
    pub fn title(self) -> &'static str {
        match self {
            DbTab::Host => "Host",
            DbTab::Database => "Database",
            DbTab::Table => "Table",
            DbTab::Data => "Data",
            DbTab::Query => "Query",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DbStatus {
    /// Connect dialog.
    #[default]
    Login,
    Connecting,
    Ready,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaNode {
    /// Empty for MySQL (a MySQL database has no schemas).
    pub name: String,
    pub tables: Option<Vec<TableSummary>>,
    pub loading: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DbNode {
    pub info: DatabaseInfo,
    /// `None` until opened; MySQL gets one unnamed schema.
    pub schemas: Option<Vec<SchemaNode>>,
    pub loading: bool,
}

/// A row of the Data grid: one that is in the database, or a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RowRef {
    Existing(usize),
    New(usize),
}

/// Grid changes not sent yet (HeidiSQL style: they wait for Post).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edits {
    /// (row, column) -> new value (`None` = NULL).
    pub changed: BTreeMap<(usize, usize), Option<String>>,
    /// New rows: per column `None` = leave out (server default), `Some(v)` = value.
    pub inserted: Vec<Vec<Option<Option<String>>>>,
    pub deleted: BTreeSet<usize>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.inserted.is_empty() && self.deleted.is_empty()
    }
    /// Rows touched.
    pub fn count(&self) -> usize {
        let changed: BTreeSet<usize> = self.changed.keys().map(|(r, _)| *r).collect();
        changed.union(&self.deleted).count() + self.inserted.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataView {
    pub table: Option<TableRef>,
    pub grid: Option<Grid>,
    pub offset: u64,
    pub limit: u64,
    pub sort: sql::Sort,
    /// The applied WHERE filter (the text box is a draft).
    pub filter: String,
    pub loading: bool,
    pub error: Option<String>,
    pub edits: Edits,
}

impl Default for DataView {
    fn default() -> Self {
        Self {
            table: None,
            grid: None,
            offset: 0,
            limit: 1000,
            sort: None,
            filter: String::new(),
            loading: false,
            error: None,
            edits: Edits::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryView {
    pub results: Vec<ResultSet>,
    pub running: bool,
    /// Result tab shown.
    pub shown: usize,
}

/// What to reload after a structure change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterDdl {
    /// Structure saved: reload its structure, data and the table list.
    ReloadTable(TableRef),
    /// Created or renamed: reload the list and open this table.
    OpenTable(TableRef),
    /// Dropped: reload the list of this database / schema.
    ReloadTables { database: String, schema: String },
}

/// A confirmation the window shows before anything changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbConfirm {
    /// Query tab statements that need a second look.
    Query {
        tab: usize,
        statements: Vec<String>,
        reasons: Vec<String>,
    },
    /// Grid changes (Post).
    Post { statements: Vec<Statement> },
    /// Structure change; `type_name` must be typed first (TRUNCATE / DROP).
    Ddl {
        title: String,
        database: String,
        statements: Vec<String>,
        type_name: Option<String>,
        after: AfterDdl,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DbSession {
    pub id: DbId,
    pub target: DbTarget,
    pub status: DbStatus,
    /// Last error (connect, load, run, save), shown in the window.
    pub error: Option<String>,
    /// Last success message, shown in the window's status line.
    pub notice: Option<String>,
    /// The OS password store can be used (else why not).
    pub store: Option<Result<(), String>>,
    /// A password is saved for this connection.
    pub saved_password: bool,
    pub version: String,
    pub read_only: bool,
    pub databases: Vec<DbNode>,
    /// Database and schema of the Database tab and of the Query tab.
    pub current: Option<(String, String)>,
    pub info: Option<TableInfo>,
    pub tab: DbTab,
    pub data: DataView,
    pub queries: Vec<QueryView>,
    pub query_tab: usize,
    pub confirm: Option<DbConfirm>,
    /// A structure change is running.
    pub ddl_running: bool,
    /// Reopened from a workspace (#99): database / schema to go back to.
    pub restore: Option<(String, String)>,
    /// Reopened from a workspace: connect once the profile is loaded, when a
    /// password is saved (else the connect form waits for the user).
    pub auto_connect: bool,
    /// SQL log (#100): this connection's history, oldest first (earlier
    /// sessions from `history.db`, then what ran now).
    pub log: Vec<telepad_core::history::Entry>,
    /// Statements sent and not answered yet, logged when they are.
    pub running_log: Option<RunningLog>,
}

/// What a running Post / structure change sent, for the SQL log (#100).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningLog {
    pub title: String,
    pub statements: Vec<String>,
    pub database: String,
}

impl DbSession {
    pub fn engine(&self) -> Engine {
        self.target.engine
    }

    /// Database and schema a workspace reopens it at (#99).
    pub fn restore_point(&self) -> (String, String) {
        self.current
            .clone()
            .or_else(|| self.restore.clone())
            .unwrap_or_default()
    }

    pub fn node(&self, database: &str) -> Option<&DbNode> {
        self.databases.iter().find(|n| n.info.name == database)
    }

    fn node_mut(&mut self, database: &str) -> Option<&mut DbNode> {
        self.databases.iter_mut().find(|n| n.info.name == database)
    }

    fn schema_mut(&mut self, database: &str, schema: &str) -> Option<&mut SchemaNode> {
        self.node_mut(database)?
            .schemas
            .as_mut()?
            .iter_mut()
            .find(|s| s.name == schema)
    }

    /// Tables of the current database / schema, if loaded.
    pub fn current_tables(&self) -> Option<&Vec<TableSummary>> {
        let (db, schema) = self.current.as_ref()?;
        self.node(db)?
            .schemas
            .as_ref()?
            .iter()
            .find(|s| &s.name == schema)?
            .tables
            .as_ref()
    }

    pub fn summary(&self, t: &TableRef) -> Option<&TableSummary> {
        self.node(&t.database)?
            .schemas
            .as_ref()?
            .iter()
            .find(|s| s.name == t.schema)?
            .tables
            .as_ref()?
            .iter()
            .find(|x| x.name == t.table)
    }

    /// Why rows of the open table cannot be edited, or `None` when they can.
    pub fn edit_blocked(&self) -> Option<String> {
        if self.read_only {
            return Some("Read-only is on".into());
        }
        let info = self.info.as_ref()?;
        if info.kind == TableKind::View {
            return Some("Views are shown read-only".into());
        }
        if info.row_key().is_none() {
            return Some(
                "No primary key or NOT NULL unique key: rows cannot be identified, so they are shown read-only (change them with a query)".into(),
            );
        }
        None
    }

    /// Column `col` of the grid can be edited.
    pub fn column_editable(&self, col: usize) -> bool {
        let (Some(info), Some(grid)) = (&self.info, &self.data.grid) else {
            return false;
        };
        grid.columns
            .get(col)
            .and_then(|n| info.column(n))
            .is_some_and(|c| c.grid_editable())
    }

    /// The value shown for a cell, with pending edits applied.
    pub fn cell_value(&self, row: RowRef, col: usize) -> Option<Cell> {
        match row {
            RowRef::Existing(r) => {
                if let Some(v) = self.data.edits.changed.get(&(r, col)) {
                    return Some(v.as_ref().map_or(Cell::Null, |s| Cell::Text(s.clone())));
                }
                self.data.grid.as_ref()?.rows.get(r)?.get(col).cloned()
            }
            RowRef::New(i) => self
                .data
                .edits
                .inserted
                .get(i)?
                .get(col)?
                .as_ref()
                .map(|v| v.as_ref().map_or(Cell::Null, |s| Cell::Text(s.clone()))),
        }
    }
}

/// Connect dialog fields and other text the database window binds to.
#[derive(Clone, Default)]
pub struct DbDraft {
    pub user: String,
    pub password: String,
    pub database: String,
    pub save_password: bool,
    pub tree_filter: String,
    pub data_filter: String,
    /// One text per query tab.
    pub queries: Vec<String>,
    /// Cell being edited: row, column, text, NULL.
    pub cell: Option<(RowRef, usize, String, bool)>,
    pub selected_row: Option<RowRef>,
    /// Text typed into a TRUNCATE / DROP confirmation.
    pub typed: String,
    /// Structure editor of the open table.
    pub structure: Option<TableEdit>,
    /// New table: database, schema, name and its columns.
    pub new_table: Option<NewTable>,
    /// Rename the open table to this name.
    pub rename: Option<String>,
    /// Full value shown in the viewer: title and text.
    pub viewer: Option<(String, String)>,
    /// SQL log panel (#100): filter text, also log SELECT / SHOW …,
    /// Clear waiting for its confirmation.
    pub log_filter: String,
    pub log_select: bool,
    pub log_clear: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTable {
    pub database: String,
    pub schema: String,
    pub name: String,
    pub edit: TableEdit,
}

impl fmt::Debug for DbDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbDraft")
            .field("user", &self.user)
            .field("password", &Secret::new(self.password.clone()))
            .field("database", &self.database)
            .field("save_password", &self.save_password)
            .finish_non_exhaustive()
    }
}

/// Work for a session's worker (one at a time, in order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbJob {
    Databases,
    Schemas {
        database: String,
    },
    Tables {
        database: String,
        schema: String,
    },
    Info(TableRef),
    Page {
        table: TableRef,
        filter: String,
        sort: sql::Sort,
        offset: u64,
        limit: u64,
    },
    Run {
        tab: usize,
        database: String,
        schema: String,
        statements: Vec<String>,
    },
    Apply {
        database: String,
        statements: Vec<Statement>,
    },
    Ddl {
        database: String,
        statements: Vec<String>,
        after: AfterDdl,
    },
    ReadOnly(bool),
}

#[derive(Debug, Clone)]
pub enum DbEffect {
    /// Saved connection, password store, saved password → [`DbMsg::Loaded`];
    /// for a container also its user name → [`DbMsg::EnvUser`].
    LoadProfile {
        id: DbId,
        target: DbTarget,
        host: HostEntry,
    },
    /// Tunnel + login → [`DbMsg::Connected`]. An empty password with
    /// `use_saved_password` reads the saved one. On success the profile is
    /// saved, and the password saved or forgotten as the profile says.
    Connect {
        id: DbId,
        target: DbTarget,
        host: HostEntry,
        login: Login,
        use_saved_password: bool,
        profile: Profile,
    },
    Job {
        id: DbId,
        job: DbJob,
    },
    /// Stop the running statement.
    Cancel(DbId),
    /// Close the connection and its tunnel.
    Close(DbId),
    SaveProfile(Profile),
    /// Delete the saved connection and password → [`DbMsg::Forgotten`].
    Forget {
        id: DbId,
        key: String,
    },
    /// Write one SQL log entry to `history.db` (#100).
    Log(telepad_core::history::Entry),
    /// This connection's history → [`DbMsg::LogLoaded`].
    LoadLog {
        id: DbId,
        key: String,
    },
    ClearLog {
        key: String,
    },
    /// Save the entries as CSV (native save dialog).
    ExportLog(Vec<telepad_core::history::Entry>),
}

#[derive(Debug, Clone)]
pub enum DbMsg {
    Open(DbTarget),
    /// Reopen from a workspace (#99): open, connect with the saved password
    /// if there is one, go back to this database / schema.
    Restore {
        target: DbTarget,
        database: String,
        schema: String,
    },
    Loaded {
        id: DbId,
        profile: Option<Profile>,
        store: Result<(), String>,
        saved_password: bool,
    },
    EnvUser {
        id: DbId,
        user: Option<String>,
    },
    Connect(DbId),
    Connected {
        id: DbId,
        result: Result<ServerInfo, String>,
    },
    Close(DbId),
    /// Drop the (lost) connection and show the connect dialog again.
    Reconnect(DbId),
    Forget(DbId),
    Forgotten {
        id: DbId,
        result: Result<(), String>,
    },
    SetReadOnly(DbId, bool),
    ReadOnlySet {
        id: DbId,
        on: bool,
        result: Result<(), String>,
    },
    Refresh(DbId),
    DatabasesLoaded {
        id: DbId,
        result: Result<Vec<DatabaseInfo>, String>,
    },
    SelectDatabase {
        id: DbId,
        database: String,
    },
    SchemasLoaded {
        id: DbId,
        database: String,
        result: Result<Vec<String>, String>,
    },
    SelectSchema {
        id: DbId,
        database: String,
        schema: String,
    },
    TablesLoaded {
        id: DbId,
        database: String,
        schema: String,
        result: Result<Vec<TableSummary>, String>,
    },
    SelectTable {
        id: DbId,
        table: TableRef,
    },
    InfoLoaded {
        id: DbId,
        table: TableRef,
        result: Result<TableInfo, String>,
    },
    Tab(DbId, DbTab),
    DataReload(DbId),
    DataApplyFilter(DbId),
    DataSort {
        id: DbId,
        column: String,
    },
    DataPage {
        id: DbId,
        forward: bool,
    },
    DataLimit {
        id: DbId,
        limit: u64,
    },
    PageLoaded {
        id: DbId,
        table: TableRef,
        offset: u64,
        result: Result<Grid, String>,
    },
    /// Cell edited (`None` = NULL).
    SetCell {
        id: DbId,
        row: RowRef,
        col: usize,
        value: Option<String>,
    },
    InsertRow(DbId),
    /// Mark an existing row for deletion (again: unmark), or drop a new row.
    DeleteRow {
        id: DbId,
        row: RowRef,
    },
    Discard(DbId),
    /// Show the SQL of the pending grid changes.
    Post(DbId),
    Posted {
        id: DbId,
        result: Result<u64, String>,
    },
    QueryNew(DbId),
    QueryClose {
        id: DbId,
        tab: usize,
    },
    QuerySelect {
        id: DbId,
        tab: usize,
    },
    QueryResultTab {
        id: DbId,
        shown: usize,
    },
    /// Run this text (all, the selection or the current statement).
    QueryRun {
        id: DbId,
        text: String,
    },
    QueryDone {
        id: DbId,
        tab: usize,
        results: Vec<ResultSet>,
    },
    Cancel(DbId),
    ConfirmOk(DbId),
    ConfirmCancel(DbId),
    StructureEdit(DbId),
    StructureDiscard(DbId),
    StructureSave(DbId),
    NewTable {
        id: DbId,
        database: String,
        schema: String,
    },
    NewTableCancel(DbId),
    NewTableSave(DbId),
    RenameTable(DbId),
    RenameCancel(DbId),
    RenameSave(DbId),
    TruncateTable {
        id: DbId,
        table: TableRef,
    },
    DropTable {
        id: DbId,
        table: TableRef,
    },
    DdlDone {
        id: DbId,
        result: Result<(), String>,
        after: AfterDdl,
    },
    LogLoaded {
        id: DbId,
        result: Result<Vec<telepad_core::history::Entry>, String>,
    },
    /// Clear the SQL log of this connection (asked first in the window).
    LogClear(DbId),
    /// Export the shown (filtered) SQL log.
    LogExport(DbId),
}

/// Databases on a host: in its running containers, then installed on it.
pub fn host_db_targets(alias: &str, report: &HostReport) -> Vec<DbTarget> {
    let mut v: Vec<DbTarget> = report
        .docker
        .containers
        .iter()
        .filter_map(|c| detect::container_target(alias, c))
        .collect();
    v.extend(detect::host_targets(alias, report));
    v
}

fn job(fx: &mut Vec<DbEffect>, id: DbId, job: DbJob) {
    fx.push(DbEffect::Job { id, job });
}

fn page_job(s: &DbSession) -> Option<DbJob> {
    Some(DbJob::Page {
        table: s.data.table.clone()?,
        filter: s.data.filter.clone(),
        sort: s.data.sort.clone(),
        offset: s.data.offset,
        limit: s.data.limit,
    })
}

fn reload_page(s: &mut DbSession, fx: &mut Vec<DbEffect>) {
    if let Some(j) = page_job(s) {
        s.data.loading = true;
        s.data.error = None;
        job(fx, s.id, j);
    }
}

/// Grid changes as statements: updates, inserts, then deletes.
pub fn post_statements(s: &DbSession) -> Result<Vec<Statement>, String> {
    if let Some(why) = s.edit_blocked() {
        return Err(why);
    }
    let info = s
        .info
        .as_ref()
        .ok_or("The table structure is still loading")?;
    let key = info.row_key().ok_or("No key to identify rows")?;
    let grid = s.data.grid.as_ref().ok_or("No rows loaded")?;
    let engine = s.engine();
    let t = &info.table;
    let name = |i: usize| grid.columns.get(i).cloned().unwrap_or_default();
    let key_of = |r: usize| -> Result<Vec<(String, Option<String>)>, String> {
        key.iter()
            .map(|k| {
                let i = grid
                    .columns
                    .iter()
                    .position(|c| c == k)
                    .ok_or_else(|| format!("Key column {k} is not in the grid"))?;
                let cell = grid
                    .rows
                    .get(r)
                    .and_then(|row| row.get(i))
                    .ok_or("Row is gone; reload")?;
                if !cell.editable() {
                    return Err(format!(
                        "Row {}: the key value is too long or binary to identify the row",
                        r + 1
                    ));
                }
                Ok((k.clone(), cell.as_value().map(str::to_string)))
            })
            .collect()
    };
    let e = &s.data.edits;
    let mut out = Vec::new();
    let rows: BTreeSet<usize> = e.changed.keys().map(|(r, _)| *r).collect();
    for r in rows.iter().filter(|r| !e.deleted.contains(r)) {
        let set: Vec<(String, Option<String>)> = e
            .changed
            .range((*r, 0)..=(*r, usize::MAX))
            .map(|((_, c), v)| (name(*c), v.clone()))
            .collect();
        out.push(sql::update_row(
            engine,
            t,
            &info.columns,
            &key_of(*r)?,
            &set,
        ));
    }
    for row in &e.inserted {
        let values: Vec<(String, Option<String>)> = row
            .iter()
            .enumerate()
            .filter_map(|(c, v)| v.clone().map(|v| (name(c), v)))
            .collect();
        out.push(sql::insert_row(engine, t, &info.columns, &values));
    }
    for r in &e.deleted {
        out.push(sql::delete_row(engine, t, &info.columns, &key_of(*r)?));
    }
    Ok(out)
}

fn ask_ddl(
    s: &mut DbSession,
    title: &str,
    database: &str,
    statements: Result<Vec<String>, String>,
    type_name: Option<String>,
    after: AfterDdl,
) {
    if s.read_only {
        s.error = Some("Read-only is on: switch it off to change the structure".into());
        return;
    }
    match statements {
        Ok(v) if v.is_empty() => s.notice = Some("Nothing changed".into()),
        Ok(statements) => {
            s.error = None;
            s.confirm = Some(DbConfirm::Ddl {
                title: title.into(),
                database: database.into(),
                statements,
                type_name,
                after,
            });
        }
        Err(e) => s.error = Some(e),
    }
}

/// Apply one database-window message. `hosts` resolves the host entry of a
/// target; `drafts` are the windows' form fields.
pub fn update(
    sessions: &mut Vec<DbSession>,
    drafts: &mut BTreeMap<DbId, DbDraft>,
    next_id: &mut DbId,
    host_entry: &dyn Fn(&str) -> Option<HostEntry>,
    msg: DbMsg,
) -> Vec<DbEffect> {
    let mut fx = Vec::new();
    if let DbMsg::Restore {
        target,
        database,
        schema,
    } = msg
    {
        let key = target.key();
        if sessions.iter().any(|s| s.target.key() == key) {
            return fx;
        }
        fx = update(sessions, drafts, next_id, host_entry, DbMsg::Open(target));
        if let Some(s) = sessions.iter_mut().find(|s| s.target.key() == key) {
            s.restore = Some((database, schema));
            s.auto_connect = true;
        }
        return fx;
    }
    if let DbMsg::Open(target) = msg {
        if sessions.iter().any(|s| s.target.key() == target.key()) {
            return fx;
        }
        let Some(host) = host_entry(&target.host) else {
            return fx;
        };
        *next_id += 1;
        let id = *next_id;
        let engine = target.engine;
        sessions.push(DbSession {
            id,
            target: target.clone(),
            queries: vec![QueryView::default()],
            ..Default::default()
        });
        drafts.insert(
            id,
            DbDraft {
                user: engine.default_user().into(),
                queries: vec![String::new()],
                ..Default::default()
            },
        );
        fx.push(DbEffect::LoadLog {
            id,
            key: target.key(),
        });
        fx.push(DbEffect::LoadProfile { id, target, host });
        return fx;
    }
    let id = msg_id(&msg);
    let Some(pos) = sessions.iter().position(|s| s.id == id) else {
        return fx;
    };
    if let DbMsg::Close(_) = msg {
        sessions.remove(pos);
        drafts.remove(&id);
        fx.push(DbEffect::Close(id));
        return fx;
    }
    let mut reopen: Option<(TableRef, DbTab)> = None;
    let mut connect_now = false;
    let s = &mut sessions[pos];
    let d = drafts.entry(id).or_default();
    let engine = s.engine();
    match msg {
        DbMsg::Open(_) | DbMsg::Restore { .. } | DbMsg::Close(_) => {}
        DbMsg::LogLoaded { result, .. } => match result {
            Ok(mut v) => {
                // Entries of this session written meanwhile stay after them.
                v.extend(s.log.drain(..).filter(|e| e.at.is_none()));
                s.log = v;
            }
            Err(e) => s.notice = Some(format!("SQL log not available: {e}")),
        },
        DbMsg::LogClear(_) => {
            d.log_clear = false;
            s.log.clear();
            fx.push(DbEffect::ClearLog {
                key: s.target.key(),
            });
        }
        DbMsg::LogExport(_) => {
            let shown: Vec<_> = s
                .log
                .iter()
                .filter(|e| log_matches(e, &d.log_filter))
                .cloned()
                .collect();
            fx.push(DbEffect::ExportLog(shown));
        }
        DbMsg::Loaded {
            profile,
            store,
            saved_password,
            ..
        } => {
            s.saved_password = saved_password && store.is_ok();
            if let Some(p) = profile {
                d.user = p.user;
                d.database = p.database;
                d.save_password = p.save_password && store.is_ok();
                s.read_only = p.read_only;
            }
            if let Some((db, _)) = s.restore.as_ref().filter(|(db, _)| !db.is_empty()) {
                d.database = db.clone();
            }
            s.store = Some(store);
            if std::mem::take(&mut s.auto_connect) && s.saved_password && d.save_password {
                connect_now = true;
            }
        }
        DbMsg::EnvUser { user, .. } => {
            if let Some(u) = user
                && d.user == engine.default_user()
                && !s.saved_password
            {
                d.user = u;
            }
        }
        DbMsg::Connect(_) => {
            if s.status != DbStatus::Login {
                return fx;
            }
            let Some(host) = host_entry(&s.target.host) else {
                s.error = Some(format!(
                    "Host {} is not in the inventory any more",
                    s.target.host
                ));
                return fx;
            };
            let password = std::mem::take(&mut d.password);
            let save = d.save_password && s.store.as_ref().is_some_and(Result::is_ok);
            s.status = DbStatus::Connecting;
            s.error = None;
            fx.push(DbEffect::Connect {
                id,
                target: s.target.clone(),
                host,
                use_saved_password: password.is_empty() && s.saved_password && save,
                login: Login {
                    user: d.user.trim().to_string(),
                    password: Secret::new(password),
                    database: d.database.trim().to_string(),
                },
                profile: Profile {
                    key: s.target.key(),
                    user: d.user.trim().to_string(),
                    database: d.database.trim().to_string(),
                    save_password: save,
                    read_only: s.read_only,
                },
            });
        }
        DbMsg::Connected { result, .. } => match result {
            Ok(info) => {
                s.status = DbStatus::Ready;
                s.version = info.version;
                s.saved_password = d.save_password;
                fx.push(DbEffect::SaveProfile(Profile {
                    key: s.target.key(),
                    user: d.user.trim().to_string(),
                    database: d.database.trim().to_string(),
                    save_password: d.save_password,
                    read_only: s.read_only,
                }));
                s.databases = info
                    .databases
                    .into_iter()
                    .map(|info| DbNode {
                        info,
                        ..Default::default()
                    })
                    .collect();
                s.tab = DbTab::Host;
                let db = d.database.trim().to_string();
                if s.node(&db).is_some() {
                    select_database(s, &db, &mut fx);
                }
            }
            Err(e) => {
                s.status = DbStatus::Login;
                s.error = Some(e);
            }
        },
        DbMsg::Reconnect(_) => {
            fx.push(DbEffect::Close(id));
            let keep = std::mem::take(s);
            *s = DbSession {
                id,
                target: keep.target,
                store: keep.store,
                saved_password: keep.saved_password,
                read_only: keep.read_only,
                queries: keep.queries,
                query_tab: keep.query_tab,
                ..Default::default()
            };
            for q in &mut s.queries {
                q.running = false;
            }
        }
        DbMsg::Forget(_) => fx.push(DbEffect::Forget {
            id,
            key: s.target.key(),
        }),
        DbMsg::Forgotten { result, .. } => match result {
            Ok(()) => {
                s.saved_password = false;
                d.save_password = false;
                d.user = engine.default_user().into();
                d.database.clear();
                s.notice = Some("Saved connection and password removed".into());
            }
            Err(e) => s.error = Some(e),
        },
        DbMsg::SetReadOnly(_, on) => {
            if s.status == DbStatus::Ready {
                job(&mut fx, id, DbJob::ReadOnly(on));
            } else {
                s.read_only = on;
            }
        }
        DbMsg::ReadOnlySet { on, result, .. } => match result {
            Ok(()) => {
                s.read_only = on;
                s.notice = Some(if on {
                    "Read-only: the server refuses changes from this window".into()
                } else {
                    "Editable".into()
                });
                fx.push(DbEffect::SaveProfile(Profile {
                    key: s.target.key(),
                    user: d.user.trim().to_string(),
                    database: d.database.trim().to_string(),
                    save_password: s.saved_password,
                    read_only: on,
                }));
            }
            Err(e) => s.error = Some(e),
        },
        DbMsg::Refresh(_) => {
            job(&mut fx, id, DbJob::Databases);
            if let Some((db, schema)) = s.current.clone() {
                if engine.has_schemas() {
                    job(
                        &mut fx,
                        id,
                        DbJob::Schemas {
                            database: db.clone(),
                        },
                    );
                }
                job(
                    &mut fx,
                    id,
                    DbJob::Tables {
                        database: db,
                        schema,
                    },
                );
            }
        }
        DbMsg::DatabasesLoaded { result, .. } => match result {
            Ok(list) => {
                let old = std::mem::take(&mut s.databases);
                s.databases = list
                    .into_iter()
                    .map(|info| {
                        let prev = old.iter().find(|n| n.info.name == info.name);
                        DbNode {
                            schemas: prev.and_then(|p| p.schemas.clone()),
                            loading: false,
                            info,
                        }
                    })
                    .collect();
            }
            Err(e) => s.error = Some(e),
        },
        DbMsg::SelectDatabase { database, .. } => select_database(s, &database, &mut fx),
        DbMsg::SchemasLoaded {
            database, result, ..
        } => {
            let first = match (&result, s.node_mut(&database)) {
                (Ok(list), Some(node)) => {
                    let old = node.schemas.take().unwrap_or_default();
                    node.schemas = Some(
                        list.iter()
                            .map(|name| {
                                old.iter().find(|o| &o.name == name).cloned().unwrap_or(
                                    SchemaNode {
                                        name: name.clone(),
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect(),
                    );
                    node.loading = false;
                    list.first().cloned()
                }
                _ => None,
            };
            if let Err(e) = result {
                s.error = Some(e);
            }
            let wanted = s
                .restore
                .take()
                .map(|(_, schema)| schema)
                .filter(|w| !w.is_empty() && s.schema_mut(&database, w).is_some());
            if let (Some(first), Some((db, schema))) = (wanted.or(first), s.current.clone())
                && db == database
                && schema.is_empty()
            {
                select_schema(s, &database, &first, &mut fx);
            }
        }
        DbMsg::SelectSchema {
            database, schema, ..
        } => select_schema(s, &database, &schema, &mut fx),
        DbMsg::TablesLoaded {
            database,
            schema,
            result,
            ..
        } => match result {
            Ok(tables) => {
                if let Some(node) = s.node_mut(&database) {
                    node.loading = false;
                    let schemas = node.schemas.get_or_insert_with(Vec::new);
                    match schemas.iter_mut().find(|x| x.name == schema) {
                        Some(x) => {
                            x.tables = Some(tables);
                            x.loading = false;
                        }
                        None => schemas.push(SchemaNode {
                            name: schema,
                            tables: Some(tables),
                            loading: false,
                        }),
                    }
                }
            }
            Err(e) => {
                if let Some(x) = s.schema_mut(&database, &schema) {
                    x.loading = false;
                }
                if let Some(node) = s.node_mut(&database) {
                    node.loading = false;
                }
                s.error = Some(e);
            }
        },
        DbMsg::SelectTable { table, .. } => {
            if s.data.table.as_ref() == Some(&table) {
                if matches!(s.tab, DbTab::Host | DbTab::Database) {
                    s.tab = DbTab::Data;
                }
                return fx;
            }
            if !s.data.edits.is_empty() {
                s.error = Some("Post or discard the changes in the Data tab first".into());
                return fx;
            }
            s.current = Some((table.database.clone(), table.schema.clone()));
            s.data = DataView {
                table: Some(table.clone()),
                limit: s.data.limit,
                ..Default::default()
            };
            s.info = None;
            d.data_filter.clear();
            d.cell = None;
            d.selected_row = None;
            d.structure = None;
            d.rename = None;
            if matches!(s.tab, DbTab::Host | DbTab::Database | DbTab::Query) {
                s.tab = DbTab::Data;
            }
            job(&mut fx, id, DbJob::Info(table));
            reload_page(s, &mut fx);
        }
        DbMsg::InfoLoaded { table, result, .. } => {
            if s.data.table.as_ref() == Some(&table) {
                match result {
                    Ok(info) => s.info = Some(info),
                    Err(e) => s.error = Some(e),
                }
            }
        }
        DbMsg::Tab(_, tab) => s.tab = tab,
        DbMsg::DataReload(_) => {
            if s.data.edits.is_empty() {
                reload_page(s, &mut fx);
            } else {
                s.error = Some("Post or discard the changes first".into());
            }
        }
        DbMsg::DataApplyFilter(_) => {
            if !s.data.edits.is_empty() {
                s.error = Some("Post or discard the changes first".into());
                return fx;
            }
            s.data.filter = d.data_filter.trim().to_string();
            s.data.offset = 0;
            reload_page(s, &mut fx);
        }
        DbMsg::DataSort { column, .. } => {
            if !s.data.edits.is_empty() {
                s.error = Some("Post or discard the changes first".into());
                return fx;
            }
            s.data.sort = match s.data.sort.take() {
                Some((c, false)) if c == column => Some((c, true)),
                Some((c, true)) if c == column => None,
                _ => Some((column, false)),
            };
            s.data.offset = 0;
            reload_page(s, &mut fx);
        }
        DbMsg::DataPage { forward, .. } => {
            if !s.data.edits.is_empty() {
                s.error = Some("Post or discard the changes first".into());
                return fx;
            }
            let full = s
                .data
                .grid
                .as_ref()
                .is_some_and(|g| g.rows.len() as u64 >= s.data.limit);
            if forward && full {
                s.data.offset += s.data.limit;
            } else if !forward && s.data.offset > 0 {
                s.data.offset = s.data.offset.saturating_sub(s.data.limit);
            } else {
                return fx;
            }
            reload_page(s, &mut fx);
        }
        DbMsg::DataLimit { limit, .. } => {
            if s.data.edits.is_empty() && limit > 0 {
                s.data.limit = limit;
                s.data.offset = 0;
                reload_page(s, &mut fx);
            }
        }
        DbMsg::PageLoaded {
            table,
            offset,
            result,
            ..
        } => {
            if s.data.table.as_ref() == Some(&table) && s.data.offset == offset {
                s.data.loading = false;
                match result {
                    Ok(g) => {
                        s.data.grid = Some(g);
                        s.data.error = None;
                    }
                    Err(e) => s.data.error = Some(e),
                }
            }
        }
        DbMsg::SetCell {
            row, col, value, ..
        } => {
            if s.edit_blocked().is_some() || !s.column_editable(col) {
                return fx;
            }
            match row {
                RowRef::Existing(r) => {
                    let original = s
                        .data
                        .grid
                        .as_ref()
                        .and_then(|g| g.rows.get(r))
                        .and_then(|row| row.get(col))
                        .cloned();
                    match original {
                        Some(o) if o.editable() && o.as_value() == value.as_deref() => {
                            s.data.edits.changed.remove(&(r, col));
                        }
                        Some(_) => {
                            s.data.edits.changed.insert((r, col), value);
                        }
                        None => {}
                    }
                }
                RowRef::New(i) => {
                    if let Some(cell) = s
                        .data
                        .edits
                        .inserted
                        .get_mut(i)
                        .and_then(|r| r.get_mut(col))
                    {
                        *cell = Some(value);
                    }
                }
            }
        }
        DbMsg::InsertRow(_) => {
            if s.edit_blocked().is_none()
                && let Some(g) = &s.data.grid
            {
                s.data.edits.inserted.push(vec![None; g.columns.len()]);
                d.selected_row = Some(RowRef::New(s.data.edits.inserted.len() - 1));
            }
        }
        DbMsg::DeleteRow { row, .. } => {
            if s.edit_blocked().is_some() {
                return fx;
            }
            match row {
                RowRef::Existing(r) => {
                    if !s.data.edits.deleted.remove(&r) {
                        s.data.edits.deleted.insert(r);
                    }
                }
                RowRef::New(i) => {
                    if i < s.data.edits.inserted.len() {
                        s.data.edits.inserted.remove(i);
                        d.selected_row = None;
                    }
                }
            }
        }
        DbMsg::Discard(_) => {
            s.data.edits = Edits::default();
            d.cell = None;
            s.error = None;
        }
        DbMsg::Post(_) => match post_statements(s) {
            Ok(v) if v.is_empty() => s.notice = Some("Nothing to post".into()),
            Ok(statements) => {
                s.error = None;
                s.confirm = Some(DbConfirm::Post { statements });
            }
            Err(e) => s.error = Some(e),
        },
        DbMsg::Posted { result, .. } => {
            s.data.loading = false;
            if let Some(run) = s.running_log.take() {
                let (outcome, rows, error) = match &result {
                    Ok(n) => (Outcome::Done, Some(*n), String::new()),
                    Err(e) => (Outcome::RolledBack, None, e.clone()),
                };
                log_sql(s, d, &mut fx, &run, outcome, rows, error, None);
            }
            match result {
                Ok(n) => {
                    s.data.edits = Edits::default();
                    d.cell = None;
                    s.notice = Some(format!("Saved: {n} row(s) changed"));
                    reload_page(s, &mut fx);
                }
                Err(e) => s.error = Some(e),
            }
        }
        DbMsg::QueryNew(_) => {
            s.queries.push(QueryView::default());
            d.queries.push(String::new());
            s.query_tab = s.queries.len() - 1;
        }
        DbMsg::QueryClose { tab, .. } => {
            if s.queries.len() > 1 && tab < s.queries.len() && !s.queries[tab].running {
                s.queries.remove(tab);
                if tab < d.queries.len() {
                    d.queries.remove(tab);
                }
                s.query_tab = s.query_tab.min(s.queries.len() - 1);
            }
        }
        DbMsg::QuerySelect { tab, .. } => {
            if tab < s.queries.len() {
                s.query_tab = tab;
            }
        }
        DbMsg::QueryResultTab { shown, .. } => {
            if let Some(q) = s.queries.get_mut(s.query_tab) {
                q.shown = shown;
            }
        }
        DbMsg::QueryRun { text, .. } => {
            let tab = s.query_tab;
            if s.queries.get(tab).is_none_or(|q| q.running) {
                return fx;
            }
            let statements: Vec<String> = sql::split_statements(engine, &text)
                .into_iter()
                .map(|r| text[r].to_string())
                .collect();
            if statements.is_empty() {
                s.notice = Some("Nothing to run".into());
                return fx;
            }
            if s.read_only
                && let Some(w) = statements.iter().find(|st| !sql::is_read_only(engine, st))
            {
                s.queries[tab].results = vec![ResultSet {
                    statement: w.clone(),
                    error: Some(
                        "Read-only is on: only SELECT, SHOW, DESCRIBE, EXPLAIN and WITH run. Switch Read-only off to change data."
                            .into(),
                    ),
                    ..Default::default()
                }];
                return fx;
            }
            let reasons: Vec<String> = statements
                .iter()
                .filter_map(|st| sql::danger(engine, st))
                .collect();
            if reasons.is_empty() {
                run_query(s, tab, statements, &mut fx);
            } else {
                s.confirm = Some(DbConfirm::Query {
                    tab,
                    statements,
                    reasons,
                });
            }
        }
        DbMsg::QueryDone { tab, results, .. } => {
            let (db, schema) = s.current.clone().unwrap_or_default();
            for r in &results {
                if !d.log_select && sql::is_read_only(engine, &r.statement) {
                    continue;
                }
                let run = RunningLog {
                    title: sql_title(&r.statement),
                    statements: vec![r.statement.clone()],
                    database: if schema.is_empty() {
                        db.clone()
                    } else {
                        format!("{db}.{schema}")
                    },
                };
                let outcome = if r.error.is_some() {
                    Outcome::Failed
                } else {
                    Outcome::Done
                };
                log_sql(
                    s,
                    d,
                    &mut fx,
                    &run,
                    outcome,
                    r.affected,
                    r.error.clone().unwrap_or_default(),
                    Some(r.millis),
                );
            }
            if let Some(q) = s.queries.get_mut(tab) {
                q.running = false;
                q.shown = 0;
                q.results = results;
            }
        }
        DbMsg::Cancel(_) => fx.push(DbEffect::Cancel(id)),
        DbMsg::ConfirmCancel(_) => {
            s.confirm = None;
            d.typed.clear();
        }
        DbMsg::ConfirmOk(_) => match s.confirm.take() {
            Some(DbConfirm::Query {
                tab, statements, ..
            }) => run_query(s, tab, statements, &mut fx),
            Some(DbConfirm::Post { statements }) => {
                if let Some(t) = &s.data.table {
                    s.running_log = Some(RunningLog {
                        title: format!(
                            "Grid changes in {}: {} statement(s)",
                            t.table,
                            statements.len()
                        ),
                        statements: statements.iter().map(|st| st.display(engine)).collect(),
                        database: t.database.clone(),
                    });
                    s.data.loading = true;
                    job(
                        &mut fx,
                        id,
                        DbJob::Apply {
                            database: t.database.clone(),
                            statements,
                        },
                    );
                }
            }
            Some(DbConfirm::Ddl {
                title,
                database,
                statements,
                type_name,
                after,
            }) => {
                if let Some(name) = &type_name
                    && d.typed.trim() != name
                {
                    s.confirm = Some(DbConfirm::Ddl {
                        title,
                        database,
                        statements,
                        type_name,
                        after,
                    });
                    return fx;
                }
                d.typed.clear();
                s.ddl_running = true;
                s.running_log = Some(RunningLog {
                    title: title.clone(),
                    statements: statements.clone(),
                    database: database.clone(),
                });
                job(
                    &mut fx,
                    id,
                    DbJob::Ddl {
                        database,
                        statements,
                        after,
                    },
                );
            }
            None => {}
        },
        DbMsg::StructureEdit(_) => {
            if let Some(info) = &s.info {
                d.structure = Some(TableEdit::from_info(info));
                d.new_table = None;
                s.tab = DbTab::Table;
            }
        }
        DbMsg::StructureDiscard(_) => {
            d.structure = None;
            s.error = None;
        }
        DbMsg::StructureSave(_) => {
            if let (Some(info), Some(edit)) = (s.info.clone(), d.structure.clone()) {
                let t = info.table.clone();
                ask_ddl(
                    s,
                    "Save table structure",
                    &t.database.clone(),
                    ddl::alter(engine, &info, &edit),
                    None,
                    AfterDdl::ReloadTable(t),
                );
            }
        }
        DbMsg::NewTable {
            database, schema, ..
        } => {
            if !s.data.edits.is_empty() {
                s.error = Some("Post or discard the changes in the Data tab first".into());
                return fx;
            }
            d.structure = None;
            d.new_table = Some(NewTable {
                database,
                schema,
                name: String::new(),
                edit: TableEdit::new_table(engine),
            });
            s.tab = DbTab::Table;
        }
        DbMsg::NewTableCancel(_) => d.new_table = None,
        DbMsg::NewTableSave(_) => {
            if let Some(nt) = d.new_table.clone() {
                let t = TableRef {
                    database: nt.database.clone(),
                    schema: nt.schema.clone(),
                    table: nt.name.trim().to_string(),
                };
                ask_ddl(
                    s,
                    "Create table",
                    &nt.database,
                    ddl::create(engine, &t, &nt.edit),
                    None,
                    AfterDdl::OpenTable(t),
                );
            }
        }
        DbMsg::RenameTable(_) => {
            if let Some(t) = &s.data.table {
                d.rename = Some(t.table.clone());
            }
        }
        DbMsg::RenameCancel(_) => d.rename = None,
        DbMsg::RenameSave(_) => {
            if let (Some(t), Some(name)) = (s.data.table.clone(), d.rename.clone()) {
                let to = TableRef {
                    table: name.trim().to_string(),
                    ..t.clone()
                };
                ask_ddl(
                    s,
                    "Rename table",
                    &t.database,
                    ddl::rename(engine, &t, &name).map(|q| vec![q]),
                    None,
                    AfterDdl::OpenTable(to),
                );
            }
        }
        DbMsg::TruncateTable { table, .. } => {
            ask_ddl(
                s,
                "Empty table (TRUNCATE)",
                &table.database.clone(),
                Ok(vec![ddl::truncate(engine, &table)]),
                Some(table.table.clone()),
                AfterDdl::ReloadTable(table),
            );
            d.typed.clear();
        }
        DbMsg::DropTable { table, .. } => {
            let (kind, what) = s
                .summary(&table)
                .map(|x| (x.kind, x.engine.clone()))
                .unwrap_or_default();
            ask_ddl(
                s,
                if kind == TableKind::View {
                    "Drop view"
                } else {
                    "Drop table"
                },
                &table.database.clone(),
                Ok(vec![ddl::drop(engine, &table, kind, &what)]),
                Some(table.table.clone()),
                AfterDdl::ReloadTables {
                    database: table.database.clone(),
                    schema: table.schema.clone(),
                },
            );
            d.typed.clear();
        }
        DbMsg::DdlDone { result, after, .. } => {
            s.ddl_running = false;
            if let Some(run) = s.running_log.take() {
                let (outcome, error) = match &result {
                    Ok(()) => (Outcome::Done, String::new()),
                    Err(e) => (Outcome::Failed, e.clone()),
                };
                log_sql(s, d, &mut fx, &run, outcome, None, error, None);
            }
            match result {
                Ok(()) => {
                    s.error = None;
                    s.notice = Some("Done".into());
                    d.structure = None;
                    d.new_table = None;
                    d.rename = None;
                    match after {
                        AfterDdl::ReloadTable(t) => {
                            job(
                                &mut fx,
                                id,
                                DbJob::Tables {
                                    database: t.database.clone(),
                                    schema: t.schema.clone(),
                                },
                            );
                            if s.data.table.as_ref() == Some(&t) {
                                s.info = None;
                                job(&mut fx, id, DbJob::Info(t));
                                s.data.edits = Edits::default();
                                reload_page(s, &mut fx);
                            }
                        }
                        AfterDdl::OpenTable(t) => {
                            job(
                                &mut fx,
                                id,
                                DbJob::Tables {
                                    database: t.database.clone(),
                                    schema: t.schema.clone(),
                                },
                            );
                            s.data.table = None;
                            s.data.edits = Edits::default();
                            reopen = Some((t, s.tab));
                        }
                        AfterDdl::ReloadTables { database, schema } => {
                            let open_gone = s
                                .data
                                .table
                                .as_ref()
                                .is_some_and(|t| t.database == database && t.schema == schema);
                            if open_gone {
                                s.data = DataView {
                                    limit: s.data.limit,
                                    ..Default::default()
                                };
                                s.info = None;
                                s.tab = DbTab::Database;
                            }
                            job(&mut fx, id, DbJob::Tables { database, schema });
                        }
                    }
                }
                Err(e) => s.error = Some(e),
            }
        }
    }
    if connect_now {
        fx.extend(update(
            sessions,
            drafts,
            next_id,
            host_entry,
            DbMsg::Connect(id),
        ));
    }
    if let Some((table, tab)) = reopen {
        let mut more = update(
            sessions,
            drafts,
            next_id,
            host_entry,
            DbMsg::SelectTable { id, table },
        );
        fx.append(&mut more);
        if let Some(s) = sessions.iter_mut().find(|s| s.id == id) {
            s.tab = if tab == DbTab::Table {
                DbTab::Table
            } else {
                DbTab::Data
            };
        }
    }
    fx
}

fn select_database(s: &mut DbSession, database: &str, fx: &mut Vec<DbEffect>) {
    let id = s.id;
    let engine = s.engine();
    s.current = Some((database.to_string(), String::new()));
    if s.tab == DbTab::Host {
        s.tab = DbTab::Database;
    }
    let Some(node) = s.node_mut(database) else {
        return;
    };
    if node.loading {
        return;
    }
    node.loading = true;
    if engine.has_schemas() {
        job(
            fx,
            id,
            DbJob::Schemas {
                database: database.to_string(),
            },
        );
    } else {
        job(
            fx,
            id,
            DbJob::Tables {
                database: database.to_string(),
                schema: String::new(),
            },
        );
    }
}

fn select_schema(s: &mut DbSession, database: &str, schema: &str, fx: &mut Vec<DbEffect>) {
    let id = s.id;
    s.current = Some((database.to_string(), schema.to_string()));
    if s.tab == DbTab::Host {
        s.tab = DbTab::Database;
    }
    if let Some(x) = s.schema_mut(database, schema) {
        x.loading = true;
    }
    job(
        fx,
        id,
        DbJob::Tables {
            database: database.to_string(),
            schema: schema.to_string(),
        },
    );
}

fn run_query(s: &mut DbSession, tab: usize, statements: Vec<String>, fx: &mut Vec<DbEffect>) {
    let Some(q) = s.queries.get_mut(tab) else {
        return;
    };
    q.running = true;
    q.results.clear();
    let (database, schema) = s.current.clone().unwrap_or_default();
    job(
        fx,
        s.id,
        DbJob::Run {
            tab,
            database,
            schema,
            statements,
        },
    );
}

fn msg_id(msg: &DbMsg) -> DbId {
    use DbMsg::*;
    match msg {
        Open(_) | Restore { .. } => 0,
        Loaded { id, .. }
        | EnvUser { id, .. }
        | Connected { id, .. }
        | Forgotten { id, .. }
        | ReadOnlySet { id, .. }
        | DatabasesLoaded { id, .. }
        | SelectDatabase { id, .. }
        | SchemasLoaded { id, .. }
        | SelectSchema { id, .. }
        | TablesLoaded { id, .. }
        | SelectTable { id, .. }
        | InfoLoaded { id, .. }
        | DataSort { id, .. }
        | DataPage { id, .. }
        | DataLimit { id, .. }
        | PageLoaded { id, .. }
        | SetCell { id, .. }
        | DeleteRow { id, .. }
        | Posted { id, .. }
        | QueryClose { id, .. }
        | QuerySelect { id, .. }
        | QueryResultTab { id, .. }
        | QueryRun { id, .. }
        | QueryDone { id, .. }
        | NewTable { id, .. }
        | TruncateTable { id, .. }
        | DropTable { id, .. }
        | LogLoaded { id, .. }
        | DdlDone { id, .. } => *id,
        Connect(id)
        | Close(id)
        | Reconnect(id)
        | Forget(id)
        | SetReadOnly(id, _)
        | Refresh(id)
        | Tab(id, _)
        | DataReload(id)
        | DataApplyFilter(id)
        | InsertRow(id)
        | Discard(id)
        | Post(id)
        | QueryNew(id)
        | Cancel(id)
        | ConfirmOk(id)
        | ConfirmCancel(id)
        | StructureEdit(id)
        | StructureDiscard(id)
        | StructureSave(id)
        | NewTableCancel(id)
        | NewTableSave(id)
        | RenameTable(id)
        | RenameCancel(id)
        | LogClear(id)
        | LogExport(id)
        | RenameSave(id) => *id,
    }
}

/// What closing this window would throw away (#99), one line each.
pub fn unsaved(s: &DbSession, d: &DbDraft) -> Vec<String> {
    let mut v = Vec::new();
    if !s.data.edits.is_empty() {
        v.push(format!(
            "{} changed row(s) in the Data grid, not posted",
            s.data.edits.count()
        ));
    }
    if d.structure.is_some() {
        v.push("Structure editor changes, not run".into());
    }
    if d.new_table.is_some() {
        v.push("New table, not created".into());
    }
    if d.rename.is_some() {
        v.push("Table rename, not run".into());
    }
    if s.confirm.is_some() {
        v.push("SQL waiting for Run".into());
    }
    let typed = d.queries.iter().filter(|q| !q.trim().is_empty()).count();
    if typed > 0 {
        v.push(format!("{typed} query tab(s) with text"));
    }
    if s.status == DbStatus::Login && !d.password.is_empty() {
        v.push("Connect form with a typed password".into());
    }
    v
}

/// First words of a statement for the log's title line: `UPDATE `items``.
pub fn sql_title(statement: &str) -> String {
    let one: String = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 80 {
        format!("{}…", one.chars().take(79).collect::<String>())
    } else {
        one
    }
}

/// An SQL log line matches the filter text (every word, any case).
pub fn log_matches(e: &LogEntry, filter: &str) -> bool {
    crate::state::matches_query(
        &format!("{} {} {} {}", e.title, e.detail, e.database, e.error),
        filter,
    )
}

#[allow(clippy::too_many_arguments)]
fn log_sql(
    s: &mut DbSession,
    d: &DbDraft,
    fx: &mut Vec<DbEffect>,
    run: &RunningLog,
    outcome: Outcome,
    rows: Option<u64>,
    error: String,
    millis: Option<u64>,
) {
    let e = LogEntry {
        at: None,
        kind: LogKind::Sql,
        host: s.target.host.clone(),
        target: s.target.key(),
        title: telepad_core::history::mask_secrets(&run.title),
        detail: telepad_core::history::mask_secrets(&run.statements.join(";\n")),
        outcome,
        error,
        rows,
        duration_ms: millis,
        method: s.engine().as_str().to_string(),
        user: d.user.trim().to_string(),
        database: run.database.clone(),
        ..Default::default()
    };
    s.log.push(e.clone());
    fx.push(DbEffect::Log(e));
}
