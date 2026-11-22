//! My keys (#101): the key pairs on this computer — list and inspect them,
//! generate a pair with `ssh-keygen`, and convert between OpenSSH, PuTTY PPK
//! (v2 / v3), PEM (PKCS#1 / SEC1), PKCS#8 (plain / encrypted), PKCS#12, JWK
//! and DER. Everything is local: nothing here runs on a monitored host.
//!
//! Private key material only lives in memory while it is converted
//! (`Zeroizing` where the crates offer it) and is written only where the user
//! saved it, with owner-only permissions ([`crate::platform::restrict_to_owner`]).

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL};
use rand_core::{OsRng, RngCore};
use ssh_key::private::{
    EcdsaKeypair, Ed25519Keypair, Ed25519PrivateKey, KeypairData, RsaKeypair, RsaPrivateKey,
};
use ssh_key::public::KeyData;
use ssh_key::{Algorithm, HashAlg, LineEnding, Mpint, PrivateKey, PublicKey};

/// Files larger than this are not keys.
const MAX_KEY_FILE: u64 = 1 << 20;

/// What a key file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// `-----BEGIN OPENSSH PRIVATE KEY-----` (ssh-keygen's default).
    OpenSsh,
    /// PuTTY `.ppk`, version 2 or 3.
    Ppk2,
    Ppk3,
    /// `-----BEGIN RSA PRIVATE KEY-----`.
    Pkcs1Pem,
    /// `-----BEGIN EC PRIVATE KEY-----`.
    Sec1Pem,
    /// `-----BEGIN PRIVATE KEY-----`.
    Pkcs8Pem,
    /// `-----BEGIN ENCRYPTED PRIVATE KEY-----`.
    Pkcs8EncryptedPem,
    /// Binary PKCS#8.
    Der,
    /// `.p12` / `.pfx`.
    Pkcs12,
    /// JSON Web Key with `"d"`.
    Jwk,
    /// `-----BEGIN PGP PRIVATE KEY BLOCK-----` (recognised, not converted).
    OpenPgp,
    /// Old PEM encrypted with `Proc-Type: 4,ENCRYPTED` (not converted).
    LegacyEncryptedPem,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::OpenSsh => "OpenSSH",
            Format::Ppk2 => "PuTTY PPK v2",
            Format::Ppk3 => "PuTTY PPK v3",
            Format::Pkcs1Pem => "PEM (PKCS#1)",
            Format::Sec1Pem => "PEM (SEC1)",
            Format::Pkcs8Pem => "PKCS#8 PEM",
            Format::Pkcs8EncryptedPem => "PKCS#8 PEM, encrypted",
            Format::Der => "DER (PKCS#8)",
            Format::Pkcs12 => "PKCS#12",
            Format::Jwk => "JWK",
            Format::OpenPgp => "OpenPGP",
            Format::LegacyEncryptedPem => "PEM, legacy encryption",
        }
    }
}

/// Formats a key can be written as (Convert / Export).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    OpenSsh,
    Ppk3,
    Ppk2,
    /// PKCS#1 for RSA, SEC1 for ECDSA.
    Pem,
    Pkcs8,
    Der,
    Pkcs12,
    Jwk,
    /// Public key only: OpenSSH line.
    PublicOpenSsh,
    /// Public key only: SubjectPublicKeyInfo PEM.
    PublicPem,
    /// Public key only: SubjectPublicKeyInfo DER.
    PublicDer,
    /// Public key only: JWK.
    PublicJwk,
}

impl Target {
    pub const ALL: [Target; 12] = [
        Target::OpenSsh,
        Target::Ppk3,
        Target::Ppk2,
        Target::Pem,
        Target::Pkcs8,
        Target::Der,
        Target::Pkcs12,
        Target::Jwk,
        Target::PublicOpenSsh,
        Target::PublicPem,
        Target::PublicDer,
        Target::PublicJwk,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Target::OpenSsh => "OpenSSH (ssh, ssh-keygen, Telepad)",
            Target::Ppk3 => "PuTTY PPK v3 (PuTTY 0.75+, WinSCP 5.20+)",
            Target::Ppk2 => "PuTTY PPK v2 (older PuTTY / WinSCP)",
            Target::Pem => "PEM (PKCS#1 RSA / SEC1 EC)",
            Target::Pkcs8 => "PKCS#8 PEM",
            Target::Der => "DER (binary PKCS#8)",
            Target::Pkcs12 => "PKCS#12 (.p12 / .pfx)",
            Target::Jwk => "JWK (JSON)",
            Target::PublicOpenSsh => "Public key: OpenSSH line",
            Target::PublicPem => "Public key: PEM",
            Target::PublicDer => "Public key: DER",
            Target::PublicJwk => "Public key: JWK",
        }
    }

    /// File name ending for the save dialog.
    pub fn extension(self) -> &'static str {
        match self {
            Target::OpenSsh => "",
            Target::Ppk3 | Target::Ppk2 => "ppk",
            Target::Pem | Target::Pkcs8 | Target::PublicPem => "pem",
            Target::Der | Target::PublicDer => "der",
            Target::Pkcs12 => "p12",
            Target::Jwk | Target::PublicJwk => "jwk",
            Target::PublicOpenSsh => "pub",
        }
    }

    pub fn is_public(self) -> bool {
        matches!(
            self,
            Target::PublicOpenSsh | Target::PublicPem | Target::PublicDer | Target::PublicJwk
        )
    }

    /// The output can carry a passphrase (else it is written in clear).
    pub fn can_encrypt(self) -> bool {
        matches!(
            self,
            Target::OpenSsh | Target::Ppk3 | Target::Ppk2 | Target::Pkcs8 | Target::Pkcs12
        )
    }

    /// PKCS#12 always needs a password.
    pub fn needs_passphrase(self) -> bool {
        self == Target::Pkcs12
    }
}

/// Why `target` cannot hold a key of `alg`, if it cannot.
pub fn unsupported(alg: &Algorithm, target: Target) -> Option<&'static str> {
    let kind = KeyKind::of(alg);
    match (kind, target) {
        (KeyKind::Other, Target::OpenSsh | Target::PublicOpenSsh) => None,
        (KeyKind::Other, _) => Some("Only OpenSSH can hold this key type"),
        (KeyKind::Dsa, Target::OpenSsh | Target::PublicOpenSsh | Target::Ppk2 | Target::Ppk3) => {
            None
        }
        (KeyKind::Dsa, _) => Some("DSA keys are legacy: OpenSSH and PuTTY only"),
        (KeyKind::P521, Target::OpenSsh | Target::PublicOpenSsh | Target::Ppk2 | Target::Ppk3) => {
            None
        }
        (KeyKind::P521, _) => Some("ECDSA P-521 converts to OpenSSH and PuTTY only"),
        (KeyKind::Ed25519, Target::Pem) => Some("Ed25519 has no PKCS#1 / SEC1 form: use PKCS#8"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Ed25519,
    Rsa,
    P256,
    P384,
    P521,
    Dsa,
    Other,
}

impl KeyKind {
    fn of(alg: &Algorithm) -> KeyKind {
        match alg {
            Algorithm::Ed25519 => KeyKind::Ed25519,
            Algorithm::Rsa { .. } => KeyKind::Rsa,
            Algorithm::Ecdsa { curve } => match curve {
                ssh_key::EcdsaCurve::NistP256 => KeyKind::P256,
                ssh_key::EcdsaCurve::NistP384 => KeyKind::P384,
                ssh_key::EcdsaCurve::NistP521 => KeyKind::P521,
            },
            Algorithm::Dsa => KeyKind::Dsa,
            _ => KeyKind::Other,
        }
    }
}

/// One key file found on this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalKey {
    pub path: PathBuf,
    pub format: Option<Format>,
    /// "Ed25519", "RSA 4096", "ECDSA P-256".
    pub kind: String,
    /// `SHA256:…` (= `ssh-keygen -lf`), when the public key is readable.
    pub fingerprint: Option<String>,
    pub comment: String,
    /// Needs a passphrase.
    pub encrypted: bool,
    /// The OpenSSH public line (`ssh-ed25519 AAAA… comment`), when known.
    pub public_line: Option<String>,
    /// `<path>.pub` exists.
    pub has_pub_file: bool,
    /// Others can read the private key (macOS / Linux mode bits).
    pub permission_warning: Option<String>,
    /// Unix seconds.
    pub modified: Option<i64>,
    /// Not readable or not a key: why.
    pub error: Option<String>,
}

/// Names in `~/.ssh` that are never keys.
fn skipped_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".pub")
        || n.starts_with("known_hosts")
        || n.starts_with("authorized_keys")
        || n == "config"
        || n.starts_with("config.")
        || n == "environment"
        || n.ends_with(".bak")
        || n.ends_with(".old")
        || n.ends_with(".telepad-part")
        || n.starts_with('.')
}

