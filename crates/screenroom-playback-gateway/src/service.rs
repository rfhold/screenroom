use crate::{
    fences::OperationFence,
    records::{self, Descriptor, Replay, TrackRecord},
    validation as v,
};
use screenroom_backend::{
    Uuid,
    connections::{ConnectionRepository, CredentialDirectory},
    playback::{
        self as db, Authority, Operation, Outcome, PlaybackRepository, Reservation, Session,
        secrets::{Capability, PlaybackSecrets},
    },
};
use screenroom_catalog_adapter::jellyfin::{LiveJellyfin, playback as provider};
use screenroom_proto::playback_gateway::v1::*;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Weak, atomic::Ordering},
    time::Duration,
};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tonic::{Request, Response, Status};

pub(crate) struct TransportRuntime {
    pub provider: LiveJellyfin,
    pub intent: provider::AuthorizationIntent,
    pub auth: Arc<provider::SessionAuth>,
    pub negotiated: Arc<provider::NegotiatedPlayback>,
    pub tracks: Vec<(String, provider::Track)>,
    pub track_resources: HashMap<String, Uuid>,
    pub root: Uuid,
    pub media_capability: String,
    pub resources: RwLock<crate::signed_resources::ResourceGraph>,
    pub signing_key: crate::signed_resources::SigningKey,
    pub playlists: Mutex<crate::signed_resources::PlaylistState>,
}

type Transports = Arc<RwLock<HashMap<(Uuid, u32), Arc<TransportRuntime>>>>;

#[derive(Clone)]
pub struct PlaybackService {
    pub(crate) repository: PlaybackRepository,
    pub(crate) connections: ConnectionRepository,
    pub(crate) credentials: CredentialDirectory,
    pub(crate) secrets: PlaybackSecrets,
    pub(crate) origin: String,
    pub(crate) transports: Transports,
    pub(crate) locks: Arc<Mutex<HashMap<Uuid, Weak<Mutex<()>>>>>,
    pub(crate) fetches: Arc<Semaphore>,
    pub(crate) stopping: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) ownership: Option<Arc<crate::runtime::RuntimeOwnership>>,
    pub(crate) controls: Arc<Semaphore>,
    pub(crate) owned_tasks: Arc<crate::owned::OwnedTasks>,
    owned_operation: Option<crate::owned::OperationSlot>,
}

impl PlaybackService {
    pub(crate) fn new(
        repository: PlaybackRepository,
        connections: ConnectionRepository,
        credentials: CredentialDirectory,
        secrets: PlaybackSecrets,
        api_origin: String,
    ) -> Result<Self, Status> {
        crate::transport::validate_origin(&api_origin)?;
        Ok(Self {
            repository,
            connections,
            credentials,
            secrets,
            origin: api_origin.trim_end_matches('/').into(),
            transports: Default::default(),
            locks: Default::default(),
            fetches: Arc::new(Semaphore::new(32)),
            stopping: Default::default(),
            ownership: None,
            controls: Arc::new(Semaphore::new(32)),
            owned_tasks: Default::default(),
            owned_operation: None,
        })
    }

    async fn lock(&self, id: Uuid) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        if let Some(lock) = locks.get(&id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(id, Arc::downgrade(&lock));
        lock
    }

    fn own(&self, operation: &Operation) {
        if let Some(owned) = &self.owned_operation {
            *owned.lock().expect("operation slot") = Some(operation.clone());
        }
    }

    fn read_replay(&self, id: Uuid) -> Result<Replay, Status> {
        serde_json::from_str(&self.secrets.read(id).map_err(records::database)?)
            .map_err(|_| records::unavailable())
    }

    fn write_replay(&self, id: Uuid, replay: &Replay) -> Result<(), Status> {
        if replay.quick_connect_secret.is_some() {
            return Err(records::unavailable());
        }
        let value = serde_json::to_string(replay).map_err(|_| records::unavailable())?;
        if value.len() > 4096 {
            return Err(records::unavailable());
        }
        self.secrets.publish(id, &value).map_err(records::database)
    }

    pub(crate) async fn view(
        &self,
        user: Uuid,
        id: Uuid,
        control: &str,
    ) -> Result<PlaybackSession, Status> {
        let view = self
            .repository
            .view(&Authority {
                user_id: user,
                session_id: id,
                control_capability: control,
            })
            .await
            .map_err(records::database)?;
        let replay = self.read_replay(view.session.replay_ref)?;
        let mut public = records::public(
            &view.session,
            view.latest_operation.as_ref(),
            &replay,
            &self.origin,
        )?;
        let (pending, failed) = self
            .repository
            .cleanup_status(id)
            .await
            .map_err(records::database)?;
        public.cleanup_pending = pending;
        if failed {
            public.failure_code = Some(12);
        }
        Ok(public)
    }

