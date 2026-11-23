use super::*;
use serde_json::{Value, json};

const PLAYER_PLAY: &str =
    "//*[@name='Playback position']/following-sibling::*//Button[@name='Play']";

const CAPTURE_STAGES: &[&str] = &[
    "exact-id-preflight",
    "app-start",
    "search",
    "exact-card-activate",
    "detail",
    "Play",
    "readiness",
    "captures-controls",
    "motion-comparison",
    "idle-observation",
    "controls-prefix",
    "normal-cleanup",
    "session-shutdown",
    "expand-sidebar",
    "selected-library",
    "collapse-sidebar",
    "search-titles",
    "search-settings-open",
    "title-only",
    "search-settings-close",
    "search-query",
    "loading-probe",
    "loading-capture",
    "media-readiness",
];

fn readiness_diagnostics() -> bool {
    std::env::var("SCREENROOM_PLAYBACK_DIAGNOSTICS").as_deref() == Ok("1")
        && std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE").as_deref() == Ok("readiness")
        && std::env::var("SCREENROOM_E2E_JOURNEY").as_deref() == Ok("playback")
}

fn cleanup_failure_category(error: &str) -> &'static str {
    if error == "Pointer selector matched additional UI controls" {
        "selector-ambiguity"
    } else if !failure_tree_allowed(error) {
        "accessibility"
    } else {
        "other-harness"
    }
}

fn bounded_diagnostic_read(path: impl AsRef<std::path::Path>) -> std::io::Result<String> {
    use std::io::Read;
    let mut value = String::new();
    fs::File::open(path)?
        .take(4097)
        .read_to_string(&mut value)?;
    if value.len() > 4096 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(value)
}

fn resource_observation(mut read: impl FnMut(&str) -> std::io::Result<String>) -> Value {
    let mut result = json!({});
    for name in ["pids.current", "pids.max", "memory.current", "memory.max"] {
        if let Ok(value) = read(name) {
            let value = value.trim();
            if let Ok(number) = value.parse::<u64>() {
                result[name] = json!(number);
            } else if value == "max" && name.ends_with(".max") {
                result[name] = json!("max");
            }
        }
    }
    for (name, keys) in [
        ("pids.events", &["max"][..]),
        (
            "memory.events",
            &["low", "high", "max", "oom", "oom_kill", "oom_group_kill"][..],
        ),
    ] {
        if let Ok(value) = read(name) {
            for line in value.lines() {
                let parts: Vec<_> = line.split_whitespace().collect();
                if let [key, count] = parts.as_slice() {
                    if keys.contains(key) {
                        if let Ok(number) = count.parse::<u64>() {
                            result[name][*key] = json!(number);
                        }
                    }
                }
            }
        }
    }
    result
}

fn resource_context() -> Value {
    resource_observation(|name| {
        bounded_diagnostic_read(std::path::Path::new("/sys/fs/cgroup").join(name))
    })
}

fn owned_process_observation(
    record: &str,
    mut read: impl FnMut(&str) -> std::io::Result<String>,
) -> Value {
    let pid = record.strip_suffix('\n').unwrap_or(record);
    if pid.len() > 10
        || !pid.bytes().all(|byte| byte.is_ascii_digit())
        || pid
            .parse::<u32>()
            .ok()
            .is_none_or(|value| value <= 1 || value.to_string() != pid)
    {
        return json!({"pid_record":"invalid","process":"unknown"});
    }
    let status = match read(&format!("{pid}/status")) {
        Ok(status) => status,
        Err(error) => {
            return json!({
                "pid_record":"valid",
                "process":if error.kind() == std::io::ErrorKind::NotFound { "not-present" } else { "unreadable" }
            });
        }
    };
    let state = status
        .lines()
        .find_map(|line| line.strip_prefix("State:"))
        .and_then(|value| value.split_whitespace().next())
        .unwrap_or("");
    let state = match state {
        "R" => "running",
        "S" => "sleeping",
        "D" => "disk-wait",
        "T" | "t" => "stopped",
        "Z" => "zombie",
        "X" | "x" => "dead",
        _ => "unknown",
    };
    let identity = match read(&format!("{pid}/exe")) {
        Ok(path)
            if std::path::Path::new(&path)
                .file_name()
                .is_some_and(|name| name == "screenroom-linux") =>
        {
            "matches"
        }
        Ok(_) => "mismatch",
        Err(_) => "unavailable",
    };
    let mut result = json!({"pid_record":"valid","process":"present","state":state,"executable_identity":identity});
    if let Some(threads) = status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        result["threads"] = json!(threads);
    }
    result
}

fn owned_process_record(
    record: std::io::Result<String>,
    read: impl FnMut(&str) -> std::io::Result<String>,
) -> Value {
    match record {
        Ok(record) => owned_process_observation(&record, read),
        Err(_) => json!({"pid_record":"unavailable","process":"unknown"}),
    }
}

fn owned_process_context() -> Value {
    owned_process_record(
        bounded_diagnostic_read("/artifacts/playback-owned.pid"),
        |relative| {
            let path = std::path::Path::new("/proc").join(relative);
            if relative.ends_with("/exe") {
                fs::read_link(path).map(|path| path.to_string_lossy().into_owned())
            } else {
                bounded_diagnostic_read(path)
            }
        },
    )
}

fn absent_cleanup_reason(attempted: bool, confirmed: bool, observed: bool) -> &'static str {
    if observed {
        "Player was observed open but its close control is now absent. Normal stop and provider cleanup are unconfirmed."
    } else if attempted && !confirmed {
        "Play was attempted but its input outcome is uncertain. Player opening, normal stop and provider cleanup are unconfirmed; no input is repeated."
    } else if attempted {
        "Play was dispatched but the player close control is absent. Player opening, normal stop and provider cleanup are unconfirmed."
    } else {
        "No Play was dispatched. No owned player was observed; provider cleanup is unconfirmed."
    }
}

async fn bounded_readiness_shutdown<Tree, TreeFuture, Kill, KillFuture>(
    accessibility_failed: bool,
    tree: Tree,
    kill: Kill,
    budget: Duration,
) -> Result
where
    Tree: FnOnce() -> TreeFuture,
    TreeFuture: std::future::Future<Output = Result>,
    Kill: FnOnce() -> KillFuture,
    KillFuture: std::future::Future<Output = Result>,
{
    tokio::time::timeout(budget, async {
        if accessibility_failed {
            drop(tree);
        } else {
            let _ = tokio::time::timeout(Duration::from_millis(250).min(budget / 2), tree()).await;
        }
        kill().await
    })
    .await
    .map_err(|_| -> Box<dyn Error> { "Readiness session shutdown exceeded its bound".into() })?
}

async fn stop_readiness(session: Arc<Session>, accessibility_failed: bool) -> Result {
    let tree_session = session.clone();
    bounded_readiness_shutdown(
        accessibility_failed,
        move || async move {
            let tree = tree_session.dump_tree().await?;
            fs::write("/artifacts/playback-accessibility.xml", tree)?;
            Ok(())
        },
        move || async move {
            Arc::try_unwrap(session)
                .map_err(|_| "Session still shared")?
                .kill()
                .await?;
            Ok(())
        },
        Duration::from_secs(6),
    )
    .await
}

#[tokio::test]
async fn readiness_shutdown_contract() -> Result {
    use std::cell::Cell;
    let trees = Cell::new(0);
    let kills = Cell::new(0);
    bounded_readiness_shutdown(
        true,
        || {
            trees.set(trees.get() + 1);
            std::future::ready(Ok(()))
        },
        || {
            kills.set(kills.get() + 1);
            std::future::ready(Ok(()))
        },
        Duration::from_millis(80),
    )
    .await?;
    assert_eq!((trees.get(), kills.get()), (0, 1));
    let kills = Cell::new(0);
    bounded_readiness_shutdown(
        false,
        || std::future::pending::<Result>(),
        || {
            kills.set(kills.get() + 1);
            std::future::ready(Ok(()))
        },
        Duration::from_millis(80),
    )
    .await?;
    assert_eq!(kills.get(), 1);
    let start = std::time::Instant::now();
    assert!(
        bounded_readiness_shutdown(
            true,
            || std::future::pending::<Result>(),
            || std::future::pending::<Result>(),
            Duration::from_millis(65)
        )
        .await
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    Ok(())
}

#[test]
fn readiness_diagnostic_contract() {
    let mut paths = Vec::new();
    let value = owned_process_observation("42\n", |path| {
        paths.push(path.to_owned());
        Ok(if path.ends_with("/status") {
            "Name:\tscreenroom-linu\nState:\tS (sleeping)\nThreads:\t17\n".into()
        } else {
            "/usr/local/bin/screenroom-linux".into()
        })
    });
    assert_eq!(paths, ["42/status", "42/exe"]);
    assert_eq!(value["state"], "sleeping");
    assert_eq!(value["executable_identity"], "matches");
    assert_eq!(value["threads"], 17);
    assert!(!value.to_string().contains("42"));
    for record in [
        "",
        "1",
        "0",
        "0042",
        "../42",
        "42/../1",
        "42\n43",
        "-42",
        "4294967296",
    ] {
        let value = owned_process_observation(record, |_| panic!("invalid PID must not be read"));
        assert_eq!(value["pid_record"], "invalid");
    }
    let absent = owned_process_observation("42", |_| Err(std::io::ErrorKind::NotFound.into()));
    assert_eq!(absent["process"], "not-present");
    let zombie = owned_process_observation("42", |path| {
        if path.ends_with("/status") {
            Ok("State:\tZ (zombie)\n".into())
        } else {
            Err(std::io::ErrorKind::NotFound.into())
        }
    });
    assert_eq!(zombie["state"], "zombie");
    assert_eq!(zombie["executable_identity"], "unavailable");
    let mut paths = Vec::new();
    let counters = resource_observation(|path| {
        paths.push(path.to_owned());
        Ok(match path {
            "pids.current" => "70",
            "pids.max" => "128",
            "pids.events" => "max 2\nprivate 100\n",
            "memory.current" => "1024",
            "memory.max" => "max",
            "memory.events" => "oom 0\noom_kill 0\nprivate secret\n",
            _ => panic!("unexpected resource path"),
        }
        .into())
    });
    assert_eq!(paths.len(), 6);
    assert_eq!(counters["pids.events"]["max"], 2);
    assert_eq!(counters["pids.max"], 128);
    assert_eq!(counters["memory.max"], "max");
    assert!(!counters.to_string().contains("private"));
    assert_eq!(
        resource_observation(|_| Ok("private invalid".into())),
        json!({})
    );
    let missing_pid = owned_process_record(Err(std::io::ErrorKind::NotFound.into()), |_| {
        panic!("missing PID must not read any process")
    });
    assert_eq!(
        missing_pid,
        json!({"pid_record":"unavailable","process":"unknown"})
    );
    assert!(absent_cleanup_reason(true, true, true).contains("observed open"));
    assert!(absent_cleanup_reason(true, true, false).contains("opening, normal stop"));
    assert!(absent_cleanup_reason(true, false, false).contains("input outcome is uncertain"));
    assert!(absent_cleanup_reason(false, false, false).starts_with("No Play"));
    for (attempted, confirmed, observed) in [
        (true, true, true),
        (true, true, false),
        (true, false, false),
        (false, false, false),
    ] {
        let reason = absent_cleanup_reason(attempted, confirmed, observed);
        assert!(reason.contains("unconfirmed"));
        if attempted {
            assert!(!reason.contains("No Play"));
        }
    }
}

fn stage_message(stage: &str, event: &str) -> Option<String> {
    (CAPTURE_STAGES.contains(&stage) && ["begin", "end", "failed"].contains(&event))
        .then(|| format!("playback-stage: {stage} {event}"))
}

fn breadcrumb(stage: &'static str, event: &'static str) {
    use std::io::Write;
    if let Some(message) = stage_message(stage, event) {
        eprintln!("{message}");
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/artifacts/playback-stages.log")
        {
            let _ = writeln!(file, "{message}");
        }
    }
}

fn enter_stage(current: &mut &'static str, next: &'static str) {
    breadcrumb(*current, "end");
    *current = next;
    breadcrumb(next, "begin");
}

fn failure_tree_allowed(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    !["cache.getitems", "atspi", "at-spi", "dbus", "d-bus"]
        .iter()
        .any(|token| message.contains(token))
}

async fn failure_context(session: &Arc<Session>, stage: &'static str, error: &dyn Error) {
    breadcrumb(stage, "failed");
    let mut context = json!({
        "stage":stage,
        "error":"Original harness error is returned unchanged; no playback outcome inferred.",
        "screenshot":"unavailable: diagnostic screenshot did not complete successfully",
        "tree":"not attempted: no responsive screenshot or accessibility failure"
    });
    if readiness_diagnostics() {
        context["owned_process_before_cleanup"] = owned_process_context();
        context["resources_before_cleanup"] = resource_context();
    }
    // A bounded screenshot is also the compositor responsiveness probe; no cache call.
    if let Ok(Ok(png)) =
        tokio::time::timeout(Duration::from_millis(750), session.take_screenshot()).await
    {
        context["screenshot"] = json!(
            if fs::write("/artifacts/playback-failure.png", png).is_ok() {
                "retained locally"
            } else {
                "local diagnostic write failed"
            }
        );
        if failure_tree_allowed(&error.to_string()) {
            if let Ok(Ok(tree)) =
                tokio::time::timeout(Duration::from_millis(250), session.dump_tree()).await
            {
                context["tree"] = json!(if fs::write("/artifacts/playback-failure-tree.xml", tree)
                    .is_ok()
                {
                    "retained locally"
                } else {
                    "local diagnostic write failed"
                });
            }
        }
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&context) {
        let _ = fs::write("/artifacts/playback-failure-context.json", bytes);
    }
}

#[test]
fn stage_diagnostics_contract() {
    for stage in CAPTURE_STAGES {
        for event in ["begin", "end", "failed"] {
            assert_eq!(
                stage_message(stage, event),
                Some(format!("playback-stage: {stage} {event}"))
            );
        }
    }
    assert_eq!(stage_message("private:item", "begin"), None);
    assert_eq!(
        stage_message("Play", "https://private.invalid/?cap=secret"),
        None
    );
    assert!(!failure_tree_allowed(
        "Cache.GetItems: I/O error: timed out"
    ));
    assert!(!failure_tree_allowed("AT-SPI bridge unavailable"));
    assert!(!failure_tree_allowed("D-Bus connection lost"));
    assert!(failure_tree_allowed("Blank compositor capture"));
}

const STATES: &[(&str, &str)] = &[
    ("detail", "Movie details"),
    ("loading", "Preparing playback"),
    ("playing", "Playing"),
    ("paused", "Paused"),
    ("idle", "Playing with controls hidden"),
    ("settings", "Playback settings"),
    ("volume-min", "Minimum volume"),
    ("volume-max", "Maximum volume"),
    ("muted", "Muted"),
    ("seek-forward", "Seek forward"),
    ("seek-back", "Seek backward"),
    ("tracks", "Audio and subtitle choices"),
    ("subtitles-off", "Subtitles Off"),
    ("fullscreen", "Fullscreen"),
    ("media-error", "Playback unavailable"),
    ("stopped", "Player stopped"),
    ("cleanup", "Cleanup outcome"),
];

struct Evidence {
    rows: Vec<Value>,
    images: Vec<(String, Vec<u8>)>,
    blocked: bool,
    play_attempted: bool,
    play_return_confirmed: bool,
    player_observed: bool,
    settings_input_uncertain: bool,
}

impl Evidence {
    fn new() -> Self {
        Self {
            rows: STATES.iter().map(|(id, label)| json!({
                "id":id, "label":label, "status":"not-observed",
                "viewport":"unknown", "reason":"State was not observed during this bounded journey.",
                "observation":"No synthetic state or injected media failure."
            })).collect(),
            images: Vec::new(),
            blocked: false,
            play_attempted: false,
            play_return_confirmed: false,
            player_observed: false,
            settings_input_uncertain: false,
        }
    }

    fn row(&mut self, id: &str) -> &mut Value {
        self.rows.iter_mut().find(|row| row["id"] == id).unwrap()
    }

    fn add_choice(&mut self, id: &str, label: &str) {
        self.rows.push(json!({
            "id":id, "label":label, "status":"not-observed", "viewport":"unknown",
            "reason":"This discrete UI choice has not been observed yet.",
            "observation":"No provider locator or private track identifier is exported."
        }));
    }

    fn unavailable(&mut self, id: &str, reason: &str) {
        let row = self.row(id);
        row["status"] = json!("unavailable");
        row["reason"] = json!(reason);
        row["observation"] =
            json!("Actual GTK menu inventory; no unavailable choice was selected.");
    }

    async fn capture(&mut self, session: &Arc<Session>, id: &str, observation: &str) -> Result {
        let png = if id == "idle" {
            public_idle_frame(session).await?
        } else {
            frame(session).await?
        };
        let filename = format!("{id}.png");
        let row = self.row(id);
        row["status"] = json!("captured");
        row["viewport"] = json!("1280x800");
        row["image"] = json!(filename);
        row["observation"] = json!(observation);
        row.as_object_mut().unwrap().remove("reason");
        self.images.push((filename, png));
        Ok(())
    }

    fn block_remaining(&mut self) {
        self.blocked = true;
        for row in &mut self.rows {
            if row["status"] != "captured"
                && row["status"] != "unavailable"
                && row["id"] != "loading"
            {
                row["status"] = json!("blocked");
                row["reason"] =
                    json!("Normal playback states are blocked by the visible real-media failure.");
            }
        }
    }

    fn write(self, item: &Item, instance: &str) -> Result {
        let directory = std::path::Path::new("/artifacts/playback");
        fs::create_dir_all(directory)?;
        for (name, png) in self.images {
            fs::write(directory.join(name), png)?;
        }
        let manifest = json!({
            "schema_version":1, "journey":"playback", "backend_instance_id":instance,
            "item":{"id":item.id, "title":item.title, "year":item.year},
            "platforms":[{
                "slug":"linux", "runtime":"Linux container; headless Mutter / PipeWire; GTK4 / GStreamer",
                "outcome":if self.blocked {"blocked"} else {"observed"}, "states":self.rows
            }]
        });
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(())
    }
}

async fn frame(session: &Arc<Session>) -> Result<Vec<u8>> {
    let png = session.take_screenshot().await?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.to_rgb8();
    validate_frame(&image)?;
    Ok(png)
}

fn validate_frame(image: &image::RgbImage) -> Result {
    if image.dimensions() != (1280, 800) {
        return Err("Playback capture requires measured 1280x800 compositor frames".into());
    }
    if image
        .pixels()
        .map(|pixel| pixel.0)
        .collect::<HashSet<_>>()
        .len()
        <= 8
    {
        return Err("Blank compositor capture".into());
    }
    Ok(())
}

async fn shown(session: &Arc<Session>, name: &str) -> Result<bool> {
    Ok(session.cached_accessibles().await?.iter().any(|item| {
        item.name.as_deref() == Some(name) && item.states.iter().any(|state| state == "showing")
    }))
}

const MEDIA_FAILURES: &[&str] = &[
    "Install the GStreamer GTK4 paintable sink (gtk4paintablesink) to play video.",
    "Install the GStreamer playback plugin (playbin).",
    "Playback requires GStreamer HTTP, MP4, H.264 and AAC plugins. Install base/good/libav plugins.",
    "Native playback failed. A decoder, HLS or subtitle plugin may be missing. Check the session or try compatible transcode once.",
    "Choose a Jellyfin profile before playback.",
    "The media source is unavailable. Close the player and try later.",
    "This title is not available to play right now.",
    "This media format is not supported by the available player plugins.",
    "Compatible transcoding is unavailable for this title.",
    "The gateway could not authorize playback.",
    "Playback capacity is full. Close the player and try later.",
];

fn media_failure_snapshot(tree: &str) -> Result<Option<&'static str>> {
    for &message in MEDIA_FAILURES {
        let query = format!(
            "{LIVE_PLAYER}[@showing='true' and @visible='true']//Label[@name={} and @showing='true' and @visible='true']",
            xpath_literal(message)
        );
        let matches = waydriver::atspi::evaluate_xpath(tree, &query)?;
        if matches.len() > 1 {
            return Err("Live media error label is ambiguous".into());
        }
        if matches.len() == 1 {
            return Ok(Some(message));
        }
    }
    Ok(None)
}

async fn media_failure(session: &Arc<Session>) -> Result<Option<&'static str>> {
    media_failure_snapshot(&session.dump_tree().await?)
}

const LIVE_PLAYER: &str = "//Window[@modal='true' and @active='true']";

struct LiveReadiness {
    pause_count: usize,
    showing: bool,
    sensitive: bool,
    enabled: bool,
    slider: Option<(String, String)>,
}

fn live_readiness(tree: &str) -> Result<LiveReadiness> {
    let mut result = LiveReadiness {
        pause_count: 0,
        showing: false,
        sensitive: false,
        enabled: false,
        slider: None,
    };
    let windows = waydriver::atspi::evaluate_xpath(tree, LIVE_PLAYER)?;
    if windows.len() > 1 {
        return Err("Live player window is ambiguous".into());
    }
    if windows.is_empty() {
        return Ok(result);
    }
    let pauses =
        waydriver::atspi::evaluate_xpath(tree, &format!("{LIVE_PLAYER}//Button[@name='Pause']"))?;
    result.pause_count = pauses.len();
    if pauses.len() > 1 {
        return Err("Live Pause button is ambiguous".into());
    }
    let sliders = waydriver::atspi::evaluate_xpath(
        tree,
        &format!("{LIVE_PLAYER}//Slider[@name='Playback position']"),
    )?;
    if sliders.len() > 1 {
        return Err("Live playback position is ambiguous".into());
    }
    let eligible = |reference: &(String, String)| {
        let reference = format!("{}|{}", reference.0, reference.1);
        tree.lines()
            .any(|line| pointer_snapshot_bounds(line, &reference).is_some())
    };
    if let [pause] = pauses.as_slice() {
        let reference = format!("{}|{}", pause.0, pause.1);
        let line = tree
            .lines()
            .find(|line| snapshot_attribute(line, "_ref") == Some(reference.as_str()))
            .ok_or("Live Pause reference is missing from its snapshot")?;
        result.showing = snapshot_attribute(line, "showing") == Some("true");
        result.sensitive = snapshot_attribute(line, "sensitive") == Some("true");
        result.enabled = eligible(pause) && eligible(&windows[0]);
        if result.enabled {
            if let [slider] = sliders.as_slice() {
                if eligible(slider) {
                    result.slider = Some(slider.clone());
                }
            }
        }
    }
    Ok(result)
}

fn live_position(current: f64, minimum: f64, maximum: f64) -> Result<f64> {
    if !current.is_finite()
        || !minimum.is_finite()
        || !maximum.is_finite()
        || minimum < 0.0
        || maximum <= minimum
        || current < minimum
        || current > maximum
    {
        return Err("Live playback position is invalid".into());
    }
    Ok(current)
}

fn position_advanced(previous: Option<f64>, current: f64) -> bool {
    previous.is_some_and(|last| current > last + 250.0)
}

#[derive(Default)]
struct ReadinessProgress {
    previous: Option<((String, String), f64)>,
}

impl ReadinessProgress {
    fn previous_ms(&self) -> Option<f64> {
        self.previous.as_ref().map(|(_, value)| *value)
    }

    fn reset(&mut self) {
        self.previous = None;
    }

    fn previous_for(&self, reference: &(String, String)) -> Option<f64> {
        self.previous
            .as_ref()
            .filter(|(previous_ref, _)| previous_ref == reference)
            .map(|(_, value)| *value)
    }

