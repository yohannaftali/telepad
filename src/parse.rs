//! Pure parsing of the remote script's output. No I/O here.

use std::collections::HashMap;

use crate::model::*;

/// Split the script output into `===NAME===` sections.
pub fn sections(output: &str) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(name) = line
            .strip_prefix("===")
            .and_then(|l| l.strip_suffix("==="))
            .filter(|n| {
                !n.is_empty()
                    && n.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
        {
            map.entry(name.to_string()).or_default();
            current = Some(name.to_string());
            continue;
        }
        if let Some(name) = &current
            && !line.trim().is_empty()
        {
            map.entry(name.clone()).or_default().push(line.to_string());
        }
    }
    map
}

pub fn parse_report(output: &str) -> HostReport {
    let s = sections(output);
    let get = |k: &str| s.get(k).map(Vec::as_slice).unwrap_or(&[]);

    let mut r = HostReport {
        hostname: get("HOSTNAME").first().cloned().unwrap_or_default(),
        ..Default::default()
    };

    let (name, pretty) = parse_os_release(get("OSRELEASE"));
    r.os_name = name;
    r.os_pretty = pretty;
    let uname = get("UNAME");
    r.os_type = uname.first().cloned().unwrap_or_default();
    r.kernel = uname.get(1).cloned().unwrap_or_default();
    r.arch = uname.get(2).cloned().unwrap_or_default();
    if r.os_pretty.is_empty() {
        r.os_pretty = format!("{} {}", r.os_type, r.kernel).trim().to_string();
    }

    r.uptime_secs = get("UPTIME")
        .first()
        .and_then(|l| l.split_whitespace().next())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v as u64);
    r.load_avg = get("LOADAVG").first().and_then(|l| {
        let v: Vec<f32> = l
            .split_whitespace()
            .take(3)
            .filter_map(|x| x.parse().ok())
            .collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    });

    for line in get("CPUINFO") {
        if let Some(c) = line.strip_prefix("cores:") {
            r.cpu_cores = c.trim().parse().ok().filter(|&n| n > 0);
        } else if let Some((_, v)) = line.split_once(':') {
            r.cpu_model = v.trim().to_string();
        }
    }
    r.cpu_usage_pct = cpu_usage(get("STAT1").first(), get("STAT2").first());
    r.memory = parse_meminfo(get("MEMINFO"));
    r.disks = parse_df(get("DF"));
    r.addresses = parse_ip(get("IPADDR"));
    r.sensors = parse_sensors(get("THERMAL"), get("HWMON"));
    let (has_bmc, bmc_error, bmc) = parse_ipmi(get("IPMI"));
    r.has_bmc = has_bmc;
    r.bmc_error = bmc_error;
    r.sensors.extend(bmc);
    r.listeners = parse_listen(get("LISTEN"));
    r.docker = parse_docker(s.get("DOCKER").map(Vec::as_slice), get("DOCKERNET"));
    r
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches('"').trim_matches('\'').to_string()
}

pub fn parse_os_release(lines: &[String]) -> (String, String) {
    let mut kv = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once('=') {
            kv.insert(k.trim().to_string(), unquote(v));
        }
    }
    let name = kv.get("NAME").cloned().unwrap_or_default();
    let pretty = kv.get("PRETTY_NAME").cloned().unwrap_or_else(|| {
        let ver = kv
            .get("VERSION")
            .or_else(|| kv.get("productversion"))
            .cloned()
            .unwrap_or_default();
        format!("{name} {ver}").trim().to_string()
    });
    (name, pretty)
}

fn stat_fields(line: &str) -> Option<(u64, u64)> {
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 4 {
        return None;
    }
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    Some((idle, v.iter().sum()))
}

pub fn cpu_usage(a: Option<&String>, b: Option<&String>) -> Option<f32> {
    let (i1, t1) = stat_fields(a?)?;
    let (i2, t2) = stat_fields(b?)?;
    let dt = t2.checked_sub(t1)?;
    if dt == 0 {
        return None;
    }
    let di = i2.saturating_sub(i1).min(dt);
    Some(((dt - di) as f64 / dt as f64 * 100.0) as f32)
}

