//! Everything that can happen ([`Msg`]) and everything the app asks the
//! outside world to do ([`Effect`]).

use std::path::PathBuf;

use telepad_core::authorized_keys::{KeyAction, KeyOutcome, PublicKey};
use telepad_core::collector;
use telepad_core::config::{HostEntry, Inventory};
use telepad_core::docker_actions::{
    ContainerAction, NodeAction, Plan, RunInfo, ServiceAction, StackAction, WorkloadAction,
};
use telepad_core::model::Container;
use telepad_core::store::{ImportMode, ImportSummary, UiPrefs};
use telepad_core::update::{Check, UpdateConfig};

/// Input to [`crate::update`]: user intents from the views, and results the
/// runtime reports back after running an [`Effect`].
#[derive(Debug, Clone)]
pub enum Msg {
    // ---- lifecycle (runtime)
    /// The database is open.
    Started {
        db_path: PathBuf,
        ssh_config_path: PathBuf,
        /// Set when this run created the database from an OpenSSH config.
        imported_from: Option<PathBuf>,
        update_config: Result<UpdateConfig, String>,
        /// Remembered view settings (defaults if they could not be read).
        ui_prefs: UiPrefs,
        /// Workspaces from `preference.db` (#94); `Err`: it could not be
        /// opened, Telepad starts with `<own>` and saves nothing there.
        prefs: Result<telepad_core::prefs::WorkspacePrefs, String>,
        /// Update Strategy and last known facts from `update.db` (#122);
        /// `Err`: it could not be opened, defaults apply and nothing is saved.
        strategy: Result<telepad_core::strategy::Loaded, String>,
    },
    /// The database could not be opened; nothing else works.
    Fatal(String),
    /// Once per frame.
    Tick {
        busy: usize,
    },
    /// Pause / resume every automatic check (#122, Update Strategy page).
    SetPaused(bool),
    /// Pause / resume these hosts' automatic checks (by alias); their own
    /// Refresh still reads them.
    PauseHosts {
        aliases: Vec<String>,
        paused: bool,
    },
    /// Open the Update Strategy page with the saved values in its form.
    StrategyOpen,
    /// Check the form and save it.
    StrategySave,
    /// Put the defaults into the form (Save still needed).
    StrategyReset,
    /// Results of a ping round: (alias, ping / SSH port check, #125).
    Pinged(Vec<(String, collector::Reach)>),

    // ---- My Computer (#129)
    /// Read this computer again now.
    LocalRefresh,
    /// This computer was read (`elapsed`: how long it took).
    LocalCollected {
        result: Result<Box<telepad_core::model::HostReport>, String>,
        elapsed: std::time::Duration,
    },
    /// A shell of this computer in a new terminal window.
    LocalTerminal,

