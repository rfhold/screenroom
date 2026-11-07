use adw::prelude::*;

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("org.screenroom.linux")
        .build();

    app.connect_activate(|app| {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Screenroom")
            .default_width(800)
            .default_height(600)
            .content(&gtk::Label::new(Some("Screenroom")))
            .build();
        window.present();
    });

    app.run()
}