/// The private keys in `dir` (not recursive) and in `extra` (IdentityFile
/// paths of the inventory), each once, inspected without a passphrase.
pub fn scan(dir: &Path, extra: &[PathBuf]) -> Vec<LocalKey> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_file() && !skipped_name(&name) {
                paths.push(p);
            }
        }
    }
    paths.sort();
    for p in extra {
        if !paths.iter().any(|x| same_file(x, p)) {
            paths.push(p.clone());
        }
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let k = inspect(&p);
            // A file in ~/.ssh that is not a key at all is left out; an
            // IdentityFile that cannot be read is shown with the reason.
            let listed = extra.iter().any(|x| same_file(x, &p));
            (k.format.is_some() || listed).then_some(k)
        })
        .collect()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Read one file and say what it is (no passphrase asked).
pub fn inspect(path: &Path) -> LocalKey {
    let mut k = LocalKey {
        path: path.to_path_buf(),
        ..Default::default()
    };
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            k.error = Some(e.to_string());
            return k;
        }
    };
    k.modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    k.permission_warning = crate::platform::private_key_permission_warning(path, &meta);
    if meta.len() > MAX_KEY_FILE {
        k.error = Some("too large for a key".into());
        return k;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => zeroize::Zeroizing::new(b),
        Err(e) => {
            k.error = Some(e.to_string());
            return k;
        }
    };
    let pub_path = pub_path(path);
    k.has_pub_file = pub_path.is_file();
    let Some(format) = detect(&bytes, path) else {
        return k;
    };
    k.format = Some(format);
    k.encrypted = matches!(
        format,
        Format::Pkcs8EncryptedPem | Format::Pkcs12 | Format::LegacyEncryptedPem
    );
    let public = match format {
        Format::OpenSsh => PrivateKey::from_openssh(&*bytes).ok().map(|p| {
            k.encrypted = p.is_encrypted();
            p.public_key().clone()
        }),
        Format::Ppk2 | Format::Ppk3 => {
            let text = String::from_utf8_lossy(&bytes);
            ppk_public(&text).ok().map(|(p, enc)| {
                k.encrypted = enc;
                p
            })
        }
        Format::OpenPgp | Format::LegacyEncryptedPem => None,
        _ if k.encrypted => None,
        _ => load(&bytes, path, None)
            .ok()
            .map(|p| p.public_key().clone()),
    };
    let public = public.or_else(|| {
        std::fs::read_to_string(&pub_path)
            .ok()
            .and_then(|t| PublicKey::from_openssh(t.trim()).ok())
    });
    if let Some(p) = public {
        describe(&mut k, &p);
    }
    // An encrypted OpenSSH key keeps its comment inside: take the .pub's.
    if k.comment.is_empty()
        && let Some(p) = std::fs::read_to_string(&pub_path)
            .ok()
            .and_then(|t| PublicKey::from_openssh(t.trim()).ok())
        && Some(p.fingerprint(HashAlg::Sha256).to_string()) == k.fingerprint
    {
        k.comment = p.comment().to_string();
        k.public_line = p.to_openssh().ok();
    }
    if format == Format::OpenPgp {
        k.kind = "OpenPGP key".into();
        k.error = Some("OpenPGP keys are listed only; use gpg to convert them".into());
    }
    if format == Format::LegacyEncryptedPem {
        k.error = Some(
            "Old encrypted PEM: run ssh-keygen -p -f <file> once to convert it to OpenSSH".into(),
        );
    }
    k
}

fn describe(k: &mut LocalKey, p: &PublicKey) {
    k.kind = kind_text(p.key_data());
    k.fingerprint = Some(p.fingerprint(HashAlg::Sha256).to_string());
    if k.comment.is_empty() {
        k.comment = p.comment().to_string();
    }
    k.public_line = p.to_openssh().ok();
}

pub fn pub_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".pub");
    PathBuf::from(s)
}

fn kind_text(k: &KeyData) -> String {
    match k {
        KeyData::Ed25519(_) => "Ed25519".into(),
        KeyData::Rsa(r) => {
            let bits =
                r.n.as_positive_bytes()
                    .map(|b| b.len() * 8)
                    .unwrap_or_default();
            format!("RSA {bits}")
        }
        KeyData::Ecdsa(e) => match e.curve() {
            ssh_key::EcdsaCurve::NistP256 => "ECDSA P-256".into(),
            ssh_key::EcdsaCurve::NistP384 => "ECDSA P-384".into(),
            ssh_key::EcdsaCurve::NistP521 => "ECDSA P-521".into(),
        },
        KeyData::Dsa(_) => "DSA (legacy)".into(),
        other => other.algorithm().to_string(),
    }
}

/// The format of a key file's bytes, if it is a private key Telepad knows.
pub fn detect(bytes: &[u8], path: &Path) -> Option<Format> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let t = head.trim_start();
    if t.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----") {
        return Some(Format::OpenSsh);
    }
    if t.starts_with("PuTTY-User-Key-File-2:") {
        return Some(Format::Ppk2);
    }
    if t.starts_with("PuTTY-User-Key-File-3:") {
        return Some(Format::Ppk3);
    }
    if t.starts_with("-----BEGIN RSA PRIVATE KEY-----")
        || t.starts_with("-----BEGIN DSA PRIVATE KEY-----")
    {
        return Some(if t.contains("Proc-Type: 4,ENCRYPTED") {
            Format::LegacyEncryptedPem
        } else {
            Format::Pkcs1Pem
        });
    }
    if t.starts_with("-----BEGIN EC PRIVATE KEY-----") {
        return Some(if t.contains("Proc-Type: 4,ENCRYPTED") {
            Format::LegacyEncryptedPem
        } else {
            Format::Sec1Pem
        });
    }
    if t.starts_with("-----BEGIN PRIVATE KEY-----") {
        return Some(Format::Pkcs8Pem);
    }
    if t.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----") {
        return Some(Format::Pkcs8EncryptedPem);
    }
    if t.starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----") {
        return Some(Format::OpenPgp);
    }
    if t.starts_with('{') {
        return serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .filter(|v| v.get("kty").is_some() && v.get("d").is_some())
            .map(|_| Format::Jwk);
    }
    if bytes.first() == Some(&0x30) {
        use pkcs8::der::Decode;
        if pkcs8::PrivateKeyInfo::from_der(bytes).is_ok() {
            return Some(Format::Der);
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if ext == "p12" || ext == "pfx" {
            return Some(Format::Pkcs12);
        }
    }
    None
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("this key needs its passphrase")]
    NeedsPassphrase,
    #[error("wrong passphrase (or the file is damaged)")]
    WrongPassphrase,
    #[error("{0}")]
    Unsupported(String),
    #[error("not a valid key: {0}")]
    Invalid(String),
}

fn invalid(e: impl std::fmt::Display) -> KeyError {
    KeyError::Invalid(e.to_string())
}

/// Read a private key in any supported format; `passphrase` for encrypted
/// ones. The result is decrypted (comment = the file name when the format
/// has none).
pub fn load(bytes: &[u8], path: &Path, passphrase: Option<&str>) -> Result<PrivateKey, KeyError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let format = detect(bytes, path).ok_or_else(|| invalid("unknown file format"))?;
    let text = || String::from_utf8_lossy(bytes).to_string();
    let mut key = match format {
        Format::OpenSsh => {
            let k = PrivateKey::from_openssh(bytes).map_err(invalid)?;
            if k.is_encrypted() {
                let p = passphrase
                    .filter(|p| !p.is_empty())
                    .ok_or(KeyError::NeedsPassphrase)?;
                k.decrypt(p).map_err(|_| KeyError::WrongPassphrase)?
            } else {
                k
            }
        }
        Format::Ppk2 | Format::Ppk3 => ppk_read(&text(), passphrase)?,
        Format::Pkcs1Pem => {
            use rsa::pkcs1::DecodeRsaPrivateKey;
            let r = rsa::RsaPrivateKey::from_pkcs1_pem(&text()).map_err(invalid)?;
            from_rsa(&r)?
        }
        Format::Sec1Pem => from_sec1_pem(&text())?,
        Format::Pkcs8Pem => {
            let (_, doc) = pkcs8::SecretDocument::from_pem(&text()).map_err(invalid)?;
            from_pkcs8_der(doc.as_bytes())?
        }
        Format::Pkcs8EncryptedPem => {
            let p = passphrase
                .filter(|p| !p.is_empty())
                .ok_or(KeyError::NeedsPassphrase)?;
            let (_, doc) = pkcs8::Document::from_pem(&text()).map_err(invalid)?;
            let enc = pkcs8::EncryptedPrivateKeyInfo::try_from(doc.as_bytes()).map_err(invalid)?;
            let plain = enc.decrypt(p).map_err(|_| KeyError::WrongPassphrase)?;
            from_pkcs8_der(plain.as_bytes())?
        }
        Format::Der => from_pkcs8_der(bytes)?,
        Format::Pkcs12 => {
            let p = passphrase.ok_or(KeyError::NeedsPassphrase)?;
            let ks = p12_keystore::KeyStore::from_pkcs12(
                bytes,
                p,
                p12_keystore::Pkcs12ImportPolicy::Relaxed,
            )
            .map_err(|_| KeyError::WrongPassphrase)?;
            let (_, chain) = ks
                .private_key_chain()
                .ok_or_else(|| invalid("no private key in this PKCS#12 file"))?;
            from_pkcs8_der(chain.key().as_der())?
        }
        Format::Jwk => from_jwk(&text())?,
        Format::OpenPgp => {
            return Err(KeyError::Unsupported(
                "OpenPGP keys are listed only; use gpg --export-ssh-key to get an OpenSSH key"
                    .into(),
            ));
        }
        Format::LegacyEncryptedPem => {
            return Err(KeyError::Unsupported(
                "Old encrypted PEM: run ssh-keygen -p -f <file> once, then convert".into(),
            ));
        }
    };
    if key.comment().is_empty() {
        key.set_comment(name);
    }
    Ok(key)
}

