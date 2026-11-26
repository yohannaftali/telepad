//! Behaviour tests for the workspace of inner windows (#77).

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::store::UiPrefs;
use telepad_core::update::UpdateConfig;

use telepad_core::prefs::{Current, WorkspacePrefs};

use crate::workspace::{
    SNAP_LAYOUTS, Win, WkMsg, Workspace, WsMsg, ZONE_MAX, edge_zone, stack_rects, tile_rects,
    zone_rect,
};
use crate::*;

fn started(layout: &str) -> (State, Vec<Effect>) {
    let mut s = State::default();
    let fx = update(
        &mut s,
        Msg::Started {
            db_path: "telepad.db".into(),
            ssh_config_path: "telepad.ssh_config".into(),
            imported_from: None,
            update_config: Ok(UpdateConfig::default()),
            ui_prefs: UiPrefs::default(),
            strategy: Ok(Default::default()),
            prefs: Ok(WorkspacePrefs {
                own: layout.into(),
                ..Default::default()
            }),
        },
        Instant::now(),
    );
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
    (s, fx)
}

fn ws(s: &mut State, m: WsMsg) -> Vec<Effect> {
    update(s, Msg::Ws(m), Instant::now())
}

/// The last write to preference.db in `fx`.
fn prefs_saved(fx: &[Effect]) -> Option<WorkspacePrefs> {
    fx.iter().rev().find_map(|e| match e {
        Effect::SavePrefs(p) => Some(p.clone()),
        _ => None,
    })
}

fn saved(fx: &[Effect]) -> Option<UiPrefs> {
    fx.iter().find_map(|e| match e {
        Effect::SaveUiPrefs(p) => Some(p.clone()),
        _ => None,
    })
}

#[test]
fn first_start_shows_the_overview_maximized() {
    let (s, fx) = started("");
    let w = &s.model.ws;
    assert_eq!(w.groups.len(), 1);
    assert_eq!(w.groups[0].tabs, [Win::Overview]);
    assert!(w.groups[0].maximized);
    assert_eq!(w.focused, Some(Win::Overview));
    assert!(saved(&fx).is_none(), "nothing changed yet");
    assert!(prefs_saved(&fx).is_none());
}

#[test]
fn sidebar_pages_open_once_and_come_back_to_the_front() {
    let (mut s, _) = started("");
    ws(&mut s, WsMsg::Open(Win::Docker));
    ws(&mut s, WsMsg::Minimize(Win::Docker));
    assert!(s.model.ws.groups[1].minimized);
    let raise = s.model.ws.groups[1].raise;
    let fx = ws(&mut s, WsMsg::Open(Win::Docker));
    assert_eq!(s.model.ws.groups.len(), 2, "no second Docker window");
    let g = &s.model.ws.groups[1];
    assert!(!g.minimized && g.raise > raise);
    assert_eq!(s.model.ws.focused, Some(Win::Docker));
    // The layout changed: it is in the current workspace (<own>), written to
    // preference.db within a second; telepad.db is not written.
    assert!(s.model.wss.own.contains("docker"));
    assert!(saved(&fx).is_none());
}

#[test]
fn maximize_minimize_move_and_close() {
    let (mut s, _) = started("");
    ws(&mut s, WsMsg::ToggleMaximize(Win::Overview));
    assert!(!s.model.ws.groups[0].maximized);
    let fx = ws(
        &mut s,
        WsMsg::Geometry {
            win: Win::Overview,
            rect: [10.0, 20.0, 600.0, 400.0],
        },
    );
    assert_eq!(s.model.ws.groups[0].rect, Some([10.0, 20.0, 600.0, 400.0]));
    let _ = fx;
    assert!(s.model.wss.own.starts_with("10,20,600,400|"));
    s.model.prefs_dirty = false;
    // The same geometry again saves nothing.
    let fx = ws(
        &mut s,
        WsMsg::Geometry {
            win: Win::Overview,
            rect: [10.0, 20.0, 600.0, 400.0],
        },
    );
    assert!(prefs_saved(&fx).is_none() && !s.model.prefs_dirty);
    ws(&mut s, WsMsg::Close(Win::Overview));
    assert!(s.model.ws.groups.is_empty());
    assert_eq!(s.model.ws.focused, None);
}

