//! Database windows (#52–#54), HeidiSQL style: connect dialog, then a tree of
//! databases / schemas / tables on the left and Host / Database / Table /
//! Data / Query tabs on the right. Each connection is an inner window of the
//! workspace (#77).

use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column as TCol, TableBuilder};
use telepad_app::db::{
    DbConfirm, DbDraft, DbId, DbMsg, DbSession, DbStatus, DbTab, PAGE_SIZES, RowRef,
};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::db::ddl::{FkEdit, IndexEdit, TableEdit};
use telepad_core::db::{
    Cell, ColumnDefault, Engine, FK_ACTIONS, Grid, IndexKind, TableKind, TableRef, human_size, sql,
};

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_button, act_small, act_text, usage_color};

const ROW: f32 = 20.0;
/// Characters of a value shown in a grid cell.
const SHOWN: usize = 200;

fn send(out: &mut Vec<Msg>, m: DbMsg) {
    out.push(Msg::Db(m));
}

fn red() -> Color32 {
    usage_color(95.0)
}

fn green() -> Color32 {
    usage_color(10.0)
}

/// One database connection: an inner window of the workspace.
pub fn db_page(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, id: DbId, out: &mut Vec<Msg>) {
    let Some(s) = m.db.iter().find(|s| s.id == id) else {
        return;
    };
    let Some(dr) = d.db.get_mut(&s.id) else {
        return;
    };
    if s.status == DbStatus::Ready {
        browser(ui, s, dr, out);
    } else {
        egui::CentralPanel::default().show(ui, |ui| login(ui, s, dr, out));
    }
    confirm(ui, s, dr, out);
    viewer(ui, dr);
}

fn login(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let connecting = s.status == DbStatus::Connecting;
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(520.0);
        ui.heading(il(icons::DATABASE, &s.target.title()));
        let place = if s.target.container.is_empty() {
            "installed on the host".to_string()
        } else {
            format!("container {}", s.target.container)
        };
        ui.label(
            RichText::new(format!(
                "{} · {place} · {}:{} through the SSH login of {} — no port is opened",
                s.target.engine.title(),
                s.target.address,
                s.target.port,
                s.target.host
            ))
            .weak(),
        );
        ui.add_space(12.0);
        egui::Grid::new("db-login")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("User");
                ui.add_enabled(
                    !connecting,
                    egui::TextEdit::singleline(&mut d.user).desired_width(280.0),
                );
                ui.end_row();
                ui.label("Password");
                let hint = if s.saved_password && d.save_password {
                    "(saved — leave empty)"
                } else {
                    ""
                };
                let pw = ui.add_enabled(
                    !connecting,
                    egui::TextEdit::singleline(&mut d.password)
                        .password(true)
                        .hint_text(hint)
                        .desired_width(280.0),
                );
                if !connecting && d.user.is_empty() && !pw.has_focus() {
                    pw.request_focus();
                }
                if pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !connecting
                {
                    send(out, DbMsg::Connect(id));
                }
                ui.end_row();
                ui.label("Database");
                ui.add_enabled(
                    !connecting,
                    egui::TextEdit::singleline(&mut d.database)
                        .hint_text(match s.target.engine {
                            Engine::MySql => "optional",
                            Engine::Postgres => "optional (postgres)",
                        })
                        .desired_width(280.0),
                );
                ui.end_row();
                ui.label("");
                let store_ok = matches!(s.store, Some(Ok(())));
                let resp = ui.add_enabled(
                    store_ok && !connecting,
                    egui::Checkbox::new(&mut d.save_password, "Save password in the system password store"),
                );
                match &s.store {
                    Some(Err(e)) => {
                        resp.on_disabled_hover_text(format!("{e} — the password is asked each time"));
                    }
                    None => {
                        resp.on_disabled_hover_text("Checking the password store…");
                    }
                    Some(Ok(())) => {
                        resp.on_hover_text(
                            "Windows Credential Manager / macOS Keychain / Secret Service (GNOME Keyring, KWallet). Never written to telepad.db.",
                        );
                    }
                }
                ui.end_row();
                ui.label("");
                let mut ro = s.read_only;
                if ui
                    .checkbox(&mut ro, "Read-only")
                    .on_hover_text("The server refuses changes; only reading queries run")
                    .changed()
                {
                    send(out, DbMsg::SetReadOnly(id, ro));
                }
                ui.end_row();
            });
        if let Some(Err(e)) = &s.store {
            ui.label(RichText::new(format!("{} {e}", icons::INFO)).weak().small());
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!connecting, egui::Button::new(il(icons::PLUGS, "Connect")))
                .clicked()
            {
                send(out, DbMsg::Connect(id));
            }
            if connecting {
                ui.spinner();
                ui.label("Opening the SSH tunnel and logging in…");
            }
            if !connecting
                && act_button(ui, true, Act::Danger, &il(icons::TRASH, "Forget saved connection"))
                    .on_hover_text("Delete the saved user, database and password")
                    .clicked()
            {
                send(out, DbMsg::Forget(id));
            }
        });
        if let Some(e) = &s.error {
            ui.add_space(8.0);
            ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
        }
        if let Some(n) = &s.notice {
            ui.label(RichText::new(n).weak());
        }
    });
}

fn browser(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    egui::Panel::top(("db-top", id)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(il(icons::DATABASE, &s.target.title())).strong());
            ui.label(RichText::new(&s.version).weak().small());
            ui.separator();
            if ui.button(il(icons::REFRESH, "Refresh")).clicked() {
                send(out, DbMsg::Refresh(id));
            }
            let mut ro = s.read_only;
            let label = if ro {
                il(icons::LOCK, "Read-only")
            } else {
                il(icons::LOCK_OPEN, "Read-only")
            };
            if ui
                .checkbox(&mut ro, label)
                .on_hover_text("On: the server refuses changes from this window")
                .changed()
            {
                send(out, DbMsg::SetReadOnly(id, ro));
            }
            if ui
                .button(il(icons::PLUGS, "Reconnect"))
                .on_hover_text("Close this connection and log in again")
                .clicked()
            {
                send(out, DbMsg::Reconnect(id));
            }
        });
    });
    egui::Panel::bottom(("db-status", id)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if let Some(e) = &s.error {
                ui.colored_label(red(), format!("{} {}", icons::X_CIRCLE, first_line(e)))
                    .on_hover_text(e);
            } else if let Some(n) = &s.notice {
                ui.colored_label(green(), format!("{} {n}", icons::CHECK_CIRCLE));
            } else {
                ui.label(
                    RichText::new(format!(
                        "{}:{} via SSH {}",
                        s.target.address, s.target.port, s.target.host
                    ))
                    .weak(),
                );
            }
        });
    });
    egui::Panel::bottom(("db-log", id))
        .resizable(true)
        .default_size(180.0)
        .min_size(28.0)
        .show(ui, |ui| sql_log(ui, s, d, out));
    egui::Panel::left(("db-tree", id))
        .resizable(true)
        .default_size(270.0)
        .show(ui, |ui| tree(ui, s, d, out));
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            for tab in DbTab::ALL {
                let text = match tab {
                    DbTab::Table | DbTab::Data => match &s.data.table {
                        Some(t) => format!("{}: {}", tab.title(), t.table),
                        None => tab.title().to_string(),
                    },
                    DbTab::Database => match &s.current {
                        Some((db, schema)) if !schema.is_empty() => {
                            format!("Database: {db}.{schema}")
                        }
                        Some((db, _)) => format!("Database: {db}"),
                        None => tab.title().to_string(),
                    },
                    _ => tab.title().to_string(),
                };
                let icon = match tab {
                    DbTab::Host => icons::DISKS,
                    DbTab::Database => icons::DATABASE,
                    DbTab::Table => icons::TABLE,
                    DbTab::Data => icons::ROWS,
                    DbTab::Query => icons::CODE,
                };
                if ui.selectable_label(s.tab == tab, il(icon, &text)).clicked() {
                    send(out, DbMsg::Tab(id, tab));
                }
            }
        });
        ui.separator();
        match s.tab {
            DbTab::Host => host_tab(ui, s, out),
            DbTab::Database => database_tab(ui, s, out),
            DbTab::Table => table_tab(ui, s, d, out),
            DbTab::Data => data_tab(ui, s, d, out),
            DbTab::Query => query_tab(ui, s, d, out),
        }
    });
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