fn from_rsa(r: &rsa::RsaPrivateKey) -> Result<PrivateKey, KeyError> {
    let kp = RsaKeypair::try_from(r).map_err(invalid)?;
    PrivateKey::new(KeypairData::Rsa(kp), "").map_err(invalid)
}

/// RSA key pair into the `rsa` crate's type (ssh-key 0.6.7's own
/// conversion passes `p` twice, so it is done here).
fn to_rsa(kp: &RsaKeypair) -> Result<rsa::RsaPrivateKey, KeyError> {
    let big = |m: &Mpint| {
        m.as_positive_bytes()
            .map(rsa::BigUint::from_bytes_be)
            .ok_or_else(|| invalid("RSA value is negative"))
    };
    let mut r = rsa::RsaPrivateKey::from_components(
        big(&kp.public.n)?,
        big(&kp.public.e)?,
        big(&kp.private.d)?,
        vec![big(&kp.private.p)?, big(&kp.private.q)?],
    )
    .map_err(invalid)?;
    r.precompute().map_err(invalid)?;
    Ok(r)
}

fn from_p256(s: &p256::SecretKey) -> Result<PrivateKey, KeyError> {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let kp = EcdsaKeypair::NistP256 {
        public: s.public_key().to_encoded_point(false),
        private: s.clone().into(),
    };
    PrivateKey::new(KeypairData::Ecdsa(kp), "").map_err(invalid)
}

fn from_p384(s: &p384::SecretKey) -> Result<PrivateKey, KeyError> {
    use p384::elliptic_curve::sec1::ToEncodedPoint;
    let kp = EcdsaKeypair::NistP384 {
        public: s.public_key().to_encoded_point(false),
        private: s.clone().into(),
    };
    PrivateKey::new(KeypairData::Ecdsa(kp), "").map_err(invalid)
}

fn from_ed25519(seed: &[u8; 32]) -> Result<PrivateKey, KeyError> {
    let kp = Ed25519Keypair::from(Ed25519PrivateKey::from_bytes(seed));
    PrivateKey::new(KeypairData::Ed25519(kp), "").map_err(invalid)
}

fn from_sec1_pem(pem: &str) -> Result<PrivateKey, KeyError> {
    if let Ok(s) = p256::SecretKey::from_sec1_pem(pem) {
        return from_p256(&s);
    }
    if let Ok(s) = p384::SecretKey::from_sec1_pem(pem) {
        return from_p384(&s);
    }
    Err(KeyError::Unsupported(
        "EC key on a curve other than P-256 / P-384".into(),
    ))
}

fn from_pkcs8_der(der: &[u8]) -> Result<PrivateKey, KeyError> {
    use pkcs8::DecodePrivateKey;
    if let Ok(k) = ed25519_dalek::SigningKey::from_pkcs8_der(der) {
        return from_ed25519(&k.to_bytes());
    }
    if let Ok(k) = rsa::RsaPrivateKey::from_pkcs8_der(der) {
        return from_rsa(&k);
    }
    if let Ok(k) = p256::SecretKey::from_pkcs8_der(der) {
        return from_p256(&k);
    }
    if let Ok(k) = p384::SecretKey::from_pkcs8_der(der) {
        return from_p384(&k);
    }
    Err(KeyError::Unsupported(
        "PKCS#8 key of a type Telepad does not convert (Ed25519, RSA, ECDSA P-256 / P-384)".into(),
    ))
}

/// The key as a PKCS#8 DER document (Ed25519, RSA, P-256, P-384).
fn to_pkcs8_der(key: &PrivateKey) -> Result<pkcs8::SecretDocument, KeyError> {
    use pkcs8::EncodePrivateKey;
    let r = match key.key_data() {
        KeypairData::Ed25519(kp) => ed25519_dalek::SigningKey::from(&kp.private).to_pkcs8_der(),
        KeypairData::Rsa(kp) => to_rsa(kp)?.to_pkcs8_der(),
        KeypairData::Ecdsa(EcdsaKeypair::NistP256 { private, .. }) => {
            p256::SecretKey::from_slice(private.as_slice())
                .map_err(invalid)?
                .to_pkcs8_der()
        }
        KeypairData::Ecdsa(EcdsaKeypair::NistP384 { private, .. }) => {
            p384::SecretKey::from_slice(private.as_slice())
                .map_err(invalid)?
                .to_pkcs8_der()
        }
        _ => {
            return Err(KeyError::Unsupported(
                "this key type has no PKCS#8 form here".into(),
            ));
        }
    };
    r.map_err(invalid)
}

/// SubjectPublicKeyInfo DER of the public key.
fn spki_der(key: &PrivateKey) -> Result<Vec<u8>, KeyError> {
    use pkcs8::EncodePublicKey;
    let doc = match key.key_data() {
        KeypairData::Ed25519(kp) => ed25519_dalek::SigningKey::from(&kp.private)
            .verifying_key()
            .to_public_key_der(),
        KeypairData::Rsa(kp) => to_rsa(kp)?.to_public_key().to_public_key_der(),
        KeypairData::Ecdsa(EcdsaKeypair::NistP256 { private, .. }) => {
            p256::SecretKey::from_slice(private.as_slice())
                .map_err(invalid)?
                .public_key()
                .to_public_key_der()
        }
        KeypairData::Ecdsa(EcdsaKeypair::NistP384 { private, .. }) => {
            p384::SecretKey::from_slice(private.as_slice())
                .map_err(invalid)?
                .public_key()
                .to_public_key_der()
        }
        _ => return Err(KeyError::Unsupported("no SubjectPublicKeyInfo form".into())),
    };
    Ok(doc.map_err(invalid)?.as_bytes().to_vec())
}

