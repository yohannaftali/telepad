//! eframe/egui desktop UI. Reads snapshots only; never blocks on ssh.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::icons::{self, label as il};
use telepad_core::collector::{self, Collector, SshSettings, Update};
use telepad_core::config::{Access, HostEntry, Inventory};
use telepad_core::model::{
    Container, HostReport, Manager, PortainerRole, SensorKind, human_kb, human_uptime,
};
use telepad_core::store::{self, ImportMode, ImportSummary};
use telepad_core::update::{self, Check, UpdateConfig, Updater};
use telepad_core::{Workspace, open_workspace, platform};

const WORKERS: usize = 12;

pub fn run(cli_config: Option<String>, cli_db: Option<String>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Telepad")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Telepad",
        options,
        Box::new(move |cc| Ok(Box::new(MonitorApp::new(cc, cli_config, cli_db)))),
    )
}

#[derive(Default)]
struct HostState {
    entry: HostEntry,
    report: Option<HostReport>,
    error: Option<String>,
    loading: bool,
    updated: Option<Instant>,
    elapsed: Option<Duration>,
}

impl HostState {
    fn online(&self) -> Option<bool> {
        match (&self.error, &self.report) {
            (Some(_), _) => Some(false),
            (None, Some(_)) => Some(true),
            _ => None,
        }
    }
}

/// Add/edit form. Text fields are kept as typed and validated on save.
struct Editor {
    entry: HostEntry,
    port: String,
    identity: String,
    options: String,
    monitored: bool,
    access: Access,
    error: Option<String>,
    confirm_delete: bool,
    cloned_from: Option<String>,
}

impl Editor {
    /// New host with the connection settings (User, Port, IdentityFile) of `src`.
    fn clone_of(src: &HostEntry) -> Self {
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

    fn new(entry: HostEntry) -> Self {
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
        }
    }

    fn build(&self) -> Result<HostEntry, String> {
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

struct ImportDialog {
    path: PathBuf,
    inventory: Inventory,
    mode: ImportMode,
    preview: Result<ImportSummary, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UpdatePhase {
    Idle,
    Checking,
    Installing,
    /// New exes are in place; a restart starts them.
    Installed(String),
}

/// Self-update state. Checks and downloads run on background threads.
struct UpdateUi {
    updater: Option<Updater>,
    tx: Sender<update::Msg>,
    rx: Receiver<update::Msg>,
    ctx: Option<egui::Context>,
    phase: UpdatePhase,
    last_check: Option<Instant>,
    /// Result of the last successful check.
    check: Option<Check>,
    error: Option<String>,
    /// "Later" hides the banner until the next start.
    later: bool,
    window: bool,
    /// The last check was started by the user: report "up to date" too.
    manual: bool,
    form: UpdateConfig,
    form_error: Option<String>,
}

impl Default for UpdateUi {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            updater: None,
            tx,
            rx,
            ctx: None,
            phase: UpdatePhase::Idle,
            last_check: None,
            check: None,
            error: None,
            later: false,
            window: false,
            manual: false,
            form: UpdateConfig::default(),
            form_error: None,
        }
    }
}

impl UpdateUi {
    fn configure(&mut self, config: UpdateConfig) {
        self.form = config.clone();
        self.updater = Some(Updater::new(config));
    }

    fn notify(&self) -> impl Fn() + Send + 'static {
        let ctx = self.ctx.clone();
        move || {
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        }
    }

    fn start_check(&mut self, manual: bool) {
        if self.phase != UpdatePhase::Idle {
            return;
        }
        let Some(u) = &self.updater else { return };
        self.manual = manual;
        self.phase = UpdatePhase::Checking;
        self.last_check = Some(Instant::now());
        u.spawn_check(self.tx.clone(), self.notify());
    }

    fn start_install(&mut self) {
        let (Some(u), Some(c)) = (&self.updater, &self.check) else {
            return;
        };
        if self.phase != UpdatePhase::Idle || !c.newer || c.asset.is_none() {
            return;
        }
        self.phase = UpdatePhase::Installing;
        self.error = None;
        u.spawn_install(c.clone(), self.tx.clone(), self.notify());
    }

    /// Returns a message for the status bar, if any.
    fn drain(&mut self) -> Option<String> {
        let mut say = None;
        while let Ok(m) = self.rx.try_recv() {
            match m {
                update::Msg::Checked(r) => {
                    self.phase = UpdatePhase::Idle;
                    match r {
                        Ok(c) => {
                            self.error = None;
                            if c.newer {
                                say = Some(format!("Update {} available", c.version));
                            } else if self.manual {
                                say = Some(format!(
                                    "Telepad {} is up to date",
                                    update::CURRENT_VERSION
                                ));
                            }
                            self.check = Some(c);
                        }
                        Err(e) => {
                            if self.manual {
                                say = Some(format!("Update check failed: {e}"));
                            }
                            self.error = Some(e);
                        }
                    }
                }
                update::Msg::Installed(r) => match r {
                    Ok(v) => {
                        say = Some(format!("Updated to {v} — restart to use it"));
                        self.phase = UpdatePhase::Installed(v);
                        self.window = true;
                    }
                    Err(e) => {
                        self.phase = UpdatePhase::Idle;
                        say = Some(format!("Update failed: {e}"));
                        self.error = Some(e);
                    }
                },
            }
        }
        say
    }

    fn enabled(&self) -> bool {
        self.updater.as_ref().is_some_and(|u| u.config.enabled)
    }

    fn banner(&self) -> Option<&Check> {
        self.check
            .as_ref()
            .filter(|c| c.newer && !self.later && self.phase == UpdatePhase::Idle)
    }
}

struct MonitorApp {
    workspace: Option<Workspace>,
    editor: Option<Editor>,
    import: Option<ImportDialog>,
    settings: Option<SshSettings>,
    hosts: Vec<HostState>,
    collector: Option<Collector>,
    rx: Option<Receiver<Update>>,
    selected: Option<usize>,
    filter: String,
    auto_refresh: bool,
    interval_secs: u32,
    last_refresh: Option<Instant>,
    message: Option<(String, Instant)>,
    fatal: Option<String>,
    ports_nginx_only: bool,
    /// Host id + alias waiting for delete confirmation.
    pending_delete: Option<(i64, String)>,
    update: UpdateUi,
}