fn matches(filter: &str, name: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(filter)
}

fn tree(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    ui.add(
        egui::TextEdit::singleline(&mut d.tree_filter)
            .hint_text(format!("{} filter databases / tables", icons::SEARCH))
            .desired_width(f32::INFINITY),
    );
    let filter = d.tree_filter.trim().to_lowercase();
    egui::ScrollArea::both()
        .id_salt(("db-tree-scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for node in &s.databases {
                let db = &node.info.name;
                let tables_match = node.schemas.iter().flatten().any(|sc| {
                    sc.tables
                        .iter()
                        .flatten()
                        .any(|t| matches(&filter, &t.name))
                });
                if !matches(&filter, db) && !tables_match {
                    continue;
                }
                let current = s.current.as_ref().is_some_and(|(c, _)| c == db);
                let mut title = RichText::new(format!("{} {db}", icons::DATABASE));
                if current {
                    title = title.strong();
                }
                // Open the current database once when it becomes current (also when
                // chosen elsewhere: the Host tab, the saved default database).
                let opened_key = egui::Id::unique(("db-node-opened", id));
                let newly_current = current
                    && ui.data(|m| m.get_temp::<String>(opened_key)).as_deref()
                        != Some(db.as_str());
                if newly_current {
                    ui.data_mut(|m| m.insert_temp(opened_key, db.clone()));
                }
                let force_open = (!filter.is_empty() && tables_match) || newly_current;
                let resp = egui::CollapsingHeader::new(title)
                    .id_salt(("db-node", id, db))
                    .open(if force_open { Some(true) } else { None })
                    .show(ui, |ui| {
                        if node.loading && node.schemas.is_none() {
                            ui.spinner();
                        }
                        let Some(schemas) = &node.schemas else {
                            return;
                        };
                        for sc in schemas {
                            if s.engine().has_schemas() {
                                let resp = egui::CollapsingHeader::new(format!(
                                    "{} {}",
                                    icons::FOLDER,
                                    sc.name
                                ))
                                .id_salt(("db-schema", id, db, &sc.name))
                                .show(ui, |ui| tables(ui, s, db, sc, &filter, out));
                                if resp.header_response.clicked() {
                                    send(
                                        out,
                                        DbMsg::SelectSchema {
                                            id,
                                            database: db.clone(),
                                            schema: sc.name.clone(),
                                        },
                                    );
                                }
                                resp.header_response.context_menu(|ui| {
                                    node_menu(ui, s, db, &sc.name, out);
                                });
                            } else {
                                tables(ui, s, db, sc, &filter, out);
                            }
                        }
                    });
                if resp.header_response.clicked() {
                    send(
                        out,
                        DbMsg::SelectDatabase {
                            id,
                            database: db.clone(),
                        },
                    );
                }
                if let Some(size) = node.info.size {
                    resp.header_response.clone().on_hover_text(human_size(size));
                }
                resp.header_response.context_menu(|ui| {
                    if !s.engine().has_schemas() {
                        node_menu(ui, s, db, "", out);
                    } else if ui.button(il(icons::REFRESH, "Open")).clicked() {
                        send(
                            out,
                            DbMsg::SelectDatabase {
                                id,
                                database: db.clone(),
                            },
                        );
                        ui.close();
                    }
                });
            }
        });
}

fn node_menu(ui: &mut egui::Ui, s: &DbSession, db: &str, schema: &str, out: &mut Vec<Msg>) {
    let id = s.id;
    if act_button(ui, !s.read_only, Act::Add, &il(icons::PLUS, "New table…")).clicked() {
        send(
            out,
            DbMsg::NewTable {
                id,
                database: db.into(),
                schema: schema.into(),
            },
        );
        ui.close();
    }
    if ui.button(il(icons::REFRESH, "Refresh")).clicked() {
        if schema.is_empty() {
            send(
                out,
                DbMsg::SelectDatabase {
                    id,
                    database: db.into(),
                },
            );
        } else {
            send(
                out,
                DbMsg::SelectSchema {
                    id,
                    database: db.into(),
                    schema: schema.into(),
                },
            );
        }
        ui.close();
    }
}

fn tables(
    ui: &mut egui::Ui,
    s: &DbSession,
    db: &str,
    sc: &telepad_app::db::SchemaNode,
    filter: &str,
    out: &mut Vec<Msg>,
) {
    let id = s.id;
    if sc.loading && sc.tables.is_none() {
        ui.spinner();
        return;
    }
    let Some(list) = &sc.tables else {
        return;
    };
    if list.is_empty() {
        ui.label(RichText::new("(no tables)").weak().small());
    }
    for t in list.iter().filter(|t| matches(filter, &t.name)) {
        let table = TableRef {
            database: db.to_string(),
            schema: sc.name.clone(),
            table: t.name.clone(),
        };
        let open = s.data.table.as_ref() == Some(&table);
        let icon = if t.kind == TableKind::View {
            icons::EYE
        } else {
            icons::TABLE
        };
        let resp = ui.selectable_label(open, format!("{icon} {}", t.name));
        let resp = match t.size {
            Some(size) => resp.on_hover_text(format!(
                "{} · ~{} rows · {}",
                t.engine,
                t.rows.unwrap_or(0),
                human_size(size)
            )),
            None => resp,
        };
        if resp.clicked() {
            send(
                out,
                DbMsg::SelectTable {
                    id,
                    table: table.clone(),
                },
            );
        }
        resp.context_menu(|ui| table_menu(ui, s, &table, t.kind, out));
    }
}

