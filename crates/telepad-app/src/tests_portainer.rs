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
