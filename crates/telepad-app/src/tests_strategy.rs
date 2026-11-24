//! Behaviour tests for #122: pause, the Update Strategy page, ping checks and
//! last known facts kept in `update.db`.

use std::time::{Duration, Instant};

use telepad_core::collector::{Reach, Scope, Update};
use telepad_core::config::{Access, HostEntry};
use telepad_core::model::HostReport;
use telepad_core::store::UiPrefs;
use telepad_core::strategy::{Facts, Loaded, SavedFacts, SlowFacts, StaticFacts, Strategy};
use telepad_core::update::UpdateConfig;

use crate::workspace::Win;
use crate::*;

const H: u64 = 3600;

fn host(id: i64, alias: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: format!("192.0.2.{id}"),
        ..HostEntry::new(alias)
    }
}

fn start(strategy: Result<(Strategy, Vec<SavedFacts>), String>, now: Instant) -> State {
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
            strategy: strategy.map(|(strategy, facts)| Loaded {
                strategy,
                facts,
                paused_hosts: Vec::new(),
            }),
        },
        now,
    );
    s
}

fn load(s: &mut State, hosts: Vec<HostEntry>, now: Instant) -> Vec<Effect> {
    update(
        s,
        Msg::HostsLoaded {
            hosts: Ok(hosts),
            ssh_config_error: None,
        },
        now,
    )
}

fn jobs(fx: &[Effect]) -> Vec<(String, Scope)> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Collect(j) => Some(j.iter().map(|j| (j.host.alias.clone(), j.scope))),
            _ => None,
        })
        .flatten()
        .collect()
}

fn pinged(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Ping(h) => Some(h.iter().map(|h| h.host.alias.clone())),
            _ => None,
        })
        .flatten()
        .collect()
}

/// A ping round result without the SSH port check.
fn icmp(ms: Option<f32>) -> Reach {
    ms.map_or(Reach::Silent, Reach::Ping)
}

fn saved_facts(fx: &[Effect]) -> Vec<Facts> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::SaveFacts(f) => Some(f.clone()),
            _ => None,
        })
        .collect()
}

fn done(
    s: &mut State,
    alias: &str,
    r: Result<HostReport, String>,
    scope: Scope,
    now: Instant,
) -> Vec<Effect> {
    update(
        s,
        Msg::Collected(Update::Done {
            alias: alias.into(),
            result: r.map(Box::new),
            scope,
            reach: Reach::Unchecked,
            elapsed: Duration::from_millis(200),
        }),
        now,
    )
}

fn ubuntu() -> HostReport {
    HostReport {
        hostname: "web01".into(),
        os_pretty: "Ubuntu 24.04.1 LTS".into(),
        os_id: "ubuntu".into(),
        os_version_id: "24.04".into(),
        kernel: "6.8.0-45-generic".into(),
        maintenance: telepad_core::model::Maintenance {
            updates: Some(7),
            ..Default::default()
        },
        uptime_secs: Some(5000),
        ..Default::default()
    }
}

#[test]
fn last_known_facts_show_at_once_and_are_read_again_only_when_due() {
    let t0 = Instant::now();
    let r = ubuntu();
    let saved = SavedFacts {
        alias: "web01".into(),
        // Older than this computer's clock can hold, still fine.
        static_info: Some((StaticFacts::of(&r), Duration::from_secs(3 * H))),
        slow: Some((SlowFacts::of(&r), Duration::from_secs(7 * H))),
    };
    let mut s = start(Ok((Strategy::default(), vec![saved])), t0);
    let fx = load(&mut s, vec![host(1, "web01"), host(2, "db01")], t0);

    // web01: OS info from 3 h ago is kept (daily), updates from 7 h ago are due (6 h).
    let j = jobs(&fx);
    assert_eq!(
        j[0],
        (
            "web01".into(),
            Scope {
                slow: true,
                ..Scope::LIVE
            }
        )
    );
    // db01 has nothing saved: everything.
    assert_eq!(j[1], ("db01".into(), Scope::FULL));
    let h = &s.model.hosts[0];
    assert_eq!(h.online(), None, "last known facts are not \"up\"");
    assert_eq!(h.shown().unwrap().os_pretty, "Ubuntu 24.04.1 LTS");
    assert!(h.report.is_none());
    assert_eq!(
        h.static_at.unwrap().age(t0 + Duration::from_secs(60)),
        Duration::from_secs(3 * H + 60)
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::PruneFacts(a) if a == &["web01", "db01"])),
        "facts of deleted / renamed hosts are dropped"
    );

    // The first collection keeps the OS info it did not read, and saves the slow tier.
    let slow = Scope {
        slow: true,
        ..Scope::LIVE
    };
    let live = HostReport {
        uptime_secs: Some(6000),
        maintenance: telepad_core::model::Maintenance {
            updates: Some(2),
            ..Default::default()
        },
        ..Default::default()
    };
    let fx = done(&mut s, "web01", Ok(live), slow, t0);
    let h = &s.model.hosts[0];
    assert!(h.known.is_none());
    let rep = h.report.as_ref().unwrap();
    assert_eq!(rep.kernel, "6.8.0-45-generic");
    assert_eq!(rep.maintenance.updates, Some(2));
    assert!(
        rep.maintenance.end_of_support.is_some(),
        "end of support from the kept OS"
    );
    let f = saved_facts(&fx);
    assert_eq!(f.len(), 1);
    assert!(f[0].static_info.is_none());
    assert_eq!(f[0].slow.as_ref().unwrap().maintenance.updates, Some(2));

    // A full read saves both; a live-only read saves nothing.
    let fx = done(&mut s, "db01", Ok(ubuntu()), Scope::FULL, t0);
    let f = saved_facts(&fx);
    assert!(f[0].static_info.is_some() && f[0].slow.is_some());
    assert!(saved_facts(&done(&mut s, "db01", Ok(ubuntu()), Scope::LIVE, t0)).is_empty());
}

