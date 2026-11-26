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
| `src/config.rs` | OpenSSH config import/export: hosts (alias, HostName, User, Port, IdentityFile, description, other options, not-monitored tag, RDP-only tag, `# telepad: group` / `# telepad: cluster` tags (#144, #145); `SetKind`, `HostSet`, `check_set_name`) + global `Host *`/`Match` text; renders the file `ssh -F` reads |
| `src/store.rs` | SQLite inventory (`rusqlite`, bundled): `hosts` (incl. `grp` / `cluster` names, #144 / #145) + `settings` + `db_connections` (saved database connections, no passwords) + `host_sets` (kind, name, description, position; schema 5: `sets` / `save_set` (rename moves hosts) / `delete_set` / `order_sets` / `assign_set`) + `portainers` (name, alias → `hosts` ON UPDATE / DELETE CASCADE, port, https, fingerprint, position; schema 6, #171: `portainers` / `save_portainer` / `delete_portainer` / `pin_portainer`; JSON `portainers`, never a token) tables, `PRAGMA user_version` schema version + `schema_meta.min_compatible` (#180: `check_newer_schema` read-only first, `NewerSchema` error, compatible newer files open untouched, `newer_schema()`), transactional import (Merge/Replace; Replace deletes only hosts missing from the import), `parse_pasted` (pasted config / JSON / `[alias] [user@]host[:port]` lines), JSON + OpenSSH export, generated `telepad.ssh_config` (atomic write) |
| `src/collector.rs` | Runs `ssh -F <config> -T -o BatchMode=yes ... <alias> sh -s` with `script.sh` on stdin; `Scope` (which tiers, ping first for hosts without ping reply; intervals from `strategy::Strategy`, #122) and `carry_over` of skipped facts; timeouts; bounded worker pool; sends `Update`s over a channel; RDP-only hosts get ping + TCP probe; `reach` (#125: ping, else a TCP connect to the SSH port → `Reach`; `Indirect` for `ProxyJump` / `ProxyCommand` hosts) after a failed SSH attempt, in the ping-first path and (down hosts only, `PingJob::port`) in the ping round |
| `src/script.sh` | Remote read-only collection script, `===SECTION===` markers; tiers `TP_STATIC` (daily by default) / `TP_SLOW` (6 h by default) switched by `collector::Scope`, live sections every round; pending updates only from the local package cache (never `apt-get update`) |
| `src/bmc_setup.sh` | Interactive, user-started BMC (`ipmitool`) setup — see Conventions |
| `src/parse.rs` | Pure parsers for each section, unit-tested against `tests/fixtures/` |
| `src/authorized_keys.rs` | Authorized keys page backend: read-only `list` / `list_all` of `~/.ssh/authorized_keys` → `HostKeys` (`AuthKey`: line, options, key, supported, group from the `#` line above; login fingerprints), `parse_authorized_keys`, validate a pasted key (type, base64, embedded type), SHA256 fingerprint, add/revoke script for `~/.ssh/authorized_keys` (Add into a group: `INSERT_AWK`, `check_group`), lockout guard (`login_keys`, public key from the private key when no `.pub` exists, #126), parallel `apply_all` |
| `src/db/` | Database browser (#52–#54): `detect` (DB containers / host listeners → `DbTarget`, user from container env), `tunnel` (`ssh -N -L`, closed on drop and by `close_all` on exit), `client` (`Session`: MySQL `mysql` / PostgreSQL `postgres` drivers, metadata, pages, queries, transactional row edits, DDL, cancel; live tests via `TELEPAD_TEST_MYSQL` / `TELEPAD_TEST_PG`), `sql` (quoting, statement split, danger / read-only checks, row SQL), `ddl` (structure editor SQL), `secret` (OS password store), `io` (#139: `Exporter` CSV / SQL / `.xlsx` via `.telepad-part`, `read_import` CSV / Excel (`calamine`) / SQL INSERTs only, `auto_map`, `import_statements`); `sql::filter_condition` / `data_where` / `count_rows` (#138 filter row, wtbapp rules); `Session::count`, `Session::import` (one transaction, error names the row) |
| `src/files/` | File window (#62; `transfer::Remote` trait: `Sftp` or `ThisComputer` for My Computer #129; `transfer::Options` = Transfer settings #131: `TransferMode` Default / Text / Binary, `convert_eol`, `newer_only`, `exclude` globs; `compare` #131: `compare` (Diff per name), `sync_plan` (`SyncRequest` → `SyncPlan`), `line_marks`): `sftp` (own SFTP v3 client over `ssh -s <alias> sftp`: list, stat, pipelined download / upload, mkdir, rename, remove, setstat), `fallback` (hosts without SFTP: `sh` listing + `scp -O`), `rsync` (availability check, `-e ssh` args with `--protect-args`, dry run), `transfer` (`plan`: recursive walk + existing files, `run`: copy via `.telepad-part`, keep mtime), `ops` (#63: `Fs` trait for `LocalFs` / `Sftp`, `check_name`, new folder / file, rename, recursive delete / chmod / measure, `dir_mode` / `file_mode`), `edit` (#64: read / save with `Stamp` check, `.telepad-part`, links followed), `text` (#64: decode / encode keeping CRLF / BOM, find / replace incl. regex, go to line, indent), `syntax` (#64: language by name / `#!`, line lexer → `Tok` spans), local listing helpers; live tests via `TELEPAD_TEST_SSH` (`tests/sshd/` lab image) |
| `src/docker_actions.rs` | Docker/compose action commands + terminal script (show, confirm, run), read-only "How it runs" (`docker inspect` → `docker run`, compose files, secrets masked) |
| `src/cluster.rs` | Docker Swarm (managers) and Kubernetes (kubectl / k3s / microk8s) model + parsers for `===SWARM===` / `===KUBE===`; cluster key so several managers show one cluster |
| `src/hostsync.rs` | Host Management (#58): block-preserving OpenSSH config model (`ConfigFile`: managed single-alias blocks vs kept text), `compare` (rows by alias), `apply` (upsert / delete blocks, new hosts before `Host *` / `Match`), `line_diff`, `write` (only if unchanged since loading; timestamped `.bak`, atomic, permissions kept) |
| `src/eol.rs` | End-of-support dates per distribution release + date arithmetic (update when vendors publish new releases) |
| `src/model.rs` | Report data types (serde `Serialize`) and formatting helpers |
| `src/update.rs` | Self-update: GitHub `releases/latest` over HTTPS (`ureq`), SemVer check, SHA-256 from the release's `.sha256`, replace both exes (`self_replace`), relaunch |
| `src/platform.rs` | Everything OS-specific: ssh binary lookup, hidden console, open terminal / RDP / URL / folder, ping, owner-only files (`restrict_to_owner` by SID, `restrict_db_files`), Windows Credential Manager for RDP (#108: `CredWriteW` / `CredReadW` / `CredDeleteW` via `windows-sys` — the only `unsafe` in the project, Windows only, password never read back) |
| `src/history.rs` | `history.db` (#100): transfer / file operation / SQL log `Entry` (kind, host, target, title, detail, outcome, rows, bytes, duration, method), `History::open` (trim to 90 days / 50 000 rows), `add`, `list(Scope)`, `clear`, `mask_secrets` (password literals), `format_utc`, `to_csv`, `history_path` |
| `src/local_keys.rs` | My keys (#101): `scan` (~/.ssh + IdentityFile paths) / `inspect` → `LocalKey` (format, type, fingerprint, comment, passphrase, permission warning), `detect` / `load` (OpenSSH, PPK v2 / v3 incl. Argon2id / AES, PKCS#1, SEC1, PKCS#8 plain / encrypted, DER, PKCS#12, JWK, OpenPGP armoured / binary via `pgp` (#107)), `convert(Target)` incl. public-only, `unsupported` per key type, `generate` (ssh-keygen into a private folder, `protect` with the passphrase, move into place, never overwrite), `convert_file`; own RSA conversion (`to_rsa`: ssh-key 0.6.7's passes `p` twice); live checks `TELEPAD_TEST_KEYGEN`, `TELEPAD_TEST_PPK_DIR` (also writes `.asc` samples for GnuPG) / `TELEPAD_TEST_PUTTYGEN` / `TELEPAD_TEST_GPG` |
| `src/rdp.rs` | Remote Desktop settings (#102): `RdpSettings` (address, port, user / domain, `Screen`, multimon, colours, clipboard / drives / printers, `Audio`, admin, gateway, `AuthLevel`, autodetect, extra lines), `validate`, `render` (.rdp text) / `parse` (drops `password 51:b:`), `decode_file` / `encode_file` (UTF-16 LE BOM), `xfreerdp_args` |
| `src/strategy.rs` | `update.db` (#122): `Strategy` (paused, `Every` on / seconds for ping, live, slow, static; SSH retry), `Check` (title, allowed `range`), `validate`, `human_secs`; `StaticFacts` / `SlowFacts` (`of` / `apply`, also used by `collector::Scope::carry_over`), `Facts`, `SavedFacts` (with age); `UpdateDb` (`open`, `load(now)` → `Loaded`, `load_strategy` / `save_strategy`, `save_facts` / `load_facts(now)`, `set_host_paused` / `paused_hosts`, `prune`), `strategy_path`, `PRAGMA user_version` |
| `src/login_log.rs` | Log info (#126): read-only `script` (identity, `$SSH_CLIENT`, ancestor pids, journal access probe, `journalctl -t sshd -t sshd-session --since -30d` `Accepted` lines capped at `LIMIT`, plus `authorized_keys::list_script`), `parse` / `parse_line` (short-iso times incl. `+07:00`, publickey / password / keyboard-interactive / certificates, IPv6) → `LoginLog` (`JournalAccess`, `this_session`: the key sshd accepted for this read, `own` keys), `read`, `register_script` / `unregister_script` / `terminal_args` (exact sudo command after Enter) |
| `src/local.rs` | My Computer (#129): `collect` this computer without SSH (Windows: `sysinfo`, `netstat -ano` → `parse_netstat`, `reg query` reboot pending, `docker` / `podman` CLI in the remote script's formats → `parse::parse_docker`; Linux / macOS: `script.sh` via local `sh -s`), `db_targets` (127.0.0.1 only), `open_terminal` (`platform::open_local_shell`) |
| `src/table_export.rs` | Table exports (#153): `TableData` (title, columns, rows as text), `Format` (CSV / Excel / PDF / JSON), `to_csv` (BOM, formulas defused as #111), `to_json` (objects in column order, repeated names numbered), `to_xlsx` (`rust_xlsxwriter`: bold frozen header, autofilter, plain numbers only), `pdf::to_pdf` (`pdf-writer` + `ttf-parser`: A4 / landscape, header per page, embedded TrueType fonts with ToUnicode, per-character font fallback), `render`, `write` (via `.telepad-part`), `file_name`, `PDF_ASK_ABOVE` |
| `src/portainer.rs` | Portainers (#171; changes #173: `Conn` (tunnel + token) with `stack` (`StackDetail` + compose file), `stack_containers` (`StackContainer`, Docker proxy, `containers_path`), `act(Action)` (`Action::StackStart` / `StackStop` / `StackDelete` / `Container { op: ContainerOp }`, `request()` = method + path), `update_stack(StackUpdate, expect UpdateDate)` → `ReadError::Changed`; `stack_locked`; `Stack::state` incl. deploying / error): `Portainer` (name, alias, port, https, pinned `fingerprint`), `check_name`, tokens in the OS password store (`load_token` / `save_token` / `forget_token`, service "Telepad Portainer"), `read` (tunnel via `db::tunnel`, own HTTP/1.1 `GET` client, TLS via `rustls` with a trust-on-first-use `Pin` verifier → `ReadResult` with the certificate seen), `parse_status` / `parse_endpoints` / `parse_stacks` (fixtures in `tests/fixtures/portainer/`, stack `Env` never read), `version_ok` (`MIN_VERSION` 2.11.0), `ReadError`, `env_host` (environment → Telepad host), `env_url` / `stack_url` |
| `src/prefs.rs` | `preference.db` (#94): named workspaces (`WorkspacePrefs`: `<own>` layout, custom (label, layout), `Current`), `check_label`, `prefs_path`, one-time `import_legacy` of `ui_workspace`, `PRAGMA user_version` |

### telepad-app (Model–View–Update)

| Path | Role |
|---|---|
| `src/state.rs` | `State { model: Model, drafts: Drafts }` (drafts include the overview `Sort` and the `KeysDraft` of the Authorized keys page (tab, server, open key), model the `KeyRun` results, `auth_keys` (`AkHost` per alias), `key_holders` and the `Workspace` `ws`), `OverviewPage`, `HostState` (+ `scope_at(now, &Strategy)`: which tiers are on and due; `Seen` times that may predate this run; `known` facts from `update.db`; ping result / misses), `StrategyDraft`, `Editor`, `ImportDialog`, `UpdateState`, `matches_filter` |
| `src/msg.rs` | `Msg` (user intents + results of effects) and `Effect` (work for the runtime) |
| `src/table.rs` | Overview order: `Sort`/`SortKey` (incl. `Group` / `Cluster`, #157), `overview_rows` (filter + stable sort); `drop_index` / `can_reorder` (drag to reorder); `docker_stacks` (Docker overview by stack); `cluster_rows` (Kubernetes / Swarm pages); host list `host_list_groups`, `worst_usage`; `fullest_disk`, `hottest` |
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
| `src/workspace.rs` | Main window workspace (#77, WinBox style): `Win` (page), `Group` (one inner window: tabs, rect, minimized / maximized), `Workspace` (`open` / `close`, `sync` with the sessions, `update(WsMsg)`, `save` / `restore` of a layout incl. `host:` / `files:` / `editor:` / `db:` session tabs (#99, `layout`, `pending`, `esc`), `open_windows` for the #92 picker), `WkMsg` (named workspaces #94: switch / add / save / delete; leave question #99: `LeaveYes` / `LeaveBack` / `AskQuit`, handled in `update.rs` with `leave_losses`), `tile_rects` / `stack_rects` (Arrange); snap layouts (#168): `Zone`, `SNAP_LAYOUTS`, `ZONE_MAX`, `zone_rect`, `edge_zone`, `WsMsg::Snap`, `Group::unsnap` |
| `src/tests_leave.rs` | Behaviour tests for #99 (windows reopened with a workspace, leave question, background transfers) |
| `src/tests_history.rs` | Behaviour tests for #100 (transfer / file operation / SQL log entries) |
| `src/tests_workspace.rs` | Behaviour tests for the workspace |
| `src/tests_keys.rs` | Behaviour tests for the Authorized keys page |
| `src/mykeys.rs` | My keys tab (#101): `MyKeys` (model), `MyKeysDraft` (selection, `GenForm`, `ConvertForm`; passphrases as `Secret`), `MkMsg` / `MkEffect`, `update`, `used_by` / `authorized_on` |
| `src/tests_mykeys.rs` | Behaviour tests for the My keys tab |
| `src/grid.rs` | Table state of the table component (#153): `TableState` (sort `(col, Dir)`, filter boxes, `Pager`; `click`, `filter_by`, `clear_filters`), `filter_match` (wtbapp rules, numbers with units via `value`), `natural` / `compare` (units, empty last), `view` (filter + stable sort), `data` (export rows), `TableMsg` (Export / Answer / Exported); value dropdown (#170): `distinct_values` (`VALUES_CAP`), `search_values`, `exact_filter`, `row_matches_except` |
| `src/tests_grid.rs` | Behaviour tests for #153 |
| `src/pager.rs` | Table pager (#138, #140, Senar / wtbapp style): `Pager` (page, typed rows per page — empty = 100, above 100 asks — `go(Go)`, `apply_page`, `record_info` "Record [a - b] of n"; continuous mode #152: `continuous`, `loaded`, `toggle`, `start_mode` (narrower than `NARROW` starts continuous), `more`, `wants_more` (auto-fill up to `FILL_MAX`), `size_label`); drawn by `widgets::pager_bar` (record info left, Rows per page / page buttons / ∞ toggle right); database grid appends via `DbMsg::DataMore` (`DataView::appending` / `exhausted`) |
| `src/logins.rs` | Log info pages (#126): `LoginSession` (model), `LoginDraft` (sort asc / desc / newest first, per-column filters, page, typed rows per page, show this PC's logins), `rows` (key names from `authorized_keys` / own keys / My keys, `Who`), `filter_match` (wtbapp rules), `page_range`, `LgMsg` / `LgEffect` (handled in `update.rs` `logins`, restored as `logins:<alias>`) |
| `src/tests_logins.rs` | Behaviour tests for #126 |
| `src/tests_strategy.rs` | Behaviour tests for #122 (pause, Update Strategy page, ping rounds, last known facts) |
| `src/tests_local.rs` | Behaviour tests for #129 (My Computer page, local reads, local file and database windows) |
| `src/sets.rs` | Groups (#144) and clusters (#145): `SetPage` ("Hosts - [name]", `""` = Ungrouped / No cluster), `members`, `stats`, `SetForm`, `AddHosts`, `StMsg::MoveTo` (drag and drop, #160), `SetPage::parent` / `back_target` / `StMsg::Back` (#169), `SetsDraft` (incl. Hosts page `hosts_columns`, Group and Cluster by default; no "By group" since #157), `StMsg` / `StEffect`, `update` (restored as `group:<name>` / `cluster:<name>`) |
| `src/tests_sets.rs` | Behaviour tests for #144 / #145 (and #163 / #169: set links, Back) |
| `src/portainer.rs` | Portainers (#171; #173: `PtPage::stack` / `containers` / `busy`, `PtConfirm` + `needs_confirm`, `StackEditor` = the Stack page (#176: `StackTab` Containers / Editor, `containers`, `busy`, `stack` summary; `OpenStack`, `OpenStackFor` by host + name via `stack_refs`, `pt_pending_open`, chooser `PtDraft::choose`; actions carry an `Origin` page / stack) / `EditorDraft` (`Win::PtStack`; variables on `Win::PtVars`: `OpenVars`, `VarsFile` / `VarsPaste` through `portainer::parse_env_text` / `merge_env`, `PtEffect::PickEnvFile`), `var_changes`, `check_var_name`, `editor_dirty`, effects `Containers` / `Act` / `LoadStack` / `Update`): `PtRow` (registered `Portainer`, `PtStatus`, last good `Snapshot`, checked, reading), `PtPage` ("Portainer - [name]", chosen environment), `PtForm` / `PtDraft`, `PtMsg` / `PtEffect` (Save with an optional new token, Delete, Read through a host, Test, Pin), `update` (reads on open / Refresh only; first certificate pinned; restored as `portainer:<name>`), `suggestions` (hosts running `portainer/portainer` with the published port) |
| `src/tests_portainer.rs` | Behaviour tests for #171 |

### telepad-gui

| Path | Role |
|---|---|
| `src/main.rs` | Arguments, `--open-ssh`; no console window in Windows release builds |
| `src/app.rs` | eframe app: each frame `runtime.frame()`, `views::draw()`, dispatch the returned `Msg`s |
| `src/runtime.rs` | Executes `Effect`s (database, ssh, collector, update threads, dialogs, clipboard read/write via `arboard`, OS) and feeds results back as `Msg`s |
| `src/views/` | One module per screen: `panels` (top bar, status bar), `detail` (host page and `local_page`, My Computer #129), `workspace` (page sidebar, inner windows: title bar, tabs, move / resize / minimize / maximize, minimized strip), `overview` (`TableView`: one group / cluster, extra Group / Cluster columns, by group; `fit_widths` auto column widths #143), `sets` (Groups / Clusters pages and "Hosts - [name]", #144 / #145), `portainer` (Portainers page, Portainer - [name], form / delete dialogs, #171), `detail`, `sensors`, `docker`, `docker_overview`, `ports`, `editor`, `import`, `keys`, `strategy` (Update Strategy page, #122), `logins` (Log info page, #126), `database` (one inner window per connection), `files` (one inner window per host), `text_editor` (its editor window: tabs, gutter, colours, find bar; `embedded` = the same editor for a text that is not a file, e.g. the Portainer stack editor #173), `host_sync`, `confirm`, `update_window`. Only the main OS window exists; pages and dialogs are drawn inside it |
| `src/table.rs` | The table component (#153) every table uses: `Col` (title, width, sort / filter / export, `named` for icon columns), `Spec` (`Kind::Simple` / `Fill` / `Paged`, row height, selection, drag, min width), `show` (in memory: `TableState` in `Drafts::tables`) and `controlled` (the view keeps the order: Hosts, file panes, database grid, Log info) → `Output`; text-fitted columns fill the page (`fill_widths`, kept `Layout`: dragged widths, double-click / Auto-fit, button columns' content width, #156); header band and hover highlight (`Palette::head`, #151), title left / sort caret right-aligned, filter row above the titles with a trailing funnel that opens the column's value dropdown (#170, `Controlled::all`), header click sort, right-click menu = `Ev::Menu` own items + Filter by / Clear All Filters / Export CSV, Excel, PDF / Copy as JSON (`export_msg`, `export_rows`) |
| `src/widgets.rs` | Shared drawing helpers and colours (bars, status dot, fan, thresholds, action buttons `Act`: red danger / yellow edit / green add / amber caution, `host_menu` (right-click host menu), `light_visuals` (blue-grey slate scale, #167) / `dark_visuals` (navy selection), `Palette::title` (focused / unfocused title bar) |
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

- **My Computer** (#129): `Win::MyComputer` (layout key `local`) shows `Model::local` (a `HostState` with
  `HostEntry::local()`, alias `config::LOCAL_ALIAS` = "This computer", which no inventory alias can be);
  read by `Effect::CollectLocal` while the page is open (`update.rs` `local_due`). File and database
  windows find it through `update.rs` `host_of`; its file window is `files::Mode::Local` (both panes
  this computer, `FileSession::local_side`, transfers through `transfer::Remote` = `ThisComputer`).
- **Tables** (#153): build every new table with `crate::table` (`show` for rows in memory, `controlled`
  when the view keeps its own sort / filter state) — never a bare `TableBuilder` / `egui::Grid` with a
  header row. Sort, filter boxes and page live in `Drafts::tables` (key per table); exports go through
  `Msg::Table` → `Effect::ExportTable` / `Effect::CopyText`.
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
  The **Portainers** pages (#171, owner decisions on #171) reach a Portainer only through
  `ssh -N -L` to the host it runs on (never a direct network call), with an API access token
  from the OS password store; its certificate is pinned on first use. Reads are `GET` only.
  Exception (owner decision on #173): stacks and their containers are changed **only through
  the Portainer API** with the user's token, from a button the user clicked — stack start /
  stop / delete (confirmed, delete needs the name typed), the stack editor's Update (review
  first, UpdateDate check unless Overwrite), container start / stop / kill / restart / pause /
  resume / remove (kill / remove confirmed); never from a refresh or in the background; each
  change logged in `history.db` (kind `portainer`, never variable values). Git, Kubernetes and
  Edge stacks are changed in Portainer. The Docker / compose terminal actions of #26 still
  refuse Portainer stacks (they would bypass Portainer).
  Also in a user-opened terminal (owner decision on #126): **Log info**'s Register / Unregister
  Systemd Journal (`login_log.rs`) prints `sudo usermod -aG systemd-journal "$U"` /
  `sudo gpasswd -d "$U" systemd-journal` (`U=$(id -un)`), runs it after Enter and lets sudo ask
  for the password. Reading the logins is plain `journalctl` without sudo, only on open / Refresh.
  Third exception, in-app (owner decision on #49): **Authorized keys** (`authorized_keys.rs`; #78
  lists each host's `~/.ssh/authorized_keys` read-only, `cat` only) adds or
  removes one validated public key in the login user's `~/.ssh/authorized_keys` on the chosen
  hosts, only after the user typed `add` / `revoke`; never from a refresh. It refuses to revoke
  the key Telepad logs in with, touches no other line, and rewrites the file via a temp copy.
  Fourth exception, in-app (owner decision on #52–#54): the **database browser** (`db/`) changes
  database rows and structure only after the user saw the exact SQL and clicked Run (DROP /
  TRUNCATE: table name typed); grid edits post in one transaction, one row per statement. It
  reaches databases only through `ssh -N -L` (never a direct database port; owner decision on #129:
  the databases of **My Computer** connect straight to `127.0.0.1:<port>`, no SSH, no other address,
  `db::client::Session::connect_local`), changes nothing on the host, and stores passwords only in the OS password store (`keyring`), never in `telepad.db`.
  New connections are editable; Read-only is a per-connection switch. Import (#139) adds rows only
  after the mapping / row / SQL preview and Import, in one transaction, never on Read-only.
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
  Synchronize (#131, `files/compare.rs`) changes files only after the user saw the preview of every
  copy and delete and clicked Synchronize; deletes only with "Delete files the source does not
  have" (off by default, never both ways); links never followed.
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
  SHA-256 before anything is replaced, and never touches `telepad.db`, `preference.db`, `history.db`, `update.db`, `config` or `.env`.
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
| Update Strategy (#122) | `update.db` next to `telepad.db` (`--update-db <path>` / `TELEPAD_UPDATE_DB`): `strategy` table (key → value: `paused`, `ping_on` / `ping_secs`, `live_*`, `slow_*`, `static_*`, `ssh_retry_secs`; out-of-range values read as defaults) + `last_facts` (alias, tier `static` / `slow`, JSON, `collected_at` Unix seconds) + `paused_hosts` (alias; follows a rename) — both pruned to the loaded aliases; one writer thread in the runtime, flushed on exit. ☰ ▸ Update Strategy… (`Win::Strategy`: Pause all, intervals, host list with per-host Pause / Refresh; also host page and right-click menu); ping rounds via `Effect::Ping` (no SSH) |
| Workspaces (#94) | `preference.db` next to `telepad.db` (`--prefs <path>` / `TELEPAD_PREFS`): `workspaces` table (label, kind `own` / `custom`, layout `x,y,w,h|minimized|maximized|active|tab;tab` per window, position, updated) + `prefs` (`current`, `legacy_imported`); written by a writer thread, autosave ≤ 1 / s, flushed on exit |
| File window folders | `settings` table `files_paths:<alias>` (`local
remote`, last folders per host) |
| Transfer settings (#131) | `settings` table `files_opts:<alias>` (`mode|newer|exclude;exclude`, `Options::to_setting`) |
| Host Management file | `settings` table `hostsync_config_path` (default `~/.ssh/config`); writes keep a `config.telepad-*.bak` next to it |
| Remote Desktop settings (#102) | `rdp_settings` table (alias → JSON, `ON UPDATE / DELETE CASCADE` with `hosts`), schema 4; in JSON export / import as `rdp`; Open RDP: `platform::open_rdp_with` (temporary owner-only `.rdp` → mstsc / `open`; Linux `xfreerdp` args); never a password |
| Data grid hidden columns (#138) | `settings` table `db_hidden:<DbTarget::key>:<db>.<schema>.<table>` (one column name per line) |
| Portainers (#171) | `portainers` table in `telepad.db` (no token); tokens in the OS password store (service "Telepad Portainer", key `portainer:<name lowercased>`); `telepad-check --portainer <name> [--json]` reads one headless |
| Database connections | `db_connections` table (user, database, save-password and read-only flags per `DbTarget::key`); passwords in the OS password store (service "Telepad database") |
| Update source | `settings` table `update_enabled` / `update_repo` / `update_auto_install` (☰ ▸ Software update…); check via the github.com `releases/latest` redirect (API only as fallback: 60 req/h per IP); default `yohannaftali/telepad` |

`.env`, `config`, `telepad.db`, `preference.db`, `history.db`, `update.db` and `telepad.ssh_config` are git-ignored: never commit an inventory.

## Validate

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
cargo run --release --bin telepad-check -- <alias> [--json]
cargo run --release --bin telepad-check -- --check-update
cargo run --release --bin telepad-check -- --logins <alias>   # Log info read (#126)
cargo run --release --bin telepad-check -- --local [--json]   # My Computer read (#129)
cargo run --release --bin telepad-check -- --portainer <name> [--json]   # Portainer read (#171)
cargo run --release --bin telepad-check -- --portainer <name> --stack <stack>   # one stack: variable names, file size, containers (#173, read-only)
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
| #107 | feat(keys): OpenPGP export and import in My keys | closed | 2026-11-22 |
| #108 | feat(rdp): save the Remote Desktop password in Windows Credential Manager (optional) | closed | 2026-11-22 |
| #110 | fix(store): create history.db, preference.db and telepad.db readable by the owner only (macOS / Linux) | closed | 2026-11-22 |
| #111 | fix(history): neutralise formula cells in the CSV export | closed | 2026-11-22 |
| #112 | fix(keys): restrict the generated key before moving it; ACL by SID, permission failure as a warning | closed | 2026-11-22 |
| #122 | feat(collector): Update Strategy page — pause, per-tier intervals, ping check, last facts kept in update.db | closed | 2026-11-23 |
| #125 | fix(collector): hosts that drop ICMP ping stay down until the SSH retry after one failed SSH attempt | closed | 2026-11-24 |
| #126 | feat(keys): Log info page — SSH logins from the systemd journal per host, Register / Unregister journal access | closed | 2026-11-24 |
| #129 | feat(local): My Computer page — monitor this PC like a host, terminal, files and local databases | closed | 2026-11-24 |
| #130 | fix(files): whole file row (.. row, icon, Modified, Size cells) selects on click; centre text vertically | closed | 2026-11-24 |
| #131 | feat(files): compare directories, synchronize, diff between panes and transfer settings | closed | 2026-11-24 |
| #134 | fix(files): Windows legacy junctions (My Documents) — hide system folders, open the link target instead of Access denied | closed | 2026-11-24 |
| #135 | feat(editor): git-style Diff colours (+ green added, - red deleted) and synchronized scrolling | closed | 2026-11-24 |
| #136 | fix(ui): opening an already open page restores and raises its window (minimized or behind) | closed | 2026-11-24 |
| #137 | fix(ui): table cells and headers react to clicks anywhere in the cell — database grid select / centring, sort on header click | closed | 2026-11-24 |
| #138 | feat(db): data grid filter row, column chooser and pager footer (Senar / wtbapp style, 100 rows default) | closed | 2026-11-24 |
| #139 | feat(db): export and import table data (SQL, CSV, Excel) | closed | 2026-11-24 |
| #140 | feat(keys): Log info pager as a sticky footer with record info and first / previous / next / last | closed | 2026-11-24 |
| #143 | fix(ui): Hosts table fills the page width, auto-resizes columns, Flags not clipped and readable | closed | 2026-11-24 |
| #144 | feat(hosts): optional host groups — Groups page, Ungrouped, Hosts - [Group] page, group on cards | closed | 2026-11-24 |
| #145 | feat(hosts): user-defined clusters separate from groups — Clusters page, Hosts - [Cluster] page, cluster on cards | closed | 2026-11-24 |
| #151 | feat(ui): shared table header component — header background, hover highlight, right-aligned sort icon | closed | 2026-11-25 |
| #152 | feat(ui): pager footer — record info left, controls right, continuous mode toggle (wtbapp / Senar style) | closed | 2026-11-25 |
| #153 | feat(ui): table framework — simple and paged tables, sort and filter row everywhere, right-click menu with Filter by, Clear All Filters, export CSV / Excel / PDF, Copy as JSON | closed | 2026-11-25 |
| #156 | fix(ui): simple tables do not fill the page like Hosts, and header title / filter heights are unbalanced | closed | 2026-11-25 |
| #157 | feat(ui): Hosts cards with equal height and whole-card click, sortable Group / Cluster columns instead of By group, shown by default | closed | 2026-11-25 |
| #160 | feat(hosts): reorder groups and clusters by drag and drop, like the Hosts page | closed | 2026-11-25 |
| #163 | feat(hosts): click a host's group or cluster on the Hosts page to open its hosts page | closed | 2026-11-26 |
| #164 | fix(ui): table filter box is shorter than the title and data rows; make them the same height | closed | 2026-11-25 |
| #167 | fix(ui): light theme blue-grey palette for sidebar, buttons, headers and title bars; dark blue title and table header in dark theme | closed | 2026-11-26 |
| #168 | feat(ui): Windows 11-style snap layouts when dragging inner windows | closed | 2026-11-26 |
| #169 | feat(hosts): click the Hosts - [group / cluster] title to go back to its parent page | closed | 2026-11-26 |
| #170 | feat(ui): filter box trailing funnel icon opens a searchable value dropdown (select2 style) | closed | 2026-11-26 |
| #171 | feat(portainer): Portainers page — register Portainer servers with an API token, per-Portainer page with environments and stacks (read-only, SSH tunnel) | closed | 2026-11-26 |
| #173 | feat(portainer): manage stacks and their containers through the Portainer API — start / stop / delete, stack editor with env variables and update, container actions | closed | 2026-11-26 |
| #176 | feat(portainer): Stack page with Containers and Editor tabs, opened by double-click on a stack or Open stack on the host page | closed | 2026-11-26 |
| #180 | fix(store): Update Telepad on a database from a newer Telepad, and open newer but compatible schemas | closed | 2026-11-26 |
