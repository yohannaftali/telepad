//! The main window's workspace (#77): pages open as inner windows that can be
//! moved, resized, minimized, maximized and grouped as tabs. Pure state: the
//! view reports geometry and clicks as [`WsMsg`]s, [`update`] changes it.
//!
//! Windows tied to a session (host detail, Host Management, Public keys,
//! files, databases, editor) are added and removed by [`Workspace::sync`]
//! from the model, so they follow their session; Overview and Docker are
//! opened and closed only here.

use telepad_core::config::SetKind;

use crate::state::Model;

/// One page that can be shown in an inner window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Win {
    /// This computer (#129): the host page of the PC Telepad runs on.
    MyComputer,
    /// Every host as a table or cards.
    Overview,
    /// Every container of every host, grouped by stack (Docker / Podman).
    Docker,
    /// Kubernetes clusters found on the hosts (#84).
    Kube,
    /// Docker Swarm clusters found on the hosts (#84).
    Swarm,
    /// The selected host (`Model::selected`).
    Host,
    /// Host Management (#58).
    HostSync,
    /// Public keys (#49).
    Keys,
    /// Update Strategy (#122): what Telepad checks by itself and how often.
    Strategy,
    /// A file window session (#62).
    Files(crate::files::FmId),
    /// A database connection (#52).
    Db(crate::db::DbId),
    /// The editor of a file window session (#64).
    Editor(crate::files::FmId),
    /// A host's Log info page (#126).
    Logins(crate::logins::LogId),
    /// The Groups or Clusters page (#144, #145).
    Sets(SetKind),
    /// "Hosts - [name]" of one group / cluster.
    SetHosts(crate::sets::SetPageId),
}

impl Win {
    /// Name used in the saved layout; `None` for windows that are not
    /// reopened on start: the host detail (no host is selected on start) and
    /// live sessions (files, databases, editor).
    pub fn key(self) -> Option<&'static str> {
        match self {
            Win::MyComputer => Some("local"),
            Win::Overview => Some("overview"),
            Win::Docker => Some("docker"),
            Win::Kube => Some("kube"),
            Win::Swarm => Some("swarm"),
            Win::HostSync => Some("hostsync"),
            Win::Keys => Some("keys"),
            Win::Strategy => Some("strategy"),
            Win::Sets(SetKind::Group) => Some("groups"),
            Win::Sets(SetKind::Cluster) => Some("clusters"),
            Win::Host
            | Win::Files(_)
            | Win::Db(_)
            | Win::Editor(_)
            | Win::Logins(_)
            | Win::SetHosts(_) => None,
        }
    }

    pub fn from_key(k: &str) -> Option<Win> {
        Some(match k {
            "local" => Win::MyComputer,
            "overview" => Win::Overview,
            "docker" => Win::Docker,
            "kube" => Win::Kube,
            "swarm" => Win::Swarm,
            "hostsync" => Win::HostSync,
            "keys" => Win::Keys,
            "strategy" => Win::Strategy,
            "groups" => Win::Sets(SetKind::Group),
            "clusters" => Win::Sets(SetKind::Cluster),
            _ => return None,
        })
    }
}

/// The named workspaces (#94): pick, add, save, delete.
#[derive(Debug, Clone, PartialEq)]
pub enum WkMsg {
    Switch(telepad_core::prefs::Current),
    /// Save the current layout now (autosave does it too).
    Save,
    /// Open the "Add workspace" dialog.
    AskAdd,
    /// Create a workspace from the current layout and switch to it.
    Add(String),
    CancelAdd,
    /// Ask before deleting this custom workspace.
    AskDelete(String),
    Delete(String),
    CancelDelete,
    /// The leave question (#99): go on and drop what it listed.
    LeaveYes,
    /// The leave question: stay in the workspace, nothing changes.
    LeaveBack,
    /// Telepad is closing: ask first if something would be lost.
    AskQuit,
}

