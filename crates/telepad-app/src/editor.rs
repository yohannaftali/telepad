//! The editor of a file window (#64): one window per host with a tab per
//! file (this computer and the host together), unsaved state, save with a
//! changed-on-disk check (Overwrite / Reload / Show diff) and the
//! unsaved-changes question when a tab or window closes. Pure state; the
//! runtime loads and saves, the GUI holds the cursor and undo history.

use std::collections::HashMap;

use telepad_core::files::edit::{SaveError, Stamp};
use telepad_core::files::syntax::Lang;
use telepad_core::files::text::{
    self, Charset, Doc, FindOpts, Format, Indent, MAX_EDIT, convert_indent,
};
use telepad_core::files::{Entry, Kind, local_join, sftp};

use crate::files::{FileDraft, FileSession, FmEffect, Mode, Side, Status};

pub type TabId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabState {
    Loading,
    Ready,
    /// Could not be opened (binary, too big, not UTF-8, gone…).
    Failed(String),
}

/// The file changed on disk since it was loaded: the question shown on Save.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    /// What it looks like now (`None`: deleted).
    pub now: Option<Stamp>,
    pub diff_loading: bool,
    /// Lines of (`' '` / `'-'` on disk / `'+'` yours, text).
    pub diff: Option<Result<Vec<(char, String)>, String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdTab {
    pub id: TabId,
    pub side: Side,
    /// Full path (local or remote).
    pub path: String,
    pub name: String,
    pub lang: Lang,
    /// What Tab inserts (#182): detected on load, changed from the status bar.
    pub indent: Indent,
    pub state: TabState,
    pub crlf: bool,
    pub mixed: bool,
    pub bom: bool,
    /// The encoding it is saved with (#203).
    pub charset: Charset,
    /// Not UTF-8: read as [`text::GUESS`] until the user picks one (#203).
    pub guessed: bool,
    /// Line ending and encoding as last loaded or saved: a different
    /// [`EdTab::format`] is an unsaved change.
    pub saved_format: Format,
    /// Format being written now.
    pub saving_format: Option<Format>,
    pub stamp: Option<Stamp>,
    /// The file's text as last loaded or saved (unsaved = draft differs).
    pub saved: String,
    /// Text being written now.
    pub saving: Option<String>,
    /// Why the last save or reload failed.
    pub error: Option<String>,
    pub conflict: Option<Conflict>,
    /// Close once the save finished (from the unsaved-changes question).
    pub close_after_save: bool,
}

impl EdTab {
    pub fn new(id: TabId, side: Side, path: String, name: String, state: TabState) -> EdTab {
        EdTab {
            id,
            side,
            lang: Lang::detect(&path, ""),
            indent: Indent::default(),
            path,
            name,
            state,
            crlf: false,
            mixed: false,
            bom: false,
            charset: Charset::Utf8,
            guessed: false,
            saved_format: Format::default(),
            saving_format: None,
            stamp: None,
            saved: String::new(),
            saving: None,
            error: None,
            conflict: None,
            close_after_save: false,
        }
    }

    /// What Save writes: line ending, encoding, byte order mark (#203).
    pub fn format(&self) -> Format {
        Format {
            crlf: self.crlf,
            charset: self.charset,
            bom: self.bom,
        }
    }
}

/// The unsaved-changes question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsaved {
    Tab(TabId),
    /// Every tab: the editor window closes, or (`window`) the file window too.
    All {
        window: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileEditor {
    pub tabs: Vec<EdTab>,
    pub active: Option<TabId>,
    pub ask: Option<Unsaved>,
    /// Close the file window once every save finished.
    pub close_window_after_save: bool,
    /// Two tabs shown side by side with their differing lines marked
    /// (Diff of the file window, #131): (left pane's file, right pane's).
    pub compare: Option<(TabId, TabId)>,
    /// Reopen with Encoding of a tab with unsaved changes asks first (#203).
    pub reopen_ask: Option<(TabId, Charset)>,
    /// Open… (#204): a folder of the same side, listed without moving the
    /// file window's panes.
    pub open: Option<OpenDialog>,
    next: TabId,
}

/// The editor's Open… dialog (#204).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenDialog {
    /// The side of the file window it lists (the host, or this computer).
    pub side: Side,
    /// Folder shown (absolute once listed).
    pub dir: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub error: Option<String>,
}