/// Write `key` as `target`. `passphrase`: protect the output with it (empty
/// = none; PKCS#12 needs one). Public targets ignore it.
pub fn convert(
    key: &PrivateKey,
    target: Target,
    passphrase: &str,
) -> Result<zeroize::Zeroizing<Vec<u8>>, KeyError> {
    use pkcs8::der::pem::LineEnding as PemLe;
    if let Some(why) = unsupported(&key.algorithm(), target) {
        return Err(KeyError::Unsupported(why.into()));
    }
    if target.needs_passphrase() && passphrase.is_empty() {
        return Err(KeyError::Unsupported("PKCS#12 needs a password".into()));
    }
    let z = |v: Vec<u8>| zeroize::Zeroizing::new(v);
    Ok(match target {
        Target::OpenSsh => {
            let out = if passphrase.is_empty() {
                key.to_openssh(LineEnding::LF).map_err(invalid)?
            } else {
                key.encrypt(&mut OsRng, passphrase)
                    .map_err(invalid)?
                    .to_openssh(LineEnding::LF)
                    .map_err(invalid)?
            };
            z(out.as_bytes().to_vec())
        }
        Target::Ppk3 => z(ppk_write(key, 3, passphrase)?.into_bytes()),
        Target::Ppk2 => z(ppk_write(key, 2, passphrase)?.into_bytes()),
        Target::Pem => match key.key_data() {
            KeypairData::Rsa(kp) => {
                use rsa::pkcs1::EncodeRsaPrivateKey;
                let r = to_rsa(kp)?;
                z(r.to_pkcs1_pem(PemLe::LF)
                    .map_err(invalid)?
                    .as_bytes()
                    .to_vec())
            }
            KeypairData::Ecdsa(EcdsaKeypair::NistP256 { private, .. }) => {
                let s = p256::SecretKey::from_slice(private.as_slice()).map_err(invalid)?;
                z(s.to_sec1_pem(PemLe::LF)
                    .map_err(invalid)?
                    .as_bytes()
                    .to_vec())
            }
            KeypairData::Ecdsa(EcdsaKeypair::NistP384 { private, .. }) => {
                let s = p384::SecretKey::from_slice(private.as_slice()).map_err(invalid)?;
                z(s.to_sec1_pem(PemLe::LF)
                    .map_err(invalid)?
                    .as_bytes()
                    .to_vec())
            }
            _ => return Err(KeyError::Unsupported("no PEM form for this key".into())),
        },
        Target::Pkcs8 => {
            let doc = to_pkcs8_der(key)?;
            if passphrase.is_empty() {
                z(doc
                    .to_pem("PRIVATE KEY", PemLe::LF)
                    .map_err(invalid)?
                    .as_bytes()
                    .to_vec())
            } else {
                let info = pkcs8::PrivateKeyInfo::try_from(doc.as_bytes()).map_err(invalid)?;
                let enc = info.encrypt(OsRng, passphrase).map_err(invalid)?;
                z(enc
                    .to_pem("ENCRYPTED PRIVATE KEY", PemLe::LF)
                    .map_err(invalid)?
                    .as_bytes()
                    .to_vec())
            }
        }
        Target::Der => z(to_pkcs8_der(key)?.as_bytes().to_vec()),
        Target::Pkcs12 => {
            let doc = to_pkcs8_der(key)?;
            let pk = p12_keystore::PrivateKey::from_der(doc.as_bytes()).map_err(invalid)?;
            let mut id = [0u8; 20];
            OsRng.fill_bytes(&mut id);
            let chain = p12_keystore::PrivateKeyChain::new(
                id.to_vec(),
                pk,
                Vec::<p12_keystore::Certificate>::new(),
            );
            let mut ks = p12_keystore::KeyStore::new();
            let alias = if key.comment().is_empty() {
                "key"
            } else {
                key.comment()
            };
            ks.add_entry(alias, p12_keystore::KeyStoreEntry::PrivateKeyChain(chain));
            z(ks.writer(passphrase).write().map_err(invalid)?)
        }
        Target::Jwk => z(to_jwk(key, true)?.into_bytes()),
        Target::PublicOpenSsh => {
            let mut line = key.public_key().to_openssh().map_err(invalid)?;
            line.push('\n');
            z(line.into_bytes())
        }
        Target::PublicPem => {
            let der = spki_der(key)?;
            let doc = pkcs8::Document::try_from(der.as_slice()).map_err(invalid)?;
            z(doc
                .to_pem("PUBLIC KEY", PemLe::LF)
                .map_err(invalid)?
                .into_bytes())
        }
        Target::PublicDer => z(spki_der(key)?),
        Target::PublicJwk => z(to_jwk(key, false)?.into_bytes()),
    })
}

// ------------------------------------------------------------------ JWK

fn b64u(b: &[u8]) -> String {
    B64URL.encode(b)
}

fn to_jwk(key: &PrivateKey, private: bool) -> Result<String, KeyError> {
    use serde_json::json;
    let mut v = match key.key_data() {
        KeypairData::Ed25519(kp) => {
            let mut v = json!({"kty": "OKP", "crv": "Ed25519", "x": b64u(&kp.public.0)});
            if private {
                v["d"] = json!(b64u(&kp.private.to_bytes()));
            }
            v
        }
        KeypairData::Rsa(kp) => {
            let r = to_rsa(kp)?;
            use rsa::traits::{PrivateKeyParts, PublicKeyParts};
            let mut v = json!({
                "kty": "RSA",
                "n": b64u(&r.n().to_bytes_be()),
                "e": b64u(&r.e().to_bytes_be()),
            });
            if private {
                let primes = r.primes();
                v["d"] = json!(b64u(&r.d().to_bytes_be()));
                v["p"] = json!(b64u(&primes[0].to_bytes_be()));
                v["q"] = json!(b64u(&primes[1].to_bytes_be()));
                if let (Some(dp), Some(dq)) = (r.dp(), r.dq()) {
                    v["dp"] = json!(b64u(&dp.to_bytes_be()));
                    v["dq"] = json!(b64u(&dq.to_bytes_be()));
                }
                if let Some(qi) = r.crt_coefficient() {
                    v["qi"] = json!(b64u(&qi.to_bytes_be()));
                }
            }
            v
        }
        KeypairData::Ecdsa(EcdsaKeypair::NistP256 { private: d, public }) => {
            let mut v = json!({
                "kty": "EC", "crv": "P-256",
                "x": b64u(public.x().map(|x| x.as_slice()).unwrap_or_default()),
                "y": b64u(public.y().map(|y| y.as_slice()).unwrap_or_default()),
            });
            if private {
                v["d"] = json!(b64u(d.as_slice()));
            }
            v
        }
        KeypairData::Ecdsa(EcdsaKeypair::NistP384 { private: d, public }) => {
            let mut v = json!({
                "kty": "EC", "crv": "P-384",
                "x": b64u(public.x().map(|x| x.as_slice()).unwrap_or_default()),
                "y": b64u(public.y().map(|y| y.as_slice()).unwrap_or_default()),
            });
            if private {
                v["d"] = json!(b64u(d.as_slice()));
            }
            v
        }
        _ => return Err(KeyError::Unsupported("no JWK form for this key".into())),
    };
    if !key.comment().is_empty() {
        v["kid"] = json!(key.comment());
    }
    let mut s = serde_json::to_string_pretty(&v).map_err(invalid)?;
    s.push('\n');
    Ok(s)
}

fn from_jwk(text: &str) -> Result<PrivateKey, KeyError> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(invalid)?;
    let field = |n: &str| -> Result<Vec<u8>, KeyError> {
        let s = v
            .get(n)
            .and_then(|x| x.as_str())
            .ok_or_else(|| invalid(format!("JWK field {n} missing")))?;
        B64URL
            .decode(s.trim_end_matches('='))
            .map_err(|_| invalid(format!("JWK field {n} is not base64url")))
    };
    let kty = v.get("kty").and_then(|x| x.as_str()).unwrap_or_default();
    let crv = v.get("crv").and_then(|x| x.as_str()).unwrap_or_default();
    let mut key = match (kty, crv) {
        ("OKP", "Ed25519") => {
            let d = field("d")?;
            let seed: [u8; 32] = d
                .as_slice()
                .try_into()
                .map_err(|_| invalid("Ed25519 d must be 32 bytes"))?;
            from_ed25519(&seed)?
        }
        ("EC", "P-256") => from_p256(&p256::SecretKey::from_slice(&field("d")?).map_err(invalid)?)?,
        ("EC", "P-384") => from_p384(&p384::SecretKey::from_slice(&field("d")?).map_err(invalid)?)?,
        ("RSA", _) => {
            let big = |n: &str| field(n).map(|b| rsa::BigUint::from_bytes_be(&b));
            let r = rsa::RsaPrivateKey::from_components(
                big("n")?,
                big("e")?,
                big("d")?,
                vec![big("p")?, big("q")?],
            )
            .map_err(invalid)?;
            from_rsa(&r)?
        }
        _ => {
            return Err(KeyError::Unsupported(format!(
                "JWK {kty} {crv} is not a key type Telepad converts"
            )));
        }
    };
    if let Some(kid) = v.get("kid").and_then(|x| x.as_str()) {
        key.set_comment(kid);
    }
    Ok(key)
}

// ------------------------------------------------------------------ PPK