    async fn receipt(
        &self,
        user: Uuid,
        operation: &Operation,
        control: &str,
    ) -> Result<PlaybackSession, Status> {
        let mut session = self
            .repository
            .get(&Authority {
                user_id: user,
                session_id: operation.session_id,
                control_capability: control,
            })
            .await
            .map_err(records::database)?;
        let operation = self
            .repository
            .operation_receipt(operation)
            .await
            .map_err(records::database)?;
        if let Some(snapshot) = operation.result.as_ref().and_then(|r| r.get("snapshot")) {
            if (matches!(operation.kind.as_str(), "create" | "update")
                || operation.status == "failed")
                && let Some(code) = operation
                    .result
                    .as_ref()
                    .and_then(|r| r.get("outcome"))
                    .and_then(|v| v.get("failure"))
                    .and_then(Value::as_i64)
            {
                return Err(records::failure(
                    i32::try_from(code).map_err(|_| records::unavailable())?,
                ));
            }
            let snapshot: db::ReceiptSnapshot =
                serde_json::from_value(snapshot.clone()).map_err(|_| records::unavailable())?;
            snapshot.restore_view(&mut session);
            let replay = self.read_replay(operation.replay_ref.unwrap_or(session.replay_ref))?;
            let mut public = records::public(&session, Some(&operation), &replay, &self.origin)?;
            public.cleanup_pending = operation
                .result
                .as_ref()
                .and_then(|r| r["cleanup_pending"].as_bool())
                .unwrap_or(public.cleanup_pending);
            if operation
                .result
                .as_ref()
                .is_some_and(|r| r["cleanup_failed"] == true)
            {
                public.failure_code = Some(12);
            }
            Ok(public)
        } else {
            self.view(user, operation.session_id, control).await
        }
    }

    async fn assembly(
        &self,
        session: &Session,
        operation: &Operation,
    ) -> Result<
        (
            LiveJellyfin,
            provider::AuthorizationIntent,
            Arc<OperationFence>,
            provider::PlaybackAdapter,
        ),
        Status,
    > {
        // Resolving managed configuration is local-only. All provider requests use the fenced adapter.
        let resolved = self
            .connections
            .resolve(&session.source_key)
            .await
            .map_err(records::database)?;
        if resolved.id() != session.connection_id {
            return Err(Status::failed_precondition("profile_changed"));
        }
        let (provider, server) = self
            .repository
            .provider_configuration(session, &self.credentials)
            .await
            .map_err(records::database)?;
        let intent = provider::AuthorizationIntent::new(
            provider::GatewayDeviceId::from_recorded(session.device_id.clone())
                .map_err(|_| records::unavailable())?,
            server.clone(),
            session.remote_profile_id.clone(),
            "",
        )
        .map_err(|_| records::unavailable())?;
        let fence = Arc::new(OperationFence::new(
            self.repository.clone(),
            operation.clone(),
            server,
            self.ownership.clone(),
        ));
        let adapter = provider
            .playback_adapter_fenced(
                intent.clone(),
                provider::PlaybackTimeouts::default(),
                fence.clone(),
            )
            .map_err(|_| records::unavailable())?;
        Ok((provider, intent, fence, adapter))
    }

    async fn fail(&self, operation: &Operation, _dispatched: bool, code: i32) {
        opentelemetry::global::meter("screenroom-playback-gateway")
            .u64_counter("playback.failures")
            .build()
            .add(
                1,
                &[opentelemetry::KeyValue::new(
                    "failure",
                    records::failure(code).message().to_owned(),
                )],
            );
        if self
            .repository
            .finish_failure_recorded(operation, code)
            .await
            .is_err()
        {
            tracing::warn!(
                error_code = "operation_finalize_failed",
                "playback operation finalization failed"
            );
        }
    }