fn table_menu(ui: &mut egui::Ui, s: &DbSession, t: &TableRef, kind: TableKind, out: &mut Vec<Msg>) {
    let id = s.id;
    ui.label(RichText::new(&t.table).strong());
    if ui.button(il(icons::ROWS, "Open data")).clicked() {
        send(
            out,
            DbMsg::SelectTable {
                id,
                table: t.clone(),
            },
        );
        send(out, DbMsg::Tab(id, DbTab::Data));
        ui.close();
    }
    if ui.button(il(icons::TABLE, "Structure")).clicked() {
        send(
            out,
            DbMsg::SelectTable {
                id,
                table: t.clone(),
            },
        );
        send(out, DbMsg::Tab(id, DbTab::Table));
        ui.close();
    }
    ui.separator();
    let writable = !s.read_only;
    if kind == TableKind::Table
        && ui
            .add_enabled(
                writable,
                egui::Button::new(il(icons::MINUS, "Empty (TRUNCATE)…")),
            )
            .clicked()
    {
        send(
            out,
            DbMsg::TruncateTable {
                id,
                table: t.clone(),
            },
        );
        ui.close();
    }
    if act_button(ui, writable, Act::Danger, &il(icons::TRASH, "Drop…")).clicked() {
        send(
            out,
            DbMsg::DropTable {
                id,
                table: t.clone(),
            },
        );
        ui.close();
    }
}

fn host_tab(ui: &mut egui::Ui, s: &DbSession, out: &mut Vec<Msg>) {
    ui.label(RichText::new(format!("{} · {}", s.target.engine.title(), s.version)).strong());
    ui.label(RichText::new(format!("{} database(s)", s.databases.len())).weak());
    ui.add_space(6.0);
    let id = s.id;
    TableBuilder::new(ui)
        .id_salt(("db-host", id))
        .striped(true)
        .column(TCol::initial(320.0).at_least(120.0).clip(true))
        .column(TCol::remainder())
        .header(ROW + 2.0, |mut h| {
            h.col(|ui| {
                ui.strong("Database");
            });
            h.col(|ui| {
                ui.strong("Size");
            });
        })
        .body(|body| {
            body.rows(ROW, s.databases.len(), |mut row| {
                let n = &s.databases[row.index()];
                row.col(|ui| {
                    if ui.link(&n.info.name).clicked() {
                        send(
                            out,
                            DbMsg::SelectDatabase {
                                id,
                                database: n.info.name.clone(),
                            },
                        );
                    }
                });
                row.col(|ui| {
                    ui.label(n.info.size.map(human_size).unwrap_or_default());
                });
            });
        });
}

fn database_tab(ui: &mut egui::Ui, s: &DbSession, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some((db, schema)) = &s.current else {
        ui.label(RichText::new("Choose a database on the left").weak());
        return;
    };
    ui.horizontal(|ui| {
        ui.heading(if schema.is_empty() {
            db.clone()
        } else {
            format!("{db}.{schema}")
        });
        if s.engine().has_schemas() && schema.is_empty() {
            ui.label(RichText::new("choose a schema on the left").weak());
            return;
        }
        if act_button(ui, !s.read_only, Act::Add, &il(icons::PLUS, "New table…")).clicked() {
            send(
                out,
                DbMsg::NewTable {
                    id,
                    database: db.clone(),
                    schema: schema.clone(),
                },
            );
        }
    });
    let Some(list) = s.current_tables() else {
        ui.spinner();
        return;
    };
    TableBuilder::new(ui)
        .id_salt(("db-tables", id))
        .striped(true)
        .resizable(true)
        .column(TCol::initial(240.0).at_least(80.0).clip(true))
        .column(TCol::initial(90.0))
        .column(TCol::initial(90.0))
        .column(TCol::initial(130.0).clip(true))
        .column(TCol::remainder().clip(true))
        .header(ROW + 2.0, |mut h| {
            for t in ["Name", "Rows (est.)", "Size", "Engine / kind", "Comment"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(ROW, list.len(), |mut row| {
                let t = &list[row.index()];
                let table = TableRef {
                    database: db.clone(),
                    schema: schema.clone(),
                    table: t.name.clone(),
                };
                row.col(|ui| {
                    let icon = if t.kind == TableKind::View {
                        icons::EYE
                    } else {
                        icons::TABLE
                    };
                    let resp = ui.link(format!("{icon} {}", t.name));
                    if resp.clicked() {
                        send(
                            out,
                            DbMsg::SelectTable {
                                id,
                                table: table.clone(),
                            },
                        );
                    }
                    resp.context_menu(|ui| table_menu(ui, s, &table, t.kind, out));
                });
                row.col(|ui| {
                    ui.label(t.rows.map(|n| n.to_string()).unwrap_or_default());
                });
                row.col(|ui| {
                    ui.label(t.size.map(human_size).unwrap_or_default());
                });
                row.col(|ui| {
                    ui.label(&t.engine);
                });
                row.col(|ui| {
                    ui.label(RichText::new(&t.comment).weak());
                });
            });
        });
}

fn default_text(d: &ColumnDefault) -> String {
    match d {
        ColumnDefault::None => String::new(),
        ColumnDefault::Null => "NULL".into(),
        ColumnDefault::Text(t) => format!("'{t}'"),
        ColumnDefault::Expression(e) => e.clone(),
    }
}

fn table_tab(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    if d.new_table.is_some() {
        new_table(ui, s, d, out);
        return;
    }
    let Some(info) = &s.info else {
        if s.data.table.is_some() {
            ui.spinner();
        } else {
            ui.label(RichText::new("Choose a table on the left").weak());
        }
        return;
    };
    let t = &info.table;
    ui.horizontal_wrapped(|ui| {
        ui.heading(&t.table);
        if info.kind == TableKind::View {
            ui.label(RichText::new("view").weak());
        }
        let writable = !s.read_only && !s.ddl_running;
        if d.structure.is_none()
            && info.kind == TableKind::Table
            && act_button(ui, writable, Act::Edit, &il(icons::EDIT, "Edit structure")).clicked()
        {
            send(out, DbMsg::StructureEdit(id));
        }
        match &mut d.rename {
            Some(name) => {
                ui.add(egui::TextEdit::singleline(name).desired_width(180.0));
                if ui.button(il(icons::CHECK, "Rename")).clicked() {
                    send(out, DbMsg::RenameSave(id));
                }
                if ui.button("Cancel").clicked() {
                    send(out, DbMsg::RenameCancel(id));
                }
            }
            None => {
                if act_button(ui, writable, Act::Edit, &il(icons::EDIT, "Rename…")).clicked() {
                    send(out, DbMsg::RenameTable(id));
                }
            }
        }
        if info.kind == TableKind::Table
            && ui
                .add_enabled(writable, egui::Button::new(il(icons::MINUS, "Empty…")))
                .clicked()
        {
            send(
                out,
                DbMsg::TruncateTable {
                    id,
                    table: t.clone(),
                },
            );
        }
        if act_button(ui, writable, Act::Danger, &il(icons::TRASH, "Drop…")).clicked() {
            send(
                out,
                DbMsg::DropTable {
                    id,
                    table: t.clone(),
                },
            );
        }
        if s.ddl_running {
            ui.spinner();
        }
    });
    ui.separator();
    if let Some(edit) = &mut d.structure {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!s.ddl_running, egui::Button::new(il(icons::SAVE, "Save…")))
                .on_hover_text("Shows the SQL first")
                .clicked()
            {
                send(out, DbMsg::StructureSave(id));
            }
            if ui.button(il(icons::UNDO, "Discard")).clicked() {
                send(out, DbMsg::StructureDiscard(id));
            }
        });
        egui::ScrollArea::both()
            .id_salt(("db-structure", id))
            .auto_shrink([false, false])
            .show(ui, |ui| structure_editor(ui, s.engine(), edit, true));
        return;
    }
    egui::ScrollArea::both()
        .id_salt(("db-structure-view", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.strong("Columns");
            egui::Grid::new(("db-cols", id))
                .striped(true)
                .num_columns(9)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    for h in [
                        "#",
                        "Name",
                        "Type",
                        "Length / set",
                        "Unsigned",
                        "NULL",
                        "Default",
                        "Extra",
                        "Comment",
                    ] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    let key = info.row_key().unwrap_or_default();
                    for (i, c) in info.columns.iter().enumerate() {
                        ui.label(RichText::new((i + 1).to_string()).weak());
                        let mut name = RichText::new(&c.name).monospace();
                        if key.contains(&c.name) {
                            name = name.strong();
                        }
                        ui.label(name);
                        ui.label(RichText::new(&c.type_name).monospace());
                        ui.label(RichText::new(&c.length).monospace());
                        ui.label(if c.unsigned { icons::CHECK } else { "" });
                        ui.label(if c.nullable { icons::CHECK } else { "" });
                        ui.label(RichText::new(default_text(&c.default)).monospace());
                        ui.label(if c.auto_increment {
                            "auto increment"
                        } else {
                            ""
                        });
                        ui.label(RichText::new(&c.comment).weak());
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            ui.strong("Indexes");
            for i in &info.indexes {
                ui.label(
                    RichText::new(format!(
                        "{} {} ({})",
                        i.kind.title(),
                        i.name,
                        i.columns.join(", ")
                    ))
                    .monospace(),
                );
            }
            if info.indexes.is_empty() {
                ui.label(RichText::new("none").weak());
            }
            ui.add_space(8.0);
            ui.strong("Foreign keys");
            for f in &info.foreign_keys {
                ui.label(
                    RichText::new(format!(
                        "{} ({}) → {}.{} ({})  ON UPDATE {}  ON DELETE {}",
                        f.name,
                        f.columns.join(", "),
                        f.ref_schema,
                        f.ref_table,
                        f.ref_columns.join(", "),
                        f.on_update,
                        f.on_delete
                    ))
                    .monospace(),
                );
            }
            if info.foreign_keys.is_empty() {
                ui.label(RichText::new("none").weak());
            }
            if !info.create_sql.is_empty() {
                ui.add_space(8.0);
                egui::CollapsingHeader::new("CREATE statement")
                    .id_salt(("db-create", id))
                    .show(ui, |ui| {
                        let mut text = info.create_sql.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .interactive(true),
                        );
                    });
            }
        });
}

