use screenroom_client_core::{Client, ClientConfig, Item, MediaLibrary, Page, SourceLibraryScope};
use std::{collections::HashSet, error::Error, fs, sync::Arc, time::Duration};
use waydriver::{CompositorRuntime, Session, SessionConfig};
use waydriver_capture_mutter::MutterCapture;
use waydriver_compositor_mutter::MutterCompositor;
use waydriver_input_mutter::MutterInput;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
const FLOWS: [&str; 5] = ["home", "sidebar", "library", "search", "detail"];
#[path = "e2e/playback.rs"]
mod playback;

fn client() -> Result<Arc<Client>> {
    Ok(Client::new(ClientConfig {
        endpoint: std::env::var("SCREENROOM_API_ENDPOINT")?,
        user_id: std::env::var("SCREENROOM_USER_ID")?,
        timeout_ms: 5000,
    })?)
}

async fn selection() -> Result<(MediaLibrary, Item)> {
    use screenroom_client_core::{SearchMethods, SearchRequest, SearchTypes};
    let client = client()?;
    let libraries = client
        .clone()
        .list_libraries(
            String::new(),
            Page {
                size: 8,
                token: String::new(),
            },
        )
        .await?;
    for library in libraries.libraries.into_iter().take(8) {
        let page = client
            .clone()
            .list_library_items(
                SourceLibraryScope {
                    source_id: String::new(),
                    library_id: library.id.clone(),
                },
                Page {
                    size: 100,
                    token: String::new(),
                },
            )
            .await?;
        for item in page
            .items
            .iter()
            .filter(|item| {
                item.kind == "Movie" && item.artwork.is_some() && !item.title.trim().is_empty()
            })
            .take(3)
            .cloned()
        {
            let results = client
                .clone()
                .search(SearchRequest {
                    query: item.title.clone(),
                    library_id: library.id.clone(),
                    types: Some(SearchTypes {
                        movie: true,
                        series: true,
                        episode: false,
                    }),
                    methods: Some(SearchMethods {
                        title_only: true,
                        text: true,
                        fuzzy: true,
                        semantic: false,
                    }),
                    source: String::new(),
                    source_id: String::new(),
                    page: Some(Page {
                        size: 100,
                        token: String::new(),
                    }),
                })
                .await?;
            if page
                .items
                .iter()
                .filter(|candidate| candidate.title == item.title)
                .count()
                != 1
            {
                continue;
            }
            if results
                .items
                .first()
                .is_some_and(|result| result.id == item.id)
                && results
                    .items
                    .iter()
                    .filter(|candidate| candidate.title == item.title)
                    .count()
                    == 1
            {
                return Ok((library, item));
            }
        }
    }
    Err("Showcase requires an artwork-backed movie on a first library page that ranks first in scoped title search; checked at most eight libraries and three candidates each".into())
}

fn xpath_literal(value: &str) -> String {
    if value.contains('\'') {
        format!("concat('{}', '')", value.replace('\'', "', \"'\", '"))
    } else {
        format!("'{value}'")
    }
}

async fn visible(session: &Arc<Session>, name: &str) -> Result {
    let value = xpath_literal(name);
    session
        .locate(&format!("//*[@name={value} or @description={value}]"))
        .first()
        .wait_for_visible()
        .await?;
    Ok(())
}

async fn click(session: &Arc<Session>, name: &str) -> Result {
    let value = xpath_literal(name);
    activate_settled(session, &format!("//*[@role='button' or @role='toggle button'][@name={value} or @description={value} or .//Label[@name={value}]]"), 0).await
}

async fn click_library(session: &Arc<Session>, library: &MediaLibrary) -> Result {
    let page = client()?
        .list_libraries(
            String::new(),
            Page {
                size: 8,
                token: String::new(),
            },
        )
        .await?;
    let index = page
        .libraries
        .iter()
        .position(|entry| entry.id == library.id)
        .ok_or("Selected library left the bounded discovery page")?;
    let occurrence = page.libraries[..index]
        .iter()
        .filter(|entry| entry.title == library.title)
        .count();
    if playback_input() {
        let count = page
            .libraries
            .iter()
            .filter(|entry| entry.title == library.title)
            .count();
        return pointer_settled(
            session,
            &format!("//Button[@name={}]", xpath_literal(&library.title)),
            occurrence,
            count,
        )
        .await;
    }
    activate_settled(
        session,
        &format!("//Button[@name={}]", xpath_literal(&library.title)),
        occurrence,
    )
    .await
}

