//! Behaviour tests for My Computer (#129).

use std::time::{Duration, Instant};

use telepad_core::config::{HostEntry, LOCAL_ALIAS};
use telepad_core::db::{DbTarget, Engine};
use telepad_core::files::{Entry, Kind, local_join};
use telepad_core::model::HostReport;

use crate::files::{FmEffect, FmMsg, Mode, Side};
use crate::workspace::{Win, WsMsg};
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

fn tick(s: &mut State, now: Instant) -> Vec<Effect> {
    update(s, Msg::Tick { busy: 1 }, now)
}

fn reads(fx: &[Effect]) -> usize {
    fx.iter()
        .filter(|e| matches!(e, Effect::CollectLocal { .. }))
        .count()
}

fn report() -> Box<HostReport> {
    Box::new(HostReport {
        hostname: "desk-01".into(),
        os_pretty: "Windows 11 Pro".into(),
        ..Default::default()
    })
}

#[test]
fn my_computer_is_read_while_its_page_is_open() {
    let mut s = state();
    let t0 = Instant::now();
    // Closed: never read.
    assert_eq!(reads(&tick(&mut s, t0)), 0);
    update(&mut s, Msg::Ws(WsMsg::Open(Win::MyComputer)), t0);
    assert_eq!(reads(&tick(&mut s, t0)), 1, "read once when it opens");
    assert!(s.model.local.loading);
    assert!(s.model.local.entry.is_local());
    assert_eq!(reads(&tick(&mut s, t0)), 0, "never two reads at once");
    update(
        &mut s,
        Msg::LocalCollected {
            result: Ok(report()),
            elapsed: Duration::from_millis(1800),
            slow: true,
        },
        t0,
    );
    let l = &s.model.local;
    assert!(!l.loading && l.error.is_none());
    assert_eq!(l.report.as_ref().unwrap().hostname, "desk-01");
    // Next read at the live interval; Refresh at once.
    let live = s.model.strategy.live.every();
    assert_eq!(reads(&tick(&mut s, t0 + live / 2)), 0);
    assert_eq!(reads(&tick(&mut s, t0 + live)), 1);
    update(
        &mut s,
        Msg::LocalCollected {
            result: Err("netstat failed".into()),
            elapsed: Duration::from_secs(1),
            slow: false,
        },
        t0 + live,
    );
    let l = &s.model.local;
    assert_eq!(l.error.as_deref(), Some("netstat failed"));
    assert!(l.report.is_some(), "the last good report stays shown");
    assert_eq!(reads(&update(&mut s, Msg::LocalRefresh, t0 + live)), 1);
}

#[test]
fn pausing_everything_stops_the_periodic_read_but_not_refresh() {
    let mut s = state();
    let t0 = Instant::now();
    update(&mut s, Msg::SetPaused(true), t0);
    update(&mut s, Msg::Ws(WsMsg::Open(Win::MyComputer)), t0);
    assert_eq!(reads(&tick(&mut s, t0)), 1, "the first read still happens");
    update(
        &mut s,
        Msg::LocalCollected {
            result: Ok(report()),
            elapsed: Duration::from_secs(1),
            slow: false,
        },
        t0,
    );
    let later = t0 + s.model.strategy.live.every() * 3;
    assert_eq!(reads(&tick(&mut s, later)), 0);
    assert_eq!(reads(&update(&mut s, Msg::LocalRefresh, later)), 1);
}

#[test]
fn my_computer_page_is_kept_in_the_workspace_layout() {
    assert_eq!(Win::MyComputer.key(), Some("local"));
    assert_eq!(Win::from_key("local"), Some(Win::MyComputer));
}

#[test]
fn terminal_opens_a_local_shell() {
    let mut s = state();
    let fx = update(&mut s, Msg::LocalTerminal, Instant::now());
    assert!(matches!(fx.as_slice(), [Effect::OpenLocalTerminal]));
}

fn files(fx: Vec<Effect>) -> Vec<FmEffect> {
    fx.into_iter()
        .filter_map(|e| match e {
            Effect::Files(FmEffect::Log(_) | FmEffect::LoadHistory { .. }) => None,
            Effect::Files(f) => Some(f),
            _ => None,
        })
        .collect()
}

#[test]
fn files_of_my_computer_show_this_computer_on_both_sides() {
    let mut s = state();
    let now = Instant::now();
    let fx = files(update(
        &mut s,
        Msg::Files(FmMsg::Open(LOCAL_ALIAS.into())),
        now,
    ));
    let host = match fx.as_slice() {
        [FmEffect::Connect { host, .. }] => host.clone(),
        other => panic!("{other:?}"),
    };
    assert!(host.is_local(), "not an inventory host, no SSH");
    let id = s.model.files[0].id;
    let home = std::env::temp_dir().display().to_string();
    let fx = files(update(
        &mut s,
        Msg::Files(FmMsg::Connected {
            id,
            result: Ok((Mode::Local, home.clone(), None)),
            local_home: home.clone(),
        }),
        now,
    ));
    assert_eq!(fx.len(), 2, "both panes list this computer's home: {fx:?}");
    let f = &s.model.files[0];
    assert_eq!(f.mode, Mode::Local);
    assert!(f.local_side(Side::Remote));
    // The right pane joins paths the way this computer does.
    update(
        &mut s,
        Msg::Files(FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok((
                home.clone(),
                vec![Entry {
                    name: "site".into(),
                    kind: Kind::Dir,
                    ..Default::default()
                }],
            )),
        }),
        now,
    );
    let fx = files(update(
        &mut s,
        Msg::Files(FmMsg::Enter {
            id,
            side: Side::Remote,
            name: "site".into(),
        }),
        now,
    ));
    match fx.as_slice() {
        [FmEffect::List { side, path, .. }] => {
            assert_eq!(*side, Side::Remote);
            assert_eq!(*path, local_join(&home, "site"));
        }
        other => panic!("{other:?}"),
    }
    // Permissions are a host-only dialog; both sides are this computer.
    assert_eq!(
        crate::files::op_blocked(
            &s.model.files[0],
            1,
            Side::Remote,
            crate::files::OpKind::Permissions
        ),
        Some("Host side only")
    );
}

