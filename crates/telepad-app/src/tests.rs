use std::time::{Duration, Instant};

use telepad_core::collector::Update;
use telepad_core::config::{Access, HostEntry, Inventory};
use telepad_core::model::HostReport;
use telepad_core::store::{ImportMode, ImportSummary};
use telepad_core::update::{CHECK_EVERY, Check, UpdateConfig};

use crate::*;

fn host(id: i64, alias: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: format!("192.0.2.{id}"),
        ..HostEntry::new(alias)
    }
}

fn started(now: Instant) -> (State, Vec<Effect>) {
    let mut s = State::default();
    let fx = update(
        &mut s,
        Msg::Started {
            db_path: "telepad.db".into(),
            ssh_config_path: "telepad.ssh_config".into(),
            imported_from: None,
            update_config: Ok(UpdateConfig::default()),
        },
        now,
    );
    (s, fx)
}

fn loaded(s: &mut State, hosts: Vec<HostEntry>, now: Instant) -> Vec<Effect> {
    update(
        s,
        Msg::HostsLoaded {
            hosts: Ok(hosts),
            ssh_config_error: None,
        },
        now,
    )
}

fn collect_aliases(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Collect(h) => Some(h.iter().map(|h| h.alias.clone())),
            _ => None,
        })
        .flatten()
        .collect()
}

fn check(version: &str, newer: bool) -> Check {
    Check {
        version: version.into(),
        notes: None,
        page_url: String::new(),
        asset: Some(telepad_core::update::Asset {
            name: "telepad.zip".into(),
            url: "https://github.com/x/y/releases/download/v1/telepad.zip".into(),
            size: 1,
            sha256_url: "https://github.com/x/y/releases/download/v1/telepad.zip.sha256".into(),
        }),
        newer,
    }
}

#[test]
fn start_loads_hosts_checks_updates_and_polls_everything_once() {
    let t0 = Instant::now();
    let (mut s, fx) = started(t0);
    assert!(matches!(fx[0], Effect::LoadHosts));
    assert!(fx.iter().any(|e| matches!(e, Effect::CheckUpdate(_))));
    assert_eq!(s.model.update.phase, UpdatePhase::Checking);

    let mut quiet = host(3, "nas01");
    quiet.unmonitored = true;
    let fx = loaded(&mut s, vec![host(1, "web01"), host(2, "db01"), quiet], t0);
    assert_eq!(collect_aliases(&fx), ["web01", "db01"]);
    assert_eq!(s.model.hosts.len(), 3);
    assert_eq!(s.model.last_refresh, Some(t0));
}

#[test]
fn fatal_and_bad_update_settings_are_reported() {
    let mut s = State::default();
    update(&mut s, Msg::Fatal("locked".into()), Instant::now());
    assert!(s.model.fatal.as_deref().unwrap().contains("locked"));

    let mut s = State::default();
    let fx = update(
        &mut s,
        Msg::Started {
            db_path: "a".into(),
            ssh_config_path: "b".into(),
            imported_from: Some("config".into()),
            update_config: Err("bad".into()),
        },
        Instant::now(),
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::CheckUpdate(_))));
    assert!(s.model.update.error.as_deref().unwrap().contains("bad"));
    assert!(s.model.message.as_ref().unwrap().0.contains("Created"));
}

#[test]
fn reload_keeps_results_and_selection_and_polls_only_changes() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01"), host(2, "db01")], t0);
    s.model.hosts[0].report = Some(HostReport::default());
    s.model.hosts[1].report = Some(HostReport::default());
    update(&mut s, Msg::Select(Some(1)), t0);

    let mut changed = host(1, "web01");
    changed.port = 2222;
    let fx = loaded(&mut s, vec![host(3, "new01"), host(2, "db01"), changed], t0);

    assert_eq!(collect_aliases(&fx), ["new01", "web01"]);
    assert!(
        s.model.hosts[1].report.is_some(),
        "unchanged db01 keeps its report"
    );
    assert!(
        s.model.hosts[2].report.is_none(),
        "changed web01 starts over"
    );
    assert_eq!(
        s.model.selected,
        Some(1),
        "db01 stays selected at its new index"
    );
}

