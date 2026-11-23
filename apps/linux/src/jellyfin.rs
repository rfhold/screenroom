use crate::home::{App, clear, label};
use adw::prelude::*;
use screenroom_client_core::{self as core, JellyfinLink, JellyfinProfile, JellyfinSource};
use std::rc::Rc;

#[derive(Default)]
pub struct ProfileState {
    sources: Vec<JellyfinSource>,
    profiles: Vec<JellyfinProfile>,
    source: String,
    selected: String,
    link: Option<JellyfinLink>,
    known: bool,
    busy: bool,
    error: bool,
    status: String,
    owner: crate::profile_change::Owner,
    task: Option<tokio::task::AbortHandle>,
    reconcile_pending: bool,
    attempted: bool,
}
impl ProfileState {
    pub fn cancel(&mut self) {
        self.owner.invalidate();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
    fn begin(&mut self) -> u64 {
        self.cancel();
        self.busy = true;
        self.known = false;
        self.error = false;
        self.status.clear();
        self.owner.stamp()
    }
}
impl App {
    pub fn ensure_profile(self: &Rc<Self>) {
        self.profile.borrow_mut().owner.enter();
        if !self.profile.borrow().attempted {
            let source = self.profile.borrow().source.clone();
            self.load_profile(source);
        }
    }
    pub fn suspend_profile(&self) {
        let mut state = self.profile.borrow_mut();
        if !state.owner.active() && !state.busy {
            return;
        }
        state.owner.leave();
        state.cancel();
        state.attempted = false;
        if state.busy {
            state.busy = false;
            state.known = false;
            state.error = state.reconcile_pending;
        }
    }
    pub fn render_profile(self: &Rc<Self>) {
        clear(&self.profile_panel);
        let state = self.profile.borrow();
        let panel = &self.profile_panel;
        panel.append(&label("Jellyfin profile", "row-heading"));
        panel.append(&label(
            "Server-saved progress mapping for this local user selector. This is not sign-in.",
            "muted",
        ));
        if state.busy {
            panel.append(&label("Loading Jellyfin profile settings…", "muted"));
        }
        if state.error {
            panel.append(&label("Jellyfin profile state could not be confirmed. Retry reads server state only; no change will be repeated.", "muted"));
            let retry = self.button("Retry Jellyfin settings", |app| {
                let source = app.profile.borrow().source.clone();
                app.load_profile(source);
            });
            retry.set_sensitive(!state.busy);
            panel.append(&retry);
        }
        if !state.status.is_empty() {
            panel.append(&label(&state.status, "muted"));
        }
        if state.sources.is_empty() {
            if !state.busy && !state.error {
                panel.append(&label("No Jellyfin sources are available.", "muted"));
            }
            return;
        }
        panel.append(&label("Jellyfin source", "muted"));
        let source_names = state
            .sources
            .iter()
            .map(|s| s.display_name.as_str())
            .collect::<Vec<_>>();
        let sources = gtk::DropDown::from_strings(&source_names);
        sources.set_selected(
            state
                .sources
                .iter()
                .position(|s| s.id == state.source)
                .unwrap_or(0) as u32,
        );
        sources.set_sensitive(!state.busy);
        crate::home::accessible_label(&sources, "Jellyfin source");
        let weak = Rc::downgrade(self);
        sources.connect_selected_notify(move |dropdown| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let source = app
                .profile
                .borrow()
                .sources
                .get(dropdown.selected() as usize)
                .map(|s| s.id.clone());
            if let Some(source) = source {
                app.load_profile(source);
            }
        });
        panel.append(&sources);
        let current = if !state.known {
            "Not confirmed"
        } else if let Some(link) = &state.link {
            state
                .profiles
                .iter()
                .find(|p| p.id == link.jellyfin_user_id)
                .map(|p| p.display_name.as_str())
                .unwrap_or("Linked profile unavailable in choices")
        } else {
            "Not linked"
        };
        panel.append(&label(&format!("Current profile: {current}"), "muted"));
        panel.append(&label("Profile", "muted"));
        let names = state
            .profiles
            .iter()
            .map(|p| p.display_name.as_str())
            .collect::<Vec<_>>();
        let profiles = gtk::DropDown::from_strings(&names);
        profiles.set_selected(
            state
                .profiles
                .iter()
                .position(|p| p.id == state.selected)
                .map(|p| p as u32)
                .unwrap_or(gtk::INVALID_LIST_POSITION),
        );
        profiles.set_sensitive(!state.busy && state.known && !state.profiles.is_empty());
        crate::home::accessible_label(&profiles, "Jellyfin profile");
        panel.append(&profiles);
        if state.known && state.profiles.is_empty() {
            panel.append(&label("No enabled profiles are available.", "muted"));
        }
        let save = self.button("Save Jellyfin profile", |app| {
            let selected = app.profile.borrow().selected.clone();
            app.change_profile(Some(selected));
        });
        save.set_sensitive(
            !state.busy && state.known && state.profiles.iter().any(|p| p.id == state.selected),
        );
        let weak = Rc::downgrade(self);
        let save_button = save.downgrade();
        profiles.connect_selected_notify(move |dropdown| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let selected = app
                .profile
                .borrow()
                .profiles
                .get(dropdown.selected() as usize)
                .map(|p| p.id.clone());
            if let Some(selected) = selected {
                let mut state = app.profile.borrow_mut();
                state.selected = selected;
                if let Some(save) = save_button.upgrade() {
                    save.set_sensitive(!state.busy && state.known);
                }
            }
        });
        panel.append(&save);
        let unlink = self.button("Unlink Jellyfin profile", |app| app.change_profile(None));
        unlink.set_sensitive(!state.busy && state.known && state.link.is_some());
        panel.append(&unlink);
    }
    pub fn load_profile(self: &Rc<Self>, target: String) {
        if !self.profile.borrow().owner.active() {
            return;
        }
        self.profile.borrow_mut().attempted = true;
        let generation = self.profile.borrow_mut().begin();
        self.render_profile();
        let client = crate::backend::client();
        let task = crate::backend::runtime().spawn(crate::backend::operation(async move {
            let client = client?;
            client.invalidate_reads();
            let mut sources = crate::backend::paginate(|page| {
                let client = client.clone();
                async move {
                    let r = client.list_jellyfin_sources(page).await?;
                    Ok((r.sources, r.next_page_token))
                }
            })
            .await?;
            let mut seen = std::collections::HashSet::new();
            sources.retain(|s| seen.insert(s.id.clone()));
            let source = sources
                .iter()
                .find(|s| s.id == target)
                .or(sources.first())
                .map(|s| s.id.clone())
                .unwrap_or_default();
            let mut profiles = if source.is_empty() {
                Vec::new()
            } else {
                crate::backend::paginate(|page| {
                    let client = client.clone();
                    let source = source.clone();
                    async move {
                        let r = client.list_jellyfin_profiles(source, page).await?;
                        Ok((r.profiles, r.next_page_token))
                    }
                })
                .await?
            };
            seen.clear();
            profiles.retain(|p| seen.insert(p.id.clone()));
            let link = if source.is_empty() {
                None
            } else {
                client.get_jellyfin_link(source.clone()).await?
            };
            Ok::<_, core::ClientError>((sources, source, profiles, link))
        }));
        self.profile.borrow_mut().task = Some(task.abort_handle());
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = app.profile.borrow_mut();
            if !state.owner.current(generation) {
                return;
            }
            state.busy = false;
            match result {
                Ok(Ok((sources, source, profiles, link))) => {
                    state.selected = link
                        .as_ref()
                        .map(|l| l.jellyfin_user_id.clone())
                        .or_else(|| profiles.first().map(|p| p.id.clone()))
                        .unwrap_or_default();
                    state.sources = sources;
                    state.source = source;
                    state.profiles = profiles;
                    state.link = link;
                    state.known = true;
                }
                _ => state.error = true,
            }
            let refresh = state.known && state.reconcile_pending;
            if refresh {
                state.reconcile_pending = false;
            }
            drop(state);
            app.render_profile();
            if refresh {
                app.refresh_profile_home();
            }
        });
    }
    fn change_profile(self: &Rc<Self>, profile: Option<String>) {
        self.stop_playback("profile_change");
        self.clear_history();
        let (source, generation) = {
            let mut state = self.profile.borrow_mut();
            if state.busy
                || !state.owner.active()
                || !state.known
                || profile
                    .as_ref()
                    .is_some_and(|id| !state.profiles.iter().any(|p| &p.id == id))
            {
                return;
            }
            state.reconcile_pending = true;
            (state.source.clone(), state.begin())
        };
        self.render_profile();
        let client = crate::backend::client();
        let task = crate::backend::runtime().spawn(crate::backend::operation(async move {
            let client = client?;
            let write_client = client.clone();
            let write_source = source.clone();
            let write_profile = profile.clone();
            let read_client = client.clone();
            let read_source = source.clone();
            crate::profile_change::reconcile(
                async move {
                    if let Some(profile) = write_profile {
                        let returned = write_client
                            .set_jellyfin_link(write_source.clone(), profile.clone())
                            .await?;
                        Ok(returned.source_id == write_source
                            && returned.jellyfin_user_id == profile)
                    } else {
                        Ok(write_client
                            .clear_jellyfin_link(write_source.clone())
                            .await?
                            == write_source)
                    }
                },
                || client.invalidate_reads(),
                move || Box::pin(read_client.get_jellyfin_link(read_source)),
                |link| match (&profile, link) {
                    (None, None) => true,
                    (Some(profile), Some(link)) => {
                        link.source_id == source && &link.jellyfin_user_id == profile
                    }
                    _ => false,
                },
            )
            .await
        }));
        self.profile.borrow_mut().task = Some(task.abort_handle());
        let weak = Rc::downgrade(self);
        gtk::glib::MainContext::default().spawn_local(async move {
            let result = task.await;
            let Some(app) = weak.upgrade() else { return; };
            let mut state = app.profile.borrow_mut();
            if !state.owner.current(generation) { return; }
            state.busy = false;
            let reconciled = match result {
                Ok(Ok((link, confirmed, ambiguous))) => {
                    state.selected = link.as_ref().map(|l| l.jellyfin_user_id.clone()).or_else(|| state.profiles.first().map(|p| p.id.clone())).unwrap_or_default();
                    state.link = link; state.known = true;
                    state.reconcile_pending = false;
                    state.status = if !confirmed { "The requested change was not confirmed. Current server state is shown; choose a profile before trying again." } else if ambiguous { "Server state checked: profile change is saved." } else { "Profile change saved on the server." }.into();
                    true
                }
                _ => { state.error = true; false }
            };
            drop(state);
            app.render_profile();
            if reconciled { app.refresh_profile_home(); }
        });
    }
    fn refresh_profile_home(self: &Rc<Self>) {
        self.home_dirty.set(true);
        if self.state.borrow().screen == crate::state::Screen::Home {
            self.reload();
        }
    }
}