#[test]
fn tabs_merge_activate_and_detach() {
    let (mut s, _) = started("");
    ws(&mut s, WsMsg::Open(Win::Docker));
    ws(
        &mut s,
        WsMsg::Merge {
            win: Win::Docker,
            into: Win::Overview,
        },
    );
    let w = &s.model.ws;
    assert_eq!(w.groups.len(), 1);
    assert_eq!(w.groups[0].tabs, [Win::Overview, Win::Docker]);
    assert_eq!(w.groups[0].active_win(), Win::Docker);
    ws(&mut s, WsMsg::ActivateTab(Win::Overview));
    assert_eq!(s.model.ws.groups[0].active_win(), Win::Overview);
    // Merging into its own window does nothing.
    ws(
        &mut s,
        WsMsg::Merge {
            win: Win::Docker,
            into: Win::Overview,
        },
    );
    assert_eq!(s.model.ws.groups.len(), 1);
    ws(&mut s, WsMsg::Detach(Win::Docker));
    let w = &s.model.ws;
    assert_eq!(w.groups.len(), 2);
    assert_eq!(w.groups[0].tabs, [Win::Overview]);
    assert_eq!(w.groups[1].tabs, [Win::Docker]);
    // A single tab cannot be detached.
    ws(&mut s, WsMsg::Detach(Win::Docker));
    assert_eq!(s.model.ws.groups.len(), 2);
    // Closing the shown tab shows the one before it.
    ws(
        &mut s,
        WsMsg::Merge {
            win: Win::Docker,
            into: Win::Overview,
        },
    );
    ws(&mut s, WsMsg::Close(Win::Docker));
    assert_eq!(s.model.ws.groups[0].tabs, [Win::Overview]);
    assert_eq!(s.model.ws.groups[0].active, 0);
}

#[test]
fn session_windows_follow_their_session() {
    let (mut s, _) = started("");
    // Clicking a host in the overview opens its detail window.
    update(&mut s, Msg::Select(Some(0)), Instant::now());
    assert!(s.model.ws.is_open(Win::Host));
    assert_eq!(s.model.ws.focused, Some(Win::Host));
    // Closing the detail window = no host selected.
    update(&mut s, Msg::Select(None), Instant::now());
    assert!(!s.model.ws.is_open(Win::Host));
    // Public keys: the window follows the keys draft.
    update(&mut s, Msg::KeysOpen, Instant::now());
    assert!(s.model.ws.is_open(Win::Keys));
    update(&mut s, Msg::KeysClose, Instant::now());
    assert!(!s.model.ws.is_open(Win::Keys));
    // A file window session opens an inner window and closes it again.
    update(
        &mut s,
        Msg::Files(crate::files::FmMsg::Open("web1".into())),
        Instant::now(),
    );
    let id = s.model.files[0].id;
    assert!(s.model.ws.is_open(Win::Files(id)));
    update(
        &mut s,
        Msg::Files(crate::files::FmMsg::Close(id)),
        Instant::now(),
    );
    assert!(s.model.files.is_empty());
    assert!(!s.model.ws.is_open(Win::Files(id)));
}

#[test]
fn layout_is_saved_and_restored() {
    let mut w = Workspace::default();
    w.restore_or_default("");
    w.update(WsMsg::ToggleMaximize(Win::Overview));
    w.update(WsMsg::Geometry {
        win: Win::Overview,
        rect: [0.0, 0.0, 700.0, 500.0],
    });
    w.update(WsMsg::Open(Win::Docker));
    w.update(WsMsg::Merge {
        win: Win::Docker,
        into: Win::Overview,
    });
    w.update(WsMsg::Open(Win::Keys));
    w.update(WsMsg::Minimize(Win::Keys));
    w.update(WsMsg::Open(Win::Files(7)));
    let text = w.save(None);
    // Live sessions (files) are not written.
    assert_eq!(
        text,
        "0,0,700,500|0|0|docker|overview;docker\n|1|0|keys|keys"
    );
    let mut back = Workspace::default();
    back.restore(&text);
    assert_eq!(back.groups.len(), 2);
    assert_eq!(back.groups[0].tabs, [Win::Overview, Win::Docker]);
    assert_eq!(back.groups[0].active_win(), Win::Docker);
    assert_eq!(back.groups[0].rect, Some([0.0, 0.0, 700.0, 500.0]));
    assert!(back.groups[1].minimized);
    // Garbage lines are skipped; nothing left = the default layout.
    back.restore_or_default("junk\n|x|y|z|nothing");
    assert_eq!(back.groups.len(), 1);
    assert!(back.groups[0].maximized);
}

