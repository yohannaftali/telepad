//! Log info page (#126): SSH logins of one host from its systemd journal,
//! one flat table (sort, filter, pages; newest first), and the user-started
//! Register / Unregister of journal access in a terminal.

use std::cmp::Ordering;
use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::login_log::{Login, LoginLog};

pub type LogId = u64;

/// One open Log info page.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginSession {
    pub id: LogId,
    pub alias: String,
    /// A read is running.
    pub loading: bool,
    /// Last read (`None` before the first one finished).
    pub log: Option<Result<LoginLog, String>>,
    pub read_at: Option<Instant>,
}

/// Columns of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Col {
    Time,
    User,
    From,
    Method,
    KeyType,
    Fingerprint,
    KeyName,
    Who,
}

impl Col {
    /// Table order: the long fingerprint last, so the names stay in view.
    pub const ALL: [Col; 8] = [
        Col::Time,
        Col::User,
        Col::From,
        Col::Method,
        Col::KeyType,
        Col::KeyName,
        Col::Who,
        Col::Fingerprint,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Col::Time => "Time (host)",
            Col::User => "User",
            Col::From => "From",
            Col::Method => "Method",
            Col::KeyType => "Key type",
            Col::Fingerprint => "Fingerprint",
            Col::KeyName => "Key name",
            Col::Who => "Telepad",
        }
    }

    pub fn index(self) -> usize {
        Col::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

/// Rows per page of a new page (the wtbapp default; #140).
pub const PAGE_SIZE: usize = 20;

/// What the view binds to: sort, filters, page, toggles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginDraft {
    /// `None`: newest first.
    pub sort: Option<(Col, Dir)>,
    /// One filter box per column ([`Col::ALL`] order).
    pub filters: [String; 8],
    /// Page, rows per page (the footer, #140).
    pub pager: crate::pager::Pager,
    /// Show the logins of this PC with Telepad's key (hidden by default:
    /// Telepad logs in on every refresh).
    pub show_own: bool,
}

impl Default for LoginDraft {
    fn default() -> Self {
        Self {
            sort: None,
            filters: Default::default(),
            pager: crate::pager::Pager::new(PAGE_SIZE),
            show_own: false,
        }
    }
}

impl LoginDraft {
    /// Header click: ascending → descending → newest first again.
    pub fn click(&mut self, c: Col) {
        self.sort = match self.sort {
            Some((s, Dir::Asc)) if s == c => Some((c, Dir::Desc)),
            Some((s, Dir::Desc)) if s == c => None,
            _ => Some((c, Dir::Asc)),
        };
        self.pager.reset();
    }

    pub fn filter(&self, c: Col) -> &str {
        &self.filters[c.index()]
    }

    pub fn filter_mut(&mut self, c: Col) -> &mut String {
        &mut self.filters[c.index()]
    }
}

/// Whose login a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// Someone else (another key or another address).
    Other,
    /// From this PC's address with a key Telepad logs in with (Telepad's
    /// refreshes, or the user's own `ssh` with that key).
    ThisPc,
    /// The login of the read that produced this table.
    ThisRead,
}

impl Who {
    pub fn text(self) -> &'static str {
        match self {
            Who::Other => "",
            Who::ThisPc => "this PC",
            Who::ThisRead => "this read",
        }
    }
}

/// One table row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub login: &'a Login,
    /// From the host's `authorized_keys` (`[group] comment`) or My keys.
    pub key_name: String,
    pub who: Who,
}

impl Row<'_> {
    pub fn cell(&self, c: Col) -> String {
        let l = self.login;
        match c {
            Col::Time => l.time.clone(),
            Col::User => l.user.clone(),
            Col::From => l.from.clone(),
            Col::Method => l.method.clone(),
            Col::KeyType => l.key_type.clone(),
            Col::Fingerprint => {
                if l.cert_id.is_empty() {
                    l.fingerprint.clone()
                } else {
                    format!("{} (cert {})", l.fingerprint, l.cert_id)
                }
            }
            Col::KeyName => self.key_name.clone(),
            Col::Who => self.who.text().to_string(),
        }
    }
}

