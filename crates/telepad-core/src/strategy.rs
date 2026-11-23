//! `update.db` (#122): how often Telepad checks the hosts (the Update
//! Strategy page) and the last known slow-changing facts of each host, so a
//! restart shows them at once and does not read them again before they are due.
//! A small SQLite file next to `telepad.db`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::model::{HostReport, Listener, Maintenance, NetAddr};
use crate::store::StoreError;

pub const STRATEGY_FILE: &str = "update.db";
const STRATEGY_VERSION: i32 = 1;

/// One automatic check: on / off and how often (seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Every {
    pub on: bool,
    pub secs: u32,
}

impl Every {
    pub fn every(self) -> Duration {
        Duration::from_secs(u64::from(self.secs))
    }
}

/// The checks Telepad runs by itself, and whether it runs them at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Strategy {
    /// Nothing runs by itself (no SSH, no ping); Refresh still works.
    pub paused: bool,
    /// Ping every monitored host (no SSH); RDP-only hosts get their port probe.
    pub ping: Every,
    /// CPU, memory, disks, sensors, Docker / cluster.
    pub live: Every,
    /// IP addresses, listening ports, pending updates, reboot needed (`TP_SLOW`).
    pub slow: Every,
    /// Hostname, OS release, kernel, architecture, CPU model (`TP_STATIC`).
    pub static_info: Every,
    /// A host without ping reply is only pinged; SSH is still tried this often
    /// (seconds), for hosts whose firewall drops ICMP.
    pub ssh_retry: u32,
}

impl Default for Strategy {
    fn default() -> Self {
        Self {
            paused: false,
            ping: Every { on: true, secs: 30 },
            live: Every { on: true, secs: 60 },
            slow: Every {
                on: true,
                secs: 6 * 3600,
            },
            static_info: Every {
                on: true,
                secs: 24 * 3600,
            },
            ssh_retry: 10 * 60,
        }
    }
}

/// Which interval a limit is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Ping,
    Live,
    Slow,
    Static,
    SshRetry,
}

impl Check {
    pub const ALL: [Check; 5] = [
        Check::Ping,
        Check::Live,
        Check::Slow,
        Check::Static,
        Check::SshRetry,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Check::Ping => "Ping",
            Check::Live => "Live metrics",
            Check::Slow => "Network & updates",
            Check::Static => "System info",
            Check::SshRetry => "Retry SSH for unreachable hosts",
        }
    }

    /// Allowed interval in seconds (inclusive).
    pub fn range(self) -> (u32, u32) {
        const DAY: u32 = 24 * 3600;
        match self {
            Check::Ping => (10, 3600),
            Check::Live => (15, DAY),
            Check::Slow | Check::Static => (60, 7 * DAY),
            Check::SshRetry => (60, DAY),
        }
    }
}

impl Strategy {
    pub fn secs(&self, c: Check) -> u32 {
        match c {
            Check::Ping => self.ping.secs,
            Check::Live => self.live.secs,
            Check::Slow => self.slow.secs,
            Check::Static => self.static_info.secs,
            Check::SshRetry => self.ssh_retry,
        }
    }

    pub fn ssh_retry_every(&self) -> Duration {
        Duration::from_secs(u64::from(self.ssh_retry))
    }

    /// Every interval within its [`Check::range`].
    pub fn validate(&self) -> Result<(), String> {
        for c in Check::ALL {
            let (lo, hi) = c.range();
            let s = self.secs(c);
            if !(lo..=hi).contains(&s) {
                return Err(format!(
                    "{}: every {} to {} (now {})",
                    c.title(),
                    human_secs(lo),
                    human_secs(hi),
                    human_secs(s)
                ));
            }
        }
        Ok(())
    }
}

/// `90` → `1 min 30 s`, `21600` → `6 h`.
pub fn human_secs(s: u32) -> String {
    let (d, h, m, sec) = (s / 86400, s / 3600 % 24, s / 60 % 60, s % 60);
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "min"), (sec, "s")]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, u)| format!("{n} {u}"))
        .collect();
    if parts.is_empty() {
        "0 s".into()
    } else {
        parts.join(" ")
    }
}

