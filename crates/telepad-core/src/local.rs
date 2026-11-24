//! My Computer (#129): the same report as a host's, read from the computer
//! Telepad runs on, without SSH. Read-only like the remote script: no admin,
//! no prompts, nothing written.
//!
//! - Windows has no `sh`: system facts come from the OS (`sysinfo`), listening
//!   ports from `netstat -ano`, pending reboot from `reg query`, Docker /
//!   Podman from their CLI with the same formats the remote script uses.
//! - Linux / macOS run the remote script itself with `sh -s`.

use std::process::{Command, Stdio};
use std::time::Duration;

use crate::model::{HostReport, Listener};
use crate::platform;

/// Longest a local read may take (Docker can be slow to answer).
pub const TIMEOUT: Duration = Duration::from_secs(40);

/// Collect everything about this computer. Blocks (run it on a thread).
pub fn collect() -> Result<HostReport, String> {
    let mut r = collect_impl()?;
    if r.hostname.is_empty() {
        r.hostname = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_default();
    }
    Ok(r)
}

#[cfg(not(target_os = "windows"))]
fn collect_impl() -> Result<HostReport, String> {
    use std::io::Write;
    let script = crate::collector::Scope::FULL.script();
    let mut child = Command::new("sh")
        .arg("-s")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run sh: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .map_err(|e| format!("cannot run the script: {e}"))?;
    }
    let out = wait(child, TIMEOUT)?;
    if !out.contains("===TELEPAD===") {
        return Err("the local script printed nothing".into());
    }
    Ok(crate::parse::parse_report(&out))
}

/// Wait for a child with a time limit, returning its standard output.
#[cfg(not(target_os = "windows"))]
fn wait(mut child: std::process::Child, limit: Duration) -> Result<String, String> {
    use std::io::Read;
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer within {} s", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    reader
        .join()
        .map_err(|_| "reading the output failed".to_string())
}

/// Output of a short read-only command, `None` when it is missing or fails.
/// Arguments are passed separately, never through a shell.
#[cfg(target_os = "windows")]
fn run(program: &str, args: &[&str]) -> Option<(bool, String)> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    platform::hide_console(&mut cmd);
    let out = cmd.output().ok()?;
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

#[cfg(target_os = "windows")]
fn collect_impl() -> Result<HostReport, String> {
    use crate::model::{Disk, Memory, NetAddr, Sensor, SensorKind};
    use sysinfo::{Components, Disks, Networks, ProcessesToUpdate, System};

    let mut sys = System::new().map_err(|e| e.to_string())?;
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(300)));
    sys.refresh_cpu_usage();

    let mut r = HostReport {
        hostname: System::host_name().unwrap_or_default(),
        os_name: System::name().unwrap_or_default(),
        os_pretty: System::long_os_version().unwrap_or_default(),
        os_id: "windows".into(),
        os_version_id: System::os_version().unwrap_or_default(),
        kernel: System::kernel_version().unwrap_or_default(),
        os_type: "Windows".into(),
        arch: System::cpu_arch(),
        uptime_secs: System::uptime().ok(),
        ..Default::default()
    };
    if let Some(c) = sys.cpus().first() {
        r.cpu_model = c.brand().trim().to_string();
    }
    r.cpu_cores = u32::try_from(sys.cpus().len()).ok().filter(|&n| n > 0);
    r.cpu_usage_pct = Some(sys.global_cpu_usage());
    r.memory = Some(Memory {
        total_kb: sys.total_memory() / 1024,
        available_kb: sys.available_memory() / 1024,
        swap_total_kb: sys.total_swap() / 1024,
        swap_free_kb: sys.free_swap() / 1024,
    });
    let disks = Disks::new_with_refreshed_list().ok();
    for d in disks.iter().flatten() {
        let total = d.total_space() / 1024;
        if total == 0 {
            continue;
        }
        let free = d.available_space() / 1024;
        r.disks.push(Disk {
            filesystem: d.file_system().to_string_lossy().into_owned(),
            mount: d.mount_point().display().to_string(),
            total_kb: total,
            used_kb: total.saturating_sub(free),
            available_kb: free,
        });
    }
    let nets = Networks::new_with_refreshed_list().ok();
    for (name, data) in nets.iter().flat_map(|n| n.iter()) {
        for ip in data.ip_networks() {
            let a = ip.addr;
            if a.is_loopback() || a.to_string().starts_with("fe80") {
                continue;
            }
            r.addresses.push(NetAddr {
                interface: name.to_string_lossy().into_owned(),
                family: if a.is_ipv4() { "IPv4" } else { "IPv6" }.into(),
                address: format!("{a}/{}", ip.prefix),
            });
        }
    }
    r.addresses
        .sort_by(|a, b| a.interface.cmp(&b.interface).then(a.family.cmp(&b.family)));
    // Usually empty without admin rights (WMI thermal zones).
    let comps = Components::new_with_refreshed_list().ok();
    for c in comps.iter().flatten() {
        if let Some(t) = c
            .temperature()
            .filter(|t| *t > -40.0 && *t < 150.0 && *t != 0.0)
        {
            r.sensors.push(Sensor {
                kind: SensorKind::Temperature,
                chip: String::new(),
                label: c.label().to_string(),
                value: t,
            });
        }
    }
    let names = |pid: u32| {
        sys.process(sysinfo::Pid::from_u32(pid))
            .map(|p| {
                p.name()
                    .to_string_lossy()
                    .trim_end_matches(".exe")
                    .to_string()
            })
            .unwrap_or_default()
    };
    if let Some((true, out)) = run("netstat", &["-ano"]) {
        r.listeners = parse_netstat(&out, names);
    }
    r.maintenance.reboot_required = Some(reboot_pending());
    r.maintenance.today = Some(crate::eol::today());
    r.docker = docker();
    Ok(r)
}