/// Fingerprints Telepad may log in with on this host: the local candidates
/// and the key sshd accepted for this read.
pub fn own_keys(log: &LoginLog) -> Vec<String> {
    let mut v: Vec<String> = log
        .keys
        .as_ref()
        .map(|k| k.login.clone())
        .unwrap_or_default();
    for (fp, _) in &log.own {
        if !v.contains(fp) {
            v.push(fp.clone());
        }
    }
    if let Some(l) = log.this_session()
        && !l.fingerprint.is_empty()
        && !v.contains(&l.fingerprint)
    {
        v.push(l.fingerprint.clone());
    }
    v
}

/// Name of a key: the host's `authorized_keys` line (`[group] comment`),
/// else a key of this computer (`mine`: fingerprint, name).
pub fn key_name(log: &LoginLog, mine: &[(String, String)], fp: &str) -> String {
    if fp.is_empty() {
        return String::new();
    }
    if let Ok(k) = &log.keys
        && let Some(a) = k.keys.iter().find(|a| a.key.fingerprint() == fp)
    {
        let c = if a.key.comment.is_empty() {
            "(no comment)"
        } else {
            &a.key.comment
        };
        return match &a.group {
            Some(g) => format!("[{g}] {c}"),
            None => c.to_string(),
        };
    }
    log.own
        .iter()
        .chain(mine)
        .find(|(f, _)| f == fp)
        .map(|(_, n)| format!("{n} (this PC)"))
        .unwrap_or_default()
}

/// The rows to show: named, filtered (own logins hidden unless
/// `show_own`) and sorted; all pages.
pub fn rows<'a>(log: &'a LoginLog, mine: &[(String, String)], d: &LoginDraft) -> Vec<Row<'a>> {
    let own = own_keys(log);
    let ip = log.client_ip();
    let this = log.this_session().map(|l| l as *const Login);
    let mut v: Vec<Row> = log
        .logins
        .iter()
        .map(|l| {
            let who = if Some(l as *const Login) == this {
                Who::ThisRead
            } else if !ip.is_empty() && l.from == ip && own.contains(&l.fingerprint) {
                Who::ThisPc
            } else {
                Who::Other
            };
            Row {
                login: l,
                key_name: key_name(log, mine, &l.fingerprint),
                who,
            }
        })
        .filter(|r| d.show_own || r.who == Who::Other)
        .filter(|r| {
            Col::ALL.iter().all(|&c| {
                let f = d.filter(c).trim();
                f.is_empty() || filter_match(&r.cell(c), f)
            })
        })
        .collect();
    match d.sort {
        // journalctl prints oldest first; ties keep that order reversed.
        None => v.reverse(),
        Some((Col::Time, dir)) => {
            v.sort_by(|a, b| {
                let o = a.login.epoch.cmp(&b.login.epoch);
                if dir == Dir::Desc { o.reverse() } else { o }
            });
        }
        Some((c, dir)) => {
            let keys: Vec<String> = v.iter().map(|r| r.cell(c).to_lowercase()).collect();
            let mut idx: Vec<usize> = (0..v.len()).collect();
            idx.sort_by(|&a, &b| {
                let (x, y) = (&keys[a], &keys[b]);
                // Empty cells last in both directions.
                match (x.is_empty(), y.is_empty()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    _ => {
                        let o = natural(x, y);
                        if dir == Dir::Desc { o.reverse() } else { o }
                    }
                }
            });
            let mut slots: Vec<Option<Row>> = v.into_iter().map(Some).collect();
            v = idx.into_iter().filter_map(|i| slots[i].take()).collect();
        }
    }
    v
}

/// The page shown: (first index, end index, page count), with `d.page`
/// clamped to the last page.
pub fn page_range(total: usize, d: &LoginDraft) -> (usize, usize, usize) {
    d.pager.range(total)
}

pub use crate::grid::filter_match;
use crate::grid::natural;

/// Messages of the Log info pages.
#[derive(Debug, Clone)]
pub enum LgMsg {
    /// Host detail's **Log info**: open (or show) this host's page.
    Open(String),
    Refresh(LogId),
    Close(LogId),
    Loaded {
        id: LogId,
        result: Box<Result<LoginLog, String>>,
    },
    /// Open the terminal that adds (`true`) / removes the login user to /
    /// from `systemd-journal`.
    Access {
        id: LogId,
        register: bool,
    },
}

/// Work for the runtime.
#[derive(Debug, Clone)]
pub enum LgEffect {
    /// Read the host's logins (background) → [`LgMsg::Loaded`].
    Read { id: LogId, host: HostEntry },
    /// Terminal with the exact `sudo` command, run after Enter → a notice.
    Terminal { host: HostEntry, register: bool },
}
