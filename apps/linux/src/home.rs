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

fn should_resume_search(state: &State) -> bool {
    state.screen == Screen::Search && state.detail.is_none() && !state.query.is_empty()
}

#[cfg(test)]
#[path = "home_artwork_lifecycle_tests.rs"]
mod artwork_lifecycle_tests;

pub struct App {
    pub state: RefCell<State>,
    pub client: Result<Arc<screenroom_client_core::Client>, screenroom_client_core::ClientError>,
    pub generation: crate::paging::Generation,
    pub loading: Cell<bool>,
    pub error: RefCell<Option<String>>,
    settings_warning: RefCell<Option<String>>,
    settings_notice: gtk::Box,
    pub preferences: RefCell<screenroom_client_core::ClientSettings>,
    pub capabilities: RefCell<screenroom_client_core::SearchCapabilities>,
    preference_revision: Cell<u64>,
    pub search_task: RefCell<Option<tokio::task::AbortHandle>>,
    load_task: RefCell<Option<tokio::task::AbortHandle>>,
    pub search_results: RefCell<gtk::glib::WeakRef<gtk::Box>>,
    pub search_count: RefCell<gtk::glib::WeakRef<gtk::Label>>,
    pub browse_paging: RefCell<crate::paging::Paging>,
    pub search_paging: RefCell<crate::paging::Paging>,
    pub browse_grid: RefCell<gtk::glib::WeakRef<gtk::FlowBox>>,
    pub search_grid: RefCell<gtk::glib::WeakRef<gtk::FlowBox>>,
    pub browse_controls: RefCell<gtk::glib::WeakRef<gtk::Box>>,
    pub search_controls: RefCell<gtk::glib::WeakRef<gtk::Box>>,
    artwork_task: RefCell<Option<tokio::task::AbortHandle>>,
    artwork_revision: Cell<u64>,
    pub profile: RefCell<crate::jellyfin::ProfileState>,
    pub profile_panel: gtk::Box,
    pub home_dirty: Cell<bool>,
    home_retry_task: RefCell<Option<tokio::task::AbortHandle>>,
    sidebar: gtk::ScrolledWindow,
    page: gtk::ScrolledWindow,
    history: RefCell<std::collections::VecDeque<HistoryEntry>>,
    pub player: RefCell<Option<Rc<crate::playback::Player>>>,
    playback_error: RefCell<Option<String>>,
    playback_notice: gtk::Label,
}

struct HistoryEntry {
    state: State,
    catalog: catalog::Catalog,
    browse: crate::paging::Paging,
    search: crate::paging::Paging,
    scroll: f64,
    focus: String,
}

