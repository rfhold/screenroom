use crate::{
    catalog::{self, Kind},
    poster,
    state::{Library, Screen, State},
};
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

pub struct App {
    pub state: RefCell<State>,
    pub client: Result<Arc<screenroom_client_core::Client>, screenroom_client_core::ClientError>,
    pub generation: Cell<u64>,
    pub loading: Cell<bool>,
    pub error: RefCell<Option<String>>,
    pub preferences: RefCell<screenroom_client_core::Preferences>,
    pub capabilities: RefCell<screenroom_client_core::SearchCapabilities>,
    preference_revision: Cell<u64>,
    preference_lock: Arc<tokio::sync::Mutex<()>>,
    pub search_task: RefCell<Option<tokio::task::AbortHandle>>,
    load_task: RefCell<Option<tokio::task::AbortHandle>>,
    pub search_results: RefCell<gtk::glib::WeakRef<gtk::Box>>,
    sidebar: gtk::ScrolledWindow,
    page: gtk::ScrolledWindow,
}

pub fn view() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.add_css_class("screenroom");
    let sidebar = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    sidebar.add_css_class("sidebar");
    let page = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    root.append(&sidebar);
    root.append(&page);
    let app = Rc::new(App {
        sidebar,
        page,
        state: RefCell::new(State::default()),
        client: crate::backend::client(),
        generation: Cell::new(0),
        loading: Cell::new(true),
        error: RefCell::new(None),
        preferences: RefCell::default(),
        capabilities: RefCell::default(),
        preference_revision: Cell::new(0),
        preference_lock: Arc::new(tokio::sync::Mutex::new(())),
        search_task: RefCell::default(),
        load_task: RefCell::default(),
        search_results: RefCell::new(gtk::glib::WeakRef::new()),
    });
    app.reload();
    // Keep state for the lifetime of the root without retaining it from child callbacks.
    root.connect_destroy(move |_| {
        app.cancel_load();
        app.cancel_search();
        if let Ok(client) = &app.client {
            client.cancel_all();
        }
        app.generation.set(app.generation.get() + 1);
    });
    root
}

