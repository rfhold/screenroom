use screenroom_backend::{Uuid, catalog::Scope};
use screenroom_catalog_adapter::jellyfin::playback::Milliseconds;
use screenroom_proto::playback_gateway::v1::*;
use tonic::{Request, Status};

#[derive(Clone, Copy)]
pub(crate) struct Deadline(pub tokio::time::Instant);

pub(crate) fn budget<T>(request: &Request<T>, maximum: std::time::Duration) -> std::time::Duration {
    request
        .extensions()
        .get::<Deadline>()
        .map(|deadline| {
            deadline
                .0
                .saturating_duration_since(tokio::time::Instant::now())
                .min(maximum)
        })
        .unwrap_or(maximum)
}

pub(crate) fn invalid() -> Status {
    Status::invalid_argument("invalid playback input")
}

pub(crate) fn uuid(value: &str) -> Result<Uuid, Status> {
    if value.len() != 36 {
        return Err(invalid());
    }
    let id = Uuid::parse_str(value).map_err(|_| invalid())?;
    if id.is_nil() {
        return Err(invalid());
    }
    Ok(id)
}

pub(crate) fn selector<T>(request: &Request<T>) -> Result<Uuid, Status> {
    let mut values = request.metadata().get_all("x-screenroom-user-id").iter();
    let value = values.next().ok_or_else(invalid)?;
    if values.next().is_some() {
        return Err(invalid());
    }
    uuid(value.to_str().map_err(|_| invalid())?)
}

pub(crate) fn capability(value: &str) -> Result<(), Status> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn position(value: &str) -> Result<Milliseconds, Status> {
    Milliseconds::parse_decimal(value).map_err(|_| invalid())
}

pub(crate) fn scope(
    value: Option<screenroom_proto::common::v1::SourceLibraryScope>,
) -> Result<Scope, Status> {
    let value = value.unwrap_or_default();
    let mut scope = Scope {
        source_keys: if value.source_id.is_empty() {
            vec![]
        } else {
            vec![value.source_id]
        },
        library_ids: if value.library_id.is_empty() {
            vec![]
        } else {
            vec![value.library_id]
        },
        providers: vec![],
        kinds: vec![],
    };
    scope.validate().map_err(|_| invalid())?;
    if [&scope.source_keys, &scope.library_ids]
        .into_iter()
        .flatten()
        .any(|value| value.chars().any(char::is_control))
    {
        return Err(invalid());
    }
    for values in [
        &mut scope.source_keys,
        &mut scope.library_ids,
        &mut scope.providers,
        &mut scope.kinds,
    ] {
        values.sort();
        values.dedup();
    }
    Ok(scope)
}

pub(crate) fn create(value: &CreatePlaybackSessionRequest) -> Result<(), Status> {
    uuid(&value.request_id)?;
    if value.item_id.is_empty()
        || value.item_id.len() > 200
        || value.item_id.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    value.client_capabilities.as_ref().ok_or_else(invalid)?;
    match PlaybackStartPolicy::try_from(value.start_policy).map_err(|_| invalid())? {
        PlaybackStartPolicy::Position => {
            position(value.position_ms.as_deref().ok_or_else(invalid)?)?;
        }
        PlaybackStartPolicy::Beginning | PlaybackStartPolicy::Resume
            if value.position_ms.is_none() => {}
        _ => return Err(invalid()),
    }
    scope(value.scope.clone())?;
    Ok(())
}

pub(crate) fn choices(value: &PlaybackTrackChoices) -> Result<(), Status> {
    if value.subtitles_off && value.subtitle_track_id.is_some() {
        return Err(invalid());
    }
    for id in [&value.audio_track_id, &value.subtitle_track_id]
        .into_iter()
        .flatten()
    {
        uuid(id)?;
    }
    Ok(())
}

pub(crate) fn event(value: &ReportPlaybackEventRequest) -> Result<(), Status> {
    uuid(&value.session_id)?;
    capability(&value.control_capability)?;
    position(&value.position_ms)?;
    if value.sequence == 0
        || value.transport_revision == 0
        || !value.volume.is_finite()
        || !(0.0..=1.0).contains(&value.volume)
        || !matches!(value.kind, 1..=6)
        || (value.kind == 3 && !value.paused)
        || (value.kind == 4 && value.paused)
    {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn reason(value: &str) -> Result<(), Status> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoding_and_owned_work_share_one_bounded_deadline() {
        use std::time::Duration;
        let mut request = Request::new(());
        let maximum = Duration::from_secs(10);
        assert_eq!(budget(&request, maximum), maximum);
        request.extensions_mut().insert(Deadline(
            tokio::time::Instant::now() + Duration::from_secs(30),
        ));
        assert_eq!(budget(&request, maximum), maximum);
        request.extensions_mut().insert(Deadline(
            tokio::time::Instant::now() - Duration::from_secs(1),
        ));
        assert_eq!(budget(&request, maximum), Duration::ZERO);
    }

    #[test]
    fn milliseconds_are_checked_without_floating_point() {
        assert_eq!(position("000123").unwrap().decimal(), "123");
        for bad in ["-1", "1.2", "1e3", "", "922337203685478"] {
            assert!(position(bad).is_err(), "{bad}");
        }
        assert_eq!(
            position("922337203685477").unwrap().ticks(),
            9223372036854770000
        );
    }

    #[test]
    fn selector_rejects_missing_duplicate_and_nil() {
        let mut request = Request::new(());
        assert!(selector(&request).is_err());
        request.metadata_mut().insert(
            "x-screenroom-user-id",
            Uuid::new_v4().to_string().parse().unwrap(),
        );
        assert!(selector(&request).is_ok());
        request.metadata_mut().append(
            "x-screenroom-user-id",
            Uuid::new_v4().to_string().parse().unwrap(),
        );
        assert!(selector(&request).is_err());
        assert!(uuid(&Uuid::nil().to_string()).is_err());
    }

    #[test]
    fn event_enums_volume_and_backward_positions() {
        let mut event = ReportPlaybackEventRequest {
            session_id: Uuid::new_v4().to_string(),
            control_capability: "a".repeat(64),
            transport_revision: 1,
            sequence: 9,
            kind: 5,
            position_ms: "1".into(),
            paused: false,
            muted: false,
            volume: 0.5,
        };
        assert!(super::event(&event).is_ok());
        for volume in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            event.volume = volume;
            assert!(super::event(&event).is_err());
        }
        event.volume = 1.0;
        for kind in [0, 7, -1] {
            event.kind = kind;
            assert!(super::event(&event).is_err());
        }
    }

    #[test]
    fn safe_labels_and_choices() {
        assert!(reason("player-exit").is_ok());
        for value in ["", "https://private", "secret token", "UPPER", "a\nb"] {
            assert!(reason(value).is_err());
        }
        assert!(
            choices(&PlaybackTrackChoices {
                subtitle_track_id: Some(Uuid::new_v4().to_string()),
                subtitles_off: true,
                ..Default::default()
            })
            .is_err()
        );
    }
}
