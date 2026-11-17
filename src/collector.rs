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
use crate::{parse, platform};

const SCRIPT: &str = include_str!("script.sh");
const BMC_SETUP: &str = include_str!("bmc_setup.sh");

/// Interactive ssh (with a tty) that shows `bmc_setup.sh`, waits for Enter and
/// runs it, so sudo can ask for the password. The script travels base64-encoded
/// as plain arguments (no spaces, quotes, `;` or `%`), so it passes unchanged
/// through Windows Terminal / `cmd start` and the remote login shell.
pub fn bmc_setup_args(settings: &SshSettings, host: &HostEntry) -> Vec<String> {
    let mut a = settings.args(host, false);
    a.insert(0, "-t".into());
    let script = BMC_SETUP.replace('\r', "");
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

fn base64(data: &[u8]) -> String {
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

/// Collect one host synchronously. Blocks for up to `total_timeout`.
pub fn collect_one(settings: &SshSettings, host: &HostEntry) -> Result<HostReport, String> {
    if host.access == Access::Rdp {
        return probe_rdp(host, settings.connect_timeout);
    }
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

    let script = SCRIPT.replace('\r', "");
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
    if !out.contains("===HOSTNAME===") {
        let msg = explain_ssh_error(&err, status.code());
        // Tell "machine is down" apart from "only SSH fails".
        let target = if host.hostname.is_empty() {
            &host.alias
        } else {
            &host.hostname
        };
        let ping = match platform::ping(target, Duration::from_secs(2)) {
            Some(ms) => format!(" (host answers ping, {ms:.0} ms)"),
            None => " (no ping reply)".to_string(),
        };
        return Err(format!("{msg}{ping}"));
    }
    Ok(parse::parse_report(&out))
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

#[derive(Debug, Clone)]
pub enum Update {
    Started(String),
    Done {
        alias: String,
        result: Result<Box<HostReport>, String>,
        elapsed: Duration,
    },
}

struct Queue {
    jobs: Mutex<VecDeque<HostEntry>>,
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

    pub fn enqueue(&self, hosts: impl IntoIterator<Item = HostEntry>) {
        let Ok(mut inflight) = self.queue.in_flight.lock() else {
            return;
        };
        let Ok(mut jobs) = self.queue.jobs.lock() else {
            return;
        };
        for h in hosts {
            if inflight.insert(h.alias.clone()) {
                jobs.push_back(h);
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
        let host = {
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
        let alias = host.alias.clone();
        if tx.send(Update::Started(alias.clone())).is_err() {
            return;
        }
        notify();
        let t = Instant::now();
        let result = collect_one(&s, &host);
        if let Ok(mut f) = q.in_flight.lock() {
            f.remove(&alias);
        }
        if tx
            .send(Update::Done {
                alias,
                result: result.map(Box::new),
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
}
