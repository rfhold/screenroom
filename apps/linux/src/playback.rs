use adw::prelude::*;
use gst::prelude::*;
use screenroom_client_core::{self as core, PlaybackEventKind as Event};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::playback_state::{Timeline, clock, media_position, session_blocked as blocked};

const DIAGNOSTIC_MARKERS: &[&str] = &[
    "open.begin",
    "open.end",
    "create.begin",
    "create.end",
    "reply.send.begin",
    "reply.send.end",
    "reply.accept.begin",
    "reply.accept.end",
    "attach.begin",
    "attach.end",
    "attach.uri.begin",
    "attach.uri.end",
    "attach.paused.begin",
    "attach.paused.end",
    "detach.begin",
    "detach.end",
    "detach.null.begin",
    "detach.null.end",
    "query.position.begin",
    "query.position.end",
    "query.duration.begin",
    "query.duration.end",
    "bus.error",
    "bus.eos",
    "bus.async-done",
    "bus.state-changed",
    "bus.playing.begin",
    "bus.playing.end",
    "gtk.heartbeat",
];

fn diagnostics_enabled(flag: Option<&str>, phase: Option<&str>, journey: Option<&str>) -> bool {
    flag == Some("1") && phase == Some("readiness") && journey == Some("playback")
}

fn diagnostic_line(sequence: u32, elapsed_ms: u128, marker: &str) -> Option<String> {
    (sequence > 0 && sequence <= 256 && DIAGNOSTIC_MARKERS.contains(&marker)).then(|| {
        format!("playback-diagnostic sequence={sequence} elapsed_ms={elapsed_ms} marker={marker}")
    })
}

struct Diagnostics {
    start: Instant,
    sequence: std::sync::atomic::AtomicU32,
    heartbeats: std::sync::atomic::AtomicU32,
}

fn diagnostics() -> Option<&'static Diagnostics> {
    static STATE: std::sync::OnceLock<Option<Diagnostics>> = std::sync::OnceLock::new();
    STATE
        .get_or_init(|| {
            diagnostics_enabled(
                std::env::var("SCREENROOM_PLAYBACK_DIAGNOSTICS")
                    .ok()
                    .as_deref(),
                std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE")
                    .ok()
                    .as_deref(),
                std::env::var("SCREENROOM_E2E_JOURNEY").ok().as_deref(),
            )
            .then(|| Diagnostics {
                start: Instant::now(),
                sequence: std::sync::atomic::AtomicU32::new(0),
                heartbeats: std::sync::atomic::AtomicU32::new(0),
            })
        })
        .as_ref()
}

fn diagnostic_event(marker: &'static str) {
    use std::sync::atomic::Ordering::Relaxed;
    if let Some(state) = diagnostics() {
        if let Ok(previous) = state
            .sequence
            .fetch_update(Relaxed, Relaxed, |value| (value < 256).then(|| value + 1))
        {
            if let Some(line) =
                diagnostic_line(previous + 1, state.start.elapsed().as_millis(), marker)
            {
                eprintln!("{line}");
            }
        }
    }
}

struct DiagnosticSpan(&'static str);

fn diagnostic_span(begin: &'static str, end: &'static str) -> DiagnosticSpan {
    diagnostic_event(begin);
    DiagnosticSpan(end)
}

impl Drop for DiagnosticSpan {
    fn drop(&mut self) {
        diagnostic_event(self.0);
    }
}

#[test]
fn playback_diagnostic_contract() {
    assert!(diagnostics_enabled(
        Some("1"),
        Some("readiness"),
        Some("playback")
    ));
    for flag in [None, Some(""), Some("0"), Some("true"), Some("01")] {
        assert!(!diagnostics_enabled(
            flag,
            Some("readiness"),
            Some("playback")
        ));
    }
    assert!(!diagnostics_enabled(
        Some("1"),
        Some("capture"),
        Some("playback")
    ));
    assert!(!diagnostics_enabled(
        Some("1"),
        Some("readiness"),
        Some("browse")
    ));
    for marker in DIAGNOSTIC_MARKERS {
        assert_eq!(
            diagnostic_line(1, 42, marker),
            Some(format!(
                "playback-diagnostic sequence=1 elapsed_ms=42 marker={marker}"
            ))
        );
    }
    for sequence in [0, 257, u32::MAX] {
        assert!(diagnostic_line(sequence, 0, "gtk.heartbeat").is_none());
    }
    assert!(diagnostic_line(1, 0, "https://private.invalid/?cap=secret").is_none());
    assert!(diagnostic_line(1, 0, "session:private").is_none());
}

fn idle_diagnostics_enabled(
    flag: Option<&str>,
    phase: Option<&str>,
    journey: Option<&str>,
) -> bool {
    flag == Some("1") && phase == Some("idle") && journey == Some("playback")
}

fn idle_decision_line(
    sequence: u32,
    elapsed_ms: u128,
    idle_age_ms: u128,
    flags: [bool; 12],
) -> Option<String> {
    (1..=128).contains(&sequence).then(|| {
        let [
            paused,
            busy,
            blocked,
            preroll,
            started,
            menu_visible,
            menu_mapped,
            control_focus,
            keyboard_engaged,
            scrubbing,
            show,
            preparing,
        ] = flags;
        serde_json::json!({
            "sequence":sequence,"elapsed_ms":elapsed_ms,"idle_age_ms":idle_age_ms,
            "paused":paused,"busy":busy,"blocked":blocked,"preroll":preroll,
            "started":started,"menu_visible":menu_visible,"menu_mapped":menu_mapped,
            "control_focus":control_focus,"keyboard_engaged":keyboard_engaged,
            "scrubbing":scrubbing,"show":show,"preparing":preparing
        })
        .to_string()
    })
}

fn idle_sample(sequence: u32, last: Duration, elapsed: Duration) -> Option<(u32, Duration)> {
    if sequence >= 128
        || (sequence > 0 && elapsed.saturating_sub(last) < Duration::from_millis(200))
    {
        None
    } else {
        Some((sequence + 1, elapsed))
    }
}

fn idle_diagnostic_start() -> Option<&'static Instant> {
    static START: std::sync::OnceLock<Option<Instant>> = std::sync::OnceLock::new();
    START
        .get_or_init(|| {
            idle_diagnostics_enabled(
                std::env::var("SCREENROOM_PLAYBACK_DIAGNOSTICS")
                    .ok()
                    .as_deref(),
                std::env::var("SCREENROOM_E2E_PLAYBACK_PHASE")
                    .ok()
                    .as_deref(),
                std::env::var("SCREENROOM_E2E_JOURNEY").ok().as_deref(),
            )
            .then(Instant::now)
        })
        .as_ref()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdleRevealOrigin {
    Key,
    Motion,
    StagePress,
    ScrubPress,
    ScrubRelease,
    MenuClosed,
}

impl IdleRevealOrigin {
    fn name(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Motion => "motion",
            Self::StagePress => "stage-press",
            Self::ScrubPress => "scrub-press",
            Self::ScrubRelease => "scrub-release",
            Self::MenuClosed => "menu-closed",
        }
    }
}

fn idle_receipt_line(
    sequence: u32,
    elapsed_ms: u128,
    idle_age_ms: u128,
    origin: IdleRevealOrigin,
    coordinates: Option<(f64, f64)>,
    previous: &mut Option<(f64, f64)>,
) -> Option<String> {
    if !(1..=128).contains(&sequence) || elapsed_ms > 128 * 200 {
        return None;
    }
    let motion = origin == IdleRevealOrigin::Motion;
    let current = coordinates.filter(|(x, y)| motion && x.is_finite() && y.is_finite());
    let prior = previous.filter(|(x, y)| motion && x.is_finite() && y.is_finite());
    let delta = current.zip(prior).and_then(|((x, y), (px, py))| {
        let (dx, dy) = (x - px, y - py);
        (dx.is_finite() && dy.is_finite()).then_some((dx, dy))
    });
    let changed = current.zip(prior).map(|(current, prior)| current != prior);
    if let Some(current) = current {
        *previous = Some(current);
    }
    Some(
        serde_json::json!({
            "sequence":sequence, "elapsed_ms":elapsed_ms, "idle_age_before_reveal_ms":idle_age_ms,
            "origin":origin.name(), "coordinate_space":"stage-local",
            "coordinates_valid":motion.then_some(current.is_some()),
            "x":current.map(|p|p.0), "y":current.map(|p|p.1),
            "previous_x":prior.map(|p|p.0), "previous_y":prior.map(|p|p.1),
            "delta_x":delta.map(|p|p.0), "delta_y":delta.map(|p|p.1),
            "coordinates_changed":changed
        })
        .to_string(),
    )
}

