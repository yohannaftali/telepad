use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize)]
pub struct HostReport {
    pub hostname: String,
    pub os_name: String,
    pub os_pretty: String,
    /// os-release `ID` (e.g. `ubuntu`) and `VERSION_ID` (e.g. `22.04`).
    pub os_id: String,
    pub os_version_id: String,
    /// Reboot / pending updates / end of support.
    pub maintenance: Maintenance,
    /// Docker Swarm / Kubernetes seen from this host.
    pub cluster: crate::cluster::Cluster,
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
    /// Why sensors are missing, or what would show more (#193): e.g. "not
    /// available on macOS", or start LibreHardwareMonitor on Windows.
    pub sensors_note: Option<String>,
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
    /// Windows Security and antivirus of My Computer (#211); `None` elsewhere.
    pub security: Option<Security>,
    /// Installed programs of My Computer on Windows (#211); `None` = not read.
    pub apps: Option<Vec<App>>,
}

/// Windows Security of this computer (#211), as Windows Security shows it.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Security {
    /// Antivirus products Windows knows (Microsoft Defender and third-party).
    pub antivirus: Vec<Antivirus>,
    /// Windows Firewall profiles (Domain / Private / Public).
    pub firewall_profiles: Vec<FirewallProfile>,
    /// Third-party firewalls registered with Windows Security that are on.
    pub firewall_products: Vec<String>,
    /// SmartScreen for apps and files ("App & browser control").
    pub app_control: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Antivirus {
    pub name: String,
    /// Real-time protection on.
    pub on: bool,
    /// Definitions up to date as the product reports it.
    pub up_to_date: Option<bool>,
    pub version: Option<String>,
    /// When the definitions were updated (`YYYY-MM-DD HH:MM UTC`).
    pub updated: Option<String>,
    pub age_days: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct FirewallProfile {
    pub name: String,
    pub on: bool,
}

/// Definitions older than this many days are flagged (#211).
pub const DEFINITIONS_MAX_AGE_DAYS: u32 = 7;

impl Security {
    /// "Virus & threat protection": some antivirus has real-time protection on.
    pub fn virus_protection(&self) -> Option<bool> {
        (!self.antivirus.is_empty()).then(|| self.antivirus.iter().any(|a| a.on))
    }

    /// "Firewall & network protection": every Windows Firewall profile on, or
    /// a third-party firewall on.
    pub fn firewall(&self) -> Option<bool> {
        if !self.firewall_products.is_empty() {
            return Some(true);
        }
        (!self.firewall_profiles.is_empty()).then(|| self.firewall_profiles.iter().all(|p| p.on))
    }

    /// Windows Firewall profiles that are off.
    pub fn firewall_off(&self) -> Vec<&str> {
        self.firewall_profiles
            .iter()
            .filter(|p| !p.on)
            .map(|p| p.name.as_str())
            .collect()
    }

    /// The antivirus that protects now (else the first one listed).
    pub fn active_antivirus(&self) -> Option<&Antivirus> {
        self.antivirus
            .iter()
            .find(|a| a.on)
            .or_else(|| self.antivirus.first())
    }

    /// The active antivirus' definitions are out of date or older than
    /// [`DEFINITIONS_MAX_AGE_DAYS`].
    pub fn definitions_stale(&self) -> bool {
        self.active_antivirus().is_some_and(|a| {
            a.up_to_date == Some(false) || a.age_days.is_some_and(|d| d > DEFINITIONS_MAX_AGE_DAYS)
        })
    }

    /// What is off or out of date, for flags (empty = all fine or unknown).
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.virus_protection() == Some(false) {
            p.push("Virus & threat protection is off".to_string());
        }
        if self.definitions_stale()
            && let Some(a) = self.active_antivirus()
        {
            p.push(match a.age_days {
                Some(d) => format!("{} definitions are {d} days old", a.name),
                None => format!("{} definitions are out of date", a.name),
            });
        }
        if self.firewall() == Some(false) {
            p.push(format!(
                "Firewall off for: {}",
                self.firewall_off().join(", ")
            ));
        }
        if self.app_control == Some(false) {
            p.push("App & browser control (SmartScreen) is off".to_string());
        }
        p
    }
}

/// An installed program (#211), from the Windows Uninstall registry keys.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct App {
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// `YYYY-MM-DD`, when the program recorded it.
    pub installed: Option<String>,
    /// As the program reports it (`EstimatedSize`).
    pub size_kb: Option<u64>,
}

/// What needs doing on a host: reboot, updates, OS release past end of support.
/// `None` = could not be determined (tool missing, unknown release).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Maintenance {
    /// The host's own date (`YYYY-MM-DD`), used for the end-of-support check.
    pub today: Option<String>,
    pub reboot_required: Option<bool>,
    /// Packages that asked for the reboot (Debian/Ubuntu).
    pub reboot_packages: Vec<String>,
    pub updates: Option<u32>,
    pub security_updates: Option<u32>,
    /// Last update installed (`YYYY-MM-DD`; Windows Update on My Computer, #211).
    pub last_update: Option<String>,
    /// End of vendor support for this release (`YYYY-MM-DD`), when known.
    pub end_of_support: Option<String>,
}

impl Maintenance {
    pub fn support(&self) -> Option<crate::eol::Support> {
        crate::eol::support(self.end_of_support.as_deref()?, self.today.as_deref()?)
    }

    /// Anything to show in the overview (reboot, updates, ending or ended support).
    pub fn needs_attention(&self) -> bool {
        self.reboot_required == Some(true)
            || self.updates.is_some_and(|n| n > 0)
            || matches!(
                self.support(),
                Some(crate::eol::Support::Ended { .. } | crate::eol::Support::EndingSoon { .. })
            )
    }
}

/// A listening socket on the host (`ss -tuln` / `netstat -tuln`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetAddr {
    pub interface: String,
    pub family: String,
    pub address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SensorKind {
    Temperature,
    /// Fan speed in RPM.
    Fan,
    /// Fan speed as a share of its maximum (GPUs report this, #193).
    FanPercent,
    /// Power draw in watts (GPUs, #193).
    Power,
}

impl SensorKind {
    /// The value with its unit, as shown in tables and `telepad-check`.
    pub fn format(self, value: f32) -> String {
        match self {
            SensorKind::Temperature => format!("{value:.1} °C"),
            SensorKind::Fan => format!("{value:.0} RPM"),
            SensorKind::FanPercent => format!("{value:.0} %"),
            SensorKind::Power => format!("{value:.1} W"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sensor {
    pub kind: SensorKind,
    pub chip: String,
    pub label: String,
    /// In the unit of its [`SensorKind`]: °C, RPM, % or W.
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
    /// Image name for narrow columns: without the `docker.io/` registry, and an
    /// untagged image ID (`sha256:…`) cut to the 12 characters Docker shows.
    pub fn short_image(&self) -> String {
        if let Some(id) = self.image.strip_prefix("sha256:") {
            return format!("{}…", id.chars().take(12).collect::<String>());
        }
        self.image
            .strip_prefix("docker.io/")
            .unwrap_or(&self.image)
            .to_string()
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
