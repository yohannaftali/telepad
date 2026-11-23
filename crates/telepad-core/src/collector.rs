//! Runs the remote script over the system OpenSSH client on worker threads.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{Access, HostEntry};
use crate::model::HostReport;
use crate::strategy::{SlowFacts, StaticFacts};
use crate::{parse, platform};

const SCRIPT: &str = include_str!("script.sh");

/// Which parts of the remote script one collection runs. Live metrics (uptime,
/// load, CPU, memory, disks, sensors, Docker, Swarm, Kubernetes) always run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    /// Hostname, OS release, kernel, architecture, CPU model (daily by default,
    /// see [`crate::strategy::Strategy`]).
    pub static_info: bool,
    /// IP addresses, listening ports, updates / reboot (every 6 hours by default).
    pub slow: bool,
    /// The host was down without a ping reply: ping first and skip ssh while it
    /// stays silent. When it answers, everything is collected.
    pub ping_first: bool,
}

impl Scope {
    pub const FULL: Self = Self {
        static_info: true,
        slow: true,
        ping_first: false,
    };
    pub const LIVE: Self = Self {
        static_info: false,
        slow: false,
        ping_first: false,
    };

    /// The remote script with this scope's tier switches in front.
    pub fn script(self) -> String {
        let on = |b: bool| if b { "1" } else { "" };
        format!(
            "TP_STATIC={}\nTP_SLOW={}\n{SCRIPT}",
            on(self.static_info),
            on(self.slow)
        )
    }

    /// Fill what this collection skipped from the previous report of the same host.
    pub fn carry_over(self, new: &mut HostReport, prev: &HostReport) {
        if !self.static_info {
            StaticFacts::of(prev).apply(new);
        }
        if self.slow {
            // Parsed without the OS release: look the end of support up again.
            new.maintenance.end_of_support =
                crate::eol::end_of_support(&new.os_id, &new.os_version_id, &new.os_pretty)
                    .map(str::to_string);
        } else {
            SlowFacts::of(prev).apply(new);
        }
    }
}

/// One host to collect and how much of it.
#[derive(Debug, Clone)]
pub struct Job {
    pub host: HostEntry,
    pub scope: Scope,
}

/// Result of [`collect`].
#[derive(Debug, Clone)]
pub struct Outcome {
    pub result: Result<HostReport, String>,
    /// What actually ran: `ping_first` stays set only when ssh was skipped.
    pub scope: Scope,
    /// The host did not answer ping (and ssh failed or was skipped).
    pub no_ping: bool,
}
const BMC_SETUP: &str = include_str!("bmc_setup.sh");

/// Interactive ssh (with a tty) that shows `bmc_setup.sh`, waits for Enter and
/// runs it, so sudo can ask for the password. The script travels base64-encoded
/// as plain arguments (no spaces, quotes, `;` or `%`), so it passes unchanged
/// through Windows Terminal / `cmd start` and the remote login shell.
pub fn bmc_setup_args(settings: &SshSettings, host: &HostEntry) -> Vec<String> {
    terminal_script_args(settings, host, BMC_SETUP)
}

/// Interactive ssh for a Docker action: the terminal shows the exact command
/// and runs it only after Enter (see `docker_actions`).
pub fn docker_action_args(
    settings: &SshSettings,
    host: &HostEntry,
    plan: &crate::docker_actions::Plan,
) -> Vec<String> {
    terminal_script_args(settings, host, &plan.script())
}

fn terminal_script_args(settings: &SshSettings, host: &HostEntry, script: &str) -> Vec<String> {
    let mut a = settings.args(host, false);
    a.insert(0, "-t".into());
    let script = script.replace('\r', "");
    a.extend(
        [
            "echo",
            &base64(script.as_bytes()),
            "|",
            "base64",
            "-d",
            "|",
            "sh",
        ]
        .map(String::from),
    );
    a
}

pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct SshSettings {
    pub config_path: PathBuf,
    pub ssh_program: PathBuf,
    /// Used only for hosts whose config block has no IdentityFile.
    pub fallback_identity: Option<PathBuf>,
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
}