pub fn view() -> gtk::Box {
    let (preferences, settings_warning) = crate::settings::load();
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
        generation: crate::paging::Generation::default(),
        loading: Cell::new(true),
        error: RefCell::new(None),
        settings_warning: RefCell::new(settings_warning),
        settings_notice: column(6),
        preferences: RefCell::new(preferences),
        capabilities: RefCell::default(),
        preference_revision: Cell::new(0),
        search_task: RefCell::default(),
        load_task: RefCell::default(),
        search_results: RefCell::new(gtk::glib::WeakRef::new()),
        search_count: RefCell::default(),
        browse_paging: RefCell::default(),
        search_paging: RefCell::default(),
        browse_grid: RefCell::default(),
        search_grid: RefCell::default(),
        browse_controls: RefCell::default(),
        search_controls: RefCell::default(),
        artwork_task: RefCell::default(),
        artwork_revision: Cell::new(0),
        profile: RefCell::default(),
        profile_panel: column(8),
        home_dirty: Cell::new(false),
        home_retry_task: RefCell::default(),
        history: RefCell::default(),
        player: RefCell::default(),
        playback_error: RefCell::default(),
        playback_notice: label("", "muted"),
    });
    {
        let pref = app.preferences.borrow();
        let mut state = app.state.borrow_mut();
        state.library = Library(
            pref.scope
                .as_ref()
                .map(|s| s.library_id.clone())
                .unwrap_or_default(),
        );
        state.source = pref.source.clone();
        if let Some(types) = &pref.types {
            state.types = [types.movie, types.series, types.episode];
        }
        if let Some(methods) = &pref.methods {
            state.methods = [methods.text, methods.fuzzy, methods.semantic];
            state.title_only = methods.title_only;
        }
    }
    app.reload();
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&app);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gtk::gdk::Key::Left && modifiers.contains(gtk::gdk::ModifierType::ALT_MASK) {
            if let Some(app) = weak.upgrade() {
                app.back();
            }
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    root.add_controller(keys);
    // Keep state for the lifetime of the root without retaining it from child callbacks.
    root.connect_destroy(move |_| {
        app.stop_playback("app_disposed");
        app.cancel_load();
        app.cancel_search();
        app.cancel_artwork();
        app.profile.borrow_mut().cancel();
        if let Ok(client) = &app.client {
            client.cancel_all();
        }
        app.generation.advance();
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

fn capture_item_description(item_id: &str) -> Option<String> {
    if std::env::var("SCREENROOM_E2E_ACCESSIBILITY_IDS")
        .ok()
        .as_deref()
        != Some("1")
    {
        return None;
    }
    Some(format!("screenroom-item:{item_id}"))
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
    pub fn stop_playback(&self, reason: &str) {
        if let Some(player) = self.player.borrow().as_ref() {
            player.close(reason);
        }
    }
    pub fn clear_history(&self) {
        self.history.borrow_mut().clear();
    }
    fn remember(&self) {
        if self.loading.get() || self.error.borrow().is_some() {
            return;
        }
        let data = catalog::snapshot();
        let focus = self
            .page
            .root()
            .and_then(|root| root.focus())
            .map(|widget| widget.widget_name().to_string())
            .unwrap_or_default();
        let mut browse = self.browse_paging.borrow().clone();
        let mut search = self.search_paging.borrow().clone();
        browse.busy = false;
        search.busy = false;
        let mut history = self.history.borrow_mut();
        history.push_back(HistoryEntry {
            state: self.state.borrow().clone(),
            catalog: data,
            browse,
            search,
            scroll: self.page.vadjustment().value(),
            focus,
        });
        while history.len() > 8 {
            history.pop_front();
        }
    }
    fn back(self: &Rc<Self>) {
        let Some(entry) = self.history.borrow_mut().pop_back() else {
            return;
        };
        self.stop_playback("navigation");
        self.cancel_load();
        self.cancel_search();
        self.cancel_artwork();
        self.suspend_profile();
        self.generation.advance();
        self.loading.set(false);
        self.error.replace(None);
        self.state.borrow_mut().restore(entry.state);
        self.browse_paging.replace(entry.browse);
        self.search_paging.replace(entry.search);
        catalog::install(entry.catalog);
        if self.home_dirty.get() && self.state.borrow().screen == Screen::Home {
            self.reload();
        } else {
            self.render();
            self.request_artwork();
        }
        let frames = Cell::new(0);
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        self.page.add_tick_callback(move |page, _| {
            let Some(app) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if !app.generation.is_current(generation) {
                return gtk::glib::ControlFlow::Break;
            }
            if app.loading.get() {
                return gtk::glib::ControlFlow::Continue;
            }
            frames.set(frames.get() + 1);
            if frames.get() < 2 {
                return gtk::glib::ControlFlow::Continue;
            }
            fn find(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
                if widget.widget_name() == name && widget.is_focusable() {
                    return Some(widget.clone());
                }
                let mut child = widget.first_child();
                while let Some(current) = child {
                    if let Some(found) = find(&current, name) {
                        return Some(found);
                    }
                    child = current.next_sibling();
                }
                None
            }
            if let Some(widget) = find(page.upcast_ref(), &entry.focus) {
                widget.grab_focus();
            }
            page.vadjustment().set_value(entry.scroll);
            gtk::glib::ControlFlow::Break
        });
        if should_resume_search(&self.state.borrow()) && self.state.borrow().search_pending {
            self.request_search();
        }
    }
    fn play_item(self: &Rc<Self>, id: String, title: String, beginning: bool) {
        self.stop_playback("replacement");
        self.player.borrow_mut().take();
        let Ok(client) = self.client.clone() else {
            return;
        };
        let Some(parent) = self.page.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let weak = Rc::downgrade(self);
        self.playback_notice.set_text("");
        match crate::playback::Player::open(
            &parent,
            client,
            id,
            &title,
            self.scope(),
            beginning,
            move |message| {
                if let Some(app) = weak.upgrade() {
                    app.home_dirty.set(true);
                    app.playback_notice.set_text(message);
                    // Keep the visible details stable; the next catalog read observes live personal state.
                }
            },
        ) {
            Ok(player) => {
                self.player.replace(Some(player));
                self.playback_error.replace(None);
            }
            Err(message) => {
                self.playback_error.replace(Some(message.into()));
                self.render_page();
            }
        }
    }
    fn default_library(&self) -> Library {
        let libraries = catalog::libraries();
        let pref = self.preferences.borrow();
        if pref
            .scope
            .as_ref()
            .is_none_or(|scope| scope.library_id.is_empty())
        {
            return Library::default();
        }
        Library(
            libraries
                .iter()
                .find(|l| Some(&l.id) == pref.scope.as_ref().map(|s| &s.library_id))
                .or(libraries.first())
                .map(|l| l.id.clone())
                .unwrap_or_default(),
        )
    }
    fn cancel_load(&self) {
        if let Some(task) = self.load_task.borrow_mut().take() {
            task.abort();
        }
        if let Some(task) = self.home_retry_task.borrow_mut().take() {
            task.abort();
        }
    }
    pub fn reload(self: &Rc<Self>) {
        self.suspend_profile();
        self.cancel_load();
        self.cancel_search();
        self.cancel_artwork();
        if self.state.borrow().screen == Screen::Home && self.home_dirty.get() {
            if let Ok(client) = &self.client {
                client.invalidate_reads();
            }
        }
        self.browse_paging.replace(crate::paging::Paging::default());
        self.generation.advance();
        let generation = self.generation.get();
        self.loading.set(true);
        self.error.replace(None);
        self.render();
        let client = self.client.clone();
        let (scope, screen, target, detail) = {
            let state = self.state.borrow();
            (
                self.scope(),
                state.screen,
                state.detail.clone().or_else(|| state.path.last().cloned()),
                state.detail.is_some(),
            )
        };
        let task = crate::backend::runtime().spawn(async move {
            crate::backend::load(client?, scope, screen, target, detail).await
        });
        self.load_task.replace(Some(task.abort_handle()));
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            let Some(app) = weak.upgrade() else {
                return;
            };
            if !app.generation.is_current(generation) {
                return;
            }
            app.load_task.borrow_mut().take();
            app.loading.set(false);
            match result {
                Ok(Ok(snapshot)) => {
                    let state = app.state.borrow();
                    let ids = if let Some(parent) = state.path.last() {
                        snapshot.catalog.children.get(parent)
                    } else {
                        snapshot.catalog.members.get(&state.library.id())
                    }
                    .cloned()
                    .unwrap_or_default();
                    let accepted = app.browse_paging.borrow_mut().accept(ids, snapshot.next);
                    drop(state);
                    if accepted.is_err() {
                        app.error.replace(Some(
                            "Catalog exceeded the item limit. Retry to read the server state."
                                .into(),
                        ));
                        app.render();
                        return;
                    }
                    catalog::install(snapshot.catalog);
                    if app.state.borrow().screen == Screen::Home {
                        app.home_dirty.set(false);
                    }
                    crate::assets::install(snapshot.art);
                    app.capabilities.replace(snapshot.capabilities);
                    app.state
                        .borrow_mut()
                        .apply_capabilities(&app.capabilities.borrow());
                }
                Ok(Err(error)) => {
                    eprintln!("Catalog read failed: {error}");
                    app.error.replace(Some(
                        "Catalog could not be loaded. Retry to read the server state.".into(),
                    ));
                }
                Err(error) => {
                    eprintln!("Catalog task failed: {error}");
                    app.error.replace(Some(
                        "Catalog could not be loaded. Retry to read the server state.".into(),
                    ));
                }
            }
            app.render();
            app.request_artwork();
            if should_resume_search(&app.state.borrow()) {
                app.request_search();
            }
        });
    }
    pub fn cancel_artwork(&self) {
        self.artwork_revision.set(self.artwork_revision.get() + 1);
        if let Some(task) = self.artwork_task.borrow_mut().take() {
            task.abort();
        }
    }
    pub fn request_artwork(self: &Rc<Self>) {
        self.cancel_artwork();
        let revision = self.artwork_revision.get();
        let Ok(client) = self.client.clone() else {
            return;
        };
        let data = crate::catalog::Catalog {
            items: catalog::catalog(),
            ..Default::default()
        };
        let mut seen = std::collections::HashSet::new();
        let records = crate::backend::artwork_records(&data)
            .into_iter()
            .filter(|r| {
                let url = catalog::text(r, "file");
                poster::needs_artwork(url)
                    && !crate::assets::contains(url)
                    && seen.insert(url.to_owned())
            })
            .collect::<Vec<_>>();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let task = crate::backend::runtime().spawn(crate::backend::operation(async move {
            for record in records {
                let art = crate::backend::artwork(client.clone(), record).await?;
                if sender.send(art).await.is_err() {
                    break;
                }
            }
            Ok(())
        }));
        self.artwork_task.replace(Some(task.abort_handle()));
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            while let Some(art) = receiver.recv().await {
                let Some(app) = weak.upgrade() else {
                    break;
                };
                if app.artwork_revision.get() != revision {
                    break;
                }
                crate::assets::merge(art);
            }
        });
    }
    pub fn page_controls(self: &Rc<Self>, search: bool) -> gtk::Box {
        let controls = column(8);
        controls.append(&label("", "muted"));
        controls.append(&label("", "muted"));
        controls.append(&self.button("Load more", move |app| {
            if search {
                app.request_search_more();
            } else {
                app.load_more();
            }
        }));
        if search {
            self.search_controls.replace(controls.downgrade());
        } else {
            self.browse_controls.replace(controls.downgrade());
        }
        self.update_page_controls(search);
        controls
    }
    pub fn update_page_controls(self: &Rc<Self>, search: bool) {
        let weak = if search {
            &self.search_controls
        } else {
            &self.browse_controls
        };
        let Some(controls) = weak.borrow().upgrade() else {
            return;
        };
        let paging = if search {
            self.search_paging.borrow()
        } else {
            self.browse_paging.borrow()
        };
        let Some(count) = controls.first_child().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(status) = count.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(button) = status.next_sibling().and_downcast::<gtk::Button>() else {
            return;
        };
        count.set_text(&format!("{} loaded", paging.ids.len()));
        status.set_text(if paging.failed { "More items could not be loaded. Loaded items are still available." } else if paging.stopped { "Loading stopped because the server returned an invalid continuation or exceeded the page limit." } else { "" });
        status.set_visible(paging.failed || paging.stopped);
        button.set_label(if paging.failed {
            "Retry loading more"
        } else if paging.busy {
            "Loading more…"
        } else {
            "Load more"
        });
        button.set_visible(!paging.next.is_empty());
    }
    fn load_more(self: &Rc<Self>) {
        let Some(token) = self.browse_paging.borrow_mut().begin() else {
            return;
        };
        let generation = self.generation.get();
        let parent = self.state.borrow().path.last().cloned();
        let scope = self.scope();
        let client = self.client.clone();
        self.update_page_controls(false);
        let task = crate::backend::runtime()
            .spawn(async move { crate::backend::item_page(client?, scope, parent, token).await });
        self.load_task.replace(Some(task.abort_handle()));
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            let Some(app) = weak.upgrade() else {
                return;
            };
            if !app.generation.is_current(generation) {
                return;
            }
            match result {
                Ok(Ok(page)) => {
                    let accepted = app.browse_paging.borrow_mut().accept(
                        page.items.iter().map(|i| i.id.clone()),
                        page.next_page_token,
                    );
                    if let Ok(ids) = accepted {
                        let state = app.state.borrow();
                        catalog::append_page(
                            &state.library.id(),
                            state.path.last().map(String::as_str),
                            ids.clone(),
                            page.items,
                        );
                        drop(state);
                        app.append_cards(false, &ids);
                        app.request_artwork();
                    }
                }
                _ => app.browse_paging.borrow_mut().fail(),
            }
            app.update_page_controls(false);
        });
    }
    pub fn append_cards(self: &Rc<Self>, search: bool, ids: &[String]) {
        let weak = if search {
            &self.search_grid
        } else {
            &self.browse_grid
        };
        if let Some(grid) = weak.borrow().upgrade() {
            let items = catalog::catalog();
            for id in ids {
                if let Some(index) = items.iter().position(|i| &i.id == id) {
                    let card = self.card(index, None);
                    card.set_halign(gtk::Align::Start);
                    grid.insert(&card, -1);
                }
            }
        }
    }
    pub fn scope(&self) -> screenroom_client_core::SourceLibraryScope {
        screenroom_client_core::SourceLibraryScope {
            source_id: self
                .preferences
                .borrow()
                .scope
                .as_ref()
                .map(|s| s.source_id.clone())
                .unwrap_or_default(),
            library_id: self.state.borrow().library.id(),
        }
    }
    pub fn retry_home(self: &Rc<Self>) {
        if self.home_retry_task.borrow().is_some() {
            return;
        }
        let Ok(client) = self.client.clone() else {
            return;
        };
        let generation = self.generation.get();
        let scope = self.scope();
        let task = crate::backend::runtime().spawn(crate::backend::operation(async move {
            client.invalidate_reads();
            client.get_home(scope).await
        }));
        self.home_retry_task.replace(Some(task.abort_handle()));
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            let Some(app) = weak.upgrade() else {
                return;
            };
            if !app.generation.is_current(generation) {
                return;
            }
            app.home_retry_task.borrow_mut().take();
            if let Ok(Ok(rows)) = result {
                app.home_dirty.set(false);
                catalog::replace_home(&app.state.borrow().library.id(), rows);
                let scroll = app.page.vadjustment().value();
                app.render_page();
                app.page.vadjustment().set_value(scroll);
                app.request_artwork();
            }
        });
    }
    pub fn save_preferences(self: &Rc<Self>) {
        let mut preferences = self.preferences.borrow().clone();
        self.preference_revision
            .set(self.preference_revision.get() + 1);
        {
            let state = self.state.borrow();
            preferences.types = Some(screenroom_client_core::SearchTypes {
                movie: state.types[0],
                series: state.types[1],
                episode: state.types[2],
            });
            preferences.methods = Some(screenroom_client_core::SearchMethods {
                text: state.methods[0],
                fuzzy: state.methods[1],
                semantic: state.methods[2],
                title_only: state.title_only,
            });
            preferences.source = state.source.clone();
            preferences.scope = Some(self.scope());
        }
        match screenroom_client_core::validate_settings(preferences) {
            Ok(preferences) => {
                self.preferences.replace(preferences);
                self.persist_preferences();
            }
            Err(error) => {
                self.settings_warning.replace(Some(error.to_string()));
                self.render_settings_notice();
            }
        }
    }
    fn persist_preferences(self: &Rc<Self>) {
        self.settings_warning.replace(
            crate::settings::save(self.preferences.borrow().clone())
                .err()
                .map(|error| {
                    format!(
                        "Local settings not saved: {error}. Choices remain active in this session."
                    )
                }),
        );
        self.render_settings_notice();
    }
    fn render_settings_notice(self: &Rc<Self>) {
        clear(&self.settings_notice);
        if let Some(warning) = self.settings_warning.borrow().as_deref() {
            self.settings_notice.append(&label(warning, "muted"));
            self.settings_notice
                .append(&self.button("Retry saving settings", |app| app.persist_preferences()));
        }
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
        self.remember();
        self.stop_playback("navigation");
        self.cancel_search();
        self.state.borrow_mut().navigate(screen, library);
        self.save_preferences();
        self.reload();
    }
    fn render_sidebar(self: &Rc<Self>) {
        let state = self.state.borrow();
        let expanded = state.expanded;
        if !expanded {
            self.suspend_profile();
        }
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
                Library::default(),
            ),
            (
                "Search titles".to_owned(),
                "system-search-symbolic",
                Screen::Search,
                state.library.clone(),
            ),
        ];
        navigation.push((
            "All libraries".into(),
            "view-grid-symbolic",
            Screen::Browse,
            Library::default(),
        ));
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
                    app.remember();
                    app.cancel_search();
                    app.state.borrow_mut().fresh_search(target.clone());
                    app.state
                        .borrow_mut()
                        .apply_capabilities(&app.capabilities.borrow());
                    app.reload();
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
            content.append(&self.button("Refresh", |app| {
                app.cancel_load();
                app.cancel_search();
                if let Ok(client) = &app.client {
                    client.invalidate_reads();
                }
                app.reload();
            }));
            if let Some(parent) = self.profile_panel.parent().and_downcast::<gtk::Box>() {
                parent.remove(&self.profile_panel);
            }
            content.append(&self.profile_panel);
            self.render_profile();
            self.ensure_profile();
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
        if let Some(parent) = self.playback_notice.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.playback_notice);
        }
        content.append(&self.playback_notice);
        if let Some(message) = self.playback_error.borrow().as_deref() {
            content.append(&label(message, "muted"));
        }
        if let Some(parent) = self.settings_notice.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.settings_notice);
        }
        content.append(&self.settings_notice);
        self.render_settings_notice();
        if self.loading.get() || self.error.borrow().is_some() {
            if self.loading.get() && self.state.borrow().search_pending {
                content.append(&label("Searching titles…", "muted"));
            }
            content.append(&label(
                self.error.borrow().as_deref().unwrap_or("Loading catalog…"),
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
                    if catalog::home_rows(&state.library.id()).is_empty() {
                        content.append(&label("No titles are available on Home.", "muted"));
                    }
                    for (title, items) in catalog::home_rows(&state.library.id()) {
                        let section = column(16);
                        section.append(&label(&title, "row-heading"));
                        if items.is_empty() && catalog::home_error(&title).is_none() {
                            section.append(&label("No titles in this row.", "muted"));
                        }
                        if let Some(error) = catalog::home_error(&title) {
                            section.append(&label(&error, "muted"));
                            section.append(&self.button("Retry row", |app| app.retry_home()));
                        }
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
                    if indices.is_empty() {
                        content.append(&label("No titles are available in this view.", "muted"));
                    }
                    let grid = self.grid(&indices);
                    self.browse_grid.replace(grid.downgrade());
                    content.append(&grid);
                    content.append(&self.page_controls(false));
                }
            }
        }
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
        if !self.history.borrow().is_empty() {
            line.prepend(&self.button("Back", |app| app.back()));
        }
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
                    app.remember();
                    {
                        let mut state = app.state.borrow_mut();
                        state.path.truncate(depth + 1);
                        state.detail = None;
                        state.screen = Screen::Browse;
                    }
                    app.reload();
                }));
            }
        }
        if state.screen == Screen::Search || state.search_origin {
            line.append(&label("›", "muted"));
            line.append(&self.button("Search", |app| {
                app.remember();
                {
                    let mut state = app.state.borrow_mut();
                    state.screen = Screen::Search;
                    state.path.clear();
                    state.detail = None;
                    state.search_origin = false;
                }
                app.reload();
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
        let state = self.state.borrow();
        let parent = (state.screen == Screen::Browse)
            .then(|| state.path.last())
            .flatten();
        let season_context = crate::presentation::season_context(
            item.parent.as_deref(),
            parent.map(String::as_str),
            parent.is_some_and(|parent| {
                items
                    .iter()
                    .any(|i| &i.id == parent && i.kind == Kind::Season)
            }),
        );
        drop(state);
        let episode = (item.kind == Kind::Episode)
            .then(|| crate::presentation::episode_labels(&item.meta, season_context));
        let id = item.id.clone();
        let button = self.button(&item.title, move |app| {
            app.remember();
            app.state.borrow_mut().open(&id, &catalog::catalog());
            app.reload();
        });
        button.add_css_class("poster-card");
        button.set_widget_name(&format!("title-{}", item.id));
        if let Some(description) = capture_item_description(&item.id) {
            button.update_property(&[gtk::accessible::Property::Description(&description)]);
        }
        button.set_height_request(328);
        let content = column(7);
        content.set_valign(gtk::Align::Start);
        content.set_width_request(160);
        let poster = poster::view_context(
            item,
            160,
            progress.or(item.progress),
            false,
            episode.as_ref().map(|labels| labels.heading()),
        );
        content.append(&poster);
        let title = label(
            episode
                .as_ref()
                .map(|l| l.primary.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(if episode.is_some() { "" } else { &item.title }),
            "card-title",
        );
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_lines(2);
        title.set_height_request(36);
        title.set_width_chars(18);
        title.set_max_width_chars(20);
        if episode
            .as_ref()
            .is_none_or(|labels| !labels.primary.is_empty())
        {
            content.append(&title);
        }
        if let Some(labels) = &episode {
            if !labels.number.is_empty() {
                content.append(&label(&labels.number, "card-meta"));
            }
        }
        let meta = label(
            &if item.kind == Kind::Episode {
                item.title.clone()
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
        let accessibility = if let Some(labels) = episode {
            [
                labels.heading(),
                item.title.clone(),
                item.playback_accessibility(progress.or(item.progress)),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
        } else {
            item.card_accessibility(progress.or(item.progress))
        };
        button.reset_relation(gtk::AccessibleRelation::LabelledBy);
        accessible_label(&button, &accessibility);
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
    fn detail(self: &Rc<Self>, index: usize) -> gtk::Box {
        let items = catalog::catalog();
        let item = &items[index];
        let detail = column(28);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 28);
        let desktop_poster = poster::view(item, 200, item.progress, true);
        header.append(&desktop_poster);
        let text = column(16);
        text.set_hexpand(true);
        text.append(&label(
            &if item.year == 0 {
                item.kind.label().to_owned()
            } else {
                format!("{} · {}", item.kind.label(), item.year)
            },
            "muted",
        ));
        text.append(&label(&item.title, "heading"));
        text.append(&label(&item.meta, "muted"));
        if item.kind == Kind::Episode && !catalog::text(&item.metadata, "date").is_empty() {
            text.append(&label(
                &format!("Aired {}", catalog::text(&item.metadata, "date")),
                "muted",
            ));
        }
        let playback = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let id = item.id.clone();
        let title = item.title.clone();
        playback.append(&self.button("Play", move |app| {
            app.play_item(id.clone(), title.clone(), false)
        }));
        let id = item.id.clone();
        let title = item.title.clone();
        playback.append(&self.button("Play from beginning", move |app| {
            app.play_item(id.clone(), title.clone(), true)
        }));
        text.append(&playback);
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
        if !crate::presentation::safe_external_url(rating_url) {
            row.append(&label(&format!("★ {score}"), "muted"));
        } else {
            row.append(&gtk::LinkButton::with_label(
                rating_url,
                &format!("★ {score}"),
            ));
        }
        for reference in item.metadata["providers"].as_array().into_iter().flatten() {
            let provider = catalog::text(reference, "provider");
            if provider == rating_provider && crate::presentation::safe_external_url(rating_url) {
                continue;
            }
            let status = crate::presentation::reference_status(catalog::text(reference, "status"));
            let url = catalog::text(reference, "url");
            if !crate::presentation::safe_external_url(url) {
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
        header.append(&text);
        detail.append(&header);
        let phone_poster = poster::view(item, 160, item.progress, true);
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
        detail
    }
}
