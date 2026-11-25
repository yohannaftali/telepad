//! "Overview — Docker": every container of every host, grouped by stack
//! (compose project). A stack deployed on several hosts shows once, with one
//! block per host for its compose / Portainer controls.

use eframe::egui::{self, Color32, RichText};
use telepad_app::{Model, Msg, StackRow, docker_stacks};
use telepad_core::model::Container;

use super::docker::{container_table, manager_text, stack_controls};
use crate::icons;
use crate::widgets::*;

pub fn docker_overview(
    ui: &mut egui::Ui,
    m: &Model,
    filter: &str,
    d: &mut telepad_app::Drafts,
    out: &mut Vec<Msg>,
) {
    let rows = docker_stacks(m, filter);
    let with_docker = m
        .hosts
        .iter()
        .filter(|h| {
            h.report
                .as_ref()
                .is_some_and(|r| r.docker.unavailable.is_none() && !r.rdp_only)
        })
        .count();
    let containers: usize = rows.iter().map(|r| r.containers.len()).sum();
    let running: usize = rows.iter().map(StackRow::running).sum();
    let stacks = rows.iter().filter(|r| !r.name.is_empty()).count();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{stacks} stack(s) · {containers} container(s), {running} running · \
                 {with_docker} host(s) with Docker / Podman"
            ))
            .strong(),
        );
        if !filter.is_empty() {
            ui.label(RichText::new(format!("matching “{filter}”")).weak());
        }
    });
    ui.add_space(4.0);
    if rows.is_empty() {
        ui.label(
            RichText::new(if m.hosts.iter().all(|h| h.report.is_none()) {
                "No host has reported yet."
            } else if filter.is_empty() {
                "No containers on any host."
            } else {
                "No stack, container, image or host matches the search."
            })
            .weak(),
        );
        return;
    }
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in &rows {
                stack(ui, m, row, d, out);
            }
        });
}

fn stack(
    ui: &mut egui::Ui,
    m: &Model,
    row: &StackRow<'_>,
    d: &mut telepad_app::Drafts,
    out: &mut Vec<Msg>,
) {
    let (running, total) = (row.running(), row.containers.len());
    let color = if running == total {
        usage_color(0.0)
    } else if running == 0 {
        Color32::GRAY
    } else {
        usage_color(80.0)
    };
    let (title, icon) = if row.name.is_empty() {
        ("Standalone containers", icons::CONTAINER)
    } else {
        (row.name, icons::STACK)
    };
    let hosts = row.hosts();
    let open_id = ui.make_persistent_id(("docker-overview-open", row.name));
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(open_id, true));
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            let arrow = if open {
                icons::CARET_DOWN
            } else {
                icons::CARET_RIGHT
            };
            let resp = ui.add(
                egui::Button::new(
                    RichText::new(format!("{arrow} {icon} {title}"))
                        .strong()
                        .size(15.0),
                )
                .frame(false),
            );
            if resp.clicked() {
                open = !open;
                ui.data_mut(|d| d.insert_persisted(open_id, open));
            }
            ui.label(RichText::new(format!("{running}/{total} running")).color(color));
            ui.label(RichText::new(format!("on {} host(s):", hosts.len())).weak());
            for &i in &hosts {
                host_link(ui, m, i, out);
            }
        });
        if !open {
            return;
        }
        for &i in &hosts {
            let cs: Vec<&Container> = row
                .containers
                .iter()
                .filter(|(h, _)| *h == i)
                .map(|(_, c)| *c)
                .collect();
            let (Some(h), Some(r)) = (
                m.hosts.get(i),
                m.hosts.get(i).and_then(|h| h.report.as_ref()),
            ) else {
                continue;
            };
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                status_dot(ui, h);
                host_link(ui, m, i, out);
                if !row.name.is_empty() {
                    let manager = r.docker.stack_manager(&cs);
                    let (text, c) = manager_text(&manager);
                    ui.label(RichText::new(text).color(c));
                    let address = if h.entry.hostname.is_empty() {
                        &h.entry.alias
                    } else {
                        &h.entry.hostname
                    };
                    stack_controls(ui, r, row.name, &manager, i, address, out);
                }
            });
            ui.indent(("docker-overview-body", row.name, i), |ui| {
                container_table(ui, r, Some(i), &h.entry.alias, row.name, &cs, d, out);
            });
        }
    });
    ui.add_space(6.0);
}

fn host_link(ui: &mut egui::Ui, m: &Model, i: usize, out: &mut Vec<Msg>) {
    let Some(h) = m.hosts.get(i) else { return };
    if ui
        .link(RichText::new(&h.entry.alias).strong())
        .on_hover_text("Open this host")
        .clicked()
    {
        out.push(Msg::Select(Some(i)));
    }
    if h.report.as_ref().is_some_and(|r| r.docker.is_podman()) {
        ui.label(RichText::new("(podman)").weak().small());
    }
}