/// Position and size inside the workspace, in points: x, y, width, height.
pub type Rect = [f32; 4];

/// One inner window: one or more pages as tabs.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: u64,
    pub tabs: Vec<Win>,
    /// Index into `tabs`.
    pub active: usize,
    /// `None` until the view has placed it (cascade).
    pub rect: Option<Rect>,
    pub minimized: bool,
    pub maximized: bool,
    /// Bumped whenever the window should come to the front.
    pub raise: u64,
    /// Snapped into a layout zone (#168): the size it had before, given
    /// back when it is dragged out of the zone.
    pub unsnap: Option<Rect>,
}

impl Group {
    pub fn active_win(&self) -> Win {
        self.tabs[self.active.min(self.tabs.len() - 1)]
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspace {
    /// Oldest first.
    pub groups: Vec<Group>,
    /// The window keys go to (the one clicked or opened last).
    pub focused: Option<Win>,
    pub sidebar_collapsed: bool,
    /// Session windows of a restored layout (host, files, editor,
    /// database), placed when their session opens (#94, #99).
    pub pending: Vec<PendingWin>,
    /// Window of each restored layout line that already exists (line, id):
    /// session tabs of that line join it.
    line_groups: Vec<(usize, u64)>,
    next_id: u64,
    raise_seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WsMsg {
    /// Show this page: open it, or restore and raise its window.
    Open(Win),
    /// Close one page (Overview / Docker; the others close their session).
    Close(Win),
    /// The window of this page was clicked: keys go to it.
    Focus(Win),
    Minimize(Win),
    ToggleMaximize(Win),
    /// Show this tab in its window.
    ActivateTab(Win),
    /// Move `win` into the window that holds `into`, as a tab.
    Merge {
        win: Win,
        into: Win,
    },
    /// Take a tab out into its own window.
    Detach(Win),
    /// The window holding `win` was moved or resized.
    Geometry {
        win: Win,
        rect: Rect,
    },
    SetSidebarCollapsed(bool),
    /// Arrange (#94): the shown windows in a grid filling the workspace
    /// (`w` × `h`, from the view).
    Tile {
        w: f32,
        h: f32,
    },
    /// Arrange (#94): the shown windows cascading from the top left.
    Stack {
        w: f32,
        h: f32,
    },
    MinimizeAll,
    RestoreAll,
    /// Snap layouts (#168): put the window holding `win` into `rect` (a
    /// zone of the workspace); `from` is where it was, for [`Group::unsnap`].
    Snap {
        win: Win,
        rect: Rect,
        from: Rect,
    },
}

impl Workspace {
    pub fn group_of(&self, win: Win) -> Option<usize> {
        self.groups.iter().position(|g| g.tabs.contains(&win))
    }

    pub fn is_open(&self, win: Win) -> bool {
        self.group_of(win).is_some()
    }

    /// Every open page for the window picker (#92): windows oldest first,
    /// tabs in their order, with whether its window is minimized.
    pub fn open_windows(&self) -> Vec<(Win, bool)> {
        self.groups
            .iter()
            .flat_map(|g| g.tabs.iter().map(move |&w| (w, g.minimized)))
            .collect()
    }

    fn bump(&mut self) -> u64 {
        self.raise_seq += 1;
        self.raise_seq
    }

    fn new_group(&mut self, tabs: Vec<Win>, rect: Option<Rect>) -> usize {
        self.next_id += 1;
        let raise = self.bump();
        self.groups.push(Group {
            id: self.next_id,
            tabs,
            active: 0,
            rect,
            minimized: false,
            maximized: false,
            raise,
            unsnap: None,
        });
        self.groups.len() - 1
    }

    /// Open `win`, or bring its window back and to the front with it as the
    /// shown tab.
    pub fn open(&mut self, win: Win) {
        let gi = match self.group_of(win) {
            Some(gi) => gi,
            None => self.new_group(vec![win], None),
        };
        let raise = self.bump();
        let g = &mut self.groups[gi];
        g.active = g.tabs.iter().position(|&t| t == win).unwrap_or(0);
        g.minimized = false;
        g.raise = raise;
        self.focused = Some(win);
    }

    /// Remove `win`; its window goes when it has no tab left.
    pub fn close(&mut self, win: Win) {
        let Some(gi) = self.group_of(win) else { return };
        let g = &mut self.groups[gi];
        let at = g.tabs.iter().position(|&t| t == win).unwrap_or(0);
        g.tabs.remove(at);
        if g.tabs.is_empty() {
            self.groups.remove(gi);
        } else if g.active >= at && g.active > 0 {
            g.active -= 1;
        }
        if self.focused == Some(win) {
            self.focused = self.groups.last().map(Group::active_win);
        }
    }

    /// Add and remove the session windows so they match the model: a window
    /// opens when its session starts and goes when it ends.
    pub fn sync(&mut self, m: &Model) {
        let mut want: Vec<Win> = Vec::new();
        if m.selected.is_some() {
            want.push(Win::Host);
        }
        if m.host_sync.is_some() {
            want.push(Win::HostSync);
        }
        // A file window left with transfers running stays in the background
        // (#99): no window until its host's files are opened again.
        for s in m.files.iter().filter(|s| !s.background) {
            want.push(Win::Files(s.id));
            if !s.editor.tabs.is_empty() {
                want.push(Win::Editor(s.id));
            }
        }
        for s in &m.db {
            want.push(Win::Db(s.id));
        }
        for s in &m.logins {
            want.push(Win::Logins(s.id));
        }
        for p in &m.set_pages {
            want.push(Win::SetHosts(p.id));
        }
        let session = |w: &Win| {
            matches!(
                w,
                Win::Host
                    | Win::HostSync
                    | Win::Files(_)
                    | Win::Db(_)
                    | Win::Editor(_)
                    | Win::Logins(_)
                    | Win::SetHosts(_)
            )
        };
        let gone: Vec<Win> = self
            .groups
            .iter()
            .flat_map(|g| g.tabs.iter().copied())
            .filter(|w| session(w) && !want.contains(w))
            .collect();
        for w in gone {
            self.close(w);
        }
        for w in want {
            if self.is_open(w) {
                continue;
            }
            let pending = session_key(m, w)
                .and_then(|k| self.pending.iter().position(|p| p.key == k))
                .map(|i| self.pending.remove(i));
            match pending {
                Some(p) => self.place(w, p),
                None => self.open(w),
            }
        }
    }

    /// Keys window: follows `Drafts::keys` (opened / closed there).
    pub fn sync_keys(&mut self, open: bool) {
        match (open, self.is_open(Win::Keys)) {
            (true, false) => self.open(Win::Keys),
            (false, true) => self.close(Win::Keys),
            _ => {}
        }
    }

    /// Change the workspace; `true` when the saved layout changed.
    pub fn update(&mut self, msg: WsMsg) -> bool {
        match msg {
            WsMsg::Open(w) => self.open(w),
            WsMsg::Close(w) => self.close(w),
            WsMsg::Focus(w) => {
                if self.focused != Some(w) {
                    self.focused = Some(w);
                }
                return false;
            }
            WsMsg::Minimize(w) => {
                let Some(gi) = self.group_of(w) else {
                    return false;
                };
                self.groups[gi].minimized = true;
                if self
                    .focused
                    .is_some_and(|f| self.groups[gi].tabs.contains(&f))
                {
                    self.focused = self
                        .groups
                        .iter()
                        .rev()
                        .find(|g| !g.minimized)
                        .map(Group::active_win);
                }
            }
            WsMsg::ToggleMaximize(w) => {
                let Some(gi) = self.group_of(w) else {
                    return false;
                };
                let raise = self.bump();
                let g = &mut self.groups[gi];
                g.maximized = !g.maximized;
                g.minimized = false;
                g.raise = raise;
                self.focused = Some(w);
            }
            WsMsg::ActivateTab(w) => {
                let Some(gi) = self.group_of(w) else {
                    return false;
                };
                let g = &mut self.groups[gi];
                g.active = g.tabs.iter().position(|&t| t == w).unwrap_or(0);
                self.focused = Some(w);
            }
            WsMsg::Merge { win, into } => {
                let (Some(from), Some(to)) = (self.group_of(win), self.group_of(into)) else {
                    return false;
                };
                if from == to {
                    return false;
                }
                let id = self.groups[to].id;
                self.close(win);
                let Some(to) = self.groups.iter().position(|g| g.id == id) else {
                    return false;
                };
                let raise = self.bump();
                let g = &mut self.groups[to];
                g.tabs.push(win);
                g.active = g.tabs.len() - 1;
                g.minimized = false;
                g.raise = raise;
                self.focused = Some(win);
            }
            WsMsg::Detach(w) => {
                let Some(gi) = self.group_of(w) else {
                    return false;
                };
                if self.groups[gi].tabs.len() < 2 {
                    return false;
                }
                let rect = self.groups[gi]
                    .rect
                    .map(|[x, y, ww, h]| [x + 32.0, y + 32.0, ww, h]);
                self.close(w);
                self.new_group(vec![w], rect);
                self.focused = Some(w);
            }
            WsMsg::Geometry { win, rect } => {
                let Some(gi) = self.group_of(win) else {
                    return false;
                };
                let g = &mut self.groups[gi];
                if g.rect == Some(rect) {
                    return false;
                }
                g.rect = Some(rect);
                // Moved or resized by hand: no longer in its zone.
                g.unsnap = None;
            }
            WsMsg::Snap { win, rect, from } => {
                let Some(gi) = self.group_of(win) else {
                    return false;
                };
                let raise = self.bump();
                let g = &mut self.groups[gi];
                // Snapped again: the size from before the first snap stays.
                g.unsnap = Some(g.unsnap.unwrap_or(from));
                g.rect = Some(rect);
                g.maximized = false;
                g.minimized = false;
                g.raise = raise;
                self.focused = Some(win);
            }
            WsMsg::Tile { w, h } => {
                let order = self.shown_back_to_front();
                self.arrange(tile_rects(order.len(), w, h), order);
            }
            WsMsg::Stack { w, h } => {
                let order = self.shown_back_to_front();
                self.arrange(stack_rects(order.len(), w, h), order);
            }
            WsMsg::MinimizeAll => {
                for g in &mut self.groups {
                    g.minimized = true;
                }
                self.focused = None;
            }
            WsMsg::RestoreAll => {
                let mut order: Vec<usize> = (0..self.groups.len()).collect();
                order.sort_by_key(|&i| self.groups[i].raise);
                for gi in order {
                    let raise = self.bump();
                    let g = &mut self.groups[gi];
                    g.minimized = false;
                    g.raise = raise;
                }
                self.focus_front();
            }
            WsMsg::SetSidebarCollapsed(c) => {
                if self.sidebar_collapsed == c {
                    return false;
                }
                self.sidebar_collapsed = c;
            }
        }
        true
    }

    /// The layout of a workspace (#94, kept in `preference.db`): one line per
    /// window with at least one page that is reopened:
    /// `x,y,w,h|minimized|maximized|active|tab;tab`. The host window is
    /// `host:<alias>` (`host` = the selected host's alias); the other session
    /// windows need the model: [`layout`].
    pub fn save(&self, host: Option<&str>) -> String {
        self.save_with(&|w| match w {
            Win::Host => host.map(|a| format!("host:{}", esc(a))),
            _ => w.key().map(String::from),
        })
    }

    /// [`Workspace::save`] with `key` naming each tab (`None`: not saved).
    pub fn save_with(&self, key: &dyn Fn(Win) -> Option<String>) -> String {
        let mut lines = Vec::new();
        for g in &self.groups {
            let tabs: Vec<String> = g.tabs.iter().filter_map(|&w| key(w)).collect();
            if tabs.is_empty() {
                continue;
            }
            let active = key(g.active_win()).unwrap_or_else(|| tabs[0].clone());
            let rect = g
                .rect
                .map(|r| format!("{},{},{},{}", r[0], r[1], r[2], r[3]))
                .unwrap_or_default();
            lines.push(format!(
                "{rect}|{}|{}|{}|{}",
                u8::from(g.minimized),
                u8::from(g.maximized),
                token_key(&active),
                tabs.join(";")
            ));
        }
        lines.join("\n")
    }

    /// Restore a saved layout exactly (nothing saved: no windows). `keys` is
    /// dropped when the Authorized keys page is closed (it follows its
    /// session); session tabs (`host:`, `files:`, `editor:`, `db:`) wait in
    /// `pending` until their session opens ([`Workspace::sync`]).
    pub fn restore(&mut self, saved: &str) {
        let sidebar = self.sidebar_collapsed;
        *self = Workspace {
            sidebar_collapsed: sidebar,
            next_id: self.next_id,
            raise_seq: self.raise_seq,
            ..Default::default()
        };
        for (line_no, line) in saved.lines().enumerate() {
            let f: Vec<&str> = line.split('|').collect();
            if f.len() != 5 {
                continue;
            }
            let tabs: Vec<Win> = f[4].split(';').filter_map(Win::from_key).collect();
            let nums: Vec<f32> = f[0].split(',').filter_map(|v| v.parse().ok()).collect();
            let rect = (nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0)
                .then(|| [nums[0], nums[1], nums[2], nums[3]]);
            let (minimized, maximized) = (f[1] == "1", f[2] == "1");
            if !tabs.is_empty() {
                let gi = self.new_group(tabs, rect);
                let g = &mut self.groups[gi];
                g.minimized = minimized;
                g.maximized = maximized;
                if let Some(a) =
                    Win::from_key(f[3]).and_then(|w| g.tabs.iter().position(|&t| t == w))
                {
                    g.active = a;
                }
                let id = g.id;
                self.line_groups.push((line_no, id));
            }
            for t in f[4].split(';') {
                let key = token_key(t);
                let known = [
                    "host:", "files:", "editor:", "db:", "logins:", "group:", "cluster:",
                ]
                .iter()
                .any(|p| key.len() > p.len() && key.starts_with(p));
                if !known || self.pending.iter().any(|p| p.key == key) {
                    continue;
                }
                self.pending.push(PendingWin {
                    key: key.to_string(),
                    data: token_data(t),
                    line: line_no,
                    active: f[3] == key,
                    rect,
                    minimized,
                    maximized,
                });
            }
        }
        self.focus_front();
    }

    /// [`Workspace::restore`], or the Hosts page maximized when nothing was
    /// saved (first start).
    pub fn restore_or_default(&mut self, saved: &str) {
        self.restore(saved);
        if self.groups.is_empty() && self.pending.is_empty() {
            let gi = self.new_group(vec![Win::Overview], None);
            self.groups[gi].maximized = true;
            self.focus_front();
        }
    }

    /// Alias of the host window of a restored layout, if one waits.
    pub fn pending_host(&self) -> Option<String> {
        self.pending
            .iter()
            .find_map(|p| p.key.strip_prefix("host:").map(unesc))
    }

    /// Pending windows of one kind (`"files:"`, `"editor:"`, `"db:"`):
    /// (key, name, data).
    pub fn pending_of(&self, prefix: &str) -> Vec<(String, String, Vec<String>)> {
        self.pending
            .iter()
            .filter_map(|p| {
                p.key
                    .strip_prefix(prefix)
                    .map(|n| (p.key.clone(), unesc(n), p.data.clone()))
            })
            .collect()
    }

    /// Forget a pending window whose session will not open.
    pub fn drop_pending(&mut self, key: &str) {
        self.pending.retain(|p| p.key != key);
    }

    fn focus_front(&mut self) {
        self.focused = self
            .groups
            .iter()
            .filter(|g| !g.minimized)
            .max_by_key(|g| g.raise)
            .map(Group::active_win);
    }

    /// Put a session window of a restored layout back where it was: as a
    /// tab of the window its line already has, else in a window of its own.
    fn place(&mut self, win: Win, p: PendingWin) {
        if self.is_open(win) {
            return;
        }
        let line_group = self
            .line_groups
            .iter()
            .find(|(l, _)| *l == p.line)
            .and_then(|(_, id)| self.groups.iter().position(|g| g.id == *id));
        if let Some(gi) = line_group {
            let raise = self.bump();
            let g = &mut self.groups[gi];
            g.tabs.push(win);
            if p.active {
                g.active = g.tabs.len() - 1;
            }
            g.raise = raise;
        } else {
            let gi = self.new_group(vec![win], p.rect);
            let g = &mut self.groups[gi];
            g.minimized = p.minimized;
            g.maximized = p.maximized;
            let id = g.id;
            self.line_groups.push((p.line, id));
        }
        self.focus_front();
    }

    /// Windows that are not minimized, back to front.
    fn shown_back_to_front(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.groups.len())
            .filter(|&i| !self.groups[i].minimized)
            .collect();
        v.sort_by_key(|&i| self.groups[i].raise);
        v
    }

    fn arrange(&mut self, rects: Vec<Rect>, order: Vec<usize>) {
        for (&gi, r) in order.iter().zip(rects) {
            let raise = self.bump();
            let g = &mut self.groups[gi];
            g.rect = Some(r);
            g.maximized = false;
            g.raise = raise;
        }
    }
}

/// A session window of a restored layout, waiting for its session (#94, #99).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingWin {
    /// `host:<alias>`, `files:<alias>`, `editor:<alias>`, `db:<target key>`.
    pub key: String,
    /// What reopens it: folders, editor tabs, database target.
    pub data: Vec<String>,
    /// Its line in the layout (tabs of one line share a window).
    line: usize,
    active: bool,
    rect: Option<Rect>,
    minimized: bool,
    maximized: bool,
}

/// `%`, `,`, `;`, `|` and line breaks escaped, so any name or path fits in
/// a layout token.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => o.push_str("%25"),
            ',' => o.push_str("%2C"),
            ';' => o.push_str("%3B"),
            '|' => o.push_str("%7C"),
            '\n' => o.push_str("%0A"),
            '\r' => o.push_str("%0D"),
            _ => o.push(c),
        }
    }
    o
}

