//! Host detail page.

use eframe::egui::{self, RichText};
use telepad_app::Msg;

use crate::icons::{self, label as il};
use crate::widgets::*;
use telepad_app::HostState;
use telepad_core::config::Access;
use telepad_core::model::{human_kb, human_uptime};

use super::docker::*;
use super::ports::*;
use super::sensors::*;

pub fn detail(
    ui: &mut egui::Ui,
    h: &HostState,
    i: usize,
    nginx_only: &mut bool,
    out: &mut Vec<Msg>,
) {
    ui.horizontal(|ui| {
        status_dot(ui, h);
        icons::os_logo(ui, host_os(h), 26.0);
        ui.heading(&h.entry.alias);
        ui.label(RichText::new(h.entry.target()).monospace());
        if h.entry.port != h.entry.access.default_port() {
            ui.label(format!("port {}", h.entry.port));
        }
        ui.separator();
        let rdp = h.entry.access == Access::Rdp;
        let open = if rdp { "Open RDP" } else { "Open SSH" };
        if ui.button(il(connect_icon(&h.entry), open)).clicked() {
            out.push(Msg::Ssh(i));
        }
        if !rdp && ui.button(il(icons::COPY, "Copy ssh command")).clicked() {
            out.push(Msg::CopySsh(i));
        }
        if ui.button(il(icons::EDIT, "Edit")).clicked() {
            out.push(Msg::Edit(i));
        }
        if ui
            .button(il(icons::CLONE, "Clone"))
            .on_hover_text("New host with this User, Port and IdentityFile")
            .clicked()
        {
            out.push(Msg::Clone(i));
        }
        if ui.button(il(icons::TRASH, "Delete")).clicked() {
            out.push(Msg::Delete(i));
        }
        if ui
            .add_enabled(!h.loading, egui::Button::new(il(icons::REFRESH, "Refresh")))
            .clicked()
        {
            out.push(Msg::Refresh(i));
        }
        if h.loading {
            ui.spinner();
        }
    });
    if !h.entry.note.is_empty() {
        ui.label(RichText::new(&h.entry.note).size(15.0).italics());
    }
    if h.entry.unmonitored {
        ui.label(RichText::new("Not monitored — enable it in Edit").weak());
    }
    ui.label(
        RichText::new(format!(
            "updated {}{}",
            ago(h.updated),
            h.elapsed.map_or(String::new(), |e| format!(
                " · took {:.1}s",
                e.as_secs_f32()
            ))
        ))
        .weak(),
    );
    if let Some(e) = &h.error {
        ui.colored_label(usage_color(95.0), format!("Last check failed: {e}"));
    }
    if h.entry.access == Access::Rdp {
        section(ui, &il(icons::WINDOWS, "Windows (Remote Desktop only)"));
        ui.label(
            "This host has no SSH, so there are no metrics. The app checks that the RDP \
             port accepts connections and opens Remote Desktop (mstsc) on request.",
        );
        ui.add_space(4.0);
        match (&h.report, &h.error) {
            (_, Some(e)) => ui.colored_label(usage_color(95.0), e),
            (Some(r), None) => {
                let ok = |b: bool| {
                    if b {
                        usage_color(0.0)
                    } else {
                        usage_color(95.0)
                    }
                };
                egui::Grid::new("rdp-status")
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Ping").strong());
                        ui.colored_label(
                            ok(r.ping_ms.is_some()),
                            r.ping_ms.map_or(
                                "no reply (ICMP may be blocked by the firewall)".into(),
                                |ms| format!("{ms:.0} ms"),
                            ),
                        );
                        ui.end_row();
                        ui.label(RichText::new(format!("RDP {}", h.entry.port)).strong());
                        ui.colored_label(
                            ok(r.rdp_open),
                            if r.rdp_open {
                                "open"
                            } else {
                                "closed / filtered"
                            },
                        );
                        ui.end_row();
                    });
                ui.label("")
            }
            (None, None) => ui.label("Checking…"),
        };
        return;
    }
    let Some(r) = &h.report else {
        if h.error.is_none() {
            ui.label("Waiting for first result…");
        }
        return;
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(ui, &il(icons::INFO, "System"));
            egui::Grid::new("sys").spacing([16.0, 4.0]).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(RichText::new(k).strong());
                    ui.label(v);
                    ui.end_row();
                };
                row("Hostname", r.hostname.clone());
                row("OS", r.os_pretty.clone());
                row("Type", format!("{} {} ({})", r.os_type, r.kernel, r.arch));
                row("Uptime", r.uptime_secs.map_or("?".into(), human_uptime));
                row(
                    "CPU",
                    format!(
                        "{} · {} cores",
                        if r.cpu_model.is_empty() {
                            "?"
                        } else {
                            &r.cpu_model
                        },
                        r.cpu_cores.map_or("?".into(), |c| c.to_string())
                    ),
                );
                if let Some(l) = r.load_avg {
                    row("Load avg", format!("{:.2}  {:.2}  {:.2}", l[0], l[1], l[2]));
                }
            });

            section(ui, &il(icons::GAUGE, "Usage"));
            egui::Grid::new("usage")
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    if let Some(c) = r.cpu_usage_pct {
                        ui.label(RichText::new("CPU").strong());
                        bar(ui, c, format!("{c:.1}%"), 360.0);
                        ui.end_row();
                    }
                    if let Some(m) = &r.memory {
                        ui.label(RichText::new("Memory").strong());
                        bar(
                            ui,
                            m.used_pct(),
                            format!(
                                "{} used of {} ({:.1}%)",
                                human_kb(m.used_kb()),
                                human_kb(m.total_kb),
                                m.used_pct()
                            ),
                            360.0,
                        );
                        ui.end_row();
                        if m.swap_total_kb > 0 {
                            ui.label(RichText::new("Swap").strong());
                            bar(
                                ui,
                                m.swap_used_pct(),
                                format!(
                                    "{} used of {}",
                                    human_kb(m.swap_used_kb()),
                                    human_kb(m.swap_total_kb)
                                ),
                                360.0,
                            );
                            ui.end_row();
                        }
                    }
                });

            section(ui, &il(icons::DISKS, "Disks"));
            if r.disks.is_empty() {
                ui.label(RichText::new("No disk data").weak());
            }
            egui::Grid::new("disks")
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    for t in ["Mount", "Filesystem", "Usage", "Free"] {
                        ui.label(RichText::new(t).strong());
                    }
                    ui.end_row();
                    for d in &r.disks {
                        ui.label(RichText::new(&d.mount).monospace());
                        ui.label(RichText::new(&d.filesystem).weak());
                        bar(
                            ui,
                            d.used_pct(),
                            format!(
                                "{} / {} ({:.1}%)",
                                human_kb(d.used_kb),
                                human_kb(d.total_kb),
                                d.used_pct()
                            ),
                            300.0,
                        );
                        ui.label(human_kb(d.available_kb));
                        ui.end_row();
                    }
                });

            section(ui, &il(icons::NETWORK, "Network"));
            egui::Grid::new("net")
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for a in &r.addresses {
                        ui.label(RichText::new(&a.interface).strong());
                        ui.label(&a.family);
                        ui.label(RichText::new(&a.address).monospace());
                        ui.end_row();
                    }
                });

            section(ui, &il(icons::THERMOMETER, "Sensors"));
            sensors(ui, r, i, out);

            section(ui, &il(icons::CONTAINER, "Docker"));
            match &r.docker.unavailable {
                Some(why) => {
                    ui.label(RichText::new(format!("Docker: {why}")).weak());
                }
                None => {
                    docker_summary(ui, r, &h.entry.hostname);
                    if r.docker.containers.is_empty() {
                        ui.label(RichText::new("No containers").weak());
                    } else {
                        docker_stacks(ui, r);
                    }
                }
            }

            section(ui, &il(icons::PLUGS, "Ports in use"));
            ports_in_use(ui, r, nginx_only);
            ui.add_space(20.0);
        });
}
