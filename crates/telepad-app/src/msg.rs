//! Everything that can happen ([`Msg`]) and everything the app asks the
//! outside world to do ([`Effect`]).

use std::path::PathBuf;

use telepad_core::collector;
use telepad_core::config::{HostEntry, Inventory};
use telepad_core::store::{ImportMode, ImportSummary};
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
    Select(Option<usize>),
    SelectAlias(String),
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
    ImportMode(ImportMode),
    ImportApply,
    ImportCancel,
    Export,

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
    UpdateChecked(Result<Check, String>),
    /// Ok(installed version)
    UpdateInstalled(Result<String, String>),
    UpdateSettingsSaved(Result<UpdateConfig, String>),
    /// Text for the status bar (e.g. from opening a terminal).
    Notice(String),
}

/// Work for the runtime. Effects are the only way to reach the database, ssh,
/// the network, files, the clipboard or the OS.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Read hosts, regenerate the ssh config → [`Msg::HostsLoaded`].
    LoadHosts,
    /// Queue collection (background) → [`Msg::Collected`].
    Collect(Vec<HostEntry>),
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
    /// → [`Msg::KeyPicked`] (nothing if cancelled).
    PickKeyFile,
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
    /// Errors come back as [`Msg::Notice`].
    OpenUrl(String),
    /// Start the new exe and close this window.
    Relaunch,
}