/// What the daily tier reads (`TP_STATIC`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StaticFacts {
    pub hostname: String,
    pub os_name: String,
    pub os_pretty: String,
    pub os_id: String,
    pub os_version_id: String,
    pub kernel: String,
    pub os_type: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_cores: Option<u32>,
}

impl StaticFacts {
    pub fn of(r: &HostReport) -> Self {
        Self {
            hostname: r.hostname.clone(),
            os_name: r.os_name.clone(),
            os_pretty: r.os_pretty.clone(),
            os_id: r.os_id.clone(),
            os_version_id: r.os_version_id.clone(),
            kernel: r.kernel.clone(),
            os_type: r.os_type.clone(),
            arch: r.arch.clone(),
            cpu_model: r.cpu_model.clone(),
            cpu_cores: r.cpu_cores,
        }
    }

    pub fn apply(&self, r: &mut HostReport) {
        r.hostname.clone_from(&self.hostname);
        r.os_name.clone_from(&self.os_name);
        r.os_pretty.clone_from(&self.os_pretty);
        r.os_id.clone_from(&self.os_id);
        r.os_version_id.clone_from(&self.os_version_id);
        r.kernel.clone_from(&self.kernel);
        r.os_type.clone_from(&self.os_type);
        r.arch.clone_from(&self.arch);
        r.cpu_model.clone_from(&self.cpu_model);
        r.cpu_cores = self.cpu_cores;
    }
}

/// What the 6-hourly tier reads (`TP_SLOW`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlowFacts {
    pub addresses: Vec<NetAddr>,
    pub listeners: Vec<Listener>,
    pub maintenance: Maintenance,
}

impl SlowFacts {
    pub fn of(r: &HostReport) -> Self {
        Self {
            addresses: r.addresses.clone(),
            listeners: r.listeners.clone(),
            maintenance: r.maintenance.clone(),
        }
    }

    pub fn apply(&self, r: &mut HostReport) {
        r.addresses.clone_from(&self.addresses);
        r.listeners.clone_from(&self.listeners);
        r.maintenance.clone_from(&self.maintenance);
    }
}

/// Facts of one host to keep: the tiers that were just read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    pub alias: String,
    pub static_info: Option<StaticFacts>,
    pub slow: Option<SlowFacts>,
}

/// One host's facts as saved, with their age when they were loaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedFacts {
    pub alias: String,
    pub static_info: Option<(StaticFacts, Duration)>,
    pub slow: Option<(SlowFacts, Duration)>,
}

/// What `update.db` holds, as read on start.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    pub strategy: Strategy,
    pub facts: Vec<SavedFacts>,
    /// Hosts paused one by one (aliases).
    pub paused_hosts: Vec<String>,
}

/// `--update-db` > `TELEPAD_UPDATE_DB` > `update.db` next to `telepad.db`.
pub fn strategy_path(db: &Path, cli: Option<&str>) -> PathBuf {
    cli.map(PathBuf::from)
        .or_else(|| std::env::var_os("TELEPAD_UPDATE_DB").map(PathBuf::from))
        .unwrap_or_else(|| db.with_file_name(STRATEGY_FILE))
}

pub struct UpdateDb {
    conn: Connection,
    permission_warning: Option<String>,
}

