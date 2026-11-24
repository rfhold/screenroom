use adw::prelude::*;

mod assets;
mod backend;
mod cache;
mod home;
mod jellyfin;
mod playback;
mod poster;
mod profile_change;
mod search;
mod settings;
use screenroom_linux::{catalog, paging, playback_state, presentation, state};

fn main() -> gtk::glib::ExitCode {
    if !cfg!(debug_assertions) {
        screenroom_client_core::initialize_client_telemetry(
            "linux".into(),
            option_env!("SCREENROOM_TELEMETRY_ENVIRONMENT")
                .unwrap_or("production")
                .into(),
        );
    }
    let app = adw::Application::builder()
        .application_id("tv.screenroom")
        .build();

    app.connect_activate(|app| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        let css = gtk::CssProvider::new();
        css.load_from_data(include_str!("theme.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Screenroom")
            .default_width(1100)
            .default_height(720)
            .content(&home::view())
            .build();
        window.present();
        window.maximize();
    });

    app.run()
}
