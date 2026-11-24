//! Behaviour tests for #144 (host groups) and #145 (clusters).

use std::collections::BTreeMap;
use std::time::Instant;

use telepad_core::config::{HostEntry, HostSet, SetKind};
use telepad_core::store::UiPrefs;
use telepad_core::update::UpdateConfig;

use crate::sets::{StEffect, StMsg, members, stats};
use crate::workspace::Win;
use crate::*;

fn host(id: i64, alias: &str, group: &str, cluster: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: format!("192.0.2.{id}"),
        group: group.into(),
        cluster: cluster.into(),
        ..HostEntry::new(alias)
    }
}

fn set(name: &str, description: &str) -> HostSet {
    HostSet {
        name: name.into(),
        description: description.into(),
    }
}

fn sets() -> BTreeMap<SetKind, Vec<HostSet>> {
    BTreeMap::from([
        (
            SetKind::Group,
            vec![set("Office", "Head office"), set("Lab", "")],
        ),
        (SetKind::Cluster, vec![set("Web", "Load balanced")]),
    ])
}

fn hosts() -> Vec<HostEntry> {
    vec![
        host(1, "web1", "Office", "Web"),
        host(2, "web2", "Lab", "Web"),
        host(3, "nas", "", ""),
    ]
}

fn started(now: Instant) -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::Started {
            db_path: "telepad.db".into(),
            ssh_config_path: "telepad.ssh_config".into(),
            imported_from: None,
            update_config: Ok(UpdateConfig {
                enabled: false,
                ..Default::default()
            }),
            ui_prefs: UiPrefs::default(),
            prefs: Ok(Default::default()),
            strategy: Err("not used".into()),
        },
        now,
    );
    loaded(&mut s, now);
    s
}

fn loaded(s: &mut State, now: Instant) {
    update(s, Msg::Sets(StMsg::Loaded(Ok(sets()))), now);
    update(
        s,
        Msg::HostsLoaded {
            hosts: Ok(hosts()),
            ssh_config_error: None,
        },
        now,
    );
}

