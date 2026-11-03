#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod export;
mod host;
mod model;
mod session;
mod theme;

/// 릴리스 빌드는 콘솔 없이 뜨는 GUI 프로그램이므로, 터미널에서 부른 경우 그 터미널에 붙여 출력한다.
fn attach_parent_console() {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn AttachConsole(process_id: u32) -> i32;
        }
        // SAFETY: ATTACH_PARENT_PROCESS(-1)로 부모 콘솔에 붙는 단순 호출이며, 실패해도 무시한다.
        unsafe {
            AttachConsole(u32::MAX);
        }
    }
}

fn main() -> eframe::Result {
    if std::env::args().any(|v| v == "--version" || v == "-V") {
        attach_parent_console();
        println!("DOTAPE {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    if std::env::args().any(|v| v == "--self-test") {
        attach_parent_console();
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
