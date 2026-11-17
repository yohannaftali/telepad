//! The eframe application: state + runtime, one view pass per frame.

use std::time::{Duration, Instant};

use eframe::egui;
use telepad_app::State;

use crate::runtime::Runtime;
use crate::{icons, views};

pub fn run(cli_config: Option<String>, cli_db: Option<String>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Telepad")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Telepad",
        options,
        Box::new(move |cc| Ok(Box::new(TelepadApp::new(cc, cli_config, cli_db)))),
    )
}

struct TelepadApp {
    state: State,
    runtime: Runtime,
}

impl TelepadApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        cli_config: Option<String>,
        cli_db: Option<String>,
    ) -> Self {
        icons::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut state = State::default();
        let runtime = Runtime::start(
            cc.egui_ctx.clone(),
            &mut state,
            cli_config.as_deref(),
            cli_db.as_deref(),
        );
        Self { state, runtime }
    }
}

impl eframe::App for TelepadApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.runtime.frame(&mut self.state);
        // Ages ("12s ago"), timers and spinners move without input.
        root.ctx().request_repaint_after(Duration::from_secs(1));

        let State { model, drafts } = &mut self.state;
        let msgs = views::draw(root, model, drafts, Instant::now());
        for msg in msgs {
            self.runtime.dispatch(&mut self.state, msg);
        }
    }
}
