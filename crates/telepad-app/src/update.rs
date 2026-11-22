//! The only place the [`Model`] changes. No I/O: anything outside the
//! process is requested as an [`Effect`], and the current time is a parameter.

use std::time::{Duration, Instant};

use telepad_core::authorized_keys::{KeyOutcome, PublicKey};
use telepad_core::collector::{self, Job, Scope};
use telepad_core::config::{Access, HostEntry};
use telepad_core::docker_actions::{ComposeProject, Plan};
use telepad_core::model::{Container, Manager};
use telepad_core::store::{ImportMode, ImportSummary};
use telepad_core::update::{CHECK_EVERY, CURRENT_VERSION, RETRY_AFTER_FAILURE, UpdateConfig};

use crate::msg::{Effect, Msg};
use crate::state::{
    Editor, HostState, ImportDialog, KeyRun, KeysDraft, Model, PasteImport, RunInfoView, State,
    UpdatePhase, key_selectable,
};

/// Apply `msg` to `state` at time `now`; return the effects to run.
pub fn update(state: &mut State, msg: Msg, now: Instant) -> Vec<Effect> {
    if let Msg::Wk(wk) = msg {
        let mut fx = workspaces(state, wk, now);
        let State {
            model: m,
            drafts: d,
        } = state;
        let mut ws = std::mem::take(&mut m.ws);
        ws.sync(m);
        ws.sync_keys(d.keys.is_some());
        m.ws = ws;
        if m.db_path.is_some() {
            autosave(m, d, now, &mut fx);
        }
        return fx;
    }
    let mut fx = Vec::new();
    let State {
        model: m,
        drafts: d,
    } = state;
    let hosts_changed = matches!(msg, Msg::HostsLoaded { .. });
    let collected_host = matches!(msg, Msg::Collected(collector::Update::Done { .. }));
    match msg {
        Msg::Started {
            db_path,
            ssh_config_path,
            imported_from,
            update_config,
            ui_prefs,
            prefs,
        } => {
            d.compact_host_list = ui_prefs.compact_host_list;
            d.group_by_status = ui_prefs.group_by_status;
            d.overview_cards = ui_prefs.overview_cards;
            d.theme = ui_prefs.theme;
            m.ws_legacy = ui_prefs.workspace.clone();
            m.ws.sidebar_collapsed = ui_prefs.sidebar_collapsed;
            match prefs {
                Ok(p) => {
                    m.wss = p;
                    m.prefs_ok = true;
                }
                Err(e) => {
                    m.wss = Default::default();
                    m.prefs_ok = false;
                    say(m, now, format!("Workspaces are not saved: {e}"));
                }
            }
            restore_current(m);
            m.ws_saved = (crate::workspace::layout(m), m.ws.sidebar_collapsed);
            fx.extend(start_page_sessions(m, d));
            if let Some(src) = imported_from {
                say(
                    m,
                    now,
                    format!("Created {} from {}", db_path.display(), src.display()),
                );
            }
            m.db_path = Some(db_path);
            m.ssh_config_path = Some(ssh_config_path);
            match update_config {
                Ok(c) => {
                    d.update_form = c.clone();
                    m.update.config = Some(c);
                }
                Err(e) => m.update.error = Some(format!("update settings: {e}")),
            }
            fx.push(Effect::LoadHosts);
            if m.update.enabled() {
                start_check(m, now, false, &mut fx);
            }
        }
        Msg::Fatal(e) => m.fatal = Some(format!("Cannot open the host database: {e}")),
        Msg::Tick { busy } => {
            m.busy = busy;
            let every = if m.update.error.is_some() {
                RETRY_AFTER_FAILURE
            } else {
                CHECK_EVERY
            };
            if m.update.enabled() && elapsed(m.update.last_check, now) >= Some(every) {
                start_check(m, now, false, &mut fx);
            }
            let interval = Duration::from_secs(u64::from(d.interval_secs));
            if d.auto_refresh && busy == 0 && elapsed(m.last_refresh, now) >= Some(interval) {
                refresh_all(m, now, false, &mut fx);
            }
        }

        // Asked for by the user: everything, not only what is due.
        Msg::RefreshAll => refresh_all(m, now, true, &mut fx),
        Msg::Refresh(i) => {
            if let Some(h) = m.hosts.get(i).filter(|h| !h.entry.unmonitored) {
                fx.push(Effect::Collect(vec![full(&h.entry)]));
            }
        }
        Msg::Ssh(i) => {
            if let Some(h) = m.hosts.get(i) {
                fx.push(match h.entry.access {
                    Access::Rdp => Effect::OpenRdp(h.entry.clone()),
                    Access::Ssh => Effect::OpenSsh(h.entry.clone()),
                });
            }
        }
        Msg::CopySsh(i) => {
            if let Some(h) = m.hosts.get(i) {
                fx.push(Effect::CopySshCommand(h.entry.clone()));
            }
        }
        Msg::BmcSetup(i) => {
            if let Some(h) = m.hosts.get(i) {
                fx.push(Effect::OpenBmcSetup(h.entry.clone()));
            }
        }
        Msg::DockerAction {
            host,
            container,
            action,
        } => {
            let found = m.hosts.get(host).and_then(|h| {
                let c = h
                    .report
                    .as_ref()?
                    .docker
                    .containers
                    .iter()
                    .find(|c| c.id == container)?;
                Some((h.entry.clone(), Plan::container(c, action)))
            });
            match found {
                Some((entry, Ok(plan))) => fx.push(Effect::OpenDockerAction(entry, plan)),
                Some((_, Err(e))) => say(m, now, e),
                None => say(m, now, "That container is gone; refresh the host"),
            }
        }
        Msg::StackAction {
            host,
            stack,
            action,
        } => {
            let found = m.hosts.get(host).and_then(|h| {
                let d = &h.report.as_ref()?.docker;
                let cs: Vec<&Container> =
                    d.containers.iter().filter(|c| c.stack == stack).collect();
                let first = cs.first()?;
                let plan = match d.stack_manager(&cs) {
                    Manager::Portainer { .. } => Err(format!(
                        "{stack} is a Portainer stack: change it in Portainer"
                    )),
                    Manager::Standalone => Err(format!("{stack} is not a compose stack")),
                    Manager::Compose { .. } => {
                        ComposeProject::from_container(first).and_then(|p| Plan::stack(&p, action))
                    }
                };
                Some((h.entry.clone(), plan))
            });
            match found {
                Some((entry, Ok(plan))) => fx.push(Effect::OpenDockerAction(entry, plan)),
                Some((_, Err(e))) => say(m, now, e),
                None => say(m, now, "That stack is gone; refresh the host"),
            }
        }
        Msg::ShowRunInfo { host, container } => {
            let found = m.hosts.get(host).and_then(|h| {
                let c = h
                    .report
                    .as_ref()?
                    .docker
                    .containers
                    .iter()
                    .find(|c| c.id == container)?;
                Some((h.entry.clone(), c.clone()))
            });
            if let Some((entry, c)) = found {
                d.run_info = Some(RunInfoView::Loading {
                    host: entry.alias.clone(),
                    container: c.name.clone(),
                });
                fx.push(Effect::FetchRunInfo(entry, Box::new(c)));
            }
        }
        Msg::CloseRunInfo => d.run_info = None,
        Msg::SwarmService {
            host,
            service,
            action,
        } => {
            let plan = swarm(m, host).map(|(e, sw)| {
                let ok = sw.services.iter().any(|s| s.name == service);
                (
                    e,
                    if ok {
                        Plan::swarm_service(&service, action)
                    } else {
                        Err(gone("service"))
                    },
                )
            });
            open_plan(m, now, &mut fx, plan);
        }
        Msg::SwarmStackRemove { host, stack } => {
            let plan = swarm(m, host).map(|(e, sw)| {
                let ok = sw.stacks.iter().any(|(s, _)| *s == stack);
                (
                    e,
                    if ok {
                        Plan::swarm_stack_remove(&stack)
                    } else {
                        Err(gone("stack"))
                    },
                )
            });
            open_plan(m, now, &mut fx, plan);
        }
        Msg::SwarmNode {
            host,
            node_id,
            action,
        } => {
            let plan = swarm(m, host).map(|(e, sw)| {
                let plan = match sw.nodes.iter().find(|n| n.id == node_id) {
                    Some(n) => Plan::swarm_node(&n.id, &n.hostname, action),
                    None => Err(gone("node")),
                };
                (e, plan)
            });
            open_plan(m, now, &mut fx, plan);
        }
        Msg::KubeWorkload {
            host,
            namespace,
            name,
            action,
        } => {
            let plan = m.hosts.get(host).and_then(|h| {
                let k = h.report.as_ref()?.cluster.kube.as_ref()?;
                let plan = match k
                    .workloads
                    .iter()
                    .find(|w| w.namespace == namespace && w.name == name)
                {
                    Some(w) => Plan::kube_workload(&k.tool, w, action),
                    None => Err(gone("workload")),
                };
                Some((h.entry.clone(), plan))
            });
            open_plan(m, now, &mut fx, plan);
        }
        Msg::RunInfoLoaded(info) => {
            // Ignore a late answer for a window that was closed or replaced.
            if matches!(&d.run_info, Some(RunInfoView::Loading { container, .. }) if *container == info.container)
            {
                d.run_info = Some(RunInfoView::Ready(info));
            }
        }
        Msg::Select(i) => {
            m.selected = i.filter(|&i| i < m.hosts.len());
            if m.selected.is_some() {
                m.ws.open(crate::workspace::Win::Host);
            }
        }
        Msg::MoveHost { from, to } => {
            if from != to && from < m.hosts.len() && to < m.hosts.len() {
                let selected = selected_id(m);
                let h = m.hosts.remove(from);
                m.hosts.insert(to, h);
                m.selected =
                    selected.and_then(|id| m.hosts.iter().position(|h| h.entry.id == Some(id)));
                fx.push(Effect::SaveOrder(
                    m.hosts.iter().filter_map(|h| h.entry.id).collect(),
                ));
            }
        }
        Msg::OrderSaved(Ok(())) => {}
        Msg::OrderSaved(Err(e)) => {
            say(m, now, format!("Cannot save the host order: {e}"));
            // Back to the order in the database; results are kept by host id.
            fx.push(Effect::LoadHosts);
        }
        Msg::ShowOverview(page) => {
            m.overview = page;
            m.ws.open(match page {
                crate::state::OverviewPage::Hosts => crate::workspace::Win::Overview,
                crate::state::OverviewPage::Docker => crate::workspace::Win::Docker,
            });
        }
        Msg::Ws(w) => {
            m.ws.update(w);
        }
        Msg::SortBy(key) => d.sort = d.sort.clicked(key),
        Msg::SetCompactHostList(compact) => {
            if d.compact_host_list != compact {
                d.compact_host_list = compact;
                fx.push(Effect::SaveUiPrefs(d.ui_prefs(m)));
            }
        }
        Msg::SetGroupByStatus(group) => {
            if d.group_by_status != group {
                d.group_by_status = group;
                fx.push(Effect::SaveUiPrefs(d.ui_prefs(m)));
            }
        }
        Msg::SetOverviewCards(cards) => {
            if d.overview_cards != cards {
                d.overview_cards = cards;
                fx.push(Effect::SaveUiPrefs(d.ui_prefs(m)));
            }
        }
        Msg::SetSort(sort) => d.sort = sort,
        Msg::SetTheme(theme) => {
            if d.theme != theme {
                d.theme = theme;
                fx.push(Effect::SaveUiPrefs(d.ui_prefs(m)));
            }
        }
        Msg::UiPrefsSaved(Ok(())) => {}
        Msg::PrefsSaved(Ok(())) => {}
        Msg::PrefsSaved(Err(e)) => say(m, now, format!("Cannot save the workspaces: {e}")),
        Msg::Wk(_) => {}
        Msg::UiPrefsSaved(Err(e)) => say(m, now, format!("Cannot save view settings: {e}")),
        Msg::SelectAlias(alias) => {
            m.selected = m.hosts.iter().position(|h| h.entry.alias == alias);
            if m.selected.is_some() {
                m.ws.open(crate::workspace::Win::Host);
            }
        }
        Msg::AddHost => {
            d.editor = Some(Editor::new(HostEntry::new("")));
            fx.push(Effect::FindKeys);
        }
        Msg::Edit(i) => {
            if let Some(h) = m.hosts.get(i) {
                d.editor = Some(Editor::new(h.entry.clone()));
                fx.push(Effect::FindKeys);
            }
        }
        Msg::Clone(i) => {
            if let Some(h) = m.hosts.get(i) {
                d.editor = Some(Editor::clone_of(&h.entry));
                fx.push(Effect::FindKeys);
            }
        }
        Msg::KeysFound(keys) => {
            if let Some(ed) = &mut d.editor {
                ed.key_choices = keys;
            }
        }
        Msg::Delete(i) => {
            if let Some(h) = m.hosts.get(i)
                && let Some(id) = h.entry.id
            {
                m.pending_delete = Some((id, h.entry.alias.clone()));
            }
        }
        Msg::ConfirmDelete => {
            if let Some((id, alias)) = m.pending_delete.take() {
                fx.push(Effect::DeleteHost { id, alias });
            }
        }
        Msg::CancelDelete => m.pending_delete = None,
        Msg::EditorSave => {
            if let Some(ed) = &mut d.editor {
                match ed.build() {
                    Ok(entry) => fx.push(Effect::SaveHost(entry)),
                    Err(e) => ed.error = Some(e),
                }
            }
        }
        Msg::EditorDelete => {
            if let Some(ed) = d.editor.take()
                && let Some(id) = ed.entry.id
            {
                fx.push(Effect::DeleteHost {
                    id,
                    alias: ed.entry.alias,
                });
            }
        }
        Msg::EditorClose => d.editor = None,
        Msg::EditorBrowseKey => {
            let current = d
                .editor
                .as_ref()
                .map(|e| e.identity.clone())
                .unwrap_or_default();
            fx.push(Effect::PickKeyFile(current));
        }
        Msg::KeyPicked(p) => {
            if let Some(ed) = &mut d.editor {
                ed.identity = p.display().to_string();
            }
        }
        Msg::Import => fx.push(Effect::PickImportFile),
        Msg::ImportSshConfig => fx.push(Effect::ReadUserSshConfig),
        Msg::ImportOpened(Ok((path, inventory))) => {
            if inventory.hosts.is_empty() {
                say(m, now, format!("No hosts found in {}", path.display()));
            } else {
                fx.push(Effect::PreviewImport {
                    inventory: inventory.clone(),
                    mode: ImportMode::Merge,
                });
                m.import = Some(ImportDialog {
                    path,
                    inventory,
                    mode: ImportMode::Merge,
                    preview: Ok(ImportSummary::default()),
                    skipped: Vec::new(),
                });
            }
        }
        Msg::ImportOpened(Err((path, e))) => {
            say(m, now, format!("Cannot import {}: {e}", path.display()));
        }
        Msg::KeysOpen => {
            if d.keys.is_none() {
                d.keys = Some(KeysDraft::default());
            }
            if m.auth_keys.is_empty() {
                fx.extend(load_keys(m, None));
            }
            if m.key_run.as_ref().is_some_and(|r| !r.running()) {
                m.key_run = None;
            }
        }
        Msg::KeysClose => {
            d.keys = None;
            if m.key_run.as_ref().is_some_and(|r| !r.running()) {
                m.key_run = None;
            }
        }
        Msg::KeysSelectAll(on) => {
            if let Some(k) = &mut d.keys {
                for i in k.listed(m) {
                    let e = &m.hosts[i].entry;
                    if let (Some(id), true) = (e.id, key_selectable(e)) {
                        if on {
                            k.selected.insert(id);
                        } else {
                            k.selected.remove(&id);
                        }
                    }
                }
            }
        }
        Msg::KeysToggle(id) => {
            if let Some(k) = &mut d.keys
                && m.hosts
                    .iter()
                    .any(|h| h.entry.id == Some(id) && key_selectable(&h.entry))
                && !k.selected.remove(&id)
            {
                k.selected.insert(id);
            }
        }
        Msg::KeysAsk(action) => {
            if let Some(k) = &mut d.keys
                && PublicKey::parse(&k.text).is_ok()
                && !k.chosen(m).is_empty()
                && !m.key_run.as_ref().is_some_and(KeyRun::running)
            {
                k.confirm = Some(action);
                k.typed.clear();
            }
        }
        Msg::AkLoad(aliases) => fx.extend(load_keys(m, aliases)),
        Msg::AkListed { alias, result } => {
            if m.hosts.iter().any(|h| h.entry.alias == alias) {
                let v = match result {
                    Ok(k) => crate::state::AkHost::Ready(k),
                    Err(e) => crate::state::AkHost::Failed(e),
                };
                m.auth_keys.insert(alias, v);
            }
        }
        Msg::AkTab(tab) => {
            if let Some(k) = &mut d.keys {
                k.tab = tab;
            }
        }
        Msg::AkPrepare {
            action,
            line,
            aliases,
        } => {
            let k = d.keys.get_or_insert_with(KeysDraft::default);
            if k.confirm.is_none() && !m.key_run.as_ref().is_some_and(KeyRun::running) {
                k.text = line;
                k.selected = m
                    .hosts
                    .iter()
                    .filter(|h| aliases.contains(&h.entry.alias) && key_selectable(&h.entry))
                    .filter_map(|h| h.entry.id)
                    .collect();
                k.tab = crate::state::KeysTab::AddRevoke;
                k.typed.clear();
                if PublicKey::parse(&k.text).is_ok() && !k.chosen(m).is_empty() {
                    k.confirm = Some(action);
                }
            }
        }
        Msg::KeysConfirmCancel => {
            if let Some(k) = &mut d.keys {
                k.confirm = None;
                k.typed.clear();
            }
        }
        Msg::KeysRun => {
            let Some(k) = &mut d.keys else { return fx };
            let Some(action) = k.confirm else { return fx };
            if k.typed.trim() != action.word() || m.key_run.as_ref().is_some_and(KeyRun::running) {
                return fx;
            }
            let Ok(key) = PublicKey::parse(&k.text) else {
                return fx;
            };
            let hosts: Vec<HostEntry> = k.chosen(m).into_iter().cloned().collect();
            let group = match telepad_core::authorized_keys::check_group(&k.group) {
                Ok(g)
                    if g.is_empty() || action != telepad_core::authorized_keys::KeyAction::Add =>
                {
                    None
                }
                Ok(g) => Some(g),
                Err(e) => {
                    say(m, now, e);
                    return fx;
                }
            };
            k.confirm = None;
            k.typed.clear();
            if hosts.is_empty() {
                return fx;
            }
            m.key_run = Some(KeyRun {
                action,
                fingerprint: key.fingerprint(),
                comment: key.comment.clone(),
                results: hosts.iter().map(|h| (h.alias.clone(), None)).collect(),
            });
            fx.push(Effect::ApplyKey {
                action,
                key,
                hosts,
                group,
            });
        }
        Msg::KeyResult { alias, outcome } => {
            let was_running = m.key_run.as_ref().is_some_and(KeyRun::running);
            if let Some(run) = &mut m.key_run
                && let Some(slot) = run
                    .results
                    .iter_mut()
                    .find(|(a, r)| *a == alias && r.is_none())
            {
                slot.1 = Some(outcome);
                if !run.running() {
                    let ok = run
                        .results
                        .iter()
                        .filter(|(_, r)| r.as_ref().is_some_and(KeyOutcome::ok))
                        .count();
                    let total = run.results.len();
                    let text = format!(
                        "{}: {ok} of {total} host(s) done{}",
                        run.action.title(),
                        if ok < total {
                            format!(", {} failed or refused", total - ok)
                        } else {
                            String::new()
                        }
                    );
                    say(m, now, text);
                }
            }
            // The run just finished: read the changed hosts again so the
            // lists show the result.
            let changed: Option<Vec<String>> = m
                .key_run
                .as_ref()
                .filter(|r| was_running && !r.running())
                .map(|r| r.results.iter().map(|(a, _)| a.clone()).collect());
            if let Some(c) = changed {
                fx.extend(load_keys(m, Some(c)));
            }
        }
        Msg::PasteImport => d.paste = Some(PasteImport::default()),
        Msg::PasteCancel => d.paste = None,
        Msg::PasteFromClipboard => fx.push(Effect::ReadClipboard),
        Msg::ClipboardRead(r) => match (&mut d.paste, r) {
            (Some(p), Ok(text)) => {
                if !p.text.trim().is_empty() && !p.text.ends_with('\n') {
                    p.text.push('\n');
                }
                p.text.push_str(&text);
            }
            (Some(p), Err(e)) => p.problems = vec![format!("Cannot read the clipboard: {e}")],
            (None, _) => {}
        },
        Msg::PasteApply => {
            if let Some(p) = &mut d.paste {
                let (inventory, problems) = telepad_core::store::parse_pasted(&p.text);
                if inventory.hosts.is_empty() {
                    p.problems = problems;
                    p.problems.insert(0, "No hosts found in the text".into());
                } else {
                    d.paste = None;
                    fx.push(Effect::PreviewImport {
                        inventory: inventory.clone(),
                        mode: ImportMode::Merge,
                    });
                    m.import = Some(ImportDialog {
                        path: "pasted text".into(),
                        inventory,
                        mode: ImportMode::Merge,
                        preview: Ok(ImportSummary::default()),
                        skipped: problems,
                    });
                }
            }
        }
        Msg::ImportMode(mode) => {
            if let Some(dlg) = &mut m.import
                && dlg.mode != mode
            {
                dlg.mode = mode;
                fx.push(Effect::PreviewImport {
                    inventory: dlg.inventory.clone(),
                    mode,
                });
            }
        }
        Msg::ImportPreviewed(r) => {
            if let Some(dlg) = &mut m.import {
                dlg.preview = r;
            }
        }
        Msg::ImportApply => {
            if let Some(dlg) = &m.import
                && dlg.preview.is_ok()
            {
                fx.push(Effect::ApplyImport {
                    inventory: dlg.inventory.clone(),
                    mode: dlg.mode,
                });
            }
        }
        Msg::Imported(Ok(s)) => {
            if let Some(dlg) = m.import.take() {
                say(
                    m,
                    now,
                    format!(
                        "Imported {}: {} added, {} updated, {} unchanged, {} removed",
                        dlg.path.display(),
                        s.added,
                        s.updated,
                        s.unchanged,
                        s.removed
                    ),
                );
            }
            fx.push(Effect::LoadHosts);
        }
        Msg::Imported(Err(e)) => {
            if let Some(dlg) = &mut m.import {
                dlg.preview = Err(format!("Import failed, nothing changed: {e}"));
            }
        }
        Msg::ImportCancel => m.import = None,
        Msg::Export => fx.push(Effect::PickExportFile),
        Msg::Exported(r) => {
            let text = match r {
                Ok((n, path)) => format!("Exported {n} hosts to {}", path.display()),
                Err(e) => format!("Export failed: {e}"),
            };
            say(m, now, text);
        }

        Msg::UpdateWindow(open) => m.update.window = open,
        Msg::UpdateCheck => start_check(m, now, true, &mut fx),
        Msg::UpdateInstall => {
            if m.update.can_install()
                && let Some(c) = m.update.check.clone()
            {
                m.update.phase = UpdatePhase::Installing;
                m.update.error = None;
                fx.push(Effect::InstallUpdate(c));
            }
        }
        Msg::UpdateLater => {
            m.update.later = true;
            m.update.window = false;
        }
        Msg::UpdateSaveSettings => {
            let c = UpdateConfig {
                repo: d.update_form.repo.trim().to_string(),
                ..d.update_form.clone()
            };
            match c.validate() {
                Ok(()) => fx.push(Effect::SaveUpdateConfig(c)),
                Err(e) => d.update_form_error = Some(e),
            }
        }
        Msg::UpdateSettingsSaved(Ok(c)) => {
            d.update_form_error = None;
            d.update_form = c.clone();
            m.update.config = Some(c);
            m.update.check = None;
            say(m, now, "Update settings saved");
            if m.update.enabled() {
                start_check(m, now, true, &mut fx);
            }
        }
        Msg::UpdateSettingsSaved(Err(e)) => d.update_form_error = Some(e),
        Msg::Restart => fx.push(Effect::Relaunch),
        Msg::OpenUrl(url) => fx.push(Effect::OpenUrl(url)),

        Msg::Collected(u) => collected(m, u, now),
        Msg::HostsLoaded {
            hosts,
            ssh_config_error,
        } => {
            if let Some(e) = ssh_config_error {
                say(m, now, e);
            }
            match hosts {
                Ok(entries) => reconcile(m, entries, now, &mut fx),
                Err(e) => say(m, now, format!("Cannot read hosts: {e}")),
            }
        }
        Msg::HostSaved(Ok((id, alias))) => {
            d.editor = None;
            m.select_after_load = Some(id);
            say(m, now, format!("Saved {alias}"));
            fx.push(Effect::LoadHosts);
        }
        Msg::HostSaved(Err(e)) => {
            if let Some(ed) = &mut d.editor {
                ed.error = Some(e);
            }
        }
        Msg::HostDeleted { id, alias, result } => match result {
            Ok(()) => {
                if selected_id(m) == Some(id) {
                    m.selected = None;
                }
                say(m, now, format!("Deleted {alias}"));
                fx.push(Effect::LoadHosts);
            }
            Err(e) => say(m, now, format!("Cannot delete {alias}: {e}")),
        },
        Msg::UpdateChecked(r) => {
            m.update.phase = UpdatePhase::Idle;
            match r {
                Ok(c) => {
                    m.update.error = None;
                    if c.newer {
                        say(m, now, format!("Update {} available", c.version));
                    } else if m.update.manual {
                        say(m, now, format!("Telepad {CURRENT_VERSION} is up to date"));
                    }
                    m.update.check = Some(c);
                    // Automatic install: only a newer release with a download for
                    // this OS, and not after the user chose "Later".
                    let auto = m.update.config.as_ref().is_some_and(|c| c.auto_install);
                    if auto
                        && !m.update.later
                        && m.update.can_install()
                        && let Some(c) = m.update.check.clone()
                    {
                        m.update.phase = UpdatePhase::Installing;
                        fx.push(Effect::InstallUpdate(c));
                    }
                }
                Err(e) => {
                    if m.update.manual {
                        say(m, now, format!("Update check failed: {e}"));
                    }
                    m.update.error = Some(e);
                }
            }
        }
        Msg::UpdateInstalled(Ok(v)) => {
            say(m, now, format!("Updated to {v} — restart to use it"));
            m.update.phase = UpdatePhase::Installed(v);
        }
        Msg::UpdateInstalled(Err(e)) => {
            m.update.phase = UpdatePhase::Idle;
            say(m, now, format!("Update failed: {e}"));
            m.update.error = Some(e);
        }
        Msg::Notice(text) => say(m, now, text),
        Msg::Db(dm) => {
            let hosts = &m.hosts;
            let entry = |alias: &str| {
                hosts
                    .iter()
                    .find(|h| h.entry.alias == alias)
                    .map(|h| h.entry.clone())
            };
            fx.extend(
                crate::db::update(&mut m.db, &mut d.db, &mut m.db_next, &entry, dm)
                    .into_iter()
                    .map(Effect::Db),
            );
        }
        Msg::Files(fm) => {
            // Saving ~/.ssh/authorized_keys in the editor: read that host's
            // list again (#85).
            let reload = match &fm {
                crate::files::FmMsg::Editor {
                    id,
                    msg: crate::editor::EdMsg::Saved { tab, result: Ok(_) },
                } => m.files.iter().find(|s| s.id == *id).and_then(|s| {
                    s.editor
                        .tab(*tab)
                        .filter(|t| {
                            t.side == crate::files::Side::Remote
                                && t.path.ends_with("/.ssh/authorized_keys")
                        })
                        .map(|_| s.host.alias.clone())
                }),
                _ => None,
            };
            if let Some(alias) = reload {
                fx.extend(load_keys(m, Some(vec![alias])));
            }
            let hosts = &m.hosts;
            let entry = |alias: &str| {
                hosts
                    .iter()
                    .find(|h| h.entry.alias == alias)
                    .map(|h| h.entry.clone())
            };
            fx.extend(
                crate::files::update(
                    &mut m.files,
                    &mut d.files,
                    &mut m.files_next,
                    &entry,
                    fm,
                    now,
                )
                .into_iter()
                .map(Effect::Files),
            );
        }
        Msg::HostSync(hm) => {
            let db: Vec<_> = m.hosts.iter().map(|h| h.entry.clone()).collect();
            let (effects, follow) =
                crate::hostsync::update(&mut m.host_sync, &mut d.host_sync, &db, hm);
            fx.extend(effects.into_iter().map(Effect::HostSync));
            if follow.iter().any(|f| f == "reload-hosts") {
                fx.push(Effect::LoadHosts);
            }
        }
    }
    if hosts_changed && let Some(hs) = m.host_sync.as_mut() {
        let db: Vec<_> = m.hosts.iter().map(|h| h.entry.clone()).collect();
        crate::hostsync::refresh(hs, &db);
    }
    // Authorized keys reopened from the saved layout: read the lists once the
    // hosts are known.
    if hosts_changed && d.keys.is_some() && m.auth_keys.is_empty() {
        fx.extend(load_keys(m, None));
    }
    // The session windows of a restored layout, once the hosts are known.
    if hosts_changed {
        place_pending_host(m);
        fx.extend(start_pending_files(m, d, now));
    }
    if hosts_changed || collected_host {
        fx.extend(start_pending_dbs(m, d, now));
    }
    // A file window left in the background closes when its transfers end.
    fx.extend(close_finished_background(m, d, now));
    // Session windows follow their sessions; save the layout when it changed.
    let mut ws = std::mem::take(&mut m.ws);
    ws.sync(m);
    ws.sync_keys(d.keys.is_some());
    m.ws = ws;
    if m.db_path.is_some() {
        autosave(m, d, now, &mut fx);
    }
    fx
}