#[test]
fn a_reload_after_start_does_not_reuse_saved_facts() {
    let t0 = Instant::now();
    let saved = SavedFacts {
        alias: "web01".into(),
        static_info: Some((StaticFacts::of(&ubuntu()), Duration::from_secs(60))),
        slow: None,
    };
    let mut s = start(Ok((Strategy::default(), vec![saved])), t0);
    load(&mut s, vec![host(2, "db01")], t0);
    assert!(s.model.restored.is_empty());
    // web01 added later (or renamed back): read in full.
    let fx = load(&mut s, vec![host(2, "db01"), host(1, "web01")], t0);
    assert_eq!(jobs(&fx), [("web01".into(), Scope::FULL)]);
}

#[test]
fn pause_is_saved_and_also_stops_collection_at_start() {
    let t0 = Instant::now();
    let st = Strategy {
        paused: true,
        ..Default::default()
    };
    let mut s = start(Ok((st, vec![])), t0);
    assert_eq!(s.model.paused_at, Some(t0));
    let fx = load(&mut s, vec![host(1, "web01")], t0);
    assert!(jobs(&fx).is_empty(), "paused: no SSH, also not on start");
    let fx = update(&mut s, Msg::Tick { busy: 0 }, t0 + Duration::from_secs(600));
    assert!(jobs(&fx).is_empty() && pinged(&fx).is_empty());

    let fx = update(&mut s, Msg::SetPaused(false), t0);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::SaveStrategy(st) if !st.paused))
    );
    assert_eq!(s.model.paused_at, None);
    // Setting the same state again does nothing.
    assert!(update(&mut s, Msg::SetPaused(false), t0).is_empty());
}

#[test]
fn strategy_page_validates_saves_and_resets() {
    let t0 = Instant::now();
    let mut s = start(Ok((Strategy::default(), vec![])), t0);
    s.drafts.strategy.form.live.secs = 999;
    update(&mut s, Msg::StrategyOpen, t0);
    assert!(s.model.ws.is_open(Win::Strategy));
    assert_eq!(
        s.drafts.strategy.form,
        Strategy::default(),
        "opening shows what is saved"
    );

    s.drafts.strategy.form.ping.secs = 5;
    let fx = update(&mut s, Msg::StrategySave, t0);
    assert!(fx.is_empty());
    assert!(
        s.drafts
            .strategy
            .error
            .as_deref()
            .unwrap()
            .starts_with("Ping")
    );
    assert_eq!(s.model.strategy, Strategy::default());

    s.drafts.strategy.form.ping.secs = 20;
    s.drafts.strategy.form.slow.on = false;
    // The form never pauses (the top bar does).
    s.drafts.strategy.form.paused = true;
    assert!(s.drafts.strategy.changed(&s.model.strategy));
    let fx = update(&mut s, Msg::StrategySave, t0);
    assert!(s.drafts.strategy.error.is_none());
    assert_eq!(s.model.strategy.ping.secs, 20);
    assert!(!s.model.strategy.slow.on);
    assert!(!s.model.strategy.paused);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::SaveStrategy(st) if st.ping.secs == 20))
    );
    assert!(!s.drafts.strategy.changed(&s.model.strategy));

    update(&mut s, Msg::StrategyReset, t0);
    assert_eq!(s.drafts.strategy.form, Strategy::default());
    assert!(
        s.drafts.strategy.changed(&s.model.strategy),
        "reset still needs Save"
    );

    // The page comes back with a saved workspace.
    assert_eq!(Win::from_key("strategy"), Some(Win::Strategy));
    assert_eq!(Win::Strategy.key(), Some("strategy"));
}