fn st(fx: &[Effect]) -> Vec<StEffect> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Sets(x) => Some(x.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn group_and_cluster_are_independent_and_the_fixed_entry_holds_the_rest() {
    let now = Instant::now();
    let s = started(now);
    let h = &s.model.hosts;
    assert_eq!(members(h, SetKind::Group, "Office"), [0]);
    assert_eq!(members(h, SetKind::Group, "Lab"), [1]);
    assert_eq!(members(h, SetKind::Group, ""), [2], "Ungrouped");
    // Hosts of two groups share one cluster.
    assert_eq!(members(h, SetKind::Cluster, "Web"), [0, 1]);
    assert_eq!(members(h, SetKind::Cluster, ""), [2], "No cluster");
    assert_eq!(stats(h, SetKind::Cluster, "Web").hosts, 2);
}

#[test]
fn pages_open_once_and_come_to_the_front() {
    let now = Instant::now();
    let mut s = started(now);
    update(&mut s, Msg::Sets(StMsg::OpenList(SetKind::Group)), now);
    assert!(s.model.ws.is_open(Win::Sets(SetKind::Group)));
    let open = |s: &mut State, name: &str| {
        update(
            s,
            Msg::Sets(StMsg::OpenHosts {
                kind: SetKind::Group,
                name: name.into(),
            }),
            now,
        )
    };
    open(&mut s, "Office");
    open(&mut s, "Office");
    assert_eq!(s.model.set_pages.len(), 1);
    let id = s.model.set_pages[0].id;
    assert!(s.model.ws.is_open(Win::SetHosts(id)));
    assert_eq!(s.model.set_pages[0].title(), "Hosts - Office");
    // The fixed entry by its shown name.
    open(&mut s, "Ungrouped");
    assert_eq!(s.model.set_pages[1].name, "");
    assert_eq!(s.model.set_pages[1].title(), "Hosts - Ungrouped");
    // Unknown names do not open a page.
    open(&mut s, "Nope");
    assert_eq!(s.model.set_pages.len(), 2);
    update(&mut s, Msg::Sets(StMsg::ClosePage(id)), now);
    assert!(!s.model.ws.is_open(Win::SetHosts(id)));
}

#[test]
fn add_rename_and_delete_go_to_the_store_and_reload() {
    let now = Instant::now();
    let mut s = started(now);
    update(&mut s, Msg::Sets(StMsg::Add(SetKind::Cluster)), now);
    // Name checks before anything is saved.
    s.drafts.sets.form.as_mut().unwrap().name = "web".into();
    let fx = update(&mut s, Msg::Sets(StMsg::FormSave), now);
    assert!(st(&fx).is_empty());
    assert!(
        s.drafts
            .sets
            .form
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("already")
    );
    s.drafts.sets.form.as_mut().unwrap().name = "No cluster".into();
    update(&mut s, Msg::Sets(StMsg::FormSave), now);
    assert!(s.drafts.sets.form.as_ref().unwrap().error.is_some());
    s.drafts.sets.form.as_mut().unwrap().name = " Ceph 1 ".into();
    s.drafts.sets.form.as_mut().unwrap().description = "Storage".into();
    let fx = update(&mut s, Msg::Sets(StMsg::FormSave), now);
    assert_eq!(
        st(&fx),
        [StEffect::Save {
            kind: SetKind::Cluster,
            old: None,
            set: set("Ceph 1", "Storage"),
        }]
    );
    let fx = update(&mut s, Msg::Sets(StMsg::Saved(Ok(()))), now);
    assert!(fx.iter().any(|e| matches!(e, Effect::LoadHosts)));
    assert!(s.drafts.sets.form.is_none());

    // Rename an open group: its page follows.
    update(
        &mut s,
        Msg::Sets(StMsg::OpenHosts {
            kind: SetKind::Group,
            name: "Office".into(),
        }),
        now,
    );
    update(
        &mut s,
        Msg::Sets(StMsg::Edit {
            kind: SetKind::Group,
            name: "Office".into(),
        }),
        now,
    );
    assert_eq!(
        s.drafts.sets.form.as_ref().unwrap().description,
        "Head office"
    );
    s.drafts.sets.form.as_mut().unwrap().name = "Head office".into();
    let fx = update(&mut s, Msg::Sets(StMsg::FormSave), now);
    assert!(matches!(
        &st(&fx)[0],
        StEffect::Save { old: Some(o), .. } if o == "Office"
    ));
    update(&mut s, Msg::Sets(StMsg::Saved(Ok(()))), now);
    assert_eq!(s.model.set_pages[0].name, "Head office");

    // Delete asks first.
    let ask = Msg::Sets(StMsg::Delete {
        kind: SetKind::Group,
        name: "Lab".into(),
    });
    assert!(st(&update(&mut s, ask, now)).is_empty());
    let fx = update(&mut s, Msg::Sets(StMsg::DeleteYes), now);
    assert_eq!(
        st(&fx),
        [StEffect::Delete {
            kind: SetKind::Group,
            name: "Lab".into()
        }]
    );
}

#[test]
fn a_deleted_group_closes_its_page_and_reorder_moves_one_place() {
    let now = Instant::now();
    let mut s = started(now);
    update(
        &mut s,
        Msg::Sets(StMsg::OpenHosts {
            kind: SetKind::Group,
            name: "Lab".into(),
        }),
        now,
    );
    let fx = update(
        &mut s,
        Msg::Sets(StMsg::Move {
            kind: SetKind::Group,
            name: "Lab".into(),
            by: -1,
        }),
        now,
    );
    assert_eq!(
        st(&fx),
        [StEffect::Order {
            kind: SetKind::Group,
            names: vec!["Lab".into(), "Office".into()]
        }]
    );
    assert_eq!(s.model.sets[&SetKind::Group][0].name, "Lab");
    // Past the end: nothing.
    let top = Msg::Sets(StMsg::Move {
        kind: SetKind::Group,
        name: "Lab".into(),
        by: -1,
    });
    assert!(st(&update(&mut s, top, now)).is_empty());
    // The store no longer has Lab: its page closes.
    let mut after = sets();
    after
        .get_mut(&SetKind::Group)
        .unwrap()
        .retain(|s| s.name != "Lab");
    update(&mut s, Msg::Sets(StMsg::Loaded(Ok(after))), now);
    assert!(s.model.set_pages.is_empty());
}

#[test]
fn hosts_are_moved_in_and_out_by_alias() {
    let now = Instant::now();
    let mut s = started(now);
    let fx = update(
        &mut s,
        Msg::Sets(StMsg::Assign {
            kind: SetKind::Cluster,
            aliases: vec!["nas".into(), "web2".into()],
            name: "Web".into(),
        }),
        now,
    );
    assert_eq!(
        st(&fx),
        [StEffect::Assign {
            kind: SetKind::Cluster,
            ids: vec![2, 3],
            name: "Web".into()
        }]
    );
    // "Add hosts…" on a group page puts the chosen hosts into that group.
    update(
        &mut s,
        Msg::Sets(StMsg::OpenHosts {
            kind: SetKind::Group,
            name: "Lab".into(),
        }),
        now,
    );
    let page = s.model.set_pages[0].id;
    update(&mut s, Msg::Sets(StMsg::AddHostsOpen(page)), now);
    s.drafts
        .sets
        .add_hosts
        .as_mut()
        .unwrap()
        .chosen
        .insert("nas".into());
    let fx = update(&mut s, Msg::Sets(StMsg::AddHostsRun), now);
    assert_eq!(
        st(&fx),
        [StEffect::Assign {
            kind: SetKind::Group,
            ids: vec![3],
            name: "Lab".into()
        }]
    );
    // Taking a host out = empty name.
    let fx = update(
        &mut s,
        Msg::Sets(StMsg::Assign {
            kind: SetKind::Group,
            aliases: vec!["web1".into()],
            name: String::new(),
        }),
        now,
    );
    assert_eq!(
        st(&fx),
        [StEffect::Assign {
            kind: SetKind::Group,
            ids: vec![1],
            name: String::new()
        }]
    );
}

#[test]
fn a_new_group_keeps_the_host_report_and_reads_nothing_again() {
    let now = Instant::now();
    let mut s = started(now);
    s.model.hosts[2].report = Some(Default::default());
    let mut moved = hosts();
    moved[2].group = "Lab".into();
    let fx = update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(moved),
            ssh_config_error: None,
        },
        now,
    );
    assert_eq!(s.model.hosts[2].entry.group, "Lab");
    assert!(s.model.hosts[2].report.is_some(), "report kept");
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Collect(_))),
        "no SSH read for a group change"
    );
}

