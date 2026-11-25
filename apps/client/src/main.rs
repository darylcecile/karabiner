mod api;
mod assets;
mod app;
mod components;
mod motion;
mod shell;
mod state;
mod theme;
mod views;

use gpui_kit::*;

fn main() {
    let api = match api::ApiClient::from_env() {
        Ok(api) => api,
        Err(error) => {
            eprintln!("failed to create HTTP client: {error:#}");
            std::process::exit(1);
        }
    };

    application().with_assets(assets::AppAssets).run(move |cx| {
        init(cx);
        theme::init(cx);
        app::bind_keys(cx);
        // Traffic-light centres (≈ y + 7) sit at y 20, the 40pt bar's midline shared by all chrome.
        let titlebar = TitlebarOptions {
            traffic_light_position: Some(point(px(14.), px(13.))),
            ..component::TitleBar::title_bar_options()
        };
        let options = WindowOptions {
            titlebar: Some(titlebar),
            window_min_size: Some(size(px(720.), px(480.))),
            window_bounds: Some(WindowBounds::centered(None, size(px(1280.), px(800.)), cx)),
            ..component::TitleBar::window_options()
        };
        if let Err(error) = open_window(options, cx, |window, cx| {
            cx.new(|cx| app::AppShell::new(api, window, cx))
        }) {
            eprintln!("failed to open window: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}
