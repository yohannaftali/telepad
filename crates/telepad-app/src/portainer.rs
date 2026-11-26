//! Portainers (#171): the Portainers page (register, edit, delete, status)
//! and one "Portainer - [name]" page per server with its environments and
//! stacks. Read-only: reads go through an SSH tunnel to the host that runs
//! Portainer, with an API token from the OS password store.

use std::time::Instant;

use telepad_core::db::Secret;
use telepad_core::portainer::{Portainer, ReadError, Snapshot, check_name};

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
            PtStatus::Error(ReadError::Other(_)) => "error".into(),
        }
    }
}

/// One "Portainer - [name]" page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtPage {
    pub id: PtPageId,
    pub name: String,
    /// The environment whose stacks are listed.
    pub env: Option<i64>,
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
                        env: None,
                    });
                }
            }
            if fresh {
                fx.extend(read(m, &name, None, now, say));
            }
        }
        PtMsg::ClosePage(id) => m.pt_pages.retain(|p| p.id != id),
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
