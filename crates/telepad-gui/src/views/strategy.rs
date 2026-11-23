//! Update Strategy page (#122): pause, and what Telepad checks by itself and
//! how often. The form is a draft; Save sends it to `update()`.

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::strategy::{Check, Strategy, human_secs};

use crate::icons::{self, label as il};
use crate::widgets::*;

/// Units offered next to an interval: (label, seconds).
const UNITS: [(&str, u32); 4] = [("s", 1), ("min", 60), ("h", 3600), ("d", 86400)];

pub fn strategy_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.label(
            RichText::new(
                "What Telepad checks by itself and how often. It only reads: it never runs \
                 apt-get update or anything else that changes a host — pending updates come \
                 from the host's own package lists. Refresh all and a host's Refresh read \
                 everything now, also while paused or with a check turned off.",
            )
            .weak(),
        );
        ui.add_space(6.0);
        pause_row(ui, m, out);
        ui.add_space(8.0);

        let form = &mut d.strategy.form;
        egui::Grid::new("strategy_grid")
            .num_columns(5)
            .spacing([14.0, 8.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("On").strong());
                ui.label(RichText::new("Check").strong());
                ui.label(RichText::new("Reads").strong());
                ui.label(RichText::new("Every").strong());
                ui.label(RichText::new("Allowed").strong());
                ui.end_row();
                for c in Check::ALL {
                    row(ui, form, c);
                    ui.end_row();
                }
            });

        if let Some(e) = &d.strategy.error {
            ui.add_space(6.0);
            ui.colored_label(usage_color(95.0), il(icons::WARNING, e));
        }
        ui.add_space(8.0);
        let changed = d.strategy.changed(&m.strategy);
        ui.horizontal(|ui| {
            if act_button(ui, changed, Act::Edit, &il(icons::CHECK, "Save"))
                .on_hover_text("Applies from the next round, no restart needed")
                .clicked()
            {
                out.push(Msg::StrategySave);
            }
            if ui
                .add_enabled(changed, egui::Button::new("Revert"))
                .on_hover_text("Back to what is saved")
                .clicked()
            {
                out.push(Msg::StrategyOpen);
            }
            let defaults = Strategy {
                paused: m.strategy.paused,
                ..Default::default()
            };
            if ui
                .add_enabled(d.strategy.form != defaults, egui::Button::new("Reset to defaults"))
                .on_hover_text("Ping 30 s, live 1 min, network & updates 6 h, system info 1 d, SSH retry 10 min (Save to keep)")
                .clicked()
            {
                out.push(Msg::StrategyReset);
            }
        });
        if !m.strategy_ok {
            ui.add_space(4.0);
            ui.label(
                RichText::new("update.db could not be opened: changes last until Telepad closes.")
                    .color(usage_color(95.0)),
            );
        }
        ui.add_space(10.0);
        ui.label(RichText::new(status_line(m)).weak().small());
        ui.add_space(10.0);
        ui.separator();
        hosts(ui, m, &mut d.strategy.host_filter, out);
    });
}

/// Every monitored host: paused or not, last checked, ping; Pause / Resume
/// and Refresh one by one or for the listed hosts.
fn hosts(ui: &mut egui::Ui, m: &Model, filter: &mut String, out: &mut Vec<Msg>) {
    ui.heading(il(icons::DESKTOP, "Hosts"));
    ui.label(
        RichText::new(
            "A paused host is not checked by itself (no SSH, no ping) and Refresh all skips it; \
             its own Refresh still reads it.",
        )
        .weak(),
    );
    let listed: Vec<usize> = (0..m.hosts.len())
        .filter(|&i| !m.hosts[i].entry.unmonitored)
        .filter(|&i| telepad_app::matches_filter(filter.trim(), &m.hosts[i]))
        .collect();
    let aliases = |paused: bool| -> Vec<String> {
        listed
            .iter()
            .map(|&i| &m.hosts[i])
            .filter(|h| m.host_paused(h) != paused)
            .map(|h| h.entry.alias.clone())
            .collect()
    };
    let (to_pause, to_resume) = (aliases(true), aliases(false));
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(filter)
                .desired_width(200.0)
                .hint_text(format!("{} host, IP, OS", icons::SEARCH)),
        );
        if ui
            .add_enabled(
                !to_pause.is_empty(),
                egui::Button::new(il(
                    icons::PAUSE,
                    &format!("Pause {} listed", to_pause.len()),
                )),
            )
            .clicked()
        {
            out.push(Msg::PauseHosts {
                aliases: to_pause,
                paused: true,
            });
        }
        if ui
            .add_enabled(
                !to_resume.is_empty(),
                egui::Button::new(il(
                    icons::PLAY,
                    &format!("Resume {} listed", to_resume.len()),
                )),
            )
            .clicked()
        {
            out.push(Msg::PauseHosts {
                aliases: to_resume,
                paused: false,
            });
        }
        ui.label(RichText::new(format!("{} paused", m.paused_hosts.len())).weak());
    });
    ui.add_space(4.0);
    egui::Grid::new("strategy_hosts")
        .num_columns(5)
        .spacing([14.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("Host").strong());
            ui.label(RichText::new("Updated").strong());
            ui.label(RichText::new("Ping").strong());
            ui.label("");
            ui.end_row();
            for &i in &listed {
                let h = &m.hosts[i];
                let paused = m.host_paused(h);
                status_dot(ui, h);
                ui.label(RichText::new(&h.entry.alias).strong());
                if paused {
                    ui.label(
                        RichText::new(il(icons::PAUSE, &format!("paused · {}", ago(h.updated))))
                            .color(usage_color(75.0)),
                    );
                } else {
                    ui.label(RichText::new(ago(h.updated)).weak());
                }
                ui.label(
                    RichText::new(match h.ping {
                        Some(Some(ms)) => format!("{ms:.0} ms"),
                        Some(None) => "no reply".into(),
                        None => "—".into(),
                    })
                    .weak(),
                );
                ui.horizontal(|ui| {
                    let (icon, text) = if paused {
                        (icons::PLAY, "Resume")
                    } else {
                        (icons::PAUSE, "Pause")
                    };
                    if ui.small_button(il(icon, text)).clicked() {
                        out.push(Msg::PauseHosts {
                            aliases: vec![h.entry.alias.clone()],
                            paused: !paused,
                        });
                    }
                    if ui
                        .add_enabled(
                            !h.loading,
                            egui::Button::new(il(icons::REFRESH, "Refresh")).small(),
                        )
                        .clicked()
                    {
                        out.push(Msg::Refresh(i));
                    }
                    if h.loading {
                        ui.spinner();
                    }
                });
                ui.end_row();
            }
        });
    if listed.is_empty() {
        ui.label(RichText::new("No matching monitored hosts").weak());
    }
}

