#[derive(Default)]
pub struct Owner {
    active: bool,
    generation: u64,
}
impl Owner {
    pub fn enter(&mut self) {
        self.active = true;
    }
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn leave(&mut self) {
        self.active = false;
        self.invalidate();
    }
    pub fn stamp(&self) -> u64 {
        self.generation
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn current(&self, stamp: u64) -> bool {
        self.active && self.generation == stamp
    }
}

pub async fn reconcile<T, E>(
    mutation: impl std::future::Future<Output = Result<bool, E>>,
    invalidate: impl FnOnce(),
    read: impl FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>>,
    matches: impl FnOnce(&T) -> bool,
) -> Result<(T, bool, bool), E> {
    let result = mutation.await;
    invalidate();
    let current = read().await?;
    let ambiguous = result.is_err();
    let confirmed = matches(&current) && result.unwrap_or(true);
    Ok((current, confirmed, ambiguous))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn departure_rejects_old_completion_after_reentry() {
        let mut owner = Owner::default();
        owner.enter();
        let old = owner.stamp();
        assert!(owner.current(old));
        owner.leave();
        assert!(!owner.current(old));
        owner.enter();
        assert!(!owner.current(old));
        assert!(owner.current(owner.stamp()));
    }
    use std::{
        cell::Cell,
        future::{Future, ready},
        sync::Arc,
        task::{Context, Wake, Waker},
    };
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    fn run<T>(future: impl Future<Output = T>) -> T {
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        match std::pin::pin!(future).poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("test futures are immediately ready"),
        }
    }
    #[test]
    fn ambiguous_mutation_writes_once_then_reads_server_truth() {
        let writes = Cell::new(0);
        let invalidated = Cell::new(false);
        let result = run(reconcile(
            async {
                writes.set(writes.get() + 1);
                Err::<bool, ()>(())
            },
            || invalidated.set(true),
            || {
                assert!(invalidated.get());
                Box::pin(ready(Ok::<_, ()>(Some("saved"))))
            },
            |link| *link == Some("saved"),
        ))
        .unwrap();
        assert_eq!(writes.get(), 1);
        assert_eq!(result, (Some("saved"), true, true));
    }
    #[test]
    fn failed_reconciliation_does_not_retry_write() {
        let writes = Cell::new(0);
        let result = run(reconcile(
            async {
                writes.set(writes.get() + 1);
                Err::<bool, ()>(())
            },
            || {},
            || Box::pin(ready(Err::<Option<String>, ()>(()))),
            |_| true,
        ));
        assert!(result.is_err());
        assert_eq!(writes.get(), 1);
    }
    #[test]
    fn cancelled_write_does_not_invalidate_or_read_after_departure() {
        let writes = Cell::new(0);
        let invalidations = Cell::new(0);
        let reads = Cell::new(0);
        let mut attempt = Box::pin(reconcile(
            async {
                writes.set(writes.get() + 1);
                std::future::pending::<()>().await;
                Ok::<_, ()>(true)
            },
            || invalidations.set(invalidations.get() + 1),
            || {
                reads.set(reads.get() + 1);
                Box::pin(ready(Ok::<_, ()>(true)))
            },
            |current| *current,
        ));
        let waker = Waker::from(Arc::new(Noop));
        assert!(
            attempt
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        drop(attempt);
        assert_eq!((writes.get(), invalidations.get(), reads.get()), (1, 0, 0));
        run(async {
            reads.set(reads.get() + 1);
        });
        assert_eq!(
            (writes.get(), reads.get()),
            (1, 1),
            "later reconciliation reads only"
        );
    }
    #[test]
    fn invalid_return_or_different_server_truth_is_not_confirmed() {
        let invalid = run(reconcile(
            ready(Ok::<_, ()>(false)),
            || {},
            || Box::pin(ready(Ok::<_, ()>("chosen"))),
            |current| *current == "chosen",
        ))
        .unwrap();
        assert_eq!(invalid, ("chosen", false, false));
        let different = run(reconcile(
            ready(Err::<bool, ()>(())),
            || {},
            || Box::pin(ready(Ok::<_, ()>("other"))),
            |current| *current == "chosen",
        ))
        .unwrap();
        assert_eq!(different, ("other", false, true));
        let unlink = run(reconcile(
            ready(Ok::<_, ()>(true)),
            || {},
            || Box::pin(ready(Ok::<Option<String>, ()>(None))),
            Option::is_none,
        ))
        .unwrap();
        assert_eq!(unlink, (None, true, false));
    }
}
