//! Docker section of the host detail: summary, stacks and containers.

use eframe::egui::{self, Color32, RichText};

use crate::icons;
use crate::widgets::*;
use telepad_core::model::{Container, HostReport, Manager, PortainerRole};

pub fn host_ports(c: &Container) -> String {
    let mut v: Vec<String> = Vec::new();
    for p in &c.published {
        let s = p.host_port.to_string();
        if !v.contains(&s) {
            v.push(s);
        }
    }
    v.join(", ")
}

/// Runtime and how this host's containers are managed (Portainer server / agent / plain).
pub fn docker_summary(ui: &mut egui::Ui, r: &HostReport, host_ip: &str) {
    egui::Grid::new("docker-summary").spacing([16.0, 4.0]).show(ui, |ui| {
        ui.label(RichText::new("Runtime").strong());
        let rt = if r.docker.runtime.is_empty() { "unknown".to_string() } else { r.docker.runtime.clone() };
        ui.label(rt);
        ui.end_row();

        ui.label(RichText::new("Management").strong());
        ui.vertical(|ui| {
            let portainer = r.docker.portainer();
            if portainer.is_empty() {
                ui.label("No Portainer on this host — stacks run via docker compose or docker run");
            }
            for (role, c) in portainer {
                let ports = host_ports(c);
                let text = match role {
                    PortainerRole::Server => format!(
                        "Portainer server ({}) — manages this host through the local socket{}{}",
                        c.name,
                        if c.mounts_socket { " /var/run/docker.sock" } else { "" },
                        if ports.is_empty() { String::new() } else { format!(" · UI on host port {ports}") }
                    ),
                    PortainerRole::Agent => format!(
                        "Portainer agent ({}) — managed by a remote Portainer server via {}:{}",
                        c.name,
                        host_ip,
                        if ports.is_empty() { "9001".to_string() } else { ports }
                    ),
                };
                ui.label(RichText::new(text).color(PORTAINER_COLOR));
            }
            let (mut pt, mut cp, mut st) = (0, 0, 0);
            for (_, cs) in r.docker.stacks() {
                match r.docker.stack_manager(&cs) {
                    Manager::Portainer { .. } => pt += 1,
                    Manager::Compose { .. } => cp += 1,
                    Manager::Standalone => st += cs.len(),
                }
            }
            ui.label(
                RichText::new(format!(
                    "{pt} Portainer stack(s) · {cp} docker compose stack(s) · {st} standalone container(s)"
                ))
                .weak(),
            );
        });
        ui.end_row();
    });
    ui.add_space(6.0);
}

pub fn manager_text(m: &Manager) -> (String, Color32) {
    match m {
        Manager::Portainer { stack_id } => {
            (format!("Portainer stack #{stack_id}"), PORTAINER_COLOR)
        }
        Manager::Compose { dir, files } => {
            let file = files.rsplit(['/', '\\']).next().unwrap_or(files);
            let f = if file.is_empty() {
                String::new()
            } else {
                format!(" ({file})")
            };
            (
                format!("docker compose in {dir}{f}"),
                Color32::from_rgb(0xa0, 0x7c, 0xd8),
            )
        }
        Manager::Standalone => ("docker run (no compose)".into(), Color32::GRAY),
    }
}

/// `2491→443/tcp` for published ports (IPv4/IPv6 duplicates merged), then internal ones.
pub fn container_ports(c: &Container) -> (String, String) {
    let mut published: Vec<String> = Vec::new();
    for p in &c.published {
        let s = format!("{}→{}/{}", p.host_port, p.container_port, p.proto);
        let s = if p.host_ip == "127.0.0.1" || p.host_ip == "::1" {
            format!("{s} (local)")
        } else {
            s
        };
        if !published.contains(&s) {
            published.push(s);
        }
    }
    let internal: Vec<&str> = c
        .ports
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty() && !x.contains("->"))
        .collect();
    (published.join("\n"), internal.join(", "))
}

pub fn docker_stacks(ui: &mut egui::Ui, r: &HostReport) {
    for (stack, containers) in r.docker.stacks() {
        let running = containers.iter().filter(|c| c.running()).count();
        let name = if stack.is_empty() {
            "(standalone)"
        } else {
            stack
        };
        let color = if running == containers.len() {
            usage_color(0.0)
        } else if running == 0 {
            Color32::GRAY
        } else {
            usage_color(80.0)
        };
        let (mtext, mcolor) = manager_text(&r.docker.stack_manager(&containers));
        let header = ui
            .horizontal(|ui| {
                let open_id = ui.make_persistent_id(("stack-open", stack));
                let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(open_id, true));
                let arrow = if open {
                    icons::CARET_DOWN
                } else {
                    icons::CARET_RIGHT
                };
                let resp = ui.add(
                    egui::Button::new(
                        RichText::new(format!(
                            "{arrow} {} {name}   {running}/{} running",
                            icons::STACK,
                            containers.len()
                        ))
                        .strong()
                        .color(color),
                    )
                    .frame(false),
                );
                ui.label(RichText::new(mtext).color(mcolor));
                if resp.clicked() {
                    open = !open;
                    ui.data_mut(|d| d.insert_persisted(open_id, open));
                }
                open
            })
            .inner;
        if !header {
            continue;
        }
        ui.indent(("stack-body", stack), |ui| {
            egui::Grid::new(("docker", stack))
                .striped(true)
                .spacing([14.0, 5.0])
                .show(ui, |ui| {
                    for t in [
                        "",
                        "Container",
                        "Image",
                        "Status",
                        "IP address",
                        "Host port → container",
                        "Internal",
                    ] {
                        ui.label(RichText::new(t).strong());
                    }
                    ui.end_row();
                    for c in containers {
                        let dot = if c.running() {
                            usage_color(0.0)
                        } else {
                            Color32::GRAY
                        };
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.5, dot);
                        let mut n = RichText::new(&c.name).strong();
                        if c.is_nginx() {
                            n = n.color(NGINX_COLOR);
                        } else if c.portainer_role().is_some() {
                            n = n.color(PORTAINER_COLOR);
                        }
                        let resp = ui.label(n);
                        if c.mounts_socket {
                            resp.on_hover_text("Mounts the Docker socket");
                        }
                        ui.label(RichText::new(&c.image).weak());
                        ui.label(&c.status);
                        let ips: Vec<String> = c
                            .networks
                            .iter()
                            .filter(|(_, ip)| !ip.is_empty())
                            .map(|(n, ip)| format!("{ip} ({n})"))
                            .collect();
                        ui.label(RichText::new(ips.join("\n")).monospace());
                        let (published, internal) = container_ports(c);
                        ui.label(RichText::new(published).monospace());
                        ui.label(RichText::new(internal).monospace().weak());
                        ui.end_row();
                    }
                });
        });
        ui.add_space(4.0);
    }
}