async fn activate_settled(session: &Arc<Session>, selector: &str, occurrence: usize) -> Result {
    if playback_input() {
        return pointer_settled(session, selector, occurrence, 1).await;
    }
    let locator = session.locate(selector);
    tokio::time::timeout(session.default_timeout(), async {
        let mut previous = None;
        loop {
            let buttons = locator.inspect_all().await?;
            if let Some(button) = buttons.get(occurrence) {
                let reference = format!("{}|{}", button.ref_.0, button.ref_.1);
                let tree = session.dump_tree().await?;
                let bounds = tree
                    .lines()
                    .find(|line| {
                        snapshot_attribute(line, "_ref") == Some(reference.as_str())
                            && snapshot_attribute(line, "showing") == Some("true")
                    })
                    .and_then(|line| snapshot_attribute(line, "bbox"))
                    .map(str::to_owned);
                if let Some(bounds) = bounds {
                    let current = (reference, bounds);
                    if previous.as_ref() == Some(&current) {
                        session.activate_ref(&button.ref_.0, &button.ref_.1).await?;
                        return Ok::<(), Box<dyn Error>>(());
                    }
                    previous = Some(current);
                } else {
                    previous = None;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    Ok(())
}

fn playback_input() -> bool {
    std::env::var("SCREENROOM_E2E_JOURNEY").as_deref() == Ok("playback")
}

fn missing_pointer_bounds(error: &waydriver::Error, bus: &str, path: &str) -> bool {
    matches!(error, waydriver::Error::Atspi { message, source: None }
        if message == &format!("click_ref: {bus}{path} has no Component bounds — element may not be laid out"))
}

// Only click_ref's pre-pointer bounds failure permits another observation.
async fn pointer_policy<T, Observe, ObserveFuture, Click, ClickFuture>(
    timeout: Duration,
    mut observe: Observe,
    mut click: Click,
) -> Result
where
    T: PartialEq,
    Observe: FnMut() -> ObserveFuture,
    ObserveFuture: std::future::Future<Output = Result<Option<T>>>,
    Click: FnMut(T) -> ClickFuture,
    ClickFuture: std::future::Future<Output = Result<bool>>,
{
    tokio::time::timeout(timeout, async {
        let mut previous = None;
        loop {
            match observe().await? {
                Some(current) if previous.as_ref() == Some(&current) => {
                    if click(current).await? {
                        return Ok::<(), Box<dyn Error>>(());
                    }
                    previous = None;
                }
                current => previous = current,
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    Ok(())
}

async fn pointer_settled(
    session: &Arc<Session>,
    selector: &str,
    occurrence: usize,
    expected_count: usize,
) -> Result {
    let locator = session.locate(selector);
    pointer_policy(
        session.default_timeout(),
        || async {
            let matches = locator.inspect_all().await?;
            if matches.len() > expected_count {
                return Err("Pointer selector matched additional UI controls".into());
            }
            if matches.len() != expected_count {
                return Ok(None);
            }
            let button = matches
                .get(occurrence)
                .ok_or("Invalid pointer selector occurrence")?;
            let reference = format!("{}|{}", button.ref_.0, button.ref_.1);
            let tree = session.dump_tree().await?;
            let bounds = tree
                .lines()
                .find_map(|line| pointer_snapshot_bounds(line, &reference));
            Ok(bounds.map(|bounds| {
                (
                    button.ref_.0.clone(),
                    button.ref_.1.clone(),
                    bounds.to_owned(),
                )
            }))
        },
        |(bus, path, _bounds)| async move {
            // Public click_ref fetches live Component bounds, then uses only pointer input.
            match session.click_ref(&bus, &path).await {
                Ok(()) => Ok(true),
                Err(error) if missing_pointer_bounds(&error, &bus, &path) => Ok(false),
                Err(error) => Err(error.into()),
            }
        },
    )
    .await
}

fn pointer_snapshot_bounds<'a>(line: &'a str, reference: &str) -> Option<&'a str> {
    if snapshot_attribute(line, "_ref") != Some(reference)
        || ["showing", "visible", "sensitive"]
            .iter()
            .any(|name| snapshot_attribute(line, name) != Some("true"))
    {
        return None;
    }
    // GTK exposes its disabled state through AT-SPI Sensitive, not Enabled.
    let bounds = snapshot_attribute(line, "bbox")?;
    let mut values = bounds.split(',').map(str::parse::<i32>);
    let (x, y, width, height) = (
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
    );
    if values.next().is_some() || x < 0 || y < 0 || width <= 0 || height <= 0 {
        return None;
    }
    x.checked_add(width)?;
    y.checked_add(height)?;
    Some(bounds)
}

#[test]
fn pointer_snapshot_eligibility_contract() {
    // Actual first focused failure, line 11; only the opaque reference is sanitized.
    let line = r#"<Button role="button" name="Expand sidebar" showing="true" visible="true" sensitive="true" focused="true" focusable="true" bbox="10,16,43,44" keyshortcuts="" toolkit="GTK" _ref=":1.2|/example">"#;
    let reference = ":1.2|/example";
    assert_eq!(
        pointer_snapshot_bounds(line, reference),
        Some("10,16,43,44")
    );
    assert!(!line.contains("enabled="));
    assert_eq!(pointer_snapshot_bounds(line, ":1.2|/stale"), None);
    for name in ["showing", "visible", "sensitive"] {
        assert_eq!(
            pointer_snapshot_bounds(
                &line.replace(&format!("{name}=\"true\""), &format!("{name}=\"false\"")),
                reference
            ),
            None,
        );
        assert_eq!(
            pointer_snapshot_bounds(&line.replace(&format!(" {name}=\"true\""), ""), reference),
            None,
        );
    }
    for bounds in [
        "",
        "10,16,0,44",
        "10,16,43,-1",
        "-1,16,43,44",
        "10,-1,43,44",
        "10,16,43",
        "10,16,43,44,1",
        "10,16,NaN,44",
        "10,16,43.5,44",
        "2147483647,16,43,44",
        "10,2147483647,43,44",
    ] {
        assert_eq!(
            pointer_snapshot_bounds(&line.replace("10,16,43,44", bounds), reference),
            None,
            "{bounds}",
        );
    }
    assert_eq!(
        pointer_snapshot_bounds(&line.replace(" bbox=\"10,16,43,44\"", ""), reference),
        None
    );
    // Enabled must never substitute for GTK's missing or false Sensitive state.
    assert_eq!(
        pointer_snapshot_bounds(
            &line.replace("sensitive=\"true\"", "enabled=\"true\""),
            reference
        ),
        None
    );
    assert_eq!(
        pointer_snapshot_bounds(
            &line.replace("sensitive=\"true\"", "sensitive=\"false\" enabled=\"true\""),
            reference
        ),
        None
    );
}

#[tokio::test]
async fn pointer_policy_contract() -> Result {
    use std::{cell::Cell, collections::VecDeque};
    let bus = ":1.2";
    let path = "/example";
    let absent =
        format!("click_ref: {bus}{path} has no Component bounds — element may not be laid out");
    assert!(missing_pointer_bounds(
        &waydriver::Error::atspi(&absent),
        bus,
        path
    ));
    for error in [
        waydriver::Error::atspi(format!("{absent} extra")),
        waydriver::Error::atspi(&absent.replace(path, "/other")),
        waydriver::Error::atspi("Component.GetExtents: timed out"),
        waydriver::Error::atspi("activate_ref: do_action(0) returned false"),
        waydriver::Error::Timeout(absent.clone()),
        waydriver::Error::atspi_with(&absent, std::io::Error::other("unknown")),
    ] {
        assert!(!missing_pointer_bounds(&error, bus, path));
    }
    let observations = Cell::new(0);
    let attempts = Cell::new(0);
    let inputs = Cell::new(0);
    let mut sequence = VecDeque::from([Some(1), Some(2), Some(2), None, Some(3), Some(3)]);
    pointer_policy(
        Duration::from_secs(1),
        || {
            observations.set(observations.get() + 1);
            std::future::ready(Ok(sequence.pop_front().flatten()))
        },
        |_| {
            attempts.set(attempts.get() + 1);
            // First attempt represents click_ref's exact pre-input missing-bounds result.
            let dispatched = attempts.get() == 2;
            inputs.set(inputs.get() + usize::from(dispatched));
            std::future::ready(Ok(dispatched))
        },
    )
    .await?;
    assert_eq!(
        (observations.get(), attempts.get(), inputs.get()),
        (6, 2, 1)
    );
    let attempts = Cell::new(0);
    let inputs = Cell::new(0);
    let failure = pointer_policy(
        Duration::from_secs(1),
        || std::future::ready(Ok(Some(1))),
        |_| {
            attempts.set(attempts.get() + 1);
            inputs.set(inputs.get() + 1);
            std::future::ready(Err("unknown error after possible input".into()))
        },
    )
    .await
    .unwrap_err();
    assert_eq!(failure.to_string(), "unknown error after possible input");
    assert_eq!((attempts.get(), inputs.get()), (1, 1));
    let attempts = Cell::new(0);
    let start = std::time::Instant::now();
    assert!(
        pointer_policy(
            Duration::from_millis(65),
            || std::future::ready(Ok(Some(1))),
            |_| {
                attempts.set(attempts.get() + 1);
                std::future::ready(Ok(false))
            }
        )
        .await
        .is_err()
    );
    assert!(attempts.get() <= 2);
    assert!(start.elapsed() < Duration::from_secs(1));
    let attempts = Cell::new(0);
    assert!(
        pointer_policy(
            Duration::from_millis(65),
            || std::future::ready(Ok::<Option<usize>, Box<dyn Error>>(None)),
            |_| {
                attempts.set(attempts.get() + 1);
                std::future::ready(Ok(true))
            }
        )
        .await
        .is_err()
    );
    assert_eq!(attempts.get(), 0);
    Ok(())
}

async fn launch(extra_env: Vec<(String, String)>) -> Result<Arc<Session>> {
    let mut compositor = MutterCompositor::new();
    let resolution = std::env::var("E2E_RESOLUTION").unwrap_or_else(|_| "1280x800".into());
    compositor.start(Some(&resolution), None).await?;
    let state = compositor.state().ok_or("Mutter state unavailable")?;
    let mut env = vec![("GDK_BACKEND".into(), "wayland".into())];
    env.extend(extra_env);
    let session = Session::start(
        Box::new(compositor),
        Box::new(MutterInput::new(state.clone())),
        Box::new(MutterCapture::new(state)),
        SessionConfig {
            command: std::env::var("E2E_APP").unwrap_or_else(|_| "screenroom-linux".into()),
            args: vec![],
            cwd: None,
            app_name: "screenroom-linux".into(),
            video_output: None,
            video_bitrate: None,
            video_fps: None,
            prewarm_visual: false,
            visual_region_tuning: Default::default(),
            visual_text_tuning: Default::default(),
            visual_click_tuning: Default::default(),
            gsettings_isolated: true,
            xdg_isolated: true,
            extra_env: env,
            capture_external_effects: false,
        },
    )
    .await?;
    session.set_default_timeout(Duration::from_secs(10));
    Ok(Arc::new(session))
}

async fn stop(session: Arc<Session>, name: &str) -> Result {
    if let Ok(tree) = session.dump_tree().await {
        fs::write(format!("/artifacts/{name}-accessibility.xml"), tree)?;
    }
    Arc::try_unwrap(session)
        .map_err(|_| "Session still shared")?
        .kill()
        .await?;
    Ok(())
}

fn home_playback_state(item: &Item, canonical: &Item) -> (usize, Vec<String>) {
    assert!(item.progress_percent.is_none_or(|value| value <= 100));
    let progress = item
        .progress_percent
        .or(canonical.progress_percent.map(|value| value.min(100)))
        .map(|value| format!("Playback progress {value}%"))
        .into_iter()
        .collect();
    (usize::from(canonical.watched == Some(true)), progress)
}

#[test]
fn home_playback_state_uses_row_override_and_canonical_fallback() {
    let canonical = Item {
        progress_percent: Some(40),
        watched: Some(false),
        ..Default::default()
    };
    let mut row = Item::default();
    assert_eq!(
        home_playback_state(&row, &canonical),
        (0, vec!["Playback progress 40%".into()])
    );
    row.progress_percent = Some(0);
    assert_eq!(
        home_playback_state(&row, &canonical),
        (0, vec!["Playback progress 0%".into()])
    );
    row.progress_percent = Some(100);
    row.watched = Some(true);
    assert_eq!(
        home_playback_state(&row, &canonical),
        (0, vec!["Playback progress 100%".into()])
    );
    row.progress_percent = None;
    row.watched = Some(false);
    assert_eq!(
        home_playback_state(
            &row,
            &Item {
                watched: Some(true),
                ..Default::default()
            }
        ),
        (1, vec![])
    );
    assert_eq!(home_playback_state(&row, &Default::default()), (0, vec![]));
}

async fn home_ready(session: &Arc<Session>) -> Result {
    visible(session, "Home").await?;
    visible(session, "Browse libraries").await?;
    if let Ok(label) = std::env::var("E2E_REQUIRED_LABEL") {
        visible(session, &label).await?;
    }
    let rows = client()?.get_home(Default::default()).await?;
    assert!(
        rows.iter()
            .any(|row| !row.items.is_empty() && row.read_error.is_none()),
        "Catalog showcase requires a nonempty server Home row"
    );
    for row in &rows {
        if let Some(error) = &row.read_error {
            let warning = if error.code == "progress_unavailable" {
                "Continue Watching is unavailable. Jellyfin progress could not be read. Other catalog rows remain available."
            } else {
                "This row could not be loaded. Other catalog rows remain available."
            };
            visible(session, warning).await?;
            continue;
        }
        if row.items.is_empty() {
            continue;
        }
        visible(session, &row.title).await?;
        let selector = format!("//*[Label[@name={}]]//Button", xpath_literal(&row.title));
        let locator = session.locate(&selector);
        locator.wait_for_count(row.items.len()).await?;
        let mut expected = row
            .items
            .iter()
            .map(|item| {
                let canonical = rows
                    .iter()
                    .flat_map(|row| &row.items)
                    .find(|candidate| candidate.id == item.id)
                    .unwrap();
                let (watched, progress) = home_playback_state(item, canonical);
                (canonical.title.clone(), watched, progress)
            })
            .collect::<Vec<_>>();
        let mut actual = Vec::new();
        for button in locator.inspect_all().await? {
            let name = button.name.ok_or("Home card must announce identity")?;
            let (_, title) = row
                .items
                .iter()
                .map(|item| (&item.id, &item.title))
                .find(|(_, title)| name.contains(title.as_str()))
                .ok_or("Unexpected Home card")?;
            let parts = name.split(", ").collect::<Vec<_>>();
            actual.push((
                title.clone(),
                parts.iter().filter(|part| **part == "Watched").count(),
                parts
                    .iter()
                    .filter(|part| part.starts_with("Playback progress "))
                    .map(|part| (*part).to_owned())
                    .collect(),
            ));
        }
        expected.sort();
        actual.sort();
        assert_eq!(
            actual, expected,
            "Home must preserve server row membership and explicit watched/progress state"
        );
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct VisiblePoster {
    reference: String,
    name: String,
}

fn snapshot_attribute<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.split_once(&format!(" {name}=\""))?.1.split('"').next()
}

fn visible_posters(tree: &str) -> Result<Vec<VisiblePoster>> {
    let mut clips: Vec<Option<[i32; 4]>> = Vec::new();
    let mut cards: Vec<(usize, bool)> = Vec::new();
    let mut posters = Vec::new();
    for line in tree.lines().map(str::trim) {
        if line.starts_with("<?") {
            continue;
        }
        if line.starts_with("</") {
            if cards.last().is_some_and(|(depth, _)| *depth == clips.len()) {
                let (_, has_image) = cards.pop().unwrap();
                if !has_image {
                    return Err(
                        "Visible poster card has no image widget; choose an artwork-backed scene"
                            .into(),
                    );
                }
            }
            clips.pop();
            continue;
        }
        if !line.starts_with('<') {
            continue;
        }
        let bounds = snapshot_attribute(line, "bbox")
            .map(|value| {
                value
                    .split(',')
                    .map(str::parse::<i32>)
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .transpose()?;
        let own: Option<[i32; 4]> = bounds
            .map(|values| values.try_into().map_err(|_| "Invalid snapshot bounds"))
            .transpose()?;
        let parent = clips.last().copied().flatten();
        let clip = match (parent, own) {
            (Some(a), Some(b)) => {
                let x = a[0].max(b[0]);
                let y = a[1].max(b[1]);
                Some([
                    x,
                    y,
                    (a[0] + a[2]).min(b[0] + b[2]) - x,
                    (a[1] + a[3]).min(b[1] + b[3]) - y,
                ])
            }
            (a, b) => a.or(b),
        };
        let showing = snapshot_attribute(line, "showing") == Some("true")
            && clip.is_some_and(|b| b[2] > 0 && b[3] > 0);
        if showing
            && line.starts_with("<Button ")
            && own.is_some_and(|b| b[2] >= 64 && b[3] > b[2] * 3 / 2)
        {
            cards.push((clips.len() + 1, false));
        }
        if showing
            && line.starts_with("<Image ")
            && own.is_some_and(|b| b[2] >= 64 && b[3] >= b[2] * 14 / 10 && b[3] <= b[2] * 16 / 10)
        {
            for (_, has_image) in &mut cards {
                *has_image = true;
            }
            posters.push(VisiblePoster {
                reference: snapshot_attribute(line, "_ref")
                    .ok_or("Poster image has no stable accessibility reference")?
                    .into(),
                name: snapshot_attribute(line, "name").unwrap_or("").into(),
            });
        }
        if !line.ends_with("/>") {
            clips.push(clip);
        }
    }
    Ok(posters)
}

#[test]
fn poster_snapshot_uses_clipped_visible_widgets() {
    let tree = r#"<Window showing="true" bbox="0,0,500,400">
<Generic showing="true" bbox="0,0,300,400">
<Button showing="true" bbox="0,0,100,200">
<Image name="Image unavailable: boundary" showing="true" bbox="0,0,100,150" _ref="pending"/>
</Button>
<Image showing="true" bbox="200,0,100,150" _ref="loaded-unnamed"/>
<Image name="Image unavailable: offscreen" showing="true" bbox="300,0,100,150" _ref="offscreen"/>
<Image showing="true" bbox="0,200,16,22" _ref="icon"/>
</Generic>
</Window>"#;
    let posters = visible_posters(tree).unwrap();
    assert_eq!(
        posters
            .iter()
            .map(|image| image.reference.as_str())
            .collect::<Vec<_>>(),
        ["pending", "loaded-unnamed"]
    );
    assert!(posters[0].name.starts_with("Image unavailable:"));
    assert!(posters[1].name.is_empty());
    assert!(visible_posters("<Window showing=\"true\" bbox=\"0,0,500,400\">\n<Button showing=\"true\" bbox=\"0,0,100,200\">\n</Button>\n</Window>").is_err());
}

fn poster_readiness(tree: &str, minimum: usize, previous: &mut HashSet<String>) -> Result<usize> {
    let current = match visible_posters(tree) {
        Ok(current) => current,
        Err(error) => {
            previous.clear();
            return Err(error);
        }
    };
    let references = current
        .iter()
        .map(|poster| poster.reference.clone())
        .collect::<HashSet<_>>();
    let stable = references == *previous;
    *previous = references;
    if current.len() < minimum {
        return Err(format!(
            "Expected at least {minimum} visible artwork-backed posters, found {}",
            current.len()
        )
        .into());
    }
    if current
        .iter()
        .any(|image| image.name.starts_with("Image unavailable:"))
    {
        return Err("Visible poster image is still unavailable".into());
    }
    if !stable {
        return Err("Visible poster image references are not yet stable".into());
    }
    Ok(current.len())
}

#[test]
fn poster_readiness_retries_intermediate_layout_without_accepting_artless_cards() {
    let missing = "<Window showing=\"true\" bbox=\"0,0,500,400\">\n<Button showing=\"true\" bbox=\"0,0,100,200\">\n</Button>\n</Window>";
    let loaded = missing.replace(
        "</Button>",
        "<Image showing=\"true\" bbox=\"0,0,100,150\" _ref=\"loaded\"/>\n</Button>",
    );
    let mut previous = HashSet::new();
    assert!(poster_readiness(missing, 1, &mut previous).is_err());
    assert!(poster_readiness(&loaded, 1, &mut previous).is_err());
    assert_eq!(poster_readiness(&loaded, 1, &mut previous).unwrap(), 1);
    for _ in 0..3 {
        assert!(
            poster_readiness(missing, 1, &mut previous)
                .unwrap_err()
                .to_string()
                .contains("no image widget")
        );
        assert!(previous.is_empty());
    }
    assert!(poster_readiness(&loaded, 1, &mut previous).is_err());
    assert!(poster_readiness(&loaded, 5, &mut previous).is_err());
    let unavailable = loaded.replace("<Image ", "<Image name=\"Image unavailable: pending\" ");
    assert!(poster_readiness(&unavailable, 1, &mut previous).is_err());
}

async fn wait_visible_posters(session: &Arc<Session>, flow: &str) -> Result {
    let minimum = if flow == "home" { 5 } else { 1 };
    let mut last_error = String::new();
    let result = tokio::time::timeout(session.default_timeout(), async {
        let mut previous = HashSet::new();
        loop {
            let tree = session.dump_tree().await?;
            fs::write(format!("/artifacts/{flow}-poster-readiness.xml"), &tree)?;
            match poster_readiness(&tree, minimum, &mut previous) {
                Ok(count) => {
                    println!("{flow}: all {count} visible poster widgets loaded");
                    return Ok::<(), Box<dyn Error>>(());
                }
                Err(error) => {
                    if last_error.is_empty() {
                        fs::write(format!("/artifacts/{flow}-poster-initial.xml"), &tree)?;
                    }
                    last_error = format!("{flow}: {error}");
                    fs::write(
                        format!("/artifacts/{flow}-poster-readiness.txt"),
                        &last_error,
                    )?;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if !matches!(&result, Ok(Ok(()))) {
        match session.take_screenshot().await {
            Ok(png) => {
                if let Err(error) = fs::write(format!("/artifacts/{flow}-poster-failure.png"), png)
                {
                    eprintln!("{flow}: could not save failure screenshot: {error}");
                }
            }
            Err(error) => eprintln!("{flow}: could not capture failure screenshot: {error}"),
        }
    }
    result.map_err(|_| {
        format!(
            "{flow}: visible poster widgets did not load before the capture deadline: {last_error}"
        )
    })??;
    Ok(())
}

async fn capture(session: &Arc<Session>, flow: &str) -> Result<Vec<u8>> {
    wait_visible_posters(session, flow).await?;
    let png = session.take_screenshot().await?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.to_rgb8();
    assert_eq!(image.dimensions(), (1280, 800), "{flow}");
    assert!(
        image
            .pixels()
            .map(|pixel| pixel.0)
            .collect::<HashSet<_>>()
            .len()
            > 8,
        "{flow}: blank compositor frame"
    );
    Ok(png)
}

async fn journey(captures: bool) -> Result {
    let (library, item) = selection().await?;
    let session = launch(vec![]).await?;
    let result = async {
        home_ready(&session).await?;
        let mut shots = vec![("home", capture(&session, "home").await?)];
        click(&session, "Expand sidebar").await?;
        visible(&session, "Collapse sidebar").await?;
        shots.push(("sidebar", capture(&session, "sidebar").await?));
        click_library(&session, &library).await?;
        session
            .locate("//GridCell")
            .first()
            .wait_for_visible()
            .await?;
        click(&session, "Collapse sidebar").await?;
        shots.push(("library", capture(&session, "library").await?));
        click(&session, "Search titles").await?;
        visible(
            &session,
            &format!("Enter a title to search {}.", library.title),
        )
        .await?;
        configure_title_search(&session).await?;
        session
            .locate("//*[@editable='true']")
            .first()
            .fill(&item.title)
            .await?;
        visible(&session, &item.title).await?;
        shots.push(("search", capture(&session, "search").await?));
        click(&session, &item.title).await?;
        visible(&session, &format!("{} · {}", item.kind, item.year)).await?;
        session
            .locate("//Button[@name='Play']")
            .wait_for_count(1)
            .await?;
        session
            .locate("//Button[@name='Resume']")
            .wait_for_count(0)
            .await?;
        let art = item.artwork.as_ref().unwrap();
        visible_detail_art(&session, &item).await?;
        let bytes = client()?.load_artwork(art.url.clone()).await?;
        assert!(
            image::load_from_memory(&bytes).is_ok(),
            "Published artwork must decode"
        );
        shots.push(("detail", capture(&session, "detail").await?));
        session
            .locate("//*[@name='Provider attribution' or @name='Data & artwork credits']")
            .wait_for_count(0)
            .await?;
        assert_eq!(
            shots.iter().map(|(flow, _)| *flow).collect::<HashSet<_>>(),
            FLOWS.into_iter().collect()
        );
        Ok::<_, Box<dyn Error>>(shots)
    }
    .await;
    stop(session, "journey").await?;
    let shots = result?;
    if captures {
        for (flow, png) in shots {
            fs::write(format!("/artifacts/{flow}.png"), png)?;
        }
    }
    Ok(())
}

async fn configure_title_search(session: &Arc<Session>) -> Result {
    click(session, "Search settings").await?;
    session
        .locate("//*[@name='Title only']")
        .first()
        .wait_for_visible()
        .await?;
    let controls = session
        .locate("//*[@name='Title only']")
        .first()
        .inspect_all()
        .await?;
    let control = controls
        .first()
        .ok_or("Title-only search control is absent")?;
    let title_only = session.cached_accessibles().await?.iter().any(|item| {
        item.name.as_deref() == Some("Title only")
            && item.states.iter().any(|state| state == "checked")
    });
    if !title_only {
        session
            .activate_ref(&control.ref_.0, &control.ref_.1)
            .await?;
    }
    click(session, "Search settings").await?;
    Ok(())
}

async fn visible_detail_art(session: &Arc<Session>, item: &Item) -> Result {
    let kind_year = xpath_literal(&format!("{} · {}", item.kind, item.year));
    let title = xpath_literal(&item.title);
    session.locate(&format!("//Generic[Generic/Label[@name={kind_year}] and Generic/Label[@name={title}]]/Generic[1]//Image")).first().wait_for_visible().await?;
    let art = item
        .artwork
        .as_ref()
        .ok_or("Selected item needs published artwork")?;
    if !art.label.is_empty() {
        visible(session, &art.label).await?;
    }
    Ok(())
}

#[tokio::test]
async fn happy_path() -> Result {
    journey(true).await
}

#[tokio::test]
async fn playback_showcase() -> Result {
    playback::journey().await
}

#[tokio::test]
async fn regression_catalog() -> Result {
    journey(false).await?;
    let session = launch(vec![]).await?;
    let result = async {
        home_ready(&session).await?;
        click(&session, "Expand sidebar").await?;
        for label in ["Subtitles", "Library tracks", "Transcription", "Off"] {
            session
                .locate(&format!("//*[@name='{label}']"))
                .wait_for_hidden()
                .await?;
        }
        settings_failure_recovery(&session).await?;
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    stop(session, "settings").await?;
    result?;
    let saved = fs::read(std::env::var("SCREENROOM_SETTINGS_PATH")?)?;
    let relaunched = launch(vec![]).await?;
    home_ready(&relaunched).await?;
    click(&relaunched, "Expand sidebar").await?;
    assert_eq!(saved, fs::read(std::env::var("SCREENROOM_SETTINGS_PATH")?)?);
    stop(relaunched, "relaunch").await
}

async fn settings_failure_recovery(session: &Arc<Session>) -> Result {
    let path = std::env::var("SCREENROOM_SETTINGS_PATH")?;
    fs::remove_file(&path)?;
    fs::create_dir(&path)?;
    click(session, "Search titles").await?;
    visible(session, "Retry saving settings").await?;
    click(session, "Home").await?;
    visible(session, "Retry saving settings").await?;
    fs::remove_dir(&path)?;
    click(session, "Retry saving settings").await?;
    session
        .locate("//Button[@name='Retry saving settings']")
        .wait_for_hidden()
        .await?;
    preferences(|settings| settings.scope.is_some()).await
}

async fn preferences(
    predicate: impl Fn(&screenroom_client_core::ClientSettings) -> bool,
) -> Result {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(std::env::var("SCREENROOM_SETTINGS_PATH")?)?)?;
            assert_eq!(value["version"], 1);
            let settings = screenroom_client_core::validate_settings(serde_json::from_value(
                value["settings"].clone(),
            )?)?;
            if predicate(&settings) {
                return Ok::<(), Box<dyn Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn regression_search() -> Result {
    let (library, item) = selection().await?;
    let session = launch(vec![]).await?;
    let result = async {
        home_ready(&session).await?;
        click(&session, "Expand sidebar").await?;
        click_library(&session, &library).await?;
        session
            .locate("//GridCell")
            .first()
            .wait_for_visible()
            .await?;
        click(&session, "Collapse sidebar").await?;
        click(&session, "Search titles").await?;
        visible(
            &session,
            &format!("Enter a title to search {}.", library.title),
        )
        .await?;
        configure_title_search(&session).await?;
        let mut chars = item.title.chars().collect::<Vec<_>>();
        let last = chars
            .pop()
            .ok_or("Discovered title must not be empty")?
            .to_string();
        let prefix = chars.into_iter().collect::<String>();
        pending_search_action(&session, &prefix, &last, "Search settings").await?;
        visible(&session, &item.title).await?;
        click(&session, "Clear query").await?;
        visible(
            &session,
            &format!("Enter a title to search {}.", library.title),
        )
        .await?;
        click(&session, "Home").await?;
        home_ready(&session).await?;
        click(&session, "Expand sidebar").await?;
        visible(&session, "Collapse sidebar").await?;
        let completed_reads = fs::read_to_string("/artifacts/app.log")?
            .lines()
            .filter(|line| line.starts_with("Core catalog ready:"))
            .count();
        click(&session, "Refresh").await?;
        tokio::time::timeout(session.default_timeout(), async {
            loop {
                if fs::read_to_string("/artifacts/app.log")?
                    .lines()
                    .filter(|line| line.starts_with("Core catalog ready:"))
                    .count()
                    > completed_reads
                {
                    return Ok::<(), Box<dyn Error>>(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await??;
        home_ready(&session).await?;
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    stop(session, "search").await?;
    result
}

async fn pending_search_action(
    session: &Arc<Session>,
    prefix: &str,
    last: &str,
    action: &str,
) -> Result {
    let query = session.locate("//*[@editable='true']").first();
    query.fill(prefix).await?;
    session
        .locate("//Label[@name='Searching titles…']")
        .wait_for_hidden()
        .await?;
    query.wait_for_text(|text| text == prefix).await?;
    pending_search_visible(session, false).await?;
    let action = xpath_literal(action);
    let buttons = session
        .locate(&format!(
            "//*[@role='button' or @role='toggle button'][@name={action} or @description={action}]"
        ))
        .inspect_all()
        .await?;
    let [button] = buttons.as_slice() else {
        return Err("Expected one pending-search action".into());
    };
    tokio::try_join!(biased;
        async {
            pending_search_visible(session, true).await?;
            session.activate_ref(&button.ref_.0, &button.ref_.1).await?;
            pending_search_visible(session, true).await?;
            Ok::<(), Box<dyn Error>>(())
        },
        async { session.type_text(last).await?; Ok::<(), Box<dyn Error>>(()) }
    )?;
    let expected = format!("{prefix}{last}");
    session
        .locate("//*[@editable='true']")
        .first()
        .wait_for_text(|text| text == expected)
        .await?;
    Ok(())
}

async fn pending_search_visible(session: &Arc<Session>, expected: bool) -> Result {
    tokio::time::timeout(session.default_timeout(), async {
        loop {
            let visible = session.cached_accessibles().await?.iter().any(|item| {
                item.role == "Label"
                    && item.name.as_deref() == Some("Searching titles…")
                    && item.states.iter().any(|state| state == "showing")
            });
            if visible == expected {
                return Ok::<(), Box<dyn Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

fn backend_address(origin: &str) -> Result<String> {
    let port: u16 = origin
        .strip_prefix("http://127.0.0.1:")
        .ok_or("Expected canonical loopback HTTP origin")?
        .parse()?;
    if port == 0 || origin != format!("http://127.0.0.1:{port}") {
        return Err("Expected explicit nonzero numeric port".into());
    }
    Ok(format!("127.0.0.1:{port}"))
}

fn spawn_fault_gate(
    listener: tokio::net::TcpListener,
    upstream: String,
    healthy: Arc<std::sync::atomic::AtomicBool>,
    connections: Arc<std::sync::atomic::AtomicUsize>,
    response_delay_ms: Arc<std::sync::atomic::AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    use std::sync::atomic::Ordering;
    tokio::spawn(async move {
        loop {
            let (mut downstream, _) = listener.accept().await.unwrap();
            let (healthy, accepted, upstream, delay) = (
                healthy.clone(),
                connections.clone(),
                upstream.clone(),
                response_delay_ms.clone(),
            );
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                if !healthy.load(Ordering::SeqCst) {
                    while !healthy.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    return;
                }
                let mut prefix = [0; 4];
                if downstream.read_exact(&mut prefix).await.is_err() {
                    return;
                }
                let artwork = &prefix == b"GET ";
                let mut request = prefix.to_vec();
                if artwork {
                    while !request.ends_with(b"\r\n\r\n") {
                        if request.len() >= 65536 {
                            return;
                        }
                        match downstream.read_u8().await {
                            Ok(byte) => request.push(byte),
                            Err(_) => return,
                        }
                    }
                    request.truncate(request.len() - 2);
                    request.extend_from_slice(b"Connection: close\r\n\r\n");
                }
                let delay = if artwork {
                    let index = accepted.fetch_add(1, Ordering::SeqCst);
                    if index == 1 {
                        return;
                    }
                    delay.load(Ordering::SeqCst)
                } else {
                    0
                };
                if let Ok(mut upstream) = tokio::net::TcpStream::connect(upstream).await {
                    if upstream.write_all(&request).await.is_err() {
                        return;
                    }
                    if delay == 0 {
                        let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
                    } else {
                        let (mut read_client, mut write_client) = downstream.split();
                        let (mut read_server, mut write_server) = upstream.split();
                        let _ = tokio::try_join!(
                            async {
                                tokio::io::copy(&mut read_client, &mut write_server).await?;
                                write_server.shutdown().await
                            },
                            async {
                                tokio::time::sleep(Duration::from_millis(delay)).await;
                                tokio::io::copy(&mut read_server, &mut write_client).await?;
                                write_client.shutdown().await
                            }
                        );
                    }
                }
            });
        }
    })
}

#[tokio::test]
async fn nondefault_origin_fault_tunnel() -> Result {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let upstream = echo.local_addr()?.to_string();
    let echo_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = echo.accept().await.unwrap();
            tokio::spawn(async move {
                let mut data = Vec::new();
                stream.read_to_end(&mut data).await.unwrap();
                let _ = stream.write_all(&data).await;
            });
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = backend_address(&format!("http://{}", listener.local_addr()?))?;
    for invalid in [
        "http://127.0.0.1:0",
        "http://127.0.0.1:bad",
        "http://other:1234",
        "http://127.0.0.1:1234/path",
    ] {
        assert!(backend_address(invalid).is_err());
    }
    let healthy = Arc::new(AtomicBool::new(false));
    let accepted = Arc::new(AtomicUsize::new(0));
    let delay = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let gate = spawn_fault_gate(
        listener,
        upstream,
        healthy.clone(),
        accepted.clone(),
        delay.clone(),
    );
    let mut blocked = tokio::net::TcpStream::connect(&address).await?;
    let mut byte = [0];
    assert!(
        tokio::time::timeout(Duration::from_millis(80), blocked.read(&mut byte))
            .await
            .is_err()
    );
    healthy.store(true, Ordering::SeqCst);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), blocked.read(&mut byte)).await??,
        0
    );
    for index in 0..3 {
        let mut stream = tokio::net::TcpStream::connect(&address).await?;
        let data = b"GET /artwork/opaque HTTP/1.1\r\n\r\ntransparent\0bytes\xff";
        stream.write_all(data).await?;
        stream.shutdown().await?;
        let mut result = Vec::new();
        let received =
            tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut result)).await?;
        if index == 1 {
            assert!(received.is_err() || result.is_empty());
        } else {
            received?;
            let mut expected = data[..data
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 2]
                .to_vec();
            expected.extend_from_slice(b"Connection: close\r\n\r\ntransparent\0bytes\xff");
            assert_eq!(result, expected);
        }
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
    delay.store(5000, Ordering::SeqCst);
    let mut metadata = tokio::net::TcpStream::connect(&address).await?;
    let preface = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
    metadata.write_all(preface).await?;
    metadata.shutdown().await?;
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), metadata.read_to_end(&mut received)).await??;
    assert_eq!(received, preface);
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        3,
        "Metadata must not consume artwork fault positions"
    );
    delay.store(20, Ordering::SeqCst);
    let started = std::time::Instant::now();
    let mut artwork = tokio::net::TcpStream::connect(&address).await?;
    let request = b"GET /artwork/delayed HTTP/1.1\r\n\r\n";
    artwork.write_all(request).await?;
    artwork.shutdown().await?;
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), artwork.read_to_end(&mut received)).await??;
    assert_eq!(
        received,
        b"GET /artwork/delayed HTTP/1.1\r\nConnection: close\r\n\r\n"
    );
    assert!(started.elapsed() >= Duration::from_millis(20));
    assert_eq!(accepted.load(Ordering::SeqCst), 4);
    gate.abort();
    echo_task.abort();
    Ok(())
}

#[tokio::test]
async fn fault_recovery() -> Result {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let listener =
        tokio::net::TcpListener::bind(backend_address(&std::env::var("SCREENROOM_API_ENDPOINT")?)?)
            .await?;
    let healthy = Arc::new(AtomicBool::new(false));
    let connections = Arc::new(AtomicUsize::new(0));
    let response_delay = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let server = spawn_fault_gate(
        listener,
        std::env::var("SCREENROOM_BACKEND_UPSTREAM")?,
        healthy.clone(),
        connections.clone(),
        response_delay.clone(),
    );
    let session = launch(vec![]).await?;
    visible(
        &session,
        "Catalog could not be loaded. Retry to read the server state.",
    )
    .await?;
    session.locate("//GridCell").wait_for_count(0).await?;
    healthy.store(true, Ordering::SeqCst);
    click(&session, "Retry").await?;
    home_ready(&session).await?;
    let (library, item) = selection().await?;
    assert!(
        connections.load(Ordering::SeqCst) >= 3,
        "Exercise separate artwork streams"
    );
    click(&session, "Expand sidebar").await?;
    click_library(&session, &library).await?;
    click(&session, "Search titles").await?;
    configure_title_search(&session).await?;
    session
        .locate("//*[@editable='true']")
        .first()
        .fill(&item.title)
        .await?;
    visible(&session, &item.title).await?;
    click(&session, &item.title).await?;
    click(&session, "Refresh").await?;
    visible_detail_art(&session, &item).await?;
    stop(session, "fault").await?;
    connections.store(0, Ordering::SeqCst);
    response_delay.store(3500, Ordering::SeqCst);
    let started = std::time::Instant::now();
    let delayed = launch(vec![(
        "SCREENROOM_E2E_OPERATION_TIMEOUT_MS".into(),
        "5000".into(),
    )])
    .await?;
    home_ready(&delayed).await?;
    assert!(
        started.elapsed() < Duration::from_secs(9),
        "Artwork must not block metadata readiness"
    );
    tokio::time::sleep(Duration::from_secs(6)).await;
    let at_deadline = connections.load(Ordering::SeqCst);
    assert!(
        at_deadline >= 4,
        "Real-data fault journey requires multiple published artwork requests"
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        connections.load(Ordering::SeqCst),
        at_deadline,
        "Deadline must stop later image requests"
    );
    response_delay.store(0, Ordering::SeqCst);
    click(&delayed, "Expand sidebar").await?;
    click_library(&delayed, &library).await?;
    click(&delayed, "Search titles").await?;
    configure_title_search(&delayed).await?;
    delayed
        .locate("//*[@editable='true']")
        .first()
        .fill(&item.title)
        .await?;
    visible(&delayed, &item.title).await?;
    click(&delayed, &item.title).await?;
    click(&delayed, "Refresh").await?;
    visible_detail_art(&delayed, &item).await?;
    stop(delayed, "artwork-deadline").await?;
    server.abort();
    Ok(())
}
