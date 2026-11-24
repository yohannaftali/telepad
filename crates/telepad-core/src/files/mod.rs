//! File window (#62–#64): local and remote folders side by side, transfers
//! over the host's normal SSH login.
//!
//! - [`sftp`]: SFTP client over `ssh -s <alias> sftp` (listing, transfers, file operations).
//! - [`fallback`]: hosts without an SFTP server — read-only listing with a
//!   POSIX `sh` script, transfers with `scp -O`.
//! - [`rsync`]: optional rsync mode when both sides have rsync.
//! - [`ops`]: new folder / file, rename, delete, permissions, sizes (#63).
//! - [`transfer`]: plan (expand folders, find files that exist) and run copies.
//! - [`edit`], [`text`], [`syntax`]: the editor (#64): load / save with a
//!   changed-on-disk check, line endings and find / replace, syntax colours.

pub mod compare;
pub mod edit;
pub mod fallback;
pub mod ops;
pub mod rsync;
pub mod sftp;
pub mod syntax;
pub mod text;
pub mod transfer;

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    File,
    Dir,
    Link,
    Other,
}

/// One row of a pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    /// A link that points to a folder.
    pub link_dir: bool,
    pub size: u64,
    /// Seconds since 1970 (UTC).
    pub mtime: Option<i64>,
    /// Unix mode bits (permissions only), when known.
    pub perm: Option<u32>,
    pub owner: String,
    /// Dot file, or the Windows hidden attribute.
    pub hidden: bool,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir || (self.kind == Kind::Link && self.link_dir)
    }

    fn from_sftp(name: String, longname: &str, a: sftp::Attrs) -> Self {
        // `drwxr-xr-x  2 root root 4096 Jan 1 00:00 name`: the owner is the 3rd field.
        let owner = longname
            .split_whitespace()
            .nth(2)
            .map(str::to_string)
            .or_else(|| a.uid.map(|u| u.to_string()))
            .unwrap_or_default();
        Self {
            hidden: name.starts_with('.'),
            name,
            kind: a.kind(),
            link_dir: false,
            size: a.size.unwrap_or(0),
            mtime: a.mtime.map(i64::from),
            perm: a.perm.map(|p| p & 0o7777),
            owner,
        }
    }
}

/// `rwxr-xr-x` for the permission bits.
pub fn perm_text(perm: Option<u32>) -> String {
    let Some(p) = perm else {
        return String::new();
    };
    let mut s = String::with_capacity(9);
    // setuid / setgid / sticky show as s / t in the x place (S / T without x), like `ls -l`.
    for (shift, special, mark) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let b = (p >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        let x = b & 1 != 0;
        s.push(match (p & special != 0, x) {
            (true, true) => mark,
            (true, false) => mark.to_ascii_uppercase(),
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    s
}

/// `2026-11-19 07:41` (UTC) for a time in seconds since 1970.
pub fn time_text(secs: Option<i64>) -> String {
    let Some(t) = secs else {
        return String::new();
    };
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// The local folder Telepad opens first: the user's home.
pub fn local_home() -> String {
    crate::expand_home("~").display().to_string()
}

#[cfg(test)]
mod home_tests {
    #[test]
    fn local_home_is_the_home_folder_not_a_tilde() {
        let h = super::local_home();
        assert_ne!(h, "~");
        assert!(std::path::Path::new(&h).is_dir(), "{h}");
    }
}

/// Top-level places for the local Up button: drive letters on Windows, `/` elsewhere.
pub fn local_roots() -> Vec<String> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(|c| format!("{}:\\", c as char))
            .filter(|d| Path::new(d).exists())
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec!["/".into()]
    }
}

/// Parent of a local folder; `None` at a root.
pub fn local_parent(path: &str) -> Option<String> {
    Path::new(path)
        .parent()
        .map(|p| p.display().to_string())
        .filter(|p| !p.is_empty())
}

pub fn local_join(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).display().to_string()
}

fn mtime_of(m: &std::fs::Metadata) -> Option<i64> {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// The entries of a local folder.
pub fn list_local(dir: &str) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("Cannot open {dir}: {e}"))?;
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(lm) = e
            .metadata()
            .or_else(|_| std::fs::symlink_metadata(e.path()))
        else {
            continue;
        };
        let link = std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.file_type().is_symlink());
        let target = std::fs::metadata(e.path()).ok();
        let m = target.as_ref().unwrap_or(&lm);
        let kind = if link {
            Kind::Link
        } else if m.is_dir() {
            Kind::Dir
        } else if m.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        out.push(Entry {
            hidden: name.starts_with('.') || hidden_attr(m),
            name,
            kind,
            link_dir: link && m.is_dir(),
            size: if m.is_dir() { 0 } else { m.len() },
            mtime: mtime_of(m),
            perm: unix_mode(m),
            owner: String::new(),
        });
    }
    Ok(out)
}

#[cfg(windows)]
fn hidden_attr(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn hidden_attr(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn unix_mode(m: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(m.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn unix_mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Where the local copy of a download goes.
pub fn local_path(dir: &str, name: &str) -> PathBuf {
    Path::new(dir).join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_bits_show_like_ls() {
        assert_eq!(perm_text(Some(0o2755)), "rwxr-sr-x");
        assert_eq!(perm_text(Some(0o1777)), "rwxrwxrwt");
        assert_eq!(perm_text(Some(0o4644)), "rwSr--r--");
        assert_eq!(perm_text(Some(0o644)), "rw-r--r--");
    }

    #[test]
    fn owner_and_permissions_from_sftp() {
        let e = Entry::from_sftp(
            ".env".into(),
            "-rw-r-----    1 deploy   www          120 Jan  1 00:00 .env",
            sftp::Attrs {
                size: Some(120),
                perm: Some(0o100640),
                mtime: Some(1_700_000_000),
                ..Default::default()
            },
        );
        assert_eq!(e.owner, "deploy");
        assert!(e.hidden);
        assert_eq!(perm_text(e.perm), "rw-r-----");
        assert_eq!(e.mtime, Some(1_700_000_000));
        assert!(!e.is_dir());
    }

    #[test]
    fn formats_times() {
        assert_eq!(time_text(Some(0)), "1970-01-01 00:00");
        assert_eq!(time_text(Some(1_763_538_060)), "2025-11-19 07:41");
        assert_eq!(time_text(None), "");
    }

    #[test]
    fn lists_a_local_folder() {
        let dir = std::env::temp_dir().join(format!("telepad-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub folder")).unwrap();
        std::fs::write(dir.join("ünïcode 'x'.txt"), b"hello").unwrap();
        let mut l = list_local(&dir.display().to_string()).unwrap();
        l.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(l.len(), 2);
        assert!(l[0].is_dir() && l[0].name == "sub folder");
        assert_eq!((l[1].name.as_str(), l[1].size), ("ünïcode 'x'.txt", 5));
        assert!(local_parent(&dir.display().to_string()).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod live_tests;