#[test]
fn disabled_tiers_are_never_read_by_themselves() {
    let t0 = Instant::now();
    let st = Strategy {
        static_info: telepad_core::strategy::Every {
            on: false,
            secs: 24 * H as u32,
        },
        slow: telepad_core::strategy::Every {
            on: false,
            secs: 6 * H as u32,
        },
        ..Default::default()
    };
    let mut s = start(Ok((st, vec![])), t0);
    let fx = load(&mut s, vec![host(1, "web01")], t0);
    assert_eq!(jobs(&fx), [("web01".into(), Scope::LIVE)]);
    // Refresh all still reads everything.
    assert_eq!(
        jobs(&update(&mut s, Msg::RefreshAll, t0)),
        [("web01".into(), Scope::FULL)]
    );
}

#[test]
fn with_live_off_due_tiers_still_run_and_failing_hosts_wait_for_the_retry() {
    let t0 = Instant::now();
    let st = Strategy {
        live: telepad_core::strategy::Every {
            on: false,
            secs: 60,
        },
        ping: telepad_core::strategy::Every {
            on: false,
            secs: 30,
        },
        ..Default::default()
    };
    let mut s = start(Ok((st, vec![])), t0);
    load(&mut s, vec![host(1, "web01"), host(2, "db01")], t0);
    done(&mut s, "web01", Ok(ubuntu()), Scope::FULL, t0);
    done(
        &mut s,
        "db01",
        Err("connection refused".into()),
        Scope::FULL,
        t0,
    );

    // Nothing is due for web01; db01 failed and waits 10 min.
    assert!(
        jobs(&update(
            &mut s,
            Msg::Tick { busy: 0 },
            t0 + Duration::from_secs(120)
        ))
        .is_empty()
    );
    let at = t0 + Duration::from_secs(601);
    assert_eq!(
        jobs(&update(&mut s, Msg::Tick { busy: 0 }, at)),
        [("db01".into(), Scope::FULL)]
    );
    // 6 h later web01's slow tier is due on its own.
    done(&mut s, "db01", Ok(ubuntu()), Scope::FULL, at);
    let fx = update(
        &mut s,
        Msg::Tick { busy: 0 },
        t0 + Duration::from_secs(6 * H),
    );
    assert_eq!(
        jobs(&fx),
        [(
            "web01".into(),
            Scope {
                slow: true,
                ..Scope::LIVE
            }
        )]
    );
}