    fn observe(&mut self, reference: &(String, String), current: f64) -> bool {
        if !current.is_finite() || current <= 0.0 {
            self.reset();
            return false;
        }
        let advanced = position_advanced(self.previous_for(reference), current);
        self.previous = Some((reference.clone(), current));
        advanced
    }
}

#[test]
fn readiness_progress_contract() -> Result {
    // Retained run: value/end rows 8, 16, 30 and the disabled observation at row 22.
    const OBSERVED: &str = r#"{"current_ms":0.0,"elapsed_ms":89,"event":"end","iteration":1,"known_enabled":true,"pause_name_matches":1,"pause_present":true,"pause_sensitive":true,"pause_showing":true,"phase_elapsed_ms":1,"position_unit":"milliseconds","previous_ms":null,"recognized_media_failure":false,"sequence":8,"step":"value"}
{"current_ms":0.0,"elapsed_ms":778,"event":"end","iteration":2,"known_enabled":true,"pause_name_matches":1,"pause_present":true,"pause_sensitive":true,"pause_showing":true,"phase_elapsed_ms":1,"position_unit":"milliseconds","previous_ms":0.0,"recognized_media_failure":false,"sequence":16,"step":"value"}
{"current_ms":null,"elapsed_ms":1469,"event":"end","iteration":3,"known_enabled":false,"pause_name_matches":1,"pause_present":true,"pause_sensitive":false,"pause_showing":true,"phase_elapsed_ms":0,"position_unit":"milliseconds","previous_ms":0.0,"recognized_media_failure":false,"sequence":22,"step":"enabled"}
{"current_ms":4763420.0,"elapsed_ms":2163,"event":"end","iteration":4,"known_enabled":true,"pause_name_matches":1,"pause_present":true,"pause_sensitive":true,"pause_showing":true,"phase_elapsed_ms":1,"position_unit":"milliseconds","previous_ms":0.0,"recognized_media_failure":false,"sequence":30,"step":"value"}"#;
    let reference = (":1.77".into(), "/fixture/slider".into());
    let mut progress = ReadinessProgress::default();
    for line in OBSERVED.lines() {
        let row: Value = serde_json::from_str(line)?;
        if row["known_enabled"] == false {
            progress.reset();
        } else {
            assert!(!progress.observe(&reference, row["current_ms"].as_f64().unwrap()));
        }
    }
    assert_eq!(progress.previous_ms(), Some(4_763_420.0));
    // Synthetic suffix: stationary resume and exact threshold cannot pass; later advance can.
    assert!(!progress.observe(&reference, 4_763_420.0));
    assert!(!progress.observe(&reference, 4_763_670.0));
    assert!(progress.observe(&reference, 4_764_000.0));
    progress.reset();
    for _ in 0..10 {
        assert!(!progress.observe(&reference, 4_763_420.0));
    }
    let replaced = (":1.77".into(), "/fixture/replaced-slider".into());
    assert!(!progress.observe(&replaced, 4_800_000.0));
    progress.reset();
    assert!(!progress.observe(&replaced, 4_900_000.0));
    assert!(!progress.observe(&replaced, 0.0));
    assert_eq!(progress.previous_ms(), None);
    assert!(!progress.observe(&replaced, 4_900_000.0));
    assert!(!progress.observe(&replaced, f64::NAN));
    assert_eq!(progress.previous_ms(), None);
    Ok(())
}

// Actual retained modal subtree; opaque references replaced, other windows omitted.
#[cfg(test)]
const MODAL: &str = r#"<Window role="window" name="Nosferatu" showing="true" visible="true" sensitive="true" active="true" modal="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node0">
    <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node1">
      <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node2">
        <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node3">
          <Generic role="generic" showing="true" visible="true" sensitive="true" focused="true" focusable="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node4">
            <Image role="image" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node5"/>
            <Group role="group" showing="true" visible="true" sensitive="true" bbox="0,0,1100,68" toolkit="GTK" _ref=":1.77|/fixture/node6">
              <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,68" toolkit="GTK" _ref=":1.77|/fixture/node7">
                <Button role="button" name="Back to details" showing="true" visible="true" sensitive="true" focusable="true" bbox="0,0,44,44" toolkit="GTK" _ref=":1.77|/fixture/node8"/>
                <Label role="label" name="Nosferatu" showing="true" visible="true" sensitive="true" bbox="52,0,72,44" toolkit="GTK" _ref=":1.77|/fixture/node9"/>
                <Button role="button" name="Enter fullscreen" showing="true" visible="true" sensitive="true" focusable="true" bbox="1032,0,44,44" toolkit="GTK" _ref=":1.77|/fixture/node10"/>
              </Generic>
            </Group>
            <Group role="group" showing="true" visible="true" sensitive="true" bbox="0,530,1100,190" toolkit="GTK" _ref=":1.77|/fixture/node11">
              <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,530,1100,190" toolkit="GTK" _ref=":1.77|/fixture/node12">
                <Label role="label" name="41:02 / 1:28:40" showing="true" visible="true" sensitive="true" bbox="962,530,114,18" toolkit="GTK" _ref=":1.77|/fixture/node13"/>
                <Slider role="slider" name="Playback position" showing="true" visible="true" sensitive="true" focusable="true" bbox="0,552,1076,68" toolkit="GTK" _ref=":1.77|/fixture/node14"/>
                <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,624,1076,44" toolkit="GTK" _ref=":1.77|/fixture/node15">
                  <Button role="button" name="Pause" showing="true" visible="true" sensitive="true" focusable="true" bbox="0,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/node16"/>
                  <Button role="button" name="Back 30 seconds" showing="true" visible="true" sensitive="true" focusable="true" bbox="48,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/node17"/>
                  <Button role="button" name="Forward 30 seconds" showing="true" visible="true" sensitive="true" focusable="true" bbox="96,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/node18"/>
                  <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="144,624,836,44" toolkit="GTK" _ref=":1.77|/fixture/node19"/>
                  <ToggleButton role="toggle button" name="Mute" showing="true" visible="true" sensitive="true" focusable="true" bbox="984,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/node20"/>
                  <Button role="button" name="Audio, subtitles and playback settings" showing="true" visible="true" sensitive="true" bbox="1032,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/node21">
                    <ToggleButton role="toggle button" name="Audio, subtitles and playback settings" showing="true" visible="true" sensitive="true" focusable="true" bbox="1032,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/node22">
                      <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="1046,624,16,44" toolkit="GTK" _ref=":1.77|/fixture/node23"/>
                    </ToggleButton>
                  </Button>
                </Generic>
              </Generic>
            </Group>
          </Generic>
        </Generic>
      </Generic>
    </Generic>
   </Window>"#;

#[test]
fn live_readiness_contract() -> Result {
    let tree = format!("<Application>\n{MODAL}\n</Application>");
    let observed = live_readiness(&tree)?;
    assert_eq!(observed.pause_count, 1);
    assert!(observed.showing && observed.sensitive && observed.enabled);
    assert_eq!(
        observed.slider,
        Some((":1.77".into(), "/fixture/node14".into()))
    );
    let pause = MODAL
        .lines()
        .find(|line| line.contains("name=\"Pause\""))
        .unwrap();
    let slider = MODAL
        .lines()
        .find(|line| line.contains("<Slider "))
        .unwrap();
    let stale_name = tree.replace("name=\"Pause\"", "name=\"Play\"");
    assert_eq!(live_readiness(&stale_name)?.pause_count, 0);
    assert!(!live_readiness(&stale_name)?.enabled);
    assert!(!live_readiness("<Application/>")?.enabled);
    let behind = format!("<Window>{pause}</Window>");
    assert_eq!(
        live_readiness(&format!("<Application>\n{behind}\n{MODAL}\n</Application>"))?.pause_count,
        1
    );
    for (from, to) in [
        ("showing=\"true\"", "showing=\"false\""),
        ("visible=\"true\"", "visible=\"false\""),
        ("sensitive=\"true\"", "sensitive=\"false\""),
        ("bbox=\"0,624,44,44\"", "bbox=\"0,624,0,44\""),
        ("<Button role=\"button\"", "<Label role=\"label\""),
    ] {
        assert!(!live_readiness(&tree.replace(pause, &pause.replace(from, to)))?.enabled);
    }
    assert!(!live_readiness(&tree.replace("active=\"true\"", "active=\"false\""))?.enabled);
    assert!(!live_readiness(&tree.replace("modal=\"true\"", "modal=\"false\""))?.enabled);
    assert!(live_readiness(&tree.replace(pause, &format!("{pause}\n{pause}"))).is_err());
    assert!(live_readiness(&format!("<Application>\n{MODAL}\n{MODAL}\n</Application>")).is_err());
    assert!(live_readiness(&tree.replace(slider, &format!("{slider}\n{slider}"))).is_err());
    assert!(live_readiness(&tree.replace(slider, ""))?.slider.is_none());
    assert!(
        live_readiness(&tree.replace(
            slider,
            &slider.replace("sensitive=\"true\"", "sensitive=\"false\"")
        ))?
        .slider
        .is_none()
    );
    assert!(!position_advanced(None, 1000.0));
    for current in [1000.0, 999.0, 1250.0] {
        assert!(!position_advanced(Some(1000.0), current));
    }
    assert!(position_advanced(Some(1000.0), 1251.0));
    assert_eq!(live_position(2_462_000.0, 0.0, 5_320_000.0)?, 2_462_000.0);
    for values in [
        (f64::NAN, 0.0, 100.0),
        (f64::INFINITY, 0.0, 100.0),
        (-1.0, 0.0, 100.0),
        (101.0, 0.0, 100.0),
        (0.0, 0.0, 0.0),
    ] {
        assert!(live_position(values.0, values.1, values.2).is_err());
    }
    let message = MEDIA_FAILURES[5];
    let error = format!(
        "<Label role=\"label\" name=\"{}\" showing=\"true\" visible=\"true\" _ref=\":1.77|/fixture/error\"/>",
        message
    );
    let error_tree = tree.replace(pause, &error);
    assert_eq!(media_failure_snapshot(&error_tree)?, Some(message));
    assert!(
        media_failure_snapshot(&error_tree.replace(&error, &format!("{error}\n{error}"))).is_err()
    );
    assert!(
        media_failure_snapshot(&error_tree.replace("showing=\"true\"", "showing=\"false\""))?
            .is_none()
    );
    assert!(media_failure_snapshot(&error_tree.replace(&error, "<Button name=\"The media source is unavailable. Close the player and try later.\" showing=\"true\" visible=\"true\"/>"))?.is_none());
    assert!(media_failure_snapshot(&tree)?.is_none());
    Ok(())
}

async fn position(session: &Arc<Session>) -> Result<f64> {
    Ok(session
        .locate("//*[@name='Playback position']")
        .first()
        .value()
        .await?
        .current)
}

#[derive(Default)]
struct ReadinessSample {
    pause_name_matches: Option<usize>,
    pause_present: Option<bool>,
    pause_showing: Option<bool>,
    pause_sensitive: Option<bool>,
    known_enabled: Option<bool>,
    previous_ms: Option<f64>,
    current_ms: Option<f64>,
    recognized_media_failure: Option<bool>,
}

struct ReadinessTrace {
    file: Option<fs::File>,
    started: std::time::Instant,
    records: usize,
}

#[tokio::test]
async fn readiness_observation_contract() -> Result {
    assert_eq!(
        cleanup_failure_category("Pointer selector matched additional UI controls"),
        "selector-ambiguity"
    );
    assert_eq!(
        cleanup_failure_category("AT-SPI Cache.GetItems failure"),
        "accessibility"
    );
    assert_eq!(
        cleanup_failure_category("unknown https://private.invalid/?cap=secret"),
        "other-harness"
    );
    let path =
        std::env::temp_dir().join(format!("screenroom-ready-{}.jsonl", uuid::Uuid::new_v4()));
    let mut disabled = ReadinessTrace::new(false, &path);
    disabled.record(
        1,
        "pause",
        "begin",
        Duration::ZERO,
        &ReadinessSample::default(),
    );
    assert!(!path.exists());
    let mut trace = ReadinessTrace::new(true, &path);
    let sample = ReadinessSample {
        pause_name_matches: Some(1),
        pause_present: Some(true),
        pause_showing: Some(true),
        pause_sensitive: Some(true),
        known_enabled: Some(true),
        previous_ms: Some(1_190_000.0),
        current_ms: Some(1_191_000.0),
        ..Default::default()
    };
    for (step, event) in [("private-url", "begin"), ("value", "raw-error")] {
        trace.record(1, step, event, Duration::ZERO, &sample);
    }
    assert!(fs::read_to_string(&path)?.is_empty());
    let pending = tokio::time::timeout(Duration::from_millis(10), async {
        trace.record(1, "value", "begin", Duration::ZERO, &sample);
        std::future::pending::<()>().await;
    })
    .await;
    assert!(pending.is_err());
    let partial: Value = serde_json::from_str(fs::read_to_string(&path)?.trim())?;
    assert_eq!(partial["event"], "begin");
    assert_eq!(partial["position_unit"], "milliseconds");
    assert_eq!(partial["previous_ms"], 1_190_000.0);
    assert_eq!(partial["current_ms"], 1_191_000.0);
    assert_eq!(partial["pause_name_matches"], 1);
    assert_eq!(partial["pause_sensitive"], true);
    assert_eq!(partial["known_enabled"], true);
    let invalid = ReadinessSample {
        previous_ms: Some(f64::NAN),
        current_ms: Some(f64::INFINITY),
        ..Default::default()
    };
    trace.record(1, "value", "end", Duration::from_millis(10), &invalid);
    for iteration in 2..200 {
        trace.record(
            iteration,
            "media-failure",
            "begin",
            Duration::ZERO,
            &ReadinessSample::default(),
        );
    }
    let rows: Vec<Value> = fs::read_to_string(&path)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(rows.len(), 128);
    assert!(rows[1]["previous_ms"].is_null() && rows[1]["current_ms"].is_null());
    assert_eq!(rows[1]["phase_elapsed_ms"], 10);
    assert!(rows[2]["known_enabled"].is_null());
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["sequence"], index + 1);
        assert_eq!(row.as_object().unwrap().len(), 15);
        assert!(row["elapsed_ms"].is_u64());
    }
    fs::remove_file(path)?;
    Ok(())
}

impl ReadinessTrace {
    fn new(enabled: bool, path: impl AsRef<std::path::Path>) -> Self {
        Self {
            file: if enabled {
                fs::File::create(path).ok()
            } else {
                None
            },
            started: std::time::Instant::now(),
            records: 0,
        }
    }

