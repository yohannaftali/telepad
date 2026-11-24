//! Transfers between a local folder and the other pane (the host over SFTP,
//! or this computer again for My Computer, #129): first a plan (folders
//! expanded, files that already exist found), then the copy with the user's
//! decisions for those files.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::sftp::{self, Attrs, Sftp};
use super::{Entry, Kind, list_local, local_join};

/// The right-hand pane of a file window: the host over SFTP, or this
/// computer ([`ThisComputer`], #129). "Upload" copies into it, "Download"
/// out of it.
pub trait Remote {
    fn join(&self, dir: &str, name: &str) -> String;
    /// (size, mtime) when the path exists.
    fn exists(&mut self, path: &str) -> Option<(u64, Option<i64>)>;
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>, String>;
    /// (size, mtime, permission bits) of a file.
    fn stat(&mut self, path: &str) -> Result<(u64, Option<i64>, Option<u32>), String>;
    fn mkdir(&mut self, path: &str) -> Result<(), String>;
    /// Write `src` to `path` (created or truncated).
    fn put(
        &mut self,
        src: &mut dyn Read,
        path: &str,
        perm: Option<u32>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String>;
    /// Read `path` into `dest`.
    fn get(
        &mut self,
        path: &str,
        dest: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String>;
    /// Best effort: keep the source's time so "newer only" works next time.
    fn set_mtime(&mut self, path: &str, mtime: i64);
    fn remove(&mut self, path: &str) -> Result<(), String>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;
    /// Remove an empty folder (Synchronize's delete, #131).
    fn rmdir(&mut self, path: &str) -> Result<(), String>;
    /// Text files written here end lines with CRLF (Text mode, #131).
    fn crlf(&self) -> bool {
        false
    }
}

impl Remote for Sftp {
    fn join(&self, dir: &str, name: &str) -> String {
        sftp::join(dir, name)
    }
    fn exists(&mut self, path: &str) -> Option<(u64, Option<i64>)> {
        remote_exists(self, path)
    }
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>, String> {
        Sftp::list(self, dir)
    }
    fn stat(&mut self, path: &str) -> Result<(u64, Option<i64>, Option<u32>), String> {
        let a = Sftp::stat(self, path)?;
        Ok((
            a.size.unwrap_or(0),
            a.mtime.map(i64::from),
            a.perm.map(|p| p & 0o7777),
        ))
    }
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        Sftp::mkdir(self, path, None)
    }
    fn put(
        &mut self,
        src: &mut dyn Read,
        path: &str,
        perm: Option<u32>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        self.upload(src, path, perm, cancel, progress).map(|_| ())
    }
    fn get(
        &mut self,
        path: &str,
        dest: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        self.download(path, dest, cancel, progress).map(|_| ())
    }
    fn set_mtime(&mut self, path: &str, m: i64) {
        let _ = self.setstat(
            path,
            Attrs {
                atime: Some(m as u32),
                mtime: Some(m as u32),
                ..Default::default()
            },
        );
    }
    fn remove(&mut self, path: &str) -> Result<(), String> {
        Sftp::remove(self, path)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        Sftp::rename(self, from, to)
    }
    fn rmdir(&mut self, path: &str) -> Result<(), String> {
        Sftp::rmdir(self, path)
    }
}

/// This computer as the right-hand pane (My Computer, #129).
#[derive(Debug, Default, Clone, Copy)]
pub struct ThisComputer;

/// Copy `src` to `dest` in chunks, reporting bytes and stopping on cancel.
fn pump(
    src: &mut dyn Read,
    dest: &mut dyn Write,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let n = src.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        dest.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        progress(n as u64);
    }
}

