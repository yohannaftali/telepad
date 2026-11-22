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
| macOS aarch64 / x86_64 | released as `.dmg` (Telepad.app, ad-hoc signed, not notarized) and portable zip; CI; terminal launch via Terminal.app (less tested) |
| Linux x86_64 / aarch64 | released (built on Ubuntu 22.04, glibc 2.35+); CI; terminal launch via common emulators (less tested) |

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
| `src/store.rs` | SQLite inventory (`rusqlite`, bundled): `hosts` + `settings` + `db_connections` (saved database connections, no passwords) tables, `PRAGMA user_version` schema version, transactional import (Merge/Replace), `parse_pasted` (pasted config / JSON / `[alias] [user@]host[:port]` lines), JSON + OpenSSH export, generated `telepad.ssh_config` (atomic write) |
| `src/collector.rs` | Runs `ssh -F <config> -T -o BatchMode=yes ... <alias> sh -s` with `script.sh` on stdin; `Scope` (which tiers, ping first for hosts without ping reply) and `carry_over` of skipped facts; timeouts; bounded worker pool; sends `Update`s over a channel; RDP-only hosts get ping + TCP probe |
| `src/script.sh` | Remote read-only collection script, `===SECTION===` markers; tiers `TP_STATIC` (daily) / `TP_SLOW` (6 h) switched by `collector::Scope`, live sections every round |
| `src/bmc_setup.sh` | Interactive, user-started BMC (`ipmitool`) setup — see Conventions |
| `src/parse.rs` | Pure parsers for each section, unit-tested against `tests/fixtures/` |
| `src/authorized_keys.rs` | Authorized keys page backend: read-only `list` / `list_all` of `~/.ssh/authorized_keys` → `HostKeys` (`AuthKey`: line, options, key, supported, group from the `#` line above; login fingerprints), `parse_authorized_keys`, validate a pasted key (type, base64, embedded type), SHA256 fingerprint, add/revoke script for `~/.ssh/authorized_keys` (Add into a group: `INSERT_AWK`, `check_group`), lockout guard (`login_keys`), parallel `apply_all` |
| `src/db/` | Database browser (#52–#54): `detect` (DB containers / host listeners → `DbTarget`, user from container env), `tunnel` (`ssh -N -L`, closed on drop and by `close_all` on exit), `client` (`Session`: MySQL `mysql` / PostgreSQL `postgres` drivers, metadata, pages, queries, transactional row edits, DDL, cancel; live tests via `TELEPAD_TEST_MYSQL` / `TELEPAD_TEST_PG`), `sql` (quoting, statement split, danger / read-only checks, row SQL), `ddl` (structure editor SQL), `secret` (OS password store) |
| `src/files/` | File window (#62): `sftp` (own SFTP v3 client over `ssh -s <alias> sftp`: list, stat, pipelined download / upload, mkdir, rename, remove, setstat), `fallback` (hosts without SFTP: `sh` listing + `scp -O`), `rsync` (availability check, `-e ssh` args with `--protect-args`, dry run), `transfer` (`plan`: recursive walk + existing files, `run`: copy via `.telepad-part`, keep mtime), `ops` (#63: `Fs` trait for `LocalFs` / `Sftp`, `check_name`, new folder / file, rename, recursive delete / chmod / measure, `dir_mode` / `file_mode`), `edit` (#64: read / save with `Stamp` check, `.telepad-part`, links followed), `text` (#64: decode / encode keeping CRLF / BOM, find / replace incl. regex, go to line, indent), `syntax` (#64: language by name / `#!`, line lexer → `Tok` spans), local listing helpers; live tests via `TELEPAD_TEST_SSH` (`tests/sshd/` lab image) |
| `src/docker_actions.rs` | Docker/compose action commands + terminal script (show, confirm, run), read-only "How it runs" (`docker inspect` → `docker run`, compose files, secrets masked) |
| `src/cluster.rs` | Docker Swarm (managers) and Kubernetes (kubectl / k3s / microk8s) model + parsers for `===SWARM===` / `===KUBE===`; cluster key so several managers show one cluster |
| `src/hostsync.rs` | Host Management (#58): block-preserving OpenSSH config model (`ConfigFile`: managed single-alias blocks vs kept text), `compare` (rows by alias), `apply` (upsert / delete blocks, new hosts before `Host *` / `Match`), `line_diff`, `write` (only if unchanged since loading; timestamped `.bak`, atomic, permissions kept) |
| `src/eol.rs` | End-of-support dates per distribution release + date arithmetic (update when vendors publish new releases) |
| `src/model.rs` | Report data types (serde `Serialize`) and formatting helpers |
| `src/update.rs` | Self-update: GitHub `releases/latest` over HTTPS (`ureq`), SemVer check, SHA-256 from the release's `.sha256`, replace both exes (`self_replace`), relaunch |
| `src/platform.rs` | Everything OS-specific: ssh binary lookup, hidden console, open terminal / RDP / URL / folder, ping, owner-only files (`restrict_to_owner` by SID, `restrict_db_files`), Windows Credential Manager for RDP (#108: `CredWriteW` / `CredReadW` / `CredDeleteW` via `windows-sys` — the only `unsafe` in the project, Windows only, password never read back) |
| `src/history.rs` | `history.db` (#100): transfer / file operation / SQL log `Entry` (kind, host, target, title, detail, outcome, rows, bytes, duration, method), `History::open` (trim to 90 days / 50 000 rows), `add`, `list(Scope)`, `clear`, `mask_secrets` (password literals), `format_utc`, `to_csv`, `history_path` |
| `src/local_keys.rs` | My keys (#101): `scan` (~/.ssh + IdentityFile paths) / `inspect` → `LocalKey` (format, type, fingerprint, comment, passphrase, permission warning), `detect` / `load` (OpenSSH, PPK v2 / v3 incl. Argon2id / AES, PKCS#1, SEC1, PKCS#8 plain / encrypted, DER, PKCS#12, JWK), `convert(Target)` incl. public-only, `unsupported` per key type, `generate` (ssh-keygen into a private folder, `protect` with the passphrase, move into place, never overwrite), `convert_file`; own RSA conversion (`to_rsa`: ssh-key 0.6.7's passes `p` twice); live checks `TELEPAD_TEST_KEYGEN`, `TELEPAD_TEST_PPK_DIR` / `TELEPAD_TEST_PUTTYGEN` |
| `src/rdp.rs` | Remote Desktop settings (#102): `RdpSettings` (address, port, user / domain, `Screen`, multimon, colours, clipboard / drives / printers, `Audio`, admin, gateway, `AuthLevel`, autodetect, extra lines), `validate`, `render` (.rdp text) / `parse` (drops `password 51:b:`), `decode_file` / `encode_file` (UTF-16 LE BOM), `xfreerdp_args` |
| `src/prefs.rs` | `preference.db` (#94): named workspaces (`WorkspacePrefs`: `<own>` layout, custom (label, layout), `Current`), `check_label`, `prefs_path`, one-time `import_legacy` of `ui_workspace`, `PRAGMA user_version` |

### telepad-app (Model–View–Update)

| Path | Role |
|---|---|
| `src/state.rs` | `State { model: Model, drafts: Drafts }` (drafts include the overview `Sort` and the `KeysDraft` of the Authorized keys page (tab, server, open key), model the `KeyRun` results, `auth_keys` (`AkHost` per alias), `key_holders` and the `Workspace` `ws`), `OverviewPage`, `HostState` (+ `scope_at`: which tiers are due), `Editor`, `ImportDialog`, `UpdateState`, `matches_filter` |
| `src/msg.rs` | `Msg` (user intents + results of effects) and `Effect` (work for the runtime) |
| `src/table.rs` | Overview order: `Sort`/`SortKey`, `overview_rows` (filter + stable sort); `drop_index` / `can_reorder` (drag to reorder); `docker_stacks` (Docker overview by stack); `cluster_rows` (Kubernetes / Swarm pages); host list `host_list_groups`, `worst_usage`; `fullest_disk`, `hottest` |
| `src/update.rs` | `update(&mut State, Msg, now) -> Vec<Effect>` — the only place the model changes |
| `src/hostsync.rs` | Host Management window: `HostSync` (model: file, pending per side, conflicts, rows, preview), `HostSyncDraft` (filter, selection, `HostForm`), `HsMsg` / `HsEffect`, `update`, `refresh` |
| `src/tests_hostsync.rs` | Behaviour tests for Host Management |
| `src/db.rs` | Database windows: `DbSession` (model), `DbDraft` (form fields), `DbMsg` / `DbEffect` / `DbJob`, `update` for them, `post_statements` (grid edits → SQL) |
| `src/tests.rs` | Behaviour tests for `update()` (no GUI, no database, no network) |
| `src/files.rs` | File windows: `FileSession` (model: panes, transfers, overwrite question, operation `Dialog`, `op_errors`), `FileDraft` (paths, filters, selection, rsync toggle, dialog name / mode), `FmMsg` / `FmEffect` / `FileOp`, `op_blocked`, `update` |
| `src/tests_files.rs` | Behaviour tests for the file windows |
| `src/editor.rs` | Editor of a file window (#64): `FileEditor` (tabs, active, unsaved question), `EdTab` (path, language, line endings, `Stamp`, saved text, conflict), `EdDraft` (texts, find bar), `EdMsg`, `update` (open, save, conflict, close / unsaved) |
| `src/tests_editor.rs` | Behaviour tests for the editor |
| `src/tests_db.rs` | Behaviour tests for the database windows |
| `src/workspace.rs` | Main window workspace (#77, WinBox style): `Win` (page), `Group` (one inner window: tabs, rect, minimized / maximized), `Workspace` (`open` / `close`, `sync` with the sessions, `update(WsMsg)`, `save` / `restore` of a layout incl. `host:` / `files:` / `editor:` / `db:` session tabs (#99, `layout`, `pending`, `esc`), `open_windows` for the #92 picker), `WkMsg` (named workspaces #94: switch / add / save / delete; leave question #99: `LeaveYes` / `LeaveBack` / `AskQuit`, handled in `update.rs` with `leave_losses`), `tile_rects` / `stack_rects` (Arrange) |
| `src/tests_leave.rs` | Behaviour tests for #99 (windows reopened with a workspace, leave question, background transfers) |
| `src/tests_history.rs` | Behaviour tests for #100 (transfer / file operation / SQL log entries) |
| `src/tests_workspace.rs` | Behaviour tests for the workspace |
| `src/tests_keys.rs` | Behaviour tests for the Authorized keys page |
| `src/mykeys.rs` | My keys tab (#101): `MyKeys` (model), `MyKeysDraft` (selection, `GenForm`, `ConvertForm`; passphrases as `Secret`), `MkMsg` / `MkEffect`, `update`, `used_by` / `authorized_on` |
| `src/tests_mykeys.rs` | Behaviour tests for the My keys tab |

### telepad-gui

| Path | Role |
|---|---|
| `src/main.rs` | Arguments, `--open-ssh`; no console window in Windows release builds |
| `src/app.rs` | eframe app: each frame `runtime.frame()`, `views::draw()`, dispatch the returned `Msg`s |
| `src/runtime.rs` | Executes `Effect`s (database, ssh, collector, update threads, dialogs, clipboard read/write via `arboard`, OS) and feeds results back as `Msg`s |
| `src/views/` | One module per screen: `panels` (top bar, status bar), `workspace` (page sidebar, inner windows: title bar, tabs, move / resize / minimize / maximize, minimized strip), `overview`, `detail`, `sensors`, `docker`, `docker_overview`, `ports`, `editor`, `import`, `keys`, `database` (one inner window per connection), `files` (one inner window per host), `text_editor` (its editor window: tabs, gutter, colours, find bar), `host_sync`, `confirm`, `update_window`. Only the main OS window exists; pages and dialogs are drawn inside it |
| `src/widgets.rs` | Shared drawing helpers and colours (bars, status dot, fan, thresholds, action buttons `Act`: red danger / yellow edit / green add / amber caution, `host_menu` (right-click host menu), `light_visuals`) |
| `src/icons.rs` | Phosphor icon font glyphs and OS logo detection; `assets/` holds the font and logos |
| `assets/icon/` | App icon (`telepad.png` window icon, `.ico` exe/installer, `.icns` macOS app), generated by `scripts/make-icon.py` |
| `build.rs` | Windows only: embeds `telepad.ico` and version info into `telepad.exe` (`winresource`) |

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
  Two exceptions, both only in an interactive terminal the user opened, never in the background:
  `telepad-core/src/bmc_setup.sh` ("Set up BMC access…") prints the exact sudo commands, waits for
  Enter and lets sudo ask for the password (installs ipmitool and a read-only sudoers rule, nothing
  else); and Docker / compose / Swarm / Kubernetes actions (`docker_actions.rs`, owner decision on #26): the terminal
  prints the exact command and runs it only after Enter (`yes` for kill / remove / compose down).
  Container IDs, project names and paths from labels are validated and single-quoted.
  Portainer-managed stacks are never changed from Telepad (Open in Portainer only).
  Third exception, in-app (owner decision on #49): **Authorized keys** (`authorized_keys.rs`; #78
  lists each host's `~/.ssh/authorized_keys` read-only, `cat` only) adds or
  removes one validated public key in the login user's `~/.ssh/authorized_keys` on the chosen
  hosts, only after the user typed `add` / `revoke`; never from a refresh. It refuses to revoke
  the key Telepad logs in with, touches no other line, and rewrites the file via a temp copy.
  Fourth exception, in-app (owner decision on #52–#54): the **database browser** (`db/`) changes
  database rows and structure only after the user saw the exact SQL and clicked Run (DROP /
  TRUNCATE: table name typed); grid edits post in one transaction, one row per statement. It
  reaches databases only through `ssh -N -L` (never a direct database port), changes nothing on
  the host, and stores passwords only in the OS password store (`keyring`), never in `telepad.db`.
  New connections are editable; Read-only is a per-connection switch.
  Fifth exception, in-app (owner decision on #62): the **file window** (`files/`) copies files
  to or from the host only on a user action (Upload / Download, F5, drag and drop), over the
  login's SFTP subsystem (fallback `scp -O`; rsync with a dry run first, no `--delete`); never
  `sudo`. Existing files are replaced only after Overwrite / Newer only; writes go to
  `<name>.telepad-part` and are renamed when complete; uploads never send Windows modes
  (`scp` without `-p`). File operations (#63, `files/ops.rs`): new folder / file, rename,
  delete, permissions only from the window's dialogs (delete confirmed with the item list);
  names checked as one path element, renames never replace, links never followed.
  Editor (#64, `files/edit.rs`): writes only on Save, via `<name>.telepad-part` + rename with
  the file's mode (owner when allowed), and only if size / mtime still match what was loaded
  (else Overwrite / Reload / Show diff).
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
  SHA-256 before anything is replaced, and never touches `telepad.db`, `preference.db`, `history.db`, `config` or `.env`.
- UI text may use any character, but the GUI test `every_character_in_the_ui_has_a_glyph` must
  pass (proportional text falls back to Hack; Phosphor holds the icons).
- Test data uses documentation addresses (`192.0.2.0/24`, `198.51.100.0/24`) and made-up
  host names — never real inventory.
- Conventional commits: `feat(collector): ...`, `fix(ui): ...`, referencing the issue (`#12`).
- Docs live in `README.md` (overview, install, quick start), `docs/USER_MANUAL.md` (every screen,
  button and message), `CONTRIBUTING.md` (build, workflow, releasing) and `SECURITY.md`. A change
  to the UI or behaviour updates the manual in the same PR.

## Configuration

| Item | Where |
|---|---|
| Host inventory | `--db <path>` / `TELEPAD_DB`, else `telepad.db` next to `config`, else next to the exe when it is portable (`portable` marker or existing db), else `platform::user_data_dir()` (`lib.rs: data_dir`) |
| First-run import source | `./config` (OpenSSH format) — or `--config <path>` / `TELEPAD_CONFIG`; only read while the DB is empty |
| Generated ssh config | `telepad.ssh_config` next to the DB (do not edit; regenerated on every change) |
| Fallback key for hosts without IdentityFile | `SSH_IDENTITY_FILE` in `.env` next to `config` |
| ssh binary override | `TELEPAD_SSH` env var (default: Windows `System32\OpenSSH\ssh.exe`, else `ssh`) |
| View settings | `settings` table `ui_overview_cards`, `ui_theme`, `ui_sidebar_collapsed`; `ui_workspace` is only read once (imported into `preference.db` as `<own>`, #94); `ui_compact_host_list` / `ui_group_by_status` are kept but unused since the host list went (#77) |
| History (#100) | `history.db` next to `telepad.db` (`--history <path>` / `TELEPAD_HISTORY`): `entries` table, one writer / reader thread in the runtime, flushed on exit; file windows (History tab) and database windows (SQL log) load their scope on open |
| Workspaces (#94) | `preference.db` next to `telepad.db` (`--prefs <path>` / `TELEPAD_PREFS`): `workspaces` table (label, kind `own` / `custom`, layout `x,y,w,h|minimized|maximized|active|tab;tab` per window, position, updated) + `prefs` (`current`, `legacy_imported`); written by a writer thread, autosave ≤ 1 / s, flushed on exit |
| File window folders | `settings` table `files_paths:<alias>` (`local
remote`, last folders per host) |
| Host Management file | `settings` table `hostsync_config_path` (default `~/.ssh/config`); writes keep a `config.telepad-*.bak` next to it |
| Remote Desktop settings (#102) | `rdp_settings` table (alias → JSON, `ON UPDATE / DELETE CASCADE` with `hosts`), schema 4; in JSON export / import as `rdp`; Open RDP: `platform::open_rdp_with` (temporary owner-only `.rdp` → mstsc / `open`; Linux `xfreerdp` args); never a password |
| Database connections | `db_connections` table (user, database, save-password and read-only flags per `DbTarget::key`); passwords in the OS password store (service "Telepad database") |
| Update source | `settings` table `update_enabled` / `update_repo` / `update_auto_install` (☰ ▸ Software update…); check via the github.com `releases/latest` redirect (API only as fallback: 60 req/h per IP); default `yohannaftali/telepad` |

`.env`, `config`, `telepad.db`, `preference.db`, `history.db` and `telepad.ssh_config` are git-ignored: never commit an inventory.

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
push tag `v<version>`. `.github/workflows/release.yml` builds `telepad-<version>-<os>-<arch>.zip`
for windows-x86_64, linux-x86_64, linux-aarch64, macos-aarch64 and macos-x86_64
(`scripts/package-windows.ps1`, `scripts/package-unix.sh`), checks each, and publishes them
all with `.sha256` files in one release. Zip names must match `update::asset_name`; on Unix
the updater sets the execute bit on `telepad-check` (`platform::make_executable`).

## Tracked Issues
| ID | Title | Status | Last Checked |
|----|-------|--------|--------------|
| #4 | feat(ui): overview table — sortable, resizable columns | closed | 2026-11-17 |
| #5 | fix(ui): host detail header buttons cut off on narrow windows | closed | 2026-11-17 |
| #6 | fix(ui): "→" renders as a box in table headers | closed | 2026-11-17 |
| #7 | fix(release): Windows .sha256 has CRLF | closed | 2026-11-17 |
| #8 | feat(ui): compact host list for large inventories | closed | 2026-11-17 |
| #16 | feat(ui): overview cards, bigger Overview button, Add host in menu | closed | 2026-11-17 |
| #10 | feat(release): Windows installer alongside the portable zip | closed | 2026-11-17 |
| #21 | feat(ui): Telepad app icon | closed | 2026-11-17 |
| #23 | fix(ui): IdentityFile Browse cannot reach ~/.ssh on macOS | closed | 2026-11-17 |
| #9 | feat(release): macOS Telepad.app in a .dmg (unsigned) | closed | 2026-11-17 |
| #25 | feat(collector): overview flags for reboot needed, pending updates and end-of-life OS | closed | 2026-11-17 |
| #26 | feat(docker): container shell, actions and compose/run command view | closed | 2026-11-17 |
| #27 | feat(ui): light / dark / system theme switch | closed | 2026-11-17 |
| #28 | feat(cluster): Docker Swarm and Kubernetes panel | closed | 2026-11-17 |
| #34 | fix(update): update check fails behind a shared IP (GitHub API rate limit) | closed | 2026-11-18 |
| #36 | perf(collector): tiered collection — live, 6-hourly, daily and ping-only checks | closed | 2026-11-18 |
| #37 | feat(ui): Overview — Docker page grouping containers of all hosts by stack | closed | 2026-11-18 |
| #38 | fix(ui): overview buttons look flat in light and dark themes | closed | 2026-11-18 |
| #41 | fix(ui): Hosts menu as icon-only hamburger at the top right | closed | 2026-11-18 |
| #42 | feat(import): import hosts from pasted plain text | closed | 2026-11-18 |
| #43 | feat(ui): reorder hosts by drag and drop in host list and overview table | closed | 2026-11-18 |
| #45 | fix(store): Merge import puts new hosts between existing ones | closed | 2026-11-18 |
| #49 | feat(keys): add or revoke an SSH public key on selected servers | closed | 2026-11-18 |
| #52 | feat(db): HeidiSQL-style database browser — connect, browse tables and run queries | closed | 2026-11-19 |
| #53 | feat(db): edit, insert and delete rows in the database Data grid | closed | 2026-11-19 |
| #54 | feat(db): edit table structure — columns, indexes, foreign keys | closed | 2026-11-19 |
| #58 | feat(hosts): Host Management window — compare and sync Telepad hosts with an OpenSSH config | closed | 2026-11-19 |
| #62 | feat(files): WinSCP-style file window — local and remote panes with transfers | closed | 2026-11-19 |
| #63 | feat(files): file operations — new, rename, delete, permissions | closed | 2026-11-20 |
| #64 | feat(files): built-in VS Code-like editor for local and remote files | closed | 2026-11-20 |
| #71 | fix(ui): copy and paste do not reach the system clipboard in text fields | closed | 2026-11-20 |
| #76 | feat(ui): colour action buttons by risk — red delete / revoke, yellow edit, green add | closed | 2026-11-20 |
| #77 | feat(ui): WinBox-style single window — page sidebar, inner windows (move, resize, minimize, maximize, tabs), in-window dialogs | closed | 2026-11-20 |
| #78 | feat(keys): Authorized keys page — list keys by server and by public key, add / revoke | closed | 2026-11-20 |
| #83 | feat(ui): Hosts page polish — full-width table, auto-fit columns, search in page, hover, SSH / Files buttons, sidebar cleanup | closed | 2026-11-20 |
| #84 | feat(docker): Docker / Podman page name and Kubernetes / Docker Swarm pages | closed | 2026-11-20 |
| #85 | feat(keys): key groups from authorized_keys comments, add into a group, edit the file in the editor | closed | 2026-11-20 |
| #86 | fix(ui): light theme too light — clearer backgrounds, window borders and buttons | closed | 2026-11-20 |
| #92 | feat(ui): searchable open-windows picker in the top bar, Theme moves to ☰ Settings | closed | 2026-11-21 |
| #93 | feat(ui): add / import / export hosts on the Host Management page, slimmer ☰ menu | closed | 2026-11-21 |
| #94 | feat(ui): named workspaces (WinBox style) saved in preference.db, window arrange (tile, stack, minimize all) | closed | 2026-11-21 |
| #99 | feat(ui): workspaces reopen file, database and editor windows; reset unsaved settings on leave, transfers keep running | closed | 2026-11-22 |
| #100 | feat(history): log file transfers and database commits (HeidiSQL-style SQL log) | closed | 2026-11-22 |
| #101 | feat(keys): My keys tab — list local keys, generate key pairs, convert between OpenSSH, PuTTY and other formats | closed | 2026-11-22 |
| #102 | feat(rdp): saved Remote Desktop settings per host, open with a generated .rdp, import / export .rdp | closed | 2026-11-22 |
| #107 | feat(keys): OpenPGP export and import in My keys | open | 2026-11-22 |
| #108 | feat(rdp): save the Remote Desktop password in Windows Credential Manager (optional) | open | 2026-11-22 |
| #110 | fix(store): create history.db, preference.db and telepad.db readable by the owner only (macOS / Linux) | open | 2026-11-22 |
| #111 | fix(history): neutralise formula cells in the CSV export | open | 2026-11-22 |
| #112 | fix(keys): restrict the generated key before moving it; ACL by SID, permission failure as a warning | open | 2026-11-22 |
