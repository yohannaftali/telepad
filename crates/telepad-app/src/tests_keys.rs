//! Behaviour tests for the Authorized keys page (#78).

use std::time::Instant;

use telepad_core::authorized_keys::{HostKeys, KeyAction, KeyOutcome, parse_authorized_keys};
use telepad_core::config::{Access, HostEntry};

use crate::*;

/// A made-up ed25519 public key line (32 bytes of `fill`), never a real one.
fn fake_key(fill: u8, comment: &str) -> String {
    let mut data = Vec::new();
    data.extend_from_slice(&11u32.to_be_bytes());
    data.extend_from_slice(b"ssh-ed25519");
    data.extend_from_slice(&32u32.to_be_bytes());
    data.extend_from_slice(&[fill; 32]);
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b64 = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                b64.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                b64.push('=');
            }
        }
    }
    format!("ssh-ed25519 {b64} {comment}")
        .trim_end()
        .to_string()
}

fn host(id: i64, alias: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: format!("192.0.2.{id}"),
        ..HostEntry::new(alias)
    }
}

fn state() -> State {
    let mut s = State::default();
    let mut rdp = host(3, "win1");
    rdp.access = Access::Rdp;
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![host(1, "web1"), host(2, "db1"), rdp]),
            ssh_config_error: None,
        },
        Instant::now(),
    );
    s
}

fn listed(s: &mut State, alias: &str, text: &str, login: Vec<String>) {
    let mut keys: HostKeys = parse_authorized_keys(text);
    keys.login = login;
    update(
        s,
        Msg::AkListed {
            alias: alias.into(),
            result: Ok(keys),
        },
        Instant::now(),
    );
}

#[test]
fn opening_the_page_reads_every_ssh_host_once() {
    let mut s = state();
    let fx = update(&mut s, Msg::KeysOpen, Instant::now());
    let hosts = fx.iter().find_map(|e| match e {
        Effect::ListKeys(h) => Some(h.iter().map(|h| h.alias.clone()).collect::<Vec<_>>()),
        _ => None,
    });
    assert_eq!(
        hosts,
        Some(vec!["web1".to_string(), "db1".to_string()]),
        "not the RDP host"
    );
    assert_eq!(s.model.auth_keys.get("web1"), Some(&AkHost::Loading));
    // Open again: already loaded, nothing read.
    update(&mut s, Msg::KeysClose, Instant::now());
    let fx = update(&mut s, Msg::KeysOpen, Instant::now());
    assert!(!fx.iter().any(|e| matches!(e, Effect::ListKeys(_))));
    // Refresh one host.
    let fx = update(
        &mut s,
        Msg::AkLoad(Some(vec!["db1".into()])),
        Instant::now(),
    );
    assert!(matches!(&fx[..], [Effect::ListKeys(h)] if h.len() == 1 && h[0].alias == "db1"));
    update(
        &mut s,
        Msg::AkListed {
            alias: "db1".into(),
            result: Err("connection timed out".into()),
        },
        Instant::now(),
    );
    assert_eq!(
        s.model.auth_keys.get("db1"),
        Some(&AkHost::Failed("connection timed out".into()))
    );
    // A result for a host that is gone is dropped.
    update(
        &mut s,
        Msg::AkListed {
            alias: "gone".into(),
            result: Ok(HostKeys::default()),
        },
        Instant::now(),
    );
    assert!(!s.model.auth_keys.contains_key("gone"));
}

#[test]
fn keys_are_grouped_by_fingerprint_with_their_servers() {
    let mut s = state();
    let alice = fake_key(1, "alice@example.com");
    let ops = fake_key(2, "ops");
    listed(&mut s, "web1", &format!("{alice}\n{ops}\n"), vec![]);
    let ops_fp = parse_authorized_keys(&ops).keys[0].key.fingerprint();
    listed(
        &mut s,
        "db1",
        &format!(
            "from=\"192.0.2.0/24\" {}\n{ops}\n",
            fake_key(1, "alice laptop")
        ),
        vec![ops_fp.clone()],
    );
    let k = key_holders(&s.model);
    assert_eq!(k.len(), 2);
    assert_eq!(k[0].comments, ["alice@example.com", "alice laptop"]);
    assert_eq!(
        k[0].hosts,
        [("web1".to_string(), false), ("db1".to_string(), false)]
    );
    assert_eq!(k[1].fingerprint, ops_fp);
    assert_eq!(
        k[1].hosts,
        [("web1".to_string(), false), ("db1".to_string(), true)],
        "db1: Telepad logs in with ops"
    );
}

#[test]
fn shortcuts_fill_the_form_and_ask_for_the_word() {
    let mut s = state();
    update(&mut s, Msg::KeysOpen, Instant::now());
    let line = fake_key(1, "alice@example.com");
    // Revoke from the list: key, host and the question are ready.
    update(
        &mut s,
        Msg::AkPrepare {
            action: KeyAction::Revoke,
            line: line.clone(),
            aliases: vec!["db1".into()],
        },
        Instant::now(),
    );
    let k = s.drafts.keys.as_ref().unwrap();
    assert_eq!(k.tab, KeysTab::AddRevoke);
    assert_eq!(k.text, line);
    assert_eq!(k.selected.iter().copied().collect::<Vec<_>>(), [2]);
    assert_eq!(k.confirm, Some(KeyAction::Revoke));
    // Nothing runs before the word is typed.
    assert!(update(&mut s, Msg::KeysRun, Instant::now()).is_empty());
    update(&mut s, Msg::KeysConfirmCancel, Instant::now());
    // "Add key" on a server: host chosen, key still to paste, no question.
    update(
        &mut s,
        Msg::AkPrepare {
            action: KeyAction::Add,
            line: String::new(),
            aliases: vec!["web1".into(), "win1".into()],
        },
        Instant::now(),
    );
    let k = s.drafts.keys.as_ref().unwrap();
    assert_eq!(
        k.selected.iter().copied().collect::<Vec<_>>(),
        [1],
        "not RDP"
    );
    assert_eq!(k.confirm, None);
    // Back to a list view.
    update(&mut s, Msg::AkTab(KeysTab::ByKey), Instant::now());
    assert_eq!(s.drafts.keys.as_ref().unwrap().tab, KeysTab::ByKey);
}

#[test]
fn a_finished_run_reads_its_hosts_again() {
    let mut s = state();
    update(&mut s, Msg::KeysOpen, Instant::now());
    update(
        &mut s,
        Msg::AkPrepare {
            action: KeyAction::Add,
            line: fake_key(5, "deploy"),
            aliases: vec!["web1".into(), "db1".into()],
        },
        Instant::now(),
    );
    s.drafts.keys.as_mut().unwrap().typed = "add".into();
    let fx = update(&mut s, Msg::KeysRun, Instant::now());
    assert!(matches!(&fx[..], [Effect::ApplyKey { hosts, .. }] if hosts.len() == 2));
    let fx = update(
        &mut s,
        Msg::KeyResult {
            alias: "web1".into(),
            outcome: KeyOutcome::Added,
        },
        Instant::now(),
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::ListKeys(_))),
        "still running"
    );
    let fx = update(
        &mut s,
        Msg::KeyResult {
            alias: "db1".into(),
            outcome: KeyOutcome::AlreadyPresent,
        },
        Instant::now(),
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::ListKeys(h) if h.len() == 2))
    );
    assert_eq!(s.model.auth_keys.get("web1"), Some(&AkHost::Loading));
}