impl Remote for ThisComputer {
    fn join(&self, dir: &str, name: &str) -> String {
        local_join(dir, name)
    }
    fn exists(&mut self, path: &str) -> Option<(u64, Option<i64>)> {
        local_exists(path)
    }
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>, String> {
        list_local(dir)
    }
    fn stat(&mut self, path: &str) -> Result<(u64, Option<i64>, Option<u32>), String> {
        let m = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
        Ok((m.len(), mtime_of(&m), local_mode(&m)))
    }
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        std::fs::create_dir(path).map_err(|e| format!("{path}: {e}"))
    }
    fn put(
        &mut self,
        src: &mut dyn Read,
        path: &str,
        _perm: Option<u32>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let mut f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
        pump(src, &mut f, cancel, progress)
    }
    fn get(
        &mut self,
        path: &str,
        dest: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let mut f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        pump(&mut f, dest, cancel, progress)
    }
    fn set_mtime(&mut self, path: &str, m: i64) {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
            let _ = f.set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(m.max(0) as u64),
            );
        }
    }
    fn remove(&mut self, path: &str) -> Result<(), String> {
        std::fs::remove_file(path).map_err(|e| format!("{path}: {e}"))
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| format!("{to}: {e}"))
    }
    fn rmdir(&mut self, path: &str) -> Result<(), String> {
        std::fs::remove_dir(path).map_err(|e| format!("{path}: {e}"))
    }
    fn crlf(&self) -> bool {
        cfg!(windows)
    }
}

/// How file contents travel (#131, Transfer settings).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TransferMode {
    /// Text for known text files ([`is_text_name`]), else binary.
    #[default]
    Default,
    /// Line endings changed to the target's (CRLF on Windows, else LF).
    Text,
    /// Bytes unchanged.
    Binary,
}

impl TransferMode {
    pub const ALL: [TransferMode; 3] = [Self::Default, Self::Text, Self::Binary];
    pub fn title(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Text => "Text",
            Self::Binary => "Binary",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Text => "text",
            Self::Binary => "binary",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "text" => Self::Text,
            "binary" => Self::Binary,
            _ => Self::Default,
        }
    }
}

/// Transfer settings of a file window (#131), saved per host.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    pub mode: TransferMode,
    /// New and updated files only: an existing target is replaced only when
    /// the source is newer, without asking.
    pub newer_only: bool,
    /// Folder names (or `*` / `?` patterns) never copied, e.g. `node_modules`.
    pub exclude: Vec<String>,
}

/// Files larger than this always travel as binary.
pub const MAX_TEXT: u64 = 64 * 1024 * 1024;

impl Options {
    /// A folder of this name is left out.
    pub fn excluded(&self, name: &str) -> bool {
        self.exclude.iter().any(|p| glob(p, name))
    }
    /// This file travels as text.
    pub fn text_for(&self, name: &str, size: u64) -> bool {
        size <= MAX_TEXT
            && match self.mode {
                TransferMode::Binary => false,
                TransferMode::Text => true,
                TransferMode::Default => is_text_name(name),
            }
    }
    /// `node_modules; .git, dist` → the list.
    pub fn parse_exclude(text: &str) -> Vec<String> {
        text.split([';', ',', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }
    /// Saved form: `mode|newer|exclude;exclude`.
    pub fn to_setting(&self) -> String {
        format!(
            "{}|{}|{}",
            self.mode.key(),
            u8::from(self.newer_only),
            self.exclude.join(";")
        )
    }
    pub fn from_setting(s: &str) -> Self {
        let mut it = s.splitn(3, '|');
        Self {
            mode: TransferMode::parse(it.next().unwrap_or("")),
            newer_only: it.next() == Some("1"),
            exclude: Self::parse_exclude(it.next().unwrap_or("")),
        }
    }
}

/// `*` (any run) and `?` (one character), ASCII case ignored.
pub fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let n: Vec<char> = name.to_ascii_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Known text files for the Default mode, by extension or name.
pub fn is_text_name(name: &str) -> bool {
    const EXT: &[&str] = &[
        "txt",
        "md",
        "markdown",
        "rst",
        "log",
        "csv",
        "tsv",
        "sh",
        "bash",
        "zsh",
        "ps1",
        "bat",
        "cmd",
        "conf",
        "cfg",
        "cnf",
        "ini",
        "env",
        "properties",
        "toml",
        "yml",
        "yaml",
        "json",
        "xml",
        "html",
        "htm",
        "css",
        "scss",
        "less",
        "js",
        "mjs",
        "cjs",
        "ts",
        "tsx",
        "jsx",
        "vue",
        "php",
        "py",
        "rb",
        "pl",
        "lua",
        "go",
        "rs",
        "java",
        "kt",
        "c",
        "h",
        "cpp",
        "hpp",
        "cs",
        "sql",
        "service",
        "timer",
        "socket",
        "rules",
        "htaccess",
        "gitignore",
        "dockerignore",
        "editorconfig",
        "svg",
    ];
    const NAMES: &[&str] = &[
        "dockerfile",
        "makefile",
        "readme",
        "license",
        "authorized_keys",
    ];
    let lower = name.to_ascii_lowercase();
    let base = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
    if NAMES.contains(&base) {
        return true;
    }
    match base.rsplit_once('.') {
        Some((_, ext)) => EXT.contains(&ext),
        None => false,
    }
}

/// Line endings made CRLF (`crlf`) or LF; `None` for binary data (a NUL
/// byte), which is then copied unchanged.
pub fn convert_eol(bytes: &[u8], crlf: bool) -> Option<Vec<u8>> {
    if bytes.contains(&0) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {}
            b'\n' => {
                if crlf {
                    out.push(b'\r');
                }
                out.push(b'\n');
            }
            b => out.push(b),
        }
        i += 1;
    }
    Some(out)
}

