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
}
