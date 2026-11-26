//! Portainer servers (#171): registered in `telepad.db`, read only through
//! an `ssh -N -L` tunnel to the Telepad host that runs them, with an API
//! access token kept in the OS password store. Telepad only sends `GET`
//! requests: it lists the environments and stacks a Portainer manages and
//! never changes anything there.
//!
//! Portainer's HTTPS certificate is usually self-signed: it is trusted on
//! first use and its SHA-256 fingerprint is pinned in `telepad.db`; a
//! different certificate later is refused until the user accepts it (like
//! SSH `StrictHostKeyChecking=accept-new`).

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::collector::SshSettings;
use crate::config::HostEntry;
use crate::db::Secret;
use crate::db::tunnel::Tunnel;

/// Oldest Portainer whose API takes an access token (`X-API-Key`).
pub const MIN_VERSION: &str = "2.11.0";

/// OS password store service of the API tokens.
const SERVICE: &str = "Telepad Portainer";

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Most bytes one API answer may have.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// A registered Portainer server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Portainer {
    /// Unique (any case), shown in the list and the page title.
    pub name: String,
    /// The Telepad host the tunnel goes through.
    pub alias: String,
    /// Portainer's port on that host (reached as 127.0.0.1:port there).
    pub port: u16,
    pub https: bool,
    /// Pinned SHA-256 of the HTTPS certificate (`AB:CD:…`), empty until the
    /// first read.
    pub fingerprint: String,
}

impl Portainer {
    /// The keyring entry of its token.
    pub fn token_key(&self) -> String {
        token_key(&self.name)
    }

    /// Its web UI for the browser (Open in Portainer): the host's address.
    pub fn web_url(&self, host: &HostEntry) -> String {
        let addr = if host.hostname.is_empty() {
            host.alias.as_str()
        } else {
            host.hostname.as_str()
        };
        let addr = if addr.contains(':') {
            format!("[{addr}]")
        } else {
            addr.to_string()
        };
        let scheme = if self.https { "https" } else { "http" };
        format!("{scheme}://{addr}:{}/", self.port)
    }
}

pub fn token_key(name: &str) -> String {
    format!("portainer:{}", name.to_lowercase())
}

/// A name is one line of 1 to 64 characters.
pub fn check_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("A name is needed".into());
    }
    if n.chars().count() > 64 {
        return Err("The name is longer than 64 characters".into());
    }
    if n.chars().any(char::is_control) {
        return Err("The name must be one line".into());
    }
    Ok(())
}

/// The saved token of `name`, `None` when there is none.
pub fn load_token(name: &str) -> Result<Option<Secret>, String> {
    crate::db::secret::load_in(SERVICE, &token_key(name))
}

pub fn save_token(name: &str, token: &Secret) -> Result<(), String> {
    crate::db::secret::save_in(SERVICE, &token_key(name), token)
}

/// Remove the saved token; no error when there was none.
pub fn forget_token(name: &str) -> Result<(), String> {
    crate::db::secret::forget_in(SERVICE, &token_key(name))
}

/// One environment (endpoint) of a Portainer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    /// Portainer's endpoint type (1 Docker, 2 Agent, 3 Azure, 4 Edge,
    /// 5 Kubernetes, 6 Kubernetes agent, 7 Kubernetes Edge).
    pub kind: i64,
    pub url: String,
    /// `None` when Portainer does not say.
    pub up: Option<bool>,
    pub stacks: Option<u64>,
    pub running: Option<u64>,
    pub total: Option<u64>,
}

