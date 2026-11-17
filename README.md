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
- **Open SSH** (a terminal logged in to the host), **Copy ssh command**, and **Open RDP**
  for Windows hosts without SSH

No agent is installed on the hosts and nothing on them is changed. There is no server
and no web UI: it is one portable desktop app plus a headless `telepad-check`.

Windows 10/11 is the first target. macOS and Linux build and are covered by CI, but
the desktop parts (terminal launch, RDP) are less tested there.

## Install

### Windows (portable zip)

1. Download `telepad-<version>-windows-x86_64.zip` from
   [Releases](https://github.com/yohannaftali/telepad/releases). Optionally compare it
   with the `.sha256` file next to it:
   `Get-FileHash telepad-*.zip -Algorithm SHA256`.
2. Unzip it anywhere you can write to, for example `C:\Tools\telepad\`.
3. Run `telepad.exe`.

Nothing is installed in the system; delete the folder to remove Telepad. The database
and generated ssh config are created next to the exe, so the folder must be writable
(`Program Files` is not a good place). Updates arrive in the app itself
([Updates](docs/USER_MANUAL.md#updates)).

Windows may show a SmartScreen warning the first time because the exe is not code-signed:
choose **More info → Run anyway**.

### With Cargo (any platform)

```sh
cargo install --git https://github.com/yohannaftali/telepad --locked telepad-gui telepad-check
```

This builds and installs `telepad` and `telepad-check` into `~/.cargo/bin`. See
[Building from source](CONTRIBUTING.md#building-from-source) for the prerequisites. When installed this way,
pass `--db <path>` (or set `TELEPAD_DB`) so the database does not live in `~/.cargo/bin`,
and update with the same `cargo install` command instead of the in-app updater.

### Requirements on your PC

- The OpenSSH client: Windows 10/11 ship it at `C:\Windows\System32\OpenSSH\ssh.exe`
  (Settings → Optional features → *OpenSSH Client* if it is missing); macOS and Linux have `ssh`.
- **Key-based** SSH access to your hosts. Password logins are not supported: Telepad
  never shows a password prompt, it reports "authentication failed" instead.
- Windows Terminal is recommended for **Open SSH** (otherwise a console window opens).

### Requirements on the hosts

Nothing to install. The collection script uses POSIX `sh` and common tools
(`/proc`, `df`, `ip`/`ifconfig`, `ss`/`netstat`, `docker`), each guarded, so BusyBox
systems and NAS devices work and missing tools only hide that section.

## Quick start

1. Put your OpenSSH client config next to `telepad.exe` and name it `config`, or start
   with an empty list. A minimal entry:

   ```
   # Web server in rack 2          <- shown as the description
   Host web01
     HostName 192.0.2.10
     User admin
     IdentityFile ~/.ssh/id_ed25519
   ```

2. Start Telepad. On the first start the hosts from `config` are imported into
   `telepad.db`, and every host is checked once.
3. Click a host on the left for details, or keep the **Overview** for all hosts at once.
4. Click **SSH** on a host to open a terminal logged in to it.

Make sure `ssh web01` works in a terminal first: Telepad uses exactly the same client,
keys and options.

Everything else is in the [user manual](docs/USER_MANUAL.md).

## Documentation

- [User manual](docs/USER_MANUAL.md): window layout, overview and host detail, adding and
  importing hosts, RDP-only hosts, BMC sensors, updates, troubleshooting, command line
- [Contributing](CONTRIBUTING.md): building from source, reporting bugs, making a change, releasing
- [Security](SECURITY.md): security model and how to report a vulnerability
- [AGENTS.md](AGENTS.md): architecture and conventions
- [Change history](CHANGE_HISTORY.md)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise,
any contribution you submit for inclusion in this work, as defined in the Apache-2.0
license, is dual licensed as above, without any additional terms or conditions.

Bundled assets: [Phosphor icons](crates/telepad-gui/assets/fonts/LICENSE-Phosphor.txt) (MIT) and
[Simple Icons](crates/telepad-gui/assets/os/ATTRIBUTION.md) OS logos (CC0).