pub fn parse_meminfo(lines: &[String]) -> Option<Memory> {
    let mut kv = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':')
            && let Some(n) = v
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok())
        {
            kv.insert(k.trim(), n);
        }
    }
    let total = *kv.get("MemTotal")?;
    let available = kv.get("MemAvailable").copied().unwrap_or_else(|| {
        // Kernels before 3.14 have no MemAvailable.
        ["MemFree", "Buffers", "Cached", "SReclaimable"]
            .iter()
            .filter_map(|k| kv.get(k))
            .sum()
    });
    Some(Memory {
        total_kb: total,
        available_kb: available.min(total),
        swap_total_kb: kv.get("SwapTotal").copied().unwrap_or(0),
        swap_free_kb: kv.get("SwapFree").copied().unwrap_or(0),
    })
}

const PSEUDO_FS: &[&str] = &[
    "tmpfs", "devtmpfs", "udev", "overlay", "shm", "none", "efivarfs", "squashfs", "cgroup",
    "proc", "sysfs", "devpts",
];

pub fn parse_df(lines: &[String]) -> Vec<Disk> {
    let mut disks = Vec::new();
    for l in lines {
        let parts: Vec<&str> = l.split_whitespace().collect();
        if parts.len() < 6 || parts[0] == "Filesystem" {
            continue;
        }
        let nums: Option<Vec<u64>> = parts[1..4].iter().map(|x| x.parse().ok()).collect();
        let Some(nums) = nums else { continue };
        let fs = parts[0];
        let mount = parts[5..].join(" ");
        if nums[0] == 0
            || PSEUDO_FS.contains(&fs)
            || mount.starts_with("/snap/")
            || mount.starts_with("/run")
            || mount.starts_with("/dev")
            || mount.starts_with("/sys")
            || mount.starts_with("/var/lib/docker/")
        {
            continue;
        }
        disks.push(Disk {
            filesystem: fs.to_string(),
            mount,
            total_kb: nums[0],
            used_kb: nums[1],
            available_kb: nums[2],
        });
    }
    disks
}

pub fn parse_ip(lines: &[String]) -> Vec<NetAddr> {
    let mut out = Vec::new();
    for l in lines {
        let p: Vec<&str> = l.split_whitespace().collect();
        // `ip -o addr`: "2: eth0    inet 10.0.0.5/24 ..."; ifconfig fallback: "eth0 inet 10.0.0.5"
        let (iface, fam, addr) = match p.as_slice() {
            [idx, iface, fam, addr, ..] if idx.ends_with(':') => (*iface, *fam, *addr),
            [iface, fam, addr, ..] => (*iface, *fam, *addr),
            _ => continue,
        };
        let iface = iface.trim_end_matches(':');
        let iface = iface.split('@').next().unwrap_or(iface);
        if fam != "inet" && fam != "inet6" {
            continue;
        }
        let ip = addr.split('/').next().unwrap_or(addr);
        // Docker bridges are shown per container, not as host addresses.
        if iface == "lo"
            || iface == "docker0"
            || iface.starts_with("br-")
            || iface.starts_with("veth")
            || ip.starts_with("127.")
            || ip == "::1"
            || ip.starts_with("fe80")
        {
            continue;
        }
        out.push(NetAddr {
            interface: iface.to_string(),
            family: if fam == "inet" { "IPv4" } else { "IPv6" }.to_string(),
            address: addr.to_string(),
        });
    }
    out
}

fn plausible_temp(c: f32) -> bool {
    c > -40.0 && c < 150.0 && c != 0.0
}

