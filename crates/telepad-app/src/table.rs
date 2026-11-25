//! Overview table: which rows to show and in what order. Pure functions, so
//! the view only draws and sorting is unit-tested here.

use std::cmp::Ordering;
use std::net::IpAddr;
use std::time::Instant;

use telepad_core::eol::Support;
use telepad_core::model::{Container, Disk, HostReport, Maintenance, SensorKind};

use crate::state::{HostState, Model, matches_filter};

/// A sortable overview column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Status,
    Host,
    Description,
    Address,
    Os,
    Cpu,
    Memory,
    Disk,
    Temp,
    Docker,
    Updated,
    /// Maintenance flags (end of support, reboot, updates), most urgent first.
    Flags,
    /// Group / cluster name (#157), A to Z; hosts without one last.
    Group,
    Cluster,
}

impl SortKey {
    /// Every key, in column order (for the card view's picker).
    pub const ALL: [SortKey; 14] = [
        Self::Status,
        Self::Host,
        Self::Description,
        Self::Address,
        Self::Os,
        Self::Cpu,
        Self::Memory,
        Self::Disk,
        Self::Temp,
        Self::Docker,
        Self::Updated,
        Self::Flags,
        Self::Group,
        Self::Cluster,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Host => "Host",
            Self::Description => "Description",
            Self::Address => "Address",
            Self::Os => "OS",
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Disk => "Fullest disk",
            Self::Temp => "Temperature",
            Self::Docker => "Docker",
            Self::Updated => "Updated",
            Self::Flags => "Flags",
            Self::Group => "Group",
            Self::Cluster => "Cluster",
        }
    }

    /// Numbers start with the largest (busiest, hottest, newest) first.
    pub fn starts_descending(self) -> bool {
        matches!(
            self,
            Self::Cpu
                | Self::Memory
                | Self::Disk
                | Self::Temp
                | Self::Docker
                | Self::Updated
                | Self::Flags
        )
    }
}

/// Current overview order; `None` = the inventory order from the database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sort {
    pub key: Option<SortKey>,
    pub descending: bool,
}

impl Sort {
    /// Header click: new column → its natural direction; same column →
    /// reversed; third click → back to inventory order.
    pub fn clicked(self, key: SortKey) -> Self {
        match self.key {
            Some(k) if k == key && self.descending != key.starts_descending() => Self::default(),
            Some(k) if k == key => Self {
                key: Some(key),
                descending: !self.descending,
            },
            _ => Self {
                key: Some(key),
                descending: key.starts_descending(),
            },
        }
    }

    /// `Some(descending)` when `key` is the sort column (for the header arrow).
    pub fn direction(self, key: SortKey) -> Option<bool> {
        (self.key == Some(key)).then_some(self.descending)
    }
}

/// Indices into `m.hosts` that match `filter`, in `sort` order (stable; hosts
/// without a value for the sort column always come last).
pub fn overview_rows(m: &Model, filter: &str, sort: Sort) -> Vec<usize> {
    let mut rows: Vec<usize> = m
        .hosts
        .iter()
        .enumerate()
        .filter(|(_, h)| matches_filter(filter, h))
        .map(|(i, _)| i)
        .collect();
    if let Some(key) = sort.key {
        rows.sort_by(|&a, &b| {
            let (a, b) = (sort_value(&m.hosts[a], key), sort_value(&m.hosts[b], key));
            match (a, b) {
                (Some(a), Some(b)) => {
                    let o = a.cmp(&b);
                    if sort.descending { o.reverse() } else { o }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        });
    }
    rows
}

#[derive(Debug)]
enum Value {
    Num(f64),
    Text(String),
    At(Instant),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Value {}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Value::Num(a), Value::Num(b)) => a.total_cmp(b),
            (Value::Text(a), Value::Text(b)) => a.cmp(b),
            (Value::At(a), Value::At(b)) => a.cmp(b),
            // Only Address mixes kinds: IPv4 numbers before names.
            (a, b) => rank(a).cmp(&rank(b)),
        }
    }
}

fn rank(v: &Value) -> u8 {
    match v {
        Value::Num(_) => 0,
        Value::Text(_) => 1,
        Value::At(_) => 2,
    }
}

fn text(s: &str) -> Option<Value> {
    let s = s.trim();
    (!s.is_empty()).then(|| Value::Text(s.to_lowercase()))
}

