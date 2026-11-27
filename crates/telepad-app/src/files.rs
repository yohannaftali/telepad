//! File windows (#62–#64): one per host, the local computer on the left and
//! the host on the right, with a transfer queue, file operations (new,
//! rename, delete, permissions, properties) and an editor window
//! ([`crate::editor`]). Pure state and update logic; the runtime does the
//! listing, copying, changing, loading and saving.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::files::compare::{Gone, SyncPlan, SyncRequest};
use telepad_core::files::edit::Stamp;
use telepad_core::files::ops::{Measure, Report, check_name};
use telepad_core::files::transfer::{Decision, Direction, Event, Item, Options, Source};
use telepad_core::files::{Entry, Kind, local_join, local_parent, sftp};

use telepad_core::history::{Entry as LogEntry, Kind as LogKind, Outcome};

use crate::editor::{EdDraft, EdMsg, FileEditor, Load, TabId, Unsaved, has_unsaved};

pub type FmId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    Local,
    Remote,
}

/// How the host is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Sftp,
    /// No SFTP server: `sh` listing and `scp -O` copies.
    Fallback,
    /// My Computer (#129): this computer on both sides, no SSH.
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortCol {
    #[default]
    Name,
    Size,
    Modified,
    Permissions,
    Owner,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pane {
    /// Absolute folder shown.
    pub path: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub error: Option<String>,
    pub sort: SortCol,
    pub descending: bool,
}

impl Pane {
    /// Entries in display order: folders first, then the chosen sort.
    pub fn sorted(&self, show_hidden: bool, filter: &str) -> Vec<&Entry> {
        let f = filter.trim().to_lowercase();
        let mut v: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| show_hidden || !e.hidden)
            .filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f))
            .collect();
        v.sort_by(|a, b| {
            let by = match self.sort {
                SortCol::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortCol::Size => a.size.cmp(&b.size),
                SortCol::Modified => a.mtime.cmp(&b.mtime),
                SortCol::Permissions => a.perm.cmp(&b.perm),
                SortCol::Owner => a.owner.cmp(&b.owner),
            };
            let by = if self.descending { by.reverse() } else { by };
            b.is_dir().cmp(&a.is_dir()).then(by)
        });
        v
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TState {
    /// Folders being expanded, existing files looked up.
    Planning,
    /// Waiting for the user's answers about existing files.
    Asking,
    /// rsync dry run done, waiting for Run.
    Previewed(Vec<String>),
    Running,
    Done {
        copied: usize,
        skipped: usize,
        failed: usize,
    },
    Failed(String),
    Cancelled,
}

impl TState {
    pub fn finished(&self) -> bool {
        matches!(
            self,
            TState::Done { .. } | TState::Failed(_) | TState::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    pub id: u64,
    pub direction: Direction,
    /// "3 items → /srv/www".
    pub title: String,
    pub sources: Vec<Source>,
    pub dest: String,
    pub rsync: bool,
    pub state: TState,
    pub items: Vec<Item>,
    pub decisions: HashMap<usize, Decision>,
    pub current: String,
    pub done_items: usize,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub started: Option<Instant>,
    pub errors: Vec<String>,
    /// Transfer settings when it was started (#131).
    pub opts: Options,
}

impl Transfer {
    /// Bytes per second since the copy started.
    pub fn speed(&self, now: Instant) -> Option<f64> {
        let t = now.duration_since(self.started?).as_secs_f64();
        (t > 0.5).then(|| self.bytes_done as f64 / t)
    }
}

/// The overwrite question for one existing file.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub transfer: u64,
    /// Item indexes of existing files, in order.
    pub conflicts: Vec<usize>,
    /// Position in `conflicts` being asked.
    pub at: usize,
}

/// A file operation the user asked for (toolbar, right-click menu, keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// F7
    NewFolder,
    NewFile,
    /// F2 (one selected item)
    Rename,
    /// Del (selected items)
    Delete,
    /// One selected item: size, dates, permissions, owner.
    Properties,
    /// Host side only: mode of the selected items.
    Permissions,
    /// F4 (one selected file): open it in the editor.
    Edit,
}

/// The dialog open in a file window.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogKind {
    NewFolder,
    NewFile,
    Rename {
        from: String,
    },
    /// Asks once for all items; `measure`: files / bytes inside each (while
    /// `None` the sizes are being counted).
    Delete {
        items: Vec<Entry>,
        measure: Option<Result<Vec<Measure>, String>>,
    },
    Properties {
        item: Entry,
        path: String,
        measure: Option<Result<Measure, String>>,
    },
    Permissions {
        items: Vec<Entry>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub side: Side,
    /// Folder the items are in.
    pub dir: String,
    pub kind: DialogKind,
    /// Why the last attempt failed (the dialog stays open).
    pub error: Option<String>,
    /// Waiting for the runtime.
    pub busy: bool,
}

/// Work for the runtime (both sides; the host side over SFTP).
#[derive(Debug, Clone, PartialEq)]
pub enum FileOp {
    NewFolder {
        dir: String,
        name: String,
    },
    NewFile {
        dir: String,
        name: String,
    },
    Rename {
        dir: String,
        from: String,
        to: String,
    },
    Delete {
        dir: String,
        items: Vec<Entry>,
    },
    Chmod {
        dir: String,
        items: Vec<Entry>,
        mode: u32,
        recursive: bool,
    },
    /// Files / bytes inside → [`FmMsg::Measured`].
    Measure {
        dir: String,
        items: Vec<Entry>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Connecting,
    Ready,
}

#[derive(Debug, Clone, Default)]
pub struct FileSession {
    pub id: FmId,
    pub host: HostEntry,
    pub status: Status,
    pub mode: Mode,
    /// Connection error (window shows it with Retry).
    pub error: Option<String>,
    pub local: Pane,
    pub remote: Pane,
    pub remote_home: String,
    pub transfers: Vec<Transfer>,
    pub ask: Option<Ask>,
    /// rsync usable on both sides (else why not); `None` while checking.
    pub rsync: Option<Result<(), String>>,
    pub dialog: Option<Dialog>,
    /// Errors of the last delete / permission change (one line per item).
    pub op_errors: Vec<String>,
    /// The editor window's tabs (#64).
    pub editor: FileEditor,
    /// A remote file (relative to the home folder) to open in the editor once
    /// connected (#85: Edit file of Authorized keys).
    pub pending_edit: Option<String>,
    /// Left with transfers running when the workspace changed (#99): no
    /// window, the transfers go on; shown again when its host's files are
    /// opened, closed when the last transfer ends.
    pub background: bool,
    /// Folders to show once connected (from a workspace), instead of the
    /// remembered ones.
    pub start_paths: Option<(String, String)>,
    /// Files to open in the editor once connected (from a workspace).
    pub open_paths: Vec<(Side, String)>,
    /// One-line note shown in the window (e.g. a folder that is gone).
    pub notice: Option<String>,
    /// History of this host's transfers and file operations (#100),
    /// oldest first.
    pub history: Vec<telepad_core::history::Entry>,
    /// Transfer settings of this host (#131): mode, new and updated only,
    /// excluded folders.
    pub opts: Options,
    /// The Synchronize dialog (#131).
    pub sync: Option<SyncView>,
    next_transfer: u64,
}

/// The Synchronize dialog: the two folders, and the plan once previewed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncView {
    pub left: String,
    pub right: String,
    /// Planning in the runtime.
    pub busy: bool,
    pub plan: Option<Result<SyncPlan, String>>,
    /// The request the plan was made for (Synchronize runs only that).
    pub planned: Option<SyncRequest>,
}

