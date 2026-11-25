//! Docker Swarm / Kubernetes section of the host detail.

use eframe::egui::{self, RichText};
use telepad_app::Msg;
use telepad_core::cluster::{Kube, Swarm};
use telepad_core::docker_actions::{NodeAction, ServiceAction, WorkloadAction};
use telepad_core::model::HostReport;

use crate::icons::{self, label as il};
use crate::table::{self, Col, Ev, Spec};
use crate::widgets::*;
use telepad_app::Drafts;

const TERMINAL_HINT: &str =
    "Opens a terminal on the host that shows the exact command and runs it only after you confirm";

/// True when the section has something to show.
pub fn has_cluster(r: &HostReport) -> bool {
    r.cluster.swarm.is_some() || r.cluster.kube.is_some() || r.cluster.kube_error.is_some()
}

pub fn cluster(
    ui: &mut egui::Ui,
    r: &HostReport,
    host: usize,
    key: &str,
    shown_by: Option<&str>,
    d: &mut Drafts,
    out: &mut Vec<Msg>,
) {
    if let Some(alias) = shown_by {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Same cluster as").weak());
            if ui.link(alias).clicked() {
                out.push(Msg::SelectAlias(alias.to_string()));
            }
            ui.label(RichText::new("— shown there").weak());
        });
        return;
    }
    let problems = r.cluster.degraded();
    if !problems.is_empty() {
        for p in &problems {
            ui.colored_label(pal().red, format!("{} {p}", icons::WARNING));
        }
        ui.add_space(4.0);
    }
    if let Some(sw) = &r.cluster.swarm {
        swarm(ui, sw, host, key, d, out);
    }
    if let Some(e) = &r.cluster.kube_error {
        ui.label(RichText::new(format!("Kubernetes: {e}")).weak());
    }
    if let Some(k) = &r.cluster.kube {
        kube(ui, k, host, key, d, out);
    }
}

/// The Kubernetes / Docker Swarm page (#84): every cluster once, with the hosts
/// Telepad reached it from; empty with a note when none was found.
pub fn cluster_page(
    ui: &mut egui::Ui,
    m: &telepad_app::Model,
    kind: telepad_app::ClusterKind,
    d: &mut Drafts,
    out: &mut Vec<Msg>,
) {
    use telepad_app::ClusterKind;
    let rows = telepad_app::cluster_rows(m, kind);
    let name = match kind {
        ClusterKind::Kube => "Kubernetes",
        ClusterKind::Swarm => "Docker Swarm",
    };
    ui.label(RichText::new(format!("{} {name} cluster(s)", rows.len())).strong());
    ui.add_space(4.0);
    if rows.is_empty() {
        ui.label(
            RichText::new(if m.hosts.iter().all(|h| h.report.is_none()) {
                "No host has reported yet.".to_string()
            } else {
                format!("No {name} cluster found on your hosts.")
            })
            .weak(),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt(("cluster-page", name))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in rows {
                let Some(r) = m.hosts.get(row.host).and_then(|h| h.report.as_ref()) else {
                    continue;
                };
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Reached from").weak());
                        for &i in &row.hosts {
                            if let Some(h) = m.hosts.get(i)
                                && ui
                                    .link(RichText::new(&h.entry.alias).strong())
                                    .on_hover_text("Open this host")
                                    .clicked()
                            {
                                out.push(Msg::Select(Some(i)));
                            }
                        }
                    });
                    let problems = r.cluster.degraded();
                    for p in &problems {
                        ui.colored_label(pal().red, format!("{} {p}", icons::WARNING));
                    }
                    let key = m.hosts[row.host].entry.alias.clone();
                    match kind {
                        ClusterKind::Kube => {
                            if let Some(k) = &r.cluster.kube {
                                kube(ui, k, row.host, &key, d, out);
                            }
                        }
                        ClusterKind::Swarm => {
                            if let Some(sw) = &r.cluster.swarm {
                                swarm(ui, sw, row.host, &key, d, out);
                            }
                        }
                    }
                });
                ui.add_space(6.0);
            }
        });
}

