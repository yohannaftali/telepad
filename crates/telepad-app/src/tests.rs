use std::time::{Duration, Instant};

use telepad_core::collector::{Scope, Update};
use telepad_core::config::{Access, HostEntry, Inventory};
use telepad_core::model::HostReport;
use telepad_core::store::{ImportMode, ImportSummary, UiPrefs};
use telepad_core::strategy::Strategy;
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
            ui_prefs: UiPrefs::default(),
            strategy: Ok(Default::default()),
            prefs: Ok(Default::default()),
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
            Effect::Collect(jobs) => Some(jobs.iter().map(|j| j.host.alias.clone())),
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
            ui_prefs: UiPrefs {
                compact_host_list: false,
                group_by_status: true,
                overview_cards: true,
                theme: telepad_core::store::ThemePref::Dark,
                ..Default::default()
            },
            strategy: Ok(Default::default()),
            prefs: Ok(Default::default()),
        },
        Instant::now(),
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::CheckUpdate(_))));
    assert!(s.model.update.error.as_deref().unwrap().contains("bad"));
    assert!(s.model.message.as_ref().unwrap().0.contains("Created"));
    assert!(
        !s.drafts.compact_host_list,
        "remembered view settings applied"
    );
    assert!(s.drafts.group_by_status);
    assert!(s.drafts.overview_cards);
    assert_eq!(s.drafts.theme, telepad_core::store::ThemePref::Dark);
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
fn auto_refresh_waits_for_interval_and_idle_collector_and_pauses() {
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

    // Paused: nothing runs by itself, not even ping; Refresh all still works.
    let fx = update(&mut s, Msg::SetPaused(true), at(62));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::SaveStrategy(st) if st.paused))
    );
    let fx = update(&mut s, Msg::Tick { busy: 0 }, at(500));
    assert!(collect_aliases(&fx).is_empty());
    assert!(!fx.iter().any(|e| matches!(e, Effect::Ping(_))));
    assert_eq!(
        collect_aliases(&update(&mut s, Msg::RefreshAll, at(500))),
        ["web01"]
    );
    // Resumed: what came due runs on the next tick.
    update(&mut s, Msg::SetPaused(false), at(501));
    assert_eq!(
        collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(501))),
        ["web01"]
    );
}

fn scopes(fx: &[Effect]) -> Vec<Scope> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Collect(jobs) => Some(jobs.iter().map(|j| j.scope)),
            _ => None,
        })
        .flatten()
        .collect()
}

fn finished(
    s: &mut State,
    alias: &str,
    result: Result<HostReport, String>,
    scope: Scope,
    no_ping: bool,
    now: Instant,
) {
    update(
        s,
        Msg::Collected(Update::Done {
            alias: alias.into(),
            result: result.map(Box::new),
            scope,
            no_ping,
            elapsed: Duration::from_millis(300),
        }),
        now,
    );
}

#[test]
fn automatic_refresh_reads_static_and_slow_facts_only_when_due() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    assert_eq!(
        scopes(&loaded(&mut s, vec![host(1, "web01")], t0)),
        [Scope::FULL]
    );
    let first = HostReport {
        os_pretty: "Ubuntu 24.04 LTS".into(),
        kernel: "6.8.0".into(),
        uptime_secs: Some(1000),
        maintenance: telepad_core::model::Maintenance {
            updates: Some(4),
            ..Default::default()
        },
        ..Default::default()
    };
    finished(&mut s, "web01", Ok(first), Scope::FULL, false, t0);

    // A minute later only live metrics; the kept facts survive a live result.
    let t1 = t0 + Duration::from_secs(60);
    assert_eq!(
        scopes(&update(&mut s, Msg::Tick { busy: 0 }, t1)),
        [Scope::LIVE]
    );
    let live = HostReport {
        uptime_secs: Some(1060),
        cpu_usage_pct: Some(12.0),
        ..Default::default()
    };
    finished(&mut s, "web01", Ok(live), Scope::LIVE, false, t1);
    let r = s.model.hosts[0].report.as_ref().unwrap();
    assert_eq!(r.os_pretty, "Ubuntu 24.04 LTS");
    assert_eq!(r.maintenance.updates, Some(4));
    assert_eq!(r.cpu_usage_pct, Some(12.0));

    // Clicking Refresh all reads everything.
    assert_eq!(scopes(&update(&mut s, Msg::RefreshAll, t1)), [Scope::FULL]);
    assert_eq!(scopes(&update(&mut s, Msg::Refresh(0), t1)), [Scope::FULL]);

    let h = &s.model.hosts[0];
    assert_eq!(
        h.scope_at(t0 + Strategy::default().slow.every(), &Strategy::default()),
        Scope {
            slow: true,
            ..Scope::LIVE
        }
    );
    assert_eq!(
        h.scope_at(
            t0 + Strategy::default().static_info.every(),
            &Strategy::default()
        ),
        Scope::FULL
    );
}

