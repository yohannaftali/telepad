//! Overview table: one row per host.

use eframe::egui::{self, RichText};
use telepad_app::Msg;

use crate::icons::{self, label as il};
use crate::widgets::*;
use telepad_app::{Model, matches_filter};
use telepad_core::config::Access;
use telepad_core::model::{PortainerRole, SensorKind, human_kb};

pub fn overview(ui: &mut egui::Ui, m: &Model, filter: &str, out: &mut Vec<Msg>) {
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("overview")
                .striped(true)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    for title in [
                        "",
                        "Host",
                        "Description",
                        "Address",
                        "OS",
                        "CPU",
                        "Memory",
                        "Fullest disk",
                        "Temp",
                        "Docker",
                        "Updated",
                        "",
                    ] {
                        ui.label(RichText::new(title).strong());
                    }
                    ui.end_row();
                    for (i, h) in m.hosts.iter().enumerate() {
                        if !matches_filter(filter, h) {
                            continue;
                        }
                        status_dot(ui, h);
                        if ui.link(RichText::new(&h.entry.alias).strong()).clicked() {
                            out.push(Msg::Select(Some(i)));
                        }
                        ui.label(RichText::new(&h.entry.note).italics());
                        ui.label(&h.entry.hostname);
                        match &h.report {
                            Some(r) => {
                                ui.horizontal(|ui| {
                                    icons::os_logo(ui, host_os(h), 16.0);
                                    ui.label(&r.os_pretty);
                                });
                                match r.cpu_usage_pct {
                                    Some(c) => bar(ui, c, format!("{c:.0}%"), 90.0),
                                    None => {
                                        ui.label("—");
                                    }
                                }
                                match &r.memory {
                                    Some(m) => bar(
                                        ui,
                                        m.used_pct(),
                                        format!(
                                            "{} / {}",
                                            human_kb(m.used_kb()),
                                            human_kb(m.total_kb)
                                        ),
                                        150.0,
                                    ),
                                    None => {
                                        ui.label("—");
                                    }
                                }
                                match r
                                    .disks
                                    .iter()
                                    .max_by(|a, b| a.used_pct().total_cmp(&b.used_pct()))
                                {
                                    Some(d) => bar(
                                        ui,
                                        d.used_pct(),
                                        format!("{} {:.0}%", d.mount, d.used_pct()),
                                        150.0,
                                    ),
                                    None => {
                                        ui.label("—");
                                    }
                                }
                                let t = r
                                    .sensors
                                    .iter()
                                    .filter(|s| s.kind == SensorKind::Temperature)
                                    .map(|s| s.value)
                                    .fold(None, |m: Option<f32>, v| {
                                        Some(m.map_or(v, |m| m.max(v)))
                                    });
                                match t {
                                    Some(t) => ui.colored_label(temp_color(t), format!("{t:.0}°C")),
                                    None => ui.label(RichText::new("n/a").weak()),
                                };
                                match &r.docker.unavailable {
                                    Some(_) => ui.label(RichText::new("—").weak()),
                                    None => {
                                        let run = r
                                            .docker
                                            .containers
                                            .iter()
                                            .filter(|c| c.running())
                                            .count();
                                        let role = match r.docker.portainer().first() {
                                            Some((PortainerRole::Server, _)) => " · Portainer",
                                            Some((PortainerRole::Agent, _)) => " · agent",
                                            None => "",
                                        };
                                        let rt = if r.docker.is_podman() {
                                            " (podman)"
                                        } else {
                                            ""
                                        };
                                        ui.label(format!(
                                            "{run}/{} running{rt}{role}",
                                            r.docker.containers.len()
                                        ))
                                    }
                                };
                            }
                            None => {
                                let msg = if let Some(e) = &h.error {
                                    RichText::new(e).color(usage_color(95.0))
                                } else if h.loading {
                                    RichText::new("loading…").weak()
                                } else {
                                    RichText::new("queued").weak()
                                };
                                ui.label(msg);
                                for _ in 0..5 {
                                    ui.label("");
                                }
                            }
                        }
                        ui.label(RichText::new(ago(h.updated)).weak());
                        let (label, tip) = match h.entry.access {
                            Access::Ssh => ("SSH", "Open an SSH session in a terminal"),
                            Access::Rdp => ("RDP", "Open Remote Desktop"),
                        };
                        if ui
                            .small_button(il(connect_icon(&h.entry), label))
                            .on_hover_text(tip)
                            .clicked()
                        {
                            out.push(Msg::Ssh(i));
                        }
                        ui.end_row();
                    }
                });
        });
}
