# AGENTS.md — Telepad

Guidance for contributors and coding agents. Telepad is a local (not web-based)
infrastructure monitor written in Rust. Hosts live in a SQLite file (`telepad.db`,
first filled from an OpenSSH `config`); it logs in to each host with the system
OpenSSH client and the IdentityFile from that config, runs a read-only POSIX `sh`
script, and shows OS, CPU, memory, disks, sensors, IP addresses, listening ports and
Docker containers (with container IPs and ports) in a desktop window. Each host has
a button that opens an interactive SSH session in a terminal (or RDP for
Windows hosts without SSH).

## Repository

remote: https://github.com/yohannaftali/telepad
platform: GitHub
default branch: `main`

## Platforms

| Platform | Status |
|---|---|
| Windows 10/11 x86_64 | **first target** — build, test and release here first |
| macOS | must keep compiling (CI); terminal launch via Terminal.app (less tested) |
| Linux | must keep compiling (CI); terminal launch via common emulators (less tested) |

## Architecture

Cargo workspace, four crates. Arrows are the only allowed dependencies:

```
telepad-gui ──► telepad-app ──► telepad-core ◄── telepad-check
     └──────────────────────────────►┘
```

| Crate | Role | May depend on |
|---|---|---|
| `crates/telepad-core` | Domain and I/O: inventory, ssh collection, parsers, platform, self-update | std + its own deps; **no GUI** |
| `crates/telepad-app` | Model–View–Update: `State`, `Msg`, `Effect`, `update()` | `telepad-core` only; **no GUI, no I/O** |
| `crates/telepad-gui` | egui views + runtime → binary `telepad` | core, app, eframe, rfd |
| `crates/telepad-check` | Headless CLI → binary `telepad-check` | core only |

### telepad-core

| Path | Role |
|---|---|
| `src/lib.rs` | Workspace: find `config` / `telepad.db`, first-run import, `.env` fallback key, `open_ssh_alias` |
| `src/config.rs` | OpenSSH config import/export: hosts (alias, HostName, User, Port, IdentityFile, description, other options, not-monitored tag, RDP-only tag) + global `Host *`/`Match` text; renders the file `ssh -F` reads |
| `src/store.rs` | SQLite inventory (`rusqlite`, bundled): `hosts` + `settings` tables, `PRAGMA user_version` schema version, transactional import (Merge/Replace), JSON + OpenSSH export, generated `telepad.ssh_config` (atomic write) |
| `src/collector.rs` | Runs `ssh -F <config> -T -o BatchMode=yes ... <alias> sh -s` with `script.sh` on stdin; timeouts; bounded worker pool; sends `Update`s over a channel; RDP-only hosts get ping + TCP probe |
| `src/script.sh` | Remote read-only collection script, `===SECTION===` markers |
| `src/bmc_setup.sh` | Interactive, user-started BMC (`ipmitool`) setup — see Conventions |
| `src/parse.rs` | Pure parsers for each section, unit-tested against `tests/fixtures/` |
| `src/model.rs` | Report data types (serde `Serialize`) and formatting helpers |
| `src/update.rs` | Self-update: GitHub `releases/latest` over HTTPS (`ureq`), SemVer check, SHA-256 from the release's `.sha256`, replace both exes (`self_replace`), relaunch |
| `src/platform.rs` | Everything OS-specific: ssh binary lookup, hidden console, open terminal / RDP / URL, ping |

### telepad-app (Model–View–Update)

| Path | Role |
|---|---|
| `src/state.rs` | `State { model: Model, drafts: Drafts }`, `HostState`, `Editor`, `ImportDialog`, `UpdateState`, `matches_filter` |
| `src/msg.rs` | `Msg` (user intents + results of effects) and `Effect` (work for the runtime) |
| `src/update.rs` | `update(&mut State, Msg, now) -> Vec<Effect>` — the only place the model changes |
| `src/tests.rs` | Behaviour tests for `update()` (no GUI, no database, no network) |

### telepad-gui

| Path | Role |
|---|---|
| `src/main.rs` | Arguments, `--open-ssh`; no console window in Windows release builds |
| `src/app.rs` | eframe app: each frame `runtime.frame()`, `views::draw()`, dispatch the returned `Msg`s |
| `src/runtime.rs` | Executes `Effect`s (database, ssh, collector, update threads, dialogs, clipboard, OS) and feeds results back as `Msg`s |
| `src/views/` | One module per screen: `panels` (top bar, status bar, host list), `overview`, `detail`, `sensors`, `docker`, `ports`, `editor`, `import`, `confirm`, `update_window` |
| `src/widgets.rs` | Shared drawing helpers and colours (bars, status dot, fan, thresholds) |
| `src/icons.rs` | Phosphor icon font glyphs and OS logo detection; `assets/` holds the font and logos |

### Data flow

