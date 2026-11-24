//! Behaviour tests for the file windows (#62).

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::files::transfer::{Decision, Direction, Event, Item};
use telepad_core::files::{Entry, Kind, local_join};

use crate::files::*;
use crate::{Effect, Msg, State, update};

fn state() -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![HostEntry {
                id: Some(1),
                hostname: "192.0.2.10".into(),
                ..HostEntry::new("web1")
            }]),
            ssh_config_error: None,
        },
        Instant::now(),
    );
    s
}

/// Effects of one message, without the history's (tested in `tests_history`).
fn send(s: &mut State, m: FmMsg) -> Vec<FmEffect> {
    update(s, Msg::Files(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::Files(FmEffect::Log(_) | FmEffect::LoadHistory { .. }) => None,
            Effect::Files(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn fs(s: &State) -> &FileSession {
    &s.model.files[0]
}

fn draft(s: &mut State) -> &mut FileDraft {
    let id = s.model.files[0].id;
    s.drafts.files.get_mut(&id).unwrap()
}

fn entry(name: &str, dir: bool, size: u64, mtime: i64) -> Entry {
    Entry {
        name: name.into(),
        kind: if dir { Kind::Dir } else { Kind::File },
        size,
        mtime: Some(mtime),
        hidden: name.starts_with('.'),
        ..Default::default()
    }
}

/// Window open and both panes listed.
fn opened(mode: Mode) -> (State, FmId) {
    let mut s = state();
    let fx = send(&mut s, FmMsg::Open("web1".into()));
    assert!(matches!(&fx[..], [FmEffect::Connect { host, .. }] if host.alias == "web1"));
    assert!(
        send(&mut s, FmMsg::Open("web1".into())).is_empty(),
        "one window per host"
    );
    let id = fs(&s).id;
    let fx = send(
        &mut s,
        FmMsg::Connected {
            id,
            result: Ok((mode, "/home/ops".into(), None)),
            local_home: "/home/me".into(),
        },
    );
    let lists: Vec<(Side, String)> = fx
        .iter()
        .filter_map(|e| match e {
            FmEffect::List { side, path, .. } => Some((*side, path.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        lists,
        vec![
            (Side::Local, "/home/me".into()),
            (Side::Remote, "/home/ops".into())
        ]
    );
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Local,
            result: Ok((
                "/home/me".into(),
                vec![
                    entry("site", true, 0, 1),
                    entry("notes.txt", false, 5, 100),
                    entry(".bashrc", false, 1, 1),
                ],
            )),
        },
    );
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok((
                "/home/ops".into(),
                vec![entry("notes.txt", false, 9, 50), entry("logs", true, 0, 1)],
            )),
        },
    );
    (s, id)
}

fn item(label: &str, dir: bool, size: u64, exists: bool) -> Item {
    Item {
        src: format!("/home/me/{label}"),
        dst: format!("/home/ops/{label}"),
        label: label.into(),
        is_dir: dir,
        size,
        mtime: Some(10),
        perm: None,
        exists: exists.then_some((1, Some(5))),
    }
}

#[test]
fn opens_lists_navigates_and_remembers_folders() {
    let (mut s, id) = opened(Mode::Sftp);
    let local = fs(&s).local.sorted(false, "");
    assert_eq!(
        local.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["site", "notes.txt"]
    );
    assert_eq!(
        fs(&s).local.sorted(true, "").len(),
        3,
        "hidden files on request"
    );
    assert_eq!(
        fs(&s).local.sorted(true, "NOTE").len(),
        1,
        "filter ignores case"
    );
    send(
        &mut s,
        FmMsg::Sort {
            id,
            side: Side::Local,
            col: SortCol::Size,
        },
    );
    send(
        &mut s,
        FmMsg::Sort {
            id,
            side: Side::Local,
            col: SortCol::Size,
        },
    );
    assert!(fs(&s).local.descending);
    assert_eq!(
        fs(&s).local.sorted(false, "")[0].name,
        "site",
        "folders stay first"
    );

    let path = |fx: &[FmEffect]| match fx {
        [FmEffect::List { path, .. }] => path.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        path(&send(
            &mut s,
            FmMsg::Enter {
                id,
                side: Side::Remote,
                name: "logs".into()
            }
        )),
        "/home/ops/logs"
    );
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok(("/home/ops/logs".into(), vec![])),
        },
    );
    assert_eq!(
        path(&send(
            &mut s,
            FmMsg::Up {
                id,
                side: Side::Remote
            }
        )),
        "/home/ops"
    );
    assert_eq!(
        path(&send(
            &mut s,
            FmMsg::Home {
                id,
                side: Side::Remote
            }
        )),
        "/home/ops"
    );
    // A folder that cannot be opened keeps the last good one.
    send(
        &mut s,
        FmMsg::Navigate {
            id,
            side: Side::Remote,
            path: " /root ".into(),
        },
    );
    assert_eq!(draft(&mut s).remote_path, "/root");
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Err("Permission denied".into()),
        },
    );
    assert_eq!(fs(&s).remote.path, "/home/ops/logs");
    assert_eq!(draft(&mut s).remote_path, "/home/ops/logs");
    assert!(fs(&s).remote.error.is_some());

    let fx = send(&mut s, FmMsg::Close(id));
    assert!(
        matches!(&fx[0], FmEffect::SavePaths { alias, remote, .. } if alias == "web1" && remote == "/home/ops/logs")
    );
    assert!(matches!(fx.last(), Some(FmEffect::Close(_))));
    assert!(s.model.files.is_empty() && s.drafts.files.is_empty());
}

