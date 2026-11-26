//! Behaviour tests for the editor of the file windows (#64).

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::files::edit::{SaveError, Stamp};
use telepad_core::files::syntax::Lang;
use telepad_core::files::text::{Doc, MAX_EDIT};
use telepad_core::files::{Entry, Kind};

use crate::editor::*;
use crate::files::*;
use crate::{Effect, Msg, State, update};

fn send(s: &mut State, m: FmMsg) -> Vec<FmEffect> {
    update(s, Msg::Files(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::Files(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn ed(s: &mut State, id: FmId, msg: EdMsg) -> Vec<FmEffect> {
    send(s, FmMsg::Editor { id, msg })
}

fn file(name: &str, size: u64) -> Entry {
    Entry {
        name: name.into(),
        kind: Kind::File,
        size,
        mtime: Some(1),
        ..Default::default()
    }
}

fn stamp(size: u64, mtime: i64) -> Stamp {
    Stamp {
        size,
        mtime: Some(mtime),
        nanos: 0,
    }
}

/// A window on `web1` (mode given) with both panes listed.
fn window(mode: Mode) -> (State, FmId) {
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
    send(&mut s, FmMsg::Open("web1".into()));
    let id = s.model.files[0].id;
    send(
        &mut s,
        FmMsg::Connected {
            id,
            result: Ok((mode, "/srv/app".into(), None)),
            local_home: "/home/me".into(),
        },
    );
    for (side, path, entries) in [
        (Side::Local, "/home/me", vec![file("notes.md", 10)]),
        (
            Side::Remote,
            "/srv/app",
            vec![
                file("docker-compose.yml", 40),
                file("dump.sql.gz", MAX_EDIT + 1),
                Entry {
                    name: "conf".into(),
                    kind: Kind::Dir,
                    ..Default::default()
                },
            ],
        ),
    ] {
        send(
            &mut s,
            FmMsg::Listed {
                id,
                side,
                result: Ok((path.into(), entries)),
            },
        );
    }
    (s, id)
}

fn editor(s: &State) -> &FileEditor {
    &s.model.files[0].editor
}

fn edraft(s: &mut State, id: FmId) -> &mut EdDraft {
    &mut s.drafts.files.get_mut(&id).unwrap().editor
}

/// Open the remote compose file and load it with `text`.
fn open_compose(s: &mut State, id: FmId, text: &str) -> TabId {
    let fx = ed(
        s,
        id,
        EdMsg::Open {
            side: Side::Remote,
            name: "docker-compose.yml".into(),
        },
    );
    let tab = editor(s).active.unwrap();
    assert!(
        matches!(&fx[..], [FmEffect::Load { path, load: Load::Open, side: Side::Remote, .. }] if path == "/srv/app/docker-compose.yml")
    );
    ed(
        s,
        id,
        EdMsg::Loaded {
            tab,
            load: Load::Open,
            result: Ok((
                Doc {
                    text: text.into(),
                    crlf: true,
                    ..Default::default()
                },
                stamp(40, 100),
            )),
        },
    );
    tab
}

#[test]
fn opens_files_as_tabs_once_each_and_refuses_big_ones() {
    let (mut s, id) = window(Mode::Sftp);
    let tab = open_compose(&mut s, id, "services:\n");
    let t = editor(&s).tab(tab).unwrap();
    assert_eq!(
        (t.state.clone(), t.lang, t.crlf),
        (TabState::Ready, Lang::Yaml, true)
    );
    assert_eq!(edraft(&mut s, id).texts[&tab], "services:\n");
    // The same file again: only switches to its tab.
    let fx = ed(
        &mut s,
        id,
        EdMsg::Open {
            side: Side::Remote,
            name: "docker-compose.yml".into(),
        },
    );
    assert!(fx.is_empty());
    assert_eq!(editor(&s).tabs.len(), 1);
    // A local file in the same window.
    let fx = ed(
        &mut s,
        id,
        EdMsg::Open {
            side: Side::Local,
            name: "notes.md".into(),
        },
    );
    assert!(matches!(
        &fx[..],
        [FmEffect::Load {
            side: Side::Local,
            ..
        }]
    ));
    assert_eq!(editor(&s).tabs.len(), 2);
    // Over 10 MiB: a tab that says why, nothing is read.
    let fx = ed(
        &mut s,
        id,
        EdMsg::Open {
            side: Side::Remote,
            name: "dump.sql.gz".into(),
        },
    );
    assert!(fx.is_empty());
    let t = editor(&s).tabs.last().unwrap();
    assert!(matches!(&t.state, TabState::Failed(w) if w.contains("10 MiB")));
    // Folders do not open; binary files fail when loaded.
    assert!(
        ed(
            &mut s,
            id,
            EdMsg::Open {
                side: Side::Remote,
                name: "conf".into()
            }
        )
        .is_empty()
    );
    let notes = editor(&s).tabs[1].id;
    ed(
        &mut s,
        id,
        EdMsg::Loaded {
            tab: notes,
            load: Load::Open,
            result: Err("Binary file".into()),
        },
    );
    assert_eq!(
        editor(&s).tab(notes).unwrap().state,
        TabState::Failed("Binary file".into())
    );
}

#[test]
fn f4_edit_needs_one_file_and_sftp_on_the_host() {
    let (mut s, id) = window(Mode::Sftp);
    let fsn = &s.model.files[0];
    assert_eq!(
        op_blocked(fsn, 0, Side::Remote, OpKind::Edit),
        Some("Select one file")
    );
    s.drafts
        .files
        .get_mut(&id)
        .unwrap()
        .selected_remote
        .insert("docker-compose.yml".into());
    let fx = send(
        &mut s,
        FmMsg::Ask {
            id,
            side: Side::Remote,
            op: OpKind::Edit,
        },
    );
    assert!(matches!(&fx[..], [FmEffect::Load { .. }]));
    assert!(s.model.files[0].dialog.is_none(), "no dialog, a tab");
    let (mut s, id) = window(Mode::Fallback);
    assert_eq!(
        op_blocked(&s.model.files[0], 1, Side::Remote, OpKind::Edit),
        Some("Needs SFTP on the host")
    );
    assert!(
        ed(
            &mut s,
            id,
            EdMsg::Open {
                side: Side::Remote,
                name: "docker-compose.yml".into()
            }
        )
        .is_empty()
    );
    assert!(editor(&s).tabs.is_empty());
    let fx = ed(
        &mut s,
        id,
        EdMsg::Open {
            side: Side::Local,
            name: "notes.md".into(),
        },
    );
    assert_eq!(fx.len(), 1, "this computer's files still open");
}

#[test]
fn dirty_state_and_save_with_line_endings_kept() {
    let (mut s, id) = window(Mode::Sftp);
    let tab = open_compose(&mut s, id, "a: 1\n");
    let t = editor(&s).tab(tab).unwrap().clone();
    assert!(!edraft(&mut s, id).dirty(&t));
    assert!(
        ed(&mut s, id, EdMsg::Save(tab)).is_empty(),
        "unchanged: not rewritten"
    );
    edraft(&mut s, id).texts.insert(tab, "a: 2\n".into());
    assert!(edraft(&mut s, id).dirty(&t));
    let fx = ed(&mut s, id, EdMsg::Save(tab));
    match &fx[..] {
        [
            FmEffect::Save {
                bytes,
                expect,
                path,
                ..
            },
        ] => {
            assert_eq!(bytes, b"a: 2\r\n", "CRLF file saved with CRLF");
            assert_eq!(*expect, Some(stamp(40, 100)));
            assert_eq!(path, "/srv/app/docker-compose.yml");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        ed(&mut s, id, EdMsg::Save(tab)).is_empty(),
        "one save at a time"
    );
    // Typing goes on while it saves; only what was written counts as saved.
    edraft(&mut s, id).texts.insert(tab, "a: 3\n".into());
    ed(
        &mut s,
        id,
        EdMsg::Saved {
            tab,
            result: Ok(stamp(6, 200)),
        },
    );
    let t = editor(&s).tab(tab).unwrap().clone();
    assert_eq!((t.saved.as_str(), t.stamp), ("a: 2\n", Some(stamp(6, 200))));
    assert!(edraft(&mut s, id).dirty(&t));
    // A failed save keeps the text and says why.
    ed(&mut s, id, EdMsg::Save(tab));
    ed(
        &mut s,
        id,
        EdMsg::Saved {
            tab,
            result: Err(SaveError::Failed("Permission denied".into())),
        },
    );
    let t = editor(&s).tab(tab).unwrap();
    assert_eq!(t.error.as_deref(), Some("Permission denied"));
    assert!(t.saving.is_none());
}

#[test]
fn save_conflict_overwrite_reload_and_diff() {
    let (mut s, id) = window(Mode::Sftp);
    let tab = open_compose(&mut s, id, "a\nb\n");
    edraft(&mut s, id).texts.insert(tab, "a\nmine\n".into());
    ed(&mut s, id, EdMsg::Save(tab));
    ed(
        &mut s,
        id,
        EdMsg::Saved {
            tab,
            result: Err(SaveError::Changed(Some(stamp(9, 300)))),
        },
    );
    let c = editor(&s).tab(tab).unwrap().conflict.clone().unwrap();
    assert_eq!(c.now, Some(stamp(9, 300)));
    // Show diff: loads the file as it is now and compares it with the text.
    let fx = ed(&mut s, id, EdMsg::ShowDiff(tab));
    assert!(matches!(
        &fx[..],
        [FmEffect::Load {
            load: Load::Diff,
            ..
        }]
    ));
    ed(
        &mut s,
        id,
        EdMsg::Loaded {
            tab,
            load: Load::Diff,
            result: Ok((
                Doc {
                    text: "a\ntheirs\n".into(),
                    ..Default::default()
                },
                stamp(9, 300),
            )),
        },
    );
    let c = editor(&s).tab(tab).unwrap().conflict.clone().unwrap();
    let lines = c.diff.unwrap().unwrap();
    assert!(lines.contains(&('-', "theirs".into())) && lines.contains(&('+', "mine".into())));
    assert_eq!(
        edraft(&mut s, id).texts[&tab],
        "a\nmine\n",
        "diff never changes the text"
    );
    // Overwrite: saves without the check.
    let fx = ed(&mut s, id, EdMsg::Overwrite(tab));
    assert!(matches!(&fx[..], [FmEffect::Save { expect: None, .. }]));
    assert!(editor(&s).tab(tab).unwrap().conflict.is_none());
    ed(
        &mut s,
        id,
        EdMsg::Saved {
            tab,
            result: Err(SaveError::Changed(None)),
        },
    );
    // Reload: the file's text replaces the edits.
    let fx = ed(&mut s, id, EdMsg::Reload(tab));
    assert!(matches!(
        &fx[..],
        [FmEffect::Load {
            load: Load::Reload,
            ..
        }]
    ));
    ed(
        &mut s,
        id,
        EdMsg::Loaded {
            tab,
            load: Load::Reload,
            result: Ok((
                Doc {
                    text: "a\ntheirs\n".into(),
                    ..Default::default()
                },
                stamp(9, 300),
            )),
        },
    );
    let t = editor(&s).tab(tab).unwrap().clone();
    assert_eq!(edraft(&mut s, id).texts[&tab], "a\ntheirs\n");
    assert!(!edraft(&mut s, id).dirty(&t));
    assert_eq!(t.stamp, Some(stamp(9, 300)));
}

#[test]
fn closing_a_tab_with_changes_asks_save_discard_cancel() {
    let (mut s, id) = window(Mode::Sftp);
    let tab = open_compose(&mut s, id, "x\n");
    // Clean: closes at once.
    ed(&mut s, id, EdMsg::Close(tab));
    assert!(editor(&s).tabs.is_empty());
    let tab = open_compose(&mut s, id, "x\n");
    edraft(&mut s, id).texts.insert(tab, "y\n".into());
    ed(&mut s, id, EdMsg::Close(tab));
    assert_eq!(editor(&s).ask, Some(Unsaved::Tab(tab)));
    ed(&mut s, id, EdMsg::Answer(Answer::Cancel));
    assert_eq!((editor(&s).ask, editor(&s).tabs.len()), (None, 1));
    // Save: closes once the save is done.
    ed(&mut s, id, EdMsg::Close(tab));
    let fx = ed(&mut s, id, EdMsg::Answer(Answer::Save));
    assert!(matches!(&fx[..], [FmEffect::Save { .. }]));
    assert_eq!(editor(&s).tabs.len(), 1);
    ed(
        &mut s,
        id,
        EdMsg::Saved {
            tab,
            result: Ok(stamp(2, 5)),
        },
    );
    assert!(editor(&s).tabs.is_empty());
    // Discard.
    let tab = open_compose(&mut s, id, "x\n");
    edraft(&mut s, id).texts.insert(tab, "y\n".into());
    ed(&mut s, id, EdMsg::Close(tab));
    assert!(ed(&mut s, id, EdMsg::Answer(Answer::Discard)).is_empty());
    assert!(editor(&s).tabs.is_empty());
    assert!(edraft(&mut s, id).texts.is_empty());
}

#[test]
fn closing_the_file_window_with_unsaved_tabs_asks_first() {
    let (mut s, id) = window(Mode::Sftp);
    let tab = open_compose(&mut s, id, "x\n");
    edraft(&mut s, id).texts.insert(tab, "y\n".into());
    let fx = send(&mut s, FmMsg::Close(id));
    assert!(fx.is_empty(), "nothing closes yet");
    assert_eq!(editor(&s).ask, Some(Unsaved::All { window: true }));
    ed(&mut s, id, EdMsg::Answer(Answer::Cancel));
    assert_eq!(s.model.files.len(), 1);
    // Save all, then the window closes when the save is done.
    send(&mut s, FmMsg::Close(id));
    let fx = ed(&mut s, id, EdMsg::Answer(Answer::Save));
    assert!(matches!(&fx[..], [FmEffect::Save { .. }]));
    assert_eq!(s.model.files.len(), 1);
    let fx = ed(
        &mut s,
        id,
        EdMsg::Saved {
            tab,
            result: Ok(stamp(2, 5)),
        },
    );
    assert!(s.model.files.is_empty());
    assert!(fx.iter().any(|e| matches!(e, FmEffect::Close(_))));
}

#[test]
fn a_failed_save_keeps_the_window_open() {
    let (mut s, id) = window(Mode::Sftp);
    let tab = open_compose(&mut s, id, "x\n");
    edraft(&mut s, id).texts.insert(tab, "y\n".into());
    send(&mut s, FmMsg::Close(id));
    ed(&mut s, id, EdMsg::Answer(Answer::Save));
    ed(
        &mut s,
        id,
        EdMsg::Saved {
            tab,
            result: Err(SaveError::Failed("No space left".into())),
        },
    );
    assert_eq!(s.model.files.len(), 1);
    let t = editor(&s).tab(tab).unwrap();
    assert_eq!(t.error.as_deref(), Some("No space left"));
    assert!(!t.close_after_save && !editor(&s).close_window_after_save);
    // Discard all from the editor's own close button closes only the editor.
    ed(&mut s, id, EdMsg::CloseWindow);
    assert_eq!(editor(&s).ask, Some(Unsaved::All { window: false }));
    ed(&mut s, id, EdMsg::Answer(Answer::Discard));
    assert!(editor(&s).tabs.is_empty());
    assert_eq!(s.model.files.len(), 1, "the file window stays");
    // Without unsaved tabs the file window closes at once.
    assert!(
        send(&mut s, FmMsg::Close(id))
            .iter()
            .any(|e| matches!(e, FmEffect::Close(_)))
    );
}

#[test]
fn indentation_is_detected_set_and_converted_without_saving() {
    // #182
    use telepad_core::files::text::Indent;
    let sp = |size| Indent { tabs: false, size };
    let (mut s, id) = window(Mode::Sftp);
    let tab = open_compose(&mut s, id, "a:\n    b: 1\n    c:\n        d: 2\n");
    let t = |s: &State| editor(s).tabs.iter().find(|t| t.id == tab).unwrap().indent;
    assert_eq!(t(&s), sp(4));
    // Indent using 2 spaces: only what Tab inserts, the text is unchanged.
    ed(&mut s, id, EdMsg::SetIndent(tab, sp(2)));
    assert_eq!(t(&s), sp(2));
    assert_eq!(
        edraft(&mut s, id).texts[&tab],
        "a:\n    b: 1\n    c:\n        d: 2\n"
    );
    // Re-indent 4 → 2 is done from what the tab says (back to 4 first).
    ed(&mut s, id, EdMsg::SetIndent(tab, sp(4)));
    let fx = ed(
        &mut s,
        id,
        EdMsg::ConvertIndent {
            tab,
            to: sp(2),
            sel: None,
        },
    );
    assert!(fx.is_empty(), "nothing is written before Save");
    assert_eq!(
        edraft(&mut s, id).texts[&tab],
        "a:\n  b: 1\n  c:\n    d: 2\n"
    );
    assert_eq!(t(&s), sp(2));
    let dirty = {
        let tb = editor(&s).tabs[0].clone();
        edraft(&mut s, id).dirty(&tb)
    };
    assert!(dirty);
    // YAML refuses tabs, both ways.
    let tabs = Indent {
        tabs: true,
        size: 2,
    };
    ed(&mut s, id, EdMsg::SetIndent(tab, tabs));
    ed(
        &mut s,
        id,
        EdMsg::ConvertIndent {
            tab,
            to: tabs,
            sel: None,
        },
    );
    assert_eq!(t(&s), sp(2));
    assert!(!edraft(&mut s, id).texts[&tab].contains('\t'));
    // Another language may use tabs; only the selected lines change.
    ed(&mut s, id, EdMsg::SetLang(tab, Lang::Plain));
    ed(
        &mut s,
        id,
        EdMsg::ConvertIndent {
            tab,
            to: tabs,
            sel: Some(3..9),
        },
    );
    assert_eq!(
        edraft(&mut s, id).texts[&tab],
        "a:\n\tb: 1\n  c:\n    d: 2\n"
    );
}
