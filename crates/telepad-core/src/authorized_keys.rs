//! List the keys in the login user's `~/.ssh/authorized_keys` (read-only, #78)
//! and add or revoke one SSH public key there on many hosts at once. This is the one in-app action that changes a remote
//! host (owner decision on #49): the app runs it only after the user typed the
//! confirmation word, never from a refresh.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use sha2::{Digest, Sha256};

use crate::collector::{SshSettings, base64, explain_ssh_error, run_script};
use crate::config::{Access, HostEntry};

/// Key types OpenSSH accepts in `authorized_keys`.
pub const KEY_TYPES: &[&str] = &[
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// One public key line: type, base64 key data, optional comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    pub kind: String,
    pub blob: String,
    pub comment: String,
}

impl PublicKey {
    /// Read one pasted `type base64 [comment]` line and check the key data.
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("Paste a public key line (the contents of a .pub file)".into());
        }
        if t.contains("PRIVATE KEY") {
            return Err(
                "This is a PRIVATE key: never paste or share it. Paste the matching .pub line"
                    .into(),
            );
        }
        if t.lines().count() > 1 {
            return Err("Paste exactly one key line".into());
        }
        let mut words = t.split_whitespace();
        let kind = words.next().unwrap_or_default();
        if !KEY_TYPES.contains(&kind) {
            return Err(if kind.contains('=') || kind.contains(',') {
                "authorized_keys options (command=, from= …) are not supported: paste the bare key"
                    .into()
            } else {
                format!(
                    "Unknown key type \"{kind}\": expected ssh-ed25519, ssh-rsa, ecdsa-sha2-… or sk-…"
                )
            });
        }
        let blob = words
            .next()
            .ok_or("The key data after the type is missing")?;
        let data = decode_base64(blob).ok_or("The key data is not valid base64")?;
        if embedded_type(&data) != Some(kind) {
            return Err(format!(
                "The key data does not belong to a {kind} key (copied incompletely?)"
            ));
        }
        let comment = words.collect::<Vec<_>>().join(" ");
        if comment.chars().any(char::is_control) {
            return Err("The comment contains control characters".into());
        }
        Ok(Self {
            kind: kind.into(),
            blob: blob.into(),
            comment,
        })
    }

    /// `SHA256:…` as `ssh-keygen -lf` prints it.
    pub fn fingerprint(&self) -> String {
        let data = decode_base64(&self.blob).unwrap_or_default();
        let hash = base64(&Sha256::digest(&data));
        format!("SHA256:{}", hash.trim_end_matches('='))
    }

    /// The line written to `authorized_keys`.
    pub fn line(&self) -> String {
        if self.comment.is_empty() {
            format!("{} {}", self.kind, self.blob)
        } else {
            format!("{} {} {}", self.kind, self.blob, self.comment)
        }
    }

    /// Same key (type and data), whatever the comment.
    pub fn same_key(&self, other: &Self) -> bool {
        self.kind == other.kind && self.blob == other.blob
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Add,
    Revoke,
}

impl KeyAction {
    /// What the user types to confirm.
    pub fn word(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Revoke => "revoke",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Add => "Add key",
            Self::Revoke => "Revoke key",
        }
    }
}

/// What happened on one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOutcome {
    Added,
    AlreadyPresent,
    /// Number of lines removed.
    Removed(usize),
    NotPresent,
    /// Not tried on this host, with the reason.
    Refused(String),
    Failed(String),
}

impl KeyOutcome {
    /// The host ends up as asked (key there after Add, gone after Revoke).
    pub fn ok(&self) -> bool {
        matches!(
            self,
            Self::Added | Self::AlreadyPresent | Self::Removed(_) | Self::NotPresent
        )
    }

    pub fn text(&self) -> String {
        match self {
            Self::Added => "added".into(),
            Self::AlreadyPresent => "already present (not changed)".into(),
            Self::Removed(1) => "removed".into(),
            Self::Removed(n) => format!("removed ({n} lines)"),
            Self::NotPresent => "not present (not changed)".into(),
            Self::Refused(why) => format!("refused: {why}"),
            Self::Failed(e) => format!("failed: {e}"),
        }
    }
}