#[test]
fn a_reboot_reads_everything_on_the_next_round() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);
    let up = |secs| HostReport {
        uptime_secs: Some(secs),
        ..Default::default()
    };
    finished(&mut s, "web01", Ok(up(5000)), Scope::FULL, false, t0);
    let t1 = t0 + Duration::from_secs(60);
    assert_eq!(
        s.model.hosts[0].scope_at(t1, &Strategy::default()),
        Scope::LIVE
    );
    finished(&mut s, "web01", Ok(up(30)), Scope::LIVE, false, t1);
    assert_eq!(
        s.model.hosts[0].scope_at(t1, &Strategy::default()),
        Scope::FULL
    );
}

#[test]
fn a_host_without_ping_reply_is_only_pinged_until_it_answers() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);
    finished(
        &mut s,
        "web01",
        Err("connection timed out (no ping reply)".into()),
        Scope::FULL,
        true,
        t0,
    );
    let h = &s.model.hosts[0];
    assert!(h.no_ping);
    assert!(
        h.scope_at(t0 + Duration::from_secs(60), &Strategy::default())
            .ping_first
    );

    // Ping still silent: ssh was skipped, so the 10-minute ssh retry clock keeps running.
    let t1 = t0 + Duration::from_secs(60);
    let pinged = Scope {
        ping_first: true,
        ..Scope::FULL
    };
    finished(
        &mut s,
        "web01",
        Err("no ping reply".into()),
        pinged,
        true,
        t1,
    );
    assert_eq!(s.model.hosts[0].ssh_at, Some(t0));
    assert!(
        !s.model.hosts[0]
            .scope_at(
                t0 + Strategy::default().ssh_retry_every(),
                &Strategy::default()
            )
            .ping_first
    );

    // It answered ping and ssh worked: back to normal.
    finished(
        &mut s,
        "web01",
        Ok(HostReport::default()),
        Scope::FULL,
        false,
        t1,
    );
    assert!(
        !s.model.hosts[0]
            .scope_at(t1, &Strategy::default())
            .ping_first
    );
    assert_eq!(s.model.hosts[0].online(), Some(true));

    // RDP-only hosts are already just ping + port probe.
    let mut rdp = host(2, "win01");
    rdp.access = Access::Rdp;
    assert_eq!(
        HostState::new(rdp).scope_at(t1, &Strategy::default()),
        Scope::FULL
    );
}

#[test]
fn docker_overview_groups_containers_of_all_hosts_by_stack() {
    use telepad_core::model::Container;
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(
        &mut s,
        vec![host(1, "docker01"), host(2, "docker02"), host(3, "db01")],
        t0,
    );
    let c = |name: &str, stack: &str, running: bool| Container {
        name: name.into(),
        stack: stack.into(),
        image: format!("example/{name}"),
        state: if running { "running" } else { "exited" }.into(),
        ..Default::default()
    };
    let with = |cs: Vec<Container>| {
        let mut r = HostReport::default();
        r.docker.containers = cs;
        r
    };
    s.model.hosts[0].report = Some(with(vec![c("web", "shop", true), c("cron", "", true)]));
    s.model.hosts[1].report = Some(with(vec![
        c("db", "shop", false),
        c("grafana", "monitoring", true),
    ]));

    let rows = docker_stacks(&s.model, "");
    let names: Vec<&str> = rows.iter().map(|r| r.name).collect();
    assert_eq!(names, ["monitoring", "shop", ""]);
    let shop = &rows[1];
    assert_eq!(shop.hosts(), [0, 1]);
    assert_eq!((shop.running(), shop.containers.len()), (1, 2));

    // Search: stack, container, image or host.
    let names = |f: &str| -> Vec<String> {
        docker_stacks(&s.model, f)
            .iter()
            .map(|r| r.name.to_string())
            .collect()
    };
    assert_eq!(names("graf"), ["monitoring"]);
    assert_eq!(names("docker01"), ["shop", ""]);
    assert!(names("nothing").is_empty());

    // The overview buttons open their page as a window; the host window stays.
    update(&mut s, Msg::Select(Some(2)), t0);
    update(&mut s, Msg::ShowOverview(OverviewPage::Docker), t0);
    assert_eq!(
        (s.model.selected, s.model.overview),
        (Some(2), OverviewPage::Docker)
    );
    assert!(s.model.ws.is_open(crate::workspace::Win::Docker));
    assert!(s.model.ws.is_open(crate::workspace::Win::Host));
    update(&mut s, Msg::ShowOverview(OverviewPage::Hosts), t0);
    assert_eq!(s.model.overview, OverviewPage::Hosts);

    // Narrow image column: no docker.io/ prefix, untagged image IDs cut like `docker ps`.
    let img = |image: &str| Container {
        image: image.into(),
        ..Default::default()
    };
    assert_eq!(
        img("docker.io/library/nginx:stable").short_image(),
        "library/nginx:stable"
    );
    assert_eq!(
        img("sha256:e00cccc803a80012cbaddb6450b8").short_image(),
        "e00cccc803a8…"
    );
    assert_eq!(img("ghcr.io/org/app:1").short_image(), "ghcr.io/org/app:1");
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
        checks(&update(
            &mut s,
            Msg::Tick { busy: 0 },
            t0 + telepad_core::update::RETRY_AFTER_FAILURE / 2
        )),
        0,
        "a failed check is retried after 30 minutes, not sooner"
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
        ..Default::default()
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
fn pasted_text_goes_to_the_import_preview_with_skipped_lines() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    update(&mut s, Msg::PasteImport, t0);
    assert!(s.drafts.paste.is_some());

    // Clipboard text is appended on its own line.
    assert!(matches!(
        update(&mut s, Msg::PasteFromClipboard, t0)[..],
        [Effect::ReadClipboard]
    ));
    s.drafts.paste.as_mut().unwrap().text = "web01 192.0.2.10".into();
    update(
        &mut s,
        Msg::ClipboardRead(Ok("web02 192.0.2.11".into())),
        t0,
    );
    assert_eq!(
        s.drafts.paste.as_ref().unwrap().text,
        "web01 192.0.2.10\nweb02 192.0.2.11"
    );

    // Nothing readable: stay in the window and say why.
    s.drafts.paste.as_mut().unwrap().text = "one two three".into();
    assert!(update(&mut s, Msg::PasteApply, t0).is_empty());
    let p = s.drafts.paste.as_ref().unwrap();
    assert_eq!(p.problems[0], "No hosts found in the text");
    assert!(p.problems[1].starts_with("line 1:"));

    // Readable hosts go to the usual preview; bad lines are listed there.
    s.drafts.paste.as_mut().unwrap().text = "web01 192.0.2.10\nx y z\n".into();
    let fx = update(&mut s, Msg::PasteApply, t0);
    assert!(matches!(fx[..], [Effect::PreviewImport { .. }]));
    assert!(s.drafts.paste.is_none());
    let dlg = s.model.import.as_ref().unwrap();
    assert_eq!(dlg.inventory.hosts.len(), 1);
    assert_eq!(dlg.skipped.len(), 1);

    update(&mut s, Msg::PasteImport, t0);
    update(&mut s, Msg::PasteCancel, t0);
    assert!(s.drafts.paste.is_none());
}