pub fn parse_sensors(thermal: &[String], hwmon: &[String]) -> Vec<Sensor> {
    let mut out = Vec::new();
    for l in hwmon {
        let p: Vec<&str> = l.split('|').collect();
        let [chip, item, label, value] = p.as_slice() else {
            continue;
        };
        let Ok(raw) = value.trim().parse::<f32>() else {
            continue;
        };
        let label = if label.is_empty() { item } else { label };
        if item.starts_with("temp") {
            let c = raw / 1000.0;
            if plausible_temp(c) {
                out.push(Sensor {
                    kind: SensorKind::Temperature,
                    chip: chip.to_string(),
                    label: label.to_string(),
                    value: c,
                });
            }
        } else if item.starts_with("fan") {
            out.push(Sensor {
                kind: SensorKind::Fan,
                chip: chip.to_string(),
                label: label.to_string(),
                value: raw,
            });
        }
    }
    let have_hwmon_temps = out.iter().any(|s| s.kind == SensorKind::Temperature);
    for l in thermal {
        let Some((kind, value)) = l.split_once('|') else {
            continue;
        };
        let Ok(raw) = value.trim().parse::<f32>() else {
            continue;
        };
        let c = raw / 1000.0;
        // hwmon usually mirrors the thermal zones; only fall back to zones.
        if plausible_temp(c) && !have_hwmon_temps {
            out.push(Sensor {
                kind: SensorKind::Temperature,
                chip: "thermal".into(),
                label: kind.to_string(),
                value: c,
            });
        }
    }
    out
}

pub fn parse_docker(ps: Option<&[String]>, net: &[String]) -> Docker {
    let Some(ps) = ps else {
        return Docker {
            unavailable: Some("no data".into()),
            containers: vec![],
            ..Default::default()
        };
    };
    let mut d = Docker::default();
    let mut nets: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    let mut socket: HashMap<&str, bool> = HashMap::new();
    for l in net {
        let mut it = l.splitn(3, '|');
        let (Some(id), Some(rest)) = (it.next(), it.next()) else {
            continue;
        };
        let mounts = it.next().unwrap_or("");
        let pairs = rest
            .split_whitespace()
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        nets.insert(id.trim(), pairs);
        socket.insert(
            id.trim(),
            mounts
                .split_whitespace()
                .any(|m| m.ends_with("docker.sock") || m.ends_with("podman.sock")),
        );
    }
    for l in ps {
        if let Some(err) = l.strip_prefix("ERR ") {
            d.unavailable = Some(err.to_string());
            continue;
        }
        if let Some(rt) = l.strip_prefix("RUNTIME ") {
            d.runtime = rt.trim().to_string();
            continue;
        }
        let p: Vec<&str> = l.splitn(9, '|').collect();
        let [id, name, image, state, status, stack, wdir, files, ports] = p.as_slice() else {
            continue;
        };
        d.containers.push(Container {
            id: id.chars().take(12).collect(),
            name: name.to_string(),
            image: image.to_string(),
            state: state.to_string(),
            status: status.to_string(),
            ports: ports.to_string(),
            stack: stack.trim().to_string(),
            working_dir: wdir.trim().to_string(),
            config_files: files.trim().to_string(),
            mounts_socket: socket.get(id).copied().unwrap_or(false),
            published: parse_published(ports),
            networks: nets.get(id).cloned().unwrap_or_default(),
        });
    }
    d.containers
        .sort_by(|a, b| b.running().cmp(&a.running()).then(a.name.cmp(&b.name)));
    d
}

/// `ipmitool sdr type Fan|Temperature` output, each block preceded by `#Fan` / `#Temperature`:
/// `Fan1A RPM        | 30h | ok  |  7.1 | 3600 RPM`
/// `Inlet Temp       | 04h | ok  |  7.1 | 23 degrees C`
/// Returns (BMC present, error, sensors). Rows with no numeric reading are skipped.
pub fn parse_ipmi(lines: &[String]) -> (bool, Option<String>, Vec<Sensor>) {
    let mut err = None;
    let mut out = Vec::new();
    for l in lines {
        if let Some(e) = l.strip_prefix("ERR ") {
            err = Some(e.trim().to_string());
            continue;
        }
        let p: Vec<&str> = l.split('|').map(str::trim).collect();
        let [name, _id, status, _entity, reading, ..] = p.as_slice() else {
            continue;
        };
        if status.eq_ignore_ascii_case("ns") {
            continue;
        }
        let mut it = reading.split_whitespace();
        let Some(value) = it.next().and_then(|v| v.parse::<f32>().ok()) else {
            continue;
        };
        let unit = it.collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        let (kind, label) = if unit == "rpm" {
            (SensorKind::Fan, name.trim_end_matches(" RPM").trim())
        } else if unit == "degrees c" {
            (SensorKind::Temperature, *name)
        } else {
            continue;
        };
        out.push(Sensor {
            kind,
            chip: "BMC".into(),
            label: label.to_string(),
            value,
        });
    }
    (!lines.is_empty(), err, out)
}

