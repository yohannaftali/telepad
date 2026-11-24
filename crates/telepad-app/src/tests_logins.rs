//! Behaviour tests for #126: the Log info page (read, sort, filter, pages,
//! Telepad's own logins, journal access).

use std::time::Instant;

use telepad_core::authorized_keys::{AuthKey, HostKeys, PublicKey};
use telepad_core::config::{Access, HostEntry};
use telepad_core::login_log::{JournalAccess, Login, LoginLog};
use telepad_core::store::UiPrefs;
use telepad_core::update::UpdateConfig;

use crate::logins::{Col, Dir, LgEffect, LgMsg, LoginDraft, Who, filter_match, page_range, rows};
use crate::workspace::Win;
use crate::*;

const OWN: &str = "SHA256:ownownownownownownownownownownownownownown";
const BOB: &str = "SHA256:bobbobbobbobbobbobbobbobbobbobbobbobbobbob";

fn host(id: i64, alias: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: format!("192.0.2.{id}"),
        ..HostEntry::new(alias)
    }
}

fn started(hosts: Vec<HostEntry>, now: Instant) -> State {
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
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(hosts),
            ssh_config_error: None,
        },
        now,
    );
    s
}

fn login(epoch: i64, user: &str, from: &str, fp: &str, pid: u32) -> Login {
    Login {
        epoch,
        time: format!("2026-11-{:02} 10:00:00", epoch),
        user: user.into(),
        from: from.into(),
        port: 22,
        method: if fp.is_empty() {
            "password"
        } else {
            "publickey"
        }
        .into(),
        key_type: if fp.is_empty() { "" } else { "ED25519" }.into(),
        fingerprint: fp.into(),
        pid,
        ..Default::default()
    }
}

/// alice logs in from 192.0.2.10 (this PC) with Telepad's key; bob from
/// elsewhere; one password login. pid 900 is this read's sshd.
fn sample() -> LoginLog {
    let bob_key = PublicKey {
        kind: "ssh-ed25519".into(),
        blob: "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl".into(),
        comment: "bob@laptop".into(),
    };
    let bob_fp = bob_key.fingerprint();
    LoginLog {
        access: JournalAccess::Readable,
        user: "alice".into(),
        groups: vec!["alice".into(), "systemd-journal".into()],
        client: "192.0.2.10 5000 22".into(),
        pids: vec![901, 900],
        first: Some("2026-10-01 00:00:00".into()),
        logins: vec![
            login(1, "alice", "192.0.2.10", OWN, 100),
            login(2, "bob", "198.51.100.9", &bob_fp, 200),
            login(3, "carol", "198.51.100.10", "", 300),
            login(4, "alice", "192.0.2.10", OWN, 900),
        ],
        truncated: false,
        keys: Ok(HostKeys {
            keys: vec![AuthKey {
                line_no: 1,
                options: String::new(),
                key: bob_key,
                supported: true,
                group: Some("Admins".into()),
            }],
            login: vec![OWN.into()],
            ..Default::default()
        }),
        own: Vec::new(),
    }
}