/// Layout changes go to the current workspace (none while `<none>` is
/// current) and are written at most once a second; the sidebar goes to
/// `telepad.db` at once.
fn autosave(m: &mut Model, d: &crate::Drafts, now: Instant, fx: &mut Vec<Effect>) {
    let layout = crate::workspace::layout(m);
    if layout != m.ws_saved.0 {
        m.ws_saved.0 = layout.clone();
        let current = m.wss.current.clone();
        if let Some(slot) = m.wss.layout_mut(&current)
            && *slot != layout
        {
            *slot = layout;
            m.prefs_dirty = true;
        }
    }
    if m.ws.sidebar_collapsed != m.ws_saved.1 {
        m.ws_saved.1 = m.ws.sidebar_collapsed;
        fx.push(Effect::SaveUiPrefs(d.ui_prefs(m)));
    }
    let due = m
        .prefs_written
        .is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(1));
    if m.prefs_dirty && due {
        write_prefs(m, now, fx);
    }
}

fn write_prefs(m: &mut Model, now: Instant, fx: &mut Vec<Effect>) {
    m.prefs_dirty = false;
    if m.prefs_ok {
        m.prefs_written = Some(now);
        fx.push(Effect::SavePrefs(m.wss.clone()));
    }
}

/// Open the windows of the current workspace (`<own>` never used yet: the
/// Hosts page, maximized; `<none>`: nothing).
fn restore_current(m: &mut Model) {
    use telepad_core::prefs::Current;
    let layout = m.wss.layout(&m.wss.current).map(str::to_string);
    match (&m.wss.current, layout) {
        (Current::None, _) | (_, None) => m.ws.restore(""),
        (Current::Own, Some(l)) => m.ws.restore_or_default(&l),
        (Current::Custom(_), Some(l)) => m.ws.restore(&l),
    }
}

