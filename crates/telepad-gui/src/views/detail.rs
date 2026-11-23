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
    cluster_shown_by: Option<&str>,
    paused: bool,
    nginx_only: &mut bool,
    out: &mut Vec<Msg>,
) {
    // Two wrapping rows: on a narrow window the buttons move to the next line
    // instead of running off the right edge.
    ui.horizontal_wrapped(|ui| {
        status_dot(ui, h);
        icons::os_logo(ui, host_os(h), 26.0);
        ui.heading(&h.entry.alias);
        ui.label(RichText::new(h.entry.target()).monospace());
        if h.entry.port != h.entry.access.default_port() {
            ui.label(format!("port {}", h.entry.port));
        }
    });
    ui.horizontal_wrapped(|ui| {
        let rdp = h.entry.access == Access::Rdp;
        let open = if rdp { "Open RDP" } else { "Open SSH" };
        if act_button(ui, true, Act::Caution, &il(connect_icon(&h.entry), open)).clicked() {
            out.push(Msg::Ssh(i));
        }
        if !rdp
            && act_button(ui, true, Act::Edit, &il(icons::FOLDER, "Open files…"))
                .on_hover_text("This computer and the host side by side: browse and copy files")
                .clicked()
        {
            out.push(Msg::Files(telepad_app::files::FmMsg::Open(
                h.entry.alias.clone(),
            )));
        }
        if !rdp && ui.button(il(icons::COPY, "Copy ssh command")).clicked() {
            out.push(Msg::CopySsh(i));
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
        // Pause this host's automatic checks (#122); Refresh still reads it.
        if !h.entry.unmonitored {
            let (icon, text, tip) = if paused {
                (
                    icons::PLAY,
                    "Resume updates",
                    "Check this host by itself again",
                )
            } else {
                (
                    icons::PAUSE,
                    "Pause updates",
                    "Stop checking this host by itself (no SSH, no ping); Refresh still reads it",
                )
            };
            if ui.button(il(icon, text)).on_hover_text(tip).clicked() {
                out.push(Msg::PauseHosts {
                    aliases: vec![h.entry.alias.clone()],
                    paused: !paused,
                });
            }
        }
        ui.separator();
        if act_button(ui, true, Act::Edit, &il(icons::EDIT, "Edit")).clicked() {
            out.push(Msg::Edit(i));
        }
        if ui
            .button(il(icons::CLONE, "Clone"))
            .on_hover_text("New host with this User, Port and IdentityFile")
            .clicked()
        {
            out.push(Msg::Clone(i));
        }
        if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Delete")).clicked() {
            out.push(Msg::Delete(i));
        }
        // Databases installed on the host itself; container databases are in
        // each container's menu.
        if let Some(r) = &h.report {
            let local = telepad_core::db::detect::host_targets(&h.entry.alias, r);
            if !local.is_empty() {
                ui.separator();
            }
            for t in local {
                if ui
                    .button(il(
                        icons::DATABASE,
                        &format!("Open {} :{}", t.engine.title(), t.port),
                    ))
                    .on_hover_text("Tables, data and queries through this host's SSH login")
                    .clicked()
                {
                    out.push(Msg::Db(telepad_app::db::DbMsg::Open(t)));
                }
            }
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
    if h.shown().is_some() && h.entry.access == Access::Ssh {
        ui.label(
            RichText::new(format!(
                "OS and CPU info read {} · addresses, ports and updates read {}",
                ago_seen(h.static_at),
                ago_seen(h.slow_at)
            ))
            .weak()
            .small(),
        )
        .on_hover_text(
            "Live metrics are read every round; OS / CPU info and addresses, \
             listening ports and pending updates less often, and kept in update.db \
             across restarts (Update Strategy in the top-right menu). Refresh reads everything now.",
        );
    }
    if let Some(ping) = h.ping {
        let text = match ping {
            Some(ms) => format!("ping {ms:.0} ms · {}", ago(h.ping_at)),
            None => format!("no ping reply · {}", ago(h.ping_at)),
        };
        ui.label(RichText::new(text).weak().small());
    }
    if h.report.is_none()
        && let Some(k) = &h.known
    {
        let mut parts = vec![k.os_pretty.clone(), k.kernel.clone()];
        if let Some(n) = k.maintenance.updates {
            parts.push(format!("{n} update(s) pending"));
        }
        parts.retain(|p| !p.is_empty());
        ui.label(RichText::new(format!("Last known: {}", parts.join(" · "))).weak())
            .on_hover_text(
                "Saved from an earlier run; read again from the host on the next collection",
            );
    }
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

            maintenance(ui, &r.maintenance);

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
                        docker_stacks(ui, r, i, &h.entry.alias, host_address(&h.entry), out);
                    }
                }
            }

            if super::cluster::has_cluster(r) {
                section(ui, &il(icons::STACK, "Cluster"));
                super::cluster::cluster(ui, r, i, cluster_shown_by, out);
            }

            section(ui, &il(icons::PLUGS, "Ports in use"));
            ports_in_use(ui, r, nginx_only);
            ui.add_space(20.0);
        });
}

/// Reboot, pending updates and end of vendor support, with what to do.
fn maintenance(ui: &mut egui::Ui, m: &telepad_core::model::Maintenance) {
    use telepad_core::eol::Support;
    section(ui, &il(icons::WARNING, "Maintenance"));
    let unknown = || RichText::new("unknown").weak();
    egui::Grid::new("maint")
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Reboot").strong());
            match m.reboot_required {
                Some(true) => {
                    let mut t = "required".to_string();
                    if !m.reboot_packages.is_empty() {
                        t.push_str(&format!(" ({})", m.reboot_packages.join(", ")));
                    }
                    ui.colored_label(usage_color(80.0), t);
                }
                Some(false) => {
                    ui.label("not needed");
                }
                None => {
                    ui.label(unknown());
                }
            }
            ui.end_row();
            ui.label(RichText::new("Updates").strong());
            match (m.updates, m.security_updates) {
                (Some(0), _) => {
                    ui.label("up to date (local package cache)");
                }
                (Some(n), Some(s)) if s > 0 => {
                    ui.colored_label(usage_color(95.0), format!("{n} pending, {s} security"));
                }
                (Some(n), _) => {
                    ui.label(format!("{n} pending"));
                }
                (None, _) => {
                    ui.label(unknown());
                }
            }
            ui.end_row();
            ui.label(RichText::new("OS support").strong());
            let date = m.end_of_support.as_deref().unwrap_or_default();
            match m.support() {
                Some(Support::Ended { days_ago }) => {
                    ui.colored_label(
                        usage_color(95.0),
                        format!("ended {date} ({days_ago} days ago) — upgrade the OS release"),
                    );
                }
                Some(Support::EndingSoon { days_left }) => {
                    ui.colored_label(
                        usage_color(80.0),
                        format!("ends {date} (in {days_left} days)"),
                    );
                }
                Some(Support::Supported { .. }) => {
                    ui.label(format!("supported until {date}"));
                }
                None => {
                    ui.label(unknown())
                        .on_hover_text("Release not in Telepad's end-of-support table");
                }
            }
            ui.end_row();
        });
}

/// Hostname from the inventory, else the alias.
fn host_address(e: &telepad_core::config::HostEntry) -> &str {
    if e.hostname.is_empty() {
        &e.alias
    } else {
        &e.hostname
    }
}