impl UpdateDb {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v > STRATEGY_VERSION {
            return Err(StoreError::Invalid(format!(
                "{} was written by a newer Telepad (version {v})",
                path.display()
            )));
        }
        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS strategy (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS last_facts (
                 alias        TEXT    NOT NULL,
                 -- 'static' or 'slow'
                 tier         TEXT    NOT NULL,
                 json         TEXT    NOT NULL,
                 -- seconds since 1970-01-01 UTC
                 collected_at INTEGER NOT NULL,
                 PRIMARY KEY (alias, tier)
             );
             -- Hosts whose automatic checks are paused one by one.
             CREATE TABLE IF NOT EXISTS paused_hosts (
                 alias TEXT PRIMARY KEY
             );
             PRAGMA user_version = {STRATEGY_VERSION};"
        ))?;
        Ok(Self {
            conn,
            permission_warning: crate::platform::restrict_db_files(path).err(),
        })
    }

    /// Why the file stays readable by others (#110), if it does.
    pub fn permission_warning(&self) -> Option<&str> {
        self.permission_warning.as_deref()
    }

    /// The saved strategy; a missing or out-of-range value is its default.
    pub fn load_strategy(&self) -> Result<Strategy, StoreError> {
        let mut st = self.conn.prepare("SELECT key, value FROM strategy")?;
        let rows: Vec<(String, String)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let get = |k: &str| rows.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        let flag = |k: &str, d: bool| get(k).map_or(d, |v| v == "1");
        let secs = |k: &str, c: Check, d: u32| {
            let (lo, hi) = c.range();
            get(k)
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|s| (lo..=hi).contains(s))
                .unwrap_or(d)
        };
        let d = Strategy::default();
        let every = |name: &str, c: Check, def: Every| Every {
            on: flag(&format!("{name}_on"), def.on),
            secs: secs(&format!("{name}_secs"), c, def.secs),
        };
        Ok(Strategy {
            paused: flag("paused", d.paused),
            ping: every("ping", Check::Ping, d.ping),
            live: every("live", Check::Live, d.live),
            slow: every("slow", Check::Slow, d.slow),
            static_info: every("static", Check::Static, d.static_info),
            ssh_retry: secs("ssh_retry_secs", Check::SshRetry, d.ssh_retry),
        })
    }

    pub fn save_strategy(&self, s: &Strategy) -> Result<(), StoreError> {
        let on = |b: bool| if b { "1" } else { "0" }.to_string();
        let rows = [
            ("paused", on(s.paused)),
            ("ping_on", on(s.ping.on)),
            ("ping_secs", s.ping.secs.to_string()),
            ("live_on", on(s.live.on)),
            ("live_secs", s.live.secs.to_string()),
            ("slow_on", on(s.slow.on)),
            ("slow_secs", s.slow.secs.to_string()),
            ("static_on", on(s.static_info.on)),
            ("static_secs", s.static_info.secs.to_string()),
            ("ssh_retry_secs", s.ssh_retry.to_string()),
        ];
        let tx = self.conn.unchecked_transaction()?;
        for (k, v) in rows {
            tx.execute(
                "INSERT OR REPLACE INTO strategy(key, value) VALUES(?1, ?2)",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Keep the tiers in `f` as read at `now` (Unix seconds).
    pub fn save_facts(&self, f: &Facts, now: i64) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let put = |tier: &str, json: String| {
            tx.execute(
                "INSERT OR REPLACE INTO last_facts(alias, tier, json, collected_at)
                 VALUES(?1, ?2, ?3, ?4)",
                params![f.alias, tier, json, now],
            )
            .map(|_| ())
        };
        if let Some(s) = &f.static_info {
            put("static", serde_json::to_string(s)?)?;
        }
        if let Some(s) = &f.slow {
            put("slow", serde_json::to_string(s)?)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every host's saved facts with their age at `now` (Unix seconds). Rows
    /// that no longer parse are skipped (read again on the next collection).
    pub fn load_facts(&self, now: i64) -> Result<Vec<SavedFacts>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT alias, tier, json, collected_at FROM last_facts ORDER BY alias, tier",
        )?;
        let rows: Vec<(String, String, String, i64)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        let mut out: Vec<SavedFacts> = Vec::new();
        for (alias, tier, json, at) in rows {
            let age = Duration::from_secs(u64::try_from(now.saturating_sub(at)).unwrap_or(0));
            if out.last().is_none_or(|f| f.alias != alias) {
                out.push(SavedFacts {
                    alias: alias.clone(),
                    ..Default::default()
                });
            }
            let Some(f) = out.last_mut() else { continue };
            match tier.as_str() {
                "static" => f.static_info = serde_json::from_str(&json).ok().map(|s| (s, age)),
                "slow" => f.slow = serde_json::from_str(&json).ok().map(|s| (s, age)),
                _ => {}
            }
        }
        out.retain(|f| f.static_info.is_some() || f.slow.is_some());
        Ok(out)
    }

    /// Forget the facts of hosts that are not in `keep` (deleted or renamed).
    pub fn prune(&self, keep: &[String]) -> Result<(), StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT alias FROM last_facts UNION SELECT alias FROM paused_hosts")?;
        let have: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let tx = self.conn.unchecked_transaction()?;
        for a in have.iter().filter(|a| !keep.contains(a)) {
            tx.execute("DELETE FROM last_facts WHERE alias = ?1", [a])?;
            tx.execute("DELETE FROM paused_hosts WHERE alias = ?1", [a])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Pause or resume one host's automatic checks.
    pub fn set_host_paused(&self, alias: &str, paused: bool) -> Result<(), StoreError> {
        if paused {
            self.conn.execute(
                "INSERT OR IGNORE INTO paused_hosts(alias) VALUES(?1)",
                [alias],
            )?;
        } else {
            self.conn
                .execute("DELETE FROM paused_hosts WHERE alias = ?1", [alias])?;
        }
        Ok(())
    }

    /// Aliases of the hosts paused one by one, A–Z.
    pub fn paused_hosts(&self) -> Result<Vec<String>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT alias FROM paused_hosts ORDER BY alias")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Everything Telepad reads from `update.db` on start.
    pub fn load(&self, now: i64) -> Result<Loaded, StoreError> {
        Ok(Loaded {
            strategy: self.load_strategy()?,
            facts: self.load_facts(now)?,
            paused_hosts: self.paused_hosts()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("telepad-strategy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", p.display()));
        }
        p
    }

    fn report() -> HostReport {
        HostReport {
            hostname: "web01".into(),
            os_pretty: "Ubuntu 24.04.1 LTS".into(),
            os_id: "ubuntu".into(),
            os_version_id: "24.04".into(),
            kernel: "6.8.0-45-generic".into(),
            cpu_cores: Some(8),
            addresses: vec![NetAddr {
                interface: "eth0".into(),
                family: "inet".into(),
                address: "192.0.2.10/24".into(),
            }],
            listeners: vec![Listener {
                proto: "tcp".into(),
                address: "0.0.0.0".into(),
                port: 22,
                process: String::new(),
            }],
            maintenance: Maintenance {
                updates: Some(5),
                reboot_required: Some(true),
                ..Default::default()
            },
            uptime_secs: Some(100),
            ..Default::default()
        }
    }

    #[test]
    fn strategy_round_trip_and_defaults() {
        let path = temp("s.db");
        let db = UpdateDb::open(&path).unwrap();
        assert_eq!(db.load_strategy().unwrap(), Strategy::default());
        let s = Strategy {
            paused: true,
            ping: Every {
                on: false,
                secs: 15,
            },
            live: Every {
                on: true,
                secs: 120,
            },
            slow: Every {
                on: false,
                secs: 3600,
            },
            static_info: Every {
                on: true,
                secs: 48 * 3600,
            },
            ssh_retry: 300,
        };
        db.save_strategy(&s).unwrap();
        drop(db);
        let db = UpdateDb::open(&path).unwrap();
        assert_eq!(db.load_strategy().unwrap(), s);
        // A value out of range (edited by hand) falls back to its default.
        db.conn
            .execute(
                "UPDATE strategy SET value = '1' WHERE key = 'live_secs'",
                [],
            )
            .unwrap();
        assert_eq!(db.load_strategy().unwrap().live.secs, 60);
    }

    #[test]
    fn validate_checks_every_range() {
        assert!(Strategy::default().validate().is_ok());
        let mut s = Strategy::default();
        s.ping.secs = 5;
        let e = s.validate().unwrap_err();
        assert!(e.starts_with("Ping"), "{e}");
        let mut s = Strategy::default();
        s.slow.secs = 8 * 24 * 3600;
        assert!(s.validate().unwrap_err().contains("7 d"));
        // Off still needs a valid interval (it is kept for when it is on again).
        let mut s = Strategy::default();
        s.live.on = false;
        s.live.secs = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn human_secs_reads_well() {
        assert_eq!(human_secs(0), "0 s");
        assert_eq!(human_secs(30), "30 s");
        assert_eq!(human_secs(90), "1 min 30 s");
        assert_eq!(human_secs(6 * 3600), "6 h");
        assert_eq!(human_secs(7 * 86400), "7 d");
    }

    #[test]
    fn facts_round_trip_with_age_and_prune() {
        let path = temp("f.db");
        let db = UpdateDb::open(&path).unwrap();
        let r = report();
        db.save_facts(
            &Facts {
                alias: "web01".into(),
                static_info: Some(StaticFacts::of(&r)),
                slow: Some(SlowFacts::of(&r)),
            },
            1_000,
        )
        .unwrap();
        // Only the slow tier ran later.
        db.save_facts(
            &Facts {
                alias: "web01".into(),
                static_info: None,
                slow: Some(SlowFacts::default()),
            },
            4_000,
        )
        .unwrap();
        db.save_facts(
            &Facts {
                alias: "db01".into(),
                static_info: Some(StaticFacts::default()),
                slow: None,
            },
            5_000,
        )
        .unwrap();
        let got = db.load_facts(5_600).unwrap();
        assert_eq!(got.len(), 2);
        let web = got.iter().find(|f| f.alias == "web01").unwrap();
        let (st, age) = web.static_info.as_ref().unwrap();
        assert_eq!(st.os_pretty, "Ubuntu 24.04.1 LTS");
        assert_eq!(st.cpu_cores, Some(8));
        assert_eq!(*age, Duration::from_secs(4_600));
        let (sl, age) = web.slow.as_ref().unwrap();
        assert_eq!(*sl, SlowFacts::default());
        assert_eq!(*age, Duration::from_secs(1_600));
        let db01 = got.iter().find(|f| f.alias == "db01").unwrap();
        assert!(db01.slow.is_none());

        // A clock that went back gives age 0, not a panic.
        assert_eq!(
            db.load_facts(0).unwrap()[0].static_info.as_ref().unwrap().1,
            Duration::ZERO
        );

        db.prune(&["db01".to_string()]).unwrap();
        let got = db.load_facts(5_600).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].alias, "db01");

        // A row that no longer parses is skipped.
        db.conn
            .execute(
                "UPDATE last_facts SET json = 'not json' WHERE alias = 'db01'",
                [],
            )
            .unwrap();
        assert!(db.load_facts(5_600).unwrap().is_empty());
    }

    #[test]
    fn hosts_are_paused_one_by_one_and_pruned() {
        let path = temp("p.db");
        let db = UpdateDb::open(&path).unwrap();
        db.set_host_paused("web01", true).unwrap();
        db.set_host_paused("web01", true).unwrap();
        db.set_host_paused("db01", true).unwrap();
        assert_eq!(db.paused_hosts().unwrap(), ["db01", "web01"]);
        db.set_host_paused("db01", false).unwrap();
        assert_eq!(db.load(0).unwrap().paused_hosts, ["web01"]);
        // Deleted or renamed: forgotten.
        db.prune(&["db01".to_string()]).unwrap();
        assert!(db.paused_hosts().unwrap().is_empty());
    }

    #[test]
    fn facts_apply_to_a_report() {
        let r = report();
        let mut new = HostReport {
            uptime_secs: Some(200),
            ..Default::default()
        };
        StaticFacts::of(&r).apply(&mut new);
        SlowFacts::of(&r).apply(&mut new);
        assert_eq!(new.kernel, "6.8.0-45-generic");
        assert_eq!(new.listeners.len(), 1);
        assert_eq!(new.maintenance.updates, Some(5));
        assert_eq!(new.uptime_secs, Some(200));
    }

    #[test]
    fn newer_file_is_refused() {
        let path = temp("newer.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("PRAGMA user_version = 99;").unwrap();
        }
        let e = UpdateDb::open(&path).err().unwrap().to_string();
        assert!(e.contains("newer Telepad"), "{e}");
    }

    #[test]
    fn path_prefers_cli_then_next_to_the_db() {
        let db = Path::new("data").join("telepad.db");
        assert_eq!(strategy_path(&db, Some("x/u.db")), PathBuf::from("x/u.db"));
        if std::env::var_os("TELEPAD_UPDATE_DB").is_none() {
            assert_eq!(
                strategy_path(&db, None),
                Path::new("data").join(STRATEGY_FILE)
            );
        }
    }
}
