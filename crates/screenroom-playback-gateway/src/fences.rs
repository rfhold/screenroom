use screenroom_backend::{
    Uuid,
    playback::{Cleanup, Operation, PlaybackRepository, Session},
};
use screenroom_catalog_adapter::jellyfin::playback::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

fn denial(error: screenroom_backend::Error) -> DispatchDenial {
    match error {
        screenroom_backend::Error::Database(_) | screenroom_backend::Error::Unavailable => {
            DispatchDenial::Unavailable
        }
        _ => DispatchDenial::Revoked,
    }
}

fn identity(
    session: &Session,
    context: &DispatchContext<'_>,
    server: &str,
) -> std::result::Result<(), DispatchDenial> {
    if session.device_id != context.intent.device().as_private_str()
        || session.remote_profile_id != context.intent.profile_id()
        || server != context.intent.server_id()
    {
        return Err(DispatchDenial::Revoked);
    }
    Ok(())
}

pub(crate) struct OperationFence {
    pub repository: PlaybackRepository,
    pub operation: Operation,
    pub server: String,
    claimed: Mutex<bool>,
    pub dispatched: AtomicBool,
    ownership: Option<std::sync::Arc<crate::runtime::RuntimeOwnership>>,
}

impl OperationFence {
    pub fn new(
        repository: PlaybackRepository,
        operation: Operation,
        server: String,
        ownership: Option<std::sync::Arc<crate::runtime::RuntimeOwnership>>,
    ) -> Self {
        Self {
            repository,
            operation,
            server,
            claimed: Mutex::new(false),
            dispatched: AtomicBool::new(false),
            ownership,
        }
    }
}

impl DispatchGuard for OperationFence {
    fn check<'a>(&'a self, context: DispatchContext<'a>) -> DispatchCheck<'a> {
        Box::pin(async move {
            if context.purpose != DispatchPurpose::Active {
                return Err(DispatchDenial::Revoked);
            }
            if let Some(ownership) = &self.ownership {
                ownership.check(DispatchPurpose::Active).await?;
            }
            let mut claimed = self.claimed.lock().await;
            let session = if *claimed {
                self.repository.revalidate_io(&self.operation).await
            } else {
                self.repository.before_io(&self.operation).await
            }
            .map_err(denial)?;
            *claimed = true;
            self.dispatched.store(true, Ordering::Release);
            identity(&session, &context, &self.server)?;
            if context
                .play_session_id
                .is_some_and(|id| session.play_session_id.as_deref() != Some(id))
            {
                return Err(DispatchDenial::Revoked);
            }
            Ok(())
        })
    }
}

pub(crate) struct MediaFence {
    pub repository: PlaybackRepository,
    pub session: Uuid,
    pub revision: u32,
    pub capability: String,
    pub anchor: Uuid,
    pub server: String,
    pub ownership: Option<std::sync::Arc<crate::runtime::RuntimeOwnership>>,
    pub diagnostic: crate::transport::MediaDiagnostics,
}

impl MediaFence {
    pub async fn validate(&self) -> std::result::Result<Session, DispatchDenial> {
        if let Some(ownership) = &self.ownership {
            ownership
                .check(DispatchPurpose::Active)
                .await
                .inspect_err(|error| {
                    crate::transport::media_failure(
                        &self.diagnostic,
                        "dispatch_fence_ownership",
                        match error {
                            DispatchDenial::Revoked => "revoked",
                            DispatchDenial::Unavailable => "unavailable",
                        },
                    );
                })?;
        }
        self.repository
            .media(self.session, self.revision, &self.capability, self.anchor)
            .await
            .map(|(session, _)| session)
            .map_err(|error| {
                crate::transport::media_backend_failure(
                    &self.diagnostic,
                    "dispatch_fence_repository",
                    &error,
                );
                denial(error)
            })
    }
}

impl DispatchGuard for MediaFence {
    fn check<'a>(&'a self, context: DispatchContext<'a>) -> DispatchCheck<'a> {
        Box::pin(async move {
            if context.purpose != DispatchPurpose::Active || context.mutation {
                crate::transport::media_failure(
                    &self.diagnostic,
                    "dispatch_fence",
                    "invalid_context",
                );
                return Err(DispatchDenial::Revoked);
            }
            let session = self.validate().await?;
            identity(&session, &context, &self.server).inspect_err(|_| {
                crate::transport::media_failure(&self.diagnostic, "dispatch_fence", "identity");
            })?;
            if context
                .play_session_id
                .is_some_and(|id| session.play_session_id.as_deref() != Some(id))
            {
                crate::transport::media_failure(
                    &self.diagnostic,
                    "dispatch_fence",
                    "play_session_mismatch",
                );
                return Err(DispatchDenial::Revoked);
            }
            Ok(())
        })
    }
}

pub(crate) struct CleanupFence {
    pub repository: PlaybackRepository,
    pub job: Cleanup,
    pub server: String,
    pub ownership: Option<std::sync::Arc<crate::runtime::RuntimeOwnership>>,
}

impl DispatchGuard for CleanupFence {
    fn check<'a>(&'a self, context: DispatchContext<'a>) -> DispatchCheck<'a> {
        Box::pin(async move {
            if context.purpose != DispatchPurpose::Cleanup {
                return Err(DispatchDenial::Revoked);
            }
            if let Some(ownership) = &self.ownership {
                ownership.check(DispatchPurpose::Cleanup).await?;
            }
            let session = self
                .repository
                .cleanup_session(&self.job)
                .await
                .map_err(denial)?;
            identity(&session, &context, &self.server)?;
            if context.intent.device().as_private_str() != self.job.device_id
                || context
                    .play_session_id
                    .is_some_and(|id| self.job.play_session_id.as_deref() != Some(id))
                || (context.mutation
                    && context.play_session_id.is_none()
                    && !self.job.revoke_device)
            {
                return Err(DispatchDenial::Revoked);
            }
            Ok(())
        })
    }
}
