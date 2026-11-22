//! My keys tab of the Authorized keys page (#101): the key pairs on this
//! computer, Generate (ssh-keygen) and Convert / Export / Import. Pure state;
//! the runtime scans, runs ssh-keygen, converts and writes files.

use std::path::PathBuf;

use telepad_core::db::Secret;
use telepad_core::local_keys::{Format, GenType, LocalKey, Target};

/// What the tab shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MyKeys {
    /// `None` until scanned once.
    pub keys: Option<Result<Vec<LocalKey>, String>>,
    pub loading: bool,
    /// `~/.ssh` (where new keys go).
    pub dir: PathBuf,
    /// `user@computer` (default comment of new keys).
    pub user_host: String,
}

/// Generate key pair dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenForm {
    pub kind: GenType,
    pub rsa_bits: u32,
    pub path: String,
    pub comment: String,
    pub pass: Secret,
    pub pass2: Secret,
    pub error: Option<String>,
    pub busy: bool,
}

/// What a converted key is protected with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassMode {
    /// The source's passphrase (none if it had none).
    #[default]
    Keep,
    New,
    Remove,
}

/// Convert / Export / Import dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertForm {
    pub source: PathBuf,
    pub format: Option<Format>,
    pub encrypted: bool,
    /// From an Import (the result goes to `~/.ssh` as OpenSSH by default).
    pub import: bool,
    pub algorithm: String,
    pub pass_in: Secret,
    pub target: Target,
    pub mode: PassMode,
    pub pass_out: Secret,
    pub pass_out2: Secret,
    /// OpenPGP user ID (#107), e.g. `Name <mail@example.com>`.
    pub user_id: String,
    pub error: Option<String>,
    pub busy: bool,
    pub done: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MyKeysDraft {
    pub selected: Option<PathBuf>,
    pub filter: String,
    pub generate: Option<GenForm>,
    pub convert: Option<ConvertForm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MkMsg {
    Load,
    Loaded {
        dir: PathBuf,
        user_host: String,
        result: Result<Vec<LocalKey>, String>,
    },
    Select(PathBuf),
    CopyPublic(PathBuf),
    OpenFolder(PathBuf),
    /// Add / revoke with this key's public line filled in.
    AddToHosts(PathBuf),
    AskGenerate,
    /// The type changed: the default file name follows it.
    GenKind(GenType),
    CancelGenerate,
    Generate,
    Generated(Result<PathBuf, String>),
    AskConvert(PathBuf),
    /// Import a key file in another format (native open dialog).
    PickImport,
    Picked(Option<LocalKey>),
    CancelConvert,
    Convert,
    Converted(Result<String, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MkEffect {
    /// List `~/.ssh` and these IdentityFile paths → [`MkMsg::Loaded`].
    Scan {
        extra: Vec<String>,
    },
    /// Run ssh-keygen; encrypt with `pass` → [`MkMsg::Generated`].
    Generate {
        kind: GenType,
        path: PathBuf,
        comment: String,
        pass: Secret,
    },
    /// Read `source` (with `pass_in`), write `target` (protected with
    /// `pass_out`) where the user saves it → [`MkMsg::Converted`].
    Convert {
        source: PathBuf,
        pass_in: Secret,
        target: Target,
        pass_out: Secret,
        /// Suggested place: `~/.ssh/<name>` for an import.
        suggest: PathBuf,
        /// OpenPGP user ID (#107).
        user_id: String,
    },
    /// Native open dialog, then inspect → [`MkMsg::Picked`].
    PickImport {
        dir: PathBuf,
    },
    CopyText(String),
    OpenFolder(PathBuf),
}

fn key<'a>(m: &'a MyKeys, path: &PathBuf) -> Option<&'a LocalKey> {
    m.keys
        .as_ref()?
        .as_ref()
        .ok()?
        .iter()
        .find(|k| &k.path == path)
}

fn default_path(m: &MyKeys, kind: GenType) -> String {
    telepad_core::local_keys::next_free(&m.dir, kind.file_stem())
        .to_string_lossy()
        .to_string()
}

/// Apply one message. `identity_files`: the inventory's IdentityFile paths
/// (they are listed too). Returns the effects and, for Add to hosts, the
/// public line to put into the Add / revoke form.
pub fn update(
    m: &mut MyKeys,
    d: &mut MyKeysDraft,
    identity_files: Vec<String>,
    msg: MkMsg,
) -> (Vec<MkEffect>, Option<String>) {
    let mut fx = Vec::new();
    let mut add = None;
    match msg {
        MkMsg::Load => {
            m.loading = true;
            fx.push(MkEffect::Scan {
                extra: identity_files,
            });
        }
        MkMsg::Loaded {
            dir,
            user_host,
            result,
        } => {
            m.loading = false;
            m.dir = dir;
            m.user_host = user_host;
            if let (Some(sel), Ok(list)) = (&d.selected, &result)
                && !list.iter().any(|k| &k.path == sel)
            {
                d.selected = None;
            }
            m.keys = Some(result);
        }
        MkMsg::Select(p) => d.selected = Some(p),
        MkMsg::CopyPublic(p) => {
            if let Some(line) = key(m, &p).and_then(|k| k.public_line.clone()) {
                fx.push(MkEffect::CopyText(line));
            }
        }
        MkMsg::OpenFolder(p) => fx.push(MkEffect::OpenFolder(p)),
        MkMsg::AddToHosts(p) => add = key(m, &p).and_then(|k| k.public_line.clone()),
        MkMsg::AskGenerate => {
            d.generate = Some(GenForm {
                kind: GenType::Ed25519,
                rsa_bits: 4096,
                path: default_path(m, GenType::Ed25519),
                comment: m.user_host.clone(),
                pass: Secret::default(),
                pass2: Secret::default(),
                error: None,
                busy: false,
            });
        }
        MkMsg::GenKind(kind) => {
            if let Some(g) = &mut d.generate {
                let was_default = g.path == default_path(m, g.kind);
                g.kind = kind;
                if was_default {
                    g.path = default_path(m, kind);
                }
            }
        }
        MkMsg::CancelGenerate => d.generate = None,
        MkMsg::Generate => {
            let Some(g) = &mut d.generate else {
                return (fx, add);
            };
            if g.busy {
                return (fx, add);
            }
            let path = telepad_core::expand_home(g.path.trim());
            g.error = if g.pass != g.pass2 {
                Some("The two passphrases differ".into())
            } else if g.comment.contains(['\n', '\r']) {
                Some("The comment must be one line".into())
            } else {
                telepad_core::local_keys::check_new_path(&path)
            };
            if g.error.is_none() {
                g.busy = true;
                let kind = match g.kind {
                    GenType::Rsa(_) => GenType::Rsa(g.rsa_bits),
                    k => k,
                };
                fx.push(MkEffect::Generate {
                    kind,
                    path,
                    comment: g.comment.trim().to_string(),
                    pass: g.pass.clone(),
                });
            }
        }
        MkMsg::Generated(result) => match result {
            Ok(p) => {
                d.generate = None;
                d.selected = Some(p);
                m.loading = true;
                fx.push(MkEffect::Scan {
                    extra: identity_files,
                });
            }
            Err(e) => {
                if let Some(g) = &mut d.generate {
                    g.busy = false;
                    g.error = Some(e);
                }
            }
        },
        MkMsg::AskConvert(p) => {
            if let Some(k) = key(m, &p) {
                d.convert = Some(form(k, false));
            }
        }
        MkMsg::PickImport => fx.push(MkEffect::PickImport { dir: m.dir.clone() }),
        MkMsg::Picked(None) => {}
        MkMsg::Picked(Some(k)) => d.convert = Some(form(&k, true)),
        MkMsg::CancelConvert => d.convert = None,
        MkMsg::Convert => {
            let Some(c) = &mut d.convert else {
                return (fx, add);
            };
            if c.busy {
                return (fx, add);
            }
            c.done = None;
            let pass_out = match c.mode {
                PassMode::Keep => c.pass_in.clone(),
                PassMode::Remove => Secret::default(),
                PassMode::New => c.pass_out.clone(),
            };
            c.error = if c.encrypted && c.pass_in.is_empty() {
                Some("Type the key's passphrase".into())
            } else if c.mode == PassMode::New && c.pass_out != c.pass_out2 {
                Some("The two new passphrases differ".into())
            } else if c.target.needs_passphrase() && pass_out.is_empty() && !c.target.is_public() {
                Some("PKCS#12 needs a password: choose New passphrase".into())
            } else {
                None
            };
            if c.error.is_none() {
                c.busy = true;
                let stem = c
                    .source
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "key".into());
                let ext = c.target.extension();
                let name = if ext.is_empty() {
                    format!("{stem}_openssh")
                } else {
                    format!("{stem}.{ext}")
                };
                fx.push(MkEffect::Convert {
                    source: c.source.clone(),
                    pass_in: c.pass_in.clone(),
                    target: c.target,
                    pass_out: if c.target.can_encrypt() {
                        pass_out
                    } else {
                        Secret::default()
                    },
                    suggest: m.dir.join(name),
                    user_id: c.user_id.trim().to_string(),
                });
            }
        }
        MkMsg::Converted(result) => {
            if let Some(c) = &mut d.convert {
                c.busy = false;
                match result {
                    Ok(done) => {
                        c.done = Some(done);
                        c.error = None;
                        m.loading = true;
                        fx.push(MkEffect::Scan {
                            extra: identity_files,
                        });
                    }
                    Err(e) => c.error = Some(e),
                }
            }
        }
    }
    (fx, add)
}