/// Pages of a restored layout that hold a session start it again.
fn start_page_sessions(m: &mut Model, d: &mut crate::Drafts) -> Vec<Effect> {
    let mut fx = Vec::new();
    if m.ws.is_open(crate::workspace::Win::Keys) {
        d.keys = Some(KeysDraft::default());
        if !m.hosts.is_empty() {
            fx.extend(load_keys(m, None));
        }
    }
    if m.ws.is_open(crate::workspace::Win::HostSync) && m.host_sync.is_none() {
        let db: Vec<_> = m.hosts.iter().map(|h| h.entry.clone()).collect();
        let (effects, _) = crate::hostsync::update(
            &mut m.host_sync,
            &mut d.host_sync,
            &db,
            crate::hostsync::HsMsg::Open,
        );
        fx.extend(effects.into_iter().map(Effect::HostSync));
    }
    fx
}

/// Select the host of a restored layout (its window is put back by
/// `Workspace::sync`); a host that no longer exists is left out.
fn place_pending_host(m: &mut Model) {
    let Some(alias) = m.ws.pending_host() else {
        return;
    };
    match m.hosts.iter().position(|h| h.entry.alias == alias) {
        Some(i) => m.selected = Some(i),
        None => {
            m.ws.drop_pending(&format!("host:{}", crate::workspace::esc(&alias)))
        }
    }
}

