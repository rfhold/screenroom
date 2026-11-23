import Foundation
import SwiftUI

private func fixtureSession(revision: UInt32 = 1) -> PlaybackSession {
    PlaybackSession(sessionId: "11111111-1111-4111-8111-111111111111", controlCapability: String(repeating: "a", count: 64),
        state: .ready, selectedSource: nil, tracks: [],
        transport: PlaybackTransport(method: .directPlay, mimeType: "video/mp4",
            mediaUrl: "https://example.invalid/playback/v1/media/test", revision: revision,
            durationMs: "120000", startPositionMs: "40000", timelineOffsetMs: "10000"),
        transportRevision: revision, lastAcceptedSequence: 0, reportOutcome: .confirmed,
        leaseExpiresAt: "", expiresAt: "", pendingOperation: nil, cleanupPending: false, failureCode: nil)
}

private actor PlaybackProbe {
    var session = fixtureSession()
    var creates: [CreatePlaybackSessionRequest] = []
    var reports: [ReportPlaybackEventRequest] = []
    var updates: [UpdatePlaybackSessionRequest] = []
    var stops: [StopPlaybackSessionRequest] = []
    var reads = 0
    var failCreate = false
    var failReport = false
    var failUpdate = false
    var failStop = false
    var delayCreate = false
    var createGate: CheckedContinuation<Void, Never>?
    var delayReport = false
    var reportGate: CheckedContinuation<Void, Never>?
    var typedCreateFailure: PlaybackErrorCode?

    func failCreateWith(_ code: PlaybackErrorCode) { typedCreateFailure = code }

    func fail(_ operation: String) {
        switch operation {
        case "create": failCreate = true
        case "report": failReport = true
        case "update": failUpdate = true
        case "stop": failStop = true
        case "delay": delayCreate = true
        case "delayReport": delayReport = true
        default: preconditionFailure()
        }
    }
    func create(_ request: CreatePlaybackSessionRequest) async throws -> CreatePlaybackSessionResponse {
        creates.append(request)
        if delayCreate { await withCheckedContinuation { createGate = $0 } }
        if let code = typedCreateFailure {
            typedCreateFailure = nil
            throw ClientError.Playback(code: code)
        }
        if failCreate { failCreate = false; throw CocoaError(.fileReadUnknown) }
        return CreatePlaybackSessionResponse(session: session)
    }
    func releaseCreate() { createGate?.resume(); createGate = nil }
    func releaseReport() { reportGate?.resume(); reportGate = nil; delayReport = false }
    func report(_ request: ReportPlaybackEventRequest) async throws -> ReportPlaybackEventResponse {
        reports.append(request)
        if delayReport { await withCheckedContinuation { reportGate = $0 } }
        if failReport { failReport = false; throw CocoaError(.fileReadUnknown) }
        session.lastAcceptedSequence = request.sequence
        return ReportPlaybackEventResponse(acceptedSequence: request.sequence, reportOutcome: .confirmed, session: session)
    }
    func update(_ request: UpdatePlaybackSessionRequest) throws -> UpdatePlaybackSessionResponse {
        updates.append(request)
        if failUpdate { failUpdate = false; throw CocoaError(.fileReadUnknown) }
        session = fixtureSession(revision: 2)
        return UpdatePlaybackSessionResponse(session: session)
    }
    func stop(_ request: StopPlaybackSessionRequest) throws -> StopPlaybackSessionResponse {
        stops.append(request)
        if failStop { failStop = false; throw CocoaError(.fileReadUnknown) }
        session.state = .stopped
        return StopPlaybackSessionResponse(session: session)
    }
    func get() -> GetPlaybackSessionResponse { reads += 1; return GetPlaybackSessionResponse(session: session) }
}

private final class PlaybackClient: Client, @unchecked Sendable {
    let probe: PlaybackProbe
    init(_ probe: PlaybackProbe) { self.probe = probe; super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("Test-only transport") }
    override func createPlaybackSession(request: CreatePlaybackSessionRequest) async throws -> CreatePlaybackSessionResponse { try await probe.create(request) }
    override func reportPlaybackEvent(request: ReportPlaybackEventRequest) async throws -> ReportPlaybackEventResponse { try await probe.report(request) }
    override func updatePlaybackSession(request: UpdatePlaybackSessionRequest) async throws -> UpdatePlaybackSessionResponse { try await probe.update(request) }
    override func stopPlaybackSession(request: StopPlaybackSessionRequest) async throws -> StopPlaybackSessionResponse { try await probe.stop(request) }
    override func getPlaybackSession(request: GetPlaybackSessionRequest) async throws -> GetPlaybackSessionResponse { await probe.get() }
}

@MainActor
private final class MediaProbe {
    var seconds = 0.0
    var installs = 0
    var seeks: [Double] = []
    var seekAdjustment = 0.0
    func owner(_ probe: PlaybackProbe) -> NativePlayback {
        NativePlayback(client: PlaybackClient(probe), itemID: "movie", scope: SourceLibraryScope(sourceId: "source", libraryId: "library"),
            prepareTransport: { _ in self.installs += 1 }, readPosition: { self.seconds },
            seekTransport: { self.seeks.append($0); self.seconds = $0 + self.seekAdjustment })
    }
}

