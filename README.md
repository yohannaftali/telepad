# Telepad

A small desktop infrastructure monitor written in Rust. Telepad logs in to your
hosts with the **system OpenSSH client** and the keys you already use, runs a
**read-only** POSIX shell script, and shows per host:

- OS name and logo, kernel, architecture, uptime
- CPU model, cores, usage %, load average
- Memory and swap capacity vs usage
- Disk capacity vs usage per mount
- Temperatures and fan speeds (hwmon/thermal; BMC via `ipmitool` on physical servers)
- IP addresses and listening ports
- Docker containers and compose stacks: image, state, container IP per network, published ports
- **Open SSH** (a terminal logged in to the host), **copy ssh command**, and **Open RDP**
  for Windows hosts without SSH

No agent is installed on the hosts and nothing on them is changed. There is no server
and no web UI: it is one portable desktop app plus a headless `telepad-check`.

Windows 10/11 is the first target. macOS and Linux build and are covered by CI, but
the desktop parts (terminal launch, RDP) are less tested there.

## Install

Download `telepad-<version>-windows-x86_64.zip` from
[Releases](https://github.com/yohannaftali/telepad/releases), unzip anywhere and run
`telepad.exe`. Nothing is installed; delete the folder to remove it. Check the
download against the `.sha256` file next to it if you like.

Requirements on your PC:

- The OpenSSH client (Windows ships it at `C:\Windows\System32\OpenSSH\ssh.exe`)
- Key-based SSH access to your hosts (password logins are not supported: the app
  never prompts)
- Windows Terminal is recommended for **Open SSH** (falls back to a console window)

## Hosts

Hosts live in **`telepad.db`** (SQLite, one file). On the first start, if an
OpenSSH client config named `config` sits next to the exe (or is given with
`--config`), its hosts are imported. After that the database is the source of truth:

- **☰ Hosts → Add host…** and **✏ Edit**: alias, description, HostName, user, port,
  IdentityFile, other ssh options, monitored on/off, access (SSH or RDP only).
- **☰ Hosts → Import…**: an OpenSSH config file or a Telepad JSON export.
  *Merge* adds and updates aliases; *Replace* makes the list exactly the file. A
  preview shows the changes; an import is all or nothing.
- **☰ Hosts → Export…**: `.json` (re-importable) or OpenSSH config format.
- **Windows servers without SSH**: set Access to **RDP only**. Telepad then only
  pings the host and checks the RDP port, and **Open RDP** starts Remote Desktop.
- When SSH fails, the error says whether the host still answers ping, so "machine
  down" and "only SSH broken" look different.

ssh reads the generated `telepad.ssh_config` (rewritten after every change), so all
OpenSSH options keep working. The comment line above a `Host` block becomes its
description. Hosts are checked in parallel (12 at a time) with an 8 s connect and
40 s total timeout; auto refresh defaults to every 60 s. New host keys are accepted
on first connect (`StrictHostKeyChecking=accept-new`); changed keys are rejected.

## Fan speeds and BMC temperatures (IPMI)

CPU temperatures work without setup. Fans and inlet/exhaust temperatures on physical
servers come from the BMC through `ipmitool`, which needs root, so each physical host
needs a one-time setup. Open the host and in **Sensors** click **Set up BMC access…**:
a terminal opens on the host showing the exact commands, waits for Enter and lets
sudo ask for your password. By hand (replace `USER` with the login user):

```bash
sudo apt install ipmitool
echo 'USER ALL=(root) NOPASSWD: /usr/bin/ipmitool sdr type Fan, /usr/bin/ipmitool sdr type Temperature' | sudo tee /etc/sudoers.d/telepad-ipmi
sudo chmod 440 /etc/sudoers.d/telepad-ipmi && sudo visudo -c
```

The rule allows only those two read-only commands. This is the only thing Telepad
ever offers to change on a host, and only in a terminal you opened.

## Updates

Telepad checks the latest [GitHub release](https://github.com/yohannaftali/telepad/releases)
at start and every 6 hours, in the background, over HTTPS (no account or token).
When a newer version exists the top bar shows **Update X available**; **Update now**
downloads the zip for your platform, verifies it against the release's `.sha256`,
replaces `telepad.exe` and `telepad-check.exe`, and offers **Restart**. Your
`telepad.db`, `config` and `.env` are never touched.

Turn checks off, or point them at a fork, in **☰ Hosts → Software update… → Update source**.
From the command line: `telepad-check --check-update` (exit 0 up to date,
10 update available, 2 check failed).

## Command line

```powershell
telepad.exe                              # GUI
telepad.exe --db D:\other\telepad.db     # use another database
telepad-check.exe                        # check all monitored hosts once, text
telepad-check.exe web01 nas01 --json     # selected hosts, JSON
telepad-check.exe --check-update         # newer release on GitHub?
```

| Setting | Where |
|---|---|
| Database | `--db <path>`, `TELEPAD_DB`, else `telepad.db` next to `config`, else next to the exe |
| First-run import | `--config <path>`, `TELEPAD_CONFIG`, `./config`, or `config` next to the exe |
| Fallback key for hosts without IdentityFile | `SSH_IDENTITY_FILE=` in a `.env` next to `config` |
| ssh binary | `TELEPAD_SSH` (default: Windows `System32\OpenSSH\ssh.exe`, else `ssh`) |

## Build from source

```powershell
cargo build --release              # target/release/telepad(.exe) and telepad-check(.exe)
cargo test
pwsh scripts/package-windows.ps1   # dist/telepad-<version>-windows-x86_64.zip + .sha256
```

Linux needs the GTK 3, xkbcommon and Wayland development packages for the file dialog
and windowing (`libgtk-3-dev libxkbcommon-dev libwayland-dev` on Debian/Ubuntu).

### Releasing

Bump `version` in `Cargo.toml`, commit and push, then push a matching tag:

```powershell
git tag v0.4.0; git push origin v0.4.0
```

The `Release` workflow builds the Windows zip and publishes it with its `.sha256`;
running apps pick it up on their next check. `scripts/package-windows.ps1 -Publish`
does the same from your PC with the GitHub CLI.

## Contributing

Issues and pull requests are welcome. See [`AGENTS.md`](AGENTS.md) for the
architecture and conventions (they apply to humans and coding agents alike), and
keep `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
`cargo test` clean.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise,
any contribution you submit for inclusion in this work, as defined in the Apache-2.0
license, is dual licensed as above, without any additional terms or conditions.

Bundled assets: [Phosphor icons](assets/fonts/LICENSE-Phosphor.txt) (MIT) and
[Simple Icons](assets/os/ATTRIBUTION.md) OS logos (CC0).
