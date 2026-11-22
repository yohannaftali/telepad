//! Behaviour tests for the database windows (#52–#54): no GUI, no network.

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::db::ddl::ColumnEdit;
use telepad_core::db::{
    Cell, Column, DatabaseInfo, DbTarget, Engine, Grid, Index, IndexKind, Profile, ResultSet,
    ServerInfo, TableInfo, TableKind, TableRef, TableSummary,
};
use telepad_core::model::{Container, HostReport};

use crate::db::*;
use crate::{Effect, Msg, State, update};

fn target(engine: Engine) -> DbTarget {
    DbTarget {
        engine,
        host: "dock1".into(),
        container: "shop-db".into(),
        container_id: "0123456789ab".into(),
        address: "172.18.0.5".into(),
        port: engine.default_port(),
    }
}

fn state() -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![HostEntry {
                id: Some(1),
                hostname: "192.0.2.1".into(),
                ..HostEntry::new("dock1")
            }]),
            ssh_config_error: None,
        },
        Instant::now(),
    );
    s
}

/// Effects of one message, without the SQL log's (tested in `tests_history`).
fn send(s: &mut State, m: DbMsg) -> Vec<DbEffect> {
    update(s, Msg::Db(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::Db(DbEffect::Log(_) | DbEffect::LoadLog { .. }) => None,
            Effect::Db(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn jobs(fx: &[DbEffect]) -> Vec<DbJob> {
    fx.iter()
        .filter_map(|e| match e {
            DbEffect::Job { job, .. } => Some(job.clone()),
            _ => None,
        })
        .collect()
}

fn session(s: &State) -> &DbSession {
    &s.model.db[0]
}

fn draft(s: &mut State) -> &mut DbDraft {
    let id = s.model.db[0].id;
    s.drafts.db.get_mut(&id).unwrap()
}

/// Open, log in and connect; returns the window id.
fn connected(s: &mut State, engine: Engine, databases: &[&str]) -> DbId {
    let fx = send(s, DbMsg::Open(target(engine)));
    assert!(matches!(fx[..], [DbEffect::LoadProfile { .. }]));
    let id = session(s).id;
    send(
        s,
        DbMsg::Loaded {
            id,
            profile: None,
            store: Ok(()),
            saved_password: false,
        },
    );
    draft(s).password = "pw".into();
    send(s, DbMsg::Connect(id));
    send(
        s,
        DbMsg::Connected {
            id,
            result: Ok(ServerInfo {
                version: "11.4.2-MariaDB".into(),
                databases: databases
                    .iter()
                    .map(|n| DatabaseInfo {
                        name: n.to_string(),
                        size: Some(1024),
                    })
                    .collect(),
            }),
        },
    );
    id
}

fn items() -> TableRef {
    TableRef {
        database: "shop".into(),
        schema: String::new(),
        table: "items".into(),
    }
}

fn col(name: &str) -> Column {
    Column {
        name: name.into(),
        type_name: "VARCHAR".into(),
        length: "50".into(),
        nullable: true,
        ..Default::default()
    }
}

fn info(with_key: bool) -> TableInfo {
    let mut id = col("id");
    id.type_name = "INT".into();
    id.nullable = false;
    TableInfo {
        table: items(),
        kind: TableKind::Table,
        columns: vec![id, col("name")],
        indexes: if with_key {
            vec![Index {
                name: "PRIMARY".into(),
                kind: IndexKind::Primary,
                columns: vec!["id".into()],
                constraint: true,
            }]
        } else {
            vec![]
        },
        foreign_keys: vec![],
        create_sql: String::new(),
    }
}

fn grid() -> Grid {
    Grid {
        columns: vec!["id".into(), "name".into()],
        rows: vec![
            vec![Cell::Text("1".into()), Cell::Text("alpha".into())],
            vec![Cell::Text("2".into()), Cell::Null],
        ],
        truncated: false,
    }
}

/// Connected to MySQL with `shop`.`items` open and loaded.
fn open_items(s: &mut State, with_key: bool) -> DbId {
    let id = connected(s, Engine::MySql, &["shop"]);
    let fx = send(s, DbMsg::SelectTable { id, table: items() });
    assert_eq!(jobs(&fx)[0], DbJob::Info(items()));
    assert!(matches!(
        &jobs(&fx)[1],
        DbJob::Page {
            offset: 0,
            limit: 1000,
            ..
        }
    ));
    assert_eq!(session(s).tab, DbTab::Data);
    send(
        s,
        DbMsg::InfoLoaded {
            id,
            table: items(),
            result: Ok(info(with_key)),
        },
    );
    send(
        s,
        DbMsg::PageLoaded {
            id,
            table: items(),
            offset: 0,
            result: Ok(grid()),
        },
    );
    id
}

#[test]
fn open_loads_the_saved_connection_and_connect_sends_the_login_once() {
    let mut s = state();
    send(&mut s, DbMsg::Open(target(Engine::MySql)));
    assert!(
        send(&mut s, DbMsg::Open(target(Engine::MySql))).is_empty(),
        "one window per target"
    );
    assert_eq!(s.model.db.len(), 1);
    let id = session(&s).id;
    assert_eq!(draft(&mut s).user, "root");
    send(
        &mut s,
        DbMsg::Loaded {
            id,
            profile: Some(Profile {
                key: target(Engine::MySql).key(),
                user: "shop".into(),
                database: "shop".into(),
                save_password: true,
                read_only: false,
            }),
            store: Ok(()),
            saved_password: true,
        },
    );
    send(
        &mut s,
        DbMsg::EnvUser {
            id,
            user: Some("other".into()),
        },
    );
    assert_eq!(
        draft(&mut s).user,
        "shop",
        "a saved user wins over the container env"
    );
    assert!(draft(&mut s).save_password);

    let fx = send(&mut s, DbMsg::Connect(id));
    match &fx[..] {
        [
            DbEffect::Connect {
                login,
                use_saved_password,
                profile,
                ..
            },
        ] => {
            assert_eq!(login.user, "shop");
            assert!(login.password.is_empty());
            assert!(*use_saved_password, "empty password + saved one");
            assert!(profile.save_password);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        send(&mut s, DbMsg::Connect(id)).is_empty(),
        "already connecting"
    );

    send(
        &mut s,
        DbMsg::Connected {
            id,
            result: Err("ERROR 1045 (28000): Access denied".into()),
        },
    );
    assert_eq!(session(&s).status, DbStatus::Login);
    assert!(session(&s).error.as_deref().unwrap().contains("1045"));

    draft(&mut s).password = "secret".into();
    let fx = send(&mut s, DbMsg::Connect(id));
    match &fx[..] {
        [
            DbEffect::Connect {
                login,
                use_saved_password,
                ..
            },
        ] => {
            assert_eq!(login.password.expose(), "secret");
            assert!(!use_saved_password);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        draft(&mut s).password.is_empty(),
        "password leaves the form"
    );
    assert!(!format!("{:?}", s.drafts).contains("secret"));

    // The saved database is opened right away.
    let fx = send(
        &mut s,
        DbMsg::Connected {
            id,
            result: Ok(ServerInfo {
                version: "8.4".into(),
                databases: vec![DatabaseInfo {
                    name: "shop".into(),
                    size: None,
                }],
            }),
        },
    );
    assert_eq!(session(&s).status, DbStatus::Ready);
    assert!(
        fx.iter()
            .any(|e| matches!(e, DbEffect::SaveProfile(p) if p.user == "shop" && p.save_password)),
        "the connection is remembered"
    );
    assert_eq!(
        jobs(&fx),
        vec![DbJob::Tables {
            database: "shop".into(),
            schema: String::new()
        }]
    );
    assert_eq!(session(&s).tab, DbTab::Database);
}

#[test]
fn no_password_store_means_no_saved_password() {
    let mut s = state();
    send(&mut s, DbMsg::Open(target(Engine::Postgres)));
    let id = session(&s).id;
    send(
        &mut s,
        DbMsg::Loaded {
            id,
            profile: Some(Profile {
                save_password: true,
                ..Default::default()
            }),
            store: Err("No password store available".into()),
            saved_password: true,
        },
    );
    assert!(!draft(&mut s).save_password);
    let fx = send(&mut s, DbMsg::Connect(id));
    assert!(
        matches!(&fx[..], [DbEffect::Connect { use_saved_password: false, profile, .. }] if !profile.save_password)
    );
}

#[test]
fn postgres_tree_loads_schemas_then_the_first_schema() {
    let mut s = state();
    let id = connected(&mut s, Engine::Postgres, &["postgres", "shop"]);
    let fx = send(
        &mut s,
        DbMsg::SelectDatabase {
            id,
            database: "shop".into(),
        },
    );
    assert_eq!(
        jobs(&fx),
        vec![DbJob::Schemas {
            database: "shop".into()
        }]
    );
    let fx = send(
        &mut s,
        DbMsg::SchemasLoaded {
            id,
            database: "shop".into(),
            result: Ok(vec!["public".into(), "sales".into()]),
        },
    );
    assert_eq!(
        jobs(&fx),
        vec![DbJob::Tables {
            database: "shop".into(),
            schema: "public".into()
        }]
    );
    send(
        &mut s,
        DbMsg::TablesLoaded {
            id,
            database: "shop".into(),
            schema: "public".into(),
            result: Ok(vec![TableSummary {
                name: "orders".into(),
                ..Default::default()
            }]),
        },
    );
    assert_eq!(session(&s).current_tables().unwrap()[0].name, "orders");
    assert_eq!(session(&s).current, Some(("shop".into(), "public".into())));
}

#[test]
fn data_paging_sorting_and_filter_reload_the_page() {
    let mut s = state();
    let id = open_items(&mut s, true);
    let page = |fx: &[DbEffect]| match jobs(fx).into_iter().next() {
        Some(DbJob::Page {
            sort,
            offset,
            filter,
            ..
        }) => (sort, offset, filter),
        other => panic!("{other:?}"),
    };
    assert!(
        send(&mut s, DbMsg::DataPage { id, forward: true }).is_empty(),
        "2 rows < 1000: no next page"
    );
    let fx = send(
        &mut s,
        DbMsg::DataSort {
            id,
            column: "name".into(),
        },
    );
    assert_eq!(page(&fx).0, Some(("name".into(), false)));
    let fx = send(
        &mut s,
        DbMsg::DataSort {
            id,
            column: "name".into(),
        },
    );
    assert_eq!(page(&fx).0, Some(("name".into(), true)));
    let fx = send(
        &mut s,
        DbMsg::DataSort {
            id,
            column: "name".into(),
        },
    );
    assert_eq!(page(&fx).0, None);
    draft(&mut s).data_filter = " id > 1 ".into();
    let fx = send(&mut s, DbMsg::DataApplyFilter(id));
    assert_eq!(page(&fx).2, "id > 1");
    let fx = send(&mut s, DbMsg::DataLimit { id, limit: 2 });
    assert!(matches!(jobs(&fx)[0], DbJob::Page { limit: 2, .. }));
    send(
        &mut s,
        DbMsg::PageLoaded {
            id,
            table: items(),
            offset: 0,
            result: Ok(grid()),
        },
    );
    let fx = send(&mut s, DbMsg::DataPage { id, forward: true });
    assert_eq!(page(&fx).1, 2);
    // A late page for another offset is ignored.
    send(
        &mut s,
        DbMsg::PageLoaded {
            id,
            table: items(),
            offset: 0,
            result: Err("late".into()),
        },
    );
    assert!(session(&s).data.error.is_none());
}

#[test]
fn grid_edits_wait_for_post_and_run_in_one_apply() {
    let mut s = state();
    let id = open_items(&mut s, true);
    send(
        &mut s,
        DbMsg::SetCell {
            id,
            row: RowRef::Existing(0),
            col: 1,
            value: Some("beta".into()),
        },
    );
    send(
        &mut s,
        DbMsg::SetCell {
            id,
            row: RowRef::Existing(1),
            col: 1,
            value: None,
        },
    );
    assert_eq!(
        session(&s).data.edits.changed.len(),
        1,
        "NULL -> NULL is no change"
    );
    send(&mut s, DbMsg::InsertRow(id));
    send(
        &mut s,
        DbMsg::SetCell {
            id,
            row: RowRef::New(0),
            col: 1,
            value: Some("gamma".into()),
        },
    );
    send(
        &mut s,
        DbMsg::DeleteRow {
            id,
            row: RowRef::Existing(1),
        },
    );
    assert_eq!(session(&s).data.edits.count(), 3);
    assert_eq!(
        session(&s).cell_value(RowRef::Existing(0), 1),
        Some(Cell::Text("beta".into()))
    );
    assert!(
        !send(&mut s, DbMsg::DataReload(id))
            .iter()
            .any(|e| matches!(e, DbEffect::Job { .. }))
    );
    assert!(
        session(&s).error.is_some(),
        "reload refused while edits are pending"
    );

    assert!(
        send(&mut s, DbMsg::Post(id)).is_empty(),
        "Post only shows the SQL"
    );
    let Some(DbConfirm::Post { statements }) = &session(&s).confirm else {
        panic!("{:?}", session(&s).confirm)
    };
    let sql: Vec<String> = statements
        .iter()
        .map(|st| st.display(Engine::MySql))
        .collect();
    assert_eq!(
        sql,
        vec![
            "UPDATE `shop`.`items` SET `name` = 'beta' WHERE `id` = '1' LIMIT 1",
            "INSERT INTO `shop`.`items` (`name`) VALUES ('gamma')",
            "DELETE FROM `shop`.`items` WHERE `id` = '2' LIMIT 1",
        ]
    );
    let fx = send(&mut s, DbMsg::ConfirmOk(id));
    assert!(
        matches!(&jobs(&fx)[..], [DbJob::Apply { database, statements }] if database == "shop" && statements.len() == 3)
    );

    send(
        &mut s,
        DbMsg::Posted {
            id,
            result: Err("1 rows matched instead of 1".into()),
        },
    );
    assert_eq!(
        session(&s).data.edits.count(),
        3,
        "a failed post keeps the edits"
    );
    let fx = send(&mut s, DbMsg::Posted { id, result: Ok(3) });
    assert!(session(&s).data.edits.is_empty());
    assert!(
        matches!(jobs(&fx)[..], [DbJob::Page { .. }]),
        "reloaded after saving"
    );

    // Undo an edit by typing the old value back; discard drops the rest.
    send(
        &mut s,
        DbMsg::SetCell {
            id,
            row: RowRef::Existing(0),
            col: 1,
            value: Some("x".into()),
        },
    );
    send(
        &mut s,
        DbMsg::SetCell {
            id,
            row: RowRef::Existing(0),
            col: 1,
            value: Some("alpha".into()),
        },
    );
    assert!(session(&s).data.edits.is_empty());
    send(
        &mut s,
        DbMsg::DeleteRow {
            id,
            row: RowRef::Existing(0),
        },
    );
    send(&mut s, DbMsg::Discard(id));
    assert!(session(&s).data.edits.is_empty());
}

#[test]
fn tables_without_a_key_and_read_only_sessions_cannot_be_edited() {
    let mut s = state();
    let id = open_items(&mut s, false);
    assert!(session(&s).edit_blocked().unwrap().contains("primary key"));
    send(
        &mut s,
        DbMsg::SetCell {
            id,
            row: RowRef::Existing(0),
            col: 1,
            value: Some("x".into()),
        },
    );
    send(&mut s, DbMsg::InsertRow(id));
    assert!(session(&s).data.edits.is_empty());

    let mut s = state();
    let id = open_items(&mut s, true);
    let fx = send(&mut s, DbMsg::SetReadOnly(id, true));
    assert_eq!(jobs(&fx), vec![DbJob::ReadOnly(true)]);
    let fx = send(
        &mut s,
        DbMsg::ReadOnlySet {
            id,
            on: true,
            result: Ok(()),
        },
    );
    assert!(matches!(&fx[..], [DbEffect::SaveProfile(p)] if p.read_only));
    send(
        &mut s,
        DbMsg::SetCell {
            id,
            row: RowRef::Existing(0),
            col: 1,
            value: Some("x".into()),
        },
    );
    assert!(session(&s).data.edits.is_empty());
    let fx = send(
        &mut s,
        DbMsg::QueryRun {
            id,
            text: "SELECT 1; UPDATE items SET name = 'x' WHERE id = 1".into(),
        },
    );
    assert!(fx.is_empty());
    assert!(
        session(&s).queries[0].results[0]
            .error
            .as_deref()
            .unwrap()
            .contains("Read-only")
    );
    let fx = send(
        &mut s,
        DbMsg::QueryRun {
            id,
            text: "SELECT 1; SHOW TABLES".into(),
        },
    );
    assert!(matches!(&jobs(&fx)[..], [DbJob::Run { statements, .. }] if statements.len() == 2));
    send(&mut s, DbMsg::StructureEdit(id));
    send(&mut s, DbMsg::TruncateTable { id, table: items() });
    assert!(session(&s).confirm.is_none());
    assert!(session(&s).error.as_deref().unwrap().contains("Read-only"));
}

#[test]
fn dangerous_queries_ask_first_and_results_come_back_per_tab() {
    let mut s = state();
    let id = open_items(&mut s, true);
    send(
        &mut s,
        DbMsg::SelectDatabase {
            id,
            database: "shop".into(),
        },
    );
    let fx = send(
        &mut s,
        DbMsg::QueryRun {
            id,
            text: "SELECT 1;\nDELETE FROM items".into(),
        },
    );
    assert!(jobs(&fx).is_empty());
    let Some(DbConfirm::Query {
        reasons,
        statements,
        ..
    }) = &session(&s).confirm
    else {
        panic!()
    };
    assert_eq!(statements.len(), 2);
    assert!(reasons[0].contains("DELETE without WHERE"));
    send(&mut s, DbMsg::ConfirmCancel(id));
    assert!(session(&s).confirm.is_none());

    send(&mut s, DbMsg::QueryNew(id));
    assert_eq!(session(&s).query_tab, 1);
    let fx = send(
        &mut s,
        DbMsg::QueryRun {
            id,
            text: "DROP TABLE x".into(),
        },
    );
    assert!(fx.is_empty());
    let fx = send(&mut s, DbMsg::ConfirmOk(id));
    assert_eq!(
        jobs(&fx),
        vec![DbJob::Run {
            tab: 1,
            database: "shop".into(),
            schema: String::new(),
            statements: vec!["DROP TABLE x".into()]
        }]
    );
    assert!(session(&s).queries[1].running);
    assert!(
        send(
            &mut s,
            DbMsg::QueryRun {
                id,
                text: "SELECT 2".into()
            }
        )
        .is_empty(),
        "one run per tab"
    );
    assert!(matches!(
        &send(&mut s, DbMsg::Cancel(id))[..],
        [DbEffect::Cancel(_)]
    ));
    send(
        &mut s,
        DbMsg::QueryDone {
            id,
            tab: 1,
            results: vec![ResultSet {
                statement: "DROP TABLE x".into(),
                affected: Some(0),
                ..Default::default()
            }],
        },
    );
    assert!(!session(&s).queries[1].running);
    assert_eq!(session(&s).queries[1].results.len(), 1);
    send(&mut s, DbMsg::QueryClose { id, tab: 1 });
    assert_eq!(session(&s).queries.len(), 1);
    assert_eq!(s.drafts.db.values().next().unwrap().queries.len(), 1);
}

#[test]
fn structure_changes_show_the_sql_and_reload_after_running() {
    let mut s = state();
    let id = open_items(&mut s, true);
    send(&mut s, DbMsg::StructureEdit(id));
    assert_eq!(session(&s).tab, DbTab::Table);
    send(&mut s, DbMsg::StructureSave(id));
    assert_eq!(session(&s).notice.as_deref(), Some("Nothing changed"));
    draft(&mut s)
        .structure
        .as_mut()
        .unwrap()
        .columns
        .push(ColumnEdit {
            original: None,
            column: col("note"),
        });
    send(&mut s, DbMsg::StructureSave(id));
    let Some(DbConfirm::Ddl {
        statements,
        type_name: None,
        ..
    }) = &session(&s).confirm
    else {
        panic!("{:?}", session(&s).confirm)
    };
    assert!(
        statements[0].contains("ADD COLUMN `note` VARCHAR(50) NULL"),
        "{statements:?}"
    );
    let fx = send(&mut s, DbMsg::ConfirmOk(id));
    let after = match &jobs(&fx)[..] {
        [DbJob::Ddl { after, .. }] => after.clone(),
        other => panic!("{other:?}"),
    };
    let fx = send(
        &mut s,
        DbMsg::DdlDone {
            id,
            result: Ok(()),
            after,
        },
    );
    assert!(draft(&mut s).structure.is_none());
    let j = jobs(&fx);
    assert!(j.contains(&DbJob::Info(items())));
    assert!(j.iter().any(|x| matches!(x, DbJob::Tables { .. })));
    assert!(j.iter().any(|x| matches!(x, DbJob::Page { .. })));

    // A broken edit is reported, not run.
    send(
        &mut s,
        DbMsg::InfoLoaded {
            id,
            table: items(),
            result: Ok(info(true)),
        },
    );
    send(&mut s, DbMsg::StructureEdit(id));
    draft(&mut s).structure.as_mut().unwrap().columns[1]
        .column
        .type_name = "INT; DROP".into();
    send(&mut s, DbMsg::StructureSave(id));
    assert!(session(&s).confirm.is_none());
    assert!(session(&s).error.is_some());
}

#[test]
fn drop_and_truncate_need_the_typed_table_name() {
    let mut s = state();
    let id = open_items(&mut s, true);
    send(&mut s, DbMsg::DropTable { id, table: items() });
    assert!(
        matches!(&session(&s).confirm, Some(DbConfirm::Ddl { type_name: Some(n), .. }) if n == "items")
    );
    draft(&mut s).typed = "item".into();
    assert!(jobs(&send(&mut s, DbMsg::ConfirmOk(id))).is_empty());
    assert!(session(&s).confirm.is_some(), "still asking");
    draft(&mut s).typed = "items".into();
    let fx = send(&mut s, DbMsg::ConfirmOk(id));
    let after = match &jobs(&fx)[..] {
        [
            DbJob::Ddl {
                statements, after, ..
            },
        ] => {
            assert_eq!(statements, &vec!["DROP TABLE `shop`.`items`".to_string()]);
            after.clone()
        }
        other => panic!("{other:?}"),
    };
    let fx = send(
        &mut s,
        DbMsg::DdlDone {
            id,
            result: Ok(()),
            after,
        },
    );
    assert_eq!(session(&s).data.table, None, "the dropped table is closed");
    assert_eq!(
        jobs(&fx),
        vec![DbJob::Tables {
            database: "shop".into(),
            schema: String::new()
        }]
    );
}

#[test]
fn new_and_renamed_tables_open_after_creating() {
    let mut s = state();
    let id = open_items(&mut s, true);
    send(
        &mut s,
        DbMsg::NewTable {
            id,
            database: "shop".into(),
            schema: String::new(),
        },
    );
    draft(&mut s).new_table.as_mut().unwrap().name = "fresh".into();
    send(&mut s, DbMsg::NewTableSave(id));
    assert!(
        matches!(&session(&s).confirm, Some(DbConfirm::Ddl { statements, .. }) if statements[0].starts_with("CREATE TABLE `shop`.`fresh`"))
    );
    let fx = send(&mut s, DbMsg::ConfirmOk(id));
    let after = match &jobs(&fx)[..] {
        [DbJob::Ddl { after, .. }] => after.clone(),
        other => panic!("{other:?}"),
    };
    let fx = send(
        &mut s,
        DbMsg::DdlDone {
            id,
            result: Ok(()),
            after,
        },
    );
    let fresh = TableRef {
        table: "fresh".into(),
        ..items()
    };
    assert!(jobs(&fx).contains(&DbJob::Info(fresh.clone())));
    assert_eq!(session(&s).data.table.as_ref(), Some(&fresh));

    send(&mut s, DbMsg::RenameTable(id));
    draft(&mut s).rename = Some("fresher".into());
    send(&mut s, DbMsg::RenameSave(id));
    assert!(
        matches!(&session(&s).confirm, Some(DbConfirm::Ddl { statements, .. }) if statements[0] == "RENAME TABLE `shop`.`fresh` TO `shop`.`fresher`")
    );
}

#[test]
fn reconnect_closes_the_connection_and_shows_the_login_again() {
    let mut s = state();
    let id = open_items(&mut s, true);
    send(
        &mut s,
        DbMsg::QueryRun {
            id,
            text: "SELECT 1".into(),
        },
    );
    let fx = send(&mut s, DbMsg::Reconnect(id));
    assert!(matches!(fx[..], [DbEffect::Close(x)] if x == id));
    assert_eq!(session(&s).status, DbStatus::Login);
    assert!(session(&s).databases.is_empty() && session(&s).data.table.is_none());
    assert!(!session(&s).queries[0].running);
    assert_eq!(draft(&mut s).user, "root", "the form keeps its fields");
    assert!(matches!(
        &send(&mut s, DbMsg::Connect(id))[..],
        [DbEffect::Connect { .. }]
    ));
}

#[test]
fn closing_drops_the_window_and_its_connection() {
    let mut s = state();
    let id = connected(&mut s, Engine::MySql, &["shop"]);
    assert_eq!(send(&mut s, DbMsg::Close(id)).len(), 1);
    assert!(s.model.db.is_empty() && s.drafts.db.is_empty());
    assert!(
        send(&mut s, DbMsg::Refresh(id)).is_empty(),
        "late messages are ignored"
    );
}

#[test]
fn databases_of_a_host_come_from_containers_and_listeners() {
    let mut r = HostReport::default();
    r.docker.containers.push(Container {
        id: "0123456789ab".into(),
        name: "shop-db".into(),
        image: "mariadb:11".into(),
        state: "running".into(),
        networks: vec![("shop_default".into(), "172.18.0.5".into())],
        ..Default::default()
    });
    r.listeners.push(telepad_core::model::Listener {
        proto: "tcp".into(),
        address: "127.0.0.1".into(),
        port: 5432,
        process: "postgres".into(),
    });
    let t = host_db_targets("dock1", &r);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].container, "shop-db");
    assert_eq!((t[1].engine, t[1].port), (Engine::Postgres, 5432));
}