fn idle_receipt(origin: IdleRevealOrigin, coordinates: Option<(f64, f64)>, idle_age: Duration) {
    let Some(start) = idle_diagnostic_start() else {
        return;
    };
    static STATE: std::sync::Mutex<(u32, Option<(f64, f64)>)> = std::sync::Mutex::new((0, None));
    let Ok(mut state) = STATE.lock() else { return };
    let sequence = state.0.saturating_add(1);
    if let Some(line) = idle_receipt_line(
        sequence,
        start.elapsed().as_millis(),
        idle_age.as_millis(),
        origin,
        coordinates,
        &mut state.1,
    ) {
        state.0 = sequence;
        eprintln!("playback-idle-receipt {line}");
    }
}

fn idle_chrome_line(
    sequence: u32,
    elapsed_ms: u128,
    requested_show: bool,
    chrome: [bool; 4],
) -> Option<String> {
    if !(1..=128).contains(&sequence) || elapsed_ms > 25600 {
        return None;
    }
    Some(
        serde_json::json!({
            "sequence":sequence, "elapsed_ms":elapsed_ms, "requested_show":requested_show,
            "header_reveal_child":chrome[0], "header_child_revealed":chrome[1],
            "footer_reveal_child":chrome[2], "footer_child_revealed":chrome[3]
        })
        .to_string(),
    )
}

#[test]
fn idle_chrome_contract() {
    for bits in 0..16 {
        let flags = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0];
        let line = idle_chrome_line(1, 200, false, flags).unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 7);
        assert_eq!(value["header_reveal_child"], flags[0]);
        assert_eq!(value["header_child_revealed"], flags[1]);
        assert_eq!(value["footer_reveal_child"], flags[2]);
        assert_eq!(value["footer_child_revealed"], flags[3]);
    }
    assert!(idle_chrome_line(0, 0, false, [false; 4]).is_none());
    assert!(idle_chrome_line(129, 0, false, [false; 4]).is_none());
    assert!(idle_chrome_line(1, 25601, false, [false; 4]).is_none());
}

fn idle_decision(idle_age: Duration, flags: [bool; 12], chrome: impl FnOnce() -> [bool; 4]) {
    struct State {
        start: Instant,
        samples: std::sync::Mutex<(u32, Duration)>,
    }
    static STATE: std::sync::OnceLock<Option<State>> = std::sync::OnceLock::new();
    let state = STATE.get_or_init(|| {
        idle_diagnostic_start().map(|start| State {
            start: *start,
            samples: std::sync::Mutex::new((0, Duration::ZERO)),
        })
    });
    let Some(state) = state else { return };
    let Ok(mut samples) = state.samples.lock() else {
        return;
    };
    let elapsed = state.start.elapsed();
    let Some(next) = idle_sample(samples.0, samples.1, elapsed) else {
        return;
    };
    *samples = next;
    if let Some(line) = idle_chrome_line(samples.0, elapsed.as_millis(), flags[10], chrome()) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(samples.0 > 1)
            .truncate(samples.0 == 1)
            .open("/artifacts/playback-idle-chrome.jsonl")
        {
            let _ = writeln!(file, "{line}");
        }
    }
    if let Some(line) =
        idle_decision_line(samples.0, elapsed.as_millis(), idle_age.as_millis(), flags)
    {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/artifacts/playback-idle-decisions.jsonl")
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[test]
fn idle_receipt_emitter_fixture() {
    if std::env::var("SCREENROOM_IDLE_RECEIPT_UNIT")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    let origins = [
        IdleRevealOrigin::Key,
        IdleRevealOrigin::Motion,
        IdleRevealOrigin::StagePress,
        IdleRevealOrigin::ScrubPress,
        IdleRevealOrigin::ScrubRelease,
        IdleRevealOrigin::MenuClosed,
    ];
    for index in 0..140 {
        idle_receipt(
            origins[index % origins.len()],
            Some((642.0, 402.0)),
            Duration::from_millis(1000),
        );
    }
}

#[test]
fn idle_receipt_contract() {
    let mut previous = None;
    let mut sample = |sequence, coordinates| -> serde_json::Value {
        serde_json::from_str(
            &idle_receipt_line(
                sequence,
                10,
                900,
                IdleRevealOrigin::Motion,
                coordinates,
                &mut previous,
            )
            .unwrap(),
        )
        .unwrap()
    };
    let first = sample(1, Some((642.0, 402.0)));
    assert!(first["previous_x"].is_null());
    let same = sample(2, Some((642.0, 402.0)));
    assert_eq!(same["coordinates_changed"], false);
    assert_eq!(same["delta_x"], 0.0);
    let small = sample(3, Some((642.0001, 402.0)));
    assert_eq!(small["coordinates_changed"], true);
    assert!(small["delta_x"].as_f64().unwrap() > 0.0);
    for coordinates in [
        None,
        Some((f64::NAN, 0.0)),
        Some((1.0, f64::INFINITY)),
        Some((f64::NEG_INFINITY, 0.0)),
    ] {
        let invalid = sample(4, coordinates);
        assert_eq!(invalid["coordinates_valid"], false);
        for key in ["x", "y", "delta_x", "delta_y", "coordinates_changed"] {
            assert!(invalid[key].is_null());
        }
        assert_eq!(invalid["previous_x"], 642.0001);
    }
    sample(5, Some((f64::MAX, f64::MAX)));
    let overflow = sample(6, Some((-f64::MAX, -f64::MAX)));
    assert!(overflow["delta_x"].is_null() && overflow["delta_y"].is_null());
    for origin in [
        IdleRevealOrigin::Key,
        IdleRevealOrigin::StagePress,
        IdleRevealOrigin::ScrubPress,
        IdleRevealOrigin::ScrubRelease,
        IdleRevealOrigin::MenuClosed,
    ] {
        let old = previous;
        let row: serde_json::Value = serde_json::from_str(
            &idle_receipt_line(7, 10, 900, origin, Some((1.0, 2.0)), &mut previous).unwrap(),
        )
        .unwrap();
        assert_eq!(previous, old);
        assert!(row["x"].is_null() && row["previous_x"].is_null());
        assert!(row["coordinates_valid"].is_null());
        assert_eq!(row["origin"], origin.name());
    }
    for (sequence, elapsed) in [(0, 0), (129, 0), (u32::MAX, 0), (1, 25601)] {
        let old = previous;
        assert!(
            idle_receipt_line(
                sequence,
                elapsed,
                0,
                IdleRevealOrigin::Motion,
                Some((1.0, 2.0)),
                &mut previous
            )
            .is_none()
        );
        assert_eq!(previous, old);
    }
    assert!(
        idle_receipt_line(128, 25600, 0, IdleRevealOrigin::Motion, None, &mut previous).is_some()
    );
    for (flag, phase, journey, expected) in [
        (None, None, None, 0),
        (Some("1"), None, None, 0),
        (Some("0"), Some("idle"), Some("playback"), 0),
        (Some("true"), Some("idle"), Some("playback"), 0),
        (Some("1"), Some("capture"), Some("playback"), 0),
        (Some("1"), Some("idle"), Some("browse"), 0),
        (Some("1"), Some("idle"), Some("playback"), 128),
    ] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "playback::idle_receipt_emitter_fixture",
                "--nocapture",
            ])
            .env("SCREENROOM_IDLE_RECEIPT_UNIT", "1");
        for (name, value) in [
            ("SCREENROOM_PLAYBACK_DIAGNOSTICS", flag),
            ("SCREENROOM_E2E_PLAYBACK_PHASE", phase),
            ("SCREENROOM_E2E_JOURNEY", journey),
        ] {
            command.env_remove(name);
            if let Some(value) = value {
                command.env(name, value);
            }
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        let receipts: Vec<_> = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("playback-idle-receipt "))
            .collect();
        assert_eq!(receipts.len(), expected);
        for (index, line) in receipts.iter().enumerate() {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(row["sequence"], index + 1);
            assert_eq!(row.as_object().unwrap().len(), 13);
            assert_eq!(row["coordinate_space"], "stage-local");
            assert!(
                [
                    "key",
                    "motion",
                    "stage-press",
                    "scrub-press",
                    "scrub-release",
                    "menu-closed"
                ]
                .contains(&row["origin"].as_str().unwrap())
            );
            assert!(row["elapsed_ms"].as_u64().unwrap() <= 25600);
            assert!(!line.contains("http") && !line.contains("NaN") && !line.contains("Infinity"));
        }
        if expected > 0 {
            let names: std::collections::HashSet<_> = receipts
                .iter()
                .map(|line| {
                    serde_json::from_str::<serde_json::Value>(line).unwrap()["origin"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect();
            assert_eq!(names.len(), 6);
        }
    }
}

#[test]
fn idle_diagnostic_contract() {
    let mut samples = (0, Duration::ZERO);
    for index in 0..128 {
        samples = idle_sample(samples.0, samples.1, Duration::from_millis(index * 200)).unwrap();
        assert_eq!(samples.0 as u64, index + 1);
        assert!(
            idle_sample(samples.0, samples.1, samples.1 + Duration::from_millis(199)).is_none()
        );
    }
    assert!(idle_sample(samples.0, samples.1, Duration::from_secs(100)).is_none());
    for flag in [None, Some(""), Some("0"), Some("1"), Some("true")] {
        for phase in [
            None,
            Some("capture"),
            Some("readiness"),
            Some("motion"),
            Some("idle"),
        ] {
            for journey in [None, Some("browse"), Some("playback")] {
                assert_eq!(
                    idle_diagnostics_enabled(flag, phase, journey),
                    flag == Some("1") && phase == Some("idle") && journey == Some("playback")
                );
            }
        }
    }
    for sequence in [0, 129, u32::MAX] {
        assert!(idle_decision_line(sequence, 0, 0, [false; 12]).is_none());
    }
    for bit in 0..12 {
        let mut flags = [false; 12];
        flags[bit] = true;
        let line = idle_decision_line(128, 200, 3000, flags).unwrap();
        let row: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(row.as_object().unwrap().len(), 15);
        assert_eq!(
            row.as_object()
                .unwrap()
                .values()
                .filter(|v| **v == true)
                .count(),
            1
        );
        assert!(!line.contains("http"));
        assert_eq!(row["idle_age_ms"], 3000);
    }
}

#[derive(Default)]
struct MotionActivity(Cell<Option<(f64, f64)>>);

impl MotionActivity {
    fn moved(&self, x: f64, y: f64) -> bool {
        if !x.is_finite() || !y.is_finite() || self.0.get() == Some((x, y)) {
            return false;
        }
        self.0.set(Some((x, y)));
        true
    }

    fn left(&self) {
        self.0.set(None);
    }
}

#[test]
fn motion_activity_contract() {
    let activity = MotionActivity::default();
    let receipts = [
        (3542, 548.0, 372.0, 590),
        (3562, 552.0, 376.0, 19),
        (3952, 552.0, 376.0, 390),
        (4961, 552.0, 376.0, 1008),
        (5983, 552.0, 376.0, 1022),
        (6961, 552.0, 376.0, 977),
        (7967, 552.0, 376.0, 1006),
        (8964, 552.0, 376.0, 996),
        (9958, 552.0, 376.0, 993),
        (10985, 552.0, 376.0, 1027),
        (11960, 552.0, 376.0, 974),
        (12964, 552.0, 376.0, 1004),
        (13965, 552.0, 376.0, 1001),
    ];
    let mut last_activity = 0;
    for (index, (elapsed, x, y, _old_age)) in receipts.into_iter().enumerate() {
        let changed = activity.moved(x, y);
        assert_eq!(changed, index < 2);
        if changed {
            last_activity = elapsed;
        }
        assert_eq!(
            screenroom_linux::player_presentation::Visibility::default()
                .show(Duration::from_millis(elapsed - last_activity)),
            elapsed < 6562,
        );
    }
    assert_eq!(last_activity, 3562);
    assert!(activity.moved(552.000001, 376.0));
    assert!(activity.moved(552.000001, 376.000001));
    assert!(!activity.moved(552.000001, 376.000001));
    for (x, y) in [
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, 1.0),
    ] {
        let old = activity.0.get();
        assert!(!activity.moved(x, y));
        assert_eq!(activity.0.get(), old);
    }
    assert!(!activity.moved(552.000001, 376.000001));
    activity.left();
    assert!(activity.moved(552.000001, 376.000001));
    assert!(!activity.moved(552.000001, 376.000001));
    let other_player = MotionActivity::default();
    assert!(other_player.moved(552.000001, 376.000001));
    assert!(!activity.moved(552.000001, 376.000001));
    other_player.left();
    assert!(!activity.moved(552.000001, 376.000001));
    let first_invalid = MotionActivity::default();
    assert!(!first_invalid.moved(f64::NAN, 0.0));
    assert!(first_invalid.moved(0.0, 0.0));
    assert!(!first_invalid.moved(-0.0, 0.0));
}