fn new_table(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some(nt) = &mut d.new_table else {
        return;
    };
    ui.horizontal(|ui| {
        let place = if nt.schema.is_empty() {
            nt.database.clone()
        } else {
            format!("{}.{}", nt.database, nt.schema)
        };
        ui.heading(format!("New table in {place}"));
    });
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut nt.name).desired_width(220.0));
        if ui
            .add_enabled(
                !s.ddl_running,
                egui::Button::new(il(icons::SAVE, "Create…")),
            )
            .on_hover_text("Shows the SQL first")
            .clicked()
        {
            send(out, DbMsg::NewTableSave(id));
        }
        if ui.button("Cancel").clicked() {
            send(out, DbMsg::NewTableCancel(id));
        }
    });
    ui.separator();
    let engine = s.engine();
    egui::ScrollArea::both()
        .id_salt(("db-new-table", id))
        .auto_shrink([false, false])
        .show(ui, |ui| structure_editor(ui, engine, &mut nt.edit, true));
}

/// Columns / indexes / foreign keys editor (Table tab, new table).
fn structure_editor(ui: &mut egui::Ui, engine: Engine, edit: &mut TableEdit, allow_move: bool) {
    let my = engine == Engine::MySql;
    ui.horizontal(|ui| {
        ui.strong("Columns");
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add column")).clicked() {
            let n = edit.columns.len() + 1;
            edit.columns.push(TableEdit::blank_column(engine, n));
        }
        if !my {
            ui.label(
                RichText::new("PostgreSQL cannot reorder columns")
                    .weak()
                    .small(),
            );
        }
    });
    let mut remove = None;
    let mut swap = None;
    let count = edit.columns.len();
    egui::Grid::new("db-edit-cols")
        .striped(true)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            let mut head = vec!["", "Name", "Type", "Length / set"];
            if my {
                head.push("Unsigned");
            }
            head.extend(["NULL", "Default", "", "Auto inc."]);
            if my {
                head.push("Collation");
            }
            head.push("Comment");
            for h in head {
                ui.strong(h);
            }
            ui.end_row();
            for (i, ce) in edit.columns.iter_mut().enumerate() {
                let c = &mut ce.column;
                ui.horizontal(|ui| {
                    if allow_move && my {
                        if ui
                            .add_enabled(i > 0, egui::Button::new(icons::CARET_UP).small())
                            .clicked()
                        {
                            swap = Some((i, i - 1));
                        }
                        if ui
                            .add_enabled(
                                i + 1 < count,
                                egui::Button::new(icons::CARET_DOWN).small(),
                            )
                            .clicked()
                        {
                            swap = Some((i, i + 1));
                        }
                    }
                    if act_small(ui, true, Act::Danger, icons::TRASH)
                        .on_hover_text("Remove column")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
                ui.add_sized([130.0, 22.0], egui::TextEdit::singleline(&mut c.name));
                ui.add_sized([120.0, 22.0], egui::TextEdit::singleline(&mut c.type_name));
                ui.add_sized([80.0, 22.0], egui::TextEdit::singleline(&mut c.length));
                if my {
                    ui.checkbox(&mut c.unsigned, "");
                }
                ui.checkbox(&mut c.nullable, "");
                let kind = match &c.default {
                    ColumnDefault::None => "No default",
                    ColumnDefault::Null => "NULL",
                    ColumnDefault::Text(_) => "Text",
                    ColumnDefault::Expression(_) => "Expression",
                };
                egui::ComboBox::from_id_salt(("db-default", i))
                    .selected_text(kind)
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        let text = match &c.default {
                            ColumnDefault::Text(t) | ColumnDefault::Expression(t) => t.clone(),
                            _ => String::new(),
                        };
                        if ui
                            .selectable_label(kind == "No default", "No default")
                            .clicked()
                        {
                            c.default = ColumnDefault::None;
                        }
                        if ui.selectable_label(kind == "NULL", "NULL").clicked() {
                            c.default = ColumnDefault::Null;
                        }
                        if ui.selectable_label(kind == "Text", "Text").clicked() {
                            c.default = ColumnDefault::Text(text.clone());
                        }
                        if ui
                            .selectable_label(kind == "Expression", "Expression")
                            .on_hover_text("SQL written as is, e.g. CURRENT_TIMESTAMP")
                            .clicked()
                        {
                            c.default = ColumnDefault::Expression(text);
                        }
                    });
                match &mut c.default {
                    ColumnDefault::Text(t) | ColumnDefault::Expression(t) => {
                        ui.add_sized([120.0, 22.0], egui::TextEdit::singleline(t));
                    }
                    _ => {
                        ui.label("");
                    }
                }
                ui.checkbox(&mut c.auto_increment, "");
                if my {
                    ui.add_sized([110.0, 22.0], egui::TextEdit::singleline(&mut c.collation));
                }
                ui.add_sized([160.0, 22.0], egui::TextEdit::singleline(&mut c.comment));
                ui.end_row();
            }
        });
    if let Some((a, b)) = swap {
        edit.columns.swap(a, b);
    }
    if let Some(i) = remove {
        edit.columns.remove(i);
    }
    let names: Vec<String> = edit.columns.iter().map(|c| c.column.name.clone()).collect();

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.strong("Indexes");
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add index")).clicked() {
            edit.indexes.push(IndexEdit {
                original: None,
                index: telepad_core::db::Index {
                    name: format!("idx_{}", edit.indexes.len() + 1),
                    kind: IndexKind::Key,
                    columns: vec![],
                    constraint: false,
                },
            });
        }
    });
    let mut remove = None;
    egui::Grid::new("db-edit-idx")
        .striped(true)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for (i, ie) in edit.indexes.iter_mut().enumerate() {
                let ix = &mut ie.index;
                if act_small(ui, true, Act::Danger, icons::TRASH)
                    .on_hover_text("Remove index")
                    .clicked()
                {
                    remove = Some(i);
                }
                egui::ComboBox::from_id_salt(("db-idx-kind", i))
                    .selected_text(ix.kind.title())
                    .show_ui(ui, |ui| {
                        for k in IndexKind::ALL {
                            if k == IndexKind::Fulltext && !my {
                                continue;
                            }
                            ui.selectable_value(&mut ix.kind, k, k.title());
                        }
                    });
                ui.add_enabled_ui(ix.kind != IndexKind::Primary || !my, |ui| {
                    ui.add_sized([140.0, 22.0], egui::TextEdit::singleline(&mut ix.name))
                });
                column_picker(ui, ("db-idx-cols", i), &names, &mut ix.columns);
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        edit.indexes.remove(i);
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.strong("Foreign keys");
        if act_button(ui, true, Act::Add, &il(icons::PLUS, "Add foreign key")).clicked() {
            edit.foreign_keys.push(FkEdit {
                original: None,
                fk: telepad_core::db::ForeignKey {
                    name: format!("fk_{}", edit.foreign_keys.len() + 1),
                    on_update: "RESTRICT".into(),
                    on_delete: "RESTRICT".into(),
                    ..Default::default()
                },
            });
        }
    });
    let mut remove = None;
    egui::Grid::new("db-edit-fk")
        .striped(true)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            if !edit.foreign_keys.is_empty() {
                for h in [
                    "",
                    "Name",
                    "Columns",
                    "References (schema / table)",
                    "Columns there",
                    "ON UPDATE",
                    "ON DELETE",
                ] {
                    ui.strong(h);
                }
                ui.end_row();
            }
            for (i, fe) in edit.foreign_keys.iter_mut().enumerate() {
                let fk = &mut fe.fk;
                if act_small(ui, true, Act::Danger, icons::TRASH)
                    .on_hover_text("Remove foreign key")
                    .clicked()
                {
                    remove = Some(i);
                }
                ui.add_sized([120.0, 22.0], egui::TextEdit::singleline(&mut fk.name));
                column_picker(ui, ("db-fk-cols", i), &names, &mut fk.columns);
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [90.0, 22.0],
                        egui::TextEdit::singleline(&mut fk.ref_schema).hint_text("same"),
                    );
                    ui.add_sized(
                        [120.0, 22.0],
                        egui::TextEdit::singleline(&mut fk.ref_table).hint_text("table"),
                    );
                });
                list_edit(ui, egui::Id::unique(("db-fk-ref", i)), &mut fk.ref_columns);
                for (n, action) in [("upd", &mut fk.on_update), ("del", &mut fk.on_delete)] {
                    egui::ComboBox::from_id_salt(("db-fk-act", i, n))
                        .selected_text(action.as_str())
                        .show_ui(ui, |ui| {
                            for a in FK_ACTIONS {
                                ui.selectable_value(action, a.to_string(), a);
                            }
                        });
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        edit.foreign_keys.remove(i);
    }
}

