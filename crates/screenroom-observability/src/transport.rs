//! Payload-free transport observations. Bytes are consumer observations, not wire bytes.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestKind {
    Provider,
    Embedding,
    StorageRead,
    StorageHead,
    StorageUpload,
}
impl RequestKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Embedding => "embedding",
            Self::StorageRead => "storage_read",
            Self::StorageHead => "storage_head",
            Self::StorageUpload => "storage_upload",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyDirection {
    DownloadConsumed,
    UploadBodyConsumed,
}
impl BodyDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DownloadConsumed => "download_consumed",
            Self::UploadBodyConsumed => "upload_body_consumed",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestOutcome {
    /// HTTP status accepted and required body consumption completed; not application success.
    Succeeded,
    Failed,
    Cancelled,
}
impl RequestOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}
/// Implementations must return promptly, including when called during cancellation/drop.
/// No payload, URL, credentials, or operation identifiers are supplied.
pub trait TransportObserver: Send + Sync {
    /// Chunks consumed from a response or yielded to the HTTP request-body consumer.
    fn body_chunk(&self, kind: RequestKind, direction: BodyDirection, bytes: u64);
    fn finished(&self, kind: RequestKind, outcome: RequestOutcome, elapsed: Duration);
}
#[derive(Clone, Default)]
pub struct TransportObservation(Option<Arc<dyn TransportObserver>>);
impl TransportObservation {
    pub fn is_enabled(&self) -> bool {
        self.0.is_some()
    }
    pub fn new(observer: Arc<dyn TransportObserver>) -> Self {
        Self(Some(observer))
    }
    pub fn start(&self, kind: RequestKind) -> RequestObservation {
        RequestObservation {
            observer: self.clone(),
            kind,
            started: Instant::now(),
            finished: false,
        }
    }
}
/// A single request, including response consumption. Dropping an unfinished guard cancels it.
pub struct RequestObservation {
    observer: TransportObservation,
    kind: RequestKind,
    started: Instant,
    finished: bool,
}
impl RequestObservation {
    pub fn is_enabled(&self) -> bool {
        self.observer.is_enabled()
    }
    pub fn body(&self) -> BodyObservation {
        BodyObservation {
            observer: self.observer.clone(),
            kind: self.kind,
        }
    }
    pub fn finish(&mut self, outcome: RequestOutcome) {
        if !self.finished {
            self.finished = true;
            if let Some(observer) = &self.observer.0 {
                observer.finished(self.kind, outcome, self.started.elapsed());
            }
        }
    }
}
impl Drop for RequestObservation {
    fn drop(&mut self) {
        self.finish(RequestOutcome::Cancelled);
    }
}
#[derive(Clone)]
pub struct BodyObservation {
    observer: TransportObservation,
    kind: RequestKind,
}
impl BodyObservation {
    pub fn chunk(&self, direction: BodyDirection, bytes: usize) {
        if let Some(observer) = &self.observer.0 {
            observer.body_chunk(self.kind, direction, bytes as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Recorded {
        chunks: Mutex<Vec<(RequestKind, BodyDirection, u64)>>,
        ends: Mutex<Vec<(RequestKind, RequestOutcome, Duration)>>,
    }
    impl TransportObserver for Recorded {
        fn body_chunk(&self, k: RequestKind, d: BodyDirection, n: u64) {
            self.chunks.lock().unwrap().push((k, d, n));
        }
        fn finished(&self, k: RequestKind, o: RequestOutcome, d: Duration) {
            self.ends.lock().unwrap().push((k, o, d));
        }
    }
    #[test]
    fn partial_failure_completion_and_drop_are_once() {
        let recorded = Arc::new(Recorded::default());
        let observer = TransportObservation::new(recorded.clone());
        for outcome in [
            RequestOutcome::Succeeded,
            RequestOutcome::Failed,
            RequestOutcome::Cancelled,
        ] {
            let mut request = observer.start(RequestKind::StorageUpload);
            request.body().chunk(BodyDirection::UploadBodyConsumed, 7);
            if outcome != RequestOutcome::Cancelled {
                request.finish(outcome);
                request.finish(RequestOutcome::Failed);
            }
        }
        assert_eq!(recorded.chunks.lock().unwrap().len(), 3);
        let ends = recorded.ends.lock().unwrap();
        assert_eq!(
            ends.iter().map(|e| e.1).collect::<Vec<_>>(),
            [
                RequestOutcome::Succeeded,
                RequestOutcome::Failed,
                RequestOutcome::Cancelled
            ]
        );
        drop(TransportObservation::default().start(RequestKind::Provider));
    }
}
