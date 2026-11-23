use std::time::Duration;

pub const IDLE_DELAY: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Visibility {
    pub paused: bool,
    pub busy: bool,
    pub blocked: bool,
    pub preparing: bool,
    pub scrubbing: bool,
    pub menu: bool,
    pub keyboard_focus: bool,
}

impl Visibility {
    pub fn show(&self, idle: Duration) -> bool {
        self.paused
            || self.busy
            || self.blocked
            || self.preparing
            || self.scrubbing
            || self.menu
            || self.keyboard_focus
            || idle < IDLE_DELAY
    }
}

#[derive(Debug, PartialEq)]
pub enum Escape {
    Menu,
    Fullscreen,
    Exit,
}

pub fn escape(menu: bool, fullscreen: bool) -> Escape {
    if menu {
        Escape::Menu
    } else if fullscreen {
        Escape::Fullscreen
    } else {
        Escape::Exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_idle_hides_at_three_seconds() {
        let state = Visibility::default();
        assert!(state.show(Duration::from_millis(2999)));
        assert!(!state.show(IDLE_DELAY));
        assert!(state.show(Duration::ZERO));
    }

    #[test]
    fn each_interaction_or_nonplaying_state_keeps_controls_visible() {
        for state in [
            Visibility {
                paused: true,
                ..Default::default()
            },
            Visibility {
                busy: true,
                ..Default::default()
            },
            Visibility {
                blocked: true,
                ..Default::default()
            },
            Visibility {
                preparing: true,
                ..Default::default()
            },
            Visibility {
                scrubbing: true,
                ..Default::default()
            },
            Visibility {
                menu: true,
                ..Default::default()
            },
            Visibility {
                keyboard_focus: true,
                ..Default::default()
            },
        ] {
            assert!(state.show(Duration::from_secs(100)));
        }
    }

    #[test]
    fn escape_dismisses_menu_before_fullscreen_before_exit() {
        assert_eq!(escape(true, true), Escape::Menu);
        assert_eq!(escape(true, false), Escape::Menu);
        assert_eq!(escape(false, true), Escape::Fullscreen);
        assert_eq!(escape(false, false), Escape::Exit);
    }

    #[test]
    fn chrome_is_unmeasured_and_media_identity_does_not_depend_on_visibility() {
        let source = include_str!("playback.rs");
        assert_eq!(source.matches("stage.set_child(Some(&picture))").count(), 1);
        for overlay in ["header", "footer", "feedback"] {
            assert!(source.contains(&format!("stage.set_measure_overlay(&{overlay}, false)")));
        }
        let presentation = source
            .split("fn refresh_presentation(&self)")
            .nth(1)
            .unwrap()
            .split("fn seek_relative")
            .next()
            .unwrap();
        for forbidden in [
            "pipeline",
            "attach(",
            "detach(",
            "Command::",
            "set_content(",
        ] {
            assert!(!presentation.contains(forbidden));
        }
        assert!(presentation.contains("chrome.set_can_target(visible)"));
        assert!(presentation.contains("chrome.set_sensitive(visible)"));
    }

    #[test]
    fn teardown_removes_both_timer_sources_before_media_detach() {
        let source = include_str!("playback.rs");
        let close = source.split("pub fn close(&self").nth(1).unwrap();
        let detach = close.find("self.detach()").unwrap();
        for timer in ["presentation_timer", "tick_timer"] {
            assert!(close.find(timer).unwrap() < detach);
        }
        assert!(close[..detach].contains("timer.remove()"));
    }
}
