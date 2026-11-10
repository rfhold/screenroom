import XCTest
import CoreGraphics

final class HomeE2ETests: XCTestCase {
    @MainActor
    func testHomeCatalogAndNavigation() throws {
        try homeCatalogAndNavigation(captures: false)
    }

    @MainActor
    func testHappyPathShowcase() throws {
        try homeCatalogAndNavigation(captures: true)
    }

    @MainActor
    private func homeCatalogAndNavigation(captures: Bool) throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        configure(app, pageSize: captures ? "100" : "1")
        app.launch()

        let content = [("page.title", "Home"), ("home.row.0", "Continue watching")]
        for (identifier, label) in content {
            let element = app.staticTexts[identifier]
            XCTAssertTrue(element.waitForExistence(timeout: 10), identifier)
            #if os(macOS)
            let text = element.label.isEmpty ? element.value as? String : element.label
            XCTAssertEqual(text, label)
            #else
            XCTAssertEqual(element.label, label)
            #endif
            XCTAssertTrue(element.isHittable, "Not visible: \(identifier)")
        }

        XCTAssertTrue(app.buttons["card.big-buck-bunny"].firstMatch.waitForExistence(timeout: 5))
        let posterReady = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Licensed core artwork ready"), object: app.buttons["card.big-buck-bunny"].firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [posterReady], timeout: 10), .completed)
        XCTAssertTrue(app.buttons["sidebar.toggle"].isHittable)

        if captures { capture(app, flow: "home") }

