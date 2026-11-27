//! Windows security, Windows Update and installed programs of My Computer
//! (#211). Read-only and without administrator rights: WMI (Microsoft
//! Defender, Windows Security Center), `reg query` (firewall profiles,
//! SmartScreen, Uninstall keys) and Windows Update's local cache (offline
//! search: never scans online, downloads or installs).
//!
//! The parsers and mapping below are plain functions, tested on every
//! platform; only the readers at the bottom are Windows-only.

use crate::model::{Antivirus, App, FirewallProfile};

/// Microsoft Defender's own status (`MSFT_MpComputerStatus`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MpStatus {
    pub antivirus_enabled: Option<bool>,
    pub real_time: Option<bool>,
    pub version: Option<String>,
    /// CIM datetime, e.g. `20261127041753.000000+000`.
    pub updated: Option<String>,
    pub age_days: Option<u32>,
}

/// A product registered with Windows Security Center (`root\SecurityCenter2`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScProduct {
    pub name: String,
    pub state: u32,
}

impl ScProduct {
    /// `productState` bit 0x1000: the product is on.
    pub fn on(&self) -> bool {
        self.state & 0x1000 != 0
    }

    /// `productState` bit 0x10: definitions out of date.
    pub fn up_to_date(&self) -> bool {
        self.state & 0x10 == 0
    }
}

fn is_defender(name: &str) -> bool {
    name.to_ascii_lowercase().contains("defender")
}

/// The antivirus list: Security Center's products, Defender completed with
/// its own status; Defender alone when Security Center is missing (Server).
pub fn antivirus(mp: Option<&MpStatus>, sc: &[ScProduct]) -> Vec<Antivirus> {
    let from_mp = |name: String, on: bool, up: Option<bool>, mp: &MpStatus| Antivirus {
        name,
        on,
        up_to_date: up.or_else(|| {
            mp.age_days
                .map(|d| d <= crate::model::DEFINITIONS_MAX_AGE_DAYS)
        }),
        version: mp.version.clone(),
        updated: mp.updated.as_deref().and_then(cim_datetime),
        age_days: mp.age_days,
    };
    let mut out: Vec<Antivirus> = sc
        .iter()
        .map(|p| match mp {
            Some(mp) if is_defender(&p.name) => from_mp(
                p.name.clone(),
                mp.real_time.unwrap_or(p.on()) && p.on(),
                Some(p.up_to_date()),
                mp,
            ),
            _ => Antivirus {
                name: p.name.clone(),
                on: p.on(),
                up_to_date: Some(p.up_to_date()),
                ..Default::default()
            },
        })
        .collect();
    if let Some(mp) = mp
        && !sc.iter().any(|p| is_defender(&p.name))
        && mp.antivirus_enabled.is_some()
    {
        let on = mp.antivirus_enabled == Some(true) && mp.real_time == Some(true);
        out.push(from_mp("Microsoft Defender".into(), on, None, mp));
    }
    out
}

/// A CIM datetime (`yyyymmddHHMMSS.ffffff±mmm`) as `YYYY-MM-DD HH:MM UTC`.
pub fn cim_datetime(s: &str) -> Option<String> {
    let s = s.trim();
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let t = s.get(r)?;
        t.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| t.parse().ok())?
    };
    let (y, mo, d) = (digits(0..4)?, digits(4..6)?, digits(6..8)?);
    let (h, mi, sec) = (digits(8..10)?, digits(10..12)?, digits(12..14)?);
    let days = crate::eol::days_from_iso(&format!("{y:04}-{mo:02}-{d:02}"))?;
    // Offset in minutes east of UTC after the fraction, e.g. `+420`.
    let offset = s
        .get(21..)
        .and_then(|o| {
            let (sign, n) = o.split_at_checked(1)?;
            let n: i64 = n.parse().ok()?;
            match sign {
                "+" => Some(n),
                "-" => Some(-n),
                _ => None,
            }
        })
        .unwrap_or(0);
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset * 60;
    let t = crate::history::format_utc(secs);
    Some(format!("{} UTC", t.get(..16)?))
}

/// One key of `reg query` output: its path and `name → (type, data)`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RegKey {
    pub path: String,
    pub values: Vec<(String, String, String)>,
}