/// Remote script: adds or removes `key` in `$HOME/.ssh/authorized_keys` and
/// prints one `TELEPAD-KEY <result>` line. The file is rebuilt in a temporary
/// file next to it (same mode) and moved over it, so a failure leaves it intact.
/// Type and data are validated (safe in single quotes); the whole line, comment
/// included, is read from a quoted here-document, so nothing in it is expanded.
pub fn script(action: KeyAction, key: &PublicKey) -> String {
    format!(
        r#"LC_ALL=C
export LC_ALL
umask 077
T='{kind}'
B='{blob}'
KEY=$(cat <<'TELEPAD_KEY_END'
{line}
TELEPAD_KEY_END
)
say() {{ echo "TELEPAD-KEY $*"; }}
[ -n "${{HOME:-}}" ] || {{ say "error HOME is not set"; exit 0; }}
D="$HOME/.ssh"
F="$D/authorized_keys"
TMP="$D/.authorized_keys.telepad.$$"
trap 'rm -f "$TMP"' EXIT
if [ -L "$F" ]; then say "error $F is a symbolic link, not changed"; exit 0; fi
has() {{
  [ -f "$F" ] && awk -v t="$T" -v b="$B" '/^[ \t]*#/ {{ next }} {{ for (i = 1; i < NF; i++) if ($i == t && $(i + 1) == b) f = 1 }} END {{ exit f ? 0 : 1 }}' "$F"
}}
if [ {action} = add ]; then
  if has; then say present; exit 0; fi
  if [ ! -d "$D" ]; then
    mkdir -p "$D" && chmod 700 "$D" || {{ say "error cannot create $D"; exit 0; }}
  fi
  if [ -f "$F" ]; then
    cp -p "$F" "$TMP" || {{ say "error cannot copy $F"; exit 0; }}
    if [ -s "$TMP" ] && [ -n "$(tail -c 1 "$TMP")" ]; then echo >> "$TMP"; fi
  else
    : > "$TMP" && chmod 600 "$TMP" || {{ say "error cannot write in $D"; exit 0; }}
  fi
  printf '%s\n' "$KEY" >> "$TMP" && mv -f "$TMP" "$F" || {{ say "error cannot write $F"; exit 0; }}
  if has; then say added; else say "error the key is not in $F after writing"; fi
  exit 0
fi
if ! has; then say absent; exit 0; fi
N=$(awk -v t="$T" -v b="$B" '/^[ \t]*#/ {{ next }} {{ for (i = 1; i < NF; i++) if ($i == t && $(i + 1) == b) {{ n++; next }} }} END {{ print n + 0 }}' "$F")
cp -p "$F" "$TMP" || {{ say "error cannot copy $F"; exit 0; }}
awk -v t="$T" -v b="$B" '/^[ \t]*#/ {{ print; next }} {{ for (i = 1; i < NF; i++) if ($i == t && $(i + 1) == b) next; print }}' "$F" > "$TMP" || {{ say "error cannot write in $D"; exit 0; }}
mv -f "$TMP" "$F" || {{ say "error cannot replace $F"; exit 0; }}
if has; then say "error the key is still in $F"; else say "removed $N"; fi
"#,
        kind = key.kind,
        blob = key.blob,
        line = key.line(),
        action = action.word(),
    )
}

/// Read the script's `TELEPAD-KEY` line.
pub fn parse_result(out: &str) -> Option<KeyOutcome> {
    let r = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("TELEPAD-KEY "))?;
    Some(match r.split_once(' ').unwrap_or((r, "")) {
        ("added", _) => KeyOutcome::Added,
        ("present", _) => KeyOutcome::AlreadyPresent,
        ("absent", _) => KeyOutcome::NotPresent,
        ("removed", n) => KeyOutcome::Removed(n.trim().parse().unwrap_or(1)),
        ("error", e) => KeyOutcome::Failed(e.to_string()),
        _ => KeyOutcome::Failed(format!("unexpected reply: {r}")),
    })
}

/// Public keys this computer may log in to `host` with: the `.pub` next to its
/// IdentityFile, the fallback key, and the default `~/.ssh/id_*` keys.
pub fn login_keys(settings: &SshSettings, host: &HostEntry) -> Vec<(String, PublicKey)> {
    let mut paths = Vec::new();
    if let Some(id) = &host.identity_file {
        paths.push(crate::expand_home(id));
    }
    if let Some(id) = &settings.fallback_identity {
        paths.push(id.clone());
    }
    for name in [
        "id_ed25519",
        "id_ecdsa",
        "id_rsa",
        "id_ed25519_sk",
        "id_ecdsa_sk",
    ] {
        paths.push(crate::expand_home(&format!("~/.ssh/{name}")));
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let mut pub_path = p.into_os_string();
            pub_path.push(".pub");
            let pub_path = std::path::PathBuf::from(pub_path);
            let text = std::fs::read_to_string(&pub_path).ok()?;
            let key = PublicKey::parse(text.lines().next()?).ok()?;
            Some((pub_path.display().to_string(), key))
        })
        .collect()
}