#[test]
fn upload_asks_about_existing_files_then_copies() {
    let (mut s, id) = opened(Mode::Sftp);
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into(), "notes.txt".into()],
        },
    );
    let tid = match &fx[..] {
        [
            FmEffect::Plan {
                transfer,
                direction: Direction::Upload,
                sources,
                dest,
                ..
            },
        ] => {
            assert_eq!(dest, "/home/ops");
            assert_eq!(
                sources,
                &vec![
                    (local_join("/home/me", "site"), true),
                    (local_join("/home/me", "notes.txt"), false)
                ]
            );
            *transfer
        }
        other => panic!("{other:?}"),
    };
    let items = vec![
        item("site/", true, 0, false),
        item("site/a.txt", false, 100, true),
        item("site/b.txt", false, 200, true),
        item("notes.txt", false, 5, true),
    ];
    send(
        &mut s,
        FmMsg::Planned {
            id,
            transfer: tid,
            result: Ok(items),
        },
    );
    assert_eq!(fs(&s).transfer(tid).unwrap().state, TState::Asking);
    assert_eq!(fs(&s).ask.as_ref().unwrap().conflicts, vec![1, 2, 3]);
    assert!(
        send(
            &mut s,
            FmMsg::Decide {
                id,
                decision: Decision::Skip
            }
        )
        .is_empty()
    );
    assert_eq!(fs(&s).ask.as_ref().unwrap().at, 1);
    draft(&mut s).apply_all = true;
    let fx = send(
        &mut s,
        FmMsg::Decide {
            id,
            decision: Decision::Overwrite,
        },
    );
    match &fx[..] {
        [
            FmEffect::Run {
                decisions, items, ..
            },
        ] => {
            assert_eq!(decisions.get(&1), Some(&Decision::Skip));
            assert_eq!(decisions.get(&2), Some(&Decision::Overwrite));
            assert_eq!(decisions.get(&3), Some(&Decision::Overwrite));
            assert_eq!(items.len(), 4);
        }
        other => panic!("{other:?}"),
    }
    assert!(fs(&s).ask.is_none());
    assert!(!draft(&mut s).apply_all, "Apply to all is per question");
    let t = fs(&s).transfer(tid).unwrap();
    assert_eq!(
        (t.state.clone(), t.bytes_total),
        (TState::Running, 205),
        "skipped file not counted"
    );

    send(
        &mut s,
        FmMsg::Progress {
            id,
            transfer: tid,
            event: Event::Start(2),
        },
    );
    send(
        &mut s,
        FmMsg::Progress {
            id,
            transfer: tid,
            event: Event::Bytes(200),
        },
    );
    send(
        &mut s,
        FmMsg::Progress {
            id,
            transfer: tid,
            event: Event::Done(3, Err("Permission denied".into())),
        },
    );
    assert_eq!(fs(&s).transfer(tid).unwrap().current, "site/b.txt");
    assert_eq!(
        fs(&s).transfer(tid).unwrap().errors,
        vec!["notes.txt: Permission denied".to_string()]
    );
    let fx = send(
        &mut s,
        FmMsg::Finished {
            id,
            transfer: tid,
            result: Ok((2, 1, 1)),
        },
    );
    assert!(
        matches!(
            &fx[..],
            [FmEffect::List {
                side: Side::Remote,
                ..
            }]
        ),
        "target pane refreshed"
    );
    assert_eq!(
        fs(&s).transfer(tid).unwrap().state,
        TState::Done {
            copied: 2,
            skipped: 1,
            failed: 1
        }
    );
    send(&mut s, FmMsg::ClearFinished(id));
    assert!(fs(&s).transfers.is_empty());
}