#[test]
fn saved_layout_reopens_its_pages_on_start() {
    let (s, fx) = started("|0|0|keys|keys\n|0|0|hostsync|hostsync");
    assert!(s.drafts.keys.is_some(), "Public keys reopened");
    assert!(s.model.ws.is_open(Win::Keys));
    assert!(s.model.host_sync.is_some(), "Host Management reopened");
    assert!(s.model.ws.is_open(Win::HostSync));
    assert!(fx.iter().any(|e| matches!(e, Effect::HostSync(_))));
    // The Authorized keys lists were read once the hosts loaded (in `started`).
    assert_eq!(s.model.auth_keys.get("web1"), Some(&crate::AkHost::Loading));
}

#[test]
fn sidebar_collapse_is_saved() {
    let (mut s, _) = started("");
    let fx = ws(&mut s, WsMsg::SetSidebarCollapsed(true));
    assert!(saved(&fx).is_some_and(|p| p.sidebar_collapsed));
    let fx = ws(&mut s, WsMsg::SetSidebarCollapsed(true));
    assert!(saved(&fx).is_none());
}

// ---------------------------------------------------------------- #92 window picker

#[test]
fn picker_query_matches_every_word_ignoring_case() {
    assert!(matches_query("Files — web1", ""));
    assert!(matches_query("Files — web1", "WEB"));
    assert!(matches_query("Files — web1", "web files"));
    assert!(!matches_query("Files — web1", "db"));
    assert!(!matches_query("Host — web1", "files web1"));
}

#[test]
fn picker_lists_every_open_window_with_minimized_ones() {
    let mut ws = Workspace::default();
    ws.open(Win::Overview);
    ws.open(Win::Docker);
    ws.update(WsMsg::Merge {
        win: Win::Docker,
        into: Win::Overview,
    });
    ws.open(Win::Kube);
    ws.update(WsMsg::Minimize(Win::Kube));
    assert_eq!(
        ws.open_windows(),
        vec![
            (Win::Overview, false),
            (Win::Docker, false),
            (Win::Kube, true)
        ]
    );
    // Picking a minimized window brings it back.
    ws.update(WsMsg::Open(Win::Kube));
    assert_eq!(ws.open_windows()[2], (Win::Kube, false));
    assert_eq!(ws.focused, Some(Win::Kube));
    assert!(Workspace::default().open_windows().is_empty());
}

// ---------------------------------------------------------------- #94 workspaces

fn wk(s: &mut State, m: WkMsg, now: Instant) -> Vec<Effect> {
    update(s, Msg::Wk(m), now)
}

#[test]
fn autosave_writes_at_most_once_a_second() {
    let t0 = Instant::now();
    let (mut s, _) = started("");
    let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
    let fx = update(&mut s, Msg::Ws(WsMsg::Open(Win::Docker)), at(0));
    assert!(prefs_saved(&fx).is_some_and(|p| p.own.contains("docker")));
    let fx = update(&mut s, Msg::Ws(WsMsg::Open(Win::Kube)), at(300));
    assert!(prefs_saved(&fx).is_none(), "too soon: waits");
    assert!(s.model.prefs_dirty);
    // The next tick after a second writes it.
    let fx = update(&mut s, Msg::Tick { busy: 0 }, at(1100));
    assert!(prefs_saved(&fx).is_some_and(|p| p.own.contains("kube")));
    assert!(!s.model.prefs_dirty);
    let fx = update(&mut s, Msg::Tick { busy: 0 }, at(2500));
    assert!(prefs_saved(&fx).is_none(), "nothing changed");
}

