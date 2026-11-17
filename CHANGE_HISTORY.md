# Change History

Newest first.

## [2026-11-17] — Telepad 0.4.0: first public release
- Published as Telepad (MIT OR Apache-2.0) at https://github.com/yohannaftali/telepad.
- Self-update from GitHub Releases: checks `releases/latest` at start and every 6 h,
  **Update now** downloads `telepad-<ver>-<os>-<arch>.zip`, verifies the `.sha256`,
  replaces `telepad` and `telepad-check`, offers Restart. `telepad-check --check-update`.
- Features carried over: SQLite host database with OpenSSH import/export, Docker stacks
  and container IPs/ports, ports in use, sensors incl. BMC via ipmitool, RDP-only hosts
  with ping status, OS logos, portable Windows zip.
- CI on Windows, Linux and macOS; tag-triggered release workflow.