#[test]
fn ping_round_marks_down_and_brings_back() {
    let t0 = Instant::now();
    let mut s = start(Ok((Strategy::default(), vec![])), t0);
    let rdp = HostEntry {
        access: Access::Rdp,
        port: 3389,
        ..host(3, "win01")
    };
    let mut quiet = host(4, "nas01");
    quiet.unmonitored = true;
    let fx = load(
        &mut s,
        vec![host(1, "web01"), host(2, "fw01"), rdp, quiet],
        t0,
    );
    assert!(
        !jobs(&fx).iter().any(|(a, _)| a == "nas01"),
        "unmonitored hosts are left alone"
    );
    done(&mut s, "web01", Ok(ubuntu()), Scope::FULL, t0);
    done(&mut s, "fw01", Ok(ubuntu()), Scope::FULL, t0);

    // First tick: ping round for SSH hosts; RDP-only hosts get their probe.
    let t1 = t0 + Duration::from_secs(1);
    let fx = update(&mut s, Msg::Tick { busy: 1 }, t1);
    assert_eq!(pinged(&fx), ["web01", "fw01"]);
    assert_eq!(jobs(&fx), [("win01".into(), Scope::FULL)]);
    assert!(s.model.pinging);
    assert!(
        pinged(&update(
            &mut s,
            Msg::Tick { busy: 1 },
            t1 + Duration::from_secs(40)
        ))
        .is_empty(),
        "no second round while one runs"
    );
    // The live round leaves RDP-only hosts to the ping round.
    let live = jobs(&update(
        &mut s,
        Msg::Tick { busy: 0 },
        t0 + Duration::from_secs(61),
    ));
    assert!(!live.iter().any(|(a, _)| a == "win01"), "{live:?}");

    let round = |s: &mut State, web: Option<f32>, fw: Option<f32>, at: Instant| {
        update(
            s,
            Msg::Pinged(vec![("web01".into(), icmp(web)), ("fw01".into(), icmp(fw))]),
            at,
        )
    };
    // fw01 drops ICMP: never marked down by ping.
    round(&mut s, Some(1.5), None, t1);
    assert!(!s.model.pinging);
    assert_eq!(s.model.hosts[0].ping, Some(Some(1.5)));
    round(&mut s, None, None, t1);
    assert_eq!(
        s.model.hosts[0].online(),
        Some(true),
        "one miss is not down"
    );
    round(&mut s, None, None, t1);
    let web = &s.model.hosts[0];
    assert_eq!(web.online(), Some(false));
    assert!(web.no_ping);
    assert!(web.error.as_deref().unwrap().contains("ping check"));
    assert_eq!(s.model.hosts[1].online(), Some(true), "fw01 still up");

    // Back: collected at once.
    let fx = round(&mut s, Some(2.0), None, t1);
    assert_eq!(jobs(&fx).len(), 1);
    assert_eq!(jobs(&fx)[0].0, "web01");
    assert!(!s.model.hosts[0].no_ping);
    done(&mut s, "web01", Ok(ubuntu()), Scope::LIVE, t1);
    assert_eq!(s.model.hosts[0].online(), Some(true));
}

#[test]
fn without_update_db_defaults_apply_and_nothing_is_written() {
    let t0 = Instant::now();
    let mut s = start(Err("update.db: locked".into()), t0);
    assert!(!s.model.strategy_ok);
    assert!(s.model.message.as_ref().unwrap().0.contains("not saved"));
    assert_eq!(s.model.strategy, Strategy::default());
    let fx = load(&mut s, vec![host(1, "web01")], t0);
    assert!(!fx.iter().any(|e| matches!(e, Effect::PruneFacts(_))));
    assert!(saved_facts(&done(&mut s, "web01", Ok(ubuntu()), Scope::FULL, t0)).is_empty());
    let fx = update(&mut s, Msg::SetPaused(true), t0);
    assert!(
        s.model.strategy.paused,
        "pause still works for this session"
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::SaveStrategy(_))));
}

#[test]
fn hosts_are_paused_one_by_one() {
    let t0 = Instant::now();
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
            strategy: Ok(Loaded {
                paused_hosts: vec!["db01".into(), "gone01".into()],
                ..Default::default()
            }),
        },
        t0,
    );
    // Paused in update.db: not read on start, not pinged; a deleted host's pause is dropped.
    let fx = load(&mut s, vec![host(1, "web01"), host(2, "db01")], t0);
    assert_eq!(jobs(&fx), [("web01".into(), Scope::FULL)]);
    assert!(!s.model.paused_hosts.contains("gone01"));
    let at = |secs| t0 + Duration::from_secs(secs);
    let fx = update(&mut s, Msg::Tick { busy: 0 }, at(61));
    assert_eq!(
        jobs(&fx).iter().map(|j| j.0.as_str()).collect::<Vec<_>>(),
        ["web01"]
    );
    assert_eq!(pinged(&fx), ["web01"]);
    // Refresh all leaves it alone; its own Refresh reads it.
    let all = jobs(&update(&mut s, Msg::RefreshAll, at(62)));
    assert!(!all.iter().any(|(a, _)| a == "db01"), "{all:?}");
    assert_eq!(
        jobs(&update(&mut s, Msg::Refresh(1), at(62))),
        [("db01".into(), Scope::FULL)]
    );

    // Pause web01 too, then resume both: saved, and read on the next tick.
    let fx = update(
        &mut s,
        Msg::PauseHosts {
            aliases: vec!["web01".into(), "nope".into()],
            paused: true,
        },
        at(63),
    );
    assert!(
        matches!(&fx[..], [Effect::SaveHostPaused { alias, paused: true }] if alias == "web01")
    );
    let fx = update(&mut s, Msg::Tick { busy: 0 }, at(200));
    assert!(jobs(&fx).is_empty() && pinged(&fx).is_empty());
    // A ping result that arrives after pausing changes nothing.
    update(
        &mut s,
        Msg::Pinged(vec![("web01".into(), Reach::Silent)]),
        at(200),
    );
    assert_eq!(s.model.hosts[0].ping, None);
    let fx = update(
        &mut s,
        Msg::PauseHosts {
            aliases: vec!["web01".into(), "db01".into()],
            paused: false,
        },
        at(201),
    );
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::SaveHostPaused { paused: false, .. }))
            .count(),
        2
    );
    assert!(
        s.model
            .message
            .as_ref()
            .unwrap()
            .0
            .contains("Resumed 2 hosts")
    );
    let both = || Msg::PauseHosts {
        aliases: vec!["web01".into(), "db01".into()],
        paused: true,
    };
    update(&mut s, both(), at(201));
    assert!(
        s.model
            .message
            .as_ref()
            .unwrap()
            .0
            .contains("their own Refresh")
    );
    update(
        &mut s,
        Msg::PauseHosts {
            aliases: vec!["web01".into(), "db01".into()],
            paused: false,
        },
        at(201),
    );
    let fx = update(&mut s, Msg::Tick { busy: 0 }, at(202));
    assert_eq!(jobs(&fx).len(), 2);

    // Renamed: the pause follows the host.
    update(
        &mut s,
        Msg::PauseHosts {
            aliases: vec!["db01".into()],
            paused: true,
        },
        at(203),
    );
    let fx = load(&mut s, vec![host(1, "web01"), host(2, "db02")], at(204));
    assert!(s.model.paused_hosts.contains("db02") && !s.model.paused_hosts.contains("db01"));
    assert!(
        fx.iter().any(
            |e| matches!(e, Effect::SaveHostPaused { alias, paused: true } if alias == "db02")
        )
    );
    assert!(
        !jobs(&fx).iter().any(|(a, _)| a == "db02"),
        "paused: not read after the rename"
    );
}

