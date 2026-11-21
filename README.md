<img src="crates/telepad-gui/assets/icon/telepad.png" width="96" height="96" alt="Telepad icon" align="right">

# Telepad

A small desktop infrastructure monitor written in Rust. Telepad logs in to your
hosts with the **system OpenSSH client** and the keys you already use, runs a
**read-only** POSIX shell script, and shows per host (everything in one window: a page
sidebar on the left, pages as windows inside Telepad that can be moved, resized,
minimized, maximized, tabbed and arranged, kept in named workspaces like MikroTik WinBox):

- OS name and logo, kernel, architecture, uptime
- CPU model, cores, usage %, load average
- Memory and swap capacity vs usage
- Disk capacity vs usage per mount
- Temperatures and fan speeds (hwmon/thermal; BMC via `ipmitool` on physical servers)
- IP addresses and listening ports
- Docker containers and compose stacks: image, state, container IP per network, published ports
- **Open SSH** (a terminal logged in to the host), **Copy ssh command**, and **Open RDP**
  for Windows hosts without SSH
- **Open files…**: a WinSCP-style file window per host, this computer on the left and the host
  on the right; upload / download with drag and drop, overwrite questions and progress,
  over SFTP (scp when the host has no SFTP server, rsync when both sides have it); new
  folder / file, rename, delete, properties and permissions on both sides; a VS Code-like
  editor (tabs, syntax colours, find / replace, line endings kept, changed-on-host check)
- **Authorized keys**: the keys in each host's `authorized_keys`, by server or by key,
  and add or revoke one SSH public key on many hosts at once
- **Host Management**: add, import (OpenSSH config, JSON, pasted text) and export hosts;
  Telepad's hosts next to `~/.ssh/config` (or another config file); copy, edit or delete
  per host, merge / replace / sync, with a preview and a backup before the file is written
- **Open database…**: a HeidiSQL-style window for MariaDB / MySQL / PostgreSQL in a
  container or on the host, through the SSH login (tree, data grid with editing, queries,
  table structure); passwords optionally in the system password store

No agent is installed on the hosts, and monitoring never changes anything on them. The
only changes are actions you start and confirm yourself (Docker actions, BMC setup,
Authorized keys, database edits, file copies). There is no server
and no web UI: it is one portable desktop app plus a headless `telepad-check`.

![Telepad: the Hosts page with the page sidebar](docs/images/hosts-dark.png)

<sub>Screenshots use made-up host names and documentation addresses.</sub>

Releases are built for Windows (x86_64), macOS (Apple silicon and Intel) and Linux
(x86_64 and ARM64). Windows is the first target; on macOS and Linux the desktop parts
(terminal launch, RDP) are less tested.

## Install

### Windows (installer)

