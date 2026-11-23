import XCTest

enum PlaybackCapturePolicy {
    struct ToolbarItem: Equatable {
        let control: String
        let enabled: Bool
        let frame: CGRect
    }

    static let televisionToolbar: Set<String> = ["play", "backward", "forward", "mute", "volume", "tracks", "session"]

    static func immediateRight(from: String, to: String, before: [ToolbarItem], current: [ToolbarItem]) -> Bool {
        guard before == current, current.count == televisionToolbar.count,
              Set(current.map(\.control)) == televisionToolbar,
              current.allSatisfy({ item in
                  !item.frame.isNull && !item.frame.isInfinite && item.frame.size.width > 0 && item.frame.size.height > 0 &&
                  [item.frame.minX, item.frame.minY, item.frame.maxX, item.frame.maxY].allSatisfy { $0.isFinite && abs($0) <= 1_000_000 }
              }),
              let origin = current.first(where: { $0.control == from }), origin.enabled,
              current.allSatisfy({ abs($0.frame.midY - origin.frame.midY) <= 1 && abs($0.frame.height - origin.frame.height) <= 1 })
        else { return false }
        let row = current.filter(\.enabled).sorted { $0.frame.minX < $1.frame.minX }
        guard let index = row.firstIndex(where: { $0.control == from }), index + 1 < row.count,
              row[index + 1].control == to, row[index].frame.maxX < row[index + 1].frame.minX else { return false }
        return zip(row, row.dropFirst()).allSatisfy { $0.frame.maxX < $1.frame.minX }
    }

    static func uniqueMenuButtonIndex(_ observations: [(label: String, enabled: Bool)], labels: Set<String>) -> Int? {
        guard observations.count == 1, labels.contains(observations[0].label), observations[0].enabled else { return nil }
        return 0
    }

    enum RemoteDirection: String { case up, down, left, right }

    static func remoteDirection(target: CGRect, focused: CGRect) -> RemoteDirection? {
        for frame in [target, focused] {
            guard !frame.isNull, !frame.isInfinite, frame.size.width > 0, frame.size.height > 0,
                  [frame.minX, frame.minY, frame.maxX, frame.maxY].allSatisfy(\.isFinite) else { return nil }
        }
        if target.minY > focused.maxY + 1 { return .down }
        if target.maxY < focused.minY - 1 { return .up }
        let dx = target.midX - focused.midX
        let dy = target.midY - focused.midY
        guard dx.isFinite && dy.isFinite else { return nil }
        if abs(dx) > 1 { return dx > 0 ? .right : .left }
        if abs(dy) > 1 { return dy > 0 ? .down : .up }
        return nil
    }

    static func settingsNamespace(runID: String) -> String? {
        guard !runID.isEmpty else { return nil }
        let namespace = "showcase-\(runID)"
        return namespace.utf8.count <= 64 ? namespace : nil
    }

    static func seekCorrelationReceipt(_ value: Any?) -> [String: Any]? {
        guard let text = value as? String, text.utf8.count <= 16384, let data = text.data(using: .utf8),
              let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              Set(root.keys) == ["schema_version", "records"],
              let version = diagnosticNumber(root["schema_version"]), version == 1,
              let records = root["records"] as? [[String: Any]], !records.isEmpty, records.count <= 24 else { return nil }
        var stages: [Int: String] = [:]
        for record in records {
            guard let operation = diagnosticNumber(record["operation"]), (1...8).contains(operation), operation.rounded() == operation,
                  let stage = record["stage"] as? String else { return nil }
            let extra: Set<String>
            switch stage {
            case "slider-ended":
                guard stages[Int(operation)] == nil else { return nil }
                extra = ["slider_received_ms"]
            case "seek-dispatch":
                guard stages[Int(operation)] == "slider-ended" else { return nil }
                extra = ["media_requested_ms"]
            case "seek-completion":
                guard stages[Int(operation)] == "seek-dispatch" else { return nil }
                extra = ["media_requested_ms", "media_actual_ms", "logical_actual_ms"]
            case "timeline-update":
                guard stages[Int(operation)] == "slider-ended" else { return nil }
                extra = []
            default: return nil
            }
            let numeric = extra.union(["logical_target_ms", "transport_offset_ms"])
            guard Set(record.keys) == numeric.union(["operation", "stage"]),
                  numeric.allSatisfy({ key in
                      guard let number = diagnosticNumber(record[key]) else { return false }
                      return (0...Double(922_337_203_685_477 as Int64)).contains(number)
                  }) else { return nil }
            stages[Int(operation)] = stage
        }
        return root
    }

    private static func diagnosticNumber(_ value: Any?) -> Double? {
        guard let number = value as? NSNumber, String(cString: number.objCType) != "c",
              number.doubleValue.isFinite else { return nil }
        return number.doubleValue
    }

    static func seekCorrelationComplete(_ value: [String: Any]?) -> Bool {
        guard let records = value?["records"] as? [[String: Any]], records.count == 3 else { return false }
        return records.compactMap { $0["stage"] as? String } == ["slider-ended", "seek-dispatch", "seek-completion"] &&
            Set(records.compactMap { ($0["operation"] as? NSNumber)?.intValue }).count == 1
    }

    struct FullscreenStep: Equatable {
        let toggle: Bool
        let entered: Bool
        let observation: String
    }

    static func fullscreenPlan(label: String?, enabled: Bool) -> [FullscreenStep]? {
        guard enabled, let label, ["Fullscreen", "Exit fullscreen"].contains(label) else { return nil }
        let fullscreen = FullscreenStep(toggle: label == "Fullscreen", entered: true, observation: label == "Fullscreen"
            ? "A unique enabled app-owned Exit fullscreen control confirms fullscreen presentation after one enter toggle."
            : "The unique enabled app-owned control already read Exit fullscreen before any fullscreen request. This captures the current fullscreen state, not an entry transition.")
        return [fullscreen, FullscreenStep(toggle: true, entered: false,
            observation: "A unique enabled app-owned Fullscreen control confirms non-fullscreen presentation after one exit toggle.")]
    }

    static func fullscreenChoice(_ controls: [(identifier: String, label: String, hittable: Bool)], menu: String?) -> Int? {
        let direct = controls.indices.filter { controls[$0].identifier == "playback.fullscreen" && controls[$0].hittable }
        if !direct.isEmpty { return direct.count == 1 ? direct[0] : nil }
        guard menu == "playback.menu.session" else { return nil }
        let choices = controls.indices.filter {
            controls[$0].hittable && ["Fullscreen", "Exit fullscreen"].contains(controls[$0].label)
        }
        return choices.count == 1 ? choices[0] : nil
    }

    enum TimelineValue: Equatable {
        case native(Double), seconds(Double), percent(Double)
    }

    static func timelineValue(_ value: Any?) -> TimelineValue? {
        if let number = value as? NSNumber {
            let value = number.doubleValue
            guard String(cString: number.objCType) != "c", value.isFinite, value >= 0 else { return nil }
            return .native(value)
        }
        guard let text = (value as? String)?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        if text.hasSuffix("%"), let value = Double(text.dropLast()), value.isFinite, (0...100).contains(value) {
            return .percent(value)
        }
        let parts = text.split(separator: ":", omittingEmptySubsequences: false)
        if (2...3).contains(parts.count), parts.allSatisfy({ !$0.isEmpty && $0.allSatisfy(\.isNumber) }),
           parts.dropFirst().allSatisfy({ $0.count == 2 && (Int($0) ?? 60) < 60 }) {
            let value = parts.reduce(0.0) { $0 * 60 + (Double($1) ?? .infinity) }
            return value.isFinite ? .seconds(value) : nil
        }
        if let value = Double(text), value.isFinite, value >= 0 { return .native(value) }
        return nil
    }

    static func timelineChanged(_ before: TimelineValue, _ after: TimelineValue) -> Bool {
        switch (before, after) {
        case let (.native(a), .native(b)), let (.seconds(a), .seconds(b)), let (.percent(a), .percent(b)):
            return abs(a - b) > 0.000001
        default: return false
        }
    }

    static func timelineAdvanced(_ before: TimelineValue, _ after: TimelineValue) -> Bool {
        switch (before, after) {
        case let (.native(a), .native(b)), let (.seconds(a), .seconds(b)), let (.percent(a), .percent(b)):
            return b - a > 0.000001
        default: return false
        }
    }

    static let timelinePairPattern = #"^\d+:\d{2}(?::\d{2})?(?:\s*[,/]\s*|\s+)\d+:\d{2}(?::\d{2})?$"#

    static func elapsedTime(_ label: String) -> TimelineValue? {
        guard label.range(of: timelinePairPattern, options: .regularExpression) != nil,
              let elapsed = label.split(whereSeparator: { $0 == "," || $0 == "/" || $0.isWhitespace }).first else { return nil }
        return timelineValue(String(elapsed))
    }

    struct ClockPosition: Equatable {
        let elapsed: Double
        let duration: Double
    }

    struct AbsoluteSeekPlan: Equatable {
        let before: ClockPosition
        let normalizedTarget: Double
        var targetSeconds: Double { before.duration * normalizedTarget }
    }

    static func clockPosition(elapsed: TimelineValue?, duration: TimelineValue?) -> ClockPosition? {
        guard case let .seconds(elapsed) = elapsed, case let .seconds(duration) = duration,
              elapsed.isFinite, duration.isFinite, duration > 0, elapsed >= 0, elapsed <= duration else { return nil }
        return ClockPosition(elapsed: elapsed, duration: duration)
    }

    static func clockPosition(_ label: String?) -> ClockPosition? {
        guard let label, label.range(of: timelinePairPattern, options: .regularExpression) != nil else { return nil }
        let parts = label.split(whereSeparator: { $0 == "," || $0 == "/" || $0.isWhitespace })
        guard parts.count == 2 else { return nil }
        return clockPosition(elapsed: timelineValue(String(parts[0])), duration: timelineValue(String(parts[1])))
    }

    static func uniqueClockPosition(_ labels: [String]) -> ClockPosition? {
        guard labels.count == 1 else { return nil }
        return clockPosition(labels[0])
    }

    static let clockIdentifier = "playback.time"

    struct ClockField {
        let identifier: String
        let role: UInt
        let label: Any?
        let value: Any?
    }

    enum ClockFieldType: String {
        case missing, string, number, other
    }

    static func clockFieldType(_ value: Any?) -> ClockFieldType {
        guard let value else { return .missing }
        if value is String { return .string }
        if value is NSNumber { return .number }
        return .other
    }

    static func clockFieldPosition(_ value: Any?) -> ClockPosition? {
        guard let text = value as? String, text.utf8.count <= 64 else { return nil }
        return clockPosition(text)
    }

