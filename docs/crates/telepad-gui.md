# telepad-gui

`crates/telepad-gui` — the desktop application. Builds the `telepad` executable: an egui
window (via eframe) that draws the [`telepad-app`](telepad-app.md) state and a runtime that
executes its effects with [`telepad-core`](telepad-core.md).

[Back to the developer guide](../DEVELOPER_GUIDE.md#crates) · other crates:
[telepad-core](telepad-core.md) · [telepad-app](telepad-app.md) · [telepad-check](telepad-check.md) ·
outside libraries: [DEPENDENCIES.md](../DEPENDENCIES.md)

## At a glance

| | |
|---|---|
| Kind | binary `telepad` (`src/main.rs`) |
| Depends on | `telepad-core`, `telepad-app`, `eframe` (egui), `egui_extras`, `rfd`, `arboard`; build: `winresource` (Windows) |
| Size | ~23 000 lines |
| Tests | `cargo test -p telepad-gui` (theme contrast, glyph coverage, PDF fonts, a few runtime helpers) |
| Run | `cargo run --release --bin telepad` |

## Rules

- **Views never do I/O.** A view reads `&Model`, may edit `&mut Drafts`, and returns `Msg`s.
  No core I/O, no `platform`, no `rfd`, no threads in `views/`.
- **The runtime is the only GUI code that touches the outside world** (database, ssh,
  network, files, clipboard, OS). Slow work runs on threads; **the UI thread never waits on
  ssh or the network**. Native file dialogs (`rfd`) are the one allowed blocking call — they
  are modal by nature.
- **Only one OS window.** Pages, inner windows and dialogs are drawn inside it.
- **Every table uses `crate::table`**, never a bare `TableBuilder` / `egui::Grid` with a header.
- **Every character in the UI has a glyph** (test `every_character_in_the_ui_has_a_glyph`).

## Files

| Path | What it does |
|---|---|
| `src/main.rs` | Arguments (`--config`, `--db`, `--prefs`, `--history`, `--update-db`, `--open-ssh <alias>`); no console window in Windows release builds (`windows_subsystem = "windows"`) |
| `src/app.rs` | `run()` opens the eframe window (title, size, icon); `TelepadApp` installs fonts, image loaders and themes; each frame: `runtime.frame()` → apply theme → `views::draw()` → `runtime.dispatch()` every returned `Msg`; `on_exit` flushes the writer threads |
| `src/runtime.rs` | `Runtime`: executes `Effect`s and feeds results back as `Msg`s — see below |
| `src/views/` | One module per screen (`panels`, `workspace`, `overview`, `detail`, `docker`, `database`, `files`, `text_editor`, `portainer`, `keys`, `mykeys`, `logins`, `strategy`, `host_sync`, …); `views::draw` is the entry point |
| `src/table.rs` | The shared table component: columns, sort, filter row, value dropdown, pager, right-click menu, exports |
| `src/widgets.rs` | Drawing helpers and colours: bars, status dot, action buttons by risk (`Act`), host right-click menu, light / dark visuals, `Palette` |
| `src/editor_theme.rs` | Editor colour schemes (`SCHEMES`) |
| `src/keymap.rs` | Editor keyboard shortcut profiles (#205): commands, the six profiles, the matcher that takes their keys out of a frame's input before the text field sees them |
| `src/icons.rs` | Phosphor icon font, font setup (`install`, `definitions`), OS logo detection |
| `assets/` | Fonts (`Phosphor.ttf`), OS logos (SVG), app icon (`.png`, `.ico`, `.icns`) |
| `build.rs` | Windows only: embeds `telepad.ico` and version info into `telepad.exe` with `winresource` |

## Runtime

`Runtime` (`src/runtime.rs`) owns everything that is not state:

| Field | What it is |
|---|---|
| `ctx` | the `egui::Context`, so threads can call `request_repaint()` when a result arrives |
| `ws`, `ssh` | the opened `telepad_core::Workspace` (store, config path) and ssh settings |
| `collector` | the collection worker pool and its `Receiver<collector::Update>` |
| `tx` / `rx` | channel for results of all other background threads, already as `Msg` |
| `db` | one worker thread per database window (dropping it closes the connection and tunnel) |
| `files` | one browse worker (one SFTP connection) and transfer cancel flags per file window |
| `prefs`, `history`, `strategy` | one writer thread each for `preference.db`, `history.db`, `update.db` |

Three entry points, all called from `app.rs`:

- `Runtime::start` — opens the workspace, starts threads, sends the first messages.
- `Runtime::frame(&mut State)` — once per frame: drains the channels (`try_recv`, never
  blocking) and dispatches the `Msg`s they hold.
- `Runtime::dispatch(&mut State, Msg)` — calls `telepad_app::update`, then runs each returned
  `Effect`: quick ones inline (save a host, read the clipboard), slow ones on a thread that
  later sends a `Msg` through `tx`.

```
frame N:   views::draw ──Msg──► dispatch ──update()──► Effect::Collect ──► worker thread
frame N+k: runtime.frame ◄── channel ◄── worker done        (ctx.request_repaint())
           dispatch(Msg::Collected) ──update()──► model changed ──► next draw shows it
```

## Views

`views::draw(ui, &Model, &mut Drafts, now) -> Vec<Msg>` draws the top bar, the page sidebar,
the inner windows of the `Workspace` and any dialog. Each view module has the same shape:

```rust
pub fn show(ui: &mut egui::Ui, model: &Model, drafts: &mut Drafts, out: &mut Vec<Msg>) {
    ui.text_edit_singleline(&mut drafts.filter);          // drafts: edit in place
    if ui.button("Refresh").clicked() {
        out.push(Msg::Refresh);                           // intent: a Msg, never I/O
    }
}
```

The [developer guide](../DEVELOPER_GUIDE.md#egui-as-telepad-uses-it) explains how egui is used
(immediate mode, ids, inner windows, the table component, the editor).

## Tests

GUI tests check facts that matter without opening a window: contrast of every theme and
editor scheme, that every character used in UI text has a glyph, that the PDF export finds its
fonts. Behaviour is tested in [`telepad-app`](telepad-app.md), not here.