fn form(k: &LocalKey, import: bool) -> ConvertForm {
    ConvertForm {
        source: k.path.clone(),
        format: k.format,
        encrypted: k.encrypted,
        import,
        algorithm: k.kind.clone(),
        pass_in: Secret::default(),
        target: if import || k.format != Some(Format::OpenSsh) {
            Target::OpenSsh
        } else {
            Target::Ppk3
        },
        mode: PassMode::Keep,
        pass_out: Secret::default(),
        pass_out2: Secret::default(),
        user_id: k.comment.clone(),
        error: None,
        busy: false,
        done: None,
    }
}

/// Hosts whose IdentityFile is this key (by path).
pub fn used_by(m: &crate::Model, k: &LocalKey) -> Vec<String> {
    m.hosts
        .iter()
        .filter(|h| {
            h.entry.identity_file.as_deref().is_some_and(|f| {
                let p = telepad_core::expand_home(f.trim());
                p == k.path
                    || (p.canonicalize().ok().is_some()
                        && p.canonicalize().ok() == k.path.canonicalize().ok())
            })
        })
        .map(|h| h.entry.alias.clone())
        .collect()
}

/// Hosts whose `authorized_keys` hold this key (from the By key lists).
pub fn authorized_on(m: &crate::Model, k: &LocalKey) -> Vec<String> {
    let Some(fp) = &k.fingerprint else {
        return Vec::new();
    };
    crate::state::key_holders(m)
        .into_iter()
        .find(|h| &h.fingerprint == fp)
        .map(|h| h.hosts.into_iter().map(|(a, _)| a).collect())
        .unwrap_or_default()
}