#[test]
fn auto_refresh_waits_for_interval_and_idle_collector() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);
    let at = |secs| t0 + Duration::from_secs(secs);

    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(59))).is_empty());
    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 1 }, at(61))).is_empty());
    assert_eq!(s.model.busy, 1);
    assert_eq!(
        collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(61))),
        ["web01"]
    );
    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(62))).is_empty());

    s.drafts.auto_refresh = false;
    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(500))).is_empty());
}

#[test]
fn update_check_repeats_every_six_hours_but_not_while_checking() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let checks = |fx: &[Effect]| {
        fx.iter()
            .filter(|e| matches!(e, Effect::CheckUpdate(_)))
            .count()
    };

    assert_eq!(
        checks(&update(&mut s, Msg::Tick { busy: 0 }, t0 + CHECK_EVERY)),
        0,
        "still checking"
    );
    update(&mut s, Msg::UpdateChecked(Err("offline".into())), t0);
    assert_eq!(s.model.update.error.as_deref(), Some("offline"));
    assert!(s.model.message.is_none(), "automatic check fails quietly");
    assert_eq!(
        checks(&update(&mut s, Msg::Tick { busy: 0 }, t0 + CHECK_EVERY / 2)),
        0
    );
    assert_eq!(
        checks(&update(&mut s, Msg::Tick { busy: 0 }, t0 + CHECK_EVERY)),
        1
    );

    update(&mut s, Msg::UpdateChecked(Err("offline".into())), t0);
    let fx = update(&mut s, Msg::UpdateCheck, t0);
    assert_eq!(checks(&fx), 1, "manual check any time");
    update(&mut s, Msg::UpdateChecked(Ok(check("0.0.1", false))), t0);
    assert!(s.model.message.as_ref().unwrap().0.contains("up to date"));
}

#[test]
fn newer_release_banner_install_and_restart() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    update(&mut s, Msg::UpdateChecked(Ok(check("9.0.0", true))), t0);
    assert_eq!(
        s.model.update.banner().map(|c| c.version.as_str()),
        Some("9.0.0")
    );

    let fx = update(&mut s, Msg::UpdateInstall, t0);
    assert!(matches!(fx[..], [Effect::InstallUpdate(_)]));
    assert!(
        s.model.update.banner().is_none(),
        "no banner while installing"
    );
    assert!(
        update(&mut s, Msg::UpdateInstall, t0).is_empty(),
        "only one install at a time"
    );

    update(&mut s, Msg::UpdateInstalled(Ok("9.0.0".into())), t0);
    assert_eq!(s.model.update.phase, UpdatePhase::Installed("9.0.0".into()));
    assert!(matches!(
        update(&mut s, Msg::Restart, t0)[..],
        [Effect::Relaunch]
    ));
}

#[test]
fn failed_install_returns_to_idle_and_later_hides_banner() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    update(&mut s, Msg::UpdateChecked(Ok(check("9.0.0", true))), t0);
    update(&mut s, Msg::UpdateInstall, t0);
    update(
        &mut s,
        Msg::UpdateInstalled(Err("hash mismatch".into())),
        t0,
    );
    assert_eq!(s.model.update.phase, UpdatePhase::Idle);
    assert!(
        s.model
            .message
            .as_ref()
            .unwrap()
            .0
            .contains("hash mismatch")
    );

    update(&mut s, Msg::UpdateWindow(true), t0);
    update(&mut s, Msg::UpdateLater, t0);
    assert!(s.model.update.banner().is_none());
    assert!(!s.model.update.window);
}

#[test]
fn update_settings_are_validated_before_saving() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    update(&mut s, Msg::UpdateChecked(Err("x".into())), t0);
    s.drafts.update_form.repo = "not a repo".into();
    assert!(update(&mut s, Msg::UpdateSaveSettings, t0).is_empty());
    assert!(s.drafts.update_form_error.is_some());

    s.drafts.update_form.repo = "  someone/fork ".into();
    let fx = update(&mut s, Msg::UpdateSaveSettings, t0);
    let Effect::SaveUpdateConfig(c) = &fx[0] else {
        panic!("{fx:?}")
    };
    assert_eq!(c.repo, "someone/fork");

    let fx = update(&mut s, Msg::UpdateSettingsSaved(Ok(c.clone())), t0);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::CheckUpdate(c) if c.repo == "someone/fork"))
    );
    assert!(s.drafts.update_form_error.is_none());
}