#[test]
fn add_switch_and_delete_workspaces() {
    let t0 = Instant::now();
    let (mut s, _) = started("");
    // Add from the current layout; the new one becomes current.
    wk(&mut s, WkMsg::AskAdd, t0);
    assert!(s.drafts.ws_add.is_some());
    let fx = wk(&mut s, WkMsg::Add("  ops ".into()), t0);
    assert!(s.drafts.ws_add.is_none());
    assert_eq!(s.model.wss.current, Current::Custom("ops".into()));
    let p = prefs_saved(&fx).expect("written at once");
    assert_eq!(p.custom[0].0, "ops");
    assert!(p.custom[0].1.contains("overview"));
    // A wrong label keeps the dialog with the reason.
    wk(&mut s, WkMsg::AskAdd, t0);
    let fx = wk(&mut s, WkMsg::Add("OPS".into()), t0);
    assert!(prefs_saved(&fx).is_none());
    let d = s.drafts.ws_add.clone().unwrap();
    assert!(d.error.unwrap().contains("exists"));
    wk(&mut s, WkMsg::CancelAdd, t0);
    // Change ops: Docker only.
    ws(&mut s, WsMsg::Close(Win::Overview));
    ws(&mut s, WsMsg::Open(Win::Docker));
    assert!(
        s.model
            .wss
            .layout(&Current::Custom("ops".into()))
            .unwrap()
            .contains("docker")
    );
    // <none>: no windows; nothing is saved while it is current.
    let fx = wk(&mut s, WkMsg::Switch(Current::None), t0);
    assert!(s.model.ws.groups.is_empty());
    assert_eq!(prefs_saved(&fx).unwrap().current, Current::None);
    ws(&mut s, WsMsg::Open(Win::Kube));
    assert!(!s.model.wss.own.contains("kube"));
    assert!(!s.model.wss.custom[0].1.contains("kube"));
    // Back to <own>: the Hosts page again.
    wk(&mut s, WkMsg::Switch(Current::Own), t0);
    assert_eq!(s.model.ws.open_windows(), vec![(Win::Overview, false)]);
    // ops: Docker.
    wk(&mut s, WkMsg::Switch(Current::Custom("ops".into())), t0);
    assert_eq!(s.model.ws.open_windows(), vec![(Win::Docker, false)]);
    // Unknown workspace: nothing happens.
    wk(&mut s, WkMsg::Switch(Current::Custom("gone".into())), t0);
    assert_eq!(s.model.wss.current, Current::Custom("ops".into()));
    // Deleting the current one asks first, then goes back to <own>.
    wk(&mut s, WkMsg::AskDelete("ops".into()), t0);
    assert_eq!(s.drafts.ws_delete.as_deref(), Some("ops"));
    let fx = wk(&mut s, WkMsg::Delete("ops".into()), t0);
    assert!(s.drafts.ws_delete.is_none());
    assert_eq!(s.model.wss.current, Current::Own);
    assert!(s.model.wss.custom.is_empty());
    assert!(prefs_saved(&fx).is_some_and(|p| p.custom.is_empty()));
    assert!(s.model.ws.is_open(Win::Overview));
}

#[test]
fn switching_closes_sessions_and_reopens_pages() {
    let t0 = Instant::now();
    let (mut s, _) = started("");
    wk(&mut s, WkMsg::Add("files".into()), t0);
    update(
        &mut s,
        Msg::Files(crate::files::FmMsg::Open("web1".into())),
        t0,
    );
    update(&mut s, Msg::KeysOpen, t0);
    assert_eq!(s.model.files.len(), 1);
    wk(&mut s, WkMsg::Switch(Current::Own), t0);
    assert!(s.model.files.is_empty(), "file session closed");
    assert!(s.drafts.keys.is_none(), "keys page closed");
    // The keys page was in "files" (it is a page): it comes back with it.
    wk(&mut s, WkMsg::Switch(Current::Custom("files".into())), t0);
    assert!(s.drafts.keys.is_some());
    assert!(s.model.ws.is_open(Win::Keys));
}