#[test]
fn download_without_conflicts_runs_and_can_be_cancelled() {
    let (mut s, id) = opened(Mode::Sftp);
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Remote,
            names: vec!["logs".into()],
        },
    );
    let tid = match &fx[..] {
        [
            FmEffect::Plan {
                transfer,
                direction: Direction::Download,
                dest,
                ..
            },
        ] => {
            assert_eq!(dest, "/home/me");
            *transfer
        }
        other => panic!("{other:?}"),
    };
    assert!(matches!(
        &send(&mut s, FmMsg::Cancel { id, transfer: tid })[..],
        [FmEffect::Cancel { .. }]
    ));
    let fx = send(
        &mut s,
        FmMsg::Planned {
            id,
            transfer: tid,
            result: Ok(vec![item("logs/", true, 0, false)]),
        },
    );
    assert!(matches!(
        &fx[..],
        [FmEffect::Run {
            direction: Direction::Download,
            ..
        }]
    ));
    send(
        &mut s,
        FmMsg::Finished {
            id,
            transfer: tid,
            result: Err("Cancelled".into()),
        },
    );
    assert_eq!(fs(&s).transfer(tid).unwrap().state, TState::Cancelled);
    assert!(
        send(
            &mut s,
            FmMsg::Copy {
                id,
                from: Side::Remote,
                names: vec![]
            }
        )
        .is_empty()
    );
    // Asking can be cancelled without the runtime.
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["notes.txt".into()],
        },
    );
    let FmEffect::Plan { transfer, .. } = &fx[0] else {
        panic!()
    };
    let t2 = *transfer;
    send(
        &mut s,
        FmMsg::Planned {
            id,
            transfer: t2,
            result: Ok(vec![item("notes.txt", false, 5, true)]),
        },
    );
    send(&mut s, FmMsg::AskCancel(id));
    assert_eq!(fs(&s).transfer(t2).unwrap().state, TState::Cancelled);
    assert!(fs(&s).ask.is_none());
}

#[test]
fn hosts_without_sftp_copy_with_scp_after_a_name_check() {
    let (mut s, id) = opened(Mode::Fallback);
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["notes.txt".into(), "site".into()],
        },
    );
    assert!(fx.is_empty(), "notes.txt exists on the host: ask first");
    let tid = fs(&s).transfers[0].id;
    assert_eq!(fs(&s).ask.as_ref().unwrap().conflicts, vec![0]);
    let fx = send(
        &mut s,
        FmMsg::Decide {
            id,
            decision: Decision::NewerOnly,
        },
    );
    match &fx[..] {
        [
            FmEffect::Scp {
                sources,
                dest,
                direction: Direction::Upload,
                ..
            },
        ] => {
            // notes.txt (local 100) is newer than the host's (50): copied.
            assert_eq!(
                sources,
                &vec![
                    (local_join("/home/me", "notes.txt"), false),
                    (local_join("/home/me", "site"), true)
                ]
            );
            assert_eq!(dest, "/home/ops");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fs(&s).transfer(tid).unwrap().state, TState::Running);
}

