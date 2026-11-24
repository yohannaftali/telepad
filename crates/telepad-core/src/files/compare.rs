//! Compare directories and Synchronize between the two panes of a file window
//! (#131). Comparing is by name, then modification time (with a little slack
//! for file systems that store whole or even seconds), then size. Links are
//! never followed and never synchronized.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use super::transfer::{Direction, Item, MAX_ITEMS, Options, Remote, plan_one};
use super::{Entry, Kind, list_local, local_join};

/// Times this close count as the same (FAT keeps even seconds).
pub const MTIME_SLACK: i64 = 2;

/// How an entry of one pane compares with the same name in the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Diff {
    OnlyLeft,
    OnlyRight,
    NewerLeft,
    NewerRight,
    /// Same time, other size (or a file on one side, a folder on the other).
    Different,
    Same,
}

impl Diff {
    pub fn title(self) -> &'static str {
        match self {
            Diff::OnlyLeft => "only on the left",
            Diff::OnlyRight => "only on the right",
            Diff::NewerLeft => "newer on the left",
            Diff::NewerRight => "newer on the right",
            Diff::Different => "different size",
            Diff::Same => "same",
        }
    }
}

/// Key a name is matched by: names differing only in case are the same file
/// when either side ignores case (Windows).
pub fn key(name: &str, case_insensitive: bool) -> String {
    if case_insensitive {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}

fn file_diff(l: &Entry, r: &Entry) -> Diff {
    if l.is_dir() != r.is_dir() {
        return Diff::Different;
    }
    if l.is_dir() {
        return Diff::Same;
    }
    match (l.mtime, r.mtime) {
        (Some(a), Some(b)) if a > b + MTIME_SLACK => Diff::NewerLeft,
        (Some(a), Some(b)) if b > a + MTIME_SLACK => Diff::NewerRight,
        _ if l.size != r.size => Diff::Different,
        _ => Diff::Same,
    }
}

/// Compare the entries both panes show (one folder, not recursive), by
/// [`key`]. Folders are only-left / only-right / same.
pub fn compare(left: &[Entry], right: &[Entry], case_insensitive: bool) -> BTreeMap<String, Diff> {
    let r: BTreeMap<String, &Entry> = right
        .iter()
        .map(|e| (key(&e.name, case_insensitive), e))
        .collect();
    let mut out = BTreeMap::new();
    for l in left {
        let k = key(&l.name, case_insensitive);
        let d = match r.get(&k) {
            Some(r) => file_diff(l, r),
            None => Diff::OnlyLeft,
        };
        out.insert(k, d);
    }
    for e in right {
        out.entry(key(&e.name, case_insensitive))
            .or_insert(Diff::OnlyRight);
    }
    out
}

/// Which way Synchronize copies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SyncDir {
    /// Make the right folder like the left one.
    #[default]
    LeftToRight,
    RightToLeft,
    /// Newer files both ways (never deletes).
    Both,
}

impl SyncDir {
    pub const ALL: [SyncDir; 3] = [Self::LeftToRight, Self::RightToLeft, Self::Both];
    pub fn title(self) -> &'static str {
        match self {
            Self::LeftToRight => "Left → right",
            Self::RightToLeft => "Right → left",
            Self::Both => "Both ways (newer wins)",
        }
    }
}

/// What Synchronize asks for, before it runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncRequest {
    pub dir: SyncDir,
    /// Also the folders inside (else only the files of the shown folders).
    pub recursive: bool,
    /// Delete what the target has and the source does not (one direction only).
    pub delete: bool,
}

/// An entry Synchronize would delete: its folder and the entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gone {
    pub dir: String,
    pub entry: Entry,
    /// Path shown to the user (relative to the synchronized folder).
    pub label: String,
}

/// Everything Synchronize would do, shown before anything runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncPlan {
    /// Left → right copies (folders before what is in them).
    pub upload: Vec<Item>,
    /// Right → left copies.
    pub download: Vec<Item>,
    pub delete_left: Vec<Gone>,
    pub delete_right: Vec<Gone>,
}

impl SyncPlan {
    pub fn is_empty(&self) -> bool {
        self.upload.is_empty()
            && self.download.is_empty()
            && self.delete_left.is_empty()
            && self.delete_right.is_empty()
    }
}

