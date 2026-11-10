#![cfg(not(target_arch = "wasm32"))]

#[path = "support/pagination.rs"]
mod pagination;
use pagination::{Limits, collect};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn limits() -> Limits {
    Limits {
        pages: 3,
        items: 4,
        timeout: Duration::from_millis(20),
    }
}

#[tokio::test]
async fn repeated_library_tokens_fail_without_returning_partial_results() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let result = collect("libraries", limits(), move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        async { Ok((vec![1], "repeated".into())) }
    })
    .await;
    assert!(
        result
            .unwrap_err()
            .contains("libraries: repeated pagination token")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unique_artwork_tokens_and_flooded_pages_are_bounded() {
    let mut next = 0;
    let result = collect::<u8, _, _>("artwork", limits(), move |_| {
        next += 1;
        let token = next.to_string();
        async { Ok((vec![], token)) }
    })
    .await;
    assert!(
        result
            .unwrap_err()
            .contains("artwork: page budget exceeded (3)")
    );
    let result = collect("artwork", limits(), |_| async {
        Ok((vec![0; 5], String::new()))
    })
    .await;
    assert!(
        result
            .unwrap_err()
            .contains("artwork: item budget exceeded (4)")
    );
    let mut next = 0;
    let result = collect(
        "libraries",
        Limits {
            pages: 10,
            ..limits()
        },
        move |_| {
            next += 1;
            let token = next.to_string();
            async { Ok((vec![0; 3], token)) }
        },
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .contains("libraries: item budget exceeded (4)")
    );
}

#[tokio::test]
async fn overall_deadline_drops_an_awaiting_request() {
    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let dropped = Arc::new(AtomicUsize::new(0));
    let observed = dropped.clone();
    let result = collect::<u8, _, _>("libraries", limits(), move |_| {
        let guard = Dropped(observed.clone());
        async move {
            let _guard = guard;
            std::future::pending().await
        }
    })
    .await;
    assert!(
        result
            .unwrap_err()
            .contains("libraries: overall time budget exceeded")
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn complete_pages_return_all_items_at_exact_limits() {
    let mut next = 0;
    let result = collect("artwork", limits(), move |_| {
        next += 1;
        let token = if next == 2 {
            String::new()
        } else {
            "second".into()
        };
        async { Ok((vec![1, 2], token)) }
    })
    .await
    .unwrap();
    assert_eq!(result, vec![1, 2, 1, 2]);
}
