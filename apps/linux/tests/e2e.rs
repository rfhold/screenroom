use std::{collections::HashSet, error::Error, fs, sync::Arc, time::Duration};
use waydriver::{CompositorRuntime, Session, SessionConfig};
use waydriver_capture_mutter::MutterCapture;
use waydriver_compositor_mutter::MutterCompositor;
use waydriver_input_mutter::MutterInput;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
const FLOWS: [&str; 5] = ["home", "sidebar", "library", "search", "detail"];

#[tokio::test]
async fn fault_recovery() -> Result {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware::{self, Next},
        response::IntoResponse,
    };
    use std::sync::Mutex;
    // Reserve the endpoint without an HTTP service so GTK exposes loading before the core deadline.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:50051").await?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut server = None;
    let mut compositor = MutterCompositor::new();
    compositor.start(Some("1280x800"), None).await?;
    let state = compositor.state().ok_or("Mutter state unavailable")?;
    let session = Arc::new(
        Session::start(
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
                extra_env: vec![("GDK_BACKEND".into(), "wayland".into())],
                capture_external_effects: false,
            },
        )
        .await?,
    );
    session.set_default_timeout(Duration::from_secs(5));
    let result = tokio::time::timeout(Duration::from_secs(55), async {
        visible(&session, "Loading mock catalog…").await?;
        eprintln!("fault recovery: GTK initial loading visible before any mock service starts");
        visible(&session, "deadline exceeded").await?;
        visible(&session, "Retry").await?;
        for name in ["Continue Watching / Up Next", "Big Buck Bunny", "Big Buck Bunny poster"] {
            session.locate(&format!("//*[@name='{name}']")).wait_for_count(0).await?;
        }
        eprintln!("fault recovery: native core deadline, Retry visible, no catalog or artwork fallback");
        let log = requests.clone();
        let router = screenroom_mock_server::router("http://127.0.0.1:50051", &[])?
            .layer(middleware::from_fn(move |request: Request<Body>, next: Next| {
                let log = log.clone();
                async move {
                    let path = request.uri().path().to_owned();
                    let response = if path == "/artwork/big-buck-bunny.jpg" {
                        StatusCode::NOT_FOUND.into_response()
                    } else { next.run(request).await };
                    log.lock().unwrap().push(format!("{} {}", response.status().as_u16(), path));
                    response
                }
            }));
        server = Some(tokio::spawn(async move { axum::serve(listener, router).await }));
        click(&session, "Retry").await?;
        visible(&session, "Continue Watching / Up Next").await?;
        visible(&session, "Big Buck Bunny").await?;
        visible(&session, "Image unavailable: Big Buck Bunny poster").await?;
        visible(&session, "His Girl Friday theatrical poster (1939)").await?;
        session.locate("//*[@name='Retry' or @name='Loading mock catalog…']").wait_for_count(0).await?;
        let log = requests.lock().unwrap();
        for expected in [
            "200 /screenroom.client.v1.BrowseService/GetHome",
            "200 /screenroom.client.v1.CatalogService/GetItem",
            "200 /screenroom.client.v1.CatalogService/GetCatalogInfo",
            "404 /artwork/big-buck-bunny.jpg",
            "200 /artwork/his-girl-friday.jpg",
            "200 /artwork/tmdb.png",
        ] {
            if !log.iter().any(|line| line == expected) { return Err(format!("Missing real request evidence: {expected}").into()); }
        }
        eprintln!("fault recovery: Retry loaded real core responses; artwork HTTP 404 exposes its accessible placeholder; other artwork visible");
        Ok::<(), Box<dyn Error>>(())
    }).await;
    let tree = session.dump_tree().await;
    let shutdown: Result = match Arc::try_unwrap(session) {
        Ok(session) => session.kill().await.map_err(Into::into),
        Err(_) => Err("Session still shared".into()),
    };
    if let Some(server) = server {
        server.abort();
        let _ = server.await;
    }
    fs::write(
        "/artifacts/fault-requests.log",
        requests.lock().unwrap().join("\n"),
    )?;
    if let Ok(tree) = tree {
        fs::write("/artifacts/fault-accessibility.xml", tree)?;
    }
    result??;
    shutdown?;
    Ok(())
}

