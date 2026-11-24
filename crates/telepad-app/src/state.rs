//! Application state: [`Model`] (changed only by [`crate::update`]) and
//! [`Drafts`] (what views may edit directly: form text, filter, toggles).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use std::collections::BTreeSet;

use telepad_core::authorized_keys::{KeyAction, KeyOutcome};
use telepad_core::collector::Scope;
use telepad_core::config::{Access, HostEntry, Inventory};
use telepad_core::model::HostReport;
use telepad_core::store::{self, ImportMode, ImportSummary, ThemePref, UiPrefs};
use telepad_core::strategy::Strategy;
use telepad_core::update::{Check, UpdateConfig};

/// How long a status-bar message stays visible.
pub const MESSAGE_TTL: Duration = Duration::from_secs(6);

#[derive(Debug, Default)]
pub struct State {
    pub model: Model,
    pub drafts: Drafts,
}

#[derive(Debug, Default)]
pub struct Model {
    pub db_path: Option<PathBuf>,
    /// The generated file ssh reads.
    pub ssh_config_path: Option<PathBuf>,
    pub hosts: Vec<HostState>,
    pub selected: Option<usize>,
    /// Which overview shows while no host is selected.
    pub overview: OverviewPage,
    pub last_refresh: Option<Instant>,
    /// Hosts queued or being collected right now (from the runtime).
    pub busy: usize,
    pub message: Option<(String, Instant)>,
    pub fatal: Option<String>,
    /// Host id + alias waiting for delete confirmation.
    pub pending_delete: Option<(i64, String)>,
    /// Select this host id once the host list is reloaded (after a save).
    pub select_after_load: Option<i64>,
    pub import: Option<ImportDialog>,
    pub update: UpdateState,
    /// Last "Public keys…" run: one result per host as they come in.
    pub key_run: Option<KeyRun>,
    /// Open database windows.
    pub db: Vec<crate::db::DbSession>,
    /// Last database window id handed out.
    pub db_next: crate::db::DbId,
    /// Host Management window.
    pub host_sync: Option<crate::hostsync::HostSync>,
    /// File windows (#62), one per host.
    pub files: Vec<crate::files::FileSession>,
    pub files_next: crate::files::FmId,
    /// Inner windows of the main window (#77).
    pub ws: crate::workspace::Workspace,
    /// Authorized keys page (#78): what each host's `authorized_keys` holds,
    /// by alias.
    pub auth_keys: std::collections::BTreeMap<String, AkHost>,
    /// The live layout and sidebar state last seen: a change goes to the
    /// current workspace (layout) or `telepad.db` (sidebar).
    pub ws_saved: (String, bool),
    /// Named workspaces (#94), as in `preference.db`.
    pub wss: telepad_core::prefs::WorkspacePrefs,
    /// `preference.db` is open; when not, nothing is saved there.
    pub prefs_ok: bool,
    /// The workspaces changed since they were last written.
    pub prefs_dirty: bool,
    /// When they were last written (autosave at most once a second).
    pub prefs_written: Option<Instant>,
    /// `ui_workspace` from `telepad.db` as loaded; kept as it is (the
    /// layouts live in `preference.db` now).
    pub ws_legacy: String,
    /// Telepad may close now (#99: nothing to lose, or the user said Yes).
    pub quit: bool,
    /// Key pairs on this computer (#101, My keys tab).
    pub my_keys: crate::mykeys::MyKeys,
    /// Saved Remote Desktop settings by alias (#102).
    pub rdp: std::collections::BTreeMap<String, telepad_core::rdp::RdpSettings>,
    /// What Telepad checks by itself and how often (#122, `update.db`).
    pub strategy: Strategy,
    /// `update.db` is open; when not, nothing is saved there.
    pub strategy_ok: bool,
    /// Since when automatic checks are paused (this session).
    pub paused_at: Option<Instant>,
    /// Last ping round, and whether one is running.
    pub last_ping: Option<Instant>,
    pub pinging: bool,
    /// Hosts paused one by one (#122), by alias, as in `update.db`.
    pub paused_hosts: BTreeSet<String>,
    /// Facts from `update.db` waiting for their host to be loaded, by alias.
    pub restored: std::collections::BTreeMap<String, telepad_core::strategy::SavedFacts>,
    /// Log info pages (#126), one per host.
    pub logins: Vec<crate::logins::LoginSession>,
    pub logins_next: crate::logins::LogId,
    /// Key sshd accepted for Telepad's login, by alias (from the last Log
    /// info read): shown on the Authorized keys page.
    pub telepad_key: std::collections::BTreeMap<String, String>,
    /// My Computer (#129): this computer's report, read without SSH while
    /// its page is open. `entry` is [`HostEntry::local`].
    pub local: HostState,
}