fn mtime_of(m: &std::fs::Metadata) -> Option<i64> {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// Most items one transfer may plan (keeps a mistaken `/` drag in check).
pub const MAX_ITEMS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
}

/// One file or folder to copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub src: String,
    pub dst: String,
    /// Path shown to the user (relative to the source folder).
    pub label: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<i64>,
    pub perm: Option<u32>,
    /// The target exists already: (size, mtime).
    pub exists: Option<(u64, Option<i64>)>,
}

/// What to do with a file that exists at the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Overwrite,
    Skip,
    /// Overwrite only when the source is newer.
    NewerOnly,
}

impl Item {
    /// Copy this item, given the user's decision for existing files.
    pub fn should_copy(&self, d: Option<Decision>) -> bool {
        match (self.exists, d) {
            (None, _) | (Some(_), Some(Decision::Overwrite)) => true,
            (Some(_), Some(Decision::Skip) | None) => false,
            (Some((_, dst)), Some(Decision::NewerOnly)) => match (self.mtime, dst) {
                (Some(s), Some(d)) => s > d,
                _ => true,
            },
        }
    }
}

/// Source: (path, is folder).
pub type Source = (String, bool);

fn name_of(path: &str) -> String {
    let p = path.trim_end_matches(['/', '\\']);
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

fn remote_exists(s: &mut Sftp, path: &str) -> Option<(u64, Option<i64>)> {
    s.stat(path)
        .ok()
        .map(|a| (a.size.unwrap_or(0), a.mtime.map(i64::from)))
}

fn local_exists(path: &str) -> Option<(u64, Option<i64>)> {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), mtime_of(&m)))
}

/// Expand `sources` (folders recursively) into items copied into `dest_dir`.
/// Folders in `opts.exclude` are left out, also inside copied folders.
pub fn plan(
    s: &mut dyn Remote,
    dir: Direction,
    sources: &[Source],
    dest_dir: &str,
    opts: &Options,
    cancel: &AtomicBool,
) -> Result<Vec<Item>, String> {
    let mut out = Vec::new();
    for (src, is_dir) in sources {
        let name = name_of(src);
        let dst = match dir {
            Direction::Upload => s.join(dest_dir, &name),
            Direction::Download => local_join(dest_dir, &name),
        };
        walk(s, dir, src, &dst, &name, *is_dir, opts, &mut out, cancel)?;
    }
    Ok(out)
}