fn ok_color(ok: bool) -> egui::Color32 {
    if ok { pal().green } else { pal().red }
}

fn menu(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.menu_button(icons::MENU, |ui| {
        ui.label(RichText::new(TERMINAL_HINT).weak().small());
        ui.separator();
        add(ui);
    });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).strong());
}

fn colored(ui: &mut egui::Ui, c: egui::Color32, t: &str) {
    ui.add(
        egui::Label::new(RichText::new(t).color(c))
            .truncate()
            .selectable(false),
    );
}

fn node_items(
    ui: &mut egui::Ui,
    n: &telepad_core::cluster::SwarmNode,
    host: usize,
    out: &mut Vec<Msg>,
) {
    for a in NodeAction::ALL {
        if a.available(&n.availability) && ui.button(a.title()).clicked() {
            out.push(Msg::SwarmNode {
                host,
                node_id: n.id.clone(),
                action: a,
            });
            ui.close();
        }
    }
}

fn service_items(ui: &mut egui::Ui, name: &str, host: usize, out: &mut Vec<Msg>) {
    for a in ServiceAction::ALL {
        if ui.button(a.title()).clicked() {
            out.push(Msg::SwarmService {
                host,
                service: name.to_string(),
                action: a,
            });
            ui.close();
        }
    }
}

fn stack_items(ui: &mut egui::Ui, name: &str, host: usize, out: &mut Vec<Msg>) {
    let t = act_text(Act::Danger, &il(icons::TRASH, "Remove stack…"));
    if ui.button(t).clicked() {
        out.push(Msg::SwarmStackRemove {
            host,
            stack: name.to_string(),
        });
        ui.close();
    }
}

fn workload_items(
    ui: &mut egui::Ui,
    w: &telepad_core::cluster::Workload,
    host: usize,
    out: &mut Vec<Msg>,
) {
    for a in WorkloadAction::ALL {
        if a == WorkloadAction::Scale && !w.kind.scalable() {
            continue;
        }
        if ui.button(a.title()).clicked() {
            out.push(Msg::KubeWorkload {
                host,
                namespace: w.namespace.clone(),
                name: w.name.clone(),
                action: a,
            });
            ui.close();
        }
    }
}

