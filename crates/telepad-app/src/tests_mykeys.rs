//! Behaviour tests for #101: the My keys tab of the Authorized keys page.

use std::path::PathBuf;
use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::local_keys::{Format, GenType, LocalKey, Target};

use crate::mykeys::{MkEffect, MkMsg, PassMode};
use crate::{Effect, KeysTab, Msg, State, update};

fn state() -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![HostEntry {
                id: Some(1),
                hostname: "192.0.2.10".into(),
                identity_file: Some("~/.ssh/work_ed25519".into()),
                ..HostEntry::new("web1")
            }]),
            ssh_config_error: None,
        },
        Instant::now(),
    );
    update(&mut s, Msg::KeysOpen, Instant::now());
    s
}

fn mk(s: &mut State, m: MkMsg) -> Vec<MkEffect> {
    update(s, Msg::MyKeys(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::MyKeys(x) => Some(x),
            _ => None,
        })
        .collect()
}

fn sample() -> LocalKey {
    LocalKey {
        path: PathBuf::from("/home/me/.ssh/id_ed25519"),
        format: Some(Format::OpenSsh),
        kind: "Ed25519".into(),
        fingerprint: Some("SHA256:abc".into()),
        comment: "me@pc".into(),
        encrypted: true,
        public_line: Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI me@pc".into()),
        has_pub_file: true,
        ..Default::default()
    }
}

fn loaded(s: &mut State) {
    mk(
        s,
        MkMsg::Loaded {
            dir: std::env::temp_dir().join("telepad-mykeys-none"),
            user_host: "me@pc".into(),
            result: Ok(vec![sample()]),
        },
    );
}

