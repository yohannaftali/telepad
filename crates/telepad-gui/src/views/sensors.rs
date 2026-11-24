//! Sensors section of the host detail: fans, temperatures, BMC hint.

use eframe::egui::{self, RichText};
use telepad_app::Msg;

use crate::icons::{self, label as il};
use crate::widgets::*;
use telepad_core::model::{HostReport, SensorKind};

/// Fans (BMC) first, then temperatures; explains why BMC data is missing when it is.
/// `host`: inventory index (`None` for My Computer, #129).
pub fn sensors(ui: &mut egui::Ui, r: &HostReport, host: Option<usize>, out: &mut Vec<Msg>) {
    if let Some(e) = &r.bmc_error
        && let Some(i) = host
    {
        let fix = if e.contains("not installed") {
            "install it on this host: sudo apt install ipmitool"
        } else {
            "add the sudoers rule for 'ipmitool sdr type Fan' and 'ipmitool sdr type Temperature' (see README)"
        };
        ui.label(
            RichText::new(format!(
                "{}  BMC found but not readable: {e} — {fix}",
                icons::WARNING
            ))
            .color(usage_color(80.0)),
        );
        if ui
            .button(il(icons::TERMINAL, "Set up BMC access…"))
            .on_hover_text(
                "Opens a terminal on this host showing the sudo commands. Press Enter to run them and type your sudo password; Ctrl+C cancels.",
            )
            .clicked()
        {
            out.push(Msg::BmcSetup(i));
        }
        ui.add_space(4.0);
    }
    if r.sensors.is_empty() {
        let why = if host.is_none() && cfg!(windows) {
            "not available — Windows shows temperatures only to administrators (WMI thermal zones)"
        } else if host.is_none() {
            "not available — no temperature or fan sensors readable on this computer"
        } else if r.has_bmc {
            "No sensor readings"
        } else {
            "No temperature or fan sensors exposed (typical for VMs)"
        };
        ui.label(RichText::new(why).weak());
        return;
    }
    let fans: Vec<_> = r
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Fan)
        .collect();
    if !fans.is_empty() {
        let max = fans.iter().map(|s| s.value).fold(1.0_f32, f32::max);
        ui.label(RichText::new(il(icons::FAN, "Fans")).strong());
        egui::Grid::new("fans")
            .striped(true)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for s in &fans {
                    spinning_fan(ui, 18.0, (s.value / max).clamp(0.0, 1.0));
                    ui.label(&s.chip);
                    ui.label(&s.label);
                    ui.add(
                        egui::ProgressBar::new((s.value / max).clamp(0.0, 1.0))
                            .desired_width(220.0)
                            .fill(crate::widgets::info_fill())
                            .text(format!("{:.0} RPM", s.value)),
                    );
                    ui.end_row();
                }
            });
        ui.add_space(6.0);
    }
    let temps: Vec<_> = r
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .collect();
    if !temps.is_empty() {
        ui.label(RichText::new(il(icons::THERMOMETER, "Temperatures")).strong());
        egui::Grid::new("temps")
            .striped(true)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for s in temps {
                    ui.label(
                        RichText::new(icons::THERMOMETER)
                            .size(16.0)
                            .color(temp_color(s.value)),
                    )
                    .on_hover_text(temp_level(s.value));
                    ui.label(&s.chip);
                    ui.label(&s.label);
                    ui.colored_label(temp_color(s.value), format!("{:.1} °C", s.value));
                    ui.end_row();
                }
            });
    }
}