#[derive(Clone)]
struct Observation {
    position: u64,
    paused: bool,
    muted: bool,
    volume: f64,
}

enum Command {
    Create,
    Check,
    Update(Option<core::PlaybackTrackChoices>, bool, Observation),
    RetryUpdate,
    Report(Event, Observation),
    Stop(Observation, String),
}

struct Reply {
    session: Option<core::PlaybackSession>,
    message: &'static str,
    blocked: bool,
    retry_create: bool,
    retry_update: bool,
    finished: bool,
    can_update: bool,
}

fn stop_request(
    session: &core::PlaybackSession,
    position: u64,
    reason: String,
    sequence: u32,
) -> core::StopPlaybackSessionRequest {
    core::StopPlaybackSessionRequest {
        session_id: session.session_id.clone(),
        control_capability: session.control_capability.clone(),
        request_id: uuid::Uuid::new_v4().to_string(),
        final_sequence: sequence,
        transport_revision: session.transport_revision,
        position_ms: position.to_string(),
        stop_reason: reason,
    }
}

async fn read(
    client: &Arc<core::Client>,
    session: &core::PlaybackSession,
) -> Result<Option<core::PlaybackSession>, core::ClientError> {
    Ok(client
        .clone()
        .get_playback_session(core::GetPlaybackSessionRequest {
            session_id: session.session_id.clone(),
            control_capability: session.control_capability.clone(),
        })
        .await?
        .session)
}