impl MonitorApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        cli_config: Option<String>,
        cli_db: Option<String>,
    ) -> Self {
        icons::install(&cc.egui_ctx);
        let mut app = Self {
            workspace: None,
            editor: None,
            import: None,
            settings: None,
            hosts: Vec::new(),
            collector: None,
            rx: None,
            selected: None,
            filter: String::new(),
            auto_refresh: true,
            interval_secs: 60,
            last_refresh: None,
            message: None,
            fatal: None,
            ports_nginx_only: false,
            pending_delete: None,
            update: UpdateUi::default(),
        };
        let ws = match open_workspace(cli_config.as_deref(), cli_db.as_deref()) {
            Ok(ws) => ws,
            Err(e) => {
                app.fatal = Some(format!("Cannot open the host database: {e}"));
                return app;
            }
        };
        if let Some(src) = &ws.imported_from {
            app.message = Some((
                format!(
                    "Created {} from {}",
                    ws.store.path().display(),
                    src.display()
                ),
                Instant::now(),
            ));
        }
        let settings = ws.settings();
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let (collector, rx) = Collector::start(
            settings.clone(),
            WORKERS,
            Arc::new(move || ctx.request_repaint()),
        );
        app.update.ctx = Some(cc.egui_ctx.clone());
        match ws.store.update_config() {
            Ok(c) => app.update.configure(c),
            Err(e) => app.update.error = Some(format!("update settings: {e}")),
        }
        if app.update.enabled() {
            app.update.start_check(false);
        }
        app.workspace = Some(ws);
        app.settings = Some(settings);
        app.collector = Some(collector);
        app.rx = Some(rx);
        app.reload_hosts();
        if let Ok(alias) = std::env::var("TELEPAD_SELECT") {
            app.selected = app.hosts.iter().position(|h| h.entry.alias == alias);
        }
        app.refresh_all();
        app
    }

    fn refresh_all(&mut self) {
        if let Some(c) = &self.collector {
            c.enqueue(
                self.hosts
                    .iter()
                    .filter(|h| !h.entry.unmonitored)
                    .map(|h| h.entry.clone()),
            );
            self.last_refresh = Some(Instant::now());
        }
    }

    fn refresh_one(&mut self, i: usize) {
        if let (Some(c), Some(h)) = (&self.collector, self.hosts.get(i))
            && !h.entry.unmonitored
        {
            c.enqueue([h.entry.clone()]);
        }
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.message = Some((msg.into(), Instant::now()));
    }

    /// Re-read hosts from the database, keep results for unchanged hosts,
    /// regenerate the ssh config and poll anything new or changed.
    fn reload_hosts(&mut self) {
        let Some(ws) = &self.workspace else { return };
        let entries = match ws.store.hosts() {
            Ok(h) => h,
            Err(e) => {
                self.say(format!("Cannot read hosts: {e}"));
                return;
            }
        };
        if let Err(e) = ws.sync_ssh_config() {
            self.say(format!("Cannot write {}: {e}", ws.ssh_config.display()));
        }
        let selected = self
            .selected
            .and_then(|i| self.hosts.get(i))
            .and_then(|h| h.entry.id);
        let mut old: Vec<HostState> = std::mem::take(&mut self.hosts);
        let mut to_poll = Vec::new();
        for entry in entries {
            let prev = old
                .iter()
                .position(|h| h.entry.id == entry.id)
                .map(|i| old.swap_remove(i));
            let state = match prev {
                Some(mut h) if h.entry == entry => {
                    h.entry = entry;
                    h
                }
                _ => {
                    if !entry.unmonitored {
                        to_poll.push(entry.clone());
                    }
                    HostState {
                        entry,
                        ..Default::default()
                    }
                }
            };
            self.hosts.push(state);
        }
        self.selected =
            selected.and_then(|id| self.hosts.iter().position(|h| h.entry.id == Some(id)));
        if let Some(c) = &self.collector {
            c.enqueue(to_poll);
        }
        if self.last_refresh.is_none() {
            self.last_refresh = Some(Instant::now());
        }
    }

    fn save_editor(&mut self) {
        let (Some(ws), Some(ed)) = (&self.workspace, &mut self.editor) else {
            return;
        };
        let entry = match ed.build() {
            Ok(e) => e,
            Err(e) => {
                ed.error = Some(e);
                return;
            }
        };
        match ws.store.save_host(&entry) {
            Ok(id) => {
                let alias = entry.alias.clone();
                self.editor = None;
                self.reload_hosts();
                self.selected = self.hosts.iter().position(|h| h.entry.id == Some(id));
                self.say(format!("Saved {alias}"));
            }
            Err(e) => ed.error = Some(e.to_string()),
        }
    }

    fn delete_editor_host(&mut self) {
        let Some(ed) = &self.editor else { return };
        match ed.entry.id {
            Some(id) => {
                let alias = ed.entry.alias.clone();
                self.editor = None;
                self.delete_host(id, &alias);
            }
            None => self.editor = None,
        }
    }

    fn delete_host(&mut self, id: i64, alias: &str) {
        let Some(ws) = &self.workspace else { return };
        match ws.store.delete_host(id) {
            Ok(()) => {
                if self
                    .selected
                    .and_then(|i| self.hosts.get(i))
                    .is_some_and(|h| h.entry.id == Some(id))
                {
                    self.selected = None;
                }
                self.reload_hosts();
                self.say(format!("Deleted {alias}"));
            }
            Err(e) => self.say(format!("Cannot delete {alias}: {e}")),
        }
    }

    fn delete_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((id, alias)) = self.pending_delete.clone() else {
            return;
        };
        let (mut yes, mut no) = (false, false);
        egui::Window::new("Delete host")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("Delete {alias} from the host database?"));
                ui.label(
                    RichText::new("Its ssh config entry is removed too. Export first if you may need it back.")
                        .weak(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new(il(icons::TRASH, "Delete")).color(usage_color(95.0)))
                        .clicked()
                    {
                        yes = true;
                    }
                    if ui.button("Cancel").clicked() {
                        no = true;
                    }
                });
            });
        if yes {
            self.pending_delete = None;
            self.delete_host(id, &alias);
        } else if no {
            self.pending_delete = None;
        }
    }

    fn save_update_settings(&mut self) {
        let mut c = self.update.form.clone();
        c.repo = c.repo.trim().to_string();
        if let Err(e) = c.validate() {
            self.update.form_error = Some(e);
            return;
        }
        let Some(ws) = &self.workspace else { return };
        match ws.store.set_update_config(&c) {
            Ok(()) => {
                self.update.form_error = None;
                self.update.configure(c);
                self.update.check = None;
                self.say("Update settings saved");
                if self.update.enabled() {
                    self.update.start_check(true);
                }
            }
            Err(e) => self.update.form_error = Some(e.to_string()),
        }
    }

    fn restart(&mut self, ctx: &egui::Context) {
        match update::relaunch() {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => self.say(e),
        }
    }

    fn update_window(&mut self, ctx: &egui::Context) {
        if !self.update.window {
            return;
        }
        let mut open = true;
        let (mut install, mut check, mut later, mut save, mut restart) =
            (false, false, false, false, false);
        let mut open_page: Option<String> = None;
        let u = &mut self.update;
        egui::Window::new(il(icons::IMPORT, "Software update"))
            .id(egui::Id::unique("update-window"))
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("Installed: {}", update::CURRENT_VERSION));
                match &u.check {
                    Some(c) => {
                        let repo = u.updater.as_ref().map_or("", |x| x.config.repo.as_str());
                        ui.label(format!("Latest release of {repo}: {}", c.version));
                        if let Some(n) = &c.notes {
                            egui::ScrollArea::vertical()
                                .max_height(160.0)
                                .show(ui, |ui| ui.label(RichText::new(n).weak()));
                        }
                        if !c.page_url.is_empty() && ui.link("Release page on GitHub").clicked() {
                            open_page = Some(c.page_url.clone());
                        }
                        if c.newer && c.asset.is_none() {
                            ui.colored_label(
                                usage_color(80.0),
                                format!(
                                    "This release has no {} download.",
                                    update::asset_name(&c.version)
                                ),
                            );
                        }
                    }
                    None => {
                        ui.label(RichText::new("Not checked yet").weak());
                    }
                }
                if let Some(e) = &u.error {
                    ui.colored_label(usage_color(95.0), e);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| match &u.phase {
                    UpdatePhase::Checking => {
                        ui.spinner();
                        ui.label("Checking…");
                    }
                    UpdatePhase::Installing => {
                        ui.spinner();
                        ui.label("Downloading and installing…");
                    }
                    UpdatePhase::Installed(v) => {
                        ui.label(format!("{v} is installed."));
                        if ui.button(il(icons::REFRESH, "Restart now")).clicked() {
                            restart = true;
                        }
                    }
                    UpdatePhase::Idle => {
                        let can_install = u
                            .check
                            .as_ref()
                            .is_some_and(|c| c.newer && c.asset.is_some());
                        if can_install
                            && ui
                                .button(RichText::new(il(icons::IMPORT, "Update now")).strong())
                                .clicked()
                        {
                            install = true;
                        }
                        if can_install && ui.button("Later").clicked() {
                            later = true;
                        }
                        if ui
                            .add_enabled(
                                u.updater.is_some(),
                                egui::Button::new(il(icons::REFRESH, "Check now")),
                            )
                            .clicked()
                        {
                            check = true;
                        }
                    }
                });
                ui.add_space(6.0);
                egui::CollapsingHeader::new("Update source")
                    .default_open(u.error.is_some())
                    .show(ui, |ui| {
                        egui::Grid::new("update-settings")
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Check for updates");
                                ui.checkbox(&mut u.form.enabled, "at start and every 6 h");
                                ui.end_row();
                                ui.label("GitHub repository");
                                ui.add(
                                    egui::TextEdit::singleline(&mut u.form.repo)
                                        .hint_text(update::DEFAULT_REPO),
                                );
                                ui.end_row();
                            });
                        ui.label(
                            RichText::new(
                                "Reads the latest GitHub release over HTTPS (no account needed) \
                                 and checks the download against its .sha256 file.",
                            )
                            .weak()
                            .small(),
                        );
                        if let Some(e) = &u.form_error {
                            ui.colored_label(usage_color(95.0), e);
                        }
                        if ui.button(il(icons::SAVE, "Save")).clicked() {
                            save = true;
                        }
                    });
            });
        if !open {
            self.update.window = false;
        }
        if later {
            self.update.later = true;
            self.update.window = false;
        }
        if check {
            self.update.start_check(true);
        }
        if install {
            self.update.start_install();
        }
        if save {
            self.save_update_settings();
        }
        if restart {
            self.restart(ctx);
        }
        if let Some(url) = open_page
            && let Err(e) = platform::open_url(&url)
        {
            self.say(e);
        }
    }

    fn start_import(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import hosts (OpenSSH config or Telepad JSON)")
            .add_filter("All files", &["*"])
            .add_filter("JSON export", &["json"])
            .pick_file()
        else {
            return;
        };
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| store::parse_import(&t).map_err(|e| e.to_string()));
        match parsed {
            Ok(inventory) if inventory.hosts.is_empty() => {
                self.say(format!("No hosts found in {}", path.display()))
            }
            Ok(inventory) => {
                let mut d = ImportDialog {
                    path,
                    inventory,
                    mode: ImportMode::Merge,
                    preview: Ok(ImportSummary::default()),
                };
                self.update_preview(&mut d);
                self.import = Some(d);
            }
            Err(e) => self.say(format!("Cannot import {}: {e}", path.display())),
        }
    }

    fn update_preview(&self, d: &mut ImportDialog) {
        d.preview = match &self.workspace {
            Some(ws) => ws
                .store
                .preview_import(&d.inventory, d.mode)
                .map_err(|e| e.to_string()),
            None => Err("no database".into()),
        };
    }

    fn apply_import(&mut self) {
        let Some(d) = self.import.take() else { return };
        let Some(ws) = &mut self.workspace else {
            return;
        };
        match ws.store.import(&d.inventory, d.mode) {
            Ok(s) => {
                self.reload_hosts();
                self.say(format!(
                    "Imported {}: {} added, {} updated, {} unchanged, {} removed",
                    d.path.display(),
                    s.added,
                    s.updated,
                    s.unchanged,
                    s.removed
                ));
            }
            Err(e) => {
                let msg = format!("Import failed, nothing changed: {e}");
                self.import = Some(ImportDialog {
                    preview: Err(msg),
                    ..d
                });
            }
        }
    }

    fn export(&mut self) {
        let Some(ws) = &self.workspace else { return };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export hosts")
            .set_file_name("telepad-hosts.json")
            .add_filter("JSON (re-importable, keeps everything)", &["json"])
            .add_filter("OpenSSH config", &["config", "conf", "txt"])
            .save_file()
        else {
            return;
        };
        let json = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let result = ws
            .store
            .inventory()
            .map_err(|e| e.to_string())
            .and_then(|inv| {
                let text = if json {
                    store::export_json(&inv).map_err(|e| e.to_string())?
                } else {
                    telepad_core::config::to_ssh_config(&inv)
                };
                std::fs::write(&path, text).map_err(|e| e.to_string())?;
                Ok(inv.hosts.len())
            });
        match result {
            Ok(n) => self.say(format!("Exported {n} hosts to {}", path.display())),
            Err(e) => self.say(format!("Export failed: {e}")),
        }
    }

    fn drain_updates(&mut self) {
        let Some(rx) = &self.rx else { return };
        while let Ok(u) = rx.try_recv() {
            match u {
                Update::Started(alias) => {
                    if let Some(h) = self.hosts.iter_mut().find(|h| h.entry.alias == alias) {
                        h.loading = true;
                    }
                }
                Update::Done {
                    alias,
                    result,
                    elapsed,
                } => {
                    if let Some(h) = self.hosts.iter_mut().find(|h| h.entry.alias == alias) {
                        h.loading = false;
                        h.updated = Some(Instant::now());
                        h.elapsed = Some(elapsed);
                        match result {
                            Ok(r) => {
                                h.report = Some(*r);
                                h.error = None;
                            }
                            Err(e) => h.error = Some(e),
                        }
                    }
                }
            }
        }
    }

    fn open_ssh(&mut self, i: usize) {
        let (Some(s), Some(h)) = (&self.settings, self.hosts.get(i)) else {
            return;
        };
        if h.entry.access == Access::Rdp {
            let host = if h.entry.hostname.is_empty() {
                &h.entry.alias
            } else {
                &h.entry.hostname
            };
            let msg = match platform::open_rdp(host, h.entry.port) {
                Ok(()) => format!("Opened Remote Desktop to {}", h.entry.alias),
                Err(e) => e,
            };
            self.message = Some((msg, Instant::now()));
            return;
        }
        let args = s.args(&h.entry, false);
        let msg = match platform::open_terminal(&s.ssh_program, &args) {
            Ok(()) => format!("Opened SSH session to {}", h.entry.alias),
            Err(e) => e,
        };
        self.message = Some((msg, Instant::now()));
    }

    /// Open a terminal on the host that shows the sudo setup and waits for Enter.
    fn bmc_setup(&mut self, i: usize) {
        let (Some(s), Some(h)) = (&self.settings, self.hosts.get(i)) else {
            return;
        };
        let args = collector::bmc_setup_args(s, &h.entry);
        let msg = match platform::open_terminal(&s.ssh_program, &args) {
            Ok(()) => format!(
                "Opened BMC setup for {}: press Enter there, type your sudo password, then Refresh",
                h.entry.alias
            ),
            Err(e) => e,
        };
        self.message = Some((msg, Instant::now()));
    }

    fn copy_ssh(&mut self, ctx: &egui::Context, i: usize) {
        let (Some(s), Some(h)) = (&self.settings, self.hosts.get(i)) else {
            return;
        };
        ctx.copy_text(platform::command_line(
            &s.ssh_program,
            &s.args(&h.entry, false),
        ));
        self.message = Some(("SSH command copied to clipboard".into(), Instant::now()));
    }

    fn matches_filter(&self, h: &HostState) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let f = self.filter.to_lowercase();
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
}