impl FileSession {
    pub fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        }
    }
    fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Local => &mut self.local,
            Side::Remote => &mut self.remote,
        }
    }
    /// The pane shows this computer: the left one always, the right one
    /// for My Computer (#129).
    pub fn local_side(&self, side: Side) -> bool {
        side == Side::Local || self.host.is_local()
    }
    pub fn transfer(&self, id: u64) -> Option<&Transfer> {
        self.transfers.iter().find(|t| t.id == id)
    }

    /// (local, remote) folders shown, or the ones it will show.
    pub fn folders(&self) -> (String, String) {
        if self.local.path.is_empty() && self.remote.path.is_empty() {
            return self.start_paths.clone().unwrap_or_default();
        }
        (self.local.path.clone(), self.remote.path.clone())
    }

    /// Transfers that keep running without the user (planning or copying).
    pub fn running_transfers(&self) -> usize {
        self.transfers
            .iter()
            .filter(|t| matches!(t.state, TState::Planning | TState::Running))
            .count()
    }

    /// Transfers waiting for the user (overwrite question, rsync preview).
    pub fn waiting_transfers(&self) -> usize {
        self.transfers
            .iter()
            .filter(|t| matches!(t.state, TState::Asking | TState::Previewed(_)))
            .count()
    }
    fn transfer_mut(&mut self, id: u64) -> Option<&mut Transfer> {
        self.transfers.iter_mut().find(|t| t.id == id)
    }
}

/// Text the window binds to.
#[derive(Debug, Clone, Default)]
pub struct FileDraft {
    pub local_path: String,
    pub remote_path: String,
    pub local_filter: String,
    pub remote_filter: String,
    pub selected_local: BTreeSet<String>,
    pub selected_remote: BTreeSet<String>,
    /// Pane that was clicked last (F5 copies from it).
    pub active: Option<Side>,
    pub show_hidden: bool,
    pub use_rsync: bool,
    /// "Apply to all" in the overwrite question.
    pub apply_all: bool,
    /// Name field of New folder / New file / Rename.
    pub name: String,
    /// Permissions dialog: mode bits and the octal text bound to them.
    pub mode: u32,
    pub mode_text: String,
    /// Permissions dialog: also everything inside folders.
    pub recursive: bool,
    /// The editor's texts and find bar.
    pub editor: EdDraft,
    /// The History tab of the transfer panel is shown (#100), its filters
    /// and whether Clear waits for its confirmation.
    pub show_history: bool,
    pub history_filter: String,
    pub history_dir: Option<Direction>,
    pub history_failed: bool,
    pub history_clear: bool,
    /// Panes whose `..` row is highlighted (#130; never copied or deleted).
    pub parent_selected: BTreeSet<Side>,
    /// Row clicked last per pane: Shift+click selects from it (#130).
    pub anchor: BTreeMap<Side, String>,
    /// Compare directories (#131) is on, and identical entries are hidden.
    pub compare: bool,
    pub hide_same: bool,
    /// The Synchronize form.
    pub sync: SyncRequest,
    /// The Transfer settings dialog is open, and its form.
    pub show_opts: bool,
    pub opts_form: Options,
    pub opts_exclude: String,
}

impl FileDraft {
    pub fn selected(&self, side: Side) -> &BTreeSet<String> {
        match side {
            Side::Local => &self.selected_local,
            Side::Remote => &self.selected_remote,
        }
    }
    pub fn selected_mut(&mut self, side: Side) -> &mut BTreeSet<String> {
        match side {
            Side::Local => &mut self.selected_local,
            Side::Remote => &mut self.selected_remote,
        }
    }
    /// Nothing selected in this pane (also not its `..` row).
    pub fn clear_selection(&mut self, side: Side) {
        self.selected_mut(side).clear();
        self.parent_selected.remove(&side);
        self.anchor.remove(&side);
    }
    /// A click on a row (#130): plain = only this row, Ctrl = toggle it,
    /// Shift = the rows from the last clicked one to this (in `order`).
    pub fn click_row(&mut self, side: Side, name: &str, order: &[&str], ctrl: bool, shift: bool) {
        self.parent_selected.remove(&side);
        let from = self
            .anchor
            .get(&side)
            .and_then(|a| order.iter().position(|n| n == a));
        let to = order.iter().position(|n| *n == name);
        let sel = self.selected_mut(side);
        match (shift, from, to) {
            (true, Some(a), Some(b)) => {
                if !ctrl {
                    sel.clear();
                }
                for n in &order[a.min(b)..=a.max(b)] {
                    sel.insert((*n).to_string());
                }
                return;
            }
            _ if ctrl => {
                if !sel.remove(name) {
                    sel.insert(name.to_string());
                }
            }
            _ => {
                sel.clear();
                sel.insert(name.to_string());
            }
        }
        self.anchor.insert(side, name.to_string());
    }
    /// A click on the `..` row: it alone is highlighted.
    pub fn click_parent(&mut self, side: Side) {
        self.clear_selection(side);
        self.parent_selected.insert(side);
    }
    fn path_mut(&mut self, side: Side) -> &mut String {
        match side {
            Side::Local => &mut self.local_path,
            Side::Remote => &mut self.remote_path,
        }
    }
}

/// How the host was reached: mode, remote home, remembered (local, remote) folders.
pub type Connection = (Mode, String, Option<(String, String)>);

#[derive(Debug, Clone)]
pub enum FmMsg {
    /// "Open files…" of a host.
    Open(String),
    /// Reopen from a workspace (#99): these folders, these editor files.
    Restore {
        alias: String,
        folders: Option<(String, String)>,
        edit: Vec<(Side, String)>,
    },
    /// Close without asking (the workspace was left: unsaved text goes).
    Discard(FmId),
    /// Leave the window with its running transfers in the background (#99):
    /// editor tabs, dialogs and waiting transfers are dropped.
    Background(FmId),
    /// Open a remote file (path relative to the login's home) of a host in the
    /// editor, connecting its file window first when needed (#85).
    OpenFile {
        alias: String,
        rel: String,
    },
    Connected {
        id: FmId,
        /// (mode, remote home, remembered (local, remote) folders).
        result: Result<Connection, String>,
        local_home: String,
    },
    Retry(FmId),
    Close(FmId),
    /// Go to a folder (absolute, or typed into the path bar).
    Navigate {
        id: FmId,
        side: Side,
        path: String,
    },
    /// Enter a folder of the list.
    Enter {
        id: FmId,
        side: Side,
        name: String,
    },
    Up {
        id: FmId,
        side: Side,
    },
    Home {
        id: FmId,
        side: Side,
    },
    Refresh {
        id: FmId,
        side: Side,
    },
    Listed {
        id: FmId,
        side: Side,
        result: Result<(String, Vec<Entry>), String>,
    },
    Sort {
        id: FmId,
        side: Side,
        col: SortCol,
    },
    /// Copy these names of `from` into the other pane's folder.
    Copy {
        id: FmId,
        from: Side,
        names: Vec<String>,
    },
    Planned {
        id: FmId,
        transfer: u64,
        result: Result<Vec<Item>, String>,
    },
    /// Answer to the overwrite question (Apply to all from the draft).
    Decide {
        id: FmId,
        decision: Decision,
    },
    /// Stop asking: cancel the whole transfer.
    AskCancel(FmId),
    Progress {
        id: FmId,
        transfer: u64,
        event: Event,
    },
    Finished {
        id: FmId,
        transfer: u64,
        result: Result<(usize, usize, usize), String>,
    },
    Cancel {
        id: FmId,
        transfer: u64,
    },
    ClearFinished(FmId),
    RsyncChecked {
        id: FmId,
        result: Result<(), String>,
    },
    RsyncPreviewed {
        id: FmId,
        transfer: u64,
        result: Result<Vec<String>, String>,
    },
    /// Run an rsync transfer after its preview.
    RsyncRun {
        id: FmId,
        transfer: u64,
    },
    /// Open the dialog of an operation for the selection of `side`.
    Ask {
        id: FmId,
        side: Side,
        op: OpKind,
    },
    /// Do what the open dialog says (name / mode from the draft).
    Confirm(FmId),
    CloseDialog(FmId),
    Measured {
        id: FmId,
        side: Side,
        result: Result<Vec<Measure>, String>,
    },
    /// An operation finished: `Ok` with what it did (and the name to select).
    OpDone {
        id: FmId,
        side: Side,
        result: Result<(Report, Option<String>), String>,
    },
    /// Copy the full paths of the selection (or of the folder) to the clipboard.
    CopyPath {
        id: FmId,
        side: Side,
    },
    DismissErrors(FmId),
    HistoryLoaded {
        id: FmId,
        result: Result<Vec<telepad_core::history::Entry>, String>,
    },
    /// Clear this host's history (asked first in the window).
    HistoryClear(FmId),
    /// Export the shown (filtered) history.
    HistoryExport(FmId),
    /// Open the local folder of a history line.
    HistoryOpenFolder {
        id: FmId,
        path: String,
    },
    /// The editor window (#64).
    Editor {
        id: FmId,
        msg: EdMsg,
    },
    /// This host's saved transfer settings (#131).
    OptionsLoaded {
        id: FmId,
        opts: Options,
    },
    /// Open the Transfer settings dialog with the current settings.
    OptionsOpen(FmId),
    /// Keep the dialog's settings (and save them for the host).
    OptionsSave(FmId),
    OptionsClose(FmId),
    /// Open the Synchronize dialog for the two folders shown.
    SyncOpen(FmId),
    /// Plan with the form's request (nothing changes yet).
    SyncPreview(FmId),
    SyncPlanned {
        id: FmId,
        result: Result<SyncPlan, String>,
    },
    /// Run the previewed plan.
    SyncRun(FmId),
    SyncClose(FmId),
    /// Compare one file of each pane in the editor, side by side.
    Diff(FmId),
}

