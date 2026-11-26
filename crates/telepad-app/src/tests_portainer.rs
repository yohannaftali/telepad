//! Behaviour tests for #171 (Portainers page and Portainer - [name] pages).

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::db::Secret;
use telepad_core::portainer::{Environment, Portainer, ReadError, Snapshot};
use telepad_core::store::UiPrefs;
use telepad_core::update::UpdateConfig;

use crate::portainer::{PtEffect, PtMsg, PtStatus};
use crate::workspace::Win;
use crate::*;

fn host(id: i64, alias: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: format!("192.0.2.{id}"),
        ..HostEntry::new(alias)
    }
}

fn portainer(name: &str, alias: &str) -> Portainer {
    Portainer {
        name: name.into(),
        alias: alias.into(),
        port: 9443,
        https: true,
        fingerprint: String::new(),
    }
}

fn started(now: Instant, layout: &str) -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::Started {
            db_path: "telepad.db".into(),
            ssh_config_path: "telepad.ssh_config".into(),
            imported_from: None,
            update_config: Ok(UpdateConfig {
                enabled: false,
                ..Default::default()
            }),
            ui_prefs: UiPrefs {
                workspace: layout.into(),
                ..Default::default()
            },
            prefs: Ok(Default::default()),
            strategy: Err("not used".into()),
        },
        now,
    );
    s.model.ws.restore_or_default(layout);
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![host(1, "docker1"), host(2, "docker2")]),
            ssh_config_error: None,
        },
        now,
    );
    s
}

