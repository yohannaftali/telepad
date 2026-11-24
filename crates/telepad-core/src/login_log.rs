//! Log info page (#126): SSH logins of one host from its systemd journal.
//!
//! Read-only: one SSH call runs `journalctl` **without sudo** (the login user
//! needs read access to the system journal: root, or the `systemd-journal`,
//! `adm` (Debian / Ubuntu) or `wheel` (Fedora / RHEL) group) and `cat`s the
//! login user's `authorized_keys` so fingerprints get names. Giving that
//! access is the user's own step in a terminal ([`register_script`] /
//! [`unregister_script`]: the exact `sudo` command, run after Enter).

use crate::authorized_keys::{self, HostKeys};
use crate::collector::{SshSettings, explain_ssh_error, run_script};
use crate::config::{Access, HostEntry};

/// Logins of the last this many days are read.
pub const DAYS: u32 = 30;
/// At most this many (newest) login lines are read.
pub const LIMIT: usize = 5000;
/// A read may take this long: a public host's journal can hold close to a
/// million sshd lines a month (password attacks), which journalctl needs
/// a minute or two to go through. It runs in the background, on request.
pub const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// Whether the login user can read the system journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalAccess {
    #[default]
    Readable,
    /// journalctl said it cannot open the system journal for this user.
    Denied,
    /// No `journalctl`: not a systemd host (Alpine, BusyBox, NAS).
    NoJournal,
}

/// One `Accepted …` line of sshd.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Login {
    /// Unix seconds (UTC); 0 when the time could not be read.
    pub epoch: i64,
    /// As the host logged it, in its own time zone: `YYYY-MM-DD HH:MM:SS`.
    pub time: String,
    pub user: String,
    pub from: String,
    pub port: u16,
    /// `publickey`, `password`, `keyboard-interactive/pam`, `gssapi-with-mic`…
    pub method: String,
    /// `ED25519`, `RSA`, `ED25519-CERT`… (public key logins only).
    pub key_type: String,
    /// `SHA256:…` (public key logins only).
    pub fingerprint: String,
    /// Certificate ID, for certificate logins.
    pub cert_id: String,
    /// sshd's process id (matches this Telepad session's ancestors).
    pub pid: u32,
}

/// What one read of a host returned.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginLog {
    pub access: JournalAccess,
    /// The login user (`id -un`).
    pub user: String,
    /// Its groups (`id -nG`).
    pub groups: Vec<String>,
    /// This session's address as the host sees it (`$SSH_CLIENT`).
    pub client: String,
    /// Process ids above this session's shell (sshd's ones among them).
    pub pids: Vec<u32>,
    /// Time of the oldest journal entry (`YYYY-MM-DD HH:MM:SS`), when known.
    pub first: Option<String>,
    /// Oldest first, as journalctl printed them.
    pub logins: Vec<Login>,
    /// [`LIMIT`] lines were read: older logins in the window are left out.
    pub truncated: bool,
    /// The login user's `authorized_keys` (names for fingerprints); `Err`
    /// when it could not be read.
    pub keys: Result<HostKeys, String>,
    /// Keys of this computer Telepad may log in with (fingerprint, file name
    /// and comment), from [`authorized_keys::login_keys`].
    pub own: Vec<(String, String)>,
}

impl Default for LoginLog {
    fn default() -> Self {
        Self {
            access: JournalAccess::default(),
            user: String::new(),
            groups: Vec::new(),
            client: String::new(),
            pids: Vec::new(),
            first: None,
            logins: Vec::new(),
            truncated: false,
            keys: Err("not read".into()),
            own: Vec::new(),
        }
    }
}

impl LoginLog {
    /// In the `systemd-journal` group (what Unregister removes).
    pub fn in_journal_group(&self) -> bool {
        self.groups.iter().any(|g| g == "systemd-journal")
    }

    /// The login of this very session: sshd logged it with one of the
    /// session's ancestor process ids, for this user from this address.
    /// Its fingerprint is the key sshd accepted for Telepad.
    pub fn this_session(&self) -> Option<&Login> {
        let ip = self.client_ip();
        self.logins.iter().rev().find(|l| {
            l.pid != 0 && self.pids.contains(&l.pid) && l.user == self.user && l.from == ip
        })
    }

    /// The address part of `$SSH_CLIENT`.
    pub fn client_ip(&self) -> &str {
        self.client.split_whitespace().next().unwrap_or("")
    }
}