/// A key added to / revoked from several hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRun {
    pub action: KeyAction,
    pub fingerprint: String,
    pub comment: String,
    /// Host alias and its result (None while it runs).
    pub results: Vec<(String, Option<KeyOutcome>)>,
}

impl KeyRun {
    pub fn finished(&self) -> usize {
        self.results.iter().filter(|(_, r)| r.is_some()).count()
    }

    pub fn running(&self) -> bool {
        self.finished() < self.results.len()
    }
}

/// Values views bind to widgets directly. Changing them never does I/O.
#[derive(Debug)]
pub struct Drafts {
    pub filter: String,
    pub ports_nginx_only: bool,
    pub editor: Option<Editor>,
    pub update_form: UpdateConfig,
    pub update_form_error: Option<String>,
    /// Overview order (header clicks); column widths are kept by the table itself.
    pub sort: crate::table::Sort,
    /// Host list: one line per host (default) or detailed rows. Saved in the database.
    pub compact_host_list: bool,
    /// Host list grouped into Down / Up / Not checked yet. Saved in the database.
    pub group_by_status: bool,
    /// Overview as cards instead of the table. Saved in the database.
    pub overview_cards: bool,
    /// System / Light / Dark. Saved in the database.
    pub theme: ThemePref,
    /// "How it runs" window.
    pub run_info: Option<RunInfoView>,
    /// "Import from pasted text" window.
    pub paste: Option<PasteImport>,
    /// "Public keys…" window.
    pub keys: Option<KeysDraft>,
    /// Form fields of each database window.
    pub db: std::collections::BTreeMap<crate::db::DbId, crate::db::DbDraft>,
    /// Host Management window: filter, selection, edit form.
    pub host_sync: crate::hostsync::HostSyncDraft,
    /// File windows: path bars, filters, selection.
    pub files: std::collections::BTreeMap<crate::files::FmId, crate::files::FileDraft>,
    /// Open-windows picker in the top bar (#92).
    pub win_picker: PickerDraft,
    /// Workspace dropdown in the top bar (#94).
    pub ws_picker: PickerDraft,
    /// "Add workspace" dialog (#94).
    pub ws_add: Option<WsAddDraft>,
    /// Custom workspace waiting for the delete confirmation (#94).
    pub ws_delete: Option<String>,
    /// Size of the workspace area as last drawn (for Tile / Stack).
    pub ws_area: [f32; 2],
    /// The leave question (#99): what would be lost by leaving.
    pub ws_leave: Option<LeaveAsk>,
    /// Update Strategy page (#122): the form and why Save refused it.
    pub strategy: StrategyDraft,
    /// Log info pages (#126): sort, filters, page.
    pub logins: std::collections::BTreeMap<crate::logins::LogId, crate::logins::LoginDraft>,
}

/// The Update Strategy page's form (#122). `paused` is not part of it (the
/// top bar's Pause button changes it at once).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrategyDraft {
    pub form: Strategy,
    pub error: Option<String>,
    /// Narrows the page's host list.
    pub host_filter: String,
}

impl StrategyDraft {
    /// The form differs from what is saved.
    pub fn changed(&self, saved: &Strategy) -> bool {
        Strategy {
            paused: saved.paused,
            ..self.form
        } != *saved
    }
}