/// Run `action` for `key` on one host (blocks up to the ssh timeout).
pub fn apply(
    settings: &SshSettings,
    host: &HostEntry,
    action: KeyAction,
    key: &PublicKey,
) -> KeyOutcome {
    if host.access == Access::Rdp {
        return KeyOutcome::Refused("RDP-only host, no SSH".into());
    }
    if action == KeyAction::Revoke
        && let Some((path, _)) = login_keys(settings, host)
            .into_iter()
            .find(|(_, k)| k.same_key(key))
    {
        return KeyOutcome::Refused(format!("Telepad logs in with this key ({path})"));
    }
    match run_script(settings, host, &script(action, key)) {
        Ok((out, err, code)) => {
            parse_result(&out).unwrap_or_else(|| KeyOutcome::Failed(explain_ssh_error(&err, code)))
        }
        Err(e) => KeyOutcome::Failed(e),
    }
}

/// Run `action` on every host, at most `workers` at a time; `done` gets each
/// host's alias and result as soon as it is known.
pub fn apply_all(
    settings: &SshSettings,
    hosts: &[HostEntry],
    action: KeyAction,
    key: &PublicKey,
    workers: usize,
    done: impl Fn(String, KeyOutcome) + Sync,
) {
    let next = AtomicUsize::new(0);
    thread::scope(|s| {
        for _ in 0..workers.clamp(1, hosts.len().max(1)) {
            s.spawn(|| {
                while let Some(h) = hosts.get(next.fetch_add(1, Ordering::Relaxed)) {
                    done(h.alias.clone(), apply(settings, h, action, key));
                }
            });
        }
    });
}

/// One key line of a host's `authorized_keys`, as read by [`list`] (#78).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthKey {
    /// 1-based line number in the file.
    pub line_no: usize,
    /// Options before the key (`from="…",no-pty`), empty when none.
    pub options: String,
    /// Type, data and comment as written (the type may be one Telepad does
    /// not add, e.g. `ssh-dss` or a certificate type).
    pub key: PublicKey,
    /// Type known to [`PublicKey::parse`] and data that belongs to it: only
    /// those can be added elsewhere or revoked from Telepad.
    pub supported: bool,
}

/// Every key line of one host's `authorized_keys`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostKeys {
    pub keys: Vec<AuthKey>,
    /// The file does not exist (no key is authorized).
    pub missing: bool,
    /// Lines that are neither blank, `#` comments nor keys (shown as a count).
    pub unreadable: usize,
    /// Fingerprints of the keys Telepad logs in to this host with (from
    /// [`login_keys`]): never revoked from Telepad.
    pub login: Vec<String>,
}

/// Read-only script: prints the login user's `~/.ssh/authorized_keys`
/// between markers (nothing is changed on the host).
pub fn list_script() -> &'static str {
    r#"LC_ALL=C
export LC_ALL
F="${HOME:-}/.ssh/authorized_keys"
if [ -z "${HOME:-}" ]; then echo "TELEPAD-AK error HOME is not set"; exit 0; fi
if [ ! -e "$F" ]; then echo "TELEPAD-AK none"; exit 0; fi
if [ ! -r "$F" ]; then echo "TELEPAD-AK error $F is not readable"; exit 0; fi
echo "TELEPAD-AK begin"
cat "$F"
echo
echo "TELEPAD-AK end"
"#
}

/// Split one `authorized_keys` line into options and the key words; `None`
/// for blank and `#` lines. Options may hold quoted text with spaces.
fn split_line(line: &str) -> Option<(String, Vec<&str>)> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let first = t.split_whitespace().next()?;
    if is_key_type(first) {
        return Some((String::new(), t.split_whitespace().collect()));
    }
    // Options: up to the first space outside double quotes.
    let mut quoted = false;
    let mut end = t.len();
    for (i, c) in t.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let rest = t[end..].trim_start();
    Some((t[..end].to_string(), rest.split_whitespace().collect()))
}

/// Looks like a key type: one of [`KEY_TYPES`], or `ssh-…`, `ecdsa-…`, `sk-…`
/// (older or certificate types are listed, not managed).
fn is_key_type(w: &str) -> bool {
    KEY_TYPES.contains(&w) || ["ssh-", "ecdsa-", "sk-"].iter().any(|p| w.starts_with(p))
}

