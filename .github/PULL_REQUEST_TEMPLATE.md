## What changed

<!-- What and why. Link the issue: "Fixes #12". -->

## How it was tested

<!-- OS you ran it on, kinds of hosts (Docker host, NAS, VM, BusyBox, ...), GUI and/or telepad-check. -->

## Checklist

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] Nothing Telepad runs can change a remote host
- [ ] No ssh or network calls on the UI thread; OS-specific code only in `src/platform.rs`
- [ ] New metric: `script.sh` section + parser + fixture test + UI
- [ ] Tests/fixtures use documentation IPs (`192.0.2.x`) and made-up host names
- [ ] `docs/USER_MANUAL.md` updated if the UI or behaviour changed