impl SshSettings {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            ssh_program: platform::ssh_program(),
            fallback_identity: None,
            connect_timeout: Duration::from_secs(8),
            total_timeout: Duration::from_secs(40),
        }
    }

    /// Base ssh arguments for `host`, without the remote command.
    pub fn args(&self, host: &HostEntry, batch: bool) -> Vec<String> {
        let mut a = vec!["-F".to_string(), self.config_path.display().to_string()];
        if batch {
            a.extend([
                "-T".into(),
                "-o".into(),
                "BatchMode=yes".into(),
                "-o".into(),
                format!("ConnectTimeout={}", self.connect_timeout.as_secs()),
                // Trust-on-first-use; a changed key is still rejected.
                "-o".into(),
                "StrictHostKeyChecking=accept-new".into(),
            ]);
        }
        if host.identity_file.is_none()
            && let Some(id) = &self.fallback_identity
        {
            a.extend(["-i".into(), id.display().to_string()]);
        }
        a.push(host.alias.clone());
        a
    }
}

/// Collect everything from one host synchronously. Blocks for up to `total_timeout`.
pub fn collect_one(settings: &SshSettings, host: &HostEntry) -> Result<HostReport, String> {
    collect(settings, host, Scope::FULL).result
}

/// Collect `scope` from one host synchronously. Blocks for up to `total_timeout`.
pub fn collect(settings: &SshSettings, host: &HostEntry, scope: Scope) -> Outcome {
    if host.access == Access::Rdp {
        return Outcome {
            result: probe_rdp(host, settings.connect_timeout),
            scope: Scope::FULL,
            no_ping: false,
        };
    }
    let target = if host.hostname.is_empty() {
        &host.alias
    } else {
        &host.hostname
    };
    let mut scope = scope;
    if scope.ping_first {
        if platform::ping(target, Duration::from_secs(2)).is_none() {
            return Outcome {
                result: Err("no ping reply — SSH is tried again when the host answers ping".into()),
                scope,
                no_ping: true,
            };
        }
        // Back after being down: anything may have changed.
        scope = Scope::FULL;
    }
    let ran = match run_script(settings, host, &scope.script()) {
        Ok((out, _, _)) if out.contains("===TELEPAD===") => {
            return Outcome {
                result: Ok(parse::parse_report(&out)),
                scope,
                no_ping: false,
            };
        }
        Ok((_, err, code)) => explain_ssh_error(&err, code),
        Err(e) => e,
    };
    // Tell "machine is down" apart from "only SSH fails".
    let (ping, no_ping) = match platform::ping(target, Duration::from_secs(2)) {
        Some(ms) => (format!(" (host answers ping, {ms:.0} ms)"), false),
        None => (" (no ping reply)".to_string(), true),
    };
    Outcome {
        result: Err(format!("{ran}{ping}")),
        scope,
        no_ping,
    }
}

/// "How it runs" for one container: read-only `docker inspect` + compose files.
pub fn fetch_run_info(
    settings: &SshSettings,
    host: &HostEntry,
    c: &crate::model::Container,
) -> crate::docker_actions::RunInfo {
    use crate::docker_actions::{RunInfo, parse_run_info, run_info_script};
    let failed = |e: String| RunInfo {
        container: c.name.clone(),
        error: Some(e),
        ..Default::default()
    };
    let script = match run_info_script(c) {
        Ok(s) => s,
        Err(e) => return failed(e),
    };
    match run_script(settings, host, &script) {
        Ok((out, _, _)) if out.contains("===INSPECT===") || out.contains("ERR ") => {
            parse_run_info(&c.name, &out)
        }
        Ok((_, err, code)) => failed(explain_ssh_error(&err, code)),
        Err(e) => failed(e),
    }
}