fn sort_value(h: &HostState, key: SortKey) -> Option<Value> {
    let r = h.report.as_ref();
    match key {
        // down, then unknown/loading, then up
        SortKey::Status => Some(Value::Num(match h.online() {
            Some(false) => 0.0,
            None => 1.0,
            Some(true) => 2.0,
        })),
        SortKey::Host => text(&h.entry.alias),
        SortKey::Description => text(&h.entry.note),
        SortKey::Address => address_value(&h.entry.hostname).or_else(|| text(&h.entry.alias)),
        SortKey::Os => r.and_then(|r| text(&r.os_pretty)),
        SortKey::Cpu => r
            .and_then(|r| r.cpu_usage_pct)
            .map(|v| Value::Num(v.into())),
        SortKey::Memory => r
            .and_then(|r| r.memory.as_ref())
            .map(|m| Value::Num(m.used_pct().into())),
        SortKey::Disk => r
            .and_then(fullest_disk)
            .map(|d| Value::Num(d.used_pct().into())),
        SortKey::Temp => r.and_then(hottest).map(|t| Value::Num(t.into())),
        SortKey::Docker => r
            .filter(|r| r.docker.unavailable.is_none())
            .map(|r| Value::Num(r.docker.containers.iter().filter(|c| c.running()).count() as f64)),
        // Most recent first when descending: newer Instants are larger.
        SortKey::Updated => h.updated.map(Value::At),
        SortKey::Flags => r.map(|r| Value::Num(host_urgency(r) as f64)),
        SortKey::Group => text(&h.entry.group),
        SortKey::Cluster => text(&h.entry.cluster),
    }
}

/// IPv4/IPv6 addresses sort numerically (10.0.0.9 before 10.0.0.10).
fn address_value(s: &str) -> Option<Value> {
    let s = s.trim();
    match s.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => Some(Value::Num(f64::from(u32::from(v4)))),
        Ok(IpAddr::V6(v6)) => Some(Value::Text(format!("~{:032x}", u128::from(v6)))),
        Err(_) => text(s),
    }
}

/// How urgent a maintenance flag is (drives icon colour).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagKind {
    Reboot,
    Updates,
    EndOfSupport,
    /// Swarm / Kubernetes nodes or services not healthy.
    Cluster,
}

/// One maintenance flag: what, how urgent, a short label and a hover text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    pub kind: FlagKind,
    pub severity: Severity,
    pub short: String,
    pub detail: String,
}

/// Flags to show for a host, most urgent first: end of support (ended = danger,
/// ending within 6 months = warn), reboot required (warn), pending updates
/// (danger with security updates, else info).
pub fn maintenance_flags(m: &Maintenance) -> Vec<Flag> {
    let mut flags = Vec::new();
    match (m.support(), m.end_of_support.as_deref()) {
        (Some(Support::Ended { days_ago }), Some(date)) => flags.push(Flag {
            kind: FlagKind::EndOfSupport,
            severity: Severity::Danger,
            short: "EOL".into(),
            detail: format!(
                "OS release out of vendor support since {date} ({days_ago} days): no more security fixes — upgrade the OS"
            ),
        }),
        (Some(Support::EndingSoon { days_left }), Some(date)) => flags.push(Flag {
            kind: FlagKind::EndOfSupport,
            severity: Severity::Warn,
            short: format!("EOL {days_left}d"),
            detail: format!("OS release support ends on {date} (in {days_left} days): plan the upgrade"),
        }),
        _ => {}
    }
    if m.reboot_required == Some(true) {
        let why = if m.reboot_packages.is_empty() {
            String::new()
        } else {
            format!(" — requested by {}", m.reboot_packages.join(", "))
        };
        flags.push(Flag {
            kind: FlagKind::Reboot,
            severity: Severity::Warn,
            short: "reboot".into(),
            detail: format!("Reboot required{why}"),
        });
    }
    if let Some(n) = m.updates.filter(|&n| n > 0) {
        let sec = m.security_updates.unwrap_or(0);
        flags.push(Flag {
            kind: FlagKind::Updates,
            severity: if sec > 0 {
                Severity::Danger
            } else {
                Severity::Info
            },
            short: if sec > 0 {
                format!("{n} ({sec} sec)")
            } else {
                n.to_string()
            },
            detail: if sec > 0 {
                format!("{n} package updates pending, {sec} of them security updates")
            } else {
                format!("{n} package updates pending")
            },
        });
    }
    flags.sort_by_key(|f| std::cmp::Reverse(f.severity));
    flags
}