#[test]
fn rsync_previews_before_it_runs() {
    let (mut s, id) = opened(Mode::Sftp);
    send(&mut s, FmMsg::RsyncChecked { id, result: Ok(()) });
    draft(&mut s).use_rsync = true;
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into()],
        },
    );
    let tid = match &fx[..] {
        [
            FmEffect::Rsync {
                transfer,
                dry_run: true,
                sources,
                dest,
                ..
            },
        ] => {
            assert_eq!(
                (sources.clone(), dest.as_str()),
                (vec![local_join("/home/me", "site")], "/home/ops")
            );
            *transfer
        }
        other => panic!("{other:?}"),
    };
    send(
        &mut s,
        FmMsg::RsyncPreviewed {
            id,
            transfer: tid,
            result: Ok(vec!["site/index.html".into()]),
        },
    );
    assert_eq!(
        fs(&s).transfer(tid).unwrap().state,
        TState::Previewed(vec!["site/index.html".into()])
    );
    let fx = send(&mut s, FmMsg::RsyncRun { id, transfer: tid });
    assert!(matches!(&fx[..], [FmEffect::Rsync { dry_run: false, .. }]));
    assert!(
        send(&mut s, FmMsg::RsyncRun { id, transfer: tid }).is_empty(),
        "runs once"
    );

    // Not available: the switch has no effect.
    send(
        &mut s,
        FmMsg::RsyncChecked {
            id,
            result: Err("rsync is not installed on this computer".into()),
        },
    );
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into()],
        },
    );
    assert!(matches!(&fx[..], [FmEffect::Plan { .. }]));
}

#[test]
fn connection_errors_can_be_retried() {
    let mut s = state();
    send(&mut s, FmMsg::Open("web1".into()));
    let id = fs(&s).id;
    send(
        &mut s,
        FmMsg::Connected {
            id,
            result: Err("Permission denied (publickey)".into()),
            local_home: "/home/me".into(),
        },
    );
    assert!(fs(&s).error.is_some());
    let fx = send(&mut s, FmMsg::Retry(id));
    assert!(matches!(
        &fx[..],
        [FmEffect::Close(_), FmEffect::Connect { .. }]
    ));
    assert!(
        send(
            &mut s,
            FmMsg::Copy {
                id,
                from: Side::Local,
                names: vec!["x".into()]
            }
        )
        .is_empty()
    );
    let fx = send(&mut s, FmMsg::Close(id));
    assert!(
        matches!(&fx[..], [FmEffect::Close(_)]),
        "nothing to remember before it connected"
    );
}

// --- File operations (#63) ---

fn select(s: &mut State, side: Side, names: &[&str]) {
    let sel = draft(s).selected_mut(side);
    sel.clear();
    sel.extend(names.iter().map(|n| n.to_string()));
}