impl MonitorApp {
    fn editor_window(&mut self, ctx: &egui::Context) {
        let Some(ed) = &mut self.editor else { return };
        let title = match (&ed.entry.id, &ed.cloned_from) {
            (Some(_), _) => format!("Edit host — {}", ed.entry.alias),
            (None, Some(src)) => format!("Add host — cloned from {src}"),
            (None, None) => "Add host".to_string(),
        };
        let (mut save, mut delete, mut cancel) = (false, false, false);
        let mut open = true;
        egui::Window::new(title)
            .id(egui::Id::unique("host-editor"))
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(src) = &ed.cloned_from {
                    ui.label(
                        RichText::new(format!(
                            "User, Port and IdentityFile copied from {src}; enter the new alias and HostName."
                        ))
                        .weak(),
                    );
                    ui.add_space(4.0);
                }
                egui::Grid::new("edit-grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Access");
                        ui.horizontal(|ui| {
                            let before = ed.access;
                            ui.radio_value(&mut ed.access, Access::Ssh, "SSH (monitored)");
                            ui.radio_value(&mut ed.access, Access::Rdp, "RDP only (Windows)");
                            if ed.access != before
                                && ed.port.trim() == before.default_port().to_string()
                            {
                                ed.port = ed.access.default_port().to_string();
                            }
                        });
                        ui.end_row();
                        if ed.access == Access::Rdp {
                            ui.label("");
                            ui.label(
                                RichText::new(
                                    "No SSH: the app only checks that the RDP port answers.                                      User and IdentityFile are not used.",
                                )
                                .weak(),
                            );
                            ui.end_row();
                        }
                        ui.label("Alias (Host)");
                        ui.add(egui::TextEdit::singleline(&mut ed.entry.alias).hint_text("e.g. mail58"));
                        ui.end_row();
                        ui.label("Description");
                        ui.add(
                            egui::TextEdit::singleline(&mut ed.entry.note)
                                .hint_text("shown under the host name"),
                        );
                        ui.end_row();
                        ui.label("HostName / IP");
                        ui.add(egui::TextEdit::singleline(&mut ed.entry.hostname).hint_text("192.0.2.58"));
                        ui.end_row();
                        ui.label("User");
                        ui.text_edit_singleline(&mut ed.entry.user);
                        ui.end_row();
                        ui.label("Port");
                        ui.add(egui::TextEdit::singleline(&mut ed.port).desired_width(80.0));
                        ui.end_row();
                        ui.label("IdentityFile");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut ed.identity)
                                    .hint_text("~/.ssh/id_ed25519"),
                            );
                            if ui.small_button("Browse…").clicked()
                                && let Some(p) = rfd::FileDialog::new()
                                    .set_title("Private key")
                                    .pick_file()
                            {
                                ed.identity = p.display().to_string();
                            }
                        });
                        ui.end_row();
                        ui.label("Monitored");
                        ui.checkbox(&mut ed.monitored, "poll this host");
                        ui.end_row();
                        ui.label("Other ssh options").on_hover_text(
                            "One 'Keyword value' per line, e.g. KexAlgorithms +diffie-hellman-group1-sha1",
                        );
                        ui.add(
                            egui::TextEdit::multiline(&mut ed.options)
                                .desired_rows(3)
                                .code_editor()
                                .hint_text("ProxyJump bastion"),
                        );
                        ui.end_row();
                    });
                if let Some(e) = &ed.error {
                    ui.colored_label(usage_color(95.0), e);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(il(icons::SAVE, "Save")).clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ed.entry.id.is_some() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ed.confirm_delete {
                                if ui
                                    .button(RichText::new("Really delete").color(usage_color(95.0)))
                                    .clicked()
                                {
                                    delete = true;
                                }
                                if ui.button("Keep").clicked() {
                                    ed.confirm_delete = false;
                                }
                            } else if ui.button(il(icons::TRASH, "Delete")).clicked() {
                                ed.confirm_delete = true;
                            }
                        });
                    }
                });
            });
        if save {
            self.save_editor();
        } else if delete {
            self.delete_editor_host();
        } else if cancel || !open {
            self.editor = None;
        }
    }

    fn import_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.import.take() else {
            return;
        };
        let (mut apply, mut cancel, mut mode_changed) = (false, false, false);
        let mut open = true;
        egui::Window::new("Import hosts")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(RichText::new(d.path.display().to_string()).monospace());
                ui.label(format!("{} hosts in file", d.inventory.hosts.len()));
                if !d.inventory.global.is_empty() {
                    ui.label(RichText::new("Includes global options (Host * / Match)").weak());
                }
                ui.add_space(6.0);
                mode_changed |= ui
                    .radio_value(
                        &mut d.mode,
                        ImportMode::Merge,
                        "Merge — add new, update same alias, keep others",
                    )
                    .changed();
                mode_changed |= ui
                    .radio_value(
                        &mut d.mode,
                        ImportMode::Replace,
                        "Replace — hosts become exactly this file",
                    )
                    .changed();
                ui.add_space(6.0);
                match &d.preview {
                    Ok(p) => {
                        ui.label(format!(
                            "{} added · {} updated · {} unchanged",
                            p.added, p.updated, p.unchanged
                        ));
                        if p.removed > 0 {
                            ui.colored_label(
                                usage_color(95.0),
                                format!("{} host(s) not in the file will be deleted", p.removed),
                            );
                        }
                    }
                    Err(e) => {
                        ui.colored_label(usage_color(95.0), e);
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            d.preview.is_ok(),
                            egui::Button::new(il(icons::IMPORT, "Import")),
                        )
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if mode_changed {
            self.update_preview(&mut d);
        }
        if cancel || !open {
            return;
        }
        self.import = Some(d);
        if apply {
            self.apply_import();
        }
    }
}