/// Maintenance flags plus a degraded Swarm / Kubernetes cluster.
pub fn host_flags(r: &HostReport) -> Vec<Flag> {
    let mut flags = maintenance_flags(&r.maintenance);
    let problems = r.cluster.degraded();
    if !problems.is_empty() {
        flags.insert(
            0,
            Flag {
                kind: FlagKind::Cluster,
                severity: Severity::Danger,
                short: "cluster".into(),
                detail: format!("Cluster degraded: {}", problems.join("; ")),
            },
        );
    }
    flags
}

/// Sort weight for the Flags column over the whole report.
pub fn host_urgency(r: &HostReport) -> u64 {
    let cluster = if r.cluster.degraded().is_empty() {
        0
    } else {
        2_000_000
    };
    cluster + urgency(&r.maintenance)
}

/// Another host earlier in the list that shows the same Swarm / Kubernetes
/// cluster (several managers, or one machine under two aliases).
pub fn cluster_shown_by(m: &Model, i: usize) -> Option<usize> {
    let key = m.hosts.get(i)?.report.as_ref()?.cluster.key()?;
    m.hosts[..i]
        .iter()
        .position(|h| h.report.as_ref().and_then(|r| r.cluster.key()).as_deref() == Some(&key))
}

/// Sort weight for the Flags column: ended support > reboot > security > updates.
pub fn urgency(m: &Maintenance) -> u64 {
    let mut u = 0;
    match m.support() {
        Some(Support::Ended { .. }) => u += 1_000_000,
        Some(Support::EndingSoon { .. }) => u += 300_000,
        _ => {}
    }
    if m.reboot_required == Some(true) {
        u += 100_000;
    }
    u += u64::from(m.security_updates.unwrap_or(0)).min(999) * 100;
    u += u64::from(m.updates.unwrap_or(0)).min(99);
    u
}

/// Status group in the host list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGroup {
    Down,
    Up,
    /// Not checked yet, loading, or not monitored.
    Pending,
}

impl HostGroup {
    pub fn title(self) -> &'static str {
        match self {
            Self::Down => "Down",
            Self::Up => "Up",
            Self::Pending => "Not checked yet",
        }
    }

    fn of(h: &HostState) -> Self {
        match h.online() {
            Some(false) => Self::Down,
            Some(true) => Self::Up,
            None => Self::Pending,
        }
    }
}

/// Host-list rows matching `filter`, in inventory order: one unnamed group, or
/// (with `by_status`) Down, Up, Not checked yet — empty groups left out.
pub fn host_list_groups(
    m: &Model,
    filter: &str,
    by_status: bool,
) -> Vec<(Option<HostGroup>, Vec<usize>)> {
    let rows = overview_rows(m, filter, Sort::default());
    if !by_status {
        return vec![(None, rows)];
    }
    [HostGroup::Down, HostGroup::Up, HostGroup::Pending]
        .into_iter()
        .map(|g| {
            let members = rows
                .iter()
                .copied()
                .filter(|&i| HostGroup::of(&m.hosts[i]) == g)
                .collect::<Vec<_>>();
            (Some(g), members)
        })
        .filter(|(_, members)| !members.is_empty())
        .collect()
}

/// One stack in the Docker overview: every container with this compose project
/// name, from every host. `name` is empty for containers outside any stack.
#[derive(Debug, Clone)]
pub struct StackRow<'a> {
    pub name: &'a str,
    /// (host index, container), by host order then running first.
    pub containers: Vec<(usize, &'a Container)>,
}

impl StackRow<'_> {
    pub fn running(&self) -> usize {
        self.containers.iter().filter(|(_, c)| c.running()).count()
    }

    /// Host indexes this stack runs on, in inventory order.
    pub fn hosts(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.containers.iter().map(|(i, _)| *i).collect();
        v.dedup();
        v
    }
}