impl FileEditor {
    pub fn tab(&self, id: TabId) -> Option<&EdTab> {
        self.tabs.iter().find(|t| t.id == id)
    }
    fn tab_mut(&mut self, id: TabId) -> Option<&mut EdTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
}

/// Which find bar is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bar {
    #[default]
    None,
    /// Ctrl+F
    Find,
    /// Ctrl+H
    Replace,
    /// Ctrl+G
    Goto,
}

/// What the editor window binds to: the text of each tab and the find bar.
#[derive(Debug, Clone, Default)]
pub struct EdDraft {
    pub texts: HashMap<TabId, String>,
    pub bar: Bar,
    pub find: String,
    pub replace: String,
    pub goto: String,
    pub opts: FindOpts,
    /// Compare (#135): the two sides scroll on their own.
    pub no_sync_scroll: bool,
    /// Open… (#204): the typed path, the name filter and the selected row.
    pub open_path: String,
    pub open_filter: String,
    pub open_sel: Option<String>,
}

impl EdDraft {
    pub fn dirty(&self, t: &EdTab) -> bool {
        t.state == TabState::Ready
            && (self.texts.get(&t.id).is_some_and(|x| *x != t.saved)
                || t.format() != t.saved_format)
    }
}

/// Some tab has changes that are not saved.
pub fn has_unsaved(e: &FileEditor, d: &EdDraft) -> bool {
    e.tabs.iter().any(|t| d.dirty(t))
}

/// Why a load was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Load {
    Open,
    /// Replace the text with the file's (changed on disk, Reload).
    Reload,
    /// Only to compare with the text (Show diff).
    Diff,
}

#[derive(Debug, Clone)]
pub enum EdMsg {
    /// Double-click / Edit / F4 on a file of a pane.
    Open {
        side: Side,
        name: String,
    },
    /// Open a file by its full path (not picked in a pane; #85).
    OpenPath {
        side: Side,
        path: String,
    },
    Loaded {
        tab: TabId,
        load: Load,
        result: Result<(Doc, Stamp), String>,
    },
    Activate(TabId),
    /// Tab close button (asks first when unsaved).
    Close(TabId),
    /// Ctrl+S
    Save(TabId),
    Saved {
        tab: TabId,
        result: Result<Stamp, SaveError>,
    },
    /// Changed on disk: write anyway / take the file's text / compare.
    Overwrite(TabId),
    Reload(TabId),
    ShowDiff(TabId),
    KeepEditing(TabId),
    /// The answer to [`FileEditor::ask`].
    Answer(Answer),
    /// The editor window's close button.
    CloseWindow,
    SetLang(TabId, Lang),
    /// Indent using tabs / spaces of a size (#182): only what Tab inserts.
    SetIndent(TabId, Indent),
    /// Rewrite the indentation of the text (only the lines of `sel` when
    /// given) to `to` (#182). Not saved until Save.
    ConvertIndent {
        tab: TabId,
        to: Indent,
        sel: Option<std::ops::Range<usize>>,
    },
    /// Leave the side-by-side compare (#131); both tabs stay open.
    EndCompare,
    /// Select End of Line Sequence (#203): `true` = CRLF. Not saved until Save.
    SetEol(TabId, bool),
    /// Save with Encoding (#203): the encoding (and byte order mark) the
    /// next Save writes.
    SaveWithEncoding(TabId, Charset, bool),
    /// Reopen with Encoding (#203): read the file again as `Charset`; asks
    /// first when the tab has unsaved changes.
    ReopenWithEncoding(TabId, Charset),
    /// The answer to [`FileEditor::reopen_ask`]: `true` = discard and reopen.
    ReopenAnswer(bool),
    /// Open… / Ctrl+O (#204): list the active file's folder (else the
    /// file window's folder) on the same side.
    ShowOpen,
    /// List another folder in the Open… dialog (typed path, `..`, a folder).
    Browse(String),
    /// The Open… dialog's listing.
    Browsed(Result<(String, Vec<Entry>), String>),
    /// Double-click / Open on a row: a folder is listed, a file opens.
    PickOpen(String),
    CloseOpen,
}

