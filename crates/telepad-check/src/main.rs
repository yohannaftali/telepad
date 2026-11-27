//! Headless check: collect hosts once and print the result.
//! Usage: telepad-check [--config <path>] [--json] [alias ...]
//!        telepad-check --check-update   (ask GitHub for a newer release)
//!        telepad-check --logins alias ...  (SSH logins from the journal, #126)
//!        telepad-check --local [--json]  (this computer, #129)
//!        telepad-check --portainer name [--json]  (a registered Portainer, #171)

use std::process::ExitCode;
use std::sync::Arc;
use std::thread;

use telepad_core::model::{human_kb, human_uptime};
use telepad_core::{collector, open_workspace, update};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut cfg, mut db, mut json, mut aliases) = (None, None, false, Vec::new());
    let mut check_update = false;
    let mut logins = false;
    let mut local = false;
    let mut portainer: Option<String> = None;
    let mut stack: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg = args.next(),
            "--db" => db = args.next(),
            "--json" => json = true,
            "--check-update" => check_update = true,
            "--logins" => logins = true,
            "--local" => local = true,
            "--portainer" => portainer = args.next(),
            "--stack" => stack = args.next(),
            "-h" | "--help" => {
                println!(
                    "telepad-check [--db <telepad.db>] [--config <ssh config>] [--json] [alias ...]
\n                     telepad-check [--db <telepad.db>] --check-update\n                     telepad-check --local [--json]\n                     telepad-check [--db <telepad.db>] --portainer <name> [--json]"
                );
                return ExitCode::SUCCESS;
            }
            _ => aliases.push(a),
        }
    }
    if local {
        let r = telepad_core::local::collect();
        return report(&[(telepad_core::config::HostEntry::local(), r)], json);
    }
    let ws = match open_workspace(cfg.as_deref(), db.as_deref()) {
        Ok(w) => w,
        Err(telepad_core::OpenError::NewerSchema(n)) => {
            eprintln!("{}", n.explanation());
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if let Some(src) = &ws.imported_from {
        eprintln!(
            "created {} from {}",
            ws.store.path().display(),
            src.display()
        );
    }
    if check_update {
        return run_check_update(&ws);
    }
    if let Some(name) = portainer {
        return run_portainer(&ws, &name, stack.as_deref(), json);
    }
    let hosts: Vec<_> = match ws.store.hosts() {
        Ok(h) => h
            .into_iter()
            .filter(|h| !h.unmonitored || !aliases.is_empty())
            .collect(),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let selected: Vec<_> = if aliases.is_empty() {
        hosts
    } else {
        let unknown: Vec<_> = aliases
            .iter()
            .filter(|a| !hosts.iter().any(|h| &h.alias == *a))
            .collect();
        if !unknown.is_empty() {
            eprintln!("unknown host alias: {unknown:?}");
            return ExitCode::from(2);
        }
        hosts
            .into_iter()
            .filter(|h| aliases.contains(&h.alias))
            .collect()
    };

    let settings = Arc::new(ws.settings());
    if logins {
        return run_logins(&settings, &selected);
    }
    let handles: Vec<_> = selected
        .into_iter()
        .map(|h| {
            let s = settings.clone();
            thread::spawn(move || {
                let r = collector::collect_one(&s, &h);
                (h, r)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().filter_map(|t| t.join().ok()).collect();
    report(&results, json)
}

type Collected = (
    telepad_core::config::HostEntry,
    Result<telepad_core::model::HostReport, String>,
);

/// Print the results (text or JSON); fails when any host failed.
fn report(results: &[Collected], json: bool) -> ExitCode {
    let failed = results.iter().filter(|(_, r)| r.is_err()).count();

    if json {
        let v: Vec<_> = results
            .iter()
            .map(|(h, r)| match r {
                Ok(rep) => serde_json::json!({"alias": h.alias, "host": h.hostname, "ok": true, "report": rep}),
                Err(e) => serde_json::json!({"alias": h.alias, "host": h.hostname, "ok": false, "error": e}),
            })
            .collect();
        match serde_json::to_string_pretty(&v) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("{e}"),
        }
    } else {
        for (h, r) in results {
            println!("== {} ({})", h.alias, h.target());
            match r {
                Err(e) => println!("   ERROR: {e}"),
                Ok(r) if r.rdp_only => {
                    let ping = r
                        .ping_ms
                        .map_or("no reply".into(), |ms| format!("{ms:.0} ms"));
                    let rdp = if r.rdp_open { "open" } else { "closed" };
                    println!("   RDP-only: ping {ping}, RDP port {} {rdp}", h.port);
                }
                Ok(r) => {
                    println!(
                        "   OS      : {} [{} {} {}]",
                        r.os_pretty, r.os_type, r.kernel, r.arch
                    );
                    if let Some(u) = r.uptime_secs {
                        println!("   Uptime  : {}", human_uptime(u));
                    }
                    println!(
                        "   CPU     : {} x{}  usage {}  load {}",
                        r.cpu_model,
                        r.cpu_cores.map_or("?".into(), |c| c.to_string()),
                        r.cpu_usage_pct.map_or("?".into(), |c| format!("{c:.1}%")),
                        r.load_avg
                            .map_or("?".into(), |l| format!("{} {} {}", l[0], l[1], l[2]))
                    );
                    if let Some(m) = &r.memory {
                        println!(
                            "   Memory  : {} / {} ({:.1}%)",
                            human_kb(m.used_kb()),
                            human_kb(m.total_kb),
                            m.used_pct()
                        );
                    }
                    for d in &r.disks {
                        println!(
                            "   Disk    : {:<20} {} / {} ({:.1}%)",
                            d.mount,
                            human_kb(d.used_kb),
                            human_kb(d.total_kb),
                            d.used_pct()
                        );
                    }
                    for a in &r.addresses {
                        println!("   IP      : {:<12} {}", a.interface, a.address);
                    }
                    if let Some(e) = &r.bmc_error {
                        println!("   BMC     : not readable: {e}");
                    }
                    for s in &r.sensors {
                        println!(
                            "   Sensor  : {} {} = {}",
                            s.chip,
                            s.label,
                            s.kind.format(s.value)
                        );
                    }
                    if let Some(n) = &r.sensors_note {
                        println!("   Sensors : {n}");
                    }
                    match &r.docker.unavailable {
                        Some(why) => println!("   Docker  : {why}"),
                        None => {
                            println!("   Runtime : {}", r.docker.runtime);
                            for (role, c) in r.docker.portainer() {
                                println!(
                                    "   Portainer: {role:?} ({}) socket={}",
                                    c.name, c.mounts_socket
                                );
                            }
                            for (stack, cs) in r.docker.stacks() {
                                println!(
                                    "   Stack   : {} — {:?}",
                                    if stack.is_empty() {
                                        "(standalone)"
                                    } else {
                                        stack
                                    },
                                    r.docker.stack_manager(&cs)
                                );
                                for c in cs {
                                    let ips: Vec<_> = c
                                        .networks
                                        .iter()
                                        .map(|(n, ip)| format!("{n}={ip}"))
                                        .collect();
                                    println!(
                                        "             {:<28} {:<8} {} | {}",
                                        c.name,
                                        c.state,
                                        ips.join(","),
                                        c.ports
                                    );
                                }
                            }
                        }
                    }
                    let usage = r.port_usage();
                    let nginx: Vec<String> = usage
                        .iter()
                        .filter(|u| u.nginx && u.proto == "tcp")
                        .map(|u| u.port.to_string())
                        .collect();
                    println!(
                        "   Ports   : {} in use; nginx: {}",
                        usage.len(),
                        nginx.join(", ")
                    );
                }
            }
        }
        println!("\n{} host(s), {} failed", results.len(), failed);
    }
    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Exit 0 = up to date, 10 = newer release available, 2 = check failed.
fn run_check_update(ws: &telepad_core::Workspace) -> ExitCode {
    let cfg = match ws.store.update_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("update settings: {e}");
            return ExitCode::from(2);
        }
    };
    let repo = cfg.repo.clone();
    match update::Updater::new(cfg).check() {
        Ok(c) => {
            println!(
                "installed {}, latest {} on github.com/{repo}; download: {}",
                update::CURRENT_VERSION,
                c.version,
                c.asset
                    .as_ref()
                    .map_or("none for this platform".to_string(), |a| a.name.clone())
            );
            if c.newer {
                println!("update available");
                ExitCode::from(10)
            } else {
                println!("up to date");
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("update check failed (github.com/{repo}): {e}");
            ExitCode::from(2)
        }
    }
}

/// `--portainer`: read a registered Portainer (#171) like its page does:
/// through the tunnel, token from the OS password store, `GET`s only.
fn run_portainer(
    ws: &telepad_core::Workspace,
    name: &str,
    stack: Option<&str>,
    json: bool,
) -> ExitCode {
    use telepad_core::portainer as pc;
    let list = match ws.store.portainers() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let Some(p) = list.iter().find(|p| p.name.eq_ignore_ascii_case(name)) else {
        let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
        eprintln!("no Portainer named {name}; registered: {names:?}");
        return ExitCode::from(2);
    };
    let hosts = ws.store.hosts().unwrap_or_default();
    let Some(host) = hosts.iter().find(|h| h.alias == p.alias) else {
        eprintln!("host {} of Portainer {} is not in Telepad", p.alias, p.name);
        return ExitCode::from(2);
    };
    let token = match pc::load_token(&p.name) {
        Ok(Some(t)) => t,
        Ok(None) => {
            eprintln!("{}", pc::ReadError::NoToken);
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("password store: {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(sname) = stack {
        return run_portainer_stack(ws, host, p, &token, sname);
    }
    match pc::read(&ws.settings(), host, p, &token, None) {
        Ok((snap, seen)) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"portainer": p.name, "host": p.alias, "certificate": seen, "snapshot": snap})
                );
                return ExitCode::SUCCESS;
            }
            println!(
                "Portainer {} on {} port {}: version {}",
                p.name, p.alias, p.port, snap.version
            );
            if let Some(fp) = &seen {
                let pinned = if p.fingerprint.is_empty() {
                    "not pinned yet (the app pins it on its first read)"
                } else if p.fingerprint.eq_ignore_ascii_case(fp) {
                    "pinned"
                } else {
                    "differs from the pinned one"
                };
                println!("  certificate {} ({pinned})", pc::short_fp(fp));
            }
            for e in &snap.environments {
                let up = match e.up {
                    Some(true) => "up",
                    Some(false) => "down",
                    None => "?",
                };
                let on = pc::env_host(e, &p.alias, &hosts)
                    .map(|h| format!(" -> {}", h.alias))
                    .unwrap_or_default();
                println!(
                    "  [{}] {} ({}, {up}) {}{on}",
                    e.id,
                    e.name,
                    e.kind_name(),
                    e.url
                );
                for s in snap.stacks.iter().filter(|s| s.env == e.id) {
                    let state = match s.active {
                        Some(true) => "active",
                        Some(false) => "inactive",
                        None => "?",
                    };
                    println!("      stack {} ({}, {state})", s.name, s.kind_name());
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Portainer {}: {e}", p.name);
            ExitCode::from(1)
        }
    }
}

/// `--portainer <name> --stack <stack>`: one stack as the editor reads it
/// (#173), read-only: variable names (never values), the compose file's
/// size and the stack's containers.
fn run_portainer_stack(
    ws: &telepad_core::Workspace,
    host: &telepad_core::config::HostEntry,
    p: &telepad_core::portainer::Portainer,
    token: &telepad_core::db::Secret,
    sname: &str,
) -> ExitCode {
    use telepad_core::portainer as pc;
    let run = || -> Result<(), pc::ReadError> {
        let conn = pc::Conn::open(&ws.settings(), host, p, token, None)?;
        let snap = pc::read(&ws.settings(), host, p, token, None)?.0;
        let stacks: Vec<_> = snap
            .stacks
            .iter()
            .filter(|s| s.name.eq_ignore_ascii_case(sname))
            .collect();
        if stacks.is_empty() {
            return Err(pc::ReadError::Other(format!("no stack named {sname}")));
        }
        for s in stacks {
            let d = match conn.stack(s.id) {
                Ok(d) => d,
                Err(e) => {
                    println!("stack {} (id {}): {e}", s.name, s.id);
                    continue;
                }
            };
            let env = snap.environments.iter().find(|e| e.id == s.env);
            println!(
                "stack {} (id {}, {}, env [{}] {}, updated {:?}{})",
                d.name,
                d.id,
                s.kind_name(),
                s.env,
                env.map_or("?", |e| e.name.as_str()),
                d.updated,
                d.git
                    .as_ref()
                    .map_or(String::new(), |g| format!(", git {g}"))
            );
            let names: Vec<&str> = d.env.iter().map(|v| v.name.as_str()).collect();
            println!("  variables: {names:?}");
            println!(
                "  compose file: {} lines, {} bytes",
                d.file.lines().count(),
                d.file.len()
            );
            if let Some(why) = pc::stack_locked(s, env) {
                println!("  locked: {why}");
            }
            match conn.stack_containers(s) {
                Ok(list) => {
                    for c in list {
                        println!(
                            "  container {} ({}) {} [{}] {}",
                            c.name, c.image, c.state, c.status, c.ports
                        );
                    }
                }
                Err(e) => println!("  containers: {e}"),
            }
        }
        Ok(())
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Portainer {}: {e}", p.name);
            ExitCode::from(1)
        }
    }
}

/// `--logins`: the Log info page's read (#126), newest 15 logins per host.
fn run_logins(s: &collector::SshSettings, hosts: &[telepad_core::config::HostEntry]) -> ExitCode {
    use telepad_core::login_log::{self, JournalAccess};
    let mut failed = false;
    for h in hosts {
        println!("== {} ({})", h.alias, h.target());
        let log = match login_log::read(s, h) {
            Ok(l) => l,
            Err(e) => {
                println!("   ERROR: {e}");
                failed = true;
                continue;
            }
        };
        let access = match log.access {
            JournalAccess::Readable => "readable",
            JournalAccess::Denied => "denied (Register Systemd Journal)",
            JournalAccess::NoJournal => "no journal on this host",
        };
        println!(
            "   user {} · groups {} · journal {access} · since {}",
            log.user,
            log.groups.join(" "),
            log.first.as_deref().unwrap_or("?")
        );
        let used = log.this_session().map(|l| l.fingerprint.clone());
        println!(
            "   logins {}{} · this session: {}",
            log.logins.len(),
            if log.truncated { " (limit)" } else { "" },
            used.as_deref().unwrap_or("not found")
        );
        for l in log.logins.iter().rev().take(15) {
            println!(
                "   {}  {:<10} {:<16} {:<10} {} {}",
                l.time, l.user, l.from, l.method, l.key_type, l.fingerprint
            );
        }
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
