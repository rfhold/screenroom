use super::*;
use screenroom_proto::playback_gateway::v1 as pb;

static REQUEST: AtomicU64 = AtomicU64::new(0);
pub(super) fn budget(op: &str) -> Option<u32> {
    match op {
        "playback_create" => Some(30_000),
        "playback_get" | "playback_update" | "playback_report" | "playback_stop" => Some(10_000),
        _ => None,
    }
}
fn bad_response() -> ClientError {
    ClientError::InvalidResponse {
        reason: "invalid playback response".into(),
    }
}
fn uuid(value: &str) -> Result<(), ClientError> {
    if !valid_uuid(value) || value == "00000000-0000-0000-0000-000000000000" {
        return Err(invalid("invalid playback UUID"));
    }
    Ok(())
}
fn capability(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn authority(id: &str, cap: &str) -> Result<(), ClientError> {
    uuid(id)?;
    if !capability(cap) {
        return Err(invalid("invalid playback capability"));
    }
    Ok(())
}
fn milliseconds(value: &str) -> Result<(), ClientError> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|b| b.is_ascii_digit())
        || !value.parse::<u64>().is_ok_and(|n| n <= 922_337_203_685_477)
    {
        return Err(invalid("invalid playback milliseconds"));
    }
    Ok(())
}
fn enum_names(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "start_policy" => &["unspecified", "resume", "beginning", "position"],
        "state" => &[
            "unspecified",
            "preparing",
            "ready",
            "playing",
            "paused",
            "stopping",
            "stopped",
            "expired",
            "failed",
            "uncertain",
        ],
        "report_outcome" | "outcome" => &[
            "unspecified",
            "not_sent",
            "pending",
            "confirmed",
            "uncertain",
            "failed",
        ],
        "method" => &["unspecified", "direct_play", "direct_stream", "transcode"],
        "subtitle_mode" => &["unspecified", "webvtt", "burn_in", "unavailable"],
        "failure_code" => &[
            "unspecified",
            "profile_required",
            "profile_changed",
            "source_unavailable",
            "item_unavailable",
            "unsupported_media",
            "transcoding_unavailable",
            "playback_authorization_unavailable",
            "playback_conflict",
            "session_expired",
            "operation_conflict",
            "capacity_exhausted",
            "upstream_unavailable",
        ],
        _ => return None,
    })
}
fn enums(value: &mut Value, to_wire: bool) -> Result<(), ClientError> {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let names = if key == "kind" {
                    Some(if to_wire {
                        &[
                            "unspecified",
                            "started",
                            "progress",
                            "paused",
                            "resumed",
                            "seeked",
                            "heartbeat",
                        ][..]
                    } else {
                        &["unspecified", "audio", "subtitle"][..]
                    })
                } else {
                    enum_names(key)
                };
                if let Some(names) = names {
                    if value.is_null() {
                        continue;
                    }
                    *value = if to_wire {
                        let index = names
                            .iter()
                            .position(|name| value.as_str() == Some(name))
                            .ok_or_else(bad_response)?;
                        if index == 0 {
                            return Err(invalid("unspecified playback enum"));
                        }
                        Value::from(index)
                    } else {
                        let index = value
                            .as_u64()
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(bad_response)?;
                        if index == 0 {
                            return Err(bad_response());
                        }
                        Value::from(*names.get(index).ok_or_else(bad_response)?)
                    };
                } else {
                    enums(value, to_wire)?;
                }
            }
        }
        Value::Array(entries) => {
            for entry in entries {
                enums(entry, to_wire)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(async_runtime = "tokio"))]