/// `true` when the file window should close now (after the question).
pub(crate) fn update(
    s: &mut FileSession,
    d: &mut FileDraft,
    msg: EdMsg,
    fx: &mut Vec<FmEffect>,
) -> bool {
    let id = s.id;
    if let EdMsg::Close(t) = &msg
        && s.editor.compare.is_some_and(|(a, b)| a == *t || b == *t)
    {
        s.editor.compare = None;
    }
    let e = &mut s.editor;
    let dr = &mut d.editor;
    match msg {
        EdMsg::EndCompare => e.compare = None,
        EdMsg::Open { side, name } => {
            if s.status != Status::Ready || (side == Side::Remote && s.mode == Mode::Fallback) {
                return false;
            }
            let pane = s.pane(side);
            let Some(entry) = pane.entries.iter().find(|x| x.name == name) else {
                return false;
            };
            if entry.is_dir() {
                return false;
            }
            let (kind, size) = (entry.kind, entry.size);
            let path = if s.local_side(side) {
                local_join(&pane.path, &name)
            } else {
                sftp::join(&pane.path, &name)
            };
            let e = &mut s.editor;
            if let Some(t) = e.tabs.iter().find(|t| t.side == side && t.path == path) {
                e.active = Some(t.id);
                return false;
            }
            e.next += 1;
            let tab = e.next;
            // A link's size is the link's own; the runtime checks the target.
            let state = if kind == Kind::File && size > MAX_EDIT {
                TabState::Failed(text::too_big(size))
            } else {
                fx.push(FmEffect::Load {
                    id,
                    tab,
                    side,
                    path: path.clone(),
                    load: Load::Open,
                    charset: None,
                });
                TabState::Loading
            };
            e.tabs.push(EdTab::new(tab, side, path, name, state));
            e.active = Some(tab);
        }
        EdMsg::OpenPath { side, path } => {
            if s.status != Status::Ready || (side == Side::Remote && s.mode == Mode::Fallback) {
                return false;
            }
            let e = &mut s.editor;
            if let Some(t) = e.tabs.iter().find(|t| t.side == side && t.path == path) {
                e.active = Some(t.id);
                return false;
            }
            e.next += 1;
            let tab = e.next;
            let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
            fx.push(FmEffect::Load {
                id,
                tab,
                side,
                path: path.clone(),
                load: Load::Open,
                charset: None,
            });
            e.tabs
                .push(EdTab::new(tab, side, path, name, TabState::Loading));
            e.active = Some(tab);
        }
        EdMsg::Loaded { tab, load, result } => {
            let mine = dr.texts.get(&tab).cloned().unwrap_or_default();
            let Some(t) = e.tab_mut(tab) else {
                return false;
            };
            match (load, result) {
                (Load::Open | Load::Reload, Ok((doc, stamp))) => {
                    if load == Load::Open && t.lang == Lang::Plain {
                        t.lang = Lang::detect(&t.path, doc.text.lines().next().unwrap_or(""));
                    }
                    t.state = TabState::Ready;
                    t.indent = Indent::detect(&doc.text);
                    t.crlf = doc.crlf;
                    t.mixed = doc.mixed;
                    t.bom = doc.bom;
                    t.charset = doc.charset;
                    t.guessed = doc.guessed;
                    t.saved_format = t.format();
                    t.stamp = Some(stamp);
                    t.saved = doc.text.clone();
                    t.error = None;
                    t.conflict = None;
                    dr.texts.insert(tab, doc.text);
                }
                (Load::Open, Err(why)) => t.state = TabState::Failed(why),
                (Load::Reload, Err(why)) => t.error = Some(why),
                (Load::Diff, result) => {
                    if let Some(c) = t.conflict.as_mut() {
                        c.diff_loading = false;
                        c.diff = Some(result.and_then(|(doc, _)| diff(&doc.text, &mine)));
                    }
                }
            }
        }
        EdMsg::Activate(tab) => {
            if e.tab(tab).is_some() {
                e.active = Some(tab);
            }
        }
        EdMsg::Close(tab) => {
            let Some(t) = e.tab(tab) else {
                return false;
            };
            if dr.dirty(t) {
                e.active = Some(tab);
                e.ask = Some(Unsaved::Tab(tab));
            } else {
                remove(e, dr, tab);
            }
        }
        EdMsg::Save(tab) => save(id, e, dr, tab, true, fx),
        EdMsg::Saved { tab, result } => {
            let Some(t) = e.tab_mut(tab) else {
                return false;
            };
            let written = t.saving.take();
            let format = t.saving_format.take();
            match result {
                Ok(stamp) => {
                    t.saved = written.unwrap_or_default();
                    if let Some(f) = format {
                        t.saved_format = f;
                        t.guessed = false;
                    }
                    t.stamp = Some(stamp);
                    t.error = None;
                    if t.close_after_save {
                        remove(e, dr, tab);
                    }
                    if e.close_window_after_save && e.tabs.iter().all(|t| t.saving.is_none()) {
                        e.close_window_after_save = false;
                        if e.tabs.iter().all(|t| !dr.dirty(t)) {
                            return true;
                        }
                    }
                }
                Err(err) => {
                    t.close_after_save = false;
                    match err {
                        SaveError::Changed(now) => {
                            t.conflict = Some(Conflict {
                                now,
                                diff_loading: false,
                                diff: None,
                            })
                        }
                        SaveError::Failed(why) => t.error = Some(why),
                    }
                    e.close_window_after_save = false;
                    e.active = Some(tab);
                }
            }
        }
        EdMsg::Overwrite(tab) => {
            if let Some(t) = e.tab_mut(tab) {
                t.conflict = None;
            }
            save(id, e, dr, tab, false, fx);
        }
        EdMsg::Reload(tab) => {
            if let Some(t) = e.tab_mut(tab) {
                t.conflict = None;
                fx.push(FmEffect::Load {
                    id,
                    tab,
                    side: t.side,
                    path: t.path.clone(),
                    load: Load::Reload,
                    charset: reread_as(t),
                });
            }
        }
        EdMsg::ShowDiff(tab) => {
            if let Some(t) = e.tab_mut(tab)
                && let Some(c) = t.conflict.as_mut()
                && !c.diff_loading
            {
                c.diff_loading = true;
                c.diff = None;
                fx.push(FmEffect::Load {
                    id,
                    tab,
                    side: t.side,
                    path: t.path.clone(),
                    load: Load::Diff,
                    charset: reread_as(t),
                });
            }
        }
        EdMsg::KeepEditing(tab) => {
            if let Some(t) = e.tab_mut(tab) {
                t.conflict = None;
            }
        }
        EdMsg::Answer(a) => {
            let Some(q) = e.ask.take() else {
                return false;
            };
            match (q, a) {
                (_, Answer::Cancel) => {}
                (Unsaved::Tab(tab), Answer::Save) => {
                    if let Some(t) = e.tab_mut(tab) {
                        t.close_after_save = true;
                    }
                    save(id, e, dr, tab, true, fx);
                }
                (Unsaved::Tab(tab), Answer::Discard) => remove(e, dr, tab),
                (Unsaved::All { window }, Answer::Save) => {
                    let ids: Vec<TabId> = e.tabs.iter().map(|t| t.id).collect();
                    for tab in ids {
                        let dirty = e.tab(tab).is_some_and(|t| dr.dirty(t));
                        if dirty {
                            if let Some(t) = e.tab_mut(tab) {
                                t.close_after_save = true;
                            }
                            save(id, e, dr, tab, true, fx);
                        } else {
                            remove(e, dr, tab);
                        }
                    }
                    if e.tabs.iter().any(|t| t.saving.is_some()) {
                        e.close_window_after_save = window;
                    } else {
                        return window;
                    }
                }
                (Unsaved::All { window }, Answer::Discard) => {
                    e.tabs.clear();
                    e.active = None;
                    dr.texts.clear();
                    return window;
                }
            }
        }
        EdMsg::CloseWindow => {
            if has_unsaved(e, dr) {
                e.ask = Some(Unsaved::All { window: false });
            } else {
                e.tabs.clear();
                e.active = None;
                dr.texts.clear();
            }
        }
        EdMsg::SetLang(tab, lang) => {
            if let Some(t) = e.tab_mut(tab) {
                t.lang = lang;
            }
        }
        EdMsg::SetIndent(tab, to) => {
            if let Some(t) = e.tab_mut(tab)
                && indent_allowed(t.lang, to)
            {
                t.indent = to;
            }
        }
        EdMsg::ConvertIndent { tab, to, sel } => {
            if let Some(t) = e.tab_mut(tab)
                && t.state == TabState::Ready
                && indent_allowed(t.lang, to)
                && let Some(text) = dr.texts.get_mut(&tab)
            {
                *text = convert_indent(text, sel, t.indent, to);
                t.indent = to;
            }
        }
        EdMsg::SetEol(tab, crlf) => {
            if let Some(t) = e.tab_mut(tab)
                && t.state == TabState::Ready
            {
                t.crlf = crlf;
                t.mixed = false;
            }
        }
        EdMsg::SaveWithEncoding(tab, charset, bom) => {
            if let Some(t) = e.tab_mut(tab)
                && t.state == TabState::Ready
            {
                t.charset = charset;
                t.bom = bom && charset.bom().is_some();
                t.error = None;
            }
        }
        EdMsg::ReopenWithEncoding(tab, charset) => {
            let Some(t) = e.tab(tab) else {
                return false;
            };
            if t.saving.is_some() || t.state == TabState::Loading {
                return false;
            }
            if dr.dirty(t) {
                e.active = Some(tab);
                e.reopen_ask = Some((tab, charset));
            } else {
                reopen(id, e, tab, charset, fx);
            }
        }
        EdMsg::ReopenAnswer(discard) => {
            if let Some((tab, charset)) = e.reopen_ask.take()
                && discard
            {
                reopen(id, e, tab, charset, fx);
            }
        }
        EdMsg::ShowOpen => {
            if s.status != Status::Ready {
                return false;
            }
            let (side, dir) = match s.editor.active.and_then(|a| s.editor.tab(a)) {
                Some(t) => {
                    let local = s.local_side(t.side);
                    let dir =
                        crate::files::parent(local, &t.path).unwrap_or_else(|| t.path.clone());
                    (t.side, dir)
                }
                None => (Side::Remote, s.remote.path.clone()),
            };
            let error = (side == Side::Remote && s.mode == Mode::Fallback)
                .then(|| "Editing needs SFTP on the host".to_string());
            d.editor.open_path = dir.clone();
            d.editor.open_filter.clear();
            d.editor.open_sel = None;
            let loading = error.is_none();
            if loading {
                fx.push(FmEffect::Browse {
                    id,
                    side,
                    path: dir.clone(),
                });
            }
            s.editor.open = Some(OpenDialog {
                side,
                dir,
                entries: Vec::new(),
                loading,
                error,
            });
        }
        EdMsg::Browse(path) => {
            let Some(o) = e.open.as_mut() else {
                return false;
            };
            let path = path.trim().to_string();
            if path.is_empty() || o.loading {
                return false;
            }
            o.loading = true;
            o.error = None;
            dr.open_sel = None;
            fx.push(FmEffect::Browse {
                id,
                side: o.side,
                path,
            });
        }
        EdMsg::Browsed(result) => {
            let Some(o) = e.open.as_mut() else {
                return false;
            };
            o.loading = false;
            match result {
                Ok((dir, mut entries)) => {
                    entries.sort_by(|a, b| {
                        b.is_dir()
                            .cmp(&a.is_dir())
                            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                    });
                    dr.open_path = dir.clone();
                    dr.open_filter.clear();
                    o.dir = dir;
                    o.entries = entries;
                    o.error = None;
                }
                Err(why) => o.error = Some(why),
            }
        }
        EdMsg::PickOpen(name) => {
            let Some(side) = s.editor.open.as_ref().map(|o| o.side) else {
                return false;
            };
            let local = s.local_side(side);
            let Some(o) = s.editor.open.as_ref() else {
                return false;
            };
            if name == ".." {
                if let Some(up) = crate::files::parent(local, &o.dir) {
                    return update(s, d, EdMsg::Browse(up), fx);
                }
                return false;
            }
            let Some(entry) = o.entries.iter().find(|x| x.name == name) else {
                return false;
            };
            let path = crate::files::join(local, &o.dir, &name);
            if entry.is_dir() {
                return update(s, d, EdMsg::Browse(path), fx);
            }
            s.editor.open = None;
            return update(s, d, EdMsg::OpenPath { side, path }, fx);
        }
        EdMsg::CloseOpen => e.open = None,
    }
    false
}