impl Environment {
    pub fn kind_name(&self) -> &'static str {
        kind_name(self.kind)
    }

    pub fn kubernetes(&self) -> bool {
        matches!(self.kind, 5..=7)
    }

    /// The address its URL points to (`tcp://192.0.2.10:9001` →
    /// `192.0.2.10`); empty for the local socket and Edge agents.
    pub fn address(&self) -> &str {
        let u = self.url.trim();
        let Some((scheme, rest)) = u.split_once("://") else {
            return "";
        };
        if scheme.eq_ignore_ascii_case("unix") || scheme.eq_ignore_ascii_case("npipe") {
            return "";
        }
        let host = rest.split(['/', '?']).next().unwrap_or("");
        let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
        if let Some(v6) = host.strip_prefix('[') {
            return v6.split(']').next().unwrap_or("");
        }
        match host.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host,
        }
    }

    /// Runs on the Portainer's own host (local socket).
    pub fn local(&self) -> bool {
        let u = self.url.trim().to_lowercase();
        u.starts_with("unix://") || u.starts_with("npipe://")
    }
}

pub fn kind_name(kind: i64) -> &'static str {
    match kind {
        1 => "Docker",
        2 => "Docker (agent)",
        3 => "Azure ACI",
        4 => "Docker (Edge)",
        5 => "Kubernetes",
        6 => "Kubernetes (agent)",
        7 => "Kubernetes (Edge)",
        _ => "Other",
    }
}

/// One stack of a Portainer. Its environment variables are never read:
/// they often hold secrets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Stack {
    pub id: i64,
    pub name: String,
    /// 1 Swarm, 2 Compose, 3 Kubernetes.
    pub kind: i64,
    pub env: i64,
    pub active: Option<bool>,
    /// Unix seconds; `None` when not known.
    pub created: Option<i64>,
    pub updated: Option<i64>,
    pub created_by: String,
    pub updated_by: String,
}

impl Stack {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            1 => "Swarm",
            2 => "Compose",
            3 => "Kubernetes",
            _ => "Other",
        }
    }
}

/// What one read of a Portainer gives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub version: String,
    pub environments: Vec<Environment>,
    pub stacks: Vec<Stack>,
}