fn ops_of(fx: &[FmEffect]) -> Vec<(Side, FileOp)> {
    fx.iter()
        .filter_map(|e| match e {
            FmEffect::Op { side, op, .. } => Some((*side, op.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn new_folder_checks_the_name_then_creates_and_selects_it() {
    let (mut s, id) = opened(Mode::Sftp);
    let side = Side::Remote;
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side,
            op: OpKind::NewFolder,
        },
    );
    assert_eq!(fs(&s).dialog.as_ref().unwrap().kind, DialogKind::NewFolder);
    // A second dialog cannot open on top.
    assert!(op_blocked(fs(&s), 0, Side::Local, OpKind::NewFile).is_some());

    for bad in ["", "..", "a/b"] {
        draft(&mut s).name = bad.into();
        assert!(
            ops_of(&send(&mut s, FmMsg::Confirm(id))).is_empty(),
            "{bad:?}"
        );
        assert!(fs(&s).dialog.as_ref().unwrap().error.is_some());
    }
    draft(&mut s).name = "new 'dir' ü".into();
    let fx = send(&mut s, FmMsg::Confirm(id));
    assert_eq!(
        ops_of(&fx),
        vec![(
            side,
            FileOp::NewFolder {
                dir: "/home/ops".into(),
                name: "new 'dir' ü".into()
            }
        )]
    );
    assert!(fs(&s).dialog.as_ref().unwrap().busy);
    assert!(
        ops_of(&send(&mut s, FmMsg::Confirm(id))).is_empty(),
        "no double submit"
    );

    // The host says no: the dialog stays with the reason.
    send(
        &mut s,
        FmMsg::OpDone {
            id,
            side,
            result: Err("new 'dir' ü already exists".into()),
        },
    );
    let dl = fs(&s).dialog.as_ref().unwrap();
    assert!(!dl.busy && dl.error.as_deref().unwrap().contains("exists"));

    send(&mut s, FmMsg::Confirm(id));
    let fx = send(
        &mut s,
        FmMsg::OpDone {
            id,
            side,
            result: Ok((Default::default(), Some("new 'dir' ü".into()))),
        },
    );
    assert!(fs(&s).dialog.is_none());
    assert!(fx.iter().any(|e| matches!(
        e,
        FmEffect::List {
            side: Side::Remote,
            ..
        }
    )));
    assert!(draft(&mut s).selected_remote.contains("new 'dir' ü"));
}

#[test]
fn rename_needs_one_item_and_keeps_an_unchanged_name() {
    let (mut s, id) = opened(Mode::Sftp);
    let side = Side::Local;
    select(&mut s, side, &["site", "notes.txt"]);
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side,
            op: OpKind::Rename,
        },
    );
    assert!(fs(&s).dialog.is_none(), "two selected");
    select(&mut s, side, &["notes.txt"]);
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side,
            op: OpKind::Rename,
        },
    );
    assert_eq!(draft(&mut s).name, "notes.txt");
    assert!(ops_of(&send(&mut s, FmMsg::Confirm(id))).is_empty());
    assert!(fs(&s).dialog.is_none(), "same name: nothing to do");

    send(
        &mut s,
        FmMsg::Ask {
            id,
            side,
            op: OpKind::Rename,
        },
    );
    draft(&mut s).name = "notes 2.txt".into();
    assert_eq!(
        ops_of(&send(&mut s, FmMsg::Confirm(id))),
        vec![(
            side,
            FileOp::Rename {
                dir: "/home/me".into(),
                from: "notes.txt".into(),
                to: "notes 2.txt".into()
            }
        )]
    );
}

#[test]
fn delete_counts_folders_first_and_reports_per_item_errors() {
    use telepad_core::files::ops::{Measure, Report};
    let (mut s, id) = opened(Mode::Sftp);
    let side = Side::Local;
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side,
            op: OpKind::Delete,
        },
    );
    assert!(fs(&s).dialog.is_none(), "nothing selected");

    select(&mut s, side, &["site", "notes.txt"]);
    let fx = send(
        &mut s,
        FmMsg::Ask {
            id,
            side,
            op: OpKind::Delete,
        },
    );
    assert!(
        matches!(&ops_of(&fx)[..], [(Side::Local, FileOp::Measure { items, .. })] if items.len() == 2)
    );
    let counted = vec![
        Measure {
            files: 12,
            dirs: 3,
            bytes: 4096,
            partial: false,
        },
        Measure {
            files: 1,
            bytes: 5,
            ..Default::default()
        },
    ];
    send(
        &mut s,
        FmMsg::Measured {
            id,
            side,
            result: Ok(counted.clone()),
        },
    );
    match &fs(&s).dialog.as_ref().unwrap().kind {
        DialogKind::Delete { items, measure } => {
            assert_eq!(items.len(), 2);
            assert_eq!(measure, &Some(Ok(counted)));
        }
        k => panic!("{k:?}"),
    }
    let fx = send(&mut s, FmMsg::Confirm(id));
    assert!(matches!(
        &ops_of(&fx)[..],
        [(Side::Local, FileOp::Delete { dir, items })] if dir == "/home/me" && items.len() == 2
    ));

    let fx = send(
        &mut s,
        FmMsg::OpDone {
            id,
            side,
            result: Ok((
                Report {
                    done: 1,
                    errors: vec!["/home/me/site/x: Permission denied".into()],
                },
                None,
            )),
        },
    );
    assert!(fs(&s).dialog.is_none());
    assert_eq!(fs(&s).op_errors.len(), 1);
    assert!(draft(&mut s).selected_local.is_empty());
    assert!(fx.iter().any(|e| matches!(
        e,
        FmEffect::List {
            side: Side::Local,
            ..
        }
    )));
    send(&mut s, FmMsg::DismissErrors(id));
    assert!(fs(&s).op_errors.is_empty());
}