/// Leaving the workspace would throw work away (#99): the list shown before
/// Yes / Back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveAsk {
    /// Workspace to go to; `None`: Telepad closes.
    pub to: Option<telepad_core::prefs::Current>,
    /// Custom workspace deleted after leaving it.
    pub delete: Option<String>,
    /// (window, what is lost).
    pub items: Vec<(String, String)>,
    /// File transfers that keep running in the background.
    pub background: usize,
}

/// "Add workspace" dialog: the label being typed and why it was refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WsAddDraft {
    pub label: String,
    pub error: Option<String>,
}

/// A searchable dropdown (select2 style): the search text and the
/// highlighted row of the filtered list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerDraft {
    pub query: String,
    pub highlight: usize,
}

/// `text` matches `query` when it contains every word of it, ignoring case.
pub fn matches_query(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .split_whitespace()
        .all(|w| text.contains(&w.to_lowercase()))
}

/// "Public keys…" window: the pasted key, the chosen hosts and the
/// confirmation being typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeysDraft {
    /// Which view of the Authorized keys page is shown (#78).
    pub tab: KeysTab,
    /// By server: the host whose keys are shown.
    pub server: Option<String>,
    /// By public key: the key (fingerprint) whose servers are shown.
    pub open_key: Option<String>,
    /// Narrows the server and key lists (alias, comment, fingerprint, group).
    pub list_filter: String,
    /// Add into this group (#85): the `#` line that names it; empty = no group.
    pub group: String,
    pub text: String,
    /// Narrows the host list (alias, address, description, OS).
    pub filter: String,
    /// Host ids.
    pub selected: BTreeSet<i64>,
    /// Waiting for the user to type the action word.
    pub confirm: Option<KeyAction>,
    pub typed: String,
    /// My keys tab (#101): selection and its dialogs.
    pub mine: crate::mykeys::MyKeysDraft,
}

impl KeysDraft {
    /// Hosts the window lists (matching the filter), as indices into `m.hosts`.
    pub fn listed(&self, m: &Model) -> Vec<usize> {
        let f = self.filter.trim();
        (0..m.hosts.len())
            .filter(|&i| matches_filter(f, &m.hosts[i]))
            .collect()
    }

    /// Selected hosts that still exist and have SSH, in list order.
    pub fn chosen<'a>(&self, m: &'a Model) -> Vec<&'a HostEntry> {
        m.hosts
            .iter()
            .map(|h| &h.entry)
            .filter(|e| key_selectable(e) && e.id.is_some_and(|id| self.selected.contains(&id)))
            .collect()
    }
}

/// The views of the Authorized keys page (#78).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeysTab {
    /// One host's keys.
    #[default]
    ByServer,
    /// Every key and the hosts that have it.
    ByKey,
    /// Paste a key, pick hosts, confirm, see the results (#49).
    AddRevoke,
    /// The key pairs on this computer: generate, convert (#101).
    MyKeys,
}

/// One host's `authorized_keys` as last read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AkHost {
    Loading,
    Ready(telepad_core::authorized_keys::HostKeys),
    Failed(String),
}

/// One distinct key (by fingerprint) and the hosts whose `authorized_keys`
/// hold it, for the "By public key" view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHolders {
    pub fingerprint: String,
    /// The first line seen (type, data, comment) and whether Telepad can manage it.
    pub key: telepad_core::authorized_keys::PublicKey,
    pub supported: bool,
    /// Comments seen for it on any host.
    pub comments: Vec<String>,
    /// Groups it is in on any host (#85); empty = no group anywhere.
    pub groups: Vec<String>,
    /// Aliases, in host order; `true` = Telepad logs in to that host with it.
    pub hosts: Vec<(String, bool)>,
}