    static func identifiedClockPosition(_ observations: [ClockField]) -> ClockPosition? {
        guard observations.count == 1, observations[0].identifier == clockIdentifier else { return nil }
        let field = observations[0]
        guard [.missing, .string].contains(clockFieldType(field.label)),
              [.missing, .string].contains(clockFieldType(field.value)) else { return nil }
        let label = clockFieldPosition(field.label)
        let value = clockFieldPosition(field.value)
        if let label, let value { return label == value ? label : nil }
        return label ?? value
    }

    static func clockFieldsReceipt(count: Int, fields: [ClockField]) -> [String: Any] {
        func numeric(_ clock: ClockPosition?) -> Any {
            guard let clock else { return NSNull() }
            return ["elapsed_seconds": clock.elapsed, "duration_seconds": clock.duration]
        }
        let clock = count == 1 ? identifiedClockPosition(fields) : nil
        return [
            "schema_version": 1, "match_count": count, "clock": numeric(clock),
            "fields": fields.prefix(1).map { field -> [String: Any] in
                ["exact_identifier": field.identifier == clockIdentifier, "role": field.role,
                 "label_type": clockFieldType(field.label).rawValue, "value_type": clockFieldType(field.value).rawValue,
                 "label_clock": numeric(clockFieldPosition(field.label)), "value_clock": numeric(clockFieldPosition(field.value))]
            }
        ]
    }

    static let stopIdentifier = "playback.status"

    struct StopField {
        let identifier: String
        let role: UInt
        let label: Any?
        let value: Any?
    }

    enum StopOutcome: String {
        case stopped, providerPending = "provider-pending", personalUnconfirmed = "personal-unconfirmed"
        case serverUnconfirmed = "server-unconfirmed"
    }

    struct StopObservation: Equatable {
        let outcome: StopOutcome
        let earlierPersonalReportUnknown: Bool

        var cleanupEvidence: String {
            let evidence: String
            switch outcome {
            case .stopped: evidence = "Visible status reports Playback stopped without a provider cleanup warning; provider cleanup is not independently confirmed."
            case .providerPending: evidence = "Stopped locally; visible status reports provider cleanup pending."
            case .personalUnconfirmed: evidence = "Stopped locally; visible status reports an unconfirmed final personal report."
            case .serverUnconfirmed: evidence = "Stopped locally; visible status reports unconfirmed server cleanup."
            }
            return evidence + (earlierPersonalReportUnknown ? " An earlier personal-report outcome remains unknown." : "")
        }
    }

    static func stopObservation(_ value: Any?) -> StopObservation? {
        guard var text = value as? String, text.utf8.count <= 512 else { return nil }
        let suffix = " An earlier personal-report outcome remains unknown."
        let earlierUnknown = text.hasSuffix(suffix)
        if earlierUnknown { text.removeLast(suffix.count) }
        let outcome: StopOutcome
        switch text {
        case "Playback stopped.": outcome = .stopped
        case "Playback stopped locally; provider cleanup is pending.": outcome = .providerPending
        case "Playback stopped locally; the final personal report is not confirmed. Check session state or retry the identical Stop request.":
            outcome = .personalUnconfirmed
        case "Playback stopped locally. Server cleanup is unconfirmed. Retry Stop uses the same final position and request.":
            guard !earlierUnknown else { return nil }
            outcome = .serverUnconfirmed
        default: return nil
        }
        return StopObservation(outcome: outcome, earlierPersonalReportUnknown: earlierUnknown)
    }

    static func identifiedStopOutcome(_ fields: [StopField]) -> StopObservation? {
        guard fields.count == 1, fields[0].identifier == stopIdentifier else { return nil }
        let field = fields[0]
        guard [.missing, .string].contains(clockFieldType(field.label)),
              [.missing, .string].contains(clockFieldType(field.value)) else { return nil }
        let label = stopObservation(field.label)
        let value = stopObservation(field.value)
        if let text = field.label as? String, !text.isEmpty && label == nil { return nil }
        if let text = field.value as? String, !text.isEmpty && value == nil { return nil }
        if let label, let value { return label == value ? label : nil }
        return label ?? value
    }

    static func stopFieldsReceipt(count: Int, fields: [StopField], gatewayProgress: Bool) -> [String: Any] {
        func safe(_ observation: StopObservation?) -> Any {
            guard let observation else { return NSNull() }
            return ["outcome": observation.outcome.rawValue,
                    "earlier_personal_report_unknown": observation.earlierPersonalReportUnknown] as [String: Any]
        }
        return [
            "schema_version": 1, "match_count": count, "gateway_progress": gatewayProgress,
            "observation": safe(count == 1 ? identifiedStopOutcome(fields) : nil),
            "fields": fields.prefix(1).map { field -> [String: Any] in
                ["exact_identifier": field.identifier == stopIdentifier, "role": field.role,
                 "label_type": clockFieldType(field.label).rawValue, "value_type": clockFieldType(field.value).rawValue,
                 "label_empty": (field.label as? String) == "", "value_empty": (field.value as? String) == "",
                 "label_outcome": safe(stopObservation(field.label)), "value_outcome": safe(stopObservation(field.value))]
            }
        ]
    }

    static func absoluteSeekPlan(_ clock: ClockPosition?) -> AbsoluteSeekPlan? {
        guard let clock, let valid = clockPosition(elapsed: .seconds(clock.elapsed), duration: .seconds(clock.duration)) else { return nil }
        let fraction = valid.elapsed / valid.duration
        let target = abs(fraction - 0.2) <= 0.1 ? 0.4 : 0.2
        guard abs(target - fraction) >= 0.1, abs(target * valid.duration - valid.elapsed) >= 2 else { return nil }
        return AbsoluteSeekPlan(before: valid, normalizedTarget: target)
    }

    static func absoluteSeekSettled(before: ClockPosition, after: ClockPosition?, playEnabled: Bool,
                                    playLabel: String?, gatewayProgress: Bool) -> Bool {
        guard let before = clockPosition(elapsed: .seconds(before.elapsed), duration: .seconds(before.duration)),
              let after = after.flatMap({ clockPosition(elapsed: .seconds($0.elapsed), duration: .seconds($0.duration)) }),
              before.duration == after.duration else { return false }
        return timelineChanged(.seconds(before.elapsed), .seconds(after.elapsed)) &&
            playEnabled && playLabel == "Play" && !gatewayProgress
    }

    static func seekStatus(_ label: String?) -> String {
        guard let label else { return "absent" }
        switch label {
        case "": return "empty"
        case "Preparing playback…": return "preparing"
        case "Playback stopped.": return "stopped"
        case "Playback stopped locally; provider cleanup is pending.": return "stopped-provider-pending"
        case "The gateway could not prepare playback. Retry uses the identical request.": return "gateway-preparation-error"
        case "Playback could not be confirmed. Retry preparation uses the identical request; it never starts a second session.": return "preparation-unconfirmed"
        default: return "redacted-unrecognized"
        }
    }

    static func absoluteSeekReceipt(clock: ClockPosition?, sliderValue: Any?, requested: AbsoluteSeekPlan?,
                                    playExists: Bool, playEnabled: Bool, playLabel: String?,
                                    sliderEnabled: Bool, gatewayProgress: Bool, status: String?, preparation: Any?) -> [String: Any] {
        var receipt: [String: Any] = [
            "schema_version": 1, "play_exists": playExists, "play_enabled": playEnabled,
            "play_label": ["Play", "Pause"].contains(playLabel ?? "") ? playLabel! : "unknown",
            "slider_enabled": sliderEnabled, "gateway_progress": gatewayProgress, "status": seekStatus(status)
        ]
        if let clock = clock.flatMap({ clockPosition(elapsed: .seconds($0.elapsed), duration: .seconds($0.duration)) }) {
            receipt["clock"] = ["elapsed_seconds": clock.elapsed, "duration_seconds": clock.duration, "normalized": clock.elapsed / clock.duration]
        } else { receipt["clock"] = NSNull() }
        if case let .seconds(value) = timelineValue(sliderValue) {
            receipt["slider_clock_seconds"] = value
        } else { receipt["slider_clock_seconds"] = NSNull() }
        if let requested, absoluteSeekPlan(requested.before) == requested {
            receipt["request"] = ["normalized": requested.normalizedTarget, "target_seconds": requested.targetSeconds]
        } else { receipt["request"] = NSNull() }
        receipt["last_preparation_receipt"] = preparationReceipt(preparation) ?? [:]
        return receipt
    }

    static func normalExitAllowed(owned: Bool, attempted: Bool) -> Bool { owned && !attempted }

    static func trackNeedsSelection(selected: Bool) -> Bool { !selected }

    static func trackSettled(selected: Bool, enabled: Bool, transportBusy: Bool) -> Bool {
        selected && enabled && !transportBusy
    }

    static func volumePercent(_ label: String) -> Int? {
        guard label.hasPrefix("Volume "), label.hasSuffix("%"),
              let value = Int(label.dropFirst(7).dropLast()), (0...100).contains(value) else { return nil }
        return value
    }

    static func volumeReadout(label: String, value: Any?) -> Int? {
        let values = Set([volumePercent(label), (value as? String).flatMap(volumePercent)].compactMap { $0 })
        return values.count == 1 ? values.first : nil
    }

    static func uniqueVolumeReadout(_ readings: [(String, Any?)]) -> Int? {
        guard readings.count == 1 else { return nil }
        return volumeReadout(label: readings[0].0, value: readings[0].1)
    }

    static func volumeSettled(actual: Int?, target: Int?, previous: Int?, controlsEnabled: Bool,
                              menuVisible: Bool, transportBusy: Bool) -> Bool {
        guard let actual else { return false }
        return (target == nil || actual == target) && (previous == nil || actual != previous) &&
            controlsEnabled && menuVisible && !transportBusy
    }

    static func volumeAction(current: Int, target: Int) -> String? {
        current == target ? nil : (current < target ? "Louder" : "Quieter")
    }

    static func preparationReceipt(_ value: Any?) -> [String: String]? {
        guard let value = value as? String else { return nil }
        let tokens = value.split(separator: " ")
        guard tokens.first == "ScreenroomPlaybackPreparation", tokens.count == 7 else { return nil }
        var fields: [String: String] = [:]
        for token in tokens.dropFirst() {
            let pair = token.split(separator: "=", omittingEmptySubsequences: false)
            guard pair.count == 2, fields[String(pair[0])] == nil else { return nil }
            fields[String(pair[0])] = String(pair[1])
        }
        let stages = ["audio-session", "create", "accept", "install", "done"]
        let codes = ["none", "untyped", "unknown", "profile_required", "profile_changed", "source_unavailable",
                     "item_unavailable", "unsupported_media", "transcoding_unavailable", "playback_authorization_unavailable",
                     "playback_conflict", "session_expired", "operation_conflict", "capacity_exhausted", "upstream_unavailable"]
        guard Set(fields.keys) == Set(["phase", "stage", "code", "blocked", "closed", "session"]),
              let phase = fields["phase"], (stages + ["failure"]).contains(phase),
              let stage = fields["stage"], stages.contains(stage),
              let code = fields["code"], codes.contains(code),
              ["blocked", "closed", "session"].allSatisfy({ ["true", "false"].contains(fields[$0] ?? "") }),
              (phase == "failure" ? code != "none" : code == "none") else { return nil }
        return fields
    }
}