/// Windows notes a pending reboot under these registry keys.
#[cfg(target_os = "windows")]
fn reboot_pending() -> bool {
    const KEYS: [&str; 2] = [
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
    ];
    KEYS.iter()
        .any(|k| matches!(run("reg", &["query", k]), Some((true, _))))
}

/// Docker (else Podman) as the remote script reads it: the same `ps` and
/// `inspect` formats, so [`crate::parse::parse_docker`] reads both.
#[cfg(target_os = "windows")]
fn docker() -> crate::model::Docker {
    let rt = ["docker", "podman"]
        .into_iter()
        .find(|rt| matches!(run(rt, &["ps", "-q"]), Some((true, _))));
    let Some(rt) = rt else {
        let installed = ["docker", "podman"]
            .iter()
            .any(|rt| run(rt, &["--version"]).is_some());
        let why = if installed {
            "not running (start Docker Desktop / the Podman machine)"
        } else {
            "not installed"
        };
        return crate::model::Docker {
            unavailable: Some(why.into()),
            ..Default::default()
        };
    };
    let label = |k: &str| {
        if rt == "podman" {
            format!("{{{{index .Labels \"{k}\"}}}}")
        } else {
            format!("{{{{.Label \"{k}\"}}}}")
        }
    };
    let p = "com.docker.compose.project";
    let format = format!(
        "{{{{.ID}}}}|{{{{.Names}}}}|{{{{.Image}}}}|{{{{.State}}}}|{{{{.Status}}}}|{}|{}|{}|{{{{.Ports}}}}",
        label(p),
        label(&format!("{p}.working_dir")),
        label(&format!("{p}.config_files")),
    );
    let mut ps: Vec<String> = Vec::new();
    if let Some((_, v)) = run(rt, &["--version"]) {
        ps.push(format!("RUNTIME {}", v.lines().next().unwrap_or("").trim()));
    }
    if let Some((_, out)) = run(rt, &["ps", "-a", "--no-trunc", "--format", &format]) {
        ps.extend(out.lines().map(str::to_string));
    }
    let mut net: Vec<String> = Vec::new();
    if let Some((_, ids)) = run(rt, &["ps", "-aq", "--no-trunc"]) {
        let ids: Vec<&str> = ids.split_whitespace().collect();
        if !ids.is_empty() {
            let mut args = vec![
                "inspect",
                "-f",
                "{{.Id}}|{{range $k, $v := .NetworkSettings.Networks}}{{$k}}={{$v.IPAddress}} {{end}}|{{range .Mounts}}{{.Source}} {{end}}",
            ];
            args.extend(ids);
            if let Some((_, out)) = run(rt, &args) {
                net.extend(out.lines().map(str::to_string));
            }
        }
    }
    crate::parse::parse_docker(Some(&ps), &net)
}

/// First port of Windows' dynamic (client) range.
const DYNAMIC_PORTS: u16 = 49152;

/// Listening sockets in `netstat -ano` output (Windows). TCP sockets count
/// when their foreign address is a wildcard (state names are translated on
/// non-English Windows, so they are not used); every bound UDP socket counts.
/// `process(pid)` names the owner.
pub fn parse_netstat(out: &str, process: impl Fn(u32) -> String) -> Vec<Listener> {
    let mut v: Vec<Listener> = Vec::new();
    for l in out.lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        let (proto, local, foreign, pid) = match t.as_slice() {
            [p, local, foreign, _state, pid] if p.eq_ignore_ascii_case("tcp") => {
                ("tcp", *local, *foreign, *pid)
            }
            [p, local, foreign, pid] if p.eq_ignore_ascii_case("udp") => {
                ("udp", *local, *foreign, *pid)
            }
            _ => continue,
        };
        let listening = match proto {
            "tcp" => matches!(foreign, "0.0.0.0:0" | "[::]:0" | "*:*"),
            _ => foreign == "*:*",
        };
        if !listening {
            continue;
        }
        let Some((address, port)) = crate::parse::split_host_port(local) else {
            continue;
        };
        // UDP sockets of programs talking out sit in the dynamic range.
        if proto == "udp" && port >= DYNAMIC_PORTS {
            continue;
        }
        let owner = pid.parse::<u32>().map(&process).unwrap_or_default();
        if v.iter()
            .any(|x| x.proto == proto && x.address == address && x.port == port)
        {
            continue;
        }
        v.push(Listener {
            proto: proto.into(),
            address,
            port,
            process: owner,
        });
    }
    v.sort_by(|a, b| a.port.cmp(&b.port).then(a.proto.cmp(&b.proto)));
    v
}