/// Every key found on the loaded hosts, grouped by fingerprint, in the order
/// first seen (hosts in inventory order).
pub fn key_holders(m: &Model) -> Vec<KeyHolders> {
    let mut out: Vec<KeyHolders> = Vec::new();
    for h in &m.hosts {
        let Some(AkHost::Ready(hk)) = m.auth_keys.get(&h.entry.alias) else {
            continue;
        };
        for ak in &hk.keys {
            let fp = ak.key.fingerprint();
            let login = hk.login.contains(&fp);
            let i = match out.iter().position(|k| k.fingerprint == fp) {
                Some(i) => i,
                None => {
                    out.push(KeyHolders {
                        fingerprint: fp.clone(),
                        key: ak.key.clone(),
                        supported: ak.supported,
                        comments: Vec::new(),
                        groups: Vec::new(),
                        hosts: Vec::new(),
                    });
                    out.len() - 1
                }
            };
            let k = &mut out[i];
            if !ak.key.comment.is_empty() && !k.comments.contains(&ak.key.comment) {
                k.comments.push(ak.key.comment.clone());
            }
            if let Some(g) = &ak.group
                && !k.groups.contains(g)
            {
                k.groups.push(g.clone());
            }
            if !k.hosts.iter().any(|(a, _)| *a == h.entry.alias) {
                k.hosts.push((h.entry.alias.clone(), login));
            }
        }
    }
    out
}

/// Every group name found on the loaded hosts, A–Z (#85).
pub fn key_groups(m: &Model) -> Vec<String> {
    let mut g: Vec<String> = m
        .auth_keys
        .values()
        .filter_map(|v| match v {
            AkHost::Ready(h) => Some(h.keys.iter().filter_map(|k| k.group.clone())),
            _ => None,
        })
        .flatten()
        .collect();
    g.sort_by_key(|s| s.to_lowercase());
    g.dedup();
    g
}

/// A host the key window can change: saved, with SSH (not RDP-only).
pub fn key_selectable(e: &HostEntry) -> bool {
    e.id.is_some() && e.access != Access::Rdp
}

/// Text the user pastes, and why parts of it could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PasteImport {
    pub text: String,
    pub problems: Vec<String>,
}

/// The "How it runs" window: waiting for the host, or the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunInfoView {
    Loading { host: String, container: String },
    Ready(telepad_core::docker_actions::RunInfo),
}

impl Default for Drafts {
    fn default() -> Self {
        Self {
            filter: String::new(),
            ports_nginx_only: false,
            editor: None,
            update_form: UpdateConfig::default(),
            update_form_error: None,
            sort: crate::table::Sort::default(),
            compact_host_list: true,
            group_by_status: false,
            overview_cards: false,
            theme: ThemePref::System,
            run_info: None,
            paste: None,
            keys: None,
            db: Default::default(),
            host_sync: Default::default(),
            files: Default::default(),
            win_picker: PickerDraft::default(),
            ws_picker: PickerDraft::default(),
            ws_add: None,
            ws_delete: None,
            ws_area: [1200.0, 700.0],
            ws_leave: None,
            strategy: StrategyDraft::default(),
            logins: Default::default(),
        }
    }
}

/// The two overview pages under the host list's buttons.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum OverviewPage {
    /// Every host as a table or cards.
    #[default]
    Hosts,
    /// Every container of every host, grouped by stack.
    Docker,
}

#[derive(Debug, Default, Clone)]
pub struct HostState {
    pub entry: HostEntry,
    pub report: Option<HostReport>,
    pub error: Option<String>,
    pub loading: bool,
    pub updated: Option<Instant>,
    pub elapsed: Option<Duration>,
    /// Last successful read of hostname / OS / kernel / CPU model (daily).
    pub static_at: Option<Seen>,
    /// Last successful read of addresses, ports and updates (every 6 hours).
    pub slow_at: Option<Seen>,
    /// Facts from `update.db` (#122) while there is no report of this
    /// session yet: shown as last known, and kept by the first collection.
    pub known: Option<HostReport>,
    /// Last ping check (#122): `Some(None)` = no reply.
    pub ping: Option<Option<f32>>,
    pub ping_at: Option<Instant>,
    /// The host answered a ping check once (hosts that drop ICMP never are
    /// marked down by ping), and how many checks in a row it missed since.
    pub ping_replied: bool,
    pub ping_misses: u8,
    /// The last check got no ping reply but the SSH port was open (#125).
    pub port_open: bool,
    /// Last attempt found the host down (no ping reply and SSH port closed;
    /// behind a proxy: two SSH failures in a row): check it before SSH.
    pub no_ping: bool,
    /// SSH attempts in a row that failed (#125).
    pub ssh_fails: u8,
    /// Last time ssh was actually tried (not skipped by a failed ping).
    pub ssh_at: Option<Instant>,
}

