use std::{
    collections::BTreeSet,
    future::Future,
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub async fn pages<T, F, Fut>(
    mut fetch: F,
    limit: usize,
    items: usize,
    duration: Duration,
) -> Result<Vec<T>, String>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(Vec<T>, String), String>>,
{
    let deadline = tokio::time::Instant::now() + duration;
    let mut token = String::new();
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for _ in 0..limit {
        if tokio::time::Instant::now() >= deadline {
            return Err("pagination deadline exceeded".into());
        }
        let (batch, next) = tokio::time::timeout_at(deadline, fetch(token))
            .await
            .map_err(|_| "pagination deadline exceeded")??;
        if tokio::time::Instant::now() >= deadline {
            return Err("pagination deadline exceeded".into());
        }
        if batch.len() > items.saturating_sub(result.len()) {
            return Err("pagination item budget exceeded".into());
        }
        result.extend(batch);
        if next.is_empty() {
            return Ok(result);
        }
        if !seen.insert(next.clone()) {
            return Err("pagination token cycle".into());
        }
        token = next;
    }
    Err("pagination page budget exceeded".into())
}

fn collect(
    mut pipe: impl Read + Send + 'static,
    flood: Arc<AtomicBool>,
    cap: usize,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(count) = pipe.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let remaining = cap.saturating_sub(bytes.len());
            bytes.extend_from_slice(&buffer[..count.min(remaining)]);
            if count > remaining {
                flood.store(true, Ordering::Relaxed);
            }
        }
        bytes
    })
}

pub fn process(mut command: Command, duration: Duration, cap: usize) -> Result<Vec<u8>, String> {
    let mut child = command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let group = format!("-{}", child.id());
    let flood = Arc::new(AtomicBool::new(false));
    let stdout = collect(child.stdout.take().unwrap(), flood.clone(), cap);
    let stderr = collect(child.stderr.take().unwrap(), flood.clone(), cap);
    let deadline = Instant::now() + duration;
    let outcome = loop {
        if flood.load(Ordering::Relaxed) {
            break Err("subprocess output budget exceeded".to_string());
        }
        if Instant::now() >= deadline {
            break Err("subprocess deadline exceeded".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => break Err(error.to_string()),
        }
    };
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
    let output = stdout.join().map_err(|_| "stdout reader failed")?;
    let errors = stderr.join().map_err(|_| "stderr reader failed")?;
    if flood.load(Ordering::Relaxed) {
        return Err("subprocess output budget exceeded".into());
    }
    let status = outcome?;
    if !status.success() {
        return Err(format!(
            "subprocess failed: {}",
            String::from_utf8_lossy(&errors)
        ));
    }
    Ok(output)
}