/// Fans (BMC) first, then temperatures; explains why BMC data is missing when it is.
fn sensors(ui: &mut egui::Ui, r: &HostReport, i: usize, action: &mut Option<Action>) {
    if let Some(e) = &r.bmc_error {
        let fix = if e.contains("not installed") {
            "install it on this host: sudo apt install ipmitool"
        } else {
            "add the sudoers rule for 'ipmitool sdr type Fan' and 'ipmitool sdr type Temperature' (see README)"
        };
        ui.label(
            RichText::new(format!(
                "{}  BMC found but not readable: {e} — {fix}",
                icons::WARNING
            ))
            .color(usage_color(80.0)),
        );
        if ui
            .button(il(icons::TERMINAL, "Set up BMC access…"))
            .on_hover_text(
                "Opens a terminal on this host showing the sudo commands. Press Enter to run them and type your sudo password; Ctrl+C cancels.",
            )
            .clicked()
        {
            *action = Some(Action::BmcSetup(i));
        }
        ui.add_space(4.0);
    }
    if r.sensors.is_empty() {
        let why = if r.has_bmc {
            "No sensor readings"
        } else {
            "No temperature or fan sensors exposed (typical for VMs)"
        };
        ui.label(RichText::new(why).weak());
        return;
    }
    let fans: Vec<_> = r
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Fan)
        .collect();
    if !fans.is_empty() {
        let max = fans.iter().map(|s| s.value).fold(1.0_f32, f32::max);
        ui.label(RichText::new(il(icons::FAN, "Fans")).strong());
        egui::Grid::new("fans")
            .striped(true)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for s in &fans {
                    spinning_fan(ui, 18.0, (s.value / max).clamp(0.0, 1.0));
                    ui.label(&s.chip);
                    ui.label(&s.label);
                    ui.add(
                        egui::ProgressBar::new((s.value / max).clamp(0.0, 1.0))
                            .desired_width(220.0)
                            .fill(Color32::from_rgb(0x2b, 0x8a, 0xd6))
                            .text(format!("{:.0} RPM", s.value)),
                    );
                    ui.end_row();
                }
            });
        ui.add_space(6.0);
    }
    let temps: Vec<_> = r
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .collect();
    if !temps.is_empty() {
        ui.label(RichText::new(il(icons::THERMOMETER, "Temperatures")).strong());
        egui::Grid::new("temps")
            .striped(true)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for s in temps {
                    ui.label(
                        RichText::new(icons::THERMOMETER)
                            .size(16.0)
                            .color(temp_color(s.value)),
                    )
                    .on_hover_text(temp_level(s.value));
                    ui.label(&s.chip);
                    ui.label(&s.label);
                    ui.colored_label(temp_color(s.value), format!("{:.1} °C", s.value));
                    ui.end_row();
                }
            });
    }
}

fn connect_icon(e: &HostEntry) -> &'static str {
    match e.access {
        Access::Ssh => icons::TERMINAL,
        Access::Rdp => icons::MONITOR,
    }
}

fn host_os(h: &HostState) -> icons::OsKind {
    if h.entry.access == Access::Rdp {
        return icons::OsKind::Windows;
    }
    match &h.report {
        Some(r) => icons::detect(&r.os_name, &r.os_pretty, &r.os_type, &r.kernel),
        None => icons::OsKind::Unknown,
    }
}

fn usage_color(pct: f32) -> Color32 {
    if pct >= 90.0 {
        Color32::from_rgb(0xd9, 0x3f, 0x3f)
    } else if pct >= 75.0 {
        Color32::from_rgb(0xe0, 0x9b, 0x1a)
    } else {
        Color32::from_rgb(0x2f, 0x9e, 0x5b)
    }
}

/// Hot = red, warm = yellow, normal = blue.
fn temp_color(c: f32) -> Color32 {
    if c >= TEMP_HOT {
        Color32::from_rgb(0xe0, 0x3e, 0x3e)
    } else if c >= TEMP_WARM {
        Color32::from_rgb(0xf0, 0xc0, 0x20)
    } else {
        Color32::from_rgb(0x3a, 0x8e, 0xe6)
    }
}

const TEMP_WARM: f32 = 70.0;
const TEMP_HOT: f32 = 85.0;

fn temp_level(c: f32) -> &'static str {
    if c >= TEMP_HOT {
        "Hot (85 °C or more)"
    } else if c >= TEMP_WARM {
        "Warm (70–85 °C)"
    } else {
        "Normal (below 70 °C)"
    }
}

/// Rotations per second for a fan at `frac` of the host's fastest fan:
/// a slow idle turn at 0, about 2.5 turns/s at full speed.
fn fan_turns_per_sec(frac: f32) -> f32 {
    0.25 + 2.25 * frac.clamp(0.0, 1.0)
}

/// A painted four-blade fan that spins faster the higher `frac` (0..=1) is.
fn spinning_fan(ui: &mut egui::Ui, size: f32, frac: f32) -> egui::Response {
    use std::f32::consts::TAU;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let time = ui.input(|i| i.time) as f32;
        // Wrap time so the angle keeps float precision in long sessions.
        let angle = (time % 3600.0) * fan_turns_per_sec(frac) * TAU;
        let c = rect.center();
        let r = size * 0.5;
        let color = Color32::from_rgb(0x2b, 0x8a, 0xd6);
        let p = ui.painter();
        p.circle_stroke(
            c,
            r - 0.5,
            egui::Stroke::new(1.0, color.gamma_multiply(0.6)),
        );
        let at = |a: f32, d: f32| c + egui::vec2(a.cos(), a.sin()) * d;
        for k in 0..4 {
            let a = angle + k as f32 * TAU / 4.0;
            let blade = vec![
                at(a - 0.30, r * 0.22),
                at(a - 0.10, r * 0.85),
                at(a + 0.45, r * 0.80),
                at(a + 0.20, r * 0.22),
            ];
            p.add(egui::Shape::convex_polygon(
                blade,
                color,
                egui::Stroke::NONE,
            ));
        }
        p.circle_filled(c, r * 0.22, color);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    resp.on_hover_text(format!("{:.0}% of the fastest fan", frac * 100.0))
}