/// The encoding to read a tab's file with again (Reload, Show diff): the one
/// it is shown in, unless that was only a guess.
fn reread_as(t: &EdTab) -> Option<Charset> {
    (t.state == TabState::Ready && !t.guessed).then_some(t.charset)
}

/// Read the file again as `charset`, replacing the tab's text (#203).
fn reopen(id: u64, e: &mut FileEditor, tab: TabId, charset: Charset, fx: &mut Vec<FmEffect>) {
    let Some(t) = e.tab_mut(tab) else {
        return;
    };
    t.conflict = None;
    t.error = None;
    let load = if matches!(t.state, TabState::Failed(_)) {
        t.state = TabState::Loading;
        Load::Open
    } else {
        Load::Reload
    };
    fx.push(FmEffect::Load {
        id,
        tab,
        side: t.side,
        path: t.path.clone(),
        load,
        charset: Some(charset),
    });
}

/// Tabs are not valid YAML indentation (#182).
pub fn indent_allowed(lang: Lang, to: Indent) -> bool {
    !(to.tabs && lang == Lang::Yaml)
}

/// Write the tab's text (`check`: only if the file is still as loaded).
fn save(
    id: u64,
    e: &mut FileEditor,
    dr: &EdDraft,
    tab: TabId,
    check: bool,
    fx: &mut Vec<FmEffect>,
) {
    let Some(t) = e.tab_mut(tab) else {
        return;
    };
    if t.state != TabState::Ready || t.saving.is_some() {
        return;
    }
    let Some(text) = dr.texts.get(&tab) else {
        return;
    };
    // An unchanged file is not rewritten (unless overwriting on purpose).
    if check && *text == t.saved && t.format() == t.saved_format {
        if t.close_after_save {
            t.close_after_save = false;
        }
        return;
    }
    let bytes = match text::encode_as(text, t.format()) {
        Ok(b) => b,
        Err(why) => {
            t.error = Some(why);
            t.close_after_save = false;
            e.close_window_after_save = false;
            return;
        }
    };
    t.error = None;
    t.saving = Some(text.clone());
    t.saving_format = Some(t.format());
    fx.push(FmEffect::Save {
        id,
        tab,
        side: t.side,
        path: t.path.clone(),
        bytes,
        expect: if check { t.stamp } else { None },
    });
}

fn remove(e: &mut FileEditor, dr: &mut EdDraft, tab: TabId) {
    let Some(pos) = e.tabs.iter().position(|t| t.id == tab) else {
        return;
    };
    e.tabs.remove(pos);
    dr.texts.remove(&tab);
    if e.ask == Some(Unsaved::Tab(tab)) {
        e.ask = None;
    }
    if e.active == Some(tab) {
        e.active = e
            .tabs
            .get(pos.min(e.tabs.len().saturating_sub(1)))
            .map(|t| t.id);
    }
}

/// Line diff of the file on disk and the editor's text (refused when huge).
fn diff(disk: &str, mine: &str) -> Result<Vec<(char, String)>, String> {
    let (a, b) = (text::line_count(disk), text::line_count(mine));
    if a.saturating_mul(b) > 25_000_000 {
        return Err(format!(
            "Too large to compare here ({a} and {b} lines): Reload or Overwrite"
        ));
    }
    Ok(telepad_core::hostsync::line_diff(disk, mine))
}