#[test]
fn dragging_a_host_reorders_keeps_selection_and_saves_the_order() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(
        &mut s,
        vec![host(1, "a"), host(2, "b"), host(3, "c"), host(4, "d")],
        t0,
    );
    s.model.hosts[2].loading = true;
    update(&mut s, Msg::Select(Some(2)), t0);
    let names = |s: &State| -> Vec<String> {
        s.model
            .hosts
            .iter()
            .map(|h| h.entry.alias.clone())
            .collect()
    };

    // c dropped on the upper half of a → first.
    let fx = update(
        &mut s,
        Msg::MoveHost {
            from: 2,
            to: drop_index(2, 0, false),
        },
        t0,
    );
    assert_eq!(names(&s), ["c", "a", "b", "d"]);
    assert!(matches!(&fx[..], [Effect::SaveOrder(ids)] if *ids == [3, 1, 2, 4]));
    assert_eq!(s.model.selected, Some(0));
    assert!(s.model.hosts[0].loading, "state moves with the host");

    // a dropped on the lower half of d → last.
    update(
        &mut s,
        Msg::MoveHost {
            from: 1,
            to: drop_index(1, 3, true),
        },
        t0,
    );
    assert_eq!(names(&s), ["c", "b", "d", "a"]);
    assert_eq!(s.model.selected, Some(0));

    // Out of range or onto itself: nothing.
    assert!(update(&mut s, Msg::MoveHost { from: 9, to: 0 }, t0).is_empty());
    assert!(update(&mut s, Msg::MoveHost { from: 1, to: 1 }, t0).is_empty());

    // A failed save reloads the database order.
    let fx = update(&mut s, Msg::OrderSaved(Err("disk full".into())), t0);
    assert!(matches!(fx[..], [Effect::LoadHosts]));
    assert!(s.model.message_at(t0).unwrap().contains("disk full"));
}

#[test]
fn drop_index_and_when_reordering_is_offered() {
    assert_eq!(drop_index(0, 2, false), 1);
    assert_eq!(drop_index(0, 2, true), 2);
    assert_eq!(drop_index(3, 0, false), 0);
    assert_eq!(drop_index(3, 0, true), 1);
    assert_eq!(drop_index(2, 2, true), 2);
    assert!(can_reorder("", false, Sort::default()));
    assert!(!can_reorder("web", false, Sort::default()));
    assert!(!can_reorder("", true, Sort::default()));
    let sorted = Sort {
        key: Some(SortKey::Cpu),
        descending: true,
    };
    assert!(!can_reorder("", false, sorted));
}