#[test]
fn host_window_is_saved_by_alias_and_comes_back() {
    let t0 = Instant::now();
    let (mut s, _) = started("");
    update(&mut s, Msg::Select(Some(0)), t0);
    ws(
        &mut s,
        WsMsg::Geometry {
            win: Win::Host,
            rect: [40.0, 30.0, 800.0, 500.0],
        },
    );
    assert!(
        s.model.wss.own.contains("|host:web1|host:web1"),
        "{}",
        s.model.wss.own
    );
    let layout = s.model.wss.own.clone();
    // Next start: the host window waits for the hosts, then opens where it was.
    let (s2, _) = started(&layout);
    assert_eq!(s2.model.selected, Some(0));
    let g = s2
        .model
        .ws
        .group_of(Win::Host)
        .map(|i| &s2.model.ws.groups[i]);
    assert_eq!(g.and_then(|g| g.rect), Some([40.0, 30.0, 800.0, 500.0]));
    // A host that no longer exists is left out.
    let (s3, _) = started("1,1,500,400|0|0|host:gone|host:gone");
    assert_eq!(s3.model.selected, None);
    assert!(!s3.model.ws.is_open(Win::Host));
}

#[test]
fn preference_db_missing_still_starts_and_saves_nothing() {
    let mut s = State::default();
    update(
        &mut s,
        Msg::Started {
            db_path: "telepad.db".into(),
            ssh_config_path: "telepad.ssh_config".into(),
            imported_from: None,
            update_config: Ok(UpdateConfig::default()),
            ui_prefs: UiPrefs::default(),
            strategy: Ok(Default::default()),
            prefs: Err("locked".into()),
        },
        Instant::now(),
    );
    assert!(s.model.message.as_ref().unwrap().0.contains("not saved"));
    assert!(s.model.ws.is_open(Win::Overview));
    let fx = ws(&mut s, WsMsg::Open(Win::Docker));
    assert!(prefs_saved(&fx).is_none());
    let fx = wk(&mut s, WkMsg::Add("x".into()), Instant::now());
    assert!(prefs_saved(&fx).is_none());
}

#[test]
fn tile_and_stack_fill_the_workspace() {
    for n in [1usize, 2, 3, 4, 7] {
        let r = tile_rects(n, 1200.0, 800.0);
        assert_eq!(r.len(), n);
        for [x, y, w, h] in &r {
            assert!(
                *x >= 0.0 && *y >= 0.0 && x + w <= 1200.0 && y + h <= 800.0,
                "{n}: {r:?}"
            );
        }
        // Every row reaches the right edge.
        let right = r.iter().map(|[x, _, w, _]| x + w).fold(0.0, f32::max);
        assert!(right >= 1190.0, "{n}");
        let s = stack_rects(n, 1200.0, 800.0);
        assert_eq!(s.len(), n);
        for w in s.windows(2) {
            assert!(w[1][0] >= w[0][0] && w[1][1] >= w[0][1], "cascade");
        }
        for [x, y, w, h] in &s {
            assert!(x + w <= 1200.0 + 0.5 && y + h <= 800.0 + 0.5);
        }
    }
    assert_eq!(
        tile_rects(4, 1000.0, 600.0)[3],
        [502.0, 302.0, 496.0, 296.0]
    );
    assert!(tile_rects(0, 100.0, 100.0).is_empty());
    for [_, _, w, h] in tile_rects(7, 50.0, 30.0) {
        assert!(w >= 1.0 && h >= 1.0, "tiny workspace");
    }
}

#[test]
fn arrange_tile_stack_minimize_and_restore_all() {
    let (mut s, _) = started("");
    ws(&mut s, WsMsg::Open(Win::Docker));
    ws(&mut s, WsMsg::Open(Win::Kube));
    ws(&mut s, WsMsg::Minimize(Win::Kube));
    ws(
        &mut s,
        WsMsg::Tile {
            w: 1000.0,
            h: 600.0,
        },
    );
    let w = &s.model.ws;
    assert!(!w.groups[0].maximized, "un-maximized first");
    assert_eq!(w.groups[0].rect.unwrap()[2], 496.0, "two side by side");
    assert_eq!(w.groups[2].rect, None, "minimized ones are left out");
    ws(
        &mut s,
        WsMsg::Stack {
            w: 1000.0,
            h: 600.0,
        },
    );
    let w = &s.model.ws;
    assert!(w.groups[1].rect.unwrap()[0] > w.groups[0].rect.unwrap()[0]);
    ws(&mut s, WsMsg::MinimizeAll);
    assert!(s.model.ws.groups.iter().all(|g| g.minimized));
    assert_eq!(s.model.ws.focused, None);
    ws(&mut s, WsMsg::RestoreAll);
    assert!(s.model.ws.groups.iter().all(|g| !g.minimized));
    assert!(s.model.ws.focused.is_some());
    assert!(s.model.wss.own.contains("kube"), "arranging is autosaved");
}

