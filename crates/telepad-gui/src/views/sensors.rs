//! Sensors section of the host detail: fans, temperatures, GPU power (#193),
//! BMC hint and why readings are missing (`HostReport::sensors_note`).

use eframe::egui::{self, RichText};
use telepad_app::Msg;

use crate::icons::{self, label as il};
use crate::widgets::*;
use telepad_core::model::{HostReport, SensorKind};

/// Fans (BMC) first, then temperatures; explains why BMC data is missing when it is.
/// `host`: inventory index (`None` for My Computer, #129).
pub fn sensors(
    ui: &mut egui::Ui,
    r: &HostReport,
    host: Option<usize>,
    alias: &str,
    d: &mut telepad_app::Drafts,
    out: &mut Vec<Msg>,
) {
    use crate::table::{self, Col, Ev, Spec};
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
        let why = if let Some(n) = &r.sensors_note {
            format!("not available — {n}")
        } else if host.is_none() {
            "not available — no temperature or fan sensors readable on this computer".into()
        } else if r.has_bmc {
            "No sensor readings".into()
        } else {
            "No temperature or fan sensors exposed (typical for VMs)".into()
        };
        ui.label(RichText::new(why).weak());
        return;
    }
    if let Some(n) = &r.sensors_note {
        ui.label(RichText::new(format!("{}  {n}", icons::INFO)).weak());
        ui.add_space(4.0);
    }
    let fans: Vec<_> = r
        .sensors
        .iter()
        .filter(|s| matches!(s.kind, SensorKind::Fan | SensorKind::FanPercent))
        .collect();
    if !fans.is_empty() {
        let max_rpm = fans
            .iter()
            .filter(|s| s.kind == SensorKind::Fan)
            .map(|s| s.value)
            .fold(1.0_f32, f32::max);
        // GPUs report a share of full speed, BMC / board fans RPM (#193).
        let share = |s: &telepad_core::model::Sensor| match s.kind {
            SensorKind::FanPercent => s.value / 100.0,
            _ => s.value / max_rpm,
        };
        ui.label(RichText::new(il(icons::FAN, "Fans")).strong());
        let cols = [
            Col::action(""),
            Col::new("Chip"),
            Col::new("Sensor"),
            Col::new("Speed").initial(230.0),
        ];
        let cells: Vec<Vec<String>> = fans
            .iter()
            .map(|s| {
                vec![
                    String::new(),
                    s.chip.clone(),
                    s.label.clone(),
                    s.kind.format(s.value),
                ]
            })
            .collect();
        let title = format!("Fans of {alias}");
        let st = table::state(d, format!("fans:{alias}"));
        let o = table::show(
            ui,
            &Spec::new(("fans", alias), &title, &cols),
            st,
            &cells,
            |ui, ev| {
                if let Ev::Cell { row, col } = ev {
                    let s = fans[row];
                    match col {
                        0 => {
                            spinning_fan(ui, 18.0, share(s).clamp(0.0, 1.0));
                        }
                        3 => {
                            ui.add(
                                egui::ProgressBar::new(share(s).clamp(0.0, 1.0))
                                    .desired_width(ui.available_width().min(220.0))
                                    .fill(crate::widgets::info_fill())
                                    .text(&cells[row][3]),
                            );
                        }
                        c => table::text(ui, &cells[row][c]),
                    }
                }
            },
        );
        out.extend(o.msgs);
        ui.add_space(6.0);
    }
    let temps: Vec<_> = r
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .collect();
    if !temps.is_empty() {
        ui.label(RichText::new(il(icons::THERMOMETER, "Temperatures")).strong());
        let cols = [
            Col::action(""),
            Col::new("Chip"),
            Col::new("Sensor"),
            Col::new("Temperature"),
        ];
        let cells: Vec<Vec<String>> = temps
            .iter()
            .map(|s| {
                vec![
                    String::new(),
                    s.chip.clone(),
                    s.label.clone(),
                    s.kind.format(s.value),
                ]
            })
            .collect();
        let title = format!("Temperatures of {alias}");
        let st = table::state(d, format!("temps:{alias}"));
        let o = table::show(
            ui,
            &Spec::new(("temps", alias), &title, &cols),
            st,
            &cells,
            |ui, ev| {
                if let Ev::Cell { row, col } = ev {
                    let s = temps[row];
                    match col {
                        0 => {
                            ui.label(
                                RichText::new(icons::THERMOMETER)
                                    .size(16.0)
                                    .color(temp_color(s.value)),
                            )
                            .on_hover_text(temp_level(s.value));
                        }
                        3 => {
                            ui.colored_label(temp_color(s.value), &cells[row][3]);
                        }
                        c => table::text(ui, &cells[row][c]),
                    }
                }
            },
        );
        out.extend(o.msgs);
    }
    let power: Vec<_> = r
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Power)
        .collect();
    if !power.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(il(icons::GAUGE, "Power")).strong());
        let cols = [Col::new("Chip"), Col::new("Sensor"), Col::new("Power")];
        let cells: Vec<Vec<String>> = power
            .iter()
            .map(|s| vec![s.chip.clone(), s.label.clone(), s.kind.format(s.value)])
            .collect();
        let title = format!("Power of {alias}");
        let st = table::state(d, format!("power:{alias}"));
        let o = table::show(
            ui,
            &Spec::new(("power", alias), &title, &cols),
            st,
            &cells,
            |ui, ev| {
                if let Ev::Cell { row, col } = ev {
                    table::text(ui, &cells[row][col]);
                }
            },
        );
        out.extend(o.msgs);
    }
}