/// Plan Synchronize of the left folder `left` (this computer) with `right`
/// (the other pane, through `s`), honouring the transfer settings (`opts`:
/// excluded folders; "new and updated files only").
pub fn sync_plan(
    s: &mut dyn Remote,
    left: &str,
    right: &str,
    req: SyncRequest,
    opts: &Options,
    cancel: &AtomicBool,
) -> Result<SyncPlan, String> {
    let mut plan = SyncPlan::default();
    let req = SyncRequest {
        delete: req.delete && req.dir != SyncDir::Both,
        ..req
    };
    sync_dir(s, left, right, "", req, opts, &mut plan, cancel)?;
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
fn sync_dir(
    s: &mut dyn Remote,
    l_dir: &str,
    r_dir: &str,
    prefix: &str,
    req: SyncRequest,
    opts: &Options,
    plan: &mut SyncPlan,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    if plan.upload.len() + plan.download.len() >= MAX_ITEMS {
        return Err(format!(
            "More than {MAX_ITEMS} files and folders; synchronize a smaller folder"
        ));
    }
    let keep = |e: &Entry| {
        e.kind != Kind::Link
            && e.kind != Kind::Other
            && !(e.is_dir() && opts.excluded(&e.name))
            && (req.recursive || !e.is_dir())
    };
    let left: Vec<Entry> = list_local(l_dir)?.into_iter().filter(keep).collect();
    let right: Vec<Entry> = s.list(r_dir)?.into_iter().filter(keep).collect();
    let ci = cfg!(windows) || s.crlf();
    let lmap: BTreeMap<String, &Entry> = left.iter().map(|e| (key(&e.name, ci), e)).collect();
    let rmap: BTreeMap<String, &Entry> = right.iter().map(|e| (key(&e.name, ci), e)).collect();
    let names: BTreeSet<&String> = lmap.keys().chain(rmap.keys()).collect();
    let to_right = req.dir != SyncDir::RightToLeft;
    let to_left = req.dir != SyncDir::LeftToRight;
    for k in names {
        let (l, r) = (lmap.get(k).copied(), rmap.get(k).copied());
        let name = l.or(r).map(|e| e.name.clone()).unwrap_or_default();
        let label = format!("{prefix}{name}");
        match (l, r) {
            (Some(l), None) => {
                let src = local_join(l_dir, &l.name);
                if to_right {
                    let dst = s.join(r_dir, &l.name);
                    let is_dir = l.is_dir();
                    plan_one(
                        s,
                        Direction::Upload,
                        &src,
                        &dst,
                        &label,
                        is_dir,
                        opts,
                        &mut plan.upload,
                        cancel,
                    )?;
                } else if req.delete {
                    plan.delete_left.push(Gone {
                        dir: l_dir.to_string(),
                        entry: l.clone(),
                        label: dir_label(&label, l),
                    });
                }
            }
            (None, Some(r)) => {
                let src = s.join(r_dir, &r.name);
                if to_left {
                    let dst = local_join(l_dir, &r.name);
                    plan_one(
                        s,
                        Direction::Download,
                        &src,
                        &dst,
                        &label,
                        r.is_dir(),
                        opts,
                        &mut plan.download,
                        cancel,
                    )?;
                } else if req.delete {
                    plan.delete_right.push(Gone {
                        dir: r_dir.to_string(),
                        entry: r.clone(),
                        label: dir_label(&label, r),
                    });
                }
            }
            (Some(l), Some(r)) if l.is_dir() && r.is_dir() => {
                let (ld, rd) = (local_join(l_dir, &l.name), s.join(r_dir, &r.name));
                sync_dir(s, &ld, &rd, &format!("{label}/"), req, opts, plan, cancel)?;
            }
            (Some(l), Some(r)) if !l.is_dir() && !r.is_dir() => {
                let d = file_diff(l, r);
                let wanted = |newer: Diff, one_way: bool| {
                    d == newer
                        || (one_way
                            && !opts.newer_only
                            && matches!(d, Diff::NewerLeft | Diff::NewerRight | Diff::Different))
                };
                if to_right && wanted(Diff::NewerLeft, req.dir == SyncDir::LeftToRight) {
                    plan.upload.push(Item {
                        src: local_join(l_dir, &l.name),
                        dst: s.join(r_dir, &r.name),
                        label,
                        is_dir: false,
                        size: l.size,
                        mtime: l.mtime,
                        perm: l.perm,
                        exists: Some((r.size, r.mtime)),
                    });
                } else if to_left && wanted(Diff::NewerRight, req.dir == SyncDir::RightToLeft) {
                    plan.download.push(Item {
                        src: s.join(r_dir, &r.name),
                        dst: local_join(l_dir, &l.name),
                        label,
                        is_dir: false,
                        size: r.size,
                        mtime: r.mtime,
                        perm: r.perm,
                        exists: Some((l.size, l.mtime)),
                    });
                }
            }
            // A file on one side and a folder on the other: left alone.
            _ => {}
        }
    }
    Ok(())
}

fn dir_label(label: &str, e: &Entry) -> String {
    if e.is_dir() {
        format!("{label}/")
    } else {
        label.to_string()
    }
}

/// Lines of `left` / `right` that differ (for the side-by-side compare in the
/// editor, #131): common lines at the start and end are matched first, then
/// an LCS on the rest when it is small enough, else the rest is marked.
pub fn line_marks(left: &str, right: &str) -> (Vec<bool>, Vec<bool>) {
    const MAX_CELLS: usize = 4_000_000;
    let a: Vec<&str> = left.lines().collect();
    let b: Vec<&str> = right.lines().collect();
    let mut ma = vec![false; a.len()];
    let mut mb = vec![false; b.len()];
    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let (mut ea, mut eb) = (a.len(), b.len());
    while ea > start && eb > start && a[ea - 1] == b[eb - 1] {
        ea -= 1;
        eb -= 1;
    }
    let (x, y) = (&a[start..ea], &b[start..eb]);
    if x.len().saturating_mul(y.len()) > MAX_CELLS {
        ma[start..ea].fill(true);
        mb[start..eb].fill(true);
        return (ma, mb);
    }
    let (n, m) = (x.len(), y.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if x[i] == y[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && x[i] == y[j] {
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            ma[start + i] = true;
            i += 1;
        } else {
            mb[start + j] = true;
            j += 1;
        }
    }
    (ma, mb)
}

/// For synchronized scrolling (#135): the line of the other side that
/// matches each line, from [`line_marks`]. A deleted / added line maps to
/// the next line both sides share.
pub fn line_map(left: &[bool], right: &[bool]) -> (Vec<usize>, Vec<usize>) {
    let (mut l2r, mut r2l) = (
        Vec::with_capacity(left.len()),
        Vec::with_capacity(right.len()),
    );
    let (mut i, mut j) = (0, 0);
    while i < left.len() || j < right.len() {
        match (left.get(i), right.get(j)) {
            (Some(true), _) => {
                l2r.push(j);
                i += 1;
            }
            (_, Some(true)) => {
                r2l.push(i);
                j += 1;
            }
            (Some(false), Some(false)) => {
                l2r.push(j);
                r2l.push(i);
                i += 1;
                j += 1;
            }
            (Some(false), None) => {
                l2r.push(j);
                i += 1;
            }
            (None, Some(false)) => {
                r2l.push(i);
                j += 1;
            }
            (None, None) => break,
        }
    }
    (l2r, r2l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::transfer::ThisComputer;

    fn e(name: &str, dir: bool, size: u64, mtime: i64) -> Entry {
        Entry {
            name: name.into(),
            kind: if dir { Kind::Dir } else { Kind::File },
            size,
            mtime: Some(mtime),
            ..Default::default()
        }
    }

    #[test]
    fn compares_by_name_time_then_size() {
        let left = [
            e("a.txt", false, 10, 1000),
            e("b.txt", false, 10, 2000),
            e("c.txt", false, 10, 1000),
            e("d.txt", false, 11, 1001),
            e("same.txt", false, 5, 500),
            e("only-l", true, 0, 0),
            e("Case.TXT", false, 1, 1),
        ];
        let right = [
            e("a.txt", false, 10, 2000),
            e("b.txt", false, 10, 1000),
            e("c.txt", false, 10, 1001),
            e("d.txt", false, 12, 1000),
            e("same.txt", false, 5, 500),
            e("only-r.txt", false, 1, 1),
            e("case.txt", false, 1, 1),
        ];
        let c = compare(&left, &right, true);
        assert_eq!(c["a.txt"], Diff::NewerRight);
        assert_eq!(c["b.txt"], Diff::NewerLeft);
        assert_eq!(c["c.txt"], Diff::Same, "within the time slack");
        assert_eq!(c["d.txt"], Diff::Different);
        assert_eq!(c["same.txt"], Diff::Same);
        assert_eq!(c["only-l"], Diff::OnlyLeft);
        assert_eq!(c["only-r.txt"], Diff::OnlyRight);
        assert_eq!(c["case.txt"], Diff::Same);
        let c = compare(&left, &right, false);
        assert_eq!(c["Case.TXT"], Diff::OnlyLeft);
    }

    #[test]
    fn maps_matching_lines_for_synchronized_scrolling() {
        // left: a b c d   right: a X b d e  (c deleted; X, e added)
        let (l, r) = line_marks("a\nb\nc\nd", "a\nX\nb\nd\ne");
        assert_eq!(l, [false, false, true, false]);
        assert_eq!(r, [false, true, false, false, true]);
        let (l2r, r2l) = line_map(&l, &r);
        assert_eq!(l2r, [0, 2, 3, 3]);
        assert_eq!(r2l, [0, 1, 1, 3, 4]);
    }

    #[test]
    fn marks_the_lines_that_differ() {
        let (a, b) = line_marks("one\ntwo\nthree\nfour", "one\n2\nthree\nfour\nfive");
        assert_eq!(a, [false, true, false, false]);
        assert_eq!(b, [false, true, false, false, true]);
        let (a, b) = line_marks("x\ny", "x\ny");
        assert!(a.iter().chain(&b).all(|m| !m));
    }

    fn touch(p: &std::path::Path, text: &str, mtime: u64) {
        std::fs::write(p, text).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime))
            .unwrap();
    }

    #[test]
    fn synchronize_plans_copies_and_deletes_by_direction() {
        let root = std::env::temp_dir().join(format!("telepad-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (l, r) = (root.join("l"), root.join("r"));
        for d in [
            l.join("sub"),
            r.join("sub"),
            l.join("node_modules"),
            r.join("extra"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        touch(&l.join("new.txt"), "n", 1_700_000_000);
        touch(&l.join("changed.txt"), "new", 1_700_000_100);
        touch(&r.join("changed.txt"), "old", 1_700_000_000);
        touch(&l.join("older.txt"), "o", 1_700_000_000);
        touch(&r.join("older.txt"), "newer", 1_700_000_100);
        touch(&l.join("sub/deep.txt"), "d", 1_700_000_000);
        touch(&l.join("node_modules/x.js"), "x", 1_700_000_000);
        touch(&r.join("gone.txt"), "g", 1_700_000_000);
        let (ls, rs) = (l.display().to_string(), r.display().to_string());
        let no = AtomicBool::new(false);
        let opts = Options {
            exclude: vec!["node_*".into()],
            ..Default::default()
        };
        let labels = |v: &[Item]| v.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
        let req = SyncRequest {
            dir: SyncDir::LeftToRight,
            recursive: true,
            delete: true,
        };
        let p = sync_plan(&mut ThisComputer, &ls, &rs, req, &opts, &no).unwrap();
        assert_eq!(
            labels(&p.upload),
            ["changed.txt", "new.txt", "older.txt", "sub/deep.txt"],
            "one way without 'newer only': the left wins, excluded folders stay out"
        );
        assert!(p.download.is_empty() && p.delete_left.is_empty());
        let gone: Vec<&str> = p.delete_right.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(gone, ["extra/", "gone.txt"]);
        // New and updated only: the right's newer file stays.
        let newer = Options {
            newer_only: true,
            ..opts.clone()
        };
        let p = sync_plan(&mut ThisComputer, &ls, &rs, req, &newer, &no).unwrap();
        assert_eq!(
            labels(&p.upload),
            ["changed.txt", "new.txt", "sub/deep.txt"]
        );
        // Both ways: newer wins, nothing deleted; not recursive: files only.
        let both = SyncRequest {
            dir: SyncDir::Both,
            recursive: false,
            delete: true,
        };
        let p = sync_plan(&mut ThisComputer, &ls, &rs, both, &opts, &no).unwrap();
        assert_eq!(labels(&p.upload), ["changed.txt", "new.txt"]);
        assert_eq!(labels(&p.download), ["gone.txt", "older.txt"]);
        assert!(p.delete_left.is_empty() && p.delete_right.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
