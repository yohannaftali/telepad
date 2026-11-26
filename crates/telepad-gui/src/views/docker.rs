//! Docker section of the host detail: summary, stacks and containers.

use eframe::egui::{self, Color32, RichText};

use crate::icons::{self, label as il};
use crate::widgets::*;
use telepad_app::Msg;
use telepad_core::docker_actions::{ContainerAction, StackAction};
use telepad_core::model::{Container, HostReport, Manager, PortainerRole};

/// Web UI of a Portainer server running on this host: https on 9443, http on 9000.
fn portainer_url(r: &HostReport, host: &str) -> Option<String> {
    let (_, c) = r
        .docker
        .portainer()
        .into_iter()
        .find(|(role, _)| *role == PortainerRole::Server)?;
    let https = c.published.iter().find(|p| p.container_port == 9443);
    let http = c.published.iter().find(|p| p.container_port == 9000);
    match (https, http) {
        (Some(p), _) => Some(format!("https://{host}:{}/", p.host_port)),
        (None, Some(p)) => Some(format!("http://{host}:{}/", p.host_port)),
        _ => None,
    }
}

const TERMINAL_HINT: &str = "Opens a terminal on the host that shows the exact command and runs it only after you press Enter";

fn action_label(text: &str, destructive: bool) -> RichText {
    if destructive {
        act_text(Act::Danger, text)
    } else {
        RichText::new(text)
    }
}

/// Actions for one container (menu).
pub fn container_menu(
    ui: &mut egui::Ui,
    c: &Container,
    host: usize,
    alias: &str,
    out: &mut Vec<Msg>,
) {
    ui.menu_button(icons::MENU, |ui| container_items(ui, c, host, alias, out))
        .response
        .on_hover_text("Shell, logs, start / stop… and how it runs");
}

/// The container's actions, in its ☰ menu and its right-click menu.
pub fn container_items(
    ui: &mut egui::Ui,
    c: &Container,
    host: usize,
    alias: &str,
    out: &mut Vec<Msg>,
) {
    {
        ui.label(RichText::new(&c.name).strong());
        if let Some(t) = telepad_core::db::detect::container_target(alias, c) {
            if ui
                .button(il(icons::DATABASE, "Open database…"))
                .on_hover_text(format!(
                    "{} through the SSH login of {alias} — tables, data, queries",
                    t.engine.title()
                ))
                .clicked()
            {
                out.push(Msg::Db(telepad_app::db::DbMsg::Open(t)));
                ui.close();
            }
            ui.separator();
        }
        ui.label(RichText::new(TERMINAL_HINT).weak().small());
        ui.separator();
        for action in ContainerAction::ALL {
            if !action.available(&c.state) {
                continue;
            }
            let icon = match action {
                ContainerAction::Shell => icons::TERMINAL,
                ContainerAction::Logs => icons::INFO,
                ContainerAction::Remove => icons::TRASH,
                ContainerAction::Restart => icons::REFRESH,
                _ => icons::CONTAINER,
            };
            if ui
                .button(action_label(
                    &format!("{icon} {}", action.title()),
                    action.destructive(),
                ))
                .clicked()
            {
                out.push(Msg::DockerAction {
                    host,
                    container: c.id.clone(),
                    action,
                });
                ui.close();
            }
        }
        ui.separator();
        if ui
            .button(il(icons::COPY, "How it runs…"))
            .on_hover_text("docker run / compose file, read-only, secrets masked")
            .clicked()
        {
            out.push(Msg::ShowRunInfo {
                host,
                container: c.id.clone(),
            });
            ui.close();
        }
    }
}

/// Compose actions for a stack, or the way to Portainer.
#[allow(clippy::too_many_arguments)]
pub fn stack_controls(
    ui: &mut egui::Ui,
    r: &HostReport,
    stack: &str,
    manager: &Manager,
    host: usize,
    alias: &str,
    address: &str,
    out: &mut Vec<Msg>,
) {
    match manager {
        Manager::Compose { .. } => {
            ui.menu_button(il(icons::STACK, "Compose"), |ui| {
                ui.label(RichText::new(TERMINAL_HINT).weak().small());
                ui.separator();
                for action in StackAction::ALL {
                    if ui
                        .button(action_label(action.title(), action.destructive()))
                        .clicked()
                    {
                        out.push(Msg::StackAction {
                            host,
                            stack: stack.to_string(),
                            action,
                        });
                        ui.close();
                    }
                }
            });
        }
        Manager::Portainer { .. } => {
            // Its Stack page in the registered Portainer that manages this host (#176).
            if ui
                .button(il(icons::STACK, "Open stack"))
                .on_hover_text("Its Stack page in Telepad: containers and editor, through the registered Portainer that manages this host")
                .clicked()
            {
                out.push(Msg::Portainer(telepad_app::portainer::PtMsg::OpenStackFor {
                    alias: alias.to_string(),
                    name: stack.to_string(),
                }));
            }
            portainer_link(ui, r, address, out);
        }
        Manager::Standalone => {}
    }
}

