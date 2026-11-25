//! Executes [`Effect`]s and turns their results into [`Msg`]s. This is the
//! only GUI code that touches the database, ssh, the network, files, the
//! clipboard or the OS. Slow work (collection, update check/download) runs on
//! background threads; the rest is quick and runs inline.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use telepad_app::db::{DbEffect, DbId, DbJob, DbMsg};
use telepad_app::editor::{EdMsg, Load};
use telepad_app::files::{FileOp, FmEffect, FmId, FmMsg, Mode, Side};
use telepad_app::hostsync::{HsEffect, HsMsg};
use telepad_app::{Effect, Msg, PingJob, State};
use telepad_core::authorized_keys;
use telepad_core::collector::{self, Collector, Reach, SshSettings, reach};
use telepad_core::db::client::{CancelSlot, Session};
use telepad_core::db::{detect, secret, tunnel};
use telepad_core::history;
use telepad_core::prefs::{self, Prefs, WorkspacePrefs};
use telepad_core::store;
use telepad_core::strategy::{self, Strategy, UpdateDb};
use telepad_core::update::{self, Updater, relaunch};
use telepad_core::{Workspace, config, open_workspace, platform};

const WORKERS: usize = 12;

pub struct Runtime {
    ctx: egui::Context,
    ws: Option<Workspace>,
    ssh: Option<SshSettings>,
    collector: Option<(Collector, Receiver<collector::Update>)>,
    /// Results of background threads other than the collector.
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// One worker thread per database window; dropping it closes the connection.
    db: HashMap<DbId, DbWorker>,
    /// File windows: browse worker (one SFTP connection) and transfer cancel flags.
    files: HashMap<FmId, FileWorker>,
    /// `preference.db` writer (#94): writes off the UI thread, latest first.
    prefs: Option<(Sender<WorkspacePrefs>, thread::JoinHandle<()>)>,
    /// `history.db` (#100): one thread writes and reads it, in order.
    history: Option<(Sender<HistCmd>, thread::JoinHandle<()>)>,
    /// `update.db` (#122): one thread writes it, in order.
    strategy: Option<(Sender<StrategyCmd>, thread::JoinHandle<()>)>,
    /// `preference.db` / `history.db` stay readable by others (#110).
    permission_notes: Vec<String>,
}

/// Work for the `update.db` thread (#122).
enum StrategyCmd {
    Save(Strategy),
    Facts(Box<strategy::Facts>),
    Prune(Vec<String>),
    HostPaused(String, bool),
}

/// Work for the `history.db` thread (#100).
enum HistCmd {
    Add(Box<history::Entry>),
    Load(history::Scope, HistReply),
    Clear(history::Scope),
}

/// Which window gets the entries it asked for.
enum HistReply {
    Files(FmId),
    Db(DbId),
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

struct FileWorker {
    jobs: Sender<FileJob>,
    cancels: HashMap<u64, Arc<AtomicBool>>,
    /// Stops a running delete / permission change when the window closes.
    stop: Arc<AtomicBool>,
    host: telepad_core::config::HostEntry,
}

/// Work for a file window's worker thread (it holds the SFTP connection).
enum FileJob {
    List(Side, String),
    Op(Side, FileOp),
    /// Editor (#64): read a file / write it back.
    Load {
        tab: u64,
        side: Side,
        path: String,
        load: Load,
    },
    Save {
        tab: u64,
        side: Side,
        path: String,
        bytes: Vec<u8>,
        expect: Option<telepad_core::files::edit::Stamp>,
    },
}

/// Run one file operation on this computer or the host.
fn run_file_op(
    fs: &mut dyn telepad_core::files::ops::Fs,
    local: bool,
    op: FileOp,
    stop: &AtomicBool,
) -> Result<FmOpOut, String> {
    use telepad_core::files::ops::{self, Report};
    let one = |select: String| {
        (
            Report {
                done: 1,
                errors: vec![],
            },
            Some(select),
        )
    };
    Ok(match op {
        FileOp::NewFolder { dir, name } => {
            FmOpOut::Done(ops::new_folder(fs, &dir, &name, local).map(|_| one(name)))
        }
        FileOp::NewFile { dir, name } => {
            FmOpOut::Done(ops::new_file(fs, &dir, &name, local).map(|_| one(name)))
        }
        FileOp::Rename { dir, from, to } => {
            FmOpOut::Done(ops::rename(fs, &dir, &from, &to, local).map(|()| one(to)))
        }
        FileOp::Delete { dir, items } => {
            FmOpOut::Done(Ok((ops::delete(fs, &dir, &items, stop), None)))
        }
        FileOp::Chmod {
            dir,
            items,
            mode,
            recursive,
        } => FmOpOut::Done(Ok((
            ops::chmod(fs, &dir, &items, mode, recursive, stop),
            None,
        ))),
        FileOp::Measure { dir, items } => {
            FmOpOut::Measured(Ok(ops::measure(fs, &dir, &items, stop)))
        }
    })
}

/// The right-hand pane of a transfer: the host over SFTP, or this computer
/// for My Computer (#129).
fn open_remote(
    ssh: &SshSettings,
    host: &telepad_core::config::HostEntry,
) -> Result<Box<dyn telepad_core::files::transfer::Remote>, String> {
    use telepad_core::files::{sftp::Sftp, transfer::ThisComputer};
    if host.is_local() {
        return Ok(Box::new(ThisComputer));
    }
    Ok(Box::new(Sftp::connect(ssh, host)?))
}

type OpDoneResult = Result<(telepad_core::files::ops::Report, Option<String>), String>;

enum FmOpOut {
    Done(OpDoneResult),
    Measured(Result<Vec<telepad_core::files::ops::Measure>, String>),
}

struct DbWorker {
    jobs: Sender<DbJob>,
    cancel: CancelSlot,
}

impl Runtime {
    /// Open the database and start the collector; reports through `state`.
    pub fn start(
        ctx: egui::Context,
        state: &mut State,
        cli_config: Option<&str>,
        cli_db: Option<&str>,
        cli_prefs: Option<&str>,
        cli_history: Option<&str>,
        cli_update_db: Option<&str>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut rt = Self {
            ctx,
            ws: None,
            ssh: None,
            collector: None,
            tx,
            rx,
            db: HashMap::new(),
            files: HashMap::new(),
            prefs: None,
            history: None,
            strategy: None,
            permission_notes: Vec::new(),
        };
        match open_workspace(cli_config, cli_db) {
            Ok(ws) => {
                let ssh = ws.settings();
                let repaint = rt.ctx.clone();
                rt.collector = Some(Collector::start(
                    ssh.clone(),
                    WORKERS,
                    Arc::new(move || repaint.request_repaint()),
                ));
                let started = Msg::Started {
                    db_path: ws.store.path().to_path_buf(),
                    ssh_config_path: ws.ssh_config.clone(),
                    imported_from: ws.imported_from.clone(),
                    update_config: ws.store.update_config().map_err(|e| e.to_string()),
                    ui_prefs: ws.store.ui_prefs().unwrap_or_default(),
                    prefs: rt.open_prefs(ws.store.path(), cli_prefs, &ws.store),
                    strategy: rt.open_strategy(ws.store.path(), cli_update_db),
                };
                let history_error = rt.open_history(ws.store.path(), cli_history).err();
                // Database files others can read (#110): said once, not fatal.
                let mut notes: Vec<String> = ws
                    .store
                    .permission_warning()
                    .map(str::to_string)
                    .into_iter()
                    .collect();
                notes.append(&mut rt.permission_notes);
                rt.ssh = Some(ssh);
                rt.ws = Some(ws);
                rt.dispatch(state, started);
                if let Some(e) = history_error {
                    rt.dispatch(state, Msg::Notice(format!("History is not kept: {e}")));
                }
                for n in notes {
                    rt.dispatch(state, Msg::Notice(n));
                }
                if let Ok(alias) = std::env::var("TELEPAD_SELECT") {
                    rt.dispatch(state, Msg::SelectAlias(alias));
                }
            }
            Err(e) => rt.dispatch(state, Msg::Fatal(e)),
        }
        rt
    }

    /// Feed background results and the frame tick into `update`.
    pub fn frame(&mut self, state: &mut State) {
        let mut incoming = Vec::new();
        if let Some((_, rx)) = &self.collector {
            incoming.extend(rx.try_iter().map(Msg::Collected));
        }
        incoming.extend(self.rx.try_iter());
        for msg in incoming {
            self.dispatch(state, msg);
        }
        let busy = self.collector.as_ref().map_or(0, |(c, _)| c.busy());
        self.dispatch(state, Msg::Tick { busy });
    }

