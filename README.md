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

**Contents:** [Install](#install) · [Quick start](#quick-start) · [User manual](#user-manual) ·
[Command line](#command-line) · [Build from source](#build-from-source) ·
[Contributing](#contributing) · [License](#license)

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
([Updates](#updates)).

Windows may show a SmartScreen warning the first time because the exe is not code-signed:
choose **More info → Run anyway**.

### With Cargo (any platform)

```sh
cargo install --git https://github.com/yohannaftali/telepad --locked
```

This builds and installs `telepad` and `telepad-check` into `~/.cargo/bin`. See
[Build from source](#build-from-source) for the prerequisites. When installed this way,
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

## User manual

### Window layout

| Area | What it shows |
|---|---|
| **Top bar** | **☰ Hosts** menu, **Refresh all**, **Auto every _N_ s**, the search box, up/down/total counts, a spinner while hosts load, and an **Update X available** button when a new release exists |
| **Host list** (left) | **Overview — all hosts**, **Add host**, then every host with a status dot, OS logo, address, description and small CPU / RAM / disk bars (or the error) |
| **Main area** | The overview table, or the detail page of the selected host |
| **Status bar** (bottom) | Path of the database in use (hover: the generated ssh config), the Telepad version, and short messages |

Status dot: green = last check succeeded, red = it failed, blue = first check running,
grey = no result yet (or not monitored). Usage bars turn yellow and red as they fill; temperatures are blue
below 70 °C, yellow from 70 °C and red from 85 °C.

### Overview

One row per host: **Host**, **Description**, **Address**, **OS**, **CPU**, **Memory**,
**Fullest disk**, **Temp** (hottest sensor), **Docker** (running/total containers) and
**Updated** (age of the data). Click a host name to open its detail page; the buttons at
the end of a row open SSH/RDP or refresh just that host.

The search box filters both the host list and the overview by host name, address, OS
or container name.

### Host detail

The header shows the alias, description, address and OS logo with buttons
**SSH** / **RDP**, **Copy ssh command**, **Refresh**, **Edit**, **Clone** and **Delete**.
Below it:

- **System**: OS, kernel, architecture, uptime, CPU model and cores, load average.
- **Usage**: CPU, Memory and Swap bars.
- **Disks**: every mount with size, used and free space.
- **Network**: interfaces and their IP addresses.
- **Sensors**: temperatures and fan speeds. Fans spin faster in the drawing the faster
  they run. On a physical server whose BMC is not readable yet, **Set up BMC access…**
  appears (see [Fan speeds and BMC temperatures](#fan-speeds-and-bmc-temperatures-ipmi)).
- **Docker**: runtime version, management (for example a Portainer agent), compose
  stacks with their folder and file, and every container with image, state, container
  IP per network and published ports.
- **Ports in use**: every listening port with the container or process that owns it
  (the process only when the login user may see it); **nginx only** narrows the list to
  the ports nginx serves.

### Refreshing

All monitored hosts are checked at start, then every 60 s while **Auto every** is ticked
(change the interval next to it, 15–3600 s). **Refresh all** checks every host now;
**Refresh** on a host checks only that one. Up to 12 hosts are checked at the same time,
each with an 8 s connect timeout and a 40 s total timeout, so one slow host never holds
up the others. The window stays responsive while hosts load.

### Adding, editing and removing hosts

**☰ Hosts → Add host…** (or **Add host** above the list) opens the editor:

| Field | Meaning |
|---|---|
| **Alias (Host)** | Name used in ssh and in Telepad: letters, digits, `.`, `_`, `-` |
| **Description** | Free text shown under the name |
| **HostName / IP** | Address to connect to (empty = the alias itself) |
| **User**, **Port**, **IdentityFile** | As in an OpenSSH config; empty = ssh's defaults |
| **Access** | **SSH (monitored)** or **RDP only (Windows)** |
| **Monitored** | Untick to keep a host in the list (for SSH/RDP buttons) without polling it |
| **Other ssh options** | One `Keyword value` per line, e.g. `ProxyJump bastion` or `KexAlgorithms +diffie-hellman-group1-sha1` |

**Edit** changes a host, **Clone** starts a new host with the same User, Port and
IdentityFile, and **Delete** removes it after a confirmation. Right-click a host in the
list for the same actions. Every change regenerates `telepad.ssh_config` immediately.

**Windows servers without SSH:** with Access set to **RDP only**, Telepad never opens an
SSH connection to the host. It pings it and tests the RDP port (3389, or the Port you
set); the host is up if either answers. **RDP** opens Remote Desktop (`mstsc` on Windows).

When an SSH check fails, the error says whether the host still answers ping, so a
machine that is down looks different from one where only SSH is broken.

### Import and export

- **☰ Hosts → Import…** reads an OpenSSH config file or a Telepad JSON export and
  shows a preview first:
  - **Merge — add new, update same alias, keep others**
  - **Replace — hosts become exactly this file**

  An import is all or nothing: if any host is invalid, nothing changes. Global
  `Host *` / `Match` blocks are imported too.
- **☰ Hosts → Export…** writes `.json` (re-importable, keeps every field) or OpenSSH
  config text. Hosts that are not monitored or RDP-only are marked with
  `# telepad: not monitored` / `# telepad: rdp only` comments so they round-trip.

`telepad.db` is a normal SQLite file: back it up by copying it while Telepad is closed,
or use Export. You can also inspect it with any SQLite tool.

### Fan speeds and BMC temperatures (IPMI)

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

The rule allows only those two read-only commands. This is the only change Telepad ever
offers to make on a host, and only in a terminal you opened yourself. Virtual machines
have no BMC; the Sensors section says which step is missing on a host that has one.

### Updates

Telepad checks the latest [GitHub release](https://github.com/yohannaftali/telepad/releases)
at start and every 6 hours, in the background, over HTTPS (no account or token).

1. When a newer version exists, the top bar shows **Update X available**.
2. Click it (or **☰ Hosts → Software update…**) to see the version and release notes.
3. **Update now** downloads the zip for your platform, verifies it against the release's
   `.sha256` and replaces `telepad.exe` and `telepad-check.exe`. If anything fails,
   nothing is replaced.
4. **Restart now** (or **Restart for X** in the top bar) starts the new version.

**Later** hides the button until the next start; **Check now** checks immediately.
Your `telepad.db`, `config` and `.env` are never touched. Under **Update source** you
can turn the checks off or point them at a fork's repository.

### Security notes

- Telepad only runs read-only commands on monitored hosts; `sudo` is only used as
  `sudo -n` (never prompts) for the BMC readings.
- New host keys are accepted on first connect (`StrictHostKeyChecking=accept-new`);
  a **changed** host key is rejected, as with plain ssh.
- Host fields are validated before they are saved, because the database is rendered
  into an ssh config; values are passed to ssh as separate arguments, never through a shell.
- Updates are downloaded only from this repository's GitHub release downloads and
  checked against the published SHA-256 before anything is replaced.

### Troubleshooting

| Message | What to check |
|---|---|
| `authentication failed` | The key is not accepted or the host wants a password. Try `ssh <alias>` in a terminal; check IdentityFile and the server's `authorized_keys`. |
| `host key mismatch` | The host's key changed. Verify why, then remove the old line from `~/.ssh/known_hosts`. |
| `connection timed out` + `(no ping reply)` | The host is down or unreachable from your network (VPN?). |
| `connection timed out` + `(host answers ping …)` | The machine is up but SSH is blocked or not running. |
| `no common SSH algorithm (legacy host)` | Add e.g. `KexAlgorithms +diffie-hellman-group1-sha1` under **Other ssh options**. |
| Sensors: `ipmitool not installed` / `sudo rule missing` | Use **Set up BMC access…** on that host. |
| Update check: `cannot reach GitHub` | No internet or a proxy blocks `api.github.com`; Telepad keeps working without updates. |

## Command line

```powershell
telepad.exe                              # GUI
telepad.exe --db D:\other\telepad.db     # use another database
telepad.exe --config D:\ssh\config       # import from this config on first start
telepad-check.exe                        # check all monitored hosts once, text
telepad-check.exe web01 nas01 --json     # selected hosts, JSON
telepad-check.exe --check-update         # newer release on GitHub?
```

`telepad-check` exits 0 when every host answered and non-zero otherwise, so it fits in
scripts and scheduled tasks. `--check-update` exits 0 when up to date, 10 when an update
is available and 2 when the check failed.

| Setting | Where |
|---|---|
| Database | `--db <path>`, `TELEPAD_DB`, else `telepad.db` next to `config`, else next to the exe |
| First-run import | `--config <path>`, `TELEPAD_CONFIG`, `./config`, or `config` next to the exe; only read while the database is empty |
| Generated ssh config | `telepad.ssh_config` next to the database (do not edit: rewritten on every change) |
| Fallback key for hosts without IdentityFile | `SSH_IDENTITY_FILE=` in a `.env` file next to `config` |
| ssh binary | `TELEPAD_SSH` (default: Windows `System32\OpenSSH\ssh.exe`, else `ssh` on `PATH`) |

## Build from source

### Prerequisites

| Platform | Install |
|---|---|
| All | Current stable Rust via [rustup](https://rustup.rs) |
| Windows | **Visual Studio Build Tools** with *Desktop development with C++* (the MSVC linker and C compiler for the bundled SQLite and TLS code); use the default `x86_64-pc-windows-msvc` toolchain |
| Debian / Ubuntu | `sudo apt install build-essential pkg-config libgtk-3-dev libxkbcommon-dev libwayland-dev` |
| Fedora | `sudo dnf install gcc pkgconf-pkg-config gtk3-devel libxkbcommon-devel wayland-devel` |
| macOS | Xcode Command Line Tools: `xcode-select --install` |

### Build and run

```sh
git clone https://github.com/yohannaftali/telepad.git
cd telepad
cargo build --release                       # target/release/telepad and telepad-check
cargo run --release                         # start the GUI
cargo run --release --bin telepad-check -- --json
```

Debug builds (`cargo run`) keep a console window on Windows; release builds do not.

### Package a Windows release zip

```powershell
pwsh scripts/package-windows.ps1     # dist/telepad-<version>-windows-x86_64.zip + .sha256
```

The zip contains both exes, a `README.txt`, `config.example` and the licences.

### Releasing (maintainers)

1. Bump `version` in `Cargo.toml` and add an entry to `CHANGE_HISTORY.md`.
2. Commit, push to `main`, wait for CI to pass.
3. Push a tag that matches the version:

   ```sh
   git tag v0.4.1
   git push origin v0.4.1
   ```

The **Release** workflow runs the tests, builds the Windows zip and publishes it with its
`.sha256`; running apps offer the update on their next check.
`pwsh scripts/package-windows.ps1 -Publish` does the same from your PC with the GitHub CLI.

## Contributing

Issues and pull requests are welcome.

**Reporting a bug.** Open an [issue](https://github.com/yohannaftali/telepad/issues) with
the Telepad version (bottom bar), your OS, what you did and what you saw. For a wrong
or missing value, include the output of `telepad-check <alias> --json` and of the matching
command on the host (`df -Pk`, `cat /proc/meminfo`, `docker ps`, …). **Remove host names,
IP addresses and anything private before posting.**

**Making a change.**

1. Fork the repository and create a branch, e.g. `fix/disk-units` or `feat/zfs-pools`.
2. Read [`AGENTS.md`](AGENTS.md): architecture, data flow and the rules every change must
   keep (they apply to humans and coding agents alike). The most important ones:
   - nothing Telepad runs may change a remote host;
   - the UI thread never runs ssh or network calls;
   - OS-specific code only in `src/platform.rs`;
   - no `unwrap()`/`expect()` on anything that depends on a host, the config or the network.
3. A new metric is a new `===SECTION===` in `src/script.sh` (POSIX `sh`, BusyBox-safe,
   every command guarded) + a parser in `src/parse.rs` + a fixture test in
   `tests/fixtures/` + its place in the UI.
4. Use documentation addresses (`192.0.2.0/24`, `198.51.100.0/24`) and made-up host
   names in tests and fixtures, never real inventory.
5. Before you push:

   ```sh
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

6. Open a pull request against `main` describing what changed and how you tested it
   (which OS, which kinds of hosts). Use [Conventional Commits](https://www.conventionalcommits.org)
   for commit messages: `feat(collector): …`, `fix(ui): …`, `docs: …`.

CI runs the same checks on Windows, Linux and macOS for every pull request.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise,
any contribution you submit for inclusion in this work, as defined in the Apache-2.0
license, is dual licensed as above, without any additional terms or conditions.

Bundled assets: [Phosphor icons](assets/fonts/LICENSE-Phosphor.txt) (MIT) and
[Simple Icons](assets/os/ATTRIBUTION.md) OS logos (CC0).