/// The read-only script: identity, journal access, logins, `authorized_keys`.
pub fn script() -> String {
    format!(
        r#"LC_ALL=C
export LC_ALL
if ! command -v journalctl >/dev/null 2>&1; then echo "TELEPAD-LOG nojournal"; exit 0; fi
echo "TELEPAD-LOG user $(id -un 2>/dev/null)"
echo "TELEPAD-LOG groups $(id -nG 2>/dev/null)"
echo "TELEPAD-LOG client ${{SSH_CLIENT:-}}"
p=$PPID; pids=""
for _ in 1 2 3 4; do
  case "$p" in ''|0|1) break ;; esac
  pids="$pids $p"
  set -- $(sed 's/^.*) //' "/proc/$p/stat" 2>/dev/null)
  p=${{2:-}}
done
echo "TELEPAD-LOG pids$pids"
journalctl -n 0 --no-pager 2>&1 | head -n 5 | sed 's/^/TELEPAD-LOG probe /'
echo "TELEPAD-LOG first $(journalctl --no-pager -o short-iso 2>/dev/null | grep -v '^--' | head -n 1)"
echo "TELEPAD-LOG begin"
journalctl --no-pager -o short-iso -t sshd -t sshd-session --since "-{DAYS}d" 2>/dev/null | grep -E ': Accepted [A-Za-z/-]+ for ' | tail -n {LIMIT}
echo "TELEPAD-LOG end"
{list}"#,
        list = authorized_keys::list_script()
    )
}

/// Read the logins of one host (read-only; blocks up to the ssh timeout).
pub fn read(settings: &SshSettings, host: &HostEntry) -> Result<LoginLog, String> {
    if host.access == Access::Rdp {
        return Err("RDP-only host, no SSH".into());
    }
    let mut settings = settings.clone();
    settings.total_timeout = settings.total_timeout.max(READ_TIMEOUT);
    let (out, err, code) = run_script(&settings, host, &script())?;
    if !out.contains("TELEPAD-LOG") {
        return Err(explain_ssh_error(&err, code));
    }
    let mut log = parse(&out);
    log.own = authorized_keys::login_keys(&settings, host)
        .iter()
        .map(|(path, k)| {
            let file = std::path::Path::new(path)
                .file_name()
                .map(|f| f.to_string_lossy().trim_end_matches(".pub").to_string())
                .unwrap_or_default();
            let name = if k.comment.is_empty() {
                file
            } else {
                format!("{file} — {}", k.comment)
            };
            (k.fingerprint(), name)
        })
        .collect();
    if let Ok(k) = &mut log.keys {
        k.login = log.own.iter().map(|(fp, _)| fp.clone()).collect();
    }
    Ok(log)
}

/// Read the script's output.
pub fn parse(out: &str) -> LoginLog {
    let mut log = LoginLog::default();
    let mut lines = out.lines();
    let mut in_body = false;
    let mut probe = String::new();
    let mut rest = String::new();
    for l in lines.by_ref() {
        let l = l.trim_end_matches('\r');
        if in_body {
            if l.trim() == "TELEPAD-LOG end" {
                in_body = false;
                continue;
            }
            if let Some(x) = parse_line(l) {
                log.logins.push(x);
            }
            continue;
        }
        if l.starts_with("TELEPAD-AK") {
            rest.push_str(l);
            rest.push('\n');
            break;
        }
        let Some(r) = l.strip_prefix("TELEPAD-LOG") else {
            continue;
        };
        let r = r.strip_prefix(' ').unwrap_or(r);
        let (word, value) = r.split_once(' ').unwrap_or((r, ""));
        match word {
            "nojournal" => log.access = JournalAccess::NoJournal,
            "user" => log.user = value.trim().to_string(),
            "groups" => log.groups = value.split_whitespace().map(String::from).collect(),
            "client" => log.client = value.trim().to_string(),
            "pids" => {
                log.pids = value
                    .split_whitespace()
                    .filter_map(|p| p.parse().ok())
                    .collect()
            }
            "probe" => {
                probe.push_str(value);
                probe.push('\n');
            }
            "first" => log.first = parse_time(value.trim()).map(|(t, _)| t),
            "begin" => in_body = true,
            _ => {}
        }
    }
    for l in lines {
        rest.push_str(l);
        rest.push('\n');
    }
    if log.access != JournalAccess::NoJournal {
        log.access = if denied(&probe) && log.user != "root" {
            JournalAccess::Denied
        } else {
            JournalAccess::Readable
        };
        if log.access == JournalAccess::Denied {
            // Without access journalctl may still show the user's own
            // journal; the oldest entry of that says nothing.
            log.first = None;
        }
    }
    log.truncated = log.logins.len() >= LIMIT;
    log.keys = authorized_keys::parse_list(&rest);
    log
}