#[test]
fn databases_of_my_computer_open_without_an_inventory_host() {
    let mut s = state();
    let target = DbTarget {
        engine: Engine::MySql,
        host: LOCAL_ALIAS.into(),
        address: "127.0.0.1".into(),
        port: 3306,
        ..Default::default()
    };
    let fx = update(
        &mut s,
        Msg::Db(crate::db::DbMsg::Open(target.clone())),
        Instant::now(),
    );
    assert_eq!(s.model.db.len(), 1);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Db(crate::db::DbEffect::LoadProfile { host, .. }) if host.is_local()
    )));
    assert_eq!(
        s.model.db[0].target.title(),
        "MariaDB / MySQL :3306 on This computer"
    );
}

fn slow_of(fx: &[Effect]) -> Option<bool> {
    fx.iter().find_map(|e| match e {
        Effect::CollectLocal { slow } => Some(*slow),
        _ => None,
    })
}

#[test]
fn slow_facts_are_read_at_the_slow_interval_and_kept_between() {
    // #211: Windows Update and installed programs only on slow reads.
    use telepad_core::model::App;
    let mut s = state();
    let t0 = Instant::now();
    update(&mut s, Msg::Ws(WsMsg::Open(Win::MyComputer)), t0);
    assert_eq!(slow_of(&tick(&mut s, t0)), Some(true), "first read is slow");
    let mut first = report();
    first.os_id = "windows".into();
    first.maintenance.updates = Some(2);
    first.maintenance.last_update = Some("2026-11-20".into());
    first.apps = Some(vec![App {
        name: "Example Editor".into(),
        ..Default::default()
    }]);
    update(
        &mut s,
        Msg::LocalCollected {
            result: Ok(first),
            elapsed: Duration::from_secs(2),
            slow: true,
        },
        t0,
    );
    let live = s.model.strategy.live.every();
    assert_eq!(slow_of(&tick(&mut s, t0 + live)), Some(false));
    let mut quick = report();
    quick.os_id = "windows".into();
    update(
        &mut s,
        Msg::LocalCollected {
            result: Ok(quick),
            elapsed: Duration::from_secs(1),
            slow: false,
        },
        t0 + live,
    );
    let r = s.model.local.report.as_ref().unwrap();
    assert_eq!(r.maintenance.updates, Some(2), "kept from the slow read");
    assert_eq!(r.maintenance.last_update.as_deref(), Some("2026-11-20"));
    assert_eq!(r.apps.as_ref().map(Vec::len), Some(1));
    // Due again after the slow interval; Refresh is always a slow read.
    let slow = s.model.strategy.slow.every();
    assert_eq!(slow_of(&tick(&mut s, t0 + slow)), Some(true));
    update(
        &mut s,
        Msg::LocalCollected {
            result: Err("busy".into()),
            elapsed: Duration::from_secs(1),
            slow: true,
        },
        t0 + slow,
    );
    assert_eq!(
        slow_of(&update(&mut s, Msg::LocalRefresh, t0 + slow)),
        Some(true)
    );
}

#[test]
fn windows_security_problems_become_a_flag() {
    // #211
    use crate::table::{FlagKind, Severity};
    use telepad_core::model::{Antivirus, FirewallProfile, Security};
    let mut r = *report();
    r.security = Some(Security {
        antivirus: vec![Antivirus {
            name: "Microsoft Defender".into(),
            on: true,
            age_days: Some(1),
            ..Default::default()
        }],
        firewall_profiles: vec![FirewallProfile {
            name: "Public".into(),
            on: true,
        }],
        app_control: Some(true),
        ..Default::default()
    });
    assert!(crate::host_flags(&r).is_empty());
    if let Some(sec) = r.security.as_mut() {
        sec.antivirus[0].age_days = Some(10);
    }
    let f = crate::host_flags(&r);
    assert_eq!(f[0].kind, FlagKind::Security);
    assert_eq!(f[0].severity, Severity::Warn, "old definitions only");
    if let Some(sec) = r.security.as_mut() {
        sec.firewall_profiles[0].on = false;
    }
    let f = crate::host_flags(&r);
    assert_eq!(f[0].severity, Severity::Danger);
    assert!(f[0].detail.contains("Firewall off for: Public"));
}