/// Pick columns for an index / key: ticked in click order.
fn column_picker(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    names: &[String],
    cols: &mut Vec<String>,
) {
    let text = if cols.is_empty() {
        "choose columns".to_string()
    } else {
        cols.join(", ")
    };
    ui.push_id(salt, |ui| {
        ui.menu_button(format!("{text} {}", icons::CARET_DOWN), |ui| {
            for n in names {
                let mut on = cols.contains(n);
                if ui.checkbox(&mut on, n).changed() {
                    if on {
                        cols.push(n.clone());
                    } else {
                        cols.retain(|c| c != n);
                    }
                }
            }
        });
    });
}

/// Comma-separated list in a text box; the raw text is kept while typing.
fn list_edit(ui: &mut egui::Ui, id: egui::Id, list: &mut Vec<String>) {
    let mut raw: String = ui
        .data_mut(|m| m.get_temp::<String>(id))
        .unwrap_or_else(|| list.join(", "));
    if ui
        .add_sized(
            [120.0, 22.0],
            egui::TextEdit::singleline(&mut raw).hint_text("a, b"),
        )
        .changed()
    {
        *list = raw
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
    }
    ui.data_mut(|m| m.insert_temp(id, raw));
}

/// What a cell shows: text (one line, cut) and whether it is NULL.
fn shown(c: &Cell) -> (String, bool) {
    match c {
        Cell::Null => ("NULL".into(), true),
        Cell::Text(t) => (cut(t), false),
        Cell::Long(t, n) => (format!("{} … ({})", cut(t), human_size(*n as u64)), false),
        Cell::Bytes(h, n) => (format!("{} ({})", cut(h), human_size(*n as u64)), false),
    }
}

fn cut(t: &str) -> String {
    let line = t.lines().next().unwrap_or("");
    let mut s: String = line.chars().take(SHOWN).collect();
    if s.len() < t.len() {
        s.push('…');
    }
    s
}

