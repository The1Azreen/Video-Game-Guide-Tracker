#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod checklist;
mod icons;
mod links;
mod model;
mod online;
mod theme;

const USAGE: &str = "\
jagt [options]

  --fresh       start with default UI settings and don't save them (library is still loaded)
  --settings    open the settings window at start
  --size WxH    initial window size
  --parse-checklist FILE   print the checklist parsed from FILE as JSON, then quit
  --html-to-text FILE      print the text that Fetch-from-guide-page reads from a saved page
  -h, --help    this text";

fn main() -> eframe::Result {
    let mut opts = app::Options::default();
    let mut size = [1280.0f32, 780.0];
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--fresh" => opts.fresh = true,
            "--settings" => opts.open_settings = true,
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = [w.parse().unwrap_or(size[0]), h.parse().unwrap_or(size[1])];
                }
            }
            "--parse-checklist" => {
                // Developer helper: print what a pasted guide turns into, as JSON.
                let text = args.next().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
                println!("{}", serde_json::to_string_pretty(&checklist::parse(&text)).unwrap_or_default());
                return Ok(());
            }
            "--html-to-text" => {
                // Developer helper: what "Fetch from guide page" reads from a saved page.
                let html = args.next().and_then(|f| std::fs::read(f).ok()).unwrap_or_default();
                println!("{}", online::html_to_text(&String::from_utf8_lossy(&html)));
                return Ok(());
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => {}
        }
    }

    let icon = {
        let img = image::load_from_memory(include_bytes!("../assets/icon.png")).map(|i| i.to_rgba8());
        img.ok().map(|i| egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() })
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("JAGT — Just Another Guide Tracker")
        .with_app_id("jagt")
        .with_inner_size(size)
        .with_min_inner_size([900.0, 560.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }

    let native = eframe::NativeOptions {
        viewport,
        persistence_path: model::local_dir().map(|d| d.join("settings.ron")),
        // eframe's own window restore re-opens maximized windows at the wrong UI scale on
        // mixed-DPI setups. The app restores size/maximized itself.
        persist_window: false,
        centered: true,
        ..Default::default()
    };
    eframe::run_native("jagt", native, Box::new(|cc| Ok(Box::new(app::App::new(cc, opts)))))
}
