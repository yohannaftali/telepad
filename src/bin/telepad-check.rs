//! Headless check: collect hosts once and print the result.
//! Usage: telepad-check [--config <path>] [--json] [alias ...]
//!        telepad-check --check-update   (ask GitHub for a newer release)

use std::process::ExitCode;
use std::sync::Arc;
use std::thread;

use telepad::model::{human_kb, human_uptime};
use telepad::{collector, open_workspace, update};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut cfg, mut db, mut json, mut aliases) = (None, None, false, Vec::new());
    let mut check_update = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg = args.next(),
            "--db" => db = args.next(),
            "--json" => json = true,
            "--check-update" => check_update = true,
            "-h" | "--help" => {
                println!(
                    "telepad-check [--db <telepad.db>] [--config <ssh config>] [--json] [alias ...]
\n                     telepad-check [--db <telepad.db>] --check-update"
                );
                return ExitCode::SUCCESS;
            }
            _ => aliases.push(a),
        }
    }
    let ws = match open_workspace(cfg.as_deref(), db.as_deref()) {
        Ok(w) => w,
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
        for (h, r) in &results {
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
                        println!("   Sensor  : {} {} = {:.1}", s.chip, s.label, s.value);
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
fn run_check_update(ws: &telepad::Workspace) -> ExitCode {
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