@main
struct NativePlaybackTests {
    @MainActor
    static func main() async throws {
        await preparationDiagnosticChecks()
        await seekDiagnosticChecks()
        await presentationChecks()
        await volumeControlChecks()
        for duration: Int64 in [-1, 0, 1, 50, 500, 999, 1_000, 120_000, 922_337_203_685_477, .max] {
            let bounds = PlaybackScrubBounds(duration: duration)
            precondition(bounds.range.lowerBound == 0 && bounds.upperBound.isFinite && bounds.upperBound > 0)
            precondition(bounds.step.isFinite && bounds.step > 0 && bounds.step <= bounds.upperBound)
            precondition(bounds.canSeek == (duration > 0))
            for stale in [-Double.infinity, .infinity, .nan, -100, 0, 1234.75, Double.greatestFiniteMagnitude] {
                let value = bounds.clamp(stale)
                precondition(value.isFinite && bounds.range.contains(value))
                precondition(bounds.milliseconds(stale) >= 0 && Double(bounds.milliseconds(stale)) <= bounds.upperBound)
                let slider = Slider(value: .constant(value), in: bounds.range, step: bounds.step)
                withExtendedLifetime(slider) {}
            }
        }
        let unknown = PlaybackScrubBounds(duration: 0)
        precondition(unknown.range == 0...1 && unknown.step == 1 && !unknown.canSeek)
        let normal = PlaybackScrubBounds(duration: 120_000)
        precondition(normal.milliseconds(12_345.75) == 12_345, "Seek targets retain logical millisecond precision")
        let shrunk = PlaybackScrubBounds(duration: 500)
        precondition(shrunk.clamp(normal.clamp(90_000)) == 500 && shrunk.milliseconds(90_000) == 500)
        let largest = try PlaybackTimeline.milliseconds("922337203685477")
        precondition(largest == 922_337_203_685_477)
        for bad in ["", "-1", "1e3", "0.5", "922337203685478"] {
            do { _ = try PlaybackTimeline.milliseconds(bad); preconditionFailure("Invalid timeline accepted") } catch {}
        }
        precondition(PlaybackTimeline.logical(seconds: 30, offset: 10000) == 40000)
        precondition(PlaybackTimeline.transport(position: 40000, offset: 10000) == 30)
        precondition(PlaybackTimeline.logical(seconds: .nan, offset: 10000) == 10000)

        let probe = PlaybackProbe()
        let media = MediaProbe()
        let owner = media.owner(probe)
        await probe.fail("create")
        await owner.prepareOnPresentation()
        precondition(owner.blocked && owner.session == nil)
        owner.presentation.setFullscreen(true)
        owner.presentation.setFullscreen(false)
        await owner.prepareOnPresentation()
        let createsAfterPresentation = await probe.creates
        precondition(createsAfterPresentation.count == 1, "Presentation transitions must not retry uncertain preparation")
        await owner.start()
        let creations = await probe.creates
        precondition(creations.count == 2 && creations[0] == creations[1])
        precondition(creations[0].startPolicy == .resume && creations[0].positionMs == nil)
        precondition(creations[0].scope?.libraryId == "library" && creations[0].clientCapabilities?.webvtt == true)
        precondition(media.seconds == 30 && media.installs == 1)
        await owner.heartbeat()
        let reportsBeforeStart = await probe.reports
        precondition(reportsBeforeStart.isEmpty, "No personal report or heartbeat before explicit Play")
        await owner.toggle()
        let originalPlayer = owner.player
        let originalSessionID = owner.session?.sessionId
        for _ in 0..<3 {
            owner.presentation.setFullscreen(true)
            await owner.handleScenePhase(.active)
            precondition(owner.presentation.fullscreen)
            owner.presentation.setFullscreen(false)
            await owner.handleScenePhase(.active)
            precondition(!owner.presentation.fullscreen)
            await owner.prepareOnPresentation()
        }
        let presentationCreates = await probe.creates.count
        let presentationReports = await probe.reports.count
        let presentationStops = await probe.stops
        precondition(owner.player === originalPlayer && owner.session?.sessionId == originalSessionID)
        precondition(!owner.closed && !owner.paused && presentationCreates == 2 && presentationReports == 1 && presentationStops.isEmpty)
        var chrome = NativePlayerChromeState()
        for _ in 0..<10 {
            chrome.rememberFocus("mute")
            chrome.expire(interaction: chrome.interaction, visibility: NativePlayerVisibility())
            chrome.televisionInput(.select, focus: "stage")
            chrome.revealFromTelevisionStage(navigation: false)
            chrome.televisionInput(.navigation, focus: "play")
            chrome.televisionInput(.select, focus: "mute")
            chrome.releaseTelevisionFocus()
            chrome.open(.tracks, anchor: "tracks")
            precondition(chrome.dismissMenu() == "tracks")
            chrome.open(.session, anchor: "session")
            precondition(chrome.dismissMenu() == "session")
        }
        let chromeCreates = await probe.creates.count
        let chromeReports = await probe.reports.count
        let chromeStops = await probe.stops
        precondition(owner.player === originalPlayer && owner.session?.sessionId == originalSessionID)
        precondition(!owner.paused && media.installs == 1 && chromeCreates == presentationCreates && chromeReports == presentationReports && chromeStops.isEmpty,
                     "Reveal, idle, menus and focus state never mutate media or personal reports")
        media.seconds = 50
        await owner.toggle()
        await owner.seek(by: -30)
        let events = await probe.reports
        precondition(events.map(\.kind) == [.started, .paused, .seeked])
        precondition(events.map(\.sequence) == [1, 2, 3])
        precondition(events.map(\.positionMs) == ["40000", "60000", "30000"])
        precondition(media.seconds == 20)
        await owner.toggle()
        await owner.heartbeat()
        let activeEvents = await probe.reports
        precondition(activeEvents.map(\.kind) == [.started, .paused, .seeked, .resumed, .heartbeat, .progress])
        precondition(activeEvents.map(\.sequence) == [1, 2, 3, 4, 5, 6])
        await owner.check()
        precondition(media.installs == 1 && media.seconds == 20, "Unchanged revision must preserve media")
        await probe.fail("update")
        await owner.choose(nil)
        precondition(owner.blocked && owner.canRetryTransport)
        await owner.retryTransport()
        let updates = await probe.updates
        precondition(updates.count == 2 && updates[0] == updates[1])
        precondition(updates[0].trackChoices?.subtitlesOff == true)
        precondition(media.installs == 2)
        await probe.fail("stop")
        media.seconds = 27
        await owner.close()
        media.seconds = 1
        await owner.close()
        let stopBeforeExplicitRetry = await probe.stops
        precondition(stopBeforeExplicitRetry.count == 1, "Repeated lifecycle exit must not retry Stop automatically")
        await owner.retryStop()
        let stops = await probe.stops
        precondition(stops.count == 2 && stops[0] == stops[1])
        precondition(stops[0].positionMs == "37000" && stops[0].transportRevision == 2)
        await owner.start()
        let finalCreates = await probe.creates.count
        precondition(finalCreates == 2, "Closed owner cannot recreate playback")

        let uncertainProbe = PlaybackProbe()
        let uncertain = MediaProbe().owner(uncertainProbe)
        await uncertain.start()
        await uncertainProbe.fail("report")
        await uncertain.toggle()
        precondition(uncertain.blocked && uncertain.paused)
        await uncertain.check()
        await uncertain.toggle()
        let uncertainReports = await uncertainProbe.reports.count
        let uncertainReads = await uncertainProbe.reads
        precondition(uncertainReports == 1, "Unknown start must never replay")
        precondition(uncertainReads >= 1, "Ambiguous mutation requires authoritative reconciliation")
        await uncertain.close()

        let delayedProbe = PlaybackProbe()
        await delayedProbe.fail("delay")
        let delayed = MediaProbe().owner(delayedProbe)
        let creation = Task { await delayed.start() }
        while await delayedProbe.createGate == nil { await Task.yield() }
        let close = Task { await delayed.close(reason: "profile_changed") }
        while !delayed.closed { await Task.yield() }
        await delayedProbe.releaseCreate()
        await creation.value
        await close.value
        precondition(delayed.closed && delayed.player.currentItem == nil)
        let delayedStops = await delayedProbe.stops.count
        let delayedReason = await delayedProbe.stops.first?.stopReason
        let delayedReports = await delayedProbe.reports
        precondition(delayedStops >= 1, "Late creation must be stopped")
        precondition(delayedReason == "profile_changed")
        precondition(delayedReports.isEmpty)
        let trackProbe = PlaybackProbe()
        let tracks = MediaProbe().owner(trackProbe)
        await tracks.start()
        await tracks.choose(PlaybackTrack(id: "audio-2", kind: .audio, displayName: "Audio", language: "en",
            selected: false, available: true, subtitleMode: nil, mediaUrl: nil))
        let choices = await trackProbe.updates
        precondition(choices.first?.trackChoices?.audioTrackId == "audio-2")
        precondition(choices.first?.trackChoices?.subtitlesOff == false)
        await tracks.close()
        let earlyProbe = PlaybackProbe()
        let early = MediaProbe().owner(earlyProbe)
        await early.start()
        await early.seek(to: 0)
        let earlier = await earlyProbe.updates
        precondition(earlier.first?.positionMs == "0" && earlier.first?.expectedTransportRevision == 1)
        await early.close()
        let remoteProbe = PlaybackProbe()
        let remote = MediaProbe().owner(remoteProbe)
        await remote.start()
        await remoteProbe.fail("delayReport")
        let remoteStart = Task { await remote.handleRemotePlayPause() }
        let remoteDeadline = ContinuousClock.now.advanced(by: .seconds(5))
        while await remoteProbe.reportGate == nil {
            precondition(ContinuousClock.now < remoteDeadline)
            await Task.yield()
        }
        await remote.handleRemotePlayPause()
        precondition(remote.paused && remote.busy, "Remote Play waits for its personal report before native playback")
        await remoteProbe.releaseReport()
        await remoteStart.value
        precondition(!remote.paused)
        await remote.handleRemotePlayPause()
        precondition(remote.paused)
        await remote.handleRemotePlayPause()
        let remoteEvents = await remoteProbe.reports
        precondition(remoteEvents.map(\.kind) == [.started, .paused, .resumed])
        precondition(remoteEvents.map(\.sequence) == [1, 2, 3], "Busy remote commands cannot bypass report ordering")
        await remoteProbe.fail("report")
        await remote.handleRemotePlayPause()
        await remote.handleRemotePlayPause()
        let uncertainRemoteEvents = await remoteProbe.reports
        precondition(remote.blocked && remote.paused && uncertainRemoteEvents.count == 4)
        await remote.close()
        for phase in [ScenePhase.inactive, .background] {
            let phaseProbe = PlaybackProbe()
            let phaseOwner = MediaProbe().owner(phaseProbe)
            await phaseOwner.prepareOnPresentation()
            await phaseOwner.handleScenePhase(phase)
            await phaseOwner.handleScenePhase(.active)
            await phaseOwner.prepareOnPresentation()
            let phaseCreates = await phaseProbe.creates
            let phaseStops = await phaseProbe.stops
            precondition(phaseOwner.closed && phaseOwner.player.currentItem == nil)
            precondition(phaseCreates.count == 1 && phaseStops.count == 1 && phaseStops[0].stopReason == "background")
        }
        let cues = try NativeSubtitleCue.parse(Data("WEBVTT\n\n00:00:10.000 --> 00:00:12.000 align:center\nHello <b>world</b>\n\nid\n00:01:03.500 --> 00:01:04.000\nNext".utf8))
        precondition(cues == [NativeSubtitleCue(start: 10000, end: 12000, text: "Hello world"), NativeSubtitleCue(start: 63500, end: 64000, text: "Next")])
        do { _ = try NativeSubtitleCue.parse(Data("not webvtt".utf8)); preconditionFailure() } catch {}
        print("Native presentation idle/menu/focus/viewport, scrub bounds/Slider construction, lifecycle, fullscreen continuity, report-aware remote Play/Pause, exact retries, ordered seek, and WebVTT checks passed")
    }