    /// Run `msg` through `update`, then its effects, then the messages those
    /// produce, until nothing is left.
    pub fn dispatch(&mut self, state: &mut State, msg: Msg) {
        let mut queue = VecDeque::from([msg]);
        while let Some(msg) = queue.pop_front() {
            for effect in telepad_app::update(state, msg, Instant::now()) {
                if let Some(next) = self.run(effect) {
                    queue.push_back(next);
                }
            }
        }
    }

    fn run(&mut self, effect: Effect) -> Option<Msg> {
        match effect {
            Effect::LoadHosts => {
                let ws = self.ws.as_ref()?;
                let ssh_config_error = ws
                    .sync_ssh_config()
                    .err()
                    .map(|e| format!("Cannot write {}: {e}", ws.ssh_config.display()));
                // Groups and clusters come with the hosts (#144, #145).
                let sets = telepad_core::config::SetKind::ALL
                    .iter()
                    .map(|&k| ws.store.sets(k).map(|v| (k, v)))
                    .collect::<Result<_, _>>()
                    .map_err(|e| e.to_string());
                let _ = self
                    .tx
                    .send(Msg::Sets(telepad_app::sets::StMsg::Loaded(sets)));
                // Remote Desktop settings come with the hosts (#102).
                let _ = self.tx.send(Msg::RdpLoaded(
                    ws.store.rdp_all().map_err(|e| e.to_string()),
                ));
                Some(Msg::HostsLoaded {
                    hosts: ws.store.hosts().map_err(|e| e.to_string()),
                    ssh_config_error,
                })
            }
            Effect::Collect(hosts) => {
                if let Some((c, _)) = &self.collector {
                    c.enqueue(hosts);
                }
                None
            }
            Effect::SaveHost(entry) => {
                let ws = self.ws.as_ref()?;
                Some(Msg::HostSaved(
                    ws.store
                        .save_host(&entry)
                        .map(|id| (id, entry.alias.clone()))
                        .map_err(|e| e.to_string()),
                ))
            }
            Effect::DeleteHost { id, alias } => {
                let ws = self.ws.as_ref()?;
                Some(Msg::HostDeleted {
                    id,
                    alias,
                    result: ws.store.delete_host(id).map_err(|e| e.to_string()),
                })
            }
            Effect::PickImportFile => {
                let path = rfd::FileDialog::new()
                    .set_title("Import hosts (OpenSSH config or JSON host list)")
                    .add_filter("All files", &["*"])
                    .add_filter("JSON", &["json"])
                    .pick_file()?;
                Some(read_import(path))
            }
            Effect::Sets(e) => {
                use telepad_app::sets::{StEffect, StMsg};
                let ws = self.ws.as_mut()?;
                let r = match e {
                    StEffect::Save { kind, old, set } => {
                        ws.store.save_set(kind, old.as_deref(), &set)
                    }
                    StEffect::Delete { kind, name } => ws.store.delete_set(kind, &name),
                    StEffect::Order { kind, names } => ws.store.order_sets(kind, &names),
                    StEffect::Assign { kind, ids, name } => ws.store.assign_set(kind, &ids, &name),
                };
                // The generated ssh config carries the names as comments.
                let r = r
                    .map_err(|e| e.to_string())
                    .and_then(|()| ws.sync_ssh_config());
                Some(Msg::Sets(StMsg::Saved(r)))
            }
            Effect::SaveOrder(ids) => {
                let ws = self.ws.as_mut()?;
                let saved = ws.store.save_order(&ids).map_err(|e| e.to_string());
                Some(Msg::OrderSaved(saved.and_then(|()| ws.sync_ssh_config())))
            }
            Effect::ReadClipboard => Some(Msg::ClipboardRead(
                arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .map_err(|e| e.to_string()),
            )),
            Effect::ReadUserSshConfig => {
                Some(read_import(telepad_core::expand_home("~/.ssh/config")))
            }
            Effect::PreviewImport { inventory, mode } => Some(Msg::ImportPreviewed(
                self.with_store(|s| s.preview_import(&inventory, mode)),
            )),
            Effect::ApplyImport { inventory, mode } => {
                let ws = self.ws.as_mut()?;
                Some(Msg::Imported(
                    ws.store.import(&inventory, mode).map_err(|e| e.to_string()),
                ))
            }
            Effect::PickExportFile => {
                let ws = self.ws.as_ref()?;
                let path = rfd::FileDialog::new()
                    .set_title("Export hosts")
                    .set_file_name("telepad-hosts.json")
                    .add_filter("JSON (re-importable, keeps everything)", &["json"])
                    .add_filter("OpenSSH config", &["config", "conf", "txt"])
                    .save_file()?;
                Some(Msg::Exported(export(ws, &path).map(|n| (n, path))))
            }
            Effect::PickKeyFile(current) => {
                // Start where keys live: ~/.ssh is hidden on macOS/Linux, so a
                // dialog opened in the home folder would not show it.
                let current = telepad_core::expand_home(current.trim());
                let start = current
                    .parent()
                    .filter(|d| !current.as_os_str().is_empty() && d.is_dir())
                    .map(Path::to_path_buf)
                    .or_else(|| Some(telepad_core::expand_home("~/.ssh")).filter(|d| d.is_dir()));
                let mut dialog = rfd::FileDialog::new().set_title("Private key");
                if let Some(dir) = start {
                    dialog = dialog.set_directory(dir);
                }
                dialog.pick_file().map(Msg::KeyPicked)
            }
            Effect::FindKeys => Some(Msg::KeysFound(telepad_core::find_ssh_keys())),
            Effect::OpenSsh(h) => {
                let s = self.ssh.as_ref()?;
                let text = match platform::open_terminal(&s.ssh_program, &s.args(&h, false)) {
                    Ok(()) => format!("Opened SSH session to {}", h.alias),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
            Effect::OpenRdp(h) => {
                let host = if h.hostname.is_empty() {
                    &h.alias
                } else {
                    &h.hostname
                };
                // RDP-only hosts keep their RDP port in `port`.
                let port = if h.access == telepad_core::config::Access::Rdp {
                    h.port
                } else {
                    3389
                };
                let saved = self
                    .ws
                    .as_ref()
                    .and_then(|ws| ws.store.rdp_all().ok())
                    .and_then(|mut m| m.remove(&h.alias));
                let text = match saved {
                    Some(r) => match platform::open_rdp_with(&r, host, port, &h.alias) {
                        Ok(not) if not.is_empty() => {
                            format!("Opened Remote Desktop to {} with its settings", h.alias)
                        }
                        Ok(not) => format!(
                            "Opened Remote Desktop to {}; not available in this client: {}",
                            h.alias,
                            not.join(", ")
                        ),
                        Err(e) => e,
                    },
                    None => match platform::open_rdp(host, port) {
                        Ok(()) => format!("Opened Remote Desktop to {}", h.alias),
                        Err(e) => e,
                    },
                };
                Some(Msg::Notice(text))
            }
            Effect::SaveRdp { alias, settings } => {
                let ws = self.ws.as_ref()?;
                let r = ws
                    .store
                    .set_rdp(&alias, settings.as_ref())
                    .map_err(|e| e.to_string());
                let _ = self.tx.send(Msg::RdpLoaded(
                    ws.store.rdp_all().map_err(|e| e.to_string()),
                ));
                Some(Msg::RdpSaved(r))
            }
            Effect::RdpCredCheck { address } => Some(Msg::RdpCredState {
                saved: platform::rdp_credential_saved(&address),
                address,
                result: Ok(String::new()),
            }),
            Effect::RdpCredSave {
                address,
                user,
                password,
            } => {
                let r = platform::save_rdp_credential(&address, &user, password.expose())
                    .map(|()| format!("Saved in Windows Credential Manager for {user}"));
                Some(Msg::RdpCredState {
                    saved: platform::rdp_credential_saved(&address),
                    address,
                    result: r,
                })
            }
            Effect::RdpCredForget { address } => {
                let r = platform::forget_rdp_credential(&address)
                    .map(|()| "Removed from Windows Credential Manager".to_string());
                Some(Msg::RdpCredState {
                    saved: platform::rdp_credential_saved(&address),
                    address,
                    result: r,
                })
            }
            Effect::PickRdpFile => {
                let path = rfd::FileDialog::new()
                    .set_title("Remote Desktop file")
                    .add_filter("Remote Desktop", &["rdp"])
                    .pick_file()?;
                let r = std::fs::metadata(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|m| {
                        if m.len() > 1 << 20 {
                            Err("too large for a .rdp file".to_string())
                        } else {
                            std::fs::read(&path).map_err(|e| e.to_string())
                        }
                    })
                    .map(|b| {
                        telepad_core::rdp::RdpSettings::parse(&telepad_core::rdp::decode_file(&b))
                    })
                    .map_err(|e| format!("Cannot read {}: {e}", path.display()));
                Some(Msg::RdpPicked(r))
            }
            Effect::ExportRdp { alias, text } => {
                let path = rfd::FileDialog::new()
                    .set_title("Save Remote Desktop file")
                    .set_file_name(format!("{alias}.rdp"))
                    .add_filter("Remote Desktop", &["rdp"])
                    .save_file()?;
                Some(Msg::Notice(
                    match std::fs::write(&path, telepad_core::rdp::encode_file(&text)) {
                        Ok(()) => format!("Saved {}", path.display()),
                        Err(e) => format!("Cannot write {}: {e}", path.display()),
                    },
                ))
            }
            Effect::OpenBmcSetup(h) => {
                let s = self.ssh.as_ref()?;
                let args = collector::bmc_setup_args(s, &h);
                let text = match platform::open_terminal(&s.ssh_program, &args) {
                    Ok(()) => format!(
                        "Opened BMC setup for {}: press Enter there, type your sudo password, then Refresh",
                        h.alias
                    ),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
            Effect::OpenDockerAction(h, plan) => {
                let s = self.ssh.as_ref()?;
                let args = collector::docker_action_args(s, &h, &plan);
                let text = match platform::open_terminal(&s.ssh_program, &args) {
                    Ok(()) => format!(
                        "Opened a terminal on {}: {} — runs after you confirm there",
                        h.alias,
                        plan.display()
                    ),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
            Effect::FetchRunInfo(h, c) => {
                let s = self.ssh.clone()?;
                self.background(move || Msg::RunInfoLoaded(collector::fetch_run_info(&s, &h, &c)));
                None
            }
            Effect::CopySshCommand(h) => {
                let s = self.ssh.as_ref()?;
                self.ctx
                    .copy_text(platform::command_line(&s.ssh_program, &s.args(&h, false)));
                Some(Msg::Notice("SSH command copied to clipboard".into()))
            }
            Effect::CheckUpdate(cfg) => {
                self.background(move || Msg::UpdateChecked(Updater::new(cfg).check()));
                None
            }
            Effect::InstallUpdate(check) => {
                self.background(move || Msg::UpdateInstalled(update::install(&check)));
                None
            }
            Effect::SaveUpdateConfig(cfg) => Some(Msg::UpdateSettingsSaved(
                self.with_store(|s| s.set_update_config(&cfg)).map(|()| cfg),
            )),
            Effect::SavePrefs(p) => match &self.prefs {
                Some((w, _)) if w.send(p).is_ok() => None,
                _ => Some(Msg::PrefsSaved(Err("preference.db is not open".into()))),
            },
            Effect::SaveStrategy(s) => self.strategy_send(StrategyCmd::Save(s)),
            Effect::SaveFacts(f) => self.strategy_send(StrategyCmd::Facts(Box::new(f))),
            Effect::PruneFacts(keep) => self.strategy_send(StrategyCmd::Prune(keep)),
            Effect::SaveHostPaused { alias, paused } => {
                self.strategy_send(StrategyCmd::HostPaused(alias, paused))
            }
            Effect::Ping(hosts) => {
                self.background(move || Msg::Pinged(ping_all(&hosts)));
                None
            }
            Effect::SaveUiPrefs(p) => {
                Some(Msg::UiPrefsSaved(self.with_store(|s| s.set_ui_prefs(&p))))
            }
            Effect::ListKeys(hosts) => {
                let s = self.ssh.clone()?;
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    authorized_keys::list_all(&s, &hosts, WORKERS, |alias, result| {
                        let _ = tx.send(Msg::AkListed { alias, result });
                        ctx.request_repaint();
                    });
                });
                None
            }
            Effect::ApplyKey {
                action,
                key,
                hosts,
                group,
            } => {
                let s = self.ssh.clone()?;
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    authorized_keys::apply_all(
                        &s,
                        &hosts,
                        action,
                        &key,
                        group.as_deref(),
                        WORKERS,
                        |alias, outcome| {
                            let _ = tx.send(Msg::KeyResult { alias, outcome });
                            ctx.request_repaint();
                        },
                    );
                });
                None
            }
            Effect::Db(e) => self.run_db(e),
            Effect::HostSync(e) => self.run_hostsync(e),
            Effect::MyKeys(e) => self.run_mykeys(e),
            Effect::Files(e) => self.run_files(e),
            Effect::Logins(e) => self.run_logins(e),
            Effect::CollectLocal => {
                self.background(|| {
                    let start = Instant::now();
                    let result = telepad_core::local::collect().map(Box::new);
                    Msg::LocalCollected {
                        result,
                        elapsed: start.elapsed(),
                    }
                });
                None
            }
            Effect::OpenLocalTerminal => {
                Some(Msg::Notice(match telepad_core::local::open_terminal() {
                    Ok(()) => "Opened a terminal on this computer".into(),
                    Err(e) => e,
                }))
            }
            Effect::CopyText { text, notice } => {
                self.ctx.copy_text(text);
                Some(Msg::Notice(notice))
            }
            Effect::ExportTable { data, format } => {
                use telepad_core::table_export::{self as te};
                let now = unix_now();
                let path = rfd::FileDialog::new()
                    .set_title(format!("Export {} as {}", data.title, format.title()))
                    .set_file_name(te::file_name(&data.title, now, format))
                    .add_filter(format.title(), &[format.ext()])
                    .save_file()?;
                let fonts = pdf_fonts();
                self.background(move || {
                    let generated = format!("Telepad · {} UTC", history::format_utc(now));
                    let refs: Vec<&[u8]> = fonts.iter().map(|f| f.as_ref()).collect();
                    let r = te::render(&data, format, &refs, &generated)
                        .and_then(|b| te::write(&path, &b))
                        .map(|()| (data.rows.len(), path));
                    Msg::Table(telepad_app::grid::TableMsg::Exported(r))
                });
                None
            }
            Effect::OpenUrl(url) => platform::open_url(&url).err().map(Msg::Notice),
            Effect::Relaunch => match relaunch() {
                Ok(()) => {
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    None
                }
                Err(e) => Some(Msg::Notice(e)),
            },
        }
    }

    /// Close every database connection and its SSH tunnel (app exit).
    /// Open `preference.db`, bring in the old `ui_workspace` layout once,
    /// read the workspaces and start its writer thread.
    fn open_prefs(
        &mut self,
        db: &Path,
        cli: Option<&str>,
        store: &store::Store,
    ) -> Result<WorkspacePrefs, String> {
        let path = prefs::prefs_path(db, cli);
        let p = Prefs::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.permission_notes
            .extend(p.permission_warning().map(str::to_string));
        let legacy = store.ui_prefs().map(|u| u.workspace).unwrap_or_default();
        p.import_legacy(&legacy).map_err(|e| e.to_string())?;
        let loaded = p.load().map_err(|e| e.to_string())?;
        let (wtx, wrx) = mpsc::channel::<WorkspacePrefs>();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let handle = thread::spawn(move || {
            while let Ok(mut next) = wrx.recv() {
                // Only the newest state matters.
                while let Ok(newer) = wrx.try_recv() {
                    next = newer;
                }
                if let Err(e) = p.save(&next) {
                    let _ = tx.send(Msg::PrefsSaved(Err(e.to_string())));
                    ctx.request_repaint();
                }
            }
        });
        self.prefs = Some((wtx, handle));
        Ok(loaded)
    }

    /// Open `update.db` (#122), read the strategy and the last known facts,
    /// and start its writer thread.
    fn open_strategy(&mut self, db: &Path, cli: Option<&str>) -> Result<strategy::Loaded, String> {
        let path = strategy::strategy_path(db, cli);
        let u = UpdateDb::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.permission_notes
            .extend(u.permission_warning().map(str::to_string));
        let loaded = u.load(unix_now()).map_err(|e| e.to_string())?;
        let (stx, srx) = mpsc::channel::<StrategyCmd>();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let handle = thread::spawn(move || {
            let mut failed = false;
            while let Ok(cmd) = srx.recv() {
                let r = match cmd {
                    StrategyCmd::Save(s) => u.save_strategy(&s),
                    StrategyCmd::Facts(f) => u.save_facts(&f, unix_now()),
                    StrategyCmd::Prune(keep) => u.prune(&keep),
                    StrategyCmd::HostPaused(alias, paused) => u.set_host_paused(&alias, paused),
                };
                // Said once: the same problem would repeat on every collection.
                if let Err(e) = r
                    && !failed
                {
                    failed = true;
                    let _ = tx.send(Msg::Notice(format!("Cannot write update.db: {e}")));
                    ctx.request_repaint();
                }
            }
        });
        self.strategy = Some((stx, handle));
        Ok(loaded)
    }

    fn strategy_send(&self, cmd: StrategyCmd) -> Option<Msg> {
        let (tx, _) = self.strategy.as_ref()?;
        tx.send(cmd)
            .err()
            .map(|_| Msg::Notice("update.db is not open".into()))
    }

    /// On exit: wait until the `update.db` thread wrote everything sent to it.
    pub fn flush_strategy(&mut self) {
        if let Some((tx, handle)) = self.strategy.take() {
            drop(tx);
            let _ = handle.join();
        }
    }

    /// Open `history.db` (trimmed to 90 days / 50 000 entries) and start
    /// its thread.
    fn open_history(&mut self, db: &Path, cli: Option<&str>) -> Result<(), String> {
        let path = history::history_path(db, cli);
        let h = history::History::open(&path, unix_now())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.permission_notes
            .extend(h.permission_warning().map(str::to_string));
        let (htx, hrx) = mpsc::channel::<HistCmd>();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let handle = thread::spawn(move || {
            let mut failed = false;
            while let Ok(cmd) = hrx.recv() {
                match cmd {
                    HistCmd::Add(e) => {
                        if let Err(e) = h.add(&e, unix_now())
                            && !failed
                        {
                            failed = true;
                            let _ = tx.send(Msg::Notice(format!("Cannot write the history: {e}")));
                            ctx.request_repaint();
                        }
                    }
                    HistCmd::Load(scope, reply) => {
                        let result = h
                            .list(&scope, history::LOAD_LIMIT)
                            .map_err(|e| e.to_string());
                        let msg = match reply {
                            HistReply::Files(id) => Msg::Files(FmMsg::HistoryLoaded { id, result }),
                            HistReply::Db(id) => Msg::Db(DbMsg::LogLoaded { id, result }),
                        };
                        let _ = tx.send(msg);
                        ctx.request_repaint();
                    }
                    HistCmd::Clear(scope) => {
                        if let Err(e) = h.clear(&scope) {
                            let _ = tx.send(Msg::Notice(format!("Cannot clear the history: {e}")));
                            ctx.request_repaint();
                        }
                    }
                }
            }
        });
        self.history = Some((htx, handle));
        Ok(())
    }

    fn history_send(&self, cmd: HistCmd) -> Option<Msg> {
        let (tx, _) = self.history.as_ref()?;
        let _ = tx.send(cmd);
        None
    }

    /// Ask for a window's history; without `history.db` the window says so.
    fn history_load(&self, scope: history::Scope, reply: HistReply) -> Option<Msg> {
        if self.history.is_some() {
            return self.history_send(HistCmd::Load(scope, reply));
        }
        let result = Err("history.db is not open".to_string());
        Some(match reply {
            HistReply::Files(id) => Msg::Files(FmMsg::HistoryLoaded { id, result }),
            HistReply::Db(id) => Msg::Db(DbMsg::LogLoaded { id, result }),
        })
    }

    /// Save history entries as CSV where the user says (native dialog).
    fn history_export(entries: &[history::Entry], name: &str) -> Option<Msg> {
        let path = rfd::FileDialog::new()
            .set_title("Export history")
            .set_file_name(name)
            .add_filter("CSV", &["csv"])
            .save_file()?;
        Some(Msg::Notice(
            match std::fs::write(&path, history::to_csv(entries)) {
                Ok(()) => format!("{} entries written to {}", entries.len(), path.display()),
                Err(e) => format!("Cannot write {}: {e}", path.display()),
            },
        ))
    }

    /// My keys tab (#101): scan, ssh-keygen and conversions run on a thread;
    /// the save / open dialogs here (modal by nature).
    fn run_mykeys(&mut self, e: telepad_app::mykeys::MkEffect) -> Option<Msg> {
        use telepad_app::mykeys::{MkEffect, MkMsg};
        use telepad_core::local_keys;
        let send = |tx: Sender<Msg>, ctx: egui::Context, m: MkMsg| {
            let _ = tx.send(Msg::MyKeys(m));
            ctx.request_repaint();
        };
        match e {
            MkEffect::Scan { extra } => {
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    let dir = telepad_core::expand_home("~/.ssh");
                    let extra: Vec<std::path::PathBuf> = extra
                        .iter()
                        .map(|f| telepad_core::expand_home(f.trim()))
                        .collect();
                    let keys = local_keys::scan(&dir, &extra);
                    send(
                        tx,
                        ctx,
                        MkMsg::Loaded {
                            dir,
                            user_host: telepad_core::platform::user_at_host(),
                            result: Ok(keys),
                        },
                    );
                });
                None
            }
            MkEffect::Generate {
                kind,
                path,
                comment,
                pass,
            } => {
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    let r = local_keys::generate(kind, &path, &comment, pass.expose());
                    if let Ok((_, Some(w))) = &r {
                        let _ = tx.send(Msg::Notice(format!(
                            "Key created, but its permissions could not be narrowed: {w}"
                        )));
                    }
                    send(tx, ctx, MkMsg::Generated(r.map(|(p, _)| p)));
                });
                None
            }
            MkEffect::Convert {
                source,
                pass_in,
                target,
                pass_out,
                suggest,
                user_id,
            } => {
                let mut dialog =
                    rfd::FileDialog::new().set_title(format!("Save as {}", target.label()));
                if let Some(dir) = suggest.parent().filter(|d| d.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(name) = suggest.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                let Some(dest) = dialog.save_file() else {
                    return Some(Msg::MyKeys(MkMsg::Converted(Err("Not saved".into()))));
                };
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    let r = local_keys::convert_file(
                        &source,
                        pass_in.expose(),
                        target,
                        pass_out.expose(),
                        &dest,
                        &user_id,
                    );
                    send(tx, ctx, MkMsg::Converted(r));
                });
                None
            }
            MkEffect::PickImport { dir } => {
                let mut dialog = rfd::FileDialog::new().set_title("Import a key file");
                if dir.is_dir() {
                    dialog = dialog.set_directory(&dir);
                }
                let path = dialog.pick_file()?;
                Some(Msg::MyKeys(MkMsg::Picked(Some(local_keys::inspect(&path)))))
            }
            MkEffect::CopyText(text) => {
                if let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
                    return Some(Msg::Notice(format!("Could not copy: {e}")));
                }
                Some(Msg::Notice("Public key copied".into()))
            }
            MkEffect::OpenFolder(path) => telepad_core::platform::open_folder(&path)
                .err()
                .map(Msg::Notice),
        }
    }

    /// On exit: wait until the history thread wrote everything sent to it.
    pub fn flush_history(&mut self) {
        if let Some((tx, handle)) = self.history.take() {
            drop(tx);
            let _ = handle.join();
        }
    }

    /// On exit: write workspaces that are still waiting for the autosave,
    /// and wait for the writer.
    pub fn flush_prefs(&mut self, m: &telepad_app::Model) {
        if let Some((w, handle)) = self.prefs.take() {
            if m.prefs_dirty && m.prefs_ok {
                let _ = w.send(m.wss.clone());
            }
            drop(w);
            let _ = handle.join();
        }
    }

    pub fn shutdown(&mut self) {
        for (_, w) in self.db.drain() {
            cancel(&w.cancel);
        }
        for (_, w) in self.files.drain() {
            w.stop.store(true, Ordering::Relaxed);
            for c in w.cancels.values() {
                c.store(true, Ordering::Relaxed);
            }
        }
        tunnel::close_all();
    }

    fn run_files(&mut self, e: FmEffect) -> Option<Msg> {
        use telepad_core::files::{self as cf, fallback, rsync as crsync, sftp::Sftp, transfer};
        let fm = |m: FmMsg| Msg::Files(m);
        match e {
            FmEffect::Log(entry) => self.history_send(HistCmd::Add(Box::new(entry))),
            FmEffect::LoadHistory { id, alias } => {
                self.history_load(history::Scope::Files(alias), HistReply::Files(id))
            }
            FmEffect::ClearHistory { alias } => {
                self.history_send(HistCmd::Clear(history::Scope::Files(alias)))
            }
            FmEffect::ExportHistory(entries) => {
                Self::history_export(&entries, "telepad-file-history.csv")
            }
            FmEffect::OpenFolder(path) => telepad_core::platform::open_folder(Path::new(&path))
                .err()
                .map(Msg::Notice),
            FmEffect::Connect { id, host } => {
                let ssh = self.ssh.clone()?;
                let remembered = self
                    .ws
                    .as_ref()
                    .and_then(|ws| ws.store.files_paths(&host.alias).ok().flatten());
                // Transfer settings of this host (#131).
                if let Some(o) = self
                    .ws
                    .as_ref()
                    .and_then(|ws| ws.store.files_options(&host.alias).ok().flatten())
                {
                    let _ = self.tx.send(fm(FmMsg::OptionsLoaded {
                        id,
                        opts: telepad_core::files::transfer::Options::from_setting(&o),
                    }));
                }
                let (jobs, inbox) = mpsc::channel::<FileJob>();
                let stop = Arc::new(AtomicBool::new(false));
                if let Some(old) = self.files.insert(
                    id,
                    FileWorker {
                        jobs,
                        cancels: HashMap::new(),
                        stop: stop.clone(),
                        host: host.clone(),
                    },
                ) {
                    old.stop.store(true, Ordering::Relaxed);
                    for c in old.cancels.values() {
                        c.store(true, Ordering::Relaxed);
                    }
                }
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                let (h2, s2) = (host.clone(), ssh.clone());
                let both_local = host.is_local();
                self.background(move || {
                    fm(FmMsg::RsyncChecked {
                        id,
                        result: if both_local {
                            Err("not used between two folders of this computer".into())
                        } else {
                            crsync::local_available()
                                .and_then(|()| crsync::remote_available(&s2, &h2))
                        },
                    })
                });
                thread::spawn(move || {
                    let send = |m: Msg| {
                        let _ = tx.send(m);
                        ctx.request_repaint();
                    };
                    let local_home = cf::local_home();
                    // SFTP first; hosts without an SFTP server get the sh / scp fallback.
                    let mut sftp = None;
                    let result = if both_local {
                        // My Computer (#129): this computer on both sides.
                        Ok((Mode::Local, local_home.clone(), remembered))
                    } else {
                        match Sftp::connect(&ssh, &host).and_then(|mut c| {
                            let home = c.realpath(".")?;
                            Ok((c, home))
                        }) {
                            Ok((c, home)) => {
                                sftp = Some(c);
                                Ok((Mode::Sftp, home, remembered))
                            }
                            Err(e) if e.contains("no SFTP server") => {
                                fallback::list(&ssh, &host, "~")
                                    .map(|(home, _)| (Mode::Fallback, home, remembered))
                            }
                            Err(e) => Err(e),
                        }
                    };
                    let ok = result.is_ok();
                    send(fm(FmMsg::Connected {
                        id,
                        result,
                        local_home,
                    }));
                    if !ok {
                        return;
                    }
                    use telepad_core::files::{edit, text};
                    // Which computer a pane's job runs on: the right pane is
                    // this computer too for My Computer.
                    let on = |side: Side| if both_local { Side::Local } else { side };
                    for job in inbox {
                        let (side, path) = match job {
                            FileJob::List(side, path) => (side, path),
                            FileJob::Load {
                                tab,
                                side,
                                path,
                                load,
                            } => {
                                let read = match (on(side), sftp.as_mut()) {
                                    (Side::Local, _) => edit::read_local(&path),
                                    (Side::Remote, Some(c)) => edit::read_remote(c, &path),
                                    (Side::Remote, None) => Err("Needs SFTP on the host".into()),
                                };
                                let result = read.and_then(|(bytes, stamp)| {
                                    text::decode(&bytes).map(|doc| (doc, stamp))
                                });
                                send(fm(FmMsg::Editor {
                                    id,
                                    msg: EdMsg::Loaded { tab, load, result },
                                }));
                                continue;
                            }
                            FileJob::Save {
                                tab,
                                side,
                                path,
                                bytes,
                                expect,
                            } => {
                                let result = match (on(side), sftp.as_mut()) {
                                    (Side::Local, _) => edit::save_local(&path, &bytes, expect),
                                    (Side::Remote, Some(c)) => {
                                        edit::save_remote(c, &path, &bytes, expect)
                                    }
                                    (Side::Remote, None) => Err(edit::SaveError::Failed(
                                        "Needs SFTP on the host".into(),
                                    )),
                                };
                                send(fm(FmMsg::Editor {
                                    id,
                                    msg: EdMsg::Saved { tab, result },
                                }));
                                continue;
                            }
                            FileJob::Op(side, op) => {
                                let out = match (on(side), sftp.as_mut()) {
                                    (Side::Local, _) => run_file_op(
                                        &mut telepad_core::files::ops::LocalFs,
                                        true,
                                        op,
                                        &stop,
                                    ),
                                    (Side::Remote, Some(c)) => run_file_op(c, false, op, &stop),
                                    (Side::Remote, None) => {
                                        Err("Needs SFTP on the host".to_string())
                                    }
                                };
                                send(fm(match out {
                                    Ok(FmOpOut::Measured(result)) => {
                                        FmMsg::Measured { id, side, result }
                                    }
                                    Ok(FmOpOut::Done(result)) => FmMsg::OpDone { id, side, result },
                                    Err(e) => FmMsg::OpDone {
                                        id,
                                        side,
                                        result: Err(e),
                                    },
                                }));
                                continue;
                            }
                        };
                        let result = match on(side) {
                            Side::Local => {
                                let p =
                                    telepad_core::expand_home(path.trim()).display().to_string();
                                cf::list_local_at(&p)
                            }
                            Side::Remote => match sftp.as_mut() {
                                Some(c) => c
                                    .realpath(&path)
                                    .and_then(|abs| c.list(&abs).map(|v| (abs, v))),
                                None => fallback::list(&ssh, &host, &path),
                            },
                        };
                        send(fm(FmMsg::Listed { id, side, result }));
                    }
                });
                None
            }
            FmEffect::List { id, side, path } => match self.files.get(&id) {
                Some(w) => w.jobs.send(FileJob::List(side, path)).err().map(|_| {
                    fm(FmMsg::Listed {
                        id,
                        side,
                        result: Err("Not connected — click Retry".into()),
                    })
                }),
                None => None,
            },
            FmEffect::Plan {
                id,
                transfer: tid,
                direction,
                sources,
                dest,
                opts,
            } => {
                let (ssh, cancel, host) = self.transfer_setup(id, tid)?;
                self.background(move || {
                    let result = open_remote(&ssh, &host).and_then(|mut c| {
                        transfer::plan(c.as_mut(), direction, &sources, &dest, &opts, &cancel)
                    });
                    fm(FmMsg::Planned {
                        id,
                        transfer: tid,
                        result,
                    })
                });
                None
            }
            FmEffect::Run {
                id,
                transfer: tid,
                direction,
                items,
                decisions,
                opts,
            } => {
                let (ssh, cancel, host) = self.transfer_setup(id, tid)?;
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    let send = |m: Msg| {
                        let _ = tx.send(m);
                        ctx.request_repaint();
                    };
                    let mut c = match open_remote(&ssh, &host) {
                        Ok(c) => c,
                        Err(e) => {
                            send(fm(FmMsg::Finished {
                                id,
                                transfer: tid,
                                result: Err(e),
                            }));
                            return;
                        }
                    };
                    // Bytes are summed and sent a few times a second.
                    let mut pending = 0u64;
                    let mut last = Instant::now();
                    let counts = transfer::run(
                        c.as_mut(),
                        direction,
                        &items,
                        &decisions,
                        &opts,
                        &cancel,
                        &mut |ev| match ev {
                            transfer::Event::Bytes(n) => {
                                pending += n;
                                if last.elapsed() < Duration::from_millis(150) {
                                    return;
                                }
                                last = Instant::now();
                                let n = std::mem::take(&mut pending);
                                send(fm(FmMsg::Progress {
                                    id,
                                    transfer: tid,
                                    event: transfer::Event::Bytes(n),
                                }));
                            }
                            other => {
                                if pending > 0 {
                                    let n = std::mem::take(&mut pending);
                                    send(fm(FmMsg::Progress {
                                        id,
                                        transfer: tid,
                                        event: transfer::Event::Bytes(n),
                                    }));
                                }
                                send(fm(FmMsg::Progress {
                                    id,
                                    transfer: tid,
                                    event: other,
                                }));
                            }
                        },
                    );
                    let result = if cancel.load(Ordering::Relaxed) {
                        Err("Cancelled".to_string())
                    } else {
                        Ok(counts)
                    };
                    send(fm(FmMsg::Finished {
                        id,
                        transfer: tid,
                        result,
                    }));
                });
                None
            }
            FmEffect::Scp {
                id,
                transfer: tid,
                direction,
                sources,
                dest,
            } => {
                let (ssh, cancel, host) = self.transfer_setup(id, tid)?;
                self.background(move || {
                    let (mut copied, mut failed, mut last_err) = (0, 0, None);
                    for (src, _) in &sources {
                        let r = match direction {
                            transfer::Direction::Upload => {
                                fallback::copy(&ssh, &host, true, src, &dest, &cancel)
                            }
                            transfer::Direction::Download => {
                                fallback::copy(&ssh, &host, false, &dest, src, &cancel)
                            }
                        };
                        match r {
                            Ok(()) => copied += 1,
                            Err(e) => {
                                failed += 1;
                                last_err = Some(e);
                            }
                        }
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                    let result = match (cancel.load(Ordering::Relaxed), copied, last_err) {
                        (true, _, _) => Err("Cancelled".into()),
                        (false, 0, Some(e)) => Err(e),
                        _ => Ok((copied, 0, failed)),
                    };
                    fm(FmMsg::Finished {
                        id,
                        transfer: tid,
                        result,
                    })
                });
                None
            }
            FmEffect::Rsync {
                id,
                transfer: tid,
                direction,
                sources,
                dest,
                dry_run,
                opts,
            } => {
                let (ssh, _cancel, host) = self.transfer_setup(id, tid)?;
                self.background(move || {
                    let upload = direction == transfer::Direction::Upload;
                    let result = crsync::run(&crsync::args(
                        &ssh, &host, upload, &sources, &dest, dry_run, &opts,
                    ));
                    if dry_run {
                        fm(FmMsg::RsyncPreviewed {
                            id,
                            transfer: tid,
                            result,
                        })
                    } else {
                        fm(FmMsg::Finished {
                            id,
                            transfer: tid,
                            result: result.map(|changed| (changed.len(), 0, 0)),
                        })
                    }
                });
                None
            }
            FmEffect::Cancel { id, transfer: tid } => {
                if let Some(c) = self.files.get(&id).and_then(|w| w.cancels.get(&tid)) {
                    c.store(true, Ordering::Relaxed);
                }
                None
            }
            FmEffect::Op { id, side, op } => match self.files.get(&id) {
                Some(w) => w.jobs.send(FileJob::Op(side, op)).err().map(|_| {
                    fm(FmMsg::OpDone {
                        id,
                        side,
                        result: Err("Not connected — click Retry".into()),
                    })
                }),
                None => None,
            },
            FmEffect::Load {
                id,
                tab,
                side,
                path,
                load,
            } => self.files.get(&id).and_then(|w| {
                w.jobs
                        .send(FileJob::Load {
                            tab,
                            side,
                            path,
                            load,
                        })
                        .err()
                        .map(|_| {
                            fm(FmMsg::Editor {
                                id,
                                msg: EdMsg::Loaded {
                                    tab,
                                    load,
                                    result: Err(
                                        "Not connected — click Retry in the file window".into()
                                    ),
                                },
                            })
                        })
            }),
            FmEffect::Save {
                id,
                tab,
                side,
                path,
                bytes,
                expect,
            } => self.files.get(&id).and_then(|w| {
                w.jobs
                    .send(FileJob::Save {
                        tab,
                        side,
                        path,
                        bytes,
                        expect,
                    })
                    .err()
                    .map(|_| {
                        fm(FmMsg::Editor {
                            id,
                            msg: EdMsg::Saved {
                                tab,
                                result: Err(telepad_core::files::edit::SaveError::Failed(
                                    "Not connected — click Retry in the file window".into(),
                                )),
                            },
                        })
                    })
            }),
            FmEffect::CopyText(text) => {
                let n = text.lines().count();
                // Straight to the OS clipboard: egui's copy only reaches it on
                // the next frame of the viewport that asked, which may not come.
                if let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
                    return Some(Msg::Notice(format!("Could not copy: {e}")));
                }
                Some(Msg::Notice(if n > 1 {
                    format!("{n} paths copied to clipboard")
                } else {
                    "Path copied to clipboard".into()
                }))
            }
            FmEffect::Close(id) => {
                if let Some(w) = self.files.remove(&id) {
                    w.stop.store(true, Ordering::Relaxed);
                    for c in w.cancels.values() {
                        c.store(true, Ordering::Relaxed);
                    }
                }
                None
            }
            FmEffect::SavePaths {
                alias,
                local,
                remote,
            } => self
                .with_store(|s| s.set_files_paths(&alias, &local, &remote))
                .err()
                .map(Msg::Notice),
            FmEffect::SaveOptions { alias, opts } => self
                .with_store(|s| s.set_files_options(&alias, &opts.to_setting()))
                .err()
                .map(Msg::Notice),
            FmEffect::SyncPlan {
                id,
                left,
                right,
                req,
                opts,
            } => {
                let ssh = self.ssh.clone()?;
                let host = self.files.get(&id)?.host.clone();
                self.background(move || {
                    let never = AtomicBool::new(false);
                    let result = open_remote(&ssh, &host).and_then(|mut c| {
                        telepad_core::files::compare::sync_plan(
                            c.as_mut(),
                            &left,
                            &right,
                            req,
                            &opts,
                            &never,
                        )
                    });
                    fm(FmMsg::SyncPlanned { id, result })
                });
                None
            }
        }
    }

    /// ssh settings, a cancel flag and the host for one transfer.
    fn transfer_setup(
        &mut self,
        id: FmId,
        tid: u64,
    ) -> Option<(
        SshSettings,
        Arc<AtomicBool>,
        telepad_core::config::HostEntry,
    )> {
        let ssh = self.ssh.clone()?;
        let w = self.files.get_mut(&id)?;
        let flag = w
            .cancels
            .entry(tid)
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone();
        Some((ssh, flag, w.host.clone()))
    }

    fn run_hostsync(&mut self, e: HsEffect) -> Option<Msg> {
        use telepad_core::hostsync;
        let msg = |m: HsMsg| Some(Msg::HostSync(m));
        match e {
            HsEffect::Load { path, remember } => {
                let ws = self.ws.as_ref()?;
                let raw = path
                    .or_else(|| ws.store.host_sync_path().ok().flatten())
                    .unwrap_or_else(|| "~/.ssh/config".into());
                let mut p = telepad_core::expand_home(raw.trim());
                if p.is_dir() {
                    p = p.join("config");
                }
                if remember {
                    let _ = ws.store.set_host_sync_path(&p.display().to_string());
                }
                msg(HsMsg::Loaded {
                    path: p.display().to_string(),
                    result: hostsync::read(&p),
                })
            }
            HsEffect::Pick { folder } => {
                let mut dialog = rfd::FileDialog::new().set_title(if folder {
                    "Folder with an OpenSSH config file"
                } else {
                    "OpenSSH config file"
                });
                let ssh = telepad_core::expand_home("~/.ssh");
                if ssh.is_dir() {
                    dialog = dialog.set_directory(ssh);
                }
                let picked = if folder {
                    dialog.pick_folder()
                } else {
                    dialog.pick_file()
                }?;
                msg(HsMsg::Picked(picked.display().to_string()))
            }
            HsEffect::Apply {
                path,
                expected,
                config_text,
                upserts,
                deletes,
            } => {
                let ws = self.ws.as_ref()?;
                let mut done = Vec::new();
                if let Some(text) = config_text {
                    match hostsync::write(Path::new(&path), &expected, &text) {
                        Ok(Some(b)) => done.push(format!("{path} saved (backup {})", b.display())),
                        Ok(None) => done.push(format!("{path} created")),
                        Err(e) => return msg(HsMsg::Applied(Err(e))),
                    }
                }
                if !upserts.is_empty() || !deletes.is_empty() {
                    if let Err(e) = ws.store.apply_host_changes(&upserts, &deletes) {
                        let e = if done.is_empty() {
                            format!("Telepad's hosts were not changed: {e}")
                        } else {
                            format!(
                                "{} — but Telepad's hosts were not changed: {e}",
                                done.join("; ")
                            )
                        };
                        return msg(HsMsg::Applied(Err(e)));
                    }
                    done.push(format!(
                        "{} Telepad change(s) saved",
                        upserts.len() + deletes.len()
                    ));
                }
                if done.is_empty() {
                    done.push("Nothing to change".into());
                }
                msg(HsMsg::Applied(Ok(done.join("; "))))
            }
        }
    }

    fn run_db(&mut self, e: DbEffect) -> Option<Msg> {
        match e {
            DbEffect::Log(entry) => self.history_send(HistCmd::Add(Box::new(entry))),
            DbEffect::LoadLog { id, key } => {
                self.history_load(history::Scope::Db(key), HistReply::Db(id))
            }
            DbEffect::ClearLog { key } => {
                self.history_send(HistCmd::Clear(history::Scope::Db(key)))
            }
            DbEffect::ExportLog(entries) => Self::history_export(&entries, "telepad-sql-log.csv"),
            DbEffect::LoadProfile { id, target, host } => {
                let key = target.key();
                let profile = self
                    .ws
                    .as_ref()
                    .and_then(|ws| ws.store.db_profile(&key).ok().flatten());
                let want = profile.as_ref().is_some_and(|p| p.save_password);
                self.background(move || {
                    let store = secret::available();
                    let saved_password =
                        want && store.is_ok() && matches!(secret::load(&key), Ok(Some(_)));
                    Msg::Db(DbMsg::Loaded {
                        id,
                        profile,
                        store,
                        saved_password,
                    })
                });
                if !target.container_id.is_empty()
                    && !host.is_local()
                    && let Some(s) = self.ssh.clone()
                {
                    self.background(move || {
                        Msg::Db(DbMsg::EnvUser {
                            id,
                            user: detect::fetch_env_user(&s, &host, &target.container_id),
                        })
                    });
                }
                None
            }
            DbEffect::Connect {
                id,
                target,
                host,
                mut login,
                use_saved_password,
                profile,
            } => {
                let ssh = self.ssh.clone()?;
                let (jobs, inbox) = mpsc::channel::<DbJob>();
                let slot: CancelSlot = Arc::new(Mutex::new(None));
                if let Some(old) = self.db.insert(
                    id,
                    DbWorker {
                        jobs,
                        cancel: slot.clone(),
                    },
                ) {
                    cancel(&old.cancel);
                }
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    let send = |m: Msg| {
                        let _ = tx.send(m);
                        ctx.request_repaint();
                    };
                    if use_saved_password {
                        match secret::load(&profile.key) {
                            Ok(Some(p)) => login.password = p,
                            Ok(None) => {}
                            Err(e) => {
                                send(Msg::Db(DbMsg::Connected {
                                    id,
                                    result: Err(format!("Cannot read the saved password: {e}")),
                                }));
                                return;
                            }
                        }
                    }
                    let password = login.password.clone();
                    let (session, info) =
                        match Session::connect(&ssh, &host, &target, login, profile.read_only) {
                            Ok(v) => v,
                            Err(e) => {
                                send(Msg::Db(DbMsg::Connected { id, result: Err(e) }));
                                return;
                            }
                        };
                    if profile.save_password {
                        if !password.is_empty()
                            && let Err(e) = secret::save(&profile.key, &password)
                        {
                            send(Msg::Notice(format!("Password not saved: {e}")));
                        }
                    } else {
                        let _ = secret::forget(&profile.key);
                    }
                    send(Msg::Db(DbMsg::Connected {
                        id,
                        result: Ok(info),
                    }));
                    let mut session = session.with_cancel_slot(slot);
                    for job in inbox {
                        // An export reports its progress while it pages (#139).
                        let msg = match job {
                            DbJob::Export { .. } => export_job(&mut session, id, job, &send),
                            job => db_job(&mut session, id, job),
                        };
                        send(Msg::Db(msg));
                    }
                });
                None
            }
            DbEffect::Export { id, name, mut job } => {
                let picked = rfd::FileDialog::new().set_file_name(&name).save_file();
                let Some(p) = picked else {
                    return Some(Msg::Db(DbMsg::Exported {
                        id,
                        result: Err("cancelled-dialog".into()),
                    }));
                };
                if let DbJob::Export { path, .. } = &mut job {
                    *path = p;
                }
                self.run_db(DbEffect::Job { id, job })
            }
            DbEffect::ExportRows {
                id,
                name,
                engine,
                grid,
                spec,
            } => {
                let Some(path) = rfd::FileDialog::new().set_file_name(&name).save_file() else {
                    return Some(Msg::Db(DbMsg::Exported {
                        id,
                        result: Err("cancelled-dialog".into()),
                    }));
                };
                self.background(move || {
                    use telepad_core::db::{TableRef, io::Exporter};
                    let t = TableRef {
                        table: "query".into(),
                        ..Default::default()
                    };
                    let file = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let result = Exporter::create(&path, engine, &t, &grid.columns, spec).and_then(
                        |mut x| match x.rows(&grid.rows, &grid.columns) {
                            Ok(()) => x.finish(),
                            Err(e) => {
                                x.abort();
                                Err(e)
                            }
                        },
                    );
                    Msg::Db(DbMsg::Exported {
                        id,
                        result: result.map(|(r, c)| (r, c, file)),
                    })
                });
                None
            }
            DbEffect::PickImport { id, engine, table } => {
                let picked = rfd::FileDialog::new()
                    .add_filter(
                        "Table data",
                        &["csv", "tsv", "txt", "sql", "xlsx", "xlsm", "xls", "ods"],
                    )
                    .pick_file()?;
                self.background(move || {
                    let name = picked
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    Msg::Db(DbMsg::ImportLoaded {
                        id,
                        result: telepad_core::db::io::read_import(&picked, engine, &table)
                            .map(|f| (name, f)),
                    })
                });
                None
            }
            DbEffect::Job { id, job } => match self.db.get(&id) {
                Some(w) => match w.jobs.send(job) {
                    Ok(()) => None,
                    Err(mpsc::SendError(job)) => Some(Msg::Db(db_failed(id, job))),
                },
                None => Some(Msg::Db(db_failed(id, job))),
            },
            DbEffect::LoadHidden { id, key, table } => {
                let hidden = self
                    .ws
                    .as_ref()
                    .and_then(|ws| ws.store.db_hidden(&key).ok().flatten())
                    .map(|v| {
                        v.split('\n')
                            .map(str::trim)
                            .filter(|c| !c.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                Some(Msg::Db(DbMsg::HiddenLoaded { id, table, hidden }))
            }
            DbEffect::SaveHidden { key, hidden } => {
                let v = hidden.into_iter().collect::<Vec<_>>().join("\n");
                self.with_store(|s| s.set_db_hidden(&key, &v))
                    .err()
                    .map(Msg::Notice)
            }
            DbEffect::Cancel(id) => {
                if let Some(w) = self.db.get(&id) {
                    let slot = w.cancel.clone();
                    thread::spawn(move || cancel(&slot));
                }
                None
            }
            DbEffect::Close(id) => {
                if let Some(w) = self.db.remove(&id) {
                    let slot = w.cancel;
                    thread::spawn(move || cancel(&slot));
                }
                None
            }
            DbEffect::SaveProfile(p) => self
                .with_store(|s| s.save_db_profile(&p))
                .err()
                .map(|e| Msg::Notice(format!("Cannot save the database connection: {e}"))),
            DbEffect::Forget { id, key } => {
                let removed = self.with_store(|s| s.delete_db_profile(&key));
                self.background(move || {
                    let result = removed.and_then(|()| match secret::available() {
                        Ok(()) => secret::forget(&key),
                        Err(_) => Ok(()),
                    });
                    Msg::Db(DbMsg::Forgotten { id, result })
                });
                None
            }
        }
    }

    fn with_store<T>(
        &self,
        f: impl FnOnce(&store::Store) -> Result<T, store::StoreError>,
    ) -> Result<T, String> {
        match &self.ws {
            Some(ws) => f(&ws.store).map_err(|e| e.to_string()),
            None => Err("no database".into()),
        }
    }

    /// Log info pages (#126): read in the background; the access terminal
    /// opens at once.
    fn run_logins(&mut self, e: telepad_app::logins::LgEffect) -> Option<Msg> {
        use telepad_app::logins::{LgEffect, LgMsg};
        match e {
            LgEffect::Read { id, host } => {
                let s = self.ssh.clone()?;
                self.background(move || {
                    Msg::Logins(LgMsg::Loaded {
                        id,
                        result: Box::new(telepad_core::login_log::read(&s, &host)),
                    })
                });
                None
            }
            LgEffect::Terminal { host, register } => {
                let s = self.ssh.as_ref()?;
                let args = telepad_core::login_log::terminal_args(s, &host, register);
                let what = if register { "Register" } else { "Unregister" };
                let text = match platform::open_terminal(&s.ssh_program, &args) {
                    Ok(()) => format!(
                        "Opened {what} Systemd Journal on {}: press Enter there, type your sudo password, then Refresh",
                        host.alias
                    ),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
            }
        }
    }

    fn background(&self, job: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }
}

/// Fonts a PDF export embeds (#153), in fallback order: egui's text font,
/// Hack (arrows, symbols), emoji, Phosphor icons.
fn pdf_fonts() -> Vec<Arc<[u8]>> {
    let defs = crate::icons::definitions();
    ["Ubuntu-Light", "Hack", "NotoEmoji-Regular", "phosphor"]
        .iter()
        .filter_map(|k| defs.font_data.get(*k))
        .map(|f| {
            let b: &[u8] = (*f.font).as_ref();
            Arc::from(b)
        })
        .collect()
}

/// Ping every host (no SSH), [`WORKERS`] at a time; (alias, ms or no reply).
/// Ping round (#122); hosts marked `port` (down at the last look) also get the
/// TCP check of their SSH port when ping stays silent (#125).
fn ping_all(hosts: &[PingJob]) -> Vec<(String, Reach)> {
    let mut out = Vec::with_capacity(hosts.len());
    for chunk in hosts.chunks(WORKERS) {
        thread::scope(|s| {
            let running: Vec<_> = chunk
                .iter()
                .map(|j| {
                    (
                        j.host.alias.clone(),
                        s.spawn(move || reach(&j.host, j.port, Duration::from_secs(3))),
                    )
                })
                .collect();
            for (alias, t) in running {
                out.push((alias, t.join().unwrap_or(Reach::Silent)));
            }
        });
    }
    out
}

/// Stop whatever statement the connection is running (no-op when idle).
fn cancel(slot: &CancelSlot) {
    if let Ok(guard) = slot.lock()
        && let Some(c) = guard.as_ref()
    {
        let _ = c.cancel();
    }
}

/// Run one database job on the worker thread and report it.
fn db_job(s: &mut Session, id: DbId, job: DbJob) -> DbMsg {
    match job {
        DbJob::Databases => DbMsg::DatabasesLoaded {
            id,
            result: s.databases(),
        },
        DbJob::Schemas { database } => DbMsg::SchemasLoaded {
            id,
            result: s.schemas(&database),
            database,
        },
        DbJob::Tables { database, schema } => DbMsg::TablesLoaded {
            id,
            result: s.tables(&database, &schema),
            database,
            schema,
        },
        DbJob::Info(table) => DbMsg::InfoLoaded {
            id,
            result: s.table_info(&table),
            table,
        },
        DbJob::Page {
            table,
            filter,
            cols,
            sort,
            offset,
            limit,
        } => {
            let result = s.page(&table, &filter, &cols, &sort, offset, limit);
            // "of N" for the pager (#138); a failed count shows "of many".
            let total = result
                .as_ref()
                .ok()
                .and_then(|_| s.count(&table, &filter, &cols).ok());
            DbMsg::PageLoaded {
                id,
                result,
                table,
                offset,
                total,
            }
        }
        DbJob::Run {
            tab,
            database,
            schema,
            statements,
        } => DbMsg::QueryDone {
            id,
            tab,
            results: s.run(&database, &schema, &statements),
        },
        DbJob::Apply {
            database,
            statements,
        } => DbMsg::Posted {
            id,
            result: s.apply(&database, &statements),
        },
        DbJob::Import {
            database,
            statements,
        } => DbMsg::Imported {
            id,
            result: s.import(&database, &statements),
        },
        DbJob::Export { .. } => DbMsg::Exported {
            id,
            result: Err("export runs on its own path".into()),
        },
        DbJob::Ddl {
            database,
            statements,
            after,
        } => DbMsg::DdlDone {
            id,
            result: s.ddl(&database, &statements),
            after,
        },
        DbJob::ReadOnly(on) => DbMsg::ReadOnlySet {
            id,
            on,
            result: s.set_read_only(on),
        },
    }
}

/// The answer to a job when the connection is gone.
fn db_failed(id: DbId, job: DbJob) -> DbMsg {
    const GONE: &str = "Not connected — click Reconnect";
    match job {
        DbJob::Databases => DbMsg::DatabasesLoaded {
            id,
            result: Err(GONE.into()),
        },
        DbJob::Schemas { database } => DbMsg::SchemasLoaded {
            id,
            database,
            result: Err(GONE.into()),
        },
        DbJob::Tables { database, schema } => DbMsg::TablesLoaded {
            id,
            database,
            schema,
            result: Err(GONE.into()),
        },
        DbJob::Info(table) => DbMsg::InfoLoaded {
            id,
            table,
            result: Err(GONE.into()),
        },
        DbJob::Page { table, offset, .. } => DbMsg::PageLoaded {
            id,
            table,
            offset,
            result: Err(GONE.into()),
            total: None,
        },
        DbJob::Run { tab, .. } => DbMsg::QueryDone {
            id,
            tab,
            results: vec![telepad_core::db::ResultSet {
                error: Some(GONE.into()),
                ..Default::default()
            }],
        },
        DbJob::Apply { .. } => DbMsg::Posted {
            id,
            result: Err(GONE.into()),
        },
        DbJob::Import { .. } => DbMsg::Imported {
            id,
            result: Err(GONE.into()),
        },
        DbJob::Export { .. } => DbMsg::Exported {
            id,
            result: Err(GONE.into()),
        },
        DbJob::Ddl { after, .. } => DbMsg::DdlDone {
            id,
            result: Err(GONE.into()),
            after,
        },
        DbJob::ReadOnly(on) => DbMsg::ReadOnlySet {
            id,
            on,
            result: Err(GONE.into()),
        },
    }
}

/// Export a table page by page into its file (#139), reporting the rows
/// written; a cancelled query (Cancel) stops it and removes the part file.
fn export_job(s: &mut Session, id: DbId, job: DbJob, send: &dyn Fn(Msg)) -> DbMsg {
    use telepad_core::db::io::Exporter;
    let DbJob::Export {
        table,
        filter,
        cols,
        sort,
        page,
        spec,
        path,
    } = job
    else {
        return DbMsg::Exported {
            id,
            result: Err("not an export".into()),
        };
    };
    const BATCH: u64 = 1000;
    let engine = s.engine();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let result = (|| {
        let (mut offset, end) = match page {
            Some((o, l)) => (o, Some(o + l)),
            None => (0, None),
        };
        let mut ex: Option<Exporter> = None;
        loop {
            let limit = end.map_or(BATCH, |e| BATCH.min(e.saturating_sub(offset)));
            if limit == 0 {
                break;
            }
            let g = match s.page(&table, &filter, &cols, &sort, offset, limit) {
                Ok(g) => g,
                Err(e) => {
                    if let Some(x) = ex.take() {
                        x.abort();
                    }
                    return Err(e);
                }
            };
            if ex.is_none() {
                ex = Some(Exporter::create(
                    &path,
                    engine,
                    &table,
                    &g.columns,
                    spec.clone(),
                )?);
            }
            let Some(x) = ex.as_mut() else { break };
            if let Err(e) = x.rows(&g.rows, &g.columns) {
                if let Some(x) = ex.take() {
                    x.abort();
                }
                return Err(e);
            }
            send(Msg::Db(DbMsg::ExportProgress { id, rows: x.rows }));
            if (g.rows.len() as u64) < limit {
                break;
            }
            offset += limit;
        }
        match ex {
            Some(x) => x.finish(),
            None => Ok((0, 0)),
        }
    })();
    DbMsg::Exported {
        id,
        result: result.map(|(rows, cut)| (rows, cut, name)),
    }
}

/// Read and parse an import file; errors keep the path for the message.
fn read_import(path: std::path::PathBuf) -> Msg {
    let parsed = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| store::parse_import(&t).map_err(|e| e.to_string()));
    Msg::ImportOpened(match parsed {
        Ok(inv) => Ok((path, inv)),
        Err(e) => Err((path, e)),
    })
}