    // ---- hosts (views)
    RefreshAll,
    Refresh(usize),
    /// SSH terminal, or Remote Desktop for RDP-only hosts.
    Ssh(usize),
    CopySsh(usize),
    /// Remote Desktop with the host's saved settings (#102), also for an
    /// SSH host that has some.
    OpenRdp(usize),
    /// Saved Remote Desktop settings (with the hosts).
    RdpLoaded(Result<std::collections::BTreeMap<String, telepad_core::rdp::RdpSettings>, String>),
    /// Host editor: fill the Remote Desktop section from a `.rdp` file.
    EditorRdpImport,
    RdpPicked(Result<telepad_core::rdp::RdpSettings, String>),
    /// Host editor: save the section as a `.rdp` file.
    EditorRdpExport,
    RdpSaved(Result<(), String>),
    /// Windows Credential Manager (#108): check / save / forget the
    /// Remote Desktop password of the edited host.
    EditorRdpCredCheck,
    EditorRdpCredSave,
    EditorRdpCredForget,
    RdpCredState {
        address: String,
        saved: bool,
        result: Result<String, String>,
    },
    BmcSetup(usize),
    /// Container action in a terminal (host index, container id).
    DockerAction {
        host: usize,
        container: String,
        action: ContainerAction,
    },
    /// docker compose action in a terminal (host index, compose project).
    StackAction {
        host: usize,
        stack: String,
        action: StackAction,
    },
    /// "How it runs" window for a container (read-only fetch).
    ShowRunInfo {
        host: usize,
        container: String,
    },
    CloseRunInfo,
    /// Swarm / Kubernetes actions in a terminal (host index + names from its report).
    SwarmService {
        host: usize,
        service: String,
        action: ServiceAction,
    },
    SwarmStackRemove {
        host: usize,
        stack: String,
    },
    SwarmNode {
        host: usize,
        node_id: String,
        action: NodeAction,
    },
    KubeWorkload {
        host: usize,
        namespace: String,
        name: String,
        action: WorkloadAction,
    },
    Select(Option<usize>),
    /// Host dragged from index `from` to index `to` (in the list after removal).
    MoveHost {
        from: usize,
        to: usize,
    },
    /// Host list buttons: "Overview — all hosts" / "Overview — Docker".
    ShowOverview(crate::state::OverviewPage),
    SelectAlias(String),
    /// Workspace: open, close, move, minimize, maximize, tab inner windows (#77).
    Ws(crate::workspace::WsMsg),
    /// Named workspaces (#94).
    Wk(crate::workspace::WkMsg),
    PrefsSaved(Result<(), String>),
    /// Host list: compact (true) or detailed rows.
    SetCompactHostList(bool),
    /// Host list grouped by status.
    SetGroupByStatus(bool),
    /// Overview as cards (true) or table.
    SetOverviewCards(bool),
    SetTheme(telepad_core::store::ThemePref),
    /// Overview order chosen in the card view's "Sort by" picker.
    SetSort(crate::table::Sort),
    /// Overview header clicked.
    SortBy(crate::table::SortKey),
    AddHost,
    Edit(usize),
    Clone(usize),
    /// Ask for confirmation first.
    Delete(usize),
    ConfirmDelete,
    CancelDelete,
    EditorSave,
    EditorDelete,
    EditorClose,
    EditorBrowseKey,
    Import,
    /// Import the user's own `~/.ssh/config` without a file dialog.
    ImportSshConfig,
    /// "Import from pasted text…" window.
    PasteImport,
    PasteFromClipboard,
    /// Read the pasted text and go to the import preview.
    PasteApply,
    PasteCancel,
    ImportMode(ImportMode),
    ImportApply,
    ImportCancel,
    Export,
    /// "Public keys…" window (add / revoke one key on chosen hosts).
    KeysOpen,
    KeysClose,
    /// Header checkbox: select (true) or clear every listed SSH host.
    KeysSelectAll(bool),
    /// Host id.
    KeysToggle(i64),
    /// Add / Revoke clicked: ask for the typed confirmation.
    KeysAsk(KeyAction),
    KeysConfirmCancel,
    /// Confirmation typed: run it.
    KeysRun,

    // ---- self-update (views)
    UpdateWindow(bool),
    UpdateCheck,
    UpdateInstall,
    UpdateLater,
    UpdateSaveSettings,
    Restart,
    OpenUrl(String),