/// journalctl's words for "you cannot read the system journal".
fn denied(probe: &str) -> bool {
    let p = probe.to_ascii_lowercase();
    p.contains("insufficient permissions")
        || p.contains("not seeing messages from other users")
        || p.contains("no journal files were opened")
}

/// One `journalctl -o short-iso` line with sshd's `Accepted …` message.
pub fn parse_line(line: &str) -> Option<Login> {
    let (ts, rest) = line.split_once(' ')?;
    let (time, epoch) = parse_time(ts)?;
    // "<host> <ident>[<pid>]: Accepted …"
    let (_host, rest) = rest.split_once(' ')?;
    let (ident, msg) = rest.split_once(": ")?;
    let pid = ident
        .split_once('[')
        .and_then(|(_, p)| p.trim_end_matches(']').parse().ok())
        .unwrap_or(0);
    let msg = msg.trim().strip_prefix("Accepted ")?;
    let w: Vec<&str> = msg.split_whitespace().collect();
    // <method> for <user> from <ip> port <n> <proto>[: <type> <fp> [ID <id> …]]
    if w.len() < 7 || w[1] != "for" || w[3] != "from" || w[5] != "port" {
        return None;
    }
    let mut l = Login {
        epoch,
        time,
        method: w[0].to_string(),
        user: w[2].to_string(),
        from: w[4].to_string(),
        port: w[6].parse().unwrap_or(0),
        pid,
        ..Login::default()
    };
    if let Some(kt) = w.get(8) {
        l.key_type = (*kt).to_string();
        l.fingerprint = w.get(9).map(|s| s.to_string()).unwrap_or_default();
    }
    if let Some(i) = w.iter().position(|&x| x == "ID") {
        l.cert_id = w.get(i + 1).map(|s| s.to_string()).unwrap_or_default();
    }
    Some(l)
}

/// `2026-11-23T17:00:14+0700` (also `+07:00`, `Z`, fractions) → shown time
/// and Unix seconds.
fn parse_time(ts: &str) -> Option<(String, i64)> {
    let b = ts.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' || b[13] != b':' {
        return None;
    }
    let days = crate::eol::days_from_iso(&ts[..10])?;
    let num = |r: std::ops::Range<usize>| ts.get(r)?.parse::<i64>().ok();
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut tz = &ts[19..];
    if let Some(f) = tz.strip_prefix('.') {
        tz = f.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match tz.as_bytes().first() {
        None | Some(b'Z') => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let d: String = tz[1..].chars().filter(char::is_ascii_digit).collect();
            let oh: i64 = d.get(..2)?.parse().ok()?;
            let om: i64 = d.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
            let o = oh * 3600 + om * 60;
            if sign == b'-' { -o } else { o }
        }
        _ => return None,
    };
    let epoch = days * 86_400 + h * 3600 + mi * 60 + s - offset;
    Some((format!("{} {}", &ts[..10], &ts[11..19]), epoch))
}

/// Interactive ssh for Register (`true`) / Unregister: the terminal shows
/// the exact command and runs it only after Enter.
pub fn terminal_args(settings: &SshSettings, host: &HostEntry, register: bool) -> Vec<String> {
    let script = if register {
        register_script()
    } else {
        unregister_script()
    };
    crate::collector::terminal_script_args(settings, host, &script)
}

/// Terminal script for **Register Systemd Journal**: shows the exact command
/// for the login user, runs it after Enter, sudo asks for the password.
pub fn register_script() -> String {
    access_script(
        "Register Systemd Journal",
        "Lets this user read the system journal (SSH logins) without sudo. Reading only: no root, logs cannot be changed. Takes effect on the next login.",
        "sudo usermod -aG systemd-journal",
        "\"$U\"",
        "",
    )
}

/// Terminal script for **Unregister**: removes the user from the group.
pub fn unregister_script() -> String {
    access_script(
        "Unregister Systemd Journal",
        "Removes this user from the systemd-journal group. Takes effect on the next login.",
        "sudo gpasswd -d",
        "\"$U\"",
        " systemd-journal",
    )
}

