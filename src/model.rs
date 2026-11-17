use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct HostReport {
    pub hostname: String,
    pub os_name: String,
    pub os_pretty: String,
    pub kernel: String,
    pub os_type: String,
    pub arch: String,
    pub uptime_secs: Option<u64>,
    pub load_avg: Option<[f32; 3]>,
    pub cpu_model: String,
    pub cpu_cores: Option<u32>,
    pub cpu_usage_pct: Option<f32>,
    pub memory: Option<Memory>,
    pub disks: Vec<Disk>,
    pub addresses: Vec<NetAddr>,
    pub sensors: Vec<Sensor>,
    /// A BMC (IPMI) device exists on this host.
    pub has_bmc: bool,
    /// Why BMC sensors could not be read (ipmitool missing, no sudo rule).
    pub bmc_error: Option<String>,
    pub docker: Docker,
    pub listeners: Vec<Listener>,
    /// Result of an RDP reachability check, not of the SSH script.
    pub rdp_only: bool,
    /// ICMP round-trip in ms (RDP hosts), `None` when there was no reply.
    pub ping_ms: Option<f32>,
    /// RDP port accepted a TCP connection (RDP hosts).
    pub rdp_open: bool,
}

/// A listening socket on the host (`ss -tuln` / `netstat -tuln`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Listener {
    pub proto: String,
    pub address: String,
    pub port: u16,
    /// Only known when the remote user may see the owning process.
    pub process: String,
}

/// A host port published by a container (`0.0.0.0:2491->443/tcp`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PortMap {
    pub host_ip: String,
    pub host_port: u16,
    pub container_port: u16,
    pub proto: String,
}