impl RegKey {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, _, v)| v.as_str())
    }

    /// A `REG_DWORD` (`0x1`) or decimal value.
    pub fn dword(&self, name: &str) -> Option<u64> {
        let v = self.get(name)?.trim();
        match v.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).ok(),
            None => v.parse().ok(),
        }
    }
}

/// Parse `reg query <key> [/s]`: keys start at column 0, values are indented
/// `name    TYPE    data` (four spaces between fields).
pub fn parse_reg(out: &str) -> Vec<RegKey> {
    let mut keys: Vec<RegKey> = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("HKEY_") {
            keys.push(RegKey {
                path: line.trim().to_string(),
                values: Vec::new(),
            });
            continue;
        }
        let Some(key) = keys.last_mut() else {
            continue;
        };
        let body = line.trim_start();
        let mut parts = body.splitn(3, "    ");
        let (Some(name), Some(kind)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !kind.starts_with("REG_") {
            continue;
        }
        let data = parts.next().unwrap_or("").to_string();
        let name = if name == "(Default)" { "" } else { name };
        key.values.push((name.to_string(), kind.to_string(), data));
    }
    keys
}

/// Firewall profiles from the registry: `(name, local, policy)` where the
/// policy value (Group Policy) wins over the local setting.
pub fn firewall_profiles(profiles: &[(&str, Option<u64>, Option<u64>)]) -> Vec<FirewallProfile> {
    profiles
        .iter()
        .filter_map(|&(name, local, policy)| {
            Some(FirewallProfile {
                name: name.to_string(),
                on: policy.or(local)? != 0,
            })
        })
        .collect()
}

/// SmartScreen for apps and files: the policy `EnableSmartScreen` (0 / 1)
/// wins; else the user setting `SmartScreenEnabled` ("Off" = off); Windows'
/// default is on.
pub fn smartscreen(policy: Option<u64>, setting: Option<&str>) -> bool {
    match policy {
        Some(v) => v != 0,
        None => !setting.is_some_and(|s| s.trim().eq_ignore_ascii_case("off")),
    }
}

/// Installed programs from the Uninstall keys: entries with a name, not
/// system components or updates; the same name and version shown once;
/// sorted by name.
pub fn apps(keys: &[RegKey]) -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    for k in keys {
        let Some(name) = k
            .get("DisplayName")
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        if k.dword("SystemComponent") == Some(1)
            || k.get("ParentKeyName").is_some_and(|v| !v.trim().is_empty())
            || k.get("ReleaseType").is_some_and(|v| {
                let v = v.to_ascii_lowercase();
                v.contains("update") || v.contains("hotfix")
            })
        {
            continue;
        }
        let version = k.get("DisplayVersion").unwrap_or("").trim().to_string();
        if out
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name) && a.version == version)
        {
            continue;
        }
        out.push(App {
            name: name.to_string(),
            version,
            publisher: k.get("Publisher").unwrap_or("").trim().to_string(),
            installed: k.get("InstallDate").and_then(install_date),
            size_kb: k.dword("EstimatedSize").filter(|&n| n > 0),
        });
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.version.cmp(&b.version))
    });
    out
}

/// `InstallDate` as `YYYY-MM-DD` (`yyyymmdd`; other forms are dropped).
pub fn install_date(v: &str) -> Option<String> {
    let v = v.trim();
    if v.len() != 8 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let iso = format!("{}-{}-{}", &v[..4], &v[4..6], &v[6..]);
    crate::eol::days_from_iso(&iso).map(|_| iso)
}

/// The Windows Update script's output: `PENDING=<n>` and `LAST=<date>`.
pub fn parse_update(out: &str) -> (Option<u32>, Option<String>) {
    let mut pending = None;
    let mut last = None;
    for l in out.lines() {
        let l = l.trim();
        if let Some(n) = l.strip_prefix("PENDING=") {
            pending = n.trim().parse().ok();
        } else if let Some(d) = l.strip_prefix("LAST=") {
            last = crate::eol::days_from_iso(d.trim()).map(|_| d.trim().to_string());
        }
    }
    (pending, last)
}

