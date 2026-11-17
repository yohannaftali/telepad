//! Self-update from GitHub Releases (default `yohannaftali/telepad`).
//!
//! The latest release is read from the public GitHub API over HTTPS (no token).
//! A release carries `telepad-<version>-<os>-<arch>.zip` and its `.sha256`.
//! Everything here blocks, so it runs on background threads.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_REPO: &str = "yohannaftali/telepad";
/// How often the GUI checks again while it is running.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
const MAX_DOWNLOAD: u64 = 300 * 1024 * 1024;

/// Stored in the `settings` table (`update_*` keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateConfig {
    pub enabled: bool,
    /// GitHub `owner/name` that publishes the releases.
    pub repo: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            repo: DEFAULT_REPO.into(),
        }
    }
}

impl UpdateConfig {
    /// The repo becomes part of a URL, so only GitHub's own name characters pass.
    pub fn validate(&self) -> Result<(), String> {
        let part = |s: &str| {
            !s.is_empty()
                && !s.starts_with('.')
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        };
        match self.repo.split_once('/') {
            Some((owner, name)) if part(owner) && part(name) => Ok(()),
            _ => Err("Repository must be a GitHub owner/name, e.g. yohannaftali/telepad".into()),
        }
    }

    pub fn api_url(&self) -> String {
        format!("https://api.github.com/repos/{}/releases/latest", self.repo)
    }

    /// Release downloads must come from this repo on github.com.
    fn download_prefix(&self) -> String {
        format!("https://github.com/{}/releases/download/", self.repo)
    }
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// URL of `<name>.sha256` in the same release.
    pub sha256_url: String,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub version: String,
    pub notes: Option<String>,
    pub page_url: String,
    /// The download for this OS/arch, if the release has one (with its .sha256).
    pub asset: Option<Asset>,
    pub newer: bool,
}

pub fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn asset_name(version: &str) -> String {
    format!("telepad-{version}-{}.zip", platform_key())
}

/// Parse GitHub's `releases/latest` JSON into what the app needs.
pub fn parse_release(text: &str, cfg: &UpdateConfig) -> Result<Check, String> {
    let r: GhRelease =
        serde_json::from_str(text).map_err(|e| format!("unexpected GitHub answer: {e}"))?;
    let version = r.tag_name.trim().trim_start_matches('v').to_string();
    semver::Version::parse(&version)
        .map_err(|e| format!("release tag '{}' is not a version: {e}", r.tag_name))?;
    let prefix = cfg.download_prefix();
    let find = |name: &str| {
        r.assets
            .iter()
            .find(|a| a.name == name && a.browser_download_url.starts_with(&prefix))
    };
    let zip_name = asset_name(&version);
    let asset = match (find(&zip_name), find(&format!("{zip_name}.sha256"))) {
        (Some(z), Some(h)) => Some(Asset {
            name: z.name.clone(),
            url: z.browser_download_url.clone(),
            size: z.size,
            sha256_url: h.browser_download_url.clone(),
        }),
        _ => None,
    };
    let page_url = if r.html_url.starts_with("https://github.com/") {
        r.html_url
    } else {
        String::new()
    };
    Ok(Check {
        newer: is_newer(&version, CURRENT_VERSION),
        version,
        notes: r
            .body
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty()),
        page_url,
        asset,
    })
}

/// First token of a `sha256sum`-style line, if it is a SHA-256.
pub fn parse_sha256_file(text: &str) -> Result<String, String> {
    let h = text
        .trim_start_matches('\u{feff}')
        .split_whitespace()
        .next()
        .unwrap_or_default();
    if h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(h.to_ascii_lowercase())
    } else {
        Err("the release's .sha256 file is not valid".into())
    }
}

/// True when `remote` is strictly newer than `current` (semver order).
pub fn is_newer(remote: &str, current: &str) -> bool {
    match (
        semver::Version::parse(remote),
        semver::Version::parse(current),
    ) {
        (Ok(r), Ok(c)) => r > c,
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct Updater {
    pub config: UpdateConfig,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(format!("telepad/{CURRENT_VERSION}"))
        .build()
        .into()
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(404) => "no release published yet".into(),
        ureq::Error::StatusCode(403 | 429) => "GitHub rate limit reached; try again later".into(),
        ureq::Error::StatusCode(c) => format!("GitHub answered HTTP {c}"),
        other => format!("cannot reach GitHub: {other}"),
    }
}

impl Updater {
    pub fn new(config: UpdateConfig) -> Self {
        Self { config }
    }

    pub fn check(&self) -> Result<Check, String> {
        self.config.validate()?;
        let mut resp = agent(Duration::from_secs(30))
            .get(&self.config.api_url())
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(http_error)?;
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("reading GitHub answer: {e}"))?;
        parse_release(&text, &self.config)
    }
}

