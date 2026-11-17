//! "Ports in use" section of the host detail.

use eframe::egui::{self, RichText};

use telepad_core::model::HostReport;

use crate::widgets::NGINX_COLOR;

pub fn ports_in_use(ui: &mut egui::Ui, r: &HostReport, nginx_only: &mut bool) {
    let usage = r.port_usage();
    if usage.is_empty() {
        ui.label(RichText::new("No listening ports reported (ss/netstat not available)").weak());
        return;
    }
    let mut nginx_ports: Vec<u16> = usage
        .iter()
        .filter(|u| u.nginx && u.proto == "tcp")
        .map(|u| u.port)
        .collect();
    nginx_ports.dedup();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("nginx host ports:")
                .strong()
                .color(NGINX_COLOR),
        );
        if nginx_ports.is_empty() {
            ui.label(RichText::new("none").weak());
        } else {
            let list = nginx_ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(RichText::new(&list).monospace().color(NGINX_COLOR));
            if ui.small_button("copy").clicked() {
                ui.ctx().copy_text(list);
            }
        }
    });
    let total_tcp = usage.iter().filter(|u| u.proto == "tcp").count();
    ui.horizontal(|ui| {
        ui.label(format!(
            "{total_tcp} TCP · {} UDP ports in use",
            usage.len() - total_tcp
        ));
        ui.checkbox(nginx_only, "nginx only");
    });
    if !r.listeners.iter().any(|l| !l.process.is_empty()) {
        ui.label(
            RichText::new(
                "Process names need passwordless sudo on this host; only Docker-published ports show an owner.",
            )
            .small()
            .weak(),
        );
    }
    egui::Grid::new("ports")
        .striped(true)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for t in ["Port", "Proto", "Bind", "Used by", "Stack", "→ Container"] {
                ui.label(RichText::new(t).strong());
            }
            ui.end_row();
            for u in usage.iter().filter(|u| !*nginx_only || u.nginx) {
                let color = u.nginx.then_some(NGINX_COLOR);
                let txt = |t: String| {
                    let rt = RichText::new(t).monospace();
                    match color {
                        Some(c) => rt.color(c),
                        None => rt,
                    }
                };
                ui.label(txt(u.port.to_string()).strong());
                ui.label(txt(u.proto.clone()));
                ui.label(txt(u.addresses.join(", ")));
                ui.label(txt(if u.owner.is_empty() {
                    "—".into()
                } else {
                    u.owner.clone()
                }));
                ui.label(txt(u.stack.clone()));
                ui.label(txt(u
                    .container_port
                    .map_or(String::new(), |p| p.to_string())));
                ui.end_row();
            }
        });
}