fn put_string(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

fn put_mpint(out: &mut Vec<u8>, m: &Mpint) {
    let b = m.as_positive_bytes().unwrap_or_default();
    let b: &[u8] = {
        let mut i = 0;
        while i < b.len() && b[i] == 0 {
            i += 1;
        }
        &b[i..]
    };
    if b.first().is_some_and(|x| x & 0x80 != 0) {
        out.extend_from_slice(&((b.len() + 1) as u32).to_be_bytes());
        out.push(0);
        out.extend_from_slice(b);
    } else {
        put_string(out, b);
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn string(&mut self) -> Result<&'a [u8], KeyError> {
        if self.0.len() < 4 {
            return Err(invalid("PPK private part is too short"));
        }
        let n = u32::from_be_bytes([self.0[0], self.0[1], self.0[2], self.0[3]]) as usize;
        if self.0.len() < 4 + n {
            return Err(invalid("PPK private part is too short"));
        }
        let (s, rest) = self.0[4..].split_at(n);
        self.0 = rest;
        Ok(s)
    }
    fn mpint(&mut self) -> Result<Mpint, KeyError> {
        let b = self.string()?;
        let b: Vec<u8> = b.iter().copied().skip_while(|x| *x == 0).collect();
        Mpint::from_positive_bytes(&b).map_err(invalid)
    }
}

/// The private part PuTTY keeps for each key type.
fn ppk_private_blob(key: &PrivateKey) -> Result<zeroize::Zeroizing<Vec<u8>>, KeyError> {
    let mut out = zeroize::Zeroizing::new(Vec::new());
    match key.key_data() {
        KeypairData::Ed25519(kp) => put_string(&mut out, &kp.private.to_bytes()),
        KeypairData::Rsa(kp) => {
            put_mpint(&mut out, &kp.private.d);
            put_mpint(&mut out, &kp.private.p);
            put_mpint(&mut out, &kp.private.q);
            put_mpint(&mut out, &kp.private.iqmp);
        }
        KeypairData::Ecdsa(e) => {
            let m = Mpint::from_positive_bytes(e.private_key_bytes()).map_err(invalid)?;
            put_mpint(&mut out, &m);
        }
        KeypairData::Dsa(kp) => put_mpint(&mut out, kp.private.as_mpint()),
        _ => {
            return Err(KeyError::Unsupported(
                "PuTTY cannot hold this key type".into(),
            ));
        }
    }
    Ok(out)
}

fn wrap64(b64: &str) -> Vec<&str> {
    b64.as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, KeyError> {
    if !s.len().is_multiple_of(2) {
        return Err(invalid("bad hex"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| invalid("bad hex")))
        .collect()
}

fn mac_data(alg: &str, enc: &str, comment: &str, public: &[u8], private: &[u8]) -> Vec<u8> {
    let mut d = Vec::new();
    put_string(&mut d, alg.as_bytes());
    put_string(&mut d, enc.as_bytes());
    put_string(&mut d, comment.as_bytes());
    put_string(&mut d, public);
    put_string(&mut d, private);
    d
}

/// Argon2 parameters of a PPK v3 file.
struct Argon {
    flavour: argon2::Algorithm,
    memory: u32,
    passes: u32,
    parallelism: u32,
    salt: Vec<u8>,
}

/// v3: key (32) + IV (16) + MAC key (32) from the passphrase.
fn argon_keys(a: &Argon, pass: &str) -> Result<zeroize::Zeroizing<[u8; 80]>, KeyError> {
    let params =
        argon2::Params::new(a.memory, a.passes, a.parallelism, Some(80)).map_err(invalid)?;
    let ctx = argon2::Argon2::new(a.flavour, argon2::Version::V0x13, params);
    let mut out = zeroize::Zeroizing::new([0u8; 80]);
    ctx.hash_password_into(pass.as_bytes(), &a.salt, &mut *out)
        .map_err(invalid)?;
    Ok(out)
}

/// v2: AES key from the passphrase (IV is zero).
fn v2_key(pass: &str) -> zeroize::Zeroizing<[u8; 32]> {
    use sha1::{Digest, Sha1};
    let mut k = zeroize::Zeroizing::new([0u8; 32]);
    let a = Sha1::new()
        .chain_update([0, 0, 0, 0])
        .chain_update(pass.as_bytes())
        .finalize();
    let b = Sha1::new()
        .chain_update([0, 0, 0, 1])
        .chain_update(pass.as_bytes())
        .finalize();
    k[..20].copy_from_slice(&a);
    k[20..].copy_from_slice(&b[..12]);
    k
}

fn v2_mac_key(pass: &str) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    Sha1::new()
        .chain_update(b"putty-private-key-file-mac-key")
        .chain_update(pass.as_bytes())
        .finalize()
        .into()
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::Mac;
    let mut m = <hmac::Hmac<sha1::Sha1>>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::Mac;
    let mut m =
        <hmac::Hmac<sha2_010::Sha256>>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

fn aes_cbc_encrypt(key: &[u8], iv: &[u8], data: &mut [u8]) {
    use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::NoPadding};
    let len = data.len();
    let _ = cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv)
        .expect("32-byte key, 16-byte IV")
        .encrypt_padded_mut::<NoPadding>(data, len);
}

fn aes_cbc_decrypt(key: &[u8], iv: &[u8], data: &mut [u8]) -> Result<(), KeyError> {
    use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::NoPadding};
    if !data.len().is_multiple_of(16) {
        return Err(invalid("PPK private part is not a whole number of blocks"));
    }
    cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
        .map_err(invalid)?
        .decrypt_padded_mut::<NoPadding>(data)
        .map_err(invalid)?;
    Ok(())
}

/// PuTTY `.ppk` text of `key` (`version` 2 or 3).
pub fn ppk_write(key: &PrivateKey, version: u8, pass: &str) -> Result<String, KeyError> {
    let alg = key.algorithm().to_string();
    let public = key.public_key().to_bytes().map_err(invalid)?;
    let mut private = ppk_private_blob(key)?;
    let comment = key.comment().to_string();
    let enc = if pass.is_empty() {
        "none"
    } else {
        "aes256-cbc"
    };
    let mut out =
        format!("PuTTY-User-Key-File-{version}: {alg}\nEncryption: {enc}\nComment: {comment}\n");
    let pub_b64 = B64.encode(&public);
    let pub_lines = wrap64(&pub_b64);
    out.push_str(&format!("Public-Lines: {}\n", pub_lines.len()));
    for l in pub_lines {
        out.push_str(l);
        out.push('\n');
    }
    let mac_key: zeroize::Zeroizing<Vec<u8>>;
    if !pass.is_empty() {
        // Pad to whole AES blocks.
        let pad = (16 - private.len() % 16) % 16;
        if version == 2 {
            use sha1::{Digest, Sha1};
            let h = Sha1::digest(&*private);
            private.extend_from_slice(&h[..pad]);
        } else {
            let mut p = vec![0u8; pad];
            OsRng.fill_bytes(&mut p);
            private.extend_from_slice(&p);
        }
    }
    let mac_input_private = private.clone();
    if version == 2 {
        mac_key = zeroize::Zeroizing::new(v2_mac_key(pass).to_vec());
        if !pass.is_empty() {
            let k = v2_key(pass);
            aes_cbc_encrypt(&*k, &[0u8; 16], &mut private);
        }
    } else if pass.is_empty() {
        mac_key = zeroize::Zeroizing::new(Vec::new());
    } else {
        let mut salt = vec![0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let a = Argon {
            flavour: argon2::Algorithm::Argon2id,
            memory: 8192,
            passes: 13,
            parallelism: 1,
            salt,
        };
        let keys = argon_keys(&a, pass)?;
        aes_cbc_encrypt(&keys[..32], &keys[32..48], &mut private);
        mac_key = zeroize::Zeroizing::new(keys[48..80].to_vec());
        out.push_str(&format!(
            "Key-Derivation: Argon2id\nArgon2-Memory: {}\nArgon2-Passes: {}\nArgon2-Parallelism: {}\nArgon2-Salt: {}\n",
            a.memory,
            a.passes,
            a.parallelism,
            hex(&a.salt)
        ));
    }
    let priv_b64 = zeroize::Zeroizing::new(B64.encode(&*private));
    let priv_lines = wrap64(&priv_b64);
    out.push_str(&format!("Private-Lines: {}\n", priv_lines.len()));
    for l in priv_lines {
        out.push_str(l);
        out.push('\n');
    }
    let data = mac_data(&alg, enc, &comment, &public, &mac_input_private);
    let mac = if version == 2 {
        hmac_sha1(&mac_key, &data)
    } else {
        hmac_sha256(&mac_key, &data)
    };
    out.push_str(&format!("Private-MAC: {}\n", hex(&mac)));
    Ok(out)
}

/// Header fields of a `.ppk` file.
struct Ppk {
    version: u8,
    alg: String,
    enc: String,
    comment: String,
    public: Vec<u8>,
    private: Vec<u8>,
    argon: Option<Argon>,
    mac: Vec<u8>,
}

fn ppk_parse(text: &str) -> Result<Ppk, KeyError> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    let mut field = |name: &str| -> Result<String, KeyError> {
        let l = lines
            .next()
            .ok_or_else(|| invalid(format!("PPK: {name} missing")))?;
        l.strip_prefix(name)
            .and_then(|r| r.strip_prefix(": "))
            .map(str::to_string)
            .ok_or_else(|| invalid(format!("PPK: expected {name}")))
    };
    let first = text.lines().next().unwrap_or_default();
    let version = if first.starts_with("PuTTY-User-Key-File-3") {
        3
    } else if first.starts_with("PuTTY-User-Key-File-2") {
        2
    } else {
        return Err(invalid("not a PuTTY key file"));
    };
    let alg = field(&format!("PuTTY-User-Key-File-{version}"))?;
    let enc = field("Encryption")?;
    let comment = field("Comment")?;
    let n: usize = field("Public-Lines")?.parse().map_err(invalid)?;
    let mut b = String::new();
    for _ in 0..n {
        b.push_str(&field_raw(&mut lines)?);
    }
    let public = B64.decode(b.trim()).map_err(invalid)?;
    let mut argon = None;
    let mut next = lines
        .next()
        .ok_or_else(|| invalid("PPK: Private-Lines missing"))?
        .to_string();
    if next.starts_with("Key-Derivation: ") {
        let flavour = match next.trim_start_matches("Key-Derivation: ") {
            "Argon2id" => argon2::Algorithm::Argon2id,
            "Argon2i" => argon2::Algorithm::Argon2i,
            "Argon2d" => argon2::Algorithm::Argon2d,
            other => return Err(KeyError::Unsupported(format!("PPK key derivation {other}"))),
        };
        let mut num = |name: &str| -> Result<u32, KeyError> {
            let l = lines
                .next()
                .ok_or_else(|| invalid(format!("PPK: {name} missing")))?;
            l.strip_prefix(&format!("{name}: "))
                .ok_or_else(|| invalid(format!("PPK: expected {name}")))?
                .parse()
                .map_err(invalid)
        };
        let memory = num("Argon2-Memory")?;
        let passes = num("Argon2-Passes")?;
        let parallelism = num("Argon2-Parallelism")?;
        let salt_line = lines
            .next()
            .ok_or_else(|| invalid("PPK: Argon2-Salt missing"))?;
        let salt = unhex(
            salt_line
                .strip_prefix("Argon2-Salt: ")
                .ok_or_else(|| invalid("PPK: expected Argon2-Salt"))?,
        )?;
        argon = Some(Argon {
            flavour,
            memory,
            passes,
            parallelism,
            salt,
        });
        next = lines
            .next()
            .ok_or_else(|| invalid("PPK: Private-Lines missing"))?
            .to_string();
    }
    let m: usize = next
        .strip_prefix("Private-Lines: ")
        .ok_or_else(|| invalid("PPK: expected Private-Lines"))?
        .parse()
        .map_err(invalid)?;
    let mut b = String::new();
    for _ in 0..m {
        b.push_str(&field_raw(&mut lines)?);
    }
    let private = B64.decode(b.trim()).map_err(invalid)?;
    let mac_line = lines
        .next()
        .ok_or_else(|| invalid("PPK: Private-MAC missing"))?;
    let mac = unhex(
        mac_line
            .strip_prefix("Private-MAC: ")
            .ok_or_else(|| invalid("PPK: expected Private-MAC"))?,
    )?;
    Ok(Ppk {
        version,
        alg,
        enc,
        comment,
        public,
        private,
        argon,
        mac,
    })
}