#[test]
fn editor_save_validates_then_saves_reloads_and_selects() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);

    update(&mut s, Msg::AddHost, t0);
    s.drafts.editor.as_mut().unwrap().entry.alias = "bad alias".into();
    assert!(update(&mut s, Msg::EditorSave, t0).is_empty());
    assert!(s.drafts.editor.as_ref().unwrap().error.is_some());

    s.drafts.editor.as_mut().unwrap().entry.alias = "db01".into();
    let fx = update(&mut s, Msg::EditorSave, t0);
    assert!(matches!(&fx[..], [Effect::SaveHost(h)] if h.alias == "db01"));

    let fx = update(&mut s, Msg::HostSaved(Ok((7, "db01".into()))), t0);
    assert!(matches!(fx[..], [Effect::LoadHosts]));
    assert!(s.drafts.editor.is_none());
    loaded(&mut s, vec![host(1, "web01"), host(7, "db01")], t0);
    assert_eq!(s.model.selected, Some(1));

    update(&mut s, Msg::Edit(0), t0);
    update(&mut s, Msg::HostSaved(Err("alias exists".into())), t0);
    assert_eq!(
        s.drafts.editor.as_ref().unwrap().error.as_deref(),
        Some("alias exists"),
        "a failed save keeps the form open"
    );
}

#[test]
fn delete_needs_confirmation_and_clears_selection() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01"), host(2, "db01")], t0);
    update(&mut s, Msg::Select(Some(1)), t0);

    assert!(update(&mut s, Msg::Delete(1), t0).is_empty());
    assert_eq!(s.model.pending_delete, Some((2, "db01".into())));
    update(&mut s, Msg::CancelDelete, t0);
    assert!(update(&mut s, Msg::ConfirmDelete, t0).is_empty());

    update(&mut s, Msg::Delete(1), t0);
    let fx = update(&mut s, Msg::ConfirmDelete, t0);
    assert!(matches!(&fx[..], [Effect::DeleteHost { id: 2, .. }]));
    let fx = update(
        &mut s,
        Msg::HostDeleted {
            id: 2,
            alias: "db01".into(),
            result: Ok(()),
        },
        t0,
    );
    assert!(matches!(fx[..], [Effect::LoadHosts]));
    assert_eq!(s.model.selected, None);
}

#[test]
fn import_previews_applies_and_keeps_dialog_on_failure() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let inv = Inventory {
        hosts: vec![HostEntry::new("web01")],
        global: String::new(),
    };

    update(
        &mut s,
        Msg::ImportOpened(Ok(("empty".into(), Inventory::default()))),
        t0,
    );
    assert!(s.model.import.is_none());
    assert!(s.model.message.as_ref().unwrap().0.contains("No hosts"));

    let fx = update(
        &mut s,
        Msg::ImportOpened(Ok(("hosts.json".into(), inv))),
        t0,
    );
    assert!(matches!(
        fx[..],
        [Effect::PreviewImport {
            mode: ImportMode::Merge,
            ..
        }]
    ));
    assert!(
        update(&mut s, Msg::ImportMode(ImportMode::Merge), t0).is_empty(),
        "same mode"
    );
    let fx = update(&mut s, Msg::ImportMode(ImportMode::Replace), t0);
    assert!(matches!(
        fx[..],
        [Effect::PreviewImport {
            mode: ImportMode::Replace,
            ..
        }]
    ));

    update(&mut s, Msg::ImportPreviewed(Err("db".into())), t0);
    assert!(
        update(&mut s, Msg::ImportApply, t0).is_empty(),
        "no apply on a failed preview"
    );
    update(
        &mut s,
        Msg::ImportPreviewed(Ok(ImportSummary::default())),
        t0,
    );
    let fx = update(&mut s, Msg::ImportApply, t0);
    assert!(matches!(
        fx[..],
        [Effect::ApplyImport {
            mode: ImportMode::Replace,
            ..
        }]
    ));

    update(&mut s, Msg::Imported(Err("constraint".into())), t0);
    let dlg = s.model.import.as_ref().unwrap();
    assert!(
        dlg.preview
            .as_ref()
            .unwrap_err()
            .contains("nothing changed")
    );

    let fx = update(&mut s, Msg::Imported(Ok(ImportSummary::default())), t0);
    assert!(matches!(fx[..], [Effect::LoadHosts]));
    assert!(s.model.import.is_none());
}

