//! Host Management (#58): compare Telepad's hosts with an OpenSSH config file
//! and change that file without disturbing anything Telepad does not manage.
//!
//! The file is kept as its original lines, split into blocks. A block for a
//! single plain alias (`Host web1`, with the comment lines just above it) is a
//! managed host that can be replaced or removed; everything else — text before
//! the first `Host`, `Host *`, `Host a b`, wildcard patterns, `Match`,
//! `Include` — is copied as it is.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{self, HostEntry};

/// One block of a config file, by line range (end exclusive).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub start: usize,
    pub end: usize,
    /// The host of a managed block; `None` for text kept as it is.
    pub host: Option<HostEntry>,
    /// Plain aliases named in a kept block (`Host a b`): shown read-only.
    pub shared: Vec<String>,
}

/// An OpenSSH config file as loaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigFile {
    /// Exactly what was read (compared again before writing).
    pub text: String,
    /// Lines without line endings.
    pub lines: Vec<String>,
    pub blocks: Vec<Block>,
    pub crlf: bool,
}

fn is_comment(l: &str) -> bool {
    l.trim_start().starts_with('#')
}

fn keyword(l: &str) -> String {
    let t = l.trim();
    let end = t.find([' ', '\t', '=']).unwrap_or(t.len());
    t[..end].to_ascii_lowercase()
}

fn plain_alias(a: &str) -> bool {
    !a.is_empty() && !a.contains(['*', '?', '!'])
}

impl ConfigFile {
    pub fn parse(text: &str) -> Self {
        let lines: Vec<String> = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
            .collect();
        // A trailing newline leaves one empty element; it is not a line.
        let lines = if text.ends_with('\n') {
            lines[..lines.len() - 1].to_vec()
        } else {
            lines
        };
        let crlf = text.contains("\r\n");
        // Where each Host / Match block starts, including its comment lines
        // directly above (no blank line in between).
        let mut starts: Vec<usize> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if matches!(keyword(l).as_str(), "host" | "match") && !is_comment(l) {
                let mut s = i;
                while s > 0 && is_comment(&lines[s - 1]) {
                    s -= 1;
                }
                starts.push(s);
            }
        }
        let mut blocks = Vec::new();
        let first = starts.first().copied().unwrap_or(lines.len());
        if first > 0 {
            blocks.push(Block {
                start: 0,
                end: first,
                host: None,
                shared: vec![],
            });
        }
        for (n, &s) in starts.iter().enumerate() {
            let mut end = starts.get(n + 1).copied().unwrap_or(lines.len());
            // Trailing blank lines stay outside the block.
            while end > s + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            let head = lines[s..end]
                .iter()
                .find(|l| !is_comment(l))
                .cloned()
                .unwrap_or_default();
            let names: Vec<&str> = head.trim()[keyword(&head).len()..]
                .trim_start_matches([' ', '\t', '='])
                .split_whitespace()
                .collect();
            let managed = keyword(&head) == "host"
                && names.len() == 1
                && plain_alias(names[0].trim_matches('"'));
            let body = lines[s..end].join("\n");
            // Blocks keep their own Include lines etc. verbatim, so only plain
            // single-alias blocks without Include are managed.
            let has_include = lines[s..end].iter().any(|l| keyword(l) == "include");
            let host = if managed && !has_include {
                config::parse(&body).into_iter().next()
            } else {
                None
            };
            let shared = if host.is_none() && keyword(&head) == "host" {
                names
                    .iter()
                    .map(|a| a.trim_matches('"'))
                    .filter(|a| plain_alias(a))
                    .map(str::to_string)
                    .collect()
            } else {
                vec![]
            };
            blocks.push(Block {
                start: s,
                end,
                host,
                shared,
            });
            // Blank lines between blocks become their own kept block.
            let next = starts.get(n + 1).copied().unwrap_or(lines.len());
            if end < next {
                blocks.push(Block {
                    start: end,
                    end: next,
                    host: None,
                    shared: vec![],
                });
            }
        }
        Self {
            text: text.to_string(),
            lines,
            blocks,
            crlf,
        }
    }

    /// Managed hosts, in file order.
    pub fn hosts(&self) -> Vec<&HostEntry> {
        self.blocks.iter().filter_map(|b| b.host.as_ref()).collect()
    }

    /// Aliases defined in blocks Telepad keeps as they are.
    pub fn shared_aliases(&self) -> BTreeSet<String> {
        self.blocks
            .iter()
            .flat_map(|b| b.shared.iter().cloned())
            .collect()
    }

    fn block_of(&self, alias: &str) -> Option<usize> {
        self.blocks
            .iter()
            .position(|b| b.host.as_ref().is_some_and(|h| h.alias == alias))
    }
}