// This actor owns authority and exact mutation bodies; GTK receives owned snapshots only.
async fn control(
    client: Arc<core::Client>,
    creation: core::CreatePlaybackSessionRequest,
    mut commands: tokio::sync::mpsc::Receiver<Command>,
    replies: tokio::sync::mpsc::Sender<Reply>,
) {
    let mut session: Option<core::PlaybackSession> = None;
    let mut update: Option<core::UpdatePlaybackSessionRequest> = None;
    let mut stopped = false;
    let mut reports = crate::playback_state::Reports::default();
    let mut fallback_used = false;
    while let Some(command) = commands.recv().await {
        let mut message = "Personal playback state confirmed.";
        let mut failed = false;
        let mut retry_create = false;
        let mut retry_update = false;
        match command {
            Command::Create if session.is_none() && !stopped => {
                let _diagnostic = diagnostic_span("create.begin", "create.end");
                match client
                    .clone()
                    .create_playback_session(creation.clone())
                    .await
                {
                    Ok(response) => session = response.session,
                    Err(_) => {
                        failed = true;
                        retry_create = true;
                        message = "Playback preparation was not confirmed. Retry retains the exact request; no new session is created.";
                    }
                }
            }
            Command::Check if !stopped => {
                if let Some(current) = &session {
                    match read(&client, current).await {
                        Ok(Some(value)) => {
                            if update.as_ref().is_some_and(|body| {
                                value.transport_revision > body.expected_transport_revision
                            }) {
                                update = None;
                            }
                            session = Some(value);
                        }
                        _ => {
                            failed = true;
                            message =
                                "Session state could not be confirmed. Playback remains stopped.";
                        }
                    }
                }
                retry_update = update.is_some()
                    && !reports.uncertain()
                    && session.as_ref().is_some_and(|s| !blocked(s));
            }
            Command::Update(choices, fallback, observation)
                if !stopped
                    && !reports.uncertain()
                    && session.as_ref().is_some_and(|s| !blocked(s))
                    && update.is_none()
                    && (!fallback || !fallback_used) =>
            {
                let current = session.as_ref().unwrap();
                fallback_used |= fallback;
                update = Some(crate::playback_state::transport_update(
                    current,
                    uuid::Uuid::new_v4().to_string(),
                    observation.position,
                    choices,
                    fallback,
                ));
                match client
                    .clone()
                    .update_playback_session(update.clone().unwrap())
                    .await
                {
                    Ok(response) => {
                        session = response.session;
                        update = None;
                    }
                    Err(_) => {
                        failed = true;
                        message = "Transport change was not confirmed. Check session state before an explicit exact retry.";
                    }
                }
            }
            Command::RetryUpdate
                if !stopped
                    && !reports.uncertain()
                    && session.as_ref().is_some_and(|s| !blocked(s)) =>
            {
                if let Some(body) = &update {
                    match client.clone().update_playback_session(body.clone()).await {
                        Ok(response) => {
                            session = response.session;
                            update = None;
                        }
                        Err(_) => {
                            failed = true;
                            message = "Transport retry was not confirmed. Check session state; no automatic replay.";
                        }
                    }
                }
            }
            Command::Report(kind, observation)
                if !stopped
                    && !reports.uncertain()
                    && update.is_none()
                    && session.as_ref().is_some_and(|s| !blocked(s)) =>
            {
                let current = session.as_ref().unwrap();
                let Some(sequence) =
                    reports.next(kind == Event::Started, current.last_accepted_sequence)
                else {
                    reports.fail();
                    let _ = replies.send(Reply {
                        session: session.clone(), message: "Playback report order could not be confirmed. Close the player.",
                        blocked: true, retry_create: false, retry_update: false, finished: false, can_update: false,
                    }).await;
                    continue;
                };
                let result = client
                    .clone()
                    .report_playback_event(core::ReportPlaybackEventRequest {
                        session_id: current.session_id.clone(),
                        control_capability: current.control_capability.clone(),
                        transport_revision: current.transport_revision,
                        sequence,
                        kind,
                        position_ms: observation.position.to_string(),
                        paused: observation.paused,
                        muted: observation.muted,
                        volume: observation.volume,
                    })
                    .await;
                match result {
                    Ok(response) => {
                        if matches!(
                            response.report_outcome,
                            core::PlaybackReportOutcome::Pending
                                | core::PlaybackReportOutcome::Uncertain
                                | core::PlaybackReportOutcome::Failed
                        ) {
                            reports.fail();
                        }
                        session = response.session;
                        if response.report_outcome == core::PlaybackReportOutcome::Confirmed {
                            client.invalidate_reads();
                        }
                    }
                    Err(_) => {
                        reports.fail();
                        failed = true;
                        message = "Personal playback outcome is unknown. Media is stopped; this event will not be repeated.";
                        if let Ok(Some(value)) = read(&client, current).await {
                            session = Some(value);
                        }
                    }
                }
            }
            Command::Stop(observation, reason) => {
                if update.is_some() || reports.uncertain() {
                    if let Some(current) = &session {
                        if let Ok(Some(value)) = read(&client, current).await {
                            session = Some(value);
                        }
                    }
                }
                let mut message = "Playback stopped locally. No session was confirmed; its outcome remains unknown.";
                if let Some(current) = &session {
                    let body = stop_request(
                        current,
                        observation.position,
                        reason,
                        reports.final_sequence(current.last_accepted_sequence),
                    );
                    match client.clone().stop_playback_session(body).await {
                        Ok(response) => {
                            message = if response
                                .session
                                .as_ref()
                                .is_some_and(|s| s.cleanup_pending)
                            {
                                "Playback stopped locally. Remote cleanup remains pending."
                            } else if response.session.as_ref().is_some_and(|s| {
                                s.report_outcome == core::PlaybackReportOutcome::Confirmed
                            }) {
                                "Playback stopped. Final personal progress is confirmed."
                            } else {
                                "Playback stopped locally. The final personal outcome remains unknown."
                            };
                        }
                        Err(_) => {
                            // An ambiguous final personal report is never replayed.
                            let _ = read(&client, current).await;
                            message = "Playback stopped locally. The final personal outcome remains unknown; no report was repeated.";
                        }
                    }
                }
                client.invalidate_reads();
                let _ = replies
                    .send(Reply {
                        session: None,
                        message,
                        blocked: true,
                        retry_create: false,
                        retry_update: false,
                        finished: true,
                        can_update: false,
                    })
                    .await;
                stopped = true;
            }
            _ => {}
        }
        if stopped {
            break;
        }
        let denied = failed
            || reports.uncertain()
            || update.is_some()
            || session.as_ref().is_none_or(blocked);
        if denied && !failed {
            message = "Playback is unavailable or personal state is uncertain. Check session state or close the player.";
        }
        if update.is_some() && retry_update {
            message = "The previous transport remains stopped. Retry the exact transport change or close the player.";
        }
        if denied && !failed {
            if let Some(code) = session.as_ref().and_then(|s| s.failure_code.as_ref()) {
                message = crate::playback_state::failure_message(code);
            }
        }
        let _diagnostic = diagnostic_span("reply.send.begin", "reply.send.end");
        let _ = replies
            .send(Reply {
                session: session.clone(),
                message,
                blocked: denied,
                retry_create,
                retry_update,
                finished: false,
                can_update: !failed
                    && !reports.uncertain()
                    && update.is_none()
                    && session.as_ref().is_some_and(|s| !blocked(s)),
            })
            .await;
    }
}

fn chrome_description(revealers: [bool; 4]) -> &'static str {
    if revealers.iter().all(|state| !state) {
        "Playback controls hidden"
    } else if revealers.iter().all(|state| *state) {
        "Playback controls visible"
    } else {
        "Playback controls changing"
    }
}

#[test]
fn chrome_description_contract() {
    for bits in 0..16 {
        let flags = std::array::from_fn(|index| bits & (1 << index) != 0);
        assert_eq!(
            chrome_description(flags),
            match bits {
                0 => "Playback controls hidden",
                15 => "Playback controls visible",
                _ => "Playback controls changing",
            }
        );
    }
}

pub struct Player {
    window: adw::Window,
    pipeline: gst::Element,
    status: gtk::Label,
    time: gtk::Label,
    seek: gtk::Scale,
    tracks: gtk::Box,
    retry_create: gtk::Button,
    retry_update: gtk::Button,
    fallback: gtk::Button,
    play: gtk::Button,
    stage: gtk::Overlay,
    header: gtk::Revealer,
    footer: gtk::Revealer,
    menu: gtk::Popover,
    spinner: gtk::Spinner,
    feedback: gtk::Box,
    fullscreen: gtk::Button,
    last_activity: Cell<Instant>,
    motion_activity: MotionActivity,
    accessible_chrome: Cell<Option<&'static str>>,
    keyboard_focus: Cell<bool>,
    scrubbing: Cell<bool>,
    presentation_timer: RefCell<Option<gtk::glib::SourceId>>,
    tick_timer: RefCell<Option<gtk::glib::SourceId>>,
    commands: tokio::sync::mpsc::Sender<Command>,
    session: RefCell<Option<core::PlaybackSession>>,
    timeline: RefCell<Timeline>,
    revision: Cell<u32>,
    attached: Cell<bool>,
    busy: Cell<bool>,
    denied: Cell<bool>,
    allow_update: Cell<bool>,
    closed: Cell<bool>,
    started: Cell<bool>,
    paused: Cell<bool>,
    seeking: Cell<bool>,
    preroll: Cell<bool>,
    initial_seek: Cell<bool>,
    fallback_used: Cell<bool>,
    was_active: Cell<bool>,
    ticks: Cell<u64>,
    watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    on_close: Box<dyn Fn(&str)>,
}

fn has(name: &str) -> bool {
    gst::ElementFactory::find(name).is_some()
}

fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("player-icon");
    name_control(&button, label);
    button
}

fn menu_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_max_width_chars(28);
        label.set_xalign(0.0);
    }
    button
}

fn name_control(widget: &impl IsA<gtk::Widget>, name: &str) {
    widget.set_tooltip_text(Some(name));
    widget
        .as_ref()
        .update_property(&[gtk::accessible::Property::Label(name)]);
}

fn chrome(child: &impl IsA<gtk::Widget>, align: gtk::Align) -> gtk::Revealer {
    gtk::Revealer::builder()
        .child(child)
        .valign(align)
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(220)
        .reveal_child(true)
        .build()
}

fn capabilities() -> core::PlaybackClientCapabilities {
    core::PlaybackClientCapabilities {
        mp4: has("qtdemux"),
        h264: ["avdec_h264", "openh264dec"].iter().any(|name| has(name)),
        aac: ["avdec_aac", "faad", "fdkaacdec"]
            .iter()
            .any(|name| has(name)),
        native_hls: (has("hlsdemux") || has("hlsdemux2"))
            && has("tsdemux")
            && has("h264parse")
            && has("aacparse"),
        mse_hls: false,
        webvtt: has("subparse") && has("subtitleoverlay") && has("textoverlay"),
    }
}