impl Client {
    pub async fn create_playback_session(
        self: Arc<Self>,
        request: CreatePlaybackSessionRequest,
    ) -> Result<CreatePlaybackSessionResponse, ClientError> {
        uuid(&request.request_id)?;
        if request.item_id.is_empty()
            || request.item_id.len() > 200
            || request.item_id.chars().any(char::is_control)
            || request.client_capabilities.is_none()
        {
            return Err(invalid("invalid playback creation"));
        }
        match (&request.start_policy, &request.position_ms) {
            (PlaybackStartPolicy::Position, Some(ms)) => milliseconds(ms)?,
            (PlaybackStartPolicy::Resume | PlaybackStartPolicy::Beginning, None) => {}
            _ => return Err(invalid("invalid playback start policy")),
        }
        self.playback_call("playback_create", request, true).await
    }
    pub async fn get_playback_session(
        self: Arc<Self>,
        request: GetPlaybackSessionRequest,
    ) -> Result<GetPlaybackSessionResponse, ClientError> {
        authority(&request.session_id, &request.control_capability)?;
        self.playback_call("playback_get", request, false).await
    }
    pub async fn update_playback_session(
        self: Arc<Self>,
        request: UpdatePlaybackSessionRequest,
    ) -> Result<UpdatePlaybackSessionResponse, ClientError> {
        authority(&request.session_id, &request.control_capability)?;
        uuid(&request.request_id)?;
        milliseconds(&request.position_ms)?;
        if request.expected_transport_revision == 0 {
            return Err(invalid("missing transport revision"));
        }
        if let Some(choices) = &request.track_choices {
            if choices.subtitles_off && choices.subtitle_track_id.is_some() {
                return Err(invalid("conflicting subtitle choices"));
            }
            for id in [&choices.audio_track_id, &choices.subtitle_track_id]
                .into_iter()
                .flatten()
            {
                uuid(id)?;
            }
        }
        self.playback_call("playback_update", request, true).await
    }
    pub async fn report_playback_event(
        self: Arc<Self>,
        request: ReportPlaybackEventRequest,
    ) -> Result<ReportPlaybackEventResponse, ClientError> {
        authority(&request.session_id, &request.control_capability)?;
        milliseconds(&request.position_ms)?;
        if request.sequence == 0
            || request.transport_revision == 0
            || !request.volume.is_finite()
            || !(0.0..=1.0).contains(&request.volume)
            || request.kind == PlaybackEventKind::Unspecified
            || (request.kind == PlaybackEventKind::Paused && !request.paused)
            || (request.kind == PlaybackEventKind::Resumed && request.paused)
        {
            return Err(invalid("invalid playback event"));
        }
        self.playback_call("playback_report", request, true).await
    }
    pub async fn stop_playback_session(
        self: Arc<Self>,
        request: StopPlaybackSessionRequest,
    ) -> Result<StopPlaybackSessionResponse, ClientError> {
        authority(&request.session_id, &request.control_capability)?;
        uuid(&request.request_id)?;
        milliseconds(&request.position_ms)?;
        if request.stop_reason.is_empty()
            || request.stop_reason.len() > 64
            || !request
                .stop_reason
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
        {
            return Err(invalid("invalid stop reason"));
        }
        self.playback_call("playback_stop", request, true).await
    }
}
impl Client {
    pub(super) fn cancel_playback_requests(&self) {
        self.playback_generation.fetch_add(1, Ordering::SeqCst);
        let pending = if let Ok(mut cache) = self.cache.lock() {
            let keys = cache
                .pending
                .keys()
                .filter(|key| key.starts_with("playback:"))
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| cache.pending.remove(&key).map(|entry| entry.abort))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for abort in pending {
            abort.abort();
        }
    }
    async fn playback_call<Q: Serialize, R: DeserializeOwned>(
        self: Arc<Self>,
        op: &'static str,
        request: Q,
        mutation: bool,
    ) -> Result<R, ClientError> {
        self.require_user()?;
        if self.mutation_waiters.fetch_add(1, Ordering::SeqCst) >= 32 {
            self.mutation_waiters.fetch_sub(1, Ordering::SeqCst);
            return Err(ClientError::ResourceExhausted {
                reason: "32 mutation requests already active".into(),
            });
        }
        let _admission = MutationAdmission(self.clone());
        let generation = self.generation.load(Ordering::SeqCst);
        let profile_generation = self.playback_generation.load(Ordering::SeqCst);
        let mut value =
            serde_json::to_value(request).map_err(|_| invalid("invalid playback request"))?;
        enums(&mut value, true)?;
        let expected_authority = value
            .get("session_id")
            .and_then(Value::as_str)
            .zip(value.get("control_capability").and_then(Value::as_str))
            .map(|(id, cap)| (id.to_owned(), cap.to_owned()));
        let expected_revision = value
            .get("expected_transport_revision")
            .or_else(|| value.get("transport_revision"))
            .and_then(Value::as_u64);
        let key = format!("playback:{}", REQUEST.fetch_add(1, Ordering::Relaxed));
        let dispatched = std::sync::atomic::AtomicBool::new(false);
        let result = timeout(
            budget(op).ok_or_else(|| invalid("unknown playback operation"))?,
            async {
                let _serial = self.mutation.lock().await;
                if self.generation.load(Ordering::SeqCst) != generation
                    || self.playback_generation.load(Ordering::SeqCst) != profile_generation
                {
                    return Err(ClientError::Cancelled);
                }
                dispatched.store(true, Ordering::SeqCst);
                self.clone().dedup(key, op.into(), value, false).await
            },
        )
        .await;
        let mut response = match result {
            Ok(value) => value,
            Err(
                ClientError::DeadlineExceeded
                | ClientError::Unavailable { .. }
                | ClientError::Cancelled,
            ) if mutation && dispatched.load(Ordering::SeqCst) => {
                return Err(ClientError::PlaybackUncertain);
            }
            Err(error) => return Err(error),
        };
        enums(&mut response, false)?;
        let session = response
            .get("session")
            .filter(|v| !v.is_null())
            .ok_or_else(bad_response)?;
        let session: PlaybackSession = bridge(session)?;
        if expected_authority.as_ref().is_some_and(|(id, cap)| {
            session.session_id != *id || session.control_capability != *cap
        }) || expected_revision
            .is_some_and(|revision| u64::from(session.transport_revision) < revision)
        {
            return Err(bad_response());
        }
        self.validate_playback_session(&session)?;
        if mutation && session.report_outcome == PlaybackReportOutcome::Confirmed {
            self.invalidate_reads();
        }
        bridge(response)
    }
    fn validate_playback_session(&self, session: &PlaybackSession) -> Result<(), ClientError> {
        authority(&session.session_id, &session.control_capability).map_err(|_| bad_response())?;
        if let Some(transport) = &session.transport {
            if transport.revision == 0 || transport.revision != session.transport_revision {
                return Err(bad_response());
            }
            for value in [&transport.start_position_ms, &transport.timeline_offset_ms]
                .into_iter()
                .chain(transport.duration_ms.as_ref())
            {
                milliseconds(value).map_err(|_| bad_response())?;
            }
            self.validate_playback_url(&transport.media_url, session)?;
            if ![
                "video/mp4",
                "application/vnd.apple.mpegurl",
                "application/x-mpegURL",
            ]
            .contains(&transport.mime_type.as_str())
            {
                return Err(bad_response());
            }
        }
        for track in &session.tracks {
            if let Some(url) = &track.media_url {
                self.validate_playback_url(url, session)?;
            }
        }
        Ok(())
    }
    fn validate_playback_url(
        &self,
        value: &str,
        session: &PlaybackSession,
    ) -> Result<(), ClientError> {
        let url = url::Url::parse(value).map_err(|_| bad_response())?;
        let origin = url::Url::parse(&self.config.endpoint).map_err(|_| bad_response())?;
        let prefix = format!(
            "/playback/v1/media/{}/{}/",
            session.session_id, session.transport_revision
        );
        let resource = url.path().strip_prefix(&prefix).ok_or_else(bad_response)?;
        if url.origin() != origin.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || !valid_uuid(resource)
            || url.query_pairs().count() != 1
            || !url
                .query_pairs()
                .any(|(k, v)| k == "cap" && capability(&v) && v != session.control_capability)
        {
            return Err(bad_response());
        }
        Ok(())
    }
    pub(super) async fn playback_network(
        &self,
        op: &str,
        value: Value,
    ) -> Result<Value, ClientError> {
        let transport = self
            .transport_budget(budget(op).ok_or_else(|| invalid("unknown playback operation"))?)
            .await?;
        let mut client = pb::playback_service_client::PlaybackServiceClient::new(transport);
        macro_rules! rpc {
            ($method:ident, $request:ty) => {{
                let body: $request = bridge(value)?;
                let response = client
                    .$method(self.request(body)?)
                    .await
                    .map_err(safe_status)?;
                bridge(response.into_inner())
            }};
        }
        match op {
            "playback_create" => rpc!(create_playback_session, pb::CreatePlaybackSessionRequest),
            "playback_get" => rpc!(get_playback_session, pb::GetPlaybackSessionRequest),
            "playback_update" => rpc!(update_playback_session, pb::UpdatePlaybackSessionRequest),
            "playback_report" => rpc!(report_playback_event, pb::ReportPlaybackEventRequest),
            "playback_stop" => rpc!(stop_playback_session, pb::StopPlaybackSessionRequest),
            _ => Err(invalid("unknown playback operation")),
        }
    }
}
fn safe_status(status: tonic::Status) -> ClientError {
    match status.code() {
        tonic::Code::DeadlineExceeded => return ClientError::DeadlineExceeded,
        tonic::Code::Cancelled => return ClientError::Cancelled,
        _ => {}
    }
    if let Some(names) = enum_names("failure_code")
        && let Some(name) = names.iter().skip(1).find(|name| **name == status.message())
        && let Ok(code) = serde_json::from_value(Value::from(*name))
    {
        return ClientError::Playback { code };
    }
    match status.code() {
        tonic::Code::InvalidArgument => invalid("invalid playback input"),
        _ => ClientError::Unavailable {
            reason: "playback service unavailable".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enum_bridge_preserves_nullable_wide_timeline_and_rejects_unknowns() {
        let mut value = serde_json::json!({"state": 2, "report_outcome": 1, "failure_code": null, "transport": {"method": 1, "duration_ms": "922337203685477"}, "tracks": [{"kind": 2, "subtitle_mode": 1}]});
        enums(&mut value, false).unwrap();
        assert_eq!(value["transport"]["duration_ms"], "922337203685477");
        assert_eq!(value["failure_code"], Value::Null);
        assert_eq!(value["tracks"][0]["kind"], "subtitle");
        assert!(enums(&mut serde_json::json!({"state": 99}), false).is_err());
        assert!(enums(&mut serde_json::json!({"kind": "provider_private"}), true).is_err());
    }
    #[test]
    fn timelines_are_bounded_decimal_strings_and_budgets_are_independent() {
        assert!(milliseconds("922337203685477").is_ok());
        for value in ["922337203685478", "-1", "1.0", "1e3", ""] {
            assert!(milliseconds(value).is_err());
        }
        assert_eq!(budget("playback_create"), Some(30_000));
        assert_eq!(budget("playback_report"), Some(10_000));
        assert_eq!(budget("get_item"), None);
    }
    #[test]
    fn error_redaction_and_ambiguity_precede_provider_text() {
        assert!(matches!(
            safe_status(tonic::Status::failed_precondition("profile_changed")),
            ClientError::Playback {
                code: PlaybackErrorCode::ProfileChanged
            }
        ));
        assert!(matches!(
            safe_status(tonic::Status::deadline_exceeded("upstream_unavailable")),
            ClientError::DeadlineExceeded
        ));
        let error = safe_status(tonic::Status::internal("http://private/?api_key=secret"));
        assert!(!error.to_string().contains("private"));
        assert!(!error.to_string().contains("secret"));
    }
    #[test]
    fn unavailable_prerequisite_preserves_only_exact_safe_application_codes() {
        assert!(matches!(
            safe_status(tonic::Status::unavailable(
                "playback_authorization_unavailable"
            )),
            ClientError::Playback {
                code: PlaybackErrorCode::PlaybackAuthorizationUnavailable
            }
        ));
        for message in [
            "private transport failure",
            "http://private/?api_key=secret",
            "playback_authorization_unavailable: secret",
        ] {
            let error = safe_status(tonic::Status::unavailable(message));
            assert!(matches!(error, ClientError::Unavailable { .. }));
            assert_eq!(
                error.to_string(),
                "unavailable: playback service unavailable"
            );
        }
        assert!(matches!(
            safe_status(tonic::Status::cancelled(
                "playback_authorization_unavailable"
            )),
            ClientError::Cancelled
        ));
    }
    #[test]
    fn gateway_urls_reject_other_origins_paths_control_authority_and_credentials() {
        let client = Client::new(ClientConfig {
            endpoint: "http://localhost:2000".into(),
            user_id: String::new(),
            timeout_ms: 3000,
        })
        .unwrap();
        let session = PlaybackSession {
            session_id: "11111111-1111-4111-8111-111111111111".into(),
            control_capability: "a".repeat(64),
            transport_revision: 3,
            ..Default::default()
        };
        let path = format!(
            "/playback/v1/media/{}/3/22222222-2222-4222-8222-222222222222?cap={}",
            session.session_id,
            "b".repeat(64)
        );
        assert!(
            client
                .validate_playback_url(&format!("http://localhost:2000{path}"), &session)
                .is_ok()
        );
        for value in [
            format!("http://localhost:2001{path}"),
            format!("http://user@localhost:2000{path}"),
            format!("http://localhost:2000{path}&extra=1"),
            format!(
                "http://localhost:2000{}",
                path.replace(&"b".repeat(64), &"a".repeat(64))
            ),
            format!("http://localhost:2000{}", path.replace("/3/", "/2/")),
        ] {
            assert!(client.validate_playback_url(&value, &session).is_err());
        }
    }
}
