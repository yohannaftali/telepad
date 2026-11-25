# Change History

Newest first.

## [2026-11-25] — feat(ui): Hosts cards — equal height, whole-card click; Group / Cluster sort instead of By group (#157)
- Issue #157 addressed on GitHub (branch `157-cards-sort`, on top of #156)
- `telepad-app`:
  - `table.rs`: `SortKey::Group` / `Cluster` (A to Z, hosts without one last), also in `ALL` for the cards'
    Sort by picker.
  - `sets.rs`: `hosts_by_group` removed, and `hosts_columns` defaults to Group and Cluster.
- `telepad-gui`:
  - `views/mod.rs`: the "By group" checkbox is gone.
  - `views/overview.rs`: Group / Cluster columns are sortable, and `TableView::by_group` is removed.
  - `views/sets.rs`.
  - `views/cards.rs`: every card is as tall as the tallest (measured each frame), and a click anywhere on a card
    opens the host.
- Verified: fmt, clippy, tests (new `group_and_cluster_columns_sort_on_a_header_click_and_show_by_default`); GUI
  on Windows: Group / Cluster checked by default, Group header sort, cards with equal height and Sort by Group,
  and a card click opens the host page.

## [2026-11-25] — fix(ui): every table fills its page; title row and filter box have the same height (#156)
- Issue #156 addressed on GitHub (branch `156-table-fill`)
- `telepad-gui/src/table.rs`:
  - `fill_widths` and a kept `Layout` (the #143 rules for every table): spare room goes to the long text
    columns, and narrow pages shrink columns toward their titles, then scroll sideways.
  - Dragged widths stay until double-click or Auto-fit (new header menu item).
  - Button columns reserve their content width, and the last fitted column fills to the edge.
  - The filter box gets vertical margin so it is as tall as the title row (header = 2 × row + spacing).
- `views/sets.rs`: the Groups / Clusters table has fitted columns instead of fixed 60 / 250 pt ones.
- Verified: fmt, clippy, tests (3 new `fill_widths` tests); GUI on Windows with screenshots: Groups, dockerho
  Docker tables (wide and narrow), Log info header.

## [2026-11-25] — feat(ui): Hosts cards with equal height and whole-card click, sortable Group / Cluster columns instead of By group, shown by default
- Issue #157 created on GitHub
- Scope: telepad-app (table.rs SortKey Group / Cluster, sets.rs), telepad-gui (Hosts toolbar, cards, overview)
- Labels: enhancement
- Owner decision: "By group" is removed, no Group by dropdown; Group / Cluster columns sort on a header click

## [2026-11-25] — fix(ui): simple tables do not fill the page like Hosts, and header title / filter heights are unbalanced
- Issue #156 created on GitHub
- Scope: telepad-gui (table.rs widths and header layout; docker, ports, sets views)
- Labels: bug

## [2026-11-25] — Telepad 0.9.10
- **Tables** (#153): every table is built from one component, simple or paged. Every table sorts on a header
  click and has a filter row above the titles (contains, `=x`, `!x`, `>x` / `<x` with units such as `>50` for
  `75%`, `NULL`, `!NULL`). The right-click menu shows the table's own commands first, then **Filter by**,
  **Clear All Filters**, **Export as CSV / Excel (.xlsx) / PDF** and **Copy as JSON**. These cover every
  filtered and sorted row on every page. A PDF with more than 5000 rows asks first. Columns fit their text, and
  wide tables scroll sideways.
- **Table headers** (#151): headers have their own colour band with a line under it. Sortable titles light up
  on hover, and the sort arrow sits at the right edge of the title.
- **Pager** (#152): the record info is on the left; Rows per page, « ‹ Page › » and a **continuous mode**
  toggle are on the right. In continuous mode, rows load as you scroll; the database grid fetches the next
  server page, and unsaved edits stay. The footer wraps in narrow windows.

## [2026-11-25] — Tested and merged: #153, #151, #152 (PR #154)
- Tested on Windows 11 with a release build against the real inventory (read-only). The GUI was driven by
  mouse and keyboard automation and checked with screenshots.
- Pages checked: Hosts, the dockerho host page, the file window, Authorized keys, Log info, and the database
  grid (tire-mariadb, nothing posted), in dark and light themes. Exports were opened and checked: CSV with BOM
  (57 lines), xlsx (57 rows, autofilter), PDF (2 pages, landscape).
- CI passed on Windows, macOS and Linux (check and package builds), plus `db-live` and `files-live`. All three
  issues were closed with test comments.
- Fixed while testing (`b4eca1d`):
  - Hosts filter boxes lost focus after one character.
  - Right-click Filter by took the first column instead of the clicked one.
  - Simple-table columns stayed squeezed.
  - PDF column titles were cut short.
  - The pager's right-hand controls wrapped out of sight.

## [2026-11-25] — feat(ui): pager footer — record info left, controls right, continuous mode (#152)
- Issue #152 addressed on GitHub (branch `152-pager-continuous`, on top of #151)
- `telepad-app/src/pager.rs`: continuous mode (`toggle`, `start_mode` < 768 pt, `more`, `wants_more` with the
  500-row auto-fill cap, `size_label` "Rows per load"), `reset` also resets the loaded rows; `db.rs`:
  `DbMsg::DataMore` adds the next server page (one request at a time, `appending` / `exhausted`), edits keep their
  rows, the mode switch reloads the first page; `widgets::pager_bar`: record info left, Rows per page | « ‹ Page ›
  » | ∞ / numbered-list toggle right-aligned (wraps when narrow); `table.rs` reports `near_bottom` / `fits`
  (within 80 pt of the bottom, as wtbapp); Log info and in-memory paged tables load more on scroll
- Verified: fmt, clippy, tests (pager continuous / narrow start, database append test). Not verified on screen
  (session locked)
- Docs: manual (Tables ▸ Pager footer, Log info and Data tab pager), AGENTS.md (`pager.rs` row)

## [2026-11-25] — feat(ui): table headers stand out — header band, hover highlight, right-aligned sort icon (#151)
- Issue #151 addressed on GitHub (branch `151-table-header`, on top of #153)
- `telepad-gui/src/table.rs` `title_line`: header band colour + line under it, sortable titles light up on hover
  (background and text), title cut with "…" left, sort caret right-aligned; `widgets.rs` `Palette::head`
  (`HeadColors`, dark and light); Hosts auto-fit always reserves the caret width (`overview.rs` `header_text`)
- Every table gets it through the #153 component (no per-view code)
- Verified: fmt, clippy, tests (new `table_header_colours_are_readable_and_stand_out`: WCAG contrast of header /
  hover text, band vs rows, both themes). Not verified on screen (session locked)

## [2026-11-25] — feat(ui): table framework — simple and paged tables, filter row, right-click menu, exports (#153)
- Issue #153 addressed on GitHub (branch `153-table-framework`)
- New: `telepad-gui/src/table.rs` (component: `show` / `controlled`, `Kind::Simple` / `Fill` / `Paged`, filter row
  above the titles, header sort, right-click = own items + Filter by / Clear All Filters / Export CSV, Excel, PDF /
  Copy as JSON), `telepad-app/src/grid.rs` (`TableState`, wtbapp filter rules with units, natural / unit sort,
  `TableMsg`), `telepad-core/src/table_export.rs` (CSV with BOM, JSON, `.xlsx`, PDF via `pdf-writer` 0.15.0 +
  `ttf-parser` 0.25.1 with embedded UI fonts, `.telepad-part` writes); `Effect::ExportTable` / `Effect::CopyText`,
  PDF above 5000 rows asks first
- Converted: Hosts (all host pages), Groups / Clusters, disks, network, ports, fans, temperatures, Docker containers
  (host page and Docker overview), Swarm / Kubernetes tables, file panes and History, database lists / structure
  columns / query results / Data grid (server sort and filter kept; CSV / Excel open the #139 Export dialog),
  Host Management, Authorized keys (by server / by key), My keys, Log info, Update Strategy hosts
- Not converted (forms / logs, not data tables): key-value grids (System, Usage, Maintenance, forms), structure
  editor, import mapping / preview, key run results, SQL log
- Verified: `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` (new: 7 core export tests, 7 app
  behaviour tests, GUI PDF font test); sample PDF rendered with PyMuPDF (header, stripes, `→`, `°C`, copyable text).
  Not verified: the GUI on screen (Windows session locked, screenshots black) — tester checklist
- Docs: `docs/USER_MANUAL.md` (new "Tables" section, filter boxes above titles), AGENTS.md (modules, table rule)

## [2026-11-25] — feat(ui): table framework — simple and paged tables, sort and filter row everywhere, right-click menu with Filter by, Clear All Filters, export CSV / Excel / PDF, Copy as JSON
- Issue #153 created on GitHub
- Scope: telepad-gui (table component, every table view), telepad-app (table state, Msg / Effect), telepad-core (table export: CSV / xlsx / PDF / JSON), runtime
- Labels: enhancement
- Owner decision: new issue (not part of #151); sort + filter row on every table, simple and paged

## [2026-11-25] — feat(ui): pager footer — record info left, controls right, continuous mode toggle (wtbapp / Senar style)
- Issue #152 created on GitHub
- Scope: telepad-app (pager.rs, db.rs, logins.rs), telepad-gui (widgets::pager_bar, database / logins views)
- Labels: enhancement

## [2026-11-25] — feat(ui): shared table header component — header background, hover highlight, right-aligned sort icon
- Issue #151 created on GitHub
- Scope: telepad-gui (shared table header module, all TableBuilder / Grid tables)
- Labels: enhancement

## [2026-11-24] — Telepad 0.9.9
- **Groups and Clusters** (#144, #145): optional, independent labels for hosts. New Groups /
  Clusters pages (sidebar under My Computer) with a fixed Ungrouped / No cluster entry; add,
  rename, describe, reorder, delete; each opens a "Hosts - [name]" page (table / cards, Add hosts…).
  Set from the host menu, the editor or Host Management; cards show them; the Hosts page can list
  by group and show Group / Cluster columns. Kept in JSON / OpenSSH export (schema 5).
- **Hosts table** (#143): columns fit the page and follow every resize; Flags gets its room and
  the normal text size (icons only, never cut off, when a row's labels do not fit); dragged widths
  stay until double-click / Auto-fit (#150).
- **Hosts that drop ping** (#125): a TCP check of the SSH port tells up from down, so they are no
  longer held down for 10 minutes after one failed SSH attempt.

## [2026-11-24] — Tested and merged: #143, #144, #145 (PR #148)
- Windows 11, release build, copy of the inventory; CI green on Windows, macOS, Linux, `db-live`,
  `files-live`. Issues closed with test comments. Fixed while testing: set dialogs were drawn
  twice with Groups and Clusters both open (doubled typing).

## [2026-11-24] — fix(ui): flags are never cut off in the Hosts table (follow-up to #143)
- With Group / Cluster columns shown and the sidebar open, the Flags cell was cut (`EOL …`).
  Group / Cluster columns now give way with the long text columns (first), and a row whose flag
  labels still do not fit shows the coloured flag icons only, details on hover.
- Files: `gui/src/views/overview.rs` (+1 `fit_widths` test), `docs/USER_MANUAL.md`.
- Verified: fmt, clippy, tests; GUI on an inventory copy with Group + Cluster columns, sidebar open.

## [2026-11-24] — feat(hosts): host groups and clusters (#144, #145)
- Issues #144 and #145 addressed on GitHub (branch `144-host-groups-clusters`, one model for both).
- Core: `HostEntry::group` / `cluster`, `SetKind` / `HostSet` / `check_set_name`; OpenSSH tags
  `# telepad: group|cluster <name>`; `store` schema 5 (`hosts.grp` / `hosts.cluster`,
  `host_sets`), `sets` / `save_set` / `delete_set` / `order_sets` / `assign_set`; JSON export /
  import with `sets`; Host Management compares Group / Cluster.
- App: `sets.rs` (Groups / Clusters pages, "Hosts - [name]" pages restored with the workspace,
  add / edit / reorder / delete, Add hosts…, assign); a group change keeps the host's report.
- GUI: sidebar My Computer – Groups – Clusters – Hosts; `views/sets.rs`; host menu Group / Cluster
  submenus; editor Group / Cluster; cards show both; Hosts page By group / Group / Cluster columns.
- Verified: fmt, clippy, tests (core +2, app +7); GUI on an inventory copy: add / rename / page
  follows / Add hosts / right-click assign / By group / cluster page with Group column / restore.

## [2026-11-24] — fix(ui): Hosts table fills the page width, columns follow the page (#143)
- Issue #143 addressed on GitHub (branch `143-hosts-table-width`).
- `views/overview.rs`: `fit_widths` (content widths over every listed host, spare width to
  Description / OS / Flags / Docker, shrink order: long text, then bars, then Host / Flags /
  Updated), recomputed every frame so resizing the page re-flows the columns; a dragged column
  keeps its width (`Layout::manual`), double-click on its border or header ▸ Auto-fit columns
  makes it automatic again; a click on a border never sorts. Flag chips in the normal text size
  (`widgets::chip_text`), all flags on hover when cut.
- Verified: fmt, clippy, tests (gui +3 `fit_widths` tests); GUI on an inventory copy: maximized
  window, sidebar collapsed, drag / double-click a border, narrow 1400 px window (scrolls).

## [2026-11-24] — Tested and merged: #125 (PR #146)
- Windows 11, release build; fmt, clippy, tests green; CI green on Windows, macOS, Linux, `db-live`, `files-live`.
- `telepad-check` on a copy of the 56-host inventory: 8 failures. GUI on the same copy for ~5 min: 49 up / 7 down, all
  among the 8 (the 8th a one-off timeout on a host that answers ping); the ICMP-blocked host with SSH up stays up.
  Issue closed with the test summary.

## [2026-11-24] — fix(collector): hosts that drop ICMP ping are tried over SSH again next round (#125)
- Issue #125 addressed on GitHub (branch `125-ping-blocked-hosts`).
- `collector::reach`: ping, else a TCP connect to the SSH port (`Reach::Ping` / `Port` /
  `Silent` / `Indirect`); used after a failed SSH attempt and in the ping-first path, and
  in the ping round only for hosts that are down (`PingJob { host, port }`), so up hosts
  get no extra connections in their sshd log. `Outcome` / `Update::Done` carry `reach`
  instead of `no_ping`; `HostEntry::is_proxied`.
- App: `no_ping` (check before SSH) only for `Silent` hosts, or proxied hosts after two SSH
  failures in a row (`HostState::ssh_fails`); `port_open` shown on the host page and the
  Update Strategy list; a down host whose SSH port opens is read at once.
- Messages: *(no ping reply, SSH port 22 open / closed)*; skipped: *not reachable: no ping
  reply, SSH port 22 closed — SSH is tried again when the host answers ping or its SSH port
  opens*.
- Files: `core/src/collector.rs`, `core/src/config.rs`, `core/src/strategy.rs`,
  `app/src/{msg,lib,state,update}.rs`, tests (`tests_strategy.rs` +4), `gui/src/runtime.rs`,
  `gui/src/views/{detail,strategy}.rs`, `docs/USER_MANUAL.md`, `AGENTS.md`.
- Verified: fmt, clippy `-D warnings`, `cargo test --workspace` (core 223, app 185, gui 7).
  Not verified yet: GUI down count against `telepad-check` on the real inventory (tester).

## [2026-11-24] — Issues #143–#145 created (Hosts table width, host groups, clusters)
- #143 fix(ui): Hosts table fills the page width, auto-resizes columns, Flags not clipped and readable — labels: bug
- #144 feat(hosts): optional host groups — Groups page, Ungrouped, Hosts - [Group] page, group on cards — labels: enhancement
- #145 feat(hosts): user-defined clusters separate from groups — Clusters page, Hosts - [Cluster] page, cluster on cards — labels: enhancement
- Platform: GitHub; scope: telepad-core (store schema 5, config tags), telepad-app, telepad-gui

## [2026-11-24] — Telepad 0.9.8 released
- Tag `v0.9.8` (PR #142); release workflow green, 8 assets with `.sha256` published;
  `telepad-check --check-update` finds it.

## [2026-11-24] — Telepad 0.9.8
- **Database grid** (#138): a filter box under every column (contains, `=x`, `!x`, `>x`,
  `<x`, `NULL`, `!NULL`) together with a Custom WHERE; **Columns** chooser with search,
  kept per table; pager footer *Record [1 - 100] of 2 345* with first / previous / page /
  next / last and typed rows per page (100 by default).
- **Export / Import** (#139): CSV, SQL INSERTs or Excel (.xlsx) — filtered rows, whole
  table, this page or a query result; import CSV / Excel / SQL INSERTs with a column map
  and preview, in one transaction (the first error undoes everything).
- **Diff** (#135): git-style colours (`-` red deleted, `+` green added) and both sides
  scroll together.
- **Log info** (#140): the pager is a footer under the table with record info and
  first / previous / next / last.
- Fixes: Windows `My Documents` and other legacy junctions (#134) are hidden and open their
  real folder; an already open window comes back to the front from its button (#136);
  clicks anywhere in a table cell or header select / sort (#137).

## [2026-11-24] — Tested and merged: #134–#140 (PR #141)
- Full pass on Windows 11 (UI Automation walkthrough, live database tests); CI green on
  Windows, macOS, Linux, `db-live`, `files-live`, release builds. Issues closed by the PR;
  test summary commented on each.

## [2026-11-24] — Tested #134–#140 (stack ending at `139-db-export-import`)
- Windows 11, release build; fmt, clippy `-D warnings`, `cargo test --workspace` (core 219,
  app 181, gui 7); live database tests against throwaway MariaDB 11 / PostgreSQL 17
  containers on 127.0.0.1 (filter boxes, `COUNT(*)`, import incl. rollback) — removed after.
- GUI via UI Automation on a copy of the inventory: #134 legacy junctions hidden, `My
  Documents` opens `Documents`; #136 a minimized database window comes back when its
  button is clicked again; #137 sort from the empty part of headers (database grid, Hosts
  table), row select from empty cell space; #138 filter box `=Springfield` → 83 of 250,
  20 rows per page → 13 pages, last page `Record [241 - 250] of 250`, Columns chooser hides
  a column (kept); #139 CSV export of 250 rows / 4 shown columns in grid order, import of 3
  CSV rows with a column map (skip, NULL, `O'Brien`) → 253, re-import rolled back with
  "Row 1 of 3: Duplicate entry"; #135 Diff `-` red / `+` green with counts, synchronized
  scrolling to the matching line; #140 Log info footer `Record [941 - 955] of 955`, stays
  visible while rows scroll.
- Fixed during testing: the database window's status line said "via SSH" for this
  computer; live MySQL test cleaned its imported rows.
- Not verified: Excel / SQL export in the GUI (unit-tested round trip), PostgreSQL GUI
  import, Linux / macOS GUI (CI builds them).

## [2026-11-24] — #134–#140 implemented (stacked branches `134-…` → `136-…` → `137-…` → `140-…` → `138-…` → `135-…` → `139-…`)
- #134 fix(files): Windows legacy junctions — hidden / system taken from the link itself,
  a link opens at its target (`list_local_at`), plain "access denied" text. Files: core
  `files/mod.rs`, gui `runtime.rs`, app `tests_files.rs`, manual.
- #136 fix(ui): a message that leads to an open page (Edit, Open files, Open database, Log
  info, host page, Authorized keys, Host Management) restores and raises it (`update.rs`
  `raise_target` / `raise_win`). Files: app `update.rs`, `tests_workspace.rs`, manual.
- #137 fix(ui): `widgets::cell_text` / `cell_click`: whole cells and header cells take
  clicks (database grids, file panes, Log info, Hosts table); database grids centred.
- #140 feat(keys): `telepad-app::pager::Pager` + `widgets::pager_bar` (Record [a - b] of n,
  first / previous / Page [n] of N / next / last, typed rows per page, ask above 100); Log
  info's pager is a sticky footer.
- #138 feat(db): filter row under the Data grid headers (`sql::filter_condition`, quoted
  literals, wtbapp rules) with the Custom WHERE; Columns chooser (search, per table in
  `settings`); pager footer with `COUNT(*)` total; 100 rows by default.
- #135 feat(editor): git-style Diff (- red deleted left, + green added right, counts) and
  synchronized scrolling to the matching line (`compare::line_map`), Sync scroll toggle.
- #139 feat(db): Export (CSV / SQL / .xlsx; filtered rows, whole table or page; shown or all
  columns; query results too) and Import (CSV / Excel / SQL INSERTs; column map, preview,
  one transaction, error names the row; never on Read-only); new deps `csv` 1.4.0,
  `rust_xlsxwriter` 0.104.0, `calamine` 0.38.0.
- Verified: fmt, clippy `-D warnings`, `cargo test --workspace` (core 219, app 181, gui 7).
  Not verified yet: GUI (tester), live MySQL / PostgreSQL import (CI `db-live`).

## [2026-11-24] — Issues #134–#140 created on GitHub
- #134 fix(files): Windows legacy junctions (My Documents) — hide system folders, open the link target instead of Access denied (bug; core files, app files)
- #135 feat(editor): git-style Diff colours (+ green added, - red deleted) and synchronized scrolling (enhancement; core compare, gui text_editor)
- #136 fix(ui): opening an already open page restores and raises its window (minimized or behind) (bug; app workspace)
- #137 fix(ui): table cells and headers react to clicks anywhere in the cell — database grid select / centring, sort on header click (bug; gui database, files)
- #138 feat(db): data grid filter row, column chooser and pager footer (Senar / wtbapp style, 100 rows default) (enhancement; core db, app db, gui)
- #139 feat(db): export and import table data (SQL, CSV, Excel) (enhancement; core db, app db, gui)
- #140 feat(keys): Log info pager as a sticky footer with record info and first / previous / next / last (enhancement; gui logins, widgets)
- Reference for #138 / #140: wtbapp `code2/public/js/y.panel.js` (`pager_*`, `filter_build`), `y.app.js` (`y_record_info`, `y_page_size_ask`, `y_filter_match`), `y.select.js`; Senar.

## [2026-11-24] — Telepad 0.9.7
- **My Computer** (#129): a page above **Hosts** for the PC Telepad runs on, without SSH —
  System, Maintenance, Usage, Disks, Network, Sensors, Docker, Ports in use; **Open
  Terminal** (a local shell), **Open files…** with this computer on both sides, and its own
  MariaDB / MySQL / PostgreSQL opened directly on `127.0.0.1`. `telepad-check --local`.
- **File windows** (#130): a click anywhere on a row selects it (also the `..` row, the
  icon and the date), Shift+click selects a range, text is centred in the row.
- **Compare, Synchronize, Diff, Transfer settings** (#131): mark what differs between the
  two folders; make one folder match the other (or both ways) after a preview of every
  copy and delete; open a file of each pane side by side in the editor with the differing
  lines marked; per host Default / Text / Binary mode, new and updated files only, and
  excluded folders.

## [2026-11-24] — Tested and merged: #129, #130, #131 (PR #132)
- Full pass on Windows 11 (UI Automation walkthrough, headless `--local` and all hosts);
  CI green on Windows, macOS, Linux, `db-live`, `files-live`, release builds. Issues closed
  by the PR; test summary commented on each.

## [2026-11-24] — Tested #130, #129, #131 (branches `130-file-row-click` → `129-my-computer` → `131-files-compare-sync`)
- Windows 11, release build; fmt, clippy `-D warnings`, `cargo test --workspace` (core 213,
  app 171, gui 7). Headless: `telepad-check --local` (≈ 1.2–2 s; 52 containers = `docker ps -a`,
  disks / IPs / ports match), all 56 hosts read-only (49 ok, 7 unreachable or password-only as
  before — remote collection unchanged).
- GUI on a copy of the inventory via UI Automation + window captures: My Computer above Hosts;
  System / Maintenance / Usage / Disks / Network / Sensors (not available) / Docker sections;
  Open Terminal opened a local shell; Open files… = This computer on both sides; Open
  PostgreSQL :5432 connected straight to 127.0.0.1 (server's own auth error, no SSH); the
  layout (My Computer, local files, database, editor) came back after a restart.
  #130: clicks on the Modified cell and on empty cells of `..` select the row, text centred.
  #131: Compare marks (only here / newer / older / different / same), Hide identical,
  Transfer settings (exclude `node_modules` saved), Synchronize preview (3 copies, 2
  deletes) then run: disk matched the preview, History 2 entries; Diff side by side with
  marked lines, edit + Save of one side written.
- Fixed during testing: database window said "through the SSH login" for this computer;
  file hint said Upload / Download for two local folders; rows did not fill the pane without
  an Owner column; editor label case; `local::run` gated to Windows (Linux / macOS clippy).
- Noted: Default mode copies known text files with the target's line endings, so a mixed-
  ending `.conf` shows "different size" after a copy between two Windows folders.
- Not verified: Linux / macOS (CI builds them), MariaDB local connect (only PostgreSQL here).

## [2026-11-24] — feat(files): compare, synchronize, diff, transfer settings (#131)
- Issue #131 addressed on GitHub (branch `131-files-compare-sync`, on top of `129-my-computer`).
- **Compare** marks both panes (only here / newer / different size / same, 2 s slack),
  **Hide identical**. **Synchronize…**: left → right, right → left or both (newer wins),
  recursive, optional delete (off by default, never both ways); **Preview** lists every
  copy and delete, **Synchronize** runs one transfer each way + the deletes (history).
  **Diff** opens one file of each pane side by side in the editor with differing lines
  marked, each side saved on its own. **Transfer settings…** per host (`files_opts:<alias>`):
  Default / Text / Binary (CRLF on Windows, LF on hosts; NUL or > 64 MiB stay binary),
  new and updated files only (no question; rsync `--update`), excluded folders (globs;
  rsync `--exclude`).
- Files: core `files/compare.rs` (new), `files/transfer.rs`, `files/rsync.rs`,
  `files/live_tests.rs`, `store.rs`; app `files.rs`, `editor.rs`, `tests_files.rs`; gui
  `views/files.rs`, `views/text_editor.rs`, `runtime.rs`; `docs/USER_MANUAL.md`,
  `README.md`, `AGENTS.md`.
- Verified: fmt, clippy `-D warnings`, `cargo test --workspace` (core 213, app 171, gui 7).
  Not verified yet: GUI (tester).

## [2026-11-24] — feat(local): My Computer page (#129)
- Issue #129 addressed on GitHub (branch `129-my-computer`, on top of `130-file-row-click`).
- **My Computer** above **Hosts** in the sidebar (`Win::MyComputer`, layout key `local`): the
  host page of this PC without SSH — System, Maintenance (reboot pending), Usage, Disks,
  Network, Sensors ("not available" without admin on Windows), Docker, Ports in use. Read
  while the page is open (live interval, not while paused) and on Refresh.
- Windows: `sysinfo`, `netstat -ano`, `reg query`, `docker` / `podman` CLI; Linux / macOS:
  `script.sh` via local `sh -s`. Buttons: Open Terminal (local shell), Open files… (this
  computer on both sides), Open MariaDB / MySQL / PostgreSQL on 127.0.0.1 (owner decision:
  no SSH for this computer's databases). No Copy ssh / Edit / Clone / Delete.
- Transfers go through a `transfer::Remote` trait (`Sftp` or `ThisComputer`).
- `telepad-check --local [--json]`.
- Files: core `local.rs` (new), `config.rs`, `platform.rs`, `eol.rs`, `parse.rs`,
  `files/transfer.rs`, `db/client.rs`, `Cargo.toml` (`sysinfo` 0.40.4, Windows only); app
  `state.rs`, `msg.rs`, `update.rs`, `workspace.rs`, `files.rs`, `editor.rs`,
  `tests_local.rs` (new); gui `views/detail.rs`, `docker.rs`, `sensors.rs`, `files.rs`,
  `workspace.rs`, `mod.rs`, `runtime.rs`; check `main.rs`; `docs/USER_MANUAL.md`,
  `README.md`, `SECURITY.md`, `AGENTS.md`.
- Verified: fmt, clippy `-D warnings`, `cargo test --workspace` (core 208, app 167, gui 7);
  `telepad-check --local` on Windows 11 (≈ 2 s: OS, CPU, memory, disks, IPs, 50 containers,
  ports). Not verified yet: GUI page, Linux / macOS local read.

## [2026-11-24] — fix(files): whole file row selects on click (#130)
- Issue #130 addressed on GitHub (branch `130-file-row-click`).
- Every cell is plain non-selectable text; the row takes click, Ctrl / Shift+click,
  double-click, drag and right-click; the `..` row can be highlighted (never copied);
  cells centred vertically (`cell_layout`).
- Files: gui `views/files.rs`; app `files.rs` (`click_row`, `click_parent`,
  `clear_selection`), `tests_files.rs`; `docs/USER_MANUAL.md`.
- Verified: fmt, clippy, tests. Not verified yet: GUI clicks.

## [2026-11-24] — feat(files): compare directories, synchronize, diff between panes and transfer settings
- Issue #131 created on GitHub
- Scope: files (core `files/`, app `files.rs`, gui `views/files.rs`)
- Labels: enhancement

## [2026-11-24] — fix(files): whole file row (.. row, icon, Modified, Size cells) selects on click; centre text vertically
- Issue #130 created on GitHub
- Scope: gui `views/files.rs`
- Labels: bug

## [2026-11-24] — feat(local): My Computer page — monitor this PC like a host, terminal, files and local databases
- Issue #129 created on GitHub
- Scope: core (local collector, platform), app, gui
- Labels: enhancement

## [2026-11-24] — Telepad 0.9.6
- **Log info** (#126): a host's **Log info** button opens a page of who logged in over SSH,
  as which user, from where, how and with which key (the host's systemd journal, last 30
  days, read without sudo, only on open / Refresh). One table, newest first, with header
  sort, a filter per column, pages and typed rows per page; key names come from
  `authorized_keys`; this PC's (Telepad's) logins are hidden unless switched on.
- **Register Systemd Journal / Unregister**: a terminal that shows and runs
  `sudo usermod -aG systemd-journal <user>` / `sudo gpasswd -d <user> systemd-journal`
  after Enter (sudo asks for the password there). Works on Debian / Ubuntu and
  Fedora / RHEL / CentOS; hosts without systemd say so.
- Authorized keys: the key sshd actually accepted for Telepad is marked **last used by
  Telepad**; the lockout guard also covers login keys without a `.pub` file.
- `telepad-check --logins <alias>`.

## [2026-11-24] — Tested and merged: #126 (PR #127)
- Full pass on Windows 11 (UI Automation walkthrough, headless `--logins` on 49 hosts);
  CI green on Windows, macOS, Linux, `db-live`, `files-live`. Issue closed by the PR; test
  summary commented.

## [2026-11-24] — Tested #126 (branch `126-login-log`)
- Windows 11, release build; fmt, clippy `-D warnings`, `cargo test --workspace` (core 205,
  app 160, gui 7). Headless `telepad-check --logins` on all 49 reachable hosts: 33 readable
  (this read's key found on every one, counts match direct `journalctl` within new logins),
  14 denied (Register shown), RDP-only refused, no crash.
- GUI on a copy of the inventory via UI Automation: Log info button, page per host, restored
  with the workspace after restart, newest first, header sort, filters (`10.212`,
  `!<address>`, `>2026-11-24`), pages, typed rows per page (50 + Enter), this PC's logins
  toggle, key names, **last used by Telepad** on Authorized keys. Register on a denied host
  opened the terminal with the exact script; closed before Enter, host unchanged (`id -nG`).
- Fixed during testing: 30-day read took 80–145 s on a public host (≈ 870 000 sshd lines a
  month) and hit the 40 s timeout → own 3 min read timeout; Key name / Telepad columns were
  off-screen → fingerprint last + horizontal scroll; real fingerprints in test data replaced.
- Not verified: Fedora / RHEL host (none in the inventory), Unregister run to the end
  (would change a host), macOS / Linux GUI (CI builds them).

## [2026-11-24] — feat(keys): Log info page — SSH logins from the journal (#126)
- Issue #126 addressed on GitHub (branch `126-login-log`).
- **Log info** button in the host header (SSH hosts, after the database buttons) opens a
  page per host (restored with the workspace as `logins:<alias>`): `Accepted …` lines of
  sshd from the last 30 days (`journalctl -t sshd -t sshd-session`, no sudo, newest 5 000),
  flat table with header sort (asc → desc → newest first), per-column filters (wtbapp
  rules), pages with typed rows per page (20), key names from `authorized_keys` / this PC's
  keys, this PC's logins hidden by default. The key sshd accepted for the read (journal
  pid = this session's ancestor) is shown and marked **last used by Telepad** on Authorized keys.
- **Register Systemd Journal / Unregister**: terminal with `sudo usermod -aG systemd-journal "$U"`
  / `sudo gpasswd -d "$U" systemd-journal`, run after Enter (sudo asks). Access check is
  "journal readable" (also `adm` / `wheel` / root); no systemd → "not available".
- `login_keys` reads the public key from the private key when no `.pub` exists (lockout
  guard covers it). `telepad-check --logins <alias>`.
- Files: core `login_log.rs` (new), `authorized_keys.rs`, `collector.rs`, `lib.rs`; app
  `logins.rs`, `tests_logins.rs` (new), `state.rs`, `msg.rs`, `update.rs`, `workspace.rs`;
  gui `views/logins.rs` (new), `detail.rs`, `keys.rs`, `mod.rs`, `workspace.rs`, `runtime.rs`;
  check `main.rs`; `docs/USER_MANUAL.md`, `README.md`, `SECURITY.md`, `AGENTS.md`.
- Verified: fmt, clippy `-D warnings`, `cargo test --workspace` (core 205, app 160, gui 7);
  `telepad-check --logins` on `mattermost` (Debian 13, `systemd-journal`) and `dockerho`
  (Ubuntu, reads via `adm`): logins and this session's key found. Not verified yet: GUI
  (tester), Register / Unregister terminal on a host, Fedora / RHEL host (none at hand).

## [2026-11-24] — feat(keys): Log info page — SSH logins from the systemd journal per host, Register / Unregister journal access
- Issue #126 created on GitHub
- Scope: telepad-core (login_log, authorized_keys), telepad-app, telepad-gui, docs
- Labels: enhancement

## [2026-11-23] — fix(collector): hosts that drop ICMP ping stay down until the SSH retry after one failed SSH attempt
- Issue #125 created on GitHub
- Scope: collector (telepad-core `collector.rs`), telepad-app (`scope_at`, `collected` / `pinged`)
- Labels: bug
- Related to: #122, #36

## [2026-11-23] — Telepad 0.9.5
- **Update Strategy** (#122, ☰ ▸ Update Strategy…): Pause all, or pause single hosts (page,
  host page, right-click menu); per check on / off and interval — ping 30 s, live metrics
  1 min, network & updates 6 h, system info 1 day, SSH retry 10 min by default; replaces
  "Auto every 60 s" in the top bar. Saved in the new `update.db` next to `telepad.db`.
- **Ping check**: hosts are pinged on their own timer (no SSH); a host that answered and
  misses two pings is shown down at once and read again as soon as it answers.
- **Last known info**: OS / CPU info, addresses, ports and pending updates are kept in
  `update.db`, shown at once after a restart and not read again before they are due.
  Still read-only: Telepad never runs `apt-get update`.

## [2026-11-23] — Tested and merged: #122 (PR #123)
- Full pass on Windows 11 (UI Automation walkthrough, runtime checks against `update.db`,
  headless `dockerho`); right-click menu item confirmed by the owner. CI green on Windows,
  macOS, Linux, `db-live`, `files-live`. Issue closed by the PR; test summary commented.

## [2026-11-23] — Tested #122 (branch `122-update-strategy`)
- Windows 11, release build, copy of the real inventory (56 hosts), headless `dockerho`
  (OS, kernel, cores, disk, 25 containers, 41 ports match direct read-only commands).
- GUI driven by Windows UI Automation (egui AccessKit) + window-only capture (PrintWindow):
  no Pause in the top bar; status bar *1 host paused* opens Update Strategy; Pause all →
  *Paused … — nothing runs by itself*, no refresh for 75 s; Resume → read at once; paused
  host row *paused · last known: …*; row Resume reads it; filter + Pause / Resume N listed;
  Ping 5 s refused (*Ping: every 10 s to 1 h*), 20 s saved to `update.db`; Live metrics off
  stops live rounds while ping rounds go on; Reset to defaults saved; restart keeps Pause
  all and reopens the page; host page Pause / Resume updates writes `paused_hosts`;
  900 px wide stays usable.
- Runtime: paused host not collected while others are; Pause all survives restart;
  saved facts not re-read after restart.
- Fixed during testing: paused host showed *queued*; plural pause message.
- Not automated: the right-click menu item (posted clicks do not open egui context menus).
- Seen, not from #122: a host without ping reply that fails SSH once at start waits for the
  SSH retry (10 min) before it is tried again (#36 behaviour).

## [2026-11-23] — feat(collector): Update Strategy page, pause, ping check, last facts in update.db (#122)
- Issue #122 addressed on GitHub (branch `122-update-strategy`)
- Core: new `strategy.rs` (`update.db`: `strategy` + `last_facts`, `Strategy` / `Every` / `Check`
  limits, `StaticFacts` / `SlowFacts` shared with `collector::Scope::carry_over`); collector
  constants `STATIC_EVERY` / `SLOW_EVERY` / `DOWN_SSH_RETRY` replaced by the strategy;
  `NetAddr` / `Listener` / `Maintenance` deserialize.
- App: `Msg::SetPaused` / `StrategyOpen` / `StrategySave` / `StrategyReset` / `Pinged`,
  `Effect::SaveStrategy` / `SaveFacts` / `PruneFacts` / `Ping`; `Win::Strategy`; `HostState`
  `Seen` times (saved facts older than this boot), `known` facts, ping result / misses;
  `tests_strategy.rs` (8 tests).
- GUI: "Auto every 60 s" removed from the top bar; ☰ ▸ Update Strategy… page manages
  refresh: Pause all, intervals, host list with per-host Pause / Resume and Refresh
  (owner request 2026-11-23: pause by host, refresh settings on a page); host page and
  right-click menu Pause updates; status bar "Paused" / "N hosts paused" opens the page;
  `update.db` writer thread (`paused_hosts` table), ping rounds, `--update-db`;
  last known OS in the Hosts table and host page.
- Docs: USER_MANUAL (Refreshing, Update Strategy, files, command line), README, AGENTS.md,
  `.gitignore`.
- Verified: fmt, clippy `-D warnings`, `cargo test --workspace` (core 197, app 149, gui 7
  incl. glyph test); release GUI on a copy of the real inventory (56 hosts): `update.db`
  created, static + slow facts saved for 48 reachable hosts; after a restart no static / slow
  tier was read again (no newer `collected_at`). Not verified: the Pause button and the
  Update Strategy page by clicking (no desktop automation), ping marking a host down live.

## [2026-11-23] — feat(collector): Update Strategy page — pause, per-tier intervals, ping check, last facts kept in update.db
- Issue #122 created on GitHub
- Scope: collector (telepad-core `collector.rs`, new `update.db`), telepad-app, telepad-gui (top bar, ☰ menu, new page)
- Labels: enhancement
- Builds on #36 (tiered collection): intervals become settings, tier facts persist across restarts, Pause replaces "Auto every 60 s"

## [2026-11-22] — Telepad 0.9.4
- **OpenPGP** in My keys (#107): Ed25519 / RSA keys export as OpenPGP (GPG) with a User
  ID and optional passphrase; OpenPGP secret keys import to OpenSSH (checked with GnuPG).
- **Remote Desktop password in Windows Credential Manager** (#108): saved as
  `TERMSRV/<address>` like Remote Desktop's *Remember me*; Forget; never stored by Telepad.
- **Remote Desktop: Scale to the window** (#120): `smart sizing` keeps the desktop size
  (e.g. 1920 × 1080) instead of being read as *Fit the window*.
- Hardening from the 0.9.3 review: database files readable by the owner only on macOS /
  Linux (#110); history CSV export defuses formula cells (#111); generated / converted keys
  narrowed before the move, by SID, a permission problem is a warning (#112).

## [2026-11-22] — Fixed and merged: #107, #108, #110, #111, #112 (PRs #114–#118)
- #111 history CSV export defuses formula cells; #110 Telepad's databases mode 600 on
  macOS / Linux (note when not possible); #112 keys narrowed before the move, by SID,
  failure as a warning; #108 RDP password in Windows Credential Manager (TERMSRV, Win32
  API via `windows-sys`, never read back); #107 OpenPGP export / import (rPGP; checked
  with GnuPG 2.4.7: import, same SSH key, signing).
- CI green on Windows, macOS, Linux, `db-live`, `files-live`, package builds.
- GUI on Windows 11 (lab, fake home): OpenPGP convert with User ID; Credential Manager save
  → `cmdkey` shows the Domain Password → Forget removes it.
- Not verified: an mstsc login with the saved credential (no Windows RDP host in the lab).
- Not released yet (main is ahead of v0.9.3).

## [2026-11-22] — Post-release review of v0.9.3 (#99–#102)
- Reviewed merges 11c452d, cec833e, fcff314, 35557a9 and release 377fca8 on `main`.
- No telemetry: the only HTTP client is the update check (GitHub, `telepad/<version>`
  user agent); other connections are TCP probes to the user's own hosts and the local
  SSH tunnel. No `unsafe`, no new shell strings (arguments passed separately), no secrets
  or key material in the tree (only PEM headers in tests), monitoring stays read-only.
- Issues filed: #110 (database files `0644` on macOS / Linux), #111 (CSV formula
  injection in history export), #112 (generated key ACL step after the move).

## [2026-11-22] — Telepad 0.9.3
- **Workspaces** (#99): file windows (folders), editor windows (paths, never text) and
  database windows (connection, database / schema) come back with their workspace and on
  start; databases reconnect with the saved password. Leaving a workspace or closing
  Telepad shows one modal listing what would be lost (unsaved editor text, grid edits,
  structure changes, query text, Host Management / Authorized keys work, open dialogs):
  **Yes** / **Back**. Running uploads and downloads keep going in the background.
- **History** (#100): `history.db` (90 days): the file window's History tab (transfers,
  file operations, editor saves) and the database window's SQL log (HeidiSQL style,
  passwords masked); export CSV, clear.
- **My keys** (#101): the key pairs on this computer, Generate (ssh-keygen, Ed25519),
  Convert / export / import between OpenSSH, PuTTY PPK v2 / v3, PEM, PKCS#8, DER, PKCS#12,
  JWK and public-only forms (PPK checked against puttygen 0.83).
- **Remote Desktop settings** (#102) per host in `telepad.db`, Import / Export `.rdp`,
  Open RDP with a temporary `.rdp` (mstsc / Windows App) or `xfreerdp` arguments.
- Fixes: RDP-only hosts imported without a Port line get 3389.
- README and manual: how Telepad works (diagrams), annotated screenshots.

## [2026-11-22] — Tested and merged: #99, #100, #101, #102 (PRs #103–#106)
- Stacked branches; CI green on Windows, macOS, Linux, `db-live`, `files-live` (and the
  package builds); issues closed with test notes.
- GUI tested on Windows 11 with lab containers (made-up hosts, documentation addresses,
  MariaDB next to one host) and a fake home folder (no real keys or paths in
  screenshots): leave modal (Back / Yes / Escape / quit), 2.3 GB upload kept in the
  background, windows back after switch and restart, SQL log and history read back,
  generate + convert to PPK v3, RDP settings + Open RDP (.rdp checked, mstsc closed).
- Fixed during the test: doubled transfer titles, "just now" → "this session", leave
  question made modal, My keys detail width, mstsc drive list, RDP-only import port.
- Follow-ups filed: #107 (OpenPGP), #108 (Windows Credential Manager for RDP).

## [2026-11-22] — feat(rdp): saved Remote Desktop settings per host, open with a generated .rdp, import / export .rdp
- Issue #102 created on GitHub
- Scope: telepad-core (store, rdp, platform), telepad-app (host editor), telepad-gui
- Labels: enhancement (windows, macos, linux)

## [2026-11-22] — feat(keys): My keys tab — list local keys, generate key pairs, convert between OpenSSH, PuTTY and other formats
- Issue #101 created on GitHub
- Scope: telepad-core (local_keys, platform), telepad-app (keys), telepad-gui (Authorized keys page)
- Labels: enhancement (windows, macos, linux)

## [2026-11-22] — feat(history): log file transfers and database commits (HeidiSQL-style SQL log)
- Issue #100 created on GitHub
- Scope: telepad-core (history store), telepad-app (files, db), telepad-gui (log panels)
- Labels: enhancement (windows, macos, linux)

## [2026-11-22] — feat(ui): workspaces reopen file, database and editor windows; reset unsaved settings on leave, transfers keep running
- Updated: leave question (Save all / Discard / Cancel) for unsaved, uncommitted or running work
- Updated: unapplied Host Management and Authorized keys changes discarded, both pages reset on leave
- Updated: leaving always discards and resets unsaved / uncommitted / unapplied work (settings too, so the same config is not edited twice); one modal lists it, Yes switches, Back stays in the previous workspace (Save all dropped)
- Updated: file transfers are not cancelled on leave; they keep running in the background and show in the same host's File window in the new workspace; only settings and opened files are reset
- Issue #99 created on GitHub
- Scope: telepad-app (workspace), telepad-core (prefs), telepad-gui (runtime)
- Labels: enhancement (windows, macos, linux)

## [2026-11-21] — Telepad 0.9.2
- **Window picker** (#92): the top bar shows the window in front and how many are open;
  click it to search every open window (minimized ones too) and bring one to the front.
  The sidebar's *Open windows* list is gone. **Theme** moved to **☰ → Settings**.
- **Host Management** (#93): **Add host…**, **Import** (file, `~/.ssh/config`, pasted text)
  and **Export…** are on its Telepad side; the ☰ menu keeps only Settings and Software
  update (Host Management and Authorized keys open from the sidebar).
- **Workspaces** (#94, WinBox style): `<none>` / `<own>` / your own, searchable dropdown
  with a delete icon per workspace and Save / Add… at its foot; switching closes the
  windows and opens the picked set (host windows by alias); saved automatically in the new
  `preference.db` next to `telepad.db` (`--prefs` / `TELEPAD_PREFS`); the old layout
  becomes `<own>` on the first start. **Arrange**: Tile, Stack, Minimize all, Restore all;
  title bars use corners-out / corners-in for maximize / restore.
- README: top bar section and new screenshots.

## [2026-11-21] — Tested and merged: #92, #93, #94 (PRs #95–#97)
- Branches stacked (#92 → #93 → #94); each merged after CI green on Windows, macOS, Linux,
  `db-live`, `files-live`; issues closed with the test notes.
- GUI tested on Windows 11 in a throwaway workspace (made-up hosts, documentation
  addresses), the window driven by a scratch PowerShell helper and captured window-only
  (desktop automation tools were not available): picker search / Enter / Escape, ☰ Settings,
  Add host / Import on Host Management, add / switch / `<none>` / delete workspaces, Tile,
  Stack, Minimize / Restore all, maximize icons, restart keeps the workspace.
- Found and fixed during the test: dropdowns opened with the first row highlighted, so Enter
  picked `<none>`; they now open on the current entry. Owner follow-up: Host Management left
  the ☰ menu too.
- Not covered by the GUI pass: the unsaved-editor stop on switch and an unreadable
  `preference.db` (unit tests); Export (native save dialog).
- README screenshots retaken from local lab containers (1000 × 480; host window left out).

## [2026-11-21] — feat(ui): named workspaces (WinBox style) saved in preference.db, window arrange (tile, stack, minimize all)
- Issue #94 created on GitHub
- Scope: core (new preference.db) / app (workspaces, arrange) / gui (top bar dropdown, Arrange menu, title-bar icons)
- Labels: enhancement

## [2026-11-21] — feat(ui): add / import / export hosts on the Host Management page, slimmer ☰ menu
- Issue #93 created on GitHub
- Scope: gui (Host Management page toolbar, ☰ menu)
- Labels: enhancement

## [2026-11-21] — feat(ui): searchable open-windows picker in the top bar, Theme moves to ☰ Settings
- Issue #92 created on GitHub
- Scope: gui (top bar, workspace sidebar, ☰ menu) / app (workspace filter)
- Labels: enhancement

## [2026-11-20] — Telepad 0.9.1
- **Hosts page** (#83): the table fills the window; double-click a column border to fit it;
  rows highlight on hover; click a row to open the host, **right-click** for its menu (Open
  SSH, Files, databases, Edit, Clone, Delete); search moved into the page; "Overview — all
  hosts" is now **Hosts**; the folded sidebar centres its icons.
- **Docker / Podman**, **Kubernetes** and **Docker Swarm** pages (#84).
- **Authorized keys** (#85): groups from the `#` comment lines, add a key into a group,
  **Edit file** opens `authorized_keys` in the editor.
- **Light theme** (#86): grey background, white bordered windows, visible buttons and inputs.
- README: screenshots and a "Using Telepad" section.

## [2026-11-20] — Tested and merged: #86, #83, #84, #85 (PRs #87–#90)
- All four merged after CI green on Windows, macOS, Linux; issues closed. Owner follow-ups
  during the work: SSH / Files buttons became icons, then a right-click host menu (with
  databases); the host name in the table is plain text so the menu works on it too.
- README screenshots taken from a throwaway workspace (made-up host names, documentation
  addresses, local lab containers), cropped without the status bar; the host window shot was
  left out because it shows this PC's hardware.

## [2026-11-20] — feat(keys): key groups, add into a group, edit the file (#85)
- Issue #85 addressed on GitHub (branch `85-key-groups`, on top of #84).
- Core: `AuthKey::group` from the `#` line above (blank lines ignored, last comment line of a
  block names it, none = No group); add script inserts via awk (`INSERT_AWK`, key and group
  passed in the environment from quoted here-documents): end of the group's block, or a new
  `# group` block at the end, or — no group — before the first comment line; `check_group`;
  `apply` / `apply_all` / `script` take the group. Tests: parsing groups, adding into a
  group / a new group / no group with the local `sh`.
- App: `KeysDraft::group`, `KeyHolders::groups`, `key_groups`; `Effect::ApplyKey { group }`;
  `FmMsg::OpenFile` + `FileSession::pending_edit` + `EdMsg::OpenPath` (opens
  `~/.ssh/authorized_keys` once the file window is connected); the list is read again after
  saving that file. Tests.
- GUI: group headings (By server), groups per key (By public key), Group field with existing
  groups (Add / revoke), yellow **Edit file** button; editor warning bar for authorized_keys.
- Docs: USER_MANUAL (groups, Edit file, Group field), AGENTS.
- Verified on Windows 11 (lab containers): Edit file connects and opens the file with the
  warning (nothing saved); fmt, clippy, tests (95 app; core group tests run with Git Bash `sh`).

## [2026-11-20] — feat(docker): Docker / Podman name, Kubernetes and Docker Swarm pages (#84)
- Issue #84 addressed on GitHub (branch `84-clusters`, on top of #83).
- App: `Win::Kube` / `Win::Swarm` (layout keys `kube`, `swarm`), `table::cluster_rows` +
  `ClusterKind` / `ClusterRow` (one row per cluster id, several managers → one cluster, Swarm
  workers skipped); test.
- GUI: `cluster::cluster_page` (hosts it was reached from, problems, the existing Swarm /
  Kubernetes panels with their confirmed actions; empty note when none — owner: buttons are
  always shown); sidebar **Docker / Podman**, **Kubernetes**, **Docker Swarm**; Docker page
  marks Podman hosts.
- Also (#83 follow-up, owner idea): the SSH / Files icon buttons became a **right-click host
  menu** on table rows and cards: Open SSH / RDP (amber), Files (yellow), detected databases
  (host and containers), Open host window, Edit, Clone, Delete…. Table labels are no longer
  selectable, otherwise each label took the row's clicks and hover (found with egui's
  interaction snapshot).
- Docs: USER_MANUAL (Kubernetes and Docker Swarm, Docker / Podman, host menu).
- Verified on Windows 11 (throwaway workspace, lab containers): right-click menu → Files opens
  the file window, row highlight, Kubernetes page empty note, Docker / Podman page; fmt,
  clippy, tests (93 app). Not verified here: a real Kubernetes / Swarm cluster (none in the
  lab; grouping covered by the test).

## [2026-11-20] — feat(ui): Hosts page polish (#83)
- Issue #83 addressed on GitHub (branch `83-hosts-page`).
- Hosts table: the last column (icon buttons) is a remainder column, so the table fills the
  window; double-click a header border fits that column to its longest value (Excel style).
  egui_extras' own double-click auto-size never fires when the table has an id (it reads
  the double-click from another id), and clipped cells report their column width, so Telepad
  detects the double-click on the border itself (column left edges, the header responses are
  wider than their columns), clipped cells report their text's natural width, and the table is
  rebuilt with the fitted width. Rows highlight on hover and open the host on click; sortable
  headers show a hand cursor.
- Row buttons (table, cards), icon only per owner request: amber SSH / RDP (`Act::Caution`,
  own shades per theme, contrast test) and yellow Files (opens the host's file window); the host
  window's Open SSH / Open files… use the same colours.
- "Overview — all hosts" is now **Hosts** (sidebar, window title; layout key unchanged); the
  search box moved from the top bar into the Hosts and Docker page headers; Files and Databases
  left the sidebar; the folded sidebar centres its icons.
- Docs: USER_MANUAL (layout wireframe, Hosts table, button colours).
- Verified on Windows 11 (throwaway workspace, lab containers, light theme): table fills the
  window, double-click fit of the OS column, row click, icon buttons, folded sidebar; fmt,
  clippy, tests.

## [2026-11-20] — fix(ui): light theme too light (#86)
- Issue #86 addressed on GitHub (branch `86-light-theme`).
- Cause: egui's light visuals put panels and windows on the same gray 248 and buttons on
  gray 230 without a border, so windows, buttons and the background did not stand apart.
- Fix: `widgets::light_visuals()` (set with `set_visuals_of(Theme::Light, …)` at start):
  panel / workspace #e6e9ef, windows white with a #aeb6c2 border and a stronger shadow,
  buttons #e2e6ec with a #a9b2bf border (hover / pressed darker), inputs white with a border,
  faint rows #f1f3f6. Contrast tests run on the new panel and window colours, plus a test that
  the layers stand apart. Dark theme unchanged.
- Verified on Windows 11 (throwaway workspace, light theme): sidebar, Overview and Authorized
  keys windows; fmt, clippy, tests.

## [2026-11-20] — fix(ui): light theme too light — clearer backgrounds, window borders and buttons
- Issue #86 created on GitHub (user findings on 0.9.0)
- Scope: gui (theme visuals, widgets palette, workspace frames)
- Labels: bug

## [2026-11-20] — feat(keys): key groups from authorized_keys comments, add into a group, edit the file in the editor
- Issue #85 created on GitHub (user findings on 0.9.0)
- Scope: core (authorized_keys parser, add script) / app / gui (keys)
- Labels: enhancement

## [2026-11-20] — feat(docker): Docker / Podman page name and Kubernetes / Docker Swarm pages
- Issue #84 created on GitHub (user findings on 0.9.0)
- Scope: gui (docker_overview, new cluster pages) / app (workspace, table)
- Labels: enhancement

## [2026-11-20] — feat(ui): Hosts page polish — full-width table, auto-fit columns, search in page, hover, SSH / Files buttons, sidebar cleanup
- Issue #83 created on GitHub (user findings on 0.9.0)
- Scope: gui (overview, cards, workspace sidebar, panels, widgets) / app
- Labels: enhancement

## [2026-11-20] — Telepad 0.9.0
- New: **one window, WinBox style** (#77) — a page sidebar on the left (Overview — all
  hosts, Overview — Docker, Authorized keys, Host Management, Files, Databases; folds to
  icons) and pages as windows inside Telepad that move, resize, minimize, maximize and
  group as tabs. The host list is gone (hosts are in the overview); files, databases and
  the editor no longer open separate windows; the layout is remembered.
- New: **Authorized keys** page (#78) — the keys in each host's `~/.ssh/authorized_keys`,
  by server or by public key, with Revoke and Add shortcuts (still confirmed by typing
  `revoke` / `add`; Telepad's own login key is never revoked).
- Changed: **button colours** (#76) — red for delete / revoke / drop, yellow for edit,
  green for add, with separate shades for the dark and light themes.

## [2026-11-20] — Tested and merged: Authorized keys page (#78)
- PR #81 merged (CI green on Windows, macOS, Linux, `db-live`, `files-live`); issue closed.
- Tested on Windows 11 against the lab containers (see the #78 entry).

## [2026-11-20] — feat(keys): Authorized keys page (#78)
- Issue #78 addressed on GitHub (branch `78-authorized-keys`).
- Core: `authorized_keys::list_script` (read-only `cat` of `~/.ssh/authorized_keys` between
  markers), `parse_list`, `parse_authorized_keys` (options with quoted spaces, comments,
  blank / `#` lines, older types listed as not managed, other lines counted), `list` (adds
  the fingerprints Telepad logs in with), `list_all`; tests incl. "the list script only reads".
- App: `Model::auth_keys` (`AkHost` Loading / Ready / Failed per alias), `key_holders`
  (grouped by fingerprint), `KeysDraft` tab / server / open key / list filter; `AkLoad`,
  `AkListed`, `AkTab`, `AkPrepare` (shortcuts fill key + hosts and ask for the word);
  `Effect::ListKeys`; lists read on open, after hosts load when the page was reopened from
  the saved layout, and for the changed hosts when a run finishes; 4 tests.
- GUI: the page (By server / By public key / Add / revoke, Refresh), red Revoke (disabled
  for Telepad's login key and unmanaged types), green Add key / Add to server…, Revoke on all
  N servers; renamed Public keys → Authorized keys in the sidebar, window title and ☰ menu.
- Docs: USER_MANUAL (Authorized keys section), README, SECURITY, AGENTS.
- Verified on Windows 11 against the lab containers (throwaway workspace `fmws`): lists
  read, lab login key marked and not revocable, Add key from By server with the made-up
  alice@example.com key on both containers (added), By public key shows it on 2 servers,
  Revoke on all 2 servers with `revoke` (removed, lists back to one key each); your
  clipboard saved and restored around the paste; fmt, clippy, tests.

## [2026-11-20] — Tested and merged: #76 (PR #79) and #77 (PR #80)
- Both merged after CI green on Windows, macOS, Linux; issues closed. See their entries.

## [2026-11-20] — feat(ui): WinBox-style single window (#77)
- Issue #77 addressed on GitHub (branch `77-workspace`).
- App: `workspace.rs` — `Win` (Overview, Docker, Host, HostSync, Keys, Files(id), Db(id),
  Editor(id)), `Group` (inner window: tabs, active, rect, minimized, maximized, raise),
  `Workspace` (`open`, `close`, `sync` = session windows follow their session, `update(WsMsg)`:
  Open / Close / Focus / Minimize / ToggleMaximize / ActivateTab / Merge / Detach / Geometry /
  SetSidebarCollapsed, `save` / `restore`); `Msg::Ws`; `Model::ws`; `Select` opens the host
  window, `ShowOverview` opens Overview / Docker; the layout is saved with the view settings
  when it changes (`UiPrefs` + `sidebar_collapsed`, `workspace`; settings `ui_sidebar_collapsed`,
  `ui_workspace`). Overview, Docker, Host Management and Public keys reopen on start; host,
  files, database and editor windows do not (sessions). First start: Overview maximized.
- GUI: `views/workspace.rs` — page sidebar (Overview — all hosts, Overview — Docker, Public
  keys, Host Management, Files → host menu, Databases → open connections; open-window list;
  fold to icons), inner windows as framed areas clipped to the workspace (title bar drag,
  right / bottom / corner resize, minimize strip, maximize / double-click, close, tab menu
  "Show as a tab in" / "Own window", middle-click closes a tab), keys only in the front window.
  The host list is gone. Files, databases and the editor are no longer OS windows
  (`show_viewport_immediate` removed); Host Management and Public keys are pages; their
  dialogs stay over the workspace. Closing Telepad with unsaved editor tabs still asks.
  Coloured action buttons no longer use a child Ui (a wrapping row now wraps them); file
  panes clip to their half; the transfer queue keeps ≤ 40 % of a small window.
- Icons: `X`, `SQUARE`, `SIDEBAR`, `APP_WINDOW` (glyph test list).
- Docs: USER_MANUAL (Window layout with wireframe, windows inside Telepad, Hosts section
  replaces Host list), README, AGENTS.
- Verified on Windows 11 (throwaway workspaces `hm/ws` and `fmws`, lab containers
  `telepad-fm-sftp`): sidebar pages, host window from the overview, drag, minimize / restore,
  maximize, Docker merged as a tab into Overview, Host Management page (toolbar wraps),
  layout restored after restart, Files window with both panes, editor window from a
  double-click and its unsaved question (Don't save; file unchanged), folded sidebar,
  Public keys page; fmt, clippy, tests (8 new workspace tests). Not verified here: a
  database window (no database lab running; same code path as the others), macOS / Linux.

## [2026-11-20] — feat(ui): colour action buttons by risk — red delete / revoke, yellow edit, green add (#76)
- Issue #76 addressed on GitHub (branch `76-button-colours`).
- `widgets.rs`: `Act` (Danger / Edit / Add), `act_button` / `act_small` (filled, hover and
  pressed shades, faded when disabled, text kept on one line), `act_text` for menus; per-theme
  `Palette::act` (dark: deep maroon / amber / green with light text; light: clear red,
  yellow with dark text, green). Test: text contrast ≥ 4.5:1 on rest, hover and pressed
  fills in both themes, and different fills per theme.
- Applied in Host Management (Add to Telepad / config, delete in Telepad / config, row edit
  and delete), Public keys (Add key green + plus, Revoke key red + trash), host detail Edit /
  Delete, host editor Delete, delete confirmation, host list menu, file window toolbar / menu
  (new = green, edit / rename = yellow, delete = red), delete / overwrite buttons, editor
  Overwrite / Don't save, database (New table, Add column / index / foreign key, Insert row =
  green; Edit structure, Rename, cell Edit = yellow; Drop, Delete row, Forget, remove buttons,
  Run of a dangerous statement = red), Docker Remove, Swarm Remove stack.
- USER_MANUAL: button colours under Window layout.
- Verified on Windows 11 (throwaway workspace `hm/ws`): host detail, Host Management and
  Public keys in dark and light theme; fmt, clippy, tests. Note: Host Management's window
  grows to fit its toolbar now that button text no longer wraps; #77 makes it a resizable
  inner window.

## [2026-11-20] — feat(keys): Authorized keys page — list keys by server and by public key, add / revoke
- Issue #78 created on GitHub (depends on #77)
- Scope: core (`authorized_keys.rs` read-only list + parser) / app / gui (`views/keys.rs` as a page)
- Labels: enhancement

## [2026-11-20] — feat(ui): WinBox-style single window — page sidebar, inner windows (move, resize, minimize, maximize, tabs), in-window dialogs
- Issue #77 created on GitHub; refined the same day to the MikroTik WinBox style (inner windows: move, resize, minimize, maximize, tabs; ☰ top right for settings)
- Scope: app (`Page` navigation) / gui (sidebar, pages; Host Management, Files, Databases, editor no longer native windows)
- Labels: enhancement

## [2026-11-20] — feat(ui): colour action buttons by risk — red delete / revoke, yellow edit, green add
- Issue #76 created on GitHub
- Scope: gui (`widgets.rs` shared button helper; host_sync, keys, files, database, detail, … views)
- Labels: enhancement

## [2026-11-20] — Telepad 0.8.5
- Fixed: copy and paste (Ctrl+C / Ctrl+X / Ctrl+V, Cmd on macOS) now work with the system
  clipboard in every text field — for example pasting a public key into Public keys…
  (#71).

## [2026-11-20] — Tested and merged: copy and paste in text fields (#71)
- PR #74 merged (CI green on Windows, macOS, Linux builds and checks, `db-live`,
  `files-live`); issue closed.
- Tested on Windows 11 with made-up clipboard text (the user's clipboard restored): paste
  into Public keys, copy back out, same in the file window.

## [2026-11-20] — fix(ui): copy and paste do not reach the system clipboard in text fields (#71)
- Issue #71 addressed on GitHub (branch `71-clipboard`).
- Cause: the workspace built `eframe` with `default-features = false` and without its
  `clipboard` feature, so egui text fields never read or wrote the OS clipboard (the reason
  #42 has its own "Paste from clipboard" button).
- Fix: `clipboard` added to eframe's features in `Cargo.toml` (adds `arboard` /
  `smithay-clipboard` to egui-winit; version unchanged). USER_MANUAL: copy and paste note,
  Public keys step 1.
- Verified on Windows 11 (throwaway workspace, made-up text on the clipboard, the user's
  clipboard restored, never printed): Ctrl+V into the Public keys box and Ctrl+A / Ctrl+C back
  out (same text), the same in the file window's path bar (extra window); fmt, clippy, tests.
  Not verified here: Linux (X11 / Wayland) and macOS clipboard at run time (CI builds them).

## [2026-11-20] — Telepad 0.8.4
- New: **file operations** in the file window (#63) — new folder (F7) / new file, rename
  (F2), delete (Del, one confirmation listing every item with its size), properties,
  permissions (checkboxes or octal, optionally everything inside), copy path; on both
  sides, from each pane's toolbar or the right-click menu; per-item errors.
- New: **editor** (#64) — double-click, Edit or F4 opens a file of either side in a
  VS Code-like editor window: tabs, line numbers, syntax colours, find / replace (regex),
  go to line, indent, status bar; saves keep line endings, encoding and permissions, and
  ask before overwriting a file that changed on the host.

## [2026-11-20] — fix(ui): copy and paste do not reach the system clipboard in text fields
- Issue #71 created on GitHub (found while testing #64)
- Scope: workspace `Cargo.toml` (eframe built without its `clipboard` feature) / gui
- Labels: bug

## [2026-11-20] — Tested and merged: built-in editor (#64)
- PR #70 merged (CI green on Windows, macOS, Linux, `db-live`, `files-live`); issue closed.
- Tested on Windows 11 against the lab containers, a throwaway database and a scratch
  folder (see the #64 entry below for the walk-through).
- Fixed while testing: the text field drops its selection without focus, so find / replace
  keep their own current match; `$2_` in a regex replacement is group 2 then `_`; Enter in
  Go to line no longer also adds a newline; diff box height; long paths no longer push
  Save / Find out of the window; current line visible in the light theme.

## [2026-11-20] — feat(files): built-in VS Code-like editor for local and remote files (#64)
- Issue #64 addressed on GitHub (branch `64-editor`).
- Core: `files/text.rs` (decode / encode keeping CRLF, BOM and the final newline; binary,
  non-UTF-8 and > 10 MiB refused with the reason; find / replace with match case, whole word,
  regex, `$1` groups even before `_`; go to line; indent / outdent), `files/syntax.rs`
  (language by file name, folder and `#!`; small line lexer for shell, YAML, JSON, TOML,
  INI / conf, .env, Dockerfile, nginx, Python, JavaScript, TypeScript, PHP, SQL, Markdown,
  Rust), `files/edit.rs` (read / save locally and over SFTP: `.telepad-part` + rename, mode
  and owner kept, links followed, `Stamp` size / mtime check → `SaveError::Changed`).
  New dependency `regex` 1.13.2.
- App `editor.rs`: tabs per file window, dirty state from the draft text, one save at a time,
  conflict (Overwrite / Reload / Show diff via `hostsync::line_diff`), unsaved-changes
  question for a tab, the editor window, the file window and Telepad; `OpKind::Edit` (F4),
  `FmMsg::Editor`, `FmEffect::Load` / `Save` (run on the window's worker / SFTP connection).
  7 `update()` tests.
- GUI `views/text_editor.rs`: editor window with tab bar (unsaved ●), path, find / replace /
  go-to-line bar, conflict box with diff, status bar (Ln / Col, indentation, language menu,
  LF / CRLF, UTF-8 / BOM), line-number gutter fixed when scrolling sideways, current line,
  match highlights, VS Code Dark+ / Light+ colours; double-click / Edit / F4 in the panes.
- Docs: USER_MANUAL "Editor", README, SECURITY, AGENTS.
- Verified: fmt, clippy, all tests (text / syntax / edit unit tests, live SFTP test: CRLF
  round trip, mode 640 kept, no part file left, changed-on-host refusal, save through a
  link, binary refused); GUI on Windows 11 (second monitor, throwaway workspace, lab
  container and a scratch folder): open by double-click and F4, colours (YAML, nginx,
  shell), unsaved dot, save, conflict box + diff + Reload + Overwrite, find / replace (regex,
  `$2_$1`, replace one / all), Esc, undo, go to line 2500, Tab indent, binary file refused,
  unsaved question on tab close / file window close / Telepad close (Save, Don't save,
  Cancel), local CRLF file kept CRLF, light theme. Not verified here: the Unix-only local
  test (mode and link), which runs on Linux / macOS CI.

## [2026-11-20] — Tested and merged: file operations (#63)
- PR #68 merged (CI green on Windows, macOS, Linux, `db-live`, `files-live`); issue closed.
- Tested on Windows 11 against local lab containers (SFTP and no-SFTP), throwaway database
  and scratch folders: new / rename / delete / properties / permissions / copy path on both
  sides, per-item errors, keys ignored while typing in a field.
- Fixed while testing: a Unix test looked inside a folder after removing its x bit.

## [2026-11-20] — feat(files): file operations — new, rename, delete, permissions (#63)
- Issue #63 addressed on GitHub (branch `63-file-operations`).
- Core `files/ops.rs`: `Fs` trait (`LocalFs`, `Sftp`), `check_name` (one path element;
  Windows-invalid characters locally), new folder / file (`Sftp::create_new`, exclusive),
  rename (never replaces; case-only rename allowed on Windows / macOS), recursive delete and
  measure (links removed, never followed; per-item errors; stops at 100 000 entries),
  recursive chmod (contents before their folder; folders `dir_mode` = +x where r, files
  `file_mode` = rw bits, x only if already executable, no setuid / setgid / sticky).
  `perm_text` shows `s` / `t`.
- App: `OpKind`, `Dialog` / `DialogKind`, `FileOp`, `op_blocked` (hosts without SFTP keep
  their side read-only), `Ask` / `Confirm` / `Measured` / `OpDone` / `CopyPath`; 7 new
  `update()` tests. Runtime: operations on the window's worker thread (reuses its SFTP
  connection; stopped when the window closes); copy path via `arboard`.
- GUI: toolbar per pane, right-click menu, F7 / F2 / Del (not while typing), dialogs for
  new / rename / delete (item list, sizes, total, cannot-be-undone warning) / properties /
  permissions (checkboxes + octal, recursive), red per-item error list with Dismiss.
- Docs: USER_MANUAL "Managing files", README, SECURITY, AGENTS.
- Verified: fmt, clippy, all tests; live SFTP test (unicode / quote names, `..`, exists,
  recursive chmod incl. a mode without x, link not followed, delete); GUI on Windows 11
  (second monitor) against the lab: new folder / file on both sides, F2 rename, duplicate
  name error, right-click menu, permissions 2770 recursive (checked with `ls -l`),
  multi-select delete with counts, per-item error (read-only folder), properties, copy
  path, host without SFTP. Not verified here: the Unix-only local tests (links, chmod),
  which run on Linux / macOS CI.

## [2026-11-19] — Telepad 0.8.3
- Fixed: hosts with an option that takes several arguments (e.g. `LocalForward 30001
  127.0.0.1:30001`) broke every ssh connection with "Missing target argument" — the
  generated `telepad.ssh_config` quoted the whole value. Options are now written exactly as
  entered; the file is regenerated on the next start.

## [2026-11-19] — Telepad 0.8.2
- New: **Open files…** (#62) — a WinSCP-style window per host: this computer on the left,
  the host on the right; upload / download by buttons, F5 or drag and drop, overwrite
  question (Overwrite / Skip / Newer only / Apply to all), progress with speed and Cancel.
  Over SFTP; hosts without SFTP use `scp`; rsync (with a dry-run preview) when both sides
  have it. Last folders are remembered per host.

## [2026-11-19] — Tested and merged: file window (#62)
- PR #65 merged (CI green on Windows, macOS, Linux, `db-live`, `files-live`); issue closed.
- Tested on Windows 11 against local lab containers (SFTP and no-SFTP), throwaway database,
  second monitor: open / close / exit end the ssh process, browsing, sort / filter / hidden,
  transfers by buttons, F5 and drag and drop, overwrite question, 300 MiB upload + Cancel,
  scp fallback modes, host down + Retry.
- Fixed while testing: spinner next to a failed connect.

## [2026-11-19] — feat(files): WinSCP-style file window (#62)
- Issue #62 addressed on GitHub (branch `62-file-window`).
- Core `files/`: own SFTP v3 client over `ssh -s <alias> sftp` (pipelined, 16 × 32 KiB),
  fallback for hosts without SFTP (`sh` listing + `scp -O`, uploads without `-p` so Windows
  modes never make files world-writable), rsync (both-sides check, dry run, `--protect-args`,
  no `--delete`), transfer plan (recursive, existing files) and run (`.telepad-part` then
  rename, mtime kept, cancel). `store`: last folders per host (`files_paths:<alias>`).
  `expand_home("~")` now returns the home folder.
- App `files.rs` (+ `tests_files.rs`): sessions, panes, sort / filter / hidden, transfers
  with progress and speed, overwrite question (Overwrite / Skip / Newer only / Apply to all),
  rsync preview → Run. GUI: **Open files…** in the host header opens one native window per host;
  drag and drop, F5, double-click to open folders.
- Test lab `tests/sshd/` (Alpine OpenSSH + sftp-server + rsync; `NO_SFTP=1`) and CI job
  `files-live`.
- Docs: USER_MANUAL "Files", README, SECURITY, AGENTS (fifth write exception, architecture).
- Verified: fmt, clippy, all tests, live tests against the lab (SFTP, fallback, rsync check);
  GUI on Windows 11 (second monitor) against the lab: browse, upload / download of a folder
  tree, drag and drop, overwrite question (Newer only, Apply to all + Overwrite), 300 MiB
  upload with progress and Cancel (no part file left), scp fallback upload / download with
  correct modes. Not verified: rsync copy from Windows (rsync not installed here; the
  rsync run is covered on Linux CI), macOS / Linux GUI.

## [2026-11-19] — Milestone "File manager (WinSCP-style)" (#62, #63, #64)
- Milestone 2 created on GitHub; three issues, all `enhancement`
- #62 feat(files): WinSCP-style file window — local and remote panes with transfers
  (scope: core `files` module over `ssh -s sftp`, app, gui; new write exception)
- #63 feat(files): file operations — new, rename, delete, permissions (depends on #62)
- #64 feat(files): built-in VS Code-like editor for local and remote files (depends on #62)

## [2026-11-19] — Telepad 0.8.1
- New: **Host Management** (#58) — ☰ → Host Management… shows Telepad's hosts next to
  `~/.ssh/config` (or another config file): compare per host, copy / edit / delete on
  either side, Merge / Replace / Sync, preview, then Apply with a timestamped backup of
  the config; comments, Include, Match and Host * stay as they are.

## [2026-11-19] — Tested and merged: Host Management (#58)
- PR #59 merged (CI green on Windows, macOS, Linux); issue closed.
- Tested on Windows 11 with a throwaway database and a made-up config (second monitor,
  200 %): compare, Sync conflict, Merge / Replace, add / edit / rename / delete, preview,
  Apply with backup, outside-change refusal, Reload with pending changes.
- Fixed while testing: preview height in a small window, shared-line aliases listed,
  Reload allowed with pending changes.

## [2026-11-19] — feat(hosts): Host Management window (#58)
- Issue #58 addressed on GitHub (branch `58-host-management`).
- Core `hostsync.rs`: block-preserving config model, compare, apply (upsert / delete in
  place, new hosts before `Host *` / `Match`), line diff, safe write (changed-on-disk check,
  timestamped `.bak`, atomic, permissions kept). `store`: `apply_host_changes` (one
  transaction), `hostsync_config_path` setting.
- App: `hostsync.rs` (pending per side, Merge / Replace / Sync with conflicts, edit / add
  form, preview, apply), 6 `update()` tests. GUI: `views/host_sync.rs`, menu entry,
  runtime load / pick / apply, icons SYNC / ARROW_LEFT / ARROW_RIGHT / DESKTOP / EQUALS /
  NOT_EQUALS.
- Docs: USER_MANUAL (Host Management), README, AGENTS.
- Verified: fmt, clippy, all tests (core 106, app 60, gui 5); GUI with a throwaway database
  and a made-up config file (never the real ~/.ssh/config): compare, Sync with a conflict,
  preview diff, Apply (backup written, comments / Include / Match / Host * / shared line
  kept, Telepad host added), shared-line rows, Add to config with an option, preview in a
  small window. Not verified: macOS / Linux file permissions (CI only).

## [2026-11-19] — feat(hosts): Host Management window — compare and sync Telepad hosts with an OpenSSH config
- Issue #58 created on GitHub
- Scope: core (config parse / write with backup, compare), app, gui (new window), docs
- Labels: enhancement

## [2026-11-19] — Telepad 0.8.0
- New: **database browser** (#52, #53, #54) — **Open database…** for MariaDB / MySQL /
  PostgreSQL in containers and on hosts, through the host's SSH login: HeidiSQL-style
  window with tree, data grid (filter, sort, edit / insert / delete with Post), queries
  (cancel, confirmation for dangerous statements), table structure editor, Read-only
  switch, saved connections, passwords optionally in the OS password store.
- `telepad.db` schema v3 (new `db_connections` table). Older Telepad versions cannot open
  a database that 0.8.0 has opened; update every copy that shares the file.

## [2026-11-19] — Tested and merged: database browser (#52, #53, #54)
- PR #55 merged (all CI green incl. macOS / Linux and the new `db-live` job); issues closed.
- Tested on Windows 11: gate checks, live driver tests (MariaDB 11, MySQL 8.4,
  PostgreSQL 17), GUI end to end through a local SSH lab with made-up data (filter, sort,
  cancel of a running query added to the coder's run); real host dockercenter: read-only
  SSH forward to a container reached MariaDB, no login or write.
- Not verified by hand: macOS Keychain, Linux Secret Service (built and tested in CI).

## [2026-11-19] — feat(db): HeidiSQL-style database browser (#52, #53, #54)
- Issues #52, #53 and #54 addressed on GitHub (one branch, `52-db-browser`).
- Core `db/`: detection of database containers / host listeners, `ssh -N -L` tunnel
  (closed on drop and on exit), `Session` on the `mysql` 28 / `postgres` 0.19 drivers
  (databases, schemas, tables, structure, pages, queries with cancel, transactional row
  edits with a one-row check, DDL), SQL helpers (quoting, statement split, danger and
  read-only checks), structure editor SQL (`ddl`), OS password store (`keyring` 4).
  `store`: schema v3, `db_connections` table (no passwords).
- App: `db.rs` (sessions, drafts, `DbMsg` / `DbEffect` / `DbJob`), 13 `update()` tests.
- GUI: `views/database.rs` (native window per connection: login, tree, Host / Database /
  Table / Data / Query tabs, structure editor, confirmations, value viewer), runtime worker
  thread per connection, **Open database…** in the container menu and host header.
- CI: `db-live` job with MariaDB 11 and PostgreSQL 17 service containers.
- Docs: USER_MANUAL (Database browser), README, SECURITY, AGENTS (fourth write exception).
- Verified: fmt, clippy, all tests; live tests against MariaDB 11, MySQL 8.4 and
  PostgreSQL 17 containers; GUI end to end through a local SSH lab container (made-up
  data): detection, user from container env, saved password (Windows Credential Manager,
  not in telepad.db), tree, data grid, edit / insert / post, invalid value error, Read-only,
  dangerous-query confirmation, run current statement, structure add column, typed-name
  confirmation, tunnel closed with the window and on exit. Not verified: macOS Keychain and
  Linux Secret Service (CI builds only), real hosts.

## [2026-11-19] — Milestone "Database browser (HeidiSQL-style)" (#52, #53, #54)
- Milestone 1 created on GitHub; the database browser split into three issues, all `enhancement`
- #52 feat(db): HeidiSQL-style database browser — connect, browse tables and run queries
  (scope: core `db` module — detection, SSH tunnel, drivers —, app, gui, docs)
- #53 feat(db): edit, insert and delete rows in the database Data grid (depends on #52)
- #54 feat(db): edit table structure — columns, indexes, foreign keys (depends on #52)
- Owner decisions: new connections editable; always over SSH; saved connections, password optionally in the OS secure store (`keyring`)

## [2026-11-18] — Telepad 0.7.1
- New: **☰ → Public keys… (add / revoke)**: add or revoke one SSH public key on many hosts
  at once, after a typed confirmation, with a result per host; refuses to revoke the key
  Telepad logs in with (#49).

## [2026-11-18] — Tested and merged: #49
- Windows 11 x86_64: fmt, clippy, 112 tests, release build; CI on Windows, macOS, Linux.
- GUI on a throwaway database: invalid key error, fingerprint (= `ssh-keygen -lf`), select
  all / indeterminate / filter, typed confirmation, Add and Revoke runs (unreachable hosts:
  clear "connection timed out" per host), lockout guard refused revoking the login key
  without starting ssh.
- Real hosts `backup` and `dockercenter` (owner's OK, owner's key): Add → dockercenter
  added / backup already present; Revoke → both removed, dockercenter file back to its
  original SHA-256, mode 600 kept, no temp files left; Add again → key left on both, as asked.
- Fixed while testing: window kept on screen with scrolling content; shorter refusal text.

## [2026-11-18] — feat(keys): add or revoke an SSH public key on selected servers (#49)
- Issue #49 addressed on GitHub.
- Core `authorized_keys.rs`: `PublicKey::parse` (type list, strict base64, embedded type,
  no options / private keys), `fingerprint` (= `ssh-keygen -lf`), remote add/revoke script
  (here-doc key, awk match on type + data, temp copy + `mv`, symlink refused),
  `login_keys` lockout guard, `apply_all` (bounded threads). `collector::run_script` /
  `base64` now `pub(crate)`.
- App: `KeysDraft` (text, filter, selected ids, typed confirmation), `KeyRun` results;
  `Msg::KeysOpen/Close/SelectAll/Toggle/Ask/ConfirmCancel/Run/KeyResult`,
  `Effect::ApplyKey`; 2 `update()` tests.
- GUI: `views/keys.rs` (☰ → Public keys…), runtime thread per run, icons KEY /
  CHECK_CIRCLE / X_CIRCLE.
- Docs: AGENTS.md (third write exception), USER_MANUAL, README, SECURITY.
- Verified: fmt, clippy, 112 tests (script run against a temp HOME with the local sh),
  release build. Not verified yet: real hosts and the GUI (tester).

## [2026-11-18] — feat(keys): add or revoke an SSH public key on selected servers
- Issue #49 created on GitHub
- Scope: telepad-core / telepad-app / telepad-gui (new key window, in-app write action)
- Labels: enhancement
- Owner decision: runs in-app after a typed confirmation (third read-only exception)

## [2026-11-18] — Telepad 0.7.0
- Faster refresh: live metrics every round, addresses / ports / updates every 6 h, OS and
  hardware facts once a day, hosts without SSH only pinged (#36).
- New: **Overview — Docker** page, containers of all hosts grouped by stack (#37).
- Overview buttons clearly look like buttons in light and dark theme (#38).
- Hosts menu is a ☰ button at the top right (#41).
- New: **Import from pasted text…** (OpenSSH config, JSON, or one host per line) (#42).
- New: drag hosts by their handle to reorder, in the host list and the table (#43).
- Fix: Merge import adds new hosts at the end of the list (#45).

## [2026-11-18] — Tested and merged: #45
- Windows 11 x86_64: fmt, clippy, 104 tests, release build; CI green on Windows, macOS, Linux (PR #46).
- GUI on a throwaway database: Merge-pasting `golf01` (new) and `bravo` (existing) into a custom
  order appended golf01 at the end; bravo kept its place with the new HostName;
  `telepad.ssh_config` in the same order. Replace order covered by the unit test.

## [2026-11-18] — fix(store): Merge import appends new hosts at the end (#45)
- Issue #45 addressed on GitHub.
- `store.rs`: `Store::import` numbers new rows after `MAX(sort_order)`; updated hosts keep
  their `sort_order`; Replace (table emptied first) keeps the import order. Test
  `merge_import_appends_new_hosts_and_keeps_the_saved_order`.
- `docs/USER_MANUAL.md`: Merge / Replace order.
- Verified: fmt, clippy `-D warnings`, 104 tests. Not verified yet: GUI import (tester).

## [2026-11-18] — fix(store): Merge import puts new hosts between existing ones
- Issue #45 created on GitHub (found while testing PR #44; related to #42, #43)
- Scope: telepad-core (`store.rs`, `Store::import`)
- Labels: bug

## [2026-11-18] — Tested and merged: #41, #42, #43
- Windows 11 x86_64: fmt, clippy `-D warnings`, 103 tests, release build pass; macOS / Linux
  via CI. Headless `telepad-check` unchanged (dockercenter collected, ccl clear error).
- GUI on a throwaway database (made-up hosts, documentation IPs):
  - #41: ☰ at the top right in dark and light theme, all menu items; bar wraps at 900 px.
  - #42: "Paste from clipboard" (test text put on the clipboard with the owner's OK),
    CRLF lines read, preview 2 added, import → User / Port in the generated ssh config.
  - #43: drag in host list and table, order kept after restart and in the ssh config,
    handles hidden with hints while searching.
- Found while testing (pre-existing, filed separately): Merge import gives new hosts
  `sort_order` 1, 2, … so they land between existing hosts instead of at the end.

## [2026-11-18] — feat(ui): reorder hosts by drag and drop (#43)
- Issue #43 addressed on GitHub.
- `store.rs`: `save_order` (one transaction; `sort_order` already in the schema).
- App: `Msg::MoveHost` / `OrderSaved`, `Effect::SaveOrder`; `drop_index`, `can_reorder`;
  selection and host state move with the host; a failed save reloads the database order.
- GUI: ⋮⋮ grip (`widgets::grip_handle`) as drag source, drop line + release on each row
  (`widgets::drop_target`) in the host list and the overview table; hidden with a hint while
  searching, grouping or sorting. Also moved a doc comment that #37 had detached from
  `worst_usage`.
- Files: `store.rs`, `msg.rs`, `table.rs`, `update.rs`, `lib.rs`, `tests.rs`, `runtime.rs`,
  `widgets.rs`, `icons.rs`, `views/panels.rs`, `views/overview.rs`, `views/mod.rs`, manual,
  AGENTS.md.
- Verified: fmt, clippy, tests; GUI on a throwaway database (4 made-up hosts): drag in the
  side bar and in the table, order kept after restart, generated ssh config in the new
  order, handles hidden with hints while searching. Not verified: on the real inventory
  (would change its saved order).

## [2026-11-18] — feat(import): import hosts from pasted plain text (#42)
- Issue #42 addressed on GitHub.
- `store.rs`: `parse_pasted` — JSON / OpenSSH config text via `parse_import`, otherwise
  `[alias] [user@]host[:port]` per line; unreadable lines and invalid hosts are listed.
- App: `Msg::PasteImport/PasteFromClipboard/ClipboardRead/PasteApply/PasteCancel`,
  `Effect::ReadClipboard`, `Drafts::paste`, `ImportDialog::skipped`; GUI paste window,
  menu item, skipped lines in the preview; `arboard` (already in the lock file via eframe).
- Files: `store.rs`, `msg.rs`, `state.rs`, `update.rs`, `lib.rs`, `tests.rs`, `runtime.rs`,
  `views/import.rs`, `views/panels.rs`, `views/mod.rs`, `Cargo.toml`s, manual, AGENTS.md.
- Verified: fmt, clippy, tests (parser + update flow); GUI: menu item, window, typed list
  with a bad line → preview "2 hosts, 1 left out (line 3)", cancelled (nothing imported).
  Not verified: "Paste from clipboard" button live (would read the user's clipboard).

## [2026-11-18] — fix(ui): Hosts menu as icon-only hamburger at the top right (#41)
- Issue #41 addressed on GitHub.
- `views/panels.rs`: the menu is an icon-only ☰ button (tooltip) placed in the top-right
  corner as its own child UI; the rest of the top bar wraps in the width left of it.
- Docs: README, manual (Window layout, every "☰ Hosts →" reference), AGENTS.md.
- Verified: fmt, clippy, tests; release GUI at 2400 px (menu opens, all items) and 900 px
  (menu stays top-right, bar wraps). Not verified: light theme (left to tester).

## [2026-11-18] — Issues #41, #42, #43 created
- #41 fix(ui): Hosts menu as icon-only hamburger at the top right — scope `telepad-gui`, label enhancement
- #42 feat(import): import hosts from pasted plain text — scope core / app / gui, label enhancement, depends on #41
- #43 feat(ui): reorder hosts by drag and drop in host list and overview table — scope core / app / gui, label enhancement
  (`sort_order` already in the schema, no migration)

## [2026-11-18] — chore: project skills, #34 status
- `.claude/skills/` (planner, coder, tester, reviewer, shared API notes) copied from the
  senopati project and adapted to Telepad (`telepad-check`, GitHub, `telepad.db`).
- #34 status changed: ready-for-review → closed.

## [2026-11-18] — Tested and merged: #36, #37, #38 (PR #39)
- Windows 11 x86_64: fmt, clippy `-D warnings`, 98 tests, release build pass; macOS / Linux
  left to CI (no local targets).
- Headless `telepad-check` on dockercenter, backup, nas72, ccl matches direct read-only ssh
  (OS, kernel, cores, updates 5/1 and 314/76, reboot flag, 115 / 10 listeners, 85/85
  containers); refused key and unreachable host reported clearly, whole run 10 s.
- GUI after 3 automatic rounds: dockercenter updated 0 s ago, OS / CPU and
  addresses / ports / updates kept from the read 2 min earlier; ccl only pinged (1.6 s,
  "SSH is tried again when the host answers ping"); Docker overview and both overview
  buttons checked in dark and light theme; usable at 900 px width.

## [2026-11-18] — feat(ui): Docker overview and overview buttons checked on real data (#37, #38)
- Issues #37 and #38 addressed on GitHub (PR #39); status ready-for-review.
- Release GUI against the real inventory: Docker page shows 68 stacks / 324 containers on
  31 hosts with Portainer and compose controls, in dark and light theme.
- Fix found while checking: untagged images (`sha256:…`) and `docker.io/` names widened the
  image column and pushed IPs / ports off screen; `Container::short_image` now shortens them
  (full name on hover).
- Fix found while checking: the inactive overview button's outline was too pale in the light
  theme; it now uses the dark accent text colour there.
- Files: `model.rs`, `views/docker_overview.rs`, `widgets.rs`, `tests.rs`.

## [2026-11-18] — perf(collector): tiered collection verified on real hosts (#36)
- Issue #36 addressed on GitHub (PR #39); status ready-for-review.
- `telepad-check --json` on the full inventory: 49 SSH hosts collected with OS, CPU,
  addresses, ports, updates and containers; failures only from existing access problems.
- Live tier against real hosts skips HOSTNAME/OSRELEASE/UNAME/CPUINFO/IPADDR/LISTEN/UPDATES:
  `backup` 5.9 s → 1.6 s, `dockercenter` (85 containers) 5.3 s → 4.5 s per round.
- Files: `script.sh`, `collector.rs`, `state.rs`, `update.rs`, `msg.rs`, `tests.rs`,
  `views/detail.rs`, `widgets.rs`, `docs/USER_MANUAL.md`, `AGENTS.md`.

## [2026-11-18] — perf(collector): tiered collection, Docker overview, overview buttons (#36, #37, #38)
- Issues #36 (enhancement), #37 (enhancement), #38 (bug, accessibility) created on GitHub.
- #36: `script.sh` tiers `TP_STATIC` (daily: hostname, OS, kernel, CPU model) and `TP_SLOW`
  (6 h: addresses, listening ports, updates / reboot); live metrics every round.
  `collector::Scope` + `carry_over` keep skipped facts; Refresh, new/edited hosts and a
  reboot read everything. Hosts down without ping reply are only pinged until they answer
  (ssh still every 10 min).
- #37: "Overview — Docker": containers of all hosts grouped by stack (`docker_stacks`).
- #38: overview buttons filled with the accent colour when shown, outlined otherwise.

## [2026-11-17] — Telepad 0.6.1
- Update check no longer uses the GitHub API, so offices sharing one IP see updates again;
  failed checks retried after 30 minutes (#34).
- New: **Install automatically** in Software update… (off by default) (#34).

## [2026-11-17] — fix(update): check via github.com, retry after failures, automatic install (#34)
- Cause: the check used `api.github.com` without a token (60 requests/hour per public IP,
  shared by a whole office behind NAT); the automatic check failed silently and only
  retried after 6 h, so no update button appeared.
- `update.rs`: latest tag from the `github.com/<repo>/releases/latest` redirect
  (`tag_from_location`, repo and semver checked), downloads from `releases/download/<tag>/`
  (`web_check`, presence probed via the `.sha256`); API only as fallback.
- Failed checks retried after 30 min (`RETRY_AFTER_FAILURE`); the error shows in
  Software update….
- `update_auto_install` (off by default): newer release with a download for this OS is
  installed in the background after a successful check; restart stays the user's click.
- Verified from the office network with the API limit at 0: check found 0.6.0 in under 2 s,
  full download + SHA-256 + replace of a scratch `telepad.exe` succeeded.

## [2026-11-17] — Telepad 0.6.0
- Maintenance flags: reboot needed, pending / security updates, OS end of support, in a
  sortable Flags column, cards, host list and the host page (#25).
- Theme switch: System / Light / Dark, readable colours in both (#27).
- Docker actions in a terminal that shows the command first: shell, logs, start / stop /
  restart / pause / resume / kill / remove, compose up / down / restart / stop; How it
  runs (rebuilt `docker run`, compose files, secrets masked); Open in Portainer (#26).
- Docker Swarm and Kubernetes: cluster panel, badge, degraded flag, one cluster shown
  once; service / node / stack and workload actions in a terminal (#28).
- App icon (#21); IdentityFile picker lists the keys in `~/.ssh` and explains keys whose
  permissions are too open (#23).

## [2026-11-17] — feat(cluster): Docker Swarm and Kubernetes panel and actions (#28)
- `script.sh` `===SWARM===` (managers: cluster ID, nodes, stacks, services, tasks not
  running; workers: role only) and `===KUBE===` (kubectl / k3s / microk8s, `sudo -n`
  only; nodes, workloads, problem pods filtered on the host, pod count). Read-only,
  `timeout 15` per call, output capped.
- `cluster.rs`: model, parsers (fixtures `swarm_manager.txt`, `kube_k3s.txt`), problems,
  badge, cluster key. `HostReport.cluster`.
- `docker_actions.rs`: `Plan` generalised (docker / compose / kubectl, replica prompt in
  the terminal for scale); Swarm service logs / rolling restart / scale, node drain /
  pause / activate, stack rm (`yes`); Kubernetes logs / rollout restart / scale. Names
  validated (Swarm names, DNS-1123, node IDs, known kubectl tools).
- `telepad-app`: `host_flags` (cluster flag first), `host_urgency`, `cluster_shown_by`,
  Swarm / Kubernetes messages (Swarm changes only from managers).
- UI: Cluster section (nodes, services, stacks, failing tasks; k8s nodes, workloads,
  problem pods) with action menus; Docker cell badge; "Same cluster as …" link.
- Verified (read-only) on a real Swarm manager reachable under two aliases: 1 node, 8
  stacks, 20 services shown; second alias shows "Same cluster as …"; service logs
  script ran end to end. No Kubernetes host available: covered by fixtures. Write
  actions not run on production.

## [2026-11-17] — feat(docker): container and compose actions in a terminal, How it runs (#26)
- Owner decision: option A, terminal only. AGENTS.md read-only rule gets its second exception.
- `docker_actions.rs`: `ContainerAction` (shell, logs, start/stop/restart/pause/resume/kill/
  remove, offered by state), `StackAction` (logs, up, restart, stop, down), `Plan` → POSIX
  script that prints the command and waits for Enter / `yes`; IDs, project names and label
  paths validated and single-quoted; Docker or Podman, `sudo` asks in the terminal.
- "How it runs": read-only `docker inspect` + compose files over BatchMode ssh →
  rebuilt `docker run` (network name for Portainer's network IDs), secrets masked.
- Collector: shared `run_script`; `docker_action_args` like the BMC setup (base64 script).
- UI: ☰ per container, Compose menu per compose stack, Open in Portainer / "change it in
  Portainer" for Portainer stacks, How it runs window with Copy.
- Verified on a real Docker host (read-only only): menus and states, How it runs, compose
  and container Logs scripts end to end. Write actions not run on production containers;
  covered by unit tests and an `sh -n` syntax check.

## [2026-11-17] — feat(ui): System / Light / Dark theme switch (#27)
- `ThemePref` saved as `ui_theme` in `settings`; `Msg::SetTheme`; combo in the top bar.
- Own colours moved into a per-theme `Palette` (text colours ≥ 4.5:1 on the panel, usage-bar
  fills ≥ 4.5:1 under egui's bar text), enforced by a contrast test.
- Verified on Windows: light theme overview and host page readable (screenshots deleted).

## [2026-11-17] — feat(collector): overview flags for reboot, pending updates, end-of-life OS (#25)
- `script.sh` `===UPDATES===` (read-only, offline): host date, reboot flag
  (`/var/run/reboot-required` + packages, `needs-restarting -r`, or running kernel older
  than the newest installed RPM kernel), pending/security update counts (apt-check /
  `apt-get -s`, dnf/yum `check-update --cacheonly [--security]`, apk).
- `eol.rs`: end-of-support table (Ubuntu, Debian, CentOS/Stream, RHEL, Rocky, Alma, Oracle,
  Alpine, openSUSE Leap) and date arithmetic against the host's date; `OsRelease` now
  carries `ID`/`VERSION_ID`; `HostReport.maintenance`.
- `telepad-app`: `maintenance_flags` (severity-ordered), `urgency`, sortable `Flags` column.
- UI: Flags column (table), flag chips (cards), most-urgent icon (compact list),
  Maintenance section (detail).
- Verified against real hosts (read-only): Ubuntu 16.04 / 20.04 and CentOS 7 flagged EOL;
  RHEL 8 security count fixed (advisories → packages, 184 of 247); reboot known on RPM
  hosts via the kernel check; flags visible in the overview and host list.

## [2026-11-17] — feat(cluster): Docker Swarm and Kubernetes panel (view first, then manage)
- Issue #28 created on GitHub
- Scope: telepad-core (script.sh, parsers, model), telepad-app, telepad-gui (cluster panel)
- Labels: enhancement — phase 2 (management) depends on the #26 decision

## [2026-11-17] — feat(ui): light / dark / system theme switch
- Issue #27 created on GitHub
- Scope: telepad-gui (theme switch, colour contrast), telepad-core (`ui_theme` setting), telepad-app
- Labels: enhancement

## [2026-11-17] — feat(docker): container shell, actions and compose/run command view (Portainer-like)
- Issue #26 created on GitHub
- Scope: telepad-core (remote commands), telepad-app, telepad-gui (Docker section)
- Labels: enhancement — needs the owner's decision on relaxing the read-only rule

## [2026-11-17] — feat(collector): overview flags for reboot needed, pending updates and end-of-life OS
- Issue #25 created on GitHub
- Scope: telepad-core (script.sh, parse, model, EOL table), telepad-app, telepad-gui
- Labels: enhancement

## [2026-11-17] — fix(ui): choosing an IdentityFile on macOS; key permission hint (#23)
- Host editor: **Browse…** opens in the current key's folder or `~/.ssh` (hidden on
  macOS/Linux, so it was unreachable from the home folder), with a Cmd+Shift+. hint; an arrow
  next to the field lists the private keys in `~/.ssh` (`find_ssh_keys` reads only each
  file's first line; `.pub`, `known_hosts`, `config`, PuTTY `.ppk` are skipped).
- ssh's "UNPROTECTED PRIVATE KEY FILE … bad permissions" is now reported as
  "private key … is readable by other users … run: chmod 600 <key>" instead of
  "authentication failed" (reported on macOS with a 0644 key).
- Verified on Windows: the editor's key list offered the four OpenSSH keys in `~/.ssh` and
  skipped `.ppk`/`.pub`/text files; tests for key detection, the editor flow and the error
  text. Not verified on a Mac: the macOS dialog starting in `~/.ssh`.

## [2026-11-17] — feat(ui): Telepad app icon (#21)
- Original icon (rounded blue gradient square, white "T", green online dot) drawn by
  `scripts/make-icon.py`; `assets/icon/` holds PNG (256, 1024), `.ico` (16–256) and `.icns`.
- Windows: `build.rs` embeds the icon and version info in `telepad.exe`; installer uses it
  (`SetupIconFile`). All platforms: window icon via `ViewportBuilder::with_icon`.
  macOS: `Telepad.app/Contents/Resources/telepad.icns` + `CFBundleIconFile`. README header.
- Verified on Windows: icon extracted from the built exe, title-bar icon in the running
  app, file properties show "Telepad" 0.5.0.

## [2026-11-17] — Telepad 0.5.0
- Windows installer (`…-windows-x86_64-setup.exe`, per-user) next to the portable zip (#10).
- macOS `Telepad.app` in a `.dmg` per architecture, ad-hoc signed, not notarized (#9).
- Installed copies keep data in the per-user folder; portable zips keep it next to the
  program (`portable` marker).
- Overview: sortable, resizable table (#4) or cards (#16); bigger Overview button; Add host
  in the Hosts menu (#16); compact host list with grouping by status (#8).
- Fixes: detail header and top bar wrap (#5), missing-glyph boxes (#6), Windows `.sha256`
  line ending (#7); import of any host-list JSON and one-click `~/.ssh/config` import.

## [2026-11-17] — feat(release): macOS Telepad.app in a .dmg (#9)
- `package-unix.sh` on macOS also builds `Telepad.app` (Info.plist, both programs, licences),
  ad-hoc signs it (`codesign -s -`; no Apple Developer account, so not notarized) and packs
  it with an Applications link into `telepad-<ver>-macos-<arch>.dmg` (+ LF `.sha256`).
- No portable marker in the app: data in `~/Library/Application Support/Telepad`.
- Self-update inside a `.app` re-applies the ad-hoc signature after swapping the executable
  (`platform::after_self_update`), so the bundle stays valid.
- Release workflow mounts each `.dmg` read-only and checks the Applications link, both
  executables, no marker, `plutil -lint`, `codesign --verify`, `telepad-check --help`.
- Not verified on a real Mac: install by drag, first start through Gatekeeper, and an
  in-app update of the bundle (CI only).

## [2026-11-17] — feat(release): Windows installer alongside the portable zip (#10)
- `installer/telepad.iss` (Inno Setup 6): per-user install to `%LOCALAPPDATA%\Programs\Telepad`
  (no admin), Start-menu entry, optional desktop icon, uninstaller; data is kept.
- Data folder: `--db` / `TELEPAD_DB` > next to `config` > next to the exe when portable
  (`portable` marker, or an existing `telepad.db`) > per-user folder (`%APPDATA%\Telepad`,
  `~/Library/Application Support/Telepad`, `~/.local/share/telepad`). Zips ship the marker,
  so portable copies behave as before; `config` is also looked for in the data folder.
- `package-windows.ps1 -Installer` builds `telepad-<ver>-windows-x86_64-setup.exe` (+ LF
  `.sha256`); the release workflow installs Inno Setup, builds it, verifies every `.sha256`,
  and smoke-tests a silent install → `telepad-check --help` → silent uninstall.
- Verified locally: a copy with the marker created its database next to itself, a copy
  without it in `%APPDATA%\Telepad` (test folder removed afterwards); zip contains `portable`.

## [2026-11-17] — feat(ui): overview cards, bigger Overview button, Add host in Hosts menu (#16)
- Overview **Table | Cards** switch (saved as `ui_overview_cards`); cards wrap to the width,
  follow search and a **Sort by** picker (`Msg::SetSort`, `SortKey::ALL`).
- Large full-width **Overview — all hosts** button; Add host removed from the host list;
  Hosts menu grouped Add / Import / Export.
- Verified on Windows with 65 real hosts: button 38 units tall, cards in 11 rows, setting
  saved, menu items present. Sort-by picker driven with real mouse clicks on test hosts:
  inventory zeta,alpha,mike → Host alpha,mike,zeta → reverse zeta,mike,alpha.
- Manual (Overview: Table / Cards) and the packaged README.txt use the new menu labels.

## [2026-11-17] — tested: #8 passes
- Windows 11, release build, 65 real hosts (read-only): compact view by default; Group by
  status → Down (16), Up (49); Detailed + grouped stored as `ui_compact_host_list=0`,
  `ui_group_by_status=1` and restored after a restart. fmt, clippy, 62 tests, release
  build; diff scanned.

## [2026-11-17] — feat(ui): compact host list, group by status (#8)
- Host list: **Compact** view (default) — one line per host with the worst of CPU / RAM /
  disk on the right, details on hover — or the previous **Detailed** rows; optional
  **Group by status** (Down / Up / Not checked yet, collapsible, down first).
- Both choices are saved in the `settings` table (`ui_compact_host_list`,
  `ui_group_by_status`) via `Effect::SaveUiPrefs`, loaded with `Msg::Started`.
- Logic in `telepad-app` (`host_list_groups`, `worst_usage`, `Msg::SetCompactHostList`,
  `Msg::SetGroupByStatus`) with 3 new tests; store round-trip test in core.
- Verified on Windows against 65 real hosts (read-only): default compact; Group by status
  showed Down (16) and Up (49); Detailed + grouped saved as 0/1 in the database and restored
  after a restart.

## [2026-11-17] — tested: #7 passes
- Release dry run on PR #14: all five platform packages verified with `sha256sum -c` /
  `shasum -c` (no CR) — windows, linux x86_64/aarch64, macos x86_64/aarch64 "zip: OK".

## [2026-11-17] — fix(release): Windows .sha256 written with LF (#7)
- `package-windows.ps1` wrote the hash file with `Set-Content`, which ends the line with
  CRLF, so `sha256sum -c` looked for a file name ending in a carriage return and failed. It
  now writes `<hash>  <file>` with an LF ending and no BOM, like `sha256sum` does.
- Release workflow: every platform's package check fails on a CR in the `.sha256` and runs
  `sha256sum -c` (`shasum -a 256 -c` on macOS) against the zip.
- Verified on Windows: the new file ends with LF, has no CR or BOM, and `sha256sum -c` → OK.

## [2026-11-17] — tested: #6 passes
- Windows 11, release build: Docker stack headers on a real Docker host show
  "Host port → container" with the arrow (read-only collection); the glyph test covers every
  GUI string (incl. "→ Container" in Ports in use) and all icon constants, and fails without
  the fallback. fmt, clippy, 58 tests, release build; diff scanned.

## [2026-11-17] — fix(ui): "→" no longer renders as a box (#6)
- Cause: egui's proportional font (Ubuntu-Light + egui-icons) has no U+2192, so the
  `Host port → container` header in Docker stacks and the `→ Container` header in Ports in
  use drew an empty box (monospace port values were fine: Hack has it).
- Fix: `icons::definitions()` adds Hack as the last fallback of the proportional family, so
  any symbol the UI font lacks is drawn by the monospace font instead of a box.
- Guard: GUI test `every_character_in_the_ui_has_a_glyph` scans every string literal in
  `crates/telepad-gui/src` and every icon constant and checks both font families with the
  real font setup; it fails (naming file:line) without the fallback.
- Verified on Windows against a real Docker host (read-only): the headers show the arrow.

## [2026-11-17] — tested: #4 and #5 pass
- Windows 11, 200 % scaling, release build; documentation-IP test hosts (unreachable, so
  long errors fill the OS column). Automated with UI Automation and real mouse input.
- #4: Host sort asc / desc / third click restores inventory order; Address sorts
  numerically (192.0.2.9 < 192.0.2.10 < 192.0.2.30 < 198.51.100.2); dragging the
  Host|Description border 200 px moved the Description column 204 px; long description and
  error text cut with "…" inside their cells; CPU header stays in the window.
- #5: at the 1800 px (900-unit) minimum width every detail and top-bar button is inside the
  window (Delete and the loading count wrap to a second line).
- fmt, clippy, 57 tests and the release build pass; diff scanned for secrets.

## [2026-11-17] — fix(ui): detail header and top bar wrap instead of clipping (#5)
- Host detail header split into a wrapping identity row and a wrapping button row; order
  Open SSH/RDP, Copy ssh command, Refresh | Edit, Clone, Delete (destructive last).
- Top bar wraps too: at the 900-unit minimum width the loading count was cut off.
- Verified on Windows (200 % scaling) at 1800 px and 2560 px wide via UI Automation: every
  header and top-bar button lies inside the window; at minimum width Delete and the
  loading count move to a second line. Test used documentation-IP hosts only.

## [2026-11-17] — feat(ui): sortable, resizable overview columns (#4)
- Overview rebuilt with `egui_extras::TableBuilder`: click a header to sort (natural
  direction, reversed, then inventory order), caret icon shows the direction; drag header
  borders to resize; cells clip with "…" and show the full text on hover, so errors and
  long OS names no longer widen columns; sticky header, horizontal scroll when needed.
- Sorting lives in `telepad-app` (`table.rs`: `Sort`, `SortKey`, `overview_rows`, shared
  `fullest_disk`/`hottest`), changed only via `Msg::SortBy`; 4 new tests (click cycle,
  numeric columns, missing values last, numeric IP order + filter).
- Verified on Windows against 65 real hosts (read-only): CPU sort desc/asc/off and
  Address sort through UI Automation; screenshots checked for clipping and the arrow.
- Not verified by automation: dragging a column border (built into TableBuilder).

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
