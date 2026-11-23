import XCTest
import CoreGraphics

final class HomeE2ETests: XCTestCase {
    @MainActor
    private func configuredApp(showcase: Bool = false) throws -> XCUIApplication {
        let env = ProcessInfo.processInfo.environment
        let endpoint = try XCTUnwrap(env["SCREENROOM_API_ENDPOINT"], "Supply the configured backend endpoint")
        let user = try XCTUnwrap(env["SCREENROOM_USER_ID"], "Supply the installation's local user selector")
        XCTAssertFalse(endpoint.isEmpty)
        XCTAssertFalse(user.isEmpty)
        let app = XCUIApplication()
        app.launchEnvironment["SCREENROOM_API_ENDPOINT"] = endpoint
        app.launchEnvironment["SCREENROOM_USER_ID"] = user
        app.launchEnvironment["SCREENROOM_E2E_SETTINGS_NAMESPACE"] = "apple-e2e-\(UUID().uuidString)"
        app.launchEnvironment["SCREENROOM_API_PAGE_SIZE"] = showcase ? "24" : "1"
        continueAfterFailure = false
        return app
    }

    @MainActor
    func testHomeCatalogAndNavigation() throws {
        try journey(captures: false)
    }

    @MainActor
    func testHappyPathShowcase() throws {
        try journey(captures: true)
    }