pub fn label(text: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .wrap(true)
        .xalign(0.0)
        .css_classes([class])
        .build()
}
pub fn column(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
pub fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub fn accessible_label(widget: &impl IsA<gtk::Widget>, title: &str) {
    let title = title.to_owned();
    widget
        .as_ref()
        .update_property(&[gtk::accessible::Property::Label(&title)]);
    widget.as_ref().connect_map(move |widget| {
        widget.update_property(&[gtk::accessible::Property::Label(&title)]);
    });
}

fn labelled_by(widget: &impl IsA<gtk::Widget>, label: &gtk::Label) {
    widget
        .as_ref()
        .update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
}

impl App {
    fn default_library(&self) -> Library {
        let libraries = catalog::libraries();
        let pref = self.preferences.borrow();
        Library(
            libraries
                .iter()
                .find(|l| l.id == pref.library_id)
                .or(libraries.first())
                .map(|l| l.id.clone())
                .unwrap_or_default(),
        )
    }
    fn cancel_load(&self) {
        if let Some(task) = self.load_task.borrow_mut().take() {
            task.abort();
        }
    }
    pub fn reload(self: &Rc<Self>) {
        self.cancel_load();
        self.cancel_search();
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        let preference_revision = self.preference_revision.get();
        self.loading.set(true);
        self.error.replace(None);
        self.render();
        let client = self.client.clone();
        let task =
            crate::backend::runtime().spawn(async move { crate::backend::load(client?).await });
        self.load_task.replace(Some(task.abort_handle()));
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            let Some(app) = weak.upgrade() else {
                return;
            };
            if app.generation.get() != generation {
                return;
            }
            app.load_task.borrow_mut().take();
            app.loading.set(false);
            match result {
                Ok(Ok(snapshot)) => {
                    let previous_id = app.state.borrow().library.id();
                    catalog::install(snapshot.catalog);
                    crate::assets::install(snapshot.art);
                    app.capabilities.replace(snapshot.capabilities);
                    let pref = snapshot.preferences;
                    {
                        let libraries = catalog::libraries();
                        let selected = if previous_id.is_empty() {
                            &pref.library_id
                        } else {
                            &previous_id
                        };
                        app.state.borrow_mut().library = Library(
                            libraries
                                .iter()
                                .find(|l| &l.id == selected)
                                .or(libraries.first())
                                .map(|l| l.id.clone())
                                .unwrap_or_default(),
                        );
                    }
                    if app.preference_revision.get() == preference_revision {
                        let mut state = app.state.borrow_mut();
                        state.subtitle = match pref.subtitle_preference.as_str() {
                            "library_tracks" => 1,
                            "transcription" => 2,
                            _ => 0,
                        };
                        if let Some(types) = &pref.search_types {
                            state.types = [types.movie, types.series, types.episode];
                        }
                        if previous_id.is_empty() {
                            state.source = pref.source.clone();
                            if let Some(methods) = &pref.search_methods {
                                state.methods = [methods.text, methods.fuzzy, methods.semantic];
                                state.title_only = methods.title_only;
                            }
                        }
                        app.preferences.replace(pref);
                    }
                    app.state
                        .borrow_mut()
                        .apply_capabilities(&app.capabilities.borrow());
                }
                Ok(Err(error)) => {
                    app.error.replace(Some(error.to_string()));
                }
                Err(error) => {
                    app.error.replace(Some(error.to_string()));
                }
            }
            app.render();
            if app.state.borrow().screen == Screen::Search && !app.state.borrow().query.is_empty() {
                app.request_search();
            }
        });
    }
    pub fn save_preferences(self: &Rc<Self>, subtitle: bool) {
        let Ok(client) = self.client.clone() else {
            return;
        };
        let mut preferences = self.preferences.borrow().clone();
        self.preference_revision
            .set(self.preference_revision.get() + 1);
        let revision = self.preference_revision.get();
        let lock = self.preference_lock.clone();
        {
            let state = self.state.borrow();
            preferences.subtitle_preference =
                ["off", "library_tracks", "transcription"][state.subtitle].into();
            preferences.search_types = Some(screenroom_client_core::SearchTypes {
                movie: state.types[0],
                series: state.types[1],
                episode: state.types[2],
            });
        }
        let paths = vec![
            if subtitle {
                "subtitle_preference"
            } else {
                "search_types"
            }
            .into(),
        ];
        let task = crate::backend::runtime().spawn(async move {
            let _guard = lock.lock().await;
            let saved = client
                .clone()
                .update_preferences(preferences, paths)
                .await?;
            let read = client.get_preferences().await?;
            if saved != read {
                return Err(screenroom_client_core::ClientError::InvalidResponse {
                    reason: "preference readback mismatch".into(),
                });
            }
            eprintln!(
                "core UpdatePreferences/readback: subtitles={}, types={:?}",
                read.subtitle_preference, read.search_types
            );
            Ok(read)
        });
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            if let Some(app) = weak.upgrade() {
                match result {
                    Ok(Ok(pref)) => {
                        if app.preference_revision.get() == revision {
                            app.preferences.replace(pref);
                        }
                    }
                    other => {
                        if app.preference_revision.get() != revision {
                            return;
                        }
                        eprintln!("Preference update failed: {other:?}");
                        app.error
                            .replace(Some("Preference update failed. Retry Refresh.".into()));
                        app.render_page();
                    }
                }
            }
        });
    }
    pub fn button(
        self: &Rc<Self>,
        title: &str,
        action: impl Fn(&Rc<Self>) + 'static,
    ) -> gtk::Button {
        let button = gtk::Button::with_label(title);
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                action(&app);
            }
        });
        button
    }
    pub fn render(self: &Rc<Self>) {
        self.render_sidebar();
        self.render_page();
    }
    pub fn navigate(self: &Rc<Self>, screen: Screen, library: Library) {
        self.cancel_search();
        self.state.borrow_mut().navigate(screen, library);
        self.render();
    }
    fn render_sidebar(self: &Rc<Self>) {
        let state = self.state.borrow();
        let expanded = state.expanded;
        self.sidebar
            .set_width_request(if expanded { 264 } else { 64 });
        let content = column(6);
        content.add_css_class("rail");
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(10);
        content.set_margin_end(10);
        let toggle = self.button(
            if expanded {
                "Collapse sidebar"
            } else {
                "Expand sidebar"
            },
            |app| {
                {
                    let mut state = app.state.borrow_mut();
                    state.expanded = !state.expanded;
                }
                app.render_sidebar();
            },
        );
        let toggle_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toggle_content.append(&gtk::Image::from_icon_name("sidebar-show-symbolic"));
        let toggle_label = label(
            if expanded {
                "Collapse sidebar"
            } else {
                "Expand sidebar"
            },
            "accessible-only",
        );
        toggle_content.append(&toggle_label);
        toggle.set_child(Some(&toggle_content));
        labelled_by(&toggle, &toggle_label);
        accessible_label(
            &toggle,
            if expanded {
                "Collapse sidebar"
            } else {
                "Expand sidebar"
            },
        );
        toggle.set_tooltip_text(Some(if expanded {
            "Collapse sidebar"
        } else {
            "Expand sidebar"
        }));
        toggle.add_css_class("rail-toggle");
        content.append(&toggle);
        let mut navigation = vec![
            (
                "Home".to_owned(),
                "go-home-symbolic",
                Screen::Home,
                self.default_library(),
            ),
            (
                "Search titles".to_owned(),
                "system-search-symbolic",
                Screen::Search,
                state.library.clone(),
            ),
        ];
        navigation.extend(catalog::libraries().into_iter().map(|library| {
            (
                library.title,
                "view-grid-symbolic",
                Screen::Browse,
                Library(library.id),
            )
        }));
        for (index, (title, icon, screen, library)) in navigation.into_iter().enumerate() {
            if index == 2 && expanded {
                content.append(&label("Libraries", "muted"));
            }
            let target = library.clone();
            let button = self.button(&title, move |app| {
                if screen == Screen::Search {
                    app.cancel_search();
                    app.state.borrow_mut().fresh_search(target.clone());
                    app.state
                        .borrow_mut()
                        .apply_capabilities(&app.capabilities.borrow());
                    app.render();
                } else {
                    app.navigate(screen, target.clone());
                }
            });
            button.set_tooltip_text(Some(&title));
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            line.append(&gtk::Image::from_icon_name(icon));
            let nav_label = label(
                &title,
                if expanded {
                    "nav-label"
                } else {
                    "accessible-only"
                },
            );
            line.append(&nav_label);
            if !expanded {
                line.set_spacing(0);
            }
            button.set_child(Some(&line));
            labelled_by(&button, &nav_label);
            accessible_label(&button, &title);
            button.add_css_class("nav-button");
            if state.screen == screen && (screen != Screen::Browse || state.library == library) {
                button.add_css_class("selected");
            }
            content.append(&button);
        }
        if expanded {
            content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            content.append(&label("Subtitles", "muted"));
            let mut group: Option<gtk::CheckButton> = None;
            for (i, title) in ["Off", "Library tracks", "Transcription"]
                .into_iter()
                .enumerate()
            {
                let choice = gtk::CheckButton::with_label(title);
                if let Some(first) = &group {
                    choice.set_group(Some(first));
                } else {
                    group = Some(choice.clone());
                }
                choice.set_active(state.subtitle == i);
                let weak = Rc::downgrade(self);
                choice.connect_toggled(move |button| {
                    if button.is_active() {
                        if let Some(app) = weak.upgrade() {
                            app.state.borrow_mut().subtitle = i;
                            app.save_preferences(true);
                        }
                    }
                });
                content.append(&choice);
            }
            content.append(&self.button("Refresh", |app| {
                app.cancel_load();
                app.cancel_search();
                if let Ok(client) = &app.client {
                    client.invalidate_reads();
                }
                app.reload();
            }));
        }
        self.sidebar.set_child(Some(&content));
    }
    pub fn render_page(self: &Rc<Self>) {
        let content = column(28);
        content.add_css_class("page");
        content.set_margin_top(24);
        content.set_margin_bottom(40);
        content.set_margin_start(32);
        content.set_margin_end(32);
        let clamp = adw::Clamp::builder()
            .maximum_size(1120)
            .tightening_threshold(1120)
            .child(&content)
            .build();
        content.append(&self.breadcrumbs());
        if self.loading.get() || self.error.borrow().is_some() {
            if self.loading.get() && self.state.borrow().search_pending {
                content.append(&label("Searching titles…", "muted"));
            }
            content.append(&label(
                self.error
                    .borrow()
                    .as_deref()
                    .unwrap_or("Loading mock catalog…"),
                "muted",
            ));
            if !self.loading.get() {
                content.append(&self.button("Retry", |app| app.reload()));
            }
            self.page.set_child(Some(&clamp));
            return;
        }
        let items = catalog::catalog();
        let state = self.state.borrow();
        if let Some(id) = &state.detail {
            if let Some(index) = items.iter().position(|i| &i.id == id) {
                content.append(&self.detail(index));
            }
        } else {
            match state.screen {
                Screen::Search => {
                    drop(state);
                    content.append(&self.search());
                    content.append(&self.credits());
                    self.page.set_child(Some(&clamp));
                    self.page.vadjustment().set_value(0.0);
                    return;
                }
                Screen::Home => {
                    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 16);
                    let title = label("Home", "heading");
                    title.set_hexpand(true);
                    heading.append(&title);
                    heading.append(&self.button("Browse libraries", |app| {
                        app.navigate(Screen::Browse, app.default_library())
                    }));
                    content.append(&heading);
                    for (title, items) in catalog::home_rows(&state.library.id()) {
                        let section = column(16);
                        section.append(&label(&title, "row-heading"));
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 28);
                        row.add_css_class("poster-row");
                        row.set_margin_top(8);
                        row.set_margin_bottom(16);
                        row.set_margin_start(12);
                        row.set_margin_end(12);
                        for (id, progress) in items {
                            if let Some(index) = catalog::catalog().iter().position(|i| i.id == id)
                            {
                                row.append(&self.card(index, progress));
                            }
                        }
                        let scroll = gtk::ScrolledWindow::builder()
                            .hscrollbar_policy(gtk::PolicyType::Automatic)
                            .vscrollbar_policy(gtk::PolicyType::Never)
                            .overlay_scrolling(false)
                            .propagate_natural_height(true)
                            .min_content_height(340)
                            .child(&row)
                            .build();
                        section.append(&scroll);
                        content.append(&section);
                    }
                }
                Screen::Browse => {
                    content.append(&label(
                        &state
                            .path
                            .last()
                            .and_then(|id| {
                                items.iter().find(|i| &i.id == id).map(|i| i.title.clone())
                            })
                            .unwrap_or_else(|| state.library.label()),
                        "heading",
                    ));
                    if let Some(item) = state
                        .path
                        .last()
                        .and_then(|id| items.iter().find(|item| &item.id == id))
                        .filter(|item| item.kind == Kind::Season)
                    {
                        let rating = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                        let (score, info) = catalog::rating_labels(&item.metadata);
                        rating.append(&label(&score, "muted"));
                        rating.append(&label(&info, "muted"));
                        content.append(&rating);
                    }
                    let ids = if let Some(parent) = state.path.last() {
                        catalog::children(parent)
                    } else {
                        catalog::members(&state.library.id())
                    };
                    let indices = ids
                        .iter()
                        .filter_map(|id| items.iter().position(|item| &item.id == id))
                        .collect::<Vec<_>>();
                    content.append(&self.grid(&indices));
                }
            }
        }
        content.append(&self.credits());
        self.page.set_child(Some(&clamp));
        self.page.vadjustment().set_value(0.0);
    }
    fn breadcrumbs(self: &Rc<Self>) -> gtk::Box {
        let items = catalog::catalog();
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.add_css_class("breadcrumbs");
        accessible_label(&line, "Breadcrumb");
        line.append(&self.button("Home", |app| {
            app.navigate(Screen::Home, app.default_library())
        }));
        let state = self.state.borrow();
        if state.screen != Screen::Home {
            line.append(&label("›", "muted"));
            let library = state.library.clone();
            line.append(&self.button(&library.label(), move |app| {
                app.navigate(Screen::Browse, library.clone())
            }));
            for (depth, id) in state.path.iter().enumerate() {
                line.append(&label("›", "muted"));
                let title = items
                    .iter()
                    .find(|i| &i.id == id)
                    .map(|i| i.title.as_str())
                    .unwrap_or("");
                line.append(&self.button(title, move |app| {
                    {
                        let mut state = app.state.borrow_mut();
                        state.path.truncate(depth + 1);
                        state.detail = None;
                        state.screen = Screen::Browse;
                    }
                    app.render_page();
                }));
            }
        }
        if state.screen == Screen::Search || state.search_origin {
            line.append(&label("›", "muted"));
            line.append(&self.button("Search", |app| {
                {
                    let mut state = app.state.borrow_mut();
                    state.screen = Screen::Search;
                    state.path.clear();
                    state.detail = None;
                    state.search_origin = false;
                }
                app.render();
            }));
        }
        if let Some(id) = &state.detail {
            line.append(&label("›", "muted"));
            let current = label(
                items
                    .iter()
                    .find(|i| &i.id == id)
                    .map(|i| i.title.as_str())
                    .unwrap_or(""),
                "muted",
            );
            current.set_wrap(false);
            current.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&current);
        }
        line
    }
    pub fn card(self: &Rc<Self>, index: usize, progress: Option<u8>) -> gtk::Button {
        let items = catalog::catalog();
        let item = &items[index];
        let id = item.id.clone();
        let button = self.button(&item.title, move |app| {
            app.state.borrow_mut().open(&id, &catalog::catalog());
            app.render();
        });
        button.add_css_class("poster-card");
        button.set_height_request(328);
        let content = column(7);
        content.set_valign(gtk::Align::Start);
        content.set_width_request(160);
        content.append(&poster::view(item, 160, progress.or(item.progress)));
        let title = label(&item.title, "card-title");
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_lines(2);
        title.set_height_request(36);
        title.set_width_chars(18);
        title.set_max_width_chars(20);
        content.append(&title);
        let meta = label(
            &if item.kind == Kind::Episode {
                item.meta.clone()
            } else {
                format!("{} · {}", item.year, item.kind.label())
            },
            "card-meta",
        );
        meta.set_wrap(false);
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        meta.set_max_width_chars(20);
        meta.set_height_request(18);
        content.append(&meta);
        button.set_child(Some(&content));
        labelled_by(&button, &title);
        accessible_label(&button, &item.title);
        button
    }
    pub fn grid(self: &Rc<Self>, indices: &[usize]) -> gtk::FlowBox {
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .column_spacing(28)
            .row_spacing(24)
            .min_children_per_line(1)
            .max_children_per_line(8)
            .homogeneous(true)
            .hexpand(true)
            .valign(gtk::Align::Start)
            .build();
        for index in indices {
            let card = self.card(*index, None);
            card.set_halign(gtk::Align::Start);
            grid.insert(&card, -1);
        }
        grid
    }
    fn detail(&self, index: usize) -> gtk::Box {
        let items = catalog::catalog();
        let item = &items[index];
        let detail = column(28);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 28);
        let desktop_poster = poster::view(item, 200, item.progress);
        header.append(&desktop_poster);
        let text = column(16);
        text.set_hexpand(true);
        text.append(&label(
            &format!("{} · {}", item.kind.label(), item.year),
            "muted",
        ));
        text.append(&label(&item.title, "heading"));
        text.append(&label(&item.meta, "muted"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_halign(gtk::Align::Start);
        let (score, rating_info) = catalog::rating_labels(&item.metadata);
        let rating_provider = catalog::text(&item.metadata, "ratingProvider");
        let rating_url = item.metadata["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|reference| catalog::text(reference, "provider") == rating_provider)
            .map(|reference| catalog::text(reference, "url"))
            .unwrap_or("");
        if rating_url.is_empty() {
            row.append(&label(&format!("★ {score}"), "muted"));
        } else {
            row.append(&gtk::LinkButton::with_label(
                rating_url,
                &format!("★ {score}"),
            ));
        }
        for reference in item.metadata["providers"].as_array().into_iter().flatten() {
            let provider = catalog::text(reference, "provider");
            if provider == rating_provider && !rating_url.is_empty() {
                continue;
            }
            let status = catalog::text(reference, "status");
            let url = catalog::text(reference, "url");
            if url.is_empty() {
                let status_label = label(&format!("{provider} · {status}"), "muted");
                accessible_label(&status_label, &format!("{provider} {status}"));
                row.append(&status_label);
            } else {
                row.append(&gtk::LinkButton::with_label(
                    url,
                    &format!("{provider} · {status}"),
                ));
            }
        }
        let info = gtk::MenuButton::builder()
            .tooltip_text("Provider information")
            .build();
        let info_icon = gtk::DrawingArea::builder()
            .content_width(20)
            .content_height(20)
            .build();
        info_icon.set_draw_func(|_, cr, width, height| {
            cr.set_source_rgb(0.75, 0.75, 0.75);
            cr.set_line_width(1.5);
            let (x, y) = (width as f64 / 2.0, height as f64 / 2.0);
            cr.arc(x, y, 8.0, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            cr.arc(x, y - 4.0, 1.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.move_to(x, y - 1.0);
            cr.line_to(x, y + 5.0);
            let _ = cr.stroke();
        });
        info.set_child(Some(&info_icon));
        accessible_label(&info, "Provider information");
        let popover = gtk::Popover::new();
        let provider = label(
            &format!(
                "{}\nRelease date: {}\n{}",
                rating_info,
                catalog::text(&item.metadata, "date"),
                catalog::provider_info(&item.metadata)
            ),
            "muted",
        );
        provider.set_max_width_chars(48);
        popover.set_child(Some(&provider));
        info.set_popover(Some(&popover));
        row.append(&info);
        text.append(&row);
        let overview = label(&item.overview, "overview");
        overview.set_max_width_chars(90);
        text.append(&overview);
        for note in item.metadata["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n.as_str())
        {
            text.append(&label(note, "muted"));
        }
        header.append(&text);
        detail.append(&header);
        let phone_poster = poster::view(item, 160, item.progress);
        let header_weak = header.downgrade();
        let desktop_weak = desktop_poster.downgrade();
        let phone_weak = phone_poster.downgrade();
        header.prepend(&phone_poster);
        phone_poster.set_visible(false);
        detail.add_tick_callback(move |detail, _| {
            if let (Some(header), Some(desktop), Some(phone)) = (
                header_weak.upgrade(),
                desktop_weak.upgrade(),
                phone_weak.upgrade(),
            ) {
                let narrow = detail.width() < 640;
                header.set_orientation(if narrow {
                    gtk::Orientation::Vertical
                } else {
                    gtk::Orientation::Horizontal
                });
                desktop.set_visible(!narrow);
                phone.set_visible(narrow);
            }
            gtk::glib::ControlFlow::Continue
        });
        for (heading, fields) in [("Cast", vec!["cast"]), ("Crew", vec!["creators", "crew"])] {
            let people = column(12);
            people.append(&label(heading, "row-heading"));
            let line = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .min_children_per_line(1)
                .max_children_per_line(8)
                .column_spacing(20)
                .row_spacing(20)
                .homogeneous(true)
                .hexpand(true)
                .build();
            for person in fields
                .iter()
                .flat_map(|field| item.metadata[*field].as_array().into_iter().flatten())
            {
                let card = column(6);
                card.set_width_request(140);
                card.set_halign(gtk::Align::Start);
                card.set_valign(gtk::Align::Start);
                let art = catalog::person_art(person);
                if let Some(art) = &art {
                    card.append(&poster::picture(
                        catalog::text(&art, "file"),
                        catalog::text(&art, "label"),
                        140,
                        140,
                    ));
                } else {
                    let initials = label(
                        &catalog::text(person, "name")
                            .split_whitespace()
                            .filter_map(|s| s.chars().next())
                            .collect::<String>(),
                        "row-heading",
                    );
                    initials.set_size_request(140, 140);
                    initials.set_xalign(0.5);
                    initials.set_yalign(0.5);
                    initials.add_css_class("portrait-placeholder");
                    accessible_label(
                        &initials,
                        &format!("No portrait included for {}", catalog::text(person, "name")),
                    );
                    card.append(&initials);
                }
                let name = label(catalog::text(person, "name"), "card-title");
                name.set_max_width_chars(18);
                name.set_width_chars(16);
                name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                card.append(&name);
                let role = label(catalog::text(person, "role"), "muted");
                role.set_max_width_chars(18);
                role.set_width_chars(16);
                role.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                card.append(&role);
                if art
                    .as_ref()
                    .is_some_and(|art| catalog::text(art, "type") == "character still")
                {
                    card.append(&label("Character still", "card-meta"));
                }
                line.insert(&card, -1);
            }
            if line.first_child().is_none() {
                people.append(&label(
                    &format!(
                        "No {} credits listed in this snapshot.",
                        heading.to_lowercase()
                    ),
                    "muted",
                ));
            }
            people.append(&line);
            detail.append(&people);
        }
        let rights = column(12);
        rights.append(&label("Title rights", "row-heading"));
        rights.append(&label(catalog::text(&item.rights, "rights"), "card-title"));
        rights.append(&label(catalog::text(&item.rights, "note"), "muted"));
        rights.append(&label(catalog::text(&item.rights, "attribution"), "muted"));
        let links = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        links.append(&gtk::LinkButton::with_label(
            catalog::text(&item.rights, "source"),
            "Rights source",
        ));
        if !catalog::text(&item.rights, "license").is_empty() {
            links.append(&gtk::LinkButton::with_label(
                catalog::text(&item.rights, "license"),
                "Film license",
            ));
        }
        rights.append(&links);
        detail.append(&rights);
        detail.append(&metadata_rights(&item.metadata));
        detail
    }
    fn credits(&self) -> gtk::Expander {
        let credits = gtk::Expander::new(Some("Data & artwork credits"));
        let content = column(12);
        let info = catalog::info();
        for art in info.artwork.iter().filter(|art| art.kind == "logo") {
            content.append(&poster::fitted_picture(&art.url, &art.label, 88, 32, true));
        }
        content.append(&label(
            &format!("Catalog retrieved {}", info.retrieved_at),
            "muted",
        ));
        let mut section = String::new();
        for notice in info.notices {
            if section != notice.section {
                section = notice.section;
                content.append(&label(&section, "row-heading"));
            }
            content.append(&label(&notice.title, "card-title"));
            content.append(&label(&notice.text, "muted"));
            if !notice.link_url.is_empty() {
                content.append(&gtk::LinkButton::with_label(
                    &notice.link_url,
                    &notice.link_label,
                ));
            }
        }
        for art in catalog::artwork() {
            content.append(&label(
                &format!(
                    "{}\n{}\n{}",
                    catalog::text(&art, "label"),
                    catalog::text(&art, "creator"),
                    catalog::text(&art, "license")
                ),
                "muted",
            ));
            for (key, title) in [
                ("licenseUrl", "Artwork license"),
                ("source", "Source & provenance"),
            ] {
                content.append(&gtk::LinkButton::with_label(
                    catalog::text(&art, key),
                    title,
                ));
            }
        }
        for item in catalog::catalog() {
            content.append(&label(
                &format!(
                    "{} · {}\n{}\n{}",
                    item.title,
                    catalog::text(&item.rights, "rights"),
                    catalog::text(&item.rights, "note"),
                    catalog::text(&item.rights, "attribution")
                ),
                "muted",
            ));
            content.append(&gtk::LinkButton::with_label(
                catalog::text(&item.rights, "source"),
                "Rights source",
            ));
            if !catalog::text(&item.rights, "license").is_empty() {
                content.append(&gtk::LinkButton::with_label(
                    catalog::text(&item.rights, "license"),
                    "Film license",
                ));
            }
            content.append(&metadata_rights(&item.metadata));
            for note in item.metadata["notes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str())
            {
                content.append(&label(note, "muted"));
            }
        }
        credits.set_child(Some(&content));
        credits
    }
}
fn metadata_rights(metadata: &serde_json::Value) -> gtk::Box {
    let rights = &metadata["rights"];
    let content = column(6);
    content.append(&label("Metadata rights", "card-title"));
    for key in ["label", "note", "attribution"] {
        let value = catalog::text(rights, key);
        if !value.is_empty() {
            content.append(&label(value, "muted"));
        }
    }
    let retrieved = catalog::text(metadata, "metadataRetrievedAt");
    if !retrieved.is_empty() {
        content.append(&label(&format!("Metadata retrieved {retrieved}"), "muted"));
    }
    for (key, title) in [
        ("source", "Metadata source"),
        ("license_url", "Metadata license"),
    ] {
        let url = catalog::text(rights, key);
        if !url.is_empty() {
            content.append(&gtk::LinkButton::with_label(url, title));
        }
    }
    content
}