fn export(ws: &Workspace, path: &Path) -> Result<usize, String> {
    let inv = ws.store.inventory().map_err(|e| e.to_string())?;
    let json = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let text = if json {
        store::export_json(&inv).map_err(|e| e.to_string())?
    } else {
        config::to_ssh_config(&inv)
    };
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    Ok(inv.hosts.len())
}

#[cfg(test)]
mod tests {
    use telepad_core::table_export::{Format, TableData, render};

    /// A PDF export (#153) embeds the UI's fonts: text, arrows and degree
    /// signs all have a glyph. `TELEPAD_TEST_PDF_OUT` keeps the file to look at.
    #[test]
    fn pdf_export_embeds_the_ui_fonts() {
        let fonts = super::pdf_fonts();
        assert!(fonts.len() >= 2, "text font and Hack at least");
        let t = TableData {
            title: "Ports in use on web-01".into(),
            columns: vec![
                "Port".into(),
                "Bind".into(),
                "→ Container".into(),
                "Temp".into(),
            ],
            rows: (0..60)
                .map(|i| {
                    vec![
                        (8000 + i).to_string(),
                        format!("192.0.2.{i}"),
                        "443/tcp".into(),
                        format!("{}.5 °C", 30 + i % 40),
                    ]
                })
                .collect(),
        };
        let refs: Vec<&[u8]> = fonts.iter().map(|f| f.as_ref()).collect();
        let b = render(&t, Format::Pdf, &refs, "Telepad · test").unwrap();
        assert!(b.starts_with(b"%PDF-"));
        if let Ok(p) = std::env::var("TELEPAD_TEST_PDF_OUT") {
            std::fs::write(p, &b).unwrap();
        }
    }
}