/// The other way of [`esc`]; an unknown `%` sequence stays as it is.
pub fn unesc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        o.push_str(&rest[..i]);
        let c = match rest.get(i + 1..i + 3) {
            Some("25") => Some('%'),
            Some("2C") => Some(','),
            Some("3B") => Some(';'),
            Some("7C") => Some('|'),
            Some("0A") => Some('\n'),
            Some("0D") => Some('\r'),
            _ => None,
        };
        match c {
            Some(c) => {
                o.push(c);
                rest = &rest[i + 3..];
            }
            None => {
                o.push('%');
                rest = &rest[i + 1..];
            }
        }
    }
    o.push_str(rest);
    o
}

/// A layout tab is `kind:name` or `kind:name,data,data…` (escaped).
fn token_key(t: &str) -> &str {
    t.split(',').next().unwrap_or(t)
}

fn token_data(t: &str) -> Vec<String> {
    t.split(',').skip(1).map(unesc).collect()
}

/// Name of a session window in a layout and in `pending`.
pub fn session_key(m: &Model, w: Win) -> Option<String> {
    Some(match w {
        Win::Host => format!("host:{}", esc(&m.hosts.get(m.selected?)?.entry.alias)),
        Win::Files(id) => format!(
            "files:{}",
            esc(&m.files.iter().find(|s| s.id == id)?.host.alias)
        ),
        Win::Editor(id) => format!(
            "editor:{}",
            esc(&m.files.iter().find(|s| s.id == id)?.host.alias)
        ),
        Win::Db(id) => format!(
            "db:{}",
            esc(&m.db.iter().find(|s| s.id == id)?.target.key())
        ),
        Win::Logins(id) => format!(
            "logins:{}",
            esc(&m.logins.iter().find(|s| s.id == id)?.alias)
        ),
        Win::SetHosts(id) => {
            let p = m.set_pages.iter().find(|p| p.id == id)?;
            // The fixed entry by its reserved name, so the key is never empty.
            format!(
                "{}:{}",
                p.kind.as_str(),
                esc(crate::sets::shown_name(p.kind, &p.name))
            )
        }
        _ => return None,
    })
}