#[test]
fn opening_the_tab_scans_once_with_the_inventorys_identity_files() {
    let mut s = state();
    let fx = update(&mut s, Msg::AkTab(KeysTab::MyKeys), Instant::now());
    let scans: Vec<_> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::MyKeys(MkEffect::Scan { extra }) => Some(extra.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(scans, vec![vec!["~/.ssh/work_ed25519".to_string()]]);
    assert!(s.model.my_keys.loading);
    assert!(
        update(&mut s, Msg::AkTab(KeysTab::MyKeys), Instant::now()).is_empty(),
        "not twice while it reads"
    );
}

#[test]
fn copy_and_add_to_hosts_use_the_public_line_only() {
    let mut s = state();
    loaded(&mut s);
    let p = sample().path;
    let fx = mk(&mut s, MkMsg::CopyPublic(p.clone()));
    assert_eq!(fx, vec![MkEffect::CopyText(sample().public_line.unwrap())]);
    mk(&mut s, MkMsg::AddToHosts(p));
    let k = s.drafts.keys.as_ref().unwrap();
    assert_eq!(k.tab, KeysTab::AddRevoke);
    assert!(k.text.starts_with("ssh-ed25519 "));
}

#[test]
fn generate_defaults_to_ed25519_in_ssh_and_checks_the_form() {
    let mut s = state();
    loaded(&mut s);
    mk(&mut s, MkMsg::AskGenerate);
    let g = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .generate
        .clone()
        .unwrap();
    assert_eq!(g.kind, GenType::Ed25519);
    assert!(g.path.ends_with("id_ed25519"), "{}", g.path);
    assert_eq!(g.comment, "me@pc");
    // The default name follows the type.
    mk(&mut s, MkMsg::GenKind(GenType::Rsa(4096)));
    let g = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .generate
        .clone()
        .unwrap();
    assert!(g.path.ends_with("id_rsa"));
    // Passphrases must match.
    {
        let g = s
            .drafts
            .keys
            .as_mut()
            .unwrap()
            .mine
            .generate
            .as_mut()
            .unwrap();
        *g.pass.expose_mut() = "one".into();
        *g.pass2.expose_mut() = "two".into();
    }
    assert!(mk(&mut s, MkMsg::Generate).is_empty());
    let g = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .generate
        .clone()
        .unwrap();
    assert!(g.error.unwrap().contains("differ"));
    {
        let g = s
            .drafts
            .keys
            .as_mut()
            .unwrap()
            .mine
            .generate
            .as_mut()
            .unwrap();
        *g.pass2.expose_mut() = "one".into();
    }
    let fx = mk(&mut s, MkMsg::Generate);
    match &fx[..] {
        [MkEffect::Generate { kind, pass, .. }] => {
            assert_eq!(*kind, GenType::Rsa(4096));
            assert_eq!(format!("{pass:?}"), "\"***\"", "never in Debug output");
        }
        other => panic!("{other:?}"),
    }
    // Done: the list is read again and the new key selected.
    let fx = mk(
        &mut s,
        MkMsg::Generated(Ok(PathBuf::from("/home/me/.ssh/id_rsa"))),
    );
    assert!(matches!(fx[..], [MkEffect::Scan { .. }]));
    let mine = &s.drafts.keys.as_ref().unwrap().mine;
    assert!(mine.generate.is_none());
    assert_eq!(mine.selected, Some(PathBuf::from("/home/me/.ssh/id_rsa")));
}

#[test]
fn a_failed_generate_keeps_the_dialog_with_the_reason() {
    let mut s = state();
    loaded(&mut s);
    mk(&mut s, MkMsg::AskGenerate);
    mk(&mut s, MkMsg::Generate);
    mk(&mut s, MkMsg::Generated(Err("ssh-keygen not found".into())));
    let g = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .generate
        .clone()
        .unwrap();
    assert!(!g.busy);
    assert_eq!(g.error.as_deref(), Some("ssh-keygen not found"));
}

#[test]
fn convert_asks_for_the_passphrase_and_pkcs12_needs_one() {
    let mut s = state();
    loaded(&mut s);
    mk(&mut s, MkMsg::AskConvert(sample().path));
    let c = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .convert
        .clone()
        .unwrap();
    assert_eq!(
        c.target,
        Target::Ppk3,
        "an OpenSSH key goes to PuTTY by default"
    );
    assert!(mk(&mut s, MkMsg::Convert).is_empty());
    let c = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .convert
        .clone()
        .unwrap();
    assert!(c.error.unwrap().contains("passphrase"));
    {
        let c = s
            .drafts
            .keys
            .as_mut()
            .unwrap()
            .mine
            .convert
            .as_mut()
            .unwrap();
        *c.pass_in.expose_mut() = "pw".into();
        c.target = Target::Pkcs12;
        c.mode = PassMode::Remove;
    }
    assert!(
        mk(&mut s, MkMsg::Convert).is_empty(),
        "PKCS#12 without a password"
    );
    {
        let c = s
            .drafts
            .keys
            .as_mut()
            .unwrap()
            .mine
            .convert
            .as_mut()
            .unwrap();
        c.mode = PassMode::Keep;
    }
    let fx = mk(&mut s, MkMsg::Convert);
    match &fx[..] {
        [
            MkEffect::Convert {
                target,
                suggest,
                pass_out,
                ..
            },
        ] => {
            assert_eq!(*target, Target::Pkcs12);
            assert!(suggest.ends_with("id_ed25519.p12"));
            assert_eq!(pass_out.expose(), "pw", "same passphrase kept");
        }
        other => panic!("{other:?}"),
    }
    mk(&mut s, MkMsg::Converted(Ok("written".into())));
    let c = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .convert
        .clone()
        .unwrap();
    assert_eq!(c.done.as_deref(), Some("written"));
}

#[test]
fn an_import_goes_to_openssh() {
    let mut s = state();
    loaded(&mut s);
    let fx = mk(&mut s, MkMsg::PickImport);
    assert!(matches!(fx[..], [MkEffect::PickImport { .. }]));
    let k = LocalKey {
        path: PathBuf::from("C:/keys/old.ppk"),
        format: Some(Format::Ppk2),
        kind: "RSA 2048".into(),
        ..Default::default()
    };
    mk(&mut s, MkMsg::Picked(Some(k)));
    let c = s
        .drafts
        .keys
        .as_ref()
        .unwrap()
        .mine
        .convert
        .clone()
        .unwrap();
    assert!(c.import);
    assert_eq!(c.target, Target::OpenSsh);
}

#[test]
fn an_open_key_dialog_is_listed_before_leaving_the_workspace() {
    let mut s = state();
    loaded(&mut s);
    mk(&mut s, MkMsg::AskGenerate);
    let (items, _) = crate::leave_losses(&s, false);
    assert!(
        items
            .iter()
            .any(|(w, x)| w == "My keys" && x.contains("Generate"))
    );
}