#[test]
fn files_only_delete_needs_no_counting() {
    let (mut s, id) = opened(Mode::Sftp);
    select(&mut s, Side::Remote, &["notes.txt"]);
    let fx = send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Remote,
            op: OpKind::Delete,
        },
    );
    assert!(ops_of(&fx).is_empty());
    assert!(matches!(
        &fs(&s).dialog.as_ref().unwrap().kind,
        DialogKind::Delete { measure: Some(Ok(m)), .. } if m[0].bytes == 9
    ));
}

#[test]
fn permissions_are_host_side_octal_and_optionally_recursive() {
    let (mut s, id) = opened(Mode::Sftp);
    select(&mut s, Side::Local, &["notes.txt"]);
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Local,
            op: OpKind::Permissions,
        },
    );
    assert!(
        fs(&s).dialog.is_none(),
        "this computer has no permissions dialog"
    );

    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok((
                "/home/ops".into(),
                vec![Entry {
                    perm: Some(0o755),
                    ..entry("logs", true, 0, 1)
                }],
            )),
        },
    );
    select(&mut s, Side::Remote, &["logs"]);
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Remote,
            op: OpKind::Permissions,
        },
    );
    assert_eq!(draft(&mut s).mode_text, "755");
    draft(&mut s).mode_text = "abc".into();
    assert!(ops_of(&send(&mut s, FmMsg::Confirm(id))).is_empty());
    assert!(fs(&s).dialog.as_ref().unwrap().error.is_some());
    draft(&mut s).mode_text = "0750".into();
    draft(&mut s).recursive = true;
    assert!(matches!(
        &ops_of(&send(&mut s, FmMsg::Confirm(id)))[..],
        [(Side::Remote, FileOp::Chmod { mode: 0o750, recursive: true, items, .. })]
            if items[0].name == "logs"
    ));
    assert_eq!(parse_mode("644"), Some(0o644));
    assert_eq!(parse_mode("2775"), Some(0o2775));
    assert_eq!(parse_mode("8"), None);
    assert_eq!(parse_mode("17777"), None);
}

#[test]
fn hosts_without_sftp_keep_their_side_read_only() {
    let (mut s, id) = opened(Mode::Fallback);
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Remote,
            op: OpKind::NewFolder,
        },
    );
    assert!(fs(&s).dialog.is_none());
    assert_eq!(
        op_blocked(fs(&s), 0, Side::Remote, OpKind::NewFolder),
        Some("Needs SFTP on the host")
    );
    send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Local,
            op: OpKind::NewFolder,
        },
    );
    assert!(fs(&s).dialog.is_some(), "this computer still works");
}

#[test]
fn copy_path_and_properties() {
    let (mut s, id) = opened(Mode::Sftp);
    let fx = send(
        &mut s,
        FmMsg::CopyPath {
            id,
            side: Side::Remote,
        },
    );
    assert!(matches!(&fx[..], [FmEffect::CopyText(t)] if t == "/home/ops"));
    select(&mut s, Side::Remote, &["logs", "notes.txt"]);
    let fx = send(
        &mut s,
        FmMsg::CopyPath {
            id,
            side: Side::Remote,
        },
    );
    assert!(
        matches!(&fx[..], [FmEffect::CopyText(t)] if t == "/home/ops/logs\n/home/ops/notes.txt")
    );

    select(&mut s, Side::Remote, &["notes.txt"]);
    let fx = send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Remote,
            op: OpKind::Properties,
        },
    );
    assert!(ops_of(&fx).is_empty());
    assert!(matches!(
        &fs(&s).dialog.as_ref().unwrap().kind,
        DialogKind::Properties { path, measure: Some(Ok(_)), .. } if path == "/home/ops/notes.txt"
    ));
    send(&mut s, FmMsg::Confirm(id));
    assert!(fs(&s).dialog.is_none());

    select(&mut s, Side::Remote, &["logs"]);
    let fx = send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Remote,
            op: OpKind::Properties,
        },
    );
    assert!(matches!(
        &ops_of(&fx)[..],
        [(Side::Remote, FileOp::Measure { .. })]
    ));
    send(&mut s, FmMsg::CloseDialog(id));
    // A late count for a closed dialog changes nothing.
    send(
        &mut s,
        FmMsg::Measured {
            id,
            side: Side::Remote,
            result: Ok(vec![]),
        },
    );
    assert!(fs(&s).dialog.is_none());
}