impl Player {
    pub fn open(
        parent: &impl IsA<gtk::Window>,
        client: Arc<core::Client>,
        item_id: String,
        title: &str,
        scope: core::SourceLibraryScope,
        beginning: bool,
        on_close: impl Fn(&str) + 'static,
    ) -> Result<Rc<Self>, &'static str> {
        let _diagnostic = diagnostic_span("open.begin", "open.end");
        gst::init().map_err(|_| "GStreamer could not be initialized.")?;
        // Disable native diagnostic output: media URIs carry bearer capabilities.
        gst::log::set_active(false);
        let sink = gst::ElementFactory::make("gtk4paintablesink")
            .build()
            .map_err(
                |_| "Install the GStreamer GTK4 paintable sink (gtk4paintablesink) to play video.",
            )?;
        let paintable = sink.property::<gtk::gdk::Paintable>("paintable");
        let pipeline = gst::ElementFactory::make("playbin")
            .build()
            .map_err(|_| "Install the GStreamer playback plugin (playbin).")?;
        let bin = pipeline
            .clone()
            .downcast::<gst::Bin>()
            .map_err(|_| "GStreamer playback bin is unavailable.")?;
        bin.connect_deep_element_added(|_, _, element| {
            if element
                .factory()
                .is_some_and(|factory| factory.name() == "souphttpsrc")
            {
                element.set_property("automatic-redirect", false);
                element.set_property("proxy", "");
                element.set_property("user-agent", "Screenroom Linux");
            }
        });
        pipeline.set_property("video-sink", &sink);
        let caps = capabilities();
        if !caps.mp4 || !caps.h264 || !caps.aac || !has("souphttpsrc") {
            return Err(
                "Playback requires GStreamer HTTP, MP4, H.264 and AAC plugins. Install base/good/libav plugins.",
            );
        }
        let window = adw::Window::builder()
            .title(title)
            .transient_for(parent)
            .modal(true)
            .default_width(1100)
            .default_height(720)
            .build();
        let stage = gtk::Overlay::new();
        stage.add_css_class("native-player");
        stage.set_focusable(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.add_css_class("player-header");
        let heading = crate::home::label(title, "row-heading");
        heading.set_wrap(false);
        heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.set_width_chars(1);
        heading.set_hexpand(true);
        let fullscreen = icon_button("view-fullscreen-symbolic", "Enter fullscreen");
        let close = icon_button("go-previous-symbolic", "Back to details");
        top.append(&close);
        top.append(&heading);
        top.append(&fullscreen);
        let picture = gtk::Picture::for_paintable(&paintable);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        stage.set_child(Some(&picture));
        // Captions keep a fixed safe area; chrome visibility never moves the media surface.
        bin.connect_deep_element_added(|_, _, element| {
            if element
                .factory()
                .is_some_and(|factory| factory.name() == "textoverlay")
                && element.find_property("ypad").is_some()
            {
                element.set_property("ypad", 156_i32);
            }
        });
        let header = chrome(&top, gtk::Align::Start);
        stage.add_overlay(&header);
        stage.set_measure_overlay(&header, false);
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 4);
        bottom.add_css_class("player-footer");
        let status = crate::home::label("Preparing playback…", "muted");
        status.set_wrap(true);
        status.set_max_width_chars(70);
        status.set_justify(gtk::Justification::Center);
        let feedback = gtk::Box::new(gtk::Orientation::Vertical, 12);
        feedback.add_css_class("player-feedback");
        feedback.set_halign(gtk::Align::Center);
        feedback.set_valign(gtk::Align::Center);
        feedback.set_can_target(false);
        let spinner = gtk::Spinner::new();
        spinner.set_spinning(true);
        feedback.append(&spinner);
        feedback.append(&status);
        stage.add_overlay(&feedback);
        stage.set_measure_overlay(&feedback, false);
        let time = crate::home::label("0:00", "muted");
        time.set_halign(gtk::Align::End);
        bottom.append(&time);
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        seek.set_draw_value(false);
        crate::home::accessible_label(&seek, "Playback position");
        bottom.append(&seek);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let play = icon_button("media-playback-pause-symbolic", "Pause");
        let backward = icon_button("media-seek-backward-symbolic", "Back 30 seconds");
        let forward = icon_button("media-seek-forward-symbolic", "Forward 30 seconds");
        let mute = gtk::ToggleButton::new();
        mute.set_icon_name("audio-volume-high-symbolic");
        mute.add_css_class("player-icon");
        name_control(&mute, "Mute");
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume.set_width_request(120);
        volume.set_value(1.0);
        volume.set_draw_value(false);
        crate::home::accessible_label(&volume, "Volume");
        controls.append(&play);
        controls.append(&backward);
        controls.append(&forward);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        controls.append(&spacer);
        controls.append(&mute);
        let settings = gtk::MenuButton::builder()
            .icon_name("emblem-system-symbolic")
            .build();
        settings.add_css_class("player-icon");
        name_control(&settings, "Audio, subtitles and playback settings");
        controls.append(&settings);
        bottom.append(&controls);
        let footer = chrome(&bottom, gtk::Align::End);
        stage.add_overlay(&footer);
        stage.set_measure_overlay(&footer, false);
        let menu = gtk::Popover::new();
        menu.add_css_class("player-menu");
        menu.set_position(gtk::PositionType::Top);
        settings.set_popover(Some(&menu));
        let options = gtk::Box::new(gtk::Orientation::Vertical, 8);
        options.append(&crate::home::label("Volume", "muted"));
        options.append(&volume);
        let tracks = crate::home::column(8);
        options.append(&tracks);
        let check = menu_button("Check session state");
        let retry_create = menu_button("Retry exact playback request");
        let retry_update = menu_button("Retry exact transport change");
        let fallback = menu_button("Try compatible transcode once");
        retry_create.set_visible(false);
        retry_update.set_visible(false);
        options.append(&check);
        options.append(&retry_create);
        options.append(&retry_update);
        options.append(&fallback);
        let stop = menu_button("Stop playback");
        options.append(&stop);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(360)
            .max_content_width(300)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .child(&options)
            .build();
        menu.set_child(Some(&scroll));
        window.set_content(Some(&stage));
        let (commands, receiver) = tokio::sync::mpsc::channel(16);
        let (sender, mut replies) = tokio::sync::mpsc::channel(16);
        crate::backend::runtime().spawn(control(
            client,
            core::CreatePlaybackSessionRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                item_id,
                scope: Some(scope),
                client_capabilities: Some(caps),
                start_policy: if beginning {
                    core::PlaybackStartPolicy::Beginning
                } else {
                    core::PlaybackStartPolicy::Resume
                },
                position_ms: None,
            },
            receiver,
            sender,
        ));
        let player = Rc::new(Self {
            window,
            pipeline,
            status,
            time,
            seek,
            tracks,
            retry_create,
            retry_update,
            fallback,
            play,
            stage,
            header,
            footer,
            menu,
            spinner,
            feedback,
            fullscreen: fullscreen.clone(),
            last_activity: Cell::new(Instant::now()),
            motion_activity: MotionActivity::default(),
            accessible_chrome: Cell::new(None),
            keyboard_focus: Cell::new(false),
            scrubbing: Cell::new(false),
            presentation_timer: RefCell::default(),
            tick_timer: RefCell::default(),
            commands,
            session: RefCell::default(),
            timeline: RefCell::default(),
            revision: Cell::new(0),
            attached: Cell::new(false),
            busy: Cell::new(false),
            denied: Cell::new(true),
            allow_update: Cell::new(false),
            closed: Cell::new(false),
            started: Cell::new(false),
            paused: Cell::new(false),
            seeking: Cell::new(false),
            preroll: Cell::new(false),
            initial_seek: Cell::new(false),
            fallback_used: Cell::new(false),
            was_active: Cell::new(false),
            ticks: Cell::new(0),
            watch: RefCell::default(),
            on_close: Box::new(on_close),
        });
        player.refresh_accessible_chrome();
        for revealer in [&player.header, &player.footer] {
            let weak = Rc::downgrade(&player);
            revealer.connect_reveal_child_notify(move |_| {
                if let Some(player) = weak.upgrade() {
                    player.refresh_accessible_chrome();
                }
            });
            let weak = Rc::downgrade(&player);
            revealer.connect_child_revealed_notify(move |_| {
                if let Some(player) = weak.upgrade() {
                    player.refresh_accessible_chrome();
                }
            });
        }
        let weak = Rc::downgrade(&player);
        let application_hold = parent.as_ref().application().map(|app| app.hold());
        gtk::glib::MainContext::default().spawn_local(async move {
            let _application_hold = application_hold;
            while let Some(reply) = replies.recv().await {
                let Some(player) = weak.upgrade() else {
                    continue;
                };
                if reply.finished {
                    (player.on_close)(reply.message);
                } else if !player.closed.get() {
                    let _diagnostic = diagnostic_span("reply.accept.begin", "reply.accept.end");
                    player.accept(reply);
                }
            }
        });
        let weak = Rc::downgrade(&player);
        close.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.close("player_exit");
            }
        });
        let weak = Rc::downgrade(&player);
        stop.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.close("player_exit");
            }
        });
        let weak = Rc::downgrade(&player);
        player.window.connect_close_request(move |_| {
            if let Some(p) = weak.upgrade() {
                p.close("player_exit");
            }
            gtk::glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&player);
        player.window.connect_is_active_notify(move |window| {
            if let Some(p) = weak.upgrade() {
                if window.is_active() {
                    p.was_active.set(true);
                } else if p.was_active.get() {
                    // Let popup focus settle, but never exempt an open menu from background stop.
                    let weak = Rc::downgrade(&p);
                    gtk::glib::idle_add_local_once(move || {
                        if let Some(p) = weak.upgrade() {
                            if !p.window.is_active() && !p.closed.get() {
                                p.close("background");
                            }
                        }
                    });
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&player);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(p) = weak.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            p.keyboard_focus.set(true);
            idle_receipt(IdleRevealOrigin::Key, None, p.last_activity.get().elapsed());
            p.reveal();
            if key == gtk::gdk::Key::Escape {
                use screenroom_linux::player_presentation::{Escape, escape};
                match escape(p.menu.is_visible(), p.window.is_fullscreen()) {
                    Escape::Menu => p.menu.popdown(),
                    Escape::Fullscreen => p.window.unfullscreen(),
                    Escape::Exit => p.close("player_exit"),
                }
                return gtk::glib::Propagation::Stop;
            }
            if p.menu.is_visible() {
                return gtk::glib::Propagation::Proceed;
            }
            if key == gtk::gdk::Key::f || key == gtk::gdk::Key::F {
                p.fullscreen.emit_clicked();
                return gtk::glib::Propagation::Stop;
            }
            let stage_focus = gtk::prelude::GtkWindowExt::focus(&p.window)
                .is_none_or(|widget| widget == p.stage.clone().upcast::<gtk::Widget>());
            if stage_focus {
                match key {
                    gtk::gdk::Key::space => p.play.emit_clicked(),
                    gtk::gdk::Key::Left => p.seek_relative(false),
                    gtk::gdk::Key::Right => p.seek_relative(true),
                    _ => return gtk::glib::Propagation::Proceed,
                }
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        player.window.add_controller(keys);
        let weak = Rc::downgrade(&player);
        fullscreen.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                if p.window.is_fullscreen() {
                    p.window.unfullscreen();
                } else {
                    p.window.fullscreen();
                }
            }
        });
        let weak = Rc::downgrade(&player);
        player.window.connect_fullscreened_notify(move |_| {
            if let Some(p) = weak.upgrade() {
                p.refresh_presentation();
            }
        });
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&player);
        motion.connect_motion(move |_, x, y| {
            if let Some(p) = weak.upgrade() {
                idle_receipt(
                    IdleRevealOrigin::Motion,
                    Some((x, y)),
                    p.last_activity.get().elapsed(),
                );
                if p.motion_activity.moved(x, y) {
                    p.reveal();
                }
            }
        });
        let weak = Rc::downgrade(&player);
        motion.connect_enter(move |_, x, y| {
            if let Some(p) = weak.upgrade() {
                idle_receipt(
                    IdleRevealOrigin::Motion,
                    Some((x, y)),
                    p.last_activity.get().elapsed(),
                );
                if p.motion_activity.moved(x, y) {
                    p.reveal();
                }
            }
        });
        let weak = Rc::downgrade(&player);
        motion.connect_leave(move |_| {
            if let Some(p) = weak.upgrade() {
                p.motion_activity.left();
            }
        });
        player.stage.add_controller(motion);
        let pointer = gtk::GestureClick::new();
        pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&player);
        pointer.connect_pressed(move |_, _, _, _| {
            if let Some(p) = weak.upgrade() {
                p.keyboard_focus.set(false);
                idle_receipt(
                    IdleRevealOrigin::StagePress,
                    None,
                    p.last_activity.get().elapsed(),
                );
                p.reveal();
            }
        });
        player.stage.add_controller(pointer);
        let scrub = gtk::EventControllerLegacy::new();
        scrub.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&player);
        scrub.connect_event(move |_, event| {
            if let Some(p) = weak.upgrade() {
                match event.event_type() {
                    gtk::gdk::EventType::ButtonPress | gtk::gdk::EventType::TouchBegin => {
                        p.scrubbing.set(true);
                        idle_receipt(
                            IdleRevealOrigin::ScrubPress,
                            None,
                            p.last_activity.get().elapsed(),
                        );
                        p.reveal();
                    }
                    gtk::gdk::EventType::ButtonRelease
                    | gtk::gdk::EventType::TouchEnd
                    | gtk::gdk::EventType::TouchCancel => {
                        p.scrubbing.set(false);
                        idle_receipt(
                            IdleRevealOrigin::ScrubRelease,
                            None,
                            p.last_activity.get().elapsed(),
                        );
                        p.reveal();
                    }
                    _ => {}
                }
            }
            gtk::glib::Propagation::Proceed
        });
        player.seek.add_controller(scrub);
        let weak = Rc::downgrade(&player);
        player.menu.connect_closed(move |_| {
            if let Some(p) = weak.upgrade() {
                idle_receipt(
                    IdleRevealOrigin::MenuClosed,
                    None,
                    p.last_activity.get().elapsed(),
                );
                p.reveal();
            }
        });
        let weak = Rc::downgrade(&player);
        backward.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.seek_relative(false);
            }
        });
        let weak = Rc::downgrade(&player);
        forward.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.seek_relative(true);
            }
        });
        let weak = Rc::downgrade(&player);
        player.play.connect_clicked(move |_| {
            let Some(p) = weak.upgrade() else { return };
            if p.denied.get() || p.busy.get() || p.preroll.get() {
                return;
            }
            p.observe();
            let paused = !p.paused.get();
            if p.pipeline
                .set_state(if paused {
                    gst::State::Paused
                } else {
                    gst::State::Playing
                })
                .is_err()
            {
                p.media_error();
                return;
            }
            p.paused.set(paused);
            p.refresh_presentation();
            p.send(Command::Report(
                if paused {
                    Event::Paused
                } else {
                    Event::Resumed
                },
                p.observation(),
            ));
        });
        let weak = Rc::downgrade(&player);
        mute.connect_toggled(move |button| {
            if let Some(p) = weak.upgrade() {
                p.pipeline.set_property("mute", button.is_active());
                button.set_icon_name(if button.is_active() {
                    "audio-volume-muted-symbolic"
                } else {
                    "audio-volume-high-symbolic"
                });
                name_control(button, if button.is_active() { "Unmute" } else { "Mute" });
            }
        });
        let weak = Rc::downgrade(&player);
        volume.connect_value_changed(move |scale| {
            if let Some(p) = weak.upgrade() {
                p.pipeline.set_property("volume", scale.value());
            }
        });
        let weak = Rc::downgrade(&player);
        player.seek.connect_change_value(move |_, _, value| {
            if let Some(p) = weak.upgrade() {
                p.seek_to(value);
            }
            gtk::glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&player);
        check.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.send(Command::Check);
            }
        });
        let weak = Rc::downgrade(&player);
        player.retry_create.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.send(Command::Create);
            }
        });
        let weak = Rc::downgrade(&player);
        player.retry_update.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.send(Command::RetryUpdate);
            }
        });
        let weak = Rc::downgrade(&player);
        player.fallback.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.update(core::PlaybackTrackChoices::default(), true);
            }
        });
        let weak = Rc::downgrade(&player);
        let guard = player
            .pipeline
            .bus()
            .ok_or("GStreamer bus is unavailable.")?
            .add_watch_local(move |_, message| {
                let Some(p) = weak.upgrade() else {
                    return gtk::glib::ControlFlow::Break;
                };
                p.bus(message);
                if p.closed.get() {
                    gtk::glib::ControlFlow::Break
                } else {
                    gtk::glib::ControlFlow::Continue
                }
            })
            .map_err(|_| "GStreamer bus could not be attached.")?;
        player.watch.replace(Some(guard));
        let weak = Rc::downgrade(&player);
        let timer = gtk::glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(p) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if p.closed.get() {
                return gtk::glib::ControlFlow::Break;
            }
            p.tick();
            gtk::glib::ControlFlow::Continue
        });
        player.tick_timer.replace(Some(timer));
        let weak = Rc::downgrade(&player);
        let timer = gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(p) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if p.closed.get() {
                return gtk::glib::ControlFlow::Break;
            }
            if let Some(state) = diagnostics() {
                use std::sync::atomic::Ordering::Relaxed;
                if let Ok(previous) = state
                    .heartbeats
                    .fetch_update(Relaxed, Relaxed, |value| (value < 300).then(|| value + 1))
                {
                    if previous % 5 == 0 {
                        diagnostic_event("gtk.heartbeat");
                    }
                }
            }
            p.refresh_presentation();
            gtk::glib::ControlFlow::Continue
        });
        player.presentation_timer.replace(Some(timer));
        player.send(Command::Create);
        player.window.present();
        player.stage.grab_focus();
        Ok(player)
    }

    fn refresh_accessible_chrome(&self) {
        if self.closed.get() {
            return;
        }
        let description = chrome_description([
            self.header.reveals_child(),
            self.header.is_child_revealed(),
            self.footer.reveals_child(),
            self.footer.is_child_revealed(),
        ]);
        if self.accessible_chrome.replace(Some(description)) != Some(description) {
            self.window
                .upcast_ref::<gtk::Widget>()
                .update_property(&[gtk::accessible::Property::Description(description)]);
        }
    }

    fn reveal(&self) {
        if self.closed.get() {
            return;
        }
        self.last_activity.set(Instant::now());
        self.refresh_presentation();
    }

    fn refresh_presentation(&self) {
        if self.closed.get() {
            return;
        }
        use screenroom_linux::player_presentation::Visibility;
        let focus_in_controls =
            gtk::prelude::GtkWindowExt::focus(&self.window).is_some_and(|focus| {
                focus.is_ancestor(&self.header) || focus.is_ancestor(&self.footer)
            });
        let visible = Visibility {
            paused: self.paused.get(),
            busy: self.busy.get(),
            blocked: self.denied.get(),
            preparing: self.preroll.get() || !self.started.get(),
            scrubbing: self.scrubbing.get(),
            menu: self.menu.is_visible(),
            keyboard_focus: self.keyboard_focus.get() && focus_in_controls,
        }
        .show(self.last_activity.get().elapsed());
        idle_decision(
            self.last_activity.get().elapsed(),
            [
                self.paused.get(),
                self.busy.get(),
                self.denied.get(),
                self.preroll.get(),
                self.started.get(),
                self.menu.is_visible(),
                self.menu.is_mapped(),
                focus_in_controls,
                self.keyboard_focus.get(),
                self.scrubbing.get(),
                visible,
                self.preroll.get() || !self.started.get(),
            ],
            || {
                [
                    self.header.reveals_child(),
                    self.header.is_child_revealed(),
                    self.footer.reveals_child(),
                    self.footer.is_child_revealed(),
                ]
            },
        );
        for chrome in [&self.header, &self.footer] {
            chrome.set_reveal_child(visible);
            chrome.set_can_target(visible);
            chrome.set_sensitive(visible);
        }
        if !visible && focus_in_controls {
            self.stage.grab_focus();
        }
        let preparing = !self.denied.get() && (self.preroll.get() || !self.started.get());
        self.spinner.set_spinning(preparing);
        self.spinner.set_visible(preparing);
        self.status
            .set_visible(preparing || self.denied.get() || !self.attached.get());
        self.feedback
            .set_visible(preparing || self.denied.get() || !self.attached.get());
        self.play.set_icon_name(if self.paused.get() {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        });
        name_control(&self.play, if self.paused.get() { "Play" } else { "Pause" });
        let fullscreen = self.window.is_fullscreen();
        self.fullscreen.set_icon_name(if fullscreen {
            "view-restore-symbolic"
        } else {
            "view-fullscreen-symbolic"
        });
        name_control(
            &self.fullscreen,
            if fullscreen {
                "Exit fullscreen"
            } else {
                "Enter fullscreen"
            },
        );
    }

    fn seek_relative(&self, forward: bool) {
        let timeline = self.timeline.borrow();
        let target = if forward {
            timeline
                .position
                .saturating_add(30_000)
                .min(timeline.duration.unwrap_or(u64::MAX))
        } else {
            timeline.position.saturating_sub(30_000)
        };
        drop(timeline);
        self.seek_to(target as f64);
    }

    fn observation(&self) -> Observation {
        Observation {
            position: self.timeline.borrow().position,
            paused: self.paused.get(),
            muted: self.pipeline.property("mute"),
            volume: self.pipeline.property::<f64>("volume").clamp(0.0, 1.0),
        }
    }

    fn send(&self, command: Command) {
        if self.closed.get() || self.busy.replace(true) {
            return;
        }
        if self.commands.try_send(command).is_err() {
            self.busy.set(false);
            self.denied.set(true);
            self.detach();
            self.status
                .set_text("Playback control is unavailable. Close the player.");
        }
    }

    fn observe(&self) {
        if self.preroll.get() || self.denied.get() {
            return;
        }
        let position = {
            let _diagnostic = diagnostic_span("query.position.begin", "query.position.end");
            self.pipeline.query_position::<gst::ClockTime>()
        };
        if let Some(position) = position {
            if self
                .timeline
                .borrow_mut()
                .observe(position.mseconds())
                .is_none()
            {
                self.denied.set(true);
                self.detach();
                self.status
                    .set_text("Playback position exceeds the supported timeline.");
            }
        }
    }

    fn detach(&self) {
        let _diagnostic = diagnostic_span("detach.begin", "detach.end");
        {
            let _diagnostic = diagnostic_span("detach.null.begin", "detach.null.end");
            let _ = self.pipeline.set_state(gst::State::Null);
        }
        self.pipeline.set_property("uri", Option::<&str>::None);
        self.pipeline.set_property("suburi", Option::<&str>::None);
        self.preroll.set(false);
        self.attached.set(false);
    }

    fn accept(self: &Rc<Self>, reply: Reply) {
        self.observe();
        self.busy.set(false);
        self.status.set_text(reply.message);
        self.retry_create.set_visible(reply.retry_create);
        self.retry_update.set_visible(reply.retry_update);
        self.denied.set(reply.blocked);
        self.allow_update.set(reply.can_update);
        if reply.blocked {
            self.detach();
        }
        let Some(session) = reply.session else { return };
        if self.session.borrow().as_ref().is_some_and(|current| {
            current.session_id != session.session_id
                || current.transport_revision > session.transport_revision
        }) {
            self.media_error();
            return;
        }
        let changed = session
            .transport
            .as_ref()
            .is_some_and(|t| t.revision != self.revision.get() || !self.attached.get());
        if changed && !reply.blocked {
            self.observe();
            self.detach();
            if self.attach(&session).is_err() {
                self.media_error();
            }
        } else if session.transport.is_none() {
            self.detach();
            self.status
                .set_text("Playback is not ready. Check session state; no automatic restart.");
        }
        self.render_tracks(&session);
        self.session.replace(Some(session));
    }

    fn attach(&self, session: &core::PlaybackSession) -> Result<(), ()> {
        let _diagnostic = diagnostic_span("attach.begin", "attach.end");
        let transport = session.transport.as_ref().ok_or(())?;
        let saved =
            (self.revision.get() == transport.revision).then(|| self.timeline.borrow().position);
        let timeline = Timeline::from_transport(transport, saved).ok_or(())?;
        if media_position(timeline.position, timeline.offset)
            .is_none_or(|ms| ms > gst::ClockTime::MAX.mseconds())
        {
            return Err(());
        }
        let caps = capabilities();
        if transport.mime_type != "video/mp4" && !caps.native_hls {
            return Err(());
        }
        self.timeline.replace(timeline);
        self.revision.set(transport.revision);
        self.attached.set(true);
        self.initial_seek.set(false);
        self.preroll.set(true);
        {
            let _diagnostic = diagnostic_span("attach.uri.begin", "attach.uri.end");
            self.pipeline.set_property("uri", &transport.media_url);
        }
        let subtitle = session
            .tracks
            .iter()
            .find(|t| t.selected && t.available && t.kind == core::PlaybackTrackKind::Subtitle);
        self.pipeline
            .set_property_from_str("flags", "video+audio+soft-colorbalance");
        if let Some(track) = subtitle {
            if track.subtitle_mode == Some(core::PlaybackSubtitleMode::Webvtt) {
                if !caps.webvtt {
                    return Err(());
                }
                self.pipeline
                    .set_property_from_str("flags", "video+audio+text+soft-colorbalance");
                self.pipeline
                    .set_property("suburi", track.media_url.as_deref());
            }
        }
        let _diagnostic = diagnostic_span("attach.paused.begin", "attach.paused.end");
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|_| ())?;
        Ok(())
    }

    fn bus(&self, message: &gst::MessageRef) {
        if self.closed.get() || self.denied.get() {
            return;
        }
        match message.view() {
            gst::MessageView::Error(_) => diagnostic_event("bus.error"),
            gst::MessageView::Eos(_) => diagnostic_event("bus.eos"),
            gst::MessageView::AsyncDone(_) => diagnostic_event("bus.async-done"),
            gst::MessageView::StateChanged(_)
                if message
                    .src()
                    .is_some_and(|src| src == self.pipeline.upcast_ref::<gst::Object>()) =>
            {
                diagnostic_event("bus.state-changed")
            }
            _ => {}
        }
        match message.view() {
            gst::MessageView::Error(_) => self.media_error(),
            gst::MessageView::Eos(_) => self.close("ended"),
            gst::MessageView::AsyncDone(_) if self.preroll.get() => {
                if !self.initial_seek.replace(true) {
                    let timeline = self.timeline.borrow();
                    let position = media_position(timeline.position, timeline.offset);
                    drop(timeline);
                    let Some(position) = position else {
                        self.media_error();
                        return;
                    };
                    if position > 0 {
                        if self
                            .pipeline
                            .seek_simple(
                                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                gst::ClockTime::from_mseconds(position),
                            )
                            .is_err()
                        {
                            self.media_error();
                        }
                        return;
                    }
                }
                self.preroll.set(false);
                if {
                    let _diagnostic = diagnostic_span("bus.playing.begin", "bus.playing.end");
                    self.pipeline
                        .set_state(if self.paused.get() {
                            gst::State::Paused
                        } else {
                            gst::State::Playing
                        })
                        .is_err()
                } {
                    self.media_error();
                    return;
                }
                if self.seeking.replace(false) && self.started.get() {
                    self.send(Command::Report(Event::Seeked, self.observation()));
                }
            }
            gst::MessageView::AsyncDone(_) if self.seeking.replace(false) => {
                self.observe();
                self.send(Command::Report(Event::Seeked, self.observation()));
            }
            gst::MessageView::StateChanged(state)
                if message
                    .src()
                    .is_some_and(|src| src == self.pipeline.upcast_ref::<gst::Object>())
                    && state.current() == gst::State::Playing
                    && !self.preroll.get()
                    && !self.started.replace(true) =>
            {
                self.send(Command::Report(Event::Started, self.observation()));
            }
            _ => {}
        }
    }

    fn tick(&self) {
        self.observe();
        let ticks = self.ticks.get() + 1;
        self.ticks.set(ticks);
        let timeline = self.timeline.borrow();
        let duration = timeline.duration.or_else(|| {
            let _diagnostic = diagnostic_span("query.duration.begin", "query.duration.end");
            self.pipeline
                .query_duration::<gst::ClockTime>()
                .and_then(|d| {
                    crate::playback_state::logical_position(d.mseconds(), timeline.offset)
                })
        });
        self.time.set_text(&format!(
            "{} / {}",
            clock(timeline.position),
            duration.map(clock).unwrap_or_else(|| "Unknown".into())
        ));
        self.seek.set_range(
            0.0,
            duration.unwrap_or(timeline.position.max(timeline.offset + 1)) as f64,
        );
        self.seek.set_value(timeline.position as f64);
        self.seek.set_sensitive(
            !self.denied.get() && !self.busy.get() && !self.preroll.get() && duration.is_some(),
        );
        drop(timeline);
        self.play
            .set_sensitive(!self.denied.get() && !self.busy.get() && !self.preroll.get());
        self.fallback.set_sensitive(
            self.allow_update.get()
                && !self.busy.get()
                && !self.retry_update.is_visible()
                && !self.fallback_used.get()
                && self.session.borrow().as_ref().is_some_and(|s| !blocked(s)),
        );
        if !self.busy.get() && !self.denied.get() && self.started.get() && !self.preroll.get() {
            if ticks % 15 == 0 {
                self.send(Command::Report(Event::Heartbeat, self.observation()));
            } else if ticks % 10 == 0 && !self.paused.get() {
                self.send(Command::Report(Event::Progress, self.observation()));
            }
        }
    }

    fn seek_to(&self, value: f64) {
        if self.denied.get()
            || self.busy.get()
            || self.preroll.get()
            || self.seeking.get()
            || !value.is_finite()
            || value < 0.0
        {
            return;
        }
        let timeline = self.timeline.borrow();
        let logical = value.round() as u64;
        let action = timeline.seek(logical);
        drop(timeline);
        let Some(action) = action else { return };
        let media = match action {
            crate::playback_state::SeekAction::Native(media) => media,
            crate::playback_state::SeekAction::Gateway(target) => {
                if !self.allow_update.get() {
                    return;
                }
                self.observe();
                let mut observation = self.observation();
                observation.position = target;
                self.denied.set(true);
                self.detach();
                self.timeline.borrow_mut().position = target;
                self.seeking.set(true);
                self.allow_update.set(false);
                self.send(Command::Update(None, false, observation));
                return;
            }
        };
        if media > gst::ClockTime::MAX.mseconds() {
            return;
        }
        self.seeking.set(true);
        if self
            .pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::from_mseconds(media),
            )
            .is_err()
        {
            self.media_error();
        }
    }

    fn render_tracks(self: &Rc<Self>, session: &core::PlaybackSession) {
        crate::home::clear(&self.tracks);
        for (kind, title) in [
            (core::PlaybackTrackKind::Audio, "Audio"),
            (core::PlaybackTrackKind::Subtitle, "Subtitles"),
        ] {
            self.tracks.append(&crate::home::label(title, "muted"));
            if kind == core::PlaybackTrackKind::Subtitle {
                let selected = !session
                    .tracks
                    .iter()
                    .any(|track| track.kind == core::PlaybackTrackKind::Subtitle && track.selected);
                let off = menu_button(if selected { "Off · selected" } else { "Off" });
                off.set_sensitive(!self.denied.get());
                let weak = Rc::downgrade(self);
                off.connect_clicked(move |_| {
                    if let Some(p) = weak.upgrade() {
                        p.update(
                            core::PlaybackTrackChoices {
                                subtitles_off: true,
                                ..Default::default()
                            },
                            false,
                        );
                    }
                });
                self.tracks.append(&off);
            }
            for track in session.tracks.iter().filter(|t| t.kind == kind) {
                let mode = match track.subtitle_mode {
                    Some(core::PlaybackSubtitleMode::BurnIn) => " · burn-in",
                    Some(core::PlaybackSubtitleMode::Webvtt) => " · WebVTT",
                    _ => "",
                };
                let button = menu_button(&format!(
                    "{}{}{}",
                    track.display_name,
                    mode,
                    if track.selected { " · selected" } else { "" }
                ));
                button.set_sensitive(
                    track.available && !track.selected && !blocked(session) && !self.denied.get(),
                );
                let id = track.id.clone();
                let audio = kind == core::PlaybackTrackKind::Audio;
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(p) = weak.upgrade() {
                        p.update(
                            core::PlaybackTrackChoices {
                                audio_track_id: audio.then(|| id.clone()),
                                subtitle_track_id: (!audio).then(|| id.clone()),
                                subtitles_off: false,
                            },
                            false,
                        );
                    }
                });
                self.tracks.append(&button);
            }
        }
        let info = gtk::Expander::builder()
            .label("Session information")
            .build();
        let method = session
            .transport
            .as_ref()
            .map(|t| match t.method {
                core::PlaybackMethod::DirectPlay => "Direct play",
                core::PlaybackMethod::DirectStream => "Direct stream",
                core::PlaybackMethod::Transcode => "Transcode",
                _ => "Not negotiated",
            })
            .unwrap_or("Not ready");
        info.set_child(Some(&crate::home::label(
            &format!(
                "{method}\nTransport revision: {}\nPersonal report: {:?}\nCleanup pending: {}",
                session.transport_revision, session.report_outcome, session.cleanup_pending
            ),
            "muted",
        )));
        self.tracks.append(&info);
    }

    fn update(&self, choices: core::PlaybackTrackChoices, fallback: bool) {
        if self.busy.get()
            || !self.allow_update.get()
            || self.closed.get()
            || self.session.borrow().as_ref().is_none_or(blocked)
            || (fallback && self.fallback_used.get())
        {
            return;
        }
        self.observe();
        let observation = self.observation();
        self.detach();
        if fallback {
            self.fallback_used.set(true);
        }
        self.allow_update.set(false);
        self.send(Command::Update(
            (!fallback).then_some(choices),
            fallback,
            observation,
        ));
    }

    fn media_error(&self) {
        self.observe();
        self.denied.set(true);
        self.detach();
        self.status.set_text("Native playback failed. A decoder, HLS or subtitle plugin may be missing. Check the session or try compatible transcode once.");
    }

    pub fn close(&self, reason: &str) {
        if self.closed.replace(true) {
            return;
        }
        if let Some(timer) = self.presentation_timer.borrow_mut().take() {
            timer.remove();
        }
        if let Some(timer) = self.tick_timer.borrow_mut().take() {
            timer.remove();
        }
        self.menu.popdown();
        self.observe();
        self.timeline.borrow_mut().freeze();
        let observation = self.observation();
        self.detach();
        self.session.borrow_mut().take();
        let commands = self.commands.clone();
        let reason = reason.to_owned();
        crate::backend::runtime().spawn(async move {
            let _ = commands.send(Command::Stop(observation, reason)).await;
        });
        self.window.set_visible(false);
        (self.on_close)("Playback stopped locally. Final personal report pending.");
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.close("player_disposed");
        self.watch.borrow_mut().take();
        self.window.destroy();
    }
}