/// File and editor windows of a restored layout (#99): connect them again
/// with their folders and files; a host that is gone is left out.
fn start_pending_files(m: &mut Model, d: &mut crate::Drafts, now: Instant) -> Vec<Effect> {
    use crate::files::{FmMsg, Side};
    let mut fx = Vec::new();
    if m.hosts.is_empty() {
        return fx;
    }
    let files = m.ws.pending_of("files:");
    let editors = m.ws.pending_of("editor:");
    let mut aliases: Vec<String> = files.iter().map(|(_, a, _)| a.clone()).collect();
    for (_, a, _) in &editors {
        if !aliases.contains(a) {
            aliases.push(a.clone());
        }
    }
    let mut gone = Vec::new();
    for alias in aliases {
        let file_key = format!("files:{}", crate::workspace::esc(&alias));
        let edit_key = format!("editor:{}", crate::workspace::esc(&alias));
        if !m.hosts.iter().any(|h| h.entry.alias == alias) {
            m.ws.drop_pending(&file_key);
            m.ws.drop_pending(&edit_key);
            gone.push(alias);
            continue;
        }
        if m.files
            .iter()
            .any(|s| s.host.alias == alias && !s.background)
        {
            continue;
        }
        let folders = files
            .iter()
            .find(|(k, _, _)| *k == file_key)
            .and_then(|(_, _, data)| match data.as_slice() {
                [l, r] if !l.is_empty() && !r.is_empty() => Some((l.clone(), r.clone())),
                _ => None,
            });
        let edit: Vec<(Side, String)> = editors
            .iter()
            .find(|(k, _, _)| *k == edit_key)
            .map(|(_, _, data)| {
                data.iter()
                    .filter_map(|t| {
                        let side = match t.chars().next()? {
                            'L' => Side::Local,
                            'R' => Side::Remote,
                            _ => return None,
                        };
                        let path = &t[1..];
                        (!path.is_empty()).then(|| (side, path.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let hosts = &m.hosts;
        let entry = |a: &str| {
            hosts
                .iter()
                .find(|h| h.entry.alias == a)
                .map(|h| h.entry.clone())
        };
        fx.extend(
            crate::files::update(
                &mut m.files,
                &mut d.files,
                &mut m.files_next,
                &entry,
                FmMsg::Restore {
                    alias,
                    folders,
                    edit,
                },
                now,
            )
            .into_iter()
            .map(Effect::Files),
        );
    }
    if !gone.is_empty() {
        say(
            m,
            now,
            format!(
                "Not in the inventory any more, file window left out: {}",
                gone.join(", ")
            ),
        );
    }
    fx
}

/// Database windows of a restored layout (#99), once their host has been
/// checked: the target is looked up again (a container's address can
/// change); one that is gone is left out with a note. A host that could not
/// be checked gets the saved target (its window shows the connect error).
fn start_pending_dbs(m: &mut Model, d: &mut crate::Drafts, now: Instant) -> Vec<Effect> {
    use telepad_core::db::{DbTarget, Engine};
    let mut fx = Vec::new();
    if m.hosts.is_empty() {
        return fx;
    }
    let mut notes = Vec::new();
    for (key, _, data) in m.ws.pending_of("db:") {
        let [engine, host, container, address, port, database, schema] = data.as_slice() else {
            m.ws.drop_pending(&key);
            continue;
        };
        let (Some(engine), Ok(port)) = (Engine::parse(engine), port.parse::<u16>()) else {
            m.ws.drop_pending(&key);
            continue;
        };
        let saved = DbTarget {
            engine,
            host: host.clone(),
            container: container.clone(),
            container_id: String::new(),
            address: address.clone(),
            port,
        };
        if m.db.iter().any(|s| s.target.key() == saved.key()) {
            continue;
        }
        let Some(h) = m.hosts.iter().find(|h| h.entry.alias == *host) else {
            m.ws.drop_pending(&key);
            notes.push(saved.title());
            continue;
        };
        let target = match (&h.report, &h.error) {
            (Some(r), _) => {
                match crate::db::host_db_targets(host, r)
                    .into_iter()
                    .find(|t| t.key() == saved.key())
                {
                    Some(t) => t,
                    None => {
                        m.ws.drop_pending(&key);
                        notes.push(saved.title());
                        continue;
                    }
                }
            }
            (None, Some(_)) => saved,
            (None, None) => continue,
        };
        let hosts = &m.hosts;
        let entry = |a: &str| {
            hosts
                .iter()
                .find(|h| h.entry.alias == a)
                .map(|h| h.entry.clone())
        };
        fx.extend(
            crate::db::update(
                &mut m.db,
                &mut d.db,
                &mut m.db_next,
                &entry,
                crate::db::DbMsg::Restore {
                    target,
                    database: database.clone(),
                    schema: schema.clone(),
                },
            )
            .into_iter()
            .map(Effect::Db),
        );
    }
    if !notes.is_empty() {
        say(
            m,
            now,
            format!("Database gone, window left out: {}", notes.join(", ")),
        );
    }
    fx
}

/// Background file windows (#99) whose transfers ended: close them.
fn close_finished_background(m: &mut Model, d: &mut crate::Drafts, now: Instant) -> Vec<Effect> {
    let done: Vec<(crate::files::FmId, String)> = m
        .files
        .iter()
        .filter(|s| s.background && s.running_transfers() == 0)
        .map(|s| (s.id, s.host.alias.clone()))
        .collect();
    let mut fx = Vec::new();
    for (id, alias) in done {
        let hosts = &m.hosts;
        let entry = |a: &str| {
            hosts
                .iter()
                .find(|h| h.entry.alias == a)
                .map(|h| h.entry.clone())
        };
        fx.extend(
            crate::files::update(
                &mut m.files,
                &mut d.files,
                &mut m.files_next,
                &entry,
                crate::files::FmMsg::Discard(id),
                now,
            )
            .into_iter()
            .map(Effect::Files),
        );
        say(m, now, format!("Background transfers to {alias} finished"));
    }
    fx
}

/// File transfers running in the background (#99), for the status bar.
pub fn background_transfers(m: &Model) -> usize {
    m.files
        .iter()
        .filter(|s| s.background)
        .map(|s| s.running_transfers())
        .sum()
}

/// What leaving the workspace would throw away (#99): (window, what) per
/// line, and how many transfers keep running. `quit`: Telepad closes, so
/// running transfers are lost too.
pub fn leave_losses(state: &State, quit: bool) -> (Vec<(String, String)>, usize) {
    let (m, d) = (&state.model, &state.drafts);
    let mut v: Vec<(String, String)> = Vec::new();
    let mut running = 0;
    for s in &m.files {
        let win = format!("Files – {}", s.host.alias);
        if let Some(fd) = d.files.get(&s.id) {
            for w in crate::files::unsaved(s, fd) {
                v.push((win.clone(), w));
            }
        }
        let n = s.running_transfers();
        if quit && n > 0 {
            v.push((win, format!("{n} running transfer(s), cancelled")));
        }
        running += n;
    }
    for s in &m.db {
        if let Some(dd) = d.db.get(&s.id) {
            for w in crate::db::unsaved(s, dd) {
                v.push((format!("Database – {}", s.target.title()), w));
            }
        }
    }
    if let Some(hs) = &m.host_sync {
        let n = hs.pending_count();
        if n > 0 {
            v.push((
                "Host Management".into(),
                format!("{n} change(s) not applied, discarded"),
            ));
        }
        if d.host_sync.form.is_some() {
            v.push(("Host Management".into(), "Host form, not saved".into()));
        }
    }
    if let Some(k) = &d.keys {
        if !k.text.trim().is_empty() {
            v.push(("Authorized keys".into(), "Pasted key, not added".into()));
        }
        if !k.selected.is_empty() {
            v.push((
                "Authorized keys".into(),
                format!("{} server(s) chosen", k.selected.len()),
            ));
        }
        if k.confirm.is_some() {
            v.push((
                "Authorized keys".into(),
                "Add / revoke waiting for confirmation".into(),
            ));
        }
    }
    if d.editor.is_some() {
        v.push(("Host editor".into(), "Host changes, not saved".into()));
    }
    if d.paste.is_some() {
        v.push(("Import".into(), "Pasted hosts, not imported".into()));
    }
    if m.import.is_some() {
        v.push(("Import".into(), "Import preview, not imported".into()));
    }
    if m.update.window && m.update.config.as_ref() != Some(&d.update_form) {
        v.push((
            "Software update".into(),
            "Settings changed, not saved".into(),
        ));
    }
    (v, if quit { 0 } else { running })
}

/// The workspace list (#94). Switching closes every window first (their
/// sessions end); unsaved editor tabs stop it and ask instead.
fn workspaces(state: &mut State, wk: crate::workspace::WkMsg, now: Instant) -> Vec<Effect> {
    use crate::workspace::WkMsg;
    use telepad_core::prefs::Current;
    let mut fx = Vec::new();
    match wk {
        WkMsg::Switch(to) => {
            let m = &state.model;
            if to == m.wss.current {
                return fx;
            }
            if let Current::Custom(l) = &to
                && !m.wss.custom.iter().any(|(x, _)| x == l)
            {
                return fx;
            }
            if ask_leave(state, Some(to.clone()), None) {
                return fx;
            }
            switch_to(state, to, now, &mut fx);
        }
        WkMsg::AskQuit => {
            if !ask_leave(state, None, None) {
                state.model.quit = true;
            }
        }
        WkMsg::LeaveBack => {
            state.drafts.ws_leave = None;
            state.drafts.ws_picker = Default::default();
        }
        WkMsg::LeaveYes => {
            let Some(ask) = state.drafts.ws_leave.take() else {
                return fx;
            };
            match ask.to {
                None => state.model.quit = true,
                Some(to) => {
                    switch_to(state, to, now, &mut fx);
                    if let Some(label) = ask.delete {
                        delete_workspace(state, &label, now, &mut fx);
                    }
                }
            }
        }
        WkMsg::Save => {
            let m = &mut state.model;
            if m.wss.current != Current::None {
                let layout = crate::workspace::layout(m);
                let current = m.wss.current.clone();
                if let Some(slot) = m.wss.layout_mut(&current) {
                    *slot = layout;
                }
                m.ws_saved.0 = crate::workspace::layout(m);
                write_prefs(m, now, &mut fx);
                say(m, now, format!("Workspace {} saved", current.label()));
            }
        }
        WkMsg::AskAdd => state.drafts.ws_add = Some(Default::default()),
        WkMsg::CancelAdd => state.drafts.ws_add = None,
        WkMsg::Add(label) => {
            let m = &mut state.model;
            match m.wss.check_label(&label) {
                Err(e) => {
                    let d = state.drafts.ws_add.get_or_insert_with(Default::default);
                    d.label = label;
                    d.error = Some(e);
                }
                Ok(l) => {
                    let layout = crate::workspace::layout(m);
                    m.wss.custom.push((l.clone(), layout.clone()));
                    m.wss.sort();
                    m.wss.current = Current::Custom(l.clone());
                    m.ws_saved.0 = layout;
                    state.drafts.ws_add = None;
                    write_prefs(m, now, &mut fx);
                    say(m, now, format!("Workspace {l} added"));
                }
            }
        }
        WkMsg::AskDelete(label) => state.drafts.ws_delete = Some(label),
        WkMsg::CancelDelete => state.drafts.ws_delete = None,
        WkMsg::Delete(label) => {
            state.drafts.ws_delete = None;
            if state.model.wss.current == Current::Custom(label.clone()) {
                if ask_leave(state, Some(Current::Own), Some(label.clone())) {
                    return fx;
                }
                switch_to(state, Current::Own, now, &mut fx);
            }
            delete_workspace(state, &label, now, &mut fx);
        }
    }
    fx
}

fn delete_workspace(state: &mut State, label: &str, now: Instant, fx: &mut Vec<Effect>) {
    let m = &mut state.model;
    let before = m.wss.custom.len();
    m.wss.custom.retain(|(l, _)| l != label);
    if m.wss.custom.len() != before {
        write_prefs(m, now, fx);
        say(m, now, format!("Workspace {label} deleted"));
    }
}

/// Something would be lost by leaving: show the list (#99) and wait for
/// Yes / Back. `to`: `None` = Telepad closes.
fn ask_leave(
    state: &mut State,
    to: Option<telepad_core::prefs::Current>,
    delete: Option<String>,
) -> bool {
    let (items, background) = leave_losses(state, to.is_none());
    if items.is_empty() {
        return false;
    }
    state.drafts.ws_leave = Some(crate::state::LeaveAsk {
        to,
        delete,
        items,
        background,
    });
    true
}

/// Close every window and open the windows of `to`. Unsaved, uncommitted
/// and unapplied work is dropped (the user said Yes, or there was none);
/// file windows with running transfers stay in the background (#99).
fn switch_to(
    state: &mut State,
    to: telepad_core::prefs::Current,
    now: Instant,
    fx: &mut Vec<Effect>,
) {
    use crate::files::FmMsg;
    // The current layout is already in its workspace (autosave). Closing
    // the windows below must not change it: nothing is current meanwhile.
    state.model.wss.current = telepad_core::prefs::Current::None;
    let files: Vec<_> = state
        .model
        .files
        .iter()
        .filter(|s| !s.background)
        .map(|s| s.id)
        .collect();
    for id in files {
        fx.extend(update(state, Msg::Files(FmMsg::Background(id)), now));
    }
    let dbs: Vec<_> = state.model.db.iter().map(|s| s.id).collect();
    for id in dbs {
        fx.extend(update(state, Msg::Db(crate::db::DbMsg::Close(id)), now));
    }
    if state.model.host_sync.is_some() {
        fx.extend(update(
            state,
            Msg::HostSync(crate::hostsync::HsMsg::Close),
            now,
        ));
    }
    state.drafts.host_sync = Default::default();
    if state.drafts.keys.is_some() {
        fx.extend(update(state, Msg::KeysClose, now));
    }
    let State {
        model: m,
        drafts: d,
    } = state;
    // Dialogs with settings not applied go too (#99): the same config may be
    // edited from the next workspace.
    d.editor = None;
    d.paste = None;
    m.import = None;
    if let Some(c) = &m.update.config {
        d.update_form = c.clone();
    }
    d.update_form_error = None;
    m.selected = None;
    m.wss.current = to;
    restore_current(m);
    place_pending_host(m);
    fx.extend(start_page_sessions(m, d));
    fx.extend(start_pending_files(m, d, now));
    fx.extend(start_pending_dbs(m, d, now));
    let mut ws = std::mem::take(&mut m.ws);
    ws.sync(m);
    ws.sync_keys(d.keys.is_some());
    m.ws = ws;
    m.ws_saved.0 = crate::workspace::layout(m);
    write_prefs(m, now, fx);
    let running = background_transfers(m);
    if running > 0 {
        say(
            m,
            now,
            format!("{running} file transfer(s) keep running in the background"),
        );
    }
}

fn say(m: &mut Model, now: Instant, text: impl Into<String>) {
    m.message = Some((text.into(), now));
}

fn elapsed(since: Option<Instant>, now: Instant) -> Option<Duration> {
    since.map(|t| now.saturating_duration_since(t))
}

fn selected_id(m: &Model) -> Option<i64> {
    m.selected
        .and_then(|i| m.hosts.get(i))
        .and_then(|h| h.entry.id)
}

/// Collect every monitored host: everything when the user asked (`manual`),
/// otherwise only the tiers that are due (live metrics always).
fn refresh_all(m: &mut Model, now: Instant, manual: bool, fx: &mut Vec<Effect>) {
    fx.push(Effect::Collect(
        m.hosts
            .iter()
            .filter(|h| !h.entry.unmonitored)
            .map(|h| Job {
                host: h.entry.clone(),
                scope: if manual { Scope::FULL } else { h.scope_at(now) },
            })
            .collect(),
    ));
    m.last_refresh = Some(now);
}

fn full(entry: &HostEntry) -> Job {
    Job {
        host: entry.clone(),
        scope: Scope::FULL,
    }
}

fn start_check(m: &mut Model, now: Instant, manual: bool, fx: &mut Vec<Effect>) {
    if m.update.phase != UpdatePhase::Idle {
        return;
    }
    let Some(c) = m.update.config.clone() else {
        return;
    };
    m.update.manual = manual;
    m.update.phase = UpdatePhase::Checking;
    m.update.last_check = Some(now);
    fx.push(Effect::CheckUpdate(c));
}

/// New host list from the database: keep results for unchanged hosts, keep
/// the selection, and collect anything new or changed.
fn reconcile(m: &mut Model, entries: Vec<HostEntry>, now: Instant, fx: &mut Vec<Effect>) {
    let selected = m.select_after_load.take().or_else(|| selected_id(m));
    let mut old = std::mem::take(&mut m.hosts);
    let mut to_poll = Vec::new();
    for entry in entries {
        let prev = old
            .iter()
            .position(|h| h.entry.id == entry.id)
            .map(|i| old.swap_remove(i));
        let state = match prev {
            Some(h) if h.entry == entry => h,
            _ => {
                if !entry.unmonitored {
                    to_poll.push(full(&entry));
                }
                HostState::new(entry)
            }
        };
        m.hosts.push(state);
    }
    m.selected = selected.and_then(|id| m.hosts.iter().position(|h| h.entry.id == Some(id)));
    if !to_poll.is_empty() {
        fx.push(Effect::Collect(to_poll));
    }
    if m.last_refresh.is_none() {
        m.last_refresh = Some(now);
    }
}

fn collected(m: &mut Model, u: collector::Update, now: Instant) {
    match u {
        collector::Update::Started(alias) => {
            if let Some(h) = m.hosts.iter_mut().find(|h| h.entry.alias == alias) {
                h.loading = true;
            }
        }
        collector::Update::Done {
            alias,
            result,
            scope,
            no_ping,
            elapsed,
        } => {
            if let Some(h) = m.hosts.iter_mut().find(|h| h.entry.alias == alias) {
                h.loading = false;
                h.updated = Some(now);
                h.elapsed = Some(elapsed);
                h.no_ping = no_ping;
                if !scope.ping_first {
                    h.ssh_at = Some(now);
                }
                match result {
                    Ok(mut r) => {
                        let prev = h.report.take();
                        if let Some(prev) = &prev {
                            scope.carry_over(&mut r, prev);
                        }
                        if scope.static_info {
                            h.static_at = Some(now);
                        }
                        if scope.slow {
                            h.slow_at = Some(now);
                        }
                        // Rebooted since the last look: kernel, updates and the
                        // reboot flag have likely changed, so read everything next time.
                        let rebooted = prev
                            .and_then(|p| p.uptime_secs)
                            .zip(r.uptime_secs)
                            .is_some_and(|(before, after)| after < before);
                        if rebooted {
                            h.static_at = None;
                            h.slow_at = None;
                        }
                        h.report = Some(*r);
                        h.error = None;
                    }
                    Err(e) => h.error = Some(e),
                }
            }
        }
    }
}

/// The host's Swarm, only on managers (workers cannot change the cluster).
fn swarm(m: &Model, host: usize) -> Option<(HostEntry, &telepad_core::cluster::Swarm)> {
    let h = m.hosts.get(host)?;
    let sw = h.report.as_ref()?.cluster.swarm.as_ref()?;
    sw.manager.then(|| (h.entry.clone(), sw))
}

fn gone(what: &str) -> String {
    format!("That {what} is gone; refresh the host")
}

fn open_plan(
    m: &mut Model,
    now: Instant,
    fx: &mut Vec<Effect>,
    plan: Option<(HostEntry, Result<Plan, String>)>,
) {
    match plan {
        Some((entry, Ok(plan))) => fx.push(Effect::OpenDockerAction(entry, plan)),
        Some((_, Err(e))) => say(m, now, e),
        None => say(m, now, "Refresh the host first"),
    }
}

/// Hosts whose `authorized_keys` the page reads: saved, SSH, among `aliases`
/// (`None` = all). Marks them loading and returns the effect.
fn load_keys(m: &mut Model, aliases: Option<Vec<String>>) -> Option<Effect> {
    let hosts: Vec<HostEntry> = m
        .hosts
        .iter()
        .map(|h| &h.entry)
        .filter(|e| key_selectable(e))
        .filter(|e| aliases.as_ref().is_none_or(|a| a.contains(&e.alias)))
        .cloned()
        .collect();
    if hosts.is_empty() {
        return None;
    }
    for h in &hosts {
        m.auth_keys
            .insert(h.alias.clone(), crate::state::AkHost::Loading);
    }
    Some(Effect::ListKeys(hosts))
}