/// Items for one source (file or folder) copied to `dst`, as [`plan`] makes
/// them; Synchronize (#131) uses it for what one side lacks.
#[allow(clippy::too_many_arguments)]
pub fn plan_one(
    s: &mut dyn Remote,
    dir: Direction,
    src: &str,
    dst: &str,
    label: &str,
    is_dir: bool,
    opts: &Options,
    out: &mut Vec<Item>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    walk(s, dir, src, dst, label, is_dir, opts, out, cancel)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    s: &mut dyn Remote,
    dir: Direction,
    src: &str,
    dst: &str,
    label: &str,
    is_dir: bool,
    opts: &Options,
    out: &mut Vec<Item>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    if is_dir && opts.excluded(&name_of(src)) {
        return Ok(());
    }
    if out.len() >= MAX_ITEMS {
        return Err(format!(
            "More than {MAX_ITEMS} files and folders; copy a smaller folder"
        ));
    }
    let exists = match dir {
        Direction::Upload => s.exists(dst),
        Direction::Download => local_exists(dst),
    };
    if is_dir {
        out.push(Item {
            src: src.into(),
            dst: dst.into(),
            label: format!("{label}/"),
            is_dir: true,
            exists,
            ..Default::default()
        });
        let children: Vec<(String, bool)> = match dir {
            Direction::Upload => list_local(src)?
                .into_iter()
                .filter(|e| e.kind != Kind::Link || e.link_dir || e.size > 0)
                .map(|e| (e.name.clone(), e.is_dir() && e.kind != Kind::Link))
                .collect(),
            Direction::Download => s
                .list(src)?
                .into_iter()
                // Links to folders are not followed (no loops); file links are copied as files.
                .filter(|e| !(e.kind == Kind::Link && e.link_dir))
                .map(|e| (e.name.clone(), e.kind == Kind::Dir))
                .collect(),
        };
        for (n, d) in children {
            let (cs, cd) = match dir {
                Direction::Upload => (local_join(src, &n), s.join(dst, &n)),
                Direction::Download => (s.join(src, &n), local_join(dst, &n)),
            };
            walk(
                s,
                dir,
                &cs,
                &cd,
                &format!("{label}/{n}"),
                d,
                opts,
                out,
                cancel,
            )?;
        }
        return Ok(());
    }
    let (size, mtime, perm) = match dir {
        Direction::Upload => {
            let m = std::fs::metadata(src).map_err(|e| format!("{src}: {e}"))?;
            (m.len(), mtime_of(&m), local_mode(&m))
        }
        Direction::Download => s.stat(src)?,
    };
    out.push(Item {
        src: src.into(),
        dst: dst.into(),
        label: label.into(),
        is_dir: false,
        size,
        mtime,
        perm,
        exists,
    });
    Ok(())
}

#[cfg(unix)]
fn local_mode(m: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn local_mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

/// What the runner reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Item index started.
    Start(usize),
    /// Bytes of the current file.
    Bytes(u64),
    /// Item done: `Ok(true)` copied, `Ok(false)` skipped, `Err` failed.
    Done(usize, Result<bool, String>),
}

/// Copy the planned items in order; a failed item does not stop the rest.
/// Returns (copied, skipped, failed).
pub fn run(
    s: &mut dyn Remote,
    dir: Direction,
    items: &[Item],
    decisions: &HashMap<usize, Decision>,
    opts: &Options,
    cancel: &AtomicBool,
    events: &mut dyn FnMut(Event),
) -> (usize, usize, usize) {
    let (mut copied, mut skipped, mut failed) = (0, 0, 0);
    for (i, it) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        events(Event::Start(i));
        let r = if it.is_dir {
            match dir {
                Direction::Upload => {
                    if it.exists.is_some() {
                        Ok(false)
                    } else {
                        s.mkdir(&it.dst).map(|()| true)
                    }
                }
                Direction::Download => std::fs::create_dir_all(&it.dst)
                    .map(|()| it.exists.is_none())
                    .map_err(|e| e.to_string()),
            }
        } else if !it.should_copy(decisions.get(&i).copied()) {
            Ok(false)
        } else {
            copy_file(s, dir, it, opts, cancel, events).map(|()| true)
        };
        match &r {
            Ok(true) => copied += 1,
            Ok(false) => skipped += 1,
            Err(_) => failed += 1,
        }
        events(Event::Done(i, r));
    }
    (copied, skipped, failed)
}