#[tokio::test]
async fn happy_path() -> Result {
    run_journey("happy-path").await
}

#[tokio::test]
async fn regression_catalog() -> Result {
    run_journey("regression-catalog").await
}

#[tokio::test]
async fn regression_search() -> Result {
    run_journey("regression-search").await
}

async fn run_journey(scenario: &str) -> Result {
    if scenario == "happy-path" {
        for flow in FLOWS {
            let _ = fs::remove_file(format!("/artifacts/{flow}.png"));
        }
    }
    let mut compositor = MutterCompositor::new();
    let resolution = std::env::var("E2E_RESOLUTION").unwrap_or_else(|_| "1280x800".into());
    compositor.start(Some(&resolution), None).await?;
    let state = compositor.state().ok_or("Mutter state unavailable")?;
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
            extra_env: vec![("GDK_BACKEND".into(), "wayland".into())],
            capture_external_effects: false,
        },
    )
    .await?;
    let session = Arc::new(session);
    session.set_default_timeout(Duration::from_secs(5));
    let started = std::time::Instant::now();
    eprintln!("journey {scenario}: assertions begin");
    let result = tokio::time::timeout(Duration::from_secs(55), async {
        if scenario == "happy-path" {
            assert_happy(&session).await
        } else {
            assert_regression(&session, scenario == "regression-catalog").await?;
            Ok(Vec::new())
        }
    })
    .await;
    eprintln!(
        "journey {scenario}: assertions ended after {:?}",
        started.elapsed()
    );
    let tree = session.dump_tree().await;
    let shutdown = Arc::try_unwrap(session)
        .map_err(|_| "Session still shared")?
        .kill()
        .await;
    if let Ok(tree) = tree {
        fs::write("/artifacts/accessibility.xml", tree)?;
    }
    let captures = result??;
    shutdown?;
    if scenario == "regression-search" {
        let log = fs::read_to_string("/artifacts/app.log")?;
        assert!(
            log.matches("core GetCatalogInfo:").count() >= 3,
            "Both Refresh actions must read actual CatalogInfo"
        );
        assert!(
            log.matches("Core catalog ready:").count() >= 3,
            "Both Refresh actions must complete real catalog and artwork loads"
        );
    }
    for (flow, png) in captures {
        fs::write(format!("/artifacts/{flow}.png"), png)?;
    }
    Ok(())
}

async fn home_ready(session: &Arc<Session>) -> Result {
    let started = std::time::Instant::now();
    let mut labels = vec![
        "Home".to_owned(),
        "Continue Watching / Up Next".to_owned(),
        "Recently Released Movies".to_owned(),
        "Browse libraries".to_owned(),
        "Big Buck Bunny".to_owned(),
    ];
    if let Ok(label) = std::env::var("E2E_REQUIRED_LABEL") {
        labels.push(label);
    }
    for label in labels {
        session
            .locate(&format!("//*[@name='{label}']"))
            .first()
            .with_timeout(Duration::from_secs(5))
            .wait_for_visible()
            .await?;
    }
    session
        .locate("//Window[@name='Screenroom']")
        .wait_for_visible()
        .await?;
    for row in [
        "Recently Added Movies",
        "Recently Released Episodes",
        "Recently Added Episodes",
    ] {
        session
            .locate(&format!("//Label[@name='{row}']"))
            .wait_for_present()
            .await?;
    }
    visible(session, "Big Buck Bunny poster").await?;
    eprintln!("home data/art readiness: {:?}", started.elapsed());
    Ok(())
}