/// The layout of the live workspace with every window that can come back
/// (#99): pages, the host window, file windows with their folders, editor
/// windows with their tabs (paths only, never text), database windows with
/// their target and database / schema.
pub fn layout(m: &Model) -> String {
    m.ws.save_with(&|w| {
        if let Some(k) = w.key() {
            return Some(k.to_string());
        }
        let key = session_key(m, w)?;
        let data: Vec<String> = match w {
            Win::Files(id) => {
                let s = m.files.iter().find(|s| s.id == id)?;
                let (l, r) = s.folders();
                vec![l, r]
            }
            Win::Editor(id) => {
                let s = m.files.iter().find(|s| s.id == id)?;
                s.editor
                    .tabs
                    .iter()
                    .map(|t| {
                        let side = match t.side {
                            crate::files::Side::Local => 'L',
                            crate::files::Side::Remote => 'R',
                        };
                        format!("{side}{}", t.path)
                    })
                    .collect()
            }
            Win::Db(id) => {
                let s = m.db.iter().find(|s| s.id == id)?;
                let t = &s.target;
                let (db, schema) = s.restore_point();
                vec![
                    t.engine.as_str().to_string(),
                    t.host.clone(),
                    t.container.clone(),
                    t.address.clone(),
                    t.port.to_string(),
                    db,
                    schema,
                ]
            }
            _ => Vec::new(),
        };
        let mut tok = key;
        for d in data {
            tok.push(',');
            tok.push_str(&esc(&d));
        }
        Some(tok)
    })
}

