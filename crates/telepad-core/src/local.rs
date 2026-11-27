//! My Computer (#129): the same report as a host's, read from the computer
//! Telepad runs on, without SSH. Read-only like the remote script: no admin,
//! no prompts, nothing written.
//!
//! - Windows has no `sh`: system facts come from the OS (`sysinfo`), listening
//!   ports from `netstat -ano`, pending reboot from `reg query`, Docker /
//!   Podman from their CLI with the same formats the remote script uses;
//!   security, Windows Update and installed programs in [`security`] (#211).
//! - Linux / macOS run the remote script itself with `sh -s`.

use std::process::{Command, Stdio};
use std::time::Duration;

use crate::model::{HostReport, Listener};
use crate::platform;

pub mod security;
pub mod sensors;

/// Longest a local read may take (Docker can be slow to answer).
pub const TIMEOUT: Duration = Duration::from_secs(40);

/// Collect everything about this computer. Blocks (run it on a thread).
/// `slow`: also the slow facts (#211: Windows Update and installed programs
/// on Windows); without it they are left empty for [`keep_slow`].
pub fn collect(slow: bool) -> Result<HostReport, String> {
    let mut r = collect_impl(slow)?;
    if r.hostname.is_empty() {
        r.hostname = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_default();
    }
    Ok(r)
}

/// Facts read only on a slow read (#211), kept from the last report.
pub fn keep_slow(new: &mut HostReport, prev: &HostReport) {
    if new.apps.is_none() {
        new.apps.clone_from(&prev.apps);
    }
    if new.os_id == "windows" {
        new.maintenance.updates = prev.maintenance.updates;
        new.maintenance
            .last_update
            .clone_from(&prev.maintenance.last_update);
    }
}

#[cfg(not(target_os = "windows"))]
fn collect_impl(_slow: bool) -> Result<HostReport, String> {
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
fn collect_impl(slow: bool) -> Result<HostReport, String> {
    use crate::model::{Disk, Memory, NetAddr};
    use sysinfo::{Disks, Networks, ProcessesToUpdate, System};

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
    (r.sensors, r.sensors_note) = sensors::read(run);
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
    r.security = Some(security::read_security(run));
    if slow {
        (r.maintenance.updates, r.maintenance.last_update) = security::read_update(run);
        r.apps = Some(security::read_apps(run));
    }
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
    if rt == "docker" {
        // Docker Desktop answers `inspect` of a running container in ~0.5 s
        // (#198), so read names / mounts from `ps` and IPs per network.
        let ps_nets = run(
            rt,
            &[
                "ps",
                "-a",
                "--no-trunc",
                "--format",
                "{{.ID}}|{{.Networks}}|{{.Mounts}}",
            ],
        );
        let ids = run(rt, &["network", "ls", "-q"])
            .map(|(_, o)| o)
            .unwrap_or_default();
        let ids: Vec<&str> = ids.split_whitespace().collect();
        let mut by_net = String::new();
        if !ids.is_empty() {
            let mut args = vec![
                "network",
                "inspect",
                "-f",
                "{{.Name}}|{{range $id, $c := .Containers}}{{$id}}={{$c.IPv4Address}} {{end}}",
            ];
            args.extend(ids);
            by_net = run(rt, &args).map(|(_, o)| o).unwrap_or_default();
        }
        if let Some((_, out)) = ps_nets {
            net = docker_net_lines(&out, &by_net);
        }
        return crate::parse::parse_docker(Some(&ps), &net);
    }
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

/// The `===DOCKERNET===` lines (`id|net=ip net=ip |mount mount `) that
/// `docker inspect` gives, built from two calls that do not get slower with
/// every running container (#198):
/// - `ps`: `docker ps -a --format '{{.ID}}|{{.Networks}}|{{.Mounts}}'` —
///   network names and mounts (comma separated) of every container;
/// - `by_net`: `docker network inspect -f '{{.Name}}|{{range $id, $c :=
///   .Containers}}{{$id}}={{$c.IPv4Address}} {{end}}'` — the IP (with its
///   prefix length) of each running container on each network.
///
/// Stopped containers keep their network names with an empty IP, as
/// `inspect` shows them; networks are sorted by name like `inspect`'s map.
pub fn docker_net_lines(ps: &str, by_net: &str) -> Vec<String> {
    let mut ips: std::collections::HashMap<(&str, &str), &str> = Default::default();
    for l in by_net.lines() {
        let Some((net, rest)) = l.split_once('|') else {
            continue;
        };
        for pair in rest.split_whitespace() {
            if let Some((id, ip)) = pair.split_once('=') {
                let ip = ip.split_once('/').map_or(ip, |(a, _)| a);
                ips.insert((net.trim(), id), ip);
            }
        }
    }
    ps.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '|');
            let id = it.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let mut nets: Vec<&str> = it
                .next()
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .collect();
            nets.sort_unstable();
            let nets: String = nets
                .iter()
                .map(|n| format!("{n}={} ", ips.get(&(*n, id)).copied().unwrap_or("")))
                .collect();
            let mounts: String = it
                .next()
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(|m| format!("{m} "))
                .collect();
            Some(format!("{id}|{nets}|{mounts}"))
        })
        .collect()
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
    fn docker_net_lines_match_what_inspect_gives() {
        // Made-up containers: web on two networks, a stopped one, one that
        // mounts the Docker socket, one with no network data at all.
        let ps = "aaa111|web_default,proxy|/srv/web/conf,web_data
bbb222|web_default|
ccc333|tools_default|/var/run/docker.sock
ddd444||
";
        let by_net = "web_default|aaa111=172.18.0.2/16
proxy|aaa111=172.19.0.5/16 ccc999=172.19.0.9/16
tools_default|ccc333=172.20.0.3/16
bridge|
";
        let lines = docker_net_lines(ps, by_net);
        assert_eq!(
            lines,
            [
                "aaa111|proxy=172.19.0.5 web_default=172.18.0.2 |/srv/web/conf web_data ",
                "bbb222|web_default= |",
                "ccc333|tools_default=172.20.0.3 |/var/run/docker.sock ",
                "ddd444||",
            ]
        );
        // The parser reads them exactly like `inspect` output.
        let ps_rows: Vec<String> = [
            "aaa111|web|nginx|running|Up|web|/srv/web|compose.yaml|",
            "bbb222|old|nginx|exited|Exited (0)|web|/srv/web|compose.yaml|",
            "ccc333|agent|portainer/agent|running|Up||||",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let d = crate::parse::parse_docker(Some(&ps_rows), &lines);
        let by = |n: &str| d.containers.iter().find(|c| c.name == n).unwrap();
        assert_eq!(
            by("web").networks,
            [
                ("proxy".to_string(), "172.19.0.5".to_string()),
                ("web_default".to_string(), "172.18.0.2".to_string())
            ]
        );
        assert_eq!(
            by("old").networks,
            [("web_default".to_string(), String::new())]
        );
        assert!(by("agent").mounts_socket && !by("web").mounts_socket);
    }

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