fn pause_row(ui: &mut egui::Ui, m: &Model, out: &mut Vec<Msg>) {
    ui.horizontal(|ui| {
        if m.strategy.paused {
            ui.label(
                RichText::new(il(icons::PAUSE, &paused_text(m)))
                    .color(usage_color(75.0))
                    .strong(),
            );
            if act_button(ui, true, Act::Add, &il(icons::PLAY, "Resume")).clicked() {
                out.push(Msg::SetPaused(false));
            }
        } else {
            ui.label(il(icons::CLOCK, "Automatic checks are running."));
            if ui
                .button(il(icons::PAUSE, "Pause all"))
                .on_hover_text("Stop every automatic check (SSH and ping) until Resume")
                .clicked()
            {
                out.push(Msg::SetPaused(true));
            }
        }
    });
}

/// "Paused 12m — nothing runs by itself".
pub fn paused_text(m: &Model) -> String {
    let since = m
        .paused_at
        .map(|t| format!(" {}", short_age(t.elapsed().as_secs())))
        .unwrap_or_default();
    format!("Paused{since} — nothing runs by itself")
}

fn short_age(s: u64) -> String {
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h {}m", s / 3600, s % 3600 / 60)
    }
}

fn row(ui: &mut egui::Ui, form: &mut Strategy, c: Check) {
    let (on, secs): (Option<&mut bool>, &mut u32) = match c {
        Check::Ping => (Some(&mut form.ping.on), &mut form.ping.secs),
        Check::Live => (Some(&mut form.live.on), &mut form.live.secs),
        Check::Slow => (Some(&mut form.slow.on), &mut form.slow.secs),
        Check::Static => (Some(&mut form.static_info.on), &mut form.static_info.secs),
        Check::SshRetry => (None, &mut form.ssh_retry),
    };
    let enabled = match on {
        Some(on) => {
            ui.checkbox(on, "");
            *on
        }
        None => {
            ui.label("");
            true
        }
    };
    ui.label(RichText::new(c.title()).strong());
    ui.label(RichText::new(reads(c)).weak());
    ui.add_enabled_ui(enabled, |ui| every(ui, c, secs));
    let (lo, hi) = c.range();
    ui.label(RichText::new(format!("{} – {}", human_secs(lo), human_secs(hi))).weak());
}

fn reads(c: Check) -> &'static str {
    match c {
        Check::Ping => "Host up or down, without SSH (RDP-only hosts: ping + port probe)",
        Check::Live => "CPU, memory, disks, sensors, Docker, Kubernetes / Swarm",
        Check::Slow => "IP addresses, listening ports, pending updates, reboot needed",
        Check::Static => "Hostname, OS release, kernel, architecture, CPU model",
        Check::SshRetry => "Hosts without ping reply are only pinged; SSH is tried this often",
    }
}

/// Number + unit; the unit shown is the largest that divides the value.
fn every(ui: &mut egui::Ui, c: Check, secs: &mut u32) {
    let (_, unit) = UNITS
        .iter()
        .rev()
        .find(|(_, u)| *secs >= *u && (*secs).is_multiple_of(*u))
        .copied()
        .unwrap_or(UNITS[0]);
    let mut n = *secs / unit;
    let mut u = unit;
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut n).range(1..=9999));
        egui::ComboBox::from_id_salt(("strategy_unit", c as u8))
            .width(56.0)
            .selected_text(UNITS.iter().find(|(_, x)| *x == u).map_or("s", |(l, _)| *l))
            .show_ui(ui, |ui| {
                for (label, x) in UNITS {
                    ui.selectable_value(&mut u, x, label);
                }
            });
    });
    *secs = n.saturating_mul(u);
}

fn status_line(m: &Model) -> String {
    let ping = match m.last_ping {
        _ if !m.strategy.ping.on => "ping check off".to_string(),
        Some(t) => format!("last ping round {}", ago(Some(t))),
        None => "no ping round yet".to_string(),
    };
    let live = match m.last_refresh {
        _ if !m.strategy.live.on => "live metrics off".to_string(),
        Some(t) => format!("last live round {}", ago(Some(t))),
        None => "no live round yet".to_string(),
    };
    format!("{live} · {ping} · saved in update.db next to telepad.db")
}