/// Windows Update, read from its local cache only (`Online = $false`): the
/// number of updates not installed and the date of the last successful
/// install in the history. Fixed text; nothing from a host goes into it.
pub const UPDATE_SCRIPT: &str = "\
$ErrorActionPreference = 'SilentlyContinue'
$s = New-Object -ComObject Microsoft.Update.Session
$q = $s.CreateUpdateSearcher()
$q.Online = $false
try { $r = $q.Search('IsInstalled=0 and IsHidden=0'); 'PENDING=' + $r.Updates.Count } catch {}
$n = $q.GetTotalHistoryCount()
if ($n -gt 0) {
  $h = $q.QueryHistory(0, [Math]::Min($n, 100)) | Where-Object { $_.Operation -eq 1 -and $_.ResultCode -eq 2 } | Sort-Object Date -Descending | Select-Object -First 1
  if ($h) { 'LAST=' + $h.Date.ToUniversalTime().ToString('yyyy-MM-dd') }
}
";

#[cfg(target_os = "windows")]
pub use win::{read_apps, read_security, read_update};

#[cfg(target_os = "windows")]
mod win {
    use serde::Deserialize;

    use super::{MpStatus, RegKey, ScProduct, parse_reg};
    use crate::model::{App, Security};

    type Run = fn(&str, &[&str]) -> Option<(bool, String)>;

    const FIREWALL: [(&str, &str); 3] = [
        ("Domain", "DomainProfile"),
        ("Private", "StandardProfile"),
        ("Public", "PublicProfile"),
    ];

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct WMp {
        antivirus_enabled: Option<bool>,
        real_time_protection_enabled: Option<bool>,
        antivirus_signature_version: Option<String>,
        antivirus_signature_last_updated: Option<String>,
        antivirus_signature_age: Option<u32>,
    }

    #[derive(Deserialize)]
    struct WProduct {
        #[serde(rename = "displayName")]
        name: Option<String>,
        #[serde(rename = "productState")]
        state: Option<u32>,
    }

    fn mp_status() -> Option<MpStatus> {
        let con =
            wmi::WMIConnection::with_namespace_path(r"ROOT\Microsoft\Windows\Defender").ok()?;
        let rows: Vec<WMp> = con
            .raw_query(
                "SELECT AntivirusEnabled, RealTimeProtectionEnabled, AntivirusSignatureVersion, \
                 AntivirusSignatureLastUpdated, AntivirusSignatureAge FROM MSFT_MpComputerStatus",
            )
            .ok()?;
        let r = rows.into_iter().next()?;
        Some(MpStatus {
            antivirus_enabled: r.antivirus_enabled,
            real_time: r.real_time_protection_enabled,
            version: r.antivirus_signature_version,
            updated: r.antivirus_signature_last_updated,
            age_days: r.antivirus_signature_age,
        })
    }