/// Minimized, or behind another window that was opened after it.
fn hidden(s: &State, w: Win) -> bool {
    let g = &s.model.ws.groups[s.model.ws.group_of(w).unwrap()];
    let top = s.model.ws.groups.iter().map(|g| g.raise).max().unwrap();
    g.minimized || g.raise < top || s.model.ws.focused != Some(w)
}

#[test]
fn asking_for_an_open_page_restores_and_raises_it() {
    // #136: Edit / Open files / Open database / Log info / host page.
    use crate::editor::EdMsg;
    use crate::files::{FmMsg, Mode, Side};
    use telepad_core::files::{Entry, Kind};
    let (mut s, _) = started("");
    let now = Instant::now();
    update(&mut s, Msg::Files(FmMsg::Open("web1".into())), now);
    let id = s.model.files[0].id;
    update(
        &mut s,
        Msg::Files(FmMsg::Connected {
            id,
            result: Ok((Mode::Sftp, "/home/ops".into(), None)),
            local_home: "/home/me".into(),
        }),
        now,
    );
    let files = vec![
        Entry {
            name: "a.conf".into(),
            kind: Kind::File,
            ..Default::default()
        },
        Entry {
            name: "b.conf".into(),
            kind: Kind::File,
            ..Default::default()
        },
    ];
    update(
        &mut s,
        Msg::Files(FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok(("/home/ops".into(), files)),
        }),
        now,
    );
    let edit = |s: &mut State, name: &str| {
        update(
            s,
            Msg::Files(FmMsg::Editor {
                id,
                msg: EdMsg::Open {
                    side: Side::Remote,
                    name: name.into(),
                },
            }),
            Instant::now(),
        )
    };
    edit(&mut s, "a.conf");
    // Editor minimized, then another page in front.
    ws(&mut s, WsMsg::Minimize(Win::Editor(id)));
    ws(&mut s, WsMsg::Open(Win::Overview));
    assert!(hidden(&s, Win::Editor(id)));
    edit(&mut s, "b.conf");
    assert!(!hidden(&s, Win::Editor(id)), "Edit brings the editor back");
    // File window behind other pages: Open files raises it.
    ws(&mut s, WsMsg::Open(Win::Docker));
    assert!(hidden(&s, Win::Files(id)));
    update(&mut s, Msg::Files(FmMsg::Open("web1".into())), now);
    assert!(!hidden(&s, Win::Files(id)));
    // Host page minimized: selecting the host again restores it.
    update(&mut s, Msg::Select(Some(0)), now);
    ws(&mut s, WsMsg::Minimize(Win::Host));
    update(&mut s, Msg::Select(Some(0)), now);
    assert!(!hidden(&s, Win::Host));
    // A maximized window stays maximized when raised.
    ws(&mut s, WsMsg::ToggleMaximize(Win::Files(id)));
    ws(&mut s, WsMsg::Open(Win::Overview));
    update(&mut s, Msg::Files(FmMsg::Open("web1".into())), now);
    let g = &s.model.ws.groups[s.model.ws.group_of(Win::Files(id)).unwrap()];
    assert!(g.maximized && !g.minimized);
}

#[test]
fn opening_an_open_database_or_log_info_raises_it() {
    use telepad_core::db::{DbTarget, Engine};
    let (mut s, _) = started("");
    let now = Instant::now();
    let t = DbTarget {
        engine: Engine::MySql,
        host: "web1".into(),
        address: "127.0.0.1".into(),
        port: 3306,
        ..Default::default()
    };
    update(&mut s, Msg::Db(crate::db::DbMsg::Open(t.clone())), now);
    let db = Win::Db(s.model.db[0].id);
    ws(&mut s, WsMsg::Minimize(db));
    update(&mut s, Msg::Db(crate::db::DbMsg::Open(t)), now);
    assert!(!hidden(&s, db));
    update(
        &mut s,
        Msg::Logins(crate::logins::LgMsg::Open("web1".into())),
        now,
    );
    let lg = Win::Logins(s.model.logins[0].id);
    ws(&mut s, WsMsg::Open(Win::Overview));
    assert!(hidden(&s, lg));
    update(
        &mut s,
        Msg::Logins(crate::logins::LgMsg::Open("web1".into())),
        now,
    );
    assert!(!hidden(&s, lg));
}