#[derive(Debug, Clone)]
pub enum FmEffect {
    /// SFTP (else the `sh` fallback), home folder, remembered folders, local
    /// home → [`FmMsg::Connected`]; also checks rsync → [`FmMsg::RsyncChecked`].
    Connect {
        id: FmId,
        host: HostEntry,
    },
    /// → [`FmMsg::Listed`]
    List {
        id: FmId,
        side: Side,
        path: String,
    },
    /// Expand folders, find existing files → [`FmMsg::Planned`].
    Plan {
        id: FmId,
        transfer: u64,
        direction: Direction,
        sources: Vec<Source>,
        dest: String,
        opts: Options,
    },
    /// Copy over SFTP → [`FmMsg::Progress`]…, [`FmMsg::Finished`].
    Run {
        id: FmId,
        transfer: u64,
        direction: Direction,
        items: Vec<Item>,
        decisions: HashMap<usize, Decision>,
        opts: Options,
    },
    /// Fallback mode: `scp -O` each source → [`FmMsg::Finished`].
    Scp {
        id: FmId,
        transfer: u64,
        direction: Direction,
        sources: Vec<Source>,
        dest: String,
    },
    /// rsync (dry run → [`FmMsg::RsyncPreviewed`], else → [`FmMsg::Finished`]).
    Rsync {
        id: FmId,
        transfer: u64,
        direction: Direction,
        sources: Vec<String>,
        dest: String,
        dry_run: bool,
        opts: Options,
    },
    Cancel {
        id: FmId,
        transfer: u64,
    },
    /// Close both connections.
    Close(FmId),
    SavePaths {
        alias: String,
        local: String,
        remote: String,
    },
    /// → [`FmMsg::OpDone`] (or [`FmMsg::Measured`] for a measure).
    Op {
        id: FmId,
        side: Side,
        op: FileOp,
    },
    CopyText(String),
    /// Write one history entry to `history.db` (#100).
    Log(telepad_core::history::Entry),
    /// This host's history → [`FmMsg::HistoryLoaded`].
    LoadHistory {
        id: FmId,
        alias: String,
    },
    ClearHistory {
        alias: String,
    },
    /// Save entries as CSV (native save dialog).
    ExportHistory(Vec<telepad_core::history::Entry>),
    /// Show a local folder in the system's file manager.
    OpenFolder(String),
    /// Read and decode a file for the editor → [`EdMsg::Loaded`].
    Load {
        id: FmId,
        tab: TabId,
        side: Side,
        path: String,
        load: Load,
        /// Read as this encoding (Reopen with Encoding, #203); `None` = detect.
        charset: Option<telepad_core::files::text::Charset>,
    },
    /// Plan Synchronize of `left` (this computer) with `right` (#131) →
    /// [`FmMsg::SyncPlanned`].
    SyncPlan {
        id: FmId,
        left: String,
        right: String,
        req: SyncRequest,
        opts: Options,
    },
    /// Keep a host's transfer settings in `telepad.db`.
    SaveOptions {
        alias: String,
        opts: Options,
    },
    /// Write a file of the editor (only if it still has `expect`, when given)
    /// → [`EdMsg::Saved`].
    Save {
        id: FmId,
        tab: TabId,
        side: Side,
        path: String,
        bytes: Vec<u8>,
        expect: Option<Stamp>,
    },
}

fn list(fx: &mut Vec<FmEffect>, s: &mut FileSession, d: &mut FileDraft, side: Side, path: String) {
    let p = s.pane_mut(side);
    p.loading = true;
    p.error = None;
    *d.path_mut(side) = path.clone();
    fx.push(FmEffect::List {
        id: s.id,
        side,
        path,
    });
}

/// `local`: a path of this computer (see [`FileSession::local_side`]).
fn join(local: bool, dir: &str, name: &str) -> String {
    if local {
        local_join(dir, name)
    } else {
        sftp::join(dir, name)
    }
}

fn parent(local: bool, dir: &str) -> Option<String> {
    if local {
        local_parent(dir)
    } else {
        (dir != "/").then(|| sftp::parent(dir))
    }
}

/// Fallback mode copies whole sources with scp; the existing check is by
/// name in the target pane.
fn fallback_items(s: &FileSession, from: Side, names: &[String]) -> Vec<Item> {
    let (src, dst) = (s.pane(from), s.pane(other(from)));
    names
        .iter()
        .filter_map(|n| src.entries.iter().find(|e| &e.name == n))
        .map(|e| Item {
            src: join(s.local_side(from), &src.path, &e.name),
            dst: join(s.local_side(other(from)), &dst.path, &e.name),
            label: e.name.clone(),
            is_dir: e.is_dir(),
            size: e.size,
            mtime: e.mtime,
            perm: e.perm,
            exists: dst
                .entries
                .iter()
                .find(|x| x.name == e.name)
                .map(|x| (x.size, x.mtime)),
        })
        .collect()
}

/// Last part of a folder path for titles (`…/www`); the full path is the transfer's `dest`.
pub fn short_dir(path: &str) -> String {
    let p = path.trim_end_matches(['/', '\\']);
    match p.rfind(['/', '\\']) {
        Some(i) if i + 1 < p.len() && p.len() > 24 => format!("…{}", &p[i..]),
        _ => path.to_string(),
    }
}

pub fn other(side: Side) -> Side {
    match side {
        Side::Local => Side::Remote,
        Side::Remote => Side::Local,
    }
}

/// Ask about the next existing file, or start the copy.
fn next_or_run(s: &mut FileSession, transfer: u64, fx: &mut Vec<FmEffect>, now: Instant) {
    let id = s.id;
    let mode = s.mode;
    if let Some(a) = &s.ask
        && a.transfer == transfer
        && a.at < a.conflicts.len()
    {
        return;
    }
    s.ask = None;
    let Some(t) = s.transfer_mut(transfer) else {
        return;
    };
    t.state = TState::Running;
    t.started = Some(now);
    t.bytes_total = t
        .items
        .iter()
        .enumerate()
        .filter(|(i, it)| !it.is_dir && it.should_copy(t.decisions.get(i).copied()))
        .map(|(_, it)| it.size)
        .sum();
    match mode {
        Mode::Sftp | Mode::Local => fx.push(FmEffect::Run {
            id,
            transfer,
            direction: t.direction,
            items: t.items.clone(),
            decisions: t.decisions.clone(),
            opts: t.opts.clone(),
        }),
        Mode::Fallback => {
            let sources: Vec<Source> = t
                .items
                .iter()
                .enumerate()
                .filter(|(i, it)| it.should_copy(t.decisions.get(i).copied()))
                .map(|(_, it)| (it.src.clone(), it.is_dir))
                .collect();
            fx.push(FmEffect::Scp {
                id,
                transfer,
                direction: t.direction,
                sources,
                dest: t.dest.clone(),
            });
        }
    }
}

/// Items planned: ask about existing files first, if any.
fn planned(
    s: &mut FileSession,
    transfer: u64,
    items: Vec<Item>,
    fx: &mut Vec<FmEffect>,
    now: Instant,
) {
    let Some(t) = s.transfer_mut(transfer) else {
        return;
    };
    let mut conflicts: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(n, i)| !i.is_dir && i.exists.is_some() && !t.decisions.contains_key(n))
        .map(|(n, _)| n)
        .collect();
    // New and updated files only (#131): no question, newer sources win.
    if t.opts.newer_only {
        for n in conflicts.drain(..) {
            t.decisions.insert(n, Decision::NewerOnly);
        }
    }
    t.items = items;
    if conflicts.is_empty() {
        next_or_run(s, transfer, fx, now);
    } else {
        t.state = TState::Asking;
        s.ask = Some(Ask {
            transfer,
            conflicts,
            at: 0,
        });
    }
}