/// Why a read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// No token saved for this Portainer.
    NoToken,
    /// The SSH tunnel did not start, or nothing answers on the port.
    Unreachable(String),
    /// The certificate is not the pinned one.
    CertChanged {
        pinned: String,
        seen: String,
    },
    /// 401 / 403: the token is wrong, expired or revoked.
    Rejected,
    /// Older than [`MIN_VERSION`].
    TooOld(String),
    Other(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::NoToken => write!(f, "No API token saved: Edit and paste one"),
            ReadError::Unreachable(e) => write!(f, "Unreachable: {e}"),
            ReadError::CertChanged { pinned, seen } => write!(
                f,
                "The HTTPS certificate changed (was {}, now {}): accept it only if you know why",
                short_fp(pinned),
                short_fp(seen)
            ),
            ReadError::Rejected => write!(
                f,
                "Token rejected (401): create a new access token in Portainer (My account ▸ Access tokens)"
            ),
            ReadError::TooOld(v) => write!(
                f,
                "Portainer {v} is too old: access tokens need {MIN_VERSION} or later"
            ),
            ReadError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// `AB:CD:EF:…` cut to the first 8 bytes for messages.
pub fn short_fp(fp: &str) -> String {
    let parts: Vec<&str> = fp.split(':').collect();
    if parts.len() > 8 {
        format!("{}…", parts[..8].join(":"))
    } else {
        fp.to_string()
    }
}

/// A read: the data, and the certificate seen (to pin on first use).
pub type ReadResult = Result<(Snapshot, Option<String>), ReadError>;

/// Read a Portainer through its host (only `GET`s). `accept` is a new
/// certificate fingerprint the user accepted, used instead of the pinned one.
pub fn read(
    settings: &SshSettings,
    host: &HostEntry,
    p: &Portainer,
    token: &Secret,
    accept: Option<&str>,
) -> ReadResult {
    if token.is_empty() {
        return Err(ReadError::NoToken);
    }
    let tunnel = Tunnel::open(settings, host, "127.0.0.1", p.port)
        .map_err(|e| ReadError::Unreachable(e.to_string()))?;
    let pin = accept
        .map(str::to_string)
        .or_else(|| (!p.fingerprint.is_empty()).then(|| p.fingerprint.clone()));
    let client = Client {
        port: tunnel.local_port,
        https: p.https,
        pin,
        seen: Arc::new(Mutex::new(None)),
    };
    let result = read_with(&client, token);
    drop(tunnel);
    let seen = client.seen.lock().ok().and_then(|s| s.clone());
    result.map(|snap| (snap, seen))
}

fn read_with(c: &Client, token: &Secret) -> Result<Snapshot, ReadError> {
    let status = c.get("/api/status", None)?;
    let version = parse_status(&status)?;
    if !version_ok(&version) {
        return Err(ReadError::TooOld(version));
    }
    let environments = parse_endpoints(&c.get("/api/endpoints", Some(token))?)?;
    let stacks = parse_stacks(&c.get("/api/stacks", Some(token))?)?;
    Ok(Snapshot {
        version,
        environments,
        stacks,
    })
}

/// `true` for [`MIN_VERSION`] or later (and versions that do not parse:
/// the API answers decide then).
pub fn version_ok(v: &str) -> bool {
    let clean = v.trim().trim_start_matches(['v', 'V']);
    let min = semver::Version::new(2, 11, 0);
    let mut parts = clean.split(['-', '+']).next().unwrap_or("").split('.');
    let mut num = || parts.next().and_then(|x| x.parse::<u64>().ok());
    match (num(), num(), num()) {
        (Some(a), b, c) => semver::Version::new(a, b.unwrap_or(0), c.unwrap_or(0)) >= min,
        _ => true,
    }
}

struct Client {
    port: u16,
    https: bool,
    pin: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl Client {
    /// One `GET`; the body of a 2xx answer.
    fn get(&self, path: &str, token: Option<&Secret>) -> Result<Vec<u8>, ReadError> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let tcp = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| {
            ReadError::Unreachable(format!("cannot connect through the tunnel: {e}"))
        })?;
        let _ = tcp.set_read_timeout(Some(TIMEOUT));
        let _ = tcp.set_write_timeout(Some(TIMEOUT));
        let mut req = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: application/json\r\nUser-Agent: Telepad\r\nConnection: close\r\n",
            self.port
        );
        if let Some(t) = token {
            req.push_str("X-API-Key: ");
            req.push_str(t.expose());
            req.push_str("\r\n");
        }
        req.push_str("\r\n");
        let raw = if self.https {
            self.tls(tcp, req.as_bytes())?
        } else {
            exchange(tcp, req.as_bytes())
                .map_err(|e| ReadError::Unreachable(format!("no answer: {e}")))?
        };
        let (code, body) = parse_response(&raw).map_err(ReadError::Other)?;
        match code {
            200..=299 => Ok(body),
            401 | 403 => Err(ReadError::Rejected),
            _ => Err(ReadError::Other(format!(
                "Portainer answered {code} for {path}{}",
                api_message(&body).map_or(String::new(), |m| format!(": {m}"))
            ))),
        }
    }

    fn tls(&self, tcp: TcpStream, req: &[u8]) -> Result<Vec<u8>, ReadError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Pin {
            want: self.pin.clone(),
            seen: self.seen.clone(),
            algs: provider.signature_verification_algorithms,
        };
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| ReadError::Other(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        let name = rustls::pki_types::ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into());
        let conn = rustls::ClientConnection::new(Arc::new(config), name)
            .map_err(|e| ReadError::Other(e.to_string()))?;
        let stream = rustls::StreamOwned::new(conn, tcp);
        exchange(stream, req).map_err(|e| {
            let seen = self.seen.lock().ok().and_then(|s| s.clone());
            match (&self.pin, seen) {
                (Some(pinned), Some(seen)) if !pinned.eq_ignore_ascii_case(&seen) => {
                    ReadError::CertChanged {
                        pinned: pinned.clone(),
                        seen,
                    }
                }
                _ => ReadError::Unreachable(format!(
                    "HTTPS failed ({e}); is the port an HTTP one? Edit and switch HTTPS off"
                )),
            }
        })
    }
}

