//! The main window's workspace (#77): pages open as inner windows that can be
//! moved, resized, minimized, maximized and grouped as tabs. Pure state: the
//! view reports geometry and clicks as [`WsMsg`]s, [`update`] changes it.
//!
//! Windows tied to a session (host detail, Host Management, Public keys,
//! files, databases, editor) are added and removed by [`Workspace::sync`]
//! from the model, so they follow their session; Overview and Docker are
//! opened and closed only here.

use crate::state::Model;

/// One page that can be shown in an inner window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Win {
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
    /// A file window session (#62).
    Files(crate::files::FmId),
    /// A database connection (#52).
    Db(crate::db::DbId),
    /// The editor of a file window session (#64).
    Editor(crate::files::FmId),
}

impl Win {
    /// Name used in the saved layout; `None` for windows that are not
    /// reopened on start: the host detail (no host is selected on start) and
    /// live sessions (files, databases, editor).
    pub fn key(self) -> Option<&'static str> {
        match self {
            Win::Overview => Some("overview"),
            Win::Docker => Some("docker"),
            Win::Kube => Some("kube"),
            Win::Swarm => Some("swarm"),
            Win::HostSync => Some("hostsync"),
            Win::Keys => Some("keys"),
            Win::Host | Win::Files(_) | Win::Db(_) | Win::Editor(_) => None,
        }
    }

    pub fn from_key(k: &str) -> Option<Win> {
        Some(match k {
            "overview" => Win::Overview,
            "docker" => Win::Docker,
            "kube" => Win::Kube,
            "swarm" => Win::Swarm,
            "hostsync" => Win::HostSync,
            "keys" => Win::Keys,
            _ => return None,
        })
    }
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
}

impl Workspace {
    pub fn group_of(&self, win: Win) -> Option<usize> {
        self.groups.iter().position(|g| g.tabs.contains(&win))
    }

    pub fn is_open(&self, win: Win) -> bool {
        self.group_of(win).is_some()
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
        for s in &m.files {
            want.push(Win::Files(s.id));
            if !s.editor.tabs.is_empty() {
                want.push(Win::Editor(s.id));
            }
        }
        for s in &m.db {
            want.push(Win::Db(s.id));
        }
        let session = |w: &Win| {
            matches!(
                w,
                Win::Host | Win::HostSync | Win::Files(_) | Win::Db(_) | Win::Editor(_)
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
            if !self.is_open(w) {
                self.open(w);
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

    /// The layout saved in `settings` (`ui_workspace`): one line per window
    /// with at least one page that is reopened on start:
    /// `x,y,w,h|minimized|maximized|active|tab;tab`.
    pub fn save(&self) -> String {
        let mut lines = Vec::new();
        for g in &self.groups {
            let tabs: Vec<&str> = g.tabs.iter().filter_map(|w| w.key()).collect();
            if tabs.is_empty() {
                continue;
            }
            let active = g.active_win().key().unwrap_or(tabs[0]);
            let rect = g
                .rect
                .map(|r| format!("{},{},{},{}", r[0], r[1], r[2], r[3]))
                .unwrap_or_default();
            lines.push(format!(
                "{rect}|{}|{}|{active}|{}",
                u8::from(g.minimized),
                u8::from(g.maximized),
                tabs.join(";")
            ));
        }
        lines.join("\n")
    }

    /// Restore a saved layout. `host` is dropped when no host is selected,
    /// `keys` when the Public keys window is closed (they follow their
    /// session). Empty or unreadable: Overview, maximized.
    pub fn restore(&mut self, saved: &str) {
        self.groups.clear();
        self.focused = None;
        for line in saved.lines() {
            let f: Vec<&str> = line.split('|').collect();
            if f.len() != 5 {
                continue;
            }
            let tabs: Vec<Win> = f[4]
                .split(';')
                .filter_map(Win::from_key)
                .filter(|w| {
                    matches!(
                        w,
                        Win::Overview
                            | Win::Docker
                            | Win::Kube
                            | Win::Swarm
                            | Win::HostSync
                            | Win::Keys
                    )
                })
                .collect();
            if tabs.is_empty() {
                continue;
            }
            let nums: Vec<f32> = f[0].split(',').filter_map(|v| v.parse().ok()).collect();
            let rect = (nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0)
                .then(|| [nums[0], nums[1], nums[2], nums[3]]);
            let gi = self.new_group(tabs, rect);
            let g = &mut self.groups[gi];
            g.minimized = f[1] == "1";
            g.maximized = f[2] == "1";
            if let Some(a) = Win::from_key(f[3]).and_then(|w| g.tabs.iter().position(|&t| t == w)) {
                g.active = a;
            }
        }
        if self.groups.is_empty() {
            let gi = self.new_group(vec![Win::Overview], None);
            self.groups[gi].maximized = true;
        }
        self.focused = self
            .groups
            .iter()
            .rev()
            .find(|g| !g.minimized)
            .map(Group::active_win);
    }
}
