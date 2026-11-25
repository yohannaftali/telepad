//! "Ports in use" section of the host detail.

use eframe::egui::{self, RichText};

use telepad_core::model::HostReport;

use crate::table::{self, Col, Ev, Spec};
use crate::widgets::nginx_color;

pub fn ports_in_use(
    ui: &mut egui::Ui,
    r: &HostReport,
    alias: &str,
    d: &mut telepad_app::Drafts,
    out: &mut Vec<telepad_app::Msg>,
) {
    let nginx_only = &mut d.ports_nginx_only;
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
                .color(nginx_color()),
        );
        if nginx_ports.is_empty() {
            ui.label(RichText::new("none").weak());
        } else {
            let list = nginx_ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(RichText::new(&list).monospace().color(nginx_color()));
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
    let shown: Vec<_> = usage.iter().filter(|u| !*nginx_only || u.nginx).collect();
    let cols = [
        Col::new("Port"),
        Col::new("Proto"),
        Col::new("Bind"),
        Col::new("Used by"),
        Col::new("Stack"),
        Col::new("→ Container"),
    ];
    let cells: Vec<Vec<String>> = shown
        .iter()
        .map(|u| {
            vec![
                u.port.to_string(),
                u.proto.clone(),
                u.addresses.join(", "),
                u.owner.clone(),
                u.stack.clone(),
                u.container_port.map_or(String::new(), |p| p.to_string()),
            ]
        })
        .collect();
    let title = format!("Ports in use on {alias}");
    let st = table::state(d, format!("ports:{alias}"));
    let o = table::show(
        ui,
        &Spec::new(("ports", alias), &title, &cols),
        st,
        &cells,
        |ui, ev| {
            if let Ev::Cell { row, col } = ev {
                let u = shown[row];
                let t = if col == 3 && u.owner.is_empty() {
                    "—"
                } else {
                    cells[row][col].as_str()
                };
                let mut rt = RichText::new(t).monospace();
                if u.nginx {
                    rt = rt.color(nginx_color());
                }
                if col == 0 {
                    rt = rt.strong();
                }
                ui.add(egui::Label::new(rt).truncate().selectable(false));
            }
        },
    );
    out.extend(o.msgs);
}