#[test]
fn import_from_user_ssh_config_reads_it_without_a_dialog() {
    let mut s = State::default();
    let fx = update(&mut s, Msg::ImportSshConfig, Instant::now());
    assert!(matches!(fx[..], [Effect::ReadUserSshConfig]));
    assert!(matches!(
        update(&mut s, Msg::Import, Instant::now())[..],
        [Effect::PickImportFile]
    ));
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
        scope: Scope::FULL,
        no_ping: false,
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

fn report(cpu: f32, mem_used_pct: f32, temp: Option<f32>, running: usize) -> HostReport {
    use telepad_core::model::{Container, Memory, Sensor, SensorKind};
    let mut r = HostReport {
        cpu_usage_pct: Some(cpu),
        memory: Some(Memory {
            total_kb: 1000,
            available_kb: (1000.0 - mem_used_pct * 10.0) as u64,
            ..Default::default()
        }),
        os_pretty: "Ubuntu 24.04".into(),
        ..Default::default()
    };
    if let Some(t) = temp {
        r.sensors.push(Sensor {
            kind: SensorKind::Temperature,
            chip: "coretemp".into(),
            label: "Package".into(),
            value: t,
        });
    }
    for _ in 0..running {
        r.docker.containers.push(Container {
            state: "running".into(),
            ..Default::default()
        });
    }
    r
}

fn sort_fixture() -> State {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let mut a = host(10, "web10");
    a.hostname = "192.0.2.10".into();
    a.note = "Web".into();
    let mut b = host(9, "db09");
    b.hostname = "192.0.2.9".into();
    let mut c = host(2, "nas02");
    c.hostname = "nas.example".into();
    c.note = "Archive".into();
    loaded(&mut s, vec![a, b, c], t0);
    s.model.hosts[0].report = Some(report(80.0, 20.0, Some(70.0), 3));
    s.model.hosts[0].updated = Some(t0);
    s.model.hosts[1].report = Some(report(5.0, 90.0, None, 0));
    s.model.hosts[1].updated = Some(t0 + Duration::from_secs(5));
    s.model.hosts[2].error = Some("connection timed out".into());
    s
}

fn order(s: &State) -> Vec<&str> {
    overview_rows(&s.model, &s.drafts.filter, s.drafts.sort)
        .into_iter()
        .map(|i| s.model.hosts[i].entry.alias.as_str())
        .collect()
}

#[test]
fn header_clicks_cycle_natural_reversed_and_inventory_order() {
    let t0 = Instant::now();
    let mut s = sort_fixture();
    assert_eq!(order(&s), ["web10", "db09", "nas02"], "inventory order");

    update(&mut s, Msg::SortBy(SortKey::Host), t0);
    assert_eq!(order(&s), ["db09", "nas02", "web10"]);
    assert_eq!(s.drafts.sort.direction(SortKey::Host), Some(false));
    update(&mut s, Msg::SortBy(SortKey::Host), t0);
    assert_eq!(order(&s), ["web10", "nas02", "db09"]);
    assert_eq!(s.drafts.sort.direction(SortKey::Host), Some(true));
    update(&mut s, Msg::SortBy(SortKey::Host), t0);
    assert_eq!(s.drafts.sort, Sort::default());
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);

    update(&mut s, Msg::SortBy(SortKey::Cpu), t0);
    assert!(s.drafts.sort.descending, "numbers start highest first");
    assert_eq!(s.drafts.sort.direction(SortKey::Host), None);
}

#[test]
fn numeric_columns_sort_by_value_and_missing_values_go_last() {
    let mut s = sort_fixture();
    let by = |s: &mut State, key, descending| {
        s.drafts.sort = Sort {
            key: Some(key),
            descending,
        };
    };
    by(&mut s, SortKey::Cpu, true);
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);
    by(&mut s, SortKey::Cpu, false);
    assert_eq!(
        order(&s),
        ["db09", "web10", "nas02"],
        "no data stays last both ways"
    );
    by(&mut s, SortKey::Memory, true);
    assert_eq!(order(&s), ["db09", "web10", "nas02"]);
    by(&mut s, SortKey::Temp, true);
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);
    by(&mut s, SortKey::Docker, true);
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);
    by(&mut s, SortKey::Updated, true);
    assert_eq!(order(&s), ["db09", "web10", "nas02"], "most recent first");
    by(&mut s, SortKey::Status, false);
    assert_eq!(order(&s)[0], "nas02", "down hosts first");
    by(&mut s, SortKey::Description, false);
    assert_eq!(
        order(&s),
        ["nas02", "web10", "db09"],
        "empty description last"
    );
}

#[test]
fn addresses_sort_numerically_and_filter_applies_before_sort() {
    let mut s = sort_fixture();
    s.drafts.sort = Sort {
        key: Some(SortKey::Address),
        descending: false,
    };
    assert_eq!(
        order(&s),
        ["db09", "web10", "nas02"],
        "192.0.2.9 < 192.0.2.10 < names"
    );
    s.drafts.filter = "192.0.2".into();
    assert_eq!(order(&s), ["db09", "web10"]);
}

#[test]
fn host_list_view_toggles_are_saved_once_per_change() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    assert!(s.drafts.compact_host_list, "compact by default");
    assert!(
        update(&mut s, Msg::SetCompactHostList(true), t0).is_empty(),
        "no change, no save"
    );
    let fx = update(&mut s, Msg::SetCompactHostList(false), t0);
    assert!(
        matches!(&fx[..], [Effect::SaveUiPrefs(p)] if !p.compact_host_list && !p.group_by_status)
    );
    let fx = update(&mut s, Msg::SetGroupByStatus(true), t0);
    assert!(
        matches!(&fx[..], [Effect::SaveUiPrefs(p)] if !p.compact_host_list && p.group_by_status)
    );
    update(&mut s, Msg::UiPrefsSaved(Err("read-only".into())), t0);
    assert!(s.model.message.as_ref().unwrap().0.contains("read-only"));
}