#[test]
fn collection_results_update_host_state() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);

    update(&mut s, Msg::Collected(Update::Started("web01".into())), t0);
    assert!(s.model.hosts[0].loading);
    let done = |result| Update::Done {
        alias: "web01".into(),
        result,
        elapsed: Duration::from_millis(300),
    };
    let t1 = t0 + Duration::from_secs(1);
    update(&mut s, Msg::Collected(done(Ok(Box::default()))), t1);
    let h = &s.model.hosts[0];
    assert!(!h.loading);
    assert_eq!(h.online(), Some(true));
    assert_eq!(h.updated, Some(t1));

    update(&mut s, Msg::Collected(done(Err("timed out".into()))), t1);
    assert_eq!(s.model.hosts[0].online(), Some(false));
    assert_eq!(s.model.offline_count(), 1);
}

#[test]
fn connect_uses_rdp_for_rdp_only_hosts() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let mut win = host(2, "win01");
    win.access = Access::Rdp;
    loaded(&mut s, vec![host(1, "web01"), win], t0);
    assert!(matches!(
        update(&mut s, Msg::Ssh(0), t0)[..],
        [Effect::OpenSsh(_)]
    ));
    assert!(matches!(
        update(&mut s, Msg::Ssh(1), t0)[..],
        [Effect::OpenRdp(_)]
    ));
    assert!(update(&mut s, Msg::Ssh(9), t0).is_empty());
}

#[test]
fn messages_expire() {
    let t0 = Instant::now();
    let mut s = State::default();
    update(&mut s, Msg::Notice("hello".into()), t0);
    assert_eq!(
        s.model.message_at(t0 + Duration::from_secs(1)),
        Some("hello")
    );
    assert_eq!(s.model.message_at(t0 + MESSAGE_TTL), None);
}

#[test]
fn clone_copies_only_connection_settings() {
    let src = HostEntry {
        id: Some(5),
        hostname: "192.0.2.32".into(),
        user: "admin".into(),
        port: 2222,
        identity_file: Some("~/.ssh/id_ops".into()),
        note: "web".into(),
        ..HostEntry::new("web32")
    };
    let ed = Editor::clone_of(&src);
    assert_eq!(ed.entry.id, None);
    assert_eq!(ed.entry.alias, "");
    assert_eq!(ed.entry.hostname, "");
    assert_eq!(
        (ed.entry.user.as_str(), ed.port.as_str()),
        ("admin", "2222")
    );
    assert_eq!(ed.identity, "~/.ssh/id_ops");
    assert!(ed.title().contains("cloned from web32"));
}

#[test]
fn switching_access_moves_a_default_port_only() {
    let mut ed = Editor::new(HostEntry::new("x"));
    ed.set_access(Access::Rdp);
    assert_eq!(ed.port, "3389");
    ed.port = "2222".into();
    ed.set_access(Access::Ssh);
    assert_eq!(ed.port, "2222");
}

#[test]
fn filter_matches_names_addresses_and_containers() {
    let mut h = HostState::new(host(1, "web01"));
    assert!(matches_filter("", &h));
    assert!(matches_filter("WEB", &h));
    assert!(matches_filter("192.0.2.1", &h));
    assert!(!matches_filter("nginx", &h));
    let mut r = HostReport::default();
    r.docker.containers.push(telepad_core::model::Container {
        name: "nginx-proxy".into(),
        ..Default::default()
    });
    h.report = Some(r);
    assert!(matches_filter("nginx", &h));
}