    fn record(
        &mut self,
        iteration: u32,
        step: &str,
        event: &str,
        phase_elapsed: Duration,
        sample: &ReadinessSample,
    ) {
        use std::io::Write;
        if self.file.is_none()
            || self.records >= 128
            || !["media-failure", "pause", "enabled", "value"].contains(&step)
            || !["begin", "end"].contains(&event)
        {
            return;
        }
        self.records += 1;
        let record = json!({
            "sequence":self.records, "iteration":iteration,
            "elapsed_ms":self.started.elapsed().as_millis(),
            "phase_elapsed_ms":phase_elapsed.as_millis(),
            "step":step, "event":event, "position_unit":"milliseconds",
            "pause_name_matches":sample.pause_name_matches,
            "pause_present":sample.pause_present, "pause_showing":sample.pause_showing,
            "pause_sensitive":sample.pause_sensitive, "known_enabled":sample.known_enabled,
            "previous_ms":sample.previous_ms.filter(|value| value.is_finite()),
            "current_ms":sample.current_ms.filter(|value| value.is_finite()),
            "recognized_media_failure":sample.recognized_media_failure,
        });
        if let Ok(mut bytes) = serde_json::to_vec(&record) {
            bytes.push(b'\n');
            if let Some(file) = &mut self.file {
                let _ = file.write_all(&bytes);
                let _ = file.flush();
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReadinessStage {
    window: (String, String),
    stage: (String, String),
    bounds: String,
}

fn readiness_stage(tree: &str, title: &str) -> Result<ReadinessStage> {
    let window = modal_window(tree, title)?;
    let reference = format!("{}|{}", window.0, window.1);
    let line = tree
        .lines()
        .find(|line| snapshot_attribute(line, "_ref") == Some(reference.as_str()))
        .ok_or("Readiness modal disappeared")?;
    let bounds = snapshot_attribute(line, "bbox").ok_or("Readiness modal has no bounds")?;
    let stages = waydriver::atspi::evaluate_xpath(
        tree,
        &format!(
            "{LIVE_PLAYER}//Generic[@role='generic' and @focusable='true' and @bbox={}]",
            xpath_literal(bounds)
        ),
    )?;
    let [stage] = stages.as_slice() else {
        return Err("Readiness requires a unique full-modal playback stage".into());
    };
    let focused = waydriver::atspi::evaluate_xpath(tree, "//*[@focused='true']")?;
    if focused.as_slice() != [stage.clone()] {
        return Err("Readiness reveal requires exclusive actual stage focus".into());
    }
    let stage_ref = format!("{}|{}", stage.0, stage.1);
    let stage_line = tree
        .lines()
        .find(|line| snapshot_attribute(line, "_ref") == Some(stage_ref.as_str()))
        .ok_or("Readiness stage disappeared")?;
    if pointer_snapshot_bounds(stage_line, &stage_ref).is_none()
        || !waydriver::atspi::evaluate_xpath(tree, &format!(
            "{LIVE_PLAYER}//*[@editable='true' or @role='text' or @role='entry' or @role='menu' or @role='menu item']"
        ))?.is_empty()
        || modal_menu_open(tree, title)?
    {
        return Err("Readiness reveal requires an eligible stage without text or menu targets".into());
    }
    Ok(ReadinessStage {
        window,
        stage: stage.clone(),
        bounds: bounds.into(),
    })
}

const READINESS_REVEAL_KEY: u32 = 0xffe1;

fn readiness_observation(
    tree: &str,
    title: &str,
    established: bool,
    attempted: u32,
) -> Result<Option<ReadinessStage>> {
    if waydriver::atspi::evaluate_xpath(tree, LIVE_PLAYER)?.is_empty() {
        if established || attempted != 0 {
            return Err("Readiness modal absent after identity or key attempt".into());
        }
        let windows = waydriver::atspi::evaluate_xpath(tree, "//Window")?;
        if windows.len() > 1
            || waydriver::atspi::evaluate_xpath(tree, "//*[@focused='true']")?.len() > 1
            || !waydriver::atspi::evaluate_xpath(
                tree,
                "//*[@modal='true' or @role='dialog' or @role='alert' or @editable='true' or @role='text' or @role='entry' or @role='menu' or @role='menu item' or (@focused='true' and not(@role='window' or @role='generic'))]",
            )?
            .is_empty()
        {
            return Err("Readiness absence has competing modal, control, text or menu targets".into());
        }
        if let [window] = windows.as_slice() {
            let reference = format!("{}|{}", window.0, window.1);
            let line = tree
                .lines()
                .find(|line| snapshot_attribute(line, "_ref") == Some(reference.as_str()))
                .ok_or("Readiness absence window identity is unavailable")?;
            if snapshot_attribute(line, "name") != Some("Screenroom") {
                return Err("Readiness absence has an unexpected window".into());
            }
        }
        return Ok(None);
    }
    readiness_stage(tree, title).map(Some)
}

struct ReadinessOwnershipTrace {
    file: std::cell::RefCell<Option<fs::File>>,
    ordinal: std::cell::Cell<u32>,
    records: std::cell::Cell<u32>,
}

impl ReadinessOwnershipTrace {
    fn new(enabled: bool, path: &std::path::Path) -> Self {
        Self {
            file: std::cell::RefCell::new(if enabled {
                fs::File::create(path).ok()
            } else {
                None
            }),
            ordinal: std::cell::Cell::new(0),
            records: std::cell::Cell::new(0),
        }
    }

    fn record(
        &self,
        ordinal: u32,
        established: bool,
        attempted: u32,
        modal_count: Option<usize>,
        classification: &str,
    ) {
        use std::io::Write;
        if self.records.get() >= 128
            || ![
                "observing",
                "pending",
                "owned-stage",
                "harness-error",
                "query-error",
                "key-attempt",
            ]
            .contains(&classification)
        {
            return;
        }
        let mut file = self.file.borrow_mut();
        let Some(file) = file.as_mut() else {
            return;
        };
        self.records.set(self.records.get() + 1);
        let data = json!({
            "sequence":self.records.get(), "ordinal":ordinal,
            "identity_established":established, "key_attempts":attempted,
            "active_modal_count":modal_count, "classification":classification
        });
        if let Ok(mut bytes) = serde_json::to_vec(&data) {
            bytes.push(b'\n');
            let _ = file.write_all(&bytes);
            let _ = file.flush();
        }
    }

    fn begin(&self, established: bool, attempted: u32) -> u32 {
        let ordinal = self.ordinal.get().saturating_add(1);
        self.ordinal.set(ordinal);
        self.record(ordinal, established, attempted, None, "observing");
        ordinal
    }
}

async fn observe_readiness(
    session: &Arc<Session>,
    title: &str,
    established: bool,
    attempted: u32,
    trace: &ReadinessOwnershipTrace,
) -> Result<Option<ReadinessStage>> {
    let ordinal = trace.begin(established, attempted);
    let tree = match session.dump_tree().await {
        Ok(tree) => tree,
        Err(error) => {
            trace.record(ordinal, established, attempted, None, "query-error");
            return Err(error.into());
        }
    };
    let observation = readiness_observation(&tree, title, established, attempted);
    let count = waydriver::atspi::evaluate_xpath(&tree, LIVE_PLAYER)
        .ok()
        .map(|windows| windows.len());
    trace.record(
        ordinal,
        established,
        attempted,
        count,
        match &observation {
            Ok(None) => "pending",
            Ok(Some(_)) => "owned-stage",
            Err(_) => "harness-error",
        },
    );
    observation
}

#[test]
fn readiness_stage_contract() -> Result {
    // Actual hidden-player subtree; only opaque references and indentation are normalized.
    const STAGE: &str = r#"<Application>
<Window role="window" name="Nosferatu" showing="true" visible="true" sensitive="true" active="true" modal="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node0">
<Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node1">
<Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node2">
<Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node3">
<Generic role="generic" showing="true" visible="true" sensitive="true" focused="true" focusable="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node4">
<Image role="image" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node5"/>
<Group role="group" showing="true" visible="true" bbox="0,0,1100,68" toolkit="GTK" _ref=":1.77|/fixture/node6"/>
<Group role="group" showing="true" visible="true" bbox="0,530,1100,190" toolkit="GTK" _ref=":1.77|/fixture/node7"/>
</Generic>
</Generic>
</Generic>
</Generic>
</Window>
</Application>"#;
    let expected = ReadinessStage {
        window: (":1.77".into(), "/fixture/node0".into()),
        stage: (":1.77".into(), "/fixture/node4".into()),
        bounds: "0,0,1100,720".into(),
    };
    assert_eq!(readiness_stage(STAGE, "Nosferatu")?, expected);
    assert!(readiness_stage(STAGE, "Other title").is_err());
    for (from, to) in [
        ("focused=\"true\"", "focused=\"false\""),
        ("focusable=\"true\"", "focusable=\"false\""),
        ("role=\"generic\"", "role=\"slider\""),
        (
            "focused=\"true\" focusable=\"true\" bbox=\"0,0,1100,720\"",
            "focused=\"true\" focusable=\"true\" bbox=\"0,0,100,100\"",
        ),
        (
            "sensitive=\"true\" focused=\"true\"",
            "sensitive=\"false\" focused=\"true\"",
        ),
        ("active=\"true\"", "active=\"false\""),
        ("modal=\"true\"", "modal=\"false\""),
    ] {
        assert!(readiness_stage(&STAGE.replace(from, to), "Nosferatu").is_err());
    }
    for node in [
        r#"<Generic role="generic" focusable="true" bbox="0,0,1100,720" _ref=":1.77|/fixture/duplicate"/>"#,
        r#"<Button role="button" focused="true" _ref=":1.77|/fixture/control"/>"#,
        r#"<Entry role="entry" editable="true" _ref=":1.77|/fixture/text"/>"#,
        r#"<Menu role="menu" _ref=":1.77|/fixture/menu"/>"#,
        r#"<Generic role="unknown" focused="true" _ref=":1.77|/fixture/unknown"/>"#,
    ] {
        let changed = STAGE.replace("</Window>", &format!("{node}</Window>"));
        assert!(readiness_stage(&changed, "Nosferatu").is_err());
    }
    assert!(
        readiness_stage(
            &STAGE.replace("</Application>", &format!("{STAGE}</Application>")),
            "Nosferatu"
        )
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn readiness_absence_contract() -> Result {
    use std::cell::{Cell, RefCell};
    async fn fails_without_input(tree: &str) -> Result {
        let inputs = Cell::new(0);
        let mut identity = None;
        let mut attempted = 0;
        assert!(
            readiness_reveal_policy(
                &mut identity,
                &mut attempted,
                |established, attempted| {
                    std::future::ready(readiness_observation(
                        tree,
                        "Nosferatu",
                        established,
                        attempted,
                    ))
                },
                |_, _| {
                    inputs.set(inputs.get() + 1);
                    std::future::ready(Ok(()))
                },
            )
            .await
            .is_err()
        );
        assert!(identity.is_none());
        assert_eq!((attempted, inputs.get()), (0, 0));
        Ok(())
    }
    // Actual later Preparing modal, reconstructed from the retained modal fixture.
    let actual = MODAL
        .replace("<Window role=\"window\" name=\"Nosferatu\" showing=", "<Window role=\"window\" name=\"Nosferatu\" description=\"Playback controls visible\" showing=")
        .replace(
            "            <Group role=\"group\" showing=\"true\" visible=\"true\" sensitive=\"true\" bbox=\"0,530,1100,190\"",
            "            <Generic role=\"generic\" showing=\"true\" visible=\"true\" sensitive=\"true\" bbox=\"452,291,196,58\" toolkit=\"GTK\" _ref=\":1.77|/fixture/preparing0\">\n              <Label role=\"label\" name=\"Preparing playback…\" showing=\"true\" visible=\"true\" sensitive=\"true\" bbox=\"452,291,156,18\" toolkit=\"GTK\" _ref=\":1.77|/fixture/preparing1\"/>\n            </Generic>\n            <Group role=\"group\" showing=\"true\" visible=\"true\" sensitive=\"true\" bbox=\"0,530,1100,190\"",
        )
        .replace("41:02 / 1:28:40", "0:00")
        .replace("962,530,114,18", "1043,530,33,18");
    assert!(actual.contains("Preparing playback…"));
    assert!(readiness_observation(&actual, "Nosferatu", false, 0)?.is_some());
    let absent = "<Application/>";
    assert!(readiness_observation(absent, "Nosferatu", false, 0)?.is_none());
    let main = r#"<Application><Window role="window" name="Screenroom" showing="true" visible="true" sensitive="true" bbox="0,0,1280,800" _ref="bus|/main"/></Application>"#;
    assert!(readiness_observation(main, "Nosferatu", false, 0)?.is_none());
    for tree in [
        "<".to_owned(),
        main.replace("Screenroom", "Wrong window"),
        main.replace("</Application>", &format!("{main}</Application>")),
        actual.replace("active=\"true\"", "active=\"false\""),
        actual.replace("name=\"Nosferatu\"", "name=\"Wrong modal\""),
        format!("<Application>{actual}{actual}</Application>"),
    ] {
        assert!(readiness_observation(&tree, "Nosferatu", false, 0).is_err());
        fails_without_input(&tree).await?;
    }
    for node in [
        r#"<Button role="button" focused="true" _ref="bus|/control"/>"#,
        r#"<Entry role="entry" editable="true" _ref="bus|/text"/>"#,
        r#"<Menu role="menu" _ref="bus|/menu"/>"#,
        r#"<Dialog role="dialog" modal="true" _ref="bus|/wrong"/>"#,
        r#"<Generic role="unknown" focused="true" _ref="bus|/unknown"/>"#,
        r#"<Generic role="generic" focused="true" _ref="bus|/one"/><Generic role="generic" focused="true" _ref="bus|/two"/>"#,
    ] {
        assert!(
            readiness_observation(
                &format!("<Application>{node}</Application>"),
                "Nosferatu",
                false,
                0
            )
            .is_err()
        );
        fails_without_input(&format!("<Application>{node}</Application>")).await?;
    }
    for (established, attempted) in [(true, 0), (false, 1), (true, 1)] {
        assert!(readiness_observation(absent, "Nosferatu", established, attempted).is_err());
    }
    let mut identity = None;
    let mut attempted = 0;
    let keys = Cell::new(0);
    assert!(
        !readiness_reveal_policy(
            &mut identity,
            &mut attempted,
            |established, attempted| std::future::ready(readiness_observation(
                absent,
                "Nosferatu",
                established,
                attempted
            )),
            |_, _| {
                keys.set(keys.get() + 1);
                std::future::ready(Ok(()))
            },
        )
        .await?
    );
    assert!(identity.is_none());
    assert_eq!((attempted, keys.get()), (0, 0));
    let observations = RefCell::new(Vec::new());
    assert!(
        readiness_reveal_policy(
            &mut identity,
            &mut attempted,
            |established, attempted| {
                observations.borrow_mut().push((established, attempted));
                std::future::ready(readiness_observation(
                    &actual,
                    "Nosferatu",
                    established,
                    attempted,
                ))
            },
            |key, attempted| {
                assert_eq!(key, 0xffe1);
                assert_eq!(attempted, 1);
                keys.set(keys.get() + 1);
                std::future::ready(Ok(()))
            },
        )
        .await?
    );
    assert_eq!(*observations.borrow(), [(false, 0), (true, 0)]);
    assert_eq!((attempted, keys.get()), (1, 1));
    assert!(
        readiness_reveal_policy(
            &mut identity,
            &mut attempted,
            |established, attempted| std::future::ready(readiness_observation(
                absent,
                "Nosferatu",
                established,
                attempted
            )),
            |_, _| {
                keys.set(keys.get() + 1);
                std::future::ready(Ok(()))
            },
        )
        .await
        .is_err()
    );
    assert_eq!(keys.get(), 1);
    let observations = Cell::new(0);
    assert!(
        readiness_reveal_policy(
            &mut None,
            &mut 0,
            |established, attempted| {
                observations.set(observations.get() + 1);
                std::future::ready(readiness_observation(
                    if observations.get() == 1 {
                        &actual
                    } else {
                        absent
                    },
                    "Nosferatu",
                    established,
                    attempted,
                ))
            },
            |_, _| {
                keys.set(keys.get() + 1);
                std::future::ready(Ok(()))
            },
        )
        .await
        .is_err()
    );
    assert_eq!(keys.get(), 1);
    let mut no_identity = None;
    let mut no_keys = 0;
    let started = std::time::Instant::now();
    assert!(
        tokio::time::timeout(Duration::from_millis(40), async {
            loop {
                assert!(
                    !readiness_reveal_policy(
                        &mut no_identity,
                        &mut no_keys,
                        |established, attempted| std::future::ready(readiness_observation(
                            absent,
                            "Nosferatu",
                            established,
                            attempted
                        )),
                        |_, _| {
                            keys.set(keys.get() + 1);
                            std::future::ready(Ok(()))
                        },
                    )
                    .await?
                );
                tokio::time::sleep(Duration::from_millis(600)).await;
            }
            #[allow(unreachable_code)]
            Ok::<(), Box<dyn Error>>(())
        })
        .await
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_millis(300));
    assert!(no_identity.is_none());
    assert_eq!(no_keys, 0);
    assert_eq!(keys.get(), 1);
    Ok(())
}

#[test]
fn readiness_ownership_trace_contract() -> Result {
    let directory =
        std::env::temp_dir().join(format!("screenroom-ownership-{}", uuid::Uuid::new_v4()));
    let path = directory.join("ownership.jsonl");
    let disabled = ReadinessOwnershipTrace::new(false, &path);
    disabled.begin(false, 0);
    assert!(!directory.exists());
    fs::create_dir(&directory)?;
    let trace = ReadinessOwnershipTrace::new(true, &path);
    let ordinal = trace.begin(false, 0);
    let pending: Value = serde_json::from_str(fs::read_to_string(&path)?.trim())?;
    assert_eq!(pending["classification"], "observing");
    assert_eq!(pending["active_modal_count"], Value::Null);
    trace.record(ordinal, false, 0, Some(0), "pending");
    trace.record(ordinal, true, 1, Some(1), "key-attempt");
    trace.record(ordinal, false, 0, Some(0), "private-url");
    for _ in 0..150 {
        trace.record(ordinal, true, 1, Some(1), "owned-stage");
    }
    let text = fs::read_to_string(&path)?;
    assert_eq!(text.lines().count(), 128);
    for (index, line) in text.lines().enumerate() {
        let value: Value = serde_json::from_str(line)?;
        assert_eq!(value["sequence"], (index + 1) as u64);
        let keys: std::collections::BTreeSet<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "sequence",
                "ordinal",
                "identity_established",
                "key_attempts",
                "active_modal_count",
                "classification"
            ]
            .into_iter()
            .collect()
        );
        assert!(!line.contains("private") && !line.contains("http") && !line.contains("/"));
    }
    drop(trace);
    drop(ReadinessOwnershipTrace::new(true, &path));
    assert_eq!(fs::read_to_string(&path)?, "");
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn readiness_reveal_contract() -> Result {
    use std::cell::{Cell, RefCell};
    let identity = ReadinessStage {
        window: ("bus".into(), "/window".into()),
        stage: ("bus".into(), "/stage".into()),
        bounds: "0,0,1100,720".into(),
    };
    let mut previous = None;
    let mut count = 0;
    let observations = Cell::new(0);
    let commands = RefCell::new(Vec::new());
    readiness_reveal_policy(
        &mut previous,
        &mut count,
        |_, _| {
            observations.set(observations.get() + 1);
            std::future::ready(Ok(Some(identity.clone())))
        },
        |key, attempted| {
            assert_eq!(attempted, 1);
            commands.borrow_mut().push(key);
            std::future::ready(Ok(()))
        },
    )
    .await?;
    assert_eq!(observations.get(), 2);
    assert_eq!(*commands.borrow(), [0xffe1]);
    assert_eq!(count, 1);
    let calls = Cell::new(0);
    let error = readiness_reveal_policy(
        &mut previous,
        &mut count,
        |_, _| std::future::ready(Ok(Some(identity.clone()))),
        |_, _| {
            calls.set(calls.get() + 1);
            std::future::ready(Err("unknown key dispatch status".into()))
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "unknown key dispatch status");
    assert_eq!(calls.get(), 1);
    assert_eq!(count, 2);
    let mut different = identity.clone();
    different.window.1 = "/replacement".into();
    let result = readiness_reveal_policy(
        &mut previous,
        &mut count,
        |_, _| std::future::ready(Ok(Some(different.clone()))),
        |_, _| {
            calls.set(calls.get() + 1);
            std::future::ready(Ok(()))
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(calls.get(), 1);
    for changed in [
        ReadinessStage {
            stage: ("bus".into(), "/other-stage".into()),
            ..identity.clone()
        },
        ReadinessStage {
            bounds: "0,0,1280,800".into(),
            ..identity.clone()
        },
    ] {
        assert!(
            readiness_reveal_policy(
                &mut previous,
                &mut count,
                |_, _| std::future::ready(Ok(Some(changed.clone()))),
                |_, _| {
                    calls.set(calls.get() + 1);
                    std::future::ready(Ok(()))
                }
            )
            .await
            .is_err()
        );
    }
    assert_eq!(calls.get(), 1);
    let observations = Cell::new(0);
    let result = readiness_reveal_policy(
        &mut None,
        &mut 0,
        |_, _| {
            observations.set(observations.get() + 1);
            std::future::ready(Ok(Some(if observations.get() == 1 {
                identity.clone()
            } else {
                different.clone()
            })))
        },
        |_, _| {
            calls.set(calls.get() + 1);
            std::future::ready(Ok(()))
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(calls.get(), 1);
    let mut capped = 50;
    assert!(
        readiness_reveal_policy(
            &mut None,
            &mut capped,
            |_, _| std::future::ready(Ok(Some(identity.clone()))),
            |_, _| std::future::ready(Ok(()))
        )
        .await
        .is_err()
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            readiness_reveal_policy(
                &mut None,
                &mut 0,
                |_, _| std::future::pending::<Result<Option<ReadinessStage>>>(),
                |_, _| std::future::ready(Ok(()))
            )
        )
        .await
        .is_err()
    );
    Ok(())
}

async fn readiness_reveal_policy<O, OF, K, KF>(
    identity: &mut Option<ReadinessStage>,
    dispatched: &mut u32,
    mut observe: O,
    key: K,
) -> Result<bool>
where
    O: FnMut(bool, u32) -> OF,
    OF: std::future::Future<Output = Result<Option<ReadinessStage>>>,
    K: FnOnce(u32, u32) -> KF,
    KF: std::future::Future<Output = Result>,
{
    if *dispatched >= 50 {
        return Err("Readiness reveal reached its 50-key bound".into());
    }
    let Some(first) = observe(identity.is_some(), *dispatched).await? else {
        if identity.is_some() || *dispatched != 0 {
            return Err("Readiness modal absent after identity or key attempt".into());
        }
        return Ok(false);
    };
    if identity.as_ref().is_some_and(|previous| previous != &first) {
        return Err("Readiness stage identity changed".into());
    }
    *identity = Some(first.clone());
    tokio::time::sleep(Duration::from_millis(20)).await;
    let second = observe(true, *dispatched)
        .await?
        .ok_or("Readiness modal absent after identity or key attempt")?;
    if first != second {
        return Err("Readiness stage changed between fresh focus observations".into());
    }
    *identity = Some(second);
    *dispatched += 1;
    key(READINESS_REVEAL_KEY, *dispatched).await?;
    Ok(true)
}

async fn ready(session: &Arc<Session>) -> Result<Option<&'static str>> {
    let diagnostic = readiness_diagnostics();
    let mut trace = ReadinessTrace::new(
        diagnostic,
        "/artifacts/playback-readiness-observations.jsonl",
    );
    let ownership = ReadinessOwnershipTrace::new(
        diagnostic || idle_diagnostics(),
        std::path::Path::new("/artifacts/playback-readiness-ownership.jsonl"),
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut progress = ReadinessProgress::default();
        let mut iteration = 0;
        let title = std::env::var("SCREENROOM_SHOWCASE_TITLE")?;
        let mut stage_identity = None;
        let mut reveal_keys = 0;
        loop {
            if !readiness_reveal_policy(
                &mut stage_identity,
                &mut reveal_keys,
                |established, attempted| {
                    observe_readiness(session, &title, established, attempted, &ownership)
                },
                |key, attempted| {
                    ownership.record(
                        ownership.ordinal.get(),
                        true,
                        attempted,
                        Some(1),
                        "key-attempt",
                    );
                    async move { session.press_keysym(key).await.map_err(Into::into) }
                },
            )
            .await?
            {
                tokio::time::sleep(Duration::from_millis(600)).await;
                continue;
            }
            iteration += 1;
            let mut sample = ReadinessSample {
                previous_ms: progress.previous_ms(),
                ..Default::default()
            };
            let started = std::time::Instant::now();
            trace.record(iteration, "media-failure", "begin", Duration::ZERO, &sample);
            let tree = session.dump_tree().await?;
            let failure = media_failure_snapshot(&tree)?;
            sample.recognized_media_failure = Some(failure.is_some());
            trace.record(
                iteration,
                "media-failure",
                "end",
                started.elapsed(),
                &sample,
            );
            if let Some(message) = failure {
                return Ok(Some(message));
            }
            let started = std::time::Instant::now();
            trace.record(iteration, "pause", "begin", Duration::ZERO, &sample);
            let observed = live_readiness(&tree)?;
            if !observed.enabled || observed.slider.is_none() {
                progress.reset();
                sample.previous_ms = None;
            }
            sample.pause_name_matches = Some(observed.pause_count);
            sample.pause_present = Some(observed.pause_count == 1);
            sample.pause_showing = Some(observed.showing);
            sample.pause_sensitive = Some(observed.sensitive);
            trace.record(iteration, "pause", "end", started.elapsed(), &sample);
            if observed.showing {
                let started = std::time::Instant::now();
                trace.record(iteration, "enabled", "begin", Duration::ZERO, &sample);
                let enabled = observed.enabled;
                sample.known_enabled = Some(enabled);
                trace.record(iteration, "enabled", "end", started.elapsed(), &sample);
                if let Some(reference) = observed.slider.filter(|_| enabled) {
                    let started = std::time::Instant::now();
                    sample.previous_ms = progress.previous_for(&reference);
                    trace.record(iteration, "value", "begin", Duration::ZERO, &sample);
                    let value = session.value_ref(&reference.0, &reference.1).await?;
                    let current = live_position(value.current, value.minimum, value.maximum)?;
                    sample.current_ms = Some(current);
                    trace.record(iteration, "value", "end", started.elapsed(), &sample);
                    if progress.observe(&reference, current) {
                        return Ok(None);
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
    })
    .await
    .map_err(|_| "No advancing playback or recognized media error within 30 seconds")?
}

async fn reveal(session: &Arc<Session>) -> Result {
    session.pointer_motion_absolute(638.0, 398.0).await?;
    session.pointer_motion_absolute(642.0, 402.0).await?;
    visible(session, "Audio, subtitles and playback settings").await
}

const PLAYBACK_SETTINGS: &str = "//ToggleButton[@name='Audio, subtitles and playback settings']";

#[test]
fn settings_control_contract() -> Result {
    // Actual retained GTK subtree; only opaque references are replaced.
    const SETTINGS: &str = r#"<Button role="button" name="Audio, subtitles and playback settings" showing="true" visible="true" sensitive="true" bbox="1032,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/outer">
<ToggleButton role="toggle button" name="Audio, subtitles and playback settings" showing="true" visible="true" sensitive="true" focusable="true" bbox="1032,624,44,44" toolkit="GTK" _ref=":1.77|/fixture/toggle">
<Generic role="generic" showing="true" visible="true" sensitive="true" bbox="1046,624,16,44" toolkit="GTK" _ref=":1.77|/fixture/icon"/>
</ToggleButton>
</Button>"#;
    let xml = format!("<Application>{SETTINGS}</Application>");
    let old = "//*[@role='button' or @role='toggle button'][@name='Audio, subtitles and playback settings' or @description='Audio, subtitles and playback settings' or .//Label[@name='Audio, subtitles and playback settings']]";
    assert_eq!(waydriver::atspi::evaluate_xpath(&xml, old)?.len(), 2);
    let selected = waydriver::atspi::evaluate_xpath(&xml, PLAYBACK_SETTINGS)?;
    assert_eq!(selected, [(":1.77".into(), "/fixture/toggle".into())]);
    let line = SETTINGS.lines().nth(1).unwrap();
    assert_eq!(
        pointer_snapshot_bounds(line, ":1.77|/fixture/toggle"),
        Some("1032,624,44,44")
    );
    let absent = xml.replace("ToggleButton", "Generic");
    assert!(waydriver::atspi::evaluate_xpath(&absent, PLAYBACK_SETTINGS)?.is_empty());
    let duplicate = format!("<Application>{SETTINGS}{SETTINGS}</Application>");
    assert_eq!(
        waydriver::atspi::evaluate_xpath(&duplicate, PLAYBACK_SETTINGS)?.len(),
        2
    );
    for (from, to) in [
        ("showing=\"true\"", "showing=\"false\""),
        ("visible=\"true\"", "visible=\"false\""),
        ("sensitive=\"true\"", "sensitive=\"false\""),
        ("bbox=\"1032,624,44,44\"", "bbox=\"1032,624,0,44\""),
        ("bbox=\"1032,624,44,44\"", "bbox=\"bad\""),
        (
            "_ref=\":1.77|/fixture/toggle\"",
            "_ref=\":1.77|/fixture/stale\"",
        ),
    ] {
        assert!(
            pointer_snapshot_bounds(&line.replace(from, to), ":1.77|/fixture/toggle").is_none()
        );
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModalFocus {
    window: (String, String),
    target: (String, String),
    focused: Option<(String, String)>,
    eligible: bool,
}

fn modal_window(tree: &str, title: &str) -> Result<(String, String)> {
    let windows = waydriver::atspi::evaluate_xpath(tree, LIVE_PLAYER)?;
    let [window] = windows.as_slice() else {
        return Err("Keyboard input requires exactly one active playback modal".into());
    };
    let reference = format!("{}|{}", window.0, window.1);
    let line = tree
        .lines()
        .find(|line| snapshot_attribute(line, "_ref") == Some(reference.as_str()))
        .ok_or("Active playback modal disappeared")?;
    if snapshot_attribute(line, "name") != Some(title)
        || pointer_snapshot_bounds(line, &reference).is_none()
    {
        return Err("Keyboard input requires the visible eligible selected playback modal".into());
    }
    Ok(window.clone())
}

fn modal_focus(tree: &str, title: &str, selector: &str, role: &str) -> Result<ModalFocus> {
    let window = modal_window(tree, title)?;
    let targets = waydriver::atspi::evaluate_xpath(tree, &format!("{LIVE_PLAYER}{selector}"))?;
    let [target] = targets.as_slice() else {
        return Err("Playback keyboard target must be unique in the active modal".into());
    };
    let reference = format!("{}|{}", target.0, target.1);
    let line = tree
        .lines()
        .find(|line| snapshot_attribute(line, "_ref") == Some(reference.as_str()))
        .ok_or("Playback keyboard target disappeared")?;
    if !matches!(role, "button" | "toggle button" | "slider")
        || snapshot_attribute(line, "role") != Some(role)
        || snapshot_attribute(line, "focusable") != Some("true")
    {
        return Err("Playback keyboard target has an unexpected role or cannot take focus".into());
    }
    let focused = waydriver::atspi::evaluate_xpath(tree, "//*[@focused='true']")?;
    if focused.len() > 1 {
        return Err("Playback keyboard focus is ambiguous".into());
    }
    let modal_focused =
        waydriver::atspi::evaluate_xpath(tree, &format!("{LIVE_PLAYER}//*[@focused='true']"))?;
    if focused != modal_focused {
        return Err("Playback keyboard focus escaped the active modal".into());
    }
    Ok(ModalFocus {
        window,
        target: target.clone(),
        focused: focused.into_iter().next(),
        eligible: pointer_snapshot_bounds(line, &reference).is_some(),
    })
}

async fn modal_keyboard_policy<O, OF, N, NF, A, AF>(
    budget: Duration,
    mut observe: O,
    mut navigate: N,
    activate: A,
) -> Result<(String, String)>
where
    O: FnMut() -> OF,
    OF: std::future::Future<Output = Result<ModalFocus>>,
    N: FnMut() -> NF,
    NF: std::future::Future<Output = Result>,
    A: FnOnce() -> AF,
    AF: std::future::Future<Output = Result>,
{
    tokio::time::timeout(budget, async {
        let mut identity = None;
        let mut settled = None;
        let mut tabs = 0;
        loop {
            control_record("focus", "begin", "none", None);
            let current = match observe().await {
                Ok(current) => current,
                Err(error) => {
                    control_record("focus", "failed", "none", None);
                    return Err(error);
                }
            };
            control_record(
                "focus",
                "observed",
                "none",
                Some((
                    current.eligible,
                    current.focused.as_ref() == Some(&current.target),
                )),
            );
            let target = (current.window.clone(), current.target.clone());
            if identity
                .as_ref()
                .is_some_and(|previous| previous != &target)
            {
                return Err("Playback keyboard target changed during focus navigation".into());
            }
            identity = Some(target);
            if current.eligible && current.focused.as_ref() == Some(&current.target) {
                if settled.as_ref() == Some(&current) {
                    activate().await?;
                    return Ok(current.target);
                }
                settled = Some(current);
            } else {
                settled = None;
                if tabs == 48 {
                    return Err("Playback keyboard target not focused within 48 Tab steps".into());
                }
                control_operation("tab", "none", navigate()).await?;
                tabs += 1;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}

async fn modal_key(
    session: &Arc<Session>,
    selector: &str,
    role: &str,
    key: u32,
) -> Result<(String, String)> {
    modal_key_tracked(session, selector, role, key, &mut false).await
}

async fn modal_key_tracked(
    session: &Arc<Session>,
    selector: &str,
    role: &str,
    key: u32,
    input_uncertain: &mut bool,
) -> Result<(String, String)> {
    let title = std::env::var("SCREENROOM_SHOWCASE_TITLE")?;
    modal_keyboard_policy(
        session.default_timeout(),
        || async { modal_focus(&session.dump_tree().await?, &title, selector, role) },
        || async { session.press_keysym(0xff09).await.map_err(Into::into) },
        || async {
            *input_uncertain = true;
            let key_name = match key {
                0x20 => "space",
                0xff50 => "home",
                0xff57 => "end",
                0xff1b => "escape",
                _ => "none",
            };
            control_operation("terminal", key_name, session.press_keysym(key)).await
        },
    )
    .await
}

async fn modal_activate(session: &Arc<Session>, tag: &str, name: &str) -> Result {
    let role = match tag {
        "Button" => "button",
        "ToggleButton" => "toggle button",
        _ => return Err("Unsupported playback keyboard activation role".into()),
    };
    modal_key(
        session,
        &format!("//{tag}[@name={}]", xpath_literal(name)),
        role,
        0x20,
    )
    .await?;
    Ok(())
}

fn modal_menu_open(tree: &str, title: &str) -> Result<bool> {
    modal_window(tree, title)?;
    let volumes = waydriver::atspi::evaluate_xpath(
        tree,
        &format!("{LIVE_PLAYER}//Slider[@name='Volume' and @showing='true' and @visible='true']"),
    )?;
    let stops = waydriver::atspi::evaluate_xpath(
        tree,
        &format!(
            "{LIVE_PLAYER}//Button[@name='Stop playback' and @showing='true' and @visible='true']"
        ),
    )?;
    match (volumes.len(), stops.len()) {
        (0, 0) => Ok(false),
        (1, 1) => Ok(true),
        _ => Err("Playback settings popover is partial or ambiguous".into()),
    }
}

fn settings_input_guard(uncertain: bool) -> Result {
    if uncertain {
        return Err(
            "Prior settings key outcome is unconfirmed; refusing further settings input".into(),
        );
    }
    Ok(())
}

async fn menu(session: &Arc<Session>, evidence: &mut Evidence) -> Result {
    settings_input_guard(evidence.settings_input_uncertain)?;
    let title = std::env::var("SCREENROOM_SHOWCASE_TITLE")?;
    let tree = session.dump_tree().await?;
    if modal_menu_open(&tree, &title)? {
        return Ok(());
    }
    reveal(session).await?;
    modal_key_tracked(
        session,
        PLAYBACK_SETTINGS,
        "toggle button",
        0x20,
        &mut evidence.settings_input_uncertain,
    )
    .await?;
    control_operation("menu-confirmation", "none", async {
        tokio::time::timeout(session.default_timeout(), async {
            loop {
                if modal_menu_open(&session.dump_tree().await?, &title)? {
                    return Ok::<(), Box<dyn Error>>(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await??;
        Ok::<(), Box<dyn Error>>(())
    })
    .await?;
    evidence.settings_input_uncertain = false;
    Ok(())
}

async fn close_menu(session: &Arc<Session>, evidence: &mut Evidence) -> Result {
    settings_input_guard(evidence.settings_input_uncertain)?;
    let title = std::env::var("SCREENROOM_SHOWCASE_TITLE")?;
    let tree = session.dump_tree().await?;
    if !modal_menu_open(&tree, &title)? {
        return Err("Cannot issue Escape without a confirmed open playback popover".into());
    }
    // A known enabled popover target keeps Escape inside the owned modal.
    modal_key_tracked(
        session,
        "//Slider[@name='Volume']",
        "slider",
        0xff1b,
        &mut evidence.settings_input_uncertain,
    )
    .await?;
    control_operation("menu-confirmation", "none", async {
        tokio::time::timeout(session.default_timeout(), async {
            loop {
                if !modal_menu_open(&session.dump_tree().await?, &title)? {
                    return Ok::<(), Box<dyn Error>>(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await??;
        Ok::<(), Box<dyn Error>>(())
    })
    .await?;
    evidence.settings_input_uncertain = false;
    Ok(())
}

#[cfg(test)]
fn focused_modal_fixture() -> String {
    // Later retained failure: focus moved from the stage to the timeline.
    MODAL.replace("sensitive=\"true\" focused=\"true\" focusable", "sensitive=\"true\" focusable")
        .replace("name=\"41:02 / 1:28:40\"", "name=\"1:19:34 / 1:28:40\"")
        .replace("bbox=\"962,530,114,18\"", "bbox=\"948,530,128,18\"")
        .replace(
            "name=\"Playback position\" showing=\"true\" visible=\"true\" sensitive=\"true\" focusable",
            "name=\"Playback position\" showing=\"true\" visible=\"true\" sensitive=\"true\" focused=\"true\" focusable",
        )
}

#[test]
fn modal_focus_contract() -> Result {
    assert!(!Evidence::new().settings_input_uncertain);
    assert!(settings_input_guard(false).is_ok());
    assert!(settings_input_guard(true).is_err());
    let tree = focused_modal_fixture();
    let initial = modal_focus(&tree, "Nosferatu", PLAYBACK_SETTINGS, "toggle button")?;
    assert_eq!(initial.target.1, "/fixture/node22");
    assert_eq!(
        initial.focused.as_ref().map(|r| r.1.as_str()),
        Some("/fixture/node14")
    );
    assert!(initial.eligible);
    let no_focus = tree.replace(" focused=\"true\"", "");
    assert!(
        modal_focus(&no_focus, "Nosferatu", PLAYBACK_SETTINGS, "toggle button")?
            .focused
            .is_none()
    );
    let settings_focused = no_focus.replace(
        "name=\"Audio, subtitles and playback settings\" showing=\"true\" visible=\"true\" sensitive=\"true\" focusable",
        "name=\"Audio, subtitles and playback settings\" showing=\"true\" visible=\"true\" sensitive=\"true\" focused=\"true\" focusable",
    );
    let settled = modal_focus(
        &settings_focused,
        "Nosferatu",
        PLAYBACK_SETTINGS,
        "toggle button",
    )?;
    assert_eq!(settled.focused.as_ref(), Some(&settled.target));
    assert!(modal_focus(&tree, "Wrong title", PLAYBACK_SETTINGS, "toggle button").is_err());
    assert!(
        modal_focus(
            &format!("<Application>{tree}{tree}</Application>"),
            "Nosferatu",
            PLAYBACK_SETTINGS,
            "toggle button"
        )
        .is_err()
    );
    assert!(
        modal_focus(
            &tree.replace("active=\"true\"", "active=\"false\""),
            "Nosferatu",
            PLAYBACK_SETTINGS,
            "toggle button"
        )
        .is_err()
    );
    assert!(modal_focus(&tree, "Nosferatu", PLAYBACK_SETTINGS, "button").is_err());
    assert!(
        modal_focus(
            &tree,
            "Nosferatu",
            "//Button[@name='Audio, subtitles and playback settings']",
            "button"
        )
        .is_err()
    );
    assert!(
        modal_focus(
            &tree,
            "Nosferatu",
            "//*[@name='Audio, subtitles and playback settings']",
            "toggle button"
        )
        .is_err()
    );
    assert!(modal_focus(&tree, "Nosferatu", "//Button[@name='Absent']", "button").is_err());
    let target = tree
        .lines()
        .find(|l| l.contains("_ref=\":1.77|/fixture/node22\""))
        .unwrap();
    for (from, to) in [
        ("showing=\"true\"", "showing=\"false\""),
        ("visible=\"true\"", "visible=\"false\""),
        ("sensitive=\"true\"", "sensitive=\"false\""),
        ("bbox=\"1032,624,44,44\"", "bbox=\"1032,624,0,44\""),
    ] {
        assert!(
            !modal_focus(
                &tree.replace(target, &target.replace(from, to)),
                "Nosferatu",
                PLAYBACK_SETTINGS,
                "toggle button"
            )?
            .eligible
        );
    }
    let outside = r#"<Button role="button" name="Other" focused="true" showing="true" visible="true" sensitive="true" focusable="true" bbox="0,0,44,44" _ref=":1.77|/fixture/outside"/>"#;
    assert!(
        modal_focus(
            &format!("<Application>{no_focus}{outside}</Application>"),
            "Nosferatu",
            PLAYBACK_SETTINGS,
            "toggle button"
        )
        .is_err()
    );
    assert!(
        modal_focus(
            &settings_focused.replace("name=\"Mute\"", "name=\"Mute\" focused=\"true\""),
            "Nosferatu",
            PLAYBACK_SETTINGS,
            "toggle button"
        )
        .is_err()
    );
    assert!(!modal_menu_open(&tree, "Nosferatu")?);

    // Source-backed popup variant, not an additional captured UI observation.
    let popup = r#"<Group role="group" showing="true" visible="true" sensitive="true" bbox="700,200,300,360" _ref=":1.77|/fixture/popup">
<Slider role="slider" name="Volume" showing="true" visible="true" sensitive="true" focused="true" focusable="true" bbox="710,210,120,44" _ref=":1.77|/fixture/volume"/>
<Button role="button" name="Stop playback" showing="true" visible="true" sensitive="true" focusable="true" bbox="710,500,150,44" _ref=":1.77|/fixture/stop"/>
</Group>"#;
    let opened = no_focus.replace("</ToggleButton>", &format!("</ToggleButton>{popup}"));
    assert!(modal_menu_open(&opened, "Nosferatu")?);
    let volume = modal_focus(&opened, "Nosferatu", "//Slider[@name='Volume']", "slider")?;
    assert_eq!(volume.focused.as_ref(), Some(&volume.target));
    assert!(
        modal_focus(
            &opened,
            "Nosferatu",
            "//Button[@name='Stop playback']",
            "button"
        )?
        .eligible
    );
    assert!(
        modal_menu_open(
            &opened.replace("name=\"Volume\"", "name=\"Absent\""),
            "Nosferatu"
        )
        .is_err()
    );
    assert!(
        modal_menu_open(
            &opened.replace(popup, &format!("{popup}{popup}")),
            "Nosferatu"
        )
        .is_err()
    );
    assert!(modal_focus(&opened, "Nosferatu", "//Slider[@name='Volume']", "button").is_err());
    Ok(())
}

#[tokio::test]
async fn modal_keyboard_policy_contract() -> Result {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    let tree = focused_modal_fixture();
    let initial = modal_focus(&tree, "Nosferatu", PLAYBACK_SETTINGS, "toggle button")?;
    let mut focused = initial.clone();
    focused.focused = Some(focused.target.clone());
    let queue = RefCell::new(VecDeque::from([
        initial.clone(),
        focused.clone(),
        focused.clone(),
    ]));
    let tabs = Cell::new(0);
    let inputs = Cell::new(0);
    let reference = modal_keyboard_policy(
        Duration::from_secs(2),
        || {
            let next = queue.borrow_mut().pop_front().unwrap();
            async { Ok(next) }
        },
        || async {
            tabs.set(tabs.get() + 1);
            Ok(())
        },
        || async {
            inputs.set(inputs.get() + 1);
            Ok(())
        },
    )
    .await?;
    assert_eq!(reference, focused.target);
    assert_eq!((tabs.get(), inputs.get()), (1, 1));

    for change_window in [false, true] {
        let mut changed = focused.clone();
        if change_window {
            changed.window.1.push_str("-replaced");
        } else {
            changed.target.1.push_str("-replaced");
            changed.focused = Some(changed.target.clone());
        }
        let queue = RefCell::new(VecDeque::from([focused.clone(), changed]));
        inputs.set(0);
        let error = modal_keyboard_policy(
            Duration::from_secs(1),
            || {
                let next = queue.borrow_mut().pop_front().unwrap();
                async { Ok(next) }
            },
            || async {
                panic!("Already-focused target must not navigate");
            },
            || async {
                inputs.set(inputs.get() + 1);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("target changed"));
        assert_eq!(inputs.get(), 0);
    }
    inputs.set(0);
    let observations = Cell::new(0);
    let error = modal_keyboard_policy(
        Duration::from_secs(1),
        || {
            observations.set(observations.get() + 1);
            let next = focused.clone();
            async { Ok(next) }
        },
        || async {
            panic!("Focused target must not navigate");
        },
        || async {
            inputs.set(inputs.get() + 1);
            Err("uncertain key dispatch".into())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "uncertain key dispatch");
    assert_eq!((observations.get(), inputs.get()), (2, 1));

    tabs.set(0);
    inputs.set(0);
    let mut missing = initial.clone();
    missing.focused = None;
    let error = modal_keyboard_policy(
        Duration::from_secs(2),
        || {
            let next = missing.clone();
            async { Ok(next) }
        },
        || async {
            tabs.set(tabs.get() + 1);
            Ok(())
        },
        || async {
            inputs.set(inputs.get() + 1);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("48 Tab"));
    assert_eq!((tabs.get(), inputs.get()), (48, 0));

    let started = std::time::Instant::now();
    let error = modal_keyboard_policy(
        Duration::from_millis(30),
        || std::future::pending::<Result<ModalFocus>>(),
        || async {
            panic!("Observation did not complete");
        },
        || async {
            inputs.set(inputs.get() + 1);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
    );
    assert!(started.elapsed() < Duration::from_millis(250));
    assert_eq!(inputs.get(), 0);
    let error = modal_keyboard_policy(
        Duration::from_secs(1),
        || {
            let next = initial.clone();
            async { Ok(next) }
        },
        || async { Err("navigation dispatch uncertain".into()) },
        || async {
            inputs.set(inputs.get() + 1);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "navigation dispatch uncertain");
    assert_eq!(inputs.get(), 0);
    inputs.set(0);
    let error = modal_keyboard_policy(
        Duration::from_millis(70),
        || {
            let next = focused.clone();
            async { Ok(next) }
        },
        || async {
            panic!("Focused target must not navigate");
        },
        || async {
            inputs.set(inputs.get() + 1);
            std::future::pending::<Result>().await
        },
    )
    .await
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
    );
    assert_eq!(inputs.get(), 1);
    Ok(())
}

fn exact_search_request(title: &str, library_id: &str) -> screenroom_client_core::SearchRequest {
    use screenroom_client_core::{SearchMethods, SearchRequest};
    SearchRequest {
        query: title.into(),
        library_id: library_id.into(),
        methods: Some(SearchMethods {
            title_only: true,
            text: true,
            fuzzy: false,
            semantic: false,
        }),
        page: Some(Page {
            size: 100,
            token: String::new(),
        }),
        ..Default::default()
    }
}

#[test]
fn exact_search_request_contract() {
    let request = exact_search_request("Nosferatu", "selected-library");
    assert_eq!(request.query, "Nosferatu");
    assert_eq!(request.library_id, "selected-library");
    assert!(request.source_id.is_empty());
    let methods = request.methods.unwrap();
    assert!(methods.title_only);
    assert!(methods.text);
    assert!(!methods.fuzzy);
    assert!(!methods.semantic);
    let page = request.page.unwrap();
    assert_eq!(page.size, 100);
    assert!(page.token.is_empty());
}

fn exact_item_index<'a>(
    items: impl Iterator<Item = (&'a str, &'a str, u32, &'a str)>,
    id: &str,
    title: &str,
    year: u32,
) -> std::result::Result<Option<usize>, &'static str> {
    let mut selected = None;
    for (index, (item_id, item_title, item_year, kind)) in items.enumerate() {
        if item_id != id {
            continue;
        }
        if item_title != title || item_year != year || kind != "Movie" {
            return Err("Explicit playback item identity/title/year/kind mismatch");
        }
        if selected.replace(index).is_some() {
            return Err("Explicit playback item ID occurs more than once in the search page");
        }
    }
    Ok(selected)
}

fn item_selector(id: &str) -> String {
    format!(
        "//Button[@description={}]",
        xpath_literal(&format!("screenroom-item:{id}"))
    )
}

fn unique_item_match<T>(matches: &[T]) -> std::result::Result<Option<&T>, &'static str> {
    match matches {
        [] => Ok(None),
        [item] => Ok(Some(item)),
        _ => Err("Item-ID accessibility selector matched multiple UI cards"),
    }
}

async fn activate_exact_item(session: &Arc<Session>, id: &str) -> Result {
    pointer_settled(session, &item_selector(id), 0, 1).await
}

const TITLE_ONLY_SELECTOR: &str = "//Checkbox[@name='Title only']";

fn title_only_checked(tree: &str) -> Result<bool> {
    let mut controls = tree.lines().filter(|line| {
        snapshot_attribute(line, "role") == Some("checkbox")
            && snapshot_attribute(line, "name") == Some("Title only")
    });
    let control = controls.next().ok_or("Title-only checkbox is absent")?;
    if controls.next().is_some() {
        return Err("Title-only selector matched multiple checkboxes".into());
    }
    let reference =
        snapshot_attribute(control, "_ref").ok_or("Title-only checkbox has no reference")?;
    if pointer_snapshot_bounds(control, reference).is_none() {
        return Err("Title-only checkbox is not visible, sensitive and laid out".into());
    }
    Ok(snapshot_attribute(control, "checked") == Some("true"))
}

#[test]
fn title_only_control_contract() {
    // Actual second focused failure lines 117–120; only opaque references sanitized.
    let tree = r#"<Checkbox role="checkbox" name="Title only" showing="true" visible="true" sensitive="true" focusable="true" checkable="true" bbox="164,562,1014,32" toolkit="GTK" _ref=":1.2|/checkbox">
  <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="164,562,20,20" toolkit="GTK" _ref=":1.2|/generic"/>
  <Label role="label" name="Title only" showing="true" visible="true" sensitive="true" bbox="188,562,990,20" toolkit="GTK" _ref=":1.2|/label"/>
</Checkbox>"#;
    assert!(!title_only_checked(tree).unwrap());
    assert!(
        title_only_checked(&tree.replacen(
            "checkable=\"true\"",
            "checked=\"true\" checkable=\"true\"",
            1
        ))
        .unwrap()
    );
    assert_eq!(TITLE_ONLY_SELECTOR, "//Checkbox[@name='Title only']");
    assert!(
        title_only_checked(&format!("{tree}\n{tree}"))
            .unwrap_err()
            .to_string()
            .contains("multiple checkboxes")
    );
    for attribute in ["showing", "visible", "sensitive"] {
        assert!(
            title_only_checked(&tree.replacen(
                &format!("{attribute}=\"true\""),
                &format!("{attribute}=\"false\""),
                1
            ))
            .is_err()
        );
        assert!(
            title_only_checked(&tree.replacen(&format!(" {attribute}=\"true\""), "", 1)).is_err()
        );
    }
    assert!(title_only_checked(&tree.replace("role=\"checkbox\"", "role=\"label\"")).is_err());
    assert!(
        title_only_checked(&tree.replace("bbox=\"164,562,1014,32\"", "bbox=\"164,562,0,32\""))
            .is_err()
    );
}

async fn configure_playback_search(session: &Arc<Session>, stage: &mut &'static str) -> Result {
    enter_stage(stage, "search-settings-open");
    click(session, "Search settings").await?;
    session
        .locate(TITLE_ONLY_SELECTOR)
        .wait_for_visible()
        .await?;
    enter_stage(stage, "title-only");
    let checked = || async { title_only_checked(&session.dump_tree().await?) };
    if !checked().await? {
        pointer_settled(session, TITLE_ONLY_SELECTOR, 0, 1).await?;
        tokio::time::timeout(session.default_timeout(), async {
            while !checked().await? {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<(), Box<dyn Error>>(())
        })
        .await??;
    }
    enter_stage(stage, "search-settings-close");
    click(session, "Search settings").await?;
    session
        .locate(TITLE_ONLY_SELECTOR)
        .wait_for_hidden()
        .await?;
    Ok(())
}

#[test]
fn exact_item_selector_contract() {
    let cards = [
        ("other", "Nosferatu", 1922, "Movie"),
        ("requested", "Nosferatu", 1922, "Movie"),
    ];
    assert_eq!(
        exact_item_index(cards.into_iter(), "requested", "Nosferatu", 1922),
        Ok(Some(1))
    );
    assert_eq!(
        exact_item_index(cards.into_iter(), "missing", "Nosferatu", 1922),
        Ok(None)
    );
    assert!(
        exact_item_index(
            [cards[1], cards[1]].into_iter(),
            "requested",
            "Nosferatu",
            1922
        )
        .is_err()
    );
    for wrong in [
        ("requested", "Wrong", 1922, "Movie"),
        ("requested", "Nosferatu", 2024, "Movie"),
        ("requested", "Nosferatu", 1922, "Episode"),
    ] {
        assert!(exact_item_index([wrong].into_iter(), "requested", "Nosferatu", 1922).is_err());
    }
    assert_eq!(
        item_selector("jellyfin:source/item"),
        "//Button[@description='screenroom-item:jellyfin:source/item']"
    );
    assert_eq!(
        item_selector("a'\"b"),
        "//Button[@description=concat('screenroom-item:a', \"'\", '\"b', '')]"
    );
    assert_eq!(unique_item_match::<u8>(&[]), Ok(None));
    assert_eq!(unique_item_match(&[7]), Ok(Some(&7)));
    assert!(unique_item_match(&[7, 7]).is_err());
}

async fn exact_selection() -> Result<(MediaLibrary, Item)> {
    let id = std::env::var("SCREENROOM_SHOWCASE_ITEM_ID")?;
    let title = std::env::var("SCREENROOM_SHOWCASE_TITLE")?;
    let year: u32 = std::env::var("SCREENROOM_SHOWCASE_YEAR")?.parse()?;
    let libraries = client()?
        .list_libraries(
            String::new(),
            Page {
                size: 8,
                token: String::new(),
            },
        )
        .await?;
    for library in libraries.libraries {
        let page = client()?
            .search(exact_search_request(&title, &library.id))
            .await?;
        let selected = exact_item_index(
            page.items.iter().map(|item| {
                (
                    item.id.as_str(),
                    item.title.as_str(),
                    item.year,
                    item.kind.as_str(),
                )
            }),
            &id,
            &title,
            year,
        )?;
        if let Some(index) = selected {
            return Ok((library, page.items[index].clone()));
        }
    }
    Err("Explicit showcase item absent from bounded title search".into())
}

fn changed_media_pixels(first: &image::RgbImage, second: &image::RgbImage) -> usize {
    // Exclude the header, footer, window borders and catalog behind the modal.
    (260..540)
        .flat_map(|y| (420..860).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            first
                .get_pixel(x, y)
                .0
                .iter()
                .zip(second.get_pixel(x, y).0)
                .any(|(a, b)| a.abs_diff(b) > 12)
        })
        .count()
}

fn motion_result(changed: usize) -> Result {
    if changed < 1000 {
        return Err(
            "Insufficient changing central media pixels; media visibility is unproven".into(),
        );
    }
    Ok(())
}

fn motion_gate(journey: &str, phase: &str, diagnostics: &str) -> bool {
    journey == "playback" && phase == "motion" && diagnostics == "1"
}

fn motion_diagnostics() -> bool {
    motion_gate(
        &std::env::var("SCREENROOM_E2E_JOURNEY").unwrap_or_default(),
        &std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE").unwrap_or_default(),
        &std::env::var("SCREENROOM_PLAYBACK_DIAGNOSTICS").unwrap_or_default(),
    )
}

fn motion_timeline_sample(start: u128, end: u128, position: Option<f64>) -> Option<Value> {
    if end < start {
        return None;
    }
    let position = position.filter(|value| value.is_finite() && *value >= 0.0);
    Some(json!({
        "elapsed_start_ms":start, "elapsed_end_ms":end,
        "status":if position.is_some() { "observed" } else { "unavailable" },
        "position_ms":position
    }))
}

struct MotionDiagnostic {
    directory: std::path::PathBuf,
    started: std::time::Instant,
    data: Value,
}

impl MotionDiagnostic {
    fn new(enabled: bool, directory: &std::path::Path) -> Option<Self> {
        if !enabled {
            return None;
        }
        let mut diagnostic = Self {
            directory: directory.to_owned(),
            started: std::time::Instant::now(),
            data: json!({
                "schemaVersion":1, "check":"motion", "status":"started",
                "viewport":[1280,800],
                "region":{"x":420,"y":260,"width":440,"height":280},
                "total_pixels":123200, "channel_delta_strictly_greater_than":12,
                "required_changed_pixels":1000, "requested_sleep_ms":2000,
                "changed_pixels":null,
                "samples":[
                    {"file":"playback-motion-first.png","capture_start_ms":null,"capture_end_ms":null,"capture_status":"not-attempted","validation":"not-attempted","dimensions":null,"retained":false},
                    {"file":"playback-motion-second.png","capture_start_ms":null,"capture_end_ms":null,"capture_status":"not-attempted","validation":"not-attempted","dimensions":null,"retained":false}
                ],
                "timeline":{"before":null,"after":null,"same_slider":null},
                "timeline_scope":"Before first capture and after second capture; positions alone do not prove changing media pixels."
            }),
        };
        diagnostic.flush();
        Some(diagnostic)
    }

    fn elapsed(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    fn flush(&mut self) -> bool {
        serde_json::to_vec_pretty(&self.data)
            .ok()
            .is_some_and(|bytes| {
                fs::write(self.directory.join("playback-motion.json"), bytes).is_ok()
            })
    }

    fn retain(&mut self, index: usize, png: &[u8]) {
        let name = self.data["samples"][index]["file"].as_str().unwrap();
        self.data["samples"][index]["retained"] =
            json!(fs::write(self.directory.join(name), png).is_ok());
        self.flush();
    }

    fn begin_capture(&mut self, index: usize) {
        self.data["samples"][index]["capture_start_ms"] = json!(self.elapsed());
        self.data["samples"][index]["capture_status"] = json!("pending");
        self.flush();
    }

    fn end_capture(&mut self, index: usize) {
        self.data["samples"][index]["capture_end_ms"] = json!(self.elapsed());
    }

    fn failed_capture(&mut self, index: usize) {
        self.data["samples"][index]["capture_status"] = json!("failed");
        self.data["status"] = json!("capture-failed");
        self.flush();
    }

    fn complete(&mut self, changed: usize, same_slider: Option<bool>) -> Result {
        self.data["timeline"]["same_slider"] = json!(same_slider);
        self.data["changed_pixels"] = json!(changed);
        self.data["status"] = json!(if changed < 1000 {
            "comparison-failed"
        } else {
            "passed"
        });
        let written = self.flush();
        motion_result(changed)?;
        if !written
            || !self.data["samples"]
                .as_array()
                .unwrap()
                .iter()
                .all(|sample| sample["retained"] == true)
        {
            return Err("Motion diagnostics could not retain exact comparison evidence".into());
        }
        Ok(())
    }

    fn validate_sample(&mut self, index: usize, png: &[u8]) -> Result {
        self.data["samples"][index]["capture_status"] = json!("acquired");
        self.retain(index, png);
        let image = match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
            Ok(image) => image.to_rgb8(),
            Err(error) => {
                self.data["samples"][index]["validation"] = json!("decode-failed");
                self.data["status"] = json!("validation-failed");
                self.flush();
                return Err(error.into());
            }
        };
        self.data["samples"][index]["dimensions"] = json!([image.width(), image.height()]);
        let validation = validate_frame(&image);
        self.data["samples"][index]["validation"] = json!(if validation.is_ok() {
            "passed"
        } else if image.dimensions() != (1280, 800) {
            "dimensions-failed"
        } else {
            "blank-failed"
        });
        if validation.is_err() {
            self.data["status"] = json!("validation-failed");
        }
        self.flush();
        validation
    }
}

async fn motion_timeline(
    session: &Arc<Session>,
    diagnostic: &mut MotionDiagnostic,
    slot: &str,
) -> Option<(String, String)> {
    let start = diagnostic.elapsed();
    let observed = tokio::time::timeout(Duration::from_millis(750), async {
        let tree = session.dump_tree().await?;
        modal_window(&tree, &std::env::var("SCREENROOM_SHOWCASE_TITLE")?)?;
        let sliders = waydriver::atspi::evaluate_xpath(
            &tree,
            &format!("{LIVE_PLAYER}//Slider[@name='Playback position']"),
        )?;
        let [reference] = sliders.as_slice() else {
            return Err("Motion timeline requires exactly one owned modal slider".into());
        };
        let value = session.value_ref(&reference.0, &reference.1).await?;
        let position = live_position(value.current, value.minimum, value.maximum)?;
        Ok::<_, Box<dyn Error>>((reference.clone(), position))
    })
    .await;
    let end = diagnostic.elapsed();
    let observed = observed.ok().and_then(std::result::Result::ok);
    diagnostic.data["timeline"][slot] =
        motion_timeline_sample(start, end, observed.as_ref().map(|(_, position)| *position))
            .unwrap_or(Value::Null);
    diagnostic.flush();
    observed.map(|(reference, _)| reference)
}

async fn motion_frame(
    session: &Arc<Session>,
    diagnostic: &mut Option<MotionDiagnostic>,
    index: usize,
) -> Result<Vec<u8>> {
    let Some(diagnostic) = diagnostic else {
        return frame(session).await;
    };
    diagnostic.begin_capture(index);
    let captured = session.take_screenshot().await;
    diagnostic.end_capture(index);
    let png = match captured {
        Ok(png) => png,
        Err(error) => {
            diagnostic.failed_capture(index);
            return Err(error.into());
        }
    };
    diagnostic.validate_sample(index, &png)?;
    Ok(png)
}

async fn moving_media(session: &Arc<Session>) -> Result {
    let mut diagnostic =
        MotionDiagnostic::new(motion_diagnostics(), std::path::Path::new("/artifacts"));
    let before = if let Some(diagnostic) = diagnostic.as_mut() {
        motion_timeline(session, diagnostic, "before").await
    } else {
        None
    };
    let first =
        image::load_from_memory(&motion_frame(session, &mut diagnostic, 0).await?)?.to_rgb8();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let second =
        image::load_from_memory(&motion_frame(session, &mut diagnostic, 1).await?)?.to_rgb8();
    let after = if let Some(diagnostic) = diagnostic.as_mut() {
        motion_timeline(session, diagnostic, "after").await
    } else {
        None
    };
    let changed = changed_media_pixels(&first, &second);
    if let Some(diagnostic) = diagnostic.as_mut() {
        return diagnostic.complete(
            changed,
            before.zip(after).map(|(before, after)| before == after),
        );
    }
    motion_result(changed)
}

#[test]
fn motion_policy_contract() {
    let first = image::RgbImage::from_pixel(1280, 800, image::Rgb([30, 30, 30]));
    let mut second = first.clone();
    assert_eq!(changed_media_pixels(&first, &second), 0);
    assert!(motion_result(0).is_err());
    for count in [999, 1000] {
        second = first.clone();
        for index in 0..count {
            second.put_pixel(
                420 + index % 440,
                260 + index / 440,
                image::Rgb([43, 30, 30]),
            );
        }
        assert_eq!(changed_media_pixels(&first, &second), count as usize);
        assert_eq!(motion_result(count as usize).is_ok(), count == 1000);
    }
    second = first.clone();
    for x in 420..860 {
        second.put_pixel(x, 260, image::Rgb([42, 42, 42]));
    }
    assert_eq!(changed_media_pixels(&first, &second), 0);
    for (x, y) in [(419, 260), (860, 260), (420, 259), (420, 540)] {
        second.put_pixel(x, y, image::Rgb([255, 255, 255]));
    }
    assert_eq!(changed_media_pixels(&first, &second), 0);
    second.put_pixel(420, 260, image::Rgb([30, 30, 43]));
    second.put_pixel(859, 539, image::Rgb([30, 43, 30]));
    assert_eq!(changed_media_pixels(&first, &second), 2);
    assert!(
        motion_result(2)
            .unwrap_err()
            .to_string()
            .contains("media visibility is unproven")
    );
    assert!(
        validate_frame(&first)
            .unwrap_err()
            .to_string()
            .contains("Blank")
    );
    assert!(
        validate_frame(&image::RgbImage::new(1, 1))
            .unwrap_err()
            .to_string()
            .contains("1280x800")
    );
}

#[test]
fn motion_diagnostic_contract() -> Result {
    fn png(image: image::RgbImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
    for journey in ["", "browse", "playback", "PLAYBACK"] {
        for phase in ["", "capture", "navigation", "readiness", "motion", "Motion"] {
            for flag in ["", "0", "1", "true", "01"] {
                assert_eq!(
                    motion_gate(journey, phase, flag),
                    journey == "playback" && phase == "motion" && flag == "1"
                );
            }
        }
    }
    let directory = std::env::temp_dir().join(format!(
        "screenroom-motion-contract-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    assert!(MotionDiagnostic::new(false, &directory).is_none());
    assert!(!directory.exists());
    fs::create_dir(&directory)?;
    let first = png(image::RgbImage::from_fn(1280, 800, |x, y| {
        image::Rgb([x as u8, y as u8, (x + y) as u8])
    }));
    let mut image = image::load_from_memory(&first)?.to_rgb8();
    for index in 0..1000 {
        let pixel = image.get_pixel_mut(420 + index % 440, 260 + index / 440);
        pixel.0[0] = pixel.0[0].wrapping_add(13);
    }
    let second = png(image);
    let mut diagnostic = MotionDiagnostic::new(true, &directory).unwrap();
    diagnostic.data["timeline"]["before"] = motion_timeline_sample(0, 0, Some(1015962.0)).unwrap();
    diagnostic.begin_capture(0);
    diagnostic.end_capture(0);
    diagnostic.validate_sample(0, &first)?;
    assert_eq!(
        fs::read(directory.join("playback-motion-first.png"))?,
        first
    );
    assert!(!directory.join("playback-motion-second.png").exists());
    diagnostic.begin_capture(1);
    diagnostic.end_capture(1);
    diagnostic.validate_sample(1, &second)?;
    assert_eq!(
        fs::read(directory.join("playback-motion-second.png"))?,
        second
    );
    assert_ne!(first, second);
    for index in 0..2 {
        let sample = &diagnostic.data["samples"][index];
        assert!(sample["capture_end_ms"].as_u64() >= sample["capture_start_ms"].as_u64());
        assert_eq!(sample["dimensions"], json!([1280, 800]));
        assert_eq!(sample["validation"], "passed");
        assert_eq!(sample["retained"], true);
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        let sample = motion_timeline_sample(1, 2, Some(value)).unwrap();
        assert_eq!(sample["position_ms"], Value::Null);
        assert_eq!(sample["status"], "unavailable");
    }
    assert!(motion_timeline_sample(2, 1, Some(42.0)).is_none());
    let sample = motion_timeline_sample(0, 2, Some(1015962.0)).unwrap();
    assert_eq!(sample["position_ms"], 1015962.0);
    assert_eq!(sample["status"], "observed");
    let elapsed = diagnostic.elapsed();
    diagnostic.data["timeline"]["after"] =
        motion_timeline_sample(elapsed, elapsed, Some(1016965.0)).unwrap();
    let changed = changed_media_pixels(
        &image::load_from_memory(&first)?.to_rgb8(),
        &image::load_from_memory(&second)?.to_rgb8(),
    );
    assert_eq!(changed, 1000);
    diagnostic.complete(changed, Some(true))?;
    let saved: Value = serde_json::from_slice(&fs::read(directory.join("playback-motion.json"))?)?;
    assert_eq!(saved["total_pixels"], 123200);
    assert_eq!(saved["status"], "passed");
    assert_eq!(saved["changed_pixels"], 1000);
    assert_eq!(
        saved["region"],
        json!({"x":420,"y":260,"width":440,"height":280})
    );
    assert!(serde_json::to_vec(&saved)?.len() < 8192);
    assert!(!directory.join("manifest.json").exists());
    let original_directory = diagnostic.directory.clone();
    diagnostic.directory = directory.join("absent");
    assert_eq!(
        diagnostic.complete(999, None).unwrap_err().to_string(),
        motion_result(999).unwrap_err().to_string()
    );
    assert!(
        diagnostic
            .complete(1000, None)
            .unwrap_err()
            .to_string()
            .contains("retain")
    );
    diagnostic.directory = original_directory;
    let partial_directory = directory.join("partial");
    fs::create_dir(&partial_directory)?;
    let mut partial = MotionDiagnostic::new(true, &partial_directory).unwrap();
    partial.begin_capture(0);
    partial.end_capture(0);
    partial.validate_sample(0, &first)?;
    partial.begin_capture(1);
    partial.end_capture(1);
    partial.failed_capture(1);
    let partial_saved: Value =
        serde_json::from_slice(&fs::read(partial_directory.join("playback-motion.json"))?)?;
    assert_eq!(partial_saved["status"], "capture-failed");
    assert_eq!(partial_saved["changed_pixels"], Value::Null);
    assert_eq!(partial_saved["samples"][1]["validation"], "not-attempted");
    assert!(
        !partial_directory
            .join("playback-motion-second.png")
            .exists()
    );
    assert_eq!(
        fs::read(partial_directory.join("playback-motion-first.png"))?,
        first
    );
    let validation_directory = directory.join("validation");
    fs::create_dir(&validation_directory)?;
    let mut diagnostic = MotionDiagnostic::new(true, &validation_directory).unwrap();
    diagnostic.validate_sample(0, &first)?;
    assert!(diagnostic.validate_sample(1, b"invalid PNG").is_err());
    assert_eq!(diagnostic.data["status"], "validation-failed");
    assert_eq!(diagnostic.data["samples"][1]["validation"], "decode-failed");
    assert_eq!(diagnostic.data["changed_pixels"], Value::Null);
    assert_eq!(
        fs::read(validation_directory.join("playback-motion-first.png"))?,
        first
    );
    assert!(
        diagnostic
            .validate_sample(1, &png(image::RgbImage::new(1, 1)))
            .is_err()
    );
    assert_eq!(
        diagnostic.data["samples"][1]["validation"],
        "dimensions-failed"
    );
    assert!(
        diagnostic
            .validate_sample(1, &png(image::RgbImage::new(1280, 800)))
            .is_err()
    );
    assert_eq!(diagnostic.data["samples"][1]["validation"], "blank-failed");
    fs::remove_dir_all(directory)?;
    Ok(())
}

fn track_buttons(section: &str) -> String {
    format!(
        "//Generic[Label[@name='Audio'] and Label[@name='Subtitles']]/Button[Label[@name=../@name]][preceding-sibling::Label[1][@name={}]]",
        xpath_literal(section)
    )
}

#[test]
fn track_structure_contract() {
    use waydriver::atspi::{evaluate_xpath, evaluate_xpath_detailed};

    let actual = r#"<Generic role="generic" showing="true" visible="true" sensitive="true" bbox="924,383,289,194" toolkit="GTK" _ref=":1.77|/fixture/track0">
  <Label role="label" name="Audio" showing="true" visible="true" sensitive="true" bbox="924,383,42,18" toolkit="GTK" _ref=":1.77|/fixture/track1"/>
  <Button role="button" name="English · DTS · 6 channels · selected" showing="true" visible="true" focusable="true" bbox="924,409,289,54" keyshortcuts="" toolkit="GTK" _ref=":1.77|/fixture/track2">
    <Label role="label" name="English · DTS · 6 channels · selected" showing="true" visible="true" sensitive="true" bbox="924,409,255,44" toolkit="GTK" _ref=":1.77|/fixture/track3"/>
  </Button>
  <Label role="label" name="Subtitles" showing="true" visible="true" sensitive="true" bbox="924,471,65,18" toolkit="GTK" _ref=":1.77|/fixture/track4"/>
  <Button role="button" name="Off · selected" showing="true" visible="true" sensitive="true" focusable="true" bbox="924,497,289,54" keyshortcuts="" toolkit="GTK" _ref=":1.77|/fixture/track5">
    <Label role="label" name="Off · selected" showing="true" visible="true" sensitive="true" bbox="924,497,255,44" toolkit="GTK" _ref=":1.77|/fixture/track6"/>
  </Button>
  <Button role="button" name="Session information" showing="true" visible="true" sensitive="true" focusable="true" expandable="true" bbox="924,559,289,18" toolkit="GTK" _ref=":1.77|/fixture/track7">
    <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="924,559,289,18" toolkit="GTK" _ref=":1.77|/fixture/track8">
      <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="924,559,289,18" toolkit="GTK" _ref=":1.77|/fixture/track9">
        <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="924,559,16,18" toolkit="GTK" _ref=":1.77|/fixture/track10"/>
        <Label role="label" name="Session information" showing="true" visible="true" sensitive="true" bbox="940,559,146,18" toolkit="GTK" _ref=":1.77|/fixture/track11"/>
      </Generic>
    </Generic>
  </Button>
</Generic>"#;
    let audio = track_buttons("Audio");
    let subtitles = track_buttons("Subtitles");
    assert_eq!(
        evaluate_xpath(actual, &audio).unwrap(),
        vec![(":1.77".into(), "/fixture/track2".into())]
    );
    assert_eq!(
        evaluate_xpath(actual, &subtitles).unwrap(),
        vec![(":1.77".into(), "/fixture/track5".into())]
    );
    let old_subtitles = subtitles.replace("/Button[Label[@name=../@name]]", "/Button");
    assert_eq!(evaluate_xpath(actual, &old_subtitles).unwrap().len(), 2);
    let selected_audio = evaluate_xpath_detailed(actual, &audio).unwrap();
    let selected_off = evaluate_xpath_detailed(actual, &subtitles).unwrap();
    for entry in [&selected_audio[0], &selected_off[0]] {
        assert!(choice_name(entry.name.as_deref().unwrap()).1);
        assert!(!track_choice_unavailable(true, &entry.states));
        assert!(!entry.states.iter().any(|state| state == "checked"));
    }
    assert!(
        !selected_audio[0]
            .states
            .iter()
            .any(|state| state == "sensitive")
    );
    assert!(
        selected_off[0]
            .states
            .iter()
            .any(|state| state == "sensitive")
    );

    fn choice(id: &str, name: &str, sensitive: bool) -> String {
        format!(
            r#"<Button role="button" name="{name}" showing="true" visible="true" {} focusable="true" bbox="924,409,289,54" _ref=":1.77|/fixture/{id}">
  <Label role="label" name="{name}" showing="true" visible="true" sensitive="true" bbox="924,409,255,44" _ref=":1.77|/fixture/{id}-label"/>
</Button>
"#,
            if sensitive { "sensitive=\"true\"" } else { "" }
        )
    }
    let audio_boundary = "  <Label role=\"label\" name=\"Subtitles\"";
    let subtitle_boundary = "  <Button role=\"button\" name=\"Session information\"";
    let variants = actual
        .replace(
            audio_boundary,
            &format!(
                "{}{}{audio_boundary}",
                choice("spanish", "Spanish · AAC · 2 channels", true),
                choice("german", "German · AAC · 2 channels", false),
            ),
        )
        .replace(
            subtitle_boundary,
            &format!(
                "{}{}{subtitle_boundary}",
                choice("french", "French · WebVTT", true),
                choice("japanese", "日本語 · burn-in", false),
            ),
        );
    let audio_choices = evaluate_xpath_detailed(&variants, &audio).unwrap();
    let subtitle_choices = evaluate_xpath_detailed(&variants, &subtitles).unwrap();
    assert_eq!(audio_choices.len(), 3);
    assert_eq!(subtitle_choices.len(), 3);
    for (entries, expected) in [
        (
            &audio_choices,
            [
                ("English · DTS · 6 channels", true, false),
                ("Spanish · AAC · 2 channels", false, false),
                ("German · AAC · 2 channels", false, true),
            ],
        ),
        (
            &subtitle_choices,
            [
                ("Off", true, false),
                ("French · WebVTT", false, false),
                ("日本語 · burn-in", false, true),
            ],
        ),
    ] {
        for (entry, (name, selected, unavailable)) in entries.iter().zip(expected) {
            assert_eq!(
                choice_name(entry.name.as_deref().unwrap()),
                (name, selected)
            );
            assert_eq!(
                track_choice_unavailable(selected, &entry.states),
                unavailable
            );
        }
    }
    let duplicate = variants.replace(
        subtitle_boundary,
        &format!(
            "{}{subtitle_boundary}",
            choice("french-duplicate", "French · WebVTT", true)
        ),
    );
    let french = format!("{subtitles}[@name='French · WebVTT']");
    assert_eq!(evaluate_xpath(&duplicate, &french).unwrap().len(), 2);
    let modal = format!(
        r#"<Window role="window" name="Nosferatu" active="true" modal="true" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" _ref=":1.77|/fixture/window">
{duplicate}
</Window>"#
    );
    assert_eq!(
        modal_focus(&modal, "Nosferatu", &french, "button")
            .err()
            .unwrap()
            .to_string(),
        "Playback keyboard target must be unique in the active modal"
    );

    let ordinary_same_name = actual.replace(
        subtitle_boundary,
        &format!(
            "{}{subtitle_boundary}",
            choice("ordinary", "Session information", true)
        ),
    );
    assert_eq!(
        evaluate_xpath(
            &ordinary_same_name,
            &format!("{subtitles}[@name='Session information']")
        )
        .unwrap(),
        vec![(":1.77".into(), "/fixture/ordinary".into())]
    );
    let mismatched_label = actual.replace(
        r#"<Label role="label" name="Off · selected""#,
        r#"<Label role="label" name="Different label""#,
    );
    assert!(
        evaluate_xpath(&mismatched_label, &subtitles)
            .unwrap()
            .is_empty()
    );
}

fn choice_name(name: &str) -> (&str, bool) {
    match name.strip_suffix(" · selected") {
        Some(base) => (base, true),
        None => (name, false),
    }
}

async fn settled_choice(session: &Arc<Session>, selector: &str, expected: &str) -> Result {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(message) = media_failure(session).await? {
                return Err::<(), Box<dyn Error>>(message.into());
            }
            let entries = session.locate(selector).inspect_all().await?;
            let name = entries
                .first()
                .and_then(|entry| entry.name.as_deref())
                .ok_or("Requested track choice disappeared")?;
            let (base, selected) = choice_name(name);
            if base != expected {
                return Err("Track inventory order or accessible name changed unexpectedly".into());
            }
            if selected
                && session.locate(PLAYER_PLAY).first().is_enabled().await?
                && session
                    .locate("//*[@name='Playback position']")
                    .first()
                    .is_enabled()
                    .await?
            {
                let before = position(session).await?;
                let first = image::load_from_memory(&frame(session).await?)?.to_rgb8();
                tokio::time::sleep(Duration::from_millis(750)).await;
                let second = image::load_from_memory(&frame(session).await?)?.to_rgb8();
                let first = image::imageops::crop_imm(&first, 420, 260, 440, 280).to_image();
                let second = image::imageops::crop_imm(&second, 420, 260, 440, 280).to_image();
                let nonblank = second
                    .pixels()
                    .map(|pixel| pixel.0)
                    .collect::<HashSet<_>>()
                    .len()
                    > 8;
                if nonblank && first == second && (position(session).await? - before).abs() <= 250.0
                {
                    // Re-read after media settling; selection must still be the requested choice.
                    let current = session
                        .locate(selector)
                        .name()
                        .await?
                        .ok_or("Settled track lost its accessible name")?;
                    if choice_name(&current) == (expected, true)
                        && session.locate(PLAYER_PLAY).first().is_enabled().await?
                        && session
                            .locate("//*[@name='Playback position']")
                            .first()
                            .is_enabled()
                            .await?
                    {
                        return Ok(());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await??;
    Ok(())
}

fn track_choice_unavailable(selected: bool, states: &[String]) -> bool {
    !selected
        && !states
            .iter()
            .any(|state| state == "enabled" || state == "sensitive")
}

#[test]
fn track_eligibility_contract() {
    let cases: &[(&str, bool, &[&str], bool, bool)] = &[
        (
            "GTK sensitive",
            false,
            &["showing", "visible", "sensitive", "focusable"],
            false,
            true,
        ),
        ("enabled alias", false, &["enabled"], false, true),
        (
            "both aliases",
            false,
            &["enabled", "sensitive"],
            false,
            true,
        ),
        (
            "genuinely insensitive",
            false,
            &["showing", "visible"],
            true,
            false,
        ),
        ("empty", false, &[], true, false),
        ("unknown only", false, &["future-state"], true, false),
        (
            "malformed aliases",
            false,
            &["Sensitive", "ENABLED", "enabled=true", " sensitive", ""],
            true,
            false,
        ),
        (
            "recognized plus unknown",
            false,
            &["sensitive", "future-state"],
            false,
            true,
        ),
        (
            "selected insensitive",
            true,
            &["showing", "visible"],
            false,
            false,
        ),
        ("selected enabled", true, &["enabled"], false, false),
        ("selected empty", true, &[], false, false),
    ];
    for &(label, selected, states, unavailable, selection_input) in cases {
        let states: Vec<String> = states.iter().map(|state| (*state).into()).collect();
        assert_eq!(
            track_choice_unavailable(selected, &states),
            unavailable,
            "{label}"
        );
        assert_eq!(!selected && !unavailable, selection_input, "{label}");
    }
    let actual_gtk_button_states =
        ["showing", "visible", "sensitive", "focusable"].map(String::from);
    assert!(!track_choice_unavailable(false, &actual_gtk_button_states));
    assert!(
        !actual_gtk_button_states
            .iter()
            .any(|state| state == "enabled")
    );
}

async fn track_states(session: &Arc<Session>, evidence: &mut Evidence) -> Result {
    let mut inventory = Vec::new();
    for (section, kind) in [("Audio", "audio"), ("Subtitles", "subtitle")] {
        let selector = track_buttons(section);
        let entries = session.locate(&selector).inspect_all().await?;
        let mut choices = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let name = entry
                .name
                .as_deref()
                .ok_or("Track choice has no accessible name")?;
            let (base, selected) = choice_name(name);
            if section == "Subtitles" && base == "Off" {
                continue;
            }
            let disabled = track_choice_unavailable(selected, &entry.states);
            choices.push((index + 1, base.to_owned(), disabled));
        }
        if choices.is_empty() {
            let id = format!("{kind}-tracks");
            evidence.add_choice(&id, &format!("{section} choices"));
            evidence.unavailable(
                &id,
                "The real session exposes no discrete choices in this section.",
            );
        }
        let mut indexed = Vec::new();
        for (index, (ui_index, expected, disabled)) in choices.into_iter().enumerate() {
            let id = format!("{kind}-track-{:02}", index + 1);
            evidence.add_choice(&id, &format!("{section} choice {}", index + 1));
            if disabled {
                evidence.unavailable(
                    &id,
                    "This discrete choice is disabled in the current GTK menu.",
                );
            }
            indexed.push((id, ui_index, expected));
        }
        inventory.push((selector, indexed));
    }
    for (selector, choices) in inventory {
        for (id, ui_index, expected) in choices {
            let choice = format!("({selector})[{ui_index}]");
            let current = session
                .locate(&choice)
                .name()
                .await?
                .ok_or("Track choice disappeared before selection")?;
            let (base, selected) = choice_name(&current);
            if base != expected {
                return Err("Track inventory changed before selection".into());
            }
            if !selected && !session.locate(&choice).is_enabled().await? {
                evidence.unavailable(
                    &id,
                    "This discrete choice is disabled in the current GTK menu.",
                );
                continue;
            }
            if !selected {
                modal_key(
                    session,
                    &format!("{selector}[@name={}]", xpath_literal(&current)),
                    "button",
                    0x20,
                )
                .await?;
            }
            control_operation(
                "track-settlement",
                "none",
                settled_choice(session, &choice, &expected),
            )
            .await?;
            evidence.capture(session, &id, if selected {
                "Already-selected real UI choice captured without a redundant request; selected label, enabled controls and stable paused media asserted."
            } else {
                "Enabled real UI choice selected normally; requested selected label, enabled controls and stable paused media asserted; no retry."
            }).await?;
        }
    }
    let off = format!("({})[1]", track_buttons("Subtitles"));
    let current = session
        .locate(&off)
        .name()
        .await?
        .ok_or("Subtitles Off is absent from the real menu")?;
    let (name, selected) = choice_name(&current);
    if name != "Off" {
        return Err("Subtitle menu no longer exposes Off as its first choice".into());
    }
    if !selected && !session.locate(&off).is_enabled().await? {
        evidence.unavailable(
            "subtitles-off",
            "Subtitles Off is disabled in the current GTK menu.",
        );
        return Ok(());
    }
    if !selected {
        modal_key(
            session,
            &format!(
                "{}[@name={}]",
                track_buttons("Subtitles"),
                xpath_literal(&current)
            ),
            "button",
            0x20,
        )
        .await?;
    }
    control_operation(
        "track-settlement",
        "none",
        settled_choice(session, &off, "Off"),
    )
    .await?;
    evidence.capture(session, "subtitles-off", "Actual Off selected label, enabled controls and stable paused media asserted; no redundant request if already selected.").await?;
    Ok(())
}

#[test]
fn track_inventory_contract() {
    assert_eq!(
        choice_name("English · WebVTT · selected"),
        ("English · WebVTT", true)
    );
    assert_eq!(choice_name("Off"), ("Off", false));
    assert_eq!(choice_name("Off · selected"), ("Off", true));
    let mut evidence = Evidence::new();
    evidence.add_choice("audio-track-01", "Audio choice 1");
    evidence.unavailable("audio-track-01", "Disabled by the real menu.");
    evidence.block_remaining();
    assert_eq!(evidence.row("audio-track-01")["status"], "unavailable");
    assert!(evidence.row("audio-track-01").get("image").is_none());
    assert_eq!(evidence.row("subtitles-off")["status"], "blocked");
}

fn idle_gate(journey: &str, phase: &str, diagnostics: &str) -> bool {
    journey == "playback" && phase == "idle" && diagnostics == "1"
}

fn idle_diagnostics() -> bool {
    idle_gate(
        &std::env::var("SCREENROOM_E2E_JOURNEY").unwrap_or_default(),
        &std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE").unwrap_or_default(),
        &std::env::var("SCREENROOM_PLAYBACK_DIAGNOSTICS").unwrap_or_default(),
    )
}

fn idle_candidate_gate(journey: &str, phase: &str, diagnostics: &str) -> bool {
    idle_gate(journey, phase, diagnostics) || controls_gate(journey, phase, diagnostics)
}

fn idle_candidate_diagnostics() -> bool {
    idle_candidate_gate(
        &std::env::var("SCREENROOM_E2E_JOURNEY").unwrap_or_default(),
        &std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE").unwrap_or_default(),
        &std::env::var("SCREENROOM_PLAYBACK_DIAGNOSTICS").unwrap_or_default(),
    )
}

fn clear_controls_candidates(root: &std::path::Path, enabled: bool) -> Result {
    if !enabled {
        return Ok(());
    }
    for name in [
        "playback-idle-candidate.png",
        "playback-idle-candidate.json",
    ] {
        match fs::remove_file(root.join(name)) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn idle_hidden(tree: &str, title: &str) -> Result<Option<bool>> {
    modal_window(tree, title)?;
    for selector in [
        "//Button[@name='Pause']",
        "//Button[@name='Play']",
        "//Button[@name='Back to details']",
        "//Button[@name='Enter fullscreen']",
        "//Button[@name='Exit fullscreen']",
        "//Slider[@name='Playback position']",
        "//ToggleButton[@name='Audio, subtitles and playback settings']",
    ] {
        if waydriver::atspi::evaluate_xpath(tree, &format!("{LIVE_PLAYER}{selector}"))?.len() > 1 {
            return Err("Idle observation matched ambiguous chrome controls".into());
        }
    }
    let visible = waydriver::atspi::evaluate_xpath(
        tree,
        &format!(
            "{LIVE_PLAYER}//*[@showing='true' and (@role='button' or @role='toggle button' or @role='slider' or @role='label')]"
        ),
    )?;
    if !visible.is_empty() {
        return Ok(Some(false));
    }
    // GTK always emits Showing for returned nodes; missing nodes/states cannot prove hidden.
    Ok(None)
}

fn public_idle_window(
    tree: &str,
    title: &str,
    expected: Option<&(String, String)>,
) -> Result<Option<(String, String)>> {
    let window = modal_window(tree, title)?;
    if expected.is_some_and(|expected| expected != &window) {
        return Err("Idle playback modal identity changed".into());
    }
    let reference = format!("{}|{}", window.0, window.1);
    let line = tree
        .lines()
        .find(|line| snapshot_attribute(line, "_ref") == Some(reference.as_str()))
        .ok_or("Idle playback modal disappeared")?;
    if snapshot_attribute(line, "description") != Some("Playback controls hidden")
        || idle_hidden(tree, title)?.is_some()
        || !waydriver::atspi::evaluate_xpath(
            tree,
            &format!(
                "{LIVE_PLAYER}//*[@role='button' or @role='toggle button' or @role='slider' or @role='label']"
            ),
        )?
        .is_empty()
    {
        return Ok(None);
    }
    Ok(Some(window))
}

fn idle_candidate_observation(
    tree: &str,
    title: &str,
    expected: Option<&(String, String)>,
) -> Value {
    let mut data = json!({
        "modal_count":null, "selected_title_matches":null,
        "same_window":null, "eligible":null, "description":"unknown",
        "role_counts":null, "showing_role_counts":null, "classification":"harness-error",
        "reason":"classification-unavailable"
    });
    let Ok(windows) = waydriver::atspi::evaluate_xpath(tree, LIVE_PLAYER) else {
        return data;
    };
    data["modal_count"] = json!(windows.len());
    let [window] = windows.as_slice() else {
        data["reason"] = json!("modal-not-unique");
        return data;
    };
    let reference = format!("{}|{}", window.0, window.1);
    let Some(line) = tree
        .lines()
        .find(|line| snapshot_attribute(line, "_ref") == Some(reference.as_str()))
    else {
        data["reason"] = json!("modal-line-unavailable");
        return data;
    };
    let title_matches = snapshot_attribute(line, "name") == Some(title);
    let eligible = pointer_snapshot_bounds(line, &reference).is_some();
    let same = expected.map(|expected| expected == window);
    let description = match snapshot_attribute(line, "description") {
        Some("Playback controls hidden") => "hidden",
        Some("Playback controls visible") => "visible",
        Some("Playback controls changing") => "changing",
        None => "missing",
        _ => "other",
    };
    data["selected_title_matches"] = json!(title_matches);
    data["same_window"] = json!(same);
    data["eligible"] = json!(eligible);
    data["description"] = json!(description);
    let mut counts = Vec::new();
    let mut showing = Vec::new();
    for role in ["button", "toggle button", "slider", "label"] {
        let all =
            waydriver::atspi::evaluate_xpath(tree, &format!("{LIVE_PLAYER}//*[@role='{role}']"));
        let visible = waydriver::atspi::evaluate_xpath(
            tree,
            &format!("{LIVE_PLAYER}//*[@role='{role}' and @showing='true']"),
        );
        let (Ok(all), Ok(visible)) = (all, visible) else {
            data["reason"] = json!("classification-unavailable");
            return data;
        };
        counts.push(all.len());
        showing.push(visible.len());
    }
    data["role_counts"] = json!(counts);
    data["showing_role_counts"] = json!(showing);
    let result = public_idle_window(tree, title, expected);
    let (classification, reason) = match result {
        Ok(Some(_)) => ("positive", "hidden-description-and-no-chrome"),
        Ok(None) if description != "hidden" => ("not-positive", "description-not-hidden"),
        Ok(None) => ("not-positive", "chrome-returned"),
        Err(_) if !title_matches => ("harness-error", "selected-title-mismatch"),
        Err(_) if !eligible => ("harness-error", "modal-ineligible"),
        Err(_) if same == Some(false) => ("harness-error", "modal-identity-changed"),
        Err(_) => ("harness-error", "chrome-ambiguity-or-query-error"),
    };
    data["classification"] = json!(classification);
    data["reason"] = json!(reason);
    data
}

struct IdleCandidateDiagnostic {
    root: std::path::PathBuf,
    started: std::time::Instant,
    data: Value,
    complete_io: bool,
}

impl IdleCandidateDiagnostic {
    fn new(enabled: bool, root: &std::path::Path) -> Option<Self> {
        if !enabled {
            return None;
        }
        let mut diagnostic = Self {
            root: root.to_owned(),
            started: std::time::Instant::now(),
            data: json!({
                "schemaVersion":1, "check":"public-idle-candidate", "status":"pending",
                "role_order":["button","toggle button","slider","label"],
                "pre":{"state":"pending"}, "capture":{"state":"not-attempted"},
                "post":{"state":"not-attempted"}, "candidate_retained":false,
                "getter_before":null, "getter_after":null
            }),
            complete_io: true,
        };
        for name in [
            "playback-idle-candidate.png",
            "playback-idle-candidate.json",
        ] {
            if let Err(error) = fs::remove_file(root.join(name)) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    diagnostic.complete_io = false;
                }
            }
        }
        diagnostic.flush();
        Some(diagnostic)
    }

    fn flush(&mut self) {
        let saved = serde_json::to_vec_pretty(&self.data)
            .ok()
            .filter(|bytes| bytes.len() <= 8192)
            .is_some_and(|bytes| {
                fs::write(self.root.join("playback-idle-candidate.json"), bytes).is_ok()
            });
        self.complete_io &= saved;
    }

    fn begin(&mut self, phase: &str) {
        self.data[phase] = json!({
            "state":"pending", "start_ms":self.started.elapsed().as_millis(), "end_ms":null
        });
        self.flush();
    }

    fn observation(
        &mut self,
        phase: &str,
        tree: &str,
        title: &str,
        expected: Option<&(String, String)>,
    ) {
        self.data[phase]["state"] = json!("observed");
        self.data[phase]["end_ms"] = json!(self.started.elapsed().as_millis());
        self.data[phase]["proof"] = idle_candidate_observation(tree, title, expected);
        let getter = if phase == "pre" {
            "getter_before"
        } else {
            "getter_after"
        };
        self.data[getter] = fresh_chrome_receipt()
            .ok()
            .map(|receipt| receipt.value())
            .unwrap_or(Value::Null);
        self.flush();
    }

    fn candidate(&mut self, png: &[u8]) {
        self.data["capture"]["state"] = json!("acquired");
        self.data["capture"]["end_ms"] = json!(self.started.elapsed().as_millis());
        let saved = fs::write(self.root.join("playback-idle-candidate.png"), png).is_ok();
        self.data["candidate_retained"] = json!(saved);
        self.complete_io &= saved;
        self.flush();
    }

    fn finish<T>(&mut self, result: &Result<T>) -> Result {
        self.data["status"] = json!(if result.is_err() {
            "failed"
        } else if self.complete_io {
            "passed"
        } else {
            "incomplete"
        });
        self.data["finished_ms"] = json!(self.started.elapsed().as_millis());
        self.flush();
        if self.complete_io {
            Ok(())
        } else {
            Err("Public idle candidate diagnostic retention is incomplete".into())
        }
    }
}

async fn public_idle_frame(session: &Arc<Session>) -> Result<Vec<u8>> {
    let title = std::env::var("SCREENROOM_SHOWCASE_TITLE")?;
    let mut diagnostic = IdleCandidateDiagnostic::new(
        idle_candidate_diagnostics(),
        std::path::Path::new("/artifacts"),
    );
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut window = None;
        loop {
            if let Some(diagnostic) = diagnostic.as_mut() {
                diagnostic.begin("pre");
            }
            let tree = session.dump_tree().await?;
            if let Some(diagnostic) = diagnostic.as_mut() {
                diagnostic.observation("pre", &tree, &title, window.as_ref());
            }
            if window.is_none() {
                window = Some(modal_window(&tree, &title)?);
            }
            if let Some(hidden) = public_idle_window(&tree, &title, window.as_ref())? {
                if let Some(diagnostic) = diagnostic.as_mut() {
                    diagnostic.begin("capture");
                }
                let png = frame(session).await?;
                if let Some(diagnostic) = diagnostic.as_mut() {
                    diagnostic.candidate(&png);
                    diagnostic.begin("post");
                }
                let after = session.dump_tree().await?;
                if let Some(diagnostic) = diagnostic.as_mut() {
                    diagnostic.observation("post", &after, &title, Some(&hidden));
                }
                if public_idle_window(&after, &title, Some(&hidden))?.is_none() {
                    return Err(
                        "Positive public hidden-chrome proof was lost after the image".into(),
                    );
                }
                return Ok::<_, Box<dyn Error>>(png);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| -> Box<dyn Error> {
        "No positive public hidden-chrome proof within 10 seconds".into()
    })
    .and_then(|result| result);
    let retention = diagnostic
        .as_mut()
        .map(|diagnostic| diagnostic.finish(&result))
        .unwrap_or(Ok(()));
    let png = result?;
    retention?;
    Ok(png)
}

async fn idle_capture_probe(session: &Arc<Session>) -> Result {
    if !idle_diagnostics() {
        return Err("Public idle candidate probe requires its explicit diagnostic gate".into());
    }
    moving_media(session).await?;
    reveal(session).await?;
    frame(session).await?;
    let png = public_idle_frame(session).await?;
    fs::write("/artifacts/playback-idle.png", png)?;
    fs::write(
        "/artifacts/playback-idle.json",
        serde_json::to_vec_pretty(&json!({
            "schemaVersion":1, "check":"idle", "status":"observed-hidden",
            "hidden_observed":true, "hidden_after_image":true, "image_retained":true,
            "visual_review":"required",
            "positive_hidden_proof":"same-active-playback-window-description",
            "candidate_evidence":"playback-idle-candidate.json"
        }))?,
    )?;
    Ok(())
}

#[tokio::test]
async fn idle_candidate_controls_contract() -> Result {
    for journey in ["", "browse", "playback", "unknown"] {
        for phase in ["", "capture", "idle", "controls", "navigation", "readiness"] {
            for flag in ["", "0", "1", "true"] {
                assert_eq!(
                    idle_candidate_gate(journey, phase, flag),
                    journey == "playback" && matches!(phase, "idle" | "controls") && flag == "1"
                );
            }
        }
    }
    let directory =
        std::env::temp_dir().join(format!("candidate-controls-{}", uuid::Uuid::new_v4()));
    clear_controls_candidates(&directory, false)?;
    assert!(!directory.exists());
    fs::create_dir(&directory)?;
    fs::write(directory.join("unrelated"), b"preserve")?;
    for name in [
        "playback-idle-candidate.png",
        "playback-idle-candidate.json",
    ] {
        fs::write(directory.join(name), b"stale")?;
    }
    clear_controls_candidates(&directory, false)?;
    assert!(directory.join("playback-idle-candidate.png").exists());
    clear_controls_candidates(&directory, true)?;
    assert!(!directory.join("playback-idle-candidate.png").exists());
    assert!(!directory.join("playback-idle-candidate.json").exists());
    assert_eq!(fs::read(directory.join("unrelated"))?, b"preserve");

    let hidden = r#"<Window role="window" name="Nosferatu" description="Playback controls hidden" showing="true" visible="true" sensitive="true" active="true" modal="true" bbox="0,0,1100,720" _ref=":1.77|/fixture/window"/>"#;
    let window = modal_window(hidden, "Nosferatu")?;
    let image = image::RgbImage::from_fn(1280, 800, |x, y| {
        image::Rgb([x as u8, y as u8, (x + y) as u8])
    });
    validate_frame(&image)?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image).write_to(&mut bytes, image::ImageFormat::Png)?;
    let png = bytes.into_inner();
    let mut diagnostic =
        IdleCandidateDiagnostic::new(idle_candidate_gate("playback", "controls", "1"), &directory)
            .unwrap();
    diagnostic.begin("pre");
    diagnostic.observation("pre", hidden, "Nosferatu", Some(&window));
    diagnostic.begin("capture");
    diagnostic.candidate(&png);
    diagnostic.begin("post");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), std::future::pending::<()>())
            .await
            .is_err()
    );
    let partial: Value =
        serde_json::from_slice(&fs::read(directory.join("playback-idle-candidate.json"))?)?;
    assert_eq!(partial["post"]["state"], "pending");
    assert_eq!(partial["post"]["end_ms"], Value::Null);
    assert_eq!(partial["candidate_retained"], true);
    assert_eq!(
        fs::read(directory.join("playback-idle-candidate.png"))?,
        png
    );

    let changing = hidden.replace("Playback controls hidden", "Playback controls changing");
    diagnostic.observation("post", &changing, "Nosferatu", Some(&window));
    let original: Result<()> =
        Err("Positive public hidden-chrome proof was lost after the image".into());
    diagnostic.finish(&original)?;
    let failed: Value =
        serde_json::from_slice(&fs::read(directory.join("playback-idle-candidate.json"))?)?;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["pre"]["proof"]["classification"], "positive");
    assert_eq!(failed["post"]["proof"]["classification"], "not-positive");
    assert_eq!(failed["post"]["proof"]["description"], "changing");
    assert_eq!(failed["candidate_retained"], true);
    assert_eq!(
        original.unwrap_err().to_string(),
        "Positive public hidden-chrome proof was lost after the image"
    );
    assert!(!directory.join("manifest.json").exists());
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn idle_candidate_contract() -> Result {
    let directory = std::env::temp_dir().join(format!(
        "screenroom-idle-candidate-{}",
        uuid::Uuid::new_v4()
    ));
    assert!(IdleCandidateDiagnostic::new(false, &directory).is_none());
    assert!(!directory.exists());
    fs::create_dir(&directory)?;
    // Exact later failure modal; only opaque references normalized.
    let hidden = r#"<Window role="window" name="Nosferatu" description="Playback controls hidden" showing="true" visible="true" sensitive="true" active="true" modal="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node0">
  <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node1">
    <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node2">
      <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node3">
        <Generic role="generic" showing="true" visible="true" sensitive="true" focused="true" focusable="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node4">
          <Image role="image" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node5"/>
          <Group role="group" showing="true" visible="true" bbox="0,0,1100,68" toolkit="GTK" _ref=":1.77|/fixture/node6"/>
          <Group role="group" showing="true" visible="true" bbox="0,530,1100,190" toolkit="GTK" _ref=":1.77|/fixture/node7"/>
        </Generic>
      </Generic>
    </Generic>
  </Generic>
</Window>"#;
    let window = public_idle_window(hidden, "Nosferatu", None)?.unwrap();
    let proof = idle_candidate_observation(hidden, "Nosferatu", Some(&window));
    assert_eq!(proof["classification"], "positive");
    assert_eq!(proof["role_counts"], json!([0, 0, 0, 0]));
    assert_eq!(proof["same_window"], true);
    let invalid = idle_candidate_observation("<", "Nosferatu", Some(&window));
    assert_eq!(invalid["modal_count"], Value::Null);
    assert_eq!(invalid["eligible"], Value::Null);
    assert_eq!(invalid["reason"], "classification-unavailable");
    for description in ["visible", "changing", "signed-private-token"] {
        let replacement = if description == "signed-private-token" {
            description.to_owned()
        } else {
            format!("Playback controls {description}")
        };
        let tree = hidden.replace("Playback controls hidden", &replacement);
        let proof = idle_candidate_observation(&tree, "Nosferatu", Some(&window));
        assert_eq!(proof["reason"], "description-not-hidden");
        assert!(!proof.to_string().contains("signed-private-token"));
    }
    let absent = hidden.replace(" description=\"Playback controls hidden\"", "");
    assert_eq!(
        idle_candidate_observation(&absent, "Nosferatu", Some(&window))["description"],
        "missing"
    );
    for states in [" showing=\"true\"", ""] {
        let tree = hidden.replace(
            "</Window>",
            &format!(
                "<Button role=\"button\" name=\"private-label\"{states} _ref=\":1.77|/private-ref\"/></Window>"
            ),
        );
        let proof = idle_candidate_observation(&tree, "Nosferatu", Some(&window));
        assert_eq!(proof["reason"], "chrome-returned");
        assert_eq!(proof["role_counts"], json!([1, 0, 0, 0]));
        assert!(!proof.to_string().contains("private"));
    }
    assert_eq!(
        idle_candidate_observation(hidden, "Nosferatu", Some(&("other".into(), "ref".into())))["reason"],
        "modal-identity-changed"
    );
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(1280, 800, |x, y| {
        image::Rgb([x as u8, y as u8, (x + y) as u8])
    }))
    .write_to(&mut bytes, image::ImageFormat::Png)?;
    let png = bytes.into_inner();
    let mut diagnostic = IdleCandidateDiagnostic::new(true, &directory).unwrap();
    diagnostic.begin("pre");
    diagnostic.observation("pre", hidden, "Nosferatu", Some(&window));
    diagnostic.begin("capture");
    diagnostic.candidate(&png);
    diagnostic.begin("post");
    let pending = tokio::time::timeout(Duration::from_millis(10), async {
        std::future::pending::<()>().await;
    })
    .await;
    assert!(pending.is_err());
    let partial: Value =
        serde_json::from_slice(&fs::read(directory.join("playback-idle-candidate.json"))?)?;
    assert_eq!(partial["post"]["state"], "pending");
    assert_eq!(partial["post"]["end_ms"], Value::Null);
    assert_eq!(partial["candidate_retained"], true);
    assert_eq!(
        fs::read(directory.join("playback-idle-candidate.png"))?,
        png
    );
    let failure: Result<Vec<u8>> =
        Err("Positive public hidden-chrome proof was lost after the image".into());
    diagnostic.finish(&failure)?;
    assert_eq!(diagnostic.data["status"], "failed");
    assert_eq!(
        failure.unwrap_err().to_string(),
        "Positive public hidden-chrome proof was lost after the image"
    );
    diagnostic.observation("post", hidden, "Nosferatu", Some(&window));
    diagnostic.finish(&Ok(()))?;
    for phase in ["pre", "capture", "post"] {
        let phase = &diagnostic.data[phase];
        assert!(phase["start_ms"].as_u64().unwrap() <= phase["end_ms"].as_u64().unwrap());
    }
    let encoded = serde_json::to_string(&diagnostic.data)?;
    assert!(encoded.len() < 8192);
    for forbidden in [
        "Nosferatu",
        ":1.77",
        "/fixture",
        "private",
        "http",
        "name",
        "bbox",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "Forbidden field or value: {forbidden}"
        );
    }
    let keys: std::collections::BTreeSet<_> = diagnostic
        .data
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "candidate_retained",
            "capture",
            "check",
            "finished_ms",
            "getter_after",
            "getter_before",
            "post",
            "pre",
            "role_order",
            "schemaVersion",
            "status"
        ]
        .into_iter()
        .collect()
    );
    assert!(!directory.join("manifest.json").exists());
    let missing = directory.join("absent");
    let mut broken = IdleCandidateDiagnostic::new(true, &missing).unwrap();
    assert!(broken.finish(&Ok(())).is_err());
    assert_eq!(broken.data["status"], "incomplete");
    let original: Result<()> = Err("original capture failure".into());
    assert!(broken.finish(&original).is_err());
    assert_eq!(
        original.unwrap_err().to_string(),
        "original capture failure"
    );
    let mut next = IdleCandidateDiagnostic::new(true, &directory).unwrap();
    assert!(!directory.join("playback-idle-candidate.png").exists());
    next.begin("capture");
    next.finish(&Err::<(), _>("original capture failure".into()))?;
    assert_eq!(next.data["capture"]["state"], "pending");
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn public_idle_contract() -> Result {
    // Actual successful probe subtree; opaque references normalized, description added.
    let hidden = r#"<Window role="window" name="Nosferatu" description="Playback controls hidden" showing="true" visible="true" sensitive="true" active="true" modal="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node0">
  <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node1">
    <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node2">
      <Generic role="generic" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node3">
        <Generic role="generic" showing="true" visible="true" sensitive="true" focused="true" focusable="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node4">
          <Image role="image" showing="true" visible="true" sensitive="true" bbox="0,0,1100,720" toolkit="GTK" _ref=":1.77|/fixture/node5"/>
          <Group role="group" showing="true" visible="true" bbox="0,0,1100,68" toolkit="GTK" _ref=":1.77|/fixture/node6"/>
          <Group role="group" showing="true" visible="true" bbox="0,530,1100,190" toolkit="GTK" _ref=":1.77|/fixture/node7"/>
        </Generic>
      </Generic>
    </Generic>
  </Generic>
</Window>"#;
    let window = public_idle_window(hidden, "Nosferatu", None)?
        .ok_or("Fixture must positively identify hidden chrome")?;
    assert_eq!(
        public_idle_window(hidden, "Nosferatu", Some(&window))?,
        Some(window.clone())
    );
    for description in [
        "Playback controls visible",
        "Playback controls changing",
        "",
        "hidden",
    ] {
        assert!(
            public_idle_window(
                &hidden.replace("Playback controls hidden", description),
                "Nosferatu",
                Some(&window)
            )?
            .is_none()
        );
    }
    assert!(
        public_idle_window(
            &hidden.replace(" description=\"Playback controls hidden\"", ""),
            "Nosferatu",
            None
        )?
        .is_none()
    );
    let wrong_object = hidden
        .replace(" description=\"Playback controls hidden\"", "")
        .replacen(
            "<Generic role=\"generic\"",
            "<Generic role=\"generic\" description=\"Playback controls hidden\"",
            1,
        );
    assert!(public_idle_window(&wrong_object, "Nosferatu", None)?.is_none());
    for control in [
        "<Button role=\"button\" name=\"Play\" showing=\"true\" visible=\"true\" _ref=\":1.77|/fixture/play\"/>",
        "<Button role=\"button\" name=\"Pause\" showing=\"true\" visible=\"true\" _ref=\":1.77|/fixture/pause\"/>",
        "<Label role=\"label\" name=\"Playback failed\" showing=\"true\" visible=\"true\" _ref=\":1.77|/fixture/error\"/>",
        "<Slider role=\"slider\" name=\"Playback position\" showing=\"true\" visible=\"true\" _ref=\":1.77|/fixture/slider\"/>",
        "<Button role=\"button\" name=\"Play\" _ref=\":1.77|/fixture/unknown-state\"/>",
        "<Label role=\"label\" name=\"Playback failed\" showing=\"false\" _ref=\":1.77|/fixture/unknown-error\"/>",
    ] {
        assert!(
            public_idle_window(
                &hidden.replace("</Window>", &format!("{control}</Window>")),
                "Nosferatu",
                None
            )?
            .is_none()
        );
    }
    for invalid in [
        hidden.replace("name=\"Nosferatu\"", "name=\"Other\""),
        hidden.replace("active=\"true\"", "active=\"false\""),
        hidden.replace("modal=\"true\"", "modal=\"false\""),
        hidden.replace("sensitive=\"true\"", "sensitive=\"false\""),
        format!("<Application>{hidden}{hidden}</Application>"),
        "<Application/>".to_owned(),
    ] {
        assert!(public_idle_window(&invalid, "Nosferatu", Some(&window)).is_err());
    }
    let replaced = hidden.replace("/fixture/node0", "/fixture/replaced");
    assert!(public_idle_window(&replaced, "Nosferatu", Some(&window)).is_err());
    let ambiguous = "<Button role=\"button\" name=\"Pause\" _ref=\":1.77|/fixture/a\"/><Button role=\"button\" name=\"Pause\" _ref=\":1.77|/fixture/b\"/>";
    assert!(
        public_idle_window(
            &hidden.replace("</Window>", &format!("{ambiguous}</Window>")),
            "Nosferatu",
            None
        )
        .is_err()
    );
    Ok(())
}

#[derive(Clone)]
struct ChromeReceipt {
    sequence: u32,
    elapsed_ms: u64,
    requested_show: bool,
    header_reveal_child: bool,
    header_child_revealed: bool,
    footer_reveal_child: bool,
    footer_child_revealed: bool,
}

impl ChromeReceipt {
    fn value(&self) -> Value {
        json!({
            "sequence":self.sequence, "elapsed_ms":self.elapsed_ms,
            "requested_show":self.requested_show,
            "header_reveal_child":self.header_reveal_child,
            "header_child_revealed":self.header_child_revealed,
            "footer_reveal_child":self.footer_reveal_child,
            "footer_child_revealed":self.footer_child_revealed
        })
    }

    fn hidden(&self) -> bool {
        !self.requested_show
            && !self.header_reveal_child
            && !self.header_child_revealed
            && !self.footer_reveal_child
            && !self.footer_child_revealed
    }
}

fn chrome_receipts(bytes: &[u8]) -> Result<ChromeReceipt> {
    if bytes.len() > 65536 {
        return Err("Idle chrome receipts exceed their bounded size".into());
    }
    let text = std::str::from_utf8(bytes)?;
    let mut last: Option<ChromeReceipt> = None;
    for (index, line) in text.lines().enumerate() {
        let value: Value = serde_json::from_str(line)?;
        if value.as_object().is_none_or(|object| object.len() != 7) {
            return Err("Idle chrome receipt fields are invalid".into());
        }
        let boolean = |name: &str| -> Result<bool> {
            value
                .get(name)
                .and_then(Value::as_bool)
                .ok_or_else(|| "Idle chrome receipt boolean is missing or invalid".into())
        };
        let sequence = value
            .get("sequence")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or("Idle chrome receipt sequence is invalid")?;
        let elapsed_ms = value
            .get("elapsed_ms")
            .and_then(Value::as_u64)
            .ok_or("Idle chrome receipt elapsed time is invalid")?;
        let receipt = ChromeReceipt {
            sequence,
            elapsed_ms,
            requested_show: boolean("requested_show")?,
            header_reveal_child: boolean("header_reveal_child")?,
            header_child_revealed: boolean("header_child_revealed")?,
            footer_reveal_child: boolean("footer_reveal_child")?,
            footer_child_revealed: boolean("footer_child_revealed")?,
        };
        if index >= 128
            || receipt.sequence as usize != index + 1
            || receipt.elapsed_ms > 25600
            || last
                .as_ref()
                .is_some_and(|previous| receipt.elapsed_ms < previous.elapsed_ms + 200)
        {
            return Err("Idle chrome receipt ordering or bounds are invalid".into());
        }
        last = Some(receipt);
    }
    last.ok_or_else(|| "Idle chrome getter proof is not observed".into())
}

fn fresh_chrome_receipt() -> Result<ChromeReceipt> {
    use std::io::Read;
    let file = fs::File::open("/artifacts/playback-idle-chrome.jsonl")?;
    if file.metadata()?.modified()?.elapsed()? > Duration::from_millis(500) {
        return Err("Idle chrome getter receipt is not fresh".into());
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    chrome_receipts(&bytes)
}

fn idle_image_proof(
    before: &ChromeReceipt,
    after: &ChromeReceipt,
    baseline: f64,
    first: f64,
    last: f64,
) -> bool {
    before.hidden()
        && after.hidden()
        && after.sequence > before.sequence
        && after.elapsed_ms > before.elapsed_ms
        && baseline.is_finite()
        && baseline > 0.0
        && first.is_finite()
        && last.is_finite()
        && position_advanced(Some(baseline), first)
        && last >= first
}

async fn idle_probe(session: &Arc<Session>) -> Result {
    let title = std::env::var("SCREENROOM_SHOWCASE_TITLE")?;
    reveal(session).await?;
    let initial = session.dump_tree().await?;
    let window = modal_window(&initial, &title)?;
    let eligible = live_readiness(&initial)?;
    let reference = eligible
        .slider
        .filter(|_| eligible.enabled)
        .ok_or("Idle observation requires the eligible advancing playback slider")?;
    let value = session.value_ref(&reference.0, &reference.1).await?;
    let baseline = live_position(value.current, value.minimum, value.maximum)?;
    if baseline <= 0.0 {
        return Err("Idle observation requires a positive playback baseline".into());
    }
    let started = std::time::Instant::now();
    let observation = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let tree = session.dump_tree().await?;
            if modal_window(&tree, &title)? != window {
                return Err("Idle playback modal identity changed".into());
            }
            if public_idle_window(&tree, &title, Some(&window))?.is_some() {
                if let Ok(receipt) = fresh_chrome_receipt() {
                    if !receipt.hidden() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    let value = session.value_ref(&reference.0, &reference.1).await?;
                    let position = live_position(value.current, value.minimum, value.maximum)?;
                    if position_advanced(Some(baseline), position) {
                        return Ok::<_, Box<dyn Error>>((receipt, position));
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| -> Box<dyn Error> { "No positive hidden-chrome proof within 10 seconds".into() })
    .and_then(|result| result);
    let png = tokio::time::timeout(Duration::from_secs(3), session.take_screenshot()).await;
    let retained = match png {
        Ok(Ok(bytes)) => fs::write("/artifacts/playback-idle.png", bytes).is_ok(),
        _ => false,
    };
    let after_floor = fresh_chrome_receipt()
        .map(|receipt| receipt.sequence)
        .unwrap_or(128);
    let after = tokio::time::timeout(Duration::from_millis(750), async {
        let tree = session.dump_tree().await?;
        let _ = fs::write("/artifacts/playback-idle-tree.xml", &tree);
        loop {
            if let Ok(receipt) = fresh_chrome_receipt() {
                if receipt.sequence > after_floor && receipt.hidden() {
                    let tree = session.dump_tree().await?;
                    let _ = fs::write("/artifacts/playback-idle-tree.xml", &tree);
                    if modal_window(&tree, &title)? != window {
                        return Err("Idle playback modal identity changed after the image".into());
                    }
                    if public_idle_window(&tree, &title, Some(&window))?.is_none() {
                        return Err::<(ChromeReceipt, f64), Box<dyn Error>>(
                            "Chrome appeared after the idle image".into(),
                        );
                    }
                    let value = session.value_ref(&reference.0, &reference.1).await?;
                    let position = live_position(value.current, value.minimum, value.maximum)?;
                    if observation
                        .as_ref()
                        .is_ok_and(|(_, before)| position >= *before)
                        && position_advanced(Some(baseline), position)
                    {
                        return Ok((receipt, position));
                    }
                    return Err("Idle image playback progress was not confirmed".into());
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .ok()
    .and_then(std::result::Result::ok);
    let positive = observation.as_ref().ok().zip(after.as_ref()).is_some_and(
        |((before, first), (after, last))| idle_image_proof(before, after, baseline, *first, *last),
    );
    let hidden_after_image = after.as_ref().map(|_| positive);
    let data = json!({
        "schemaVersion":1, "check":"idle", "observation_elapsed_ms":started.elapsed().as_millis(),
        "observation_bound_ms":10000, "hidden_observed":observation.is_ok(),
        "hidden_after_image":hidden_after_image, "image_retained":retained,
        "ax_hidden_proof":"same-active-playback-window-description",
        "positive_hidden_proof":"actual-gtk-revealer-getters-with-public-window-description",
        "chrome_before_image":observation.as_ref().ok().map(|(receipt, _)| receipt.value()),
        "chrome_after_image":after.as_ref().map(|(receipt, _)| receipt.value()),
        "baseline_position_ms":baseline,
        "before_image_position_ms":observation.as_ref().ok().map(|(_, position)| position),
        "after_image_position_ms":after.as_ref().map(|(_, position)| position),
        "visual_review":"required", "status":if positive && retained { "observed-hidden" } else { "failed" }
    });
    let saved = fs::write(
        "/artifacts/playback-idle.json",
        serde_json::to_vec_pretty(&data)?,
    )
    .is_ok();
    observation?;
    if hidden_after_image != Some(true) || !retained || !saved {
        return Err("Idle diagnostic evidence is incomplete or controls became visible".into());
    }
    Ok(())
}

#[test]
fn chrome_receipt_contract() -> Result {
    let base = json!({
        "sequence":1, "elapsed_ms":200, "requested_show":false,
        "header_reveal_child":false, "header_child_revealed":false,
        "footer_reveal_child":false, "footer_child_revealed":false
    });
    assert!(chrome_receipts(base.to_string().as_bytes())?.hidden());
    for field in [
        "requested_show",
        "header_reveal_child",
        "header_child_revealed",
        "footer_reveal_child",
        "footer_child_revealed",
    ] {
        let mut active = base.clone();
        active[field] = json!(true);
        assert!(!chrome_receipts(active.to_string().as_bytes())?.hidden());
        active[field] = Value::Null;
        assert!(chrome_receipts(active.to_string().as_bytes()).is_err());
    }
    let mut extra = base.clone();
    extra["private"] = json!("not allowed");
    assert!(chrome_receipts(extra.to_string().as_bytes()).is_err());
    assert!(chrome_receipts(b"").is_err());
    assert!(chrome_receipts(&vec![b' '; 65537]).is_err());
    let mut second = base.clone();
    second["sequence"] = json!(2);
    second["elapsed_ms"] = json!(400);
    assert_eq!(
        chrome_receipts(format!("{base}\n{second}\n").as_bytes())?.sequence,
        2
    );
    for elapsed in [199, 399, 25601] {
        second["elapsed_ms"] = json!(elapsed);
        assert!(chrome_receipts(format!("{base}\n{second}\n").as_bytes()).is_err());
    }
    assert!(chrome_receipts(format!("{base}\n{base}\n").as_bytes()).is_err());
    assert!(chrome_receipts(second.to_string().as_bytes()).is_err());
    let before = chrome_receipts(base.to_string().as_bytes())?;
    let mut after = before.clone();
    after.sequence = 2;
    after.elapsed_ms = 400;
    assert!(idle_image_proof(&before, &after, 1000.0, 1300.0, 1300.0));
    assert!(!idle_image_proof(&before, &before, 1000.0, 1300.0, 1400.0));
    for value in [0.0, f64::NAN, f64::INFINITY] {
        assert!(!idle_image_proof(&before, &after, value, 1300.0, 1400.0));
    }
    for (first, last) in [
        (1250.0, 1500.0),
        (1300.0, 1299.0),
        (f64::NAN, 1500.0),
        (1300.0, f64::INFINITY),
    ] {
        assert!(!idle_image_proof(&before, &after, 1000.0, first, last));
    }
    after.header_child_revealed = true;
    assert!(!idle_image_proof(&before, &after, 1000.0, 1300.0, 1400.0));
    Ok(())
}

#[test]
fn idle_observation_contract() -> Result {
    for journey in ["", "browse", "playback"] {
        for phase in ["", "capture", "readiness", "motion", "idle"] {
            for flag in ["", "0", "1", "true"] {
                assert_eq!(
                    idle_gate(journey, phase, flag),
                    journey == "playback" && phase == "idle" && flag == "1"
                );
            }
        }
    }
    assert_eq!(idle_hidden(MODAL, "Nosferatu")?, Some(false));
    assert_eq!(
        idle_hidden(
            &MODAL.replace("name=\"Pause\"", "name=\"Play\""),
            "Nosferatu"
        )?,
        Some(false)
    );
    assert_eq!(
        idle_hidden(
            &MODAL.replace("name=\"Pause\"", "name=\"Playback failed\""),
            "Nosferatu"
        )?,
        Some(false)
    );
    let absent_pause = MODAL
        .lines()
        .filter(|line| !line.contains("name=\"Pause\""))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(idle_hidden(&absent_pause, "Nosferatu")?, Some(false));
    assert_eq!(
        idle_hidden(
            &MODAL.replace(
                "name=\"Pause\" showing=\"true\" visible=\"true\" sensitive=\"true\"",
                "name=\"Pause\" showing=\"true\" visible=\"true\" sensitive=\"false\""
            ),
            "Nosferatu"
        )?,
        Some(false)
    );
    let unknown = MODAL
        .lines()
        .filter(|line| {
            !line.contains("role=\"button\"")
                && !line.contains("role=\"toggle button\"")
                && !line.contains("role=\"slider\"")
                && !line.contains("role=\"label\"")
        })
        .map(|line| {
            if line.trim() == "</Button>" || line.trim() == "</ToggleButton>" {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(idle_hidden(&unknown, "Nosferatu")?, None);
    let media_error = unknown.replace("</Window>", &format!(
        "<Label role=\"label\" name=\"{}\" showing=\"true\" visible=\"true\" _ref=\":1.77|/fixture/error\"/></Window>",
        MEDIA_FAILURES[0]
    ));
    assert_eq!(idle_hidden(&media_error, "Nosferatu")?, Some(false));
    let unknown_states = MODAL
        .lines()
        .map(|line| {
            if line.contains("<Window ") {
                line.to_owned()
            } else {
                line.replace(" showing=\"true\"", "")
                    .replace(" visible=\"true\"", "")
                    .replace(" sensitive=\"true\"", "")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(idle_hidden(&unknown_states, "Nosferatu")?, None);
    assert_ne!(
        idle_hidden(
            &unknown_states.replace("role=\"button\"", "role=\"button\" showing=\"false\""),
            "Nosferatu"
        )?,
        Some(true)
    );
    let duplicate = MODAL.replace("name=\"Back 30 seconds\"", "name=\"Pause\"");
    assert!(idle_hidden(&duplicate, "Nosferatu").is_err());
    assert!(
        idle_hidden(
            &format!("<Application>{MODAL}{MODAL}</Application>"),
            "Nosferatu"
        )
        .is_err()
    );
    assert!(idle_hidden(MODAL, "Wrong title").is_err());
    assert!(idle_hidden("<Application/>", "Nosferatu").is_err());
    Ok(())
}

fn controls_gate(journey: &str, phase: &str, diagnostics: &str) -> bool {
    journey == "playback" && phase == "controls" && diagnostics == "1"
}

fn controls_diagnostics() -> bool {
    controls_gate(
        &std::env::var("SCREENROOM_E2E_JOURNEY").unwrap_or_default(),
        &std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE").unwrap_or_default(),
        &std::env::var("SCREENROOM_PLAYBACK_DIAGNOSTICS").unwrap_or_default(),
    )
}

const CONTROL_STEPS: &[&str] = &[
    "motion",
    "playing-reveal",
    "playing-image",
    "idle-image",
    "pause-reveal",
    "pause-key",
    "play-visible",
    "play-enabled",
    "pause-stability",
    "paused-image",
    "settings-open",
    "volume-visible",
    "settings-image",
    "volume-home",
    "volume-end",
    "audio-visible",
    "subtitles-visible",
    "tracks-image",
    "track-choices",
    "settings-close",
    "prefix",
    "cleanup",
];

struct ControlTrace {
    file: Option<fs::File>,
    started: std::time::Instant,
    records: u32,
    bytes: usize,
    attempts: u32,
    step: &'static str,
    failed: bool,
}

impl ControlTrace {
    fn new(enabled: bool, path: &std::path::Path) -> Option<Self> {
        if !enabled {
            return None;
        }
        let file = fs::File::create(path).ok();
        Some(Self {
            failed: file.is_none(),
            file,
            started: std::time::Instant::now(),
            records: 0,
            bytes: 0,
            attempts: 0,
            step: "prefix",
        })
    }

    fn record(&mut self, operation: &str, event: &str, key: &str, flags: Option<(bool, bool)>) {
        use std::io::Write;
        if !CONTROL_STEPS.contains(&self.step)
            || ![
                "step",
                "focus",
                "tab",
                "terminal",
                "menu-confirmation",
                "value-bound",
                "track-settlement",
            ]
            .contains(&operation)
            || !["begin", "end", "failed", "observed"].contains(&event)
            || !["none", "space", "home", "end", "escape"].contains(&key)
        {
            self.failed = true;
            return;
        }
        if operation == "terminal" && event == "begin" {
            self.attempts += 1;
        }
        if self.records == 256 {
            self.failed = true;
            return;
        }
        let value = json!({
            "sequence":self.records + 1, "elapsed_ms":self.started.elapsed().as_millis(),
            "step":self.step, "operation":operation, "event":event, "key":key,
            "eligible":flags.map(|v| v.0), "focused":flags.map(|v| v.1),
            "terminal_attempts":self.attempts
        });
        let Some(mut bytes) = serde_json::to_vec(&value).ok() else {
            self.failed = true;
            return;
        };
        bytes.push(b'\n');
        if self.bytes + bytes.len() > 65536 {
            self.failed = true;
            return;
        }
        self.records += 1;
        self.bytes += bytes.len();
        if self
            .file
            .as_mut()
            .is_none_or(|file| file.write_all(&bytes).and_then(|_| file.flush()).is_err())
        {
            self.failed = true;
        }
    }
}

fn control_trace() -> Option<&'static std::sync::Mutex<ControlTrace>> {
    static TRACE: std::sync::OnceLock<Option<std::sync::Mutex<ControlTrace>>> =
        std::sync::OnceLock::new();
    TRACE
        .get_or_init(|| {
            ControlTrace::new(
                controls_diagnostics(),
                std::path::Path::new("/artifacts/playback-controls.jsonl"),
            )
            .map(std::sync::Mutex::new)
        })
        .as_ref()
}

fn control_record(operation: &str, event: &str, key: &str, flags: Option<(bool, bool)>) {
    if let Some(trace) = control_trace() {
        if let Ok(mut trace) = trace.lock() {
            trace.record(operation, event, key, flags);
        }
    }
}

async fn control_await<T, E, F>(mut record: impl FnMut(&str), future: F) -> Result<T>
where
    E: Into<Box<dyn Error>>,
    F: std::future::Future<Output = std::result::Result<T, E>>,
{
    record("begin");
    let result = future.await.map_err(Into::into);
    record(if result.is_ok() { "end" } else { "failed" });
    result
}

fn control_operation<T, E, F>(
    operation: &str,
    key: &str,
    future: F,
) -> impl std::future::Future<Output = Result<T>>
where
    E: Into<Box<dyn Error>>,
    F: std::future::Future<Output = std::result::Result<T, E>>,
{
    let future = Box::pin(future);
    async move { control_await(|event| control_record(operation, event, key, None), future).await }
}

fn control_step<T, E, F>(
    step: &'static str,
    future: F,
) -> impl std::future::Future<Output = Result<T>>
where
    E: Into<Box<dyn Error>>,
    F: std::future::Future<Output = std::result::Result<T, E>>,
{
    let future = Box::pin(future);
    async move {
        let previous = control_trace().and_then(|trace| {
            trace.lock().ok().map(|mut trace| {
                let previous = trace.step;
                trace.step = step;
                previous
            })
        });
        let result = control_operation("step", "none", future).await;
        if let (Some(previous), Some(trace)) = (previous, control_trace()) {
            if let Ok(mut trace) = trace.lock() {
                trace.step = previous;
            }
        }
        result
    }
}

fn controls_retained() -> Result {
    if let Some(trace) = control_trace() {
        if trace.lock().map_or(true, |trace| trace.failed) {
            return Err("Control prefix provenance is incomplete".into());
        }
    }
    Ok(())
}

#[tokio::test]
async fn controls_future_layout_contract() -> Result {
    async fn prefix_without_session(
        session: Option<&Arc<Session>>,
        evidence: &mut Evidence,
    ) -> Result {
        if let Some(session) = session {
            control_prefix(session, evidence).await?;
        }
        Ok(())
    }

    let mut evidence = Evidence::new();
    let future = prefix_without_session(None, &mut evidence);
    let prefix_bytes = std::mem::size_of_val(&future);
    assert!(prefix_bytes < 16_384, "prefix future: {prefix_bytes}");
    let future = control_step("prefix", future);
    let step_bytes = std::mem::size_of_val(&future);
    assert!(step_bytes < 512, "step future: {step_bytes}");
    eprintln!("controls future bytes: prefix={prefix_bytes}, step={step_bytes}");
    future.await?;

    let calls = std::cell::Cell::new(0);
    let error = control_step("pause", async {
        calls.set(calls.get() + 1);
        Err::<(), Box<dyn Error>>("unknown terminal dispatch".into())
    })
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "unknown terminal dispatch");
    assert_eq!(calls.get(), 1);

    struct Dropped<'a>(&'a std::cell::Cell<u32>);
    impl Drop for Dropped<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let dropped = std::cell::Cell::new(0);
    let guard = Dropped(&dropped);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(10),
            control_step("pause", async move {
                let _guard = guard;
                std::future::pending::<Result>().await
            }),
        )
        .await
        .is_err()
    );
    assert_eq!(dropped.get(), 1);
    Ok(())
}

#[tokio::test]
async fn controls_trace_contract() -> Result {
    let directory = std::env::temp_dir().join(format!("controls-contract-{}", std::process::id()));
    fs::create_dir_all(&directory)?;
    let path = directory.join("trace.jsonl");
    for journey in ["browse", "playback", "unknown", ""] {
        for phase in ["capture", "controls", "idle", ""] {
            for flag in ["0", "1", "", "true"] {
                assert_eq!(
                    controls_gate(journey, phase, flag),
                    journey == "playback" && phase == "controls" && flag == "1"
                );
            }
        }
    }
    assert!(ControlTrace::new(false, &path).is_none());
    assert!(!path.exists());
    let mut trace = ControlTrace::new(true, &path).unwrap();
    let mut dispatches = 0;
    let error = control_await(
        |event| trace.record("terminal", event, "home", None),
        async {
            let before: Value =
                serde_json::from_str(fs::read_to_string(&path)?.lines().last().unwrap())?;
            assert_eq!(before["event"], "begin");
            assert_eq!(before["terminal_attempts"], 1);
            dispatches += 1;
            Err::<(), Box<dyn Error>>("unknown input outcome".into())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "unknown input outcome");
    assert_eq!(dispatches, 1);
    assert_eq!(trace.attempts, 1);
    let lines: Vec<Value> = fs::read_to_string(&path)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["event"], "begin");
    assert_eq!(lines[0]["terminal_attempts"], 1);
    assert_eq!(lines[1]["event"], "failed");
    assert!(lines[0]["eligible"].is_null());
    let pending = tokio::time::timeout(
        Duration::from_millis(10),
        control_await(
            |event| trace.record("focus", event, "none", None),
            std::future::pending::<Result>(),
        ),
    )
    .await;
    assert!(pending.is_err());
    assert_eq!(
        fs::read_to_string(&path)?
            .lines()
            .last()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["event"].clone()),
        Some(json!("begin"))
    );
    trace.record("focus", "observed", "none", Some((true, false)));
    for _ in 0..300 {
        trace.record("tab", "begin", "none", None);
    }
    let text = fs::read_to_string(&path)?;
    assert_eq!(text.lines().count(), 256);
    assert!(text.len() <= 65536 && trace.failed);
    for (index, line) in text.lines().enumerate() {
        let row: Value = serde_json::from_str(line)?;
        assert_eq!(row.as_object().unwrap().len(), 9);
        assert_eq!(row["sequence"], index + 1);
        assert!(row["elapsed_ms"].as_u64().is_some());
        assert_eq!(row["step"], "prefix");
        assert!(!line.contains("unknown input") && !line.contains("http"));
    }
    let count = trace.records;
    trace.record("private-item", "begin", "none", None);
    assert_eq!(trace.records, count);
    let mut broken = ControlTrace::new(true, &directory).unwrap();
    let original = control_await(
        |event| broken.record("terminal", event, "space", None),
        async { Err::<(), Box<dyn Error>>("original".into()) },
    )
    .await
    .unwrap_err();
    assert_eq!(original.to_string(), "original");
    assert!(broken.failed);
    Ok(())
}

async fn control_prefix(session: &Arc<Session>, evidence: &mut Evidence) -> Result {
    control_step("motion", moving_media(session)).await?;
    control_step("playing-reveal", reveal(session)).await?;
    control_step("playing-image", evidence.capture(session, "playing", "Pause enabled; timeline advanced; two central video crops differed by over 1000 pixels.")).await?;
    control_step("idle-image", evidence
        .capture(
            session,
            "idle",
            "The same active playback Window described its controls as hidden before and after the image; fresh snapshots contained no visible chrome.",
        ))
        .await?;
    control_step("pause-reveal", reveal(session)).await?;
    control_step("pause-key", modal_activate(session, "Button", "Pause")).await?;
    control_step("play-visible", visible(session, "Play")).await?;
    control_step(
        "play-enabled",
        session.locate(PLAYER_PLAY).first().wait_for_enabled(),
    )
    .await?;
    control_step("pause-stability", async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let paused = position(session).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        if (position(session).await? - paused).abs() > 250.0 {
            return Err("Pause did not stabilize the actual timeline".into());
        }
        Ok::<(), Box<dyn Error>>(())
    })
    .await?;
    control_step(
        "paused-image",
        evidence.capture(
            session,
            "paused",
            "Player Play label visible; timeline stable across two seconds.",
        ),
    )
    .await?;
    control_step("settings-open", menu(session, evidence)).await?;
    control_step("volume-visible", visible(session, "Volume")).await?;
    control_step(
        "settings-image",
        evidence.capture(
            session,
            "settings",
            "Normal playback settings popover is visible.",
        ),
    )
    .await?;
    for (key, id, expected) in [(0xff50, "volume-min", 0.0), (0xff57, "volume-max", 1.0)] {
        control_step(
            if key == 0xff50 {
                "volume-home"
            } else {
                "volume-end"
            },
            async {
                let reference =
                    modal_key(session, "//Slider[@name='Volume']", "slider", key).await?;
                control_operation("value-bound", "none", async {
                    if (session.value_ref(&reference.0, &reference.1).await?.current - expected)
                        .abs()
                        > 0.01
                    {
                        return Err(
                            "Volume keyboard bound did not change the actual accessible value"
                                .into(),
                        );
                    }
                    Ok::<(), Box<dyn Error>>(())
                })
                .await?;
                evidence
                    .capture(
                        session,
                        id,
                        "Actual Volume slider value asserted at its representative bound.",
                    )
                    .await?;
                Ok::<(), Box<dyn Error>>(())
            },
        )
        .await?;
    }
    control_step("audio-visible", visible(session, "Audio")).await?;
    control_step("subtitles-visible", visible(session, "Subtitles")).await?;
    control_step("tracks-image", evidence.capture(session, "tracks", "Actual audio/subtitle menu; selected and disabled choices are shown as supplied by the real session.")).await?;
    control_step("track-choices", track_states(session, evidence)).await?;
    control_step("settings-close", close_menu(session, evidence)).await?;
    Ok(())
}

async fn normal_states(session: &Arc<Session>, evidence: &mut Evidence) -> Result {
    control_prefix(session, evidence).await?;
    session
        .locate("//*[@name='Playback position']")
        .first()
        .wait_for_enabled()
        .await?;
    modal_activate(session, "ToggleButton", "Mute").await?;
    visible(session, "Unmute").await?;
    evidence
        .capture(
            session,
            "muted",
            "Unmute label confirms the actual mute toggle is active.",
        )
        .await?;
    modal_activate(session, "ToggleButton", "Unmute").await?;
    for (button, id, forward) in [
        ("Forward 30 seconds", "seek-forward", true),
        ("Back 30 seconds", "seek-back", false),
    ] {
        let before = position(session).await?;
        modal_activate(session, "Button", button).await?;
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(message) = media_failure(session).await? {
                    return Err::<(), Box<dyn Error>>(message.into());
                }
                let after = position(session).await?;
                if if forward {
                    after > before + 20_000.0
                } else {
                    after < before - 20_000.0
                } {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await??;
        evidence
            .capture(
                session,
                id,
                "Actual timeline moved more than 20 seconds in the requested 30-second direction.",
            )
            .await?;
    }
    modal_activate(session, "Button", "Enter fullscreen").await?;
    visible(session, "Exit fullscreen").await?;
    evidence
        .capture(
            session,
            "fullscreen",
            "Exit fullscreen label confirms fullscreen; compositor screenshot measured 1280x800.",
        )
        .await?;
    modal_activate(session, "Button", "Exit fullscreen").await?;
    Ok(())
}

async fn close_player(session: &Arc<Session>, evidence: &mut Evidence) -> Result {
    if session
        .locate("//*[@name='Back to details']")
        .count()
        .await?
        > 0
    {
        menu(session, evidence).await?;
        modal_activate(session, "Button", "Stop playback").await?;
        session
            .locate("//*[@name='Back to details']")
            .first()
            .wait_for_hidden()
            .await?;
        evidence
            .capture(
                session,
                "stopped",
                "Normal Stop playback closed the owned player.",
            )
            .await?;
        let notice = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                for message in [
                    "Playback stopped locally. No session was confirmed; its outcome remains unknown.",
                    "Playback stopped locally. Remote cleanup remains pending.",
                    "Playback stopped. Final personal progress is confirmed.",
                    "Playback stopped locally. The final personal outcome remains unknown.",
                    "Playback stopped locally. The final personal outcome remains unknown; no report was repeated.",
                ] {
                    if shown(session, message).await? {
                        return Ok::<_, Box<dyn Error>>(message);
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }).await??;
        evidence.capture(session, "cleanup", &format!("{notice} Provider cleanup remains unknown; personal progress confirmation is not provider cleanup proof.")).await?;
    } else {
        let reason = absent_cleanup_reason(
            evidence.play_attempted,
            evidence.play_return_confirmed,
            evidence.player_observed,
        );
        if readiness_diagnostics() {
            if let Ok(bytes) = serde_json::to_vec_pretty(&json!({
                "close_control":"absent", "play_attempted":evidence.play_attempted,
                "play_return_confirmed":evidence.play_return_confirmed,
                "player_observed":evidence.player_observed, "normal_stop_confirmed":false,
                "provider_cleanup":"unconfirmed", "reason":reason
            })) {
                let _ = fs::write("/artifacts/playback-cleanup-context.json", bytes);
            }
        }
        let row = evidence.row("cleanup");
        row["status"] = json!("unavailable");
        row["reason"] = json!(reason);
    }
    Ok(())
}

pub(super) async fn journey() -> Result {
    let phase = match std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("capture") => "capture",
        Ok("navigation") => "navigation",
        Ok("readiness") if readiness_diagnostics() => "readiness",
        Ok("motion") if motion_diagnostics() => "motion",
        Ok("idle") if idle_diagnostics() => "idle",
        Ok("controls") if controls_diagnostics() => "controls",
        _ => return Err("Invalid playback phase".into()),
    };
    if readiness_diagnostics() {
        let _ = fs::write("/artifacts/playback-readiness-observations.jsonl", b"");
    }
    if readiness_diagnostics() || idle_diagnostics() {
        match fs::remove_file("/artifacts/playback-readiness-ownership.jsonl") {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    if phase == "idle" {
        for path in [
            "/artifacts/playback-idle-chrome.jsonl",
            "/artifacts/playback-idle-candidate.png",
            "/artifacts/playback-idle-candidate.json",
        ] {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
        }
    }
    clear_controls_candidates(std::path::Path::new("/artifacts"), controls_diagnostics())?;
    let navigation_only = phase == "navigation";
    let mut stage = "exact-id-preflight";
    breadcrumb(stage, "begin");
    let instance = std::env::var("SCREENROOM_BACKEND_INSTANCE_ID")?;
    let (library, item) =
        tokio::time::timeout(Duration::from_secs(15), exact_selection()).await??;
    enter_stage(&mut stage, "app-start");
    let session = tokio::time::timeout(Duration::from_secs(15), launch(vec![])).await??;
    let mut evidence = Evidence::new();
    let result = tokio::time::timeout(Duration::from_secs(100), async {
        visible(&session, "Home").await?;
        enter_stage(&mut stage, "search");
        enter_stage(&mut stage, "expand-sidebar");
        click(&session, "Expand sidebar").await?;
        enter_stage(&mut stage, "selected-library");
        click_library(&session, &library).await?;
        enter_stage(&mut stage, "collapse-sidebar");
        click(&session, "Collapse sidebar").await?;
        enter_stage(&mut stage, "search-titles");
        click(&session, "Search titles").await?;
        visible(&session, &format!("Enter a title to search {}.", library.title)).await?;
        configure_playback_search(&session, &mut stage).await?;
        enter_stage(&mut stage, "search-query");
        session.locate("//*[@editable='true']").first().fill(&item.title).await?;
        enter_stage(&mut stage, "exact-card-activate");
        activate_exact_item(&session, &item.id).await?;
        enter_stage(&mut stage, "detail");
        visible(&session, &format!("Movie · {}", item.year)).await?;
        visible(&session, &item.title).await?;
        visible(&session, "Play").await?;
        if navigation_only {
            fs::write("/artifacts/playback-navigation.png", session.take_screenshot().await?)?;
            return Ok(());
        }
        evidence.capture(&session, "detail", "Exact backend item/title/year/kind resolved by scoped search; exactly one matching item-ID accessibility description activated; matching title/year displayed in details.").await?;
        enter_stage(&mut stage, "Play");
        if phase == "readiness" {
            fs::write("/artifacts/playback-resource-baseline.json", serde_json::to_vec_pretty(&json!({
                "owned_process":owned_process_context(), "resources":resource_context()
            }))?)?;
        }
        evidence.play_attempted = true;
        click(&session, "Play").await?;
        evidence.play_return_confirmed = true;
        enter_stage(&mut stage, "readiness");
        if phase == "readiness" { enter_stage(&mut stage, "loading-probe"); }
        if shown(&session, "Preparing playback…").await? {
            evidence.player_observed = true;
            if phase == "readiness" { enter_stage(&mut stage, "loading-capture"); }
            evidence.capture(&session, "loading", "Preparing playback… appeared naturally after normal Play.").await?;
        }
        if phase == "readiness" { enter_stage(&mut stage, "media-readiness"); }
        let readiness_result = ready(&session).await?;
        if readiness_result.is_none() { evidence.player_observed = true; }
        if phase == "readiness" {
            fs::write("/artifacts/playback-readiness.png", session.take_screenshot().await?)?;
            return Ok(());
        }
        if phase == "motion" {
            if readiness_result.is_some() {
                return Err("A recognized media failure prevents the motion diagnostic".into());
            }
            enter_stage(&mut stage, "motion-comparison");
            moving_media(&session).await?;
            return Ok(());
        }
        if phase == "idle" {
            if readiness_result.is_some() {
                return Err("A recognized media failure prevents the idle diagnostic".into());
            }
            enter_stage(&mut stage, "idle-observation");
            idle_capture_probe(&session).await?;
            return Ok(());
        }
        if phase == "controls" {
            if readiness_result.is_some() {
                return Err("A recognized media failure prevents the control prefix diagnostic".into());
            }
            enter_stage(&mut stage, "controls-prefix");
            control_step("prefix", control_prefix(&session, &mut evidence)).await?;
            return Ok(());
        }
        enter_stage(&mut stage, "captures-controls");
        match readiness_result {
            Some(message) => {
                evidence.capture(&session, "media-error", message).await?;
                evidence.block_remaining();
            }
            None => {
                if let Err(error) = normal_states(&session, &mut evidence).await {
                    if let Some(message) = media_failure(&session).await? {
                        evidence.capture(&session, "media-error", message).await?;
                        evidence.block_remaining();
                    } else {
                        return Err(error);
                    }
                }
            },
        }
        Ok::<(), Box<dyn Error>>(())
    }).await.map_err(|_| -> Box<dyn Error> { "Playback journey exceeded its 100-second bound".into() }).and_then(|result| result);
    if let Err(error) = &result {
        failure_context(&session, stage, error.as_ref()).await;
    } else {
        breadcrumb(stage, "end");
    }
    // Attempt normal application-owned stop even after an assertion failure.
    stage = "normal-cleanup";
    breadcrumb(stage, "begin");
    let cleanup = tokio::time::timeout(
        Duration::from_secs(30),
        control_step("cleanup", close_player(&session, &mut evidence)),
    )
    .await;
    if readiness_diagnostics() {
        let category = match &cleanup {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(cleanup_failure_category(&error.to_string())),
            Err(_) => Some("timeout"),
        };
        if let Some(category) = category {
            if let Ok(bytes) = serde_json::to_vec_pretty(&json!({
                "cleanup_failure_category":category,
                "play_attempted":evidence.play_attempted,
                "play_return_confirmed":evidence.play_return_confirmed,
                "player_observed":evidence.player_observed,
                "normal_stop_confirmed":false, "provider_cleanup":"unconfirmed"
            })) {
                let _ = fs::write("/artifacts/playback-cleanup-context.json", bytes);
            }
        }
    }
    match &cleanup {
        Ok(Ok(())) => breadcrumb(stage, "end"),
        Ok(Err(error)) if result.is_ok() => failure_context(&session, stage, error.as_ref()).await,
        Err(error) if result.is_ok() => failure_context(&session, stage, error).await,
        _ => breadcrumb(stage, "failed"),
    }
    stage = "session-shutdown";
    breadcrumb(stage, "begin");
    let shutdown =
        if phase == "readiness" || phase == "motion" || phase == "idle" || phase == "controls" {
            let accessibility_failed = result
                .as_ref()
                .err()
                .is_some_and(|error| !failure_tree_allowed(&error.to_string()))
                || matches!(&cleanup, Ok(Err(error)) if !failure_tree_allowed(&error.to_string()));
            stop_readiness(session, accessibility_failed).await
        } else {
            stop(session, "playback").await
        };
    breadcrumb(stage, if shutdown.is_ok() { "end" } else { "failed" });
    result?;
    cleanup??;
    shutdown?;
    if phase == "controls" {
        controls_retained()?;
    }
    if phase != "capture" {
        // Focused diagnostics never create exportable playback evidence.
        Ok(())
    } else {
        evidence.write(&item, &instance)
    }
}