#[test]
fn host_list_groups_by_status_down_first_and_keeps_filter() {
    let mut s = sort_fixture();
    // web10 and db09 up (reports), nas02 down (error), plus one not checked yet
    s.model.hosts.push(HostState::new(host(4, "new04")));
    let all = host_list_groups(&s.model, "", false);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, None);
    assert_eq!(all[0].1.len(), 4);

    let g = host_list_groups(&s.model, "", true);
    let names: Vec<(&str, Vec<&str>)> = g
        .iter()
        .map(|(grp, rows)| {
            (
                grp.unwrap().title(),
                rows.iter()
                    .map(|&i| s.model.hosts[i].entry.alias.as_str())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("Down", vec!["nas02"]),
            ("Up", vec!["web10", "db09"]),
            ("Not checked yet", vec!["new04"]),
        ]
    );
    let filtered = host_list_groups(&s.model, "web", true);
    assert_eq!(filtered.len(), 1, "empty groups are left out");
    assert_eq!(filtered[0].0, Some(HostGroup::Up));
}

#[test]
fn worst_usage_picks_the_highest_of_cpu_ram_and_disk() {
    let r = report(30.0, 90.0, None, 0);
    assert_eq!(worst_usage(&r), Some(("RAM", 90.0)));
    let r = report(95.0, 10.0, None, 0);
    assert_eq!(worst_usage(&r).map(|w| w.0), Some("CPU"));
    assert_eq!(worst_usage(&HostReport::default()), None);
}

#[test]
fn overview_card_view_is_saved_and_sort_can_be_picked() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    assert!(!s.drafts.overview_cards, "table by default");
    let fx = update(&mut s, Msg::SetOverviewCards(true), t0);
    assert!(matches!(&fx[..], [Effect::SaveUiPrefs(p)] if p.overview_cards && p.compact_host_list));
    assert!(update(&mut s, Msg::SetOverviewCards(true), t0).is_empty());

    let pick = Sort {
        key: Some(SortKey::Disk),
        descending: true,
    };
    assert!(
        update(&mut s, Msg::SetSort(pick), t0).is_empty(),
        "view-only, nothing saved"
    );
    assert_eq!(s.drafts.sort, pick);
    assert_eq!(SortKey::ALL.len(), 12);
    assert!(SortKey::ALL.iter().all(|k| !k.title().is_empty()));
}

#[test]
fn editor_offers_keys_from_ssh_dir_and_browse_starts_at_current_key() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let fx = update(&mut s, Msg::AddHost, t0);
    assert!(matches!(fx[..], [Effect::FindKeys]));
    update(
        &mut s,
        Msg::KeysFound(vec!["~/.ssh/id_ed25519".into(), "~/.ssh/work".into()]),
        t0,
    );
    assert_eq!(s.drafts.editor.as_ref().unwrap().key_choices.len(), 2);

    s.drafts.editor.as_mut().unwrap().identity = "~/.ssh/work".into();
    let fx = update(&mut s, Msg::EditorBrowseKey, t0);
    assert!(matches!(&fx[..], [Effect::PickKeyFile(p)] if p == "~/.ssh/work"));

    update(&mut s, Msg::EditorClose, t0);
    update(&mut s, Msg::KeysFound(vec!["x".into()]), t0);
    assert!(
        s.drafts.editor.is_none(),
        "late result without an editor is ignored"
    );
}

#[test]
fn maintenance_flags_are_ordered_by_severity_and_sortable() {
    use telepad_core::model::Maintenance;
    let today = Some("2026-11-17".to_string());
    let eol = Maintenance {
        today: today.clone(),
        reboot_required: Some(true),
        reboot_packages: vec!["linux-base".into()],
        updates: Some(98),
        security_updates: Some(6),
        end_of_support: Some("2021-04-30".into()),
    };
    let f = maintenance_flags(&eol);
    let kinds: Vec<FlagKind> = f.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        [FlagKind::EndOfSupport, FlagKind::Updates, FlagKind::Reboot]
    );
    assert_eq!(f[0].severity, Severity::Danger);
    assert!(f[0].detail.contains("2021-04-30"));
    assert_eq!(f[1].short, "98 (6 sec)");
    assert!(f[2].detail.contains("linux-base"));

    let soon = Maintenance {
        today: today.clone(),
        end_of_support: Some("2027-01-31".into()),
        updates: Some(3),
        security_updates: Some(0),
        ..Default::default()
    };
    let f = maintenance_flags(&soon);
    assert_eq!(
        (f[0].kind, f[0].severity),
        (FlagKind::EndOfSupport, Severity::Warn)
    );
    assert_eq!((f[1].short.as_str(), f[1].severity), ("3", Severity::Info));

    let fine = Maintenance {
        today,
        reboot_required: Some(false),
        updates: Some(0),
        end_of_support: Some("2030-01-01".into()),
        ..Default::default()
    };
    assert!(maintenance_flags(&fine).is_empty());
    assert!(urgency(&eol) > urgency(&soon) && urgency(&soon) > urgency(&fine));
    assert!(SortKey::Flags.starts_descending());
}

#[test]
fn theme_choice_is_saved_once_per_change() {
    use telepad_core::store::ThemePref;
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    assert_eq!(
        s.drafts.theme,
        ThemePref::System,
        "follow the OS by default"
    );
    assert!(update(&mut s, Msg::SetTheme(ThemePref::System), t0).is_empty());
    let fx = update(&mut s, Msg::SetTheme(ThemePref::Light), t0);
    assert!(matches!(&fx[..], [Effect::SaveUiPrefs(p)] if p.theme == ThemePref::Light));
    assert_eq!(ThemePref::parse("dark"), ThemePref::Dark);
    assert_eq!(ThemePref::parse("bogus"), ThemePref::System);
}