fn status_dot(ui: &mut egui::Ui, h: &HostState) {
    let color = match (h.loading, h.online()) {
        (_, Some(false)) => Color32::from_rgb(0xd9, 0x3f, 0x3f),
        (_, Some(true)) => Color32::from_rgb(0x2f, 0x9e, 0x5b),
        (true, None) => Color32::from_rgb(0x4a, 0x90, 0xd9),
        (false, None) => Color32::GRAY,
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

fn bar(ui: &mut egui::Ui, pct: f32, text: String, width: f32) {
    ui.add(
        egui::ProgressBar::new((pct / 100.0).clamp(0.0, 1.0))
            .desired_width(width)
            .fill(usage_color(pct))
            .text(text),
    );
}

fn ago(t: Option<Instant>) -> String {
    match t {
        None => "never".into(),
        Some(t) => {
            let s = t.elapsed().as_secs();
            if s < 60 {
                format!("{s}s ago")
            } else {
                format!("{}m ago", s / 60)
            }
        }
    }
}

impl eframe::App for MonitorApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        self.drain_updates();
        if let Some(m) = self.update.drain() {
            self.say(m);
        }
        if self.update.enabled()
            && self
                .update
                .last_check
                .is_some_and(|t| t.elapsed() >= update::CHECK_EVERY)
        {
            self.update.start_check(false);
        }
        let busy = self.collector.as_ref().map_or(0, |c| c.busy());
        if self.auto_refresh
            && busy == 0
            && self
                .last_refresh
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(self.interval_secs as u64))
        {
            self.refresh_all();
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        let mut action: Option<Action> = None;

        egui::Panel::top("top").show(root, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading(il(icons::DATABASE, "Telepad"));
                ui.separator();
                ui.menu_button(il(icons::MENU, "Hosts"), |ui| {
                    if ui.button(il(icons::PLUS, "Add host…")).clicked() {
                        action = Some(Action::AddHost);
                        ui.close();
                    }
                    if ui.button(il(icons::IMPORT, "Import…")).clicked() {
                        action = Some(Action::Import);
                        ui.close();
                    }
                    if ui.button(il(icons::EXPORT, "Export…")).clicked() {
                        action = Some(Action::Export);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(il(icons::IMPORT, "Software update…")).clicked() {
                        action = Some(Action::UpdateWindow);
                        ui.close();
                    }
                });
                if let UpdatePhase::Installed(v) = &self.update.phase {
                    if ui
                        .button(
                            RichText::new(il(icons::REFRESH, &format!("Restart for {v}")))
                                .color(usage_color(0.0))
                                .strong(),
                        )
                        .clicked()
                    {
                        action = Some(Action::Restart);
                    }
                } else if self.update.phase == UpdatePhase::Installing {
                    ui.spinner();
                    ui.label("updating…");
                } else if let Some(c) = self.update.banner()
                    && ui
                        .button(
                            RichText::new(il(
                                icons::IMPORT,
                                &format!("Update {} available", c.version),
                            ))
                            .color(usage_color(0.0))
                            .strong(),
                        )
                        .on_hover_text(format!("Installed: {}", update::CURRENT_VERSION))
                        .clicked()
                {
                    action = Some(Action::UpdateWindow);
                }
                if ui
                    .add_enabled(
                        busy == 0,
                        egui::Button::new(il(icons::REFRESH, "Refresh all")),
                    )
                    .clicked()
                {
                    action = Some(Action::RefreshAll);
                }
                ui.checkbox(&mut self.auto_refresh, "Auto every");
                ui.add(
                    egui::DragValue::new(&mut self.interval_secs)
                        .range(15..=3600)
                        .suffix(" s"),
                );
                ui.separator();
                ui.label(RichText::new(icons::SEARCH).size(16.0));
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .desired_width(180.0)
                        .hint_text("host, IP, OS, container"),
                );
                ui.separator();
                let online = self
                    .hosts
                    .iter()
                    .filter(|h| h.online() == Some(true))
                    .count();
                let offline = self
                    .hosts
                    .iter()
                    .filter(|h| h.online() == Some(false))
                    .count();
                ui.label(
                    RichText::new(format!("{} {online} up", icons::DOT)).color(usage_color(0.0)),
                );
                ui.label(
                    RichText::new(format!("{} {offline} down", icons::DOT))
                        .color(usage_color(95.0)),
                );
                ui.label(format!("{} hosts", self.hosts.len()));
                if busy > 0 {
                    ui.spinner();
                    ui.label(format!("{busy} loading"));
                }
            });
            ui.add_space(4.0);
        });

        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                if let Some(ws) = &self.workspace {
                    ui.label(
                        RichText::new(format!("database: {}", ws.store.path().display())).weak(),
                    )
                    .on_hover_text(format!(
                        "ssh uses the generated file {}",
                        ws.ssh_config.display()
                    ));
                }
                ui.separator();
                let v = ui.label(RichText::new(format!("v{}", update::CURRENT_VERSION)).weak());
                if let Some(e) = &self.update.error {
                    let tip = format!("Update check failed: {e}");
                    v.on_hover_text(&tip);
                    ui.label(RichText::new(icons::WARNING).weak())
                        .on_hover_text(tip);
                }
                if let Some((m, t)) = &self.message
                    && t.elapsed() < Duration::from_secs(6)
                {
                    ui.separator();
                    ui.label(m);
                }
            });
        });

        if let Some(f) = &self.fatal {
            egui::CentralPanel::default().show(root, |ui| {
                ui.colored_label(usage_color(95.0), f);
            });
            return;
        }

        egui::Panel::left("hosts")
            .resizable(true)
            .default_size(290.0)
            .show(root, |ui| {
                ui.add_space(4.0);
                if ui
                    .selectable_label(
                        self.selected.is_none(),
                        RichText::new(il(icons::OVERVIEW, "Overview — all hosts")).strong(),
                    )
                    .clicked()
                {
                    self.selected = None;
                }
                if ui.button(il(icons::PLUS, "Add host")).clicked() {
                    action = Some(Action::AddHost);
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (i, h) in self.hosts.iter().enumerate() {
                            if !self.matches_filter(h) {
                                continue;
                            }
                            let sel = self.selected == Some(i);
                            let resp = ui
                                .horizontal(|ui| {
                                    status_dot(ui, h);
                                    let mut name = RichText::new(&h.entry.alias).strong();
                                    if h.entry.unmonitored {
                                        name = name.weak();
                                    }
                                    icons::os_logo(ui, host_os(h), 16.0);
                                    let label = ui.selectable_label(sel, name);
                                    ui.label(RichText::new(&h.entry.hostname).weak().small());
                                    label
                                })
                                .inner;
                            if resp.clicked() {
                                self.selected = Some(i);
                            }
                            resp.context_menu(|ui| {
                                if ui.button(il(icons::EDIT, "Edit")).clicked() {
                                    action = Some(Action::Edit(i));
                                    ui.close();
                                }
                                if ui
                                    .button(il(icons::CLONE, "Clone (User, Port, IdentityFile)"))
                                    .clicked()
                                {
                                    action = Some(Action::Clone(i));
                                    ui.close();
                                }
                                if ui.button(il(icons::TRASH, "Delete")).clicked() {
                                    action = Some(Action::Delete(i));
                                    ui.close();
                                }
                            });
                            if !h.entry.note.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    ui.label(RichText::new(&h.entry.note).small().italics());
                                });
                            }
                            if let Some(r) = &h.report {
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    if let Some(c) = r.cpu_usage_pct {
                                        bar(ui, c, format!("CPU {c:.0}%"), 80.0);
                                    }
                                    if let Some(m) = &r.memory {
                                        bar(
                                            ui,
                                            m.used_pct(),
                                            format!("RAM {:.0}%", m.used_pct()),
                                            80.0,
                                        );
                                    }
                                    if let Some(d) = r
                                        .disks
                                        .iter()
                                        .max_by(|a, b| a.used_pct().total_cmp(&b.used_pct()))
                                    {
                                        bar(
                                            ui,
                                            d.used_pct(),
                                            format!("Disk {:.0}%", d.used_pct()),
                                            80.0,
                                        );
                                    }
                                });
                            } else if let Some(e) = &h.error {
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    ui.label(RichText::new(e).small().color(usage_color(95.0)));
                                });
                            }
                            ui.add_space(2.0);
                        }
                    });
            });

        let mut nginx_only = self.ports_nginx_only;
        egui::CentralPanel::default().show(root, |ui| match self.selected {
            None => overview(ui, self, &mut action),
            Some(i) => {
                if let Some(h) = self.hosts.get(i) {
                    detail(ui, h, i, &mut nginx_only, &mut action);
                }
            }
        });
        self.ports_nginx_only = nginx_only;

        match action {
            Some(Action::RefreshAll) => self.refresh_all(),
            Some(Action::Refresh(i)) => self.refresh_one(i),
            Some(Action::Ssh(i)) => self.open_ssh(i),
            Some(Action::CopySsh(i)) => self.copy_ssh(ctx, i),
            Some(Action::BmcSetup(i)) => self.bmc_setup(i),
            Some(Action::Select(i)) => self.selected = Some(i),
            Some(Action::AddHost) => self.editor = Some(Editor::new(HostEntry::new(""))),
            Some(Action::Edit(i)) => {
                if let Some(h) = self.hosts.get(i) {
                    self.editor = Some(Editor::new(h.entry.clone()));
                }
            }
            Some(Action::Clone(i)) => {
                if let Some(h) = self.hosts.get(i) {
                    self.editor = Some(Editor::clone_of(&h.entry));
                }
            }
            Some(Action::Delete(i)) => {
                if let Some(h) = self.hosts.get(i)
                    && let Some(id) = h.entry.id
                {
                    self.pending_delete = Some((id, h.entry.alias.clone()));
                }
            }
            Some(Action::Import) => self.start_import(),
            Some(Action::Export) => self.export(),
            Some(Action::UpdateWindow) => self.update.window = true,
            Some(Action::Restart) => self.restart(ctx),
            None => {}
        }
        self.editor_window(ctx);
        self.import_window(ctx);
        self.delete_confirm_window(ctx);
        self.update_window(ctx);
    }
}