pub fn update(
    sessions: &mut Vec<FileSession>,
    drafts: &mut BTreeMap<FmId, FileDraft>,
    next_id: &mut FmId,
    host_entry: &dyn Fn(&str) -> Option<HostEntry>,
    msg: FmMsg,
    now: Instant,
) -> Vec<FmEffect> {
    let mut fx = Vec::new();
    if let FmMsg::OpenFile { alias, rel } = msg {
        if let Some(pos) = sessions.iter().position(|s| s.host.alias == alias) {
            let s = &mut sessions[pos];
            if s.status == Status::Ready {
                let path = sftp::join(&s.remote_home, &rel);
                let d = drafts.entry(s.id).or_default();
                let close = crate::editor::update(
                    s,
                    d,
                    crate::editor::EdMsg::OpenPath {
                        side: Side::Remote,
                        path,
                    },
                    &mut fx,
                );
                debug_assert!(!close);
            } else {
                s.pending_edit = Some(rel);
            }
            return fx;
        }
        let Some(host) = host_entry(&alias) else {
            return fx;
        };
        *next_id += 1;
        let id = *next_id;
        sessions.push(FileSession {
            id,
            host: host.clone(),
            pending_edit: Some(rel),
            ..Default::default()
        });
        drafts.insert(id, FileDraft::default());
        fx.push(FmEffect::LoadHistory {
            id,
            alias: host.alias.clone(),
        });
        fx.push(FmEffect::Connect { id, host });
        return fx;
    }
    if let FmMsg::Restore {
        alias,
        folders,
        edit,
    } = msg
    {
        if let Some(s) = sessions.iter_mut().find(|s| s.host.alias == alias) {
            s.background = false;
            let id = s.id;
            if s.status == Status::Ready {
                let d = drafts.entry(id).or_default();
                for (side, path) in edit {
                    crate::editor::update(
                        s,
                        d,
                        crate::editor::EdMsg::OpenPath { side, path },
                        &mut fx,
                    );
                }
            } else {
                s.open_paths.extend(edit);
            }
            return fx;
        }
        let Some(host) = host_entry(&alias) else {
            return fx;
        };
        *next_id += 1;
        let id = *next_id;
        sessions.push(FileSession {
            id,
            host: host.clone(),
            start_paths: folders,
            open_paths: edit,
            ..Default::default()
        });
        drafts.insert(id, FileDraft::default());
        fx.push(FmEffect::LoadHistory {
            id,
            alias: host.alias.clone(),
        });
        fx.push(FmEffect::Connect { id, host });
        return fx;
    }
    if let FmMsg::Open(alias) = msg {
        if let Some(s) = sessions.iter_mut().find(|s| s.host.alias == alias) {
            s.background = false;
            return fx;
        }
        let Some(host) = host_entry(&alias) else {
            return fx;
        };
        *next_id += 1;
        let id = *next_id;
        sessions.push(FileSession {
            id,
            host: host.clone(),
            ..Default::default()
        });
        drafts.insert(id, FileDraft::default());
        fx.push(FmEffect::LoadHistory {
            id,
            alias: host.alias.clone(),
        });
        fx.push(FmEffect::Connect { id, host });
        return fx;
    }
    let id = msg_id(&msg);
    let Some(pos) = sessions.iter().position(|s| s.id == id) else {
        return fx;
    };
    if let FmMsg::Editor { msg, .. } = msg {
        if let EdMsg::Saved { tab, result } = &msg
            && let Some(t) = sessions[pos].editor.tab(*tab)
        {
            let (outcome, error) = match result {
                Ok(_) => (Outcome::Done, String::new()),
                Err(e) => (Outcome::Failed, format!("{e:?}")),
            };
            let e = file_entry(
                &sessions[pos],
                LogKind::FileOp,
                format!("Save {}", side_label(t.side, &t.path)),
                t.path.clone(),
                outcome,
                error,
            );
            log(&mut sessions[pos], &mut fx, e);
        }
        let d = drafts.entry(id).or_default();
        if crate::editor::update(&mut sessions[pos], d, msg, &mut fx) {
            close_session(sessions, drafts, pos, &mut fx);
        }
        return fx;
    }
    if let FmMsg::Discard(_) = msg {
        close_session(sessions, drafts, pos, &mut fx);
        return fx;
    }
    if let FmMsg::Background(_) = msg {
        let s = &mut sessions[pos];
        if s.running_transfers() == 0 {
            close_session(sessions, drafts, pos, &mut fx);
            return fx;
        }
        for t in s.transfers.iter_mut() {
            if matches!(t.state, TState::Asking | TState::Previewed(_)) {
                t.state = TState::Cancelled;
            }
        }
        s.ask = None;
        s.dialog = None;
        s.editor = Default::default();
        s.pending_edit = None;
        s.open_paths.clear();
        s.background = true;
        if let Some(d) = drafts.get_mut(&id) {
            d.editor = Default::default();
            d.name.clear();
        }
        return fx;
    }
    if let FmMsg::Close(_) = msg {
        // Unsaved editor tabs: ask first (the window stays open).
        let d = drafts.entry(id).or_default();
        let s = &mut sessions[pos];
        if has_unsaved(&s.editor, &d.editor) {
            s.editor.ask = Some(Unsaved::All { window: true });
            return fx;
        }
        close_session(sessions, drafts, pos, &mut fx);
        return fx;
    }
    let s = &mut sessions[pos];
    let d = drafts.entry(id).or_default();
    match msg {
        FmMsg::Open(_)
        | FmMsg::Restore { .. }
        | FmMsg::Discard(_)
        | FmMsg::Background(_)
        | FmMsg::OpenFile { .. }
        | FmMsg::Close(_)
        | FmMsg::Editor { .. } => {}
        FmMsg::Connected {
            result, local_home, ..
        } => match result {
            Ok((mode, home, remembered)) => {
                s.status = Status::Ready;
                s.mode = mode;
                s.error = None;
                s.remote_home = home.clone();
                let (l, r) = s
                    .start_paths
                    .take()
                    .filter(|(l, r)| !l.is_empty() && !r.is_empty())
                    .or(remembered)
                    .unwrap_or((local_home, home));
                list(&mut fx, s, d, Side::Local, l);
                list(&mut fx, s, d, Side::Remote, r);
                for (side, path) in std::mem::take(&mut s.open_paths) {
                    crate::editor::update(
                        s,
                        d,
                        crate::editor::EdMsg::OpenPath { side, path },
                        &mut fx,
                    );
                }
                if let Some(rel) = s.pending_edit.take() {
                    let path = sftp::join(&s.remote_home, &rel);
                    crate::editor::update(
                        s,
                        d,
                        crate::editor::EdMsg::OpenPath {
                            side: Side::Remote,
                            path,
                        },
                        &mut fx,
                    );
                }
            }
            Err(e) => s.error = Some(e),
        },
        FmMsg::Retry(_) => {
            s.error = None;
            s.status = Status::Connecting;
            fx.push(FmEffect::Close(id));
            fx.push(FmEffect::Connect {
                id,
                host: s.host.clone(),
            });
        }
        FmMsg::Navigate { side, path, .. } => {
            let p = path.trim().to_string();
            if !p.is_empty() {
                d.clear_selection(side);
                list(&mut fx, s, d, side, p);
            }
        }
        FmMsg::Enter { side, name, .. } => {
            let p = join(s.local_side(side), &s.pane(side).path, &name);
            d.clear_selection(side);
            list(&mut fx, s, d, side, p);
        }
        FmMsg::Up { side, .. } => {
            if let Some(p) = parent(s.local_side(side), &s.pane(side).path) {
                d.clear_selection(side);
                list(&mut fx, s, d, side, p);
            }
        }
        FmMsg::Home { side, .. } => {
            let p = match side {
                Side::Local => telepad_core::files::local_home(),
                Side::Remote => s.remote_home.clone(),
            };
            d.clear_selection(side);
            list(&mut fx, s, d, side, p);
        }
        FmMsg::Refresh { side, .. } => {
            let p = s.pane(side).path.clone();
            list(&mut fx, s, d, side, p);
        }
        FmMsg::Listed { side, result, .. } => {
            let p = s.pane_mut(side);
            p.loading = false;
            match result {
                Ok((path, entries)) => {
                    p.path = path.clone();
                    p.entries = entries;
                    p.error = None;
                    *d.path_mut(side) = path;
                    let names: BTreeSet<String> =
                        p.entries.iter().map(|e| e.name.clone()).collect();
                    d.selected_mut(side).retain(|n| names.contains(n));
                }
                Err(e) if p.path.is_empty() => {
                    // A folder saved in the workspace is gone (#99): the home
                    // folder instead, with a note.
                    let home = match side {
                        Side::Local => telepad_core::files::local_home(),
                        Side::Remote => s.remote_home.clone(),
                    };
                    let gone = d.path_mut(side).clone();
                    if !home.is_empty() && home != gone {
                        s.notice = Some(format!(
                            "{gone} cannot be opened ({e}); showing the home folder"
                        ));
                        list(&mut fx, s, d, side, home);
                    } else {
                        s.pane_mut(side).error = Some(e);
                    }
                }
                Err(e) => {
                    // Keep showing the folder that worked; the path bar goes back to it.
                    p.error = Some(e);
                    *d.path_mut(side) = p.path.clone();
                }
            }
        }
        FmMsg::Sort { side, col, .. } => {
            let p = s.pane_mut(side);
            if p.sort == col {
                p.descending = !p.descending;
            } else {
                p.sort = col;
                p.descending = false;
            }
        }
        FmMsg::Copy { from, names, .. } => {
            if names.is_empty() || s.status != Status::Ready {
                return fx;
            }
            let direction = match from {
                Side::Local => Direction::Upload,
                Side::Remote => Direction::Download,
            };
            let src_dir = s.pane(from).path.clone();
            let src_local = s.local_side(from);
            let dest = s.pane(other(from)).path.clone();
            // Excluded folders (#131) stay out, also when picked themselves.
            let names: Vec<String> = names
                .into_iter()
                .filter(|n| {
                    !s.pane(from)
                        .entries
                        .iter()
                        .any(|e| &e.name == n && e.is_dir() && s.opts.excluded(n))
                })
                .collect();
            let sources: Vec<Source> = names
                .iter()
                .filter_map(|n| s.pane(from).entries.iter().find(|e| &e.name == n))
                .map(|e| (join(src_local, &src_dir, &e.name), e.is_dir()))
                .collect();
            if sources.is_empty() {
                s.notice =
                    Some("Nothing to copy: the folders are excluded in Transfer settings".into());
                return fx;
            }
            s.next_transfer += 1;
            let tid = s.next_transfer;
            let rsync = d.use_rsync && matches!(s.rsync, Some(Ok(())));
            let what = if names.len() == 1 {
                names[0].clone()
            } else {
                format!("{} items", names.len())
            };
            s.transfers.push(Transfer {
                id: tid,
                direction,
                title: format!(
                    "{} {what} {} {}",
                    if direction == Direction::Upload {
                        "Upload"
                    } else {
                        "Download"
                    },
                    "→",
                    short_dir(&dest)
                ),
                sources: sources.clone(),
                dest: dest.clone(),
                rsync,
                state: TState::Planning,
                items: vec![],
                decisions: HashMap::new(),
                current: String::new(),
                done_items: 0,
                bytes_total: 0,
                bytes_done: 0,
                started: None,
                errors: vec![],
                opts: s.opts.clone(),
            });
            if rsync {
                fx.push(FmEffect::Rsync {
                    id,
                    transfer: tid,
                    direction,
                    sources: sources.into_iter().map(|(p, _)| p).collect(),
                    dest,
                    dry_run: true,
                    opts: s.opts.clone(),
                });
            } else if s.mode == Mode::Fallback {
                let items = fallback_items(s, from, &names);
                planned(s, tid, items, &mut fx, now);
            } else {
                fx.push(FmEffect::Plan {
                    id,
                    transfer: tid,
                    direction,
                    sources,
                    dest,
                    opts: s.opts.clone(),
                });
            }
        }
        FmMsg::Planned {
            transfer, result, ..
        } => match result {
            Ok(items) => planned(s, transfer, items, &mut fx, now),
            Err(e) => {
                if let Some(t) = s.transfer_mut(transfer) {
                    t.state = if e.contains("Cancelled") {
                        TState::Cancelled
                    } else {
                        TState::Failed(e)
                    };
                }
            }
        },
        FmMsg::Decide { decision, .. } => {
            let Some(a) = s.ask.clone() else {
                return fx;
            };
            let all = d.apply_all;
            if let Some(t) = s.transfer_mut(a.transfer) {
                let targets: Vec<usize> = if all {
                    a.conflicts[a.at..].to_vec()
                } else {
                    vec![a.conflicts[a.at]]
                };
                for i in targets {
                    t.decisions.insert(i, decision);
                }
            }
            let at = if all { a.conflicts.len() } else { a.at + 1 };
            s.ask = Some(Ask { at, ..a.clone() });
            d.apply_all = false;
            next_or_run(s, a.transfer, &mut fx, now);
        }
        FmMsg::AskCancel(_) => {
            if let Some(a) = s.ask.take()
                && let Some(t) = s.transfer_mut(a.transfer)
            {
                t.state = TState::Cancelled;
            }
        }
        FmMsg::Progress {
            transfer, event, ..
        } => {
            if let Some(t) = s.transfer_mut(transfer) {
                match event {
                    Event::Start(i) => {
                        if let Some(it) = t.items.get(i) {
                            t.current = it.label.clone();
                        }
                    }
                    Event::Bytes(n) => t.bytes_done += n,
                    Event::Done(i, r) => {
                        t.done_items += 1;
                        if let Err(e) = r
                            && !e.contains("Cancelled")
                        {
                            let label = t
                                .items
                                .get(i)
                                .map(|it| it.label.clone())
                                .unwrap_or_default();
                            t.errors.push(format!("{label}: {e}"));
                        }
                    }
                }
            }
        }
        FmMsg::Finished {
            transfer, result, ..
        } => {
            let mut refresh = None;
            let mut entry = None;
            let mode = s.mode;
            if let Some(t) = s.transfer_mut(transfer) {
                t.current.clear();
                t.state = match result {
                    Ok((copied, skipped, failed)) => TState::Done {
                        copied,
                        skipped,
                        failed,
                    },
                    Err(e) if e.contains("Cancelled") => TState::Cancelled,
                    Err(e) => TState::Failed(e),
                };
                refresh = Some(match t.direction {
                    Direction::Upload => Side::Remote,
                    Direction::Download => Side::Local,
                });
                entry = Some(transfer_entry(t, mode, now));
            }
            if let Some(e) = entry {
                let e = LogEntry {
                    host: s.host.alias.clone(),
                    ..e
                };
                log(s, &mut fx, e);
            }
            if let Some(side) = refresh {
                let p = s.pane(side).path.clone();
                list(&mut fx, s, d, side, p);
            }
        }
        FmMsg::Cancel { transfer, .. } => {
            if let Some(t) = s.transfer_mut(transfer) {
                match t.state {
                    TState::Asking | TState::Previewed(_) => {
                        t.state = TState::Cancelled;
                        if s.ask.as_ref().is_some_and(|a| a.transfer == transfer) {
                            s.ask = None;
                        }
                    }
                    TState::Planning | TState::Running => {
                        fx.push(FmEffect::Cancel { id, transfer })
                    }
                    _ => {}
                }
            }
        }
        FmMsg::ClearFinished(_) => s.transfers.retain(|t| !t.state.finished()),
        FmMsg::RsyncChecked { result, .. } => s.rsync = Some(result),
        FmMsg::RsyncPreviewed {
            transfer, result, ..
        } => {
            if let Some(t) = s.transfer_mut(transfer) {
                t.state = match result {
                    Ok(list) => TState::Previewed(list),
                    Err(e) => TState::Failed(e),
                };
            }
        }
        FmMsg::Ask { side, op, .. } => ask(s, d, side, op, &mut fx),
        FmMsg::Confirm(_) => confirm(s, d, &mut fx),
        FmMsg::CloseDialog(_) => s.dialog = None,
        FmMsg::Measured { side, result, .. } => {
            if let Some(dl) = s.dialog.as_mut()
                && dl.side == side
            {
                match &mut dl.kind {
                    DialogKind::Delete { measure, .. } => *measure = Some(result),
                    DialogKind::Properties { measure, .. } => {
                        *measure = Some(result.and_then(|v| {
                            v.into_iter()
                                .next()
                                .ok_or_else(|| "nothing measured".to_string())
                        }))
                    }
                    _ => {}
                }
            }
        }
        FmMsg::OpDone { side, result, .. } => {
            let path = s.pane(side).path.clone();
            if let Some(dl) = s.dialog.as_ref().filter(|dl| dl.side == side) {
                let (title, detail) = op_text(dl, s.local_side(dl.side), &d.name, d.mode);
                let (outcome, error) = match &result {
                    Ok((r, _)) if r.errors.is_empty() => (Outcome::Done, String::new()),
                    Ok((r, _)) => (Outcome::Failed, r.errors.join("\n")),
                    Err(e) => (Outcome::Failed, e.clone()),
                };
                if !matches!(dl.kind, DialogKind::Properties { .. }) {
                    let e = file_entry(
                        s,
                        LogKind::FileOp,
                        side_label(side, &title),
                        detail,
                        outcome,
                        error,
                    );
                    log(s, &mut fx, e);
                }
            }
            match result {
                Ok((report, select)) => {
                    s.dialog = None;
                    s.op_errors = report.errors;
                    let sel = d.selected_mut(side);
                    sel.clear();
                    if let Some(n) = select {
                        sel.insert(n);
                    }
                    list(&mut fx, s, d, side, path);
                }
                Err(e) => match s.dialog.as_mut() {
                    // New / Rename: say why and let the user try another name.
                    Some(dl) if dl.side == side => {
                        dl.busy = false;
                        dl.error = Some(e);
                    }
                    _ => s.op_errors = vec![e],
                },
            }
        }
        FmMsg::CopyPath { side, .. } => {
            let p = s.pane(side);
            let sel = d.selected(side);
            let text = if sel.is_empty() {
                p.path.clone()
            } else {
                sel.iter()
                    .map(|n| join(s.local_side(side), &p.path, n))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            if !text.is_empty() {
                fx.push(FmEffect::CopyText(text));
            }
        }
        FmMsg::DismissErrors(_) => {
            s.op_errors.clear();
            s.notice = None;
        }
        FmMsg::HistoryLoaded { result, .. } => match result {
            Ok(mut v) => {
                v.extend(s.history.drain(..).filter(|e| e.at.is_none()));
                s.history = v;
            }
            Err(e) => s.notice = Some(format!("History not available: {e}")),
        },
        FmMsg::HistoryClear(_) => {
            d.history_clear = false;
            s.history.clear();
            fx.push(FmEffect::ClearHistory {
                alias: s.host.alias.clone(),
            });
        }
        FmMsg::HistoryExport(_) => {
            fx.push(FmEffect::ExportHistory(
                history_shown(s, d).into_iter().cloned().collect(),
            ));
        }
        FmMsg::HistoryOpenFolder { path, .. } => fx.push(FmEffect::OpenFolder(path)),
        FmMsg::RsyncRun { transfer, .. } => {
            if let Some(t) = s.transfer_mut(transfer)
                && matches!(t.state, TState::Previewed(_))
            {
                t.state = TState::Running;
                t.started = Some(now);
                fx.push(FmEffect::Rsync {
                    id,
                    transfer,
                    direction: t.direction,
                    sources: t.sources.iter().map(|(p, _)| p.clone()).collect(),
                    dest: t.dest.clone(),
                    dry_run: false,
                    opts: t.opts.clone(),
                });
            }
        }
        FmMsg::OptionsLoaded { opts, .. } => s.opts = opts,
        FmMsg::OptionsOpen(_) => {
            d.opts_form = s.opts.clone();
            d.opts_exclude = s.opts.exclude.join("; ");
            d.show_opts = true;
        }
        FmMsg::OptionsSave(_) => {
            let opts = Options {
                exclude: Options::parse_exclude(&d.opts_exclude),
                ..d.opts_form.clone()
            };
            d.show_opts = false;
            if opts != s.opts {
                s.opts = opts.clone();
                fx.push(FmEffect::SaveOptions {
                    alias: s.host.alias.clone(),
                    opts,
                });
            }
        }
        FmMsg::OptionsClose(_) => d.show_opts = false,
        FmMsg::SyncOpen(_) => {
            if s.status != Status::Ready {
                return fx;
            }
            if s.mode == Mode::Fallback {
                s.notice = Some("Synchronize needs SFTP on the host".into());
                return fx;
            }
            s.sync = Some(SyncView {
                left: s.local.path.clone(),
                right: s.remote.path.clone(),
                ..Default::default()
            });
        }
        FmMsg::SyncPreview(_) => {
            if let Some(v) = s.sync.as_mut()
                && !v.busy
            {
                v.busy = true;
                v.plan = None;
                v.planned = Some(d.sync);
                fx.push(FmEffect::SyncPlan {
                    id,
                    left: v.left.clone(),
                    right: v.right.clone(),
                    req: d.sync,
                    opts: s.opts.clone(),
                });
            }
        }
        FmMsg::SyncPlanned { result, .. } => {
            if let Some(v) = s.sync.as_mut() {
                v.busy = false;
                v.plan = Some(result);
            }
        }
        FmMsg::SyncRun(_) => {
            let ready = matches!(
                &s.sync,
                Some(SyncView {
                    busy: false,
                    plan: Some(Ok(_)),
                    planned: Some(_),
                    ..
                })
            );
            if !ready {
                return fx;
            }
            let Some(SyncView {
                left,
                right,
                plan: Some(Ok(plan)),
                planned: Some(_),
                ..
            }) = s.sync.take()
            else {
                return fx;
            };
            run_sync(s, plan, &left, &right, &mut fx, now);
        }
        FmMsg::SyncClose(_) => s.sync = None,
        FmMsg::Diff(_) => diff(s, d, &mut fx),
    }
    fx
}

/// Run a previewed Synchronize plan: one transfer each way (existing files
/// replaced without asking: the preview showed them), and the deletes.
fn run_sync(
    s: &mut FileSession,
    plan: SyncPlan,
    left: &str,
    right: &str,
    fx: &mut Vec<FmEffect>,
    now: Instant,
) {
    let id = s.id;
    for (direction, items, dest) in [
        (Direction::Upload, plan.upload, right),
        (Direction::Download, plan.download, left),
    ] {
        if items.is_empty() {
            continue;
        }
        s.next_transfer += 1;
        let tid = s.next_transfer;
        let decisions: HashMap<usize, Decision> = items
            .iter()
            .enumerate()
            .filter(|(_, i)| !i.is_dir && i.exists.is_some())
            .map(|(n, _)| (n, Decision::Overwrite))
            .collect();
        let files = items.iter().filter(|i| !i.is_dir).count();
        s.transfers.push(Transfer {
            id: tid,
            direction,
            title: format!("Synchronize {files} file(s) {} {}", "→", short_dir(dest)),
            sources: vec![],
            dest: dest.to_string(),
            rsync: false,
            state: TState::Planning,
            items: vec![],
            decisions,
            current: String::new(),
            done_items: 0,
            bytes_total: 0,
            bytes_done: 0,
            started: None,
            errors: vec![],
            // Decided already; "newer only" was applied by the plan.
            opts: Options {
                newer_only: false,
                ..s.opts.clone()
            },
        });
        planned(s, tid, items, fx, now);
    }
    for (side, gone) in [
        (Side::Local, plan.delete_left),
        (Side::Remote, plan.delete_right),
    ] {
        let mut by_dir: BTreeMap<String, Vec<Gone>> = BTreeMap::new();
        for g in gone {
            by_dir.entry(g.dir.clone()).or_default().push(g);
        }
        for (dir, v) in by_dir {
            let detail = v
                .iter()
                .map(|g| g.label.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let e = file_entry(
                s,
                LogKind::FileOp,
                side_label(side, &format!("Synchronize: delete {} item(s)", v.len())),
                detail,
                Outcome::Done,
                String::new(),
            );
            log(s, fx, e);
            fx.push(FmEffect::Op {
                id,
                side,
                op: FileOp::Delete {
                    dir,
                    items: v.into_iter().map(|g| g.entry).collect(),
                },
            });
        }
    }
}

/// Compare one file of each pane in the editor, side by side (#131): the
/// selected file of each pane, or the one selected file and the file of the
/// same name in the other pane.
fn diff(s: &mut FileSession, d: &mut FileDraft, fx: &mut Vec<FmEffect>) {
    let one = |side: Side| -> Option<String> {
        let sel = d.selected(side);
        if sel.len() != 1 {
            return None;
        }
        let n = sel.iter().next()?;
        s.pane(side)
            .entries
            .iter()
            .find(|e| &e.name == n && !e.is_dir())
            .map(|e| e.name.clone())
    };
    let has = |side: Side, n: &str| {
        s.pane(side)
            .entries
            .iter()
            .any(|e| e.name == n && !e.is_dir())
    };
    let pair = match (one(Side::Local), one(Side::Remote)) {
        (Some(l), Some(r)) => Some((l, r)),
        (Some(l), None) if d.selected(Side::Remote).is_empty() && has(Side::Remote, &l) => {
            Some((l.clone(), l))
        }
        (None, Some(r)) if d.selected(Side::Local).is_empty() && has(Side::Local, &r) => {
            Some((r.clone(), r))
        }
        _ => None,
    };
    let Some((l, r)) = pair else {
        s.notice =
            Some("Diff: select one file in each pane, or one file whose name is in both".into());
        return;
    };
    if s.mode == Mode::Fallback {
        s.notice = Some("Diff needs SFTP on the host".into());
        return;
    }
    crate::editor::update(
        s,
        d,
        EdMsg::Open {
            side: Side::Local,
            name: l.clone(),
        },
        fx,
    );
    crate::editor::update(
        s,
        d,
        EdMsg::Open {
            side: Side::Remote,
            name: r.clone(),
        },
        fx,
    );
    let path = |side: Side, n: &str| join(s.local_side(side), &s.pane(side).path, n);
    let (lp, rp) = (path(Side::Local, &l), path(Side::Remote, &r));
    let find = |side: Side, p: &str| {
        s.editor
            .tabs
            .iter()
            .find(|t| t.side == side && t.path == p)
            .map(|t| t.id)
    };
    if let (Some(a), Some(b)) = (find(Side::Local, &lp), find(Side::Remote, &rp)) {
        s.editor.compare = Some((a, b));
        s.editor.active = Some(a);
    }
}

fn close_session(
    sessions: &mut Vec<FileSession>,
    drafts: &mut BTreeMap<FmId, FileDraft>,
    pos: usize,
    fx: &mut Vec<FmEffect>,
) {
    let s = sessions.remove(pos);
    let id = s.id;
    drafts.remove(&id);
    if s.status == Status::Ready {
        fx.push(FmEffect::SavePaths {
            alias: s.host.alias.clone(),
            local: s.local.path.clone(),
            remote: s.remote.path.clone(),
        });
    }
    for t in s.transfers.iter().filter(|t| !t.state.finished()) {
        fx.push(FmEffect::Cancel { id, transfer: t.id });
    }
    fx.push(FmEffect::Close(id));
}

/// Why `op` cannot run on `side` now (with `sel` items selected), if it cannot.
pub fn op_blocked(s: &FileSession, sel: usize, side: Side, op: OpKind) -> Option<&'static str> {
    if s.status != Status::Ready {
        return Some("Not connected");
    }
    if side == Side::Remote && s.mode == Mode::Fallback {
        return Some("Needs SFTP on the host");
    }
    if s.dialog.is_some() {
        return Some("Another dialog is open");
    }
    match op {
        OpKind::NewFolder | OpKind::NewFile => None,
        OpKind::Rename | OpKind::Properties if sel != 1 => Some("Select one item"),
        OpKind::Edit if sel != 1 => Some("Select one file"),
        OpKind::Delete if sel == 0 => Some("Select items first"),
        OpKind::Permissions if s.local_side(side) => Some("Host side only"),
        OpKind::Permissions if sel == 0 => Some("Select items first"),
        _ => None,
    }
}

fn selected_entries(s: &FileSession, d: &FileDraft, side: Side) -> Vec<Entry> {
    let sel = d.selected(side);
    s.pane(side)
        .entries
        .iter()
        .filter(|e| sel.contains(&e.name))
        .cloned()
        .collect()
}

fn measure_op(fx: &mut Vec<FmEffect>, id: FmId, side: Side, dir: &str, items: &[Entry]) {
    fx.push(FmEffect::Op {
        id,
        side,
        op: FileOp::Measure {
            dir: dir.to_string(),
            items: items.to_vec(),
        },
    });
}

fn file_measure(e: &Entry) -> Measure {
    Measure {
        files: 1,
        bytes: e.size,
        ..Default::default()
    }
}

fn ask(s: &mut FileSession, d: &mut FileDraft, side: Side, op: OpKind, fx: &mut Vec<FmEffect>) {
    let items = selected_entries(s, d, side);
    if op_blocked(s, items.len(), side, op).is_some() {
        return;
    }
    if op == OpKind::Edit {
        d.active = Some(side);
        let name = items[0].name.clone();
        crate::editor::update(s, d, EdMsg::Open { side, name }, fx);
        return;
    }
    let id = s.id;
    let dir = s.pane(side).path.clone();
    d.active = Some(side);
    let kind = match op {
        OpKind::NewFolder => {
            d.name.clear();
            DialogKind::NewFolder
        }
        OpKind::NewFile => {
            d.name.clear();
            DialogKind::NewFile
        }
        OpKind::Rename => {
            d.name = items[0].name.clone();
            DialogKind::Rename {
                from: items[0].name.clone(),
            }
        }
        OpKind::Delete => {
            // Folders are counted on the worker; files are known already.
            let measure = if items.iter().any(|e| e.kind == Kind::Dir) {
                measure_op(fx, id, side, &dir, &items);
                None
            } else {
                Some(Ok(items.iter().map(file_measure).collect()))
            };
            DialogKind::Delete { items, measure }
        }
        OpKind::Properties => {
            let item = items[0].clone();
            let measure = if item.kind == Kind::Dir {
                measure_op(fx, id, side, &dir, std::slice::from_ref(&item));
                None
            } else {
                Some(Ok(file_measure(&item)))
            };
            DialogKind::Properties {
                path: join(s.local_side(side), &dir, &item.name),
                item,
                measure,
            }
        }
        OpKind::Edit => return,
        OpKind::Permissions => {
            d.mode = items.iter().find_map(|e| e.perm).unwrap_or(0o644) & 0o7777;
            d.mode_text = format!("{:o}", d.mode);
            d.recursive = false;
            DialogKind::Permissions { items }
        }
    };
    s.op_errors.clear();
    s.dialog = Some(Dialog {
        side,
        dir,
        kind,
        error: None,
        busy: false,
    });
}

/// Octal mode typed by the user (`644`, `0755`, `2775`).
pub fn parse_mode(text: &str) -> Option<u32> {
    let t = text.trim();
    if t.is_empty() || t.len() > 5 {
        return None;
    }
    u32::from_str_radix(t, 8).ok().filter(|m| *m <= 0o7777)
}

fn confirm(s: &mut FileSession, d: &FileDraft, fx: &mut Vec<FmEffect>) {
    let id = s.id;
    let local_host = s.host.is_local();
    let Some(dl) = s.dialog.as_mut() else {
        return;
    };
    if dl.busy {
        return;
    }
    let local = dl.side == Side::Local || local_host;
    let name = d.name.clone();
    let name_ok = |dl: &mut Dialog| match check_name(&name, local) {
        Ok(()) => true,
        Err(e) => {
            dl.error = Some(e);
            false
        }
    };
    let dir = dl.dir.clone();
    let op = match dl.kind.clone() {
        DialogKind::NewFolder => name_ok(dl).then(|| FileOp::NewFolder {
            dir,
            name: name.clone(),
        }),
        DialogKind::NewFile => name_ok(dl).then(|| FileOp::NewFile {
            dir,
            name: name.clone(),
        }),
        DialogKind::Rename { from } => {
            if from == name {
                s.dialog = None;
                return;
            }
            name_ok(dl).then(|| FileOp::Rename {
                dir,
                from,
                to: name.clone(),
            })
        }
        DialogKind::Delete { items, .. } => Some(FileOp::Delete { dir, items }),
        DialogKind::Permissions { items } => match parse_mode(&d.mode_text) {
            Some(mode) => Some(FileOp::Chmod {
                dir,
                items,
                mode,
                recursive: d.recursive,
            }),
            None => {
                dl.error = Some("Permissions are 3 or 4 octal digits, for example 644".into());
                None
            }
        },
        DialogKind::Properties { .. } => {
            s.dialog = None;
            return;
        }
    };
    if let Some(op) = op {
        dl.error = None;
        dl.busy = true;
        fx.push(FmEffect::Op {
            id,
            side: dl.side,
            op,
        });
    }
}

fn msg_id(m: &FmMsg) -> FmId {
    use FmMsg::*;
    match m {
        Open(_) | Restore { .. } | OpenFile { .. } => 0,
        Discard(id) | Background(id) | HistoryClear(id) | HistoryExport(id) => *id,
        HistoryLoaded { id, .. } | HistoryOpenFolder { id, .. } => *id,
        Connected { id, .. }
        | Navigate { id, .. }
        | Enter { id, .. }
        | Up { id, .. }
        | Home { id, .. }
        | Refresh { id, .. }
        | Listed { id, .. }
        | Sort { id, .. }
        | Copy { id, .. }
        | Planned { id, .. }
        | Decide { id, .. }
        | Progress { id, .. }
        | Finished { id, .. }
        | Cancel { id, .. }
        | RsyncChecked { id, .. }
        | RsyncPreviewed { id, .. }
        | RsyncRun { id, .. }
        | Ask { id, .. }
        | Measured { id, .. }
        | OpDone { id, .. }
        | CopyPath { id, .. }
        | OptionsLoaded { id, .. }
        | SyncPlanned { id, .. }
        | Editor { id, .. } => *id,
        Retry(id) | Close(id) | AskCancel(id) | ClearFinished(id) | Confirm(id)
        | CloseDialog(id) | DismissErrors(id) => *id,
        OptionsOpen(id) | OptionsSave(id) | OptionsClose(id) | SyncOpen(id) | SyncPreview(id)
        | SyncRun(id) | SyncClose(id) | Diff(id) => *id,
    }
}

/// What closing this window would throw away (#99), one line each.
pub fn unsaved(s: &FileSession, d: &FileDraft) -> Vec<String> {
    let mut v = Vec::new();
    let dirty: Vec<String> = s
        .editor
        .tabs
        .iter()
        .filter(|t| d.editor.dirty(t))
        .map(|t| {
            t.path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&t.path)
                .to_string()
        })
        .collect();
    if !dirty.is_empty() {
        v.push(format!("Unsaved editor text: {}", dirty.join(", ")));
    }
    if let Some(dl) = &s.dialog {
        let what = match dl.kind {
            DialogKind::NewFolder => "New folder",
            DialogKind::NewFile => "New file",
            DialogKind::Rename { .. } => "Rename",
            DialogKind::Delete { .. } => "Delete",
            DialogKind::Properties { .. } => "Properties",
            DialogKind::Permissions { .. } => "Permissions",
        };
        v.push(format!("{what} dialog, not done"));
    }
    let waiting = s.waiting_transfers();
    if waiting > 0 {
        v.push(format!(
            "{waiting} transfer(s) waiting for your answer, cancelled"
        ));
    }
    v
}