    @MainActor
    private static func seekDiagnosticChecks() async {
        let previous = ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"]
        defer {
            if let previous { setenv("SCREENROOM_PLAYBACK_DIAGNOSTICS", previous, 1) }
            else { unsetenv("SCREENROOM_PLAYBACK_DIAGNOSTICS") }
        }
        for enabled in [false, true] {
            if enabled { setenv("SCREENROOM_PLAYBACK_DIAGNOSTICS", "1", 1) }
            else { unsetenv("SCREENROOM_PLAYBACK_DIAGNOSTICS") }
            let probe = PlaybackProbe()
            let media = MediaProbe()
            let owner = media.owner(probe)
            await owner.prepareOnPresentation()
            await owner.toggle()
            await owner.toggle()
            media.seekAdjustment = 7
            let raw = 90_000.75
            let target = PlaybackScrubBounds(duration: owner.duration).milliseconds(raw)
            owner.recordSliderReceived(raw)
            let operation = owner.recordSliderEnded(target: target)
            precondition((operation != nil) == enabled)
            await owner.seek(to: target, diagnostic: operation)
            precondition(media.seeks == [30, 80] && media.seconds == 87 && owner.position == 97_000)
            let reports = await probe.reports
            let creates = await probe.creates.count
            let updates = await probe.updates.count
            let stops = await probe.stops.count
            let reads = await probe.reads
            precondition(creates == 1 && updates == 0 && stops == 0 && reads == 0 && media.installs == 1)
            precondition(reports.map(\.kind) == [.started, .paused, .seeked])
            precondition(reports.map(\.sequence) == [1, 2, 3] && reports.map(\.positionMs) == ["40000", "40000", "97000"])
            precondition(owner.paused && !owner.busy && !owner.closed && !owner.blocked)
            if enabled {
                let receipt = try! JSONSerialization.jsonObject(with: Data(owner.seekDiagnostic!.utf8)) as! [String: Any]
                let records = receipt["records"] as! [[String: Any]]
                precondition(records.count == 3)
                precondition(records.map { $0["stage"] as! String } == ["slider-ended", "seek-dispatch", "seek-completion"])
                precondition(records.allSatisfy { ($0["operation"] as! NSNumber).intValue == 1 })
                precondition((records[0]["slider_received_ms"] as! NSNumber).doubleValue == raw)
                precondition(records.allSatisfy { ($0["logical_target_ms"] as! NSNumber).int64Value == 90_000 })
                precondition(records.allSatisfy { ($0["transport_offset_ms"] as! NSNumber).int64Value == 10_000 })
                precondition((records[1]["media_requested_ms"] as! NSNumber).int64Value == 80_000)
                precondition((records[2]["media_actual_ms"] as! NSNumber).int64Value == 87_000)
                precondition((records[2]["logical_actual_ms"] as! NSNumber).int64Value == 97_000)
                let safe = owner.seekDiagnostic
                for invalid in [Double.nan, .infinity, -.infinity, -1, Double.greatestFiniteMagnitude] {
                    owner.recordSliderReceived(invalid)
                    precondition(owner.recordSliderEnded(target: 90_000) == nil && owner.seekDiagnostic == safe)
                }
                owner.recordSliderReceived(90_000)
                precondition(owner.recordSliderEnded(target: -1) == nil && owner.seekDiagnostic == safe)
                for _ in 0..<12 {
                    owner.recordSliderReceived(raw)
                    let operation = owner.recordSliderEnded(target: target)
                    await owner.seek(to: target, diagnostic: operation)
                }
                let capped = try! JSONSerialization.jsonObject(with: Data(owner.seekDiagnostic!.utf8)) as! [String: Any]
                let cappedRecords = capped["records"] as! [[String: Any]]
                precondition(cappedRecords.count == 24 && Set(cappedRecords.map { ($0["operation"] as! NSNumber).intValue }) == Set(1...8))
                let finalReports = await probe.reports.count
                precondition(media.seeks.count == 14 && finalReports == 15, "Diagnostics add no seeks, reports, or retries after the cap")
                unsetenv("SCREENROOM_PLAYBACK_DIAGNOSTICS")
                precondition(owner.diagnosticValue == nil, "Diagnostic AX stays opt-in even after earlier receipts")
            } else {
                precondition(owner.seekDiagnostic == nil && owner.diagnosticValue == nil)
            }
            await owner.close(reason: "test")
        }
        setenv("SCREENROOM_PLAYBACK_DIAGNOSTICS", "1", 1)
        let invalidProbe = PlaybackProbe()
        let invalidMedia = MediaProbe()
        let invalidOwner = invalidMedia.owner(invalidProbe)
        await invalidOwner.prepareOnPresentation()
        await invalidOwner.toggle()
        await invalidOwner.toggle()
        for adjustment in [Double.nan, .infinity, -.infinity, -100] {
            invalidMedia.seekAdjustment = adjustment
            invalidOwner.recordSliderReceived(90_000)
            let operation = invalidOwner.recordSliderEnded(target: 90_000)!
            await invalidOwner.seek(to: 90_000, diagnostic: operation)
            let root = try! JSONSerialization.jsonObject(with: Data(invalidOwner.seekDiagnostic!.utf8)) as! [String: Any]
            let records = root["records"] as! [[String: Any]]
            let stages = records.filter { ($0["operation"] as! NSNumber).intValue == operation }.map { $0["stage"] as! String }
            precondition(stages == ["slider-ended", "seek-dispatch"], "Invalid actual media samples cannot emit a numeric completion")
        }
        let invalidReports = await invalidProbe.reports.count
        precondition(invalidMedia.seeks.count == 5 && invalidReports == 6)
        await invalidOwner.close(reason: "test")
        var trace = NativePlaybackSeekDiagnostic()
        let operation = trace.begin(received: 90_000, target: 90_000, offset: 10_000)!
        precondition(!trace.dispatch(operation, target: 90_000, offset: 10_000, media: .nan))
        precondition(trace.dispatch(operation, target: 90_000, offset: 10_000, media: 80_000))
        for invalid in [Double.nan, .infinity, -.infinity, -1, Double.greatestFiniteMagnitude] {
            precondition(!trace.complete(operation, media: invalid, logical: 90_000, offset: 10_000))
        }
        precondition(!trace.complete(operation, media: 80_000, logical: 90_000, offset: 0))
        precondition(trace.complete(operation, media: 80_000, logical: 90_000, offset: 10_000))
        precondition(!trace.complete(operation, media: 80_000, logical: 90_000, offset: 10_000))
        print("Actual controller seek diagnostics: default-off, raw/clamped/offset/media correlation, first existing post-seek position read, finite bounds, 8-operation cap, and unchanged control/RPC counts passed")
    }