/// Read an `authorized_keys` file's text.
pub fn parse_authorized_keys(text: &str) -> HostKeys {
    let mut out = HostKeys::default();
    for (n, line) in text.lines().enumerate() {
        let Some((options, words)) = split_line(line) else {
            continue;
        };
        let (Some(kind), Some(blob)) = (words.first(), words.get(1)) else {
            out.unreadable += 1;
            continue;
        };
        if !is_key_type(kind) {
            out.unreadable += 1;
            continue;
        }
        let comment = words[2..].join(" ");
        let bare = format!("{kind} {blob} {comment}");
        let supported = PublicKey::parse(bare.trim()).is_ok();
        out.keys.push(AuthKey {
            line_no: n + 1,
            options,
            key: PublicKey {
                kind: (*kind).to_string(),
                blob: (*blob).to_string(),
                comment,
            },
            supported,
        });
    }
    out
}

/// Read the list script's output.
pub fn parse_list(out: &str) -> Result<HostKeys, String> {
    let mut lines = out.lines();
    loop {
        let Some(l) = lines.next() else {
            return Err("no answer from the host".into());
        };
        match l.trim().strip_prefix("TELEPAD-AK ") {
            Some("none") => {
                return Ok(HostKeys {
                    missing: true,
                    ..HostKeys::default()
                });
            }
            Some("begin") => break,
            Some(r) => {
                return Err(r.strip_prefix("error ").unwrap_or(r).to_string());
            }
            None => {}
        }
    }
    let body: Vec<&str> = lines.take_while(|l| l.trim() != "TELEPAD-AK end").collect();
    Ok(parse_authorized_keys(&body.join("\n")))
}

/// List the keys authorized on one host (read-only; blocks up to the ssh timeout).
pub fn list(settings: &SshSettings, host: &HostEntry) -> Result<HostKeys, String> {
    if host.access == Access::Rdp {
        return Err("RDP-only host, no SSH".into());
    }
    let (out, err, code) = run_script(settings, host, list_script())?;
    let mut keys = match parse_list(&out) {
        Ok(k) => k,
        Err(e) if out.contains("TELEPAD-AK") => return Err(e),
        Err(_) => return Err(explain_ssh_error(&err, code)),
    };
    keys.login = login_keys(settings, host)
        .iter()
        .map(|(_, k)| k.fingerprint())
        .collect();
    Ok(keys)
}

/// [`list`] on every host, at most `workers` at a time.
pub fn list_all(
    settings: &SshSettings,
    hosts: &[HostEntry],
    workers: usize,
    done: impl Fn(String, Result<HostKeys, String>) + Sync,
) {
    let next = AtomicUsize::new(0);
    thread::scope(|s| {
        for _ in 0..workers.clamp(1, hosts.len().max(1)) {
            s.spawn(|| {
                while let Some(h) = hosts.get(next.fetch_add(1, Ordering::Relaxed)) {
                    done(h.alias.clone(), list(settings, h));
                }
            });
        }
    });
}

/// The key type string at the start of the key data (SSH wire format).
fn embedded_type(data: &[u8]) -> Option<&str> {
    let len = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
    let kind = data.get(4..4 + len)?;
    // A real key has key material after its type.
    if data.len() <= 4 + len {
        return None;
    }
    std::str::from_utf8(kind).ok()
}

