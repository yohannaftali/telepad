# Outside libraries

Every crate Telepad depends on directly, answered the same way (5W 1H):

- **What** — what the library is.
- **Who** — which Telepad crate depends on it.
- **Where** — the modules that use it.
- **When** — the situations where it is useful (what the user is doing when it runs).
- **Why** — why Telepad uses it rather than something else or its own code.
- **How** — how Telepad uses it: features turned on or off, the API we call, rules.

Versions are in the workspace [`Cargo.toml`](../Cargo.toml) (`[workspace.dependencies]`) and
in [`crates/telepad-core/Cargo.toml`](../crates/telepad-core/Cargo.toml). Crate docs:
[telepad-core](crates/telepad-core.md) · [telepad-app](crates/telepad-app.md) ·
[telepad-gui](crates/telepad-gui.md) · [telepad-check](crates/telepad-check.md) ·
[developer guide](DEVELOPER_GUIDE.md).

**Adding a library:** prefer the standard library or a few lines of our own code; when a
library is worth it, add it with only the features we need (`default-features = false` where
it helps), keep it in the crate that needs it (never a GUI library outside `telepad-gui`, no
library at all in `telepad-app`), and add an entry here in the same PR.

**Contents:** [Overview](#overview) · [GUI](#gui) · [Storage and data](#storage-and-data) ·
[Network, TLS and updates](#network-tls-and-updates) · [Databases](#databases) ·
[Files and exports](#files-and-exports) · [Keys and cryptography](#keys-and-cryptography) ·
[Platform](#platform) · [Build](#build)

## Overview

| Library | Crate | Used for |
|---|---|---|
| [eframe / egui](#eframe-and-egui) | gui | the window and every widget |
| [egui_extras](#egui_extras) | gui | table builder, SVG / image loaders |
| [rfd](#rfd) | gui | native open / save file dialogs |
| [arboard](#arboard) | gui | system clipboard |
| [rusqlite](#rusqlite) | core | `telepad.db`, `preference.db`, `history.db`, `update.db` |
| [serde / serde_json](#serde-and-serde_json) | core, check | JSON import / export, `--json`, Portainer API, saved facts |
| [thiserror](#thiserror) | core | typed error enums |
| [regex](#regex) | core | editor find / replace |
| [ureq](#ureq) | core | self-update HTTPS requests |
| [semver](#semver) | core | version comparison (updates, Portainer minimum) |
| [self-replace](#self-replace) | core | replacing the running executable |
| [zip](#zip) | core | unpacking the release zip |
| [sha2](#sha2) | core | SHA-256 of downloads, key fingerprints, certificate pins |
| [rustls](#rustls) | core | TLS to Portainer with a pinned certificate |
| [keyring](#keyring) | core | database passwords and Portainer tokens in the OS password store |
| [mysql](#mysql) / [postgres](#postgres) | core | database browser drivers |
| [csv](#csv) / [calamine](#calamine) / [rust_xlsxwriter](#rust_xlsxwriter) | core | table data export / import (CSV, Excel) |
| [pdf-writer / ttf-parser / miniz_oxide](#pdf-writer-ttf-parser-and-miniz_oxide) | core | PDF table export |
| [ssh-key](#ssh-key) | core | OpenSSH keys in My keys |
| [RustCrypto key crates](#rustcrypto-key-and-cipher-crates) (rsa, ed25519-dalek, p256, p384, pkcs8, sec1, aes, cbc, argon2, hmac, sha1, base64, rand_core, zeroize) | core | reading and converting private key formats |
| [p12-keystore](#p12-keystore) | core | PKCS#12 (`.p12` / `.pfx`) keys |
| [pgp](#pgp) | core | OpenPGP key import / export |
| [sysinfo](#sysinfo) | core (Windows) | My Computer facts on Windows |
| [wmi](#wmi) | core (Windows) | thermal zone check and, if it runs, LibreHardwareMonitor's sensors on My Computer |
| [windows-sys](#windows-sys) | core (Windows) | Windows Credential Manager for RDP passwords |
| [winresource](#winresource) | gui (build, Windows) | icon and version info in `telepad.exe` |

Not a library but just as important: Telepad uses the **system OpenSSH client** (`ssh`,
`scp`, `ssh-keygen`) as a process, never an SSH library, so the user's `config`, keys, agent
and `known_hosts` work exactly as in their terminal.

## GUI

### eframe and egui

- **What:** [egui](https://github.com/emilk/egui) is an immediate-mode GUI library in pure
  Rust: every frame the code describes the whole UI again, and egui works out what to draw
  and what was clicked. **eframe** is its official app framework: it opens the OS window,
  creates the OpenGL context, runs the event loop and calls our `App::ui` once per frame.
  egui is re-exported by eframe (`eframe::egui`), so we depend on eframe only.
- **Who:** `telepad-gui` only.
- **Where:** `app.rs` (`eframe::run_native`, `NativeOptions`, `ViewportBuilder`, window icon,
  `on_exit`), every file in `views/`, `table.rs`, `widgets.rs`, `editor_theme.rs`, `icons.rs`
  (fonts), `runtime.rs` (`egui::Context::request_repaint` from threads).
- **When:** all the time — every page, inner window, dialog, table and the editor is egui.
- **Why:** a single native executable with no web runtime, no C++ toolkit and no installer
  needs; the same code on Windows, macOS and Linux; immediate mode fits MVU well — the view is
  a plain function of `(&Model, &mut Drafts)` with no widget tree to keep in sync.
- **How:** `default-features = false` with `glow` (OpenGL renderer — smaller than wgpu and
  works on old GPUs and RDP sessions), `default_fonts`, `accesskit` (screen readers),
  `clipboard`, `wayland` and `x11` (Linux). One OS window only; inner windows are drawn with
  `egui::Area` / painter calls by `views/workspace.rs`. Themes are `egui::Visuals` from
  `widgets::light_visuals` / `dark_visuals`. Fonts: egui's defaults plus Phosphor icons
  (`icons::install`). See the [developer guide](DEVELOPER_GUIDE.md#egui-as-telepad-uses-it).

### egui_extras

- **What:** companion crate of egui with extra widgets and loaders.
- **Who:** `telepad-gui`.
- **Where:** `table.rs` (`TableBuilder`, `Column`) and the views that pass column specs to it
  (`overview`, `files`, `database`, `host_sync`, `logins`); `app.rs`
  (`install_image_loaders`).
- **When:** every table (Hosts, file panes, database grid, Log info, …); drawing the OS logos.
- **Why:** `TableBuilder` gives virtual scrolling (only visible rows are laid out — the
  database grid and file panes can have tens of thousands of rows), resizable columns and a
  sticky header; the image loaders render the bundled SVG logos.
- **How:** `default-features = false`, feature `svg`. Views never use `TableBuilder` directly:
  they go through `crate::table` (`show` / `controlled`), which adds sort, the filter row, the
  value dropdown, the pager and exports on top.

### rfd

- **What:** "Rust File Dialog" — the operating system's native open / save / folder dialogs
  (Windows common dialogs, macOS `NSOpenPanel`, GTK / portal on Linux).
- **Who:** `telepad-gui`.
- **Where:** `runtime.rs` only (`rfd::FileDialog::new()…pick_file()` / `save_file()`).
- **When:** choosing an IdentityFile, importing / exporting hosts, `.rdp` files, keys,
  database import / export files, table exports, the Host Management config file.
- **Why:** users expect their own OS dialog (favourites, network drives, recent places);
  drawing our own in egui would be worse and much more code.
- **How:** a view returns a `Msg`, `update()` returns an `Effect::Pick…`, and the runtime opens
  the dialog. It is the **one allowed blocking call on the UI thread** (dialogs are modal
  anyway); views never call it.

### arboard

- **What:** cross-platform access to the system clipboard (text and images).
- **Who:** `telepad-gui`.
- **Where:** `runtime.rs` (`arboard::Clipboard::new()`, `set_text`, `get_text`).
- **When:** copy buttons (Copy as JSON from a table, copy paths in the file window, copy a
  key in My keys) and "Paste from clipboard" buttons (pasted host import, #42).
- **Why:** Ctrl+C / Ctrl+V inside text fields go through eframe's `clipboard` feature (#71),
  but egui only hands copied text to the OS on the next frame of the viewport that asked,
  which may not come; a button that copies or pastes needs the clipboard right away.
- **How:** `default-features = false` (text only, no image support). Called only from the
  runtime for `Effect::ReadClipboard` and the `CopyText` effects; a failure is shown as a
  message, never a panic.

## Storage and data

### rusqlite

- **What:** Rust bindings for SQLite, the embedded SQL database.
- **Who:** `telepad-core`.
- **Where:** `store.rs` (`telepad.db`), `prefs.rs` (`preference.db`), `history.rs`
  (`history.db`), `strategy.rs` (`update.db`).
- **When:** on start (load hosts, settings, workspaces, Update Strategy), whenever the user
  saves a host / group / connection / Portainer, on every logged transfer or SQL statement,
  and when facts are kept for the next start.
- **Why:** one file per store, transactions (an import either fully happens or not at all),
  schema migrations, no server to run, and the file is easy to back up or copy to another PC.
- **How:** feature `bundled` — SQLite is compiled in, so no system library is needed and every
  platform has the same version. Schema version in `PRAGMA user_version`, migrations step by
  step in `Store::open`; `check_newer_schema` opens read-only first (#180). Files are made
  owner-only (`platform::restrict_db_files`, #110). In the GUI each of `preference.db`,
  `history.db`, `update.db` has its own writer thread.

### serde and serde_json

- **What:** serde is Rust's standard serialization framework (`#[derive(Serialize,
  Deserialize)]`); serde_json reads and writes JSON with it.
- **Who:** `telepad-core` (both), `telepad-check` (serde_json).
- **Where:** `model.rs` (reports for `--json`), `store.rs` (JSON host export / import, Remote
  Desktop settings column), `rdp.rs`, `strategy.rs` (saved facts), `portainer.rs` (API
  responses), `update.rs` (GitHub API fallback), `cluster.rs` / `docker_actions.rs` (Docker
  and Kubernetes JSON output), `table_export.rs` (Copy as JSON), `local_keys.rs` (JWK keys),
  `telepad-check/src/main.rs`.
- **When:** exporting / importing hosts as JSON, `telepad-check --json`, every Portainer read,
  saving last known facts, Copy as JSON, reading `docker inspect` output.
- **Why:** the de-facto standard; derive macros keep the types and their JSON in one place.
- **How:** feature `derive`. Remote JSON (Portainer, Docker) is read into small structs with
  only the fields we need, so unknown fields are ignored and secrets such as stack `Env`
  values are never even parsed.

### thiserror

- **What:** derive macro that implements `std::error::Error` and `Display` for an error enum.
- **Who:** `telepad-core`.
- **Where:** `lib.rs` (`OpenError`), `store.rs` (`store::Error`), `config.rs`
  (`ConfigError`), `local_keys.rs` (`KeyError`).
- **When:** whenever opening the workspace, the database, a config or a key file can fail in
  a way the caller must tell apart (for example `OpenError::NewerSchema` gets its own dialog).
- **Why:** typed errors without hand-written boilerplate; no runtime cost.
- **How:** `#[derive(Debug, thiserror::Error)]` with `#[error("…")]` per variant. Simple
  failures elsewhere stay `Result<_, String>`.

### regex

- **What:** the standard Rust regular expression engine (linear time, no catastrophic
  backtracking).
- **Who:** `telepad-core`.
- **Where:** `files/text.rs` (`matcher`, find / replace).
- **When:** the editor's find bar with "Regular expression" on, Replace / Replace all, and
  also plain search with whole word / match case (the query is escaped and built as a regex).
- **Why:** users type their own patterns; a linear-time engine cannot hang the app on a large
  file or a bad pattern.
- **How:** `default-features = false` with `std`, `perf` and `unicode`. `RegexBuilder` with
  case-insensitivity as an option, `regex::escape` for plain text, `NoExpand` when the
  replacement must be taken literally. The parsers of collected output do **not** use regex —
  they are plain string splitting, which is faster and easier to test.

## Network, TLS and updates

### ureq

- **What:** a small, blocking HTTP client.
- **Who:** `telepad-core`.
- **Where:** `update.rs` (`agent`, check and download).
- **When:** ☰ ▸ Software update…, the automatic update check, `telepad-check --check-update`.
- **Why:** blocking fits our threads (no async runtime to pull in); it is small and uses
  rustls, so no OpenSSL is needed on any platform.
- **How:** an `Agent` with timeouts; the check follows the github.com `releases/latest`
  redirect (the API only as a fallback — it is rate limited per IP, #34). Downloads only from
  `https://github.com/<repo>/releases/download/`. Runs on a runtime thread, never the UI thread.

### semver

- **What:** parsing and comparing Semantic Versioning strings (`0.9.14 < 0.10.0`).
- **Who:** `telepad-core`.
- **Where:** `update.rs` (is the release newer?), `portainer.rs` (`version_ok`, minimum
  Portainer 2.11.0).
- **When:** every update check; every Portainer read.
- **Why:** comparing version strings as text is wrong (`0.10` < `0.9`); semver gets
  pre-releases and ordering right.
- **How:** `semver::Version::parse` / `Version::new` and `>=`.

### self-replace

- **What:** replaces (or deletes) the currently running executable, including on Windows,
  where a running `.exe` cannot simply be overwritten.
- **Who:** `telepad-core`.
- **Where:** `update.rs`.
- **When:** after the user clicks Install in the update window and the SHA-256 check passed.
- **Why:** doing this correctly on Windows (rename-then-replace, cleanup on next start) is
  fiddly; this crate is the standard solution.
- **How:** `install_executables` replaces `telepad-check` next to the running exe first, then
  `self_replace::self_replace(new)` for the running `telepad`; then relaunch. It never touches
  the data files (`telepad.db`, …).

### zip

- **What:** reading and writing ZIP archives.
- **Who:** `telepad-core`.
- **Where:** `update.rs` (extract the two executables from the release zip); tests in
  `table_export.rs` read the generated `.xlsx` (which is a zip) back.
- **When:** installing an update.
- **Why:** releases are published as zips on every platform.
- **How:** `default-features = false`, feature `deflate` only (no encryption, no other
  compression methods). Only the two exact executable names are taken from the archive, so no
  path inside the zip can make it write anywhere else.

### sha2

- **What:** SHA-256 / SHA-512 hash functions from RustCrypto.
- **Who:** `telepad-core`.
- **Where:** `update.rs` (download check), `authorized_keys.rs` (SHA256 key fingerprints as
  `ssh-keygen -l` shows them), `portainer.rs` (certificate fingerprint pin). A second copy
  `sha2_010` (the 0.10 API, renamed in `Cargo.toml`) is used by `local_keys.rs`, because the
  key crates it works with still use that version.
- **When:** every update install, every key listed on Authorized keys, every Portainer TLS
  connection.
- **Why:** audited, pure Rust, no OpenSSL.
- **How:** `Sha256::digest(bytes)`; the update aborts before anything is replaced if the hash
  differs from the release's `.sha256` file.

### rustls

- **What:** a TLS library in pure Rust.
- **Who:** `telepad-core`.
- **Where:** `portainer.rs` (`tls`, the `Pin` certificate verifier).
- **When:** reading or changing a Portainer that serves HTTPS, through its SSH tunnel.
- **Why:** Portainer usually has a self-signed certificate. We need *trust on first use*: pin
  the certificate the first time, then refuse any other — rustls lets us plug in our own
  `ServerCertVerifier`, and it needs no OpenSSL.
- **How:** `default-features = false` with `ring` (crypto provider), `std`, `tls12`. A
  `ClientConnection` to `127.0.0.1` (the tunnel end) driven by our own small HTTP/1.1 client;
  `Pin` compares the SHA-256 of the server certificate with the stored `fingerprint` and still
  checks the TLS signatures.

### keyring

- **What:** a cross-platform interface to the OS password store: Windows Credential Manager,
  macOS Keychain, the Secret Service on Linux.
- **Who:** `telepad-core`.
- **Where:** `db/secret.rs` (`load_in` / `save_in` / `forget_in`), used by the database
  browser and by `portainer.rs` (`load_token` / `save_token` / `forget_token`).
- **When:** connecting to a database with "Save password", saving a Portainer API token,
  removing a connection or Portainer.
- **Why:** secrets must never be written to `telepad.db` or any file we own; the OS store is
  encrypted per user and survives updates.
- **How:** `keyring::Entry::new(service, key)` with services "Telepad database" and "Telepad
  Portainer"; `NoEntry` is treated as "not saved". The Remote Desktop password is a separate
  case: it must be readable by `mstsc`, so it is written with `windows-sys` (see below).

## Databases

### mysql

- **What:** a synchronous MySQL / MariaDB client in pure Rust.
- **Who:** `telepad-core`.
- **Where:** `db/client.rs` (`Session` for MySQL).
- **When:** opening a MySQL / MariaDB database window on a host (or My Computer).
- **Why:** pure Rust, blocking (fits one worker thread per window), supports transactions and
  cancelling a running query (`KILL QUERY` from a second connection).
- **How:** `default-features = false`, feature `minimal-rust` (no native TLS, no compression
  — the connection already goes through the `ssh -N -L` tunnel). `OptsBuilder` to
  `127.0.0.1:<tunnel port>`, `Queryable`, `TxOpts` for grid edits in one transaction.

### postgres

- **What:** the synchronous PostgreSQL client (a blocking wrapper over `tokio-postgres`).
- **Who:** `telepad-core`.
- **Where:** `db/client.rs` (`Session` for PostgreSQL).
- **When:** opening a PostgreSQL database window.
- **Why:** the standard Rust PostgreSQL driver, with error positions (`ErrorPosition`) so the
  SQL editor can point at the failing token.
- **How:** `postgres::Client::connect` to the tunnel end without TLS (the tunnel is SSH),
  `cancel_token()` for Cancel, transactions for edits and imports.

## Files and exports

### csv

- **What:** a fast, RFC 4180 CSV reader and writer.
- **Who:** `telepad-core`.
- **Where:** `db/io.rs` (database table export and CSV import).
- **When:** exporting a database table to CSV, importing a CSV into a table.
- **Why:** correct quoting of commas, quotes and new lines in cell values; streaming, so large
  tables do not have to fit in memory.
- **How:** `csv::WriterBuilder` into `<name>.telepad-part`, renamed when complete. The small
  in-memory table exports (`table_export.rs`) and the history CSV write their own lines and
  defuse formula cells (#111).

### calamine

- **What:** a reader for Excel (`.xlsx`, `.xls`, `.xlsb`) and OpenDocument (`.ods`)
  spreadsheets.
- **Who:** `telepad-core`.
- **Where:** `db/io.rs` (`read_import`).
- **When:** importing an Excel file into a database table (#139).
- **Why:** pure Rust, reads every common spreadsheet format, no Excel installation needed.
- **How:** `open_workbook_auto(path)` (format from the file), `worksheet_range_at(0)` (first
  sheet), first row as headers, cells as text for the column mapping and the row preview.

### rust_xlsxwriter

- **What:** a writer for Excel `.xlsx` files.
- **Who:** `telepad-core`.
- **Where:** `db/io.rs` (database table export), `table_export.rs` (`to_xlsx`, any table's
  right-click Export Excel).
- **When:** Export ▸ Excel from any table, or Excel export of a database table.
- **Why:** real Excel files (bold frozen header, autofilter, numbers as numbers) rather than
  CSV that Excel guesses at.
- **How:** feature `constant_memory` (rows are streamed to disk, so a big database export does
  not hold the whole sheet in memory). Only plain numbers become numeric cells; everything
  else stays text, so values like `007` or `1e5` keep their meaning.

### pdf-writer, ttf-parser and miniz_oxide

- **What:** `pdf-writer` writes PDF objects at a low level; `ttf-parser` reads TrueType /
  OpenType font tables; `miniz_oxide` is a pure-Rust zlib compressor.
- **Who:** `telepad-core`.
- **Where:** `table_export.rs` (`pdf::to_pdf`).
- **When:** Export ▸ PDF from any table.
- **Why:** a small PDF with exactly the UI's fonts (so every character, arrow and icon prints
  the same as on screen) without a PDF engine, a browser or a system font lookup.
- **How:** the GUI passes the fonts it already uses (`runtime::pdf_fonts`: Ubuntu-Light, Hack,
  Noto Emoji, Phosphor). `ttf-parser` gives glyph ids and widths for each character (first font
  that has it), `pdf-writer` writes A4 / landscape pages with the header repeated, embedded
  CID fonts with a ToUnicode map (text can be copied out of the PDF), and `miniz_oxide`
  compresses the font and page streams.

## Keys and cryptography

### ssh-key

- **What:** RustCrypto's reader / writer for OpenSSH private and public keys (including
  encrypted `bcrypt-pbkdf` keys) and certificates.
- **Who:** `telepad-core`.
- **Where:** `local_keys.rs` (My keys); test helpers in `authorized_keys.rs`.
- **When:** My keys lists, inspects, converts or protects a key; Authorized keys needs the
  public key of a private key that has no `.pub` file (#126).
- **Why:** the OpenSSH format is the hub every conversion goes through.
- **How:** features `std`, `ed25519`, `p256`, `p384`, `rsa`, `encryption`, `getrandom`.
  `PrivateKey::from_openssh`, `decrypt`, `encrypt`, `to_openssh`, `public_key().to_openssh()`.
  RSA keys are converted with our own `to_rsa`, because ssh-key 0.6.7 passes `p` twice.

### RustCrypto key and cipher crates

All in `telepad-core/src/local_keys.rs`, used when **My keys** loads, converts or generates a
key in a format other than plain OpenSSH (#101, #107). They are pure Rust (no OpenSSL) and
from the same audited RustCrypto family, so their types work together. Secret bytes are held
in `zeroize::Zeroizing` so they are wiped from memory when dropped.

| Crate | What it is | How Telepad uses it |
|---|---|---|
| `rsa` (`pem`) | RSA keys | PKCS#1 / PKCS#8 RSA keys, building keys from their numbers |
| `ed25519-dalek` (`pkcs8`, `pem`) | Ed25519 keys | Ed25519 in PKCS#8, PKCS#12 and JWK |
| `p256`, `p384` (`pkcs8`, `pem`) | NIST P-256 / P-384 elliptic curve keys | ECDSA keys in SEC1, PKCS#8, JWK |
| `pkcs8` (`encryption`, `pem`, `std`) | PKCS#8 containers | plain and password-encrypted `BEGIN PRIVATE KEY` / `BEGIN ENCRYPTED PRIVATE KEY` |
| `sec1` (`pem`, `std`) | SEC1 EC key format | `BEGIN EC PRIVATE KEY` |
| `aes`, `cbc` | AES block cipher, CBC mode | decrypting / encrypting PuTTY `.ppk` private blobs (`aes256-cbc`) |
| `argon2` | Argon2 key derivation | PPK v3 passphrase → key (Argon2id / i / d) |
| `hmac`, `sha1` | HMAC, SHA-1 | PPK v2 (HMAC-SHA1) and v3 (HMAC-SHA256) MAC checks; PPK v2 key derivation |
| `base64` | Base64 | PPK and JWK bodies, public key blobs |
| `rand_core` (`getrandom`) | OS random numbers | salts and IVs when writing an encrypted key |
| `zeroize` | memory wiping | `Zeroizing<Vec<u8>>` for private blobs and derived keys |

The workspace `Cargo.toml` builds these with `opt-level = 3` even in debug builds, because key
derivation (Argon2, bcrypt-pbkdf, scrypt) and big-number maths are very slow unoptimised.

### p12-keystore

- **What:** reader / writer for PKCS#12 keystores (`.p12`, `.pfx`).
- **Who:** `telepad-core`.
- **Where:** `local_keys.rs`.
- **When:** importing a `.p12` / `.pfx` in My keys, or converting a key to PKCS#12.
- **Why:** a common format for keys exported from Windows and Java tools; pure Rust.
- **How:** `KeyStore::from_pkcs12(bytes, passphrase, Pkcs12ImportPolicy::Relaxed)` to read;
  the private key is then handled as PKCS#8.

### pgp

- **What:** an OpenPGP implementation in pure Rust (rPGP).
- **Who:** `telepad-core`.
- **Where:** `local_keys.rs` (`openpgp` module).
- **When:** importing an armoured or binary OpenPGP secret key in My keys, or exporting a key
  as OpenPGP (#107).
- **Why:** lets a GnuPG authentication key be used for SSH and the other way round without
  GnuPG installed.
- **How:** `openpgp::import` parses the secret key, unlocks it with the passphrase and turns
  the key material into an OpenSSH `PrivateKey`; `openpgp::export` (Convert ▸ OpenPGP) builds an
  armoured OpenPGP secret key from one.

## Platform

### sysinfo

- **What:** cross-platform system information: CPU, memory, disks, network interfaces,
  processes, temperature sensors.
- **Who:** `telepad-core`, **Windows only** (`[target."cfg(windows)".dependencies]`).
- **Where:** `local.rs` (`collect` for My Computer, #129).
- **When:** the My Computer page is open on Windows, or `telepad-check --local` runs there.
- **Why:** Windows has no POSIX `sh` to run `script.sh`; sysinfo reads the same facts through
  Windows APIs. On Linux / macOS the normal script is run locally instead, so the crate is not
  needed there.
- **How:** `default-features = false` with `component`, `disk`, `network`, `system`. Two CPU
  samples `MINIMUM_CPU_UPDATE_INTERVAL` apart for usage; process names for the listening ports
  from `netstat -ano`. Results are filled into the same `HostReport` as a remote host. Its
  temperature list (ACPI thermal zone) is only the last fallback for sensors (#193, see
  [wmi](#wmi)).

### wmi

- **What:** a safe Rust client for WMI (Windows Management Instrumentation) queries, built on
  the `windows` crate that `sysinfo` already uses; it initialises COM for the calling thread.
- **Who:** `telepad-core`, **Windows only**.
- **Where:** `local/sensors.rs` (`wmi_read::lhm`, `wmi_read::thermal_zone_probe`, #193).
- **When:** each My Computer read on Windows (page open, live interval, Refresh,
  `telepad-check --local`).
- **Why:** the direct ACPI thermal-zone query (below), and LibreHardwareMonitor: if the user
  happens to run it (Telepad never needs, suggests, installs or starts it — owner decision on
  #193), it publishes CPU, board, fan, drive and GPU sensors of every vendor in the WMI namespace `root\LibreHardwareMonitor`, readable without
  administrator rights. And the direct ACPI thermal-zone query returns the WMI error code
  `sysinfo` hides, which tells "needs administrator" (`WBEM_E_ACCESS_DENIED`) from "this PC
  publishes none" (`WBEM_E_NOT_SUPPORTED`) for the note under **Sensors**. Calling COM ourselves
  would mean more `unsafe` code.
- **How:** `default-features = false` (no `chrono`). `WMIConnection::with_namespace_path` and
  `raw_query` into small `serde` structs (`Sensor`: Name, SensorType, Value, Parent; `Hardware`:
  Identifier, Name). A missing namespace (LibreHardwareMonitor not running) simply gives no
  rows and no message. Read-only: only `SELECT` queries, nothing is installed, started or
  written.

### windows-sys

- **What:** Microsoft's raw Rust bindings to the Win32 API.
- **Who:** `telepad-core`, **Windows only**.
- **Where:** `platform.rs` (`CredWriteW`, `CredReadW`, `CredDeleteW`).
- **When:** saving, checking or removing a Remote Desktop password for a host (#108).
- **Why:** `mstsc` only reads credentials stored as `TERMSRV/<host>` domain passwords;
  `keyring` writes generic credentials, so we call the API directly.
- **How:** features `Win32_Foundation`, `Win32_Security_Credentials`. This is the **only
  `unsafe` code in Telepad**, kept small and in `platform.rs`. The password is written, its
  existence checked, or deleted — never read back.

## Build

### winresource

- **What:** a build-script helper that compiles a Windows resource file into an executable.
- **Who:** `telepad-gui`, build dependency, **Windows only**.
- **Where:** `crates/telepad-gui/build.rs`.
- **When:** every Windows build of `telepad.exe`.
- **Why:** so Explorer, the taskbar, the Start menu and the installer show the Telepad icon and
  product name.
- **How:** `WindowsResource::new().set_icon("assets/icon/telepad.ico")` plus `ProductName`,
  `FileDescription`, `LegalCopyright`; a failure panics, so a release never ships without the
  icon.
