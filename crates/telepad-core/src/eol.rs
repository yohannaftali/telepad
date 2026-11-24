//! End of vendor support per distribution release, and the little date
//! arithmetic needed to compare it with the host's date. Dates are the end of
//! regular security support (Debian including LTS, Ubuntu standard support
//! without ESM, RHEL family full/maintenance support). Approximate by nature —
//! check the vendor when it matters; update this table when releases change.

/// (os-release `ID`, `VERSION_ID` major or full, end of support `YYYY-MM-DD`).
const TABLE: &[(&str, &str, &str)] = &[
    // Ubuntu (standard support; LTS and recent interim releases)
    ("ubuntu", "14.04", "2019-04-30"),
    ("ubuntu", "16.04", "2021-04-30"),
    ("ubuntu", "18.04", "2023-05-31"),
    ("ubuntu", "20.04", "2025-05-31"),
    ("ubuntu", "22.04", "2027-06-30"),
    ("ubuntu", "23.04", "2024-01-25"),
    ("ubuntu", "23.10", "2024-07-11"),
    ("ubuntu", "24.04", "2029-05-31"),
    ("ubuntu", "24.10", "2025-07-10"),
    // Debian (security support including LTS)
    ("debian", "8", "2020-06-30"),
    ("debian", "9", "2022-06-30"),
    ("debian", "10", "2024-06-30"),
    ("debian", "11", "2026-08-31"),
    ("debian", "12", "2028-06-30"),
    ("debian", "13", "2030-06-30"),
    // RHEL family
    ("centos", "6", "2020-11-30"),
    ("centos", "7", "2024-06-30"),
    ("centos", "8", "2021-12-31"),
    ("rhel", "6", "2020-11-30"),
    ("rhel", "7", "2024-06-30"),
    ("rhel", "8", "2029-05-31"),
    ("rhel", "9", "2032-05-31"),
    ("rocky", "8", "2029-05-31"),
    ("rocky", "9", "2032-05-31"),
    ("almalinux", "8", "2029-03-01"),
    ("almalinux", "9", "2032-05-31"),
    ("ol", "7", "2024-12-31"),
    ("ol", "8", "2029-07-31"),
    ("ol", "9", "2032-06-30"),
    // Alpine
    ("alpine", "3.16", "2024-05-23"),
    ("alpine", "3.17", "2024-11-22"),
    ("alpine", "3.18", "2025-05-09"),
    ("alpine", "3.19", "2025-11-01"),
    ("alpine", "3.20", "2026-04-01"),
    ("alpine", "3.21", "2026-11-01"),
    ("alpine", "3.22", "2027-05-01"),
    // openSUSE Leap
    ("opensuse-leap", "15.4", "2023-12-07"),
    ("opensuse-leap", "15.5", "2024-12-31"),
    ("opensuse-leap", "15.6", "2025-12-31"),
];

/// CentOS Stream has the same `ID`/`VERSION_ID` as CentOS Linux but other dates.
const CENTOS_STREAM: &[(&str, &str)] = &[("8", "2024-05-31"), ("9", "2027-05-31")];

/// End-of-support date for a release, if known. `pretty` tells CentOS Stream apart.
pub fn end_of_support(id: &str, version_id: &str, pretty: &str) -> Option<&'static str> {
    let id = id.trim().to_ascii_lowercase();
    let v = version_id.trim();
    if v.is_empty() {
        return None;
    }
    // Ubuntu/openSUSE use "22.04"; Debian/RHEL family "12" / "9.4"; Alpine "3.20.3".
    let major = v.split('.').next().unwrap_or(v);
    let major_minor: String = v.split('.').take(2).collect::<Vec<_>>().join(".");
    if id == "centos" && pretty.to_ascii_lowercase().contains("stream") {
        return CENTOS_STREAM
            .iter()
            .find(|(m, _)| *m == major)
            .map(|(_, d)| *d);
    }
    TABLE
        .iter()
        .find(|(i, ver, _)| *i == id && (*ver == v || *ver == major_minor || *ver == major))
        .map(|(_, _, d)| *d)
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` date (proleptic Gregorian).
pub fn days_from_iso(date: &str) -> Option<i64> {
    let mut it = date.trim().splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.get(..2)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `YYYY-MM-DD` of a day count since 1970-01-01 (inverse of [`days_from_iso`]).
pub fn iso_from_days(z: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// This computer's date (UTC), as the remote script's `date +%F` gives it.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    iso_from_days(secs.div_euclid(86_400))
}

/// Support status of a release on the host's `today`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// More than [`SOON_DAYS`] left.
    Supported { days_left: i64 },
    /// Support ends within [`SOON_DAYS`].
    EndingSoon { days_left: i64 },
    /// Support ended this many days ago.
    Ended { days_ago: i64 },
}

/// Warn this long before the end of support.
pub const SOON_DAYS: i64 = 180;

pub fn support(end: &str, today: &str) -> Option<Support> {
    let left = days_from_iso(end)? - days_from_iso(today)?;
    Some(if left < 0 {
        Support::Ended { days_ago: -left }
    } else if left <= SOON_DAYS {
        Support::EndingSoon { days_left: left }
    } else {
        Support::Supported { days_left: left }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_releases_by_id_and_version() {
        assert_eq!(
            end_of_support("ubuntu", "16.04", "Ubuntu 16.04.5 LTS"),
            Some("2021-04-30")
        );
        assert_eq!(
            end_of_support("debian", "12", "Debian GNU/Linux 12"),
            Some("2028-06-30")
        );
        assert_eq!(
            end_of_support("rocky", "9.4", "Rocky Linux 9.4"),
            Some("2032-05-31")
        );
        assert_eq!(
            end_of_support("alpine", "3.20.3", "Alpine Linux v3.20"),
            Some("2026-04-01")
        );
        assert_eq!(
            end_of_support("centos", "7", "CentOS Linux 7 (Core)"),
            Some("2024-06-30")
        );
        assert_eq!(
            end_of_support("centos", "9", "CentOS Stream 9"),
            Some("2027-05-31")
        );
        assert_eq!(end_of_support("ubuntu", "99.04", "future"), None);
        assert_eq!(end_of_support("synology", "", "Synology DSM 7.2"), None);
    }

    #[test]
    fn date_arithmetic_and_support_status() {
        assert_eq!(days_from_iso("1970-01-01"), Some(0));
        assert_eq!(days_from_iso("2000-03-01"), Some(11_017));
        assert_eq!(
            days_from_iso("2026-11-17").unwrap() - days_from_iso("2026-11-16").unwrap(),
            1
        );
        assert_eq!(
            days_from_iso("2024-03-01").unwrap() - days_from_iso("2024-02-28").unwrap(),
            2,
            "leap year"
        );
        assert_eq!(days_from_iso("not a date"), None);
        for d in [
            "1970-01-01",
            "2000-02-29",
            "2024-12-31",
            "2026-11-24",
            "1969-07-20",
        ] {
            assert_eq!(iso_from_days(days_from_iso(d).unwrap()), d);
        }
        assert_eq!(
            support("2024-06-30", "2026-11-17"),
            Some(Support::Ended { days_ago: 870 })
        );
        assert_eq!(
            support("2027-01-31", "2026-11-17"),
            Some(Support::EndingSoon { days_left: 75 })
        );
        assert!(matches!(
            support("2032-05-31", "2026-11-17"),
            Some(Support::Supported { .. })
        ));
        assert_eq!(support("2032-05-31", ""), None, "no host date, no verdict");
    }
}