/// The text of one managed host block (no trailing newline).
pub fn host_block(h: &HostEntry) -> String {
    let inv = config::Inventory {
        hosts: vec![HostEntry {
            id: None,
            ..h.clone()
        }],
        global: String::new(),
        ..Default::default()
    };
    let text = config::to_ssh_config(&inv);
    text.lines()
        .skip_while(|l| l.starts_with("# Generated by Telepad") || l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A change to the config file.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigChange {
    /// Replace the host's block, or add it when the file has none.
    Upsert(HostEntry),
    Delete(String),
}

/// The file text after `changes`. Kept blocks are untouched; new hosts go
/// after the last managed host (else before the first `Host` / `Match`
/// block, else at the end) so `Host *` defaults keep coming after them.
pub fn apply(file: &ConfigFile, changes: &[ConfigChange]) -> Result<String, String> {
    let shared = file.shared_aliases();
    let mut replace: Vec<Option<String>> = vec![None; file.blocks.len()];
    let mut removed = vec![false; file.blocks.len()];
    let mut added: Vec<String> = Vec::new();
    for c in changes {
        match c {
            ConfigChange::Upsert(h) => {
                if shared.contains(&h.alias) {
                    return Err(format!(
                        "{} is defined on a shared Host line in the file; change it there by hand",
                        h.alias
                    ));
                }
                h.validate().map_err(|e| format!("{}: {e}", h.alias))?;
                match file.block_of(&h.alias) {
                    Some(i) => replace[i] = Some(host_block(h)),
                    None => added.push(host_block(h)),
                }
            }
            ConfigChange::Delete(a) => {
                if let Some(i) = file.block_of(a) {
                    removed[i] = true;
                }
            }
        }
    }
    let is_section = |b: &Block| {
        file.lines[b.start..b.end]
            .iter()
            .find(|l| !is_comment(l) && !l.trim().is_empty())
            .is_some_and(|l| matches!(keyword(l).as_str(), "host" | "match"))
    };
    // After the last managed host; else before the first Host / Match section.
    let anchor = (0..file.blocks.len())
        .rev()
        .find(|&i| file.blocks[i].host.is_some() && !removed[i])
        .map(|i| (i, true))
        .or_else(|| {
            (0..file.blocks.len())
                .find(|&i| is_section(&file.blocks[i]) && !removed[i])
                .map(|i| (i, false))
        });
    let mut out: Vec<String> = Vec::new();
    let push_added = |out: &mut Vec<String>| {
        for b in &added {
            if out.last().is_some_and(|l: &String| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.extend(b.lines().map(str::to_string));
        }
    };
    let mut done_added = added.is_empty();
    for (i, b) in file.blocks.iter().enumerate() {
        if !done_added && anchor == Some((i, false)) {
            push_added(&mut out);
            out.push(String::new());
            done_added = true;
        }
        if removed[i] {
            continue;
        }
        match &replace[i] {
            Some(text) => out.extend(text.lines().map(str::to_string)),
            None => out.extend(file.lines[b.start..b.end].iter().cloned()),
        }
        if !done_added && anchor == Some((i, true)) {
            push_added(&mut out);
            done_added = true;
        }
    }
    if !done_added {
        push_added(&mut out);
    }
    // Removing blocks can leave double blank lines.
    let mut tidy: Vec<String> = Vec::with_capacity(out.len());
    for l in out {
        if l.trim().is_empty() && tidy.last().is_some_and(|p: &String| p.trim().is_empty()) {
            continue;
        }
        tidy.push(l);
    }
    while tidy.first().is_some_and(|l| l.trim().is_empty()) {
        tidy.remove(0);
    }
    while tidy.last().is_some_and(|l| l.trim().is_empty()) {
        tidy.pop();
    }
    let nl = if file.crlf { "\r\n" } else { "\n" };
    let mut text = tidy.join(nl);
    if !text.is_empty() {
        text.push_str(nl);
    }
    Ok(text)
}

/// Host fields that differ (labels as shown in the window).
pub fn differences(a: &HostEntry, b: &HostEntry) -> Vec<&'static str> {
    let mut d = Vec::new();
    if a.hostname != b.hostname {
        d.push("HostName");
    }
    if a.user != b.user {
        d.push("User");
    }
    if a.port != b.port {
        d.push("Port");
    }
    if a.identity_file != b.identity_file {
        d.push("IdentityFile");
    }
    if a.options != b.options {
        d.push("Options");
    }
    if a.note != b.note {
        d.push("Description");
    }
    if a.unmonitored != b.unmonitored {
        d.push("Not monitored");
    }
    if a.access != b.access {
        d.push("RDP only");
    }
    d
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Same,
    Different,
    OnlyTelepad,
    OnlyConfig,
}

/// One alias in the comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub alias: String,
    pub db: Option<HostEntry>,
    pub cfg: Option<HostEntry>,
    pub state: RowState,
    pub diff: Vec<&'static str>,
    /// The config defines it on a shared / wildcard line: cannot be written there.
    pub cfg_read_only: bool,
}

/// Rows in Telepad's order, then aliases only the config has (file order).
pub fn compare(db: &[HostEntry], file: &ConfigFile) -> Vec<Row> {
    let cfg = file.hosts();
    let shared = file.shared_aliases();
    // Hosts on shared lines are read through the normal parser.
    let all = config::parse(&file.text);
    let mut rows: Vec<Row> = db
        .iter()
        .map(|d| {
            let c = cfg
                .iter()
                .find(|c| c.alias == d.alias)
                .map(|c| (*c).clone())
                .or_else(|| {
                    shared
                        .contains(&d.alias)
                        .then(|| all.iter().find(|h| h.alias == d.alias).cloned())
                        .flatten()
                });
            let diff = c.as_ref().map(|c| differences(d, c)).unwrap_or_default();
            let state = match &c {
                None => RowState::OnlyTelepad,
                Some(_) if diff.is_empty() => RowState::Same,
                Some(_) => RowState::Different,
            };
            Row {
                alias: d.alias.clone(),
                db: Some(d.clone()),
                cfg: c,
                state,
                diff,
                cfg_read_only: shared.contains(&d.alias),
            }
        })
        .collect();
    // Aliases on shared / wildcard lines: listed, but read-only in the config.
    for a in &shared {
        if !db.iter().any(|d| &d.alias == a) {
            rows.push(Row {
                alias: a.clone(),
                db: None,
                cfg: all.iter().find(|h| &h.alias == a).cloned(),
                state: RowState::OnlyConfig,
                diff: vec![],
                cfg_read_only: true,
            });
        }
    }
    for c in cfg {
        if !db.iter().any(|d| d.alias == c.alias) {
            rows.push(Row {
                alias: c.alias.clone(),
                db: None,
                cfg: Some(c.clone()),
                state: RowState::OnlyConfig,
                diff: vec![],
                cfg_read_only: false,
            });
        }
    }
    rows
}

/// `-` / `+` / ` ` and the line, for the preview (LCS line diff).
pub fn line_diff(old: &str, new: &str) -> Vec<(char, String)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            out.push((' ', a[i].to_string()));
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push(('-', a[i].to_string()));
            i += 1;
        } else {
            out.push(('+', b[j].to_string()));
            j += 1;
        }
    }
    out
}

