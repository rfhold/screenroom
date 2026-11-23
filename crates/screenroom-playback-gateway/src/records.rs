use screenroom_backend::{
    Uuid,
    playback::{Operation, Session},
};
use screenroom_proto::playback_gateway::v1::*;
use serde::{Deserialize, Serialize};
use tonic::Status;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Replay {
    pub control: String,
    pub media: String,
    // Read legacy receipts; new authorization material is published separately.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quick_connect_secret: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Descriptor {
    pub method: i32,
    pub mime_type: String,
    pub root: Uuid,
    pub source: String,
    pub duration_ms: Option<String>,
    pub start_position_ms: String,
    pub timeline_offset_ms: String,
    pub compatible_used: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrackRecord {
    pub id: String,
    pub kind: i32,
    pub display_name: String,
    pub language: Option<String>,
    pub selected: bool,
    pub available: bool,
    pub subtitle_mode: Option<i32>,
    pub resource: Option<Uuid>,
}

pub(crate) fn path(session: Uuid, revision: u32, resource: Uuid, capability: &str) -> String {
    format!("/playback/v1/media/{session}/{revision}/{resource}?cap={capability}")
}

pub(crate) fn outcome(operation: Option<&Operation>) -> i32 {
    match operation.map(|o| o.status.as_str()) {
        Some("pending") => PlaybackReportOutcome::Pending as i32,
        Some("confirmed") => PlaybackReportOutcome::Confirmed as i32,
        Some("uncertain") => PlaybackReportOutcome::Uncertain as i32,
        Some("failed") => PlaybackReportOutcome::Failed as i32,
        _ => PlaybackReportOutcome::NotSent as i32,
    }
}

pub(crate) fn public(
    session: &Session,
    operation: Option<&Operation>,
    replay: &Replay,
    origin: &str,
) -> Result<PlaybackSession, Status> {
    let revision = u32::try_from(session.transport_revision).map_err(|_| unavailable())?;
    let active = matches!(session.state.as_str(), "ready" | "playing" | "paused");
    let descriptor: Option<Descriptor> = session
        .transport
        .clone()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| unavailable())?;
    let tracks: Vec<TrackRecord> =
        serde_json::from_value(session.tracks.clone()).map_err(|_| unavailable())?;
    let transport = descriptor
        .as_ref()
        .filter(|_| active)
        .map(|d| PlaybackTransport {
            method: d.method,
            mime_type: d.mime_type.clone(),
            media_url: format!(
                "{origin}{}",
                path(session.id, revision, d.root, &replay.media)
            ),
            revision,
            duration_ms: d.duration_ms.clone(),
            start_position_ms: d.start_position_ms.clone(),
            timeline_offset_ms: d.timeline_offset_ms.clone(),
        });
    Ok(PlaybackSession {
        session_id: session.id.to_string(),
        control_capability: replay.control.clone(),
        state: match session.state.as_str() {
            "preparing" => 1,
            "ready" => 2,
            "playing" => 3,
            "paused" => 4,
            "stopping" => 5,
            "stopped" => 6,
            "expired" => 7,
            "failed" => 8,
            "uncertain" => 9,
            _ => return Err(unavailable()),
        },
        selected_source: descriptor.as_ref().map(|d| PlaybackMediaSource {
            id: d.source.clone(),
            display_name: "Media".into(),
        }),
        tracks: tracks
            .into_iter()
            .map(|t| PlaybackTrack {
                id: t.id,
                kind: t.kind,
                display_name: t.display_name,
                language: t.language,
                selected: t.selected,
                available: t.available,
                subtitle_mode: t.subtitle_mode,
                media_url: t
                    .resource
                    .filter(|_| active)
                    .map(|id| format!("{origin}{}", path(session.id, revision, id, &replay.media))),
            })
            .collect(),
        transport,
        transport_revision: revision,
        last_accepted_sequence: u32::try_from(session.last_sequence).map_err(|_| unavailable())?,
        report_outcome: outcome(operation),
        lease_expires_at: session.lease_until.to_rfc3339(),
        expires_at: session.absolute_until.to_rfc3339(),
        pending_operation: operation
            .filter(|o| matches!(o.status.as_str(), "pending" | "uncertain"))
            .map(|o| PlaybackOperation {
                request_id: o.request_id.map(|id| id.to_string()),
                sequence: o.sequence.and_then(|s| u32::try_from(s).ok()),
                outcome: outcome(Some(o)),
            }),
        cleanup_pending: matches!(
            session.state.as_str(),
            "stopping" | "expired" | "failed" | "uncertain"
        ),
        failure_code: operation
            .and_then(|o| o.result.as_ref())
            .and_then(|v| v.get("outcome").unwrap_or(v).get("failure"))
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok()),
    })
}

pub(crate) fn unavailable() -> Status {
    Status::unavailable("upstream_unavailable")
}