const GAP: f32 = 4.0;
const STEP: f32 = 28.0;

/// A snap zone (#168) as fractions of the workspace: x, y, width, height.
pub type Zone = [f32; 4];

/// The whole workspace: maximize.
pub const ZONE_MAX: Zone = [0.0, 0.0, 1.0, 1.0];

/// The layouts of the snap picker (#168, Windows 11 style): halves,
/// two thirds + one third, thirds, quarters.
pub const SNAP_LAYOUTS: [&[Zone]; 4] = [
    &[[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]],
    &[[0.0, 0.0, 2.0 / 3.0, 1.0], [2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0]],
    &[
        [0.0, 0.0, 1.0 / 3.0, 1.0],
        [1.0 / 3.0, 0.0, 1.0 / 3.0, 1.0],
        [2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0],
    ],
    &[
        [0.0, 0.0, 0.5, 0.5],
        [0.5, 0.0, 0.5, 0.5],
        [0.0, 0.5, 0.5, 0.5],
        [0.5, 0.5, 0.5, 0.5],
    ],
];

/// How close to an edge of the workspace the pointer snaps (#168), and how
/// far along an edge counts as a corner.
pub const SNAP_EDGE: f32 = 6.0;
pub const SNAP_CORNER: f32 = 80.0;

/// A zone of a `w` × `h` workspace in points, with a small gap around it
/// as the arranged windows have (#94); the whole workspace stays whole.
pub fn zone_rect(z: Zone, w: f32, h: f32) -> Rect {
    if z == ZONE_MAX {
        return [0.0, 0.0, w.round(), h.round()];
    }
    let (x0, y0) = (z[0] * w, z[1] * h);
    let (x1, y1) = ((z[0] + z[2]) * w, (z[1] + z[3]) * h);
    [
        (x0 + GAP / 2.0).round(),
        (y0 + GAP / 2.0).round(),
        (x1 - x0 - GAP).max(1.0).round(),
        (y1 - y0 - GAP).max(1.0).round(),
    ]
}

