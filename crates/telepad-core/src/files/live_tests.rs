//! Against a real SSH server: set `TELEPAD_TEST_SSH` to
//! `<ssh config path>|<alias with SFTP>|<alias without SFTP>` (the Docker
//! image in `tests/sshd`; CI starts it). Skipped when unset. Works in a
//! throwaway folder under the login user's home.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use super::sftp::{self, Sftp};
use super::transfer::{self, Decision, Direction};
use super::{fallback, rsync};
use crate::collector::SshSettings;
use crate::config::HostEntry;

fn lab() -> Option<(SshSettings, HostEntry, HostEntry)> {
    let v = std::env::var("TELEPAD_TEST_SSH").ok()?;
    let mut it = v.split('|');
    let cfg = PathBuf::from(it.next()?);
    let a = HostEntry {
        identity_file: Some("set in the config".into()),
        ..HostEntry::new(it.next()?)
    };
    let b = HostEntry {
        identity_file: Some("set in the config".into()),
        ..HostEntry::new(it.next()?)
    };
    Some((SshSettings::new(cfg), a, b))
}

fn local_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("telepad-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Bytes that are not all the same, more than one SFTP chunk window.
fn sample(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
}

#[test]
fn sftp_list_transfer_plan_and_operations() {
    let Some((s, host, _)) = lab() else {
        eprintln!("TELEPAD_TEST_SSH not set: skipped");
        return;
    };
    let mut c = Sftp::connect(&s, &host).unwrap();
    let home = c.realpath(".").unwrap();
    assert!(home.starts_with('/'), "{home}");
    let base = sftp::join(&home, "telepad live ünï 'test'");
    let _ = c.rmdir(&base);
    c.mkdir(&base, Some(0o755)).unwrap();

    // Upload a folder with an awkward name and a file over the window size.
    let src = local_dir("src");
    let site = src.join("my site");
    std::fs::create_dir_all(site.join("img")).unwrap();
    let big = sample(1_300_000);
    std::fs::write(site.join("big.bin"), &big).unwrap();
    std::fs::write(site.join("img").join("a 'b'.txt"), b"hello").unwrap();
    let opts = transfer::Options::default();
    let cancel = AtomicBool::new(false);
    let sources = vec![(site.display().to_string(), true)];
    let items = transfer::plan(&mut c, Direction::Upload, &sources, &base, &opts, &cancel).unwrap();
    assert_eq!(items.iter().filter(|i| i.is_dir).count(), 2);
    assert!(items.iter().all(|i| i.exists.is_none()));
    let mut bytes = 0;
    let (copied, skipped, failed) = transfer::run(
        &mut c,
        Direction::Upload,
        &items,
        &HashMap::new(),
        &opts,
        &cancel,
        &mut |e| {
            if let transfer::Event::Bytes(n) = e {
                bytes += n;
            }
        },
    );
    assert_eq!((copied, skipped, failed), (4, 0, 0));
    assert_eq!(bytes, big.len() as u64 + 5);

    let remote_site = sftp::join(&base, "my site");
    let list = c.list(&remote_site).unwrap();
    let mut names: Vec<_> = list.iter().map(|e| (e.name.clone(), e.is_dir())).collect();
    names.sort();
    assert_eq!(names, vec![("big.bin".into(), false), ("img".into(), true)]);
    assert_eq!(
        list.iter().find(|e| e.name == "big.bin").unwrap().size,
        big.len() as u64
    );
    assert_eq!(list[0].owner, "telepad");

    // Second plan sees the existing files; decisions decide.
    let again = transfer::plan(&mut c, Direction::Upload, &sources, &base, &opts, &cancel).unwrap();
    let conflicts: Vec<usize> = again
        .iter()
        .enumerate()
        .filter(|(_, i)| !i.is_dir && i.exists.is_some())
        .map(|(n, _)| n)
        .collect();
    assert_eq!(conflicts.len(), 2);
    let skip_all: HashMap<usize, Decision> =
        conflicts.iter().map(|&n| (n, Decision::Skip)).collect();
    let (copied, skipped, _) = transfer::run(
        &mut c,
        Direction::Upload,
        &again,
        &skip_all,
        &opts,
        &cancel,
        &mut |_| {},
    );
    assert_eq!(
        (copied, skipped),
        (0, 4),
        "2 folders exist, 2 files skipped"
    );
    let newer: HashMap<usize, Decision> = conflicts
        .iter()
        .map(|&n| (n, Decision::NewerOnly))
        .collect();
    let (copied, _, _) = transfer::run(
        &mut c,
        Direction::Upload,
        &again,
        &newer,
        &opts,
        &cancel,
        &mut |_| {},
    );
    assert_eq!(copied, 0, "times were kept on upload, so nothing is newer");

    // Download it back and compare.
    let dst = local_dir("dst");
    let back = transfer::plan(
        &mut c,
        Direction::Download,
        &[(remote_site.clone(), true)],
        &dst.display().to_string(),
        &opts,
        &cancel,
    )
    .unwrap();
    let (copied, _, failed) = transfer::run(
        &mut c,
        Direction::Download,
        &back,
        &HashMap::new(),
        &opts,
        &cancel,
        &mut |_| {},
    );
    assert_eq!((copied, failed), (4, 0));
    assert_eq!(
        std::fs::read(dst.join("my site").join("big.bin")).unwrap(),
        big
    );
    assert_eq!(
        std::fs::read(dst.join("my site").join("img").join("a 'b'.txt")).unwrap(),
        b"hello"
    );
    assert!(!dst.join("my site").join("big.bin.telepad-part").exists());

    // A cancelled upload leaves nothing behind (no part file, old file kept).
    let stop_up = AtomicBool::new(true);
    let one = vec![(site.join("big.bin").display().to_string(), false)];
    let up_again = transfer::plan(
        &mut c,
        Direction::Upload,
        &one,
        &remote_site,
        &opts,
        &cancel,
    )
    .unwrap();
    let all: HashMap<usize, Decision> = [(0, Decision::Overwrite)].into();
    let (copied, _, _) = transfer::run(
        &mut c,
        Direction::Upload,
        &up_again,
        &all,
        &opts,
        &stop_up,
        &mut |_| {},
    );
    assert_eq!(copied, 0);
    let names: Vec<String> = c
        .list(&remote_site)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(
        !names.iter().any(|n| n.ends_with(".telepad-part")),
        "{names:?}"
    );
    assert_eq!(
        c.stat(&sftp::join(&remote_site, "big.bin")).unwrap().size,
        Some(big.len() as u64)
    );

    // Cancel stops a transfer.
    let stop = AtomicBool::new(true);
    let mut sink = Vec::new();
    assert!(
        c.download(
            &sftp::join(&remote_site, "big.bin"),
            &mut sink,
            &stop,
            &mut |_| {}
        )
        .unwrap_err()
        .contains("Cancelled")
    );

    // Operations used by #63.
    let f = sftp::join(&remote_site, "big.bin");
    let g = sftp::join(&remote_site, "renamed.bin");
    c.rename(&f, &g).unwrap();
    c.setstat(
        &g,
        sftp::Attrs {
            perm: Some(0o600),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(c.stat(&g).unwrap().perm.map(|p| p & 0o777), Some(0o600));
    assert!(
        c.list("/no/such/folder")
            .unwrap_err()
            .contains("No such file")
    );
    for p in [g, sftp::join(&sftp::join(&remote_site, "img"), "a 'b'.txt")] {
        c.remove(&p).unwrap();
    }
    c.rmdir(&sftp::join(&remote_site, "img")).unwrap();
    c.rmdir(&remote_site).unwrap();
    c.rmdir(&base).unwrap();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

#[test]
fn hosts_without_sftp_fall_back_to_sh_and_scp() {
    let Some((s, _, host)) = lab() else {
        eprintln!("TELEPAD_TEST_SSH not set: skipped");
        return;
    };
    let err = Sftp::connect(&s, &host).err().expect("no SFTP server");
    assert!(err.contains("no SFTP server"), "{err}");
    let (home, entries) = fallback::list(&s, &host, "~").unwrap();
    assert!(home.starts_with('/'));
    let _ = entries;
    let src = local_dir("scp");
    let file = src.join("note one.txt");
    std::fs::write(&file, b"made-up note").unwrap();
    let cancel = AtomicBool::new(false);
    let remote = sftp::join(&home, "note one.txt");
    fallback::copy(&s, &host, true, &file.display().to_string(), &home, &cancel).unwrap();
    let (_, entries) = fallback::list(&s, &host, &home).unwrap();
    let up = entries.iter().find(|e| e.name == "note one.txt").unwrap();
    assert_eq!(up.size, 12);
    // The host's umask decides the mode, never world-writable.
    assert_eq!(up.perm.map(|p| p & 0o002), Some(0), "{:?}", up.perm);
    let back = src.join("back");
    std::fs::create_dir_all(&back).unwrap();
    fallback::copy(
        &s,
        &host,
        false,
        &back.display().to_string(),
        &remote,
        &cancel,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(back.join("note one.txt")).unwrap(),
        b"made-up note"
    );
    let _ = crate::collector::run_script(&s, &host, "rm -f -- \"$HOME/note one.txt\"\n");
    let _ = std::fs::remove_dir_all(&src);
}

#[test]
fn rsync_when_both_sides_have_it() {
    let Some((s, host, _)) = lab() else {
        eprintln!("TELEPAD_TEST_SSH not set: skipped");
        return;
    };
    assert_eq!(rsync::remote_available(&s, &host), Ok(()));
    if let Err(why) = rsync::local_available() {
        eprintln!("{why}: rsync run skipped");
        return;
    }
    let src = local_dir("rsync");
    std::fs::create_dir_all(src.join("site")).unwrap();
    std::fs::write(src.join("site").join("index.html"), b"<p>made up</p>").unwrap();
    let mut c = Sftp::connect(&s, &host).unwrap();
    let home = c.realpath(".").unwrap();
    let sources = vec![src.join("site").display().to_string()];
    let dry = rsync::run(&rsync::args(
        &s,
        &host,
        true,
        &sources,
        &home,
        true,
        &transfer::Options::default(),
    ))
    .unwrap();
    assert!(dry.iter().any(|p| p == "site/index.html"), "{dry:?}");
    assert!(
        c.stat(&sftp::join(&home, "site")).is_err(),
        "dry run changes nothing"
    );
    rsync::run(&rsync::args(
        &s,
        &host,
        true,
        &sources,
        &home,
        false,
        &transfer::Options::default(),
    ))
    .unwrap();
    assert_eq!(
        c.stat(&sftp::join(&home, "site/index.html")).unwrap().size,
        Some(14)
    );
    c.remove(&sftp::join(&home, "site/index.html")).unwrap();
    c.rmdir(&sftp::join(&home, "site")).unwrap();
    let _ = std::fs::remove_dir_all(&src);
}

/// Run a command on the lab host (test setup only).
fn lab_sh(s: &SshSettings, host: &HostEntry, cmd: &str) {
    let st = std::process::Command::new(&s.ssh_program)
        .arg("-F")
        .arg(&s.config_path)
        .args(["-n", "-o", "BatchMode=yes", &host.alias, cmd])
        .status()
        .unwrap();
    assert!(st.success(), "{cmd}");
}

#[test]
fn sftp_file_operations_new_rename_chmod_measure_delete() {
    use super::ops::{self, Measure};
    let Some((s, host, _)) = lab() else {
        eprintln!("TELEPAD_TEST_SSH not set: skipped");
        return;
    };
    let mut c = Sftp::connect(&s, &host).unwrap();
    let home = c.realpath(".").unwrap();
    let cancel = AtomicBool::new(false);
    let find = |c: &mut Sftp, dir: &str, name: &str| {
        c.list(dir)
            .unwrap()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
    };
    // Clean a leftover of an earlier failed run.
    if let Ok(Some(e)) = c
        .list(&home)
        .map(|v| v.into_iter().find(|e| e.name == "ops ü 'test'"))
    {
        let _ = ops::Fs::chmod(&mut c, &sftp::join(&home, &e.name), 0o755);
        ops::delete(&mut c, &home, &[e], &cancel);
    }

    let base = ops::new_folder(&mut c, &home, "ops ü 'test'", false).unwrap();
    assert!(
        ops::new_folder(&mut c, &home, "ops ü 'test'", false)
            .unwrap_err()
            .contains("exists")
    );
    assert!(ops::new_folder(&mut c, &base, "../escape", false).is_err());
    ops::new_file(&mut c, &base, "a \"b\".txt", false).unwrap();
    assert!(ops::new_file(&mut c, &base, "a \"b\".txt", false).is_err());
    ops::new_folder(&mut c, &base, "sub", false).unwrap();
    ops::new_file(&mut c, &sftp::join(&base, "sub"), "f", false).unwrap();
    // A link to a folder outside the tree: never followed by delete / chmod.
    lab_sh(
        &s,
        &host,
        "mkdir -p ~/ops-keep && touch ~/ops-keep/precious && chmod 755 ~/ops-keep",
    );
    lab_sh(
        &s,
        &host,
        "ln -s ~/ops-keep \"$HOME/ops ü 'test'/sub/to-keep\"",
    );

    ops::rename(&mut c, &base, "a \"b\".txt", "renamed ü.txt", false).unwrap();
    assert!(
        ops::rename(&mut c, &base, "renamed ü.txt", "sub", false)
            .unwrap_err()
            .contains("exists")
    );
    let names: Vec<String> = c.list(&base).unwrap().into_iter().map(|e| e.name).collect();
    assert!(names.contains(&"renamed ü.txt".to_string()), "{names:?}");

    let e = find(&mut c, &home, "ops ü 'test'");
    assert_eq!(
        ops::measure(&mut c, &home, std::slice::from_ref(&e), &cancel)[0],
        Measure {
            files: 3,
            dirs: 2,
            bytes: c
                .lstat(&sftp::join(&sftp::join(&base, "sub"), "to-keep"))
                .unwrap()
                .size
                .unwrap(),
            partial: false
        }
    );

    let r = ops::chmod(
        &mut c,
        &home,
        std::slice::from_ref(&e),
        0o640,
        true,
        &cancel,
    );
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let perm = |c: &mut Sftp, p: &str| c.stat(p).unwrap().perm.unwrap() & 0o777;
    assert_eq!(perm(&mut c, &base), 0o640);
    // Without x the folder cannot be entered: give it back to look inside.
    ops::chmod(
        &mut c,
        &home,
        std::slice::from_ref(&e),
        0o750,
        false,
        &cancel,
    );
    assert_eq!(perm(&mut c, &sftp::join(&base, "sub")), 0o750);
    assert_eq!(
        perm(&mut c, &sftp::join(&sftp::join(&base, "sub"), "f")),
        0o640
    );
    assert_eq!(perm(&mut c, &sftp::join(&home, "ops-keep")), 0o755);

    let e = find(&mut c, &home, "ops ü 'test'");
    let r = ops::delete(&mut c, &home, &[e], &cancel);
    assert_eq!(r.errors, Vec::<String>::new());
    assert_eq!(r.done, 1);
    assert!(c.lstat(&base).is_err());
    assert!(
        c.lstat(&sftp::join(&sftp::join(&home, "ops-keep"), "precious"))
            .is_ok()
    );
    lab_sh(&s, &host, "rm -rf ~/ops-keep");
}

#[test]
fn sftp_editor_load_save_conflict_and_links() {
    use super::edit::{self, SaveError};
    use super::text;
    let Some((s, host, _)) = lab() else {
        eprintln!("TELEPAD_TEST_SSH not set: skipped");
        return;
    };
    let mut c = Sftp::connect(&s, &host).unwrap();
    let home = c.realpath(".").unwrap();
    let base = sftp::join(&home, "edit-test");
    lab_sh(&s, &host, "rm -rf ~/edit-test && mkdir ~/edit-test");
    let cancel = AtomicBool::new(false);
    let put = |c: &mut Sftp, p: &str, b: &[u8], perm: u32| {
        let mut src = b;
        c.upload(&mut src, p, Some(perm), &cancel, &mut |_| {})
            .unwrap();
        c.setstat(
            p,
            sftp::Attrs {
                perm: Some(perm),
                ..Default::default()
            },
        )
        .unwrap();
    };
    let p = sftp::join(&base, "app.env");
    put(&mut c, &p, b"A=1\r\nB=2\r\n", 0o640);

    // Load, edit, save: CRLF and mode kept, no part file left.
    let (bytes, st) = edit::read_remote(&mut c, &p).unwrap();
    let doc = text::decode(&bytes).unwrap();
    assert!(doc.crlf && !doc.bom);
    let out = text::encode(&doc.text.replace("B=2", "B=ü"), doc.crlf, doc.bom);
    let st2 = edit::save_remote(&mut c, &p, &out, Some(st)).unwrap();
    assert_eq!(
        edit::read_remote(&mut c, &p).unwrap().0,
        "A=1\r\nB=ü\r\n".as_bytes()
    );
    assert_eq!(c.stat(&p).unwrap().perm.unwrap() & 0o7777, 0o640);
    let names: Vec<String> = c.list(&base).unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["app.env".to_string()]);

    // Changed on the host meanwhile: refused, the host's text stays.
    put(&mut c, &p, b"A=1\r\nB=changed on the host\r\n", 0o640);
    match edit::save_remote(&mut c, &p, b"mine", Some(st2)) {
        Err(SaveError::Changed(Some(now))) => assert_eq!(now.size, 28),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        edit::read_remote(&mut c, &p).unwrap().0,
        b"A=1\r\nB=changed on the host\r\n"
    );
    edit::save_remote(&mut c, &p, b"overwritten\n", None).unwrap();

    // Through a link: the target changes, the link stays a link.
    lab_sh(&s, &host, "ln -s app.env ~/edit-test/link.env");
    let l = sftp::join(&base, "link.env");
    let (_, lst) = edit::read_remote(&mut c, &l).unwrap();
    edit::save_remote(&mut c, &l, b"via link\n", Some(lst)).unwrap();
    assert_eq!(c.lstat(&l).unwrap().kind(), super::Kind::Link);
    assert_eq!(edit::read_remote(&mut c, &p).unwrap().0, b"via link\n");

    // Binary files and folders are refused with the reason.
    let bin = sftp::join(&base, "x.bin");
    put(&mut c, &bin, b"\x7fELF\x00\x01", 0o644);
    let (b, _) = edit::read_remote(&mut c, &bin).unwrap();
    assert!(text::decode(&b).unwrap_err().contains("Binary"));
    assert!(
        edit::read_remote(&mut c, &base)
            .unwrap_err()
            .contains("not a file")
    );
    lab_sh(&s, &host, "rm -rf ~/edit-test");
}
