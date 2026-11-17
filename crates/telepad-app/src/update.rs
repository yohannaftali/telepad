//! The only place the [`Model`] changes. No I/O: anything outside the
//! process is requested as an [`Effect`], and the current time is a parameter.

use std::time::{Duration, Instant};

use telepad_core::collector;
use telepad_core::config::{Access, HostEntry};
use telepad_core::store::{ImportMode, ImportSummary};
use telepad_core::update::{CHECK_EVERY, CURRENT_VERSION, UpdateConfig};

use crate::msg::{Effect, Msg};
use crate::state::{Editor, HostState, ImportDialog, Model, State, UpdatePhase};

/// Apply `msg` to `state` at time `now`; return the effects to run.
pub fn update(state: &mut State, msg: Msg, now: Instant) -> Vec<Effect> {
    let mut fx = Vec::new();
    let State {
        model: m,
        drafts: d,
    } = state;
    match msg {
        Msg::Started {
            db_path,
            ssh_config_path,
            imported_from,
            update_config,
        } => {
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
            if m.update.enabled() && elapsed(m.update.last_check, now) >= Some(CHECK_EVERY) {
                start_check(m, now, false, &mut fx);
            }
            let interval = Duration::from_secs(u64::from(d.interval_secs));
            if d.auto_refresh && busy == 0 && elapsed(m.last_refresh, now) >= Some(interval) {
                refresh_all(m, now, &mut fx);
            }
        }

        Msg::RefreshAll => refresh_all(m, now, &mut fx),
        Msg::Refresh(i) => {
            if let Some(h) = m.hosts.get(i).filter(|h| !h.entry.unmonitored) {
                fx.push(Effect::Collect(vec![h.entry.clone()]));
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
        Msg::Select(i) => m.selected = i.filter(|&i| i < m.hosts.len()),
        Msg::SelectAlias(alias) => {
            m.selected = m.hosts.iter().position(|h| h.entry.alias == alias);
        }
        Msg::AddHost => d.editor = Some(Editor::new(HostEntry::new(""))),
        Msg::Edit(i) => {
            if let Some(h) = m.hosts.get(i) {
                d.editor = Some(Editor::new(h.entry.clone()));
            }
        }
        Msg::Clone(i) => {
            if let Some(h) = m.hosts.get(i) {
                d.editor = Some(Editor::clone_of(&h.entry));
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
        Msg::EditorBrowseKey => fx.push(Effect::PickKeyFile),
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
                });
            }
        }
        Msg::ImportOpened(Err((path, e))) => {
            say(m, now, format!("Cannot import {}: {e}", path.display()));
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
            m.update.window = true;
        }
        Msg::UpdateInstalled(Err(e)) => {
            m.update.phase = UpdatePhase::Idle;
            say(m, now, format!("Update failed: {e}"));
            m.update.error = Some(e);
        }
        Msg::Notice(text) => say(m, now, text),
    }
    fx
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

fn refresh_all(m: &mut Model, now: Instant, fx: &mut Vec<Effect>) {
    fx.push(Effect::Collect(
        m.hosts
            .iter()
            .filter(|h| !h.entry.unmonitored)
            .map(|h| h.entry.clone())
            .collect(),
    ));
    m.last_refresh = Some(now);
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
                    to_poll.push(entry.clone());
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
            elapsed,
        } => {
            if let Some(h) = m.hosts.iter_mut().find(|h| h.entry.alias == alias) {
                h.loading = false;
                h.updated = Some(now);
                h.elapsed = Some(elapsed);
                match result {
                    Ok(r) => {
                        h.report = Some(*r);
                        h.error = None;
                    }
                    Err(e) => h.error = Some(e),
                }
            }
        }
    }
}