1. Download `telepad-<version>-windows-x86_64-setup.exe` from
   [Releases](https://github.com/yohannaftali/telepad/releases) and run it. It installs for
   your user only (no administrator rights) to `%LOCALAPPDATA%\Programs\Telepad` and adds a
   Start-menu entry; uninstall it in **Settings → Apps**.
2. Start **Telepad** from the Start menu.

Your hosts and settings live in `%APPDATA%\Telepad\telepad.db` (workspaces in
`preference.db` next to it), and uninstalling keeps them. To import an existing OpenSSH config on the first start, put it there as `config`, or
use **Host Management → Import → From ~/.ssh/config**.

### Windows (portable zip)

1. Download `telepad-<version>-windows-x86_64.zip` from
   [Releases](https://github.com/yohannaftali/telepad/releases). Optionally compare it
   with the `.sha256` file next to it:
   `Get-FileHash telepad-*.zip -Algorithm SHA256`.
2. Unzip it anywhere you can write to, for example `C:\Tools\telepad\`.
3. Run `telepad.exe`.

Nothing is installed in the system; delete the folder to remove Telepad. Because of the
`portable` file in the zip, the database and generated ssh config are created next to the
exe, so the folder must be writable
(`Program Files` is not a good place).

Both are updated from inside the app ([Updates](docs/USER_MANUAL.md#updates)). Windows may
show a SmartScreen warning the first time because the files are not code-signed:
choose **More info → Run anyway**.

### macOS

1. Download `telepad-<version>-macos-aarch64.dmg` (Apple silicon) or
   `telepad-<version>-macos-x86_64.dmg` (Intel) from
   [Releases](https://github.com/yohannaftali/telepad/releases), open it and drag
   **Telepad** to **Applications**.
2. Telepad is not notarized by Apple, so the first start is blocked. Either right-click
   Telepad in Applications → **Open** → **Open**, or run once in Terminal:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Telepad.app
   ```

3. Start Telepad from Applications or Launchpad. **Open SSH** uses Terminal.app.

Hosts and settings live in `~/Library/Application Support/Telepad/telepad.db` (workspaces
in `preference.db` next to it); put an
OpenSSH config there as `config` to import it on the first start, or use
**Host Management → Import → From ~/.ssh/config**. A portable zip (`telepad-<version>-macos-<arch>.zip`)
is also published; it keeps its data next to the program and needs the same `xattr` step
for its folder.

### Linux

1. Download `telepad-<version>-linux-x86_64.zip` or `telepad-<version>-linux-aarch64.zip`
   from [Releases](https://github.com/yohannaftali/telepad/releases) and unzip it somewhere
   you can write to, e.g. `~/.local/opt/`.
2. Start `./telepad`. It runs on X11 and Wayland; file dialogs use the XDG desktop portal
   (`xdg-desktop-portal`, present on GNOME and KDE desktops). **Open SSH** uses the first
   terminal it finds: `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal`,
   `alacritty`, `kitty` or `xterm`.

The Linux builds are made on Ubuntu 22.04 and need glibc 2.35 or newer.

On every platform the folder must stay writable: `telepad.db` and `preference.db` are
created next to the program, and in-app updates replace the programs in place.

### With Cargo (any platform)

```sh
cargo install --git https://github.com/yohannaftali/telepad --locked telepad-gui telepad-check
```

This builds and installs `telepad` and `telepad-check` into `~/.cargo/bin`. See
[Building from source](CONTRIBUTING.md#building-from-source) for the prerequisites. When installed this way,
pass `--db <path>` (or set `TELEPAD_DB`) so the database does not live in `~/.cargo/bin`
(`preference.db` goes next to it; `--prefs <path>` / `TELEPAD_PREFS` to put it elsewhere),
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
   `telepad.db`, and every host is checked once. No config? Open **Host Management** in
   the sidebar and use **Add host…** or **Import → From ~/.ssh/config**.
3. The **Hosts** page shows every host: status, OS, CPU, memory, disks, Docker. Click a
   row for the host's own window (details, sensors, ports, containers).
4. **Right-click** a host for its menu: **Open SSH** (a terminal logged in to it),
   **Files** (copy files to and from it, edit them), its databases, Edit, Clone, Delete.

![Right-click menu of a host](docs/images/host-menu.png)

## Using Telepad

Telepad is one window. The **sidebar** on the left opens pages; each page opens as a window
**inside** Telepad that you can move (drag its title), resize (drag its right or bottom
edge), minimize (**−**, it waits in a strip at the bottom), maximize (corners out, or
double-click the title) and close (**×**). Put two pages in one window as tabs with the
**⧉** button; the **▯ ‹** button folds the sidebar to icons.

The **top bar** holds the tools for those windows:

- **Workspace** (`<own>` at first): saved sets of windows with their places, like MikroTik
  WinBox. **Add…** keeps the windows that are open now under a name, picking a workspace
  closes the current windows and opens its own, the trash icon deletes one, and `<none>`
  closes every window. Changes are saved automatically in `preference.db`.
- **Window picker**: the window in front and how many are open; click it for the list of
  every open window — type to search, Enter brings that window to the front.
- **Arrange**: **Tile** side by side, **Stack** as a cascade, **Minimize all**, **Restore all**.
- **☰**: **Settings → Theme** (System / Light / Dark) and **Software update…**.

![Two custom workspaces, three windows tiled](docs/images/workspaces.png)

| Page | What you do there |
|---|---|
| **Hosts** | All hosts as a table or cards. Search box at the top; click a header to sort, drag a border to resize, double-click a border to fit the column. Click a row to open the host, right-click for its menu |
| **Docker / Podman** | Every container of every host, grouped by compose stack, with start / stop / logs / shell actions (each one shown and confirmed in a terminal) |
| **Kubernetes**, **Docker Swarm** | The clusters found on your hosts: nodes, workloads, services, problems |
| **Authorized keys** | Who can log in with which SSH key: by server or by key, grouped by the `#` comment lines in `authorized_keys`. Add or revoke a key on many servers at once (you type `add` / `revoke` to confirm), or edit the file |
| **Host Management** | **Add host…**, **Import** (from a file, `~/.ssh/config` or pasted text) and **Export…**; compare Telepad's hosts with `~/.ssh/config` and copy, edit or delete hosts on either side |

![Authorized keys, by server](docs/images/authorized-keys.png)

**Buttons tell you what they do by colour:** red deletes or revokes, yellow edits, green
adds, amber opens a session (SSH / RDP). **☰ → Settings → Theme** switches System / Light /
Dark:

![The Hosts page in the light theme](docs/images/hosts-light.png)

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