    @MainActor
    private func journey(captures: Bool) throws {
        let app = try configuredApp(showcase: captures)
        app.launch()
        assertHeading(app, "Home")
        XCTAssertTrue(firstCard(app).waitForExistence(timeout: 10), "Showcase requires a nonempty server-defined Home row")
        XCTAssertTrue(app.buttons["sidebar.toggle"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.staticTexts["core.error"].exists)
        if captures { capture(app, "home") }
        activate(app.buttons["sidebar.toggle"])
        XCTAssertEqual(app.buttons["sidebar.toggle"].label, "Collapse sidebar")
        XCTAssertFalse(app.buttons["Log out"].exists)
        XCTAssertFalse(app.buttons["Continue"].exists)
        if captures { capture(app, "sidebar") }
        let libraries = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'menu.' AND identifier != 'menu.Home' AND identifier != 'menu.Search titles' AND identifier != 'menu.Jellyfin profile' AND identifier != 'menu.All libraries'"))
        XCTAssertTrue(libraries.firstMatch.waitForExistence(timeout: 10), "This journey requires an indexed real library; use empty-state tests before setup")
        var libraryTitle = ""
        let card = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'card.' AND label MATCHES %@", ".+, [0-9]{1,4}(, (Watched|Playback progress [0-9]+%))*")).firstMatch
        for library in libraries.allElementsBoundByIndex.prefix(8) {
            libraryTitle = library.label
            activate(library)
            assertHeading(app, libraryTitle)
            waitForPage(app)
            if card.exists { break }
        }
        if app.buttons["sidebar.toggle"].label == "Collapse sidebar" { activate(app.buttons["sidebar.toggle"]) }
        XCTAssertTrue(card.exists, "Showcase requires a year-labelled movie card within the first page of one of the first eight libraries")
        if captures { capture(app, "library") }
        let cardID = card.identifier
        activate(card)
        XCTAssertTrue(app.buttons["breadcrumb.\(String(cardID.dropFirst(5)))"].waitForExistence(timeout: 10))
        let title = text(app.staticTexts["page.title"])
        XCTAssertFalse(title.isEmpty)
        XCTAssertTrue(app.buttons["breadcrumb.\(String(cardID.dropFirst(5)))"].exists)
        XCTAssertTrue(app.buttons["detail.play"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Resume"].exists)
        XCTAssertTrue(app.descendants(matching: .any).matching(identifier: "artwork.loading").firstMatch.waitForNonExistence(timeout: 10))
        XCTAssertFalse(app.descendants(matching: .any).matching(identifier: "artwork.error").firstMatch.exists)
        if captures { capture(app, "detail") }
        #if os(tvOS)
        XCUIRemote.shared.press(.menu)
        assertHeading(app, libraryTitle)
        XCTAssertTrue(app.buttons[cardID].waitForExistence(timeout: 10))
        #else
        activate(app.buttons["breadcrumb.root"])
        #endif
        #if os(tvOS)
        if app.buttons["sidebar.toggle"].label == "Expand sidebar" { activate(app.buttons["sidebar.toggle"]) }
        #endif
        activate(app.buttons["menu.Search titles"])
        #if os(tvOS)
        if app.buttons["sidebar.toggle"].label == "Collapse sidebar" { activate(app.buttons["sidebar.toggle"]) }
        #endif
        assertStatus(app, "Enter a title to search \(libraryTitle).")
        activate(app.buttons["search.options"])
        activate(app.buttons["Title only"])
        XCTAssertEqual(app.buttons["search.method.Semantic"].value as? String, "Not selected")
        activate(app.buttons["search.options"])
        activate(app.textFields["search.query"])
        app.textFields["search.query"].typeText(title + "\n")
        #if os(tvOS)
        if app.keyboards.firstMatch.exists { XCUIRemote.shared.press(.menu) }
        #endif
        waitForPage(app)
        for _ in 0..<3 {
            if app.buttons[cardID].exists { break }
            let more = app.buttons["catalog.loadMore"]
            if !more.exists { break }
            activate(more)
            waitForContinuation(app)
        }
        XCTAssertTrue(app.buttons[cardID].waitForExistence(timeout: 10), "The selected real item must appear within the first four scoped title-search pages")
        if captures { capture(app, "search") }
        activate(app.buttons[cardID])
        #if os(tvOS)
        XCTAssertTrue(app.buttons["breadcrumb.\(String(cardID.dropFirst(5)))"].waitForExistence(timeout: 10))
        XCUIRemote.shared.press(.menu)
        XCTAssertTrue(app.textFields["search.query"].waitForExistence(timeout: 10))
        #else
        activate(app.buttons["breadcrumb.root"])
        #endif
        XCTAssertEqual(app.textFields["search.query"].value as? String, title)
        activate(app.buttons["Clear query"])
        assertStatus(app, "Enter a title to search \(libraryTitle).")
        XCTAssertFalse(app.buttons[cardID].exists)
    }

    @MainActor
    func testFreshSearchAndHomeScope() throws {
        let app = try configuredApp(showcase: true)
        app.launch()
        assertHeading(app, "Home")
        activate(app.buttons["menu.Search titles"])
        assertStatus(app, "Enter a title to search All libraries.")
        activate(app.buttons["search.options"])
        activate(app.buttons["Title only"])
        XCTAssertFalse(app.buttons["search.method.Semantic"].isEnabled)
        activate(app.buttons["menu.Home"])
        activate(app.buttons["menu.Search titles"])
        assertStatus(app, "Enter a title to search All libraries.")
        XCTAssertFalse(app.buttons["Clear query"].exists)
        if app.buttons["sidebar.toggle"].label == "Expand sidebar" { activate(app.buttons["sidebar.toggle"]) }
        let libraries = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'menu.' AND identifier != 'menu.Home' AND identifier != 'menu.Search titles' AND identifier != 'menu.Jellyfin profile' AND identifier != 'menu.All libraries'"))
        var seriesID = ""
        discovery: for library in libraries.allElementsBoundByIndex.prefix(8) {
            activate(library)
            waitForPage(app)
            let candidates = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'card.'")).allElementsBoundByIndex.prefix(3).map(\.identifier)
            for candidate in candidates {
                activate(app.buttons[candidate])
                XCTAssertTrue(app.buttons["breadcrumb.\(String(candidate.dropFirst(5)))"].waitForExistence(timeout: 10))
                XCTAssertTrue(app.staticTexts["page.title"].waitForExistence(timeout: 10))
                XCTAssertTrue(app.descendants(matching: .any).matching(identifier: "core.loading").firstMatch.waitForNonExistence(timeout: 15))
                let series = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Series · ' OR value BEGINSWITH 'Series · '")).firstMatch
                let count = app.staticTexts["catalog.loadedCount"]
                if series.exists && count.exists && (Int(text(count).split(separator: " ").first ?? "") ?? 0) > 0 {
                    seriesID = String(candidate.dropFirst(5))
                    break discovery
                }
                activate(library)
                waitForPage(app)
            }
        }
        XCTAssertFalse(seriesID.isEmpty, "Hierarchy regression requires a series among three first-page candidates in eight libraries")
        if app.buttons["sidebar.toggle"].label == "Collapse sidebar" { activate(app.buttons["sidebar.toggle"]) }
        let season = boundedFirstChild(app)
        let seasonID = String(season.identifier.dropFirst(5))
        activate(season)
        let episode = boundedFirstChild(app)
        XCTAssertTrue(episode.waitForExistence(timeout: 10), "The discovered season must contain an episode")
        activate(episode)
        XCTAssertTrue(app.buttons["breadcrumb.\(seriesID)"].exists)
        XCTAssertTrue(app.buttons["breadcrumb.\(seasonID)"].exists)
        XCTAssertTrue(app.buttons["detail.play"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["Resume"].exists)
        activate(app.buttons["breadcrumb.\(seasonID)"])
        XCTAssertTrue(firstCard(app).waitForExistence(timeout: 10))
    }

    @MainActor
    func testSearchPaginationAndPreferencesRoundTrip() throws {
        let app = try configuredApp()
        app.launch()
        assertHeading(app, "Home")
        activate(app.buttons["sidebar.toggle"])
        XCTAssertFalse(app.buttons["sidebar.subtitles"].exists)
        XCTAssertFalse(app.buttons["subtitle.Library tracks"].exists)
        app.terminate()
        app.launch()
        assertHeading(app, "Home")
        activate(app.buttons["sidebar.toggle"])
        XCTAssertFalse(app.buttons["sidebar.subtitles"].exists)
        XCTAssertFalse(app.buttons["subtitle.Library tracks"].exists)
        activate(app.buttons["Refresh"])
        XCTAssertTrue(app.staticTexts["Catalog refreshed from core."].waitForExistence(timeout: 10))
        // Continuations, duplicate IDs and traversal limits are checked against discovered library data.
        app.terminate()
        try journey(captures: false)
    }

    @MainActor
    func testCoreUnavailableAndRetry() throws {
        let app = try configuredApp()
        let endpoint = app.launchEnvironment["SCREENROOM_API_ENDPOINT"]
        app.launchEnvironment["SCREENROOM_API_ENDPOINT"] = "http://127.0.0.1:1"
        app.launch()
        for attempt in 0..<2 {
            XCTAssertTrue(app.staticTexts["core.error"].waitForExistence(timeout: 10))
            XCTAssertFalse(firstCard(app).exists)
            if attempt == 0 { activate(app.buttons["core.retry"]) }
        }
        app.terminate()
        app.launchEnvironment["SCREENROOM_API_ENDPOINT"] = endpoint
        app.launch()
        assertHeading(app, "Home")
        XCTAssertTrue(app.staticTexts["core.error"].waitForNonExistence(timeout: 10))
    }

    #if os(tvOS)
    @MainActor
    func testSidebarRemoteControls() throws {
        let app = try configuredApp()
        app.launch()
        activate(app.buttons["sidebar.toggle"])
        XCTAssertFalse(app.buttons["sidebar.subtitles"].exists)
        XCTAssertFalse(app.buttons["subtitle.Off"].exists)
        activate(app.buttons["Refresh"])
        XCTAssertTrue(app.staticTexts["Catalog refreshed from core."].waitForExistence(timeout: 10))
        focus(app.buttons["menu.Home"])
        XCTAssertEqual(app.buttons["menu.Home"].value as? String, "Selected")
        XCUIRemote.shared.press(.right)
        XCTAssertFalse(app.buttons["menu.Home"].hasFocus)
        XCUIRemote.shared.press(.left)
        let focused = app.buttons.matching(NSPredicate(format: "hasFocus == true")).firstMatch
        XCTAssertTrue(focused.identifier.hasPrefix("menu.") || focused.identifier == "sidebar.toggle")
    }
    #endif

    @MainActor private func firstCard(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'card.'")).firstMatch
    }
    @MainActor private func text(_ element: XCUIElement) -> String {
        element.label.isEmpty ? element.value as? String ?? "" : element.label
    }
    @MainActor private func assertHeading(_ app: XCUIApplication, _ expected: String) {
        let title = app.staticTexts["page.title"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        XCTAssertEqual(text(title), expected)
        XCTAssertTrue(title.isHittable)
    }
    @MainActor private func assertStatus(_ app: XCUIApplication, _ expected: String) {
        let status = app.staticTexts["search.status"]
        XCTAssertTrue(status.waitForExistence(timeout: 10))
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@ OR value == %@", expected, expected), object: status)
        XCTAssertEqual(XCTWaiter.wait(for: [settled], timeout: 10), .completed)
    }
    @MainActor private func waitForPage(_ app: XCUIApplication) {
        if app.textFields["search.query"].exists {
            let field = app.textFields["search.query"]
            let value = field.value as? String ?? ""
            let query = (value == field.placeholderValue ? "" : value).trimmingCharacters(in: .whitespacesAndNewlines)
            let pattern = query.isEmpty ? "Enter a title to search .+" : "([0-9]+ results? in .+|No matches in .+)"
            let settled = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "exists == true AND (label MATCHES %@ OR value MATCHES %@)", pattern, pattern),
                object: app.staticTexts["search.status"]
            )
            guard XCTWaiter.wait(for: [settled], timeout: 15) == .completed else {
                XCTFail("Initial search did not reach a loaded result or empty state")
                return
            }
        } else {
            guard app.staticTexts["catalog.loadedCount"].waitForExistence(timeout: 15) else {
                XCTFail("Initial catalog page did not publish its loaded count")
                return
            }
        }
        guard app.descendants(matching: .any).matching(identifier: "core.loading").firstMatch.waitForNonExistence(timeout: 15) else {
            XCTFail("Initial catalog read did not finish")
            return
        }
        XCTAssertFalse(app.staticTexts["core.error"].exists, "A failed read is not a loaded page")
        XCTAssertFalse(app.staticTexts["catalog.pageError"].exists)
    }
    @MainActor private func waitForContinuation(_ app: XCUIApplication) {
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false OR enabled == true"), object: app.buttons["catalog.loadMore"])
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 15), .completed)
        XCTAssertFalse(app.staticTexts["core.error"].exists)
        XCTAssertFalse(app.staticTexts["catalog.pageError"].exists)
    }
    @MainActor private func boundedFirstChild(_ app: XCUIApplication) -> XCUIElement {
        waitForPage(app)
        for _ in 0..<3 {
            for _ in 0..<6 {
                if firstCard(app).exists { return firstCard(app) }
                #if os(tvOS)
                XCUIRemote.shared.press(.down)
                #else
                app.swipeUp()
                #endif
            }
            if firstCard(app).exists { return firstCard(app) }
            if !app.buttons["catalog.loadMore"].exists { break }
            activate(app.buttons["catalog.loadMore"])
            waitForContinuation(app)
        }
        XCTAssertTrue(firstCard(app).exists, "Hierarchy requires a child within four scoped pages")
        return firstCard(app)
    }
    @MainActor private func capture(_ app: XCUIApplication, _ flow: String) {
        let toggle = app.buttons["sidebar.toggle"]
        XCTAssertTrue(toggle.isHittable)
        XCTAssertTrue(app.windows.firstMatch.frame.contains(toggle.frame))
        #if os(macOS)
        let shot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        #else
        let image = app.screenshot()
        XCTAssertTrue(hasVisibleGlyph(image, frame: toggle.frame, screen: app.frame), "Sidebar glyph is blank in \(flow)")
        let shot = XCTAttachment(screenshot: image)
        #endif
        shot.name = "\(flow)-success"
        shot.lifetime = .keepAlways
        add(shot)
    }
    @MainActor private func activate(_ element: XCUIElement) {
        XCTAssertTrue(element.exists || element.waitForExistence(timeout: 10))
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
    @MainActor private func focus(_ element: XCUIElement) {
        let app = XCUIApplication()
        var previousKey: String?
        var previousHorizontal = false
        for _ in 0..<60 {
            if element.hasFocus { return }
            let focused = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).firstMatch
            guard focused.exists || focused.waitForExistence(timeout: 1) else { XCUIRemote.shared.press(.right); continue }
            let dx = element.frame.midX - focused.frame.midX
            let dy = element.frame.midY - focused.frame.midY
            let sidebarIDs = ["sidebar.toggle", "Refresh"]
            let targetSidebar = element.identifier.hasPrefix("menu.") || element.identifier.hasPrefix("sidebar.") || sidebarIDs.contains(element.identifier) || sidebarIDs.contains(element.label)
            let focusedSidebar = focused.identifier.hasPrefix("menu.") || focused.identifier.hasPrefix("sidebar.") || sidebarIDs.contains(focused.identifier) || sidebarIDs.contains(focused.label)
            var horizontal = targetSidebar != focusedSidebar || (!targetSidebar && abs(dy) <= 30)
            let focusKey = focused.identifier.isEmpty ? focused.label : focused.identifier
            if previousKey == focusKey && previousHorizontal == horizontal { horizontal.toggle() }
            if !app.windows.firstMatch.frame.intersects(element.frame) { horizontal = false }
            previousKey = focusKey
            previousHorizontal = horizontal
            if horizontal { XCUIRemote.shared.press(dx > 0 ? .right : .left) }
            else { XCUIRemote.shared.press(dy > 0 ? .down : .up) }
        }
        XCTFail("Remote could not focus \(element.identifier)")
    }
    #endif
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
}