pub(crate) fn failure(code: i32) -> Status {
    match code {
        1 => Status::failed_precondition("profile_required"),
        2 => Status::failed_precondition("profile_changed"),
        3 => Status::failed_precondition("source_unavailable"),
        4 => Status::failed_precondition("item_unavailable"),
        5 => Status::failed_precondition("unsupported_media"),
        6 => Status::failed_precondition("transcoding_unavailable"),
        7 => Status::unavailable("playback_authorization_unavailable"),
        8 => Status::already_exists("playback_conflict"),
        9 => Status::failed_precondition("session_expired"),
        10 => Status::already_exists("operation_conflict"),
        11 => Status::resource_exhausted("capacity_exhausted"),
        _ => unavailable(),
    }
}

pub(crate) fn database(error: screenroom_backend::Error) -> Status {
    use screenroom_backend::Error;
    match error {
        Error::NotFound => Status::not_found("session_unavailable"),
        Error::Stale => Status::failed_precondition("session_unavailable"),
        Error::Invalid(code) => match code {
            "capacity_exhausted" => Status::resource_exhausted(code),
            "playback_conflict" | "operation_conflict" | "operation_pending" => {
                Status::already_exists(code)
            }
            "profile_required" | "item_unavailable" => Status::failed_precondition(code),
            "personal_state_unknown" => Status::failed_precondition("outcome_uncertain"),
            _ => Status::invalid_argument("invalid playback input"),
        },
        _ => unavailable(),
    }
}

pub(crate) fn provider_code(
    error: &screenroom_catalog_adapter::jellyfin::playback::PlaybackError,
) -> i32 {
    use screenroom_catalog_adapter::jellyfin::playback::Failure;
    match error.failure {
        Failure::AuthorizationUnavailable => 7,
        Failure::Unsupported => 5,
        Failure::FenceDenied => 2,
        _ => 12,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> Session {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        Session {
            id,
            user_id: Uuid::new_v4(),
            public_item_id: "public-item".into(),
            source_item_id: Uuid::new_v4(),
            source_id: Uuid::new_v4(),
            source_key: "private-source-key".into(),
            scope: json!({"private":"scope"}),
            external_item_id: "private-remote-item".into(),
            connection_id: Uuid::new_v4(),
            connection_generation: 1,
            binding_revision: "private-binding".into(),
            remote_profile_id: "private-profile".into(),
            device_id: "private-device".into(),
            play_session_id: Some("private-play-session".into()),
            credential_ref: Uuid::new_v4(),
            replay_ref: Uuid::new_v4(),
            control_hash: vec![1; 32],
            media_hash: Some(vec![2; 32]),
            state: "ready".into(),
            transport_revision: 1,
            last_sequence: 1,
            capabilities: json!({}),
            tracks: json!([]),
            transport: Some(
                serde_json::to_value(Descriptor {
                    method: 1,
                    mime_type: "video/mp4".into(),
                    root: Uuid::new_v4(),
                    source: Uuid::new_v4().to_string(),
                    duration_ms: Some("922337203685477".into()),
                    start_position_ms: "9000".into(),
                    timeline_offset_ms: "0".into(),
                    compatible_used: false,
                })
                .unwrap(),
            ),
            lease_until: now,
            absolute_until: now,
            paused_at: None,
        }
    }

    #[test]
    fn public_projection_has_separate_authorities_no_provider_identity_and_lossless_timeline() {
        let mut session = session();
        let replay = Replay {
            control: "a".repeat(64),
            media: "b".repeat(64),
            quick_connect_secret: Some("private-quick-connect".into()),
        };
        let view = public(&session, None, &replay, "https://api.test").unwrap();
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("private-"));
        assert!(!json.contains("credential_ref"));
        assert!(!json.contains("control_hash"));
        assert_eq!(view.control_capability, replay.control);
        let transport = view.transport.unwrap();
        assert_eq!(transport.duration_ms.as_deref(), Some("922337203685477"));
        assert!(
            transport
                .media_url
                .ends_with(&format!("?cap={}", replay.media))
        );
        assert!(!transport.media_url.contains(&replay.control));
        for state in ["expired", "stopping", "stopped", "uncertain", "failed"] {
            session.state = state.into();
            assert!(
                public(&session, None, &replay, "https://api.test")
                    .unwrap()
                    .transport
                    .is_none()
            );
        }
        session.state = "unknown".into();
        assert!(public(&session, None, &replay, "https://api.test").is_err());
    }

    #[test]
    fn immutable_authority_replay_excludes_authorization_material_and_fits_private_file() {
        let replay = Replay {
            control: "a".repeat(64),
            media: "b".repeat(64),
            quick_connect_secret: None,
        };
        let encoded = serde_json::to_string(&replay).unwrap();
        assert!(encoded.len() <= 4096);
        assert!(!encoded.contains("quick_connect_secret"));
    }
}
