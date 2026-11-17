# Contributing to Telepad

Issues and pull requests are welcome. This page covers building from source, the
rules every change keeps, and how releases are made. The architecture and full list
of conventions are in [AGENTS.md](AGENTS.md) (written for humans and coding agents alike).

## Reporting a bug

Open an [issue](https://github.com/yohannaftali/telepad/issues) with
the Telepad version (bottom bar), your OS, what you did and what you saw. For a wrong
or missing value, include the output of `telepad-check <alias> --json` and of the matching
command on the host (`df -Pk`, `cat /proc/meminfo`, `docker ps`, …). **Remove host names,
IP addresses and anything private before posting.** Security problems: see
[SECURITY.md](SECURITY.md) instead of a public issue.

## Making a change

1. Fork the repository and create a branch, e.g. `fix/disk-units` or `feat/zfs-pools`.
2. Read [`AGENTS.md`](AGENTS.md): architecture, data flow and the rules every change must
   keep (they apply to humans and coding agents alike). The most important ones:
   - nothing Telepad runs may change a remote host;
   - views only read the model and send messages; `update()` in `telepad-app` is the only
     place state changes (with a test), and the GUI runtime is the only place that does I/O;
   - the UI thread never runs ssh or network calls;
   - OS-specific code only in `crates/telepad-core/src/platform.rs`;
   - no `unwrap()`/`expect()` on anything that depends on a host, the config or the network.
3. A new metric is a new `===SECTION===` in `crates/telepad-core/src/script.sh` (POSIX `sh`,
   BusyBox-safe, every command guarded) + a parser in `crates/telepad-core/src/parse.rs` + a
   fixture test in `crates/telepad-core/tests/fixtures/` + its place in a view under
   `crates/telepad-gui/src/views/`.
4. Use documentation addresses (`192.0.2.0/24`, `198.51.100.0/24`) and made-up host
   names in tests and fixtures, never real inventory.
5. Before you push:

   ```sh
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

6. Open a pull request against `main` describing what changed and how you tested it
   (which OS, which kinds of hosts). Use [Conventional Commits](https://www.conventionalcommits.org)
   for commit messages: `feat(collector): …`, `fix(ui): …`, `docs: …`.

CI runs the same checks on Windows, Linux and macOS for every pull request.

## Building from source

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
cargo run --release --bin telepad           # start the GUI
cargo run --release --bin telepad-check -- --json
```

Debug builds (`cargo run --bin telepad`) keep a console window on Windows; release builds do not.

### Package a release zip

```sh
pwsh scripts/package-windows.ps1     # Windows: dist/telepad-<version>-windows-x86_64.zip + .sha256
sh scripts/package-unix.sh           # macOS / Linux: dist/telepad-<version>-<os>-<arch>.zip + .sha256
sh scripts/package-unix.sh x86_64-apple-darwin   # cross-build for another target
```

Each zip contains both programs, a `README.txt`, `config.example` and the licences. The
`<os>-<arch>` part must match `std::env::consts::{OS, ARCH}` of the build: that is what the
in-app updater looks for.

## Releasing (maintainers)

1. Bump `version` in `Cargo.toml` and add an entry to `CHANGE_HISTORY.md`.
2. Commit, push to `main`, wait for CI to pass.
3. Push a tag that matches the version:

   ```sh
   git tag v0.4.1
   git push origin v0.4.1
   ```

The **Release** workflow tests and builds every platform (`windows-x86_64`, `linux-x86_64`,
`linux-aarch64`, `macos-aarch64`, `macos-x86_64`), checks each zip, and publishes all of them
with their `.sha256` files in one release; running apps offer the update on their next check.
A pull request that changes the release files runs the same builds as a dry run.

## License of contributions

Unless you explicitly state otherwise, any contribution you submit for inclusion in this
work, as defined in the Apache-2.0 license, is dual licensed under MIT OR Apache-2.0,
without any additional terms or conditions.