        activate(app.buttons["sidebar.toggle"])
        XCTAssertEqual(app.buttons["sidebar.toggle"].label, "Collapse sidebar")
        XCTAssertTrue(app.buttons["menu.Movies"].exists)
        XCTAssertEqual(app.buttons["menu.Movies"].label, "Movies")
        XCTAssertTrue(app.windows.firstMatch.frame.contains(app.buttons["menu.Movies"].frame))
        XCTAssertTrue(app.buttons["sidebar.toggle"].isHittable)
        XCTAssertTrue(app.windows.firstMatch.frame.contains(app.buttons["sidebar.toggle"].frame))
        XCTAssertFalse(app.buttons["Log out"].exists)
        XCTAssertFalse(app.buttons["Continue"].exists)
        #if os(tvOS)
        XCTAssertEqual(app.buttons["sidebar.subtitles"].value as? String, "Off; Collapsed")
        XCTAssertFalse(app.buttons["subtitle.Off"].exists)
        XCTAssertEqual(app.buttons["menu.Home"].value as? String, "Selected")
        focus(app.buttons["menu.Movies"])
        XCTAssertTrue(app.buttons["menu.Movies"].hasFocus)
        XCTAssertEqual(app.buttons["menu.Movies"].value as? String, "Not selected")
        assertHeading(app, "Home")
        for title in ["Refresh"] {
            XCTAssertTrue(app.windows.firstMatch.frame.contains(app.buttons[title].frame))
        }
        #else
        XCTAssertTrue(app.buttons["Off"].exists)
        #endif
        if captures { capture(app, flow: "sidebar") }
        #if os(tvOS)
        activate(app.buttons["menu.Movies"])
        #endif
        activate(app.buttons["sidebar.toggle"])
        XCTAssertEqual(app.buttons["sidebar.toggle"].label, "Expand sidebar")
        #if !os(tvOS)
        activate(app.buttons["menu.Movies"])
        #endif
        assertHeading(app, "Movies")
        XCTAssertTrue(app.buttons["card.nosferatu"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["card.nosferatu"].isHittable)
        XCTAssertEqual(app.buttons["card.nosferatu"].value as? String, "Designed placeholder; poster not included")
        if captures { capture(app, flow: "library") }
        activate(app.buttons["card.big-buck-bunny"])
        assertHeading(app, "Big Buck Bunny")
        XCTAssertTrue(app.buttons["breadcrumb.big-buck-bunny"].exists)
        XCTAssertFalse(app.staticTexts["metadata.disclosure"].exists)
        XCTAssertFalse(app.buttons["Play"].exists)
        if captures {
            XCTAssertTrue(app.staticTexts["Cast"].waitForExistence(timeout: 5))
            XCTAssertTrue(app.descendants(matching: .any).matching(identifier: "artwork.loading").firstMatch.waitForNonExistence(timeout: 10))
            XCTAssertFalse(app.descendants(matching: .any).matching(identifier: "artwork.error").firstMatch.exists)
            XCTAssertFalse(app.buttons["Resume"].exists)
            capture(app, flow: "detail")
            #if os(tvOS)
            focus(app.buttons["navigation.back"])
            XCUIRemote.shared.press(.left)
            XCTAssertTrue(app.buttons["sidebar.toggle"].hasFocus)
            activate(app.buttons["sidebar.toggle"])
            #endif
            activate(app.buttons["menu.Home"])
            showcaseSearch(app)
            return
        }
        #if os(tvOS)
        focus(app.buttons["navigation.back"])
        XCUIRemote.shared.press(.left)
        XCTAssertTrue(app.buttons["sidebar.toggle"].hasFocus)
        XCTAssertEqual(app.buttons["menu.Movies"].value as? String, "Selected")
        XCUIRemote.shared.press(.right)
        XCTAssertFalse(app.buttons["sidebar.toggle"].hasFocus)
        assertHeading(app, "Big Buck Bunny")
        #endif
        activate(app.buttons["breadcrumb.root"])
        activate(app.buttons["card.nosferatu"])
        assertHeading(app, "Nosferatu")
        XCTAssertTrue(app.buttons["breadcrumb.nosferatu"].exists)
        XCTAssertTrue(app.staticTexts["Cast"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["metadata.info"].exists)
        activate(app.buttons["metadata.info"])
        XCTAssertTrue(app.staticTexts["metadata.disclosure"].exists)
        activate(app.buttons["metadata.info"])
        XCTAssertFalse(app.buttons["Play"].exists)
        XCTAssertFalse(app.buttons["Resume"].exists)

        #if os(tvOS)
        activate(app.buttons["sidebar.toggle"])
        #endif
        activate(app.buttons["menu.TV Shows"])
        activate(app.buttons["card.beverly-hillbillies"])
        activate(app.buttons["card.beverly-hillbillies-s1"])
        activate(app.buttons["metadata.info"])
        let missingRating = app.staticTexts["metadata.disclosure"]
        let missingRatingText = missingRating.label.isEmpty ? missingRating.value as? String ?? "" : missingRating.label
        XCTAssertTrue(missingRatingText.contains("Vote count not available"))
        XCTAssertFalse(missingRatingText.contains("0 votes"))
        XCTAssertTrue(missingRatingText.contains("6.1/10"))
        activate(app.buttons["metadata.info"])
        activate(app.buttons["card.beverly-hillbillies-s1-e1"])
        XCTAssertTrue(app.staticTexts["page.title"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Play"].exists)
        XCTAssertFalse(app.buttons["Resume"].exists)
        activate(app.buttons["breadcrumb.root"])
        XCTAssertTrue(app.buttons["card.beverly-hillbillies"].waitForExistence(timeout: 5))
        activate(app.buttons["menu.Search titles"])
        #if os(tvOS)
        activate(app.buttons["sidebar.toggle"])
        #endif
        XCTAssertTrue(app.textFields["search.query"].waitForExistence(timeout: 5))
        assertStatus(app, "Enter a title to search TV Shows.")
        XCTAssertFalse(app.buttons["search.type.Movie"].exists)
        XCTAssertFalse(app.buttons["search.source.TMDB"].exists)
        activate(app.buttons["search.options"])
        XCTAssertTrue(app.buttons["Title only"].waitForExistence(timeout: 5))
        #if os(tvOS)
        for method in ["Text", "Fuzzy", "Semantic"] {
            XCTAssertEqual(app.buttons["search.method.\(method)"].value as? String, "Selected")
        }
        activate(app.buttons["search.source.Jellyfin"])
        assertStatus(app, "Enter a title to search TV Shows.")
        XCTAssertEqual(app.buttons["search.source.Jellyfin"].value as? String, "Selected")
        activate(app.buttons["search.source.TMDB"])
        #endif
        activate(app.buttons["Title only"])
        XCTAssertFalse(app.buttons["Semantic"].isEnabled)
        activate(app.buttons["search.options"])
        #if !os(tvOS)
        activate(app.textFields["search.query"])
        app.textFields["search.query"].typeText("beverly\n")
        XCTAssertTrue(app.buttons["card.beverly-hillbillies"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["card.beverly-hillbillies"].isHittable)
        assertStatus(app, "1 result in TV Shows")
        #if os(iOS)
        XCTAssertFalse(app.keyboards.firstMatch.exists)
        #endif
        activate(app.buttons["card.beverly-hillbillies"])
        activate(app.buttons["breadcrumb.root"])
        XCTAssertEqual(app.textFields["search.query"].value as? String, "beverly")
        activate(app.buttons["search.options"])
        activate(app.buttons["search.source.Jellyfin"])
        XCTAssertEqual(app.buttons["search.source.Jellyfin"].value as? String, "Selected")
        assertStatus(app, "1 result in TV Shows")
        activate(app.buttons["search.options"])
        XCTAssertTrue(app.buttons["card.beverly-hillbillies"].waitForExistence(timeout: 10))
        activate(app.buttons["search.options"])
        activate(app.buttons["search.source.TMDB"])
        XCTAssertEqual(app.buttons["Title only"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.method.Text"].value as? String, "Selected")
        activate(app.buttons["search.options"])
        activate(app.buttons["Clear query"])
        XCTAssertFalse(app.buttons["card.beverly-hillbillies"].exists)
        activate(app.textFields["search.query"])
        app.textFields["search.query"].typeText("zzzzzzz")
        let noMatches = NSPredicate(format: "label CONTAINS %@ OR value CONTAINS %@", "No matches", "No matches")
        XCTAssertTrue(app.staticTexts.matching(noMatches).firstMatch.waitForExistence(timeout: 5))
        activate(app.buttons["Clear query"])
        #endif
        activate(app.buttons["sidebar.toggle"])
        activate(app.buttons["Refresh"])
        XCTAssertTrue(app.staticTexts["refresh.status"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Catalog refreshed from core."].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Log out"].exists)
        XCTAssertFalse(app.buttons["Continue"].exists)
        activate(app.buttons["menu.Home"])
        activate(app.buttons["sidebar.toggle"])
        XCTAssertTrue(app.staticTexts["home.row.0"].waitForExistence(timeout: 5))
    }

    #if os(tvOS)
    @MainActor
    func testSidebarRemoteControls() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        configure(app)
        app.launch()
        activate(app.buttons["sidebar.toggle"])
        for choice in ["Library tracks", "Transcription", "Off"] {
            activate(app.buttons["sidebar.subtitles"])
            for option in ["Off", "Library tracks", "Transcription"] {
                XCTAssertTrue(app.buttons["subtitle.\(option)"].exists)
            }
            activate(app.buttons["subtitle.\(choice)"])
            XCTAssertEqual(app.buttons["sidebar.subtitles"].value as? String, "\(choice); Collapsed")
            XCTAssertFalse(app.buttons["subtitle.Off"].exists)
        }
        activate(app.buttons["Refresh"])
        XCTAssertTrue(app.staticTexts["Catalog refreshed from core."].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Log out"].exists)
        XCTAssertFalse(app.buttons["Continue"].exists)
        assertHeading(app, "Home")
        focus(app.buttons["menu.Home"])
        XCTAssertEqual(app.buttons["menu.Home"].value as? String, "Selected")
        XCUIRemote.shared.press(.right)
        XCTAssertFalse(app.buttons["menu.Home"].hasFocus)
        XCUIRemote.shared.press(.left)
        let returnedFocus = app.buttons.matching(NSPredicate(format: "hasFocus == true")).firstMatch
        XCTAssertTrue(returnedFocus.identifier.hasPrefix("menu.") || returnedFocus.identifier == "sidebar.toggle")
        XCTAssertEqual(app.buttons["menu.Home"].value as? String, "Selected")
        assertHeading(app, "Home")
    }
    #endif

    @MainActor
    private func showcaseSearch(_ app: XCUIApplication) {
        activate(app.buttons["menu.Search titles"])
        if app.buttons["sidebar.toggle"].label == "Collapse sidebar" { activate(app.buttons["sidebar.toggle"]) }
        assertStatus(app, "Enter a title to search All libraries.")
        XCTAssertFalse(app.buttons["search.type.Movie"].exists)
        activate(app.buttons["search.options"])
        XCTAssertEqual(app.buttons["search.source.All catalogs"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.type.Movie"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.type.Series"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.type.Episode"].value as? String, "Selected")
        activate(app.buttons["Title only"])
        XCTAssertEqual(app.buttons["search.method.Semantic"].value as? String, "Not selected")
        activate(app.buttons["search.options"])
        activate(app.textFields["search.query"])
        app.textFields["search.query"].typeText("Bunny\n")
        #if os(tvOS)
        if app.keyboards.firstMatch.exists { XCUIRemote.shared.press(.menu) }
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        #endif
        XCTAssertTrue(app.buttons["card.big-buck-bunny"].waitForExistence(timeout: 5))
        let artwork = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Licensed core artwork ready"), object: app.buttons["card.big-buck-bunny"].firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [artwork], timeout: 10), .completed)
        assertStatus(app, "1 result in All libraries")
        XCTAssertFalse(app.buttons["search.source.TMDB"].exists)
        #if os(iOS) || os(tvOS)
        XCTAssertFalse(app.keyboards.firstMatch.exists)
        #endif
        capture(app, flow: "search")
    }

    @MainActor
    func testFreshSearchAndHomeScope() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        configure(app)
        app.launch()
        activate(app.buttons["menu.TV Shows"])
        activate(app.buttons["card.beverly-hillbillies"])
        activate(app.buttons["card.beverly-hillbillies-s1"])
        activate(app.buttons["menu.Search titles"])
        assertStatus(app, "Enter a title to search TV Shows.")
        activate(app.buttons["search.options"])
        activate(app.buttons["search.type.Movie"])
        activate(app.buttons["search.method.Text"])
        activate(app.buttons["Title only"])
        activate(app.buttons["search.source.Jellyfin"])
        #if !os(tvOS)
        activate(app.textFields["search.query"])
        app.textFields["search.query"].typeText("zzzzzzz")
        #endif
        activate(app.buttons["menu.Search titles"])
        assertFreshSearch(app, scope: "TV Shows", titleOnly: true, methods: ["Fuzzy"])
        activate(app.buttons["search.method.Text"])
        activate(app.buttons["Title only"])
        activate(app.buttons["menu.Search titles"])
        assertFreshSearch(app, scope: "TV Shows", titleOnly: false, methods: ["Text", "Fuzzy"])
        activate(app.buttons["search.options"])
        activate(app.buttons["search.back"])
        XCTAssertTrue(app.buttons["card.beverly-hillbillies-s1-e1"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["breadcrumb.beverly-hillbillies-s1"].exists)
        activate(app.buttons["menu.Home"])
        activate(app.buttons["menu.Search titles"])
        assertStatus(app, "Enter a title to search All libraries.")
        activate(app.buttons["search.options"])
        XCTAssertEqual(app.buttons["search.source.Jellyfin"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.type.Episode"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.type.Movie"].value as? String, "Not selected")
        #if !os(tvOS)
        activate(app.buttons["search.source.All catalogs"])
        activate(app.buttons["search.type.Movie"])
        activate(app.buttons["search.options"])
        activate(app.textFields["search.query"])
        app.textFields["search.query"].typeText("nosferatu\n")
        XCTAssertTrue(app.buttons["card.nosferatu"].waitForExistence(timeout: 5))
        activate(app.buttons["Clear query"])
        #endif
    }

    @MainActor
    private func configure(_ app: XCUIApplication, pageSize: String = "1") {
        app.launchEnvironment["SCREENROOM_API_ENDPOINT"] = ProcessInfo.processInfo.environment["SCREENROOM_API_ENDPOINT"] ?? "http://127.0.0.1:50051"
        app.launchEnvironment["SCREENROOM_USER_ID"] = ProcessInfo.processInfo.environment["SCREENROOM_USER_ID"] ?? "00000000-0000-0000-0000-000000000001"
        app.launchEnvironment["SCREENROOM_E2E_SETTINGS_NAMESPACE"] = "apple-e2e-\(UUID().uuidString)"
        app.launchEnvironment["SCREENROOM_API_PAGE_SIZE"] = pageSize
    }

    @MainActor
    func testCoreUnavailableAndRetry() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        configure(app)
        let healthyEndpoint = app.launchEnvironment["SCREENROOM_API_ENDPOINT"]
        app.launchEnvironment["SCREENROOM_API_ENDPOINT"] = "http://127.0.0.1:1"
        app.launch()
        XCTAssertTrue(app.staticTexts["core.error"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.staticTexts["home.row.0"].exists)
        XCTAssertFalse(app.buttons["card.big-buck-bunny"].exists)
        activate(app.buttons["core.retry"])
        XCTAssertTrue(app.staticTexts["core.error"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.staticTexts["home.row.0"].exists)
        XCTAssertFalse(app.buttons["card.big-buck-bunny"].exists)
        app.terminate()
        app.launchEnvironment["SCREENROOM_API_ENDPOINT"] = healthyEndpoint
        app.launch()
        XCTAssertTrue(app.staticTexts["home.row.0"].waitForExistence(timeout: 10))
        assertHeading(app, "Home")
        let poster = app.buttons["card.big-buck-bunny"].firstMatch
        XCTAssertTrue(poster.waitForExistence(timeout: 5))
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Licensed core artwork ready"), object: poster)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 10), .completed)
        XCTAssertFalse(app.staticTexts["core.error"].exists)
    }

    @MainActor
    func testSearchPaginationAndPreferencesRoundTrip() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        configure(app)
        app.launch()
        activate(app.buttons["menu.Search titles"])
        activate(app.textFields["search.query"])
        app.textFields["search.query"].typeText("Bunny\n")
        #if os(tvOS)
        if app.keyboards.firstMatch.exists { XCUIRemote.shared.press(.menu) }
        #endif
        let complete = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@ OR value == %@", "11 results in All libraries", "11 results in All libraries"), object: app.staticTexts["search.status"])
        XCTAssertEqual(XCTWaiter.wait(for: [complete], timeout: 10), .completed)
        activate(app.buttons["sidebar.toggle"])
        activate(app.buttons["Refresh"])
        XCTAssertTrue(app.staticTexts["Catalog refreshed from core."].waitForExistence(timeout: 10))
        let refreshed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@ OR value == %@", "11 results in All libraries", "11 results in All libraries"), object: app.staticTexts["search.status"])
        XCTAssertEqual(XCTWaiter.wait(for: [refreshed], timeout: 10), .completed)
        XCTAssertEqual(app.textFields["search.query"].value as? String, "Bunny")
        activate(app.buttons["search.options"])
        activate(app.buttons["search.type.Movie"])
        activate(app.buttons["search.type.Episode"])
        activate(app.buttons["search.source.TMDB"])
        activate(app.buttons["Title only"])
        activate(app.buttons["search.method.Text"])
        activate(app.buttons["menu.TV Shows"])
        assertHeading(app, "TV Shows")
        app.terminate()
        app.launch()
        assertHeading(app, "TV Shows")
        activate(app.buttons["menu.Search titles"])
        activate(app.buttons["search.options"])
        XCTAssertEqual(app.buttons["search.type.Movie"].value as? String, "Not selected")
        XCTAssertEqual(app.buttons["search.type.Series"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.type.Episode"].value as? String, "Not selected")
        XCTAssertEqual(app.buttons["Title only"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.method.Semantic"].value as? String, "Not selected")
        XCTAssertEqual(app.buttons["search.method.Text"].value as? String, "Not selected")
        XCTAssertEqual(app.buttons["search.method.Fuzzy"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.source.TMDB"].value as? String, "Selected")
        activate(app.buttons["sidebar.toggle"])
        #if os(tvOS)
        activate(app.buttons["sidebar.subtitles"])
        #endif
        activate(app.buttons["subtitle.Library tracks"])
        #if os(tvOS)
        let saved = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Library tracks; Collapsed"), object: app.buttons["sidebar.subtitles"])
        #else
        let saved = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "Selected"), object: app.buttons["subtitle.Library tracks"])
        #endif
        XCTAssertEqual(XCTWaiter.wait(for: [saved], timeout: 5), .completed)
        app.terminate()
        app.launch()
        activate(app.buttons["sidebar.toggle"])
        #if os(tvOS)
        XCTAssertEqual(app.buttons["sidebar.subtitles"].value as? String, "Library tracks; Collapsed")
        #else
        XCTAssertEqual(app.buttons["subtitle.Library tracks"].value as? String, "Selected")
        #endif
    }

    @MainActor
    private func assertHeading(_ app: XCUIApplication, _ expected: String) {
        let title = app.staticTexts["page.title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        #if os(macOS)
        XCTAssertEqual(title.label.isEmpty ? title.value as? String : title.label, expected)
        #else
        XCTAssertEqual(title.label, expected)
        #endif
        XCTAssertTrue(title.isHittable)
    }

    @MainActor
    private func capture(_ app: XCUIApplication, flow: String) {
        #if os(macOS)
        let screenshot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        #else
        let toggle = app.buttons["sidebar.toggle"]
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "hittable == true"), object: toggle)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 5), .completed)
        XCTAssertTrue(toggle.isHittable)
        XCTAssertTrue(app.windows.firstMatch.frame.contains(toggle.frame))
        let image = app.screenshot()
        XCTAssertTrue(hasVisibleGlyph(image, frame: toggle.frame, screen: app.frame), "Sidebar glyph is blank in \(flow)")
        let screenshot = XCTAttachment(screenshot: image)
        #endif
        screenshot.name = "\(flow)-success"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    #if !os(macOS)
    private func hasVisibleGlyph(_ screenshot: XCUIScreenshot, frame: CGRect, screen: CGRect) -> Bool {
        guard let image = screenshot.image.cgImage else { return false }
        let scale = CGFloat(image.width) / screen.width
        let center = frame.insetBy(dx: frame.width * 0.2, dy: frame.height * 0.2)
        let rect = CGRect(x: (center.minX - screen.minX) * scale, y: (center.minY - screen.minY) * scale, width: center.width * scale, height: center.height * scale)
        guard let crop = image.cropping(to: rect) else { return false }
        var pixels = [UInt8](repeating: 0, count: crop.width * crop.height * 4)
        return pixels.withUnsafeMutableBytes { bytes in
            guard let context = CGContext(data: bytes.baseAddress, width: crop.width, height: crop.height, bitsPerComponent: 8, bytesPerRow: crop.width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.draw(crop, in: CGRect(x: 0, y: 0, width: crop.width, height: crop.height))
            let data = bytes.bindMemory(to: UInt8.self)
            return stride(from: 0, to: data.count, by: 4).filter { data[$0] > 180 && data[$0 + 1] > 180 && data[$0 + 2] > 180 }.count >= 8
        }
    }
    #endif

    @MainActor
    private func assertFreshSearch(_ app: XCUIApplication, scope: String, titleOnly: Bool, methods: Set<String>) {
        assertStatus(app, "Enter a title to search \(scope).")
        XCTAssertFalse(app.buttons["Clear query"].exists)
        XCTAssertFalse(app.buttons["card.beverly-hillbillies"].exists)
        XCTAssertFalse(app.buttons["search.method.Text"].exists)
        XCTAssertFalse(app.buttons["search.type.Episode"].exists)
        activate(app.buttons["search.options"])
        XCTAssertEqual(app.buttons["search.type.Episode"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["search.type.Movie"].value as? String, "Not selected")
        XCTAssertEqual(app.buttons["search.type.Series"].value as? String, "Selected")
        XCTAssertEqual(app.buttons["Title only"].value as? String, titleOnly ? "Selected" : "Not selected")
        for method in ["Text", "Fuzzy", "Semantic"] {
            XCTAssertEqual(app.buttons["search.method.\(method)"].value as? String, methods.contains(method) ? "Selected" : "Not selected")
        }
    }

    @MainActor
    private func assertStatus(_ app: XCUIApplication, _ expected: String) {
        let status = app.staticTexts["search.status"]
        XCTAssertTrue(status.waitForExistence(timeout: 5))
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@ OR value == %@", expected, expected), object: status)
        XCTAssertEqual(XCTWaiter.wait(for: [settled], timeout: 10), .completed)
        #if os(macOS)
        XCTAssertEqual(status.label.isEmpty ? status.value as? String : status.label, expected)
        #else
        XCTAssertEqual(status.label, expected)
        #endif
    }

    @MainActor
    private func activate(_ element: XCUIElement) {
        XCTAssertTrue(element.waitForExistence(timeout: 5))
        #if os(tvOS)
        focus(element)
        XCUIRemote.shared.press(.select)
        #elseif os(macOS)
        let app = XCUIApplication()
        let target = element.identifier.isEmpty ? element.label : element.identifier
        let scrollView = app.scrollViews.containing(.any, identifier: target).firstMatch
        for _ in 0..<6 {
            if element.isHittable { break }
            XCTAssertTrue(scrollView.exists)
            scrollView.scroll(byDeltaX: 0, deltaY: element.frame.midY > app.windows.firstMatch.frame.midY ? -160 : 160)
        }
        element.click()
        #else
        element.tap()
        #endif
    }

    #if os(tvOS)
    @MainActor
    private func focus(_ element: XCUIElement) {
        let app = XCUIApplication()
        var previousKey: String?
        var previousHorizontal = false
        for _ in 0..<35 {
            if element.hasFocus { break }
            let focused = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).firstMatch
            guard focused.exists || focused.waitForExistence(timeout: 1) else { XCUIRemote.shared.press(.right); continue }
            let dx = element.frame.midX - focused.frame.midX
            let dy = element.frame.midY - focused.frame.midY
            let sidebarIDs = ["sidebar.toggle", "Off", "Library tracks", "Transcription", "Refresh"]
            let targetSidebar = element.identifier.hasPrefix("menu.") || element.identifier.hasPrefix("sidebar.") || element.identifier.hasPrefix("subtitle.") || sidebarIDs.contains(element.identifier) || sidebarIDs.contains(element.label)
            let focusedSidebar = focused.identifier.hasPrefix("menu.") || focused.identifier.hasPrefix("sidebar.") || focused.identifier.hasPrefix("subtitle.") || sidebarIDs.contains(focused.identifier) || sidebarIDs.contains(focused.label)
            var horizontal = targetSidebar != focusedSidebar || (!targetSidebar && abs(dy) <= 30)
            let focusKey = focused.identifier.isEmpty ? focused.label : focused.identifier
            if previousKey == focusKey && previousHorizontal == horizontal { horizontal.toggle() }
            if !app.windows.firstMatch.frame.intersects(element.frame) { horizontal = false }
            previousKey = focusKey
            previousHorizontal = horizontal
            if horizontal { XCUIRemote.shared.press(dx > 0 ? .right : .left) }
            else { XCUIRemote.shared.press(dy > 0 ? .down : .up) }
        }
        XCTAssertTrue(element.hasFocus, "Remote could not focus \(element)")
    }
    #endif
}
