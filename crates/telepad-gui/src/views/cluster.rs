//! Docker Swarm / Kubernetes section of the host detail.

use eframe::egui::{self, RichText};
use telepad_app::Msg;
use telepad_core::cluster::{Kube, Swarm};
use telepad_core::docker_actions::{NodeAction, ServiceAction, WorkloadAction};
use telepad_core::model::HostReport;

use crate::icons::{self, label as il};
use crate::widgets::*;

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
    shown_by: Option<&str>,
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
        swarm(ui, sw, host, out);
    }
    if let Some(e) = &r.cluster.kube_error {
        ui.label(RichText::new(format!("Kubernetes: {e}")).weak());
    }
    if let Some(k) = &r.cluster.kube {
        kube(ui, k, host, out);
    }
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

fn swarm(ui: &mut egui::Ui, sw: &Swarm, host: usize, out: &mut Vec<Msg>) {
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
    egui::Grid::new("swarm-nodes")
        .striped(true)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            for t in ["Node", "Status", "Availability", "Manager", "Engine", ""] {
                ui.label(RichText::new(t).strong());
            }
            ui.end_row();
            for n in &sw.nodes {
                let name = if n.is_self {
                    format!("{} (this host)", n.hostname)
                } else {
                    n.hostname.clone()
                };
                ui.label(name);
                ui.colored_label(ok_color(n.status == "Ready"), &n.status);
                ui.label(&n.availability);
                ui.label(&n.manager_status);
                ui.label(RichText::new(&n.engine).weak());
                menu(ui, |ui| {
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
                });
                ui.end_row();
            }
        });

    heading(ui, "Services");
    egui::Grid::new("swarm-services")
        .striped(true)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            for t in ["Service", "Stack", "Mode", "Replicas", "Image", "Ports", ""] {
                ui.label(RichText::new(t).strong());
            }
            ui.end_row();
            for s in &sw.services {
                ui.label(&s.name);
                ui.label(RichText::new(&s.stack).weak());
                ui.label(&s.mode);
                ui.colored_label(
                    ok_color(!s.degraded()),
                    format!("{}/{}", s.running, s.desired),
                );
                ui.label(RichText::new(&s.image).weak());
                ui.label(RichText::new(&s.ports).monospace());
                menu(ui, |ui| {
                    for a in ServiceAction::ALL {
                        if ui.button(a.title()).clicked() {
                            out.push(Msg::SwarmService {
                                host,
                                service: s.name.clone(),
                                action: a,
                            });
                            ui.close();
                        }
                    }
                });
                ui.end_row();
            }
        });

    if !sw.stacks.is_empty() {
        heading(ui, "Stacks");
        egui::Grid::new("swarm-stacks")
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                for (name, n) in &sw.stacks {
                    ui.label(il(icons::STACK, name));
                    ui.label(RichText::new(format!("{n} service(s)")).weak());
                    menu(ui, |ui| {
                        let t = act_text(Act::Danger, &il(icons::TRASH, "Remove stack…"));
                        if ui.button(t).clicked() {
                            out.push(Msg::SwarmStackRemove {
                                host,
                                stack: name.clone(),
                            });
                            ui.close();
                        }
                    });
                    ui.end_row();
                }
            });
    }

    if !sw.failing_tasks.is_empty() {
        heading(ui, "Tasks not running");
        egui::Grid::new("swarm-tasks")
            .striped(true)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                for t in ["Task", "Node", "State", "Error"] {
                    ui.label(RichText::new(t).strong());
                }
                ui.end_row();
                for t in &sw.failing_tasks {
                    ui.label(&t.name);
                    ui.label(&t.node);
                    ui.colored_label(pal().amber, &t.state);
                    ui.label(RichText::new(&t.error).weak());
                    ui.end_row();
                }
            });
    }
}

fn kube(ui: &mut egui::Ui, k: &Kube, host: usize, out: &mut Vec<Msg>) {
    ui.label(format!(
        "Kubernetes via {} · {} node(s) · {} namespace(s) · {} workload(s) · {} pod(s)",
        k.tool,
        k.nodes.len(),
        k.namespaces().len(),
        k.workloads.len(),
        k.pods_total
    ));

    heading(ui, "Nodes");
    egui::Grid::new("kube-nodes")
        .striped(true)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            for t in ["Node", "Ready", "Version"] {
                ui.label(RichText::new(t).strong());
            }
            ui.end_row();
            for n in &k.nodes {
                ui.label(&n.name);
                ui.colored_label(
                    ok_color(n.ready),
                    if n.ready { "Ready" } else { "Not ready" },
                );
                ui.label(RichText::new(&n.version).weak());
                ui.end_row();
            }
        });

    egui::CollapsingHeader::new(
        RichText::new(format!("Workloads ({})", k.workloads.len())).strong(),
    )
    .id_salt("kube-workloads")
    .default_open(k.workloads.len() <= 60)
    .show(ui, |ui| {
        egui::Grid::new("kube-wl")
            .striped(true)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                for t in ["Namespace", "Name", "Kind", "Ready", ""] {
                    ui.label(RichText::new(t).strong());
                }
                ui.end_row();
                for w in &k.workloads {
                    ui.label(RichText::new(&w.namespace).weak());
                    ui.label(&w.name);
                    ui.label(w.kind.singular());
                    ui.colored_label(
                        ok_color(!w.degraded()),
                        format!("{}/{}", w.ready, w.desired),
                    );
                    menu(ui, |ui| {
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
                    });
                    ui.end_row();
                }
            });
    });

    if !k.problem_pods.is_empty() {
        heading(ui, "Problem pods");
        egui::Grid::new("kube-pods")
            .striped(true)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                for t in ["Namespace", "Pod", "Phase", "Reason", "Restarts"] {
                    ui.label(RichText::new(t).strong());
                }
                ui.end_row();
                for p in &k.problem_pods {
                    ui.label(RichText::new(&p.namespace).weak());
                    ui.label(&p.name);
                    ui.label(&p.phase);
                    ui.colored_label(pal().red, &p.reason);
                    ui.label(p.restarts.to_string());
                    ui.end_row();
                }
            });
    }
}