impl HostState {
    pub fn new(entry: HostEntry) -> Self {
        Self {
            entry,
            ..Default::default()
        }
    }

    /// What the next automatic collection needs from this host: the tiers
    /// that are on and due.
    pub fn scope_at(&self, now: Instant, st: &Strategy) -> Scope {
        if self.entry.access == Access::Rdp {
            return Scope::FULL;
        }
        let due = |t: Option<Seen>, every: telepad_core::strategy::Every| {
            every.on && t.is_none_or(|t| t.age(now) >= every.every())
        };
        Scope {
            static_info: due(self.static_at, st.static_info),
            slow: due(self.slow_at, st.slow),
            ping_first: self.no_ping
                && self
                    .ssh_at
                    .is_some_and(|t| now.saturating_duration_since(t) < st.ssh_retry_every()),
        }
    }

    /// The report to show: this session's, else the last known facts.
    pub fn shown(&self) -> Option<&HostReport> {
        self.report.as_ref().or(self.known.as_ref())
    }

    pub fn online(&self) -> Option<bool> {
        match (&self.error, &self.report) {
            (Some(_), _) => Some(false),
            (None, Some(_)) => Some(true),
            _ => None,
        }
    }
}

/// A moment in the past: `at` minus `before`. Saved times can lie before
/// this computer's monotonic clock started (e.g. facts from yesterday after a
/// reboot), which an [`Instant`] cannot hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    pub at: Instant,
    pub before: Duration,
}

impl Seen {
    pub fn at(t: Instant) -> Self {
        Self {
            at: t,
            before: Duration::ZERO,
        }
    }

    /// `age` before `now`.
    pub fn ago(age: Duration, now: Instant) -> Self {
        Self {
            at: now,
            before: age,
        }
    }

    pub fn age(self, now: Instant) -> Duration {
        now.saturating_duration_since(self.at) + self.before
    }
}

/// Add/edit form. Text fields are kept as typed and validated on save.
#[derive(Debug, Clone)]
pub struct Editor {
    pub entry: HostEntry,
    pub port: String,
    pub identity: String,
    pub options: String,
    pub monitored: bool,
    pub access: Access,
    pub error: Option<String>,
    pub confirm_delete: bool,
    pub cloned_from: Option<String>,
    /// Private keys found in ~/.ssh, offered next to the IdentityFile field.
    pub key_choices: Vec<String>,
    /// Remote Desktop section (#102): on = settings are saved for the host.
    pub rdp_on: bool,
    pub rdp: telepad_core::rdp::RdpSettings,
    /// Text fields bound to numbers (port, window width / height).
    pub rdp_port: String,
    pub rdp_width: String,
    pub rdp_height: String,
    /// What is saved now (to know whether Save changes it).
    pub rdp_saved: Option<telepad_core::rdp::RdpSettings>,
    /// Settings to write once the host itself is saved.
    pub rdp_pending: Option<Option<telepad_core::rdp::RdpSettings>>,
    /// Windows Credential Manager (#108): a password is saved for the
    /// address (`None` while unknown), the two password fields, the result.
    pub rdp_cred_saved: Option<bool>,
    pub rdp_cred_pass: telepad_core::db::Secret,
    pub rdp_cred_pass2: telepad_core::db::Secret,
    pub rdp_cred_note: Option<Result<String, String>>,
}