/// Send `req`, read until the server closes (a TLS close without
/// close_notify still counts when something came).
fn exchange(mut s: impl Read + Write, req: &[u8]) -> std::io::Result<Vec<u8>> {
    s.write_all(req)?;
    s.flush()?;
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > MAX_BODY {
                    return Err(std::io::Error::other("answer too large"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !out.is_empty() => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Status code and body of an HTTP/1.1 answer (Content-Length, chunked or
/// until close).
pub fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no HTTP answer (is this Portainer's port?)")?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("");
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .filter(|_| status.starts_with("HTTP/"))
        .ok_or_else(|| {
            format!(
                "not an HTTP answer: {}",
                status.chars().take(60).collect::<String>()
            )
        })?;
    let mut chunked = false;
    let mut length: Option<usize> = None;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            } else if k == "content-length" {
                length = v.parse().ok();
            }
        }
    }
    let rest = &raw[end + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else if let Some(n) = length {
        rest.get(..n).ok_or("the answer was cut short")?.to_vec()
    } else {
        rest.to_vec()
    };
    Ok((code, body))
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = b
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("bad chunked answer")?;
        let size_text = String::from_utf8_lossy(&b[..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "bad chunk size")?;
        b = &b[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        out.extend_from_slice(b.get(..size).ok_or("the answer was cut short")?);
        b = b.get(size + 2..).unwrap_or(&[]);
    }
}

/// Portainer's error text (`{"message": …, "details": …}`).
fn api_message(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let m = v.get("message").and_then(Value::as_str)?;
    let d = v.get("details").and_then(Value::as_str).unwrap_or("");
    Some(if d.is_empty() || d == m {
        m.to_string()
    } else {
        format!("{m} ({d})")
    })
}

fn json(body: &[u8]) -> Result<Value, ReadError> {
    serde_json::from_slice(body).map_err(|e| ReadError::Other(format!("not Portainer's JSON: {e}")))
}

/// `/api/status` (or `/api/system/status`): the version.
pub fn parse_status(body: &[u8]) -> Result<String, ReadError> {
    let v = json(body)?;
    v.get("Version")
        .or_else(|| v.get("version"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ReadError::Other("this does not look like Portainer (no Version in /api/status)".into())
        })
}

fn int(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn count(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(Value::as_u64)
}

/// `/api/endpoints`: the environments, counts from their latest snapshot.
pub fn parse_endpoints(body: &[u8]) -> Result<Vec<Environment>, ReadError> {
    let v = json(body)?;
    let list = v
        .as_array()
        .or_else(|| v.get("value").and_then(Value::as_array))
        .ok_or_else(|| ReadError::Other("no list of environments in /api/endpoints".into()))?;
    Ok(list
        .iter()
        .map(|e| {
            let snap = e
                .get("Snapshots")
                .and_then(Value::as_array)
                .and_then(|s| s.last());
            let (running, total, stacks) = match snap {
                Some(s) => {
                    let run = count(s, "RunningContainerCount");
                    let stop = count(s, "StoppedContainerCount");
                    (
                        run,
                        run.zip(stop).map(|(a, b)| a + b),
                        count(s, "StackCount"),
                    )
                }
                None => (None, None, None),
            };
            Environment {
                id: int(e, "Id").unwrap_or_default(),
                name: text(e, "Name"),
                kind: int(e, "Type").unwrap_or_default(),
                url: text(e, "URL"),
                up: int(e, "Status").map(|s| s == 1),
                stacks,
                running,
                total,
            }
        })
        .collect())
}

/// `/api/stacks`: every stack (no environment variables).
pub fn parse_stacks(body: &[u8]) -> Result<Vec<Stack>, ReadError> {
    let v = json(body)?;
    let list = v
        .as_array()
        .ok_or_else(|| ReadError::Other("no list of stacks in /api/stacks".into()))?;
    let time = |s: &Value, k: &str| int(s, k).filter(|t| *t > 0);
    Ok(list
        .iter()
        .map(|s| Stack {
            id: int(s, "Id").unwrap_or_default(),
            name: text(s, "Name"),
            kind: int(s, "Type").unwrap_or_default(),
            env: int(s, "EndpointId").unwrap_or_default(),
            active: int(s, "Status").map(|x| x == 1),
            created: time(s, "CreationDate"),
            updated: time(s, "UpdateDate"),
            created_by: text(s, "CreatedBy"),
            updated_by: text(s, "UpdatedBy"),
        })
        .collect())
}

/// SHA-256 of a DER certificate as `AB:CD:…`.
pub fn fingerprint(der: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Trust on first use: any certificate when nothing is pinned, else only
/// the pinned one. The handshake signature is still checked against the
/// certificate's key.
#[derive(Debug)]
struct Pin {
    want: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    algs: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl rustls::client::danger::ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity.as_ref());
        if let Ok(mut s) = self.seen.lock() {
            *s = Some(fp.clone());
        }
        match &self.want {
            Some(w) if !w.eq_ignore_ascii_case(&fp) => Err(rustls::Error::General(
                "the certificate is not the pinned one".into(),
            )),
            _ => Ok(rustls::client::danger::ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// The Telepad host an environment runs on (#171): the Portainer's own
/// host for the local socket, else the host whose address or alias is the
/// URL's address, or whose alias is the environment's name (Edge).
pub fn env_host<'a>(
    env: &Environment,
    portainer_alias: &str,
    hosts: &'a [HostEntry],
) -> Option<&'a HostEntry> {
    if env.local() {
        return hosts.iter().find(|h| h.alias == portainer_alias);
    }
    let addr = env.address();
    if !addr.is_empty()
        && let Some(h) = hosts
            .iter()
            .find(|h| h.hostname.eq_ignore_ascii_case(addr) || h.alias.eq_ignore_ascii_case(addr))
    {
        return Some(h);
    }
    hosts
        .iter()
        .find(|h| !env.name.is_empty() && h.alias.eq_ignore_ascii_case(&env.name))
}

/// The page of an environment in Portainer's web UI.
pub fn env_url(base: &str, env: &Environment) -> String {
    let area = if env.kubernetes() {
        "kubernetes"
    } else {
        "docker"
    };
    format!("{base}#!/{}/{area}/dashboard", env.id)
}

/// The page of a stack in Portainer's web UI.
pub fn stack_url(base: &str, s: &Stack) -> String {
    if s.kind == 3 {
        format!("{base}#!/{}/kubernetes/applications", s.env)
    } else {
        format!(
            "{base}#!/{}/docker/stacks/{}?id={}&type={}&regular=true",
            s.env, s.name, s.id, s.kind
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/portainer")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn parses_status_endpoints_and_stacks() {
        assert_eq!(parse_status(&fixture("status.json")).unwrap(), "2.21.4");
        let envs = parse_endpoints(&fixture("endpoints.json")).unwrap();
        assert_eq!(envs.len(), 4);
        let local = &envs[0];
        assert_eq!(
            (local.id, local.name.as_str(), local.kind_name(), local.up),
            (1, "local", "Docker", Some(true))
        );
        assert_eq!(
            (local.running, local.total, local.stacks),
            (Some(7), Some(9), Some(3))
        );
        assert!(local.local() && local.address().is_empty());
        let agent = &envs[1];
        assert_eq!(agent.kind_name(), "Docker (agent)");
        assert_eq!(agent.address(), "192.0.2.21");
        assert_eq!(agent.up, Some(false));
        assert_eq!(envs[2].kind_name(), "Kubernetes (agent)");
        assert!(envs[2].kubernetes());
        // Edge: no snapshot yet, no URL.
        assert_eq!((envs[3].running, envs[3].stacks), (None, None));
        let stacks = parse_stacks(&fixture("stacks.json")).unwrap();
        assert_eq!(stacks.len(), 3);
        let s = &stacks[0];
        assert_eq!(
            (s.name.as_str(), s.kind_name(), s.env, s.active),
            ("shop", "Compose", 1, Some(true))
        );
        assert_eq!((s.created, s.updated), (Some(1_763_000_000), None));
        assert_eq!(s.created_by, "admin");
        assert_eq!(stacks[1].active, Some(false));
        assert_eq!(stacks[2].kind_name(), "Swarm");
        // Environment variables are not kept anywhere.
        assert!(!format!("{stacks:?}").contains("hunter2"));
    }

    #[test]
    fn versions_from_2_11_take_a_token() {
        assert!(version_ok("2.11.0"));
        assert!(version_ok("2.21.4"));
        assert!(version_ok("v2.19.4-ee"));
        assert!(version_ok("3.0"));
        assert!(!version_ok("2.10.9"));
        assert!(!version_ok("1.24.2"));
        assert!(version_ok("unknown"));
    }

    #[test]
    fn http_answers_with_length_chunks_or_until_close() {
        let r = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX: y\r\n\r\nhello-extra";
        assert_eq!(parse_response(r).unwrap(), (200, b"hello".to_vec()));
        let r = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n6;x=1\r\npedia \r\n0\r\n\r\n";
        assert_eq!(parse_response(r).unwrap(), (200, b"Wikipedia ".to_vec()));
        let r = b"HTTP/1.0 401 Unauthorized\r\n\r\n{\"message\":\"Unauthorized\"}";
        let (code, body) = parse_response(r).unwrap();
        assert_eq!(code, 401);
        assert_eq!(api_message(&body).as_deref(), Some("Unauthorized"));
        assert!(parse_response(b"SSH-2.0-OpenSSH_9.6\r\n\r\n").is_err());
        assert!(parse_response(b"garbage").is_err());
        let cut = b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nshort";
        assert!(parse_response(cut).is_err());
    }

    #[test]
    fn names_tokens_urls_and_hosts() {
        assert!(check_name("Main").is_ok());
        assert!(check_name("  ").is_err());
        assert!(check_name("a\nb").is_err());
        assert!(check_name(&"x".repeat(65)).is_err());
        assert_eq!(token_key("Main"), "portainer:main");
        let host = HostEntry {
            hostname: "192.0.2.20".into(),
            ..HostEntry::new("docker1")
        };
        let p = Portainer {
            name: "Main".into(),
            alias: "docker1".into(),
            port: 9443,
            https: true,
            fingerprint: String::new(),
        };
        let base = p.web_url(&host);
        assert_eq!(base, "https://192.0.2.20:9443/");
        let envs = parse_endpoints(&fixture("endpoints.json")).unwrap();
        assert_eq!(
            env_url(&base, &envs[0]),
            "https://192.0.2.20:9443/#!/1/docker/dashboard"
        );
        assert_eq!(
            env_url(&base, &envs[2]),
            "https://192.0.2.20:9443/#!/3/kubernetes/dashboard"
        );
        let stacks = parse_stacks(&fixture("stacks.json")).unwrap();
        assert_eq!(
            stack_url(&base, &stacks[0]),
            "https://192.0.2.20:9443/#!/1/docker/stacks/shop?id=11&type=2&regular=true"
        );
        let other = HostEntry {
            hostname: "192.0.2.21".into(),
            ..HostEntry::new("docker2")
        };
        let edge = HostEntry::new("edge-box");
        let hosts = vec![host.clone(), other, edge];
        let alias = |e: &Environment| env_host(e, "docker1", &hosts).map(|h| h.alias.clone());
        assert_eq!(alias(&envs[0]).as_deref(), Some("docker1"), "local socket");
        assert_eq!(alias(&envs[1]).as_deref(), Some("docker2"), "agent address");
        assert_eq!(alias(&envs[2]), None);
        assert_eq!(alias(&envs[3]).as_deref(), Some("edge-box"), "Edge by name");
        let fp = fingerprint(b"not really a certificate");
        assert_eq!(fp.len(), 32 * 3 - 1);
        assert!(short_fp(&fp).ends_with('…'));
    }
}