    @MainActor
    private static func preparationDiagnosticChecks() async {
        let previous = ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"]
        unsetenv("SCREENROOM_PLAYBACK_DIAGNOSTICS")
        defer {
            if let previous { setenv("SCREENROOM_PLAYBACK_DIAGNOSTICS", previous, 1) }
            else { unsetenv("SCREENROOM_PLAYBACK_DIAGNOSTICS") }
        }
        let offProbe = PlaybackProbe()
        await offProbe.failCreateWith(.upstreamUnavailable)
        let off = MediaProbe().owner(offProbe)
        await off.prepareOnPresentation()
        precondition(off.preparationDiagnostic == nil && off.blocked)
        await off.close(reason: "test")
        let codes: [(PlaybackErrorCode, String)] = [
            (.unspecified, "unknown"), (.profileRequired, "profile_required"), (.profileChanged, "profile_changed"),
            (.sourceUnavailable, "source_unavailable"), (.itemUnavailable, "item_unavailable"),
            (.unsupportedMedia, "unsupported_media"), (.transcodingUnavailable, "transcoding_unavailable"),
            (.playbackAuthorizationUnavailable, "playback_authorization_unavailable"), (.playbackConflict, "playback_conflict"),
            (.sessionExpired, "session_expired"), (.operationConflict, "operation_conflict"),
            (.capacityExhausted, "capacity_exhausted"), (.upstreamUnavailable, "upstream_unavailable")
        ]
        let secret = "private-body https://private.invalid/?cap=secret profile=private"
        let arbitrary: [Error] = [
            NSError(domain: secret, code: 42, userInfo: [NSLocalizedDescriptionKey: secret]),
            ClientError.Rpc(code: 13, reason: secret), ClientError.Unavailable(reason: secret), ClientError.PlaybackUncertain
        ]
        for error in arbitrary {
            precondition(NativePlaybackPreparationDiagnostic.code(error) == "untyped")
            precondition(NativePlaybackPreparationDiagnostic.line(enabled: false, phase: .failure, stage: .create,
                error: error, blocked: true, closed: false, session: false) == nil)
            let line = NativePlaybackPreparationDiagnostic.line(enabled: true, phase: .failure, stage: .create,
                error: error, blocked: true, closed: false, session: false)!
            precondition(line == "ScreenroomPlaybackPreparation phase=failure stage=create code=untyped blocked=true closed=false session=false")
            precondition(!line.contains(secret) && !line.contains("https") && !line.contains("cap="))
        }
        setenv("SCREENROOM_PLAYBACK_DIAGNOSTICS", "1", 1)
        for (code, name) in codes {
            precondition(NativePlaybackPreparationDiagnostic.code(ClientError.Playback(code: code)) == name)
            let probe = PlaybackProbe()
            await probe.failCreateWith(code)
            let owner = MediaProbe().owner(probe)
            await owner.prepareOnPresentation()
            precondition(owner.blocked && !owner.busy && owner.session == nil)
            precondition(owner.preparationDiagnostic == "ScreenroomPlaybackPreparation phase=failure stage=create code=\(name) blocked=true closed=false session=false")
            let requests = await probe.creates
            precondition(requests.count == 1, "Diagnostics must not retry preparation")
            await owner.close(reason: "test")
        }
        let ready = MediaProbe().owner(PlaybackProbe())
        await ready.prepareOnPresentation()
        precondition(ready.preparationDiagnostic == "ScreenroomPlaybackPreparation phase=done stage=done code=none blocked=false closed=false session=true")
        await ready.close(reason: "test")
        print("Preparation diagnostics: all 13 typed codes, exact create-stage failure, default-off, arbitrary-error redaction, no retry, and ready result passed")
    }

    @MainActor
    private static func volumeControlChecks() async {
        let probe = PlaybackProbe()
        let owner = MediaProbe().owner(probe)
        await owner.prepareOnPresentation()
        await owner.toggle()
        await owner.toggle()
        let reports = await probe.reports.count
        let session = owner.session?.sessionId
        for _ in 0..<12 {
            for (steps, delta, target) in [(10, Float(-0.1), Float(0)), (5, Float(0.1), Float(0.5)), (5, Float(0.1), Float(1))] {
                for _ in 0..<steps {
                    owner.adjustVolume(by: delta)
                    precondition(abs(owner.player.volume - owner.volume) < 0.00001)
                    precondition(!owner.busy && !owner.blocked && !owner.closed && owner.paused)
                    precondition(owner.session?.sessionId == session)
                }
                precondition(abs(owner.volume - target) < 0.00001)
            }
            owner.toggleMute()
            precondition(owner.muted && owner.player.isMuted)
            owner.toggleMute()
            precondition(!owner.muted && !owner.player.isMuted)
        }
        let reportsAfterVolume = await probe.reports.count
        precondition(reportsAfterVolume == reports, "Local volume and mute must not dispatch gateway controls")
        await owner.close(reason: "test")
        print("Actual NativePlayback repeated volume 0/50/100 and mute controls passed without busy, close, or gateway effects")
    }