fn data_tab(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some(_t) = &s.data.table else {
        ui.label(RichText::new("Choose a table on the left").weak());
        return;
    };
    let pending = !s.data.edits.is_empty();
    let blocked = s.edit_blocked();
    ui.horizontal_wrapped(|ui| {
        let f = ui.add(
            egui::TextEdit::singleline(&mut d.data_filter)
                .hint_text(format!("{} WHERE …  e.g. id > 100", icons::FUNNEL))
                .desired_width(280.0),
        );
        if f.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            send(out, DbMsg::DataApplyFilter(id));
        }
        if ui.button("Filter").clicked() {
            send(out, DbMsg::DataApplyFilter(id));
        }
        if !s.data.filter.is_empty() && ui.button("Clear").clicked() {
            d.data_filter.clear();
            send(out, DbMsg::DataApplyFilter(id));
        }
        ui.separator();
        let rows = s.data.grid.as_ref().map_or(0, |g| g.rows.len()) as u64;
        if ui
            .add_enabled(s.data.offset > 0, egui::Button::new(icons::CARET_LEFT))
            .on_hover_text("Previous page")
            .clicked()
        {
            send(out, DbMsg::DataPage { id, forward: false });
        }
        ui.label(if rows == 0 {
            "no rows".to_string()
        } else {
            format!("rows {}–{}", s.data.offset + 1, s.data.offset + rows)
        });
        if ui
            .add_enabled(rows >= s.data.limit, egui::Button::new(icons::CARET_RIGHT))
            .on_hover_text("Next page")
            .clicked()
        {
            send(out, DbMsg::DataPage { id, forward: true });
        }
        egui::ComboBox::from_id_salt(("db-limit", id))
            .selected_text(format!("{} / page", s.data.limit))
            .show_ui(ui, |ui| {
                for n in PAGE_SIZES {
                    if ui
                        .selectable_label(s.data.limit == n, format!("{n}"))
                        .clicked()
                    {
                        send(out, DbMsg::DataLimit { id, limit: n });
                    }
                }
            });
        if ui.button(icons::REFRESH).on_hover_text("Reload").clicked() {
            send(out, DbMsg::DataReload(id));
        }
        if s.data.loading {
            ui.spinner();
            if ui.button(il(icons::STOP, "Cancel")).clicked() {
                send(out, DbMsg::Cancel(id));
            }
        }
        ui.separator();
        let editable = blocked.is_none() && s.data.grid.is_some();
        if act_button(ui, editable, Act::Add, &il(icons::ROW_PLUS, "Insert row")).clicked() {
            send(out, DbMsg::InsertRow(id));
        }
        if act_button(
            ui,
            editable && d.selected_row.is_some(),
            Act::Danger,
            &il(icons::TRASH, "Delete row"),
        )
        .on_hover_text("Marks the selected row; again to unmark")
        .clicked()
            && let Some(row) = d.selected_row
        {
            send(out, DbMsg::DeleteRow { id, row });
        }
        let post = if pending {
            format!("Post ({})", s.data.edits.count())
        } else {
            "Post".into()
        };
        if ui
            .add_enabled(pending, egui::Button::new(il(icons::SAVE, &post)))
            .on_hover_text("Shows the SQL, then saves all changes in one transaction")
            .clicked()
        {
            send(out, DbMsg::Post(id));
        }
        if ui
            .add_enabled(pending, egui::Button::new(il(icons::UNDO, "Discard")))
            .clicked()
        {
            send(out, DbMsg::Discard(id));
        }
    });
    if let Some(why) = &blocked {
        ui.label(
            RichText::new(format!("{} {why}", icons::LOCK))
                .weak()
                .small(),
        );
    }
    if let Some(e) = &s.data.error {
        ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
    }
    let Some(grid) = &s.data.grid else {
        if s.data.loading {
            ui.spinner();
        }
        return;
    };
    data_grid(ui, s, d, grid, blocked.is_none(), out);
}

fn data_grid(
    ui: &mut egui::Ui,
    s: &DbSession,
    d: &mut DbDraft,
    grid: &Grid,
    editable: bool,
    out: &mut Vec<Msg>,
) {
    let id = s.id;
    let existing = grid.rows.len();
    let total = existing + s.data.edits.inserted.len();
    let changed_fill = ui.visuals().selection.bg_fill.gamma_multiply(0.45);
    let deleted_color = red();
    let ncols = grid.columns.len();
    egui::ScrollArea::horizontal()
        .id_salt(("db-grid-h", id))
        .show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt((
                    "db-grid",
                    id,
                    s.data.table.as_ref().map(|t| t.table.clone()),
                ))
                .striped(true)
                .resizable(true)
                .auto_shrink([false, false])
                .column(TCol::exact(52.0))
                .columns(TCol::initial(140.0).at_least(40.0).clip(true), ncols)
                .header(ROW + 2.0, |mut h| {
                    h.col(|ui| {
                        ui.label(RichText::new("#").weak());
                    });
                    for c in &grid.columns {
                        h.col(|ui| {
                            let arrow = match &s.data.sort {
                                Some((n, false)) if n == c => format!(" {}", icons::CARET_UP),
                                Some((n, true)) if n == c => format!(" {}", icons::CARET_DOWN),
                                _ => String::new(),
                            };
                            let resp = ui
                                .add(
                                    egui::Label::new(RichText::new(format!("{c}{arrow}")).strong())
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text("Sort: ascending → descending → off");
                            if resp.clicked() {
                                send(
                                    out,
                                    DbMsg::DataSort {
                                        id,
                                        column: c.clone(),
                                    },
                                );
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW, total, |mut row| {
                        let i = row.index();
                        let r = if i < existing {
                            RowRef::Existing(i)
                        } else {
                            RowRef::New(i - existing)
                        };
                        let deleted =
                            matches!(r, RowRef::Existing(x) if s.data.edits.deleted.contains(&x));
                        let selected = d.selected_row == Some(r);
                        row.set_selected(selected);
                        row.col(|ui| {
                            let label = match r {
                                RowRef::Existing(x) => (s.data.offset + x as u64 + 1).to_string(),
                                RowRef::New(_) => format!("{} new", icons::PLUS),
                            };
                            let mut t = RichText::new(label).weak();
                            if deleted {
                                t = t.strikethrough().color(deleted_color);
                            }
                            if ui
                                .add(egui::Label::new(t).sense(egui::Sense::click()))
                                .clicked()
                            {
                                d.selected_row = Some(r);
                            }
                        });
                        for c in 0..ncols {
                            row.col(|ui| {
                                cell(ui, s, d, r, c, deleted, editable, changed_fill, out);
                            });
                        }
                    });
                });
        });
}

#[allow(clippy::too_many_arguments)]
fn cell(
    ui: &mut egui::Ui,
    s: &DbSession,
    d: &mut DbDraft,
    r: RowRef,
    c: usize,
    deleted: bool,
    editable: bool,
    changed_fill: Color32,
    out: &mut Vec<Msg>,
) {
    let id = s.id;
    if let Some((er, ec, text, null)) = &mut d.cell
        && *er == r
        && *ec == c
    {
        let mut output = egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .show(ui);
        let resp = output.response.response.clone();
        if !resp.has_focus() && !resp.lost_focus() {
            // Start with the whole value selected, as HeidiSQL does.
            resp.request_focus();
            output
                .state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::select_all_str(text)));
            output.state.store(ui.ctx(), resp.id);
        }
        if *null && resp.changed() {
            *null = false;
        }
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc {
            d.cell = None;
        } else if resp.lost_focus() {
            let value = if *null { None } else { Some(text.clone()) };
            send(
                out,
                DbMsg::SetCell {
                    id,
                    row: r,
                    col: c,
                    value,
                },
            );
            d.cell = None;
        }
        return;
    }
    let value = s.cell_value(r, c);
    let changed = match r {
        RowRef::Existing(x) => s.data.edits.changed.contains_key(&(x, c)),
        RowRef::New(_) => value.is_some(),
    };
    if changed {
        ui.painter().rect_filled(ui.max_rect(), 0.0, changed_fill);
    }
    let (text, null) = match &value {
        Some(v) => shown(v),
        None => ("(default)".into(), true),
    };
    let mut rt = RichText::new(text).monospace();
    if null {
        rt = rt.weak().italics();
    }
    if deleted {
        rt = rt.strikethrough().color(red());
    }
    let resp = ui.add(egui::Label::new(rt).truncate().sense(egui::Sense::click()));
    if resp.clicked() {
        d.selected_row = Some(r);
    }
    let can_edit =
        editable && !deleted && s.column_editable(c) && value.as_ref().is_none_or(Cell::editable);
    if resp.double_clicked() && can_edit {
        let text = value
            .as_ref()
            .and_then(|v| v.as_value())
            .unwrap_or("")
            .to_string();
        d.cell = Some((r, c, text, false));
    }
    resp.context_menu(|ui| {
        let full = value
            .as_ref()
            .and_then(|v| v.as_value())
            .map(str::to_string);
        if ui
            .add_enabled(
                can_edit,
                egui::Button::new(act_text(Act::Edit, &il(icons::EDIT, "Edit"))),
            )
            .clicked()
        {
            d.cell = Some((r, c, full.clone().unwrap_or_default(), false));
            ui.close();
        }
        if ui
            .add_enabled(can_edit, egui::Button::new("Set NULL"))
            .clicked()
        {
            send(
                out,
                DbMsg::SetCell {
                    id,
                    row: r,
                    col: c,
                    value: None,
                },
            );
            ui.close();
        }
        if ui
            .add_enabled(
                full.is_some(),
                egui::Button::new(il(icons::COPY, "Copy value")),
            )
            .clicked()
        {
            ui.ctx().copy_text(full.clone().unwrap_or_default());
            ui.close();
        }
        if ui
            .add_enabled(
                full.is_some(),
                egui::Button::new(il(icons::EYE, "View value…")),
            )
            .clicked()
        {
            let col = s
                .data
                .grid
                .as_ref()
                .and_then(|g| g.columns.get(c))
                .cloned()
                .unwrap_or_default();
            let mut text = full.unwrap_or_default();
            if let Some(Cell::Long(_, n) | Cell::Bytes(_, n)) = &value {
                text.push_str(&format!(
                    "\n\n… cut: the value has {} in total",
                    human_size(*n as u64)
                ));
            }
            d.viewer = Some((col, text));
            ui.close();
        }
    });
}

