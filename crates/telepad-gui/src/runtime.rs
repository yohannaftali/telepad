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
use std::time::Instant;

use eframe::egui;
use telepad_app::db::{DbEffect, DbId, DbJob, DbMsg};
use telepad_app::editor::{EdMsg, Load};
use telepad_app::files::{FileOp, FmEffect, FmId, FmMsg, Mode, Side};
use telepad_app::hostsync::{HsEffect, HsMsg};
use telepad_app::{Effect, Msg, State};
use telepad_core::authorized_keys;
use telepad_core::collector::{self, Collector, SshSettings};
use telepad_core::db::client::{CancelSlot, Session};
use telepad_core::db::{detect, secret, tunnel};
use telepad_core::store;
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
                };
                rt.ssh = Some(ssh);
                rt.ws = Some(ws);
                rt.dispatch(state, started);
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
                let text = match platform::open_rdp(host, h.port) {
                    Ok(()) => format!("Opened Remote Desktop to {}", h.alias),
                    Err(e) => e,
                };
                Some(Msg::Notice(text))
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
            Effect::Files(e) => self.run_files(e),
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
            FmEffect::Connect { id, host } => {
                let ssh = self.ssh.clone()?;
                let remembered = self
                    .ws
                    .as_ref()
                    .and_then(|ws| ws.store.files_paths(&host.alias).ok().flatten());
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
                self.background(move || {
                    fm(FmMsg::RsyncChecked {
                        id,
                        result: crsync::local_available()
                            .and_then(|()| crsync::remote_available(&s2, &h2)),
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
                    let result = match Sftp::connect(&ssh, &host).and_then(|mut c| {
                        let home = c.realpath(".")?;
                        Ok((c, home))
                    }) {
                        Ok((c, home)) => {
                            sftp = Some(c);
                            Ok((Mode::Sftp, home, remembered))
                        }
                        Err(e) if e.contains("no SFTP server") => fallback::list(&ssh, &host, "~")
                            .map(|(home, _)| (Mode::Fallback, home, remembered)),
                        Err(e) => Err(e),
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
                    for job in inbox {
                        let (side, path) = match job {
                            FileJob::List(side, path) => (side, path),
                            FileJob::Load {
                                tab,
                                side,
                                path,
                                load,
                            } => {
                                let read = match (side, sftp.as_mut()) {
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
                                let result = match (side, sftp.as_mut()) {
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
                                let out = match (side, sftp.as_mut()) {
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
                        let result = match side {
                            Side::Local => {
                                let p =
                                    telepad_core::expand_home(path.trim()).display().to_string();
                                cf::list_local(&p).map(|v| (p, v))
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
            } => {
                let (ssh, cancel, host) = self.transfer_setup(id, tid)?;
                self.background(move || {
                    let result = Sftp::connect(&ssh, &host).and_then(|mut c| {
                        transfer::plan(&mut c, direction, &sources, &dest, &cancel)
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
            } => {
                let (ssh, cancel, host) = self.transfer_setup(id, tid)?;
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                thread::spawn(move || {
                    let send = |m: Msg| {
                        let _ = tx.send(m);
                        ctx.request_repaint();
                    };
                    let mut c = match Sftp::connect(&ssh, &host) {
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
                    let counts =
                        transfer::run(&mut c, direction, &items, &decisions, &cancel, &mut |ev| {
                            match ev {
                                transfer::Event::Bytes(n) => {
                                    pending += n;
                                    if last.elapsed() < std::time::Duration::from_millis(150) {
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
                            }
                        });
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
            } => {
                let (ssh, _cancel, host) = self.transfer_setup(id, tid)?;
                self.background(move || {
                    let upload = direction == transfer::Direction::Upload;
                    let result =
                        crsync::run(&crsync::args(&ssh, &host, upload, &sources, &dest, dry_run));
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
                        send(Msg::Db(db_job(&mut session, id, job)));
                    }
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

    fn background(&self, job: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }
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
            sort,
            offset,
            limit,
        } => DbMsg::PageLoaded {
            id,
            result: s.page(&table, &filter, &sort, offset, limit),
            table,
            offset,
        },
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
