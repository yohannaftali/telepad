# telepad-app

`crates/telepad-app` — the application state and all of its behaviour, written in the
Model–View–Update (MVU) style. No GUI toolkit, no I/O: it can be tested completely without a
window, a network or a disk.

[Back to the developer guide](../DEVELOPER_GUIDE.md#crates) · other crates:
[telepad-core](telepad-core.md) · [telepad-gui](telepad-gui.md) · [telepad-check](telepad-check.md) ·
outside libraries: [DEPENDENCIES.md](../DEPENDENCIES.md)

## At a glance

| | |
|---|---|
| Kind | library (`telepad_app`) |
| Used by | [`telepad-gui`](telepad-gui.md) |
| Depends on | [`telepad-core`](telepad-core.md) only — **no outside libraries at all** |
| Size | ~26 000 lines, about half of them behaviour tests |
| Tests | `cargo test -p telepad-app` |

## Rules

- **`update()` is the only code that changes the `Model`.**
- **No I/O.** No files, sockets, processes, threads, clipboard or clock. Work for the outside
  world is returned as an `Effect`; its result comes back later as a `Msg`.
- **Time is a parameter.** `update(&mut State, Msg, now: Instant)` — tests pass any time they like.
- **No GUI types.** No egui; the workspace keeps window rectangles as its own `Rect = [f32; 4]`.
- **Every behaviour change gets a test** in the matching `tests_*.rs` file.

## The core types

| Type | Where | Meaning |
|---|---|---|
| `State { model, drafts }` | `state.rs` | everything the app knows |
| `Model` | `state.rs` | facts: hosts and their `HostState`, sessions (database, file, editor, Portainer, Log info), the `Workspace` (inner windows), update state, messages. Changed only by `update()` |
| `Drafts` | `state.rs` | what the user is typing or toggling: form fields, search text, sort, table states (`Drafts::tables`), theme. Views may write these directly — they are not facts yet |
| `Msg` | `msg.rs` | a user intent (`Msg::Files(FmMsg::Open(alias))`, a button click) or the result of an effect (`Msg::HostsLoaded`, `Msg::Collected`) |
| `Effect` | `msg.rs` | work for the runtime (`Effect::LoadHosts`, `Effect::Collect(jobs)`, `Effect::SaveUiPrefs`, …) |
| `update` | `update.rs` | `fn update(&mut State, Msg, Instant) -> Vec<Effect>` |

Big features have their own small MVU inside: a model struct, a draft struct, a message enum
and an effect enum, with an `update` that the main `update()` forwards to.

| Feature | Model / Draft | Messages / Effects | Tests |
|---|---|---|---|
| Database windows | `db.rs`: `DbSession` / `DbDraft` | `DbMsg` / `DbEffect`, `DbJob` | `tests_db.rs` |
| File windows | `files.rs`: `FileSession` / `FileDraft` | `FmMsg` / `FmEffect`, `FileOp` | `tests_files.rs` |
| Editor | `editor.rs`: `FileEditor`, `EdTab` / `EdDraft` | `EdMsg` | `tests_editor.rs` |
| Host Management | `hostsync.rs`: `HostSync` / `HostSyncDraft` | `HsMsg` / `HsEffect` | `tests_hostsync.rs` |
| Workspace (inner windows) | `workspace.rs`: `Workspace`, `Group`, `Win` | `WsMsg`, `WkMsg` | `tests_workspace.rs`, `tests_leave.rs` |
| My keys | `mykeys.rs`: `MyKeys` / `MyKeysDraft` | `MkMsg` / `MkEffect` | `tests_mykeys.rs` |
| Log info | `logins.rs`: `LoginSession` / `LoginDraft` | `LgMsg` / `LgEffect` | `tests_logins.rs` |
| Groups and clusters | `sets.rs`: `SetPage`, `SetsDraft` | `StMsg` / `StEffect` | `tests_sets.rs` |
| Portainers | `portainer.rs`: `PtRow`, `PtPage`, `StackEditor` / `PtDraft`, `EditorDraft` | `PtMsg` / `PtEffect` | `tests_portainer.rs` |

Shared, toolkit-free helpers used by the views:

| Module | What it does |
|---|---|
| `table.rs` | Hosts order (`Sort`, `overview_rows`), drag to reorder (`drop_index`), Docker stacks, cluster rows, host flags and urgency |
| `grid.rs` | State of the shared table component: sort, filter boxes (`filter_match`), natural compare with units, the value dropdown (`distinct_values`), export rows |
| `pager.rs` | Pager: page, rows per page, record info, continuous mode |

## One round trip

```rust
// test: opening a file window asks the runtime to connect
let mut s = State::default();
let fx = update(&mut s, Msg::Files(FmMsg::Open("web1".into())), now);
assert!(fx.iter().any(|e| matches!(e, Effect::Files(FmEffect::Connect { .. }))));
// the runtime would now connect and send back Msg::Files(FmMsg::Connected { .. })
```

In the real app:

1. A view returns `Msg::Files(FmMsg::Open(alias))` from a button click.
2. `telepad-gui`'s runtime calls `update()`, which opens the inner window in the `Workspace`,
   creates a `FileSession` and returns the effect.
3. The runtime connects over SFTP on a thread, then dispatches the result `Msg`.
4. `update()` stores the listing; the next frame draws it.

## Adding a feature

1. Add the `Msg` (and an `Effect` if it needs the outside world) in `msg.rs` or the
   feature's own enum.
2. Handle it in `update()` and write a test in `tests_*.rs`.
3. Run the effect in [`telepad-gui`'s runtime](telepad-gui.md#runtime).
4. Draw it / emit the `Msg` from a view.

The [developer guide](../DEVELOPER_GUIDE.md#adding-a-feature-end-to-end) walks through a full
example.

## Tests

The tests drive `update()` with messages and check the model and the returned effects —
no window, no ssh, no database. Helpers such as a `State` with a few made-up hosts live at the
top of `tests.rs`. Test data uses documentation addresses (`192.0.2.0/24`,
`198.51.100.0/24`) and made-up host names.

```
cargo test -p telepad-app
cargo test -p telepad-app tests_files     # one feature
```
