# telepad-core

`crates/telepad-core` — the domain and I/O library. Everything that talks to the outside
world (ssh, SQLite, the network, the OS password store, files, processes) or is pure domain
logic (parsers, config rendering, SQL quoting, key formats) lives here.

[Back to the developer guide](../DEVELOPER_GUIDE.md#crates) · other crates:
[telepad-app](telepad-app.md) · [telepad-gui](telepad-gui.md) · [telepad-check](telepad-check.md) ·
outside libraries: [DEPENDENCIES.md](../DEPENDENCIES.md)

## At a glance

| | |
|---|---|
| Kind | library (`telepad_core`) |
| Used by | [`telepad-app`](telepad-app.md) (types only), [`telepad-gui`](telepad-gui.md) (runtime calls it), [`telepad-check`](telepad-check.md) |
| Depends on | std and outside libraries only — **no GUI toolkit, no other Telepad crate** |
| Size | ~30 000 lines, 40+ modules |
| Tests | `cargo test -p telepad-core` (parsers against `tests/fixtures/`, pure helpers next to the code, live tests behind env vars) |

## Rules

- **No GUI.** Nothing here may depend on egui, eframe, rfd or arboard. A function that needs
  a file chosen takes the path; the GUI's runtime opens the dialog.
- **OS-specific code only in `platform.rs`**, behind `#[cfg(target_os = ...)]`. The one
  `unsafe` in the project (Windows Credential Manager via `windows-sys`) is there.
- **Read-only on remote hosts.** The collection script and every background read change
  nothing; the few in-app changes (authorized keys, database edits, file operations,
  Portainer actions) are separate functions only called from a user action — see
  [`AGENTS.md`](../../AGENTS.md#conventions) for the full list.
- **No `unwrap()` / `expect()`** on anything that depends on a host, the config or the network;
  errors are `Result<_, String>` or a `thiserror` enum (`store::Error`, `config::ConfigError`,
  `OpenError`, `local_keys::KeyError`).
- **Host values are separate process arguments**, never part of a shell string.
- **Blocking is fine here.** Functions in this crate may block (ssh, SQL, HTTP); it is the
  caller's job (the GUI runtime) to run them on a thread.

## Modules by area

### Workspace and inventory

| Module | What it does |
|---|---|
| `lib.rs` | `open_workspace` finds `config` and `telepad.db` (`data_dir`, `resolve_config`), imports the config on first run, returns a `Workspace { store, ssh_config, imported_from }`; `.env` fallback key (`dotenv_value`); `open_ssh_alias` (used by `telepad --open-ssh`) |
| `config.rs` | OpenSSH config import / export: `HostEntry` (alias, HostName, User, Port, IdentityFile, description, other options, `# telepad:` tags for not-monitored, RDP-only, group, cluster), `HostEntry::validate`, `HostSet` / `SetKind`, rendering the file `ssh -F` reads |
| `store.rs` | `telepad.db` (SQLite): hosts, settings, groups / clusters, database connections, Remote Desktop settings, Portainers; schema version in `PRAGMA user_version` with step-by-step migration; `check_newer_schema` before opening for writing; transactional import (Merge / Replace), `parse_pasted`, JSON + OpenSSH export, generated `telepad.ssh_config` |
| `hostsync.rs` | Host Management: block-preserving model of an OpenSSH config file, compare / apply / diff, safe write (`.bak`, atomic, only if unchanged since loading) |
| `prefs.rs` | `preference.db`: named workspaces and their layouts |
| `history.rs` | `history.db`: log of transfers, file operations, SQL and Portainer changes; trimming, `mask_secrets`, CSV export |
| `strategy.rs` | `update.db`: Update Strategy (intervals, pause), last known static / slow facts, paused hosts |

### Collection

| Module | What it does |
|---|---|
| `collector.rs` | Runs `ssh -F <config> -T -o BatchMode=yes … <alias> sh -s` with `script.sh` on stdin, bounded worker pool, timeouts, `Scope` (which tiers), `carry_over` of skipped facts, ping / TCP `reach` checks; results go back as `Update`s over a channel |
| `script.sh` | The remote, read-only POSIX `sh` script, embedded with `include_str!`; one `===SECTION===` per metric; `TP_STATIC` / `TP_SLOW` switch the slower tiers |
| `parse.rs` | Pure parsers for every section → `model` types; tested against `tests/fixtures/*.txt` |
| `model.rs` | Report types (`HostReport`, disks, sensors, containers, …; `serde::Serialize` for `--json`) and formatting helpers (`human_kb`, `human_uptime`) |
| `cluster.rs` | Docker Swarm and Kubernetes model and parsers |
| `eol.rs` | End-of-support dates per distribution release |
| `local.rs` | My Computer: collect this PC without SSH (Windows via `sysinfo` and `netstat`; Linux / macOS by running `script.sh` locally) |
| `login_log.rs` | Log info: read SSH logins from the systemd journal, parse `Accepted` lines |

### Actions on hosts (user-started only)

| Module | What it does |
|---|---|
| `docker_actions.rs` | Docker / compose command lines and the terminal script that shows, confirms and runs them; read-only "How it runs" |
| `authorized_keys.rs` | List, validate, add and revoke public keys in `~/.ssh/authorized_keys`; lockout guard |
| `bmc_setup.sh` | Interactive BMC (`ipmitool`) setup in a user-opened terminal |
| `rdp.rs` | Remote Desktop settings, `.rdp` render / parse, `xfreerdp` arguments |

### Files, databases, Portainer

| Module | What it does |
|---|---|
| `files/` | Own SFTP v3 client (`sftp.rs`), `scp -O` fallback, rsync, transfer planning and copying via `.telepad-part`, file operations (`ops.rs`), the editor's load / save (`edit.rs`), text helpers incl. regex find / replace (`text.rs`), syntax lexer (`syntax.rs`), directory compare and sync (`compare.rs`) |
| `db/` | Database browser: detect databases on a host (`detect.rs`), `ssh -N -L` tunnels (`tunnel.rs`), MySQL / PostgreSQL sessions (`client.rs`), SQL quoting and checks (`sql.rs`), DDL (`ddl.rs`), passwords in the OS store (`secret.rs`), export / import CSV, SQL, Excel (`io.rs`) |
| `portainer.rs` | Portainer API client over an SSH tunnel: own HTTP/1.1 client, TLS via `rustls` with a pinned certificate, token in the OS password store, parsers, stack and container actions |
| `table_export.rs` | Table exports: CSV, JSON, Excel (`rust_xlsxwriter`), PDF (`pdf-writer` + `ttf-parser`, fonts passed in by the caller) |

### Keys, updates, platform

| Module | What it does |
|---|---|
| `local_keys.rs` | My keys: scan `~/.ssh`, detect and load many private key formats (OpenSSH, PuTTY PPK v2 / v3, PKCS#1 / #8, SEC1, PKCS#12, JWK, OpenPGP), convert, generate with `ssh-keygen` |
| `update.rs` | Self-update from GitHub releases: check, download, SHA-256 verify, replace both executables, relaunch |
| `platform.rs` | ssh binary lookup, hidden console, open terminal / RDP / URL / folder, ping, owner-only file permissions, Windows Credential Manager |

## How the other crates use it

- **[telepad-app](telepad-app.md)** only uses its *types* (`HostEntry`, `HostReport`,
  `collector::Job`, `db::DbTarget`, `files::Entry`, …) and pure functions (validation, SQL
  building, filters). It never calls the I/O functions — it returns an `Effect` instead.
- **[telepad-gui](telepad-gui.md)**'s `runtime.rs` calls the I/O functions on threads and
  turns the results into `Msg`s.
- **[telepad-check](telepad-check.md)** calls the same functions directly and prints.

A typical call chain, collecting one host:

```
telepad-app  update(Msg::Tick)        → Effect::Collect(jobs)
telepad-gui  runtime                  → collector::Collector::enqueue(jobs)  (worker pool)
telepad-core collector                → ssh … sh -s < script.sh → parse::* → HostReport
telepad-gui  runtime.frame()          ← collector::Update            → Msg::Collected
telepad-app  update(Msg::Collected)   → model.hosts[alias].report = …
```

## Adding to telepad-core

- **New metric:** a new section in `script.sh` (POSIX `sh`, BusyBox-safe, every command
  guarded, `LC_ALL=C`), a parser in `parse.rs`, a fixture test, a field in `model.rs`.
- **New SQLite column or table:** bump `SCHEMA_VERSION` in `store.rs` and migrate from the old
  `user_version`; bump `MIN_COMPATIBLE_SCHEMA` only when older builds would misread the file.
- **New OS call:** a function in `platform.rs` with an implementation per `#[cfg(target_os)]`.
- **New outside library:** add it to `Cargo.toml` and describe it in
  [DEPENDENCIES.md](../DEPENDENCIES.md).

## Tests

| Kind | Where |
|---|---|
| Parsers | `parse.rs`, `cluster.rs`, `portainer.rs`, `login_log.rs` against `tests/fixtures/` |
| Pure helpers | `#[cfg(test)] mod tests` next to the code (SQL quoting, key formats, config rendering, schema migration on a temp file, …) |
| Live (skipped unless the variable is set) | `TELEPAD_TEST_SSH` (`tests/sshd` lab image), `TELEPAD_TEST_MYSQL`, `TELEPAD_TEST_PG`, `TELEPAD_TEST_KEYGEN`, `TELEPAD_TEST_PPK_DIR`, `TELEPAD_TEST_PUTTYGEN`, `TELEPAD_TEST_GPG` |

Some key-format crates are slow unoptimised; the workspace `Cargo.toml` builds them with
`opt-level = 3` even in debug so the tests stay fast.
