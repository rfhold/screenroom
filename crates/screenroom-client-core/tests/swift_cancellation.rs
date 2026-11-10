#![cfg(not(target_arch = "wasm32"))]

mod support;
use std::{
    io::Read,
    process::{Child, Command, ExitStatus},
    time::Duration,
};

const CAPTURE_LIMIT: usize = 64 * 1024;

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct OwnedServer(tokio::task::JoinHandle<()>);
impl Drop for OwnedServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Default)]
struct Capture {
    total: usize,
    tail: Vec<u8>,
}
fn drain(mut pipe: impl Read) -> std::io::Result<Capture> {
    let mut capture = Capture::default();
    let mut chunk = [0; 4096];
    loop {
        let size = pipe.read(&mut chunk)?;
        if size == 0 {
            return Ok(capture);
        }
        capture.total += size;
        let discard = (capture.tail.len() + size).saturating_sub(CAPTURE_LIMIT);
        capture.tail.drain(..discard);
        capture.tail.extend_from_slice(&chunk[..size]);
    }
}

struct ConsumerOutput {
    status: ExitStatus,
    timed_out: bool,
    stdout: Capture,
    stderr: Capture,
}
impl ConsumerOutput {
    fn diagnostic(&self) -> String {
        format!(
            "status={} timed_out={}\nstdout ({} bytes total; bounded tail):\n{}\nstderr ({} bytes total; bounded tail, including last progress stage):\n{}",
            self.status,
            self.timed_out,
            self.stdout.total,
            String::from_utf8_lossy(&self.stdout.tail),
            self.stderr.total,
            String::from_utf8_lossy(&self.stderr.tail)
        )
    }
}

async fn run_consumer(mut command: Command, timeout: Duration) -> ConsumerOutput {
    let mut child = OwnedChild(
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    // Drain concurrently: waiting for exit before reading can deadlock on full pipes.
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || drain(stdout));
    let stderr = std::thread::spawn(move || drain(stderr));
    let deadline = tokio::time::Instant::now() + timeout;
    let mut timed_out = false;
    while child.0.try_wait().unwrap().is_none() {
        if tokio::time::Instant::now() >= deadline {
            timed_out = true;
            // A concurrent exit can make kill fail; wait still reaps the owned child.
            let _ = child.0.kill();
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let status = child.0.wait().unwrap();
    ConsumerOutput {
        status,
        timed_out,
        stdout: stdout.join().unwrap().unwrap(),
        stderr: stderr.join().unwrap().unwrap(),
    }
}

fn user(label: &str) -> String {
    format!(
        "00000000-0000-0000-0000-{:012}",
        label.bytes().map(u64::from).sum::<u64>()
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run through python3 scripts/test-swift-cancellation.py"]
async fn swift_tasks_cancel_individual_uniffi_futures() {
    let (endpoint, counts, task) = support::start().await;
    let _server = OwnedServer(task);
    let executable = std::env::var("SCREENROOM_SWIFT_TEST_BINARY").unwrap();
    let mut command = Command::new(executable);
    command.env("SCREENROOM_SWIFT_TEST_ENDPOINT", endpoint);
    let output = run_consumer(command, Duration::from_secs(30)).await;
    println!("{}", output.diagnostic());
    assert!(
        !output.timed_out,
        "Swift cancellation consumer exceeded 30 seconds: {}",
        output.diagnostic()
    );
    assert!(output.status.success(), "{}", output.diagnostic());
    let counts = counts.lock().unwrap();
    for label in ["swift-owner-before-poll", "swift-pre-cancel"] {
        assert!(!counts.keys().any(|(id, _)| id == &user(label)));
    }
    assert_eq!(counts.get(&(user("swift-dedup"), "item".into())), Some(&1));
    assert_eq!(
        counts.get(&(user("swift-catalog-info"), "info".into())),
        Some(&1)
    );
    assert_eq!(
        counts.get(&(user("swift-capacity"), "item".into())),
        Some(&64)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn drains_both_pipes_beyond_capture_limit_before_child_exit() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "i=0; while [ \"$i\" -lt 4096 ]; do printf '0123456789abcdef0123456789abcdef'; printf '0123456789abcdef0123456789abcdef' >&2; i=$((i+1)); done; printf 'stdout-complete\\n'; printf 'stderr-complete\\n' >&2"]);
    let output = run_consumer(command, Duration::from_secs(5)).await;
    assert!(
        !output.timed_out && output.status.success(),
        "{}",
        output.diagnostic()
    );
    for (capture, marker) in [
        (&output.stdout, "stdout-complete\n"),
        (&output.stderr, "stderr-complete\n"),
    ] {
        assert!(capture.total > CAPTURE_LIMIT);
        assert_eq!(capture.tail.len(), CAPTURE_LIMIT);
        assert!(capture.tail.ends_with(marker.as_bytes()));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_kills_reaps_and_retains_last_progress() {
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "printf 'last-stage: deliberately stalled\\n' >&2; exec sleep 60",
    ]);
    let output = run_consumer(command, Duration::from_millis(50)).await;
    assert!(
        output.timed_out && !output.status.success(),
        "{}",
        output.diagnostic()
    );
    assert!(
        String::from_utf8_lossy(&output.stderr.tail).contains("last-stage: deliberately stalled")
    );
}