    async fn publish(
        &self,
        session: &Session,
        operation: &Operation,
        runtime: Arc<TransportRuntime>,
        replay: &Replay,
        compatible_used: bool,
        state: &str,
    ) -> Result<(), Status> {
        let negotiated = &runtime.negotiated;
        let revision = u32::try_from(session.transport_revision)
            .ok()
            .and_then(|r| r.checked_add(1))
            .ok_or_else(v::invalid)?;
        let resources = runtime.resources.read().await;
        let root = runtime.root;
        let mut tracks = Vec::new();
        for (id, track) in &runtime.tracks {
            let resource = runtime.track_resources.get(id).copied();
            tracks.push(TrackRecord {
                id: id.clone(),
                kind: if track.kind == provider::TrackKind::Audio {
                    1
                } else {
                    2
                },
                display_name: track.display_name.clone(),
                language: track.language.clone(),
                selected: track.selected,
                available: track.available,
                subtitle_mode: track.subtitle_mode.map(|m| match m {
                    provider::SubtitleMode::WebVtt => 1,
                    provider::SubtitleMode::BurnIn => 2,
                    provider::SubtitleMode::Unavailable => 3,
                }),
                resource,
            });
        }
        let descriptor = Descriptor {
            method: match negotiated.method {
                provider::PlaybackMethod::DirectPlay => 1,
                provider::PlaybackMethod::DirectStream => 2,
                provider::PlaybackMethod::Transcode => 3,
            },
            mime_type: negotiated.mime_type.into(),
            root,
            source: Uuid::new_v4().to_string(),
            duration_ms: negotiated.duration.map(|d| d.decimal()),
            start_position_ms: negotiated.start_position.decimal(),
            timeline_offset_ms: negotiated.timeline_offset.decimal(),
            compatible_used,
        };
        let descriptor = serde_json::to_value(descriptor).map_err(|_| records::unavailable())?;
        let tracks = serde_json::to_value(tracks).map_err(|_| records::unavailable())?;
        let durable: Vec<_> = resources
            .durable_ids()
            .map(|id| db::Resource {
                id: *id,
                session_id: session.id,
                transport_revision: i64::from(revision),
                source_item_id: session.source_item_id,
                media_source_id: negotiated.private_source_id().into(),
                private_mapping: json!({"runtime_resource":id}),
            })
            .collect();
        self.repository
            .publish_transport(
                operation,
                db::Transport {
                    play_session_id: negotiated.private_play_session_id(),
                    media_hash: &db::secrets::capability_hash(&replay.media),
                    tracks: &tracks,
                    descriptor: &descriptor,
                    resources: &durable,
                },
            )
            .await
            .map_err(records::database)?;
        drop(resources);
        self.transports
            .write()
            .await
            .insert((session.id, revision), runtime);
        self.repository
            .finish_recorded(
                operation,
                Outcome::Confirmed(&json!({"transport_revision":revision})),
                state,
            )
            .await
            .map_err(records::database)?;
        self.transports
            .write()
            .await
            .retain(|(id, rev), _| *id != session.id || *rev == revision);
        Ok(())
    }

    fn runtime(
        provider: LiveJellyfin,
        intent: provider::AuthorizationIntent,
        auth: Arc<provider::SessionAuth>,
        negotiated: provider::NegotiatedPlayback,
        media_capability: String,
    ) -> Result<Arc<TransportRuntime>, Status> {
        let signing_key =
            crate::signed_resources::SigningKey::generate().map_err(|_| records::unavailable())?;
        let mut resources = HashMap::new();
        let root = Uuid::new_v4();
        resources.insert(root, negotiated.resource.clone());
        let mut tracks = Vec::new();
        let mut track_resources = HashMap::new();
        for track in &negotiated.tracks {
            let id = Uuid::new_v4().to_string();
            if let Some(resource) = &track.resource {
                let resource_id = Uuid::new_v4();
                resources.insert(resource_id, resource.clone());
                track_resources.insert(id.clone(), resource_id);
            }
            tracks.push((id, track.clone()));
        }
        let resources = crate::signed_resources::ResourceGraph::new(resources)
            .map_err(|_| records::unavailable())?;
        Ok(Arc::new(TransportRuntime {
            provider,
            intent,
            auth,
            negotiated: Arc::new(negotiated),
            tracks,
            track_resources,
            root,
            media_capability,
            resources: RwLock::new(resources),
            signing_key,
            playlists: Default::default(),
        }))
    }