    /// Security Center's products of one class; empty on Windows Server
    /// (no `SecurityCenter2`) or when WMI fails.
    fn products(class: &str) -> Vec<ScProduct> {
        let Ok(con) = wmi::WMIConnection::with_namespace_path(r"ROOT\SecurityCenter2") else {
            return Vec::new();
        };
        con.raw_query::<WProduct>(format!("SELECT displayName, productState FROM {class}"))
            .map(|v| {
                v.into_iter()
                    .filter_map(|p| {
                        Some(ScProduct {
                            name: p.name?,
                            state: p.state?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One key without subkeys; `None` when it does not exist.
    fn reg(run: Run, key: &str) -> Option<RegKey> {
        match run("reg", &["query", key]) {
            Some((true, out)) => parse_reg(&out).into_iter().next(),
            _ => None,
        }
    }

    pub fn read_security(run: Run) -> Security {
        let mp = mp_status();
        let antivirus = super::antivirus(mp.as_ref(), &products("AntiVirusProduct"));
        let policy_root = r"HKLM\SOFTWARE\Policies\Microsoft\WindowsFirewall";
        let local_root =
            r"HKLM\SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy";
        let profiles: Vec<(&str, Option<u64>, Option<u64>)> = FIREWALL
            .iter()
            .map(|&(name, key)| {
                let local = reg(run, &format!(r"{local_root}\{key}"))
                    .and_then(|k| k.dword("EnableFirewall"));
                let policy = reg(run, &format!(r"{policy_root}\{key}"))
                    .and_then(|k| k.dword("EnableFirewall"));
                (name, local, policy)
            })
            .collect();
        let firewall_products = products("FirewallProduct")
            .into_iter()
            .filter(ScProduct::on)
            .map(|p| p.name)
            .collect();
        let policy = reg(run, r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System")
            .and_then(|k| k.dword("EnableSmartScreen"));
        let setting = reg(
            run,
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer",
        )
        .and_then(|k| k.get("SmartScreenEnabled").map(str::to_string));
        Security {
            antivirus,
            firewall_profiles: super::firewall_profiles(&profiles),
            firewall_products,
            app_control: Some(super::smartscreen(policy, setting.as_deref())),
        }
    }

    /// Installed programs (64-bit, 32-bit and this user's Uninstall keys).
    pub fn read_apps(run: Run) -> Vec<App> {
        const KEYS: [&str; 3] = [
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ];
        let keys: Vec<RegKey> = KEYS
            .iter()
            .filter_map(|k| match run("reg", &["query", k, "/s"]) {
                Some((true, out)) => Some(parse_reg(&out)),
                _ => None,
            })
            .flatten()
            .collect();
        super::apps(&keys)
    }

    /// Pending updates and the last install date from Windows Update's cache.
    pub fn read_update(run: Run) -> (Option<u32>, Option<String>) {
        match run(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                super::UPDATE_SCRIPT,
            ],
        ) {
            Some((_, out)) => super::parse_update(&out),
            None => (None, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Security;

    fn mp() -> MpStatus {
        MpStatus {
            antivirus_enabled: Some(true),
            real_time: Some(true),
            version: Some("1.400.12.0".into()),
            updated: Some("20261120083000.000000+000".into()),
            age_days: Some(2),
        }
    }

    #[test]
    fn product_state_is_decoded() {
        // Defender on and current; a third-party product off and outdated.
        let on = ScProduct {
            name: "Windows Defender".into(),
            state: 0x061100,
        };
        assert!(on.on() && on.up_to_date());
        let off = ScProduct {
            name: "Example AV".into(),
            state: 0x060010,
        };
        assert!(!off.on() && !off.up_to_date());
    }

    #[test]
    fn defender_is_completed_with_its_own_status() {
        let sc = [ScProduct {
            name: "Windows Defender".into(),
            state: 0x061100,
        }];
        let av = antivirus(Some(&mp()), &sc);
        assert_eq!(av.len(), 1);
        assert!(av[0].on);
        assert_eq!(av[0].version.as_deref(), Some("1.400.12.0"));
        assert_eq!(av[0].updated.as_deref(), Some("2026-11-20 08:30 UTC"));
        assert_eq!(av[0].age_days, Some(2));
        // Real-time protection switched off in Defender.
        let mut m = mp();
        m.real_time = Some(false);
        assert!(!antivirus(Some(&m), &sc)[0].on);
        // Windows Server: no Security Center, Defender from its own status.
        let av = antivirus(Some(&mp()), &[]);
        assert_eq!(av[0].name, "Microsoft Defender");
        assert!(av[0].on);
        assert_eq!(av[0].up_to_date, Some(true));
        // A third-party antivirus takes over; Defender passive.
        let sc = [
            ScProduct {
                name: "Example AV".into(),
                state: 0x041000,
            },
            ScProduct {
                name: "Windows Defender".into(),
                state: 0x060100,
            },
        ];
        let mut m = mp();
        m.real_time = Some(false);
        let av = antivirus(Some(&m), &sc);
        let sec = Security {
            antivirus: av,
            ..Default::default()
        };
        assert_eq!(sec.virus_protection(), Some(true));
        assert_eq!(
            sec.active_antivirus().map(|a| a.name.as_str()),
            Some("Example AV")
        );
        // Nothing readable: unknown.
        assert!(antivirus(None, &[]).is_empty());
    }

    #[test]
    fn cim_datetimes_become_utc() {
        assert_eq!(
            cim_datetime("20261127041753.000000+000").as_deref(),
            Some("2026-11-27 04:17 UTC")
        );
        // +07:00 (420 minutes east) → 7 hours earlier, across midnight.
        assert_eq!(
            cim_datetime("20261127031500.000000+420").as_deref(),
            Some("2026-11-26 20:15 UTC")
        );
        assert_eq!(
            cim_datetime("20261231230000.000000-060").as_deref(),
            Some("2027-01-01 00:00 UTC")
        );
        assert_eq!(cim_datetime("garbage"), None);
        assert_eq!(cim_datetime(""), None);
    }

    const UNINSTALL: &str = "\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ExampleEditor\r
    DisplayName    REG_SZ    Example Editor\r
    DisplayVersion    REG_SZ    3.2.1\r
    Publisher    REG_SZ    Example Corp\r
    InstallDate    REG_SZ    20260315\r
    EstimatedSize    REG_DWORD    0x2800\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{0000-HIDDEN}\r
    DisplayName    REG_SZ    Example Runtime Helper\r
    SystemComponent    REG_DWORD    0x1\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\KB0000001\r
    DisplayName    REG_SZ    Update for Example Editor (KB0000001)\r
    ParentKeyName    REG_SZ    ExampleEditor\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\NoName\r
    UninstallString    REG_SZ    C:\\Example\\uninstall.exe\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ExampleEditor\r
    DisplayName    REG_SZ    Example Editor\r
    DisplayVersion    REG_SZ    3.2.1\r
\r
HKEY_CURRENT_USER\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\apptool\r
    DisplayName    REG_SZ    another tool\r
    DisplayVersion    REG_SZ    0.9\r
    InstallDate    REG_SZ    2026/03/15\r
    ReleaseType    REG_SZ    Application\r
";

    #[test]
    fn uninstall_keys_become_installed_programs() {
        let keys = parse_reg(UNINSTALL);
        assert_eq!(keys.len(), 6);
        assert_eq!(keys[0].dword("EstimatedSize"), Some(0x2800));
        let a = apps(&keys);
        assert_eq!(
            a,
            vec![
                App {
                    name: "another tool".into(),
                    version: "0.9".into(),
                    publisher: String::new(),
                    installed: None,
                    size_kb: None,
                },
                App {
                    name: "Example Editor".into(),
                    version: "3.2.1".into(),
                    publisher: "Example Corp".into(),
                    installed: Some("2026-03-15".into()),
                    size_kb: Some(10_240),
                },
            ]
        );
    }

    #[test]
    fn install_dates_are_checked() {
        assert_eq!(install_date("20260315").as_deref(), Some("2026-03-15"));
        assert_eq!(install_date("20261399"), None);
        assert_eq!(install_date("3/15/2026"), None);
    }

    #[test]
    fn firewall_and_smartscreen_rules() {
        let p = firewall_profiles(&[
            ("Domain", Some(1), None),
            ("Private", Some(1), Some(0)),
            ("Public", None, None),
        ]);
        assert_eq!(
            p,
            vec![
                FirewallProfile {
                    name: "Domain".into(),
                    on: true
                },
                FirewallProfile {
                    name: "Private".into(),
                    on: false
                },
            ]
        );
        let s = Security {
            firewall_profiles: p.clone(),
            ..Default::default()
        };
        assert_eq!(s.firewall(), Some(false));
        assert_eq!(s.firewall_off(), vec!["Private"]);
        // A third-party firewall counts as on.
        let s = Security {
            firewall_profiles: p,
            firewall_products: vec!["Example Firewall".into()],
            ..Default::default()
        };
        assert_eq!(s.firewall(), Some(true));
        assert_eq!(Security::default().firewall(), None);

        assert!(smartscreen(None, None), "Windows default is on");
        assert!(smartscreen(None, Some("Warn")));
        assert!(!smartscreen(None, Some("Off")));
        assert!(!smartscreen(Some(0), Some("Warn")), "policy wins");
        assert!(smartscreen(Some(1), Some("Off")));
    }

    #[test]
    fn problems_name_what_is_off() {
        let mut s = Security {
            antivirus: antivirus(Some(&mp()), &[]),
            firewall_profiles: firewall_profiles(&[("Public", Some(1), None)]),
            app_control: Some(true),
            ..Default::default()
        };
        assert!(s.problems().is_empty());
        s.antivirus[0].age_days = Some(12);
        s.antivirus[0].on = false;
        s.firewall_profiles[0].on = false;
        s.app_control = Some(false);
        assert_eq!(
            s.problems(),
            vec![
                "Virus & threat protection is off".to_string(),
                "Microsoft Defender definitions are 12 days old".to_string(),
                "Firewall off for: Public".to_string(),
                "App & browser control (SmartScreen) is off".to_string(),
            ]
        );
    }

    #[test]
    fn windows_update_output() {
        assert_eq!(
            parse_update("PENDING=3\r\nLAST=2026-11-26\r\n"),
            (Some(3), Some("2026-11-26".into()))
        );
        assert_eq!(parse_update("LAST=bad\n"), (None, None));
        assert_eq!(parse_update(""), (None, None));
    }
}