fn swarm(
    ui: &mut egui::Ui,
    sw: &Swarm,
    host: usize,
    key: &str,
    d: &mut Drafts,
    out: &mut Vec<Msg>,
) {
    if !sw.manager {
        ui.label(
            RichText::new("Swarm worker — nodes and services can only be read on a manager").weak(),
        );
        return;
    }
    ui.label(format!(
        "Docker Swarm manager · {} node(s) · {} stack(s) · {} service(s)",
        sw.nodes.len(),
        sw.stacks.len(),
        sw.services.len()
    ));

    heading(ui, "Nodes");
    let cols = [
        Col::new("Node"),
        Col::new("Status"),
        Col::new("Availability"),
        Col::new("Manager"),
        Col::new("Engine"),
        Col::action(""),
    ];
    let cells: Vec<Vec<String>> = sw
        .nodes
        .iter()
        .map(|n| {
            let name = if n.is_self {
                format!("{} (this host)", n.hostname)
            } else {
                n.hostname.clone()
            };
            vec![
                name,
                n.status.clone(),
                n.availability.clone(),
                n.manager_status.clone(),
                n.engine.clone(),
                String::new(),
            ]
        })
        .collect();
    let title = format!("Swarm nodes ({key})");
    let spec = Spec::new(("swarm-nodes", key), &title, &cols);
    let st = table::state(d, format!("swarm-nodes:{key}"));
    let o = table::show(ui, &spec, st, &cells, |ui, ev| match ev {
        Ev::Cell { row, col } => {
            let n = &sw.nodes[row];
            match col {
                1 => colored(ui, ok_color(n.status == "Ready"), &n.status),
                4 => table::weak(ui, &n.engine),
                5 => menu(ui, |ui| node_items(ui, n, host, out)),
                c => table::text(ui, &cells[row][c]),
            }
        }
        Ev::Menu { row, .. } => node_items(ui, &sw.nodes[row], host, out),
        Ev::HeaderMenu => {}
    });
    out.extend(o.msgs);

    heading(ui, "Services");
    let cols = [
        Col::new("Service"),
        Col::new("Stack"),
        Col::new("Mode"),
        Col::new("Replicas"),
        Col::new("Image"),
        Col::new("Ports"),
        Col::action(""),
    ];
    let cells: Vec<Vec<String>> = sw
        .services
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.stack.clone(),
                s.mode.clone(),
                format!("{}/{}", s.running, s.desired),
                s.image.clone(),
                s.ports.clone(),
                String::new(),
            ]
        })
        .collect();
    let title = format!("Swarm services ({key})");
    let spec = Spec::new(("swarm-services", key), &title, &cols);
    let st = table::state(d, format!("swarm-services:{key}"));
    let o = table::show(ui, &spec, st, &cells, |ui, ev| match ev {
        Ev::Cell { row, col } => {
            let s = &sw.services[row];
            match col {
                1 | 4 => table::weak(ui, &cells[row][col]),
                3 => colored(ui, ok_color(!s.degraded()), &cells[row][3]),
                5 => table::mono(ui, &s.ports),
                6 => menu(ui, |ui| service_items(ui, &s.name, host, out)),
                c => table::text(ui, &cells[row][c]),
            }
        }
        Ev::Menu { row, .. } => service_items(ui, &sw.services[row].name, host, out),
        Ev::HeaderMenu => {}
    });
    out.extend(o.msgs);

    if !sw.stacks.is_empty() {
        heading(ui, "Stacks");
        let stacks: Vec<(String, u32)> = sw.stacks.iter().map(|(n, c)| (n.clone(), *c)).collect();
        let cols = [Col::new("Stack"), Col::new("Services"), Col::action("")];
        let cells: Vec<Vec<String>> = stacks
            .iter()
            .map(|(name, n)| vec![name.clone(), n.to_string(), String::new()])
            .collect();
        let title = format!("Swarm stacks ({key})");
        let spec = Spec::new(("swarm-stacks", key), &title, &cols);
        let st = table::state(d, format!("swarm-stacks:{key}"));
        let o = table::show(ui, &spec, st, &cells, |ui, ev| match ev {
            Ev::Cell { row, col } => match col {
                0 => table::text(ui, &il(icons::STACK, &cells[row][0])),
                1 => table::weak(ui, &format!("{} service(s)", cells[row][1])),
                _ => menu(ui, |ui| stack_items(ui, &stacks[row].0, host, out)),
            },
            Ev::Menu { row, .. } => stack_items(ui, &stacks[row].0, host, out),
            Ev::HeaderMenu => {}
        });
        out.extend(o.msgs);
    }

    if !sw.failing_tasks.is_empty() {
        heading(ui, "Tasks not running");
        let cols = [
            Col::new("Task"),
            Col::new("Node"),
            Col::new("State"),
            Col::new("Error"),
        ];
        let cells: Vec<Vec<String>> = sw
            .failing_tasks
            .iter()
            .map(|t| {
                vec![
                    t.name.clone(),
                    t.node.clone(),
                    t.state.clone(),
                    t.error.clone(),
                ]
            })
            .collect();
        let title = format!("Swarm tasks not running ({key})");
        let spec = Spec::new(("swarm-tasks", key), &title, &cols);
        let st = table::state(d, format!("swarm-tasks:{key}"));
        let o = table::show(ui, &spec, st, &cells, |ui, ev| {
            if let Ev::Cell { row, col } = ev {
                match col {
                    2 => colored(ui, pal().amber, &cells[row][2]),
                    3 => table::weak(ui, &cells[row][3]),
                    c => table::text(ui, &cells[row][c]),
                }
            }
        });
        out.extend(o.msgs);
    }
}