    async fn create(
        &self,
        user: Uuid,
        request: CreatePlaybackSessionRequest,
    ) -> Result<PlaybackSession, Status> {
        let scope = v::scope(request.scope.clone())?;
        let caps = request.client_capabilities.as_ref().unwrap();
        let capabilities = json!({"mp4":caps.mp4,"h264":caps.h264,"aac":caps.aac,"native_hls":caps.native_hls,"mse_hls":caps.mse_hls,"webvtt":caps.webvtt});
        let normalized = json!({"start_policy":request.start_policy,"position_ms":request.position_ms.as_deref().map(v::position).transpose()?.map(|p|p.decimal())});
        let control = Capability::generate().map_err(records::database)?;
        let media = Capability::generate().map_err(records::database)?;
        let created = self
            .repository
            .create(db::Create {
                user_id: user,
                public_item_id: &request.item_id,
                scope: &scope,
                request_id: v::uuid(&request.request_id)?,
                normalized_body: &normalized,
                capabilities: &capabilities,
                control_hash: &control.hash(),
            })
            .await
            .map_err(records::database)?;
        if created.replay {
            let lock = self.lock(created.session.id).await;
            let _locked = lock.lock().await;
            let replay = self.read_replay(
                created
                    .operation
                    .replay_ref
                    .ok_or_else(records::unavailable)?,
            );
            let replay = match replay {
                Ok(replay) => replay,
                Err(_) => {
                    // No replacement authority: an unpublished, never-dispatched
                    // Create can only become a terminal failed receipt.
                    if created.operation.status == "pending" {
                        let _ = self
                            .repository
                            .finish_recorded(
                                &created.operation,
                                Outcome::NotSent(&json!({"failure":12})),
                                "failed",
                            )
                            .await;
                    }
                    let operation = self
                        .repository
                        .operation_receipt(&created.operation)
                        .await
                        .map_err(records::database)?;
                    if created.session.transport_revision == 0
                        && matches!(operation.status.as_str(), "failed" | "uncertain")
                    {
                        return Err(records::failure(12));
                    }
                    // Publication/dispatch may have won the race with terminalization.
                    self.read_replay(created.session.replay_ref)?
                }
            };
            return self
                .receipt(user, &created.operation, &replay.control)
                .await;
        }
        let replay = Replay {
            control: control.expose().into(),
            media: media.expose().into(),
            quick_connect_secret: None,
        };
        let session = &created.session;
        let operation = &created.operation;
        // Synchronous, immutable publication is before every provider request and
        // before the next cancellation point after learning the committed receipt.
        self.own(operation);
        self.write_replay(session.replay_ref, &replay)?;
        let lock = self.lock(session.id).await;
        let _locked = lock.lock().await;
        let assembly = self.assembly(session, operation).await;
        let (provider, intent, fence, adapter) = match assembly {
            Ok(value) => value,
            Err(error) => {
                self.fail(operation, false, 12).await;
                let _ = error;
                return self.receipt(user, operation, &replay.control).await;
            }
        };
        let run = async {
            let pending = adapter
                .initiate(&intent)
                .await
                .map_err(|e| (records::provider_code(&e), records::unavailable()))?;
            let quick_connect_ref = session
                .capabilities
                .get("_quick_connect_ref")
                .and_then(Value::as_str)
                .ok_or_else(|| (12, records::unavailable()))?;
            let quick_connect_ref =
                Uuid::parse_str(quick_connect_ref).map_err(|_| (12, records::unavailable()))?;
            self.secrets
                .publish(quick_connect_ref, pending.secret_for_private_store())
                .map_err(|e| (12, records::database(e)))?;
            let authorized = adapter
                .authorize(pending)
                .await
                .map_err(|e| (records::provider_code(&e), records::unavailable()))?;
            let auth = Arc::new(
                adapter
                    .exchange(authorized)
                    .await
                    .map_err(|e| (records::provider_code(&e), records::unavailable()))?,
            );
            self.secrets
                .publish(
                    session.credential_ref,
                    auth.token().expose_for_private_store(),
                )
                .map_err(|e| (12, records::database(e)))?;
            let position = match request.start_policy {
                1 => adapter
                    .resume_position(&auth, &session.external_item_id)
                    .await
                    .map_err(|e| (records::provider_code(&e), records::unavailable()))?,
                2 => provider::Milliseconds::ZERO,
                _ => v::position(request.position_ms.as_deref().unwrap()).map_err(|e| (5, e))?,
            };
            let negotiated = adapter
                .negotiate(
                    &auth,
                    provider::NegotiationRequest {
                        item_id: &session.external_item_id,
                        capabilities: provider_caps(caps),
                        position,
                        selection: None,
                    },
                )
                .await
                .map_err(|e| (records::provider_code(&e), records::unavailable()))?;
            let runtime = Self::runtime(provider, intent, auth, negotiated, replay.media.clone())
                .map_err(|error| (12, error))?;
            self.publish(session, operation, runtime, &replay, false, "ready")
                .await
                .map_err(|e| (12, e))
        };
        let result = tokio::time::timeout(Duration::from_secs(30), run).await;
        match result {
            Ok(Ok(())) => {}
            result => {
                // Immutable publication may already have succeeded; never overwrite or remove it.
                if self.secrets.read(session.replay_ref).is_err() {
                    let _ = self.write_replay(session.replay_ref, &replay);
                }
                let (code, error) = match result {
                    Err(_) => (12, Status::deadline_exceeded("upstream_unavailable")),
                    Ok(Err(error)) => error,
                    Ok(Ok(())) => unreachable!(),
                };
                self.fail(operation, fence.dispatched.load(Ordering::Acquire), code)
                    .await;
                let _ = error;
                return self.receipt(user, operation, &replay.control).await;
            }
        }
        self.receipt(user, operation, &replay.control).await
    }