/// Run a script with `sh -s` over BatchMode ssh; (stdout, stderr, exit code).
pub(crate) fn run_script(
    settings: &SshSettings,
    host: &HostEntry,
    script: &str,
) -> Result<(String, String, Option<i32>), String> {
    let mut cmd = Command::new(&settings.ssh_program);
    cmd.args(settings.args(host, true))
        .args(["sh", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    platform::hide_console(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", settings.ssh_program.display()))?;

    let script = script.replace('\r', "");
    let mut stdin = child.stdin.take();
    let writer = thread::spawn(move || {
        if let Some(s) = stdin.as_mut() {
            let _ = s.write_all(script.as_bytes());
        }
        drop(stdin);
    });
    let stdout = pipe_reader(child.stdout.take());
    let stderr = pipe_reader(child.stderr.take());

    let deadline = Instant::now() + settings.total_timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("ssh wait failed: {e}")),
        }
    };
    let _ = writer.join();
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();

    let Some(status) = status else {
        return Err(format!(
            "timed out after {}s",
            settings.total_timeout.as_secs()
        ));
    };
    Ok((out, err, status.code()))
}

/// RDP-only Windows hosts: no metrics, just ping + "does the RDP port accept a TCP
/// connection". Up when either answers (Windows Server blocks ping by default).
pub fn probe_rdp(host: &HostEntry, timeout: Duration) -> Result<HostReport, String> {
    let target = if host.hostname.is_empty() {
        &host.alias
    } else {
        &host.hostname
    };
    let ping_target = target.clone();
    let pinger = thread::spawn(move || platform::ping(&ping_target, Duration::from_secs(2)));
    let rdp = tcp_open(target, host.port, timeout);
    let ping_ms = pinger.join().ok().flatten();
    if ping_ms.is_none() && rdp.is_err() {
        return Err(format!(
            "no ping reply; RDP port {}: {}",
            host.port,
            rdp.err().unwrap_or_default()
        ));
    }
    Ok(HostReport {
        hostname: target.clone(),
        os_type: "Windows".into(),
        os_name: "Windows".into(),
        os_pretty: "Windows (Remote Desktop)".into(),
        rdp_only: true,
        ping_ms,
        rdp_open: rdp.is_ok(),
        ..Default::default()
    })
}

fn tcp_open(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    use std::net::{TcpStream, ToSocketAddrs};
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {host}: {e}"))?
        .collect();
    let mut last = format!("no address for {host}");
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(_) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn pipe_reader<R: Read + Send + 'static>(r: Option<R>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            let _ = r.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

pub fn explain_ssh_error(stderr: &str, code: Option<i32>) -> String {
    // ssh skips a private key that others can read and then fails with
    // "Permission denied", which alone looks like a wrong key.
    if let Some(key) = key_with_bad_permissions(stderr) {
        return format!(
            "private key {key} is readable by other users, so ssh ignores it — run: chmod 600 {key}"
        );
    }
    let lower = stderr.to_ascii_lowercase();
    let hint = if lower.contains("permission denied") {
        "authentication failed (key not accepted, or host needs a password)"
    } else if lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
    {
        "host key mismatch — check known_hosts"
    } else if lower.contains("timed out") {
        "connection timed out"
    } else if lower.contains("connection refused") {
        "connection refused"
    } else if lower.contains("could not resolve") {
        "hostname not resolvable"
    } else if lower.contains("no route to host") || lower.contains("network is unreachable") {
        "host unreachable"
    } else if lower.contains("no matching") || lower.contains("unable to negotiate") {
        "no common SSH algorithm (legacy host)"
    } else {
        ""
    };
    let last = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    match (hint.is_empty(), last.is_empty()) {
        (false, _) => hint.to_string(),
        (true, false) => last.to_string(),
        (true, true) => format!(
            "ssh exited with code {}",
            code.map_or("?".into(), |c| c.to_string())
        ),
    }
}

/// `Load key "/Users/a/.ssh/id_ed25519": bad permissions` → that path.
fn key_with_bad_permissions(stderr: &str) -> Option<&str> {
    stderr.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("Load key \"")?;
        let (path, tail) = rest.split_once('"')?;
        tail.contains("bad permissions").then_some(path)
    })
}