/// Docker overview: containers of all hosts grouped by stack, named stacks A–Z,
/// standalone containers last. The search matches stack, container, image or host.
pub fn docker_stacks<'a>(m: &'a Model, filter: &str) -> Vec<StackRow<'a>> {
    let f = filter.to_lowercase();
    let mut rows: Vec<StackRow<'a>> = Vec::new();
    for (i, h) in m.hosts.iter().enumerate() {
        let Some(r) = &h.report else { continue };
        let host_hit = f.is_empty()
            || h.entry.alias.to_lowercase().contains(&f)
            || h.entry.hostname.to_lowercase().contains(&f);
        for c in &r.docker.containers {
            let hit = host_hit
                || [c.stack.as_str(), &c.name, &c.image]
                    .iter()
                    .any(|s| s.to_lowercase().contains(&f));
            if !hit {
                continue;
            }
            match rows.iter_mut().find(|s| s.name == c.stack) {
                Some(s) => s.containers.push((i, c)),
                None => rows.push(StackRow {
                    name: &c.stack,
                    containers: vec![(i, c)],
                }),
            }
        }
    }
    rows.sort_by(|a, b| {
        a.name
            .is_empty()
            .cmp(&b.name.is_empty())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for s in &mut rows {
        // Stable: hosts keep inventory order, containers their report order.
        s.containers
            .sort_by(|a, b| a.0.cmp(&b.0).then(b.1.running().cmp(&a.1.running())));
    }
    rows
}

/// Where a dragged host ends up: dropped on the upper or lower half (`below`)
/// of the row at `onto`, as an index into the list after it was taken out.
pub fn drop_index(from: usize, onto: usize, below: bool) -> usize {
    let before = onto + usize::from(below);
    if from < before { before - 1 } else { before }
}

/// Dragging hosts is offered only in plain inventory order: no search, no
/// grouping, no column sort (the position would be ambiguous otherwise).
pub fn can_reorder(filter: &str, grouped: bool, sort: Sort) -> bool {
    filter.trim().is_empty() && !grouped && sort.key.is_none()
}

/// The highest of CPU, memory and fullest-disk usage, with its name.
pub fn worst_usage(r: &HostReport) -> Option<(&'static str, f32)> {
    [
        ("CPU", r.cpu_usage_pct),
        ("RAM", r.memory.as_ref().map(|m| m.used_pct())),
        ("Disk", fullest_disk(r).map(|d| d.used_pct())),
    ]
    .into_iter()
    .filter_map(|(n, v)| v.map(|v| (n, v)))
    .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// The mount with the highest usage.
pub fn fullest_disk(r: &HostReport) -> Option<&Disk> {
    r.disks
        .iter()
        .max_by(|a, b| a.used_pct().total_cmp(&b.used_pct()))
}

/// Highest temperature reading, in °C.
pub fn hottest(r: &HostReport) -> Option<f32> {
    r.sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .map(|s| s.value)
        .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))))
}

/// Which kind of cluster a page lists (#84).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterKind {
    Kube,
    Swarm,
}

/// One cluster for the Kubernetes / Docker Swarm pages: the host whose report
/// is shown, and every host that reported the same cluster (several managers,
/// or one machine under two aliases).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterRow {
    /// `k8s:<id>` / `swarm:<id>`, or `host:<alias>` when the id is unknown.
    pub key: String,
    pub host: usize,
    pub hosts: Vec<usize>,
}

/// Clusters of `kind` in inventory order of their first host. Swarm: only
/// managers know the cluster (workers are listed as its nodes).
pub fn cluster_rows(m: &Model, kind: ClusterKind) -> Vec<ClusterRow> {
    let mut out: Vec<ClusterRow> = Vec::new();
    for (i, h) in m.hosts.iter().enumerate() {
        let Some(r) = &h.report else { continue };
        let key = match kind {
            ClusterKind::Kube => match &r.cluster.kube {
                Some(k) if !k.cluster_id.is_empty() => format!("k8s:{}", k.cluster_id),
                Some(_) => format!("host:{}", h.entry.alias),
                None => continue,
            },
            ClusterKind::Swarm => match &r.cluster.swarm {
                Some(s) if s.manager && !s.cluster_id.is_empty() => {
                    format!("swarm:{}", s.cluster_id)
                }
                Some(s) if s.manager => format!("host:{}", h.entry.alias),
                _ => continue,
            },
        };
        match out.iter_mut().find(|c| c.key == key) {
            Some(c) => c.hosts.push(i),
            None => out.push(ClusterRow {
                key,
                host: i,
                hosts: vec![i],
            }),
        }
    }
    out
}