fn log(s: &mut FileSession, fx: &mut Vec<FmEffect>, e: LogEntry) {
    s.history.push(e.clone());
    fx.push(FmEffect::Log(e));
}

fn side_label(side: Side, text: &str) -> String {
    match side {
        Side::Local => format!("{text} (this computer)"),
        Side::Remote => text.to_string(),
    }
}

fn file_entry(
    s: &FileSession,
    kind: LogKind,
    title: String,
    detail: String,
    outcome: Outcome,
    error: String,
) -> LogEntry {
    LogEntry {
        kind,
        host: s.host.alias.clone(),
        title,
        detail,
        outcome,
        error,
        method: match s.mode {
            Mode::Sftp => "SFTP".into(),
            Mode::Fallback => "sh".into(),
            Mode::Local => "local".into(),
        },
        ..Default::default()
    }
}

/// A finished transfer as a history entry (#100); `host` is set by the caller.
fn transfer_entry(t: &Transfer, mode: Mode, now: Instant) -> LogEntry {
    let dir = match t.direction {
        Direction::Upload => "Upload",
        Direction::Download => "Download",
    };
    let (outcome, error, files) = match &t.state {
        TState::Done {
            copied,
            skipped,
            failed,
        } => {
            let mut why = Vec::new();
            if *skipped > 0 {
                why.push(format!("{skipped} skipped (exists)"));
            }
            if *failed > 0 {
                why.push(format!("{failed} failed"));
            }
            why.extend(t.errors.iter().cloned());
            let outcome = if *failed > 0 {
                Outcome::Failed
            } else {
                Outcome::Done
            };
            (outcome, why.join("\n"), Some(*copied as u64))
        }
        TState::Cancelled => (Outcome::Cancelled, String::new(), None),
        TState::Failed(e) => (Outcome::Failed, e.clone(), None),
        _ => (Outcome::Done, String::new(), None),
    };
    let mut detail: Vec<String> = t.sources.iter().map(|(p, _)| p.clone()).collect();
    detail.push(format!("→ {}", t.dest));
    LogEntry {
        kind: LogKind::Transfer,
        title: if t.title.starts_with(dir) {
            t.title.clone()
        } else {
            format!("{dir} {}", t.title)
        },
        detail: detail.join("\n"),
        outcome,
        error,
        bytes: Some(t.bytes_done),
        files,
        duration_ms: t
            .started
            .map(|st| now.saturating_duration_since(st).as_millis() as u64),
        method: if t.rsync {
            "rsync".into()
        } else {
            match mode {
                Mode::Sftp => "SFTP".into(),
                Mode::Fallback => "scp -O".into(),
                Mode::Local => "local".into(),
            }
        },
        ..Default::default()
    }
}