    // ---- results (runtime)
    Collected(collector::Update),
    HostsLoaded {
        hosts: Result<Vec<HostEntry>, String>,
        /// Writing the generated ssh config failed.
        ssh_config_error: Option<String>,
    },
    /// Ok((id, alias))
    HostSaved(Result<(i64, String), String>),
    HostDeleted {
        id: i64,
        alias: String,
        result: Result<(), String>,
    },
    /// Err((path, error))
    ImportOpened(Result<(PathBuf, Inventory), (PathBuf, String)>),
    ImportPreviewed(Result<ImportSummary, String>),
    Imported(Result<ImportSummary, String>),
    /// Ok((host count, path))
    Exported(Result<(usize, PathBuf), String>),
    KeyPicked(PathBuf),
    /// Private keys found in ~/.ssh for the editor's key list.
    KeysFound(Vec<String>),
    UpdateChecked(Result<Check, String>),
    /// Ok(installed version)
    UpdateInstalled(Result<String, String>),
    UpdateSettingsSaved(Result<UpdateConfig, String>),
    UiPrefsSaved(Result<(), String>),
    OrderSaved(Result<(), String>),
    RunInfoLoaded(RunInfo),
    ClipboardRead(Result<String, String>),
    /// Authorized keys page (#78): read `authorized_keys` again on these hosts
    /// (`None` = every SSH host).
    AkLoad(Option<Vec<String>>),
    /// One host's `authorized_keys` was read.
    AkListed {
        alias: String,
        result: Result<telepad_core::authorized_keys::HostKeys, String>,
    },
    /// Switch the page's view.
    AkTab(crate::state::KeysTab),
    /// My keys tab (#101).
    MyKeys(crate::mykeys::MkMsg),
    /// Shortcut from a list: put `line` in the key box, choose `aliases`, go to
    /// Add / revoke and ask for the confirmation word (when the key is valid).
    AkPrepare {
        action: KeyAction,
        line: String,
        aliases: Vec<String>,
    },
    /// One host of a key run finished.
    KeyResult {
        alias: String,
        outcome: KeyOutcome,
    },
    /// Text for the status bar (e.g. from opening a terminal).
    Notice(String),
    /// Database windows (#52–#54).
    Db(crate::db::DbMsg),
    /// Host Management window (#58).
    HostSync(crate::hostsync::HsMsg),
    /// File windows (#62).
    Files(crate::files::FmMsg),
    /// Log info pages (#126).
    Logins(crate::logins::LgMsg),
    /// Groups / clusters (#144, #145).
    Sets(crate::sets::StMsg),
    /// Portainers (#171).
    Portainer(crate::portainer::PtMsg),
    /// Table menu exports (#153).
    Table(crate::grid::TableMsg),
}

