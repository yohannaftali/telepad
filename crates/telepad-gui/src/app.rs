//! The eframe application: state + runtime, one view pass per frame.

use std::time::{Duration, Instant};

use eframe::egui;
use telepad_app::State;

use crate::runtime::Runtime;
use crate::{icons, views};

pub fn run(
    cli_config: Option<String>,
    cli_db: Option<String>,
    cli_prefs: Option<String>,
    cli_history: Option<String>,
    cli_update_db: Option<String>,
) -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Telepad")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([900.0, 500.0]);
    // Window / taskbar / dock icon; a broken image just leaves the default icon.
    if let Ok(icon) =
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/telepad.png"))
    {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Telepad",
        options,
        Box::new(move |cc| {
            Ok(Box::new(TelepadApp::new(
                cc,
                cli_config,
                cli_db,
                cli_prefs,
                cli_history,
                cli_update_db,
            )))
        }),
    )
}

struct TelepadApp {
    state: State,
    runtime: Runtime,
    applied_theme: Option<telepad_core::store::ThemePref>,
}

impl TelepadApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        cli_config: Option<String>,
        cli_db: Option<String>,
        cli_prefs: Option<String>,
        cli_history: Option<String>,
        cli_update_db: Option<String>,
    ) -> Self {
        icons::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        cc.egui_ctx
            .set_visuals_of(egui::Theme::Light, crate::widgets::light_visuals());
        let mut state = State::default();
        let runtime = Runtime::start(
            cc.egui_ctx.clone(),
            &mut state,
            cli_config.as_deref(),
            cli_db.as_deref(),
            cli_prefs.as_deref(),
            cli_history.as_deref(),
            cli_update_db.as_deref(),
        );
        Self {
            state,
            runtime,
            applied_theme: None,
        }
    }
}

impl eframe::App for TelepadApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.runtime.flush_prefs(&self.state.model);
        self.runtime.flush_history();
        self.runtime.flush_strategy();
        self.runtime.shutdown();
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.runtime.frame(&mut self.state);
        let want = self.state.drafts.theme;
        if self.applied_theme != Some(want) {
            use telepad_core::store::ThemePref;
            root.ctx().set_theme(match want {
                ThemePref::System => egui::ThemePreference::System,
                ThemePref::Light => egui::ThemePreference::Light,
                ThemePref::Dark => egui::ThemePreference::Dark,
            });
            self.applied_theme = Some(want);
        }
        crate::widgets::set_dark_mode(root.style().visuals.dark_mode);
        // Ages ("12s ago"), timers and spinners move without input.
        root.ctx().request_repaint_after(Duration::from_secs(1));

        let State { model, drafts } = &mut self.state;
        let msgs = views::draw(root, model, drafts, Instant::now());
        for msg in msgs {
            self.runtime.dispatch(&mut self.state, msg);
        }
    }
}