/// Read the config; a missing file is an empty one.
pub fn read(path: &Path) -> Result<String, String> {
    match std::fs::read(path) {
        Ok(b) => String::from_utf8(b).map_err(|_| format!("{} is not UTF-8 text", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

/// Days since 1970-01-01 → (year, month, day) (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `config.telepad-YYYYMMDD-HHMMSS.bak` (UTC) next to the file.
pub fn backup_path(path: &Path, unix_secs: u64) -> PathBuf {
    let (y, mo, d) = civil_from_days((unix_secs / 86_400) as i64);
    let s = unix_secs % 86_400;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".into());
    path.with_file_name(format!(
        "{name}.telepad-{y:04}{mo:02}{d:02}-{:02}{:02}{:02}.bak",
        s / 3600,
        s % 3600 / 60,
        s % 60
    ))
}

/// Write `new_text` if the file still holds `expected` (what was loaded).
/// A copy of the old file is kept as a timestamped `.bak` first; the write is
/// atomic (temp file + rename) and keeps the file's permissions. Returns the
/// backup path (`None` when the file did not exist).
pub fn write(path: &Path, expected: &str, new_text: &str) -> Result<Option<PathBuf>, String> {
    let now = read(path)?;
    if now != expected {
        return Err(format!(
            "{} was changed outside Telepad since it was loaded; click Reload and check again",
            path.display()
        ));
    }
    let existed = path.exists();
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(d) = dir {
        std::fs::create_dir_all(d).map_err(|e| format!("Cannot create {}: {e}", d.display()))?;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = if existed {
        let b = backup_path(path, secs);
        std::fs::copy(path, &b)
            .map_err(|e| format!("Cannot write the backup {}: {e}", b.display()))?;
        Some(b)
    } else {
        None
    };
    let tmp = path.with_file_name(format!(
        ".{}.telepad-tmp",
        path.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(new_text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        if existed {
            let perms = std::fs::metadata(path)?.permissions();
            std::fs::set_permissions(&tmp, perms)?;
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Cannot write {}: {e}", path.display()));
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Access;

    const FILE: &str = "# my laptop config\nInclude ~/.ssh/work.d/*\n\n# Web server\nHost web1\n  HostName 192.0.2.10\n  User deploy\n\nHost jump bastion\n  HostName 198.51.100.1\n\nHost db1\n  HostName 192.0.2.20\n  Port 2222\n  # tuned for the old server\n  KexAlgorithms +diffie-hellman-group14-sha1\n\nMatch host *.example.com\n  User ops\n\nHost *\n  ServerAliveInterval 30\n";

    fn h(alias: &str, ip: &str) -> HostEntry {
        HostEntry {
            hostname: ip.into(),
            ..HostEntry::new(alias)
        }
    }

    #[test]
    fn splits_managed_and_kept_blocks() {
        let f = ConfigFile::parse(FILE);
        let hosts: Vec<&str> = f.hosts().iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(hosts, vec!["web1", "db1"]);
        assert_eq!(f.hosts()[0].note, "Web server");
        assert_eq!(f.hosts()[1].port, 2222);
        assert_eq!(
            f.shared_aliases().into_iter().collect::<Vec<_>>(),
            vec!["bastion".to_string(), "jump".to_string()]
        );
        // Unchanged: byte for byte.
        assert_eq!(apply(&f, &[]).unwrap(), FILE);
    }

    #[test]
    fn replaces_deletes_and_adds_in_place() {
        let f = ConfigFile::parse(FILE);
        let mut web = f.hosts()[0].clone();
        web.user = "admin".into();
        let new = apply(
            &f,
            &[
                ConfigChange::Upsert(web),
                ConfigChange::Delete("db1".into()),
                ConfigChange::Upsert(h("app1", "192.0.2.30")),
            ],
        )
        .unwrap();
        assert_eq!(
            new,
            "# my laptop config\nInclude ~/.ssh/work.d/*\n\n# Web server\nHost web1\n  HostName 192.0.2.10\n  User admin\n  Port 22\n\nHost app1\n  HostName 192.0.2.30\n  Port 22\n\nHost jump bastion\n  HostName 198.51.100.1\n\nMatch host *.example.com\n  User ops\n\nHost *\n  ServerAliveInterval 30\n"
        );
        // Hosts are added after the last managed one, before `Host *`.
        let added = apply(&f, &[ConfigChange::Upsert(h("zz", "192.0.2.99"))]).unwrap();
        assert!(added.find("Host zz").unwrap() < added.find("Match host").unwrap());
        assert!(added.find("Host zz").unwrap() > added.find("KexAlgorithms").unwrap());
    }

    #[test]
    fn refuses_shared_aliases_and_invalid_hosts() {
        let f = ConfigFile::parse(FILE);
        assert!(
            apply(&f, &[ConfigChange::Upsert(h("jump", "x"))])
                .unwrap_err()
                .contains("shared")
        );
        assert!(apply(&f, &[ConfigChange::Upsert(h("bad alias", "x"))]).is_err());
    }

    #[test]
    fn telepad_tags_round_trip_and_crlf_is_kept() {
        let mut win = h("win1", "192.0.2.40");
        win.access = Access::Rdp;
        win.port = 3389;
        win.unmonitored = true;
        win.note = "Front desk PC".into();
        let f = ConfigFile::parse("Host a\r\n  HostName 192.0.2.1\r\n");
        let text = apply(&f, &[ConfigChange::Upsert(win.clone())]).unwrap();
        assert!(text.contains("\r\n# telepad: rdp only\r\n"), "{text:?}");
        let back = ConfigFile::parse(&text);
        let got = back
            .hosts()
            .into_iter()
            .find(|h| h.alias == "win1")
            .unwrap()
            .clone();
        assert!(
            differences(&got, &win).is_empty(),
            "{:?}",
            differences(&got, &win)
        );
    }

    #[test]
    fn empty_and_host_only_files() {
        let empty = ConfigFile::parse("");
        assert_eq!(
            apply(&empty, &[ConfigChange::Upsert(h("a", "192.0.2.1"))]).unwrap(),
            "Host a\n  HostName 192.0.2.1\n  Port 22\n"
        );
        let star = ConfigFile::parse("Host *\n  User me\n");
        let t = apply(&star, &[ConfigChange::Upsert(h("a", "192.0.2.1"))]).unwrap();
        assert!(t.starts_with("Host a\n"), "{t}");
        assert!(t.ends_with("Host *\n  User me\n"));
    }

    #[test]
    fn compares_by_alias() {
        let f = ConfigFile::parse(FILE);
        let mut web = f.hosts()[0].clone();
        web.id = Some(1);
        web.user = "other".into();
        let db = vec![web, h("app1", "192.0.2.30"), h("jump", "198.51.100.1")];
        let rows = compare(&db, &f);
        let s: Vec<(&str, RowState)> = rows.iter().map(|r| (r.alias.as_str(), r.state)).collect();
        assert_eq!(
            s,
            vec![
                ("web1", RowState::Different),
                ("app1", RowState::OnlyTelepad),
                ("jump", RowState::Same),
                ("bastion", RowState::OnlyConfig),
                ("db1", RowState::OnlyConfig),
            ]
        );
        assert_eq!(rows[0].diff, vec!["User"]);
        assert!(rows[2].cfg_read_only);
    }

    #[test]
    fn line_diff_marks_changes() {
        let d = line_diff("a\nb\nc\n", "a\nx\nc\nd\n");
        assert_eq!(
            d,
            vec![
                (' ', "a".into()),
                ('-', "b".into()),
                ('+', "x".into()),
                (' ', "c".into()),
                ('+', "d".into())
            ]
        );
    }

    #[test]
    fn writes_with_backup_and_refuses_outside_changes() {
        let dir = std::env::temp_dir().join(format!("telepad-hostsync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config");
        assert_eq!(
            write(&path, "", "Host a\n").unwrap(),
            None,
            "new file, no backup"
        );
        assert_eq!(read(&path).unwrap(), "Host a\n");
        let b = write(&path, "Host a\n", "Host b\n").unwrap().unwrap();
        assert_eq!(read(&b).unwrap(), "Host a\n");
        assert!(
            b.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("config.telepad-")
        );
        assert!(
            write(&path, "Host a\n", "Host c\n")
                .unwrap_err()
                .contains("changed outside")
        );
        assert_eq!(read(&path).unwrap(), "Host b\n");
        assert!(!dir.join(".config.telepad-tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            backup_path(Path::new("/x/config"), 86_400 * 365 + 3661),
            PathBuf::from("/x/config.telepad-19710101-010101.bak")
        );
    }
}
