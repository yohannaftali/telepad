//! Host groups (#144) and clusters (#145): the Groups / Clusters pages (list,
//! add, rename, describe, reorder, delete) and the "Hosts - [name]" pages
//! that show the hosts of one group or cluster. Both kinds work the same and
//! are independent: a host has at most one group and at most one cluster.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use telepad_core::config::{HostSet, SetKind, check_set_name};

use crate::msg::Effect;
use crate::state::{HostState, Model};
use crate::workspace::Win;

pub type SetPageId = u64;

/// One "Hosts - [name]" page. `name` empty = the fixed entry (Ungrouped /
/// No cluster).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetPage {
    pub id: SetPageId,
    pub kind: SetKind,
    pub name: String,
}

impl SetPage {
    /// "Hosts - Office", "Hosts - Ungrouped".
    pub fn title(&self) -> String {
        format!("Hosts - {}", shown_name(self.kind, &self.name))
    }
}

/// A set's name as shown: the fixed entry's name for "".
pub fn shown_name(kind: SetKind, name: &str) -> &str {
    if name.is_empty() {
        kind.none_name()
    } else {
        name
    }
}

/// Indexes of the hosts in `name` (empty = hosts without one), in inventory order.
pub fn members(hosts: &[HostState], kind: SetKind, name: &str) -> Vec<usize> {
    hosts
        .iter()
        .enumerate()
        .filter(|(_, h)| h.entry.set(kind).eq_ignore_ascii_case(name))
        .map(|(i, _)| i)
        .collect()
}

/// Counts shown on the Groups / Clusters page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetStats {
    pub hosts: usize,
    pub down: usize,
    pub flagged: usize,
}

pub fn stats(hosts: &[HostState], kind: SetKind, name: &str) -> SetStats {
    let mut s = SetStats::default();
    for i in members(hosts, kind, name) {
        let h = &hosts[i];
        s.hosts += 1;
        if h.online() == Some(false) {
            s.down += 1;
        }
        if h.shown()
            .is_some_and(|r| !crate::table::host_flags(r).is_empty())
        {
            s.flagged += 1;
        }
    }
    s
}

/// Add / edit form of a group or cluster.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetForm {
    pub kind: Option<SetKind>,
    /// Name before editing; `None` when adding.
    pub original: Option<String>,
    pub name: String,
    pub description: String,
    pub error: Option<String>,
}

