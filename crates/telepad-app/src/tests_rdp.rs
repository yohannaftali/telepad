//! Behaviour tests for #102: saved Remote Desktop settings in the host editor.

use std::collections::BTreeMap;
use std::time::Instant;

use telepad_core::config::{Access, HostEntry};
use telepad_core::rdp::{RdpSettings, Screen};

use crate::{Effect, Msg, State, update};

fn state() -> State {
    let mut s = State::default();
    let t = Instant::now();
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![
                HostEntry {
                    id: Some(1),
                    hostname: "198.51.100.7".into(),
                    access: Access::Rdp,
                    port: 3389,
                    ..HostEntry::new("win1")
                },
                HostEntry {
                    id: Some(2),
                    hostname: "192.0.2.10".into(),
                    ..HostEntry::new("web1")
                },
            ]),
            ssh_config_error: None,
        },
        t,
    );
    let mut map = BTreeMap::new();
    map.insert(
        "win1".to_string(),
        RdpSettings {
            user: "ops".into(),
            ..Default::default()
        },
    );
    update(&mut s, Msg::RdpLoaded(Ok(map)), t);
    s
}

fn send(s: &mut State, m: Msg) -> Vec<Effect> {
    update(s, m, Instant::now())
}

#[test]
fn editing_a_host_shows_its_saved_settings() {
    let mut s = state();
    send(&mut s, Msg::Edit(0));
    let ed = s.drafts.editor.as_ref().unwrap();
    assert!(ed.rdp_on);
    assert_eq!(ed.rdp.user, "ops");
    send(&mut s, Msg::EditorClose);
    send(&mut s, Msg::Edit(1));
    assert!(
        !s.drafts.editor.as_ref().unwrap().rdp_on,
        "none saved for web1"
    );
}

#[test]
fn settings_are_written_after_the_host_under_its_new_alias() {
    let mut s = state();
    send(&mut s, Msg::Edit(0));
    {
        let ed = s.drafts.editor.as_mut().unwrap();
        ed.entry.alias = "win-renamed".into();
        ed.rdp.screen = Screen::Window {
            width: 0,
            height: 0,
        };
        ed.rdp_width = "1280".into();
        ed.rdp_height = "800".into();
    }
    let fx = send(&mut s, Msg::EditorSave);
    assert!(matches!(&fx[..], [Effect::SaveHost(h)] if h.alias == "win-renamed"));
    let fx = send(&mut s, Msg::HostSaved(Ok((1, "win-renamed".into()))));
    let saved = fx.iter().find_map(|e| match e {
        Effect::SaveRdp { alias, settings } => Some((alias.clone(), settings.clone())),
        _ => None,
    });
    let (alias, settings) = saved.expect("written after the host");
    assert_eq!(alias, "win-renamed");
    assert_eq!(
        settings.unwrap().screen,
        Screen::Window {
            width: 1280,
            height: 800
        }
    );
}

#[test]
fn unchanged_settings_are_not_written_and_off_forgets_them() {
    let mut s = state();
    send(&mut s, Msg::Edit(0));
    send(&mut s, Msg::EditorSave);
    let fx = send(&mut s, Msg::HostSaved(Ok((1, "win1".into()))));
    assert!(!fx.iter().any(|e| matches!(e, Effect::SaveRdp { .. })));
    send(&mut s, Msg::Edit(0));
    s.drafts.editor.as_mut().unwrap().rdp_on = false;
    send(&mut s, Msg::EditorSave);
    let fx = send(&mut s, Msg::HostSaved(Ok((1, "win1".into()))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::SaveRdp { settings: None, .. }))
    );
}

#[test]
fn bad_settings_keep_the_editor_open_with_the_reason() {
    let mut s = state();
    send(&mut s, Msg::Edit(0));
    s.drafts.editor.as_mut().unwrap().rdp_port = "port".into();
    let fx = send(&mut s, Msg::EditorSave);
    assert!(fx.is_empty());
    assert!(
        s.drafts
            .editor
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("port")
    );
}

#[test]
fn an_imported_rdp_file_fills_the_section_and_export_renders_it() {
    let mut s = state();
    send(&mut s, Msg::Edit(1));
    assert!(matches!(
        &send(&mut s, Msg::EditorRdpImport)[..],
        [Effect::PickRdpFile]
    ));
    let r = RdpSettings::parse("full address:s:192.0.2.55:3390\r\nusername:s:LAB\\ops\r\n");
    send(&mut s, Msg::RdpPicked(Ok(r)));
    let ed = s.drafts.editor.as_ref().unwrap();
    assert!(ed.rdp_on);
    assert_eq!(ed.rdp_port, "3390");
    assert_eq!(ed.rdp.domain, "LAB");
    let fx = send(&mut s, Msg::EditorRdpExport);
    match &fx[..] {
        [Effect::ExportRdp { alias, text }] => {
            assert_eq!(alias, "web1");
            assert!(
                text.starts_with("full address:s:192.0.2.55:3390\r\n"),
                "{text}"
            );
            assert!(!text.contains("password"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_rdp_password_goes_to_the_credential_manager_never_into_telepad() {
    let mut s = state();
    send(&mut s, Msg::Edit(0));
    {
        let ed = s.drafts.editor.as_mut().unwrap();
        ed.rdp.domain = "LAB".into();
        *ed.rdp_cred_pass.expose_mut() = "one".into();
        *ed.rdp_cred_pass2.expose_mut() = "two".into();
    }
    assert!(send(&mut s, Msg::EditorRdpCredSave).is_empty());
    assert!(
        s.drafts
            .editor
            .as_ref()
            .unwrap()
            .rdp_cred_note
            .clone()
            .unwrap()
            .is_err()
    );
    *s.drafts
        .editor
        .as_mut()
        .unwrap()
        .rdp_cred_pass2
        .expose_mut() = "one".into();
    let fx = send(&mut s, Msg::EditorRdpCredSave);
    match &fx[..] {
        [
            Effect::RdpCredSave {
                address,
                user,
                password,
            },
        ] => {
            assert_eq!(address, "198.51.100.7");
            assert_eq!(user, "LAB\\ops");
            assert_eq!(password.expose(), "one");
            assert_eq!(format!("{password:?}"), "\"***\"");
        }
        other => panic!("{other:?}"),
    }
    let ed = s.drafts.editor.as_ref().unwrap();
    assert!(
        ed.rdp_cred_pass.is_empty() && ed.rdp_cred_pass2.is_empty(),
        "fields cleared"
    );
    send(
        &mut s,
        Msg::RdpCredState {
            address: "198.51.100.7".into(),
            saved: true,
            result: Ok("Saved".into()),
        },
    );
    assert_eq!(s.drafts.editor.as_ref().unwrap().rdp_cred_saved, Some(true));
    let fx = send(&mut s, Msg::EditorRdpCredForget);
    assert!(matches!(&fx[..], [Effect::RdpCredForget { address }] if address == "198.51.100.7"));
}

#[test]
fn open_rdp_works_for_ssh_hosts_too() {
    let mut s = state();
    let fx = send(&mut s, Msg::OpenRdp(1));
    assert!(matches!(&fx[..], [Effect::OpenRdp(h)] if h.alias == "web1"));
}