/// The zone a window dragged with the pointer at (`x`, `y`) of a `w` × `h`
/// workspace snaps to (#168): a corner gives a quarter, the left / right
/// edge a half, the top edge the whole workspace (maximize).
pub fn edge_zone(x: f32, y: f32, w: f32, h: f32) -> Option<Zone> {
    let left = x <= SNAP_EDGE;
    let right = x >= w - SNAP_EDGE;
    let top = y <= SNAP_EDGE;
    let bottom = y >= h - SNAP_EDGE;
    let near_top = y <= SNAP_CORNER;
    let near_bottom = y >= h - SNAP_CORNER;
    let near_left = x <= SNAP_CORNER;
    let near_right = x >= w - SNAP_CORNER;
    let quarter = |l: bool, t: bool| -> Zone {
        [
            if l { 0.0 } else { 0.5 },
            if t { 0.0 } else { 0.5 },
            0.5,
            0.5,
        ]
    };
    Some(if (left && near_top) || (top && near_left) {
        quarter(true, true)
    } else if (right && near_top) || (top && near_right) {
        quarter(false, true)
    } else if (left && near_bottom) || (bottom && near_left) {
        quarter(true, false)
    } else if (right && near_bottom) || (bottom && near_right) {
        quarter(false, false)
    } else if left {
        SNAP_LAYOUTS[0][0]
    } else if right {
        SNAP_LAYOUTS[0][1]
    } else if top {
        ZONE_MAX
    } else {
        return None;
    })
}

