use crate::home::{App, accessible_label, clear, column, label};
use crate::state::Screen;
use adw::prelude::*;
use std::{rc::Rc, time::Duration};

impl App {
    pub fn search(self: &Rc<Self>) -> gtk::Box {
        let content = column(20);
        content.append(&label("Search", "heading"));
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let state = self.state.borrow();
        let capabilities = self.capabilities.borrow().clone();
        let entry = gtk::Entry::builder()
            .placeholder_text(format!("Search {}", state.library.label()))
            .text(&state.query)
            .max_length(
                self.capabilities
                    .borrow()
                    .maximum_query_length
                    .min(i32::MAX as u32) as i32,
            )
            .hexpand(true)
            .build();
        accessible_label(&entry, &format!("Search {}", state.library.label()));
        toolbar.append(&entry);
        let clear_button = gtk::Button::with_label("Clear query");
        clear_button.set_visible(!state.query.is_empty());
        toolbar.append(&clear_button);
        let settings = gtk::ToggleButton::with_label("Settings");
        settings.set_tooltip_text(Some("Search settings"));
        accessible_label(&settings, "Search settings");
        settings.set_active(state.settings);
        toolbar.append(&settings);
        content.append(&toolbar);
        let results = column(20);
        let types = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        let type_buttons: Vec<_> = ["Movies", "Shows", "Episodes"]
            .into_iter()
            .enumerate()
            .filter(|(i, _)| {
                capabilities
                    .types
                    .iter()
                    .any(|kind| kind.eq_ignore_ascii_case(["movie", "series", "episode"][*i]))
            })
            .map(|(i, title)| {
                let button = gtk::CheckButton::with_label(title);
                button.set_active(state.types[i]);
                types.append(&button);
                (i, button)
            })
            .collect();
        let panel = column(12);
        panel.set_margin_top(20);
        panel.set_margin_bottom(20);
        panel.set_margin_start(20);
        panel.set_margin_end(20);
        let surface = column(0);
        surface.add_css_class("settings");
        surface.set_visible(state.settings);
        surface.append(&panel);
        panel.append(&label("Catalog", "row-heading"));
        let sources = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let mut first: Option<gtk::CheckButton> = None;
        let source_buttons: Vec<_> = capabilities
            .sources
            .iter()
            .map(|title| {
                let button = gtk::CheckButton::with_label(&source_label(title));
                if let Some(first) = &first {
                    button.set_group(Some(first));
                } else {
                    first = Some(button.clone());
                }
                button.set_active(state.source == *title);
                sources.append(&button);
                button
            })
            .collect();
        panel.append(&sources);
        panel.append(&label("Content types", "row-heading"));
        panel.append(&types);
        panel.append(&label("Search methods", "row-heading"));
        panel.append(&label("Keep at least one method selected.", "muted"));
        let title_only = gtk::CheckButton::with_label("Title only");
        title_only.set_active(state.title_only);
        if capabilities.methods.iter().any(|m| m == "title_only")
            && capabilities
                .methods
                .iter()
                .any(|m| m == "text" || m == "fuzzy")
        {
            panel.append(&title_only);
            panel.append(&label(
                "Limit text to titles and turn off Semantic.",
                "muted",
            ));
        }
        let method_buttons: Vec<_> = [
            (
                if state.title_only {
                    "Text (titles only)"
                } else {
                    "Text (titles and metadata)"
                },
                "Match text in the selected fields.",
            ),
            (
                "Fuzzy titles",
                "Match titles with small spelling differences.",
            ),
            ("Semantic", capabilities.semantic_description.as_str()),
        ]
        .into_iter()
        .enumerate()
        .filter(|(i, _)| {
            capabilities
                .methods
                .iter()
                .any(|method| method == ["text", "fuzzy", "semantic"][*i])
        })
        .map(|(i, (title, help))| {
            let button = gtk::CheckButton::with_label(title);
            button.set_active(state.methods[i]);
            panel.append(&button);
            panel.append(&label(help, "muted"));
            (i, button)
        })
        .collect();
        drop(state);
        settings.update_relation(&[gtk::accessible::Relation::Controls(&[surface.upcast_ref()])]);
        for (source, button) in capabilities.sources.iter().zip(&source_buttons) {
            let source = source.clone();
            let weak = Rc::downgrade(self);
            button.connect_toggled(move |button| {
                if button.is_active() {
                    if let Some(app) = weak.upgrade() {
                        app.state.borrow_mut().source = source.clone();
                        // Source changes share the query revision and pending path.
                        let query = app.state.borrow().query.clone();
                        app.cancel_search();
                        let revision = app.state.borrow_mut().edit_query(query);
                        app.queue_results(revision);
                    }
                }
            });
        }
        content.append(&surface);
        content.append(&results);
        self.search_results.replace(results.downgrade());
        self.update_results(&results);
        sync_options(self, &type_buttons, &method_buttons);
        for (i, button) in &type_buttons {
            let i = *i;
            let weak = Rc::downgrade(self);
            let types = type_buttons
                .iter()
                .map(|(i, b)| (*i, b.downgrade()))
                .collect::<Vec<_>>();
            let methods = method_buttons
                .iter()
                .map(|(i, b)| (*i, b.downgrade()))
                .collect::<Vec<_>>();
            let results = results.downgrade();
            button.connect_toggled(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.state.borrow_mut().toggle_type(i);
                    app.save_preferences(false);
                    app.options_changed();
                    sync_options(&app, &upgrade(&types), &upgrade(&methods));
                    if let Some(results) = results.upgrade() {
                        app.update_results(&results);
                    }
                }
            });
        }
        for (i, button) in &method_buttons {
            let i = *i;
            let weak = Rc::downgrade(self);
            let types = type_buttons
                .iter()
                .map(|(i, b)| (*i, b.downgrade()))
                .collect::<Vec<_>>();
            let methods = method_buttons
                .iter()
                .map(|(i, b)| (*i, b.downgrade()))
                .collect::<Vec<_>>();
            let results = results.downgrade();
            button.connect_toggled(move |button| {
                if let Some(app) = weak.upgrade() {
                    // Programmatic changes already applied by Title only need no second toggle.
                    if app.state.borrow().methods[i] != button.is_active() {
                        app.state.borrow_mut().toggle_method(i);
                        app.options_changed();
                    }
                    sync_options(&app, &upgrade(&types), &upgrade(&methods));
                    if let Some(results) = results.upgrade() {
                        app.update_results(&results);
                    }
                }
            });
        }
        let weak = Rc::downgrade(self);
        let types = type_buttons
            .iter()
            .map(|(i, b)| (*i, b.downgrade()))
            .collect::<Vec<_>>();
        let methods = method_buttons
            .iter()
            .map(|(i, b)| (*i, b.downgrade()))
            .collect::<Vec<_>>();
        let output = results.downgrade();
        title_only.connect_toggled(move |_| {
            if let Some(app) = weak.upgrade() {
                app.state.borrow_mut().toggle_title_only();
                app.state
                    .borrow_mut()
                    .apply_capabilities(&app.capabilities.borrow());
                app.options_changed();
                sync_options(&app, &upgrade(&types), &upgrade(&methods));
                if let Some(output) = output.upgrade() {
                    app.update_results(&output);
                }
            }
        });
        let weak = Rc::downgrade(self);
        let panel = surface.downgrade();
        settings.connect_toggled(move |button| {
            if let Some(app) = weak.upgrade() {
                app.state.borrow_mut().settings = button.is_active();
                if let Some(panel) = panel.upgrade() {
                    panel.set_visible(button.is_active());
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        let settings_button = settings.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(button) = settings_button.upgrade() {
                    if button.is_active() {
                        button.set_active(false);
                        button.grab_focus();
                        return gtk::glib::Propagation::Stop;
                    }
                }
            }
            gtk::glib::Propagation::Proceed
        });
        content.add_controller(keys);
        let weak_entry = entry.downgrade();
        clear_button.connect_clicked(move |_| {
            if let Some(entry) = weak_entry.upgrade() {
                entry.set_text("");
                entry.grab_focus();
            }
        });
        let weak = Rc::downgrade(self);
        let output = results.downgrade();
        let clear_button = clear_button.downgrade();
        entry.connect_changed(move |entry| {
            if let Some(app) = weak.upgrade() {
                app.cancel_search();
                let revision = app.state.borrow_mut().edit_query(entry.text().into());
                if let Some(button) = clear_button.upgrade() {
                    button.set_visible(!entry.text().is_empty());
                }
                let Some(output) = output.upgrade() else {
                    return;
                };
                clear(&output);
                if entry.text().trim().is_empty() {
                    app.update_results(&output);
                    return;
                }
                output.append(&label("Searching titles…", "muted"));
                app.queue_results(revision);
            }
        });
        content
    }
    fn queue_results(self: &Rc<Self>, revision: u64) {
        if let Some(output) = self.search_results.borrow().upgrade() {
            self.update_results(&output);
        }
        let weak = Rc::downgrade(self);
        gtk::glib::timeout_add_local_once(Duration::from_millis(350), move || {
            if let Some(app) = weak.upgrade() {
                let ready = {
                    let mut state = app.state.borrow_mut();
                    state.settle_query(revision)
                        && state.screen == Screen::Search
                        && state.detail.is_none()
                };
                if ready {
                    app.request_search();
                }
            }
        });
    }
    fn update_results(self: &Rc<Self>, output: &gtk::Box) {
        clear(output);
        let state = self.state.borrow();
        if state.search_pending {
            output.append(&label("Searching titles…", "muted"));
            return;
        }
        if let Some(error) = &state.search_error {
            output.append(&label(error, "muted"));
            output.append(&self.button("Retry search", |app| app.request_search()));
            return;
        }
        let results = state.results(&crate::catalog::catalog());
        let status = if state.query.trim().is_empty() {
            format!("Enter a title to search {}.", state.library.label())
        } else if results.is_empty() {
            format!(
                "No matches in {}. Try another search.",
                state.library.label()
            )
        } else {
            format!(
                "{} {} in {}",
                results.len(),
                if results.len() == 1 {
                    "result"
                } else {
                    "results"
                },
                state.library.label()
            )
        };
        output.append(&label(&status, "muted"));
        if !state.query.trim().is_empty() {
            output.append(&self.grid(&results));
        }
    }
    fn options_changed(self: &Rc<Self>) {
        if !self.state.borrow().query_debouncing {
            self.request_search();
        }
    }
    pub fn request_search(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        self.cancel_search();
        use screenroom_client_core::{Page, SearchMethods, SearchRequest, SearchTypes};
        let Ok(client) = self.client.clone() else {
            return;
        };
        let (request, revision) = {
            let mut state = self.state.borrow_mut();
            if state.query.trim().is_empty() {
                state.result_ids.clear();
                return;
            }
            state.search_revision += 1;
            state.search_pending = true;
            state.query_debouncing = false;
            state.search_error = None;
            (
                SearchRequest {
                    query: state.query.clone(),
                    library_id: state.library.id().into(),
                    types: Some(SearchTypes {
                        movie: state.types[0],
                        series: state.types[1],
                        episode: state.types[2],
                    }),
                    methods: Some(SearchMethods {
                        text: state.methods[0],
                        fuzzy: state.methods[1],
                        semantic: state.methods[2],
                        title_only: state.title_only,
                    }),
                    source: state.source.clone(),
                    page: Some(Page {
                        size: 100,
                        token: String::new(),
                    }),
                },
                state.search_revision,
            )
        };
        if let Some(output) = self.search_results.borrow().upgrade() {
            self.update_results(&output);
        }
        let task = crate::backend::runtime().spawn(async move {
            crate::backend::paginate(|page| {
                let client = client.clone();
                let mut request = request.clone();
                request.page = Some(page);
                async move {
                    let response = client.search(request.clone()).await?;
                    eprintln!(
                        "core Search({:?}, {}, {}): {} results",
                        request.query,
                        request.library_id,
                        request.source,
                        response.items.len()
                    );
                    Ok((
                        response.items.into_iter().map(|i| i.id).collect(),
                        response.next_page_token,
                    ))
                }
            })
            .await
        });
        let weak = Rc::downgrade(self);
        self.search_task.replace(Some(task.abort_handle()));
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            let Some(app) = weak.upgrade() else {
                return;
            };
            {
                let mut state = app.state.borrow_mut();
                if state.search_revision != revision || state.screen != Screen::Search {
                    return;
                }
                state.search_pending = false;
                match result {
                    Ok(Ok(ids)) => state.result_ids = ids,
                    Ok(Err(error)) => state.search_error = Some(error.to_string()),
                    Err(error) => state.search_error = Some(error.to_string()),
                }
            }
            if let Some(output) = app.search_results.borrow().upgrade() {
                app.update_results(&output);
            }
        });
    }
    pub fn cancel_search(&self) {
        if let Some(task) = self.search_task.borrow_mut().take() {
            task.abort();
        }
    }
}
fn upgrade(
    buttons: &[(usize, gtk::glib::WeakRef<gtk::CheckButton>)],
) -> Vec<(usize, gtk::CheckButton)> {
    buttons
        .iter()
        .filter_map(|(i, b)| b.upgrade().map(|button| (*i, button)))
        .collect()
}
fn sync_options(
    app: &App,
    types: &[(usize, gtk::CheckButton)],
    methods: &[(usize, gtk::CheckButton)],
) {
    let (type_values, method_values, title_only) = {
        let state = app.state.borrow();
        (state.types, state.methods, state.title_only)
    };
    for (i, button) in types {
        let i = *i;
        button.set_sensitive(!type_values[i] || type_values.iter().filter(|v| **v).count() > 1);
    }
    for (i, button) in methods {
        let i = *i;
        button.set_sensitive(
            !(i == 2 && title_only)
                && (!method_values[i] || method_values.iter().filter(|v| **v).count() > 1),
        );
        button.set_active(method_values[i]);
        if i == 0 {
            button.set_label(Some(if title_only {
                "Text (titles only)"
            } else {
                "Text (titles and metadata)"
            }));
        }
    }
}
fn source_label(source: &str) -> String {
    match source {
        "all" => "All".into(),
        "tmdb" => "TMDB".into(),
        "jellyfin" => "Jellyfin".into(),
        _ => source.to_owned(),
    }
}
