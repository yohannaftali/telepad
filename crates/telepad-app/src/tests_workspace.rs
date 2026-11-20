//! Behaviour tests for the workspace of inner windows (#77).

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::store::UiPrefs;
use telepad_core::update::UpdateConfig;

use crate::workspace::{Win, Workspace, WsMsg};
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
            ui_prefs: UiPrefs {
                workspace: layout.into(),
                ..Default::default()
            },
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
    // The layout changed, so it is saved.
    assert!(saved(&fx).is_some_and(|p| p.workspace.contains("docker")));
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
    assert!(saved(&fx).is_some());
    // The same geometry again saves nothing.
    let fx = ws(
        &mut s,
        WsMsg::Geometry {
            win: Win::Overview,
            rect: [10.0, 20.0, 600.0, 400.0],
        },
    );
    assert!(saved(&fx).is_none());
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
    w.restore("");
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
    let text = w.save();
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
    back.restore("junk\n|x|y|z|nothing");
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
}

#[test]
fn sidebar_collapse_is_saved() {
    let (mut s, _) = started("");
    let fx = ws(&mut s, WsMsg::SetSidebarCollapsed(true));
    assert!(saved(&fx).is_some_and(|p| p.sidebar_collapsed));
    let fx = ws(&mut s, WsMsg::SetSidebarCollapsed(true));
    assert!(saved(&fx).is_none());
}