async fn assert_happy(session: &Arc<Session>) -> Result<Vec<(&'static str, Vec<u8>)>> {
    let mut captures = Vec::with_capacity(FLOWS.len());
    home_ready(session).await?;
    visible(session, "Expand sidebar").await?;
    captures.push(("home", capture(session, "home").await?));
    click(session, "Expand sidebar").await?;
    visible(session, "Subtitles").await?;
    visible(session, "Off").await?;
    captures.push(("sidebar", capture(session, "sidebar").await?));
    click(session, "Movies").await?;
    session.locate("//GridCell").wait_for_count(8).await?;
    visible(session, "Nosferatu").await?;
    click(session, "Collapse sidebar").await?;
    visible(session, "Expand sidebar").await?;
    captures.push(("library", capture(session, "library").await?));
    click(session, "Home").await?;
    visible(session, "Continue Watching / Up Next").await?;
    click(session, "Search titles").await?;
    visible(session, "Enter a title to search All libraries.").await?;
    session
        .locate("//*[@editable='true']")
        .first()
        .fill("Bunny")
        .await?;
    visible(session, "1 result in All libraries").await?;
    visible(session, "Big Buck Bunny poster").await?;
    session
        .locate("//Label[@name='Search methods']")
        .wait_for_hidden()
        .await?;
    captures.push(("search", capture(session, "search").await?));
    click(session, "Movies").await?;
    session.locate("//GridCell").wait_for_count(8).await?;
    click(session, "Big Buck Bunny").await?;
    visible(session, "Movie · 2008").await?;
    visible(session, "Big Buck Bunny poster").await?;
    visible(session, "Jellyfin Not connected").await?;
    session
        .locate("//*[@name='Sacha Goedegebure portrait']")
        .wait_for_present()
        .await?;
    session
        .locate("//*[@name='Andy Goralczyk portrait']")
        .wait_for_present()
        .await?;
    session
        .locate("//Label[@name='CC BY 3.0']")
        .wait_for_present()
        .await?;
    no_playback(session).await?;
    visible(session, "Expand sidebar").await?;
    captures.push(("detail", capture(session, "detail").await?));
    Ok(captures)
}