// --- #125: hosts that drop ICMP ping -------------------------------------

fn reached(
    s: &mut State,
    alias: &str,
    r: Result<HostReport, String>,
    scope: Scope,
    reach: Reach,
    now: Instant,
) {
    update(
        s,
        Msg::Collected(Update::Done {
            alias: alias.into(),
            result: r.map(Box::new),
            scope,
            reach,
            elapsed: Duration::from_millis(200),
        }),
        now,
    );
}

fn ping_jobs(fx: &[Effect]) -> Vec<(String, bool)> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Ping(h) => Some(h.iter().map(|j| (j.host.alias.clone(), j.port))),
            _ => None,
        })
        .flatten()
        .collect()
}

const SKIPPED: Scope = Scope {
    ping_first: true,
    ..Scope::FULL
};

#[test]
fn a_host_that_drops_ping_is_tried_over_ssh_on_the_next_round() {
    let t0 = Instant::now();
    let mut s = start(Ok((Strategy::default(), vec![])), t0);
    load(&mut s, vec![host(1, "fw01")], t0);
    // A transient SSH timeout; no ping reply, but its SSH port is open.
    reached(
        &mut s,
        "fw01",
        Err("connection timed out (no ping reply, SSH port 22 open)".into()),
        Scope::FULL,
        Reach::Port,
        t0,
    );
    let h = &s.model.hosts[0];
    assert!(!h.no_ping, "up: not held back by ping");
    assert!(h.port_open);
    assert_eq!(h.ssh_fails, 1);
    // The next live round runs SSH (not the ping-first check).
    let fx = update(&mut s, Msg::Tick { busy: 0 }, t0 + Duration::from_secs(61));
    let live = jobs(&fx);
    assert_eq!(live.len(), 1);
    assert!(!live[0].1.ping_first, "{live:?}");
    // The ping round does not probe its port while it is not down.
    assert_eq!(ping_jobs(&fx), [("fw01".to_string(), false)]);
    reached(
        &mut s,
        "fw01",
        Ok(ubuntu()),
        live[0].1,
        Reach::Unchecked,
        t0 + Duration::from_secs(62),
    );
    assert_eq!(s.model.hosts[0].online(), Some(true));
    assert_eq!(s.model.hosts[0].ssh_fails, 0);
}