final class PlayerScreenshots: XCTestCase, @unchecked Sendable {
    private var states: [[String: String]] = []
    private var viewport = "unknown"
    private var ownsPlayback = false
    private var exitAttempted = false
    private var clockReceiptsEnabled = false
    private var remoteFocusReceiptsEnabled = false
    private var remoteFocusReceiptIndex = 0

    @MainActor
    func testNosferatuPlayerStates() throws {
        try runJourney(volumeOnly: false)
    }

    @MainActor
    func testNosferatuVolumeControls() throws {
        try runJourney(volumeOnly: true)
    }

    @MainActor
    func testNosferatuPreparation() throws {
        try runJourney(volumeOnly: false, preparationOnly: true)
    }

    @MainActor
    func testNosferatuAbsoluteSeek() throws {
        try runJourney(volumeOnly: false, absoluteSeekOnly: true)
    }

    @MainActor
    func testNosferatuTelevisionFocus() throws {
        try runJourney(volumeOnly: false, remoteFocusOnly: true)
    }

    @MainActor
    private func runJourney(volumeOnly: Bool, preparationOnly: Bool = false, absoluteSeekOnly: Bool = false, remoteFocusOnly: Bool = false) throws {
        let env = ProcessInfo.processInfo.environment
        guard env["SCREENROOM_NATIVE_PLAYBACK_RELEASED"] == "1",
              env["SCREENROOM_E2E_JOURNEY"] == "playback",
              (!volumeOnly && !preparationOnly && !absoluteSeekOnly && !remoteFocusOnly) || env["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1",
              !absoluteSeekOnly || env["SCREENROOM_E2E_ABSOLUTE_SEEK_DIAGNOSTIC"] == "1",
              !remoteFocusOnly || env["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] == "1" else {
            throw XCTSkip("Explicit playback journey and coordinator release required")
        }
        #if os(macOS)
        if volumeOnly { XCTAssertEqual(env["SCREENROOM_MACOS_GUI_READY"], "1") }
        #endif
        continueAfterFailure = false
        clockReceiptsEnabled = volumeOnly || preparationOnly || absoluteSeekOnly || remoteFocusOnly
        remoteFocusReceiptsEnabled = remoteFocusOnly
        let endpoint = try XCTUnwrap(env["SCREENROOM_API_ENDPOINT"])
        let item = try XCTUnwrap(env["SCREENROOM_SHOWCASE_ITEM_ID"])
        let title = try XCTUnwrap(env["SCREENROOM_SHOWCASE_TITLE"])
        let year = try XCTUnwrap(Int(try XCTUnwrap(env["SCREENROOM_SHOWCASE_YEAR"])))
        let instance = try XCTUnwrap(env["SCREENROOM_BACKEND_INSTANCE_ID"])
        let profile = try XCTUnwrap(env["SCREENROOM_SHOWCASE_PLATFORM"])
        let runID = try XCTUnwrap(env["SCREENROOM_SHOWCASE_RUN_ID"])
        let settingsNamespace = try XCTUnwrap(PlaybackCapturePolicy.settingsNamespace(runID: runID),
                                              "Capture settings namespace requires a nonempty run ID and at most 64 UTF-8 bytes")
        XCTAssertTrue(["ios", "ipad", "tvos", "macos"].contains(profile))
        if preparationOnly || absoluteSeekOnly { XCTAssertEqual(profile, "ios") }
        if remoteFocusOnly { XCTAssertEqual(profile, "tvos") }
        let app = XCUIApplication()
        XCTAssertFalse(app.buttons["playback.session"].exists || app.buttons["Show playback controls"].exists,
                       "Preserve pre-existing playback; coordinator must release it first")
        app.launchEnvironment["SCREENROOM_API_ENDPOINT"] = endpoint
        app.launchEnvironment["SCREENROOM_USER_ID"] = try XCTUnwrap(env["SCREENROOM_USER_ID"])
        app.launchEnvironment["SCREENROOM_E2E_SETTINGS_NAMESPACE"] = settingsNamespace
        if volumeOnly || preparationOnly || absoluteSeekOnly || remoteFocusOnly { app.launchEnvironment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] = "1" }
        if remoteFocusOnly { app.launchEnvironment["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] = "1" }
        app.launch()
        XCTAssertTrue(app.buttons["sidebar.toggle"].waitForExistence(timeout: 20))
        let card = app.buttons["card.\(item)"]
        if !card.waitForExistence(timeout: 10) {
            activate(app.buttons["menu.Search titles"], in: app)
            let query = app.textFields["search.query"]
            XCTAssertTrue(query.waitForExistence(timeout: 10))
            activate(app.buttons["search.options"], in: app)
            activate(app.buttons["Title only"], in: app)
            activate(app.buttons["search.options"], in: app)
            activate(query, in: app)
            query.typeText("\(title)\n")
            #if os(tvOS)
            if app.keyboards.firstMatch.exists { XCUIRemote.shared.press(.menu) }
            #endif
        }
        XCTAssertTrue(card.waitForExistence(timeout: 20), "Explicit selected item must exist")
        activate(card, in: app)
        XCTAssertTrue(app.buttons["detail.play"].waitForExistence(timeout: 20))
        XCTAssertEqual(app.staticTexts["page.title"].label, title, "Selected item must match the explicit showcase title")
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ OR value CONTAINS %@", "\(year)", "\(year)")).firstMatch.exists)
        capture(app, "detail", "Selected title", "Explicit item card opened; requested year visible in details.")
        activate(app.buttons["detail.play"], in: app)
        let created = app.buttons["playback.session"].waitForExistence(timeout: 30)
        if created {
            ownsPlayback = true
            addTeardownBlock { @MainActor in
                self.failureExit(app)
            }
        }
        XCTAssertTrue(created)
        if remoteFocusOnly {
            try observeTelevisionFocus(app)
            return
        }
        if preparationOnly {
            observePreparation(app)
            return
        }
        if absoluteSeekOnly {
            observeAbsoluteSeek(app)
            return
        }
        let loading = app.progressIndicators.matching(NSPredicate(format: "label == 'Contacting playback gateway'")).firstMatch
        if loading.exists {
            capture(app, "loading", "Loading", "Playback gateway progress indicator is visible.")
        } else {
            missing("loading", "Loading", "not-observed", "Transient loading indicator was not observed.")
        }
        let play = app.buttons["playback.play"]
        XCTAssertTrue(play.waitForExistence(timeout: 20))
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: play)
        let ready = XCTWaiter.wait(for: [settled], timeout: 25) == .completed
        if volumeOnly { XCTAssertTrue(ready, "Focused volume reproduction requires usable playback") }
        var blocked = false
        if !ready {
            let status = app.staticTexts["playback.status"]
            XCTAssertTrue(status.exists && !status.label.isEmpty, "Disabled control without visible natural error is a harness failure")
            XCTAssertFalse(loading.exists, "A gateway timeout is not a settled media error")
            XCTAssertTrue(naturalError(in: app),
                          "Unexpected/transport preparation errors must fail the suite, not become successful captures")
            capture(app, "error", "Natural playback error", "Playback control remained disabled and a nonempty playback status notice is visible. No recovery request was sent.")
            blocked = true
            for id in ["playing", "paused", "idle", "seek-forward", "seek-backward", "seek-absolute", "volume", "volume-minimum", "volume-midpoint", "volume-maximum", "muted", "unmuted", "tracks", "subtitles-off", "fullscreen", "windowed"] {
                missing(id, id.replacingOccurrences(of: "-", with: " "), "blocked", "Visible natural playback error prevents observing this state.")
            }
        } else {
            if play.label == "Play" { activate(play, in: app) }
            XCTAssertTrue(wait(play, "label == 'Pause' AND enabled == true"))
            capture(app, "playing", "Playing", "Enabled Pause control confirms the player reports playing; decoded-frame quality is not asserted.")
            if !volumeOnly {
            let hidden = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.buttons["playback.session"])
            if XCTWaiter.wait(for: [hidden], timeout: 8) == .completed {
                capture(app, "idle", "Controls hidden", "Playback settings control disappeared during uninterrupted playback.")
            } else { missing("idle", "Controls hidden", "not-observed", "Controls did not auto-hide within the observation window.") }
            }
            reveal(app)
            activate(play, in: app)
            XCTAssertTrue(wait(play, "label == 'Play' AND enabled == true"))
            capture(app, "paused", "Paused", "Enabled Play control confirms paused state.")
            if volumeOnly { attachClockFields("volume-paused", in: app) }
            if !volumeOnly {
            for (id, control) in [("seek-forward", "playback.forward"), ("seek-backward", "playback.backward")] {
                var button = app.buttons[control]
                if !button.exists {
                    openMenu("session", in: app)
                    button = app.buttons[id == "seek-forward" ? "Forward 30 seconds" : "Back 30 seconds"]
                }
                #if os(tvOS)
                let position = app.progressIndicators["Playback position"]
                #else
                let position = app.sliders["playback.position"]
                #endif
                if button.exists && button.isEnabled && position.exists {
                    let before = try XCTUnwrap(timelineReading(position, in: app), "Timeline must expose a native number, time value, or readable elapsed/duration text")
                    activate(button, in: app)
                    XCTAssertTrue(waitForSeek(position, from: before, in: app), "Seek must visibly change the actual timeline while paused")
                    capture(app, id, id, "Native timeline value or readable elapsed time changed after the named seek control; the player remains paused.")
                } else { missing(id, id, "unavailable", "Seek control or readable timeline is not exposed in this layout.") }
                closeMenu(app)
            }
            #if !os(tvOS)
            let position = app.sliders["playback.position"]
            if position.exists && position.isEnabled {
                let before = try XCTUnwrap(absoluteClock(in: app), "Adjustable timeline requires one actual elapsed/duration clock; unknown units cannot authorize input")
                let plan = try XCTUnwrap(PlaybackCapturePolicy.absoluteSeekPlan(before), "Actual clock must support a well-separated slider target")
                let progress = app.progressIndicators["Contacting playback gateway"]
                XCTAssertTrue(play.exists && play.isEnabled && play.label == "Play" && !progress.exists,
                              "Absolute seek requires settled paused controls before the single request")
                position.adjust(toNormalizedSliderPosition: plan.normalizedTarget)
                let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                    PlaybackCapturePolicy.absoluteSeekSettled(
                        before: plan.before, after: self.absoluteClock(in: app), playEnabled: play.exists && play.isEnabled,
                        playLabel: play.exists ? play.label : nil, gatewayProgress: progress.exists)
                }, object: app)
                XCTAssertEqual(XCTWaiter.wait(for: [settled], timeout: 15), .completed,
                               "One separated slider adjustment must change the actual settled paused clock")
                let observed = try XCTUnwrap(absoluteClock(in: app), "Actual clock must remain readable before capture")
                XCTAssertTrue(PlaybackCapturePolicy.absoluteSeekSettled(
                    before: before, after: observed, playEnabled: play.exists && play.isEnabled,
                    playLabel: play.exists ? play.label : nil, gatewayProgress: progress.exists),
                    "Changed paused clock, enabled Play, and no gateway progress must hold before the screenshot")
                capture(app, "seek-absolute", "Timeline seek",
                        "One slider request targeted \(plan.normalizedTarget * 100) percent (ideal \(plan.targetSeconds) seconds); actual elapsed clock changed from \(before.elapsed) to \(observed.elapsed) seconds. This proves a changed paused timeline, not exact target landing. Enabled Play and no gateway progress were checked before and after the screenshot.")
                XCTAssertTrue(PlaybackCapturePolicy.absoluteSeekSettled(
                    before: before, after: absoluteClock(in: app), playEnabled: play.exists && play.isEnabled,
                    playLabel: play.exists ? play.label : nil, gatewayProgress: progress.exists),
                    "Changed paused clock, enabled Play, and no gateway progress must remain after the screenshot")
            } else { missing("seek-absolute", "Timeline seek", "unavailable", "Timeline slider is not adjustable.") }
            #else
            missing("seek-absolute", "Timeline seek", "unavailable", "tvOS exposes relative remote seek, not an adjustable timeline slider.")
            #endif
            }
            openMenu("volume", in: app)
            XCTAssertTrue(waitForVolume(in: app), "Volume menu requires one readable, settled AX readout")
            capture(app, "volume", "Volume menu", "Volume percentage and volume controls are visible.")
            for (id, target) in [("volume-minimum", 0), ("volume-midpoint", 50), ("volume-maximum", 100)] {
                try setVolume(target, in: app)
                capture(app, id, "Volume \(target)%", "Visible volume percentage equals \(target)%; no adjustment was requested after reaching this bound.")
            }
            activate(enabledButton("Mute", in: app), in: app)
            XCTAssertTrue(enabledButton("Unmute", in: app).waitForExistence(timeout: 10))
            capture(app, "muted", "Muted", "Enabled Unmute action confirms muted state.")
            activate(enabledButton("Unmute", in: app), in: app)
            XCTAssertTrue(enabledButton("Mute", in: app).waitForExistence(timeout: 10))
            capture(app, "unmuted", "Unmuted", "Enabled Mute action confirms unmuted state.")
            closeMenu(app)
            if !volumeOnly {
            openMenu("tracks", in: app)
            XCTAssertTrue(app.buttons["Subtitles Off"].exists)
            capture(app, "tracks", "Track inventory", "Visible audio/subtitle menu inventory; selected rows expose Selected accessibility values.")
            let choices = app.buttons.allElementsBoundByIndex.filter { $0.label.hasPrefix("Audio:") || $0.label.hasPrefix("Subtitle:") }.map(\.label)
            if choices.isEmpty { missing("track-choices", "Track choices", "unavailable", "Track menu exposes no audio or subtitle choices.") }
            for (index, label) in choices.enumerated() {
                let choice = app.buttons[label]
                let id = "track-\(index)"
                if blocked {
                    missing(id, label, "blocked", "A natural track-update error prevents further selection requests.")
                    continue
                }
                if !choice.isEnabled {
                    missing(id, label, "unavailable", "Track choice is present but disabled.")
                    continue
                }
                blocked = !captureTrack(label, id: id, in: app)
            }
            let off = app.buttons["Subtitles Off"]
            if blocked {
                missing("subtitles-off", "Subtitles Off", "blocked", "A natural track-update error prevents further selection requests.")
            } else if off.isEnabled {
                blocked = !captureTrack("Subtitles Off", id: "subtitles-off", in: app)
            } else { missing("subtitles-off", "Subtitles Off", "unavailable", "Off is present but disabled.") }
            closeMenu(app)
            if blocked {
                missing("fullscreen", "Fullscreen", "blocked", "Natural track-update error prevents further player controls.")
                missing("windowed", "Windowed", "blocked", "Natural track-update error prevents further player controls.")
            } else {
            #if !os(tvOS)
            exposeFullscreenControl(in: app)
            let initialFullscreen = try XCTUnwrap(fullscreenControl(in: app), "A unique app-owned fullscreen control is required")
            let fullscreenPlan = try XCTUnwrap(PlaybackCapturePolicy.fullscreenPlan(label: initialFullscreen.label, enabled: initialFullscreen.isEnabled),
                                               "Unknown or disabled initial fullscreen state must fail")
            for step in fullscreenPlan {
                if step.toggle {
                    exposeFullscreenControl(in: app)
                    let full = try XCTUnwrap(fullscreenControl(in: app), "A unique app-owned fullscreen control is required")
                    XCTAssertTrue(full.isEnabled)
                    XCTAssertEqual(full.label, step.entered ? "Fullscreen" : "Exit fullscreen", "Observe the pre-toggle state before one normal UI request")
                    activate(full, in: app)
                    exposeFullscreenControl(in: app)
                }
                XCTAssertTrue(waitForFullscreen(step.entered, in: app), "Fullscreen state must settle through pure observation; no automatic toggle retry")
                closeMenu(app)
                capture(app, step.entered ? "fullscreen" : "windowed", step.entered ? "Fullscreen" : "Windowed", step.observation)
            }
            #else
            missing("fullscreen", "Fullscreen", "unavailable", "tvOS playback already occupies the television stage; no fullscreen toggle.")
            missing("windowed", "Windowed", "unavailable", "tvOS has no windowed player mode.")
            #endif
            missing("error", "Natural playback error", "not-observed", "No natural media error was observed; errors were not forced.")
            }
            }
        }
        openMenu("session", in: app)
        XCTAssertTrue(menuClose(app).exists)
        capture(app, "settings", "Playback settings", "Playback settings menu and its dismiss action are visible; no retry or recovery action was requested.")
        let stop = app.buttons["Stop playback"]
        var cleanupEvidence = "Stop was unavailable; provider cleanup is unconfirmed."
        if stop.exists && stop.isEnabled {
            activate(stop, in: app)
            let outcome = try XCTUnwrap(waitForStoppedOutcome(in: app), "Stop must expose one complete known local outcome with no gateway progress")
            cleanupEvidence = outcome.cleanupEvidence
            capture(app, "stopped", "Stopped", "\(cleanupEvidence) One Stop request only; no retry.")
        } else { missing("stopped", "Stopped", "unavailable", "Stop is not available; normal Back is used without a retry.") }
        closeMenu(app)
        reveal(app)
        let back = control("playback.back", in: app)
        XCTAssertTrue(back.exists)
        exitAttempted = true
        activate(back, in: app)
        XCTAssertTrue(app.buttons["detail.play"].waitForExistence(timeout: 20), "Owned playback must return normally to details")
        ownsPlayback = false
        capture(app, "cleanup", "Playback closed", "Normal Back returned to item details. \(cleanupEvidence) Provider cleanup is not independently queried.")
        if volumeOnly {
            let attachment = XCTAttachment(string: "Focused actual volume/mute reproduction completed with one normal Stop and Back. No showcase manifest was emitted.")
            attachment.name = "playback-focused-volume-complete.txt"
            attachment.lifetime = .keepAlways
            add(attachment)
            return
        }
        let manifest: [String: Any] = [
            "schema_version": 1, "journey": "playback", "backend_instance_id": instance,
            "item": ["id": item, "title": title, "year": year],
            "platforms": [["slug": profile == "ipad" ? "ipados" : profile,
                           "runtime": ProcessInfo.processInfo.operatingSystemVersionString,
                           "outcome": blocked ? "blocked" : "observed", "states": states]]
        ]
        let data = try JSONSerialization.data(withJSONObject: ["run_id": runID, "manifest": manifest], options: [.sortedKeys])
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.json")
        attachment.name = "playback-manifest-success.json"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor private func absoluteClock(in app: XCUIApplication) -> PlaybackCapturePolicy.ClockPosition? {
        let observed = clockFields(in: app)
        return observed.count == 1 ? PlaybackCapturePolicy.identifiedClockPosition(observed.fields) : nil
    }

    @MainActor private func clockFields(in app: XCUIApplication) -> (count: Int, fields: [PlaybackCapturePolicy.ClockField]) {
        let clocks = app.descendants(matching: .any).matching(identifier: PlaybackCapturePolicy.clockIdentifier).allElementsBoundByIndex
        guard clocks.count == 1 else { return (clocks.count, []) }
        return (clocks.count, clocks.map {
            PlaybackCapturePolicy.ClockField(identifier: $0.identifier, role: $0.elementType.rawValue, label: $0.label, value: $0.value)
        })
    }

    @MainActor private func clockFieldsReceipt(in app: XCUIApplication) -> [String: Any] {
        let observed = clockFields(in: app)
        return PlaybackCapturePolicy.clockFieldsReceipt(count: observed.count, fields: observed.fields)
    }

    @MainActor private func attachClockFields(_ name: String, in app: XCUIApplication) {
        guard clockReceiptsEnabled else { return }
        guard let data = try? JSONSerialization.data(withJSONObject: clockFieldsReceipt(in: app), options: [.sortedKeys]) else {
            XCTFail("Safe clock fields receipt must serialize")
            return
        }
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.json")
        attachment.name = "playback-clock-fields-\(name).json"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor private func absoluteSeekSnapshot(_ app: XCUIApplication, clock: PlaybackCapturePolicy.ClockPosition?,
                                                requested: PlaybackCapturePolicy.AbsoluteSeekPlan?) -> [String: Any] {
        let play = app.buttons["playback.play"]
        let position = app.sliders["playback.position"]
        let status = app.staticTexts["playback.status"]
        let session = app.buttons["playback.session"]
        let diagnostic = session.exists ? session.value : nil
        var receipt = PlaybackCapturePolicy.absoluteSeekReceipt(
            clock: clock, sliderValue: position.exists ? position.value : nil, requested: requested,
            playExists: play.exists, playEnabled: play.exists && play.isEnabled, playLabel: play.exists ? play.label : nil,
            sliderEnabled: position.exists && position.isEnabled,
            gatewayProgress: app.progressIndicators["Contacting playback gateway"].exists,
            status: status.exists ? status.label : nil, preparation: diagnostic)
        receipt["seek_correlation"] = PlaybackCapturePolicy.seekCorrelationReceipt(diagnostic) ?? NSNull()
        receipt["clock_fields"] = clockFieldsReceipt(in: app)
        return receipt
    }

    @MainActor private func attachAbsoluteSeek(_ name: String, _ receipt: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: receipt, options: [.prettyPrinted, .sortedKeys]) else {
            XCTFail("Safe absolute-seek receipt must serialize")
            return
        }
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.json")
        attachment.name = "playback-absolute-seek-\(name).json"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor private func observeAbsoluteSeek(_ app: XCUIApplication) {
        #if os(iOS)
        var issues: [String] = []
        let play = app.buttons["playback.play"]
        let position = app.sliders["playback.position"]
        let progress = app.progressIndicators["Contacting playback gateway"]
        if wait(play, "enabled == true AND label == 'Play'"), activate(play, in: app) {
            if wait(play, "enabled == true AND label == 'Pause'"), activate(play, in: app) {
                let paused = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                    play.exists && play.isEnabled && play.label == "Play" && !progress.exists
                }, object: app)
                if XCTWaiter.wait(for: [paused], timeout: 15) != .completed {
                    issues.append("Explicit Pause did not settle with enabled Play and no gateway progress")
                }
            } else { issues.append("One explicit Play did not expose enabled Pause") }
        } else { issues.append("Preparation did not expose enabled Play") }
        let before = absoluteClock(in: app)
        let plan = issues.isEmpty ? PlaybackCapturePolicy.absoluteSeekPlan(before) : nil
        attachAbsoluteSeek("before", absoluteSeekSnapshot(app, clock: before, requested: nil))
        capture(app, "absolute-seek-before", "Before absolute seek", "Actual player before any slider adjustment; safe clock and control receipt attached.")
        attachAbsoluteSeek("request", absoluteSeekSnapshot(app, clock: before, requested: plan))
        var attempted = false
        var changed = false
        if let plan, position.exists && position.isEnabled {
            attempted = true
            position.adjust(toNormalizedSliderPosition: plan.normalizedTarget)
            let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                PlaybackCapturePolicy.absoluteSeekSettled(
                    before: plan.before, after: self.absoluteClock(in: app), playEnabled: play.exists && play.isEnabled,
                    playLabel: play.exists ? play.label : nil, gatewayProgress: progress.exists)
            }, object: app)
            changed = XCTWaiter.wait(for: [settled], timeout: 15) == .completed
            if !changed { issues.append("One separated slider adjustment did not change the actual settled paused clock") }
        } else {
            issues.append("No slider input: unique valid elapsed/duration clock, separated target, and enabled slider required")
        }
        let afterClock = absoluteClock(in: app)
        var after = absoluteSeekSnapshot(app, clock: afterClock, requested: plan)
        let finalSettled = plan.map {
            PlaybackCapturePolicy.absoluteSeekSettled(
                before: $0.before, after: afterClock,
                playEnabled: after["play_exists"] as? Bool == true && after["play_enabled"] as? Bool == true,
                playLabel: after["play_label"] as? String, gatewayProgress: after["gateway_progress"] as? Bool ?? true)
        } ?? false
        if changed && !finalSettled { issues.append("Final receipt no longer confirms the changed settled paused clock") }
        after["adjustment_attempted"] = attempted
        after["strict_changed_settled_pause"] = changed && finalSettled
        if attempted && !PlaybackCapturePolicy.seekCorrelationComplete(after["seek_correlation"] as? [String: Any]) {
            issues.append("One slider action requires a complete safe slider/dispatch/completion correlation; no extra input or retry")
        }
        attachAbsoluteSeek("after", after)
        capture(app, "absolute-seek-after", "After absolute seek attempt", "Actual player after at most one slider adjustment; receipt records whether the strict clock/control/progress predicate passed.")
        var cleanup: [String: Any] = ["stop_attempted": false, "back_attempted": false, "details_observed": false,
                                      "provider_confirmation": "unknown"]
        openMenu("session", in: app)
        let stop = app.buttons["Stop playback"]
        if stop.exists && stop.isEnabled {
            cleanup["stop_attempted"] = true
            activate(stop, in: app)
            let stopped = wait(app.staticTexts["playback.status"], "label CONTAINS 'stopped'")
            cleanup["visible_stopped_outcome"] = stopped
            if !stopped { issues.append("One normal Stop did not reach a visible stopped outcome") }
        } else { issues.append("Normal Stop was unavailable; no substitute or retry") }
        let status = app.staticTexts["playback.status"]
        cleanup["status"] = PlaybackCapturePolicy.seekStatus(status.exists ? status.label : nil)
        if closeMenu(app) {
            reveal(app)
            let back = control("playback.back", in: app)
            if back.exists && back.isEnabled {
                exitAttempted = true
                cleanup["back_attempted"] = true
                if activate(back, in: app), app.buttons["detail.play"].waitForExistence(timeout: 20) {
                    ownsPlayback = false
                    cleanup["details_observed"] = true
                    capture(app, "absolute-seek-cleanup", "Item details after cleanup", "One normal Back request returned to item details; provider cleanup is not independently confirmed.")
                } else { issues.append("One normal Back did not confirm item details") }
            } else { issues.append("Normal Back was unavailable") }
        } else { issues.append("Menu dismissal unconfirmed; no covered Back request") }
        attachAbsoluteSeek("cleanup", cleanup)
        XCTAssertTrue(issues.isEmpty, issues.joined(separator: "; "))
        #else
        XCTFail("Focused absolute-seek diagnostic requires iOS")
        #endif
    }

    @MainActor private func observePreparation(_ app: XCUIApplication) {
        let session = app.buttons["playback.session"]
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard let receipt = PlaybackCapturePolicy.preparationReceipt(session.value) else { return false }
            return ["done", "failure"].contains(receipt["phase"] ?? "")
        }, object: app)
        var issues: [String] = []
        if XCTWaiter.wait(for: [settled], timeout: 25) != .completed {
            issues.append("Preparation did not reach a terminal safe diagnostic phase")
        }
        let receipt = PlaybackCapturePolicy.preparationReceipt(session.value)
        let attachment = XCTAttachment(string: receipt.map { fields in
            fields.keys.sorted().map { "\($0)=\(fields[$0]!)" }.joined(separator: "\n")
        } ?? "Safe preparation receipt unavailable")
        attachment.name = "playback-preparation-receipt.txt"
        attachment.lifetime = .keepAlways
        add(attachment)
        capture(app, "preparation-result", "Preparation result", "Actual player after one detail Play request; safe preparation receipt attached separately. No retry.")
        let ready = receipt?["phase"] == "done" && receipt?["blocked"] == "false" &&
            receipt?["closed"] == "false" && receipt?["session"] == "true"
        if ready {
            let play = app.buttons["playback.play"]
            if wait(play, "enabled == true AND label == 'Play'") {
                activate(play, in: app)
                let started = wait(play, "enabled == true AND label == 'Pause'")
                if !started { issues.append("Explicit player Play did not reach enabled Pause") }
                if started, let before = advancingTimeline(in: app) {
                    let advanced = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                        guard let after = self.advancingTimeline(in: app) else { return false }
                        return PlaybackCapturePolicy.timelineAdvanced(before, after) && play.isEnabled && play.label == "Pause"
                    }, object: app)
                    if XCTWaiter.wait(for: [advanced], timeout: 15) != .completed {
                        issues.append("Actual playback timeline did not advance with enabled Pause")
                    } else {
                        capture(app, "preparation-playing", "Playback after preparation", "Enabled Pause and an actual changed timeline confirm playback after explicit Play; no old failure cause is inferred.")
                    }
                } else if started { issues.append("Actual playback timeline was unreadable") }
            } else { issues.append("Prepared player did not expose enabled Play") }
            openMenu("session", in: app)
            let stop = app.buttons["Stop playback"]
            if stop.exists && stop.isEnabled {
                activate(stop, in: app)
                if !wait(app.staticTexts["playback.status"], "label CONTAINS 'stopped'") {
                    issues.append("One Stop request did not reach a visible stopped outcome")
                }
            } else { issues.append("Prepared session exposed no enabled normal Stop") }
        } else {
            issues.append("Preparation failed or remained unavailable; safe phase=\(receipt?["phase"] ?? "unknown"), code=\(receipt?["code"] ?? "unknown")")
        }
        if closeMenu(app) {
            reveal(app)
            let back = control("playback.back", in: app)
            if back.exists && back.isEnabled {
                exitAttempted = true
                if activate(back, in: app), app.buttons["detail.play"].waitForExistence(timeout: 20) {
                    ownsPlayback = false
                } else { issues.append("One normal Back request did not confirm item details") }
            } else { issues.append("Normal Back was unavailable") }
        } else { issues.append("Menu dismissal was not confirmed; no covered Back request") }
        let cleanup = XCTAttachment(string: ownsPlayback
            ? "Normal exit is unconfirmed. No repeated Back or backend cleanup request."
            : "One normal Back request returned to details. Provider cleanup is not independently confirmed.")
        cleanup.name = "playback-preparation-cleanup.txt"
        cleanup.lifetime = .keepAlways
        add(cleanup)
        XCTAssertTrue(issues.isEmpty, issues.joined(separator: "; "))
    }

    @MainActor private func advancingTimeline(in app: XCUIApplication) -> PlaybackCapturePolicy.TimelineValue? {
        if let clock = absoluteClock(in: app) { return .seconds(clock.elapsed) }
        return timelineReading(app.sliders["playback.position"], in: app)
    }

    @MainActor private func observeTelevisionFocus(_ app: XCUIApplication) throws {
        #if os(tvOS)
        let play = app.buttons["playback.play"]
        XCTAssertTrue(wait(play, "enabled == true"))
        XCTAssertEqual(play.label, "Play")
        guard activate(play, in: app) else { return }
        XCTAssertTrue(wait(play, "label == 'Pause' AND enabled == true"))
        guard activate(play, in: app) else { return }
        XCTAssertTrue(wait(play, "label == 'Play' AND enabled == true"))
        capture(app, "remote-paused", "Paused remote focus probe", "Normal remote Select started then paused playback; enabled Play confirms paused state.")
        for identifier in ["playback.backward", "playback.forward", "playback.mute", "playback.volume",
                           "playback.tracks", "playback.session", "playback.back"] {
            XCTAssertTrue(focus(app.buttons[identifier], in: app), "Every current toolbar control and Back must accept real remote focus")
            XCTAssertTrue(play.isEnabled && play.label == "Play")
        }
        let initialVolume = app.buttons.matching(identifier: "playback.volume").allElementsBoundByIndex
        let initial = try XCTUnwrap(initialVolume.count == 1 ? initialVolume[0] : nil)
        guard focus(initial, in: app) else { return }
        let initialBegin = ProcessInfo.processInfo.systemUptime
        let initialWall = Date().timeIntervalSince1970
        let finalInitial = app.buttons.matching(identifier: "playback.volume").allElementsBoundByIndex
        let initialFocused = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).allElementsBoundByIndex
        XCTAssertTrue(finalInitial.count == 1 && initial.isEnabled && initial.hasFocus && initialFocused.count == 1 &&
                      initialFocused[0].identifier == "playback.volume")
        try televisionProofRead(step: "volume-initial", kind: "initial",
                                fields: ["count": finalInitial.count, "focused_count": initialFocused.count,
                                         "enabled": initial.isEnabled, "has_focus": initial.hasFocus],
                                begin: initialBegin, wall: initialWall)
        try televisionProofPress(.select, button: "select", step: "volume-initial", in: app)
        let menus = app.scrollViews.matching(identifier: "playback.menu.volume").allElementsBoundByIndex
        let volume = try XCTUnwrap(menus.count == 1 ? menus[0] : nil, "Exactly one actual volume-menu container is required")
        let mute = try televisionMenuButton(["Mute", "Unmute"], within: volume)
        for button in [try televisionMenuButton(["Quieter"], within: volume),
                       try televisionMenuButton(["Louder"], within: volume), mute] {
            XCTAssertTrue(focus(button, in: app))
        }
        try televisionProofMenu("volume", step: "volume-initial", in: app)
        try televisionProofPress(.menu, button: "menu", step: "volume-close", in: app)
        try televisionProofReady("volume", step: "volume-return", in: app)
        try televisionProofPress(.select, button: "select", step: "volume-return", in: app)
        try televisionProofMenu("volume", step: "volume-return", in: app)
        try televisionProofPress(.menu, button: "menu", step: "volume-reclose", in: app)
        try televisionProofAdjacent("volume", to: "tracks", in: app)
        try televisionProofPress(.select, button: "select", step: "tracks-initial", in: app)
        try televisionProofMenu("tracks", step: "tracks-initial", in: app)
        try televisionProofPress(.menu, button: "menu", step: "tracks-close", in: app)
        try televisionProofReady("tracks", step: "tracks-return", in: app)
        try televisionProofPress(.select, button: "select", step: "tracks-return", in: app)
        try televisionProofMenu("tracks", step: "tracks-return", in: app)
        try televisionProofPress(.menu, button: "menu", step: "tracks-reclose", in: app)
        try televisionProofAdjacent("tracks", to: "session", in: app)
        try televisionProofPress(.select, button: "select", step: "session-initial", in: app)
        let session = try televisionProofMenu("session", step: "session-initial", in: app)
        let stop = try televisionMenuButton(["Stop playback"], within: session)
        try televisionProofMenuFocus(stop, in: app)
        try televisionProofPress(.select, button: "select", step: "stop", in: app)
        let stopBegin = ProcessInfo.processInfo.systemUptime
        let stopWall = Date().timeIntervalSince1970
        let outcome = try XCTUnwrap(waitForStoppedOutcome(in: app))
        try televisionProofRead(step: "stop", kind: "stop", fields: ["outcome": outcome.outcome.rawValue,
                                "earlier_personal_report_unknown": outcome.earlierPersonalReportUnknown],
                                begin: stopBegin, wall: stopWall)
        capture(app, "remote-stopped", "Stopped remote focus probe", outcome.cleanupEvidence)
        try televisionProofPress(.menu, button: "menu", step: "session-close", in: app)
        try televisionProofReady("back", step: "back", stopped: true, in: app)
        exitAttempted = true
        try televisionProofPress(.select, button: "select", step: "back", in: app)
        let detailsBegin = ProcessInfo.processInfo.systemUptime
        let detailsWall = Date().timeIntervalSince1970
        let details = app.buttons["detail.play"].waitForExistence(timeout: 20)
        if details { ownsPlayback = false }
        XCTAssertTrue(details, "One normal Back must return to item details")
        let title = app.staticTexts.matching(identifier: "page.title").allElementsBoundByIndex
        let year = app.staticTexts.matching(NSPredicate(format: "label == %@", "Movie · 1922")).allElementsBoundByIndex
        let exact = title.count == 1 && title[0].label == "Nosferatu" && year.count == 1 &&
            app.buttons.matching(identifier: "detail.play").count == 1 && app.buttons["detail.play"].isEnabled
        XCTAssertTrue(exact, "Back must return to exact Nosferatu 1922 details")
        try televisionProofRead(step: "details", kind: "details", fields: ["exact": exact],
                                begin: detailsBegin, wall: detailsWall)
        let receipt: [String: Any] = ["schema_version": 1, "stop_outcome": outcome.outcome.rawValue,
                                    "details_observed": details, "provider_confirmation": "unknown",
                                    "success_manifest_emitted": false]
        let attachment = XCTAttachment(data: try JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys]),
                                       uniformTypeIdentifier: "public.json")
        attachment.name = "playback-focused-television-complete.json"
        attachment.lifetime = .keepAlways
        add(attachment)
        #else
        XCTFail("Focused remote navigation requires tvOS")
        #endif
    }

    #if os(tvOS)
    @MainActor private func televisionProofMenuFocus(_ target: XCUIElement, in app: XCUIApplication) throws {
        for _ in 0..<30 {
            let focused = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).allElementsBoundByIndex
            guard focused.count == 1, target.exists, target.isEnabled else {
                XCTFail("Focused menu navigation requires one actual focused item; no empty-focus search")
                throw NSError(domain: "TelevisionProof", code: 2)
            }
            if target.hasFocus {
                remoteFocusReceipt(target, focused: focused, failure: false)
                return
            }
            let direction = try XCTUnwrap(PlaybackCapturePolicy.remoteDirection(target: target.frame, focused: focused[0].frame))
            switch direction {
            case .up: XCUIRemote.shared.press(.up)
            case .down: XCUIRemote.shared.press(.down)
            case .left: XCUIRemote.shared.press(.left)
            case .right: XCUIRemote.shared.press(.right)
            }
        }
        XCTFail("Focused menu navigation exceeded its bound")
        throw NSError(domain: "TelevisionProof", code: 3)
    }

    @MainActor private func televisionProofRead(step: String, kind: String, fields: [String: Any],
                                               begin: Double? = nil, wall: Double? = nil) throws {
        var receipt = fields
        receipt["schema_version"] = 1
        receipt["kind"] = kind
        receipt["step"] = step
        receipt["begin"] = begin ?? ProcessInfo.processInfo.systemUptime
        receipt["end"] = ProcessInfo.processInfo.systemUptime
        receipt["wall_begin"] = wall ?? Date().timeIntervalSince1970
        receipt["wall_end"] = Date().timeIntervalSince1970
        let attachment = XCTAttachment(data: try JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys]),
                                       uniformTypeIdentifier: "public.json")
        attachment.name = "playback-tv-proof-\(kind)-\(step).json"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor private func televisionProofPress(_ button: XCUIRemote.Button, button name: String,
                                                step: String, in app: XCUIApplication) throws {
        let begin = ProcessInfo.processInfo.systemUptime
        let wall = Date().timeIntervalSince1970
        XCUIRemote.shared.press(button)
        try televisionProofRead(step: step, kind: "press", fields: ["button": name], begin: begin, wall: wall)
    }

    @discardableResult
    @MainActor private func televisionProofMenu(_ menu: String, step: String, in app: XCUIApplication) throws -> XCUIElement {
        let begin = ProcessInfo.processInfo.systemUptime
        let wall = Date().timeIntervalSince1970
        let containers = app.scrollViews.matching(identifier: "playback.menu.\(menu)")
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            containers.count == 1
        }, object: app)], timeout: 15), .completed)
        let all = containers.allElementsBoundByIndex
        let container = try XCTUnwrap(all.count == 1 ? all[0] : nil, "Exactly one expected menu is required")
        var audio = 0
        var off = 0
        if menu == "volume" {
            _ = try televisionMenuButton(["Quieter"], within: container)
            _ = try televisionMenuButton(["Louder"], within: container)
            _ = try televisionMenuButton(["Mute", "Unmute"], within: container)
        } else if menu == "tracks" {
            audio = container.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Audio:")).count
            off = container.buttons.matching(NSPredicate(format: "label == %@", "Subtitles Off")).count
            XCTAssertTrue(audio > 0 && audio <= 64 && off == 1, "Actual audio and Off inventory is required")
        } else {
            _ = try televisionMenuButton(["Stop playback"], within: container)
        }
        XCTAssertEqual(app.scrollViews.matching(NSPredicate(format: "identifier BEGINSWITH %@", "playback.menu.")).count, 1)
        try televisionProofRead(step: step, kind: "menu", fields: ["menu": menu, "count": all.count,
                                                                 "audio": audio, "off": off], begin: begin, wall: wall)
        return container
    }

    @MainActor private func televisionProofReady(_ control: String, step: String, stopped: Bool = false,
                                                in app: XCUIApplication) throws {
        let begin = ProcessInfo.processInfo.systemUptime
        let wall = Date().timeIntervalSince1970
        let predicate = NSPredicate { _, _ in
            let buttons = app.buttons.matching(identifier: "playback.\(control)").allElementsBoundByIndex
            return buttons.count == 1 && buttons[0].isEnabled && buttons[0].isHittable &&
                !self.menuClose(app).exists && !app.progressIndicators["Contacting playback gateway"].exists &&
                (stopped || (app.buttons["playback.play"].isEnabled && app.buttons["playback.play"].label == "Play"))
        }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: predicate, object: app)], timeout: 15), .completed,
                       "Public UI readiness is only a scheduling hint; the independent native gate remains mandatory")
        let buttons = app.buttons.matching(identifier: "playback.\(control)").allElementsBoundByIndex
        let ready = predicate.evaluate(with: app)
        XCTAssertTrue(ready)
        try televisionProofRead(step: step, kind: "ready", fields: ["control": control, "count": buttons.count,
                                                                  "ready": ready], begin: begin, wall: wall)
    }

    @MainActor private func televisionProofAdjacent(_ from: String, to: String, in app: XCUIApplication) throws {
        try televisionProofReady(from, step: "\(from)-adjacent", in: app)
        let begin = ProcessInfo.processInfo.systemUptime
        let wall = Date().timeIntervalSince1970
        func inventory() -> [PlaybackCapturePolicy.ToolbarItem] {
            let origin = app.buttons["playback.\(from)"].frame
            return app.buttons.allElementsBoundByIndex.filter {
                abs($0.frame.midY - origin.midY) <= 1 && abs($0.frame.height - origin.height) <= 1
            }.map {
                let id = $0.identifier
                let control = id.hasPrefix("playback.") ? String(id.dropFirst(9)) : "other"
                return .init(control: PlaybackCapturePolicy.televisionToolbar.contains(control) ? control : "other",
                             enabled: $0.isEnabled, frame: $0.frame)
            }
        }
        let before = inventory()
        let current = inventory()
        guard PlaybackCapturePolicy.immediateRight(from: from, to: to, before: before, current: current) else {
            XCTFail("One adjacent step requires an unchanged complete unique eligible toolbar row")
            throw NSError(domain: "TelevisionProof", code: 1)
        }
        let items: [[String: Any]] = current.map {
            ["control": $0.control, "enabled": $0.enabled, "x": $0.frame.minX, "y": $0.frame.minY,
             "width": $0.frame.width, "height": $0.frame.height]
        }
        try televisionProofRead(step: to, kind: "adjacent", fields: ["from": from, "to": to, "inventory": items],
                                begin: begin, wall: wall)
        try televisionProofPress(.right, button: "right", step: to, in: app)
        try televisionProofReady(to, step: "\(to)-before-select", in: app)
    }

    @MainActor private func observeRestoredToolbarFocus(_ identifier: String, in app: XCUIApplication) -> Bool {
        let restored = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            let targets = app.buttons.matching(identifier: identifier).allElementsBoundByIndex
            let focused = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).allElementsBoundByIndex
            return targets.count == 1 && focused.count == 1 && targets[0].isEnabled &&
                targets[0].hasFocus && focused[0].identifier == identifier
        }, object: app)
        let completed = XCTWaiter.wait(for: [restored], timeout: 15) == .completed
        let targets = app.buttons.matching(identifier: identifier).allElementsBoundByIndex
        let focused = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).allElementsBoundByIndex
        let ready = completed && targets.count == 1 && focused.count == 1 && targets[0].isEnabled &&
            targets[0].hasFocus && focused[0].identifier == identifier
        if targets.count == 1 { remoteFocusReceipt(targets[0], focused: focused, failure: !ready) }
        XCTAssertTrue(ready, "Menu close must restore its actual toolbar anchor before any further remote direction")
        return ready
    }

    @MainActor private func televisionMenuButton(_ labels: [String], within menu: XCUIElement) throws -> XCUIElement {
        let buttons = menu.buttons.matching(NSPredicate(format: "label IN %@", labels as NSArray)).allElementsBoundByIndex
        let index = try XCTUnwrap(PlaybackCapturePolicy.uniqueMenuButtonIndex(
            buttons.map { (label: $0.label, enabled: $0.isEnabled) }, labels: Set(labels)),
            "Exactly one enabled leaf button inside the actual menu is required")
        return buttons[index]
    }
    #endif

    @MainActor private func stopFields(in app: XCUIApplication) -> (count: Int, fields: [PlaybackCapturePolicy.StopField]) {
        let matches = app.descendants(matching: .any).matching(identifier: PlaybackCapturePolicy.stopIdentifier).allElementsBoundByIndex
        guard matches.count == 1 else { return (matches.count, []) }
        return (1, matches.map {
            PlaybackCapturePolicy.StopField(identifier: $0.identifier, role: $0.elementType.rawValue, label: $0.label, value: $0.value)
        })
    }

    @MainActor private func waitForStoppedOutcome(in app: XCUIApplication) -> PlaybackCapturePolicy.StopObservation? {
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            let snapshot = self.stopFields(in: app)
            return snapshot.count == 1 && PlaybackCapturePolicy.identifiedStopOutcome(snapshot.fields) != nil &&
                !app.progressIndicators["Contacting playback gateway"].exists
        }, object: app)
        let ready = XCTWaiter.wait(for: [settled], timeout: 15) == .completed
        let snapshot = stopFields(in: app)
        let progress = app.progressIndicators["Contacting playback gateway"].exists
        if clockReceiptsEnabled {
            let receipt = PlaybackCapturePolicy.stopFieldsReceipt(count: snapshot.count, fields: snapshot.fields, gatewayProgress: progress)
            if let data = try? JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys]) {
                let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.json")
                attachment.name = "playback-stop-fields.json"
                attachment.lifetime = .keepAlways
                add(attachment)
            } else { XCTFail("Safe Stop field receipt must serialize") }
        }
        guard ready, snapshot.count == 1, !progress else { return nil }
        return PlaybackCapturePolicy.identifiedStopOutcome(snapshot.fields)
    }

    @MainActor private func wait(_ element: XCUIElement, _ format: String, _ arguments: CVarArg...) -> Bool {
        let predicate = NSPredicate(format: format, argumentArray: arguments)
        return XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: predicate, object: element)], timeout: 15) == .completed
    }

    @MainActor private func timelineReading(_ position: XCUIElement, in app: XCUIApplication) -> PlaybackCapturePolicy.TimelineValue? {
        if let value = PlaybackCapturePolicy.timelineValue(position.value) { return value }
        guard let clock = absoluteClock(in: app) else { return nil }
        return .seconds(clock.elapsed)
    }

    @MainActor private func waitForSeek(_ position: XCUIElement, from before: PlaybackCapturePolicy.TimelineValue, in app: XCUIApplication) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard let after = self.timelineReading(position, in: app) else { return false }
            let play = app.buttons["playback.play"]
            return PlaybackCapturePolicy.timelineChanged(before, after) && play.isEnabled && play.label == "Play" &&
                !app.progressIndicators["Contacting playback gateway"].exists
        }, object: app)
        return XCTWaiter.wait(for: [expectation], timeout: 15) == .completed
    }

    @MainActor private func failureExit(_ app: XCUIApplication) {
        guard ownsPlayback else { return }
        let previous = continueAfterFailure
        continueAfterFailure = true
        defer { continueAfterFailure = previous }
        attachClockFields("failure", in: app)
        #if os(tvOS)
        if remoteFocusReceiptsEnabled {
            var evidence = "Focused proof failed; no blind Back search or retry. Local exit and provider cleanup remain unconfirmed."
            let back = app.buttons.matching(identifier: "playback.back").allElementsBoundByIndex
            let focused = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).allElementsBoundByIndex
            if !exitAttempted && !menuClose(app).exists && back.count == 1 && back[0].isEnabled &&
                back[0].hasFocus && focused.count == 1 && focused[0].identifier == "playback.back" {
                exitAttempted = true
                XCUIRemote.shared.press(.select)
                if app.buttons["detail.play"].waitForExistence(timeout: 20) {
                    ownsPlayback = false
                    evidence = "Failure teardown sent one normal Back request and observed item details. Provider cleanup is not independently confirmed."
                }
            }
            let attachment = XCTAttachment(string: evidence)
            attachment.name = "playback-failure-cleanup.txt"
            attachment.lifetime = .keepAlways
            add(attachment)
            return
        }
        #endif
        var evidence = "A normal Back request was already sent; its outcome is unconfirmed. No repeated exit request."
        if PlaybackCapturePolicy.normalExitAllowed(owned: ownsPlayback, attempted: exitAttempted) {
            if closeMenu(app) {
                reveal(app)
                let back = control("playback.back", in: app)
                if back.exists && back.isEnabled {
                    exitAttempted = true
                    if activate(back, in: app), app.buttons["detail.play"].waitForExistence(timeout: 20) {
                        ownsPlayback = false
                        evidence = "Failure teardown sent one normal Back request and observed item details. Provider cleanup is not independently confirmed."
                    } else {
                        evidence = "Failure teardown attempted normal Back once; return to details is unconfirmed. No retry."
                    }
                } else {
                    evidence = "Failure teardown found no enabled normal Back control. Local exit and provider cleanup remain unconfirmed."
                }
            } else {
                evidence = "Failure teardown could not confirm menu dismissal; no covered Back action was sent. Local exit and provider cleanup remain unconfirmed."
            }
        }
        let attachment = XCTAttachment(string: evidence)
        attachment.name = "playback-failure-cleanup.txt"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor private func naturalError(in app: XCUIApplication) -> Bool {
        let status = app.staticTexts["playback.status"]
        let prefixes = [
            "This media is unsupported.", "The server cannot provide a compatible transport.",
            "This title or its source is unavailable.", "This transport cannot be played.",
            "Playback is unavailable or its personal-state outcome is uncertain.",
            "Choose a Jellyfin profile before playback.", "The Jellyfin profile changed.",
            "The playback session expired.", "The playback gateway has no available capacity."
        ]
        return status.exists && prefixes.contains(where: { status.label.hasPrefix($0) })
    }

    @MainActor private func captureTrack(_ label: String, id: String, in app: XCUIApplication) -> Bool {
        let choice = app.buttons[label]
        let alreadySelected = choice.value as? String == "Selected"
        if PlaybackCapturePolicy.trackNeedsSelection(selected: alreadySelected) { activate(choice, in: app) }
        let loading = app.progressIndicators["Contacting playback gateway"]
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            let row = app.buttons[label]
            let busy = loading.exists
            return (!busy && self.naturalError(in: app)) ||
                PlaybackCapturePolicy.trackSettled(selected: row.value as? String == "Selected",
                                                    enabled: row.exists && row.isEnabled, transportBusy: busy)
        }, object: app)
        XCTAssertEqual(XCTWaiter.wait(for: [settled], timeout: 25), .completed,
                       "Track selection must settle; a stale Selected flag while controls are disabled is not evidence")
        if naturalError(in: app) {
            capture(app, "error", "Natural track-update error", "A known media/session error is visible after one selection request; gateway progress has ended. No retry was sent.")
            missing(id, label, "blocked", "Natural track-update error prevented a settled requested selection.")
            return false
        }
        XCTAssertTrue(PlaybackCapturePolicy.trackSettled(selected: choice.value as? String == "Selected",
                                                        enabled: choice.isEnabled, transportBusy: loading.exists))
        capture(app, id, label, alreadySelected
                ? "Current row is Selected with controls enabled and no gateway progress; no redundant request was sent."
                : "Requested row is Selected with controls enabled and no gateway progress after normal UI selection.")
        return true
    }

    @MainActor private func volumePercent(in app: XCUIApplication) -> Int? {
        let matches = app.descendants(matching: .any).matching(identifier: "playback.volume.readout")
        return PlaybackCapturePolicy.uniqueVolumeReadout(matches.allElementsBoundByIndex.map { ($0.label, $0.value) })
    }

    @MainActor private func waitForVolume(_ target: Int? = nil, changedFrom previous: Int? = nil, in app: XCUIApplication) -> Bool {
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            PlaybackCapturePolicy.volumeSettled(actual: self.volumePercent(in: app), target: target, previous: previous,
                                               controlsEnabled: app.buttons["Quieter"].isEnabled && app.buttons["Louder"].isEnabled,
                                               menuVisible: self.menuClose(app).exists,
                                               transportBusy: app.progressIndicators["Contacting playback gateway"].exists)
        }, object: app)
        let ready = XCTWaiter.wait(for: [settled], timeout: 15) == .completed
        if !ready {
            let attachment = XCTAttachment(string: app.debugDescription)
            attachment.name = "playback-volume-ax-failure.txt"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        return ready
    }

    @MainActor private func setVolume(_ target: Int, in app: XCUIApplication) throws {
        for _ in 0..<12 {
            let current = try XCTUnwrap(volumePercent(in: app), "Readable actual AX volume percentage required")
            guard let action = PlaybackCapturePolicy.volumeAction(current: current, target: target) else {
                XCTAssertTrue(waitForVolume(target, in: app), "Exact volume endpoint must settle before capture")
                return
            }
            let button = app.buttons[action]
            XCTAssertTrue(wait(button, "enabled == true"))
            volumeDiagnostic("before-\(action)-from-\(current)", in: app)
            activate(button, in: app)
            let changed = waitForVolume(changedFrom: current, in: app)
            volumeDiagnostic("after-\(action)-from-\(current)", in: app)
            XCTAssertTrue(changed, "Actual AX volume must change and controls must settle after each single adjustment")
        }
        XCTAssertTrue(waitForVolume(target, in: app), "Bounded volume navigation must reach the explicit actual percentage")
    }

    @MainActor private func volumeDiagnostic(_ event: String, in app: XCUIApplication) {
        guard ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1" else { return }
        let quieter = app.buttons["Quieter"]
        let louder = app.buttons["Louder"]
        let attachment = XCTAttachment(string: [
            "event=\(event)", "appState=\(app.state.rawValue)", "volume=\(String(describing: volumePercent(in: app)))",
            "quieterEnabled=\(quieter.isEnabled)", "quieterHittable=\(quieter.isHittable)",
            "louderEnabled=\(louder.isEnabled)", "louderHittable=\(louder.isHittable)",
            "menuVisible=\(menuClose(app).exists)",
            "gatewayProgress=\(app.progressIndicators["Contacting playback gateway"].exists)"
        ].joined(separator: "\n"))
        attachment.name = "playback-volume-input-\(event).txt"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func missing(_ id: String, _ label: String, _ status: String, _ reason: String) {
        states.append(["id": id, "label": label, "status": status, "reason": reason,
                       "viewport": viewport, "observation": "Not captured: \(reason)"])
    }

    @MainActor private func capture(_ app: XCUIApplication, _ id: String, _ label: String, _ observation: String) {
        #if os(macOS)
        let target = app.windows.firstMatch
        #else
        let target: XCUIElement = app
        #endif
        let frame = target.frame
        viewport = frame.width > 0 && frame.height > 0 ? "\(Int(frame.width))x\(Int(frame.height)) points" : "unknown"
        let attachment = XCTAttachment(screenshot: target.screenshot())
        attachment.name = "playback-\(id).png"
        attachment.lifetime = .keepAlways
        add(attachment)
        states.append(["id": id, "label": label, "status": "captured", "viewport": viewport,
                       "image": "\(id).png", "observation": observation])
    }

    @MainActor private func reveal(_ app: XCUIApplication) {
        if app.buttons["playback.session"].exists { return }
        let stage = app.buttons["Show playback controls"]
        if stage.exists { activate(stage, in: app) }
        #if os(tvOS)
        if !stage.exists { XCUIRemote.shared.press(.down) }
        #endif
    }

    @MainActor private func openMenu(_ name: String, in app: XCUIApplication) {
        reveal(app)
        let direct = app.buttons["playback.\(name)"]
        if name == "session" {
            XCTAssertTrue(direct.waitForExistence(timeout: 15), "Session menu control must settle before navigation")
            activate(direct, in: app)
        } else if direct.exists { activate(direct, in: app) }
        else {
            activate(app.buttons["playback.session"], in: app)
            activate(enabledButton(name == "volume" ? "Volume" : "Audio and subtitles", in: app), in: app)
        }
        XCTAssertTrue(menuClose(app).waitForExistence(timeout: 10))
    }

    @MainActor private func menuClose(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == 'Dismiss playback menu' AND identifier BEGINSWITH 'playback.menu.'")).firstMatch
    }

    @MainActor private func enabledButton(_ label: String, in app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == %@ AND enabled == true", label)).firstMatch
    }

    @MainActor private func control(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
        let matches = app.buttons.matching(identifier: identifier)
        return matches.allElementsBoundByIndex.first(where: { $0.isHittable }) ?? matches.firstMatch
    }

    #if !os(tvOS)
    @MainActor private func fullscreenControl(in app: XCUIApplication) -> XCUIElement? {
        let choices = app.buttons.matching(NSPredicate(format: "identifier == 'playback.fullscreen' OR label == 'Fullscreen' OR label == 'Exit fullscreen'")).allElementsBoundByIndex
        let close = menuClose(app)
        let menu = close.exists ? close.identifier : nil
        let index = PlaybackCapturePolicy.fullscreenChoice(choices.map { ($0.identifier, $0.label, $0.isHittable) }, menu: menu)
        return index.map { choices[$0] }
    }

    @MainActor private func exposeFullscreenControl(in app: XCUIApplication) {
        reveal(app)
        XCTAssertTrue(app.buttons["playback.session"].waitForExistence(timeout: 15), "Player controls must settle before fullscreen navigation")
        closeMenu(app)
        if !app.buttons["playback.fullscreen"].waitForExistence(timeout: 5) {
            openMenu("session", in: app)
        }
        let available = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            self.fullscreenControl(in: app)?.isEnabled == true
        }, object: app)
        XCTAssertEqual(XCTWaiter.wait(for: [available], timeout: 15), .completed,
                       "Missing fullscreen query evidence is a harness failure, not proof of unsupported fullscreen")
    }

    @MainActor private func waitForFullscreen(_ entered: Bool, in app: XCUIApplication) -> Bool {
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard let button = self.fullscreenControl(in: app) else { return false }
            return button.isEnabled && button.label == (entered ? "Exit fullscreen" : "Fullscreen")
        }, object: app)
        let ready = XCTWaiter.wait(for: [settled], timeout: 20) == .completed
        if !ready {
            let attachment = XCTAttachment(string: app.debugDescription)
            attachment.name = "playback-fullscreen-ax-failure.txt"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        return ready
    }
    #endif

    @discardableResult
    @MainActor private func closeMenu(_ app: XCUIApplication) -> Bool {
        let close = menuClose(app)
        if close.exists {
            guard activate(close, in: app) else { return false }
            let dismissed = wait(close, "exists == false")
            XCTAssertTrue(dismissed, "Menu dismissal must complete before another control request")
            return dismissed
        }
        return true
    }

    @discardableResult
    @MainActor private func activate(_ element: XCUIElement, in app: XCUIApplication) -> Bool {
        guard element.exists || element.waitForExistence(timeout: 10) else {
            XCTFail("Required UI control does not exist")
            return false
        }
        #if os(tvOS)
        guard focus(element, in: app) else { return false }
        XCUIRemote.shared.press(.select)
        #elseif os(macOS)
        for _ in 0..<6 {
            if element.isHittable { break }
            let scroll = app.scrollViews.firstMatch
            if !scroll.exists { break }
            scroll.scroll(byDeltaX: 0, deltaY: -160)
        }
        element.click()
        #else
        for _ in 0..<6 {
            if element.isHittable { break }
            app.swipeUp()
        }
        element.tap()
        #endif
        return true
    }

    #if os(tvOS)
    @MainActor private func focus(_ element: XCUIElement, in app: XCUIApplication) -> Bool {
        guard element.exists || element.waitForExistence(timeout: 10) else {
            XCTFail("Required remote focus target does not exist")
            return false
        }
        for _ in 0..<30 {
            let matches = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).allElementsBoundByIndex
            remoteFocusReceipt(element, focused: matches, failure: false)
            if element.hasFocus { return true }
            if matches.isEmpty { XCUIRemote.shared.press(.right); continue }
            guard matches.count == 1 else {
                remoteFocusReceipt(element, focused: matches, failure: true)
                XCTFail("Remote navigation requires one actual focused element")
                return false
            }
            let focused = matches[0]
            guard let direction = PlaybackCapturePolicy.remoteDirection(target: element.frame, focused: focused.frame) else {
                remoteFocusReceipt(element, focused: matches, failure: true)
                XCTFail("Remote navigation requires distinct finite actual target and focus geometry")
                return false
            }
            switch direction {
            case .up: XCUIRemote.shared.press(.up)
            case .down: XCUIRemote.shared.press(.down)
            case .left: XCUIRemote.shared.press(.left)
            case .right: XCUIRemote.shared.press(.right)
            }
        }
        let finalFocus = app.descendants(matching: .any).matching(NSPredicate(format: "hasFocus == true")).allElementsBoundByIndex
        let ready = element.hasFocus
        remoteFocusReceipt(element, focused: finalFocus, failure: !ready)
        if ready { return true }
        XCTFail("Bounded remote focus navigation failed")
        return false
    }

    @MainActor private func remoteFocusReceipt(_ target: XCUIElement, focused: [XCUIElement], failure: Bool) {
        guard remoteFocusReceiptsEnabled || failure else { return }
        func node(_ element: XCUIElement) -> [String: Any] {
            let identifiers = ["playback.back", "playback.play", "playback.backward", "playback.forward", "playback.mute",
                               "playback.volume", "playback.tracks", "playback.session", "playback.menu.close"]
            let labels = ["Show playback controls", "Quieter", "Louder", "Mute", "Unmute", "Stop playback"]
            let id = identifiers.contains(element.identifier) ? element.identifier :
                (labels.contains(element.label) ? element.label : "other")
            return ["control": id, "role": element.elementType.rawValue, "has_focus": element.hasFocus,
                    "enabled": element.isEnabled]
        }
        var receipt: [String: Any] = ["schema_version": 1, "failure": failure, "target": node(target),
                                    "focused_count": focused.count, "focused": focused.prefix(1).map(node)]
        if remoteFocusReceiptsEnabled { receipt["observation_uptime"] = ProcessInfo.processInfo.systemUptime }
        guard let data = try? JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys]) else {
            XCTFail("Safe remote focus receipt must serialize")
            return
        }
        remoteFocusReceiptIndex += 1
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.json")
        attachment.name = "playback-remote-focus-\(remoteFocusReceiptIndex).json"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
    #endif
}