fn viewer(ui: &mut egui::Ui, d: &mut DbDraft) {
    let Some((title, text)) = &d.viewer else {
        return;
    };
    let mut open = true;
    let mut text = text.clone();
    egui::Window::new(format!("Value: {title}"))
        .open(&mut open)
        .default_size([620.0, 420.0])
        .show(ui.ctx(), |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
            });
        });
    if !open {
        d.viewer = None;
    }
}

/// Char index → byte index in `s`.
fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(b, _)| b)
}

fn query_tab(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    while d.queries.len() < s.queries.len() {
        d.queries.push(String::new());
    }
    ui.horizontal(|ui| {
        for i in 0..s.queries.len() {
            let mut t = format!("Query {}", i + 1);
            if s.queries[i].running {
                t.push_str(" …");
            }
            if ui.selectable_label(s.query_tab == i, t).clicked() {
                send(out, DbMsg::QuerySelect { id, tab: i });
            }
        }
        if ui
            .button(icons::PLUS)
            .on_hover_text("New query tab")
            .clicked()
        {
            send(out, DbMsg::QueryNew(id));
        }
        if s.queries.len() > 1
            && ui
                .button(icons::X_CIRCLE)
                .on_hover_text("Close this query tab")
                .clicked()
        {
            send(
                out,
                DbMsg::QueryClose {
                    id,
                    tab: s.query_tab,
                },
            );
        }
    });
    let tab = s.query_tab.min(s.queries.len().saturating_sub(1));
    let Some(q) = s.queries.get(tab) else {
        return;
    };
    let context = match &s.current {
        Some((db, schema)) if !schema.is_empty() => format!("runs in {db}, schema {schema}"),
        Some((db, _)) => format!("runs in {db}"),
        None => "no database chosen (choose one on the left)".into(),
    };
    let text = &mut d.queries[tab];
    let height = (ui.available_height() * 0.42).max(120.0);
    let output = egui::ScrollArea::vertical()
        .id_salt(("db-query-edit", id, tab))
        .max_height(height)
        .show(ui, |ui| {
            egui::TextEdit::multiline(text)
                .code_editor()
                .desired_rows(10)
                .desired_width(f32::INFINITY)
                .hint_text("SELECT * FROM …;  F9 / Ctrl+Enter: run (selection, else all) · Shift+F9 / Ctrl+Shift+Enter: current statement")
                .show(ui)
        })
        .inner;
    let (sel_start, sel_end, cursor) = match output
        .cursor_range
        .or_else(|| output.state.cursor.char_range())
    {
        Some(r) => {
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            (
                byte_at(text, a.min(b)),
                byte_at(text, a.max(b)),
                byte_at(text, r.primary.index.0),
            )
        }
        None => (0, 0, text.len()),
    };
    let focused = output.response.has_focus();
    let (run_key, current_key) = ui.input(|i| {
        let enter = i.key_pressed(egui::Key::Enter) && i.modifiers.command;
        let run = i.key_pressed(egui::Key::F9) || enter;
        (run && !i.modifiers.shift, run && i.modifiers.shift)
    });
    let selection = (sel_end > sel_start).then(|| text[sel_start..sel_end].to_string());
    let current = sql::statement_at(s.engine(), text, cursor).map(|r| text[r].to_string());
    let all = text.clone();
    ui.horizontal_wrapped(|ui| {
        let idle = !q.running;
        if ui
            .add_enabled(idle, egui::Button::new(il(icons::PLAY, "Run")))
            .on_hover_text("The selection, else everything (F9, Ctrl+Enter)")
            .clicked()
            || (idle && focused && run_key)
        {
            send(
                out,
                DbMsg::QueryRun {
                    id,
                    text: selection.clone().unwrap_or(all.clone()),
                },
            );
        }
        if (ui
            .add_enabled(
                idle && current.is_some(),
                egui::Button::new(il(icons::PLAY, "Run current statement")),
            )
            .on_hover_text("The statement at the cursor (Shift+F9, Ctrl+Shift+Enter)")
            .clicked()
            || (idle && focused && current_key))
            && let Some(c) = &current
        {
            send(
                out,
                DbMsg::QueryRun {
                    id,
                    text: c.clone(),
                },
            );
        }
        if q.running {
            ui.spinner();
            if ui.button(il(icons::STOP, "Cancel")).clicked() {
                send(out, DbMsg::Cancel(id));
            }
        }
        ui.label(RichText::new(context).weak());
        if s.read_only {
            ui.label(RichText::new(format!("{} read-only", icons::LOCK)).weak());
        }
    });
    ui.separator();
    if q.results.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (i, r) in q.results.iter().enumerate() {
            let label = match (&r.error, &r.grid, r.affected) {
                (Some(_), _, _) => format!("{} Error", icons::X_CIRCLE),
                (None, Some(g), _) => format!("Result {} ({} rows)", i + 1, g.rows.len()),
                (None, None, Some(n)) => format!("Result {} ({n} affected)", i + 1),
                _ => format!("Result {}", i + 1),
            };
            if ui.selectable_label(q.shown == i, label).clicked() {
                send(out, DbMsg::QueryResultTab { id, shown: i });
            }
        }
    });
    let Some(r) = q.results.get(q.shown.min(q.results.len() - 1)) else {
        return;
    };
    ui.label(RichText::new(first_line(&r.statement)).monospace().weak());
    if let Some(e) = &r.error {
        ui.colored_label(red(), e);
        return;
    }
    ui.label(match (&r.grid, r.affected) {
        (Some(g), _) => format!(
            "{} row(s){} · {} ms",
            g.rows.len(),
            if g.truncated {
                " shown (more were cut)"
            } else {
                ""
            },
            r.millis
        ),
        (None, Some(n)) => format!("{n} row(s) affected · {} ms", r.millis),
        _ => format!("{} ms", r.millis),
    });
    if let Some(g) = &r.grid {
        result_grid(ui, ("db-result", id, tab, q.shown), g, d);
    }
}