/// The databases of this computer the database window can open: servers in
/// the listening ports, and containers that publish their port here. All are
/// reached on 127.0.0.1, never through SSH (owner decision #129); a server
/// bound to another address only (or to `::1`) is left out.
pub fn db_targets(report: &HostReport) -> Vec<crate::db::DbTarget> {
    use crate::db::detect::{host_targets, image_engine};
    let alias = crate::config::LOCAL_ALIAS;
    let mut out: Vec<crate::db::DbTarget> = host_targets(alias, report)
        .into_iter()
        .filter(|t| t.address == "127.0.0.1")
        .collect();
    for c in report.docker.containers.iter().filter(|c| c.running()) {
        let Some(engine) = image_engine(&c.image) else {
            continue;
        };
        let Some(p) = c
            .published
            .iter()
            .find(|p| p.container_port == engine.default_port())
        else {
            continue;
        };
        out.retain(|t| t.port != p.host_port);
        out.push(crate::db::DbTarget {
            engine,
            host: alias.to_string(),
            container: c.name.clone(),
            container_id: c.id.clone(),
            address: "127.0.0.1".into(),
            port: p.host_port,
        });
    }
    out
}

/// Open an interactive shell of this computer in a new terminal window, in
/// the home folder.
pub fn open_terminal() -> Result<(), String> {
    platform::open_local_shell().map_err(|e| format!("cannot open a terminal: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETSTAT: &str = "
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1108
  TCP    0.0.0.0:3306           0.0.0.0:0              ABHÖREN         4242
  TCP    192.0.2.10:52000       198.51.100.7:443       ESTABLISHED     900
  TCP    [::]:135               [::]:0                 LISTENING       1108
  TCP    [::1]:5432             [::]:0                 LISTENING       77
  UDP    0.0.0.0:123            *:*                                    1500
  UDP    0.0.0.0:123            *:*                                    1500
  UDP    [fe80::1%12]:1900      *:*                                    3100
  UDP    0.0.0.0:61234          *:*                                    3100
";

    #[test]
    fn reads_listening_sockets_from_netstat() {
        let v = parse_netstat(NETSTAT, |pid| match pid {
            4242 => "mysqld".into(),
            77 => "postgres".into(),
            _ => String::new(),
        });
        let got: Vec<(&str, &str, u16, &str)> = v
            .iter()
            .map(|l| {
                (
                    l.proto.as_str(),
                    l.address.as_str(),
                    l.port,
                    l.process.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("udp", "0.0.0.0", 123, ""),
                ("tcp", "0.0.0.0", 135, ""),
                ("tcp", "::", 135, ""),
                ("udp", "fe80::1", 1900, ""),
                ("tcp", "0.0.0.0", 3306, "mysqld"),
                ("tcp", "::1", 5432, "postgres"),
            ]
        );
    }

    #[test]
    fn local_databases_are_reached_on_loopback_only() {
        use crate::model::{Container, PortMap};
        let mut r = HostReport {
            listeners: parse_netstat(NETSTAT, |pid| match pid {
                4242 => "mysqld".into(),
                77 => "postgres".into(),
                _ => String::new(),
            }),
            ..Default::default()
        };
        r.listeners.push(Listener {
            proto: "tcp".into(),
            address: "192.0.2.10".into(),
            port: 3307,
            process: "mariadbd".into(),
        });
        r.docker.containers.push(Container {
            name: "shop-db".into(),
            image: "postgres:16".into(),
            state: "running".into(),
            published: vec![PortMap {
                host_ip: "0.0.0.0".into(),
                host_port: 15432,
                container_port: 5432,
                proto: "tcp".into(),
            }],
            ..Default::default()
        });
        let t = db_targets(&r);
        let got: Vec<(&str, u16, &str)> = t
            .iter()
            .map(|t| (t.address.as_str(), t.port, t.container.as_str()))
            .collect();
        assert_eq!(
            got,
            [("127.0.0.1", 3306, ""), ("127.0.0.1", 15432, "shop-db")]
        );
        assert!(t.iter().all(|t| t.host == crate::config::LOCAL_ALIAS));
    }
}
