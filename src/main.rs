#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod export;
mod model;
mod session;
mod theme;

fn main() -> eframe::Result {
    if std::env::args().any(|v| v == "--self-test") {
        match export::self_test() {
            Ok(report) => {
                let root = std::env::var_os("SORI_TEST_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::temp_dir().join("Sori-self-test"));
                let _ = std::fs::write(root.join("self-test.txt"), &report);
                println!("{report}");
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("DOTAPE")
            .with_inner_size([1180.0, 820.0])
            .with_min_inner_size([940.0, 680.0])
            .with_icon(eframe::egui::IconData::default()),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "DOTAPE",
        options,
        Box::new(|cc| Ok(Box::new(app::SoriApp::new(cc)))),
    )
}
