# telepad-check

`crates/telepad-check` — the headless command-line tool. Builds the `telepad-check`
executable, which uses [`telepad-core`](telepad-core.md) directly to collect hosts once and
print the result as text or JSON, check for updates, or read Log info, My Computer and
Portainers — without a window.

[Back to the developer guide](../DEVELOPER_GUIDE.md#crates) · other crates:
[telepad-core](telepad-core.md) · [telepad-app](telepad-app.md) · [telepad-gui](telepad-gui.md) ·
outside libraries: [DEPENDENCIES.md](../DEPENDENCIES.md)

## At a glance

| | |
|---|---|
| Kind | binary `telepad-check` (`src/main.rs`, one file) |
| Depends on | `telepad-core`, `serde_json` |
| Shipped | in every release zip next to `telepad`; the self-updater replaces both |
| Run | `cargo run --release --bin telepad-check -- <args>` |

## Why it exists

- **Testing without the GUI.** Collection, parsers, the update check and the Portainer
  client can be tried against real hosts from a terminal; the `tester` workflow uses it, and the release workflow runs `telepad-check --help` on every package.
- **Scripting.** `--json` gives the same `HostReport` the GUI shows, for other tools.
- **Debugging a host.** When a host looks wrong in the GUI, `telepad-check <alias>` shows
  what was collected and the error, if any.

It uses the same `telepad.db`, generated `telepad.ssh_config` and ssh settings as the GUI
(`open_workspace`), so a host that works here works in the GUI.

## Usage

| Command | What it does |
|---|---|
| `telepad-check [alias ...]` | Collect the named hosts (all monitored hosts if none) in parallel, one thread per host, and print a summary per host |
| `telepad-check --json [alias ...]` | The same as JSON: `[{"alias", "host", "ok", "report" \| "error"}]` |
| `telepad-check --check-update` | Ask GitHub for the latest release of the configured update repository |
| `telepad-check --logins alias ...` | SSH logins from the systemd journal (Log info, #126) |
| `telepad-check --local [--json]` | Collect this computer (My Computer, #129) |
| `telepad-check --portainer <name> [--json]` | Read a registered Portainer: status, environments, stacks (#171) |
| `telepad-check --portainer <name> --stack <stack>` | One stack: variable *names*, compose file size, containers (#173); read-only, never values |

Common options: `--db <telepad.db>`, `--config <ssh config>`, `-h` / `--help`.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | everything succeeded / up to date |
| 1 | at least one host (or the Portainer read) failed |
| 2 | setup error: database could not be opened (including a newer schema), unknown alias, bad settings, update check failed |
| 10 | `--check-update`: a newer release is available |

## How it is built

`main()` parses the arguments by hand (no argument-parser dependency), opens the workspace,
and calls one `run_*` function:

```rust
let ws = open_workspace(cfg.as_deref(), db.as_deref())?;        // same as the GUI
let settings = Arc::new(ws.settings());
let handles: Vec<_> = hosts.into_iter().map(|h| {
    let s = settings.clone();
    thread::spawn(move || (h.clone(), collector::collect_one(&s, &h)))
}).collect();
report(&results, json)                                           // text or serde_json
```

It never writes to a host and never changes `telepad.db` (except creating it on the very
first run from `config`, exactly as the GUI would, and saying so on stderr).

## Adding a command

1. Add the flag in the argument loop of `main()`.
2. Write a `run_<name>` function that calls `telepad-core` and returns an `ExitCode`
   following the table above.
3. Add a line to the usage comment at the top of `main.rs`, the `--help` text, the
   *Validate* list in [`AGENTS.md`](../../AGENTS.md#validate) and the command-line section of
   the [user manual](../USER_MANUAL.md).