    async fn update(
        &self,
        user: Uuid,
        request: UpdatePlaybackSessionRequest,
    ) -> Result<PlaybackSession, Status> {
        let id = v::uuid(&request.session_id)?;
        let lock = self.lock(id).await;
        let _locked = lock.lock().await;
        let authority = Authority {
            user_id: user,
            session_id: id,
            control_capability: &request.control_capability,
        };
        let session = self
            .repository
            .get(&authority)
            .await
            .map_err(records::database)?;
        let choices = request.track_choices.clone().unwrap_or_default();
        let fallback = request.compatible_transcode_fallback.unwrap_or(false);
        let body = json!({"position_ms":v::position(&request.position_ms)?.decimal(),"audio_track_id":choices.audio_track_id,"subtitle_track_id":choices.subtitle_track_id,"subtitles_off":choices.subtitles_off,"compatible_transcode_fallback":fallback});
        let operation = match self
            .repository
            .reserve(
                &authority,
                db::Mutation::Update {
                    request_id: v::uuid(&request.request_id)?,
                },
                request.expected_transport_revision,
                &body,
                false,
            )
            .await
            .map_err(records::database)?
        {
            Reservation::New(operation) => operation,
            Reservation::Replay(operation) => {
                return self
                    .receipt(user, &operation, &request.control_capability)
                    .await;
            }
            Reservation::Older { .. } => return Err(v::invalid()),
        };
        self.own(&operation);
        let (_, compatible_reserved) = self
            .repository
            .orchestration_history(&operation)
            .await
            .map_err(records::database)?;
        if fallback && compatible_reserved {
            self.fail(&operation, false, 6).await;
            return self
                .receipt(user, &operation, &request.control_capability)
                .await;
        }
        let runtime = self
            .transports
            .read()
            .await
            .get(&(id, request.expected_transport_revision))
            .cloned();
        let Some(runtime) = runtime else {
            self.fail(&operation, false, 9).await;
            return self
                .receipt(user, &operation, &request.control_capability)
                .await;
        };
        let select = |id: Option<&str>, kind| -> Result<Option<&provider::Track>, Status> {
            id.map(|id| {
                runtime
                    .tracks
                    .iter()
                    .find(|(choice, track)| choice == id && track.kind == kind && track.available)
                    .map(|(_, track)| track)
                    .ok_or_else(v::invalid)
            })
            .transpose()
        };
        let selection = (|| {
            let audio = select(
                choices.audio_track_id.as_deref(),
                provider::TrackKind::Audio,
            )?;
            let subtitle = if choices.subtitles_off {
                None
            } else if choices.subtitle_track_id.is_some() {
                select(
                    choices.subtitle_track_id.as_deref(),
                    provider::TrackKind::Subtitle,
                )?
            } else {
                runtime
                    .tracks
                    .iter()
                    .find(|(_, t)| t.kind == provider::TrackKind::Subtitle && t.selected)
                    .map(|(_, t)| t)
            };
            runtime
                .negotiated
                .selection(audio, subtitle)
                .map_err(|_| v::invalid())
        })();
        let selection = match selection {
            Ok(v) => v,
            Err(_) => {
                self.fail(&operation, false, 5).await;
                return self
                    .receipt(user, &operation, &request.control_capability)
                    .await;
            }
        };
        let (provider, intent, fence, adapter) = self.assembly(&session, &operation).await?;
        let replay = Replay {
            control: request.control_capability.clone(),
            media: Capability::generate()
                .map_err(records::database)?
                .expose()
                .into(),
            quick_connect_secret: None,
        };
        self.write_replay(
            operation.replay_ref.ok_or_else(records::unavailable)?,
            &replay,
        )?;
        let caps: Value = session.capabilities.clone();
        let caps = provider::ClientCapabilities {
            mp4: caps["mp4"] == true,
            h264: caps["h264"] == true,
            aac: caps["aac"] == true,
            native_hls: caps["native_hls"] == true,
            mse_hls: caps["mse_hls"] == true,
            webvtt: caps["webvtt"] == true,
        };
        let provider_failure = std::sync::atomic::AtomicI32::new(12);
        let run = async {
            let negotiation = provider::NegotiationRequest {
                item_id: &session.external_item_id,
                capabilities: caps,
                position: v::position(&request.position_ms)?,
                selection: Some(&selection),
            };
            let negotiated = if fallback {
                let permit = provider::NegotiationBudget::from_recorded(false)
                    .reserve_compatible()
                    .map_err(|_| v::invalid())?;
                adapter
                    .compatible_transcode(&runtime.auth, negotiation, permit)
                    .await
            } else {
                adapter.negotiate(&runtime.auth, negotiation).await
            }
            .map_err(|error| {
                provider_failure.store(
                    if fallback && error.failure == provider::Failure::Unsupported {
                        6
                    } else {
                        records::provider_code(&error)
                    },
                    Ordering::Release,
                );
                records::unavailable()
            })?;
            let replacement = Self::runtime(
                provider,
                intent,
                runtime.auth.clone(),
                negotiated,
                replay.media.clone(),
            )?;
            let descriptor: Descriptor =
                serde_json::from_value(session.transport.clone().ok_or_else(records::unavailable)?)
                    .map_err(|_| records::unavailable())?;
            self.publish(
                &session,
                &operation,
                replacement,
                &replay,
                fallback || descriptor.compatible_used,
                &session.state,
            )
            .await
        };
        match tokio::time::timeout(Duration::from_secs(10), run).await {
            Ok(Ok(())) => {
                self.receipt(user, &operation, &request.control_capability)
                    .await
            }
            result => {
                self.fail(
                    &operation,
                    fence.dispatched.load(Ordering::Acquire),
                    provider_failure.load(Ordering::Acquire),
                )
                .await;
                let _ = result;
                self.receipt(user, &operation, &request.control_capability)
                    .await
            }
        }
    }