/// Title and detail lines of a file operation dialog, for the history.
fn op_text(dl: &Dialog, local: bool, name: &str, mode: u32) -> (String, String) {
    let at = |n: &str| join(local, &dl.dir, n);
    let names = |items: &[Entry]| {
        items
            .iter()
            .map(|e| at(&e.name))
            .collect::<Vec<_>>()
            .join("\n")
    };
    match &dl.kind {
        DialogKind::NewFolder => (format!("New folder {}", name.trim()), at(name.trim())),
        DialogKind::NewFile => (format!("New file {}", name.trim()), at(name.trim())),
        DialogKind::Rename { from } => (
            format!("Rename {from} → {}", name.trim()),
            format!("{}\n→ {}", at(from), at(name.trim())),
        ),
        DialogKind::Delete { items, .. } => {
            (format!("Delete {} item(s)", items.len()), names(items))
        }
        DialogKind::Permissions { items } => (
            format!("Permissions {:o} on {} item(s)", mode, items.len()),
            names(items),
        ),
        DialogKind::Properties { .. } => (String::new(), String::new()),
    }
}

/// The History tab's lines (#100) after its filters, oldest first.
pub fn history_shown<'a>(s: &'a FileSession, d: &FileDraft) -> Vec<&'a LogEntry> {
    s.history
        .iter()
        .filter(|e| {
            crate::state::matches_query(
                &format!("{} {} {}", e.title, e.detail, e.error),
                &d.history_filter,
            )
        })
        .filter(|e| match d.history_dir {
            None => true,
            Some(Direction::Upload) => e.kind == LogKind::Transfer && e.title.starts_with("Upload"),
            Some(Direction::Download) => {
                e.kind == LogKind::Transfer && e.title.starts_with("Download")
            }
        })
        .filter(|e| !d.history_failed || e.outcome != Outcome::Done)
        .collect()
}