#[test]
fn group_pages_come_back_with_the_workspace() {
    let now = Instant::now();
    let mut s = started(now);
    for (kind, name) in [(SetKind::Group, "Office"), (SetKind::Cluster, "No cluster")] {
        update(
            &mut s,
            Msg::Sets(StMsg::OpenHosts {
                kind,
                name: name.into(),
            }),
            now,
        );
    }
    update(&mut s, Msg::Sets(StMsg::OpenList(SetKind::Cluster)), now);
    let layout = crate::workspace::layout(&s.model);
    assert!(layout.contains("group:Office"), "{layout}");
    assert!(layout.contains("cluster:No cluster"), "{layout}");
    assert!(layout.contains("clusters"), "{layout}");
    let mut s2 = State::default();
    s2.model.ws.restore(&layout);
    s2.model.db_path = Some("telepad.db".into());
    // Hosts first, groups later: the pages wait for the groups.
    update(
        &mut s2,
        Msg::HostsLoaded {
            hosts: Ok(hosts()),
            ssh_config_error: None,
        },
        now,
    );
    assert!(s2.model.set_pages.is_empty());
    update(&mut s2, Msg::Sets(StMsg::Loaded(Ok(sets()))), now);
    assert_eq!(s2.model.set_pages.len(), 2);
    for p in &s2.model.set_pages {
        assert!(s2.model.ws.is_open(Win::SetHosts(p.id)), "{p:?}");
    }
    assert!(s2.model.ws.is_open(Win::Sets(SetKind::Cluster)));
}