fn field_raw<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Result<String, KeyError> {
    lines
        .next()
        .map(|l| l.trim().to_string())
        .ok_or_else(|| invalid("PPK: file ends early"))
}

/// Public key and "encrypted" of a `.ppk`, without the passphrase.
fn ppk_public(text: &str) -> Result<(PublicKey, bool), KeyError> {
    let p = ppk_parse(text)?;
    let mut k = PublicKey::from_bytes(&p.public).map_err(invalid)?;
    k.set_comment(p.comment);
    Ok((k, p.enc != "none"))
}

/// Read a `.ppk` (v2 / v3, plain or aes256-cbc) and check its MAC.
pub fn ppk_read(text: &str, pass: Option<&str>) -> Result<PrivateKey, KeyError> {
    let mut p = ppk_parse(text)?;
    let pass = pass.unwrap_or_default();
    let encrypted = match p.enc.as_str() {
        "none" => false,
        "aes256-cbc" => true,
        other => return Err(KeyError::Unsupported(format!("PPK encryption {other}"))),
    };
    if encrypted && pass.is_empty() {
        return Err(KeyError::NeedsPassphrase);
    }
    let mac_key: zeroize::Zeroizing<Vec<u8>> = if p.version == 2 {
        if encrypted {
            let k = v2_key(pass);
            aes_cbc_decrypt(&*k, &[0u8; 16], &mut p.private)?;
        }
        zeroize::Zeroizing::new(v2_mac_key(if encrypted { pass } else { "" }).to_vec())
    } else if encrypted {
        let a = p
            .argon
            .as_ref()
            .ok_or_else(|| invalid("PPK v3: key derivation missing"))?;
        let keys = argon_keys(a, pass)?;
        aes_cbc_decrypt(&keys[..32], &keys[32..48], &mut p.private)?;
        zeroize::Zeroizing::new(keys[48..80].to_vec())
    } else {
        zeroize::Zeroizing::new(Vec::new())
    };
    let data = mac_data(&p.alg, &p.enc, &p.comment, &p.public, &p.private);
    let mac = if p.version == 2 {
        hmac_sha1(&mac_key, &data)
    } else {
        hmac_sha256(&mac_key, &data)
    };
    if mac != p.mac {
        p.private.iter_mut().for_each(|b| *b = 0);
        return Err(if encrypted {
            KeyError::WrongPassphrase
        } else {
            invalid("PPK MAC does not match (file damaged)")
        });
    }
    let public = PublicKey::from_bytes(&p.public).map_err(invalid)?;
    let mut r = Reader(&p.private);
    let data = match public.key_data() {
        KeyData::Ed25519(_) => {
            let seed: [u8; 32] = r
                .string()?
                .try_into()
                .map_err(|_| invalid("Ed25519 private part must be 32 bytes"))?;
            KeypairData::Ed25519(Ed25519Keypair::from(Ed25519PrivateKey::from_bytes(&seed)))
        }
        KeyData::Rsa(pk) => {
            let private = RsaPrivateKey {
                d: r.mpint()?,
                p: r.mpint()?,
                q: r.mpint()?,
                iqmp: r.mpint()?,
            };
            KeypairData::Rsa(RsaKeypair {
                public: pk.clone(),
                private,
            })
        }
        KeyData::Ecdsa(e) => {
            let m = r.mpint()?;
            let raw = m.as_positive_bytes().unwrap_or_default();
            let pad = |n: usize| {
                let mut v = vec![0u8; n.saturating_sub(raw.len())];
                v.extend_from_slice(raw);
                v
            };
            match e.curve() {
                ssh_key::EcdsaCurve::NistP256 => {
                    let s = p256::SecretKey::from_slice(&pad(32)).map_err(invalid)?;
                    from_p256(&s)?.key_data().clone()
                }
                ssh_key::EcdsaCurve::NistP384 => {
                    let s = p384::SecretKey::from_slice(&pad(48)).map_err(invalid)?;
                    from_p384(&s)?.key_data().clone()
                }
                ssh_key::EcdsaCurve::NistP521 => {
                    return Err(KeyError::Unsupported(
                        "ECDSA P-521 PuTTY keys: convert with puttygen".into(),
                    ));
                }
            }
        }
        _ => {
            return Err(KeyError::Unsupported(format!(
                "PuTTY key type {} is not converted",
                p.alg
            )));
        }
    };
    p.private.iter_mut().for_each(|b| *b = 0);
    let key = PrivateKey::new(data, p.comment.clone()).map_err(invalid)?;
    if key.public_key().key_data() != public.key_data() {
        return Err(invalid("PPK private part does not match its public key"));
    }
    Ok(key)
}

// ------------------------------------------------------------------ generate

/// Key types offered by Generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GenType {
    #[default]
    Ed25519,
    Ecdsa256,
    Ecdsa384,
    Ecdsa521,
    Rsa(u32),
}

impl GenType {
    pub fn label(self) -> String {
        match self {
            GenType::Ed25519 => "Ed25519 (recommended)".into(),
            GenType::Ecdsa256 => "ECDSA P-256".into(),
            GenType::Ecdsa384 => "ECDSA P-384".into(),
            GenType::Ecdsa521 => "ECDSA P-521".into(),
            GenType::Rsa(b) => format!("RSA {b}"),
        }
    }
    /// Default file name in `~/.ssh`.
    pub fn file_stem(self) -> &'static str {
        match self {
            GenType::Ed25519 => "id_ed25519",
            GenType::Ecdsa256 | GenType::Ecdsa384 | GenType::Ecdsa521 => "id_ecdsa",
            GenType::Rsa(_) => "id_rsa",
        }
    }
}

/// `dir/stem`, or `dir/stem_2`, `_3` … when it (or its `.pub`) exists.
pub fn next_free(dir: &Path, stem: &str) -> PathBuf {
    let free = |p: &Path| !p.exists() && !pub_path(p).exists();
    let first = dir.join(stem);
    if free(&first) {
        return first;
    }
    (2..1000)
        .map(|i| dir.join(format!("{stem}_{i}")))
        .find(|p| free(p))
        .unwrap_or_else(|| dir.join(format!("{stem}_new")))
}