#[test]
fn snap_zones_cover_halves_thirds_quarters_and_the_whole_workspace() {
    // #168, a 1200 × 800 workspace: 2 points of gap around each zone.
    let (w, h) = (1200.0, 800.0);
    let r = |z| zone_rect(z, w, h);
    assert_eq!(r(ZONE_MAX), [0.0, 0.0, 1200.0, 800.0]);
    assert_eq!(r(SNAP_LAYOUTS[0][0]), [2.0, 2.0, 596.0, 796.0]);
    assert_eq!(r(SNAP_LAYOUTS[0][1]), [602.0, 2.0, 596.0, 796.0]);
    assert_eq!(r(SNAP_LAYOUTS[1][0]), [2.0, 2.0, 796.0, 796.0]);
    assert_eq!(r(SNAP_LAYOUTS[1][1]), [802.0, 2.0, 396.0, 796.0]);
    let thirds: Vec<_> = SNAP_LAYOUTS[2].iter().map(|&z| r(z)).collect();
    assert_eq!(
        thirds,
        [
            [2.0, 2.0, 396.0, 796.0],
            [402.0, 2.0, 396.0, 796.0],
            [802.0, 2.0, 396.0, 796.0]
        ]
    );
    assert_eq!(r(SNAP_LAYOUTS[3][3]), [602.0, 402.0, 596.0, 396.0]);
    // Edges: left / right half, top maximizes, corners quarter, inside nothing.
    assert_eq!(edge_zone(0.0, 400.0, w, h), Some(SNAP_LAYOUTS[0][0]));
    assert_eq!(edge_zone(1199.0, 400.0, w, h), Some(SNAP_LAYOUTS[0][1]));
    assert_eq!(edge_zone(600.0, 0.0, w, h), Some(ZONE_MAX));
    assert_eq!(edge_zone(0.0, 30.0, w, h), Some(SNAP_LAYOUTS[3][0]));
    assert_eq!(edge_zone(1150.0, 0.0, w, h), Some(SNAP_LAYOUTS[3][1]));
    assert_eq!(edge_zone(2.0, 790.0, w, h), Some(SNAP_LAYOUTS[3][2]));
    assert_eq!(edge_zone(1199.0, 799.0, w, h), Some(SNAP_LAYOUTS[3][3]));
    assert_eq!(edge_zone(600.0, 400.0, w, h), None);
}

#[test]
fn a_snapped_window_keeps_its_old_size_until_moved_by_hand() {
    // #168
    let (mut s, _) = started("");
    let free = [10.0, 20.0, 600.0, 400.0];
    ws(&mut s, WsMsg::ToggleMaximize(Win::Overview));
    ws(
        &mut s,
        WsMsg::Geometry {
            win: Win::Overview,
            rect: free,
        },
    );
    let left = zone_rect(SNAP_LAYOUTS[0][0], 1200.0, 800.0);
    ws(
        &mut s,
        WsMsg::Snap {
            win: Win::Overview,
            rect: left,
            from: free,
        },
    );
    let g = &s.model.ws.groups[0];
    assert_eq!((g.rect, g.unsnap), (Some(left), Some(free)));
    assert!(
        s.model.wss.own.starts_with("2,2,596,796|"),
        "saved with the layout"
    );
    // Snapped again (also from maximized): the first size stays.
    ws(&mut s, WsMsg::ToggleMaximize(Win::Overview));
    let quarter = zone_rect(SNAP_LAYOUTS[3][1], 1200.0, 800.0);
    ws(
        &mut s,
        WsMsg::Snap {
            win: Win::Overview,
            rect: quarter,
            from: left,
        },
    );
    let g = &s.model.ws.groups[0];
    assert!(!g.maximized);
    assert_eq!((g.rect, g.unsnap), (Some(quarter), Some(free)));
    // Dragged out (the view gives back the old size): no longer snapped.
    ws(
        &mut s,
        WsMsg::Geometry {
            win: Win::Overview,
            rect: [300.0, 40.0, 600.0, 400.0],
        },
    );
    assert_eq!(s.model.ws.groups[0].unsnap, None);
}