enum Action {
    RefreshAll,
    Refresh(usize),
    Ssh(usize),
    CopySsh(usize),
    BmcSetup(usize),
    Select(usize),
    AddHost,
    Edit(usize),
    Clone(usize),
    Delete(usize),
    Import,
    Export,
    UpdateWindow,
    Restart,
}

fn overview(ui: &mut egui::Ui, app: &MonitorApp, action: &mut Option<Action>) {
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("overview")
                .striped(true)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    for title in [
                        "",
                        "Host",
                        "Description",
                        "Address",
                        "OS",
                        "CPU",
                        "Memory",
                        "Fullest disk",
                        "Temp",
                        "Docker",
                        "Updated",
                        "",
                    ] {
                        ui.label(RichText::new(title).strong());
                    }
                    ui.end_row();
                    for (i, h) in app.hosts.iter().enumerate() {
                        if !app.matches_filter(h) {
                            continue;
                        }
                        status_dot(ui, h);
                        if ui.link(RichText::new(&h.entry.alias).strong()).clicked() {
                            *action = Some(Action::Select(i));
                        }
                        ui.label(RichText::new(&h.entry.note).italics());
                        ui.label(&h.entry.hostname);
                        match &h.report {
                            Some(r) => {
                                ui.horizontal(|ui| {
                                    icons::os_logo(ui, host_os(h), 16.0);
                                    ui.label(&r.os_pretty);
                                });
                                match r.cpu_usage_pct {
                                    Some(c) => bar(ui, c, format!("{c:.0}%"), 90.0),
                                    None => {
                                        ui.label("—");
                                    }
                                }
                                match &r.memory {
                                    Some(m) => bar(
                                        ui,
                                        m.used_pct(),
                                        format!(
                                            "{} / {}",
                                            human_kb(m.used_kb()),
                                            human_kb(m.total_kb)
                                        ),
                                        150.0,
                                    ),
                                    None => {
                                        ui.label("—");
                                    }
                                }
                                match r
                                    .disks
                                    .iter()
                                    .max_by(|a, b| a.used_pct().total_cmp(&b.used_pct()))
                                {
                                    Some(d) => bar(
                                        ui,
                                        d.used_pct(),
                                        format!("{} {:.0}%", d.mount, d.used_pct()),
                                        150.0,
                                    ),
                                    None => {
                                        ui.label("—");
                                    }
                                }
                                let t = r
                                    .sensors
                                    .iter()
                                    .filter(|s| s.kind == SensorKind::Temperature)
                                    .map(|s| s.value)
                                    .fold(None, |m: Option<f32>, v| {
                                        Some(m.map_or(v, |m| m.max(v)))
                                    });
                                match t {
                                    Some(t) => ui.colored_label(temp_color(t), format!("{t:.0}°C")),
                                    None => ui.label(RichText::new("n/a").weak()),
                                };
                                match &r.docker.unavailable {
                                    Some(_) => ui.label(RichText::new("—").weak()),
                                    None => {
                                        let run = r
                                            .docker
                                            .containers
                                            .iter()
                                            .filter(|c| c.running())
                                            .count();
                                        let role = match r.docker.portainer().first() {
                                            Some((PortainerRole::Server, _)) => " · Portainer",
                                            Some((PortainerRole::Agent, _)) => " · agent",
                                            None => "",
                                        };
                                        let rt = if r.docker.is_podman() {
                                            " (podman)"
                                        } else {
                                            ""
                                        };
                                        ui.label(format!(
                                            "{run}/{} running{rt}{role}",
                                            r.docker.containers.len()
                                        ))
                                    }
                                };
                            }
                            None => {
                                let msg = if let Some(e) = &h.error {
                                    RichText::new(e).color(usage_color(95.0))
                                } else if h.loading {
                                    RichText::new("loading…").weak()
                                } else {
                                    RichText::new("queued").weak()
                                };
                                ui.label(msg);
                                for _ in 0..5 {
                                    ui.label("");
                                }
                            }
                        }
                        ui.label(RichText::new(ago(h.updated)).weak());
                        let (label, tip) = match h.entry.access {
                            Access::Ssh => ("SSH", "Open an SSH session in a terminal"),
                            Access::Rdp => ("RDP", "Open Remote Desktop"),
                        };
                        if ui
                            .small_button(il(connect_icon(&h.entry), label))
                            .on_hover_text(tip)
                            .clicked()
                        {
                            *action = Some(Action::Ssh(i));
                        }
                        ui.end_row();
                    }
                });
        });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).heading().size(17.0));
    ui.separator();
}

