# Telepad user manual

How to use Telepad once it is [installed](../README.md#install). For a first run see the
[quick start](../README.md#quick-start).

**Contents:** [Window layout](#window-layout) · [Overview](#overview) · [Host detail](#host-detail) · [Refreshing](#refreshing) · [Adding, editing and removing hosts](#adding-editing-and-removing-hosts) · [Import and export](#import-and-export) · [Fan speeds and BMC temperatures (IPMI)](#fan-speeds-and-bmc-temperatures-ipmi) · [Updates](#updates) · [Troubleshooting](#troubleshooting) · [Command line](#command-line)

## Window layout

Telepad is one window (like MikroTik WinBox): pages open as windows **inside** it.

```
------------------------------------------------------------------
Telepad | Refresh all | Theme | 2 up 1 down                   ☰ |
------------------------------------------------------------------
Pages            | +-[Hosts]-[Docker / Podman]---------- _ □ × ⧉ + |
 Hosts           | | search | Table / Cards                     | |
 Docker / Podman | | web1  192.0.2.10  …            [ssh][files]| |
 Authorized keys |  +---------------------------------------------+ |
 Host Management |        +-[Files — web1]---------- _ □ × ⧉ +     |
                 |        | this computer | web1              |     |
                 |        +----------------------------------+     |
                 | [Host — db01]  (minimized)                      |
------------------------------------------------------------------
```

| Area | What it shows |
|---|---|
| **Top bar** | **Refresh all**, **Auto every _N_ s**, theme, up/down/total counts, a spinner while hosts load, an **Update X available** button when a new release exists, and at the top right the **☰** menu (settings: add, import, export, Host Management, Authorized keys, software update) |
| **Pages** (left sidebar) | One button per page: **Hosts**, **Docker / Podman**, **Kubernetes**, **Docker Swarm**, **Authorized keys**, **Host Management** (file and database windows open from a host: the **Files** button in its row or window, **Databases** in its window or a container's menu). Below them, **Open windows** lists every open window (click one to bring it back). The **▯ ‹** button at the top folds the sidebar to centred icons only; Telepad remembers it |
| **Workspace** | The open windows. Minimized windows wait in a strip at its bottom |
| **Status bar** (bottom) | Path of the database in use (hover: the generated ssh config), the Telepad version, and short messages |

**Windows inside Telepad.** A page button opens its page in a window, or brings the window
back to the front if it is already open. Each window has a title bar:

- **Move**: drag the title bar. **Resize**: drag the right edge, the bottom edge or the
  bottom-right corner. Windows stay inside the workspace.
- **_ (Minimize)** puts it in the strip at the bottom; click it there (or in **Open
  windows**) to bring it back. **□ (Maximize)** fills the workspace; click again, or
  double-click the title bar, to get the old size back. **×** closes it (a file window
  with unsaved editor tabs asks first).
- **⧉ (Tabs)**: **Show as a tab in** → another window puts this page in that window as a
  tab; click a tab to show it, middle-click to close it; **Own window** takes it out again.
- Keys (F2, F4, F5, F7, Del in a file window, Ctrl+S and the others in the editor) go to the
  window in front; click a window to bring it to the front.
- Clicking a host in the overview opens its **Host — _name_** window; Files, the editor and
  each database open as windows too. Dialogs (edit host, import, delete, update, file
  questions) open over the workspace, never as a separate window of the system.
- Telepad remembers the windows of Overview, Docker, Host Management and Authorized keys (place,
  size, tabs, minimized / maximized) and opens them again on the next start. Host, file,
  editor and database windows are not reopened (they belong to a session). The first start
  shows the overview, maximized.

Status dot: green = last check succeeded, red = it failed, blue = first check running,
grey = no result yet (or not monitored). Usage bars turn yellow and red as they fill; temperatures are blue
below 70 °C, yellow from 70 °C and red from 85 °C.

**Button colours**: a button's colour says what it does. **Red** (with the trash icon)
deletes, removes, drops, revokes or overwrites something; **yellow** edits or renames;
**green** adds or creates; **amber** opens a session that can change things (SSH, RDP).
Other buttons keep the neutral look. Each theme has its own
shades (deep fills with light text in the dark theme, brighter fills in the light theme),
and in menus the same meaning shows as red / yellow / green text.

**Copy and paste**: every text field (forms, filters, the Authorized keys box, database
queries, the file window's path bars, the editor) works with the system clipboard:
Ctrl+C / Ctrl+X / Ctrl+V (Cmd on macOS), in every window.

## Theme

**Theme** in the top bar switches between **System** (follows the Windows / macOS / desktop
setting, the default), **Light** and **Dark**. The choice is saved in the database. The light
theme uses a grey background behind white windows with a border, and buttons and input
boxes with a visible fill and border, so every layer stands apart.

## Hosts

Telepad 0.9 has no host list on the left any more: the hosts are in **Overview — all
hosts** (table or cards). Click a host there to open its window; right-click for Edit /
Clone / Delete.

**Reorder hosts:** drag a host by its **⋮⋮** handle in the overview table and drop it on the
upper or lower half of another host; a line shows where it will land. The order is saved in
`telepad.db` at once, used by the table, cards and the generated ssh config, and kept after a
restart. The handles are hidden while the search box has text or a column is sorted.

## Overview

Click **Hosts** in the sidebar to see every host at once. The page header has the
**search box** (filters by host name, address, OS or container name) and the **View** switch:
**Table** or **Cards**; the choice is remembered.

### Docker overview

**Docker / Podman** (below **Hosts**) lists the containers of every host
grouped by stack (the compose project name), stacks A–Z, then **Standalone containers**. A
stack running on several hosts appears once; inside it, each host has its own line with a
link to the host, how the stack is managed (docker compose folder or Portainer stack) and
the same **Compose** menu / **Open in Portainer** button as on the host page. Each container
shows image, status, container IPs and published ports, and its **☰** menu has the usual
[Docker actions](#docker-actions). Click a stack name to fold it. The search box filters by
stack, container, image or host.

### Table

One row per host: **Host**, **Description**, **Address**, **OS**, **Flags**, **CPU**, **Memory**,
**Fullest disk**, **Temp** (hottest sensor), **Docker** (running/total containers) and
**Updated** (age of the data), then two icon buttons: the amber **terminal** opens SSH (or
RDP for Windows hosts) and the yellow **folder** opens the host's file window. Click a row
(or the host name) to open its host window; the row under the mouse is highlighted. The
table always fills the window: the last column takes the spare width.

In inventory order (no column sorted, no search) each row has a **⋮⋮** handle in the first
column: drag it to reorder hosts. While a column is sorted or the
search box has text, the handles are hidden and a hint above the table says what to clear.

- **Sort:** click a column header. Numbers start with the highest (busiest CPU, fullest
  disk, hottest, most containers, most recently updated), text columns from A to Z; the
  arrow next to the header shows the direction. Click again to reverse, a third time to
  return to the inventory order. The unlabelled first column sorts by status (down hosts
  first). Addresses sort numerically (`10.0.0.9` before `10.0.0.10`), and hosts without a
  value for the column (no data yet, or unreachable) always stay at the bottom.
- **Resize:** drag the border between two column headers. **Double-click** a border to fit
  the column to its longest value, like Excel. Text that does not fit is cut with "…";
  hover over it to read all of it. Error messages stay inside the OS column. Sortable
  headers show a hand cursor.
- The header row stays in place while you scroll; when the columns are wider than the
  window, scroll sideways.

The search box (in the Hosts and Docker / Podman page headers) filters both pages by host
name, address, OS or container name.

### Cards

One card per host, wrapping to the window width: status, OS logo, alias (click it for the
detail page), address and description, OS, CPU / RAM / fullest-disk bars, hottest
temperature, Docker summary, how long ago it was checked, and an **SSH** / **RDP** button.
Unreachable hosts show their error instead of the bars. The search box filters the cards;
**Sort by** picks the order (the same columns as the table, or inventory order) and the
arrow button next to it reverses it.

### Maintenance flags

The **Flags** column, the cards and the host's **Maintenance**
section show what needs doing on a host (hover for details):

| Flag | Meaning | Source on the host (read-only, offline) |
|---|---|---|
| red **EOL** | the OS release is past its vendor's end of support: no more security fixes | `/etc/os-release` `ID` + `VERSION_ID` against Telepad's built-in table |
| yellow **EOL 75d** | support ends within 6 months | same |
| yellow **reboot** | a reboot is pending (with the packages that asked for it) | `/var/run/reboot-required`, `needs-restarting -r`, or the running kernel older than the newest installed one |
| **⬇ 12 (3 sec)** | 12 package updates waiting, 3 of them security updates (red when there are security updates) | `apt-check` / `apt-get -s`, `dnf` / `yum check-update --cacheonly`, `apk` |

Update counts come from the host's local package cache, so they are only as fresh as the
host's last `apt update` / `dnf makecache`. Releases not in the table show "unknown"
support. Sort by **Flags** to see the most urgent hosts first.

## Host detail

The header shows the status, OS logo, alias and `user@address`. Below it are the buttons
**Open SSH** / **Open RDP**, **Open files…** (see [Files](#files-upload--download--manage--edit)),
**Copy ssh command**, **Refresh**, then **Edit**, **Clone** and **Delete**. On a narrow window the buttons (and the top bar) wrap onto a second line instead
of being cut off. Below that:

- **System**: OS, kernel, architecture, uptime, CPU model and cores, load average.
- **Usage**: CPU, Memory and Swap bars.
- **Disks**: every mount with size, used and free space.
- **Network**: interfaces and their IP addresses.
- **Sensors**: temperatures and fan speeds. Fans spin faster in the drawing the faster
  they run. On a physical server whose BMC is not readable yet, **Set up BMC access…**
  appears (see [Fan speeds and BMC temperatures](#fan-speeds-and-bmc-temperatures-ipmi)).
- Under the description: when the OS / CPU facts and the addresses / ports / updates were
  last read (see [Refreshing](#refreshing)).
- **Docker**: runtime version, management (for example a Portainer agent), compose
  stacks with their folder and file, and every container with image, state, container
  IP per network and published ports. See [Docker actions](#docker-actions).
- **Ports in use**: every listening port with the container or process that owns it
  (the process only when the login user may see it); **nginx only** narrows the list to
  the ports nginx serves.

## Docker actions

Each container row has a **☰** menu, and each docker compose stack a **Compose** menu.
Telepad never changes a container by itself: every action opens a terminal on the host
that prints the exact command and runs it only after you press **Enter**. **Kill**,
**Remove** and compose **Down** ask you to type `yes`. If your user may not use Docker
directly, the command runs with `sudo` and sudo asks for your password there.

| Menu | Actions |
|---|---|
| Container ☰ | Shell (`docker exec -it`, bash if present), Logs (`logs -f --tail 200`), Start, Stop, Restart, Pause, Resume, Kill, Remove (`rm` without `-f`: stop it first; volumes are kept) — only the ones that fit the container's state |
| Compose | Logs, Up (`up -d`), Restart, Stop, Down (`down` without `-v`: named volumes are kept), run with the stack's own folder, project name and files |
| Portainer stack | **Open in Portainer** when the Portainer server runs on this host, otherwise "change it in Portainer": Portainer stacks are only changed in Portainer |

**How it runs…** (container ☰) reads `docker inspect` and the stack's compose files
(read-only) and shows a `docker run` command rebuilt from them, with **Copy** buttons.
Image defaults (environment, command) appear as options, and values whose name looks
like a secret (`PASSWORD`, `TOKEN`, `KEY`, …) or passwords inside URLs are shown as `***`.
Portainer keeps its stacks' compose files inside its own volume, so they are not shown.

Example (made-up container):

```
docker run \
  -d \
  --name shop-db \
  --restart unless-stopped \
  --network shop_default \
  -p 127.0.0.1:3307:3306 \
  -v shop_db:/var/lib/mysql \
  -e 'MARIADB_ROOT_PASSWORD=***' \
  mariadb:11
```

## Swarm and Kubernetes

When a host is a **Docker Swarm** manager, or can read a **Kubernetes** cluster with
`kubectl`, `k3s kubectl` or `microk8s kubectl` (its own kubeconfig, or `sudo -n` when
allowed), the host page gets a **Cluster** section and the overview's Docker cell a badge
(*Swarm manager*, *Swarm worker*, *k8s*).

- **Swarm**: nodes (status, availability, leader), services with replicas running /
  wanted, stacks, and tasks that should run but do not. Workers only show their role: the
  cluster can only be read on a manager.
- **Kubernetes**: nodes (ready), deployments, statefulsets and daemonsets with ready /
  wanted pods, and problem pods (not running, waiting such as `CrashLoopBackOff`, or
  restarted 5 times or more).
- A degraded cluster (node down, missing replicas, problem pods) raises a red
  **cluster** flag in the Flags column.
- Several managers of one cluster, or one machine under two aliases, show the cluster
  once; the others say "Same cluster as …" with a link.

Actions work like the [Docker actions](#docker-actions): a terminal shows the exact
command and runs it after you confirm.

| Where | Actions |
|---|---|
| Swarm service ☰ | Logs (`docker service logs -f`), Restart (rolling, `service update --force`), Scale… (the terminal asks for the number of replicas) |
| Swarm node ☰ | Drain, Pause, Activate (`docker node update --availability`) |
| Swarm stack ☰ | Remove stack (`docker stack rm`, type `yes`) |
| Kubernetes workload ☰ | Logs, Rollout restart, Scale… (not for daemonsets) |

Collection stays read-only, every cluster command has a time limit, and long lists are
capped (500 services, 1000 workloads per kind, 100 problem pods).

## Refreshing

All monitored hosts are checked at start, then every 60 s while **Auto every** is ticked
(change the interval next to it, 15–3600 s). **Refresh all** checks every host now;
**Refresh** on a host checks only that one.

Not everything is read on every round. Automatic rounds read:

| What | How often |
|---|---|
| Uptime, load, CPU, memory, disks, sensors, Docker, Swarm, Kubernetes | every round (**Auto every**) |
| IP addresses, listening ports, pending updates, reboot needed | every 6 hours |
| Hostname, OS release, kernel, architecture, CPU model | once a day |

**Refresh** and **Refresh all**, a new or edited host and a host that has just rebooted
(uptime went down) read everything. A host that is down and does not answer ping is only
pinged on the following rounds (no ssh connection, no timeout to wait for); as soon as it
answers ping everything is read again, and ssh is still tried every 10 minutes for hosts
whose firewall drops ping. Remote Desktop hosts are always just ping + port check.

Up to 12 hosts are checked at the same time,
each with an 8 s connect timeout and a 40 s total timeout, so one slow host never holds
up the others. The window stays responsive while hosts load.

## Adding, editing and removing hosts

**☰ → Add one host…** opens the editor:

| Field | Meaning |
|---|---|
| **Alias (Host)** | Name used in ssh and in Telepad: letters, digits, `.`, `_`, `-` |
| **Description** | Free text shown under the name |
| **HostName / IP** | Address to connect to (empty = the alias itself) |
| **User**, **Port**, **IdentityFile** | As in an OpenSSH config; empty = ssh's defaults. For IdentityFile, the arrow next to the field lists the private keys found in `~/.ssh`; **Browse…** opens in `~/.ssh` (on macOS press Cmd+Shift+. in the dialog to see other hidden folders); typing `~/.ssh/id_ed25519` works too |
| **Access** | **SSH (monitored)** or **RDP only (Windows)** |
| **Monitored** | Untick to keep a host in the list (for SSH/RDP buttons) without polling it |
| **Other ssh options** | One `Keyword value` per line, e.g. `ProxyJump bastion`, `KexAlgorithms +diffie-hellman-group1-sha1` or `LocalForward 8080 127.0.0.1:80`; values are written to the ssh config exactly as typed (quote an argument yourself if it contains spaces) |

**Edit** changes a host, **Clone** starts a new host with the same User, Port and
IdentityFile, and **Delete** removes it after a confirmation. Right-click a host in the
list for the same actions. Every change regenerates `telepad.ssh_config` immediately.

**Windows servers without SSH:** with Access set to **RDP only**, Telepad never opens an
SSH connection to the host. It pings it and tests the RDP port (3389, or the Port you
set); the host is up if either answers. **RDP** opens Remote Desktop (`mstsc` on Windows).

When an SSH check fails, the error says whether the host still answers ping, so a
machine that is down looks different from one where only SSH is broken.

## Import and export

- **☰ → Import ~/.ssh/config** imports your own OpenSSH client config in one click.
  To compare it host by host and copy changes in either direction, use
  [Host Management](#host-management-compare-with-an-openssh-config).
- **☰ → Import from a file…** reads any file you pick:
  - an **OpenSSH client config** (any file that is not JSON);
  - **JSON** with a list of hosts: a Telepad export from any version, `{"hosts": [...]}`
    or a bare `[...]`. Common field names are understood, in any case and with or
    without `_`/`-`: `alias`/`name`/`host`, `hostname`/`ip`/`address`, `user`/`username`,
    `port`, `identity_file`/`key`, `description`/`note`/`comment`, `access` (`ssh`/`rdp`),
    `monitored`/`unmonitored`, and `options` as `[["Keyword","value"]]` or `{"Keyword":"value"}`.
    A host needs at least a name or an address.

  Either way a preview comes first:
  - **Merge — add new, update same alias, keep others**: new hosts are added at the
    end of the list in imported order; updated hosts keep their place.
  - **Replace — hosts become exactly this list**, in the order of the import.

  An import is all or nothing: if any host is invalid, nothing changes. Global
  `Host *` / `Match` blocks are imported too.
- **☰ → Import from pasted text…** opens a text box: paste with Ctrl+V or
  **Paste from clipboard**, then **Next: preview**. It reads OpenSSH config text
  (`Host` blocks) or JSON as above, or a simple list with one host per line:

  ```
  web01 admin@192.0.2.10
  nas01 192.0.2.20:2222
  198.51.100.7
  ```

  that is `[alias] [user@]host[:port]` (without an alias the address becomes the
  alias; port 22 when none is given). Blank lines and `#` comments are skipped.
  Lines that cannot be read and invalid hosts are left out and listed with their
  line number in the preview; the other hosts can still be imported. If nothing
  can be read, the window stays open and says why.
- **☰ → Export…** writes `.json` (re-importable, keeps every field) or OpenSSH
  config text. Hosts that are not monitored or RDP-only are marked with
  `# telepad: not monitored` / `# telepad: rdp only` comments so they round-trip.

`telepad.db` is a normal SQLite file: back it up by copying it while Telepad is closed,
or use Export. You can also inspect it with any SQLite tool.

## Host Management (compare with an OpenSSH config)

**☰ → Host Management… (compare with ~/.ssh/config)** shows Telepad's hosts (database
icon, left) next to the hosts of an OpenSSH config file (monitor icon, right), so the two
can be kept in step in either direction. Nothing is written until you click **Apply**.

**Which file:** `~/.ssh/config` at first. **File…** picks another file, **Folder…** a folder
(its `config` file is used); the choice is remembered. **Reload** reads the file again. A
file that does not exist yet counts as empty and is created on Apply.

**The list:** one row per alias, with

- **=** the same on both sides,
- **≠** different — the column on the right names the fields (HostName, User, Port,
  IdentityFile, Options, Description, Not monitored, RDP only),
- the database icon: only in Telepad; the monitor icon: only in the config.

**Filter** narrows the rows by alias, address or description; **Only differences** hides the
rows that are the same. Aliases defined together with others on one line (`Host jump
bastion`) or by a pattern are listed as **shared Host line**: they can be copied to Telepad,
but not changed in the file (do that by hand).

**Per host:** **→** copies Telepad's host to the config, **←** the config's host to Telepad;
the pencil edits the host on that side, the bin deletes it there; **↶** undoes the pending
changes of that host. Tick several rows to do the same for all of them (copy, delete, undo).

**All at once:**

- **Telepad → config** / **Config → Telepad**, each as **Merge** (add missing hosts, update
  different ones, keep hosts only the target has) or **Replace** (the target becomes a copy:
  hosts only the target has are removed; shared lines, `Host *` and `Match` stay).
- **Sync**: hosts only on one side are copied to the other; hosts that differ are marked
  red and you pick a side for each with → or ← before you can apply.

**Add to Telepad… / Add to config…** opens a small form (alias, HostName, User, Port,
IdentityFile, description, other options one per line, Not monitored, RDP only). Edits use
the same form. Hosts are checked like in the host editor.

Changed hosts are marked **pending** and counted at the bottom. **Discard** drops them.
**Preview and apply…** shows Telepad's changes and the config file as a diff (`-` removed,
`+` added); **Apply** writes them:

- Before the config is written, a copy is saved next to it as
  `config.telepad-YYYYMMDD-HHMMSS.bak` (UTC time). The file is replaced in one step and
  keeps its permissions.
- Only the blocks of the hosts you changed are rewritten. Comments, `Include`, `Match`,
  `Host *`, shared lines and the order of everything else stay as they are; new hosts go
  after the last host block, before `Host *` / `Match`. Telepad's description and its
  "not monitored" / "rdp only" marks are written as `#` comments, as by Export.
- If the file was changed by something else since it was loaded, Apply stops: click
  **Reload** (your pending changes stay) and look at the preview again.
- Telepad's changes are saved in one transaction; a renamed host keeps its place in the list.

## Files (upload / download / manage / edit)

**Open files…** in a host's window (or **Files** in the sidebar) opens a file window, WinSCP style: **This computer**
on the left, the host on the right. It logs in as the host's normal SSH login (the same
user and key as **Open SSH**); nothing has to be installed on the host. Several file
windows can be open at once, one per host.

- **Top bar**: the host and `user@address`, then how files are copied:
  - **SFTP** — the normal case (the host's `sftp-server`, which every OpenSSH server has
    unless it was switched off).
  - **⚠ no SFTP on the host: copies with scp** — the host has no SFTP server. Telepad then
    lists folders with a small read-only `sh` script and copies with `scp -O`. There is no
    overwrite question and no progress bar in this mode: scp replaces files of the same name.
  - **Hidden files** shows names starting with `.`.
  - **Use rsync** — only shown when rsync is installed both on this computer and on the
    host (otherwise the reason, for example *rsync is not installed on this computer*).
- **Panes**: each has **Up**, **Home** and **Refresh**, the folder path (type a path and
  press Enter to go there; on this computer `~` is your home folder) and a filter box that narrows the list
  by name. The columns are Name, Size, Modified (UTC) and Permissions, plus Owner on the
  host side; click a header to sort, click it again to reverse. Folders are listed first.
  - Click a row to select it; Ctrl+click adds or removes rows. The pane title shows how
    many are selected.
  - Double-click a folder (or **..**) to open it.
  - Both folders are remembered per host for the next time the window opens.
- **Copy** the selected files and folders (folders with everything in them):
  - **Upload** (local → host folder) or **Download** (host → local folder), or
  - **F5** copies the selection of the pane you clicked last to the other pane, or
  - drag the files onto the other pane.
- **The file exists**: when a file is already there, Telepad shows both sizes and times and
  asks **Overwrite**, **Skip**, **Newer only** (overwrite only when the copied file is
  newer) or **Cancel transfer**. Tick **Apply to all … existing file(s)** to answer once for
  the whole transfer.
- **Transfers** (bottom): every copy, newest first, with a progress bar (bytes done, speed,
  current file) and **Cancel**. A finished row says how many files were copied, skipped
  or failed; failures are listed under **error(s)**. **Clear finished** removes the
  finished rows.

Files are written next to their target as `<name>.telepad-part` and renamed only when
complete, so a cancelled or failed copy never leaves half a file in place of a good one.
Copied files keep their modified time (so **Newer only** works the next time). On the host,
new files get the permissions the host's umask gives them; Telepad never runs `sudo`, so
only folders the login user may write to can be changed.

**rsync**: with **Use rsync** ticked, Upload / Download first run `rsync --dry-run` and list
what would change (*rsync would change N path(s)*); **Run rsync** then copies them. rsync
copies only what differs and never deletes anything on the other side.

### Managing files

Each pane has a toolbar under its path, and the same actions on the right-click menu of a
row (right-clicking a row that is not selected selects it first). The keys act on the pane
you clicked last, never while you are typing in a field.

| Action | Toolbar / menu | Key | What it does |
|---|---|---|---|
| Edit | **Edit** | F4 | One selected file opens in the editor (see [Editor](#editor)); double-clicking a file does the same. |
| New folder | **Folder** | F7 | Asks for a name and creates the folder in the pane's folder; it is selected afterwards. |
| New file | **File** | | Creates an empty file. |
| Rename | **Rename** | F2 | One selected item, in the same folder. An existing name is never replaced. |
| Delete | **Delete** | Del | The selected items, folders with everything in them. |
| Properties | **Properties** | | One selected item: full path, type, size (for a folder: what is inside), modified time, permissions and owner. |
| Permissions | **Permissions** (host side only) | | See below. |
| Copy path | **Path** / **Copy path** | | The full paths of the selection (one per line), or the folder's path when nothing is selected. |

- **Names**: a name is one part of a path. Empty names, `.`, `..`, `/`, control characters
  and leading or trailing spaces are refused (on Windows also `\ < > : " | ? *` and a final
  dot). The reason appears in the dialog, which stays open so you can fix the name; so does
  *… already exists*.
- **Delete** asks once, listing every item with its size; for a folder it counts what is
  inside (*2.4 MiB in 12 file(s) and 3 subfolder(s)*), then shows the total. It says that the
  delete cannot be undone: the host has no recycle bin, and on this computer the files are
  deleted, not moved to the Recycle Bin. Links are removed themselves; what they point to
  is never touched.
- **Permissions…** (host side, one or more items): the current mode as checkboxes (owner /
  group / others × read / write / execute) and as an octal number (`644`, `2775`); either
  one updates the other. **Also everything inside the folders** applies it to what is
  inside too: folders get the mode plus *x* wherever *r* is set (so they stay openable),
  files get its read / write bits, keep *x* only if they already had it, and never get
  setuid / setgid / sticky. Links are not changed. The permissions column shows setuid /
  setgid / sticky as `s` / `t`, like `ls -l`.
- **Errors**: when some items cannot be deleted or changed (for example *Permission
  denied*), the others are still done and a red list at the top of the window names each
  failed item; **Dismiss** hides it.
- Telepad never uses `sudo` here: only what the login user may change is changed.
- On a host **without SFTP** the host side's actions are disabled (*Needs SFTP on the
  host*); copying and this computer's side still work.

### Editor

**Edit** / F4 or a double-click on a file opens it in the host's **Editor** window, which
looks and works like VS Code. One editor window per file window: each file is a tab, files
of this computer and of the host side by side (the tab's icon says which; hover it for the
full path). Opening a file that already has a tab switches to it.

- **Refused files** show the reason in their tab instead of the text: binary files, files
  that are not UTF-8 text, and files over 10 MiB (download them instead). **Close tab**
  closes it.
- **Look**: light or dark with Telepad's theme, line numbers, the line with the cursor
  highlighted, monospace font, colours by language. The language comes from the file name
  (`.sh`, `.yml` / `.yaml`, `.json`, `.toml`, `.ini` / `.conf` / `.cnf`, `.env`, `Dockerfile`,
  nginx files (`nginx.conf`, files under `/nginx/`), `.py`, `.js`, `.ts`, `.php`, `.sql`,
  `.md`, `.rs`), else from a `#!` first line (`#!/bin/sh`); the status bar's language menu
  changes it. Files over 2 MiB are shown without colours.
- **Tabs**: a tab with changes that are not saved shows **●**; **×** (or a middle click,
  or Ctrl+W) closes it.
- **Status bar**: *Saved* / *Not saved* / *Saving…*, then `Ln 12, Col 5` (and how many
  characters are selected), the indentation (*Spaces: 2* or *Tab*, from the file), the
  language, the line endings (**LF** or **CRLF**) and the encoding (**UTF-8**, or **UTF-8 with
  BOM**).

| Key | Does |
|---|---|
| Ctrl+S | Save (also the **Save** button) |
| Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z) | Undo / redo, per tab |
| Ctrl+F | Find (a selection on one line becomes the search text) |
| Ctrl+H | Replace |
| Ctrl+G | Go to line |
| Enter / Shift+Enter, F3 / Shift+F3 | Next / previous match |
| Esc | Close the find bar, back to the text with the match selected |
| Tab / Shift+Tab | Indent / outdent the selected lines (one line: Tab inserts at the cursor) |
| Ctrl+W | Close the tab |

- **Find / replace**: **Aa** match case, **ab** whole word, **.\*** regular expression;
  every match is highlighted, the current one stronger, with *3 of 12*. **Replace** replaces
  the current match and goes to the next; **Replace all** replaces every match (*12
  replaced*). With **.\*** the replacement may use `$1`, `$2`… for the groups (`$2_$1` swaps
  `APP_ENV` to `ENV_APP`; `$$` is a `$`). An invalid expression says why in red.
- **Line endings and encoding are kept**: a CRLF file is saved with CRLF, an LF file with LF,
  a byte order mark stays, and so does the presence or absence of a newline at the end. A
  file with both kinds of line endings is saved with the more common one (hover **CRLF** /
  **LF** to see it).
- **Saving** writes the text to `<name>.telepad-part` next to the file, gives it the file's
  permissions (and owner, when the login may set it), then puts it in place of the file:
  a failed save never leaves half a file. A link is followed: the file it points to is
  saved, the link stays. If the folder cannot be written, the red line under the path says
  so (*Cannot write a temporary file in /etc: Permission denied*); the text stays in the tab.
- **Changed meanwhile**: before writing, Telepad checks that the file's size and time are
  still what it loaded. If someone changed it on the host (or another program on this
  computer), a yellow box says so with its size and time now:
  - **Overwrite** — save your text anyway;
  - **Reload** — discard your changes and load the file as it is now;
  - **Show diff** — the lines that differ (`−` in the file now, `+` your text, unchanged
    runs folded);
  - **Keep editing** — close the box (Save asks again).
- **Unsaved changes**: closing a tab, the editor window, the file window or Telepad itself
  while a tab has changes asks first: **Save** (the window closes when the saves are done;
  a save that fails keeps it open), **Don't save**, or **Cancel**. When Telepad itself was
  closing, close it again after answering.
- Editing on the host needs SFTP: on a host without it, only this computer's files open.

## Authorized keys (list, add, revoke)

**Authorized keys** in the sidebar (or **☰ → Authorized keys…**) shows who can log in to
your hosts with an SSH key, and gives someone SSH access to many hosts at once, or takes it
away, using the access Telepad already has. It reads and changes only the
`~/.ssh/authorized_keys` file of the user Telepad logs in as on each host (no `sudo`).

When the page opens, Telepad reads that file on every SSH host (read-only, in parallel);
**Refresh** reads them all again, the small ↻ next to a server only that one. The page has
three tabs:

- **By server**: the servers on the left with their number of keys (**no file** when there
  is no `authorized_keys`, **failed** with the reason on hover when the host could not be
  read). Pick one to see its keys: type, comment, SHA256 fingerprint and options
  (`from=`, `command=` …). The key Telepad itself logs in with is marked
  **Telepad logs in with this key** and cannot be revoked. A red **Revoke** removes that key
  from this server; the green **Add key** opens Add / revoke with this server ticked.
  Older key types (e.g. `ssh-dss`) are listed but not managed; lines that are not keys are
  counted and left alone.
- **By public key**: every key found, grouped by fingerprint, with its comments and the
  number of servers that have it. Click it to see those servers, each with a red
  **Revoke**, and **Revoke on all _N_ servers** (not the servers Telepad logs in to with
  it). The green **Add to server…** opens Add / revoke with the key filled in, to tick more
  servers.
- **Add / revoke**: the form below. Every shortcut above opens it with the key and the
  servers filled in and the confirmation waiting; nothing changes until you type the word.

After a run, the changed servers are read again, so the lists show the result.

1. Paste one public key (Ctrl+V): the single line from a `.pub` file, for example
   `ssh-ed25519 AAAAC3Nza… alice@example.com`. Telepad shows its type, its SHA256
   fingerprint (the same as `ssh-keygen -lf`) and its comment, or why it is not a valid
   key. Private keys, several lines and keys with options (`from=`, `command=`) are refused.
2. Tick the hosts. **Select all** ticks every host the filter shows (click again to clear
   them); the filter box narrows the list by name, address, description or OS. RDP-only
   hosts are listed but cannot be ticked. The number of selected hosts is shown.
3. Click **Add key** or **Revoke key**. Telepad lists the key and every selected host;
   type `add` or `revoke` and press Enter (or the button) to start, or **Cancel**.
4. The hosts are done in parallel; each shows its result:
   - **added** / **already present (not changed)** for Add,
   - **removed** / **not present (not changed)** for Revoke,
   - **refused** — an RDP-only host, or the key Telepad itself logs in with (revoking it
     would lock Telepad out of that host),
   - **failed** with the reason (host down, `~/.ssh` not writable, …).

   The status bar sums it up when all hosts are done.

Add creates `~/.ssh` (mode 700) and `authorized_keys` (mode 600) when they are missing
and never adds a key twice. Revoke removes every line with that key, whatever its comment
or options. Other lines are never touched, and the file is rewritten through a temporary
copy, so a failure leaves it as it was.

## Database browser (MariaDB / MySQL / PostgreSQL)

Telepad opens the databases it already sees, HeidiSQL style, through the SSH login it
already uses: no database port is opened to the network and no tunnel or client has to
be set up by hand.

**Where the button is**

- A running container whose image is MariaDB, MySQL, Percona or PostgreSQL (also
  `bitnami/postgresql`, PostGIS, TimescaleDB): its **☰** menu (host detail → Docker, and
  Overview — Docker) has **Open database…**. Tools such as exporters, phpMyAdmin,
  Adminer and pgAdmin are not offered.
- A database installed on the host itself (`mysqld` / `mariadbd` / `postgres` in the
  listening ports, or port 3306 / 5432 with no visible owner that no container
  publishes): the host detail header shows **Open MariaDB / MySQL :3306** or
  **Open PostgreSQL :5432**.

Each database opens in its own window inside Telepad. Opening the same one again does nothing while
its window is open.

**Connect**

The window starts with the connect dialog (like HeidiSQL's session manager):

- **User**, **Password**, **Database** (optional; PostgreSQL uses `postgres` when empty).
  For a container the user is filled in from its `MARIADB_USER` / `MYSQL_USER` /
  `POSTGRES_USER`; only that name is read (the filtering happens on the host, so
  passwords in the container's environment never leave it).
- **Save password in the system password store**: Windows Credential Manager, macOS
  Keychain, or the Linux Secret Service (GNOME Keyring on Ubuntu and CentOS / RHEL
  desktops, KWallet on KDE). With a saved password, leave the field empty. When no store
  is available (e.g. a Linux desktop without a keyring) the box is disabled and the
  password is asked for each time. Passwords are never written to `telepad.db` or any
  other file.
- **Read-only** (off by default): the server refuses changes from this window, and only
  `SELECT`, `SHOW`, `DESCRIBE`, `EXPLAIN` and `WITH` run in the Query tab. It can be
  switched on and off at any time from the toolbar and is remembered per connection.
- **Connect** opens `ssh -N -L` to the container's address on its Docker network (or to
  the address the host database listens on) and logs in. User, database and the switches
  are remembered. **Forget saved connection** deletes them and the saved password.

If the SSH server does not allow port forwarding (`AllowTcpForwarding no`) or the host
cannot reach the database address, the dialog says so.

**The window**

- Toolbar: **Refresh** (database list and the open database), **Read-only**,
  **Reconnect** (close the connection and log in again, e.g. after the network dropped).
- Left: the tree. MariaDB / MySQL: databases → tables and views. PostgreSQL:
  databases → schemas → tables and views. Click a database or schema to load it, a table
  to open it; right-click for **Open data**, **Structure**, **New table…**,
  **Empty (TRUNCATE)…** and **Drop…**. The box on top filters by name.
- Tabs:
  - **Host**: server version, databases with their size.
  - **Database**: tables of the chosen database / schema with estimated rows, size,
    engine or kind and comment; **New table…**.
  - **Table**: columns (name, type, length / set, unsigned, NULL, default, extra,
    comment), indexes, foreign keys, and for MySQL the CREATE statement.
  - **Data**: the rows (see below).
  - **Query**: SQL (see below).
- The status line at the bottom shows the last error or success.

**Data tab**

- 1000 rows per page (100 / 500 / 5000 in the page box), **◀ ▶** for the previous /
  next page, **↻** to reload. Click a column header to sort: ascending, descending, off.
- The filter box takes a `WHERE` condition in SQL, e.g. `city = 'Springfield'`; Enter or
  **Filter** applies it, **Clear** removes it.
- NULL is shown as a grey *NULL*. Long values and binary data are cut (the full length is
  shown); right-click a cell for **Copy value** and **View value…**.
- Editing (only for tables with a primary key or a NOT NULL unique key, and not in
  Read-only): double-click a cell (the value is selected, type to replace it, Enter to
  keep, Esc to cancel), or right-click → **Edit** / **Set NULL**. **Insert row** adds a row
  whose empty cells take the server's default; **Delete row** marks the selected row
  (again to unmark). Changed cells are highlighted.
- Nothing is sent until **Post (n)**: it shows the exact SQL; **Run** saves everything in
  one transaction, and every statement must change exactly one row, otherwise nothing is
  saved and the error is shown (the edits stay so they can be corrected). **Discard** drops
  them. Reload, sort, filter and paging wait until the changes are posted or discarded.
- Tables without such a key, views, binary columns and cut values are shown read-only,
  with the reason above the grid; change them with a query.

**Query tab**

- Several query tabs (**+**, and the close button for the current tab). Queries run in
  the database (PostgreSQL: and schema) chosen in the tree.
- **Run** (F9 or Ctrl+Enter) runs the selection, or everything when nothing is selected;
  **Run current statement** (Shift+F9 or Ctrl+Shift+Enter) runs the statement at the
  cursor. Statements run one after the other and stop at the first error, which shows the
  server's message and line. **Cancel** stops a running statement.
- `DROP`, `TRUNCATE`, `ALTER … DROP`, and `UPDATE` / `DELETE` without `WHERE` first show
  a confirmation with the statements.
- Each statement's result is a tab: rows (up to 10 000 kept) or rows affected, with the
  time it took.

**Table structure**

- **Edit structure** turns the Table tab into an editor: columns (name, type, length /
  set, unsigned, NULL, default — no default, NULL, text or an SQL expression —,
  auto-increment / identity, collation, comment; add, remove, and for MySQL move up /
  down), indexes (PRIMARY, UNIQUE, KEY, and FULLTEXT for MySQL; pick the columns) and
  foreign keys (columns, referenced schema / table and columns, ON UPDATE / ON DELETE).
- **Save…** shows the generated `ALTER TABLE` statements and runs them only after
  **Run**; **Discard** drops the edits. Index and key columns follow a renamed column.
  PostgreSQL runs all statements in one transaction (a failure changes nothing) and
  cannot reorder columns; MySQL commits each statement on its own.
- **New table…** (Database tab or right-click a database / schema) starts with an
  auto-increment `id` primary key; **Create…** shows the `CREATE TABLE`.
- **Rename…**, **Empty…** (TRUNCATE) and **Drop…** show the statement; Empty and Drop
  also ask you to type the table name.

Closing the window, **Reconnect**, or quitting Telepad stops the SSH tunnel.

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

The rule allows only those two read-only commands. This is the only change Telepad ever
offers to make on a host, and only in a terminal you opened yourself. Virtual machines
have no BMC; the Sensors section says which step is missing on a host that has one.

## Updates

Telepad checks the latest [GitHub release](https://github.com/yohannaftali/telepad/releases)
at start and every 6 hours, in the background, over HTTPS (no account or token). It reads
github.com's normal release pages, not the GitHub API, so an office where many people
share one internet address does not run into GitHub's API limit. If a check fails, it is
tried again after 30 minutes and the reason shows in **Software update…**.

1. When a newer version exists, the top bar shows **Update X available**.
2. Click it (or **☰ → Software update…**) to see the version and a link to the
   release notes.
3. **Update now** downloads the zip for your platform, verifies it against the release's
   `.sha256` and replaces `telepad.exe` and `telepad-check.exe`. If anything fails,
   nothing is replaced.
4. **Restart now** (or **Restart for X** in the top bar) starts the new version.

**Later** hides the button until the next start; **Check now** checks immediately.
Your `telepad.db`, `config` and `.env` are never touched. Under **Update source** you
can turn the checks off or point them at a fork's repository.

**Install automatically** (under **Update source**, off by default) skips steps 1–3:
when a check finds a newer version, Telepad downloads it, verifies the `.sha256` and
installs it in the background. Nothing restarts by itself: the top bar shows
**Restart for X**, or the new version starts the next time you open Telepad. **Later**
also pauses automatic installs until the next start.

## Troubleshooting

| Message | What to check |
|---|---|
| `authentication failed` | The key is not accepted or the host wants a password. Try `ssh <alias>` in a terminal; check IdentityFile and the server's `authorized_keys`. A key with a passphrase must be loaded in the ssh agent first (macOS: `ssh-add --apple-use-keychain ~/.ssh/id_ed25519`). |
| `private key … is readable by other users` | ssh ignores private keys that others can read (common after copying keys from another computer). Run the `chmod 600 <key>` shown in the message, and `chmod 700 ~/.ssh`. |
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
| Database | `--db <path>`, `TELEPAD_DB`, else `telepad.db` next to `config`, else next to the exe for a portable copy (`portable` file or an existing `telepad.db` there), else the per-user data folder: `%APPDATA%\Telepad`, `~/Library/Application Support/Telepad`, `~/.local/share/telepad` |
| First-run import | `--config <path>`, `TELEPAD_CONFIG`, `./config`, `config` next to the exe, or `config` in the data folder; only read while the database is empty |
| Generated ssh config | `telepad.ssh_config` next to the database (do not edit: rewritten on every change) |
| Fallback key for hosts without IdentityFile | `SSH_IDENTITY_FILE=` in a `.env` file next to `config` |
| ssh binary | `TELEPAD_SSH` (default: Windows `System32\OpenSSH\ssh.exe`, else `ssh` on `PATH`) |

Security model: see [SECURITY.md](../SECURITY.md).
