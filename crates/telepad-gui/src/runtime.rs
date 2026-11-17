//! Executes [`Effect`]s and turns their results into [`Msg`]s. This is the
//! only GUI code that touches the database, ssh, the network, files, the
//! clipboard or the OS. Slow work (collection, update check/download) runs on
//! background threads; the rest is quick and runs inline.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use eframe::egui;
use telepad_app::{Effect, Msg, State};
use telepad_core::collector::{self, Collector, SshSettings};
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
                    .set_title("Import hosts (OpenSSH config or Telepad JSON)")
                    .add_filter("All files", &["*"])
                    .add_filter("JSON export", &["json"])
                    .pick_file()?;
                let parsed = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| store::parse_import(&t).map_err(|e| e.to_string()));
                Some(Msg::ImportOpened(match parsed {
                    Ok(inv) => Ok((path, inv)),
                    Err(e) => Err((path, e)),
                }))
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
            Effect::PickKeyFile => rfd::FileDialog::new()
                .set_title("Private key")
                .pick_file()
                .map(Msg::KeyPicked),
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
