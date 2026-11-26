//! Portainers (#171): the Portainers page (register, edit, delete, status)
//! and one "Portainer - [name]" page per server with its environments and
//! stacks. Everything goes through an SSH tunnel to the host that runs
//! Portainer, with the user's API token from the OS password store. Reads
//! happen on open / Refresh; changes (#173: stack start / stop / delete,
//! container actions, the stack editor's Update) only after the user
//! confirmed them ([`needs_confirm`]), and are logged in `history.db`.

use std::time::Instant;

use telepad_core::db::Secret;
use telepad_core::portainer::{
    Action, ContainerOp, EnvVar, Portainer, ReadError, Snapshot, StackContainer, StackDetail,
    StackUpdate, check_name,
};

use crate::msg::Effect;
use crate::state::Model;
use crate::workspace::Win;

pub type PtPageId = u64;

/// A registered Portainer and what Telepad last learnt from it.
#[derive(Debug, Clone, PartialEq)]
pub struct PtRow {
    pub p: Portainer,
    pub status: PtStatus,
    /// Last good read.
    pub data: Option<Snapshot>,
    /// When it was last read (good or not).
    pub checked: Option<Instant>,
    pub reading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PtStatus {
    /// Not read yet in this run.
    #[default]
    Unknown,
    Ok,
    Error(ReadError),
}

impl PtStatus {
    pub fn text(&self) -> String {
        match self {
            PtStatus::Unknown => "not read yet".into(),
            PtStatus::Ok => "OK".into(),
            PtStatus::Error(ReadError::NoToken) => "no token".into(),
            PtStatus::Error(ReadError::Unreachable(_)) => "unreachable".into(),
            PtStatus::Error(ReadError::Rejected) => "token rejected".into(),
            PtStatus::Error(ReadError::CertChanged { .. }) => "certificate changed".into(),
            PtStatus::Error(ReadError::TooOld(v)) => format!("too old ({v})"),
            PtStatus::Error(ReadError::Other(_) | ReadError::Changed) => "error".into(),
        }
    }
}

/// One "Portainer - [name]" page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PtPage {
    pub id: PtPageId,
    pub name: String,
    /// The environment whose stacks are listed.
    pub env: Option<i64>,
    /// The stack whose containers are listed (#173).
    pub stack: Option<i64>,
    pub containers: Option<Result<Vec<StackContainer>, String>>,
    pub loading_containers: bool,
    /// An action running from this page ("Stopping stack shop…").
    pub busy: Option<String>,
}

pub type EditorId = u64;

/// A stack editor window (#173), like Portainer's Editor tab.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StackEditor {
    pub id: EditorId,
    pub portainer: String,
    pub stack_id: i64,
    pub env_id: i64,
    pub name: String,
    /// As loaded from Portainer (file, variables, UpdateDate).
    pub detail: Option<StackDetail>,
    pub loading: bool,
    pub saving: bool,
    pub error: Option<String>,
    /// Changed in Portainer since it was loaded: Reload or Overwrite.
    pub conflict: bool,
}

impl StackEditor {
    pub fn title(&self) -> String {
        format!("Stack - {}", self.name)
    }
}

/// What the user typed in a stack editor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditorDraft {
    pub file: String,
    pub vars: Vec<EnvVar>,
    /// Show variable values.
    pub reveal: bool,
    pub prune: bool,
    /// Re-pull image and redeploy.
    pub repull: bool,
    /// The review step (diff, changed names) before Update.
    pub review: bool,
    /// Closing with unsaved changes: asking.
    pub ask_close: bool,
}

/// A confirmation waiting for the user (#173).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtConfirm {
    pub portainer: String,
    pub page: PtPageId,
    pub act: Action,
    /// "Stop stack shop on Docker Center".
    pub title: String,
    /// The name to type (stack delete), else empty.
    pub type_name: String,
    pub typed: String,
}

impl PtPage {
    pub fn title(&self) -> String {
        format!("Portainer - {}", self.name)
    }
}

