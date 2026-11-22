//! Behaviour tests for #100: file transfers, file operations and database
//! statements are logged to the history, and windows show it.

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::db::{
    DatabaseInfo, DbTarget, Engine, ResultSet, ServerInfo, Statement, TableRef,
};
use telepad_core::files::ops::Report;
use telepad_core::files::transfer::Item;
use telepad_core::files::{Entry, Kind};
use telepad_core::history::{Entry as LogEntry, Kind as LogKind, Outcome};

use crate::db::{AfterDdl, DbConfirm, DbEffect, DbMsg};
use crate::files::{FmEffect, FmMsg, Mode, OpKind, Side};
use crate::{Effect, Msg, State, update};

fn state() -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![
                HostEntry {
                    id: Some(1),
                    hostname: "192.0.2.10".into(),
                    ..HostEntry::new("web1")
                },
                HostEntry {
                    id: Some(2),
                    hostname: "192.0.2.20".into(),
                    ..HostEntry::new("dock1")
                },
            ]),
            ssh_config_error: None,
        },
        Instant::now(),
    );
    s
}

fn fm(s: &mut State, m: FmMsg) -> Vec<FmEffect> {
    update(s, Msg::Files(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::Files(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn db(s: &mut State, m: DbMsg) -> Vec<DbEffect> {
    update(s, Msg::Db(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::Db(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn logged_f(fx: &[FmEffect]) -> Vec<LogEntry> {
    fx.iter()
        .filter_map(|e| match e {
            FmEffect::Log(l) => Some(l.clone()),
            _ => None,
        })
        .collect()
}

fn logged_d(fx: &[DbEffect]) -> Vec<LogEntry> {
    fx.iter()
        .filter_map(|e| match e {
            DbEffect::Log(l) => Some(l.clone()),
            _ => None,
        })
        .collect()
}

/// A connected file window on web1 with one local file listed.
fn files_open(s: &mut State) -> u64 {
    let fx = fm(s, FmMsg::Open("web1".into()));
    assert!(
        fx.iter()
            .any(|e| matches!(e, FmEffect::LoadHistory { alias, .. } if alias == "web1")),
        "the window reads its history"
    );
    let id = s.model.files[0].id;
    fm(
        s,
        FmMsg::Connected {
            id,
            result: Ok((Mode::Sftp, "/home/ops".into(), None)),
            local_home: "/home/me".into(),
        },
    );
    fm(
        s,
        FmMsg::Listed {
            id,
            side: Side::Local,
            result: Ok((
                "/home/me".into(),
                vec![Entry {
                    name: "site.tar".into(),
                    kind: Kind::File,
                    size: 2048,
                    ..Default::default()
                }],
            )),
        },
    );
    fm(
        s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok(("/srv/www".into(), vec![])),
        },
    );
    id
}

#[test]
fn a_finished_transfer_is_logged_with_paths_size_and_method() {
    let mut s = state();
    let id = files_open(&mut s);
    fm(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site.tar".into()],
        },
    );
    let transfer = s.model.files[0].transfers[0].id;
    fm(
        &mut s,
        FmMsg::Planned {
            id,
            transfer,
            result: Ok(vec![Item {
                src: "/home/me/site.tar".into(),
                dst: "/srv/www/site.tar".into(),
                label: "site.tar".into(),
                is_dir: false,
                size: 2048,
                mtime: None,
                perm: None,
                exists: None,
            }]),
        },
    );
    s.model.files[0].transfers[0].bytes_done = 2048;
    let fx = fm(
        &mut s,
        FmMsg::Finished {
            id,
            transfer,
            result: Ok((1, 0, 0)),
        },
    );
    let l = logged_f(&fx);
    assert_eq!(l.len(), 1);
    let e = &l[0];
    assert_eq!(e.kind, LogKind::Transfer);
    assert_eq!(e.host, "web1");
    assert!(e.title.starts_with("Upload"), "{}", e.title);
    assert!(
        e.detail.contains("site.tar") && e.detail.contains("→ /srv/www"),
        "{}",
        e.detail
    );
    assert_eq!((e.bytes, e.files), (Some(2048), Some(1)));
    assert_eq!(e.method, "SFTP");
    assert_eq!(e.outcome, Outcome::Done);
    assert!(e.duration_ms.is_some());
    assert_eq!(
        s.model.files[0].history.len(),
        1,
        "shown in the window at once"
    );
}

#[test]
fn failed_and_cancelled_transfers_are_logged_too() {
    let mut s = state();
    let id = files_open(&mut s);
    for (n, result, want) in [
        (0, Err("Permission denied".to_string()), Outcome::Failed),
        (1, Err("Cancelled".to_string()), Outcome::Cancelled),
    ] {
        fm(
            &mut s,
            FmMsg::Copy {
                id,
                from: Side::Local,
                names: vec!["site.tar".into()],
            },
        );
        let transfer = s.model.files[0].transfers[n].id;
        let fx = fm(
            &mut s,
            FmMsg::Finished {
                id,
                transfer,
                result,
            },
        );
        assert_eq!(logged_f(&fx)[0].outcome, want);
    }
    assert_eq!(s.model.files[0].history[0].error, "Permission denied");
}

#[test]
fn file_operations_and_editor_saves_are_logged() {
    let mut s = state();
    let id = files_open(&mut s);
    fm(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Remote,
            op: OpKind::NewFolder,
        },
    );
    s.drafts.files.get_mut(&id).unwrap().name = "backup".into();
    fm(&mut s, FmMsg::Confirm(id));
    let fx = fm(
        &mut s,
        FmMsg::OpDone {
            id,
            side: Side::Remote,
            result: Ok((
                Report {
                    done: 1,
                    errors: vec![],
                },
                Some("backup".into()),
            )),
        },
    );
    let l = logged_f(&fx);
    assert_eq!(l[0].kind, LogKind::FileOp);
    assert_eq!(l[0].title, "New folder backup");
    assert_eq!(l[0].detail, "/srv/www/backup");
    // A local operation says so.
    fm(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Local,
            op: OpKind::NewFolder,
        },
    );
    s.drafts.files.get_mut(&id).unwrap().name = "x".into();
    fm(&mut s, FmMsg::Confirm(id));
    let fx = fm(
        &mut s,
        FmMsg::OpDone {
            id,
            side: Side::Local,
            result: Err("exists".into()),
        },
    );
    let l = logged_f(&fx);
    assert!(l[0].title.ends_with("(this computer)"));
    assert_eq!(l[0].outcome, Outcome::Failed);
}

#[test]
fn history_read_back_keeps_this_sessions_lines_after_it() {
    let mut s = state();
    let id = files_open(&mut s);
    s.model.files[0].history.push(LogEntry {
        title: "now".into(),
        ..Default::default()
    });
    fm(
        &mut s,
        FmMsg::HistoryLoaded {
            id,
            result: Ok(vec![LogEntry {
                at: Some(1),
                title: "earlier".into(),
                ..Default::default()
            }]),
        },
    );
    let titles: Vec<&str> = s.model.files[0]
        .history
        .iter()
        .map(|e| e.title.as_str())
        .collect();
    assert_eq!(titles, ["earlier", "now"]);
    let fx = fm(&mut s, FmMsg::HistoryClear(id));
    assert!(
        fx.iter()
            .any(|e| matches!(e, FmEffect::ClearHistory { alias } if alias == "web1"))
    );
    assert!(s.model.files[0].history.is_empty());
}

fn target() -> DbTarget {
    DbTarget {
        engine: Engine::MySql,
        host: "dock1".into(),
        container: "shop-db".into(),
        container_id: "0123".into(),
        address: "172.18.0.5".into(),
        port: 3306,
    }
}

fn db_connected(s: &mut State) -> u64 {
    let fx = db(s, DbMsg::Open(target()));
    assert!(
        fx.iter()
            .any(|e| matches!(e, DbEffect::LoadLog { key, .. } if *key == target().key()))
    );
    let id = s.model.db[0].id;
    db(
        s,
        DbMsg::Loaded {
            id,
            profile: None,
            store: Ok(()),
            saved_password: false,
        },
    );
    s.drafts.db.get_mut(&id).unwrap().password = "pw".into();
    db(s, DbMsg::Connect(id));
    db(
        s,
        DbMsg::Connected {
            id,
            result: Ok(ServerInfo {
                version: "11.4".into(),
                databases: vec![DatabaseInfo {
                    name: "shop".into(),
                    size: None,
                }],
            }),
        },
    );
    db(
        s,
        DbMsg::SelectDatabase {
            id,
            database: "shop".into(),
        },
    );
    id
}

fn done(statement: &str, affected: Option<u64>, error: Option<&str>) -> ResultSet {
    ResultSet {
        statement: statement.into(),
        affected,
        millis: 12,
        error: error.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn query_tab_changes_are_logged_selects_only_when_asked() {
    let mut s = state();
    let id = db_connected(&mut s);
    let fx = db(
        &mut s,
        DbMsg::QueryDone {
            id,
            tab: 0,
            results: vec![
                done("SELECT * FROM items", None, None),
                done("UPDATE items SET n = 2 WHERE id = 1", Some(1), None),
                done(
                    "CREATE USER 'app'@'%' IDENTIFIED BY 'hunter2'",
                    Some(0),
                    None,
                ),
                done("DELETE FROM nope", None, Some("no such table")),
            ],
        },
    );
    let l = logged_d(&fx);
    assert_eq!(l.len(), 3, "SELECT is not logged by default");
    assert_eq!(l[0].rows, Some(1));
    assert_eq!(l[0].duration_ms, Some(12));
    assert_eq!(l[0].database, "shop");
    assert_eq!(l[0].target, target().key());
    assert_eq!(l[0].user, "root");
    assert!(!l[1].detail.contains("hunter2"), "{}", l[1].detail);
    assert!(l[1].detail.contains("'***'"));
    assert_eq!(l[2].outcome, Outcome::Failed);
    assert_eq!(l[2].error, "no such table");
    s.drafts.db.get_mut(&id).unwrap().log_select = true;
    let fx = db(
        &mut s,
        DbMsg::QueryDone {
            id,
            tab: 0,
            results: vec![done("SELECT 1", None, None)],
        },
    );
    assert_eq!(logged_d(&fx).len(), 1);
    assert_eq!(s.model.db[0].log.len(), 4);
}

#[test]
fn grid_posts_log_each_statement_and_a_rollback() {
    let mut s = state();
    let id = db_connected(&mut s);
    s.model.db[0].data.table = Some(TableRef {
        database: "shop".into(),
        schema: String::new(),
        table: "items".into(),
    });
    for (result, want) in [
        (Ok(2), Outcome::Done),
        (Err("Duplicate entry".to_string()), Outcome::RolledBack),
    ] {
        s.model.db[0].confirm = Some(DbConfirm::Post {
            statements: vec![
                Statement {
                    sql: "UPDATE `items` SET `name` = ? WHERE `id` = ?".into(),
                    params: vec![Some("beta".into()), Some("1".into())],
                },
                Statement {
                    sql: "DELETE FROM `items` WHERE `id` = ?".into(),
                    params: vec![Some("2".into())],
                },
            ],
        });
        db(&mut s, DbMsg::ConfirmOk(id));
        assert!(logged_d(&[]).is_empty());
        let fx = db(&mut s, DbMsg::Posted { id, result });
        let l = logged_d(&fx);
        assert_eq!(l.len(), 1, "one entry per post");
        assert_eq!(l[0].outcome, want);
        assert!(
            l[0].detail.contains("'beta'"),
            "values written in: {}",
            l[0].detail
        );
        assert!(l[0].detail.contains("DELETE FROM"));
        assert!(l[0].title.contains("items"));
    }
}

#[test]
fn structure_changes_are_logged() {
    let mut s = state();
    let id = db_connected(&mut s);
    s.model.db[0].confirm = Some(DbConfirm::Ddl {
        title: "Add column".into(),
        database: "shop".into(),
        statements: vec!["ALTER TABLE `items` ADD COLUMN `x` INT".into()],
        type_name: None,
        after: AfterDdl::ReloadTables {
            database: "shop".into(),
            schema: String::new(),
        },
    });
    db(&mut s, DbMsg::ConfirmOk(id));
    let fx = db(
        &mut s,
        DbMsg::DdlDone {
            id,
            result: Ok(()),
            after: AfterDdl::ReloadTables {
                database: "shop".into(),
                schema: String::new(),
            },
        },
    );
    let l = logged_d(&fx);
    assert_eq!(l[0].title, "Add column");
    assert!(l[0].detail.starts_with("ALTER TABLE"));
    assert_eq!(l[0].method, "mysql");
    // Clear asks in the window, then empties both.
    let fx = db(&mut s, DbMsg::LogClear(id));
    assert!(fx.iter().any(|e| matches!(e, DbEffect::ClearLog { .. })));
    assert!(s.model.db[0].log.is_empty());
}