fn result_grid(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    g: &Grid,
    d: &mut DbDraft,
) {
    let n = g.columns.len();
    egui::ScrollArea::horizontal()
        .id_salt(("h", &salt))
        .show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt(salt)
                .striped(true)
                .resizable(true)
                .auto_shrink([false, false])
                .columns(TCol::initial(140.0).at_least(40.0).clip(true), n)
                .header(ROW + 2.0, |mut h| {
                    for c in &g.columns {
                        h.col(|ui| {
                            ui.strong(c);
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW, g.rows.len(), |mut row| {
                        let cells = &g.rows[row.index()];
                        for (ci, c) in cells.iter().enumerate() {
                            row.col(|ui| {
                                let (t, null) = shown(c);
                                let mut rt = RichText::new(t).monospace();
                                if null {
                                    rt = rt.weak().italics();
                                }
                                let resp = ui.add(
                                    egui::Label::new(rt).truncate().sense(egui::Sense::click()),
                                );
                                resp.context_menu(|ui| {
                                    if let Some(v) = c.as_value() {
                                        if ui.button(il(icons::COPY, "Copy value")).clicked() {
                                            ui.ctx().copy_text(v.to_string());
                                            ui.close();
                                        }
                                        if ui.button(il(icons::EYE, "View value…")).clicked() {
                                            d.viewer = Some((
                                                g.columns.get(ci).cloned().unwrap_or_default(),
                                                v.to_string(),
                                            ));
                                            ui.close();
                                        }
                                    }
                                });
                            });
                        }
                    });
                });
        });
}

/// The confirmation shown before anything changes: exact SQL, Run / Cancel.
fn confirm(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some(c) = &s.confirm else {
        return;
    };
    let engine = s.engine();
    let (title, reasons, sql_text, type_name): (String, Vec<String>, String, Option<&String>) = match c {
        DbConfirm::Query { reasons, statements, .. } => (
            "Run these statements?".into(),
            reasons.clone(),
            statements.join(";\n\n"),
            None,
        ),
        DbConfirm::Post { statements } => (
            format!("Save {} change(s)?", statements.len()),
            vec!["Runs in one transaction; each statement must change exactly one row, or nothing is saved".into()],
            statements.iter().map(|st| st.display(engine)).collect::<Vec<_>>().join(";\n"),
            None,
        ),
        DbConfirm::Ddl { title, statements, type_name, .. } => (
            format!("{title}?"),
            match engine {
                Engine::Postgres => vec!["Runs in one transaction".into()],
                Engine::MySql => vec!["MySQL commits each structure statement on its own".into()],
            },
            statements.join(";\n\n"),
            type_name.as_ref(),
        ),
    };
    let modal = egui::Modal::new(egui::Id::unique(("db-confirm", id))).show(ui.ctx(), |ui| {
        ui.set_width(640.0);
        ui.heading(RichText::new(format!("{} {title}", icons::WARNING)).color(red()));
        for r in &reasons {
            ui.label(r);
        }
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                let mut t = sql_text.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut t)
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
            });
        let mut ready = true;
        if let Some(name) = type_name {
            ui.horizontal(|ui| {
                ui.label(format!("Type {name} to confirm:"));
                let f = ui.add(egui::TextEdit::singleline(&mut d.typed).desired_width(200.0));
                if d.typed.is_empty() && !f.has_focus() {
                    f.request_focus();
                }
            });
            ready = d.typed.trim() == name;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if act_button(ui, ready, Act::Danger, &il(icons::PLAY, "Run")).clicked() {
                send(out, DbMsg::ConfirmOk(id));
            }
            if ui.button("Cancel").clicked() {
                send(out, DbMsg::ConfirmCancel(id));
            }
        });
    });
    if modal.should_close() {
        send(out, DbMsg::ConfirmCancel(id));
    }
}

/// SQL log (#100, HeidiSQL style): what this connection changed, newest
/// last, also from earlier sessions.
fn sql_log(ui: &mut egui::Ui, s: &DbSession, d: &mut DbDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let shown: Vec<_> = s
        .log
        .iter()
        .filter(|e| telepad_app::db::log_matches(e, &d.log_filter))
        .collect();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{} SQL log ({})", icons::CODE, s.log.len())).strong())
            .on_hover_text(
                "Statements that changed data or structure, as they ran (passwords masked)",
            );
        ui.add(
            egui::TextEdit::singleline(&mut d.log_filter)
                .hint_text(format!("{} Filter", icons::SEARCH))
                .desired_width(160.0),
        );
        ui.checkbox(&mut d.log_select, "Log SELECT too")
            .on_hover_text("Also log read-only statements of the Query tab from now on");
        if ui
            .button(il(icons::COPY, "Copy all"))
            .on_hover_text("The SQL of the lines shown")
            .clicked()
        {
            let text = shown
                .iter()
                .map(|e| {
                    format!(
                        "-- {} {}\n{};",
                        crate::views::files::when(e.at),
                        e.title,
                        e.detail
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            ui.ctx().copy_text(text);
        }
        if ui.button(il(icons::EXPORT, "Export…")).clicked() {
            send(out, DbMsg::LogExport(id));
        }
        if d.log_clear {
            ui.label("Clear this connection's log?");
            if act_button(ui, true, Act::Danger, &il(icons::TRASH, "Clear")).clicked() {
                send(out, DbMsg::LogClear(id));
            }
            if ui.button("Cancel").clicked() {
                d.log_clear = false;
            }
        } else if act_button(
            ui,
            !s.log.is_empty(),
            Act::Danger,
            &il(icons::TRASH, "Clear…"),
        )
        .clicked()
        {
            d.log_clear = true;
        }
    });
    egui::ScrollArea::vertical()
        .id_salt(("db-log-scroll", id))
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if shown.is_empty() {
                ui.label(RichText::new("Nothing logged yet: grid posts, structure changes and statements of the Query tab that change data show here.").weak());
            }
            for e in shown {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(crate::views::files::when(e.at)).weak().small().monospace());
                    let (text, color) = crate::views::files::outcome_text(e.outcome);
                    ui.colored_label(color, text);
                    if let Some(r) = e.rows {
                        ui.label(RichText::new(format!("{r} row(s)")).weak().small());
                    }
                    if let Some(ms) = e.duration_ms {
                        ui.label(RichText::new(format!("{ms} ms")).weak().small());
                    }
                    if !e.database.is_empty() {
                        ui.label(RichText::new(&e.database).weak().small());
                    }
                    if ui.small_button(icons::COPY).on_hover_text("Copy this SQL").clicked() {
                        ui.ctx().copy_text(e.detail.clone());
                    }
                });
                ui.label(RichText::new(&e.detail).monospace().small());
                if !e.error.is_empty() {
                    ui.colored_label(red(), RichText::new(&e.error).small());
                }
                ui.separator();
            }
        });
}