#[test]
fn docker_actions_open_terminals_and_portainer_stacks_are_refused() {
    use telepad_core::docker_actions::{ContainerAction, RunInfo, StackAction};
    use telepad_core::model::{Container, HostReport};
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let id = |n: u8| format!("{:064x}", n);
    let compose = |name: &str, n: u8| Container {
        id: id(n),
        name: name.into(),
        state: "running".into(),
        stack: "shop".into(),
        working_dir: "/srv/shop".into(),
        config_files: "/srv/shop/compose.yml".into(),
        ..Default::default()
    };
    let portainer = Container {
        id: id(3),
        name: "erp-web".into(),
        state: "running".into(),
        stack: "erp".into(),
        working_dir: "/data/compose/12".into(),
        config_files: "/data/compose/12/docker-compose.yml".into(),
        ..Default::default()
    };
    let mut r = HostReport::default();
    r.docker.containers = vec![compose("shop-web", 1), compose("shop-db", 2), portainer];
    let mut h = HostState::new(host(1, "docker01"));
    h.report = Some(r);
    s.model.hosts = vec![h];

    let fx = update(
        &mut s,
        Msg::DockerAction {
            host: 0,
            container: id(2),
            action: ContainerAction::Restart,
        },
        t0,
    );
    assert!(
        matches!(&fx[..], [Effect::OpenDockerAction(h, p)] if h.alias == "docker01" && p.display() == format!("docker restart {}", id(2)))
    );

    let fx = update(
        &mut s,
        Msg::StackAction {
            host: 0,
            stack: "shop".into(),
            action: StackAction::Up,
        },
        t0,
    );
    assert!(
        matches!(&fx[..], [Effect::OpenDockerAction(_, p)] if p.display().ends_with("-f /srv/shop/compose.yml up -d"))
    );

    let fx = update(
        &mut s,
        Msg::StackAction {
            host: 0,
            stack: "erp".into(),
            action: StackAction::Down,
        },
        t0,
    );
    assert!(fx.is_empty(), "Portainer stacks are changed in Portainer");
    assert!(s.model.message.as_ref().unwrap().0.contains("Portainer"));

    let fx = update(
        &mut s,
        Msg::ShowRunInfo {
            host: 0,
            container: id(1),
        },
        t0,
    );
    assert!(matches!(&fx[..], [Effect::FetchRunInfo(_, c)] if c.name == "shop-web"));
    let stale = RunInfo {
        container: "other".into(),
        ..Default::default()
    };
    update(&mut s, Msg::RunInfoLoaded(stale), t0);
    assert!(matches!(
        s.drafts.run_info,
        Some(RunInfoView::Loading { .. })
    ));
    let ready = RunInfo {
        container: "shop-web".into(),
        docker_run: "docker run".into(),
        ..Default::default()
    };
    update(&mut s, Msg::RunInfoLoaded(ready.clone()), t0);
    assert_eq!(s.drafts.run_info, Some(RunInfoView::Ready(ready)));
    update(&mut s, Msg::CloseRunInfo, t0);
    assert_eq!(s.drafts.run_info, None);
}

#[test]
fn swarm_actions_cluster_flag_and_cluster_shown_once() {
    use telepad_core::cluster::{Cluster, Swarm, SwarmNode, SwarmService};
    use telepad_core::docker_actions::{NodeAction, ServiceAction};
    use telepad_core::model::HostReport;
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let swarm = |manager: bool| Swarm {
        manager,
        cluster_id: "c1u5t3r".into(),
        nodes: vec![SwarmNode {
            id: "a1b2c3d4e5f6g7h8i9j0klmno".into(),
            hostname: "worker-02".into(),
            status: "Ready".into(),
            availability: "Active".into(),
            ..Default::default()
        }],
        services: vec![SwarmService {
            name: "shop_web".into(),
            running: 1,
            desired: 3,
            ..Default::default()
        }],
        ..Default::default()
    };
    let report = |manager: bool| HostReport {
        cluster: Cluster {
            swarm: Some(swarm(manager)),
            ..Default::default()
        },
        ..Default::default()
    };
    for (i, (alias, manager)) in [("mgr-a", true), ("mgr-b", true), ("worker", false)]
        .into_iter()
        .enumerate()
    {
        let mut h = HostState::new(host(i as i64 + 1, alias));
        h.report = Some(report(manager));
        s.model.hosts.push(h);
    }
    let first = s.model.hosts.len() - 3;

    let flags = host_flags(s.model.hosts[first].report.as_ref().unwrap());
    assert_eq!(flags[0].kind, FlagKind::Cluster);
    assert!(flags[0].detail.contains("shop_web has 1/3"));

    assert_eq!(cluster_shown_by(&s.model, first), None);
    assert_eq!(
        cluster_shown_by(&s.model, first + 1),
        Some(first),
        "same cluster ID"
    );
    assert_eq!(
        cluster_shown_by(&s.model, first + 2),
        None,
        "workers have no cluster key"
    );

    let fx = update(
        &mut s,
        Msg::SwarmService {
            host: first,
            service: "shop_web".into(),
            action: ServiceAction::Restart,
        },
        t0,
    );
    assert!(
        matches!(&fx[..], [Effect::OpenDockerAction(h, p)] if h.alias == "mgr-a" && p.display() == "docker service update --force shop_web")
    );
    let fx = update(
        &mut s,
        Msg::SwarmNode {
            host: first,
            node_id: "a1b2c3d4e5f6g7h8i9j0klmno".into(),
            action: NodeAction::Drain,
        },
        t0,
    );
    assert_eq!(fx.len(), 1);
    let fx = update(
        &mut s,
        Msg::SwarmService {
            host: first + 2,
            service: "shop_web".into(),
            action: ServiceAction::Scale,
        },
        t0,
    );
    assert!(fx.is_empty(), "workers cannot change the swarm");
    let fx = update(
        &mut s,
        Msg::SwarmStackRemove {
            host: first,
            stack: "nope".into(),
        },
        t0,
    );
    assert!(fx.is_empty());
    assert!(s.model.message.as_ref().unwrap().0.contains("gone"));
}