/// `[::]`, `*`, `0.0.0.0` -> as-is but unbracketed; strips `%iface` scope.
fn split_host_port(s: &str) -> Option<(String, u16)> {
    let (addr, port) = s.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let addr = addr.trim_start_matches('[').trim_end_matches(']');
    let addr = addr.split('%').next().unwrap_or(addr);
    Some((addr.to_string(), port))
}

/// `ss -tuln[p]` or `netstat -tuln[p]` output.
pub fn parse_listen(lines: &[String]) -> Vec<Listener> {
    let mut out = Vec::new();
    for l in lines {
        let t: Vec<&str> = l.split_whitespace().collect();
        let Some(first) = t.first() else { continue };
        let proto = match first.to_ascii_lowercase().as_str() {
            p if p.starts_with("tcp") => "tcp",
            p if p.starts_with("udp") => "udp",
            _ => continue,
        };
        // ss: Netid State Recv-Q Send-Q Local Peer [Process]
        // netstat: Proto Recv-Q Send-Q Local Foreign [State] [PID/Program]
        let is_ss = t.get(1).is_some_and(|x| x.parse::<u64>().is_err());
        let local = if is_ss { t.get(4) } else { t.get(3) };
        let Some((address, port)) = local.and_then(|x| split_host_port(x)) else {
            continue;
        };
        let process = if is_ss {
            l.split_once("((\"")
                .and_then(|(_, r)| r.split_once('"'))
                .map(|(n, _)| n.to_string())
                .unwrap_or_default()
        } else {
            t.iter()
                .skip(4)
                .filter_map(|x| x.split_once('/'))
                .find(|(pid, _)| pid.parse::<u32>().is_ok())
                .map(|(_, n)| n.trim_end_matches(':').to_string())
                .unwrap_or_default()
        };
        out.push(Listener {
            proto: proto.into(),
            address,
            port,
            process,
        });
    }
    out
}