/// Add / edit form.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PtForm {
    /// Name before editing; `None` when adding.
    pub original: Option<String>,
    pub name: String,
    pub alias: String,
    pub port: String,
    pub https: bool,
    /// Typed token; empty when editing = keep the saved one.
    pub token: Secret,
    pub error: Option<String>,
    /// Test result: version and environments, or why it failed.
    pub test: Option<Result<String, String>>,
    pub testing: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PtDraft {
    pub form: Option<PtForm>,
    /// Waiting for the delete confirmation.
    pub delete: Option<String>,
    /// A stack / container action waiting for its confirmation (#173).
    pub confirm: Option<PtConfirm>,
    pub editors: std::collections::BTreeMap<EditorId, EditorDraft>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PtMsg {
    /// The registered Portainers (comes with every host load).
    Loaded(Result<Vec<Portainer>, String>),
    /// The Portainers page; reads the ones not read yet.
    OpenList,
    /// "Portainer - [name]"; reads it.
    Open(String),
    ClosePage(PtPageId),
    Refresh(String),
    RefreshAll,
    Add,
    Edit(String),
    FormSave,
    FormCancel,
    /// Try the form's settings (and token) without saving.
    FormTest,
    Tested(Result<String, String>),
    Delete(String),
    DeleteYes,
    DeleteNo,
    /// A read finished. `seen` is the certificate to pin on first use.
    Read {
        name: String,
        result: Box<Result<(Snapshot, Option<String>), ReadError>>,
    },
    /// Trust the changed certificate shown in the error and read again.
    AcceptCert(String),
    /// Show this environment's stacks on a page.
    SelectEnv {
        page: PtPageId,
        env: Option<i64>,
    },
    /// A store change finished: the list is read again.
    Saved(Result<(), String>),
    /// Show this stack's containers on a page (#173).
    SelectStack {
        page: PtPageId,
        stack: Option<i64>,
    },
    ContainersLoaded {
        page: PtPageId,
        stack: i64,
        result: Result<Vec<StackContainer>, String>,
    },
    /// A stack / container action: asks first when [`needs_confirm`].
    Ask {
        page: PtPageId,
        act: Action,
        /// "Stop stack shop on Docker Center".
        title: String,
        /// For a stack delete: the stack name to type.
        type_name: String,
    },
    ConfirmYes,
    ConfirmNo,
    /// Container Remove: also its anonymous volumes.
    ConfirmVolumes(bool),
    Acted {
        portainer: String,
        page: PtPageId,
        title: String,
        result: Result<(), String>,
    },
    /// Open the stack editor (#173).
    EditStack {
        page: PtPageId,
        stack: i64,
    },
    StackLoaded {
        editor: EditorId,
        result: Box<Result<StackDetail, ReadError>>,
    },
    /// Update the stack…: check the form, then show the review.
    EditorReview(EditorId),
    EditorBack(EditorId),
    /// Send the update; `force` = overwrite a stack changed meanwhile.
    EditorUpdate {
        editor: EditorId,
        force: bool,
    },
    StackUpdated {
        editor: EditorId,
        result: Result<(), ReadError>,
    },
    /// Load the stack again, dropping the edits.
    EditorReload(EditorId),
    /// Close; asks first with unsaved edits.
    EditorClose(EditorId),
    EditorDiscard(EditorId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PtEffect {
    /// Add / change it; `token` = a new token for the password store.
    Save {
        old: Option<String>,
        p: Portainer,
        token: Option<Secret>,
    },
    /// Remove it and its token.
    Delete(String),
    /// Read it through `host` (token from the password store); `accept` =
    /// a changed certificate the user accepted.
    Read {
        p: Portainer,
        host: telepad_core::config::HostEntry,
        accept: Option<String>,
    },
    /// The form's Test: `token` typed, or `None` for the saved one.
    Test {
        p: Portainer,
        host: telepad_core::config::HostEntry,
        token: Option<Secret>,
    },
    /// Pin a certificate fingerprint.
    Pin { name: String, fingerprint: String },
    /// The containers of a stack (#173).
    Containers {
        p: Portainer,
        host: telepad_core::config::HostEntry,
        page: PtPageId,
        stack: telepad_core::portainer::Stack,
    },
    /// A confirmed action (#173); logged in `history.db`.
    Act {
        p: Portainer,
        host: telepad_core::config::HostEntry,
        page: PtPageId,
        act: Action,
        title: String,
    },
    /// The stack and its compose file for the editor.
    LoadStack {
        p: Portainer,
        host: telepad_core::config::HostEntry,
        editor: EditorId,
        stack: i64,
    },
    /// The editor's Update the stack; `expect` = the UpdateDate it was
    /// loaded with (`None` = overwrite whatever is there).
    Update {
        p: Portainer,
        host: telepad_core::config::HostEntry,
        editor: EditorId,
        upd: StackUpdate,
        expect: Option<Option<i64>>,
        /// What changed, for the history log (names only, never values).
        summary: String,
    },
}

/// Asks before running: stack start / stop / delete, container kill /
/// remove. Container start / stop / restart / pause / resume run at once.
pub fn needs_confirm(a: &Action) -> bool {
    match a {
        Action::Container { op, .. } => op.risky(),
        _ => true,
    }
}

/// The variable names added, removed or changed between `old` and `new`
/// (for the review; values are never listed).
pub fn var_changes(old: &[EnvVar], new: &[EnvVar]) -> Vec<String> {
    let mut out = Vec::new();
    for v in new {
        match old.iter().find(|o| o.name == v.name) {
            None => out.push(format!("+ {}", v.name)),
            Some(o) if o.value != v.value => out.push(format!("~ {}", v.name)),
            _ => {}
        }
    }
    for o in old {
        if !new.iter().any(|v| v.name == o.name) {
            out.push(format!("- {}", o.name));
        }
    }
    out
}

/// A variable name Portainer / compose accept: one word, no `=`.
pub fn check_var_name(n: &str) -> Result<(), String> {
    if n.is_empty() {
        return Err("A variable needs a name".into());
    }
    if n.chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '=')
    {
        return Err(format!("'{n}': no spaces or '=' in a variable name"));
    }
    Ok(())
}

/// The editor has edits not sent yet.
pub fn editor_dirty(e: &StackEditor, d: &EditorDraft) -> bool {
    e.detail
        .as_ref()
        .is_some_and(|x| x.file != d.file || x.env != d.vars)
}

pub(crate) fn update(
    m: &mut Model,
    d: &mut PtDraft,
    msg: PtMsg,
    now: Instant,
    say: &mut dyn FnMut(&mut Model, String),
) -> Vec<Effect> {
    let mut fx = Vec::new();
    match msg {
        PtMsg::Loaded(Ok(list)) => {
            let old = std::mem::take(&mut m.portainers);
            m.portainers = list
                .into_iter()
                .map(|p| {
                    match old.iter().find(|r| r.p.name.eq_ignore_ascii_case(&p.name)) {
                        // Same server: what was read stays.
                        Some(r) if same_server(&r.p, &p) => PtRow { p, ..r.clone() },
                        _ => PtRow {
                            p,
                            status: PtStatus::Unknown,
                            data: None,
                            checked: None,
                            reading: false,
                        },
                    }
                })
                .collect();
            m.portainers_loaded = true;
            // A page whose Portainer was renamed follows it; a deleted one closes.
            m.pt_editors
                .retain(|e| m.portainers.iter().any(|r| r.p.name == e.portainer));
            m.pt_pages.retain_mut(|pg| {
                match m
                    .portainers
                    .iter()
                    .find(|r| r.p.name.eq_ignore_ascii_case(&pg.name))
                {
                    Some(r) => {
                        pg.name = r.p.name.clone();
                        true
                    }
                    None => false,
                }
            });
            // Pages and the open list read what they show.
            let mut want: Vec<String> = m.pt_pages.iter().map(|p| p.name.clone()).collect();
            if m.ws.is_open(Win::Portainers) {
                want.extend(m.portainers.iter().map(|r| r.p.name.clone()));
            }
            for name in want {
                if m.pt(&name)
                    .is_some_and(|r| r.checked.is_none() && !r.reading)
                {
                    fx.extend(read(m, &name, None, now, say));
                }
            }
        }
        PtMsg::Loaded(Err(e)) => say(m, format!("Portainers: {e}")),
        PtMsg::OpenList => {
            m.ws.open(Win::Portainers);
            let names: Vec<String> = m
                .portainers
                .iter()
                .filter(|r| r.checked.is_none() && !r.reading)
                .map(|r| r.p.name.clone())
                .collect();
            for n in names {
                fx.extend(read(m, &n, None, now, say));
            }
        }
        PtMsg::Open(name) => {
            let Some(row) = m.pt(&name) else {
                say(m, format!("Portainer '{name}' no longer exists"));
                return fx;
            };
            let name = row.p.name.clone();
            let fresh = row.checked.is_none() && !row.reading;
            match m.pt_pages.iter().find(|p| p.name == name) {
                // Its window comes to the front (#136).
                Some(p) => m.ws.open(Win::Portainer(p.id)),
                // Its window opens with the workspace sync.
                None => {
                    m.pt_pages_next += 1;
                    let id = m.pt_pages_next;
                    m.pt_pages.push(PtPage {
                        id,
                        name: name.clone(),
                        ..Default::default()
                    });
                }
            }
            if fresh {
                fx.extend(read(m, &name, None, now, say));
            }
        }
        PtMsg::ClosePage(id) => m.pt_pages.retain(|p| p.id != id),
        PtMsg::SelectStack { page, stack } => {
            let Some(pg) = m.pt_pages.iter_mut().find(|p| p.id == page) else {
                return fx;
            };
            pg.stack = stack;
            pg.containers = None;
            if let Some(s) = stack {
                fx.extend(load_containers(m, page, s));
            }
        }
        PtMsg::ContainersLoaded {
            page,
            stack,
            result,
        } => {
            if let Some(pg) = m
                .pt_pages
                .iter_mut()
                .find(|p| p.id == page && p.stack == Some(stack))
            {
                pg.loading_containers = false;
                pg.containers = Some(result);
            }
        }
        PtMsg::Ask {
            page,
            act,
            title,
            type_name,
        } => {
            let Some(pg) = m.pt_pages.iter().find(|p| p.id == page) else {
                return fx;
            };
            let portainer = pg.name.clone();
            if needs_confirm(&act) {
                d.confirm = Some(PtConfirm {
                    portainer,
                    page,
                    act,
                    title,
                    type_name,
                    typed: String::new(),
                });
            } else {
                fx.extend(run_action(m, &portainer, page, act, title));
            }
        }
        PtMsg::ConfirmNo => d.confirm = None,
        PtMsg::ConfirmVolumes(v) => {
            if let Some(c) = &mut d.confirm
                && let Action::Container {
                    op: ContainerOp::Remove { volumes },
                    ..
                } = &mut c.act
            {
                *volumes = v;
            }
        }
        PtMsg::ConfirmYes => {
            let Some(c) = d.confirm.take() else {
                return fx;
            };
            // A stack delete runs only with its name typed.
            if !c.type_name.is_empty() && c.typed.trim() != c.type_name {
                d.confirm = Some(c);
                return fx;
            }
            fx.extend(run_action(m, &c.portainer, c.page, c.act, c.title));
        }
        PtMsg::Acted {
            portainer,
            page,
            title,
            result,
        } => {
            if let Some(pg) = m.pt_pages.iter_mut().find(|p| p.id == page) {
                pg.busy = None;
            }
            match result {
                Ok(()) => say(m, format!("Portainer {portainer}: {title}: done")),
                Err(e) => say(m, format!("Portainer {portainer}: {title}: {e}")),
            }
            // What changed is read again: the stacks, and the containers shown.
            fx.extend(read(m, &portainer, None, now, say));
            let shown: Vec<(PtPageId, i64)> = m
                .pt_pages
                .iter()
                .filter(|p| p.name == portainer)
                .filter_map(|p| p.stack.map(|s| (p.id, s)))
                .collect();
            for (pg, s) in shown {
                fx.extend(load_containers(m, pg, s));
            }
        }
        PtMsg::EditStack { page, stack } => {
            let Some(pg) = m.pt_pages.iter().find(|p| p.id == page) else {
                return fx;
            };
            let portainer = pg.name.clone();
            if let Some(e) = m
                .pt_editors
                .iter()
                .find(|e| e.portainer == portainer && e.stack_id == stack)
            {
                m.ws.open(Win::PtStack(e.id));
                return fx;
            }
            let Some(s) = m
                .pt(&portainer)
                .and_then(|r| r.data.as_ref())
                .and_then(|d| d.stacks.iter().find(|s| s.id == stack))
                .cloned()
            else {
                return fx;
            };
            m.pt_editors_next += 1;
            let id = m.pt_editors_next;
            m.pt_editors.push(StackEditor {
                id,
                portainer: portainer.clone(),
                stack_id: s.id,
                env_id: s.env,
                name: s.name.clone(),
                loading: true,
                ..Default::default()
            });
            d.editors.insert(id, EditorDraft::default());
            fx.extend(load_stack(m, id));
        }
        PtMsg::StackLoaded { editor, result } => {
            let Some(e) = m.pt_editors.iter_mut().find(|e| e.id == editor) else {
                return fx;
            };
            e.loading = false;
            match *result {
                Ok(detail) => {
                    e.error = None;
                    e.conflict = false;
                    let dr = d.editors.entry(editor).or_default();
                    dr.file = detail.file.clone();
                    dr.vars = detail.env.clone();
                    dr.review = false;
                    e.detail = Some(detail);
                }
                Err(err) => e.error = Some(err.to_string()),
            }
        }
        PtMsg::EditorReview(editor) => {
            let (Some(e), Some(dr)) = (
                m.pt_editors.iter_mut().find(|e| e.id == editor),
                d.editors.get_mut(&editor),
            ) else {
                return fx;
            };
            e.error = None;
            if e.detail.as_ref().is_some_and(|x| x.git.is_some()) {
                e.error = Some("A git-based stack is edited in Portainer".into());
                return fx;
            }
            if dr.file.trim().is_empty() {
                e.error = Some("The compose file is empty".into());
                return fx;
            }
            let mut seen = std::collections::BTreeSet::new();
            for v in &dr.vars {
                if let Err(err) = check_var_name(v.name.trim()) {
                    e.error = Some(err);
                    return fx;
                }
                if !seen.insert(v.name.trim().to_string()) {
                    e.error = Some(format!("The variable {} is there twice", v.name.trim()));
                    return fx;
                }
            }
            dr.review = true;
        }
        PtMsg::EditorBack(editor) => {
            if let Some(dr) = d.editors.get_mut(&editor) {
                dr.review = false;
            }
        }
        PtMsg::EditorUpdate { editor, force } => {
            let (Some(e), Some(dr)) = (
                m.pt_editors.iter().find(|e| e.id == editor),
                d.editors.get(&editor),
            ) else {
                return fx;
            };
            let Some(detail) = &e.detail else {
                return fx;
            };
            if e.saving || detail.git.is_some() {
                return fx;
            }
            let vars: Vec<EnvVar> = dr
                .vars
                .iter()
                .map(|v| EnvVar {
                    name: v.name.trim().to_string(),
                    value: v.value.clone(),
                })
                .collect();
            let mut changes = var_changes(&detail.env, &vars);
            if detail.file != dr.file {
                changes.insert(0, "compose file changed".into());
            }
            let summary = if changes.is_empty() {
                "redeploy without changes".to_string()
            } else {
                changes.join(", ")
            };
            let upd = StackUpdate {
                id: e.stack_id,
                env: e.env_id,
                file: dr.file.clone(),
                vars,
                prune: dr.prune,
                repull: dr.repull,
            };
            let expect = (!force).then_some(detail.updated);
            let Some((p, host)) = target(m, &e.portainer) else {
                return fx;
            };
            if let Some(e) = m.pt_editors.iter_mut().find(|e| e.id == editor) {
                e.saving = true;
                e.error = None;
            }
            fx.push(Effect::Portainer(PtEffect::Update {
                p,
                host,
                editor,
                upd,
                expect,
                summary,
            }));
        }
        PtMsg::StackUpdated { editor, result } => {
            let Some(e) = m.pt_editors.iter_mut().find(|e| e.id == editor) else {
                return fx;
            };
            e.saving = false;
            match result {
                Ok(()) => {
                    let (portainer, name) = (e.portainer.clone(), e.name.clone());
                    if let Some(dr) = d.editors.get_mut(&editor) {
                        dr.review = false;
                    }
                    say(m, format!("Portainer {portainer}: stack {name} updated"));
                    fx.extend(load_stack(m, editor));
                    fx.extend(read(m, &portainer, None, now, say));
                }
                Err(ReadError::Changed) => e.conflict = true,
                Err(err) => e.error = Some(err.to_string()),
            }
        }
        PtMsg::EditorReload(editor) => fx.extend(load_stack(m, editor)),
        PtMsg::EditorClose(editor) => {
            let dirty = match (
                m.pt_editors.iter().find(|e| e.id == editor),
                d.editors.get(&editor),
            ) {
                (Some(e), Some(dr)) => editor_dirty(e, dr),
                _ => false,
            };
            if dirty {
                if let Some(dr) = d.editors.get_mut(&editor) {
                    dr.ask_close = true;
                }
            } else {
                m.pt_editors.retain(|e| e.id != editor);
                d.editors.remove(&editor);
            }
        }
        PtMsg::EditorDiscard(editor) => {
            m.pt_editors.retain(|e| e.id != editor);
            d.editors.remove(&editor);
        }
        PtMsg::Refresh(name) => fx.extend(read(m, &name, None, now, say)),
        PtMsg::RefreshAll => {
            let names: Vec<String> = m.portainers.iter().map(|r| r.p.name.clone()).collect();
            for n in names {
                fx.extend(read(m, &n, None, now, say));
            }
        }
        PtMsg::Add => {
            // The first host that runs a Portainer server, with its port.
            let (alias, port, https) = suggestions(m).into_iter().next().unwrap_or_default();
            d.form = Some(PtForm {
                alias,
                port: if port == 0 {
                    "9443".into()
                } else {
                    port.to_string()
                },
                https: port == 0 || https,
                ..Default::default()
            });
        }
        PtMsg::Edit(name) => {
            if let Some(r) = m.pt(&name) {
                d.form = Some(PtForm {
                    original: Some(r.p.name.clone()),
                    name: r.p.name.clone(),
                    alias: r.p.alias.clone(),
                    port: r.p.port.to_string(),
                    https: r.p.https,
                    ..Default::default()
                });
            }
        }
        PtMsg::FormCancel => d.form = None,
        PtMsg::FormSave => {
            let Some(form) = &mut d.form else {
                return fx;
            };
            match checked_form(m, form) {
                Ok(p) => {
                    form.error = None;
                    if form.original.is_none() && form.token.is_empty() {
                        form.error = Some("Paste the API access token (Portainer: My account ▸ Access tokens ▸ Add access token)".into());
                        return fx;
                    }
                    let token = (!form.token.is_empty()).then(|| form.token.clone());
                    fx.push(Effect::Portainer(PtEffect::Save {
                        old: form.original.clone(),
                        p,
                        token,
                    }));
                }
                Err(e) => form.error = Some(e),
            }
        }
        PtMsg::FormTest => {
            let Some(form) = &mut d.form else {
                return fx;
            };
            let p = match checked_form(m, form) {
                Ok(p) => p,
                Err(e) => {
                    form.error = Some(e);
                    return fx;
                }
            };
            form.error = None;
            let Some(host) = m.hosts.iter().find(|h| h.entry.alias == p.alias) else {
                return fx;
            };
            // A token typed now, else the one saved for this Portainer.
            let token = (!form.token.is_empty()).then(|| form.token.clone());
            if token.is_none() && form.original.is_none() {
                form.test = Some(Err("Paste the API access token first".into()));
                return fx;
            }
            let p = Portainer {
                name: form.original.clone().unwrap_or(p.name),
                ..p
            };
            form.testing = true;
            form.test = None;
            fx.push(Effect::Portainer(PtEffect::Test {
                p,
                host: host.entry.clone(),
                token,
            }));
        }
        PtMsg::Tested(r) => {
            if let Some(f) = &mut d.form {
                f.testing = false;
                f.test = Some(r);
            }
        }
        PtMsg::Delete(name) => d.delete = Some(name),
        PtMsg::DeleteNo => d.delete = None,
        PtMsg::DeleteYes => {
            if let Some(name) = d.delete.take() {
                fx.push(Effect::Portainer(PtEffect::Delete(name)));
            }
        }
        PtMsg::Read { name, result } => {
            let Some(at) = m
                .portainers
                .iter()
                .position(|r| r.p.name.eq_ignore_ascii_case(&name))
            else {
                return fx;
            };
            let row = &mut m.portainers[at];
            row.reading = false;
            row.checked = Some(now);
            match *result {
                Ok((snap, seen)) => {
                    row.status = PtStatus::Ok;
                    row.data = Some(snap);
                    // Trust on first use (or the accepted change): pin it.
                    if let Some(fp) = seen.filter(|fp| !row.p.fingerprint.eq_ignore_ascii_case(fp))
                    {
                        row.p.fingerprint = fp.clone();
                        fx.push(Effect::Portainer(PtEffect::Pin {
                            name: row.p.name.clone(),
                            fingerprint: fp,
                        }));
                    }
                }
                Err(e) => row.status = PtStatus::Error(e),
            }
        }
        PtMsg::AcceptCert(name) => {
            let seen = match m.pt(&name).map(|r| &r.status) {
                Some(PtStatus::Error(ReadError::CertChanged { seen, .. })) => seen.clone(),
                _ => return fx,
            };
            fx.extend(read(m, &name, Some(seen), now, say));
        }
        PtMsg::SelectEnv { page, env } => {
            if let Some(p) = m.pt_pages.iter_mut().find(|p| p.id == page) {
                p.env = env;
            }
        }
        PtMsg::Saved(Ok(())) => {
            // A renamed Portainer keeps its open page.
            if let Some(PtForm {
                original: Some(old),
                name,
                ..
            }) = d.form.take()
            {
                for p in m.pt_pages.iter_mut().filter(|p| p.name == old) {
                    p.name = name.trim().to_string();
                }
                if let Some(r) = m.portainers.iter_mut().find(|r| r.p.name == old) {
                    r.p.name = name.trim().to_string();
                }
            }
            d.delete = None;
            fx.push(Effect::LoadHosts);
        }
        PtMsg::Saved(Err(e)) => {
            match &mut d.form {
                Some(f) => f.error = Some(e),
                None => say(m, e),
            }
            fx.push(Effect::LoadHosts);
        }
    }
    fx
}

/// The Portainer `name` and the host its tunnel goes through.
fn target(m: &Model, name: &str) -> Option<(Portainer, telepad_core::config::HostEntry)> {
    let p = m.pt(name)?.p.clone();
    let host = m
        .hosts
        .iter()
        .find(|h| h.entry.alias == p.alias)?
        .entry
        .clone();
    Some((p, host))
}

fn run_action(
    m: &mut Model,
    portainer: &str,
    page: PtPageId,
    act: Action,
    title: String,
) -> Vec<Effect> {
    let Some((p, host)) = target(m, portainer) else {
        return Vec::new();
    };
    if let Some(pg) = m.pt_pages.iter_mut().find(|p| p.id == page) {
        pg.busy = Some(format!("{title}…"));
    }
    vec![Effect::Portainer(PtEffect::Act {
        p,
        host,
        page,
        act,
        title,
    })]
}

fn load_containers(m: &mut Model, page: PtPageId, stack: i64) -> Vec<Effect> {
    let Some(pg) = m.pt_pages.iter().find(|p| p.id == page) else {
        return Vec::new();
    };
    let name = pg.name.clone();
    let Some(s) = m
        .pt(&name)
        .and_then(|r| r.data.as_ref())
        .and_then(|d| d.stacks.iter().find(|s| s.id == stack))
        .cloned()
    else {
        return Vec::new();
    };
    let Some((p, host)) = target(m, &name) else {
        return Vec::new();
    };
    if let Some(pg) = m.pt_pages.iter_mut().find(|p| p.id == page) {
        pg.loading_containers = true;
    }
    vec![Effect::Portainer(PtEffect::Containers {
        p,
        host,
        page,
        stack: s,
    })]
}

fn load_stack(m: &mut Model, editor: EditorId) -> Vec<Effect> {
    let Some(e) = m.pt_editors.iter().find(|e| e.id == editor) else {
        return Vec::new();
    };
    let (name, stack) = (e.portainer.clone(), e.stack_id);
    let Some((p, host)) = target(m, &name) else {
        return Vec::new();
    };
    if let Some(e) = m.pt_editors.iter_mut().find(|e| e.id == editor) {
        e.loading = true;
    }
    vec![Effect::Portainer(PtEffect::LoadStack {
        p,
        host,
        editor,
        stack,
    })]
}

/// Same address and port: what was read from it still holds.
fn same_server(a: &Portainer, b: &Portainer) -> bool {
    a.alias == b.alias && a.port == b.port && a.https == b.https
}

/// Start a read of `name` (none while one is running).
fn read(
    m: &mut Model,
    name: &str,
    accept: Option<String>,
    _now: Instant,
    say: &mut dyn FnMut(&mut Model, String),
) -> Vec<Effect> {
    let Some(at) = m
        .portainers
        .iter()
        .position(|r| r.p.name.eq_ignore_ascii_case(name))
    else {
        return Vec::new();
    };
    if m.portainers[at].reading {
        return Vec::new();
    }
    let p = m.portainers[at].p.clone();
    let Some(host) = m.hosts.iter().find(|h| h.entry.alias == p.alias) else {
        say(
            m,
            format!("Portainer '{}': host {} is not in Telepad", p.name, p.alias),
        );
        return Vec::new();
    };
    let host = host.entry.clone();
    m.portainers[at].reading = true;
    vec![Effect::Portainer(PtEffect::Read { p, host, accept })]
}

/// The form as a Portainer, or what is wrong with it.
fn checked_form(m: &Model, f: &PtForm) -> Result<Portainer, String> {
    let name = f.name.trim();
    check_name(name)?;
    let taken = m.portainers.iter().any(|r| {
        r.p.name.eq_ignore_ascii_case(name)
            && f.original
                .as_ref()
                .is_none_or(|o| !o.eq_ignore_ascii_case(&r.p.name))
    });
    if taken {
        return Err(format!("There is already a Portainer named '{name}'"));
    }
    if !m.hosts.iter().any(|h| h.entry.alias == f.alias) {
        return Err("Pick the host Portainer runs on".into());
    }
    let port: u16 = f
        .port
        .trim()
        .parse()
        .ok()
        .filter(|p| *p > 0)
        .ok_or("The port is a number from 1 to 65535")?;
    let fingerprint = f
        .original
        .as_ref()
        .and_then(|o| m.pt(o))
        .filter(|r| r.p.alias == f.alias && r.p.port == port && r.p.https == f.https)
        .map(|r| r.p.fingerprint.clone())
        .unwrap_or_default();
    Ok(Portainer {
        name: name.to_string(),
        alias: f.alias.clone(),
        port,
        https: f.https,
        fingerprint,
    })
}

/// Hosts that run a Portainer server (from their last report), with the
/// published port: (alias, port, https). HTTPS 9443 first, then HTTP 9000.
pub fn suggestions(m: &Model) -> Vec<(String, u16, bool)> {
    use telepad_core::model::PortainerRole;
    let mut out = Vec::new();
    for h in &m.hosts {
        let Some(r) = h.shown() else { continue };
        for (role, c) in r.docker.portainer() {
            if role != PortainerRole::Server {
                continue;
            }
            let https = c.published.iter().find(|p| p.container_port == 9443);
            let http = c.published.iter().find(|p| p.container_port == 9000);
            if let Some(p) = https {
                out.push((h.entry.alias.clone(), p.host_port, true));
            } else if let Some(p) = http {
                out.push((h.entry.alias.clone(), p.host_port, false));
            }
        }
    }
    out
}

impl Model {
    /// The registered Portainer named `name` (any case).
    pub fn pt(&self, name: &str) -> Option<&PtRow> {
        self.portainers
            .iter()
            .find(|r| r.p.name.eq_ignore_ascii_case(name))
    }
}