/// Download, verify and install `check`'s release. The running app keeps
/// running the old code until it is restarted.
pub fn install(check: &Check) -> Result<String, String> {
    if !check.newer {
        return Err(format!(
            "{} is not newer than {CURRENT_VERSION}",
            check.version
        ));
    }
    let asset = check.asset.as_ref().ok_or_else(|| {
        format!(
            "release {} has no {} download",
            check.version,
            asset_name(&check.version)
        )
    })?;
    if asset.size > MAX_DOWNLOAD {
        return Err(format!(
            "{} is too large ({} bytes)",
            asset.name, asset.size
        ));
    }
    let a = agent(Duration::from_secs(600));
    let expected = a
        .get(&asset.sha256_url)
        .call()
        .map_err(http_error)?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("downloading {}.sha256: {e}", asset.name))?;
    let expected = parse_sha256_file(&expected)?;
    let data = a
        .get(&asset.url)
        .call()
        .map_err(http_error)?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()
        .map_err(|e| format!("downloading {}: {e}", asset.name))?;
    verify_sha256(&data, &expected)?;
    let dir = work_dir()?;
    let result = extract_executables(&data, &dir)
        .and_then(|new| install_executables(&new))
        .map(|()| check.version.clone());
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn work_dir() -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("telepad-update-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn verify_sha256(data: &[u8], expected: &str) -> Result<(), String> {
    let got = sha256_hex(data);
    if got.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "download corrupted: SHA-256 {got} does not match {expected}; nothing was changed"
        ))
    }
}

pub fn monitor_exe() -> String {
    format!("telepad{}", std::env::consts::EXE_SUFFIX)
}

pub fn check_exe() -> String {
    format!("telepad-check{}", std::env::consts::EXE_SUFFIX)
}

/// The two executables from a release zip (any folder inside it), written into
/// `dir/new/`. Only these exact file names are taken, so no zip path is ever
/// used to build an output path.
pub struct NewExecutables {
    pub monitor: PathBuf,
    pub check: PathBuf,
}

pub fn extract_executables(zip_data: &[u8], dir: &Path) -> Result<NewExecutables, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_data))
        .map_err(|e| format!("release zip: {e}"))?;
    let out = dir.join("new");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let (monitor_name, check_name) = (monitor_exe(), check_exe());
    let (mut monitor, mut check) = (None, None);
    for i in 0..archive.len() {
        let mut f = archive
            .by_index(i)
            .map_err(|e| format!("release zip: {e}"))?;
        if f.is_dir() {
            continue;
        }
        let full = f
            .name()
            .map_err(|e| format!("release zip: {e}"))?
            .to_string();
        let base = full.rsplit(['/', '\\']).next().unwrap_or_default();
        let (name, slot) = if base.eq_ignore_ascii_case(&monitor_name) {
            (&monitor_name, &mut monitor)
        } else if base.eq_ignore_ascii_case(&check_name) {
            (&check_name, &mut check)
        } else {
            continue;
        };
        let path = out.join(name);
        let mut w = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut w).map_err(|e| format!("release zip: {e}"))?;
        *slot = Some(path);
    }
    match (monitor, check) {
        (Some(monitor), Some(check)) => Ok(NewExecutables { monitor, check }),
        _ => Err(format!(
            "release zip does not contain {monitor_name} and {check_name}"
        )),
    }
}

/// Replace `telepad-check` next to the running exe, then the running exe
/// itself. `telepad.db`, `config` and `.env` are never touched.
fn install_executables(new: &NewExecutables) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the running exe: {e}"))?;
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let (monitor_name, check_name) = (monitor_exe(), check_exe());
    if !name.eq_ignore_ascii_case(&monitor_name) {
        return Err(format!(
            "self-update only replaces {monitor_name} (running: {name})"
        ));
    }
    let dir = exe
        .parent()
        .ok_or("cannot find the program folder")?
        .to_path_buf();
    let check = dir.join(&check_name);
    let staged = dir.join(format!("{check_name}.new"));
    std::fs::copy(&new.check, &staged)
        .map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
    if let Err(e) = std::fs::rename(&staged, &check) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("cannot replace {check_name} (is it running?): {e}"));
    }
    self_replace::self_replace(&new.monitor)
        .map_err(|e| format!("cannot replace {monitor_name}: {e}"))
}

