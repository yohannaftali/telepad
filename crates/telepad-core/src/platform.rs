//! Everything OS-specific lives here.

use std::path::PathBuf;
use std::process::Command;

/// The OpenSSH client to use. `TELEPAD_SSH` overrides the default.
pub fn ssh_program() -> PathBuf {
    if let Some(p) = std::env::var_os("TELEPAD_SSH") {
        return PathBuf::from(p);
    }
    #[cfg(target_os = "windows")]
    {
        // The Windows-bundled OpenSSH understands Windows paths in `-F` and
        // `~` in IdentityFile; Git-for-Windows' ssh may not be on PATH anyway.
        if let Some(root) = std::env::var_os("SystemRoot") {
            let p = PathBuf::from(root)
                .join("System32")
                .join("OpenSSH")
                .join("ssh.exe");
            if p.exists() {
                return p;
            }
        }
    }
    PathBuf::from("ssh")
}

/// Open an https:// link in the default browser (release notes).
pub fn open_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_control() || c == '"' || c == ' ') {
        return Err(format!("not an https link: {url}"));
    }
    open_url_impl(url).map_err(|e| format!("cannot open the browser: {e}"))
}

#[cfg(target_os = "windows")]
fn open_url_impl(url: &str) -> std::io::Result<()> {
    // No shell involved: the URL is one argument to the protocol handler.
    Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_url_impl(url: &str) -> std::io::Result<()> {
    Command::new("open").arg(url).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_url_impl(url: &str) -> std::io::Result<()> {
    Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

/// `ssh-keygen` next to the OpenSSH client Telepad uses (#101).
pub fn ssh_keygen_program() -> PathBuf {
    let ssh = ssh_program();
    let name = if cfg!(target_os = "windows") {
        "ssh-keygen.exe"
    } else {
        "ssh-keygen"
    };
    match ssh.parent() {
        Some(dir) if !dir.as_os_str().is_empty() && dir.join(name).exists() => dir.join(name),
        _ => PathBuf::from(name),
    }
}

/// A private key others can read (#101). macOS / Linux: group / other mode
/// bits; Windows: not checked here (ssh-keygen and the user profile set the
/// ACL).
pub fn private_key_permission_warning(
    _path: &std::path::Path,
    meta: &std::fs::Metadata,
) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Some(format!(
                "mode {mode:o}: others can read it (ssh refuses such keys); chmod 600"
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    None
}

/// What to install when `ssh-keygen` is missing (#101).
pub fn ssh_keygen_missing() -> String {
    if cfg!(target_os = "windows") {
        "ssh-keygen not found: add the Windows feature \"OpenSSH Client\" (Settings > System > Optional features)".into()
    } else if cfg!(target_os = "macos") {
        "ssh-keygen not found (it comes with macOS; check PATH or TELEPAD_SSH)".into()
    } else {
        "ssh-keygen not found: install the openssh-client package".into()
    }
}

/// `user@computer`, the default comment of a new key.
pub fn user_at_host() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "me".into());
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "computer".into());
    format!("{user}@{host}")
}

/// A folder for keys: owner only (`0700` on macOS / Linux; Windows: the
/// profile's own ACL, inherited).
pub fn restrict_dir_to_owner(dir: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot set the permissions of {}: {e}", dir.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Make a file readable by its owner only (#101: written private keys).
/// macOS / Linux: mode 600. Windows: no inherited entries, full control for
/// the current user only (`icacls`, arguments passed separately).
pub fn restrict_to_owner(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot set the permissions of {}: {e}", path.display()))
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let user = std::env::var("USERNAME").map_err(|_| "USERNAME is not set".to_string())?;
        let out = Command::new("icacls.exe")
            .arg(path)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("{user}:F"))
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|e| format!("icacls: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "cannot restrict {} to your user: {}",
                path.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = path;
        Ok(())
    }
}

/// Show a local folder in the system's file manager (#100: a history
/// line's folder). Only an existing folder; no shell involved.
pub fn open_folder(path: &std::path::Path) -> Result<(), String> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .filter(|p| p.is_dir())
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| format!("{} does not exist any more", path.display()))?
    };
    open_folder_impl(&dir).map_err(|e| format!("cannot open {}: {e}", dir.display()))
}