#[test]
fn row_clicks_select_like_a_file_manager() {
    // #130: plain click, Ctrl toggles, Shift selects a range from the last
    // clicked row; the `..` row is highlighted alone and leaves on navigation.
    let mut d = FileDraft::default();
    let order = ["a", "b", "c", "d"];
    let sel = |d: &FileDraft| d.selected(Side::Local).iter().cloned().collect::<Vec<_>>();
    d.click_row(Side::Local, "b", &order, false, false);
    assert_eq!(sel(&d), ["b"]);
    d.click_row(Side::Local, "d", &order, false, true);
    assert_eq!(sel(&d), ["b", "c", "d"]);
    d.click_row(Side::Local, "c", &order, true, false);
    assert_eq!(sel(&d), ["b", "d"]);
    d.click_row(Side::Local, "a", &order, true, true);
    assert_eq!(sel(&d), ["a", "b", "c", "d"]);
    d.click_parent(Side::Local);
    assert!(sel(&d).is_empty());
    assert!(d.parent_selected.contains(&Side::Local));
    d.click_row(Side::Local, "a", &order, false, false);
    assert!(!d.parent_selected.contains(&Side::Local));
    // Shift without an earlier click selects just the row.
    d.click_row(Side::Remote, "c", &order, false, true);
    assert_eq!(d.selected(Side::Remote).len(), 1);

    let (mut s, id) = opened(Mode::Sftp);
    draft(&mut s).click_parent(Side::Local);
    send(
        &mut s,
        FmMsg::Up {
            id,
            side: Side::Local,
        },
    );
    assert!(draft(&mut s).parent_selected.is_empty());
}

#[test]
fn transfer_settings_are_saved_and_used_by_copies() {
    // #131: mode, new and updated only, excluded folders.
    use telepad_core::files::transfer::{Options, TransferMode};
    let (mut s, id) = opened(Mode::Sftp);
    send(
        &mut s,
        FmMsg::OptionsLoaded {
            id,
            opts: Options {
                mode: TransferMode::Binary,
                ..Default::default()
            },
        },
    );
    send(&mut s, FmMsg::OptionsOpen(id));
    assert!(draft(&mut s).show_opts);
    assert_eq!(draft(&mut s).opts_form.mode, TransferMode::Binary);
    draft(&mut s).opts_form.mode = TransferMode::Text;
    draft(&mut s).opts_form.newer_only = true;
    draft(&mut s).opts_exclude = "site; node_modules".into();
    let fx = send(&mut s, FmMsg::OptionsSave(id));
    let want = Options {
        mode: TransferMode::Text,
        newer_only: true,
        exclude: vec!["site".into(), "node_modules".into()],
    };
    assert!(
        matches!(&fx[..], [FmEffect::SaveOptions { alias, opts }] if alias == "web1" && *opts == want)
    );
    assert!(!draft(&mut s).show_opts);
    // An excluded folder is not copied, even when picked.
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into(), "notes.txt".into()],
        },
    );
    match &fx[..] {
        [FmEffect::Plan { sources, opts, .. }] => {
            assert_eq!(sources.len(), 1, "{sources:?}");
            assert!(sources[0].0.ends_with("notes.txt"));
            assert_eq!(*opts, want);
        }
        other => panic!("{other:?}"),
    }
    // New and updated only: existing files are decided, no question.
    let tid = fs(&s).transfers[0].id;
    let fx = send(
        &mut s,
        FmMsg::Planned {
            id,
            transfer: tid,
            result: Ok(vec![item("notes.txt", false, 5, true)]),
        },
    );
    assert!(fs(&s).ask.is_none());
    match &fx[..] {
        [
            FmEffect::Run {
                decisions, opts, ..
            },
        ] => {
            assert_eq!(decisions.get(&0), Some(&Decision::NewerOnly));
            assert_eq!(opts.mode, TransferMode::Text);
        }
        other => panic!("{other:?}"),
    }
    // Everything excluded: nothing starts, the window says why.
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into()],
        },
    );
    assert!(fx.is_empty() && fs(&s).notice.is_some());
}