/// Why a file name for Generate is not usable, if it is not.
pub fn check_new_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_string();
    if name.is_empty() || name.ends_with(".pub") {
        return Some("Give a file name (without .pub)".into());
    }
    if path.exists() || pub_path(path).exists() {
        return Some(format!(
            "{} already exists: Telepad never overwrites a key",
            path.display()
        ));
    }
    if !path
        .parent()
        .is_some_and(|p| p.as_os_str().is_empty() || p.is_dir() || !p.exists())
    {
        return Some("The folder cannot be used".into());
    }
    None
}

/// `ssh-keygen` arguments (separate, never a shell string). The passphrase
/// is not here: the key is written without one into a private folder and
/// encrypted by Telepad ([`protect`]) before it is moved into place.
pub fn keygen_args(t: GenType, comment: &str, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = vec!["-q".into(), "-t".into()];
    match t {
        GenType::Ed25519 => a.push("ed25519".into()),
        GenType::Ecdsa256 | GenType::Ecdsa384 | GenType::Ecdsa521 => {
            a.push("ecdsa".into());
            a.push("-b".into());
            a.push(
                match t {
                    GenType::Ecdsa256 => "256",
                    GenType::Ecdsa384 => "384",
                    _ => "521",
                }
                .into(),
            );
        }
        GenType::Rsa(bits) => {
            a.push("rsa".into());
            a.push("-b".into());
            a.push(bits.clamp(2048, 8192).to_string());
        }
    }
    a.extend([
        "-C".into(),
        comment.into(),
        "-N".into(),
        String::new(),
        "-f".into(),
        out.to_string_lossy().to_string(),
    ]);
    a
}

/// Encrypt a freshly generated OpenSSH key in place with `pass` (no-op when
/// empty); the `.pub` stays as written.
pub fn protect(path: &Path, pass: &str) -> Result<(), KeyError> {
    if pass.is_empty() {
        return Ok(());
    }
    let bytes = zeroize::Zeroizing::new(std::fs::read(path).map_err(invalid)?);
    let key = PrivateKey::from_openssh(&*bytes).map_err(invalid)?;
    let enc = key
        .encrypt(&mut OsRng, pass)
        .map_err(invalid)?
        .to_openssh(LineEnding::LF)
        .map_err(invalid)?;
    std::fs::write(path, enc.as_bytes()).map_err(invalid)
}