    @MainActor
    private static func nativeItemObservationChecks() {
        typealias Trace = NativePlayerNativeItemTrace
        let frame = Trace.Frame(CGRect(x: 10, y: 20, width: 72, height: 72))!
        let native = Trace.Item(ordinal: 3, kind: .other, control: .other, relation: .owned,
                               canBecomeFocused: true, frameSpace: .window, frame: frame)
        let proposed = Trace.Item(ordinal: 4, kind: .view, control: .volume, relation: .owned,
                                 canBecomeFocused: false, frameSpace: .window, frame: frame)
        let snapshot = Trace.Snapshot(window: 1, system: 2, keyWindow: true, viewport: frame,
            context: true, previous: native, next: proposed, current: native, anchor: native)
        var reads = 0
        var writes = 0
        let disabled = Trace(enabled: false, clock: { reads += 1; return 1 }, sink: { _ in writes += 1 })
        let window = NSObject()
        disabled.bind(window)
        precondition(disabled.windowObject == nil && !disabled.arm(anchor: window))
        disabled.record(.updated, generation: 1, interaction: 1) { reads += 1; return snapshot }
        precondition(reads == 0 && writes == 0 && disabled.records.isEmpty)
        precondition(!Trace().enabled)
        let trace = Trace(enabled: true, clock: { reads += 1; return Double(reads) }, sink: { _ in writes += 1 })
        trace.bind(window)
        trace.record(.updated, generation: 1, interaction: 1) { reads += 1; return snapshot }
        precondition(reads == 0)
        precondition(trace.arm(anchor: window))
        trace.record(.failed, generation: 29, interaction: 29) { snapshot }
        let failed = trace.records[0]["snapshot"] as! [String: Any]
        precondition((failed["next"] as! [String: Any])["ordinal"] as? Int == 4)
        precondition((failed["current"] as! [String: Any])["ordinal"] as? Int == 3)
        precondition(trace.records[0]["event"] as? String == "failed")
        for _ in 0..<1024 { trace.record(.updated, generation: 29, interaction: 29) { snapshot } }
        precondition(trace.records.count == 128 && reads == 128 && writes == 128)
        precondition(trace.records.last?["capped"] as? Bool == true)
        trace.unbind()
        precondition(trace.windowObject == nil && trace.anchorObject == nil && !trace.arm(anchor: window))
        let weakTrace = Trace(enabled: true, clock: { 1 }, sink: { _ in })
        var owned: NSObject? = NSObject()
        var anchor: NSObject? = NSObject()
        weak var weakWindow = owned
        weak var weakAnchor = anchor
        weakTrace.bind(owned!)
        precondition(weakTrace.arm(anchor: anchor) && weakTrace.anchorCaptured)
        precondition(weakTrace.ordinal(anchor) == weakTrace.ordinal(anchor))
        anchor = nil
        owned = nil
        precondition(weakAnchor == nil && weakWindow == nil)
        precondition(weakTrace.anchorObject == nil && weakTrace.windowObject == nil)
        weakTrace.record(.updated, generation: 1, interaction: 1) { preconditionFailure() }
        let directory = Trace(enabled: true, clock: { 1 }, sink: { _ in })
        let objects = (0..<129).map { _ in NSObject() }
        for index in 0..<128 { precondition(directory.ordinal(objects[index]) == index + 1) }
        precondition(directory.ordinal(objects[0]) == 1 && directory.ordinal(objects[128]) == nil)
        precondition(!Trace.acceptsNotification(systemMatches: false, previous: .owned, next: .owned, current: .owned))
        precondition(!Trace.acceptsNotification(systemMatches: true, previous: .other, next: .other, current: .other))
        precondition(!Trace.acceptsNotification(systemMatches: nil, previous: .other, next: .other, current: .owned))
        precondition(!Trace.acceptsNotification(systemMatches: true, previous: .other, next: .missing, current: .owned))
        precondition(!Trace.acceptsNotification(systemMatches: nil, previous: .missing, next: .missing, current: .owned))
        precondition(!Trace.acceptsNotification(systemMatches: nil, previous: .unknown, next: .missing, current: .missing))
        precondition(!Trace.acceptsNotification(systemMatches: true, previous: .unknown, next: .unknown, current: .unknown))
        precondition(Trace.acceptsNotification(systemMatches: nil, previous: .owned, next: .missing, current: .missing))
        precondition(Trace.acceptsNotification(systemMatches: true, previous: .missing, next: .missing, current: .missing))
        precondition(Trace.control("playback.volume") == .volume && Trace.control("playback.menu.close") == .menuClose)
        for identifier in [nil, "private-native-item", "missing", String(repeating: "secret", count: 1000)] {
            precondition(Trace.control(identifier) == .other)
        }
        for rect in [CGRect.infinite, .null, CGRect(x: CGFloat.infinity, y: 0, width: 1, height: 1),
                     CGRect(x: 0, y: 0, width: -1, height: 1), CGRect(x: 0, y: 0, width: 1_000_001, height: 1)] {
            precondition(Trace.Frame(rect) == nil)
        }
        let invalid = Trace(enabled: true, clock: { .nan }, sink: { _ in preconditionFailure() })
        invalid.bind(window)
        precondition(invalid.arm(anchor: nil))
        invalid.record(.arm, generation: 1, interaction: 1) { snapshot }
        precondition(invalid.records.isEmpty)
        let redacted = Trace.Item(ordinal: nil, kind: .other, control: .init("private-item-secret"),
                                 relation: .unknown, canBecomeFocused: false, frameSpace: .unowned, frame: nil)
        let redaction = Trace.Snapshot(window: 1, system: nil, keyWindow: false, viewport: nil,
            context: false, previous: .missing, next: .missing, current: redacted, anchor: .missing)
        var emitted: [String] = []
        let fixture = Trace(enabled: true, clock: { 7 }, sink: { emitted.append($0) })
        fixture.bind(window)
        precondition(fixture.arm(anchor: window))
        fixture.record(.arm, generation: 29, interaction: 29) { redaction }
        fixture.record(.failed, generation: 29, interaction: 29) { snapshot }
        let data = try! JSONSerialization.data(withJSONObject: fixture.records, options: [.sortedKeys])
        let text = String(data: data, encoding: .utf8)!
        precondition(!text.contains("private-item-secret"))
        precondition(emitted.count == 2 && emitted.allSatisfy({ $0.hasPrefix("ScreenroomPlayerNativeItemTrace ") }))
        if ProcessInfo.processInfo.environment["SCREENROOM_NATIVE_ITEM_TRACE_FIXTURE"] == "1" {
            print("NativePlayerNativeItemFixture " + text)
            let lines = try! JSONSerialization.data(withJSONObject: emitted, options: [.sortedKeys])
            print("NativePlayerNativeItemSinkFixture " + String(data: lines, encoding: .utf8)!)
        }
        print("Native item trace: default-off lazy reads, window/anchor weak lifetime, ownership, failure/current separation, finite frames, redaction and 128-record cap passed")
    }