#[test]
fn failed_update_check_is_retried_after_half_an_hour() {
    use telepad_core::update::RETRY_AFTER_FAILURE;
    let t0 = Instant::now();
    let (mut s, fx) = started(t0);
    assert!(fx.iter().any(|e| matches!(e, Effect::CheckUpdate(_))));
    update(
        &mut s,
        Msg::UpdateChecked(Err("GitHub rate limit reached".into())),
        t0,
    );
    assert!(
        s.model.message.is_none(),
        "automatic failures stay quiet in the status bar"
    );
    let again = |s: &mut State, t| {
        update(s, Msg::Tick { busy: 0 }, t)
            .iter()
            .any(|e| matches!(e, Effect::CheckUpdate(_)))
    };
    assert!(!again(&mut s, t0 + RETRY_AFTER_FAILURE / 2));
    assert!(again(&mut s, t0 + RETRY_AFTER_FAILURE));
}

#[test]
fn automatic_install_only_when_enabled_and_installable() {
    use telepad_core::update::{Asset, Check};
    let t0 = Instant::now();
    let check = |asset: bool| Check {
        version: "99.0.0".into(),
        notes: None,
        page_url: String::new(),
        asset: asset.then(|| Asset {
            name: "telepad-99.0.0.zip".into(),
            url: "https://github.com/yohannaftali/telepad/releases/download/v99.0.0/x.zip".into(),
            size: 0,
            sha256_url:
                "https://github.com/yohannaftali/telepad/releases/download/v99.0.0/x.zip.sha256"
                    .into(),
        }),
        newer: true,
    };
    let installs = |fx: &[Effect]| fx.iter().any(|e| matches!(e, Effect::InstallUpdate(_)));

    // Off (default): only the banner.
    let (mut s, _) = started(t0);
    assert!(!installs(&update(
        &mut s,
        Msg::UpdateChecked(Ok(check(true))),
        t0
    )));
    assert!(s.model.update.banner().is_some());

    // On: installs in the background, without opening the window.
    let (mut s, _) = started(t0);
    s.model.update.config.as_mut().unwrap().auto_install = true;
    assert!(installs(&update(
        &mut s,
        Msg::UpdateChecked(Ok(check(true))),
        t0
    )));
    assert_eq!(s.model.update.phase, UpdatePhase::Installing);
    update(&mut s, Msg::UpdateInstalled(Ok("99.0.0".into())), t0);
    assert_eq!(
        s.model.update.phase,
        UpdatePhase::Installed("99.0.0".into())
    );
    assert!(!s.model.update.window);

    // On, but no download for this OS: nothing to install.
    let (mut s, _) = started(t0);
    s.model.update.config.as_mut().unwrap().auto_install = true;
    assert!(!installs(&update(
        &mut s,
        Msg::UpdateChecked(Ok(check(false))),
        t0
    )));
}

/// A made-up ed25519 public key (never a real one).
const FAKE_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcH alice@example.com";

fn keys_state(t0: Instant) -> State {
    let (mut s, _) = started(t0);
    let mut win = host(4, "win01");
    win.access = Access::Rdp;
    loaded(
        &mut s,
        vec![host(1, "web01"), host(2, "web02"), host(3, "db01"), win],
        t0,
    );
    update(&mut s, Msg::KeysOpen, t0);
    s
}

#[test]
fn key_window_selects_all_listed_ssh_hosts_and_toggles() {
    let t0 = Instant::now();
    let mut s = keys_state(t0);
    let sel = |s: &State| s.drafts.keys.as_ref().unwrap().selected.clone();

    update(&mut s, Msg::KeysSelectAll(true), t0);
    assert_eq!(
        sel(&s),
        [1, 2, 3].into(),
        "RDP-only win01 is not selectable"
    );

    update(&mut s, Msg::KeysSelectAll(false), t0);
    assert!(sel(&s).is_empty());

    // Select all respects the filter.
    s.drafts.keys.as_mut().unwrap().filter = "web".into();
    update(&mut s, Msg::KeysSelectAll(true), t0);
    assert_eq!(sel(&s), [1, 2].into());

    update(&mut s, Msg::KeysToggle(3), t0);
    update(&mut s, Msg::KeysToggle(1), t0);
    update(&mut s, Msg::KeysToggle(4), t0);
    update(&mut s, Msg::KeysToggle(99), t0);
    assert_eq!(sel(&s), [2, 3].into());
}

