//! Optional rsync mode: only when `rsync` runs on this computer and on the
//! host. Copies go through Telepad's ssh config (`-e "ssh -F …"`) and every
//! run can be previewed with `--dry-run`.

use std::process::{Command, Stdio};

use crate::collector::SshSettings;
use crate::config::HostEntry;
use crate::platform;

/// `Ok(())` when rsync can be used here, else why not.
pub fn local_available() -> Result<(), String> {
    let mut c = Command::new("rsync");
    c.arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    platform::hide_console(&mut c);
    match c.status() {
        Ok(s) if s.success() => Ok(()),
        _ => Err("rsync is not installed on this computer".into()),
    }
}

/// Read-only check on the host.
pub const REMOTE_CHECK: &str =
    "command -v rsync >/dev/null 2>&1 && echo TELEPAD-RSYNC yes || echo TELEPAD-RSYNC no\n";

pub fn remote_has(out: &str) -> bool {
    out.lines().any(|l| l.trim() == "TELEPAD-RSYNC yes")
}

/// Single-quote for rsync's `-e` string (rsync splits it like a shell).
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The `-e` command: ssh with Telepad's config and batch options.
pub fn ssh_command(settings: &SshSettings, host: &HostEntry) -> String {
    let mut parts = vec![
        q(&settings.ssh_program.display().to_string()),
        "-F".into(),
        q(&settings.config_path.display().to_string()),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={}", settings.connect_timeout.as_secs()),
    ];
    if host.identity_file.is_none()
        && let Some(id) = &settings.fallback_identity
    {
        parts.extend(["-i".into(), q(&id.display().to_string())]);
    }
    parts.join(" ")
}

/// rsync arguments. `sources` are copied *into* `dest_dir` (keeps their
/// names); `--protect-args` keeps remote names away from the remote shell.
pub fn args(
    settings: &SshSettings,
    host: &HostEntry,
    upload: bool,
    sources: &[String],
    dest_dir: &str,
    dry_run: bool,
    opts: &super::transfer::Options,
) -> Vec<String> {
    let mut a = vec![
        "-a".to_string(),
        "--protect-args".into(),
        "--itemize-changes".into(),
        "--human-readable".into(),
        "-e".into(),
        ssh_command(settings, host),
    ];
    if dry_run {
        a.push("--dry-run".into());
    }
    // Transfer settings (#131): rsync copies bytes as they are (no Text mode).
    if opts.newer_only {
        a.push("--update".into());
    }
    for p in &opts.exclude {
        a.push(format!("--exclude={p}/"));
    }
    a.push("--".into());
    let remote = |p: &str| format!("{}:{p}", host.alias);
    let dir = if dest_dir.ends_with('/') {
        dest_dir.to_string()
    } else {
        format!("{dest_dir}/")
    };
    if upload {
        a.extend(sources.iter().cloned());
        a.push(remote(&dir));
    } else {
        a.extend(sources.iter().map(|s| remote(s)));
        a.push(dir);
    }
    a
}

/// Changed paths from `--itemize-changes` output (`>f+++++++++ name`).
pub fn itemized(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let (code, name) = l.split_once(' ')?;
            (code.len() >= 9 && !code.starts_with('.')).then(|| name.trim().to_string())
        })
        .filter(|n| !n.is_empty())
        .collect()
}

/// Whether the host has rsync (read-only check).
pub fn remote_available(settings: &SshSettings, host: &HostEntry) -> Result<(), String> {
    let (out, err, code) = crate::collector::run_script(settings, host, REMOTE_CHECK)?;
    if remote_has(&out) {
        Ok(())
    } else if code == Some(0) {
        Err("rsync is not installed on the host".into())
    } else {
        Err(crate::collector::explain_ssh_error(&err, code))
    }
}

/// Run rsync; returns the changed paths (`--itemize-changes`).
pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    let mut c = Command::new("rsync");
    c.args(args).stdin(Stdio::null());
    platform::hide_console(&mut c);
    let out = c.output().map_err(|e| format!("cannot start rsync: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        Ok(itemized(&text))
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("rsync failed: {}", err.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn builds_rsync_arguments() {
        let mut s = SshSettings::new(PathBuf::from("/home/me/my telepad/telepad.ssh_config"));
        s.ssh_program = PathBuf::from("/usr/bin/ssh");
        let h = HostEntry::new("web1");
        let opts = super::super::transfer::Options {
            newer_only: true,
            exclude: vec!["node_modules".into()],
            ..Default::default()
        };
        let up = args(&s, &h, true, &["/tmp/site".into()], "/srv/www", true, &opts);
        assert!(up.contains(&"--update".to_string()));
        assert!(up.contains(&"--exclude=node_modules/".to_string()));
        assert_eq!(
            up[up.iter().position(|a| a == "-e").unwrap() + 1],
            "'/usr/bin/ssh' -F '/home/me/my telepad/telepad.ssh_config' -o BatchMode=yes -o ConnectTimeout=8"
        );
        assert!(
            up.contains(&"--dry-run".to_string()) && up.contains(&"--protect-args".to_string())
        );
        assert_eq!(
            &up[up.len() - 2..],
            &["/tmp/site".to_string(), "web1:/srv/www/".into()]
        );
        let down = args(
            &s,
            &h,
            false,
            &["/srv/a b.log".into()],
            "/tmp/in",
            false,
            &Default::default(),
        );
        assert!(!down.contains(&"--update".to_string()));
        assert_eq!(
            &down[down.len() - 2..],
            &["web1:/srv/a b.log".to_string(), "/tmp/in/".into()]
        );
        assert!(!down.contains(&"--dry-run".to_string()));
    }

    #[test]
    fn reads_remote_check_and_itemized_output() {
        assert!(remote_has("TELEPAD-RSYNC yes\n"));
        assert!(!remote_has("TELEPAD-RSYNC no\n"));
        assert_eq!(
            itemized(
                "sending incremental file list\n>f+++++++++ site/index.html\ncd+++++++++ site/img/\n.d..t...... site/\n"
            ),
            vec!["site/index.html".to_string(), "site/img/".into()]
        );
    }
}