fn copy_file(
    s: &mut dyn Remote,
    dir: Direction,
    it: &Item,
    opts: &Options,
    cancel: &AtomicBool,
    events: &mut dyn FnMut(Event),
) -> Result<(), String> {
    let mut progress = |n: u64| events(Event::Bytes(n));
    let text = opts.text_for(&name_of(&it.src), it.size);
    match dir {
        Direction::Upload => {
            let mut f = std::fs::File::open(&it.src).map_err(|e| format!("{}: {e}", it.src))?;
            // Upload next to the target, then rename: a cancelled or failed
            // upload never leaves half a file in place of a good one.
            let part = format!("{}.telepad-part", it.dst);
            let result = (|| {
                if text {
                    let mut bytes = Vec::new();
                    f.read_to_end(&mut bytes)
                        .map_err(|e| format!("{}: {e}", it.src))?;
                    let bytes = convert_eol(&bytes, s.crlf()).unwrap_or(bytes);
                    s.put(&mut bytes.as_slice(), &part, it.perm, cancel, &mut progress)?;
                } else {
                    s.put(&mut f, &part, it.perm, cancel, &mut progress)?;
                }
                if let Some(m) = it.mtime {
                    s.set_mtime(&part, m);
                }
                if it.exists.is_some() {
                    s.remove(&it.dst)?;
                }
                s.rename(&part, &it.dst)
            })();
            if result.is_err() {
                let _ = s.remove(&part);
            }
            result
        }
        Direction::Download => {
            // Write next to the target, then rename: a failed copy never
            // leaves half a file in place of a good one.
            let part = format!("{}.telepad-part", it.dst);
            let result = (|| {
                let mut f = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", it.dst))?;
                if text {
                    let mut bytes = Vec::new();
                    s.get(&it.src, &mut bytes, cancel, &mut progress)?;
                    let bytes = convert_eol(&bytes, cfg!(windows)).unwrap_or(bytes);
                    f.write_all(&bytes)
                        .map_err(|e| format!("{}: {e}", it.dst))?;
                } else {
                    s.get(&it.src, &mut f, cancel, &mut progress)?;
                }
                if let Some(m) = it.mtime {
                    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(m.max(0) as u64);
                    let _ = f.set_modified(t);
                }
                drop(f);
                if Path::new(&it.dst).exists() {
                    std::fs::remove_file(&it.dst).map_err(|e| format!("{}: {e}", it.dst))?;
                }
                std::fs::rename(&part, &it.dst).map_err(|e| format!("{}: {e}", it.dst))
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&part);
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(src_m: Option<i64>, dst: Option<(u64, Option<i64>)>) -> Item {
        Item {
            mtime: src_m,
            exists: dst,
            ..Default::default()
        }
    }

    #[test]
    fn decisions_for_existing_files() {
        assert!(item(None, None).should_copy(None), "new files always copy");
        let older = item(Some(10), Some((1, Some(20))));
        let newer = item(Some(30), Some((1, Some(20))));
        assert!(
            !older.should_copy(None),
            "undecided existing files are skipped"
        );
        assert!(older.should_copy(Some(Decision::Overwrite)));
        assert!(!older.should_copy(Some(Decision::Skip)));
        assert!(!older.should_copy(Some(Decision::NewerOnly)));
        assert!(newer.should_copy(Some(Decision::NewerOnly)));
        assert!(item(None, Some((1, None))).should_copy(Some(Decision::NewerOnly)));
    }

    #[test]
    fn copies_between_two_folders_of_this_computer() {
        // #129: My Computer's file window has this computer on both sides.
        let root = std::env::temp_dir().join(format!("telepad-xfer-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(a.join("site/css")).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("site/index.html"), "<h1>hi</h1>").unwrap();
        std::fs::write(a.join("site/css/x.css"), "p{}").unwrap();
        std::fs::write(b.join("old.txt"), "keep").unwrap();
        let src = a.join("site").display().to_string();
        let dest = b.display().to_string();
        let no = AtomicBool::new(false);
        let mut fs = ThisComputer;
        let opts = Options::default();
        let items = plan(
            &mut fs,
            Direction::Upload,
            &[(src, true)],
            &dest,
            &opts,
            &no,
        )
        .unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"site/") && labels.contains(&"site/css/x.css"));
        let (copied, skipped, failed) = run(
            &mut fs,
            Direction::Upload,
            &items,
            &HashMap::new(),
            &opts,
            &no,
            &mut |_| {},
        );
        assert_eq!((copied, skipped, failed), (items.len(), 0, 0));
        assert_eq!(
            std::fs::read_to_string(b.join("site/css/x.css")).unwrap(),
            "p{}"
        );
        assert_eq!(std::fs::read_to_string(b.join("old.txt")).unwrap(), "keep");
        // Copying again finds every file and skips them without a decision.
        let again = plan(
            &mut fs,
            Direction::Upload,
            &[(a.join("site").display().to_string(), true)],
            &dest,
            &opts,
            &no,
        )
        .unwrap();
        assert!(
            again
                .iter()
                .filter(|i| !i.is_dir)
                .all(|i| i.exists.is_some())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn names_of_local_and_remote_paths() {
        assert_eq!(name_of("/srv/www/"), "www");
        assert_eq!(name_of("C:\\Users\\me\\site"), "site");
        assert_eq!(name_of("a b.txt"), "a b.txt");
    }
}

#[cfg(test)]
mod options_tests {
    use super::*;

    #[test]
    fn transfer_settings_text_mode_and_excludes() {
        assert!(glob("node_*", "node_modules"));
        assert!(glob("*.git", "repo.GIT"));
        assert!(glob("?ist", "dist"));
        assert!(!glob("dist", "distro"));
        let o = Options {
            mode: TransferMode::Default,
            newer_only: true,
            exclude: Options::parse_exclude("node_modules; .git, dist\n"),
        };
        assert_eq!(o.exclude, ["node_modules", ".git", "dist"]);
        assert_eq!(Options::from_setting(&o.to_setting()), o);
        assert_eq!(Options::from_setting(""), Options::default());
        assert!(o.excluded(".git") && !o.excluded("src"));
        assert!(o.text_for("deploy.sh", 10) && o.text_for("Dockerfile", 10));
        assert!(!o.text_for("photo.jpg", 10));
        assert!(!o.text_for("huge.log", MAX_TEXT + 1));
        let bin = Options {
            mode: TransferMode::Binary,
            ..Default::default()
        };
        assert!(!bin.text_for("a.txt", 1));
        let text = Options {
            mode: TransferMode::Text,
            ..Default::default()
        };
        assert!(text.text_for("noext", 1));
        assert_eq!(convert_eol(b"a\r\nb\nc", true).unwrap(), b"a\r\nb\r\nc");
        assert_eq!(convert_eol(b"a\r\nb\nc\r\n", false).unwrap(), b"a\nb\nc\n");
        assert_eq!(
            convert_eol(b"bin\0\n", false),
            None,
            "binary stays as it is"
        );
    }

    #[test]
    fn text_mode_converts_line_endings_between_folders() {
        let root = std::env::temp_dir().join(format!("telepad-xfer-text-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join("a.conf"), b"x=1\r\ny=2\r\n").unwrap();
        let no = AtomicBool::new(false);
        let opts = Options::default();
        let mut fs = ThisComputer;
        let items = plan(
            &mut fs,
            Direction::Upload,
            &[(root.join("a.conf").display().to_string(), false)],
            &root.join("out").display().to_string(),
            &opts,
            &no,
        )
        .unwrap();
        run(
            &mut fs,
            Direction::Upload,
            &items,
            &HashMap::new(),
            &opts,
            &no,
            &mut |_| {},
        );
        let want: &[u8] = if cfg!(windows) {
            b"x=1\r\ny=2\r\n"
        } else {
            b"x=1\ny=2\n"
        };
        assert_eq!(
            std::fs::read(root.join("out").join("a.conf")).unwrap(),
            want
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