#[test]
fn key_run_needs_a_valid_key_hosts_and_the_typed_word() {
    use telepad_core::authorized_keys::{KeyAction, KeyOutcome};
    let t0 = Instant::now();
    let mut s = keys_state(t0);

    // No key / no hosts: no confirmation.
    update(&mut s, Msg::KeysSelectAll(true), t0);
    update(&mut s, Msg::KeysAsk(KeyAction::Revoke), t0);
    assert_eq!(s.drafts.keys.as_ref().unwrap().confirm, None);
    s.drafts.keys.as_mut().unwrap().text = FAKE_KEY.into();
    update(&mut s, Msg::KeysSelectAll(false), t0);
    update(&mut s, Msg::KeysAsk(KeyAction::Revoke), t0);
    assert_eq!(s.drafts.keys.as_ref().unwrap().confirm, None);

    update(&mut s, Msg::KeysToggle(3), t0);
    update(&mut s, Msg::KeysToggle(1), t0);
    update(&mut s, Msg::KeysAsk(KeyAction::Revoke), t0);
    assert_eq!(
        s.drafts.keys.as_ref().unwrap().confirm,
        Some(KeyAction::Revoke)
    );

    // Wrong word: nothing runs.
    s.drafts.keys.as_mut().unwrap().typed = "add".into();
    assert!(update(&mut s, Msg::KeysRun, t0).is_empty());
    s.drafts.keys.as_mut().unwrap().typed = " revoke ".into();
    let fx = update(&mut s, Msg::KeysRun, t0);
    let [
        Effect::ApplyKey {
            action, key, hosts, ..
        },
    ] = &fx[..]
    else {
        panic!("{fx:?}")
    };
    assert_eq!(*action, KeyAction::Revoke);
    assert_eq!(key.comment, "alice@example.com");
    let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
    assert_eq!(aliases, ["web01", "db01"], "list order");
    assert_eq!(s.drafts.keys.as_ref().unwrap().confirm, None);
    let run = s.model.key_run.as_ref().unwrap();
    assert!(run.running());
    assert!(run.fingerprint.starts_with("SHA256:"));

    // While it runs, no second run.
    update(&mut s, Msg::KeysAsk(KeyAction::Add), t0);
    assert_eq!(s.drafts.keys.as_ref().unwrap().confirm, None);

    // Results arrive in any order; the last one reports a summary.
    update(
        &mut s,
        Msg::KeyResult {
            alias: "db01".into(),
            outcome: KeyOutcome::Removed(1),
        },
        t0,
    );
    assert!(s.model.key_run.as_ref().unwrap().running());
    update(
        &mut s,
        Msg::KeyResult {
            alias: "web01".into(),
            outcome: KeyOutcome::Failed("connection timed out".into()),
        },
        t0,
    );
    let run = s.model.key_run.as_ref().unwrap();
    assert!(!run.running());
    assert_eq!(run.results[1].1, Some(KeyOutcome::Removed(1)));
    let msg = s.model.message_at(t0).unwrap();
    assert!(msg.contains("1 of 2") && msg.contains("1 failed"), "{msg}");

    // Closing a finished run clears it.
    update(&mut s, Msg::KeysClose, t0);
    assert!(s.drafts.keys.is_none() && s.model.key_run.is_none());
}

#[test]
fn cluster_pages_group_hosts_by_cluster() {
    use telepad_core::cluster::{Cluster, Kube, Swarm};
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(
        &mut s,
        vec![
            host(1, "mgr1"),
            host(2, "mgr2"),
            host(3, "worker"),
            host(4, "k3s1"),
        ],
        t0,
    );
    let swarm = |manager: bool| Cluster {
        swarm: Some(Swarm {
            manager,
            cluster_id: if manager {
                "swarm-a".into()
            } else {
                String::new()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let report = |c: Cluster| {
        Some(HostReport {
            cluster: c,
            ..Default::default()
        })
    };
    s.model.hosts[0].report = report(swarm(true));
    s.model.hosts[1].report = report(swarm(true));
    s.model.hosts[2].report = report(swarm(false));
    s.model.hosts[3].report = report(Cluster {
        kube: Some(Kube {
            tool: "k3s".into(),
            cluster_id: "kube-b".into(),
            ..Default::default()
        }),
        ..Default::default()
    });
    let sw = cluster_rows(&s.model, ClusterKind::Swarm);
    assert_eq!(sw.len(), 1, "two managers, one swarm; the worker is a node");
    assert_eq!((sw[0].host, sw[0].hosts.clone()), (0, vec![0, 1]));
    let k = cluster_rows(&s.model, ClusterKind::Kube);
    assert_eq!(k.len(), 1);
    assert_eq!(k[0].key, "k8s:kube-b");
    // No cluster at all: the page is empty.
    s.model.hosts[3].report = report(Cluster::default());
    assert!(cluster_rows(&s.model, ClusterKind::Kube).is_empty());
}