/// Tile (#94): `n` windows in a grid filling a `w` × `h` workspace, rows
/// top to bottom; a last, shorter row is stretched to the full width.
pub fn tile_rects(n: usize, w: f32, h: f32) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let (w, h) = (w.max(1.0), h.max(1.0));
    let cols = (n as f32).sqrt().ceil() as usize;
    let rows = n.div_ceil(cols);
    let ch = h / rows as f32;
    let mut out = Vec::with_capacity(n);
    for r in 0..rows {
        let in_row = if r == rows - 1 {
            n - cols * (rows - 1)
        } else {
            cols
        };
        let cw = w / in_row as f32;
        for c in 0..in_row {
            out.push([
                (c as f32 * cw + GAP / 2.0).round(),
                (r as f32 * ch + GAP / 2.0).round(),
                (cw - GAP).max(1.0).round(),
                (ch - GAP).max(1.0).round(),
            ]);
        }
    }
    out
}

/// Stack (#94): `n` windows cascading from the top left, one title bar
/// apart, all the same size; the last one is in front.
pub fn stack_rects(n: usize, w: f32, h: f32) -> Vec<Rect> {
    let off = STEP * n.saturating_sub(1) as f32;
    let size_w = (w - off - GAP * 2.0).max(w * 0.5).max(1.0).round();
    let size_h = (h - off - GAP * 2.0).max(h * 0.5).max(1.0).round();
    (0..n)
        .map(|i| {
            let k = STEP * i as f32;
            [
                (GAP + k).min((w - size_w).max(0.0)).round(),
                (GAP + k).min((h - size_h).max(0.0)).round(),
                size_w,
                size_h,
            ]
        })
        .collect()
}