#[cfg(target_os = "windows")]
fn open_folder_impl(dir: &std::path::Path) -> std::io::Result<()> {
    Command::new("explorer.exe").arg(dir).spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_folder_impl(dir: &std::path::Path) -> std::io::Result<()> {
    Command::new("open").arg(dir).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_folder_impl(dir: &std::path::Path) -> std::io::Result<()> {
    Command::new("xdg-open").arg(dir).spawn().map(|_| ())
}

/// Per-user data folder for an installed Telepad: `%APPDATA%\Telepad` on Windows,
/// `~/Library/Application Support/Telepad` on macOS, `$XDG_DATA_HOME/telepad`
/// (or `~/.local/share/telepad`) elsewhere.
pub fn user_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Telepad"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Application Support")
                .join("Telepad")
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("telepad"))
            .or_else(|| {
                std::env::var_os("HOME").map(|h| {
                    PathBuf::from(h)
                        .join(".local")
                        .join("share")
                        .join("telepad")
                })
            })
    }
}

/// The `.app` bundle an executable lives in (`…/Telepad.app/Contents/MacOS/telepad`).
pub fn app_bundle_of(exe: &std::path::Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("app"));
    is_bundle.then(|| bundle.to_path_buf())
}

/// After the updater swapped the executable: inside a macOS `.app`, re-apply the
/// ad-hoc signature, because replacing a file breaks the bundle's seal and
/// macOS may then refuse to start it. Elsewhere nothing to do.
pub fn after_self_update(exe: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = app_bundle_of(exe) {
        let out = Command::new("/usr/bin/codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&bundle)
            .output()
            .map_err(|e| format!("cannot run codesign: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "updated, but re-signing {} failed: {}",
                bundle.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = exe;
    Ok(())
}

/// Add execute permission (rwxr-xr-x) on Unix; nothing to do on Windows.
pub fn make_executable(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(path)?.permissions();
        perm.set_mode(perm.mode() | 0o755);
        std::fs::set_permissions(path, perm)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Stop background ssh processes from flashing a console window on Windows.
pub fn hide_console(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = cmd;
}

/// POSIX-shell quoting for display and for terminals that need a command string.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@=~+,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

pub fn command_line(program: &std::path::Path, args: &[String]) -> String {
    std::iter::once(program.display().to_string())
        .chain(args.iter().cloned())
        .map(|a| shell_quote(&a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One ICMP echo via the system `ping` (no admin rights needed). Returns the
/// round-trip time in ms when a reply came back. `host` is validated so it can
/// never be read as a ping option.
pub fn ping(host: &str, timeout: std::time::Duration) -> Option<f32> {
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c))
    {
        return None;
    }
    let ms = timeout.as_millis().max(500);
    let mut cmd = Command::new("ping");
    #[cfg(target_os = "windows")]
    cmd.args(["-n", "1", "-w", &ms.to_string(), host]);
    #[cfg(target_os = "macos")]
    cmd.args(["-c", "1", "-W", &ms.to_string(), host]);
    #[cfg(all(unix, not(target_os = "macos")))]
    cmd.args(["-c", "1", "-W", &ms.div_ceil(1000).to_string(), host]);
    hide_console(&mut cmd);
    let out = cmd.stdin(std::process::Stdio::null()).output().ok()?;
    parse_ping(&String::from_utf8_lossy(&out.stdout))
}

/// A reply line contains `TTL=`/`ttl=` in every locale; Windows `ping` exits 0
/// even for "Destination host unreachable", so the exit code is not used.
pub fn parse_ping(out: &str) -> Option<f32> {
    let line = out
        .lines()
        .find(|l| l.to_ascii_lowercase().contains("ttl="))?;
    let lower = line.to_ascii_lowercase();
    if let Some(i) = lower.find("time<") {
        let _ = i;
        return Some(0.5);
    }
    // Localised output (e.g. "waktu=3ms") still has the TTL line: a reply, time unknown.
    let Some(t) = lower.split("time=").nth(1) else {
        return Some(0.0);
    };
    let num: String = t
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    Some(num.parse().unwrap_or(0.0))
}

/// Open a Remote Desktop session to `host:port`.
pub fn open_rdp(host: &str, port: u16) -> Result<(), String> {
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_:[]".contains(c))
    {
        return Err(format!("invalid RDP host '{host}'"));
    }
    let target = if port == 3389 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    open_rdp_impl(&target).map_err(|e| format!("cannot start Remote Desktop: {e}"))
}

#[cfg(target_os = "windows")]
fn open_rdp_impl(target: &str) -> std::io::Result<()> {
    // mstsc is a GUI app; the target is one validated argument, no shell involved.
    Command::new("mstsc.exe")
        .arg(format!("/v:{target}"))
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_rdp_impl(target: &str) -> std::io::Result<()> {
    // Microsoft Remote Desktop / Windows App handles rdp:// URLs.
    Command::new("open")
        .arg(format!("rdp://full%20address=s:{target}"))
        .spawn()
        .map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_rdp_impl(target: &str) -> std::io::Result<()> {
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no RDP client found");
    for (prog, arg) in [
        ("xfreerdp3", format!("/v:{target}")),
        ("xfreerdp", format!("/v:{target}")),
        ("remmina", format!("rdp://{target}")),
    ] {
        match Command::new(prog).arg(&arg).spawn() {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Open Remote Desktop with saved settings (#102): Windows / macOS get a
/// temporary `.rdp` file (owner-only, removed after the client read it);
/// Linux gets `xfreerdp` arguments (Remmina the file). Returns what the
/// client could not take, for a note.
pub fn open_rdp_with(
    settings: &crate::rdp::RdpSettings,
    host: &str,
    port: u16,
    alias: &str,
) -> Result<Vec<&'static str>, String> {
    settings.validate()?;
    let target_host = if settings.address.is_empty() {
        host
    } else {
        settings.address.as_str()
    };
    if !target_host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || ".-_:[]".contains(c))
        || target_host.is_empty()
    {
        return Err(format!("invalid RDP host '{target_host}'"));
    }
    let safe: String = alias
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let write_file = || -> Result<PathBuf, String> {
        let dir = std::env::temp_dir().join("telepad-rdp");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        restrict_dir_to_owner(&dir)?;
        let file = dir.join(format!("{safe}-{}.rdp", std::process::id()));
        std::fs::write(&file, crate::rdp::encode_file(&settings.render(host, port)))
            .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
        restrict_to_owner(&file)?;
        Ok(file)
    };
    // The client reads the file when it starts; remove it a little later.
    let remove_later = |file: PathBuf| {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(30));
            let _ = std::fs::remove_file(file);
        });
    };
    #[cfg(target_os = "windows")]
    {
        let file = write_file()?;
        Command::new("mstsc.exe")
            .arg(&file)
            .spawn()
            .map_err(|e| format!("cannot start Remote Desktop: {e}"))?;
        remove_later(file);
        Ok(Vec::new())
    }
    #[cfg(target_os = "macos")]
    {
        let file = write_file()?;
        Command::new("open")
            .arg(&file)
            .spawn()
            .map_err(|e| format!("cannot open Windows App / Microsoft Remote Desktop: {e}"))?;
        remove_later(file);
        Ok(Vec::new())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let (args, not) = settings.xfreerdp_args(host, port);
        for prog in ["xfreerdp3", "xfreerdp"] {
            if Command::new(prog).args(&args).spawn().is_ok() {
                return Ok(not);
            }
        }
        let file = write_file()?;
        Command::new("remmina")
            .arg(&file)
            .spawn()
            .map_err(|e| format!("no RDP client found (xfreerdp, Remmina): {e}"))?;
        remove_later(file);
        Ok(Vec::new())
    }
}

/// Open an interactive ssh session in a new terminal window.
pub fn open_terminal(program: &std::path::Path, args: &[String]) -> Result<(), String> {
    open_terminal_impl(program, args).map_err(|e| format!("cannot open terminal: {e}"))
}

/// Windows: Rust always passes the parent's std handles to the child, and a
/// GUI parent has none, so spawning ssh.exe directly (even with
/// CREATE_NEW_CONSOLE) leaves it with no terminal. Instead, hand the command
/// to Windows Terminal, or to `cmd /c start`, which give it a real console.
#[cfg(target_os = "windows")]
fn open_terminal_impl(program: &std::path::Path, args: &[String]) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let bad = |chars: &str| {
        std::iter::once(program.display().to_string())
            .chain(args.iter().cloned())
            .any(|a| a.chars().any(|c| chars.contains(c) || c.is_control()))
    };

    let wt = std::env::var_os("LOCALAPPDATA")
        .map(|d| std::path::PathBuf::from(d).join(r"Microsoft\WindowsApps\wt.exe"))
        .filter(|p| p.exists());
    // `;` separates commands in wt's command line.
    if let Some(wt) = wt.filter(|_| !bad(";\"")) {
        let title = args.last().cloned().unwrap_or_else(|| "ssh".into());
        // One named window; each host opens as a new tab in it.
        return Command::new(wt)
            .args(["-w", "telepad", "new-tab", "--title", &title])
            .arg(program)
            .args(args)
            .spawn()
            .map(|_| ());
    }

    // Every argument is double-quoted, so only these could escape cmd's parsing.
    if bad("\"%!^") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config path or host alias contains characters that cannot be passed safely to cmd.exe",
        ));
    }
    let quoted: Vec<String> = std::iter::once(program.display().to_string())
        .chain(args.iter().cloned())
        .map(|a| format!("\"{a}\""))
        .collect();
    let title = args.last().cloned().unwrap_or_else(|| "ssh".into());
    Command::new("cmd.exe")
        .raw_arg(format!("/d /c start \"{title}\" {}", quoted.join(" ")))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_terminal_impl(program: &std::path::Path, args: &[String]) -> std::io::Result<()> {
    let line = command_line(program, args);
    let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
    Command::new("osascript")
        .args([
            "-e",
            &format!("tell application \"Terminal\" to do script \"{escaped}\""),
            "-e",
            "tell application \"Terminal\" to activate",
        ])
        .spawn()
        .map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_terminal_impl(program: &std::path::Path, args: &[String]) -> std::io::Result<()> {
    let candidates: &[(&str, &[&str])] = &[
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-x"]),
        ("alacritty", &["-e"]),
        ("kitty", &[]),
        ("xterm", &["-e"]),
    ];
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no terminal emulator found");
    for (term, pre) in candidates {
        match Command::new(term)
            .args(*pre)
            .arg(program)
            .args(args)
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ping_replies_and_ignores_unreachable() {
        assert_eq!(
            parse_ping("Reply from 198.51.100.5: bytes=32 time<1ms TTL=127"),
            Some(0.5)
        );
        assert_eq!(
            parse_ping("Reply from 10.0.0.1: bytes=32 time=12ms TTL=64"),
            Some(12.0)
        );
        assert_eq!(
            parse_ping("64 bytes from 10.0.0.1: icmp_seq=1 ttl=64 time=0.531 ms"),
            Some(0.531)
        );
        assert_eq!(
            parse_ping("Balasan dari 10.0.0.1: byte=32 waktu=3ms TTL=64"),
            Some(0.0)
        );
        assert_eq!(
            parse_ping("Reply from 198.51.100.1: Destination host unreachable."),
            None
        );
        assert_eq!(parse_ping("Request timed out."), None);
        assert_eq!(ping("-n", std::time::Duration::from_secs(1)), None);
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(shell_quote("git"), "git");
        assert_eq!(
            shell_quote("C:\\Users\\IT\\config"),
            "'C:\\Users\\IT\\config'"
        );
        assert_eq!(shell_quote("a b'c"), "'a b'\\''c'");
    }

    #[test]
    fn finds_the_app_bundle_of_an_executable() {
        let b = app_bundle_of(std::path::Path::new(
            "/Applications/Telepad.app/Contents/MacOS/telepad",
        ));
        assert_eq!(b, Some(PathBuf::from("/Applications/Telepad.app")));
        assert_eq!(
            app_bundle_of(std::path::Path::new("/opt/telepad/telepad")),
            None
        );
        assert_eq!(
            app_bundle_of(std::path::Path::new("/x/Contents/MacOS/telepad")),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn make_executable_sets_execute_bits() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!("telepad-x-{}", std::process::id()));
        std::fs::write(
            &p,
            b"#!/bin/sh
",
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        make_executable(&p).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        std::fs::remove_file(&p).ok();
        assert_eq!(mode, 0o755);
    }
}