fn detail(
    ui: &mut egui::Ui,
    h: &HostState,
    i: usize,
    nginx_only: &mut bool,
    action: &mut Option<Action>,
) {
    ui.horizontal(|ui| {
        status_dot(ui, h);
        icons::os_logo(ui, host_os(h), 26.0);
        ui.heading(&h.entry.alias);
        ui.label(RichText::new(h.entry.target()).monospace());
        if h.entry.port != h.entry.access.default_port() {
            ui.label(format!("port {}", h.entry.port));
        }
        ui.separator();
        let rdp = h.entry.access == Access::Rdp;
        let open = if rdp { "Open RDP" } else { "Open SSH" };
        if ui.button(il(connect_icon(&h.entry), open)).clicked() {
            *action = Some(Action::Ssh(i));
        }
        if !rdp && ui.button(il(icons::COPY, "Copy ssh command")).clicked() {
            *action = Some(Action::CopySsh(i));
        }
        if ui.button(il(icons::EDIT, "Edit")).clicked() {
            *action = Some(Action::Edit(i));
        }
        if ui
            .button(il(icons::CLONE, "Clone"))
            .on_hover_text("New host with this User, Port and IdentityFile")
            .clicked()
        {
            *action = Some(Action::Clone(i));
        }
        if ui.button(il(icons::TRASH, "Delete")).clicked() {
            *action = Some(Action::Delete(i));
        }
        if ui
            .add_enabled(!h.loading, egui::Button::new(il(icons::REFRESH, "Refresh")))
            .clicked()
        {
            *action = Some(Action::Refresh(i));
        }
        if h.loading {
            ui.spinner();
        }
    });
    if !h.entry.note.is_empty() {
        ui.label(RichText::new(&h.entry.note).size(15.0).italics());
    }
    if h.entry.unmonitored {
        ui.label(RichText::new("Not monitored — enable it in Edit").weak());
    }
    ui.label(
        RichText::new(format!(
            "updated {}{}",
            ago(h.updated),
            h.elapsed.map_or(String::new(), |e| format!(
                " · took {:.1}s",
                e.as_secs_f32()
            ))
        ))
        .weak(),
    );
    if let Some(e) = &h.error {
        ui.colored_label(usage_color(95.0), format!("Last check failed: {e}"));
    }
    if h.entry.access == Access::Rdp {
        section(ui, &il(icons::WINDOWS, "Windows (Remote Desktop only)"));
        ui.label(
            "This host has no SSH, so there are no metrics. The app checks that the RDP \
             port accepts connections and opens Remote Desktop (mstsc) on request.",
        );
        ui.add_space(4.0);
        match (&h.report, &h.error) {
            (_, Some(e)) => ui.colored_label(usage_color(95.0), e),
            (Some(r), None) => {
                let ok = |b: bool| {
                    if b {
                        usage_color(0.0)
                    } else {
                        usage_color(95.0)
                    }
                };
                egui::Grid::new("rdp-status")
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Ping").strong());
                        ui.colored_label(
                            ok(r.ping_ms.is_some()),
                            r.ping_ms.map_or(
                                "no reply (ICMP may be blocked by the firewall)".into(),
                                |ms| format!("{ms:.0} ms"),
                            ),
                        );
                        ui.end_row();
                        ui.label(RichText::new(format!("RDP {}", h.entry.port)).strong());
                        ui.colored_label(
                            ok(r.rdp_open),
                            if r.rdp_open {
                                "open"
                            } else {
                                "closed / filtered"
                            },
                        );
                        ui.end_row();
                    });
                ui.label("")
            }
            (None, None) => ui.label("Checking…"),
        };
        return;
    }
    let Some(r) = &h.report else {
        if h.error.is_none() {
            ui.label("Waiting for first result…");
        }
        return;
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(ui, &il(icons::INFO, "System"));
            egui::Grid::new("sys").spacing([16.0, 4.0]).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(RichText::new(k).strong());
                    ui.label(v);
                    ui.end_row();
                };
                row("Hostname", r.hostname.clone());
                row("OS", r.os_pretty.clone());
                row("Type", format!("{} {} ({})", r.os_type, r.kernel, r.arch));
                row("Uptime", r.uptime_secs.map_or("?".into(), human_uptime));
                row(
                    "CPU",
                    format!(
                        "{} · {} cores",
                        if r.cpu_model.is_empty() {
                            "?"
                        } else {
                            &r.cpu_model
                        },
                        r.cpu_cores.map_or("?".into(), |c| c.to_string())
                    ),
                );
                if let Some(l) = r.load_avg {
                    row("Load avg", format!("{:.2}  {:.2}  {:.2}", l[0], l[1], l[2]));
                }
            });

            section(ui, &il(icons::GAUGE, "Usage"));
            egui::Grid::new("usage")
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    if let Some(c) = r.cpu_usage_pct {
                        ui.label(RichText::new("CPU").strong());
                        bar(ui, c, format!("{c:.1}%"), 360.0);
                        ui.end_row();
                    }
                    if let Some(m) = &r.memory {
                        ui.label(RichText::new("Memory").strong());
                        bar(
                            ui,
                            m.used_pct(),
                            format!(
                                "{} used of {} ({:.1}%)",
                                human_kb(m.used_kb()),
                                human_kb(m.total_kb),
                                m.used_pct()
                            ),
                            360.0,
                        );
                        ui.end_row();
                        if m.swap_total_kb > 0 {
                            ui.label(RichText::new("Swap").strong());
                            bar(
                                ui,
                                m.swap_used_pct(),
                                format!(
                                    "{} used of {}",
                                    human_kb(m.swap_used_kb()),
                                    human_kb(m.swap_total_kb)
                                ),
                                360.0,
                            );
                            ui.end_row();
                        }
                    }
                });

            section(ui, &il(icons::DISKS, "Disks"));
            if r.disks.is_empty() {
                ui.label(RichText::new("No disk data").weak());
            }
            egui::Grid::new("disks")
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    for t in ["Mount", "Filesystem", "Usage", "Free"] {
                        ui.label(RichText::new(t).strong());
                    }
                    ui.end_row();
                    for d in &r.disks {
                        ui.label(RichText::new(&d.mount).monospace());
                        ui.label(RichText::new(&d.filesystem).weak());
                        bar(
                            ui,
                            d.used_pct(),
                            format!(
                                "{} / {} ({:.1}%)",
                                human_kb(d.used_kb),
                                human_kb(d.total_kb),
                                d.used_pct()
                            ),
                            300.0,
                        );
                        ui.label(human_kb(d.available_kb));
                        ui.end_row();
                    }
                });

            section(ui, &il(icons::NETWORK, "Network"));
            egui::Grid::new("net")
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for a in &r.addresses {
                        ui.label(RichText::new(&a.interface).strong());
                        ui.label(&a.family);
                        ui.label(RichText::new(&a.address).monospace());
                        ui.end_row();
                    }
                });

            section(ui, &il(icons::THERMOMETER, "Sensors"));
            sensors(ui, r, i, action);

            section(ui, &il(icons::CONTAINER, "Docker"));
            match &r.docker.unavailable {
                Some(why) => {
                    ui.label(RichText::new(format!("Docker: {why}")).weak());
                }
                None => {
                    docker_summary(ui, r, &h.entry.hostname);
                    if r.docker.containers.is_empty() {
                        ui.label(RichText::new("No containers").weak());
                    } else {
                        docker_stacks(ui, r);
                    }
                }
            }

            section(ui, &il(icons::PLUGS, "Ports in use"));
            ports_in_use(ui, r, nginx_only);
            ui.add_space(20.0);
        });
}

const NGINX_COLOR: Color32 = Color32::from_rgb(0x2b, 0x8a, 0xd6);
const PORTAINER_COLOR: Color32 = Color32::from_rgb(0x13, 0xb5, 0xea);

fn host_ports(c: &Container) -> String {
    let mut v: Vec<String> = Vec::new();
    for p in &c.published {
        let s = p.host_port.to_string();
        if !v.contains(&s) {
            v.push(s);
        }
    }
    v.join(", ")
}

/// Runtime and how this host's containers are managed (Portainer server / agent / plain).
fn docker_summary(ui: &mut egui::Ui, r: &HostReport, host_ip: &str) {
    egui::Grid::new("docker-summary").spacing([16.0, 4.0]).show(ui, |ui| {
        ui.label(RichText::new("Runtime").strong());
        let rt = if r.docker.runtime.is_empty() { "unknown".to_string() } else { r.docker.runtime.clone() };
        ui.label(rt);
        ui.end_row();

        ui.label(RichText::new("Management").strong());
        ui.vertical(|ui| {
            let portainer = r.docker.portainer();
            if portainer.is_empty() {
                ui.label("No Portainer on this host — stacks run via docker compose or docker run");
            }
            for (role, c) in portainer {
                let ports = host_ports(c);
                let text = match role {
                    PortainerRole::Server => format!(
                        "Portainer server ({}) — manages this host through the local socket{}{}",
                        c.name,
                        if c.mounts_socket { " /var/run/docker.sock" } else { "" },
                        if ports.is_empty() { String::new() } else { format!(" · UI on host port {ports}") }
                    ),
                    PortainerRole::Agent => format!(
                        "Portainer agent ({}) — managed by a remote Portainer server via {}:{}",
                        c.name,
                        host_ip,
                        if ports.is_empty() { "9001".to_string() } else { ports }
                    ),
                };
                ui.label(RichText::new(text).color(PORTAINER_COLOR));
            }
            let (mut pt, mut cp, mut st) = (0, 0, 0);
            for (_, cs) in r.docker.stacks() {
                match r.docker.stack_manager(&cs) {
                    Manager::Portainer { .. } => pt += 1,
                    Manager::Compose { .. } => cp += 1,
                    Manager::Standalone => st += cs.len(),
                }
            }
            ui.label(
                RichText::new(format!(
                    "{pt} Portainer stack(s) · {cp} docker compose stack(s) · {st} standalone container(s)"
                ))
                .weak(),
            );
        });
        ui.end_row();
    });
    ui.add_space(6.0);
}

