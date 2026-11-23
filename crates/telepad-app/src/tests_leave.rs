//! Behaviour tests for #99: file, database and editor windows come back
//! with their workspace; leaving a workspace lists what would be lost first;
//! file transfers keep running in the background.

use std::time::{Duration, Instant};

use telepad_core::collector::{Scope, Update as CUpdate};
use telepad_core::config::HostEntry;
use telepad_core::db::{DbTarget, Engine, Profile};
use telepad_core::files::{Entry, Kind};
use telepad_core::prefs::{Current, WorkspacePrefs};
use telepad_core::store::UiPrefs;
use telepad_core::update::UpdateConfig;

use crate::db::{DbEffect, DbMsg, DbStatus};
use crate::files::{FmEffect, FmMsg, Mode, Side};
use crate::workspace::{Win, WkMsg, esc, layout, unesc};
use crate::*;

fn hosts() -> Msg {
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
    }
}

/// Started with `own` as the `<own>` layout and one custom workspace "b".
fn started(own: &str) -> (State, Vec<Effect>) {
    let mut s = State::default();
    let t = Instant::now();
    let mut fx = update(
        &mut s,
        Msg::Started {
            db_path: "telepad.db".into(),
            ssh_config_path: "telepad.ssh_config".into(),
            imported_from: None,
            update_config: Ok(UpdateConfig::default()),
            ui_prefs: UiPrefs::default(),
            strategy: Ok(Default::default()),
            prefs: Ok(WorkspacePrefs {
                own: own.into(),
                custom: vec![("b".into(), "|0|0|docker|docker".into())],
                ..Default::default()
            }),
        },
        t,
    );
    fx.extend(update(&mut s, hosts(), t));
    (s, fx)
}

fn files_fx(fx: &[Effect]) -> Vec<FmEffect> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Files(f) => Some(f.clone()),
            _ => None,
        })
        .collect()
}

fn db_fx(fx: &[Effect]) -> Vec<DbEffect> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Db(f) => Some(f.clone()),
            _ => None,
        })
        .collect()
}

fn wk(s: &mut State, m: WkMsg) -> Vec<Effect> {
    update(s, Msg::Wk(m), Instant::now())
}

fn connected(s: &mut State, alias: &str) -> Vec<Effect> {
    let id = s
        .model
        .files
        .iter()
        .find(|f| f.host.alias == alias)
        .unwrap()
        .id;
    update(
        s,
        Msg::Files(FmMsg::Connected {
            id,
            result: Ok((Mode::Sftp, "/home/ops".into(), None)),
            local_home: "/home/me".into(),
        }),
        Instant::now(),
    )
}

#[test]
fn escaping_keeps_any_path_in_one_token() {
    for p in ["/srv/a,b;c|d", "C:\\data\\100%", "plain", "line\nbreak"] {
        let e = esc(p);
        assert!(!e.contains([',', ';', '|', '\n']), "{e}");
        assert_eq!(unesc(&e), p);
    }
    assert_eq!(unesc("50%zz"), "50%zz", "unknown sequences stay");
}

#[test]
fn file_and_editor_windows_are_saved_with_folders_and_paths() {
    let (mut s, _) = started("");
    update(
        &mut s,
        Msg::Files(FmMsg::Open("web1".into())),
        Instant::now(),
    );
    connected(&mut s, "web1");
    let id = s.model.files[0].id;
    for (side, path) in [(Side::Local, "/home/me"), (Side::Remote, "/srv/www,old")] {
        update(
            &mut s,
            Msg::Files(FmMsg::Listed {
                id,
                side,
                result: Ok((path.into(), vec![])),
            }),
            Instant::now(),
        );
    }
    update(
        &mut s,
        Msg::Files(FmMsg::Editor {
            id,
            msg: crate::editor::EdMsg::OpenPath {
                side: Side::Remote,
                path: "/etc/nginx/nginx.conf".into(),
            },
        }),
        Instant::now(),
    );
    let l = layout(&s.model);
    assert!(l.contains("files:web1,/home/me,/srv/www%2Cold"), "{l}");
    assert!(l.contains("editor:web1,R/etc/nginx/nginx.conf"), "{l}");
    assert!(!l.contains("server {"), "never file text");
}