/// Generate a key pair at `path` with the system's `ssh-keygen` (#101).
/// It writes into a private folder first (owner-only), Telepad encrypts the
/// key with `pass` there, then both files are moved into place; nothing is
/// ever overwritten. The passphrase never reaches a command line.
pub fn generate(kind: GenType, path: &Path, comment: &str, pass: &str) -> Result<PathBuf, String> {
    if let Some(why) = check_new_path(path) {
        return Err(why);
    }
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("Choose a folder for the key")?;
    if !dir.exists() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        crate::platform::restrict_dir_to_owner(dir)?;
    }
    let work = dir.join(format!(".telepad-keygen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir(&work).map_err(|e| format!("cannot create {}: {e}", work.display()))?;
    let result = (|| {
        crate::platform::restrict_dir_to_owner(&work)?;
        let tmp = work.join("key");
        let program = crate::platform::ssh_keygen_program();
        let mut cmd = std::process::Command::new(&program);
        cmd.args(keygen_args(kind, comment, &tmp))
            .stdin(std::process::Stdio::null());
        crate::platform::hide_console(&mut cmd);
        let out = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                crate::platform::ssh_keygen_missing()
            } else {
                format!("cannot run {}: {e}", program.display())
            }
        })?;
        if !out.status.success() {
            return Err(format!(
                "ssh-keygen failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        protect(&tmp, pass).map_err(|e| e.to_string())?;
        if path.exists() || pub_path(path).exists() {
            return Err(format!(
                "{} appeared meanwhile: not overwritten",
                path.display()
            ));
        }
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot move the key: {e}"))?;
        std::fs::rename(pub_path(&tmp), pub_path(path))
            .map_err(|e| format!("cannot move the public key: {e}"))?;
        crate::platform::restrict_to_owner(path)?;
        Ok(path.to_path_buf())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Read `source` (with `pass_in`), write it to `dest` as `target`
/// (protected with `pass_out`). Never over the source; private output gets
/// owner-only permissions. Returns what was written, for the dialog.
pub fn convert_file(
    source: &Path,
    pass_in: &str,
    target: Target,
    pass_out: &str,
    dest: &Path,
) -> Result<String, String> {
    if same_file(source, dest) || source == dest {
        return Err("Choose another file: the source is never overwritten".into());
    }
    let meta = std::fs::metadata(source).map_err(|e| format!("{}: {e}", source.display()))?;
    if meta.len() > MAX_KEY_FILE {
        return Err("too large for a key".into());
    }
    let bytes = zeroize::Zeroizing::new(
        std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?,
    );
    let key = load(&bytes, source, (!pass_in.is_empty()).then_some(pass_in))
        .map_err(|e| e.to_string())?;
    let out = convert(&key, target, pass_out).map_err(|e| e.to_string())?;
    std::fs::write(dest, &*out).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    if !target.is_public() {
        crate::platform::restrict_to_owner(dest)?;
    }
    // ssh wants the public half next to an OpenSSH key.
    if target == Target::OpenSsh && !pub_path(dest).exists() {
        let line = convert(&key, Target::PublicOpenSsh, "").map_err(|e| e.to_string())?;
        std::fs::write(pub_path(dest), &*line)
            .map_err(|e| format!("cannot write {}: {e}", pub_path(dest).display()))?;
    }
    Ok(format!(
        "{} written as {}{}",
        dest.display(),
        target.label(),
        if target.can_encrypt() && !pass_out.is_empty() {
            ", with a passphrase"
        } else {
            ""
        }
    ))
}

/// The algorithm from the type text the scan gave ("Ed25519", "RSA 4096", …).
pub fn algorithm_of(kind: &str) -> Option<Algorithm> {
    use ssh_key::EcdsaCurve;
    Some(if kind.starts_with("Ed25519") {
        Algorithm::Ed25519
    } else if kind.starts_with("RSA") {
        Algorithm::Rsa { hash: None }
    } else if kind == "ECDSA P-256" {
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        }
    } else if kind == "ECDSA P-384" {
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP384,
        }
    } else if kind == "ECDSA P-521" {
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP521,
        }
    } else if kind.starts_with("DSA") {
        Algorithm::Dsa
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<PrivateKey> {
        let mut v = vec![
            PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap(),
            PrivateKey::random(
                &mut OsRng,
                Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP256,
                },
            )
            .unwrap(),
            PrivateKey::random(
                &mut OsRng,
                Algorithm::Ecdsa {
                    curve: ssh_key::EcdsaCurve::NistP384,
                },
            )
            .unwrap(),
        ];
        let r = rsa::RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        v.push(from_rsa(&r).unwrap());
        for k in &mut v {
            k.set_comment("test@example");
        }
        v
    }

    fn fp(k: &PrivateKey) -> String {
        k.fingerprint(HashAlg::Sha256).to_string()
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("telepad-keys-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn every_target_round_trips_to_the_same_fingerprint() {
        let dir = tmp("rt");
        for key in keys() {
            for t in Target::ALL {
                if t.is_public() || unsupported(&key.algorithm(), t).is_some() {
                    continue;
                }
                for pass in ["", "correct horse"] {
                    if (!t.can_encrypt() && !pass.is_empty())
                        || (t.needs_passphrase() && pass.is_empty())
                    {
                        continue;
                    }
                    let out = convert(&key, t, pass)
                        .unwrap_or_else(|e| panic!("{t:?} {}: {e}", key.algorithm()));
                    let path = dir.join(format!(
                        "k.{}",
                        if t.extension().is_empty() {
                            "key"
                        } else {
                            t.extension()
                        }
                    ));
                    std::fs::write(&path, &*out).unwrap();
                    let back = load(&out, &path, (!pass.is_empty()).then_some(pass))
                        .unwrap_or_else(|e| panic!("{t:?} {} pass={pass:?}: {e}", key.algorithm()));
                    assert_eq!(fp(&back), fp(&key), "{t:?} {}", key.algorithm());
                    let info = inspect(&path);
                    assert!(info.format.is_some(), "{t:?} detected");
                    if !pass.is_empty() {
                        assert_eq!(
                            load(&out, &path, Some("wrong")).unwrap_err(),
                            KeyError::WrongPassphrase,
                            "{t:?}"
                        );
                        assert!(info.encrypted, "{t:?} encrypted");
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ppk_keeps_the_comment_and_shows_the_public_key_without_passphrase() {
        let key = &keys()[0];
        for v in [2, 3] {
            let text = ppk_write(key, v, "pw").unwrap();
            assert!(text.starts_with(&format!("PuTTY-User-Key-File-{v}: ssh-ed25519")));
            assert!(text.contains("Comment: test@example"));
            let (p, enc) = ppk_public(&text).unwrap();
            assert!(enc);
            assert_eq!(
                p.fingerprint(HashAlg::Sha256),
                key.fingerprint(HashAlg::Sha256)
            );
            assert_eq!(
                ppk_read(&text, None).unwrap_err(),
                KeyError::NeedsPassphrase
            );
            assert_eq!(
                ppk_read(&text, Some("pw")).unwrap().comment(),
                "test@example"
            );
            // A changed byte fails the MAC.
            let bad = text.replace("Comment: test@example", "Comment: test@exampla");
            assert!(ppk_read(&bad, Some("pw")).is_err());
        }
    }

    #[test]
    fn unsupported_combinations_say_why() {
        let ed = Algorithm::Ed25519;
        assert!(unsupported(&ed, Target::Pem).unwrap().contains("PKCS#8"));
        assert!(unsupported(&ed, Target::Pkcs8).is_none());
        let p521 = Algorithm::Ecdsa {
            curve: ssh_key::EcdsaCurve::NistP521,
        };
        assert!(unsupported(&p521, Target::Jwk).is_some());
        assert!(unsupported(&p521, Target::Ppk3).is_none());
        let key = &keys()[0];
        assert!(
            convert(key, Target::Pkcs12, "").is_err(),
            "PKCS#12 needs a password"
        );
    }

    #[test]
    fn public_exports() {
        for key in keys() {
            let line =
                String::from_utf8(convert(&key, Target::PublicOpenSsh, "").unwrap().to_vec())
                    .unwrap();
            assert!(line.ends_with("test@example\n"));
            let pem =
                String::from_utf8(convert(&key, Target::PublicPem, "").unwrap().to_vec()).unwrap();
            assert!(pem.starts_with("-----BEGIN PUBLIC KEY-----"));
            let jwk =
                String::from_utf8(convert(&key, Target::PublicJwk, "").unwrap().to_vec()).unwrap();
            assert!(!jwk.contains("\"d\""), "no private part: {jwk}");
            assert!(!convert(&key, Target::PublicDer, "").unwrap().is_empty());
        }
    }

    #[test]
    fn scan_lists_keys_skips_other_files_and_reads_pub_files() {
        let dir = tmp("scan");
        let key = &keys()[0];
        std::fs::write(
            dir.join("id_ed25519"),
            &*convert(key, Target::OpenSsh, "pw").unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("id_ed25519.pub"),
            &*convert(key, Target::PublicOpenSsh, "").unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("work.ppk"), ppk_write(key, 3, "").unwrap()).unwrap();
        std::fs::write(dir.join("known_hosts"), "x").unwrap();
        std::fs::write(dir.join("config"), "Host *").unwrap();
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let missing = dir.join("gone_key");
        let v = scan(&dir, std::slice::from_ref(&missing));
        let names: Vec<String> = v
            .iter()
            .map(|k| k.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["id_ed25519", "work.ppk", "gone_key"]);
        let k = &v[0];
        assert_eq!(k.format, Some(Format::OpenSsh));
        assert!(k.encrypted);
        assert!(k.has_pub_file);
        assert_eq!(k.kind, "Ed25519");
        assert_eq!(k.fingerprint.as_deref(), Some(fp(key).as_str()));
        assert_eq!(k.comment, "test@example");
        assert_eq!(v[1].format, Some(Format::Ppk3));
        assert!(!v[1].encrypted);
        assert!(
            v[2].error.is_some(),
            "an IdentityFile that is gone says why"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn next_free_never_reuses_a_name() {
        let dir = tmp("free");
        assert_eq!(next_free(&dir, "id_ed25519"), dir.join("id_ed25519"));
        std::fs::write(dir.join("id_ed25519.pub"), "x").unwrap();
        assert_eq!(next_free(&dir, "id_ed25519"), dir.join("id_ed25519_2"));
        assert!(check_new_path(&dir.join("id_ed25519")).is_some());
        assert!(check_new_path(&dir.join("id_ed25519_2")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keygen_arguments_are_separate_and_carry_no_passphrase() {
        let a = keygen_args(
            GenType::Rsa(4096),
            "me@pc",
            Path::new("/home/me/.ssh/id_rsa"),
        );
        assert_eq!(
            a,
            [
                "-q",
                "-t",
                "rsa",
                "-b",
                "4096",
                "-C",
                "me@pc",
                "-N",
                "",
                "-f",
                "/home/me/.ssh/id_rsa"
            ]
        );
        let a = keygen_args(GenType::Ed25519, "x; rm -rf /", Path::new("k"));
        assert!(
            a.contains(&"x; rm -rf /".to_string()),
            "one argument, no shell"
        );
    }

    /// The real ssh-keygen: run with `TELEPAD_TEST_KEYGEN=1`.
    #[test]
    fn generate_with_ssh_keygen() {
        if std::env::var_os("TELEPAD_TEST_KEYGEN").is_none() {
            return;
        }
        let dir = tmp("keygen");
        let path = dir.join("id_ed25519");
        let made = generate(GenType::Ed25519, &path, "test@example", "pw").unwrap();
        assert_eq!(made, path);
        let k = inspect(&path);
        assert_eq!(k.format, Some(Format::OpenSsh));
        assert!(k.encrypted, "protected with the passphrase");
        assert_eq!(k.comment, "test@example");
        assert!(k.has_pub_file);
        let bytes = std::fs::read(&path).unwrap();
        assert!(load(&bytes, &path, Some("pw")).is_ok());
        assert!(
            generate(GenType::Ed25519, &path, "x", "").is_err(),
            "never overwrites"
        );
        assert!(
            !dir.read_dir().unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".telepad-keygen")),
            "work folder removed"
        );
        let rsa = dir.join("id_rsa");
        generate(GenType::Rsa(3072), &rsa, "r", "").unwrap();
        assert_eq!(inspect(&rsa).kind, "RSA 3072");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// puttygen (PuTTY) reads what Telepad writes: run with
    /// `TELEPAD_TEST_PUTTYGEN=<path to puttygen>`; keys are made at run time.
    #[test]
    fn puttygen_reads_our_ppk() {
        // `TELEPAD_TEST_PPK_DIR=<dir>`: only write samples (and their
        // fingerprints) for a puttygen elsewhere, e.g. in a container.
        if let Some(out) = std::env::var_os("TELEPAD_TEST_PPK_DIR") {
            let out = PathBuf::from(out);
            // Files puttygen wrote from our samples: read them back.
            if out.join("back-k0.fp").exists() {
                for i in 0..4 {
                    let orig = std::fs::read(out.join(format!("k{i}.openssh"))).unwrap();
                    let want = fp(&PrivateKey::from_openssh(&orig).unwrap());
                    for v in [2, 3] {
                        for (pw, pass) in [("plain", None), ("pw", Some("pw"))] {
                            let p = out.join(format!("back-k{i}-v{v}-{pw}.ppk"));
                            let text = std::fs::read_to_string(&p).unwrap();
                            let k = ppk_read(&text, pass)
                                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                            assert_eq!(fp(&k), want, "{}", p.display());
                        }
                    }
                }
                return;
            }
            let mut list = String::new();
            for (i, key) in keys().into_iter().enumerate() {
                for (v, pass) in [(2, ""), (3, ""), (2, "pw"), (3, "pw")] {
                    let name = format!(
                        "k{i}-v{v}-{}.ppk",
                        if pass.is_empty() { "plain" } else { "pw" }
                    );
                    std::fs::write(out.join(&name), ppk_write(&key, v, pass).unwrap()).unwrap();
                    list.push_str(&format!(
                        "{name} {pass} {}
",
                        fp(&key)
                    ));
                }
                std::fs::write(
                    out.join(format!("k{i}.openssh")),
                    &*convert(&key, Target::OpenSsh, "").unwrap(),
                )
                .unwrap();
            }
            std::fs::write(out.join("list.txt"), list).unwrap();
            return;
        }
        let Some(bin) = std::env::var_os("TELEPAD_TEST_PUTTYGEN") else {
            return;
        };
        let dir = tmp("puttygen");
        for key in keys() {
            for (v, pass) in [(2, ""), (3, ""), (2, "pw"), (3, "pw")] {
                let p = dir.join("k.ppk");
                std::fs::write(&p, ppk_write(&key, v, pass).unwrap()).unwrap();
                let pf = dir.join("pass.txt");
                std::fs::write(&pf, pass).unwrap();
                let mut cmd = std::process::Command::new(&bin);
                cmd.arg(&p).args(["-O", "fingerprint"]);
                if !pass.is_empty() {
                    cmd.arg("--old-passphrase").arg(&pf);
                }
                let out = cmd.output().unwrap();
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(
                    out.status.success(),
                    "v{v} {}: {}",
                    key.algorithm(),
                    String::from_utf8_lossy(&out.stderr)
                );
                assert!(text.contains(&fp(&key)), "v{v} {}: {text}", key.algorithm());
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
