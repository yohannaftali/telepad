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
    },
    /// The database could not be opened; nothing else works.
    Fatal(String),
    /// Once per frame.
    Tick {
        busy: usize,
    },

    // ---- hosts (views)
    RefreshAll,
    Refresh(usize),
    /// SSH terminal, or Remote Desktop for RDP-only hosts.
    Ssh(usize),
    CopySsh(usize),
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
    /// Add / revoke `key` in `~/.ssh/authorized_keys` on each host, in the
    /// background → one [`Msg::KeyResult`] per host.
    ApplyKey {
        action: KeyAction,
        key: PublicKey,
        hosts: Vec<HostEntry>,
    },
    /// Database windows: connect, load, run, save (worker per window).
    Db(crate::db::DbEffect),
    /// Host Management: read / pick / write the config, apply Telepad changes.
    HostSync(crate::hostsync::HsEffect),
    /// File windows: connect, list, copy (worker threads per window).
    Files(crate::files::FmEffect),
}