#[test]
fn file_and_editor_windows_come_back_with_their_folders_and_files() {
    let saved = "10,20,900,500|0|0|files:web1|files:web1,/home/me/src,/srv/www\n\
                 30,40,700,400|1|0|editor:web1|editor:web1,R/etc/hosts,L/home/me/notes.txt";
    let (mut s, fx) = started(saved);
    // Connected again once the hosts are known.
    assert!(
        files_fx(&fx)
            .iter()
            .any(|e| matches!(e, FmEffect::Connect { host, .. } if host.alias == "web1"))
    );
    let g = &s.model.ws.groups[s
        .model
        .ws
        .group_of(Win::Files(s.model.files[0].id))
        .unwrap()];
    assert_eq!(g.rect, Some([10.0, 20.0, 900.0, 500.0]));
    let fx = files_fx(&connected(&mut s, "web1"));
    let lists: Vec<(Side, String)> = fx
        .iter()
        .filter_map(|e| match e {
            FmEffect::List { side, path, .. } => Some((*side, path.clone())),
            _ => None,
        })
        .collect();
    assert!(lists.contains(&(Side::Local, "/home/me/src".into())));
    assert!(lists.contains(&(Side::Remote, "/srv/www".into())));
    let loads: Vec<(Side, String)> = fx
        .iter()
        .filter_map(|e| match e {
            FmEffect::Load { side, path, .. } => Some((*side, path.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        loads,
        vec![
            (Side::Remote, "/etc/hosts".into()),
            (Side::Local, "/home/me/notes.txt".into())
        ],
        "read from disk / host, not from the layout"
    );
    // The editor window is back where it was, minimized.
    let id = s.model.files[0].id;
    let g = &s.model.ws.groups[s.model.ws.group_of(Win::Editor(id)).unwrap()];
    assert_eq!(g.rect, Some([30.0, 40.0, 700.0, 400.0]));
    assert!(g.minimized);
}

#[test]
fn a_saved_folder_that_is_gone_falls_back_to_home_with_a_note() {
    let (mut s, _) = started("|0|0|files:web1|files:web1,/home/me,/srv/gone");
    connected(&mut s, "web1");
    let id = s.model.files[0].id;
    let fx = files_fx(&update(
        &mut s,
        Msg::Files(FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Err("No such file".into()),
        }),
        Instant::now(),
    ));
    assert!(fx.iter().any(
        |e| matches!(e, FmEffect::List { side: Side::Remote, path, .. } if path == "/home/ops")
    ));
    assert!(
        s.model.files[0]
            .notice
            .as_ref()
            .unwrap()
            .contains("/srv/gone")
    );
}

#[test]
fn windows_of_a_host_that_is_gone_are_left_out_with_a_note() {
    let (s, _) = started("|0|0|files:old1|files:old1,/a,/b");
    assert!(s.model.files.is_empty());
    assert!(s.model.message.as_ref().unwrap().0.contains("old1"));
    assert!(s.model.ws.pending.is_empty());
}

fn db_token(container: &str) -> String {
    format!(
        "db:{},mysql,dock1,{container},172.18.0.5,3306,shop,",
        esc(&format!("dock1 / container {container} / mysql"))
    )
}

fn report_done(alias: &str, result: Result<Box<telepad_core::model::HostReport>, String>) -> Msg {
    Msg::Collected(CUpdate::Done {
        alias: alias.into(),
        result,
        scope: Scope::FULL,
        no_ping: false,
        elapsed: Duration::from_millis(10),
    })
}

#[test]
fn database_windows_wait_for_their_host_then_reconnect() {
    let (mut s, _) = started(&format!("|0|0|{0}|{0}", db_token("shop-db")));
    assert!(s.model.db.is_empty(), "waits for the host to be checked");
    // The host could not be checked: the saved target, the window shows the error.
    let fx = db_fx(&update(
        &mut s,
        report_done("dock1", Err("timeout".into())),
        Instant::now(),
    ));
    assert_eq!(s.model.db.len(), 1);
    assert!(fx.iter().any(|e| matches!(e, DbEffect::LoadProfile { .. })));
    let id = s.model.db[0].id;
    let key = s.model.db[0].target.key();
    assert!(s.model.ws.is_open(Win::Db(id)));
    // A saved password: it connects by itself, to the saved database.
    let fx = db_fx(&update(
        &mut s,
        Msg::Db(DbMsg::Loaded {
            id,
            profile: Some(Profile {
                key,
                user: "shop".into(),
                database: "other".into(),
                save_password: true,
                read_only: false,
            }),
            store: Ok(()),
            saved_password: true,
        }),
        Instant::now(),
    ));
    assert!(
        fx.iter()
            .any(|e| matches!(e, DbEffect::Connect { login, .. } if login.database == "shop"))
    );
    assert_eq!(s.model.db[0].status, DbStatus::Connecting);
}

#[test]
fn a_database_without_a_saved_password_waits_on_the_connect_form() {
    let (mut s, _) = started(&format!("|0|0|{0}|{0}", db_token("shop-db")));
    update(
        &mut s,
        report_done("dock1", Err("timeout".into())),
        Instant::now(),
    );
    let id = s.model.db[0].id;
    let fx = db_fx(&update(
        &mut s,
        Msg::Db(DbMsg::Loaded {
            id,
            profile: None,
            store: Ok(()),
            saved_password: false,
        }),
        Instant::now(),
    ));
    assert!(!fx.iter().any(|e| matches!(e, DbEffect::Connect { .. })));
    assert_eq!(s.model.db[0].status, DbStatus::Login);
}

#[test]
fn a_database_that_is_gone_is_left_out_with_a_note() {
    let (mut s, _) = started(&format!("|0|0|{0}|{0}", db_token("removed-db")));
    let report = telepad_core::model::HostReport::default();
    update(
        &mut s,
        report_done("dock1", Ok(Box::new(report))),
        Instant::now(),
    );
    assert!(s.model.db.is_empty());
    assert!(s.model.message.as_ref().unwrap().0.contains("removed-db"));
}

#[test]
fn database_windows_are_saved_with_target_and_database() {
    let (mut s, _) = started("");
    let t = DbTarget {
        engine: Engine::MySql,
        host: "dock1".into(),
        container: "shop-db".into(),
        container_id: "0123".into(),
        address: "172.18.0.5".into(),
        port: 3306,
    };
    update(&mut s, Msg::Db(DbMsg::Open(t)), Instant::now());
    s.model.db[0].current = Some(("shop".into(), String::new()));
    let l = layout(&s.model);
    assert!(l.contains(&db_token("shop-db")), "{l}");
}

fn unsaved_editor(s: &mut State) {
    update(s, Msg::Files(FmMsg::Open("web1".into())), Instant::now());
    connected(s, "web1");
    let id = s.model.files[0].id;
    update(
        s,
        Msg::Files(FmMsg::Editor {
            id,
            msg: crate::editor::EdMsg::OpenPath {
                side: Side::Remote,
                path: "/etc/hosts".into(),
            },
        }),
        Instant::now(),
    );
    let tab = s.model.files[0].editor.tabs[0].id;
    update(
        s,
        Msg::Files(FmMsg::Editor {
            id,
            msg: crate::editor::EdMsg::Loaded {
                tab,
                load: crate::editor::Load::Open,
                result: Ok((
                    telepad_core::files::text::Doc {
                        text: "127.0.0.1 localhost
"
                        .into(),
                        ..Default::default()
                    },
                    telepad_core::files::edit::Stamp {
                        size: 20,
                        mtime: Some(1),
                        nanos: 0,
                    },
                )),
            },
        }),
        Instant::now(),
    );
    let d = s.drafts.files.get_mut(&id).unwrap();
    d.editor.texts.insert(tab, "changed".into());
}

#[test]
fn leaving_with_unsaved_work_asks_and_back_keeps_everything() {
    let (mut s, _) = started("");
    unsaved_editor(&mut s);
    s.drafts.keys = Some(KeysDraft {
        text: "ssh-ed25519 AAAA test".into(),
        ..Default::default()
    });
    wk(&mut s, WkMsg::Switch(Current::Custom("b".into())));
    let ask = s.drafts.ws_leave.clone().expect("asks first");
    assert_eq!(ask.to, Some(Current::Custom("b".into())));
    let text: Vec<String> = ask.items.iter().map(|(w, x)| format!("{w}: {x}")).collect();
    assert!(
        text.iter()
            .any(|t| t.contains("Files – web1") && t.contains("hosts")),
        "{text:?}"
    );
    assert!(
        text.iter().any(|t| t.contains("Authorized keys")),
        "{text:?}"
    );
    assert_eq!(s.model.wss.current, Current::Own, "not switched yet");
    // Back: nothing changes.
    wk(&mut s, WkMsg::LeaveBack);
    assert!(s.drafts.ws_leave.is_none());
    assert_eq!(s.model.wss.current, Current::Own);
    assert_eq!(s.model.files.len(), 1);
    assert!(s.drafts.keys.is_some());
}

#[test]
fn yes_discards_and_resets_then_switches() {
    let (mut s, _) = started("");
    unsaved_editor(&mut s);
    s.drafts.keys = Some(KeysDraft {
        text: "ssh-ed25519 AAAA test".into(),
        ..Default::default()
    });
    wk(&mut s, WkMsg::Switch(Current::Custom("b".into())));
    wk(&mut s, WkMsg::LeaveYes);
    assert_eq!(s.model.wss.current, Current::Custom("b".into()));
    assert!(
        s.model.files.is_empty(),
        "unsaved text dropped, window closed"
    );
    assert!(s.drafts.keys.is_none());
    assert!(s.model.ws.is_open(Win::Docker));
    // Coming back: the editor tab is reopened from the host, the key form is empty.
    let fx = wk(&mut s, WkMsg::Switch(Current::Own));
    assert!(s.drafts.ws_leave.is_none(), "nothing to lose in b");
    assert!(
        files_fx(&fx)
            .iter()
            .any(|e| matches!(e, FmEffect::Connect { .. }))
    );
}

#[test]
fn nothing_to_lose_switches_without_asking() {
    let (mut s, _) = started("");
    wk(&mut s, WkMsg::Switch(Current::Custom("b".into())));
    assert!(s.drafts.ws_leave.is_none());
    assert_eq!(s.model.wss.current, Current::Custom("b".into()));
}

fn with_running_transfer(s: &mut State) -> u64 {
    update(s, Msg::Files(FmMsg::Open("web1".into())), Instant::now());
    connected(s, "web1");
    let id = s.model.files[0].id;
    update(
        s,
        Msg::Files(FmMsg::Listed {
            id,
            side: Side::Local,
            result: Ok((
                "/home/me".into(),
                vec![Entry {
                    name: "big.iso".into(),
                    kind: Kind::File,
                    size: 1 << 30,
                    ..Default::default()
                }],
            )),
        }),
        Instant::now(),
    );
    update(
        s,
        Msg::Files(FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["big.iso".into()],
        }),
        Instant::now(),
    );
    assert_eq!(s.model.files[0].running_transfers(), 1);
    id
}

#[test]
fn running_transfers_keep_going_in_the_background_and_show_again() {
    let (mut s, _) = started("");
    let id = with_running_transfer(&mut s);
    let fx = wk(&mut s, WkMsg::Switch(Current::Custom("b".into())));
    assert!(
        s.drafts.ws_leave.is_none(),
        "a running transfer is not a loss"
    );
    assert!(
        !files_fx(&fx)
            .iter()
            .any(|e| matches!(e, FmEffect::Cancel { .. } | FmEffect::Close(_))),
        "not cancelled"
    );
    assert!(s.model.files[0].background);
    assert!(!s.model.ws.is_open(Win::Files(id)), "no window in b");
    assert_eq!(background_transfers(&s.model), 1);
    assert!(s.model.message.as_ref().unwrap().0.contains("background"));
    // Opening the host's files in b shows the same window with its transfer.
    update(
        &mut s,
        Msg::Files(FmMsg::Open("web1".into())),
        Instant::now(),
    );
    assert!(!s.model.files[0].background);
    assert!(s.model.ws.is_open(Win::Files(id)));
    assert_eq!(s.model.files[0].running_transfers(), 1);
}

#[test]
fn a_background_window_closes_when_its_transfers_end() {
    let (mut s, _) = started("");
    let id = with_running_transfer(&mut s);
    wk(&mut s, WkMsg::Switch(Current::Custom("b".into())));
    let transfer = s.model.files[0].transfers[0].id;
    let fx = update(
        &mut s,
        Msg::Files(FmMsg::Finished {
            id,
            transfer,
            result: Ok((1, 0, 0)),
        }),
        Instant::now(),
    );
    assert!(s.model.files.is_empty());
    assert!(
        files_fx(&fx)
            .iter()
            .any(|e| matches!(e, FmEffect::Close(i) if *i == id))
    );
    assert!(s.model.message.as_ref().unwrap().0.contains("finished"));
}

#[test]
fn quitting_asks_when_work_would_be_lost_and_counts_transfers() {
    let (mut s, _) = started("");
    wk(&mut s, WkMsg::AskQuit);
    assert!(s.model.quit, "nothing to lose: closes");
    let (mut s, _) = started("");
    with_running_transfer(&mut s);
    wk(&mut s, WkMsg::AskQuit);
    assert!(!s.model.quit);
    let ask = s.drafts.ws_leave.clone().unwrap();
    assert_eq!(ask.to, None);
    assert!(ask.items.iter().any(|(_, w)| w.contains("cancelled")));
    wk(&mut s, WkMsg::LeaveYes);
    assert!(s.model.quit);
}

#[test]
fn deleting_the_current_workspace_with_unsaved_work_asks_first() {
    let (mut s, _) = started("");
    wk(&mut s, WkMsg::Switch(Current::Custom("b".into())));
    unsaved_editor(&mut s);
    wk(&mut s, WkMsg::Delete("b".into()));
    assert!(s.drafts.ws_leave.is_some());
    assert_eq!(s.model.wss.custom.len(), 1, "not deleted yet");
    wk(&mut s, WkMsg::LeaveYes);
    assert_eq!(s.model.wss.current, Current::Own);
    assert!(s.model.wss.custom.is_empty());
}