    async fn report(
        &self,
        user: Uuid,
        request: ReportPlaybackEventRequest,
    ) -> Result<ReportPlaybackEventResponse, Status> {
        let id = v::uuid(&request.session_id)?;
        let lock = self.lock(id).await;
        let _locked = lock.lock().await;
        let authority = Authority {
            user_id: user,
            session_id: id,
            control_capability: &request.control_capability,
        };
        let current = self
            .repository
            .get(&authority)
            .await
            .map_err(records::database)?;
        if current.state == "uncertain" && i64::from(request.sequence) > current.last_sequence {
            return Err(Status::failed_precondition("outcome_uncertain"));
        }
        let event = db::EventBody {
            kind: match request.kind {
                1 => db::EventKind::Started,
                2 => db::EventKind::Progress,
                3 => db::EventKind::Paused,
                4 => db::EventKind::Resumed,
                5 => db::EventKind::Seeked,
                _ => db::EventKind::Heartbeat,
            },
            position_ms: v::position(&request.position_ms)?.decimal(),
            paused: request.paused,
            muted: request.muted,
            volume: Some((request.volume * 100.0).round() as u32),
        };
        let runtime = self
            .transports
            .read()
            .await
            .get(&(id, request.transport_revision))
            .cloned();
        let heartbeat_local = request.kind == 6
            && runtime
                .as_ref()
                .is_some_and(|r| r.negotiated.method == provider::PlaybackMethod::DirectPlay);
        let reservation = if heartbeat_local {
            self.repository
                .heartbeat_recorded(
                    &authority,
                    request.sequence,
                    request.transport_revision,
                    &event,
                )
                .await
        } else {
            self.repository
                .reserve_event(
                    &authority,
                    request.sequence,
                    request.transport_revision,
                    &event,
                )
                .await
        }
        .map_err(records::database)?;
        let receipt = match &reservation {
            Reservation::New(op) | Reservation::Replay(op) => Some(op.clone()),
            _ => None,
        };
        if let Reservation::New(operation) = reservation {
            self.own(&operation);
            if !heartbeat_local {
                let session = self
                    .repository
                    .get(&authority)
                    .await
                    .map_err(records::database)?;
                let Some(runtime) = runtime else {
                    self.fail(&operation, false, 9).await;
                    return Err(Status::failed_precondition("session_expired"));
                };
                if request.kind == 1
                    && self
                        .repository
                        .orchestration_history(&operation)
                        .await
                        .map_err(records::database)?
                        .0
                {
                    self.fail(&operation, false, 10).await;
                    return Err(Status::already_exists("operation_conflict"));
                }
                let (_, _, fence, adapter) = self.assembly(&session, &operation).await?;
                let run = async {
                    if request.kind == 6 {
                        adapter
                            .ping_encoding(&runtime.auth, &runtime.negotiated)
                            .await
                    } else {
                        adapter
                            .report(
                                &runtime.auth,
                                &runtime.negotiated,
                                provider::PlaybackReport {
                                    kind: if request.kind == 1 {
                                        provider::ReportKind::Start
                                    } else {
                                        provider::ReportKind::Progress
                                    },
                                    position: v::position(&request.position_ms).unwrap(),
                                    paused: request.paused,
                                    muted: request.muted,
                                    volume: request.volume,
                                },
                            )
                            .await
                    }
                };
                match tokio::time::timeout(Duration::from_secs(10), run).await {
                    Ok(Ok(())) => self
                        .repository
                        .finish_recorded(
                            &operation,
                            Outcome::Confirmed(&json!({})),
                            if request.paused { "paused" } else { "playing" },
                        )
                        .await
                        .map_err(records::database)?,
                    _ => {
                        self.fail(&operation, fence.dispatched.load(Ordering::Acquire), 12)
                            .await
                    }
                }
            }
        }
        let session = if let Some(operation) = receipt {
            self.receipt(user, &operation, &request.control_capability)
                .await?
        } else {
            self.view(user, id, &request.control_capability).await?
        };
        Ok(ReportPlaybackEventResponse {
            accepted_sequence: session.last_accepted_sequence,
            report_outcome: session.report_outcome,
            session: Some(session),
        })
    }

    async fn stop(
        &self,
        user: Uuid,
        request: StopPlaybackSessionRequest,
    ) -> Result<PlaybackSession, Status> {
        let id = v::uuid(&request.session_id)?;
        let lock = self.lock(id).await;
        let _locked = lock.lock().await;
        let authority = Authority {
            user_id: user,
            session_id: id,
            control_capability: &request.control_capability,
        };
        let session = self
            .repository
            .get(&authority)
            .await
            .map_err(records::database)?;
        let body = json!({"position_ms":v::position(&request.position_ms)?.decimal(),"reason":request.stop_reason});
        let reservation = self
            .repository
            .reserve(
                &authority,
                db::Mutation::Stop {
                    request_id: v::uuid(&request.request_id)?,
                    final_sequence: request.final_sequence,
                },
                request.transport_revision,
                &body,
                true,
            )
            .await
            .map_err(records::database)?;
        let receipt = match &reservation {
            Reservation::New(op) | Reservation::Replay(op) => Some(op.clone()),
            _ => None,
        };
        if let Reservation::New(operation) = reservation {
            self.own(&operation);
            let runtime = self
                .transports
                .write()
                .await
                .remove(&(id, request.transport_revision));
            let Some(runtime) = runtime else {
                self.fail(&operation, false, 9).await;
                return self.view(user, id, &request.control_capability).await;
            };
            let (_, _, fence, adapter) = self.assembly(&session, &operation).await?;
            match tokio::time::timeout(
                Duration::from_secs(10),
                adapter.report(
                    &runtime.auth,
                    &runtime.negotiated,
                    provider::PlaybackReport {
                        kind: provider::ReportKind::Stopped,
                        position: v::position(&request.position_ms)?,
                        paused: false,
                        muted: false,
                        volume: 1.0,
                    },
                ),
            )
            .await
            {
                Ok(Ok(())) => self
                    .repository
                    .finish_recorded(&operation, Outcome::Confirmed(&json!({})), "stopping")
                    .await
                    .map_err(records::database)?,
                _ => {
                    self.fail(&operation, fence.dispatched.load(Ordering::Acquire), 12)
                        .await
                }
            }
        }
        if let Some(operation) = receipt {
            self.receipt(user, &operation, &request.control_capability)
                .await
        } else {
            self.view(user, id, &request.control_capability).await
        }
    }