fn access_script(title: &str, what: &str, cmd: &str, user: &str, tail: &str) -> String {
    let q = crate::platform::shell_quote;
    format!(
        r#"U=$(id -un)
echo {title}
echo
echo {what}
echo
if ! grep -q '^systemd-journal:' /etc/group 2>/dev/null; then
  echo 'This host has no systemd-journal group (no systemd?): nothing to do.'
  printf 'Press Enter to close. '
  read _ </dev/tty
  exit 1
fi
echo 'Command:'
printf '  %s %s%s\n\n' {cmd} "$U" {tail}
printf 'Press Enter to run it, or Ctrl+C to cancel. '
read _ </dev/tty
{cmd_raw} {user}{tail_raw} </dev/tty
rc=$?
echo
if [ "$rc" = 0 ]; then echo 'Done. Click Refresh on the Log info page in Telepad.'; fi
printf 'Finished (exit %s). Press Enter to close. ' "$rc"
read _ </dev/tty
"#,
        title = q(&format!("=== Telepad: {title} ===")),
        what = q(what),
        cmd = q(cmd),
        tail = q(tail),
        cmd_raw = cmd,
        tail_raw = tail,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBIAN: &str = "TELEPAD-LOG user alice
TELEPAD-LOG groups alice sudo systemd-journal
TELEPAD-LOG client 192.0.2.10 51544 22
TELEPAD-LOG pids 2001 2000 1500
TELEPAD-LOG probe -- No entries --
TELEPAD-LOG first 2026-10-01T08:00:00+0700 web1 kernel: Linux version 6.12
TELEPAD-LOG begin
2026-11-23T17:00:14+0700 web1 sshd-session[918413]: Accepted publickey for alice from 192.0.2.10 port 11441 ssh2: ED25519 SHA256:Qm3xTestKeyAliceAAAAAAAAAAAAAAAAAAAAAAAAAAA
2026-11-23T17:00:27+07:00 web1 sshd-session[918816]: Accepted publickey for bob from 198.51.100.7 port 61728 ssh2: ED25519 SHA256:7hZpTestKeyBobBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
2026-11-24T07:49:22+0700 web1 sshd[2000]: Accepted publickey for alice from 192.0.2.10 port 50634 ssh2: ED25519 SHA256:Qm3xTestKeyAliceAAAAAAAAAAAAAAAAAAAAAAAAAAA
TELEPAD-LOG end
TELEPAD-AK begin
# Admins
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl alice@laptop

TELEPAD-AK end
";

    #[test]
    fn reads_a_debian_host() {
        let log = parse(DEBIAN);
        assert_eq!(log.access, JournalAccess::Readable);
        assert_eq!(log.user, "alice");
        assert!(log.in_journal_group());
        assert_eq!(log.client_ip(), "192.0.2.10");
        assert_eq!(log.pids, vec![2001, 2000, 1500]);
        assert_eq!(log.first.as_deref(), Some("2026-10-01 08:00:00"));
        assert_eq!(log.logins.len(), 3);
        let b = &log.logins[1];
        assert_eq!(
            (b.user.as_str(), b.from.as_str(), b.port, b.method.as_str()),
            ("bob", "198.51.100.7", 61728, "publickey")
        );
        assert_eq!(b.key_type, "ED25519");
        assert_eq!(
            b.fingerprint,
            "SHA256:7hZpTestKeyBobBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
        );
        assert_eq!(b.pid, 918816);
        assert!(!log.truncated);
        let keys = log.keys.as_ref().unwrap();
        assert_eq!(keys.keys.len(), 1);
        assert_eq!(keys.keys[0].group.as_deref(), Some("Admins"));
        // sshd[2000] is an ancestor of this session: its key is the one used.
        assert_eq!(log.this_session().map(|l| l.pid), Some(2000));
    }

    #[test]
    fn times_with_both_offset_styles_are_the_same_instant() {
        let a = parse_time("2026-11-23T17:00:14+0700").unwrap();
        let b = parse_time("2026-11-23T17:00:14+07:00").unwrap();
        let z = parse_time("2026-11-23T10:00:14Z").unwrap();
        let f = parse_time("2026-11-23T10:00:14.123456+00:00").unwrap();
        assert_eq!(a.1, b.1);
        assert_eq!(a.1, z.1);
        assert_eq!(z.1, f.1);
        assert_eq!(a.0, "2026-11-23 17:00:14");
        let neg = parse_time("2026-11-23T05:00:14-0500").unwrap();
        assert_eq!(neg.1, z.1);
        assert!(parse_time("Nov 23 17:00:14").is_none());
    }

    #[test]
    fn reads_rhel_and_other_methods() {
        // RHEL / CentOS (sshd), password, keyboard-interactive, certificate, IPv6.
        let lines = [
            "2026-11-20T09:15:02+0000 db1 sshd[4321]: Accepted publickey for root from 192.0.2.5 port 40022 ssh2: RSA SHA256:RsaTestKeyRootCCCCCCCCCCCCCCCCCCCCCCCCCCCCC",
            "2026-11-20T09:16:00+0000 db1 sshd[4400]: Accepted password for carol from 2001:db8::5 port 50000 ssh2",
            "2026-11-20T09:17:00+0000 db1 sshd[4401]: Accepted keyboard-interactive/pam for dave from 192.0.2.6 port 50001 ssh2",
            "2026-11-20T09:18:00+0000 db1 sshd[4402]: Accepted publickey for erin from 192.0.2.7 port 50002 ssh2: ED25519-CERT SHA256:abc ID erin@corp (serial 7) CA ED25519 SHA256:def",
        ];
        let l: Vec<Login> = lines.iter().filter_map(|l| parse_line(l)).collect();
        assert_eq!(l.len(), 4);
        assert_eq!(
            (l[0].user.as_str(), l[0].key_type.as_str()),
            ("root", "RSA")
        );
        assert_eq!(l[1].method, "password");
        assert_eq!(l[1].from, "2001:db8::5");
        assert!(l[1].fingerprint.is_empty());
        assert_eq!(l[2].method, "keyboard-interactive/pam");
        assert_eq!(l[3].key_type, "ED25519-CERT");
        assert_eq!(l[3].fingerprint, "SHA256:abc");
        assert_eq!(l[3].cert_id, "erin@corp");
        assert!(parse_line("2026-11-20T09:18:00+0000 db1 sshd[1]: Failed password for x from 192.0.2.1 port 1 ssh2").is_none());
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn no_access_and_no_journal() {
        let out = "TELEPAD-LOG user alice
TELEPAD-LOG groups alice
TELEPAD-LOG client 192.0.2.10 1 22
TELEPAD-LOG pids 10 9
TELEPAD-LOG probe Hint: You are currently not seeing messages from other users and the system.
TELEPAD-LOG probe       Users in groups 'adm', 'systemd-journal' can see all messages.
TELEPAD-LOG first 2026-11-01T00:00:00+0000 h systemd[1]: user stuff
TELEPAD-LOG begin
TELEPAD-LOG end
TELEPAD-AK none
";
        let log = parse(out);
        assert_eq!(log.access, JournalAccess::Denied);
        assert!(!log.in_journal_group());
        assert!(log.first.is_none());
        assert!(log.keys.as_ref().unwrap().missing);
        let old = parse(
            "TELEPAD-LOG user bob\nTELEPAD-LOG probe No journal files were opened due to insufficient permissions.\nTELEPAD-LOG begin\nTELEPAD-LOG end\n",
        );
        assert_eq!(old.access, JournalAccess::Denied);
        let root = parse("TELEPAD-LOG user root\nTELEPAD-LOG probe insufficient permissions\n");
        assert_eq!(root.access, JournalAccess::Readable);
        assert_eq!(
            parse("TELEPAD-LOG nojournal\n").access,
            JournalAccess::NoJournal
        );
    }

    #[test]
    fn the_limit_marks_the_log_truncated() {
        let mut out = String::from("TELEPAD-LOG user a\nTELEPAD-LOG begin\n");
        for i in 0..LIMIT {
            out.push_str(&format!(
                "2026-11-20T09:00:00+0000 h sshd[{i}]: Accepted password for a from 192.0.2.1 port 1 ssh2\n"
            ));
        }
        out.push_str("TELEPAD-LOG end\n");
        assert!(parse(&out).truncated);
    }

    #[test]
    fn the_scripts_are_read_only_and_valid_sh() {
        let s = script();
        assert!(!s.contains("sudo"), "reading never uses sudo");
        assert!(s.contains("-t sshd -t sshd-session"));
        assert!(s.contains(&format!("--since \"-{DAYS}d\"")));
        assert!(s.contains(&format!("tail -n {LIMIT}")));
        assert!(s.contains("TELEPAD-AK begin"));
        let r = register_script();
        assert!(r.contains("sudo usermod -aG systemd-journal \"$U\" </dev/tty"));
        assert!(
            r.contains("read _ </dev/tty\nsudo usermod"),
            "runs only after Enter"
        );
        let u = unregister_script();
        assert!(u.contains("sudo gpasswd -d \"$U\" systemd-journal </dev/tty"));
        for sh in [s, r, u] {
            if let Ok(out) = std::process::Command::new("sh")
                .args(["-n", "-c", &sh])
                .output()
            {
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
}