#[test]
fn log_info_opens_one_page_per_host_and_reads_it() {
    let now = Instant::now();
    let mut s = started(vec![host(1, "web1")], now);
    let fx = update(&mut s, Msg::Logins(LgMsg::Open("web1".into())), now);
    let id = s.model.logins[0].id;
    let reads: Vec<_> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Logins(LgEffect::Read { id, host }) => Some((*id, host.alias.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(reads, [(id, "web1".to_string())]);
    assert!(s.model.ws.is_open(Win::Logins(id)));
    assert!(s.model.logins[0].loading);
    // Opening again shows the same page, no second read.
    let fx = update(&mut s, Msg::Logins(LgMsg::Open("web1".into())), now);
    assert!(fx.iter().all(|e| !matches!(e, Effect::Logins(_))));
    assert_eq!(s.model.logins.len(), 1);
    // Result: the key of this read is remembered for Authorized keys.
    update(
        &mut s,
        Msg::Logins(LgMsg::Loaded {
            id,
            result: Box::new(Ok(sample())),
        }),
        now,
    );
    assert!(!s.model.logins[0].loading);
    assert_eq!(
        s.model.telepad_key.get("web1").map(String::as_str),
        Some(OWN)
    );
    // Refresh reads again; a second Refresh while it runs does nothing.
    let reads = |fx: Vec<Effect>| {
        fx.iter()
            .filter(|e| matches!(e, Effect::Logins(LgEffect::Read { .. })))
            .count()
    };
    assert_eq!(
        reads(update(&mut s, Msg::Logins(LgMsg::Refresh(id)), now)),
        1
    );
    assert_eq!(
        reads(update(&mut s, Msg::Logins(LgMsg::Refresh(id)), now)),
        0
    );
    // Closing the window ends the page.
    update(&mut s, Msg::Logins(LgMsg::Close(id)), now);
    assert!(s.model.logins.is_empty());
    assert!(!s.model.ws.is_open(Win::Logins(id)));
    assert!(s.drafts.logins.is_empty());
}

#[test]
fn log_info_needs_ssh() {
    let now = Instant::now();
    let mut rdp = host(2, "win1");
    rdp.access = Access::Rdp;
    let mut s = started(vec![rdp], now);
    let fx = update(&mut s, Msg::Logins(LgMsg::Open("win1".into())), now);
    assert!(fx.iter().all(|e| !matches!(e, Effect::Logins(_))));
    assert!(s.model.logins.is_empty());
}

#[test]
fn register_and_unregister_open_a_terminal_only() {
    let now = Instant::now();
    let mut s = started(vec![host(1, "web1")], now);
    update(&mut s, Msg::Logins(LgMsg::Open("web1".into())), now);
    let id = s.model.logins[0].id;
    for register in [true, false] {
        let fx = update(&mut s, Msg::Logins(LgMsg::Access { id, register }), now);
        let t: Vec<_> = fx
            .iter()
            .filter_map(|e| match e {
                Effect::Logins(LgEffect::Terminal { host, register }) => {
                    Some((host.alias.clone(), *register))
                }
                Effect::Logins(LgEffect::Read { .. }) => panic!("no read"),
                _ => None,
            })
            .collect();
        assert_eq!(t, [("web1".to_string(), register)]);
    }
}

#[test]
fn own_logins_are_hidden_by_default_and_newest_comes_first() {
    let log = sample();
    let d = LoginDraft::default();
    let r = rows(&log, &[], &d);
    assert_eq!(
        r.iter().map(|r| r.login.user.as_str()).collect::<Vec<_>>(),
        ["carol", "bob"],
        "newest first, this PC's logins hidden"
    );
    assert_eq!(r[1].key_name, "[Admins] bob@laptop");
    let d = LoginDraft {
        show_own: true,
        ..Default::default()
    };
    let r = rows(&log, &[], &d);
    assert_eq!(r.len(), 4);
    assert_eq!(r[0].who, Who::ThisRead);
    assert_eq!(r[3].who, Who::ThisPc);
    assert_eq!(r[1].who, Who::Other);
}

#[test]
fn header_clicks_sort_asc_desc_then_newest_first() {
    let log = sample();
    let mut d = LoginDraft {
        show_own: true,
        ..Default::default()
    };
    d.click(Col::User);
    assert_eq!(d.sort, Some((Col::User, Dir::Asc)));
    let users = |d: &LoginDraft| {
        rows(&log, &[], d)
            .iter()
            .map(|r| r.login.user.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(users(&d), ["alice", "alice", "bob", "carol"]);
    d.click(Col::User);
    assert_eq!(users(&d), ["carol", "bob", "alice", "alice"]);
    d.click(Col::User);
    assert_eq!(d.sort, None);
    assert_eq!(users(&d), ["alice", "carol", "bob", "alice"]);
    // Empty cells (password login has no key type) last both ways.
    d.click(Col::KeyType);
    assert_eq!(users(&d).last().map(String::as_str), Some("carol"));
    d.click(Col::KeyType);
    assert_eq!(users(&d).last().map(String::as_str), Some("carol"));
    d.click(Col::Time);
    assert_eq!(users(&d)[0], "alice");
    assert_eq!(rows(&log, &[], &d)[0].login.epoch, 1);
}

#[test]
fn filters_follow_the_wtbapp_rules() {
    assert!(filter_match("Bob@Laptop", "laptop"));
    assert!(filter_match("bob", "=BOB"));
    assert!(!filter_match("bobby", "=bob"));
    assert!(filter_match("bobby", "!bob"));
    assert!(filter_match("", "NULL"));
    assert!(filter_match("x", "!null"));
    assert!(filter_match("22", ">3"), "numbers by value");
    assert!(
        filter_match("2026-11-24 08:00:00", ">2026-11-20"),
        "times as text"
    );
    assert!(filter_match("2026-11-01 08:00:00", "<2026-11-20"));
    let log = sample();
    let mut d = LoginDraft {
        show_own: true,
        ..Default::default()
    };
    *d.filter_mut(Col::User) = "alice".into();
    assert_eq!(rows(&log, &[], &d).len(), 2);
    *d.filter_mut(Col::KeyType) = "NULL".into();
    assert!(rows(&log, &[], &d).is_empty());
    *d.filter_mut(Col::User) = String::new();
    assert_eq!(rows(&log, &[], &d).len(), 1);
}

#[test]
fn pages_and_typed_rows_per_page() {
    let mut d = LoginDraft::default();
    assert_eq!(page_range(45, &d), (0, 20, 3));
    d.page = 2;
    assert_eq!(page_range(45, &d), (40, 45, 3));
    d.page = 9;
    assert_eq!(page_range(45, &d), (40, 45, 3), "clamped to the last page");
    assert_eq!(page_range(0, &d), (0, 0, 1));
    d.per_page_text = "50 rows".into();
    d.apply_per_page();
    assert_eq!(
        (d.per_page, d.page, d.per_page_text.as_str()),
        (50, 0, "50")
    );
    d.per_page_text = "abc".into();
    d.apply_per_page();
    assert_eq!(d.per_page, 20);
    d.per_page_text = "999999".into();
    d.apply_per_page();
    assert_eq!(d.per_page, crate::logins::PAGE_SIZE_MAX);
}

#[test]
fn key_names_fall_back_to_this_pcs_keys() {
    let log = sample();
    let mine = vec![(OWN.to_string(), "id_ed25519".to_string())];
    let d = LoginDraft {
        show_own: true,
        ..Default::default()
    };
    let r = rows(&log, &mine, &d);
    assert_eq!(r[0].key_name, "id_ed25519 (this PC)");
    assert_eq!(r[1].key_name, "", "password login: no key");
    let _ = BOB;
}

#[test]
fn own_key_names_come_from_the_read() {
    let mut log = sample();
    log.own = vec![(OWN.to_string(), "id_ed25519 — alice@pc".to_string())];
    if let Ok(k) = &mut log.keys {
        k.login.clear();
    }
    let d = LoginDraft {
        show_own: true,
        ..Default::default()
    };
    let r = rows(&log, &[], &d);
    assert_eq!(r[0].key_name, "id_ed25519 — alice@pc (this PC)");
    assert_eq!(r[3].who, Who::ThisPc, "own keys also from the read");
}

#[test]
fn log_info_pages_come_back_with_the_workspace() {
    let now = Instant::now();
    let mut s = started(vec![host(1, "web1")], now);
    update(&mut s, Msg::Logins(LgMsg::Open("web1".into())), now);
    let layout = crate::workspace::layout(&s.model);
    assert!(layout.contains("logins:web1"), "{layout}");
    // A new start with that layout reopens and reads the page.
    let mut s2 = State::default();
    s2.model.ws.restore(&layout);
    s2.model.db_path = Some("telepad.db".into());
    let fx = update(
        &mut s2,
        Msg::HostsLoaded {
            hosts: Ok(vec![host(1, "web1")]),
            ssh_config_error: None,
        },
        now,
    );
    assert_eq!(s2.model.logins.len(), 1);
    assert!(s2.model.ws.is_open(Win::Logins(s2.model.logins[0].id)));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Logins(LgEffect::Read { .. })))
    );
}
