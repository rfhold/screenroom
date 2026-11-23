#[derive(Debug, PartialEq, Eq)]
pub struct EpisodeLabels {
    pub primary: String,
    pub number: String,
}
impl EpisodeLabels {
    pub fn heading(&self) -> String {
        [&self.primary, &self.number]
            .into_iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ")
    }
}
pub fn episode_labels(meta: &str, season: bool) -> EpisodeLabels {
    let fallback = || EpisodeLabels {
        primary: "Episode".into(),
        number: String::new(),
    };
    let parts = meta.trim().split(" · ").collect::<Vec<_>>();
    let parsed = parts.iter().enumerate().rev().find_map(|(index, part)| {
        let (s, e) = part.strip_prefix('S')?.split_once('E')?;
        let s = s.trim_end();
        (!s.is_empty()
            && !e.is_empty()
            && s.bytes().all(|b| b.is_ascii_digit())
            && e.bytes().all(|b| b.is_ascii_digit()))
        .then_some((index, s, e))
    });
    let Some((index, s, e)) = parsed else {
        return fallback();
    };
    let series = parts[..index].join(" · ");
    let pad = |n: &str| {
        if n.len() < 2 {
            format!("0{n}")
        } else {
            n.to_owned()
        }
    };
    EpisodeLabels {
        primary: if season { String::new() } else { series },
        number: if season {
            format!("E{}", pad(e))
        } else {
            format!("S{}:E{}", pad(s), pad(e))
        },
    }
}
pub fn row_error(code: &str) -> &'static str {
    if code == "progress_unavailable" {
        "Continue Watching is unavailable. Jellyfin progress could not be read. Other catalog rows remain available."
    } else {
        "This row could not be loaded. Other catalog rows remain available."
    }
}
pub fn season_context(item_parent: Option<&str>, displayed: Option<&str>, is_season: bool) -> bool {
    is_season && item_parent.is_some() && item_parent == displayed
}
pub fn safe_external_url(value: &str) -> bool {
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) || value.contains('\\') {
        return false;
    }
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "https" | "http")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}
pub fn reference_status(value: &str) -> String {
    if value == "source_supplied" {
        "Identifier supplied by Jellyfin".into()
    } else {
        value.replace('_', " ")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_links_reject_credentials_and_non_web_schemes() {
        assert!(safe_external_url("https://example.com/title/123"));
        for url in [
            "javascript:alert(1)",
            "file:///private/key",
            "//example.com",
            "https://user:secret@example.com",
            "https://example.com/\nsecret",
            "https://example.com\\other",
        ] {
            assert!(!safe_external_url(url));
        }
    }
    #[test]
    fn short_number_requires_matching_season_parent() {
        assert!(season_context(Some("season"), Some("season"), true));
        for (parent, displayed, season) in [
            (Some("other"), Some("season"), true),
            (None, Some("season"), true),
            (Some("season"), Some("season"), false),
            (None, None, true),
        ] {
            assert!(!season_context(parent, displayed, season));
            assert_eq!(
                episode_labels("Series · S05E09", season_context(parent, displayed, season))
                    .heading(),
                "Series · S05:E09"
            );
        }
    }
    #[test]
    fn contextual_episode_labels() {
        for meta in ["Series · S05E09", "Series · S5 E9 · 24 min"] {
            assert_eq!(episode_labels(meta, false).heading(), "Series · S05:E09");
            assert_eq!(episode_labels(meta, true).heading(), "E09");
        }
        assert_eq!(
            episode_labels("Series · S0E100", false).heading(),
            "Series · S00:E100"
        );
        assert_eq!(
            episode_labels("S1 Show · S05E09", false).heading(),
            "S1 Show · S05:E09"
        );
        assert_eq!(
            episode_labels("Series · Part 2 · S05E09", false).heading(),
            "Series · Part 2 · S05:E09"
        );
        for invalid in [
            "Series",
            "Series · SxE2",
            "Series · S1E2junk",
            "S-1E2",
            "Series · S1E 2",
        ] {
            assert_eq!(episode_labels(invalid, true).heading(), "Episode");
        }
    }
    #[test]
    fn row_errors_never_echo_private_text() {
        assert!(row_error("progress_unavailable").contains("Continue Watching"));
        assert!(!row_error("private-id: raw backend error").contains("private"));
    }
}