#[derive(Debug, Clone)]
pub enum Update {
    Started(String),
    Done {
        alias: String,
        result: Result<Box<HostReport>, String>,
        /// What ran (see [`Outcome::scope`]).
        scope: Scope,
        no_ping: bool,
        elapsed: Duration,
    },
}

struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
    in_flight: Mutex<HashSet<String>>,
}

/// Bounded worker pool. Jobs for a host already queued or running are ignored.
pub struct Collector {
    queue: Arc<Queue>,
}

impl Collector {
    pub fn start(
        settings: SshSettings,
        workers: usize,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> (Self, Receiver<Update>) {
        let (tx, rx) = mpsc::channel();
        let queue = Arc::new(Queue {
            jobs: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
            in_flight: Mutex::new(HashSet::new()),
        });
        let settings = Arc::new(settings);
        for _ in 0..workers.max(1) {
            let (q, s, tx, n) = (queue.clone(), settings.clone(), tx.clone(), notify.clone());
            thread::spawn(move || worker(q, s, tx, n));
        }
        (Self { queue }, rx)
    }

    pub fn enqueue(&self, new_jobs: impl IntoIterator<Item = Job>) {
        let Ok(mut inflight) = self.queue.in_flight.lock() else {
            return;
        };
        let Ok(mut jobs) = self.queue.jobs.lock() else {
            return;
        };
        for j in new_jobs {
            if inflight.insert(j.host.alias.clone()) {
                jobs.push_back(j);
            }
        }
        self.queue.ready.notify_all();
    }

    pub fn busy(&self) -> usize {
        self.queue.in_flight.lock().map(|s| s.len()).unwrap_or(0)
    }
}

fn worker(
    q: Arc<Queue>,
    s: Arc<SshSettings>,
    tx: Sender<Update>,
    notify: Arc<dyn Fn() + Send + Sync>,
) {
    loop {
        let job = {
            let Ok(mut jobs) = q.jobs.lock() else { return };
            loop {
                if let Some(h) = jobs.pop_front() {
                    break h;
                }
                jobs = match q.ready.wait(jobs) {
                    Ok(j) => j,
                    Err(_) => return,
                };
            }
        };
        let alias = job.host.alias.clone();
        if tx.send(Update::Started(alias.clone())).is_err() {
            return;
        }
        notify();
        let t = Instant::now();
        let o = collect(&s, &job.host, job.scope);
        if let Ok(mut f) = q.in_flight.lock() {
            f.remove(&alias);
        }
        if tx
            .send(Update::Done {
                alias,
                result: o.result.map(Box::new),
                scope: o.scope,
                no_ping: o.no_ping,
                elapsed: t.elapsed(),
            })
            .is_err()
        {
            return;
        }
        notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_switches_tiers_in_front_of_the_script() {
        let live = Scope::LIVE.script();
        assert!(live.starts_with("TP_STATIC=\nTP_SLOW=\n"), "{live}");
        assert!(Scope::FULL.script().starts_with("TP_STATIC=1\nTP_SLOW=1\n"));
        assert!(live.contains("===TELEPAD==="));
        // Parse-only check with the local sh when there is one (CI on Linux/macOS).
        // The script goes through stdin, as over ssh (Windows mangles quotes in arguments).
        let child = Command::new("sh")
            .arg("-n")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(live.replace('\r', "").as_bytes()).unwrap();
            }
            let out = child.wait_with_output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        // Tiers wrap whole sections: static before the live metrics, slow after.
        let at = |m: &str| SCRIPT.find(m).unwrap();
        assert!(at("if [ -n \"$TP_STATIC\" ]") < at("===HOSTNAME==="));
        assert!(at("===CPUINFO===") < at("===UPTIME==="));
        assert!(at("if [ -n \"$TP_SLOW\" ]") < at("===IPADDR==="));
        assert!(
            at("===IPADDR===") < at("===LISTEN===") && at("===UPDATES===") < at("===DOCKER===")
        );
    }

    #[test]
    fn live_collection_keeps_static_and_slow_facts_from_the_previous_report() {
        let prev = HostReport {
            hostname: "web01".into(),
            os_pretty: "Ubuntu 22.04.4 LTS".into(),
            os_id: "ubuntu".into(),
            os_version_id: "22.04".into(),
            kernel: "5.15.0".into(),
            cpu_cores: Some(4),
            addresses: vec![crate::model::NetAddr {
                interface: "eth0".into(),
                family: "inet".into(),
                address: "192.0.2.10/24".into(),
            }],
            maintenance: crate::model::Maintenance {
                updates: Some(3),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut new = HostReport {
            uptime_secs: Some(100),
            ..Default::default()
        };
        Scope::LIVE.carry_over(&mut new, &prev);
        assert_eq!(new.hostname, "web01");
        assert_eq!(new.cpu_cores, Some(4));
        assert_eq!(new.addresses.len(), 1);
        assert_eq!(new.maintenance.updates, Some(3));
        assert_eq!(new.uptime_secs, Some(100));

        // Slow tier ran: its own results win, end of support comes from the kept OS.
        let mut new = HostReport::default();
        let slow = Scope {
            slow: true,
            ..Scope::LIVE
        };
        slow.carry_over(&mut new, &prev);
        assert!(new.addresses.is_empty());
        assert_eq!(new.maintenance.updates, None);
        assert_eq!(new.os_id, "ubuntu");
        assert!(new.maintenance.end_of_support.is_some());
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn bmc_setup_args_are_shell_and_cmd_safe() {
        let s = SshSettings {
            config_path: "C:/x/telepad.ssh_config".into(),
            ssh_program: "ssh".into(),
            fallback_identity: None,
            connect_timeout: Duration::from_secs(8),
            total_timeout: Duration::from_secs(40),
        };
        let a = bmc_setup_args(&s, &HostEntry::new("docker02"));
        assert_eq!(a[0], "-t");
        let at = a.iter().position(|x| x == "docker02").unwrap();
        assert_eq!(a[at + 1], "echo");
        assert_eq!(&a[a.len() - 5..], ["|", "base64", "-d", "|", "sh"]);
        for x in &a[at + 1..] {
            assert!(
                !x.chars()
                    .any(|c| c.is_whitespace() || "\";%!^'`$&".contains(c)),
                "{x}"
            );
        }
    }

    #[test]
    fn args_are_separate_and_use_fallback_identity_only_when_missing() {
        let mut s = SshSettings::new(PathBuf::from("cfg"));
        s.fallback_identity = Some(PathBuf::from("key"));
        let mut h = HostEntry {
            alias: "nas01".into(),
            ..Default::default()
        };
        let a = s.args(&h, true);
        assert_eq!(a.last().unwrap(), "nas01");
        assert!(a.windows(2).any(|w| w == ["-i", "key"]));
        assert!(a.iter().any(|x| x == "BatchMode=yes"));
        assert!(!a.windows(2).any(|w| w[0] == "-o" && w[1] == "-o"));
        h.identity_file = Some("~/.ssh/id_rsa".into());
        assert!(!s.args(&h, false).iter().any(|x| x == "-i"));
    }

    #[test]
    fn explains_common_errors() {
        assert!(
            explain_ssh_error("user@x: Permission denied (publickey).", Some(255))
                .starts_with("authentication")
        );
        assert_eq!(explain_ssh_error("", Some(255)), "ssh exited with code 255");
    }

    #[test]
    fn explains_a_private_key_with_too_open_permissions() {
        let stderr = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@
            @         WARNING: UNPROTECTED PRIVATE KEY FILE!          @
            Permissions 0644 for '/Users/alice/.ssh/id_ed25519' are too open.
            This private key will be ignored.
            Load key \"/Users/alice/.ssh/id_ed25519\": bad permissions
            alice@192.0.2.10: Permission denied (publickey).";
        let msg = explain_ssh_error(stderr, Some(255));
        assert!(
            msg.contains("chmod 600 /Users/alice/.ssh/id_ed25519"),
            "{msg}"
        );
        assert!(!msg.starts_with("authentication"), "{msg}");
    }
}