/// One host port in use, merged from listeners and Docker port mappings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PortUse {
    pub port: u16,
    pub proto: String,
    pub addresses: Vec<String>,
    /// Container name, process name, or empty when unknown.
    pub owner: String,
    pub stack: String,
    pub container_port: Option<u16>,
    pub nginx: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Memory {
    pub total_kb: u64,
    pub available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

impl Memory {
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.available_kb)
    }
    pub fn used_pct(&self) -> f32 {
        pct(self.used_kb(), self.total_kb)
    }
    pub fn swap_used_kb(&self) -> u64 {
        self.swap_total_kb.saturating_sub(self.swap_free_kb)
    }
    pub fn swap_used_pct(&self) -> f32 {
        pct(self.swap_used_kb(), self.swap_total_kb)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Disk {
    pub filesystem: String,
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
    pub available_kb: u64,
}

impl Disk {
    pub fn used_pct(&self) -> f32 {
        // df's Use% is used / (used + available): reserved blocks are excluded.
        pct(self.used_kb, self.used_kb + self.available_kb)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NetAddr {
    pub interface: String,
    pub family: String,
    pub address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SensorKind {
    Temperature,
    Fan,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sensor {
    pub kind: SensorKind,
    pub chip: String,
    pub label: String,
    /// °C for temperatures, RPM for fans.
    pub value: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Docker {
    /// `None` when Docker is usable; otherwise why it is not (not installed, permission).
    pub unavailable: Option<String>,
    /// First line of `docker --version` / `podman --version`.
    pub runtime: String,
    pub containers: Vec<Container>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    /// Docker Compose / Portainer stack (`com.docker.compose.project`), empty if standalone.
    pub stack: String,
    /// `com.docker.compose.project.working_dir`.
    pub working_dir: String,
    /// `com.docker.compose.project.config_files`.
    pub config_files: String,
    /// Mounts the Docker/Podman socket (Portainer server, agents, CI runners...).
    pub mounts_socket: bool,
    pub published: Vec<PortMap>,
    /// `network=ip` pairs.
    pub networks: Vec<(String, String)>,
}

impl Container {
    pub fn running(&self) -> bool {
        self.state == "running"
    }
    pub fn is_nginx(&self) -> bool {
        self.image.contains("nginx") || self.name.contains("nginx")
    }

    pub fn portainer_role(&self) -> Option<PortainerRole> {
        let img = self.image.to_ascii_lowercase();
        if img.contains("portainer/agent") {
            Some(PortainerRole::Agent)
        } else if img.contains("portainer/portainer") {
            Some(PortainerRole::Server)
        } else {
            None
        }
    }

    /// Who deployed this container, from its compose labels.
    pub fn manager(&self) -> Manager {
        if self.stack.is_empty() {
            return Manager::Standalone;
        }
        // Portainer writes every stack it deploys to /data/compose/<stack id>/.
        if let Some(rest) = self.working_dir.strip_prefix("/data/compose/") {
            let id = rest.split('/').next().unwrap_or(rest).to_string();
            return Manager::Portainer { stack_id: id };
        }
        Manager::Compose {
            dir: self.working_dir.clone(),
            files: self.config_files.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PortainerRole {
    /// The Portainer UI/server; it manages this host via the local socket.
    Server,
    /// `portainer/agent`; a Portainer server elsewhere manages this host through it (:9001).
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Manager {
    /// Deployed as a Portainer stack (compose files under /data/compose/<id>).
    Portainer { stack_id: String },
    /// `docker compose up` run by hand in `dir`.
    Compose { dir: String, files: String },
    /// `docker run` / no compose labels.
    Standalone,
}

pub fn pct(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64 * 100.0) as f32
    }
}

pub fn human_kb(kb: u64) -> String {
    let units = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = kb as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", units[i])
    } else {
        format!("{v:.1} {}", units[i])
    }
}

pub fn human_uptime(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

impl Docker {
    pub fn is_podman(&self) -> bool {
        self.runtime.to_ascii_lowercase().contains("podman")
    }

    /// Portainer containers on this host and the published host port of each.
    pub fn portainer(&self) -> Vec<(PortainerRole, &Container)> {
        self.containers
            .iter()
            .filter_map(|c| c.portainer_role().map(|r| (r, c)))
            .collect()
    }

    /// How a stack is managed, plus the host-level Portainer connection when relevant.
    pub fn stack_manager(&self, containers: &[&Container]) -> Manager {
        containers
            .iter()
            .map(|c| c.manager())
            .find(|m| *m != Manager::Standalone)
            .unwrap_or(Manager::Standalone)
    }

    /// Containers grouped by stack, named stacks first (sorted), standalone last.
    pub fn stacks(&self) -> Vec<(&str, Vec<&Container>)> {
        let mut groups: Vec<(&str, Vec<&Container>)> = Vec::new();
        for c in &self.containers {
            match groups.iter_mut().find(|(s, _)| *s == c.stack) {
                Some((_, v)) => v.push(c),
                None => groups.push((&c.stack, vec![c])),
            }
        }
        groups.sort_by(|a, b| a.0.is_empty().cmp(&b.0.is_empty()).then(a.0.cmp(b.0)));
        groups
    }
}

impl HostReport {
    /// Every host port in use, sorted by port, with the container or process owning it.
    pub fn port_usage(&self) -> Vec<PortUse> {
        let mut out: Vec<PortUse> = Vec::new();
        let mut add = |port: u16, proto: &str, addr: &str| -> usize {
            let idx = match out.iter().position(|u| u.port == port && u.proto == proto) {
                Some(i) => i,
                None => {
                    out.push(PortUse {
                        port,
                        proto: proto.to_string(),
                        ..Default::default()
                    });
                    out.len() - 1
                }
            };
            if !addr.is_empty() && !out[idx].addresses.iter().any(|a| a == addr) {
                out[idx].addresses.push(addr.to_string());
            }
            idx
        };
        let mut owners: Vec<(usize, String)> = Vec::new();
        for l in &self.listeners {
            let i = add(l.port, &l.proto, &l.address);
            if !l.process.is_empty() {
                owners.push((i, l.process.clone()));
            }
        }
        let mut published: Vec<(usize, &Container, &PortMap)> = Vec::new();
        for c in &self.docker.containers {
            for p in &c.published {
                published.push((add(p.host_port, &p.proto, &p.host_ip), c, p));
            }
        }
        for (i, name) in owners {
            out[i].nginx |= name == "nginx";
            out[i].owner = name;
        }
        // A container mapping beats `docker-proxy` as the owner.
        for (i, c, p) in published {
            let u = &mut out[i];
            u.owner = c.name.clone();
            u.stack = c.stack.clone();
            u.container_port = Some(p.container_port);
            u.nginx = c.is_nginx();
        }
        out.sort_by(|a, b| a.port.cmp(&b.port).then(a.proto.cmp(&b.proto)));
        out
    }
}
