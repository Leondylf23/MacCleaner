#[cfg(not(target_os = "macos"))]
compile_error!("MacCleaner only runs on macOS.");

mod app;
mod startup;
mod storage;
mod system;
mod ui;
mod util;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MacCleaner")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([960.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MacCleaner",
        options,
        Box::new(|cc| Ok(Box::new(app::MacCleanerApp::new(cc)))),
    )
}