impl Editor {
    pub fn new(entry: HostEntry) -> Self {
        Self {
            cloned_from: None,
            port: entry.port.to_string(),
            identity: entry.identity_file.clone().unwrap_or_default(),
            options: store::options_text(&entry.options),
            monitored: !entry.unmonitored,
            access: entry.access,
            entry,
            error: None,
            confirm_delete: false,
            key_choices: Vec::new(),
            rdp_on: false,
            rdp: Default::default(),
            rdp_port: String::new(),
            rdp_width: "1600".into(),
            rdp_height: "900".into(),
            rdp_saved: None,
            rdp_pending: None,
            rdp_cred_saved: None,
            rdp_cred_pass: Default::default(),
            rdp_cred_pass2: Default::default(),
            rdp_cred_note: None,
        }
    }

    /// The address Remote Desktop connects to (its credential's target).
    pub fn rdp_address(&self) -> String {
        let a = self.rdp.address.trim();
        if !a.is_empty() {
            return a.to_string();
        }
        let h = self.entry.hostname.trim();
        if h.is_empty() {
            self.entry.alias.trim().to_string()
        } else {
            h.to_string()
        }
    }

    /// `DOMAIN\user` (or `user`) for the credential.
    pub fn rdp_login(&self) -> String {
        let (d, u) = (self.rdp.domain.trim(), self.rdp.user.trim());
        let u = if u.is_empty() {
            self.entry.user.trim()
        } else {
            u
        };
        if d.is_empty() || u.is_empty() {
            u.to_string()
        } else {
            format!("{d}\\{u}")
        }
    }

    /// Fill the Remote Desktop section from saved settings (#102).
    pub fn set_rdp(&mut self, saved: Option<&telepad_core::rdp::RdpSettings>) {
        self.rdp_saved = saved.cloned();
        self.rdp_on = saved.is_some();
        let r = saved.cloned().unwrap_or_default();
        self.rdp_port = if r.port == 0 {
            String::new()
        } else {
            r.port.to_string()
        };
        if let telepad_core::rdp::Screen::Window { width, height } = r.screen {
            self.rdp_width = width.to_string();
            self.rdp_height = height.to_string();
        }
        self.rdp = r;
    }

    /// The Remote Desktop settings the form says (`None` = none saved).
    pub fn build_rdp(&self) -> Result<Option<telepad_core::rdp::RdpSettings>, String> {
        if !self.rdp_on {
            return Ok(None);
        }
        let mut r = self.rdp.clone();
        r.address = r.address.trim().to_string();
        r.user = r.user.trim().to_string();
        r.domain = r.domain.trim().to_string();
        r.gateway = r.gateway.trim().to_string();
        r.port = match self.rdp_port.trim() {
            "" => 0,
            p => p
                .parse()
                .map_err(|_| "Remote Desktop port must be 1-65535 (or empty)".to_string())?,
        };
        if let telepad_core::rdp::Screen::Window { .. } = r.screen {
            let n = |s: &str| {
                s.trim()
                    .parse::<u16>()
                    .map_err(|_| "Window size must be numbers".to_string())
            };
            r.screen = telepad_core::rdp::Screen::Window {
                width: n(&self.rdp_width)?,
                height: n(&self.rdp_height)?,
            };
        }
        r.validate()?;
        Ok(Some(r))
    }

    /// New host with the connection settings (User, Port, IdentityFile) of `src`.
    pub fn clone_of(src: &HostEntry) -> Self {
        let entry = HostEntry {
            user: src.user.clone(),
            port: src.port,
            identity_file: src.identity_file.clone(),
            access: src.access,
            ..HostEntry::new("")
        };
        Self {
            cloned_from: Some(src.alias.clone()),
            ..Self::new(entry)
        }
    }

    pub fn title(&self) -> String {
        match (&self.entry.id, &self.cloned_from) {
            (Some(_), _) => format!("Edit host — {}", self.entry.alias),
            (None, Some(src)) => format!("Add host — cloned from {src}"),
            (None, None) => "Add host".to_string(),
        }
    }

    /// Switch Access; a port still at the old default follows to the new one.
    pub fn set_access(&mut self, access: Access) {
        if self.port.trim() == self.access.default_port().to_string() {
            self.port = access.default_port().to_string();
        }
        self.access = access;
    }