fn manager_text(m: &Manager) -> (String, Color32) {
    match m {
        Manager::Portainer { stack_id } => {
            (format!("Portainer stack #{stack_id}"), PORTAINER_COLOR)
        }
        Manager::Compose { dir, files } => {
            let file = files.rsplit(['/', '\\']).next().unwrap_or(files);
            let f = if file.is_empty() {
                String::new()
            } else {
                format!(" ({file})")
            };
            (
                format!("docker compose in {dir}{f}"),
                Color32::from_rgb(0xa0, 0x7c, 0xd8),
            )
        }
        Manager::Standalone => ("docker run (no compose)".into(), Color32::GRAY),
    }
}

/// `2491→443/tcp` for published ports (IPv4/IPv6 duplicates merged), then internal ones.
fn container_ports(c: &Container) -> (String, String) {
    let mut published: Vec<String> = Vec::new();
    for p in &c.published {
        let s = format!("{}→{}/{}", p.host_port, p.container_port, p.proto);
        let s = if p.host_ip == "127.0.0.1" || p.host_ip == "::1" {
            format!("{s} (local)")
        } else {
            s
        };
        if !published.contains(&s) {
            published.push(s);
        }
    }
    let internal: Vec<&str> = c
        .ports
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty() && !x.contains("->"))
        .collect();
    (published.join("\n"), internal.join(", "))
}

fn docker_stacks(ui: &mut egui::Ui, r: &HostReport) {
    for (stack, containers) in r.docker.stacks() {
        let running = containers.iter().filter(|c| c.running()).count();
        let name = if stack.is_empty() {
            "(standalone)"
        } else {
            stack
        };
        let color = if running == containers.len() {
            usage_color(0.0)
        } else if running == 0 {
            Color32::GRAY
        } else {
            usage_color(80.0)
        };
        let (mtext, mcolor) = manager_text(&r.docker.stack_manager(&containers));
        let header = ui
            .horizontal(|ui| {
                let open_id = ui.make_persistent_id(("stack-open", stack));
                let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(open_id, true));
                let arrow = if open {
                    icons::CARET_DOWN
                } else {
                    icons::CARET_RIGHT
                };
                let resp = ui.add(
                    egui::Button::new(
                        RichText::new(format!(
                            "{arrow} {} {name}   {running}/{} running",
                            icons::STACK,
                            containers.len()
                        ))
                        .strong()
                        .color(color),
                    )
                    .frame(false),
                );
                ui.label(RichText::new(mtext).color(mcolor));
                if resp.clicked() {
                    open = !open;
                    ui.data_mut(|d| d.insert_persisted(open_id, open));
                }
                open
            })
            .inner;
        if !header {
            continue;
        }
        ui.indent(("stack-body", stack), |ui| {
            egui::Grid::new(("docker", stack))
                .striped(true)
                .spacing([14.0, 5.0])
                .show(ui, |ui| {
                    for t in [
                        "",
                        "Container",
                        "Image",
                        "Status",
                        "IP address",
                        "Host port → container",
                        "Internal",
                    ] {
                        ui.label(RichText::new(t).strong());
                    }
                    ui.end_row();
                    for c in containers {
                        let dot = if c.running() {
                            usage_color(0.0)
                        } else {
                            Color32::GRAY
                        };
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.5, dot);
                        let mut n = RichText::new(&c.name).strong();
                        if c.is_nginx() {
                            n = n.color(NGINX_COLOR);
                        } else if c.portainer_role().is_some() {
                            n = n.color(PORTAINER_COLOR);
                        }
                        let resp = ui.label(n);
                        if c.mounts_socket {
                            resp.on_hover_text("Mounts the Docker socket");
                        }
                        ui.label(RichText::new(&c.image).weak());
                        ui.label(&c.status);
                        let ips: Vec<String> = c
                            .networks
                            .iter()
                            .filter(|(_, ip)| !ip.is_empty())
                            .map(|(n, ip)| format!("{ip} ({n})"))
                            .collect();
                        ui.label(RichText::new(ips.join("\n")).monospace());
                        let (published, internal) = container_ports(c);
                        ui.label(RichText::new(published).monospace());
                        ui.label(RichText::new(internal).monospace().weak());
                        ui.end_row();
                    }
                });
        });
        ui.add_space(4.0);
    }
}

fn ports_in_use(ui: &mut egui::Ui, r: &HostReport, nginx_only: &mut bool) {
    let usage = r.port_usage();
    if usage.is_empty() {
        ui.label(RichText::new("No listening ports reported (ss/netstat not available)").weak());
        return;
    }
    let mut nginx_ports: Vec<u16> = usage
        .iter()
        .filter(|u| u.nginx && u.proto == "tcp")
        .map(|u| u.port)
        .collect();
    nginx_ports.dedup();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("nginx host ports:")
                .strong()
                .color(NGINX_COLOR),
        );
        if nginx_ports.is_empty() {
            ui.label(RichText::new("none").weak());
        } else {
            let list = nginx_ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(RichText::new(&list).monospace().color(NGINX_COLOR));
            if ui.small_button("copy").clicked() {
                ui.ctx().copy_text(list);
            }
        }
    });
    let total_tcp = usage.iter().filter(|u| u.proto == "tcp").count();
    ui.horizontal(|ui| {
        ui.label(format!(
            "{total_tcp} TCP · {} UDP ports in use",
            usage.len() - total_tcp
        ));
        ui.checkbox(nginx_only, "nginx only");
    });
    if !r.listeners.iter().any(|l| !l.process.is_empty()) {
        ui.label(
            RichText::new(
                "Process names need passwordless sudo on this host; only Docker-published ports show an owner.",
            )
            .small()
            .weak(),
        );
    }
    egui::Grid::new("ports")
        .striped(true)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for t in ["Port", "Proto", "Bind", "Used by", "Stack", "→ Container"] {
                ui.label(RichText::new(t).strong());
            }
            ui.end_row();
            for u in usage.iter().filter(|u| !*nginx_only || u.nginx) {
                let color = u.nginx.then_some(NGINX_COLOR);
                let txt = |t: String| {
                    let rt = RichText::new(t).monospace();
                    match color {
                        Some(c) => rt.color(c),
                        None => rt,
                    }
                };
                ui.label(txt(u.port.to_string()).strong());
                ui.label(txt(u.proto.clone()));
                ui.label(txt(u.addresses.join(", ")));
                ui.label(txt(if u.owner.is_empty() {
                    "—".into()
                } else {
                    u.owner.clone()
                }));
                ui.label(txt(u.stack.clone()));
                ui.label(txt(u
                    .container_port
                    .map_or(String::new(), |p| p.to_string())));
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_colours_and_fan_speed() {
        let blue = temp_color(45.0);
        let yellow = temp_color(75.0);
        let red = temp_color(90.0);
        assert!(blue.b() > blue.r() && red.r() > red.b() && yellow.g() > yellow.b());
        assert_eq!(temp_color(TEMP_WARM), yellow);
        assert_eq!(temp_color(TEMP_HOT), red);
        assert!(fan_turns_per_sec(1.0) > fan_turns_per_sec(0.5));
        assert!(fan_turns_per_sec(0.5) > fan_turns_per_sec(0.0));
        assert!(fan_turns_per_sec(0.0) > 0.0);
    }

    #[test]
    fn clone_copies_only_connection_settings_and_saves_as_new_host() {
        let src = HostEntry {
            id: Some(7),
            alias: "senar".into(),
            hostname: "192.0.2.32".into(),
            user: "admin".into(),
            port: 2222,
            identity_file: Some("~/.ssh/id_ed25519".into()),
            note: "VM01".into(),
            options: vec![("KexAlgorithms".into(), "+x".into())],
            unmonitored: false,
            access: Access::Ssh,
        };
        let mut ed = Editor::clone_of(&src);
        assert_eq!(ed.entry.id, None);
        assert_eq!(
            (ed.entry.user.as_str(), ed.port.as_str()),
            ("admin", "2222")
        );
        assert_eq!(ed.identity, "~/.ssh/id_ed25519");
        assert!(ed.entry.alias.is_empty() && ed.entry.hostname.is_empty());
        assert!(ed.entry.note.is_empty() && ed.options.is_empty());
        assert!(ed.build().is_err(), "alias is required");

        ed.entry.alias = "senar2".into();
        ed.entry.hostname = "192.0.2.34".into();
        let store = telepad_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        store.save_host(&src.clone()).ok();
        let first = store.hosts().unwrap().len();
        store.save_host(&ed.build().unwrap()).unwrap();
        let hosts = store.hosts().unwrap();
        assert_eq!(hosts.len(), first + 1);
        let new = hosts.iter().find(|h| h.alias == "senar2").unwrap();
        assert_eq!((new.user.as_str(), new.port), ("admin", 2222));
    }
}