fn pt(s: &mut State, msg: PtMsg) -> Vec<PtEffect> {
    update(s, Msg::Portainer(msg), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::Portainer(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn loaded(s: &mut State) -> Vec<PtEffect> {
    pt(
        s,
        PtMsg::Loaded(Ok(vec![
            portainer("Main", "docker1"),
            portainer("Lab", "docker2"),
        ])),
    )
}

fn reads(fx: &[PtEffect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            PtEffect::Read { p, .. } => Some(p.name.clone()),
            _ => None,
        })
        .collect()
}

fn snapshot() -> Snapshot {
    Snapshot {
        version: "2.21.4".into(),
        environments: vec![Environment {
            id: 1,
            name: "local".into(),
            kind: 1,
            url: "unix:///var/run/docker.sock".into(),
            up: Some(true),
            ..Default::default()
        }],
        stacks: Vec::new(),
    }
}

fn done(
    s: &mut State,
    name: &str,
    r: Result<(Snapshot, Option<String>), ReadError>,
) -> Vec<PtEffect> {
    pt(
        s,
        PtMsg::Read {
            name: name.into(),
            result: Box::new(r),
        },
    )
}

#[test]
fn the_list_reads_every_portainer_once_and_pins_the_first_certificate() {
    let mut s = started(Instant::now(), "");
    assert!(
        reads(&loaded(&mut s)).is_empty(),
        "nothing open, nothing read"
    );
    let fx = pt(&mut s, PtMsg::OpenList);
    assert!(s.model.ws.is_open(Win::Portainers));
    assert_eq!(reads(&fx), ["Main", "Lab"]);
    // Through the Portainer's own host.
    assert!(matches!(&fx[0], PtEffect::Read { host, accept: None, .. } if host.alias == "docker1"));
    // A second open while reading reads nothing again.
    assert!(reads(&pt(&mut s, PtMsg::OpenList)).is_empty());
    // First read: trusted, pinned.
    let fx = done(&mut s, "Main", Ok((snapshot(), Some("AA:BB".into()))));
    assert_eq!(
        fx,
        [PtEffect::Pin {
            name: "Main".into(),
            fingerprint: "AA:BB".into()
        }]
    );
    let row = s.model.pt("main").unwrap();
    assert_eq!(row.status, PtStatus::Ok);
    assert_eq!(row.data.as_ref().unwrap().version, "2.21.4");
    assert!(row.checked.is_some() && !row.reading);
    // The same certificate again: nothing to pin.
    pt(&mut s, PtMsg::Refresh("Main".into()));
    assert!(done(&mut s, "Main", Ok((snapshot(), Some("AA:BB".into())))).is_empty());
    // Errors stay on the row with a short status.
    done(&mut s, "Lab", Err(ReadError::Rejected));
    assert_eq!(s.model.pt("Lab").unwrap().status.text(), "token rejected");
    // Reloading the list (same servers) keeps what was read.
    loaded(&mut s);
    assert_eq!(s.model.pt("Main").unwrap().status, PtStatus::Ok);
}

#[test]
fn a_changed_certificate_is_read_again_only_when_accepted() {
    let mut s = started(Instant::now(), "");
    loaded(&mut s);
    pt(&mut s, PtMsg::Open("Main".into()));
    done(
        &mut s,
        "Main",
        Err(ReadError::CertChanged {
            pinned: "AA".into(),
            seen: "CC".into(),
        }),
    );
    assert_eq!(
        s.model.pt("Main").unwrap().status.text(),
        "certificate changed"
    );
    let fx = pt(&mut s, PtMsg::AcceptCert("Main".into()));
    assert!(matches!(&fx[..], [PtEffect::Read { accept: Some(a), .. }] if a == "CC"));
    // Accepting is only possible for a changed certificate.
    done(&mut s, "Main", Ok((snapshot(), Some("CC".into()))));
    assert!(pt(&mut s, PtMsg::AcceptCert("Main".into())).is_empty());
}

#[test]
fn pages_open_once_follow_renames_and_close_with_their_portainer() {
    let mut s = started(Instant::now(), "");
    loaded(&mut s);
    let fx = pt(&mut s, PtMsg::Open("main".into()));
    assert_eq!(reads(&fx), ["Main"]);
    assert_eq!(s.model.pt_pages.len(), 1);
    let id = s.model.pt_pages[0].id;
    assert_eq!(s.model.pt_pages[0].title(), "Portainer - Main");
    assert!(s.model.ws.is_open(Win::Portainer(id)));
    assert_eq!(s.model.ws.focused, Some(Win::Portainer(id)));
    // Again: the same page to the front, no second read while one runs.
    s.model.ws.open(Win::Overview);
    assert!(reads(&pt(&mut s, PtMsg::Open("Main".into()))).is_empty());
    assert_eq!(s.model.pt_pages.len(), 1);
    assert_eq!(s.model.ws.focused, Some(Win::Portainer(id)));
    pt(
        &mut s,
        PtMsg::SelectEnv {
            page: id,
            env: Some(1),
        },
    );
    assert_eq!(s.model.pt_pages[0].env, Some(1));
    // Renamed (the store reloads the list): the page follows.
    pt(&mut s, PtMsg::Edit("Main".into()));
    s.drafts.pt.form.as_mut().unwrap().name = "Prod".into();
    pt(&mut s, PtMsg::Saved(Ok(())));
    assert_eq!(s.model.pt_pages[0].name, "Prod");
    pt(
        &mut s,
        PtMsg::Loaded(Ok(vec![
            portainer("Prod", "docker1"),
            portainer("Lab", "docker2"),
        ])),
    );
    assert_eq!(s.model.pt_pages[0].title(), "Portainer - Prod");
    // Deleted: its page closes.
    pt(&mut s, PtMsg::Loaded(Ok(vec![portainer("Lab", "docker2")])));
    assert!(s.model.pt_pages.is_empty());
    assert!(!s.model.ws.is_open(Win::Portainer(id)));
}

#[test]
fn pages_come_back_with_the_workspace() {
    let mut s = started(Instant::now(), "");
    loaded(&mut s);
    pt(&mut s, PtMsg::OpenList);
    pt(&mut s, PtMsg::Open("Lab".into()));
    let layout = crate::workspace::layout(&s.model);
    assert!(layout.contains("portainers"), "{layout}");
    assert!(layout.contains("portainer:Lab"), "{layout}");
    let mut s2 = started(Instant::now(), &layout);
    assert!(s2.model.pt_pages.is_empty(), "waits for the list");
    let fx = loaded(&mut s2);
    assert_eq!(s2.model.pt_pages.len(), 1);
    assert_eq!(s2.model.pt_pages[0].name, "Lab");
    assert!(s2.model.ws.is_open(Win::Portainer(s2.model.pt_pages[0].id)));
    // The restored list and page are read (each once).
    let mut r = reads(&fx);
    r.sort();
    assert_eq!(r, ["Lab", "Main"]);
}

#[test]
fn the_form_checks_its_fields_and_saves_the_token_apart() {
    let mut s = started(Instant::now(), "");
    loaded(&mut s);
    pt(&mut s, PtMsg::Add);
    let f = s.drafts.pt.form.as_mut().unwrap();
    assert_eq!((f.port.as_str(), f.https), ("9443", true), "default port");
    f.name = "main".into();
    f.alias = "docker1".into();
    assert!(pt(&mut s, PtMsg::FormSave).is_empty());
    let err = |s: &State| {
        s.drafts
            .pt
            .form
            .as_ref()
            .unwrap()
            .error
            .clone()
            .unwrap_or_default()
    };
    assert!(err(&s).contains("already"), "{}", err(&s));
    let f = s.drafts.pt.form.as_mut().unwrap();
    f.name = "Edge".into();
    f.port = "0".into();
    pt(&mut s, PtMsg::FormSave);
    assert!(err(&s).contains("port"), "{}", err(&s));
    s.drafts.pt.form.as_mut().unwrap().port = "9000".into();
    s.drafts.pt.form.as_mut().unwrap().https = false;
    pt(&mut s, PtMsg::FormSave);
    assert!(
        err(&s).contains("token"),
        "a new one needs a token: {}",
        err(&s)
    );
    // Test without saving: the typed token goes to the effect only.
    s.drafts.pt.form.as_mut().unwrap().token = Secret::new("ptr_abc");
    let fx = pt(&mut s, PtMsg::FormTest);
    assert!(
        matches!(&fx[..], [PtEffect::Test { token: Some(t), host, .. }] if t.expose() == "ptr_abc" && host.alias == "docker1")
    );
    assert!(s.drafts.pt.form.as_ref().unwrap().testing);
    pt(
        &mut s,
        PtMsg::Tested(Ok("Portainer 2.21.4 · 2 environments".into())),
    );
    assert!(!s.drafts.pt.form.as_ref().unwrap().testing);
    let fx = pt(&mut s, PtMsg::FormSave);
    match &fx[..] {
        [
            PtEffect::Save {
                old: None,
                p,
                token: Some(t),
            },
        ] => {
            assert_eq!((p.name.as_str(), p.port, p.https), ("Edge", 9000, false));
            assert_eq!(t.expose(), "ptr_abc");
        }
        other => panic!("{other:?}"),
    }
    // Editing without a new token keeps the saved one.
    pt(&mut s, PtMsg::FormCancel);
    pt(&mut s, PtMsg::Edit("Lab".into()));
    let fx = pt(&mut s, PtMsg::FormSave);
    assert!(matches!(&fx[..], [PtEffect::Save { old: Some(o), token: None, .. }] if o == "Lab"));
    // Delete asks first.
    pt(&mut s, PtMsg::Delete("Lab".into()));
    assert_eq!(s.drafts.pt.delete.as_deref(), Some("Lab"));
    assert_eq!(
        pt(&mut s, PtMsg::DeleteYes),
        [PtEffect::Delete("Lab".into())]
    );
    assert!(s.drafts.pt.delete.is_none());
}

fn with_stack() -> Snapshot {
    let mut s = snapshot();
    s.stacks.push(telepad_core::portainer::Stack {
        id: 11,
        name: "shop".into(),
        kind: 2,
        env: 1,
        active: Some(true),
        ..Default::default()
    });
    s
}

fn page_with_stack(s: &mut State) -> crate::portainer::PtPageId {
    loaded(s);
    pt(s, PtMsg::Open("Main".into()));
    done(s, "Main", Ok((with_stack(), None)));
    s.model.pt_pages[0].id
}

#[test]
fn stack_actions_ask_first_and_read_again_after() {
    // #173, from the Portainer page and from a Stack page (#176).
    use crate::portainer::{Origin, StackTab};
    use telepad_core::portainer::{Action, ContainerOp};
    let mut s = started(Instant::now(), "");
    let page = page_with_stack(&mut s);
    // A single click only selects; the Stack page lists the containers.
    assert!(
        pt(
            &mut s,
            PtMsg::SelectStack {
                page,
                stack: Some(11)
            }
        )
        .is_empty()
    );
    assert_eq!(s.model.pt_pages[0].stack, Some(11));
    let fx = pt(
        &mut s,
        PtMsg::OpenStack {
            portainer: "Main".into(),
            stack: 11,
            tab: StackTab::Containers,
        },
    );
    let ed = s.model.pt_editors[0].id;
    assert!(
        fx.iter()
            .any(|e| matches!(e, PtEffect::LoadStack { stack: 11, .. }))
    );
    assert!(fx.iter().any(
        |e| matches!(e, PtEffect::Containers { stack, editor, .. } if stack.name == "shop" && *editor == ed)
    ));
    assert!(s.model.pt_editors[0].loading_containers);
    pt(
        &mut s,
        PtMsg::StackContainers {
            editor: ed,
            result: Ok(vec![]),
        },
    );
    assert_eq!(s.model.pt_editors[0].containers, Some(Ok(vec![])));
    // Stop asks first; No runs nothing.
    let stop = Action::StackStop { id: 11, env: 1 };
    let ask = |origin: Origin, act: Action, type_name: &str| PtMsg::Ask {
        origin,
        act,
        title: "Stop stack shop".into(),
        type_name: type_name.into(),
    };
    assert!(pt(&mut s, ask(Origin::Page(page), stop.clone(), "")).is_empty());
    assert!(s.drafts.pt.confirm.is_some());
    assert!(pt(&mut s, PtMsg::ConfirmNo).is_empty());
    assert!(s.drafts.pt.confirm.is_none());
    // Yes runs it, through the Portainer's host; the page shows it busy.
    pt(&mut s, ask(Origin::Page(page), stop.clone(), ""));
    let fx = pt(&mut s, PtMsg::ConfirmYes);
    assert!(
        matches!(&fx[..], [PtEffect::Act { act, host, .. }] if *act == stop && host.alias == "docker1")
    );
    assert!(s.model.pt_pages[0].busy.is_some());
    // Done: the Portainer and the open Stack page's containers are read again.
    let fx = pt(
        &mut s,
        PtMsg::Acted {
            portainer: "Main".into(),
            origin: Origin::Page(page),
            act: stop.clone(),
            title: "Stop stack shop".into(),
            result: Ok(()),
        },
    );
    assert!(s.model.pt_pages[0].busy.is_none());
    assert_eq!(reads(&fx), ["Main"]);
    assert!(fx.iter().any(|e| matches!(e, PtEffect::Containers { .. })));
    // From the Stack page: its own busy line; container restart runs at
    // once, remove asks with the volume option.
    let c = |op| Action::Container {
        env: 1,
        id: "abc".into(),
        op,
    };
    let fx = pt(&mut s, ask(Origin::Stack(ed), c(ContainerOp::Restart), ""));
    assert!(matches!(&fx[..], [PtEffect::Act { origin: Origin::Stack(e), .. }] if *e == ed));
    assert!(s.model.pt_editors[0].busy.is_some());
    assert!(
        pt(
            &mut s,
            ask(
                Origin::Stack(ed),
                c(ContainerOp::Remove { volumes: false }),
                ""
            )
        )
        .is_empty()
    );
    pt(&mut s, PtMsg::ConfirmVolumes(true));
    assert!(matches!(
        &pt(&mut s, PtMsg::ConfirmYes)[..],
        [PtEffect::Act {
            act: Action::Container {
                op: ContainerOp::Remove { volumes: true },
                ..
            },
            ..
        }]
    ));
    // Delete needs the name typed; once done, its Stack page closes.
    let del = Action::StackDelete { id: 11, env: 1 };
    pt(&mut s, ask(Origin::Stack(ed), del.clone(), "shop"));
    s.drafts.pt.confirm.as_mut().unwrap().typed = "sho".into();
    assert!(pt(&mut s, PtMsg::ConfirmYes).is_empty());
    assert!(s.drafts.pt.confirm.is_some(), "still asking");
    s.drafts.pt.confirm.as_mut().unwrap().typed = "shop".into();
    assert!(
        matches!(&pt(&mut s, PtMsg::ConfirmYes)[..], [PtEffect::Act { act, .. }] if *act == del)
    );
    pt(
        &mut s,
        PtMsg::Acted {
            portainer: "Main".into(),
            origin: Origin::Stack(ed),
            act: del,
            title: "Delete stack shop".into(),
            result: Ok(()),
        },
    );
    assert!(s.model.pt_editors.is_empty());
    assert!(!s.model.ws.is_open(Win::PtStack(ed)));
}

#[test]
fn a_host_page_opens_the_stack_in_the_portainer_that_manages_it() {
    // #176: by host + stack name; read first when needed; several → choose.
    use crate::portainer::{StackRef, StackTab};
    let mut s = started(Instant::now(), "");
    loaded(&mut s);
    // Not read yet: the Portainers are read, then the page opens.
    let fx = pt(
        &mut s,
        PtMsg::OpenStackFor {
            alias: "docker1".into(),
            name: "shop".into(),
        },
    );
    assert_eq!(reads(&fx), ["Main", "Lab"]);
    assert!(s.model.pt_editors.is_empty());
    done(&mut s, "Main", Ok((with_stack(), None)));
    assert!(s.model.pt_editors.is_empty(), "waits for every read");
    let fx = done(&mut s, "Lab", Ok((snapshot(), None)));
    assert_eq!(s.model.pt_editors.len(), 1);
    let e = &s.model.pt_editors[0];
    assert_eq!(
        (e.portainer.as_str(), e.stack_id, e.tab),
        ("Main", 11, StackTab::Containers)
    );
    assert_eq!(e.env_name, "local");
    assert!(fx.iter().any(|x| matches!(x, PtEffect::Containers { .. })));
    // Known now: opens at once (raises the same page), even by another case.
    let ed = e.id;
    s.model.ws.open(Win::Overview);
    assert!(
        pt(
            &mut s,
            PtMsg::OpenStackFor {
                alias: "docker1".into(),
                name: "SHOP".into()
            }
        )
        .is_empty()
    );
    assert_eq!(s.model.ws.focused, Some(Win::PtStack(ed)));
    // Another host has no such stack.
    assert!(crate::portainer::stack_refs(&s.model, "docker2", "shop").is_empty());
    pt(
        &mut s,
        PtMsg::OpenStackFor {
            alias: "docker2".into(),
            name: "shop".into(),
        },
    );
    assert_eq!(s.model.pt_editors.len(), 1);
    // Two Portainers manage the same host: the user picks.
    done(&mut s, "Lab", Ok((with_stack(), None)));
    let lab_host = s
        .model
        .portainers
        .iter()
        .position(|r| r.p.name == "Lab")
        .unwrap();
    s.model.portainers[lab_host].p.alias = "docker1".into();
    assert_eq!(
        crate::portainer::stack_refs(&s.model, "docker1", "shop").len(),
        2
    );
    pt(
        &mut s,
        PtMsg::OpenStackFor {
            alias: "docker1".into(),
            name: "shop".into(),
        },
    );
    let choices = s.drafts.pt.choose.clone().unwrap();
    assert_eq!(choices.len(), 2);
    let lab: StackRef = choices.into_iter().find(|c| c.portainer == "Lab").unwrap();
    pt(&mut s, PtMsg::ChooseStack(Some(lab)));
    assert!(s.drafts.pt.choose.is_none());
    assert_eq!(s.model.pt_editors.len(), 2);
    assert_eq!(s.model.pt_editors[1].portainer, "Lab");
    // Tabs keep the edits.
    let ed2 = s.model.pt_editors[1].id;
    s.drafts.pt.editors.get_mut(&ed2).unwrap().file = "edited".into();
    pt(
        &mut s,
        PtMsg::SetTab {
            editor: ed2,
            tab: StackTab::Editor,
        },
    );
    pt(
        &mut s,
        PtMsg::SetTab {
            editor: ed2,
            tab: StackTab::Containers,
        },
    );
    assert_eq!(s.drafts.pt.editors[&ed2].file, "edited");
}

#[test]
fn the_stack_editor_reviews_updates_and_handles_a_changed_stack() {
    // #173
    use telepad_core::portainer::{EnvVar, StackDetail};
    let mut s = started(Instant::now(), "");
    let _page = page_with_stack(&mut s);
    let fx = pt(
        &mut s,
        PtMsg::OpenStack {
            portainer: "Main".into(),
            stack: 11,
            tab: crate::portainer::StackTab::Editor,
        },
    );
    assert!(matches!(
        &fx[..],
        [
            PtEffect::LoadStack { stack: 11, .. },
            PtEffect::Containers { .. }
        ]
    ));
    let ed = s.model.pt_editors[0].id;
    assert_eq!(s.model.pt_editors[0].title(), "Stack - shop");
    assert!(s.model.ws.is_open(Win::PtStack(ed)));
    // Opening it again brings the same window.
    assert!(
        pt(
            &mut s,
            PtMsg::OpenStack {
                portainer: "Main".into(),
                stack: 11,
                tab: crate::portainer::StackTab::Editor
            }
        )
        .is_empty()
    );
    assert_eq!(s.model.pt_editors.len(), 1);
    let var = |n: &str, v: &str| EnvVar {
        name: n.into(),
        value: v.into(),
    };
    let detail = StackDetail {
        id: 11,
        name: "shop".into(),
        kind: 2,
        env_id: 1,
        env: vec![var("MODE", "dev"), var("OLD", "x")],
        updated: Some(100),
        file: "services: {}\n".into(),
        ..Default::default()
    };
    let load = |d: StackDetail| PtMsg::StackLoaded {
        editor: ed,
        result: Box::new(Ok(d)),
    };
    pt(&mut s, load(detail.clone()));
    let dr = s.drafts.pt.editors.get_mut(&ed).unwrap();
    assert_eq!(dr.file, "services: {}\n");
    // Bad names stop the review.
    dr.vars = vec![var("MODE", "prod"), var("BAD NAME", "1")];
    pt(&mut s, PtMsg::EditorReview(ed));
    let err = s.model.pt_editors[0].error.clone().unwrap_or_default();
    assert!(err.contains("no spaces"), "{err}");
    let dr = s.drafts.pt.editors.get_mut(&ed).unwrap();
    dr.vars = vec![var("MODE", "prod"), var("NEW", "1")];
    dr.repull = true;
    pt(&mut s, PtMsg::EditorReview(ed));
    assert!(s.drafts.pt.editors[&ed].review);
    assert_eq!(
        crate::portainer::var_changes(&detail.env, &s.drafts.pt.editors[&ed].vars),
        ["~ MODE", "+ NEW", "- OLD"]
    );
    // Update: sent with the UpdateDate it was loaded with; values never in the summary.
    let fx = pt(
        &mut s,
        PtMsg::EditorUpdate {
            editor: ed,
            force: false,
        },
    );
    match &fx[..] {
        [
            PtEffect::Update {
                upd,
                expect,
                summary,
                ..
            },
        ] => {
            assert_eq!(*expect, Some(Some(100)));
            assert!(upd.repull && !upd.prune);
            assert_eq!(upd.vars.len(), 2);
            assert_eq!(summary, "~ MODE, + NEW, - OLD");
            assert!(!summary.contains("prod"));
        }
        other => panic!("{other:?}"),
    }
    assert!(s.model.pt_editors[0].saving);
    // Changed in Portainer meanwhile: conflict; Overwrite sends without the check.
    pt(
        &mut s,
        PtMsg::StackUpdated {
            editor: ed,
            result: Err(ReadError::Changed),
        },
    );
    assert!(s.model.pt_editors[0].conflict && !s.model.pt_editors[0].saving);
    let fx = pt(
        &mut s,
        PtMsg::EditorUpdate {
            editor: ed,
            force: true,
        },
    );
    assert!(matches!(&fx[..], [PtEffect::Update { expect: None, .. }]));
    // Updated: loaded again and the Portainer read again.
    let fx = pt(
        &mut s,
        PtMsg::StackUpdated {
            editor: ed,
            result: Ok(()),
        },
    );
    assert!(fx.iter().any(|e| matches!(e, PtEffect::LoadStack { .. })));
    assert_eq!(reads(&fx), ["Main"]);
    // Closing with edits asks; Discard closes.
    pt(&mut s, load(detail.clone()));
    s.drafts
        .pt
        .editors
        .get_mut(&ed)
        .unwrap()
        .file
        .push_str("# x\n");
    pt(&mut s, PtMsg::EditorClose(ed));
    assert!(s.drafts.pt.editors[&ed].ask_close);
    assert_eq!(s.model.pt_editors.len(), 1);
    pt(&mut s, PtMsg::EditorDiscard(ed));
    assert!(s.model.pt_editors.is_empty());
    assert!(!s.model.ws.is_open(Win::PtStack(ed)));
    // A git stack is not sent from Telepad.
    pt(
        &mut s,
        PtMsg::OpenStack {
            portainer: "Main".into(),
            stack: 11,
            tab: crate::portainer::StackTab::Editor,
        },
    );
    let ed = s.model.pt_editors[0].id;
    let git = StackDetail {
        git: Some("https://git.example/shop.git".into()),
        ..detail
    };
    pt(
        &mut s,
        PtMsg::StackLoaded {
            editor: ed,
            result: Box::new(Ok(git)),
        },
    );
    pt(&mut s, PtMsg::EditorReview(ed));
    let err = s.model.pt_editors[0].error.clone().unwrap_or_default();
    assert!(err.contains("git"), "{err}");
    assert!(
        pt(
            &mut s,
            PtMsg::EditorUpdate {
                editor: ed,
                force: false
            }
        )
        .is_empty()
    );
}

#[test]
fn variables_open_as_their_own_page_and_come_from_a_file_or_paste() {
    // #173: the Variables page, .env import and paste.
    use telepad_core::portainer::{EnvVar, StackDetail};
    let mut s = started(Instant::now(), "");
    let _page = page_with_stack(&mut s);
    pt(
        &mut s,
        PtMsg::OpenStack {
            portainer: "Main".into(),
            stack: 11,
            tab: crate::portainer::StackTab::Editor,
        },
    );
    let ed = s.model.pt_editors[0].id;
    let detail = StackDetail {
        id: 11,
        name: "shop".into(),
        kind: 2,
        env_id: 1,
        env: vec![EnvVar {
            name: "MODE".into(),
            value: "dev".into(),
        }],
        file: "services: {}\n".into(),
        ..Default::default()
    };
    pt(
        &mut s,
        PtMsg::StackLoaded {
            editor: ed,
            result: Box::new(Ok(detail.clone())),
        },
    );
    // Its own page, next to the editor; once.
    pt(&mut s, PtMsg::OpenVars(ed));
    assert!(s.model.ws.is_open(Win::PtVars(ed)));
    assert_eq!(s.model.pt_editors[0].vars_title(), "Variables - shop");
    pt(&mut s, PtMsg::OpenVars(ed));
    assert_eq!(s.model.ws.focused, Some(Win::PtVars(ed)));
    // A file: the runtime picks and reads it.
    assert_eq!(
        pt(&mut s, PtMsg::VarsFromFile(ed)),
        [PtEffect::PickEnvFile { editor: ed }]
    );
    pt(
        &mut s,
        PtMsg::VarsFile {
            editor: ed,
            name: "app.env".into(),
            result: Ok("# made up\nMODE=prod\nexport DB_HOST=db\n".into()),
        },
    );
    let names = |s: &State| -> Vec<(String, String)> {
        s.drafts.pt.editors[&ed]
            .vars
            .iter()
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect()
    };
    assert_eq!(
        names(&s),
        [
            ("MODE".to_string(), "prod".to_string()),
            ("DB_HOST".to_string(), "db".to_string())
        ]
    );
    assert_eq!(
        s.model.pt_editors[0].notice,
        Some(Ok("1 added, 1 changed from app.env".into()))
    );
    // A bad file takes nothing.
    pt(
        &mut s,
        PtMsg::VarsFile {
            editor: ed,
            name: "bad.env".into(),
            result: Ok("OK=1\nnot a line\n".into()),
        },
    );
    assert_eq!(names(&s).len(), 2);
    assert!(matches!(&s.model.pt_editors[0].notice, Some(Err(e)) if e.contains("line 2")));
    // Paste: the dialog's text, then it closes.
    pt(&mut s, PtMsg::VarsPasteOpen(ed));
    *s.drafts
        .pt
        .editors
        .get_mut(&ed)
        .unwrap()
        .paste
        .as_mut()
        .unwrap() = "TOKEN=\"a b\"\n".into();
    pt(&mut s, PtMsg::VarsPaste(ed));
    assert!(s.drafts.pt.editors[&ed].paste.is_none());
    assert_eq!(names(&s)[2], ("TOKEN".to_string(), "a b".to_string()));
    // The editor sees the same variables (the review lists names only).
    pt(&mut s, PtMsg::EditorReview(ed));
    assert_eq!(
        crate::portainer::var_changes(&detail.env, &s.drafts.pt.editors[&ed].vars),
        ["~ MODE", "+ DB_HOST", "+ TOKEN"]
    );
    // Closing the page keeps the edits; closing the editor closes it too.
    pt(&mut s, PtMsg::CloseVars(ed));
    assert!(!s.model.ws.is_open(Win::PtVars(ed)));
    assert_eq!(names(&s).len(), 3);
    pt(&mut s, PtMsg::OpenVars(ed));
    pt(&mut s, PtMsg::EditorDiscard(ed));
    assert!(!s.model.ws.is_open(Win::PtVars(ed)));
    // Git stacks: no import or paste.
    pt(
        &mut s,
        PtMsg::OpenStack {
            portainer: "Main".into(),
            stack: 11,
            tab: crate::portainer::StackTab::Editor,
        },
    );
    let ed = s.model.pt_editors[0].id;
    pt(
        &mut s,
        PtMsg::StackLoaded {
            editor: ed,
            result: Box::new(Ok(StackDetail {
                git: Some("https://git.example/shop.git".into()),
                ..detail
            })),
        },
    );
    assert!(pt(&mut s, PtMsg::VarsFromFile(ed)).is_empty());
    pt(&mut s, PtMsg::VarsPasteOpen(ed));
    assert!(s.drafts.pt.editors[&ed].paste.is_none());
}
