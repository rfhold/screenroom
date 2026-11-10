mod support;
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[tokio::test]
async fn empty_pages_cannot_cycle_or_advance_forever() {
    let cycle = support::pages(
        |_| async { Ok((Vec::<()>::new(), "same".into())) },
        512,
        10000,
        Duration::from_secs(1),
    )
    .await;
    assert!(cycle.unwrap_err().contains("cycle"));
    let mut count = 0;
    let advancing = support::pages(
        |_| {
            count += 1;
            let token = count.to_string();
            async { Ok((Vec::<()>::new(), token)) }
        },
        512,
        10000,
        Duration::from_secs(1),
    )
    .await;
    assert!(advancing.unwrap_err().contains("page budget"));
    assert_eq!(count, 512);
}

#[tokio::test]
async fn item_flood_is_not_partial_success() {
    let result = support::pages(
        |_| async { Ok((vec![(); 10001], String::new())) },
        512,
        10000,
        Duration::from_secs(1),
    )
    .await;
    assert!(result.unwrap_err().contains("item budget"));
}

#[tokio::test]
async fn advancing_pages_share_one_deadline() {
    let mut count = 0;
    let result = support::pages(
        |_| {
            count += 1;
            let token = count.to_string();
            async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                Ok((Vec::<()>::new(), token))
            }
        },
        512,
        10000,
        Duration::from_millis(20),
    )
    .await;
    assert!(result.unwrap_err().contains("deadline"));
    assert!(count < 512);
}

#[tokio::test]
async fn late_ready_response_is_not_partial_success() {
    let result = support::pages(
        |_| async {
            std::thread::sleep(Duration::from_millis(30));
            Ok((vec![()], String::new()))
        },
        512,
        10000,
        Duration::from_millis(20),
    )
    .await;
    assert!(result.unwrap_err().contains("deadline"));
}

#[tokio::test]
async fn whole_deadline_drops_the_inflight_request() {
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let result = support::pages(
        |_| {
            let guard = Dropped(dropped.clone());
            async move {
                let _guard = guard;
                std::future::pending::<()>().await;
                Ok((Vec::<()>::new(), String::new()))
            }
        },
        512,
        10000,
        Duration::from_millis(20),
    )
    .await;
    assert!(result.unwrap_err().contains("deadline"));
    assert!(dropped.load(Ordering::Relaxed));
}

#[test]
fn hung_subprocess_and_descendants_are_terminated_and_waited() {
    let file = std::env::temp_dir().join(format!("screenroom-t-bounds-{}", std::process::id()));
    let mut command = Command::new("python3");
    command.args(["-c", "import subprocess,time,sys; p=subprocess.Popen(['sleep','60']); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(60)"])
        .arg(&file);
    let start = Instant::now();
    assert!(
        support::process(command, Duration::from_millis(500), 4096)
            .unwrap_err()
            .contains("deadline")
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    let pid = std::fs::read_to_string(&file).unwrap();
    let state = Command::new("ps")
        .args(["-o", "stat=", "-p", pid.trim()])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&state.stdout);
    assert!(
        state.trim().is_empty() || state.trim().starts_with('Z'),
        "descendant survived: {state}"
    );
    std::fs::remove_file(file).unwrap();
}

#[test]
fn subprocess_output_flood_is_bounded() {
    let mut command = Command::new("python3");
    command.args(["-c", "import os;\nwhile True: os.write(1,b'x'*4096)"]);
    let start = Instant::now();
    assert!(
        support::process(command, Duration::from_secs(2), 4096)
            .unwrap_err()
            .contains("output budget")
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}