    pub fn build(&self) -> Result<HostEntry, String> {
        let mut e = self.entry.clone();
        e.alias = e.alias.trim().to_string();
        e.hostname = e.hostname.trim().to_string();
        e.user = e.user.trim().to_string();
        e.note = e.note.trim().to_string();
        e.port = self
            .port
            .trim()
            .parse()
            .map_err(|_| "Port must be a number 1-65535".to_string())?;
        let id = self.identity.trim();
        e.identity_file = (!id.is_empty()).then(|| id.to_string());
        e.options = store::decode_options(&self.options);
        e.unmonitored = !self.monitored;
        e.access = self.access;
        e.validate()?;
        Ok(e)
    }
}

#[derive(Debug, Clone)]
pub struct ImportDialog {
    pub path: PathBuf,
    pub inventory: Inventory,
    pub mode: ImportMode,
    pub preview: Result<ImportSummary, String>,
    /// Pasted lines or hosts left out (shown in the preview).
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    Installing,
    /// New exes are in place; a restart starts them.
    Installed(String),
}

#[derive(Debug, Clone, Default)]
pub struct UpdateState {
    /// None until the settings were read from the database.
    pub config: Option<UpdateConfig>,
    pub phase: UpdatePhase,
    pub last_check: Option<Instant>,
    /// Result of the last successful check.
    pub check: Option<Check>,
    pub error: Option<String>,
    /// "Later" hides the banner until the next start.
    pub later: bool,
    pub window: bool,
    /// The last check was started by the user: report "up to date" too.
    pub manual: bool,
}

impl UpdateState {
    pub fn enabled(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c.enabled)
    }

    /// The release to advertise in the top bar, if any.
    pub fn banner(&self) -> Option<&Check> {
        self.check
            .as_ref()
            .filter(|c| c.newer && !self.later && self.phase == UpdatePhase::Idle)
    }

    pub fn can_install(&self) -> bool {
        self.phase == UpdatePhase::Idle
            && self
                .check
                .as_ref()
                .is_some_and(|c| c.newer && c.asset.is_some())
    }
}

impl Drafts {
    /// View settings to save; the sidebar comes from the model, the old
    /// `ui_workspace` is written back unchanged.
    pub fn ui_prefs(&self, m: &Model) -> UiPrefs {
        let ws = &m.ws;
        UiPrefs {
            compact_host_list: self.compact_host_list,
            group_by_status: self.group_by_status,
            overview_cards: self.overview_cards,
            theme: self.theme,
            sidebar_collapsed: ws.sidebar_collapsed,
            workspace: m.ws_legacy.clone(),
        }
    }
}

impl Model {
    pub fn online_count(&self) -> usize {
        self.hosts
            .iter()
            .filter(|h| h.online() == Some(true))
            .count()
    }

    pub fn offline_count(&self) -> usize {
        self.hosts
            .iter()
            .filter(|h| h.online() == Some(false))
            .count()
    }

    /// This host's automatic checks are paused (one by one, #122).
    pub fn host_paused(&self, h: &HostState) -> bool {
        self.paused_hosts.contains(&h.entry.alias)
    }

    pub fn message_at(&self, now: Instant) -> Option<&str> {
        self.message
            .as_ref()
            .filter(|(_, t)| now.saturating_duration_since(*t) < MESSAGE_TTL)
            .map(|(m, _)| m.as_str())
    }
}

/// Search box match: alias, address, description, OS, reported hostname or a container name.
pub fn matches_filter(filter: &str, h: &HostState) -> bool {
    if filter.is_empty() {
        return true;
    }
    let f = filter.to_lowercase();
    let r = h.report.as_ref();
    [
        h.entry.alias.as_str(),
        h.entry.hostname.as_str(),
        h.entry.note.as_str(),
        r.map_or("", |r| r.os_pretty.as_str()),
        r.map_or("", |r| r.hostname.as_str()),
    ]
    .iter()
    .any(|s| s.to_lowercase().contains(&f))
        || r.is_some_and(|r| {
            r.docker
                .containers
                .iter()
                .any(|c| c.name.to_lowercase().contains(&f))
        })
}
