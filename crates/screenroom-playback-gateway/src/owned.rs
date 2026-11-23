use screenroom_backend::playback::Operation;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;

pub(crate) type OperationSlot = Arc<std::sync::Mutex<Option<Operation>>>;

struct Entry {
    task: JoinHandle<()>,
    operation: OperationSlot,
}

#[derive(Default)]
struct State {
    closed: bool,
    next: u64,
    tasks: HashMap<u64, Entry>,
}

#[derive(Default)]
pub(crate) struct OwnedTasks {
    state: Mutex<State>,
    changed: Notify,
}

impl OwnedTasks {
    pub(crate) async fn spawn<F, Fut>(
        self: &Arc<Self>,
        operation: OperationSlot,
        work: F,
    ) -> Result<(), tonic::Status>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut state = self.state.lock().await;
        if state.closed {
            return Err(tonic::Status::unavailable("gateway_stopping"));
        }
        state.next += 1;
        let id = state.next;
        let registry = self.clone();
        let future = work();
        let task = tokio::spawn(async move {
            future.await;
            // Work includes durable finalization. No provider I/O follows removal.
            registry.state.lock().await.tasks.remove(&id);
            registry.changed.notify_waiters();
        });
        state.tasks.insert(id, Entry { task, operation });
        Ok(())
    }

    pub(crate) async fn close(&self) {
        self.state.lock().await.closed = true;
    }

    async fn wait_empty(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.lock().await.tasks.is_empty() {
                return;
            }
            changed.await;
        }
    }

    /// Revoke dispatch before calling. Forced cancellation is followed by durable
    /// classification by the caller, never by an inference of remote quiescence.
    pub(crate) async fn drain(&self, budget: Duration) -> Vec<OperationSlot> {
        self.close().await;
        if tokio::time::timeout(budget, self.wait_empty())
            .await
            .is_ok()
        {
            return Vec::new();
        }
        let tasks: Vec<_> = self
            .state
            .lock()
            .await
            .tasks
            .drain()
            .map(|(_, t)| t)
            .collect();
        for entry in &tasks {
            entry.task.abort();
        }
        let mut operations = Vec::with_capacity(tasks.len());
        for entry in tasks {
            let _ = entry.task.await;
            operations.push(entry.operation);
        }
        operations
    }

    pub(crate) async fn prepare_cleanup<F, Fut>(
        &self,
        budget: Duration,
        classify: F,
    ) -> Result<(), tonic::Status>
    where
        F: FnOnce(Vec<OperationSlot>) -> Fut,
        Fut: Future<Output = Result<(), tonic::Status>>,
    {
        let forced = self.drain(budget).await;
        tokio::time::timeout(Duration::from_secs(10), classify(forced))
            .await
            .map_err(|_| tonic::Status::unavailable("shutdown_classification_unavailable"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[tokio::test]
    async fn shutdown_closes_admission_atomically_across_racing_clones() {
        let registry = Arc::new(OwnedTasks::default());
        let barrier = Arc::new(tokio::sync::Barrier::new(33));
        let dispatched = Arc::new(AtomicUsize::new(0));
        let mut callers = Vec::new();
        for _ in 0..32 {
            let registry = registry.clone();
            let barrier = barrier.clone();
            let dispatched = dispatched.clone();
            callers.push(tokio::spawn(async move {
                barrier.wait().await;
                registry
                    .spawn(Default::default(), move || async move {
                        dispatched.fetch_add(1, Ordering::SeqCst);
                    })
                    .await
            }));
        }
        barrier.wait().await;
        registry.close().await;
        let mut admitted = 0;
        for caller in callers {
            match caller.await.unwrap() {
                Ok(()) => admitted += 1,
                Err(error) => assert_eq!(error.message(), "gateway_stopping"),
            }
        }
        assert!(registry.drain(Duration::from_secs(1)).await.is_empty());
        assert_eq!(dispatched.load(Ordering::SeqCst), admitted);
        assert!(
            registry
                .spawn(Default::default(), || async {})
                .await
                .is_err()
        );
        assert!(registry.state.lock().await.tasks.is_empty());
    }

    #[tokio::test]
    async fn forced_drain_drops_dispatch_future_before_returning_finalization_slots() {
        struct InFlight(Arc<AtomicBool>);
        impl Drop for InFlight {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let registry = Arc::new(OwnedTasks::default());
        let dispatching = Arc::new(AtomicBool::new(true));
        let guard = InFlight(dispatching.clone());
        registry
            .spawn(Default::default(), || async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            })
            .await
            .unwrap();
        let forced = registry.drain(Duration::from_millis(5)).await;
        assert!(!dispatching.load(Ordering::SeqCst));
        assert_eq!(forced.len(), 1);
        assert!(registry.state.lock().await.tasks.is_empty());
        assert!(forced[0].lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn cleanup_gate_waits_for_classification_and_stays_closed_on_failure() {
        for fail in [false, true] {
            let registry = Arc::new(OwnedTasks::default());
            let cleanup = Arc::new(AtomicBool::new(false));
            let classified_unknown = Arc::new(AtomicBool::new(false));
            let (started, entered) = tokio::sync::oneshot::channel();
            let (release, proceed) = tokio::sync::oneshot::channel();
            let observed_cleanup = cleanup.clone();
            let observed_outcome = classified_unknown.clone();
            let shutdown = tokio::spawn(async move {
                let result = registry
                    .prepare_cleanup(Duration::from_secs(1), |_| async {
                        started.send(()).unwrap();
                        proceed.await.unwrap();
                        if fail {
                            return Err(tonic::Status::unavailable("classification_unavailable"));
                        }
                        observed_outcome.store(true, Ordering::SeqCst);
                        Ok(())
                    })
                    .await;
                if result.is_ok() {
                    assert!(observed_outcome.load(Ordering::SeqCst));
                    observed_cleanup.store(true, Ordering::SeqCst);
                }
                result
            });
            entered.await.unwrap();
            assert!(!cleanup.load(Ordering::SeqCst));
            release.send(()).unwrap();
            let result = shutdown.await.unwrap();
            assert_eq!(result.is_err(), fail);
            assert_eq!(cleanup.load(Ordering::SeqCst), !fail);
            if let Err(error) = result {
                assert_eq!(error.code(), tonic::Code::Unavailable);
            }
        }
    }
}