/// "Add hosts…" picker of a "Hosts - [name]" page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddHosts {
    pub page: SetPageId,
    pub filter: String,
    pub chosen: BTreeSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SetsDraft {
    /// Search box of the Groups / Clusters page.
    pub filter: BTreeMap<SetKind, String>,
    /// Search box and Table / Cards of each "Hosts - [name]" page.
    pub page_filter: BTreeMap<SetPageId, String>,
    pub page_cards: BTreeSet<SetPageId>,
    pub form: Option<SetForm>,
    /// Waiting for the delete confirmation.
    pub delete: Option<(SetKind, String)>,
    pub add_hosts: Option<AddHosts>,
    /// Hosts page: in group order, ungrouped last (shows the Group column).
    pub hosts_by_group: bool,
    /// Hosts page: Group / Cluster columns shown.
    pub hosts_columns: BTreeSet<SetKind>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StMsg {
    /// Groups and clusters as stored (comes with every host load).
    Loaded(Result<BTreeMap<SetKind, Vec<HostSet>>, String>),
    /// The Groups / Clusters page.
    OpenList(SetKind),
    /// "Hosts - [name]" (empty = the fixed entry).
    OpenHosts {
        kind: SetKind,
        name: String,
    },
    ClosePage(SetPageId),
    Add(SetKind),
    Edit {
        kind: SetKind,
        name: String,
    },
    FormSave,
    FormCancel,
    Delete {
        kind: SetKind,
        name: String,
    },
    DeleteYes,
    DeleteNo,
    /// Move a set one place up (-1) or down (+1).
    Move {
        kind: SetKind,
        name: String,
        by: i32,
    },
    /// Put hosts into `name` (empty = take them out).
    Assign {
        kind: SetKind,
        aliases: Vec<String>,
        name: String,
    },
    AddHostsOpen(SetPageId),
    AddHostsRun,
    AddHostsCancel,
    /// A store change finished: hosts and sets are read again.
    Saved(Result<(), String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum StEffect {
    Save {
        kind: SetKind,
        old: Option<String>,
        set: HostSet,
    },
    Delete {
        kind: SetKind,
        name: String,
    },
    Order {
        kind: SetKind,
        names: Vec<String>,
    },
    Assign {
        kind: SetKind,
        ids: Vec<i64>,
        name: String,
    },
}

/// The sets of `kind` as loaded.
pub fn list(m: &Model, kind: SetKind) -> &[HostSet] {
    m.sets.get(&kind).map(Vec::as_slice).unwrap_or(&[])
}

pub(crate) fn update(
    m: &mut Model,
    d: &mut SetsDraft,
    msg: StMsg,
    _now: Instant,
    say: &mut dyn FnMut(&mut Model, String),
) -> Vec<Effect> {
    let mut fx = Vec::new();
    match msg {
        StMsg::Loaded(Ok(sets)) => {
            m.sets = sets;
            prune_pages(m, d);
        }
        StMsg::Loaded(Err(e)) => say(m, format!("Groups and clusters: {e}")),
        StMsg::OpenList(kind) => m.ws.open(Win::Sets(kind)),
        StMsg::OpenHosts { kind, name } => {
            let name = if name.eq_ignore_ascii_case(kind.none_name()) {
                String::new()
            } else {
                name
            };
            if !name.is_empty() && !list(m, kind).iter().any(|s| s.name == name) {
                say(m, format!("{} '{name}' no longer exists", kind.title()));
                return fx;
            }
            // Its window opens (or comes to the front) with the workspace sync.
            if !m.set_pages.iter().any(|p| p.kind == kind && p.name == name) {
                m.set_pages_next += 1;
                let id = m.set_pages_next;
                m.set_pages.push(SetPage { id, kind, name });
            }
        }
        StMsg::ClosePage(id) => {
            m.set_pages.retain(|p| p.id != id);
            d.page_filter.remove(&id);
            d.page_cards.remove(&id);
            if d.add_hosts.as_ref().is_some_and(|a| a.page == id) {
                d.add_hosts = None;
            }
        }
        StMsg::Add(kind) => {
            d.form = Some(SetForm {
                kind: Some(kind),
                ..Default::default()
            });
        }
        StMsg::Edit { kind, name } => {
            if let Some(s) = list(m, kind).iter().find(|s| s.name == name) {
                d.form = Some(SetForm {
                    kind: Some(kind),
                    original: Some(s.name.clone()),
                    name: s.name.clone(),
                    description: s.description.clone(),
                    error: None,
                });
            }
        }
        StMsg::FormSave => {
            let Some(form) = &mut d.form else {
                return fx;
            };
            let Some(kind) = form.kind else {
                return fx;
            };
            let name = form.name.trim().to_string();
            let description = form.description.trim().to_string();
            if let Err(e) = check_set_name(kind, &name) {
                form.error = Some(e);
                return fx;
            }
            let taken = list(m, kind).iter().any(|s| {
                s.name.eq_ignore_ascii_case(&name)
                    && form
                        .original
                        .as_ref()
                        .is_none_or(|o| !o.eq_ignore_ascii_case(&s.name))
            });
            if taken {
                form.error = Some(format!(
                    "There is already a {} named '{name}'",
                    kind.as_str()
                ));
                return fx;
            }
            fx.push(Effect::Sets(StEffect::Save {
                kind,
                old: form.original.clone(),
                set: HostSet { name, description },
            }));
        }
        StMsg::FormCancel => d.form = None,
        StMsg::Delete { kind, name } => d.delete = Some((kind, name)),
        StMsg::DeleteNo => d.delete = None,
        StMsg::DeleteYes => {
            if let Some((kind, name)) = d.delete.take() {
                fx.push(Effect::Sets(StEffect::Delete { kind, name }));
            }
        }
        StMsg::Move { kind, name, by } => {
            let mut names: Vec<String> = list(m, kind).iter().map(|s| s.name.clone()).collect();
            let Some(at) = names.iter().position(|n| *n == name) else {
                return fx;
            };
            let to = at as i64 + i64::from(by);
            if to < 0 || to >= names.len() as i64 {
                return fx;
            }
            names.swap(at, to as usize);
            // Shown at once; the store follows.
            if let Some(v) = m.sets.get_mut(&kind) {
                v.swap(at, to as usize);
            }
            fx.push(Effect::Sets(StEffect::Order { kind, names }));
        }
        StMsg::Assign {
            kind,
            aliases,
            name,
        } => {
            if !name.is_empty()
                && let Err(e) = check_set_name(kind, &name)
            {
                say(m, e);
                return fx;
            }
            let ids: Vec<i64> = m
                .hosts
                .iter()
                .filter(|h| aliases.contains(&h.entry.alias))
                .filter_map(|h| h.entry.id)
                .collect();
            if !ids.is_empty() {
                fx.push(Effect::Sets(StEffect::Assign { kind, ids, name }));
            }
        }
        StMsg::AddHostsOpen(page) => {
            d.add_hosts = Some(AddHosts {
                page,
                ..Default::default()
            });
        }
        StMsg::AddHostsCancel => d.add_hosts = None,
        StMsg::AddHostsRun => {
            let Some(a) = d.add_hosts.take() else {
                return fx;
            };
            let Some(p) = m.set_pages.iter().find(|p| p.id == a.page).cloned() else {
                return fx;
            };
            let aliases: Vec<String> = a.chosen.into_iter().collect();
            fx.extend(update(
                m,
                d,
                StMsg::Assign {
                    kind: p.kind,
                    aliases,
                    name: p.name,
                },
                _now,
                say,
            ));
        }
        StMsg::Saved(Ok(())) => {
            // A renamed group / cluster keeps its open page.
            if let Some(SetForm {
                kind: Some(kind),
                original: Some(old),
                name,
                ..
            }) = d.form.take()
            {
                for p in m.set_pages.iter_mut() {
                    if p.kind == kind && p.name == old {
                        p.name = name.trim().to_string();
                    }
                }
            }
            fx.push(Effect::LoadHosts);
        }
        StMsg::Saved(Err(e)) => {
            match &mut d.form {
                Some(f) => f.error = Some(e),
                None => say(m, e),
            }
            fx.push(Effect::LoadHosts);
        }
    }
    fx
}

/// A page whose group / cluster was renamed follows it; one that was
/// deleted closes.
fn prune_pages(m: &mut Model, d: &mut SetsDraft) {
    let gone: Vec<SetPageId> = m
        .set_pages
        .iter()
        .filter(|p| {
            !p.name.is_empty()
                && !m
                    .sets
                    .get(&p.kind)
                    .is_some_and(|v| v.iter().any(|s| s.name == p.name))
        })
        .map(|p| p.id)
        .collect();
    for id in gone {
        // A rename that only changed the case keeps the page.
        let Some(p) = m.set_pages.iter_mut().find(|p| p.id == id) else {
            continue;
        };
        let same = m
            .sets
            .get(&p.kind)
            .and_then(|v| v.iter().find(|s| s.name.eq_ignore_ascii_case(&p.name)));
        match same {
            Some(s) => p.name = s.name.clone(),
            None => {
                m.set_pages.retain(|p| p.id != id);
                d.page_filter.remove(&id);
                d.page_cards.remove(&id);
            }
        }
    }
}