    async fn supervised<T, F, Work>(&self, duration: Duration, task: Work) -> Result<T, Status>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, Status>> + Send + 'static,
        Work: FnOnce(Self) -> F + Send + 'static,
    {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Status::unavailable("gateway_stopping"));
        }
        let permit = self
            .controls
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("capacity_exhausted"))?;
        let owned = Arc::new(std::sync::Mutex::new(None));
        let mut service = self.clone();
        service.owned_operation = Some(owned.clone());
        let repository = self.repository.clone();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        // Registration and shutdown admission are atomic across every service clone.
        self.owned_tasks
            .spawn(owned.clone(), move || async move {
                let _permit = permit;
                use futures_util::FutureExt;
                let work = std::panic::AssertUnwindSafe(task(service)).catch_unwind();
                let result = match tokio::time::timeout(duration, work).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err(records::unavailable()),
                    Err(_) => Err(Status::deadline_exceeded("upstream_unavailable")),
                };
                let operation = owned.lock().expect("operation slot").clone();
                if let Some(operation) = operation
                    && repository.finalize_abandoned(&operation).await.is_err()
                {
                    tracing::warn!(
                        error_code = "operation_finalize_failed",
                        "playback operation finalization failed"
                    );
                }
                let _ = result_tx.send(result);
            })
            .await?;
        tokio::time::timeout(duration, result_rx)
            .await
            .map_err(|_| Status::deadline_exceeded("upstream_unavailable"))?
            .map_err(|_| records::unavailable())?
    }
}

fn provider_caps(caps: &PlaybackClientCapabilities) -> provider::ClientCapabilities {
    provider::ClientCapabilities {
        mp4: caps.mp4,
        h264: caps.h264,
        aac: caps.aac,
        native_hls: caps.native_hls,
        mse_hls: caps.mse_hls,
        webvtt: caps.webvtt,
    }
}