/// Work for the runtime. Effects are the only way to reach the database, ssh,
/// the network, files, the clipboard or the OS.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Read hosts, regenerate the ssh config → [`Msg::HostsLoaded`].
    LoadHosts,
    /// Queue collection (background) → [`Msg::Collected`]. Each job says
    /// which tiers to read (see [`collector::Scope`]).
    Collect(Vec<collector::Job>),
    /// → [`Msg::HostSaved`]
    SaveHost(HostEntry),
    /// Save / forget a host's Remote Desktop settings → [`Msg::RdpSaved`].
    SaveRdp {
        alias: String,
        settings: Option<telepad_core::rdp::RdpSettings>,
    },
    /// Open a `.rdp` file → [`Msg::RdpPicked`].
    PickRdpFile,
    /// Windows Credential Manager (#108) → [`Msg::RdpCredState`].
    RdpCredCheck {
        address: String,
    },
    RdpCredSave {
        address: String,
        user: String,
        password: telepad_core::db::Secret,
    },
    RdpCredForget {
        address: String,
    },
    /// Save a `.rdp` file (UTF-16 as mstsc writes it).
    ExportRdp {
        alias: String,
        text: String,
    },
    /// → [`Msg::HostDeleted`]
    DeleteHost {
        id: i64,
        alias: String,
    },
    /// File dialog + parse → [`Msg::ImportOpened`] (nothing if cancelled).
    PickImportFile,
    /// Read `~/.ssh/config` → [`Msg::ImportOpened`].
    ReadUserSshConfig,
    /// → [`Msg::ImportPreviewed`]
    PreviewImport {
        inventory: Inventory,
        mode: ImportMode,
    },
    /// → [`Msg::Imported`]
    ApplyImport {
        inventory: Inventory,
        mode: ImportMode,
    },
    /// File dialog + write → [`Msg::Exported`] (nothing if cancelled).
    PickExportFile,
    /// File dialog → [`Msg::KeyPicked`] (nothing if cancelled). Starts in the
    /// folder of the given IdentityFile, else `~/.ssh`.
    PickKeyFile(String),
    /// List private keys in `~/.ssh` → [`Msg::KeysFound`].
    FindKeys,
    /// Text on the clipboard → [`Msg::ClipboardRead`].
    ReadClipboard,
    /// The following four report with [`Msg::Notice`].
    OpenSsh(HostEntry),
    OpenRdp(HostEntry),
    OpenBmcSetup(HostEntry),
    CopySshCommand(HostEntry),
    /// Background → [`Msg::UpdateChecked`]
    CheckUpdate(UpdateConfig),
    /// Background → [`Msg::UpdateInstalled`]
    InstallUpdate(Check),
    /// → [`Msg::UpdateSettingsSaved`]
    SaveUpdateConfig(UpdateConfig),
    /// → [`Msg::UiPrefsSaved`]
    SaveUiPrefs(UiPrefs),
    /// Write the workspaces to `preference.db` → [`Msg::PrefsSaved`].
    SavePrefs(telepad_core::prefs::WorkspacePrefs),
    /// Write the Update Strategy to `update.db` (#122); errors come back as
    /// [`Msg::Notice`].
    SaveStrategy(telepad_core::strategy::Strategy),
    /// Pause / resume one host in `update.db`.
    SaveHostPaused {
        alias: String,
        paused: bool,
    },
    /// Keep a host's freshly read static / slow facts in `update.db`.
    SaveFacts(telepad_core::strategy::Facts),
    /// Forget saved facts of hosts not in this list (deleted, renamed).
    PruneFacts(Vec<String>),
    /// Ping these hosts (background, no SSH) → [`Msg::Pinged`].
    Ping(Vec<PingJob>),
    /// Host ids in display order; saved in one transaction, ssh config
    /// regenerated → [`Msg::OrderSaved`].
    SaveOrder(Vec<i64>),
    /// Errors come back as [`Msg::Notice`].
    /// Terminal on the host that shows the command and runs it after Enter.
    OpenDockerAction(HostEntry, Plan),
    /// Read-only `docker inspect` + compose files → [`Msg::RunInfoLoaded`].
    FetchRunInfo(HostEntry, Box<Container>),
    OpenUrl(String),
    /// Start the new exe and close this window.
    Relaunch,
    /// Read `~/.ssh/authorized_keys` on each host (read-only), in the
    /// background → one [`Msg::AkListed`] per host.
    ListKeys(Vec<HostEntry>),
    /// Add / revoke `key` in `~/.ssh/authorized_keys` on each host, in the
    /// background → one [`Msg::KeyResult`] per host.
    ApplyKey {
        action: KeyAction,
        key: PublicKey,
        hosts: Vec<HostEntry>,
        /// Add into this group of `authorized_keys` (#85).
        group: Option<String>,
    },
    /// Database windows: connect, load, run, save (worker per window).
    Db(crate::db::DbEffect),
    /// Host Management: read / pick / write the config, apply Telepad changes.
    HostSync(crate::hostsync::HsEffect),
    /// My keys tab (#101).
    MyKeys(crate::mykeys::MkEffect),
    /// File windows: connect, list, copy (worker threads per window).
    Files(crate::files::FmEffect),
    /// Log info pages (#126): read the journal, open the access terminal.
    Logins(crate::logins::LgEffect),
    /// Change groups / clusters in `telepad.db` → [`crate::sets::StMsg::Saved`].
    Sets(crate::sets::StEffect),
    /// Portainers (#171): store changes, tokens, reads through a tunnel.
    Portainer(crate::portainer::PtEffect),
    /// Read this computer (#129, background) → [`Msg::LocalCollected`].
    CollectLocal,
    /// Errors come back as [`Msg::Notice`].
    OpenLocalTerminal,
    /// Save dialog, then write the table on a thread (#153) →
    /// [`crate::grid::TableMsg::Exported`] (nothing if cancelled).
    ExportTable {
        data: crate::grid::TableData,
        format: crate::grid::Format,
    },
    /// Put text on the clipboard, then show `notice` in the status bar.
    CopyText {
        text: String,
        notice: String,
    },
}

/// One host of a ping round (#122). `port`: also try the SSH port when ping
/// gets no reply (#125; only hosts that were down, to keep sshd logs quiet).
#[derive(Debug, Clone, PartialEq)]
pub struct PingJob {
    pub host: HostEntry,
    pub port: bool,
}