/// Docker's Ports column: `0.0.0.0:2491->443/tcp, [::]:2491->443/tcp, 9000/tcp`.
/// Unpublished (container-only) ports are skipped.
pub fn parse_published(ports: &str) -> Vec<PortMap> {
    let mut out = Vec::new();
    for item in ports.split(',').map(str::trim) {
        let Some((host, cont)) = item.split_once("->") else {
            continue;
        };
        let (cont, proto) = cont.split_once('/').unwrap_or((cont, "tcp"));
        let Some((ip, hport)) = host.rsplit_once(':') else {
            continue;
        };
        let ip = ip.trim_start_matches('[').trim_end_matches(']');
        let range = |r: &str| -> Option<(u16, u16)> {
            match r.split_once('-') {
                Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
                None => r.parse().ok().map(|p| (p, p)),
            }
        };
        let (Some((h0, h1)), Some((c0, _))) = (range(hport), range(cont)) else {
            continue;
        };
        for (k, hp) in (h0..=h1.min(h0.saturating_add(255))).enumerate() {
            out.push(PortMap {
                host_ip: ip.to_string(),
                host_port: hp,
                container_port: c0.saturating_add(k as u16),
                proto: proto.to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/ubuntu_docker.txt");
    const BUSYBOX: &str = include_str!("../tests/fixtures/busybox_nodocker.txt");

    #[test]
    fn parses_full_linux_host() {
        let r = parse_report(SAMPLE);
        assert_eq!(r.hostname, "docker01");
        assert_eq!(r.os_pretty, "Ubuntu 24.04.1 LTS");
        assert_eq!(r.os_type, "Linux");
        assert_eq!(r.arch, "x86_64");
        assert_eq!(r.cpu_cores, Some(4));
        let cpu = r.cpu_usage_pct.unwrap();
        assert!((cpu - 25.0).abs() < 0.01, "{cpu}");
        let m = r.memory.unwrap();
        assert_eq!(m.total_kb, 8_000_000);
        assert_eq!(m.used_kb(), 6_000_000);
        assert_eq!(r.disks.len(), 2, "{:?}", r.disks);
        assert_eq!(r.disks[0].mount, "/");
        assert_eq!(r.addresses.iter().filter(|a| a.family == "IPv4").count(), 1);
        assert!(
            r.addresses
                .iter()
                .all(|a| a.interface != "lo" && !a.interface.starts_with("veth"))
        );
        assert!(r.docker.unavailable.is_none());
        assert_eq!(r.docker.containers.len(), 3);
        let web = r
            .docker
            .containers
            .iter()
            .find(|c| c.name == "web")
            .unwrap();
        assert_eq!(web.name, "web");
        assert_eq!(
            web.networks,
            vec![("bridge".to_string(), "172.17.0.2".to_string())]
        );
        assert!(web.ports.contains("8080->80/tcp"));
        assert!(!r.docker.containers.last().unwrap().running());
        assert_eq!(
            r.sensors
                .iter()
                .filter(|s| s.kind == SensorKind::Temperature)
                .count(),
            2
        );
        assert_eq!(
            r.sensors
                .iter()
                .filter(|s| s.kind == SensorKind::Fan)
                .count(),
            1
        );
    }

    #[test]
    fn tolerates_busybox_without_docker() {
        let r = parse_report(BUSYBOX);
        assert_eq!(r.hostname, "nas");
        assert_eq!(r.os_pretty, "Synology DSM 7.2");
        assert!(r.memory.is_some());
        assert_eq!(r.docker.unavailable.as_deref(), Some("not installed"));
        assert!(r.docker.containers.is_empty());
        assert_eq!(r.sensors.len(), 1);
        assert_eq!(r.sensors[0].label, "x86_pkg_temp");
        assert_eq!(r.addresses[0].address, "198.51.100.72");
    }

    #[test]
    fn parses_listeners_ss_old_ss_and_netstat() {
        let l: Vec<String> = [
            "Netid State  Recv-Q Send-Q Local Address:Port  Peer Address:PortProcess",
            "udp   UNCONN 0      0      127.0.0.53%lo:53         0.0.0.0:*",
            "tcp   LISTEN 0      4096         0.0.0.0:2491      0.0.0.0:*",
            "tcp   LISTEN 0      511             [::]:80           [::]:*    users:((\"nginx\",pid=12,fd=6))",
            "tcp    LISTEN     0      128       *:22                    *:*",
            "tcp        0      0 0.0.0.0:443             0.0.0.0:*               LISTEN      812/nginx: master",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let v = parse_listen(&l);
        assert_eq!(v.len(), 5);
        assert_eq!(
            (v[0].proto.as_str(), v[0].address.as_str(), v[0].port),
            ("udp", "127.0.0.53", 53)
        );
        assert_eq!(
            (v[2].address.as_str(), v[2].port, v[2].process.as_str()),
            ("::", 80, "nginx")
        );
        assert_eq!((v[3].address.as_str(), v[3].port), ("*", 22));
        assert_eq!((v[4].port, v[4].process.as_str()), (443, "nginx"));
    }

    #[test]
    fn parses_published_ports_and_ranges() {
        let p = parse_published(
            "80/tcp, 0.0.0.0:2490->443/tcp, [::]:2490->443/tcp, 127.0.0.1:7000-7001->8000-8001/udp",
        );
        assert_eq!(p.len(), 4);
        assert_eq!(
            p[0],
            PortMap {
                host_ip: "0.0.0.0".into(),
                host_port: 2490,
                container_port: 443,
                proto: "tcp".into()
            }
        );
        assert_eq!(p[1].host_ip, "::");
        assert_eq!(
            (p[3].host_port, p[3].container_port, p[3].proto.as_str()),
            (7001, 8001, "udp")
        );
    }

    #[test]
    fn port_usage_merges_and_attributes_to_containers() {
        let r = parse_report(SAMPLE);
        let u = r.port_usage();
        let p8080 = u.iter().find(|x| x.port == 8080).unwrap();
        assert_eq!(p8080.owner, "web");
        assert_eq!(p8080.stack, "shop");
        assert!(p8080.nginx);
        assert_eq!(p8080.addresses, vec!["0.0.0.0", "::"]);
        let ssh = u.iter().find(|x| x.port == 22).unwrap();
        assert_eq!(ssh.owner, "");
        assert!(!ssh.nginx);
        assert!(u.windows(2).all(|w| w[0].port <= w[1].port));
        let stacks = r.docker.stacks();
        assert_eq!(stacks[0].0, "portainer");
        assert_eq!(stacks[1].0, "shop");
        assert_eq!(stacks.last().unwrap().0, "");
    }

    #[test]
    fn detects_runtime_and_who_manages_each_stack() {
        let r = parse_report(SAMPLE);
        let d = &r.docker;
        assert_eq!(d.runtime, "Docker version 29.8.0, build 88096ef");
        assert!(!d.is_podman());
        let by = |n: &str| d.containers.iter().find(|c| c.name == n).unwrap();
        assert_eq!(
            by("web").manager(),
            Manager::Portainer {
                stack_id: "47".into()
            }
        );
        assert_eq!(
            by("portainer_agent").manager(),
            Manager::Compose {
                dir: "/home/alice/docker/portainer-agent".into(),
                files: "/home/alice/docker/portainer-agent/compose.yaml".into()
            }
        );
        assert_eq!(by("old-job").manager(), Manager::Standalone);
        assert!(by("portainer_agent").mounts_socket);
        assert!(!by("web").mounts_socket);
        let p = d.portainer();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, PortainerRole::Agent);
    }

    #[test]
    fn parses_ipmitool_fans_and_temps() {
        let l: Vec<String> = [
            "#Fan",
            "Fan1A RPM        | 30h | ok  |  7.1 | 3600 RPM",
            "Fan2A RPM        | 31h | ok  |  7.1 | 3480 RPM",
            "Fan3A RPM        | 32h | ns  |  7.1 | No Reading",
            "Fan Redundancy   | 75h | ok  |  7.1 | Fully Redundant",
            "Fan6             | 35h | ok  |  7.1 | Disabled",
            "#Temperature",
            "Inlet Temp       | 04h | ok  |  7.1 | 23 degrees C",
            "Exhaust Temp     | 01h | ok  |  7.1 | 34 degrees C",
            "Temp             | 0Eh | ok  |  3.1 | 52 degrees C",
            "Temp             | 0Fh | ns  |  3.2 | Disabled",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (bmc, err, s) = parse_ipmi(&l);
        assert!(bmc);
        assert!(err.is_none());
        let fans: Vec<_> = s.iter().filter(|x| x.kind == SensorKind::Fan).collect();
        assert_eq!(fans.len(), 2);
        assert_eq!((fans[0].label.as_str(), fans[0].value), ("Fan1A", 3600.0));
        let temps: Vec<_> = s
            .iter()
            .filter(|x| x.kind == SensorKind::Temperature)
            .collect();
        assert_eq!(temps.len(), 3);
        assert_eq!(
            (temps[0].label.as_str(), temps[0].value),
            ("Inlet Temp", 23.0)
        );
        assert!(s.iter().all(|x| x.chip == "BMC"));
    }

    #[test]
    fn ipmi_states() {
        assert_eq!(parse_ipmi(&[]), (false, None, vec![]));
        let (bmc, err, s) = parse_ipmi(&["ERR ipmitool not installed".to_string()]);
        assert!(bmc && s.is_empty());
        assert_eq!(err.as_deref(), Some("ipmitool not installed"));
    }

    #[test]
    fn empty_output_does_not_panic() {
        let r = parse_report("");
        assert!(r.memory.is_none());
        assert!(r.disks.is_empty());
        assert!(r.cpu_usage_pct.is_none());
    }
}