fn kube(ui: &mut egui::Ui, k: &Kube, host: usize, key: &str, d: &mut Drafts, out: &mut Vec<Msg>) {
    ui.label(format!(
        "Kubernetes via {} · {} node(s) · {} namespace(s) · {} workload(s) · {} pod(s)",
        k.tool,
        k.nodes.len(),
        k.namespaces().len(),
        k.workloads.len(),
        k.pods_total
    ));

    heading(ui, "Nodes");
    let cols = [Col::new("Node"), Col::new("Ready"), Col::new("Version")];
    let cells: Vec<Vec<String>> = k
        .nodes
        .iter()
        .map(|n| {
            vec![
                n.name.clone(),
                if n.ready { "Ready" } else { "Not ready" }.to_string(),
                n.version.clone(),
            ]
        })
        .collect();
    let title = format!("Kubernetes nodes ({key})");
    let spec = Spec::new(("kube-nodes", key), &title, &cols);
    let st = table::state(d, format!("kube-nodes:{key}"));
    let o = table::show(ui, &spec, st, &cells, |ui, ev| {
        if let Ev::Cell { row, col } = ev {
            match col {
                1 => colored(ui, ok_color(k.nodes[row].ready), &cells[row][1]),
                2 => table::weak(ui, &cells[row][2]),
                c => table::text(ui, &cells[row][c]),
            }
        }
    });
    out.extend(o.msgs);

    egui::CollapsingHeader::new(
        RichText::new(format!("Workloads ({})", k.workloads.len())).strong(),
    )
    .id_salt(("kube-workloads", key))
    .default_open(k.workloads.len() <= 60)
    .show(ui, |ui| {
        let cols = [
            Col::new("Namespace"),
            Col::new("Name"),
            Col::new("Kind"),
            Col::new("Ready"),
            Col::action(""),
        ];
        let cells: Vec<Vec<String>> = k
            .workloads
            .iter()
            .map(|w| {
                vec![
                    w.namespace.clone(),
                    w.name.clone(),
                    w.kind.singular().to_string(),
                    format!("{}/{}", w.ready, w.desired),
                    String::new(),
                ]
            })
            .collect();
        let title = format!("Kubernetes workloads ({key})");
        let spec = Spec::new(("kube-wl", key), &title, &cols);
        let st = table::state(d, format!("kube-wl:{key}"));
        let o = table::show(ui, &spec, st, &cells, |ui, ev| match ev {
            Ev::Cell { row, col } => {
                let w = &k.workloads[row];
                match col {
                    0 => table::weak(ui, &w.namespace),
                    3 => colored(ui, ok_color(!w.degraded()), &cells[row][3]),
                    4 => menu(ui, |ui| workload_items(ui, w, host, out)),
                    c => table::text(ui, &cells[row][c]),
                }
            }
            Ev::Menu { row, .. } => workload_items(ui, &k.workloads[row], host, out),
            Ev::HeaderMenu => {}
        });
        out.extend(o.msgs);
    });

    if !k.problem_pods.is_empty() {
        heading(ui, "Problem pods");
        let cols = [
            Col::new("Namespace"),
            Col::new("Pod"),
            Col::new("Phase"),
            Col::new("Reason"),
            Col::new("Restarts"),
        ];
        let cells: Vec<Vec<String>> = k
            .problem_pods
            .iter()
            .map(|p| {
                vec![
                    p.namespace.clone(),
                    p.name.clone(),
                    p.phase.clone(),
                    p.reason.clone(),
                    p.restarts.to_string(),
                ]
            })
            .collect();
        let title = format!("Kubernetes problem pods ({key})");
        let spec = Spec::new(("kube-pods", key), &title, &cols);
        let st = table::state(d, format!("kube-pods:{key}"));
        let o = table::show(ui, &spec, st, &cells, |ui, ev| {
            if let Ev::Cell { row, col } = ev {
                match col {
                    0 => table::weak(ui, &cells[row][0]),
                    3 => colored(ui, pal().red, &cells[row][3]),
                    c => table::text(ui, &cells[row][c]),
                }
            }
        });
        out.extend(o.msgs);
    }
}