async fn assert_regression(session: &Arc<Session>, catalog: bool) -> Result {
    home_ready(session).await?;
    if catalog {
        let before_scroll = session.take_screenshot().await?;
        session.pointer_motion_absolute(1150.0, 700.0).await?;
        session
            .pointer_axis_discrete(waydriver::backend::PointerAxis::Vertical, 20)
            .await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
        let after_scroll = session.take_screenshot().await?;
        if before_scroll == after_scroll {
            return Err("Home did not respond to vertical scrolling".into());
        }
        session
            .pointer_axis_discrete(waydriver::backend::PointerAxis::Vertical, -20)
            .await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
        click(session, "Expand sidebar").await?;
        visible(session, "Subtitles").await?;
        visible(session, "Off").await?;
        session
            .locate("//Radio[@name='Library tracks']")
            .first()
            .click()
            .await?;
        preferences(|p| p.subtitle_preference == "library_tracks").await?;
        let isolated = screenroom_client_core::Client::new(screenroom_client_core::ClientConfig {
            endpoint: "http://127.0.0.1:50051".into(),
            mock_profile: "linux-isolated-e2e".into(),
            timeout_ms: 1000,
        })?
        .get_preferences()
        .await?;
        assert_eq!(isolated.subtitle_preference, "off");
        assert!(!isolated.search_types.unwrap().episode);
        for (label, value) in [("Transcription", "transcription"), ("Off", "off")] {
            session
                .locate(&format!("//Radio[@name='{label}']"))
                .first()
                .click()
                .await?;
            preferences(|p| p.subtitle_preference == value).await?;
        }
        click(session, "All libraries").await?;
        session.locate("//GridCell").wait_for_count(9).await?;
        click(session, "Movies").await?;
        session
            .locate("//Button[.//Label[@name='Nosferatu']]")
            .wait_for_count(1)
            .await?;
        session
            .locate("//Label[@name='The Beverly Hillbillies']")
            .wait_for_count(0)
            .await?;
        session.locate("//GridCell").wait_for_count(8).await?;
        click(session, "Collapse sidebar").await?;
        visible(session, "Expand sidebar").await?;
        click(session, "Big Buck Bunny").await?;
        visible(session, "Movie · 2008").await?;
        visible(session, "Big Buck Bunny poster").await?;
        visible(session, "Jellyfin Not connected").await?;
        session
            .locate("//Label[@name='portrait' or @name='Character still']")
            .wait_for_count(0)
            .await?;
        session
            .locate("//Label[@name='CC BY 3.0']")
            .wait_for_present()
            .await?;
        session
            .locate("//*[@name='Sacha Goedegebure portrait']")
            .wait_for_present()
            .await?;
        session
            .locate("//*[@name='Andy Goralczyk portrait']")
            .wait_for_present()
            .await?;
        click(session, "Provider information").await?;
        session
            .locate("//Label[contains(@name,'votes · Snapshot 2026-11-07')]")
            .wait_for_visible()
            .await?;
        session.press_keysym(0xff1b).await?;
        no_playback(session).await?;
        visible(session, "Expand sidebar").await?;
        // The library breadcrumb returns to the grid rather than a separate Back action.
        session
            .locate("//Button[@name='Movies']")
            .last()
            .click()
            .await?;
        session.locate("//GridCell").wait_for_count(8).await?;
        click(session, "The General").await?;
        visible(session, "Movie · 1926").await?;
        session
            .locate("//Label[@name='Character still']")
            .first()
            .wait_for_visible()
            .await?;
        session
        .locate(
            "//*[@name='Buster Keaton as Johnnie Gray — The General film still, not a portrait']",
        )
        .first()
        .wait_for_present()
        .await?;
        click(session, "TV shows").await?;
        click(session, "The Beverly Hillbillies").await?;
        session.locate("//GridCell").wait_for_count(1).await?;
        click(session, "Season 1").await?;
        visible(session, "TMDB 6.1/10").await?;
        session
            .locate("//Label[contains(@name,'Vote count unavailable · Snapshot ')]")
            .wait_for_visible()
            .await?;
        session.locate("//Label[@name='TMDB —' or @name='TMDB · Unrated' or @name='TMDB rating snapshot not imported.']").wait_for_count(0).await?;
        eprintln!("Season 1: real core rating 6.1/10 with unknown vote count remains numeric");
        click(session, "The Clampetts Strike Oil").await?;
        visible(session, "Episode · 1962").await?;
        visible(session, "The Beverly Hillbillies · S1 E1 · ~25m").await?;
        no_playback(session).await?;
        session.locate("//Button[@name='Season 1']").click().await?;
        visible(session, "Getting Settled").await?;
        session
            .locate("//Button[@name='The Beverly Hillbillies']")
            .click()
            .await?;
        visible(session, "Season 1").await?;
        return Ok(());
    }
    click(session, "TV shows").await?;
    session.locate("//GridCell").wait_for_count(1).await?;
    visible(session, "The Beverly Hillbillies").await?;
    click(session, "Search titles").await?;
    visible(session, "Enter a title to search TV shows.").await?;
    pending_search_action(session, "beverl", "y", "Search").await?;
    visible(session, "1 result in TV shows").await?;
    visible(session, "The Beverly Hillbillies").await?;
    for name in ["Catalog", "Content types", "Search methods"] {
        session
            .locate(&format!("//Label[@name='{name}']"))
            .wait_for_hidden()
            .await?;
    }
    click(session, "Data & artwork credits").await?;
    session.locate("//Label[@name='This product uses the TMDB API but is not endorsed or certified by TMDB.']").wait_for_present().await?;
    session
        .locate("//*[@name='TMDB approved primary short blue logo']")
        .wait_for_present()
        .await?;
    session
        .locate("//Label[contains(@name,'William Maanders')]")
        .first()
        .wait_for_present()
        .await?;
    let client = screenroom_client_core::Client::new(screenroom_client_core::ClientConfig {
        endpoint: "http://127.0.0.1:50051".into(),
        mock_profile: "linux-credits-e2e".into(),
        timeout_ms: 3000,
    })?;
    let info = client.clone().get_catalog_info().await?;
    let names: Vec<_> = session
        .cached_accessibles()
        .await?
        .into_iter()
        .filter_map(|a| a.name)
        .collect();
    for notice in &info.notices {
        assert!(
            names.iter().any(|name| name == &notice.text),
            "Missing server notice {}",
            notice.id
        );
    }
    for art in &info.artwork {
        assert!(
            names.iter().any(|name| name.contains(&art.label)
                && name.contains(&art.creator)
                && name.contains(&art.license)),
            "Missing global artwork credit {}",
            art.id
        );
        if art.kind == "logo" {
            let bytes = client.clone().load_artwork(art.url.clone()).await?;
            let image = image::load_from_memory(&bytes)?;
            assert!(image.width() > 0 && image.height() > 0);
            eprintln!(
                "actual CatalogInfo logo {} decoded: {} bytes, {}x{}",
                art.url,
                bytes.len(),
                image.width(),
                image.height()
            );
        }
    }
    click(session, "Data & artwork credits").await?;
    let query = session.locate("//*[@editable='true']").first();
    click(session, "Search settings").await?;
    visible(session, "Search methods").await?;
    session
        .locate("//*[@name='All' and @checkable='true']")
        .wait_for_present()
        .await?;
    session
        .locate("//*[@name='Jellyfin' and @checkable='true']")
        .click()
        .await?;
    visible(session, "No matches in TV shows. Try another search.").await?;
    session.locate("//GridCell").wait_for_count(0).await?;
    session
        .locate("//*[@name='TMDB' and @checkable='true']")
        .click()
        .await?;
    visible(session, "1 result in TV shows").await?;
    session
        .locate("//*[@name='All' and @checkable='true']")
        .click()
        .await?;
    visible(session, "1 result in TV shows").await?;
    session
        .locate("//*[@name='Title only' and @checkable='true']")
        .click()
        .await?;
    let semantic = session.locate("//*[@name='Semantic' and @checkable='true']");
    session
        .locate("//*[@name='Semantic' and @checkable='true' and not(@sensitive='true')]")
        .wait_for_present()
        .await?;
    session
        .locate("//*[@name='Title only' and @checkable='true']")
        .click()
        .await?;
    semantic.wait_for_enabled().await?;
    if semantic.is_checked().await? {
        return Err("Semantic was re-enabled by turning Title only off".into());
    }
    // Escape closes the inline settings and restores focus to its disclosure.
    session.press_keysym(0xff1b).await?;
    session
        .locate("//Label[@name='Search methods']")
        .wait_for_hidden()
        .await?;
    query.fill("clampetts strike").await?;
    click(session, "Search settings").await?;
    session
        .locate("//*[@name='Episodes' and @checkable='true']")
        .click()
        .await?;
    preferences(|p| p.search_types.as_ref().is_some_and(|t| t.episode)).await?;
    visible(session, "1 result in TV shows").await?;
    click(session, "Search settings").await?;
    click(session, "The Clampetts Strike Oil").await?;
    visible(session, "Episode · 1962").await?;
    no_playback(session).await?;
    session.locate("//Button[@name='Search']").click().await?;
    visible(session, "1 result in TV shows").await?;
    let query = session.locate("//*[@editable='true']").first();
    query
        .wait_for_text(|text| text == "clampetts strike")
        .await?;
    click(session, "Search settings").await?;
    visible(session, "Search methods").await?;
    if session
        .locate("//*[@name='Semantic' and @checkable='true']")
        .is_checked()
        .await?
    {
        return Err("Search breadcrumb reset the selected methods".into());
    }
    session
        .locate("//*[@name='Episodes' and @checkable='true']")
        .click()
        .await?;
    click(session, "Search settings").await?;
    session
        .locate("//Label[@name='Search methods']")
        .wait_for_hidden()
        .await?;
    click(session, "Clear query").await?;
    visible(session, "Enter a title to search TV shows.").await?;
    query.fill("zzzzzzzz").await?;
    visible(session, "No matches in TV shows. Try another search.").await?;
    click(session, "Search settings").await?;
    session
        .locate("//*[@name='Title only' and @checkable='true']")
        .click()
        .await?;
    click(session, "Home").await?;
    visible(session, "Continue Watching / Up Next").await?;
    click(session, "Search titles").await?;
    fresh_search(session, "All libraries").await?;
    session
        .locate("//*[@editable='true']")
        .first()
        .fill("Bunny")
        .await?;
    visible(session, "1 result in All libraries").await?;
    visible(session, "Big Buck Bunny").await?;
    session
        .locate("//*[@editable='true']")
        .first()
        .fill("general")
        .await?;
    visible(session, "1 result in All libraries").await?;
    click(session, "Search settings").await?;
    session
        .locate("//*[@name='Title only' and @checkable='true']")
        .click()
        .await?;
    click(session, "Movies").await?;
    session.locate("//GridCell").wait_for_count(8).await?;
    click(session, "Search titles").await?;
    fresh_search(session, "Movies").await?;
    click(session, "Home").await?;
    visible(session, "Continue Watching / Up Next").await?;
    session
        .locate("//Button[@name='Log out' or @name='Continue']")
        .wait_for_count(0)
        .await?;
    click(session, "Search titles").await?;
    visible(session, "Enter a title to search All libraries.").await?;
    click(session, "Expand sidebar").await?;
    pending_search_action(session, "sinte", "l", "Refresh").await?;
    visible(session, "1 result in All libraries").await?;
    visible(session, "Sintel").await?;
    click(session, "Home").await?;
    visible(session, "Continue Watching / Up Next").await?;
    click(session, "Refresh").await?;
    visible(session, "Loading mock catalog…").await?;
    session
        .locate("//Label[@name='Loading mock catalog…']")
        .wait_for_hidden()
        .await?;
    visible(session, "Continue Watching / Up Next").await?;
    session
        .locate("//Button[@name='Retry']")
        .wait_for_count(0)
        .await?;
    session
        .locate("//Label[contains(@name,'Refreshed (')]")
        .wait_for_count(0)
        .await?;
    click(session, "Collapse sidebar").await?;
    Ok(())
}