/// Start the (new) exe with the same arguments; the caller then exits.
pub fn relaunch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(exe)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot restart: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SHA: &str = "aa3f8f4c1f0f6ff08ab7ec0fd33a41bfa0aaf0b5ba0b26d1e2e7c8b4f5f56b0e";

    fn release_json(tag: &str, base: &str) -> String {
        let zip = asset_name(tag.trim_start_matches('v'));
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/yohannaftali/telepad/releases/tag/{tag}",
               "body":"Fixes\r\n","draft":false,
               "assets":[{{"name":"{zip}","browser_download_url":"{base}/{tag}/{zip}","size":6000000}},
                         {{"name":"{zip}.sha256","browser_download_url":"{base}/{tag}/{zip}.sha256","size":90}},
                         {{"name":"other.txt","browser_download_url":"{base}/{tag}/other.txt","size":1}}]}}"#
        )
    }

    const BASE: &str = "https://github.com/yohannaftali/telepad/releases/download";

    #[test]
    fn parses_github_release_and_picks_this_platform() {
        let cfg = UpdateConfig::default();
        let c = parse_release(&release_json("v99.0.0", BASE), &cfg).unwrap();
        assert_eq!(c.version, "99.0.0");
        assert!(c.newer);
        assert_eq!(c.notes.as_deref(), Some("Fixes"));
        let a = c.asset.unwrap();
        assert_eq!(a.name, asset_name("99.0.0"));
        assert!(a.url.ends_with(&a.name));
        assert!(a.sha256_url.ends_with(".sha256"));

        let same =
            parse_release(&release_json(&format!("v{CURRENT_VERSION}"), BASE), &cfg).unwrap();
        assert!(!same.newer);
        assert!(parse_release(&release_json("latest", BASE), &cfg).is_err());
        assert!(parse_release("<html>", &cfg).is_err());
    }

    #[test]
    fn ignores_downloads_from_other_hosts_or_repos() {
        let cfg = UpdateConfig::default();
        for base in [
            "https://evil.example/yohannaftali/telepad/releases/download",
            "https://github.com/someone/else/releases/download",
            "http://github.com/yohannaftali/telepad/releases/download",
        ] {
            let c = parse_release(&release_json("v99.0.0", base), &cfg).unwrap();
            assert!(c.asset.is_none(), "{base}");
        }
    }

    #[test]
    fn newer_uses_semver_order() {
        assert!(is_newer("0.10.0", "0.9.1"));
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("0.4.0-rc.1", "0.4.0"));
        assert!(!is_newer("garbage", "0.3.0"));
    }

    #[test]
    fn repo_validation() {
        assert!(UpdateConfig::default().validate().is_ok());
        for bad in [
            "", "telepad", "a/b/c", "../x", "a/b?x=1", "a b/c", "/telepad", "a/.git",
        ] {
            let c = UpdateConfig {
                repo: bad.into(),
                ..UpdateConfig::default()
            };
            assert!(c.validate().is_err(), "{bad}");
        }
        assert_eq!(
            UpdateConfig::default().api_url(),
            "https://api.github.com/repos/yohannaftali/telepad/releases/latest"
        );
    }

    #[test]
    fn sha256_file_formats() {
        assert_eq!(
            parse_sha256_file(&format!("{SHA}  telepad.zip\n")).unwrap(),
            SHA
        );
        assert_eq!(
            parse_sha256_file(&format!("\u{feff}{}\r\n", SHA.to_uppercase())).unwrap(),
            SHA
        );
        assert!(parse_sha256_file("abc  x.zip").is_err());
        assert!(parse_sha256_file("").is_err());
    }

    fn zip_with(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn extracts_only_the_two_executables_and_checks_hash() {
        let data = zip_with(&[
            (
                &format!("telepad-0.4.0-windows-x86_64/{}", monitor_exe()),
                b"GUI",
            ),
            (
                &format!("telepad-0.4.0-windows-x86_64/{}", check_exe()),
                b"CLI",
            ),
            ("telepad-0.4.0-windows-x86_64/README.txt", b"readme"),
            ("../../evil.txt", b"x"),
        ]);
        let dir = work_dir().unwrap();
        let new = extract_executables(&data, &dir).unwrap();
        assert_eq!(std::fs::read(&new.monitor).unwrap(), b"GUI");
        assert_eq!(std::fs::read(&new.check).unwrap(), b"CLI");
        let names: Vec<_> = std::fs::read_dir(dir.join("new"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2);
        std::fs::remove_dir_all(&dir).ok();

        assert!(verify_sha256(&data, &sha256_hex(&data)).is_ok());
        assert!(verify_sha256(&data, SHA).is_err());
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let missing = zip_with(&[(&monitor_exe(), b"GUI")]);
        let dir = work_dir().unwrap();
        assert!(extract_executables(&missing, &dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