#[tonic::async_trait]
impl playback_service_server::PlaybackService for PlaybackService {
    async fn create_playback_session(
        &self,
        request: Request<CreatePlaybackSessionRequest>,
    ) -> Result<Response<CreatePlaybackSessionResponse>, Status> {
        let duration = v::budget(&request, Duration::from_secs(30));
        let user = v::selector(&request)?;
        let request = request.into_inner();
        v::create(&request)?;
        self.supervised(duration, move |service| async move {
            Ok(CreatePlaybackSessionResponse {
                session: Some(service.create(user, request).await?),
            })
        })
        .await
        .map(Response::new)
    }
    async fn get_playback_session(
        &self,
        request: Request<GetPlaybackSessionRequest>,
    ) -> Result<Response<GetPlaybackSessionResponse>, Status> {
        let duration = v::budget(&request, Duration::from_secs(10));
        let user = v::selector(&request)?;
        let request = request.into_inner();
        let id = v::uuid(&request.session_id)?;
        v::capability(&request.control_capability)?;
        let session =
            tokio::time::timeout(duration, self.view(user, id, &request.control_capability))
                .await
                .map_err(|_| records::unavailable())??;
        Ok(Response::new(GetPlaybackSessionResponse {
            session: Some(session),
        }))
    }
    async fn update_playback_session(
        &self,
        request: Request<UpdatePlaybackSessionRequest>,
    ) -> Result<Response<UpdatePlaybackSessionResponse>, Status> {
        let duration = v::budget(&request, Duration::from_secs(10));
        let user = v::selector(&request)?;
        let request = request.into_inner();
        v::uuid(&request.session_id)?;
        v::uuid(&request.request_id)?;
        v::capability(&request.control_capability)?;
        v::position(&request.position_ms)?;
        if request.expected_transport_revision == 0 {
            return Err(v::invalid());
        }
        v::choices(&request.track_choices.clone().unwrap_or_default())?;
        self.supervised(duration, move |service| async move {
            Ok(UpdatePlaybackSessionResponse {
                session: Some(service.update(user, request).await?),
            })
        })
        .await
        .map(Response::new)
    }
    async fn report_playback_event(
        &self,
        request: Request<ReportPlaybackEventRequest>,
    ) -> Result<Response<ReportPlaybackEventResponse>, Status> {
        let duration = v::budget(&request, Duration::from_secs(10));
        let user = v::selector(&request)?;
        let request = request.into_inner();
        v::event(&request)?;
        self.supervised(duration, move |service| async move {
            service.report(user, request).await
        })
        .await
        .map(Response::new)
    }
    async fn stop_playback_session(
        &self,
        request: Request<StopPlaybackSessionRequest>,
    ) -> Result<Response<StopPlaybackSessionResponse>, Status> {
        let duration = v::budget(&request, Duration::from_secs(10));
        let user = v::selector(&request)?;
        let request = request.into_inner();
        v::uuid(&request.session_id)?;
        v::uuid(&request.request_id)?;
        v::capability(&request.control_capability)?;
        v::position(&request.position_ms)?;
        v::reason(&request.stop_reason)?;
        if request.transport_revision == 0 {
            return Err(v::invalid());
        }
        self.supervised(duration, move |service| async move {
            Ok(StopPlaybackSessionResponse {
                session: Some(service.stop(user, request).await?),
            })
        })
        .await
        .map(Response::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropping_rpc_waiter_does_not_drop_owned_mutation() {
        let (service, pool) = crate::tests::service();
        let shutdown_service = service.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        let (finished, done) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            service
                .supervised(Duration::from_secs(2), move |_| async move {
                    started.send(()).unwrap();
                    resume.await.unwrap();
                    finished.send(()).unwrap();
                    Ok::<_, Status>(())
                })
                .await
        });
        ready.await.unwrap();
        waiter.abort();
        let _ = waiter.await;
        let (drained, drain_result) = tokio::sync::oneshot::channel();
        let shutdown = tokio::spawn(async move {
            shutdown_service.owned_tasks.close().await;
            shutdown_service.stopping.store(true, Ordering::Release);
            let forced = shutdown_service
                .owned_tasks
                .drain(Duration::from_secs(1))
                .await;
            assert!(forced.is_empty());
            drained.send(()).unwrap();
        });
        tokio::task::yield_now().await;
        assert!(!shutdown.is_finished());
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), done)
            .await
            .unwrap()
            .unwrap();
        drain_result.await.unwrap();
        shutdown.await.unwrap();
        assert_eq!(pool.size(), 0);
    }

    #[tokio::test]
    async fn deadline_before_initiation_preserves_published_authority_without_overwrite() {
        let (service, pool) = crate::tests::service();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("playback-deadline-{}", Uuid::new_v4()));
        let mut service = service;
        service.secrets = PlaybackSecrets::new(root.clone()).unwrap();
        let reference = Uuid::new_v4();
        let replay = Replay {
            control: "a".repeat(64),
            media: "b".repeat(64),
            quick_connect_secret: None,
        };
        let expected = replay.control.clone();
        let waiter = service
            .supervised(Duration::from_millis(5), move |service| async move {
                service.write_replay(reference, &replay)?;
                // Assembly/metadata stalls before initiation; the owned deadline drops it.
                std::future::pending::<Result<(), Status>>().await
            })
            .await;
        assert_eq!(waiter.unwrap_err().code(), tonic::Code::DeadlineExceeded);
        service.owned_tasks.drain(Duration::from_secs(1)).await;
        assert_eq!(service.read_replay(reference).unwrap().control, expected);
        let replacement = Replay {
            control: "c".repeat(64),
            media: "d".repeat(64),
            quick_connect_secret: None,
        };
        assert!(service.write_replay(reference, &replacement).is_err());
        assert_eq!(service.read_replay(reference).unwrap().control, expected);
        let quick_connect_ref = Uuid::new_v4();
        service
            .secrets
            .publish(quick_connect_ref, &"q".repeat(4096))
            .unwrap();
        assert_eq!(service.secrets.read(quick_connect_ref).unwrap().len(), 4096);
        assert_eq!(service.read_replay(reference).unwrap().control, expected);
        assert_eq!(pool.size(), 0);
        std::fs::remove_file(root.join(format!("{quick_connect_ref}.key"))).unwrap();
        std::fs::remove_file(root.join(format!("{reference}.key"))).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[tokio::test]
    async fn owned_deadline_and_panic_are_sanitized() {
        let (service, pool) = crate::tests::service();
        let error = service
            .supervised(Duration::from_millis(5), |_| async {
                std::future::pending::<Result<(), Status>>().await
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::DeadlineExceeded);
        let error = service
            .supervised(Duration::from_secs(1), |_| async {
                panic!("isolated test panic");
                #[allow(unreachable_code)]
                Ok::<_, Status>(())
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::Unavailable);
        assert_eq!(error.message(), "upstream_unavailable");
        assert_eq!(pool.size(), 0);
    }
}