async fn pending_search_action(
    session: &Arc<Session>,
    prefix: &str,
    last: &str,
    action: &str,
) -> Result {
    let query = session.locate("//*[@editable='true']").first();
    let fill_started = std::time::Instant::now();
    query.fill(prefix).await?;
    let fill_elapsed = fill_started.elapsed();
    session
        .locate("//Label[@name='Searching titles…']")
        .wait_for_hidden()
        .await?;
    query.wait_for_text(|text| text == prefix).await?;
    pending_search_visible(session, false).await?;
    let buttons = session
        .locate(&format!("//Button[@name='{action}']"))
        .inspect_all()
        .await?;
    let [button] = buttons.as_slice() else {
        return Err(format!("Expected exactly one pending-search {action} button").into());
    };

    // Observe the final edit while Mutter still completes its key-release tail.
    let started = std::time::Instant::now();
    let ((), ()) = tokio::try_join!(
        biased;
        async {
            pending_search_visible(session, true).await?;
            let observed = started.elapsed();
            session.activate_ref(&button.ref_.0, &button.ref_.1).await?;
            let acted = started.elapsed();
            pending_search_visible(session, true).await?;
            eprintln!(
                "pending search {action}: prefix fill {fill_elapsed:?}, pending observed {observed:?}, action completed {acted:?}, pending preserved {:?}",
                started.elapsed()
            );
            Ok::<(), Box<dyn Error>>(())
        },
        async {
            session.type_text(last).await?;
            Ok::<(), Box<dyn Error>>(())
        },
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
    // Bulk AT-SPI reads avoid a full widget-tree walk during the 350 ms state.
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
    .await
    .map_err(|_| format!("Pending search visibility did not become {expected}"))??;
    Ok(())
}

async fn capture(session: &Arc<Session>, flow: &str) -> Result<Vec<u8>> {
    let started = std::time::Instant::now();
    eprintln!("capture {flow}: start");
    // Let GTK paint the asserted state before requesting the compositor frame.
    tokio::time::sleep(Duration::from_millis(120)).await;
    let png = session.take_screenshot().await?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.to_rgb8();
    if image.dimensions() != (1280, 800) {
        return Err(format!("{flow}: expected a 1280x800 compositor PNG").into());
    }
    let colors: HashSet<_> = image.pixels().map(|pixel| pixel.0).collect();
    if colors.len() <= 8 {
        return Err(format!("{flow}: compositor screenshot has at most eight RGB colors").into());
    }
    eprintln!("capture {flow}: validated in {:?}", started.elapsed());
    Ok(png)
}

async fn preferences(predicate: impl Fn(&screenroom_client_core::Preferences) -> bool) -> Result {
    use screenroom_client_core::{Client, ClientConfig};
    let client = Client::new(ClientConfig {
        endpoint: "http://127.0.0.1:50051".into(),
        mock_profile: "linux-demo".into(),
        timeout_ms: 1000,
    })?;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let preferences = client.clone().get_preferences().await?;
            if predicate(&preferences) {
                eprintln!("UI preference verified through core: {preferences:?}");
                return Ok::<(), Box<dyn Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

async fn visible(session: &Arc<Session>, name: &str) -> Result {
    session
        .locate(&format!("//*[@name='{name}' or @description='{name}']"))
        .first()
        .wait_for_visible()
        .await?;
    Ok(())
}
async fn click(session: &Arc<Session>, name: &str) -> Result {
    let started = std::time::Instant::now();
    eprintln!("action {name}: start");
    let locator = session
        .locate(&format!(
            "//*[@role='button' or @role='toggle button'][@name='{name}' or @description='{name}' or .//Label[@name='{name}']]"
        ))
        .first();
    locator.wait_for_visible().await?;
    let buttons = locator.inspect_all().await?;
    let button = buttons
        .first()
        .ok_or_else(|| format!("Missing button: {name}"))?;
    session.activate_ref(&button.ref_.0, &button.ref_.1).await?;
    eprintln!("action {name}: completed in {:?}", started.elapsed());
    Ok(())
}
async fn no_playback(session: &Arc<Session>) -> Result {
    if session
        .locate("//Button[@name='Play' or @name='Resume' or @name='Back']")
        .count()
        .await?
        != 0
    {
        return Err("Unexpected playback or Back control".into());
    }
    Ok(())
}

async fn fresh_search(session: &Arc<Session>, library: &str) -> Result {
    visible(session, &format!("Enter a title to search {library}.")).await?;
    session
        .locate("//*[@editable='true']")
        .first()
        .wait_for_text(|text| text.is_empty())
        .await?;
    session
        .locate("//Label[@name='Search methods']")
        .wait_for_hidden()
        .await?;
    click(session, "Search settings").await?;
    visible(session, "Search methods").await?;
    for (name, expected) in [
        ("All", true),
        ("Movies", true),
        ("Shows", true),
        ("Episodes", false),
        ("Title only", false),
        ("Text (titles and metadata)", true),
        ("Fuzzy titles", true),
        ("Semantic", true),
    ] {
        if session
            .locate(&format!("//*[@name='{name}' and @checkable='true']"))
            .is_checked()
            .await?
            != expected
        {
            return Err(format!("Fresh search has an unexpected {name} option").into());
        }
    }
    click(session, "Search settings").await?;
    session
        .locate("//Label[@name='Search methods']")
        .wait_for_hidden()
        .await?;
    Ok(())
}