/// Open in Portainer, or where its stacks are changed.
fn portainer_link(ui: &mut egui::Ui, r: &HostReport, address: &str, out: &mut Vec<Msg>) {
    match portainer_url(r, address) {
        Some(url) => {
            if ui
                .button(RichText::new("Open in Portainer").color(portainer_color()))
                .on_hover_text(format!("{url}\nPortainer stacks are changed in Portainer"))
                .clicked()
            {
                out.push(Msg::OpenUrl(url));
            }
        }
        None => {
            ui.label(RichText::new("change it in Portainer").weak())
                .on_hover_text("Managed by a Portainer server elsewhere (agent on this host)");
        }
    }
}

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
                ui.label(RichText::new(text).color(portainer_color()));
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
            (format!("Portainer stack #{stack_id}"), portainer_color())
        }
        Manager::Compose { dir, files } => {
            let file = files.rsplit(['/', '\\']).next().unwrap_or(files);
            let f = if file.is_empty() {
                String::new()
            } else {
                format!(" ({file})")
            };
            (format!("docker compose in {dir}{f}"), pal().purple)
        }
        Manager::Standalone => ("docker run (no compose)".into(), pal().grey),
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

/// `host`: inventory index; `None` for My Computer (#129): no actions, only
/// "Open database" for containers that publish a database port here.
pub fn docker_stacks(
    ui: &mut egui::Ui,
    r: &HostReport,
    host: Option<usize>,
    alias: &str,
    address: &str,
    d: &mut telepad_app::Drafts,
    out: &mut Vec<Msg>,
) {
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
        let manager = r.docker.stack_manager(&containers);
        let (mtext, mcolor) = manager_text(&manager);
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
                if let Some(host) = host {
                    stack_controls(ui, r, stack, &manager, host, alias, address, out);
                }
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
            container_table(ui, r, host, alias, stack, &containers, d, out);
        });
        ui.add_space(4.0);
    }
}

/// Text of a container's cells: Container, Image, Status, IP address, Host
/// port → container, Internal (one line each).
pub fn container_cells(c: &Container) -> Vec<String> {
    let ips: Vec<String> = c
        .networks
        .iter()
        .filter(|(_, ip)| !ip.is_empty())
        .map(|(n, ip)| format!("{ip} ({n})"))
        .collect();
    let (published, internal) = container_ports(c);
    vec![
        c.name.clone(),
        c.image.clone(),
        c.status.clone(),
        ips.join(", "),
        published.replace('\n', ", "),
        internal,
    ]
}

/// The containers of one stack (#153: sort, filter, menu).
#[allow(clippy::too_many_arguments)]
pub fn container_table(
    ui: &mut egui::Ui,
    r: &HostReport,
    host: Option<usize>,
    alias: &str,
    stack: &str,
    containers: &[&Container],
    d: &mut telepad_app::Drafts,
    out: &mut Vec<Msg>,
) {
    use crate::table::{self, Col, Ev, Spec};
    let cols = [
        Col::action(""),
        Col::new("Container"),
        Col::new("Image"),
        Col::new("Status"),
        Col::new("IP address").mono(),
        Col::new("Host port → container").mono(),
        Col::new("Internal").mono(),
        Col::action(""),
    ];
    let cells: Vec<Vec<String>> = containers
        .iter()
        .map(|c| {
            let mut v = vec![String::new()];
            v.extend(container_cells(c));
            v.push(String::new());
            v
        })
        .collect();
    let title = format!("Containers of {alias} {stack}");
    let st = table::state(d, format!("docker:{alias}:{stack}"));
    let spec = Spec::new(("docker", alias, stack), &title, &cols);
    let o = table::show(ui, &spec, st, &cells, |ui, ev| match ev {
        Ev::Cell { row, col } => {
            let c = containers[row];
            match col {
                0 => {
                    let dot = if c.running() {
                        usage_color(0.0)
                    } else {
                        Color32::GRAY
                    };
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.5, dot);
                }
                1 => {
                    let mut n = RichText::new(&c.name).strong();
                    if c.is_nginx() {
                        n = n.color(nginx_color());
                    } else if c.portainer_role().is_some() {
                        n = n.color(portainer_color());
                    }
                    let resp = ui.add(egui::Label::new(n).truncate().selectable(false));
                    if c.mounts_socket {
                        resp.on_hover_text("Mounts the Docker socket");
                    }
                }
                2 => table::weak(ui, &cells[row][2]),
                4 | 5 => {
                    ui.add(
                        egui::Label::new(RichText::new(&cells[row][col]).monospace())
                            .truncate()
                            .selectable(false),
                    );
                }
                6 => {
                    ui.add(
                        egui::Label::new(RichText::new(&cells[row][6]).monospace().weak())
                            .truncate()
                            .selectable(false),
                    );
                }
                7 => match host {
                    Some(host) => container_menu(ui, c, host, alias, out),
                    None => local_container_db(ui, r, c, out),
                },
                c => table::text(ui, &cells[row][c]),
            }
        }
        Ev::Menu { row, .. } => {
            if let Some(host) = host {
                container_items(ui, containers[row], host, alias, out);
            }
        }
        Ev::HeaderMenu => {}
    });
    out.extend(o.msgs);
}

/// My Computer (#129): a database container published on this computer
/// opens on 127.0.0.1.
fn local_container_db(ui: &mut egui::Ui, r: &HostReport, c: &Container, out: &mut Vec<Msg>) {
    let t = telepad_core::local::db_targets(r)
        .into_iter()
        .find(|t| t.container == c.name);
    match t {
        Some(t) => {
            if ui
                .button(icons::DATABASE)
                .on_hover_text(format!(
                    "Open {} on 127.0.0.1:{} — tables, data, queries",
                    t.engine.title(),
                    t.port
                ))
                .clicked()
            {
                out.push(Msg::Db(telepad_app::db::DbMsg::Open(t)));
            }
        }
        None => {
            ui.label("");
        }
    }
}