#[test]
fn a_host_that_is_really_down_backs_off_and_comes_back_by_its_ssh_port() {
    let t0 = Instant::now();
    let mut s = start(Ok((Strategy::default(), vec![])), t0);
    load(&mut s, vec![host(1, "fw01")], t0);
    reached(
        &mut s,
        "fw01",
        Err("connection timed out (no ping reply, SSH port 22 closed)".into()),
        Scope::FULL,
        Reach::Silent,
        t0,
    );
    assert!(s.model.hosts[0].no_ping);
    // Next live round: only the quick check, no SSH timeout.
    let t1 = t0 + Duration::from_secs(61);
    let fx1 = update(&mut s, Msg::Tick { busy: 0 }, t1);
    let live = jobs(&fx1);
    assert!(live[0].1.ping_first, "{live:?}");
    reached(
        &mut s,
        "fw01",
        Err("not reachable".into()),
        SKIPPED,
        Reach::Silent,
        t1,
    );
    assert_eq!(s.model.hosts[0].ssh_at, Some(t0), "SSH was not tried");
    assert_eq!(s.model.hosts[0].ssh_fails, 1);
    // The ping round checks its SSH port too while it is down.
    assert_eq!(ping_jobs(&fx1), [("fw01".to_string(), true)]);
    let t2 = t1 + Duration::from_secs(1);
    // The port opens: collected at once with everything.
    let fx = update(&mut s, Msg::Pinged(vec![("fw01".into(), Reach::Port)]), t2);
    assert_eq!(jobs(&fx), [("fw01".to_string(), Scope::FULL)]);
    assert!(!s.model.hosts[0].no_ping);
    assert!(s.model.hosts[0].port_open);
}

#[test]
fn a_host_that_answered_ping_before_still_goes_ping_first() {
    let t0 = Instant::now();
    let mut s = start(Ok((Strategy::default(), vec![])), t0);
    load(&mut s, vec![host(1, "web01")], t0);
    reached(
        &mut s,
        "web01",
        Ok(ubuntu()),
        Scope::FULL,
        Reach::Unchecked,
        t0,
    );
    update(
        &mut s,
        Msg::Pinged(vec![("web01".into(), Reach::Ping(1.0))]),
        t0,
    );
    assert!(s.model.hosts[0].ping_replied);
    let t1 = t0 + Duration::from_secs(61);
    reached(
        &mut s,
        "web01",
        Err("connection timed out (no ping reply, SSH port 22 closed)".into()),
        Scope::LIVE,
        Reach::Silent,
        t1,
    );
    assert!(
        s.model.hosts[0]
            .scope_at(t1 + Duration::from_secs(60), &Strategy::default())
            .ping_first
    );
    // Two silent ping checks in a row mark an up host down (#122 kept).
    reached(
        &mut s,
        "web01",
        Ok(ubuntu()),
        Scope::FULL,
        Reach::Unchecked,
        t1,
    );
    for _ in 0..2 {
        update(
            &mut s,
            Msg::Pinged(vec![("web01".into(), Reach::Silent)]),
            t1,
        );
    }
    assert_eq!(s.model.hosts[0].online(), Some(false));
    assert!(s.model.hosts[0].no_ping);
}

#[test]
fn a_host_behind_a_proxy_backs_off_after_two_ssh_failures() {
    let t0 = Instant::now();
    let mut s = start(Ok((Strategy::default(), vec![])), t0);
    let mut db = host(1, "db01");
    db.options.push(("ProxyJump".into(), "bastion".into()));
    load(&mut s, vec![db], t0);
    let fail = |s: &mut State, scope, at| {
        reached(
            s,
            "db01",
            Err("connection timed out".into()),
            scope,
            Reach::Indirect,
            at,
        )
    };
    fail(&mut s, Scope::FULL, t0);
    assert!(
        !s.model.hosts[0].no_ping,
        "one failure: tried again next round"
    );
    let t1 = t0 + Duration::from_secs(61);
    assert!(
        !s.model.hosts[0]
            .scope_at(t1, &Strategy::default())
            .ping_first
    );
    fail(&mut s, Scope::LIVE, t1);
    assert!(s.model.hosts[0].no_ping, "two in a row: back off");
    let t2 = t1 + Duration::from_secs(61);
    assert!(
        s.model.hosts[0]
            .scope_at(t2, &Strategy::default())
            .ping_first
    );
    fail(&mut s, SKIPPED, t2);
    assert!(s.model.hosts[0].no_ping, "still waiting for the SSH retry");
    // Ping rounds cannot say anything about it.
    update(
        &mut s,
        Msg::Pinged(vec![("db01".into(), Reach::Indirect)]),
        t2,
    );
    assert_eq!(s.model.hosts[0].ping, None);
    // After the SSH retry time SSH runs again.
    assert!(
        !s.model.hosts[0]
            .scope_at(
                t1 + Strategy::default().ssh_retry_every(),
                &Strategy::default()
            )
            .ping_first
    );
}