#[test]
fn synchronize_previews_then_copies_both_ways_and_deletes() {
    use telepad_core::files::compare::{Gone, SyncDir, SyncPlan, SyncRequest};
    let (mut s, id) = opened(Mode::Sftp);
    send(&mut s, FmMsg::SyncOpen(id));
    let v = fs(&s).sync.clone().unwrap();
    assert_eq!(
        (v.left.as_str(), v.right.as_str()),
        ("/home/me", "/home/ops")
    );
    let req = SyncRequest {
        dir: SyncDir::LeftToRight,
        recursive: true,
        delete: true,
    };
    draft(&mut s).sync = req;
    let fx = send(&mut s, FmMsg::SyncPreview(id));
    assert!(
        matches!(&fx[..], [FmEffect::SyncPlan { req: r, left, right, .. }]
        if *r == req && left == "/home/me" && right == "/home/ops")
    );
    assert!(fs(&s).sync.as_ref().unwrap().busy);
    assert!(
        send(&mut s, FmMsg::SyncRun(id)).is_empty(),
        "nothing runs before the preview"
    );
    assert!(fs(&s).sync.is_some());
    let plan = SyncPlan {
        upload: vec![
            item("site/", true, 0, false),
            item("site/a.txt", false, 3, true),
        ],
        download: vec![],
        delete_left: vec![],
        delete_right: vec![Gone {
            dir: "/home/ops".into(),
            entry: entry("logs", true, 0, 1),
            label: "logs/".into(),
        }],
    };
    send(
        &mut s,
        FmMsg::SyncPlanned {
            id,
            result: Ok(plan),
        },
    );
    let fx = update(&mut s, Msg::Files(FmMsg::SyncRun(id)), Instant::now());
    let files: Vec<&FmEffect> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Files(f) => Some(f),
            _ => None,
        })
        .collect();
    assert!(
        files
            .iter()
            .any(|e| matches!(e, FmEffect::Run { items, decisions, .. }
        if items.len() == 2 && decisions.get(&1) == Some(&Decision::Overwrite)))
    );
    assert!(files.iter().any(
        |e| matches!(e, FmEffect::Op { side: Side::Remote, op: FileOp::Delete { dir, items }, .. }
        if dir == "/home/ops" && items[0].name == "logs")
    ));
    assert!(
        files
            .iter()
            .any(|e| matches!(e, FmEffect::Log(l) if l.title.contains("Synchronize: delete"))),
        "the delete is in the history"
    );
    assert!(fs(&s).sync.is_none());
    assert_eq!(fs(&s).transfers.len(), 1);
    assert_eq!(fs(&s).transfers[0].state, TState::Running);
}

#[test]
fn synchronize_needs_sftp() {
    let (mut s, id) = opened(Mode::Fallback);
    send(&mut s, FmMsg::SyncOpen(id));
    assert!(fs(&s).sync.is_none());
    assert!(fs(&s).notice.as_deref().unwrap().contains("SFTP"));
}

#[test]
fn diff_opens_both_files_side_by_side_in_the_editor() {
    let (mut s, id) = opened(Mode::Sftp);
    // One file selected, the same name in the other pane.
    select(&mut s, Side::Local, &["notes.txt"]);
    let fx = send(&mut s, FmMsg::Diff(id));
    let loads: Vec<(Side, String)> = fx
        .iter()
        .filter_map(|e| match e {
            FmEffect::Load { side, path, .. } => Some((*side, path.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        loads,
        [
            (Side::Local, local_join("/home/me", "notes.txt")),
            (Side::Remote, "/home/ops/notes.txt".into())
        ]
    );
    let (a, b) = fs(&s).editor.compare.unwrap();
    assert_eq!(fs(&s).editor.active, Some(a));
    // Closing one of them leaves the compare; the other tab stays.
    send(
        &mut s,
        FmMsg::Editor {
            id,
            msg: crate::editor::EdMsg::Close(b),
        },
    );
    assert!(fs(&s).editor.compare.is_none());
    // A folder, or nothing to pair with: say how to use it.
    select(&mut s, Side::Local, &["site"]);
    send(&mut s, FmMsg::Diff(id));
    assert!(
        fs(&s)
            .notice
            .as_deref()
            .unwrap()
            .contains("select one file")
    );
}
