# Change History

Newest first.

## [2026-11-17] — Telepad 0.4.1: macOS and Linux releases
- Release workflow builds and publishes windows-x86_64, linux-x86_64, linux-aarch64,
  macos-aarch64 and macos-x86_64 zips (+ `.sha256`) in one release; each zip is unpacked and
  checked (execute bits, `telepad-check --help`) before publishing. PRs touching release files
  run it as a dry run.
- `scripts/package-unix.sh` (macOS/Linux, optional cross target); `package-windows.ps1 -Publish`
  removed so a release always contains every platform.
- Fix: the self-updater now sets the execute bit on the new `telepad-check` on macOS/Linux
  (`platform::make_executable`); before, it would have become non-executable after an update.
- README: install steps for macOS (quarantine) and Linux (portal, terminals, glibc).
- Import is lenient: any JSON with a list of hosts (Telepad export of any version or app
  name, `{"hosts": [...]}`, a bare `[...]`, common field names); everything else is read as
  an OpenSSH config. New **Hosts → Import ~/.ssh/config** (no file dialog). Exports made
  before the rename import again.

## [2026-11-17] — refactor: Cargo workspace + Model–View–Update (#1)
- Workspace: `telepad-core` (inventory, ssh collection, parsers, platform, self-update; no GUI),
  `telepad-app` (State/Msg/Effect/`update()`; no GUI, no I/O), `telepad-gui` (egui views +
  runtime, binary `telepad`), `telepad-check` (CLI, no longer compiles eframe).
- `ui.rs` (2,400 lines) split into `views/` per screen, `widgets.rs`, `runtime.rs`, `app.rs`.
  Views read the model, edit drafts and return messages; `update()` is the only place state
  changes and is covered by 17 behaviour tests; the runtime is the only GUI code doing I/O.
- No behaviour change; same binaries, database and files. Docs (AGENTS, CONTRIBUTING, PR
  template) describe the rules; packaging script and CI updated for the workspace.

## [2026-11-17] — Telepad 0.4.0: first public release
- Published as Telepad (MIT OR Apache-2.0) at https://github.com/yohannaftali/telepad.
- Self-update from GitHub Releases: checks `releases/latest` at start and every 6 h,
  **Update now** downloads `telepad-<ver>-<os>-<arch>.zip`, verifies the `.sha256`,
  replaces `telepad` and `telepad-check`, offers Restart. `telepad-check --check-update`.
- Features carried over: SQLite host database with OpenSSH import/export, Docker stacks
  and container IPs/ports, ports in use, sensors incl. BMC via ipmitool, RDP-only hosts
  with ping status, OS logos, portable Windows zip.
- CI on Windows, Linux and macOS; tag-triggered release workflow.