/// Strict standard base64 (with padding, no whitespace).
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(4) {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes = s.as_bytes();
    let pad = bytes.iter().rev().take_while(|&&c| c == b'=').count();
    if pad > 2 || bytes[..bytes.len() - pad].contains(&b'=') {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        let mut real = 0;
        for &c in chunk {
            n <<= 6;
            if c != b'=' {
                n |= val(c)?;
                real += 1;
            }
        }
        let b = n.to_be_bytes();
        out.extend_from_slice(&b[1..real]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// A made-up ed25519 public key (32 bytes of `fill`), never a real one.
    fn fake_key(fill: u8, comment: &str) -> String {
        let mut data = Vec::new();
        data.extend_from_slice(&11u32.to_be_bytes());
        data.extend_from_slice(b"ssh-ed25519");
        data.extend_from_slice(&32u32.to_be_bytes());
        data.extend_from_slice(&[fill; 32]);
        format!("ssh-ed25519 {} {comment}", base64(&data))
            .trim_end()
            .to_string()
    }

    #[test]
    fn lists_keys_with_options_comments_and_odd_lines() {
        let text = format!(
            "# managed by hand\n\n{}\nfrom=\"192.0.2.0/24,198.51.100.7\",command=\"echo a b\" {}\n\
             ssh-dss AAAAB3NzaC1kc3M= old-key\nnot a key line\n{}\n",
            fake_key(1, "alice@example.com"),
            fake_key(2, "backup job"),
            fake_key(1, "alice laptop"),
        );
        let h = parse_authorized_keys(&text);
        assert_eq!(h.keys.len(), 4);
        assert_eq!(h.unreadable, 1);
        let k0 = &h.keys[0];
        assert_eq!((k0.line_no, k0.options.as_str()), (3, ""));
        assert_eq!(k0.key.comment, "alice@example.com");
        assert!(k0.supported);
        let k1 = &h.keys[1];
        assert_eq!(
            k1.options,
            "from=\"192.0.2.0/24,198.51.100.7\",command=\"echo a b\""
        );
        assert_eq!(k1.key.comment, "backup job");
        assert!(k1.supported);
        // An old type is listed but not managed.
        assert_eq!(h.keys[2].key.kind, "ssh-dss");
        assert!(!h.keys[2].supported);
        // Same key twice (other comment): same fingerprint.
        assert_eq!(h.keys[3].key.fingerprint(), k0.key.fingerprint());
    }

    #[test]
    fn reads_the_list_script_output() {
        let out = format!(
            "motd noise\nTELEPAD-AK begin\n{}\n\nTELEPAD-AK end\n",
            fake_key(3, "ops")
        );
        let h = parse_list(&out).unwrap();
        assert_eq!(h.keys.len(), 1);
        assert!(!h.missing);
        assert!(parse_list("TELEPAD-AK none\n").unwrap().missing);
        assert_eq!(
            parse_list("TELEPAD-AK error HOME is not set\n").unwrap_err(),
            "HOME is not set"
        );
        assert!(parse_list("").is_err());
        // The script only reads.
        let sc = list_script();
        for w in ["mv ", "rm ", "cp ", ">>", "chmod", "mkdir", "awk"] {
            assert!(!sc.contains(w), "list script must not contain {w}");
        }
    }

    #[test]
    fn parses_and_fingerprints_a_key() {
        let k = PublicKey::parse(&format!("  {}  \n", fake_key(7, "alice@example.com"))).unwrap();
        assert_eq!(k.kind, "ssh-ed25519");
        assert_eq!(k.comment, "alice@example.com");
        assert_eq!(k.line(), fake_key(7, "alice@example.com"));
        // ssh-keygen -lf on the same made-up line.
        assert_eq!(
            k.fingerprint(),
            "SHA256:gNSIRW+2Iyiuvsdp/bgjy38bvWHw6wQm3tuoXrl3WjQ"
        );
        let no_comment = PublicKey::parse(&fake_key(7, "")).unwrap();
        assert!(no_comment.same_key(&k));
        assert_eq!(no_comment.line(), fake_key(7, ""));
    }

    #[test]
    fn rejects_what_is_not_one_public_key() {
        let err = |t: &str| PublicKey::parse(t).unwrap_err();
        assert!(err("").contains("Paste"));
        assert!(err("-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbg==\n").contains("PRIVATE"));
        assert!(err(&format!("{}\n{}", fake_key(1, ""), fake_key(2, ""))).contains("one key"));
        assert!(err("ssh-dss AAAAB3NzaC1kc3M= x").contains("Unknown key type"));
        assert!(err(&format!("from=\"192.0.2.1\" {}", fake_key(1, ""))).contains("options"));
        assert!(err("ssh-ed25519").contains("missing"));
        assert!(err("ssh-ed25519 not*base64 x").contains("base64"));
        // ssh-rsa data under an ssh-ed25519 label.
        let rsa = fake_key(1, "").replace("ssh-ed25519 ", "ssh-rsa ");
        assert!(err(&rsa).contains("does not belong"));
        // Cut short: type only, no key material.
        let mut short = Vec::new();
        short.extend_from_slice(&11u32.to_be_bytes());
        short.extend_from_slice(b"ssh-ed25519");
        assert!(err(&format!("ssh-ed25519 {}", base64(&short))).contains("does not belong"));
    }

    #[test]
    fn decodes_base64_strictly() {
        assert_eq!(decode_base64("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(decode_base64("Zm8=").unwrap(), b"fo");
        assert_eq!(decode_base64("Zg==").unwrap(), b"f");
        assert!(decode_base64("Zg=").is_none());
        assert!(decode_base64("Z=g=").is_none());
        assert!(decode_base64("Zm9v YmFy").is_none());
    }

    #[test]
    fn reads_the_script_result() {
        assert_eq!(
            parse_result("x\nTELEPAD-KEY added\n"),
            Some(KeyOutcome::Added)
        );
        assert_eq!(
            parse_result("TELEPAD-KEY present"),
            Some(KeyOutcome::AlreadyPresent)
        );
        assert_eq!(
            parse_result("TELEPAD-KEY removed 2"),
            Some(KeyOutcome::Removed(2))
        );
        assert_eq!(
            parse_result("TELEPAD-KEY absent"),
            Some(KeyOutcome::NotPresent)
        );
        assert_eq!(
            parse_result("TELEPAD-KEY error cannot write /home/x/.ssh/authorized_keys"),
            Some(KeyOutcome::Failed(
                "cannot write /home/x/.ssh/authorized_keys".into()
            ))
        );
        assert_eq!(parse_result("Permission denied"), None);
        assert!(KeyOutcome::NotPresent.ok() && !KeyOutcome::Refused("x".into()).ok());
    }

    #[test]
    fn rdp_hosts_and_the_login_key_are_refused() {
        let s = SshSettings::new("missing-config".into());
        let key = PublicKey::parse(&fake_key(3, "")).unwrap();
        let mut rdp = HostEntry::new("win01");
        rdp.access = Access::Rdp;
        assert!(matches!(
            apply(&s, &rdp, KeyAction::Add, &key),
            KeyOutcome::Refused(_)
        ));

        let dir = std::env::temp_dir().join(format!("telepad-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = dir.join("id_test");
        std::fs::write(id.with_extension("pub"), fake_key(3, "telepad@example.com")).unwrap();
        let mut host = HostEntry::new("web01");
        host.identity_file = Some(id.display().to_string());
        let found = login_keys(&s, &host);
        assert!(found.iter().any(|(_, k)| k.same_key(&key)));
        // Refused before any ssh is started.
        assert!(matches!(
            apply(&s, &host, KeyAction::Revoke, &key),
            KeyOutcome::Refused(why) if why.contains("logs in with this key")
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Run the remote script with the local sh against a temporary HOME
    /// (skipped where there is no sh). Returns the script's result.
    fn run_local(home: &std::path::Path, action: KeyAction, key: &PublicKey) -> Option<KeyOutcome> {
        let mut child = Command::new("sh")
            .arg("-s")
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        child
            .stdin
            .take()?
            .write_all(script(action, key).as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        Some(parse_result(&text).unwrap_or_else(|| {
            panic!("no result: {text} {}", String::from_utf8_lossy(&out.stderr))
        }))
    }

    #[test]
    fn script_adds_and_revokes_without_touching_other_lines() {
        let home = std::env::temp_dir().join(format!("telepad-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let key = PublicKey::parse(&fake_key(9, "bob@example.com")).unwrap();
        let other = fake_key(5, "carol@example.com");
        let file = home.join(".ssh").join("authorized_keys");

        // No ~/.ssh yet: created with the key.
        let Some(first) = run_local(&home, KeyAction::Add, &key) else {
            return; // no local sh
        };
        assert_eq!(first, KeyOutcome::Added);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("{}\n", key.line())
        );

        // Existing file without a final newline; key already there with options.
        std::fs::write(
            &file,
            format!(
                "# team keys\n{other}\nfrom=\"192.0.2.0/24\" {} old comment",
                key.kind.clone() + " " + &key.blob
            ),
        )
        .unwrap();
        assert_eq!(
            run_local(&home, KeyAction::Add, &key),
            Some(KeyOutcome::AlreadyPresent)
        );
        assert_eq!(
            run_local(&home, KeyAction::Revoke, &key),
            Some(KeyOutcome::Removed(1))
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("# team keys\n{other}\n")
        );
        assert_eq!(
            run_local(&home, KeyAction::Revoke, &key),
            Some(KeyOutcome::NotPresent)
        );

        // Appended after a last line without newline; nothing left behind.
        std::fs::write(&file, &other).unwrap();
        assert_eq!(
            run_local(&home, KeyAction::Add, &key),
            Some(KeyOutcome::Added)
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("{other}\n{}\n", key.line())
        );
        let leftovers = std::fs::read_dir(home.join(".ssh")).unwrap().count();
        assert_eq!(leftovers, 1, "only authorized_keys");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
