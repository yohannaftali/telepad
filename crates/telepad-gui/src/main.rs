// No console window behind the GUI in release builds on Windows.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod icons;
mod runtime;
mod views;
mod widgets;

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut cfg = None;
    let mut open_ssh = None;
    let mut db = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg = args.next(),
            "--db" => db = args.next(),
            // Same code path as the GUI's "Open SSH" button, without the window.
            "--open-ssh" => open_ssh = args.next(),
            _ => {}
        }
    }
    if let Some(alias) = open_ssh {
        let code = telepad_core::open_ssh_alias(cfg.as_deref(), db.as_deref(), &alias);
        std::process::exit(if code.is_ok() { 0 } else { 1 });
    }
    app::run(cfg, db)
}
