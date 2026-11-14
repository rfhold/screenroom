use adw::prelude::*;

mod assets;
mod backend;
mod cache;
mod catalog;
mod home;
mod jellyfin;
mod paging;
mod poster;
mod presentation;
mod profile_change;
mod search;
mod settings;
mod state;

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("org.screenroom.linux")
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