    @MainActor
    private static func presentationChecks() async {
        nativeItemObservationChecks()
        let traceEnvironment = ["SCREENROOM_PLAYBACK_DIAGNOSTICS": "1", "SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC": "1"]
        precondition(NativePlayerFocusTrace.isEnabled(environment: traceEnvironment, television: true))
        precondition(!NativePlayerFocusTrace.isEnabled(environment: traceEnvironment, television: false))
        for key in traceEnvironment.keys {
            for value in [nil, "", "0", "true", "01", "1 "] {
                var environment = traceEnvironment
                environment[key] = value
                precondition(!NativePlayerFocusTrace.isEnabled(environment: environment, television: true))
            }
        }
        let traceState = NativePlayerFocusTrace.State(generation: 1, interaction: 2, requestGeneration: 1,
            pendingGeneration: 1, issuedGeneration: nil, capturedGeneration: 1, capturedInteraction: 2,
            anchor: .volume, focusState: .back, accessibilityState: .missing, callbackValue: .other,
            target: .volume, pending: true, issued: false, visible: true, menuOpen: false,
            closed: false, active: true, eligibility: .eligible)
        var clockReads = 0
        var traceLines: [String] = []
        let disabledTrace = NativePlayerFocusTrace(enabled: false, clock: { clockReads += 1; return 1 },
            sink: { traceLines.append($0) })
        disabledTrace.record(.scheduleBefore, state: traceState)
        precondition(clockReads == 0 && traceLines.isEmpty && disabledTrace.records.isEmpty)
        let beforeDismiss = NativePlayerFocusTrace.State(generation: 0, interaction: 2, requestGeneration: nil,
            pendingGeneration: nil, issuedGeneration: nil, capturedGeneration: nil, capturedInteraction: nil,
            anchor: .volume, focusState: .menuClose, accessibilityState: .missing, callbackValue: .missing,
            target: .missing, pending: false, issued: false, visible: true, menuOpen: true,
            closed: false, active: true, eligibility: .menuOpen)
        let earlyTrace = NativePlayerFocusTrace(enabled: true, clock: { 1 }, sink: { _ in })
        earlyTrace.record(.dismissBefore, state: beforeDismiss)
        earlyTrace.record(.toolbarAppear, state: traceState)
        precondition(earlyTrace.records.count == 2,
            "Trace arms before modal mutation, so a synchronous eligible-toolbar appearance cannot escape the window")
        let enabledTrace = NativePlayerFocusTrace(enabled: true, clock: { clockReads += 1; return Double(clockReads) },
            sink: { traceLines.append($0) })
        enabledTrace.record(.nativeBefore, state: traceState)
        precondition(clockReads == 0 && traceLines.isEmpty, "The bounded window starts at first menu dismissal, not initial focus")
        enabledTrace.record(.scheduleBefore, state: traceState)
        enabledTrace.record(.preferred, state: traceState)
        enabledTrace.record(.preferred, state: traceState)
        precondition(enabledTrace.records.count == 2, "Identical preferred-target evaluations do not exhaust the trace")
        for _ in 0..<1024 { enabledTrace.record(.nativeAfter, state: traceState) }
        precondition(enabledTrace.records.count == 128 && traceLines.count == 128 && clockReads == 128)
        precondition(enabledTrace.records.last?["capped"] as? Bool == true)
        for (index, line) in traceLines.enumerated() {
            precondition(line.hasPrefix("ScreenroomPlayerFocusTrace "))
            let json = Data(line.dropFirst("ScreenroomPlayerFocusTrace ".count).utf8)
            let record = try! JSONSerialization.jsonObject(with: json) as! [String: Any]
            precondition(record["sequence"] as? Int == index + 1 && record["schema_version"] as? Int == 1)
        }
        for raw in ["private-control", "track.secret", "https://private.invalid/token", "volume\nsecret"] {
            precondition(NativePlayerFocusTrace.Control(raw) == .other)
        }
        precondition(NativePlayerFocusTrace.Control(nil) == .missing)
        let invalidClockTrace = NativePlayerFocusTrace(enabled: true, clock: { .nan }, sink: { _ in preconditionFailure() })
        invalidClockTrace.record(.scheduleBefore, state: traceState)
        precondition(invalidClockTrace.records.isEmpty)
        let defaultTrace = NativePlayerFocusTrace(enabled: true, clock: { 1 }, sink: { _ in })
        defaultTrace.record(.scheduleBefore, state: traceState)
        defaultTrace.record(.defaultPreference, state: traceState)
        defaultTrace.record(.defaultPreference, state: traceState)
        precondition(defaultTrace.records.count == 2, "Repeated identical declarations cannot consume the trace budget")
        print("Default-off dual-opt-in TV focus trace: inert reference, no disabled I/O, finite clock, fixed redaction, preferred dedup and 128-record cap passed")
        func target(_ state: NativePlayerMenuReturn, _ request: NativePlayerMenuReturn.Request?,
                    layout: NativePlayerStageLayout = .init(size: CGSize(width: 1920, height: 1080), television: true),
                    interaction: Int = 1, visible: Bool = true, menuOpen: Bool = false,
                    closed: Bool = false, active: Bool = true, focus: String? = nil,
                    accessibilityFocus: String? = nil) -> String? {
            state.target(request, layout: layout, interaction: interaction, visible: visible,
                menuOpen: menuOpen, closed: closed, active: active, focus: focus, accessibilityFocus: accessibilityFocus)
        }
        let televisionLayout = NativePlayerStageLayout(size: CGSize(width: 1920, height: 1080), television: true)
        var menuReturn = NativePlayerMenuReturn()
        precondition(target(menuReturn, nil) == nil, "Initial TV focus has no restore intent and still permits idle")
        menuReturn.schedule(anchor: "volume", interaction: 1, focus: "menu.close", accessibilityFocus: nil)
        let firstReturn = menuReturn.pending
        precondition(menuReturn.preservesEngagement(interaction: 1, visible: true, menuOpen: false, closed: false, active: true))
        precondition(!menuReturn.preservesEngagement(interaction: 2, visible: true, menuOpen: false, closed: false, active: true))
        var engagedReturn = NativePlayerChromeState()
        engagedReturn.televisionInput(.navigation, focus: "volume")
        if !menuReturn.preservesEngagement(interaction: 1, visible: true, menuOpen: false, closed: false, active: true) {
            engagedReturn.releaseTelevisionFocus()
        }
        precondition(engagedReturn.televisionControlEngaged, "A modal handoff preserves prior human intent across transient nil focus")
        let unengagedReturn = NativePlayerChromeState()
        precondition(!unengagedReturn.televisionControlEngaged, "The return policy never manufactures engagement")
        precondition(target(menuReturn, firstReturn, menuOpen: true) == nil, "No focus request before toolbar eligibility")
        precondition(menuReturn.nativeFocusChanged("back", interaction: 1, visible: true,
            menuOpen: false, closed: false, active: true), "Automatic engine fallback is not new human intent")
        precondition(target(menuReturn, firstReturn, focus: "back") == "volume")
        precondition(menuReturn.claim(firstReturn, layout: televisionLayout, interaction: 1,
            visible: true, menuOpen: false, closed: false, active: true, focus: nil, accessibilityFocus: nil) == "volume")
        precondition(menuReturn.claim(firstReturn, layout: televisionLayout, interaction: 1,
            visible: true, menuOpen: false, closed: false, active: true, focus: nil, accessibilityFocus: nil) == nil,
            "One generation issues at most one native anchor assignment, never a timed retry")
        precondition(menuReturn.accessibilityChanged("volume", interaction: 1, visible: true,
            menuOpen: false, closed: false, active: true))
        precondition(menuReturn.pending == nil, "The matching VoiceOver acknowledgement completes the intent")
        precondition(menuReturn.nativeFocusChanged("volume", interaction: 1, visible: true,
            menuOpen: false, closed: false, active: true), "The subsequent native acknowledgement is still our own")
        menuReturn.schedule(anchor: "volume", interaction: 1, focus: "menu.close", accessibilityFocus: nil)
        let cancelledReturn = menuReturn.pending
        precondition(target(menuReturn, cancelledReturn, focus: "tracks") == "volume",
            "A raw native focus value alone does not prove new keyboard input")
        precondition(target(menuReturn, cancelledReturn, accessibilityFocus: "tracks") == nil, "New VoiceOver focus wins")
        precondition(target(menuReturn, cancelledReturn, interaction: 2) == nil, "New interaction wins before observation callbacks")
        for flag in 0..<4 {
            precondition(target(menuReturn, cancelledReturn, visible: flag != 0, menuOpen: flag == 1,
                closed: flag == 2, active: flag != 3) == nil)
        }
        menuReturn.cancel()
        precondition(target(menuReturn, cancelledReturn) == nil, "Hide, teardown, closed, input and menu-open invalidate the generation")
        menuReturn.schedule(anchor: "tracks", interaction: 2, focus: "menu.close", accessibilityFocus: nil)
        precondition(target(menuReturn, cancelledReturn, interaction: 2) == nil, "Close A then open B cannot restore A")
        precondition(target(menuReturn, menuReturn.pending, interaction: 2) == "tracks")
        menuReturn.cancel()
        precondition(menuReturn.pending == nil, "The actual input observer cancels before human focus moves")
        menuReturn.schedule(anchor: "volume", interaction: 1, focus: nil, accessibilityFocus: nil)
        let televisionCompact = NativePlayerStageLayout(size: CGSize(width: 120, height: 150), television: true)
        precondition(!televisionCompact.controls.contains(.volume))
        precondition(target(menuReturn, menuReturn.pending, layout: televisionCompact) == "session",
            "Only a genuinely absent saved anchor uses the current layout's semantic fallback")
        menuReturn.schedule(anchor: "back", interaction: 1, focus: nil, accessibilityFocus: nil)
        precondition(target(menuReturn, menuReturn.pending, layout: televisionCompact) == "back")
        for _ in 0..<32 {
            let obsolete = menuReturn.pending
            menuReturn.schedule(anchor: "session", interaction: 1, focus: nil, accessibilityFocus: nil)
            precondition(target(menuReturn, obsolete) == nil)
        }
        var observedReturn = NativePlayerMenuReturn()
        for _ in 0..<28 { observedReturn.cancel() }
        observedReturn.schedule(anchor: "volume", interaction: 29, focus: "menu.close", accessibilityFocus: nil)
        let observedRequest = observedReturn.pending!
        precondition(observedRequest.generation == 29)
        precondition(observedReturn.claim(observedRequest, layout: televisionLayout, interaction: 29, visible: true,
            menuOpen: false, closed: false, active: true, focus: "menu.close", accessibilityFocus: nil) == "volume")
        for value in [String?.none, "back"] {
            precondition(observedReturn.nativeFocusChanged(value, interaction: 29, visible: true,
                menuOpen: false, closed: false, active: true))
            precondition(observedReturn.registeredTarget(layout: televisionLayout, interaction: 29, visible: true,
                menuOpen: false, closed: false, active: true, accessibilityFocus: nil) == "volume")
        }
        precondition(observedReturn.claim(observedRequest, layout: televisionLayout, interaction: 29, visible: true,
            menuOpen: false, closed: false, active: true, focus: "back", accessibilityFocus: nil) == nil)
        precondition(observedReturn.accessibilityChanged("volume", interaction: 29, visible: true,
            menuOpen: false, closed: false, active: true) && observedReturn.pending == nil)
        precondition(observedReturn.registeredTarget(layout: televisionLayout, interaction: 29, visible: true,
            menuOpen: false, closed: false, active: true, accessibilityFocus: "volume") == "volume",
            "A local self acknowledgement cannot withdraw the registered native default preference")
        for (interaction, visible, menuOpen, closed, active, accessibilityFocus) in [
            (30, true, false, false, true, String?.none),
            (29, false, false, false, true, nil),
            (29, true, true, false, true, nil),
            (29, true, false, true, true, nil),
            (29, true, false, false, false, nil),
            (29, true, false, false, true, "tracks"),
        ] {
            precondition(observedReturn.registeredTarget(layout: televisionLayout, interaction: interaction,
                visible: visible, menuOpen: menuOpen, closed: closed, active: active,
                accessibilityFocus: accessibilityFocus) == nil)
        }
        observedReturn.cancel()
        precondition(observedReturn.registeredTarget(layout: televisionLayout, interaction: 29, visible: true,
            menuOpen: false, closed: false, active: true, accessibilityFocus: nil) == nil)
        print("TV menu-return generations, stable native preference, cancellation, VoiceOver and semantic layout fallback passed")
        let playing = NativePlayerVisibility()
        var state = NativePlayerChromeState()
        precondition(state.visible && state.menu == nil && state.revealFocus == "play")
        precondition(!state.televisionStageFocusable, "Visible controls exclude the full-stage focus target")
        precondition(state.expire(interaction: state.interaction, visibility: playing) && !state.visible)
        precondition(state.televisionStageFocusable, "Hidden controls permit the reveal-only stage focus target")
        state.reveal()
        precondition(!state.televisionStageFocusable)
        let obsolete = state.interaction
        state.reveal()
        precondition(!state.expire(interaction: obsolete, visibility: playing) && state.visible, "Stale idle deadlines cannot hide new interaction")
        for pinned in [
            NativePlayerVisibility(paused: true), NativePlayerVisibility(busy: true),
            NativePlayerVisibility(blocked: true), NativePlayerVisibility(closed: true),
            NativePlayerVisibility(notice: true), NativePlayerVisibility(scrubbing: true),
            NativePlayerVisibility(menuOpen: true), NativePlayerVisibility(controlFocused: true),
        ] {
            precondition(!state.expire(interaction: state.interaction, visibility: pinned) && state.visible)
        }
        for menu in [NativePlayerMenu.volume, .tracks, .session] {
            state.open(menu, anchor: menu.rawValue)
            precondition(state.visible && state.menu == menu && state.menuAnchor == menu.rawValue)
            precondition(!state.televisionStageFocusable, "Menus exclude stage focus")
            precondition(!state.expire(interaction: state.interaction, visibility: playing), "An open menu pins controls independently of caller flags")
            precondition(state.dismissMenu() == menu.rawValue && state.menu == nil && state.menuAnchor == nil)
            precondition(state.dismissMenu() == nil, "Escape consumes a menu dismissal once, before fullscreen or player exit")
        }
        precondition(!NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: false, voiceOver: false, focus: "play"),
                     "Initial or sticky TV focus must not prevent idle hiding")
        precondition(NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: true, voiceOver: false, focus: "play"),
                     "Human keyboard focus also pins TV controls")
        precondition(NativePlayerChromeState.focusPinsControls(television: false, keyboardInput: true, voiceOver: false, focus: "play"))
        precondition(!NativePlayerChromeState.focusPinsControls(television: false, keyboardInput: false, voiceOver: false, focus: "play"),
                     "Programmatic focus is not human keyboard input")
        precondition(NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: false, voiceOver: true, focus: "play"))
        precondition(!NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: false, voiceOver: true, focus: "stage"))
        precondition(NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: false, voiceOver: true,
            focus: "stage", accessibilityFocus: "play"), "VoiceOver uses accessibility focus, not only keyboard focus")
        precondition(!NativePlayerChromeState.focusPinsControls(television: false, keyboardInput: false, voiceOver: false,
            focus: "stage", accessibilityFocus: "play"))
        state.rememberFocus("mute")
        state.rememberFocus("stage")
        state.rememberFocus(nil)
        precondition(state.expire(interaction: state.interaction, visibility: playing) && state.revealFocus == "mute")
        state.reveal()
        precondition(state.visible && state.revealFocus == "mute", "The first remote reveal restores focus without a media action")
        for input in [NativePlayerTelevisionInput.navigation, .select] {
            var television = NativePlayerChromeState()
            television.rememberFocus("play")
            let defaultFocus = NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: false,
                voiceOver: false, focus: "play", televisionControlEngaged: television.televisionControlEngaged)
            precondition(television.expire(interaction: television.interaction, visibility: NativePlayerVisibility(controlFocused: defaultFocus)),
                          "Default/programmatic TV button focus permits idle")
            precondition(television.televisionStageFocusable)
            television.televisionInput(input, focus: "stage")
            precondition(!television.visible && !television.televisionControlEngaged,
                         "A hidden-stage press does not expose controls before the stage handles it")
            television.revealFromTelevisionStage(navigation: input == .navigation)
            television.rememberFocus("play")
            precondition(television.visible && !television.televisionControlEngaged && television.revealFocus == "play",
                          "First hidden D-pad/Select only reveals and restores; it neither pins nor plays")
            precondition(!television.televisionStageFocusable)
            precondition(television.expire(interaction: television.interaction, visibility: NativePlayerVisibility()))
        }
        var engaged = NativePlayerChromeState()
        engaged.televisionInput(.navigation, focus: "play")
        engaged.rememberFocus("forward")
        let navigationFocus = NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: false,
            voiceOver: false, focus: "forward", televisionControlEngaged: engaged.televisionControlEngaged)
        precondition(navigationFocus && !engaged.expire(interaction: engaged.interaction, visibility: NativePlayerVisibility(controlFocused: navigationFocus)),
                      "Human D-pad navigation pins the intentional control focus before Select")
        precondition(!engaged.televisionStageFocusable,
                     "Intentional toolbar navigation cannot enter the stage and restore the previous control")
        engaged.televisionInput(.select, focus: "forward")
        let selectedFocus = NativePlayerChromeState.focusPinsControls(television: true, keyboardInput: false,
            voiceOver: false, focus: "forward", televisionControlEngaged: engaged.televisionControlEngaged)
        precondition(selectedFocus && !engaged.expire(interaction: engaged.interaction, visibility: NativePlayerVisibility(controlFocused: selectedFocus)),
                     "Human D-pad navigation then Select preserves usable controls through idle")
        engaged.releaseTelevisionFocus()
        precondition(!engaged.televisionControlEngaged, "Pointer or stage focus exit releases intentional TV engagement")
        precondition(engaged.expire(interaction: engaged.interaction, visibility: NativePlayerVisibility()))
        engaged.reveal()
        engaged.televisionInput(.keyboard, focus: "play")
        precondition(engaged.televisionControlEngaged)
        engaged.revealFromTelevisionStage(navigation: false)
        precondition(!engaged.televisionControlEngaged, "An explicit stage reveal does not inherit stale human engagement")
        let compact = NativePlayerStageLayout(size: CGSize(width: 320, height: 568), television: false)
        precondition(compact.controls == [.play, .backward, .forward, .mute, .session, .fullscreen])
        precondition(NativePlayerStageLayout.sessionMenus.contains(.volume) && NativePlayerStageLayout.sessionMenus.contains(.tracks),
                     "At 320 points, primary seek stays visible and session options expose volume and every track menu")
        precondition(NativePlayerStageLayout(size: CGSize(width: 1200, height: 700), television: false).controls == NativePlayerControl.allCases)
        precondition(NativePlayerIdleTimer.delay == .seconds(3))
        let cancelled = Task { await NativePlayerIdleTimer.wait(for: .seconds(60)) }
        cancelled.cancel()
        let cancelledResult = await cancelled.value
        precondition(!cancelledResult, "Unmount cancellation cannot commit an idle transition")
        let elapsed = await NativePlayerIdleTimer.wait(for: .milliseconds(1))
        precondition(elapsed)
        for television in [false, true] {
            for size in [CGSize(width: 160, height: 180), CGSize(width: 240, height: 160),
                         CGSize(width: 320, height: 568), CGSize(width: 600, height: 240),
                         CGSize(width: 1920, height: 1080)] {
                let layout = NativePlayerStageLayout(size: size, television: television)
                precondition(layout.target >= (television ? 64 : 44))
                precondition(layout.toolbarWidth <= size.width - layout.inset * 2, "Narrow viewports use secondary options instead of squeezing targets")
                if layout.minimalHeight {
                    precondition(layout.target + layout.inset * 2 + layout.footerHeight <= size.height,
                                 "Short viewports reserve full-height header and footer targets")
                }
                precondition(layout.controls.contains(.session) && Set(layout.controls).count == layout.controls.count)
                let restored = layout.restoredFocus("volume")
                precondition(layout.controls.contains(where: { $0.rawValue == restored }))
                precondition(layout.restoredFocus("back") == "back")
                precondition(layout.restoredFocus("position", canSeek: false) != "position")
                precondition(layout.restoredFocus("position") == (television ? (layout.controls.contains(.play) ? "play" : "session") : "position"))
                if television { precondition(!layout.controls.contains(.fullscreen)) }
                precondition(layout.popup(for: .tracks, anchor: "session") == layout.popup(for: .session),
                             "Secondary menus retain the actual trigger anchor")
                for menu in [NativePlayerMenu.volume, .tracks, .session] {
                    let popup = layout.popup(for: menu)
                    precondition(popup.minX >= 0 && popup.minY >= 0 && popup.maxX <= size.width && popup.maxY <= size.height)
                    precondition(popup.width > 0 && popup.height > 0)
                    precondition(layout.size == size, "Menus do not participate in video stage sizing")
                }
            }
        }
    }
}