```
          ┌──────── Msg (click, typing done, Tick) ────────┐
          │                                                ▼
 views::draw(&Model, &mut Drafts)            update(&mut State, Msg, now)
          ▲                                                │ Vec<Effect>
          │                                                ▼
        State ◄──── Msg (HostsLoaded, Collected, …) ─── Runtime
                                                   (db, ssh, collector, http, OS)
```

- **Views** read `&Model`, may edit `&mut Drafts` (form text, search filter, toggles —
  never I/O), and return `Msg`s. They never call core I/O, `platform`, `rfd` or threads.
- **`update()`** is the only code that changes the `Model`. It does no I/O, takes the time
  as a parameter, and returns `Effect`s. Every behaviour change gets a test in `tests.rs`.
- **The runtime** is the only GUI code that touches the database, ssh, network, files,
  clipboard or OS. Quick effects run inline and return a `Msg` immediately; slow ones
  (collection, update check/download) run on threads and report through channels that
  `Runtime::frame` drains. The UI thread never waits on ssh or the network.

New feature checklist: add the `Msg` (and `Effect` if it needs the outside world) → handle it
in `update()` with a test → run the effect in `runtime.rs` → draw/emit it in a view.

## Conventions

- Rust edition 2024; `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings` clean;
  `cargo test --workspace` green.
- No `unwrap()`/`expect()` on anything that depends on a host, the config or the network.
- **Read-only**: nothing the app runs may change a remote host. `sudo` only as `sudo -n`.
  One exception: `telepad-core/src/bmc_setup.sh` ("Set up BMC access…"), which only runs in an interactive
  terminal the user opened, prints the exact sudo commands, waits for Enter, and lets sudo ask
  for the password. It installs ipmitool and a read-only sudoers rule, nothing else.
- Remote script: POSIX `sh`, BusyBox-safe, every command guarded, `LC_ALL=C`.
- New metric = new section in `script.sh` + parser + fixture test (core) + view.
- OS-specific code only in `telepad-core/src/platform.rs` behind `#[cfg(target_os = ...)]`.
- Host fields are validated (`HostEntry::validate`) before saving or importing: single-line values,
  safe alias, no `Host`/`Match`/`Include` as per-host options — the DB is rendered into an ssh config.
- Schema changes: bump `SCHEMA_VERSION` in `store.rs` and migrate from the old `user_version`.
- Native file dialogs (`rfd`) are the one allowed blocking call on the UI thread (modal by nature);
  they are opened by the runtime, never by a view.
- Host values are passed to processes as separate arguments, never through a shell string.
- SSH: `BatchMode=yes` (never prompts), `StrictHostKeyChecking=accept-new`
  (trust on first use; changed keys still rejected).
- Self-update downloads only from `https://github.com/<repo>/releases/download/`, verifies the
  SHA-256 before anything is replaced, and never touches `telepad.db`, `config` or `.env`.
- Test data uses documentation addresses (`192.0.2.0/24`, `198.51.100.0/24`) and made-up
  host names — never real inventory.
- Conventional commits: `feat(collector): ...`, `fix(ui): ...`, referencing the issue (`#12`).
- Docs live in `README.md` (overview, install, quick start), `docs/USER_MANUAL.md` (every screen,
  button and message), `CONTRIBUTING.md` (build, workflow, releasing) and `SECURITY.md`. A change
  to the UI or behaviour updates the manual in the same PR.

## Configuration

| Item | Where |
|---|---|
| Host inventory | `telepad.db` next to `config` (else next to the exe) — or `--db <path>` / `TELEPAD_DB` |
| First-run import source | `./config` (OpenSSH format) — or `--config <path>` / `TELEPAD_CONFIG`; only read while the DB is empty |
| Generated ssh config | `telepad.ssh_config` next to the DB (do not edit; regenerated on every change) |
| Fallback key for hosts without IdentityFile | `SSH_IDENTITY_FILE` in `.env` next to `config` |
| ssh binary override | `TELEPAD_SSH` env var (default: Windows `System32\OpenSSH\ssh.exe`, else `ssh`) |
| Update source | `settings` table `update_enabled` / `update_repo` (☰ Hosts ▸ Software update…); default `yohannaftali/telepad` |

`.env`, `config`, `telepad.db` and `telepad.ssh_config` are git-ignored: never commit an inventory.

## Validate

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
cargo run --release --bin telepad-check -- <alias> [--json]
cargo run --release --bin telepad-check -- --check-update
cargo run --release --bin telepad   # GUI
```

## Release

Bump `version` in `Cargo.toml`, add a `CHANGE_HISTORY.md` entry, commit, push, then
push tag `v<version>`. `.github/workflows/release.yml` builds the Windows zip and
publishes it with its `.sha256` (`scripts/package-windows.ps1 -Publish` is the local
equivalent via `gh`).

## Tracked Issues
| ID | Title | Status | Last Checked |
|----|-------|--------|--------------|
