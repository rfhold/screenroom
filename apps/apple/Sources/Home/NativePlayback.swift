import AVKit
import Observation
import SwiftUI

enum PlaybackTimeline {
    static func milliseconds(_ value: String) throws -> Int64 {
        guard !value.isEmpty, value.utf8.allSatisfy({ (48...57).contains($0) }),
              let number = Int64(value), number <= 922_337_203_685_477 else {
            throw CocoaError(.coderReadCorrupt)
        }
        return number
    }

    static func logical(seconds: Double, offset: Int64) -> Int64 {
        guard seconds.isFinite else { return offset }
        return min(922_337_203_685_477, max(0, Int64(min(max(0, seconds * 1000), 922_337_203_685_477)) + offset))
    }

    static func transport(position: Int64, offset: Int64) -> Double {
        Double(max(0, position - offset)) / 1000
    }
}

struct NativeSubtitleCue: Equatable {
    let start: Int64
    let end: Int64
    let text: String

    static func parse(_ data: Data) throws -> [NativeSubtitleCue] {
        guard data.count <= 2 * 1024 * 1024, let decoded = String(data: data, encoding: .utf8),
              decoded.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix("WEBVTT") else { throw CocoaError(.coderReadCorrupt) }
        let text = decoded.replacingOccurrences(of: "\r\n", with: "\n")
        var cues: [NativeSubtitleCue] = []
        for block in text.components(separatedBy: "\n\n") {
            let lines = block.components(separatedBy: "\n")
            guard let index = lines.firstIndex(where: { $0.contains(" --> ") }) else { continue }
            let times = lines[index].components(separatedBy: " --> ")
            guard times.count == 2, let endValue = times[1].split(separator: " ").first,
                  let start = timestamp(times[0]), let end = timestamp(String(endValue)), end > start else { throw CocoaError(.coderReadCorrupt) }
            let payload = lines.dropFirst(index + 1).joined(separator: "\n")
                .replacingOccurrences(of: "<[^>]*>", with: "", options: .regularExpression)
                .replacingOccurrences(of: "&lt;", with: "<").replacingOccurrences(of: "&gt;", with: ">")
                .replacingOccurrences(of: "&amp;", with: "&").replacingOccurrences(of: "&nbsp;", with: " ")
            guard cues.count < 10000, payload.utf8.count <= 16384 else { throw CocoaError(.coderReadCorrupt) }
            cues.append(NativeSubtitleCue(start: start, end: end, text: payload))
        }
        return cues
    }

    private static func timestamp(_ text: String) -> Int64? {
        let parts = text.split(separator: ":")
        guard (2...3).contains(parts.count), let seconds = parts.last else { return nil }
        let fractions = seconds.split(separator: ".")
        guard fractions.count == 2, fractions[1].count == 3,
              let sec = Int64(fractions[0]), (0..<60).contains(sec),
              let ms = Int64(fractions[1]), (0..<1000).contains(ms),
              let minutes = Int64(parts[parts.count - 2]), (0..<60).contains(minutes),
              let hours = parts.count == 3 ? Int64(parts[0]) : 0, hours >= 0, hours <= 256_204_778 else { return nil }
        return ((hours * 60 + minutes) * 60 + sec) * 1000 + ms
    }
}

struct PlaybackScrubBounds {
    let upperBound: Double
    let canSeek: Bool
    let step = 1.0
    var range: ClosedRange<Double> { 0...upperBound }

    init(duration: Int64) {
        let bounded = min(922_337_203_685_477, max(0, duration))
        upperBound = Double(max(1, bounded))
        canSeek = bounded > 0
    }

    func clamp(_ value: Double) -> Double {
        guard value.isFinite else { return 0 }
        return min(upperBound, max(0, value))
    }

    func milliseconds(_ value: Double) -> Int64 { Int64(clamp(value)) }
}

private final class SubtitleRedirectPolicy: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

@MainActor @Observable
final class NativePlaybackPresentation {
    private(set) var fullscreen = false
    #if os(macOS)
    weak var window: NSWindow?
    #endif

    func setFullscreen(_ value: Bool) { fullscreen = value }

    func toggle() {
        #if os(macOS)
        window?.toggleFullScreen(nil)
        #else
        fullscreen.toggle()
        #endif
    }
}

enum NativePlaybackPreparationPhase: String {
    case audioSession = "audio-session"
    case create, accept, install, done, failure
}

enum NativePlaybackPreparationDiagnostic {
    static func code(_ error: Error?) -> String {
        guard let error else { return "none" }
        guard let error = error as? ClientError, case .Playback(let code) = error else { return "untyped" }
        switch code {
        case .unspecified: return "unknown"
        case .profileRequired: return "profile_required"
        case .profileChanged: return "profile_changed"
        case .sourceUnavailable: return "source_unavailable"
        case .itemUnavailable: return "item_unavailable"
        case .unsupportedMedia: return "unsupported_media"
        case .transcodingUnavailable: return "transcoding_unavailable"
        case .playbackAuthorizationUnavailable: return "playback_authorization_unavailable"
        case .playbackConflict: return "playback_conflict"
        case .sessionExpired: return "session_expired"
        case .operationConflict: return "operation_conflict"
        case .capacityExhausted: return "capacity_exhausted"
        case .upstreamUnavailable: return "upstream_unavailable"
        }
    }

    static func line(enabled: Bool, phase: NativePlaybackPreparationPhase, stage: NativePlaybackPreparationPhase,
                     error: Error? = nil, blocked: Bool, closed: Bool, session: Bool) -> String? {
        guard enabled else { return nil }
        return "ScreenroomPlaybackPreparation phase=\(phase.rawValue) stage=\(stage.rawValue) code=\(code(error)) blocked=\(blocked) closed=\(closed) session=\(session)"
    }
}

struct NativePlaybackSeekDiagnostic {
    enum Stage: String, Codable {
        case sliderEnded = "slider-ended", dispatch = "seek-dispatch", completion = "seek-completion", timelineUpdate = "timeline-update"
    }

    struct Record: Codable, Equatable {
        let operation: Int
        let stage: Stage
        let logicalTargetMs: Double
        let transportOffsetMs: Double
        var sliderReceivedMs: Double?
        var mediaRequestedMs: Double?
        var mediaActualMs: Double?
        var logicalActualMs: Double?
    }

    private struct Receipt: Encodable {
        let schemaVersion = 1
        let records: [Record]
    }

    static let limit = Double(922_337_203_685_477 as Int64)
    private(set) var records: [Record] = []
    private var operations = 0

    static func valid(_ value: Double) -> Bool { value.isFinite && (0...limit).contains(value) }

    mutating func begin(received: Double, target: Int64, offset: Int64) -> Int? {
        guard operations < 8, Self.valid(received), Self.valid(Double(target)), Self.valid(Double(offset)) else { return nil }
        operations += 1
        records.append(Record(operation: operations, stage: .sliderEnded, logicalTargetMs: Double(target),
                              transportOffsetMs: Double(offset), sliderReceivedMs: received))
        return operations
    }

    mutating func dispatch(_ operation: Int, target: Int64, offset: Int64, media: Double?) -> Bool {
        guard let previous = records.last(where: { $0.operation == operation }), previous.stage == .sliderEnded,
              records.count < 24, Self.valid(Double(target)), Self.valid(Double(offset)),
              media.map(Self.valid) ?? true else { return false }
        records.append(Record(operation: operation, stage: media == nil ? .timelineUpdate : .dispatch,
                              logicalTargetMs: Double(target), transportOffsetMs: Double(offset), mediaRequestedMs: media))
        return true
    }

    mutating func complete(_ operation: Int, media: Double, logical: Int64, offset: Int64) -> Bool {
        guard let previous = records.last(where: { $0.operation == operation }), previous.stage == .dispatch,
              previous.transportOffsetMs == Double(offset), records.count < 24,
              Self.valid(media), Self.valid(Double(logical)) else { return false }
        let record = Record(operation: operation, stage: .completion, logicalTargetMs: previous.logicalTargetMs,
                            transportOffsetMs: previous.transportOffsetMs, mediaRequestedMs: previous.mediaRequestedMs,
                            mediaActualMs: media, logicalActualMs: Double(logical))
        records.append(record)
        return true
    }

    var receipt: String? {
        guard !records.isEmpty else { return nil }
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        encoder.outputFormatting = [.sortedKeys]
        return (try? encoder.encode(Receipt(records: records))).flatMap { String(data: $0, encoding: .utf8) }
    }
}

@MainActor @Observable
final class NativePlayback {
    let player = AVPlayer()
    let presentation = NativePlaybackPresentation()
    private(set) var session: PlaybackSession?
    private(set) var busy = false {
        didSet {
            recordDiagnostic("busy")
            if !busy {
                let waiters = idleWaiters
                idleWaiters = []
                for waiter in waiters { waiter.resume() }
            }
        }
    }
    private var idleWaiters: [CheckedContinuation<Void, Never>] = []
    private(set) var message = ""
    private(set) var preparationDiagnostic: String?
    private(set) var seekDiagnostic: String?
    @ObservationIgnored private var seekTrace = NativePlaybackSeekDiagnostic()
    @ObservationIgnored private var receivedSliderPosition: Double?
    @ObservationIgnored private var pendingSeekDiagnostic: (operation: Int, generation: Int)?
    private(set) var blocked = false
    private(set) var position: Int64 = 0
    private(set) var duration: Int64 = 0
    private(set) var paused = true
    private(set) var closed = false
    private(set) var muted = false
    private(set) var volume: Float = 1
    private var subtitleCues: [NativeSubtitleCue] = []
    private(set) var subtitleWarning = ""
    var subtitle: String { subtitleCues.filter { $0.start <= position && position < $0.end }.map(\.text).joined(separator: "\n") }
    private var client: Client
    private var createRequest: CreatePlaybackSessionRequest
    private var updateRequest: UpdatePlaybackSessionRequest?
    private var stopRequest: StopPlaybackSessionRequest?
    private var stopAttempted = false
    private var presented = false
    private var startupTime: TimeInterval?
    private var sequence: UInt32 = 0
    private var offset: Int64 = 0
    private var installedRevision: UInt32 = 0
    private var mediaPositionReady = false
    private var started = false
    private var personalUncertain = false
    private var fallbackUsed = false
    private var loop: Task<Void, Never>?
    private var generation = 0
    private var finalPosition: Int64?
    private var exitReason = "player_exit"
    private var ticks = 0
    private let prepareTransport: ((PlaybackTransport) async throws -> Void)?
    private let readPosition: (() -> Double)?
    private let seekTransport: ((Double) async -> Void)?

    var diagnosticValue: String? {
        guard ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1" else { return nil }
        return seekDiagnostic ?? preparationDiagnostic
    }

    func recordSliderReceived(_ value: Double) {
        guard ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1" else { return }
        receivedSliderPosition = NativePlaybackSeekDiagnostic.valid(value) ? value : nil
    }

    func recordSliderEnded(target: Int64) -> Int? {
        guard ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1" else { return nil }
        defer { receivedSliderPosition = nil }
        guard let receivedSliderPosition, let operation = seekTrace.begin(received: receivedSliderPosition, target: target, offset: offset) else { return nil }
        seekDiagnostic = seekTrace.receipt
        return operation
    }

    private func recordSeekDispatch(_ operation: Int?, target: Int64, media: Double?) {
        guard ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1", let operation,
              seekTrace.dispatch(operation, target: target, offset: offset, media: media) else { return }
        seekDiagnostic = seekTrace.receipt
    }

    private func recordSeekCompletion(seconds: Double) {
        // Reuse the first existing position sample after the seek await; never add a media read.
        guard ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1", let pendingSeekDiagnostic,
              pendingSeekDiagnostic.generation == generation,
              seekTrace.complete(pendingSeekDiagnostic.operation, media: seconds * 1000, logical: position, offset: offset) else { return }
        self.pendingSeekDiagnostic = nil
        seekDiagnostic = seekTrace.receipt
    }

    func recordDiagnostic(_ event: String, enabled: Bool? = nil, phase: ScenePhase? = nil) {
        guard ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1" else { return }
        var fields = [
            "event=\(event)", "time=\(ProcessInfo.processInfo.systemUptime)",
            "volume=\(Int((volume * 100).rounded()))", "busy=\(busy)", "blocked=\(blocked)",
            "closed=\(closed)", "paused=\(paused)", "session=\(session != nil)",
            "generation=\(generation)", "sequence=\(sequence)"
        ]
        if let enabled { fields.append("environmentEnabled=\(enabled)") }
        if let phase { fields.append("phase=\(phase)") }
        #if os(macOS)
        fields.append("appActive=\(NSApp?.isActive == true)")
        fields.append("keyWindow=\(presentation.window?.isKeyWindow == true)")
        #endif
        print("ScreenroomPlaybackDiagnostic " + fields.joined(separator: " "))
    }

    init(client: Client, itemID: String, scope: SourceLibraryScope,
         prepareTransport: ((PlaybackTransport) async throws -> Void)? = nil,
         readPosition: (() -> Double)? = nil, seekTransport: ((Double) async -> Void)? = nil) {
        self.client = client
        self.prepareTransport = prepareTransport
        self.readPosition = readPosition
        self.seekTransport = seekTransport
        createRequest = CreatePlaybackSessionRequest(
            requestId: UUID().uuidString.lowercased(), itemId: itemID, scope: scope,
            clientCapabilities: PlaybackClientCapabilities(mp4: true, h264: true, aac: true, nativeHls: true, mseHls: false, webvtt: true),
            startPolicy: .resume, positionMs: nil)
        player.preventsDisplaySleepDuringVideoPlayback = true
        player.appliesMediaSelectionCriteriaAutomatically = false
    }

    func prepareOnPresentation() async {
        guard !presented else { return }
        presented = true
        startupTime = ProcessInfo.processInfo.systemUptime
        await start()
    }

    func firstFrameReady() {
        guard !closed, let startupTime, session?.transport != nil else { return }
        self.startupTime = nil
        #if !DEBUG
        recordPlaybackFirstFrame(elapsedMs: UInt64((ProcessInfo.processInfo.systemUptime - startupTime) * 1000))
        #endif
    }

    func start() async {
        guard session == nil, !busy, !closed else { return }
        busy = true
        message = "Preparing playback…"
        var stage = NativePlaybackPreparationPhase.audioSession
        recordPreparation(stage, stage: stage)
        do {
            #if !os(macOS)
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
            try AVAudioSession.sharedInstance().setActive(true)
            #endif
            stage = .create
            recordPreparation(stage, stage: stage)
            let result = try await client.createPlaybackSession(request: createRequest)
            stage = .accept
            recordPreparation(stage, stage: stage)
            guard let value = result.session else { throw CocoaError(.coderReadCorrupt) }
            try accept(value)
            if closed {
                busy = false
                await sendStop()
                return
            }
            stage = .install
            recordPreparation(stage, stage: stage)
            try await install(value)
            recordPreparation(.done, stage: .done)
        } catch {
            blocked = true
            message = preparationError(error)
            recordPreparation(.failure, stage: stage, error: error)
        }
        busy = false
    }

    private func recordPreparation(_ phase: NativePlaybackPreparationPhase, stage: NativePlaybackPreparationPhase,
                                   error: Error? = nil) {
        guard let line = NativePlaybackPreparationDiagnostic.line(
            enabled: ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1",
            phase: phase, stage: stage, error: error, blocked: blocked, closed: closed, session: session != nil) else { return }
        preparationDiagnostic = line
        print(line)
    }

    private func preparationError(_ error: Error) -> String {
        guard let error = error as? ClientError, case .Playback(let code) = error else {
            return "Playback could not be confirmed. Retry preparation uses the identical request; it never starts a second session."
        }
        switch code {
        case .profileRequired: return "Choose a Jellyfin profile before playback."
        case .profileChanged: return "The Jellyfin profile changed. Return to details and press Play again."
        case .unsupportedMedia: return "This media is unsupported. Native playback requires H.264/AAC MP4 or HLS."
        case .transcodingUnavailable: return "The server cannot provide a compatible transport."
        case .sourceUnavailable, .itemUnavailable: return "This title or its source is unavailable."
        case .sessionExpired: return "The playback session expired. Return to details and press Play again."
        case .capacityExhausted: return "The playback gateway has no available capacity."
        default: return "The gateway could not prepare playback. Retry uses the identical request."
        }
    }

    private func usable(_ value: PlaybackSession) -> Bool {
        !personalUncertain && ![PlaybackState.uncertain, .failed, .expired, .stopped, .stopping].contains(value.state)
            && ![PlaybackReportOutcome.uncertain, .pending, .failed].contains(value.reportOutcome)
            && value.pendingOperation == nil
    }

    private func accept(_ value: PlaybackSession) throws {
        if let current = session {
            guard value.sessionId == current.sessionId, value.transportRevision >= current.transportRevision else {
                throw CocoaError(.coderReadCorrupt)
            }
        }
        session = value
        sequence = max(sequence, value.lastAcceptedSequence)
    }

    private func install(_ value: PlaybackSession) async throws {
        guard !closed else { return }
        try accept(value)
        guard usable(value), let transport = value.transport else {
            suspend("Playback is unavailable or its personal-state outcome is uncertain. Check session state or stop.")
            return
        }
        guard installedRevision != transport.revision else { return }
        let nextOffset = try PlaybackTimeline.milliseconds(transport.timelineOffsetMs)
        let start = try PlaybackTimeline.milliseconds(transport.startPositionMs)
        let length = try transport.durationMs.map(PlaybackTimeline.milliseconds) ?? 0
        guard let url = URL(string: transport.mediaUrl) else { throw CocoaError(.coderReadCorrupt) }
        player.pause()
        mediaPositionReady = false
        if let prepareTransport { try await prepareTransport(transport) }
        else { player.replaceCurrentItem(with: AVPlayerItem(url: url)) }
        offset = nextOffset
        position = start
        duration = length
        installedRevision = transport.revision
        let current = generation
        await seekMedia(PlaybackTimeline.transport(position: start, offset: offset))
        guard !closed, generation == current else { return }
        mediaPositionReady = true
        if prepareTransport == nil, let item = player.currentItem,
           let group = try? await item.asset.loadMediaSelectionGroup(for: .legible) {
            guard !closed, player.currentItem === item else { return }
            item.select(nil, in: group)
        }
        guard !closed, generation == current else { return }
        await loadSubtitles(value)
        guard !closed, generation == current else { return }
        blocked = false
        paused = true
        message = "Ready. Press Play to start."
        beginLoop()
    }

    private func loadSubtitles(_ value: PlaybackSession) async {
        subtitleCues = []
        subtitleWarning = ""
        guard let track = value.tracks.first(where: { $0.kind == .subtitle && $0.selected && $0.available && $0.subtitleMode == .webvtt }),
              let address = track.mediaUrl, let url = URL(string: address) else { return }
        let config = URLSessionConfiguration.ephemeral
        config.httpCookieStorage = nil
        config.urlCache = nil
        config.timeoutIntervalForRequest = 10
        config.timeoutIntervalForResource = 10
        let loader = URLSession(configuration: config, delegate: SubtitleRedirectPolicy(), delegateQueue: nil)
        defer { loader.invalidateAndCancel() }
        do {
            let (stream, response) = try await loader.bytes(from: url)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else { throw CocoaError(.fileReadUnknown) }
            var bytes = Data()
            for try await byte in stream {
                guard bytes.count < 2 * 1024 * 1024, !closed else { throw CocoaError(.fileReadTooLarge) }
                bytes.append(byte)
            }
            guard !closed, value.transportRevision == installedRevision else { return }
            subtitleCues = try NativeSubtitleCue.parse(bytes)
        } catch {
            if !closed { subtitleWarning = "Subtitles could not be loaded. Choose another track or Off." }
        }
    }

    private func suspend(_ text: String) {
        capturePosition()
        player.pause()
        paused = true
        blocked = true
        message = text
    }

    private func capturePosition() {
        guard finalPosition == nil, installedRevision != 0,
              readPosition != nil || (mediaPositionReady && player.currentItem?.status == .readyToPlay) else { return }
        let seconds = readPosition?() ?? player.currentTime().seconds
        if seconds.isFinite {
            position = PlaybackTimeline.logical(seconds: seconds, offset: offset)
            recordSeekCompletion(seconds: seconds)
        }
    }

    private func seekMedia(_ seconds: Double) async {
        if let seekTransport { await seekTransport(seconds) }
        else {
            await player.seek(to: CMTime(seconds: seconds, preferredTimescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero)
        }
    }
    private func beginLoop() {
        guard loop == nil else { return }
        loop = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                guard let self, !self.closed else { return }
                self.capturePosition()
                if self.player.currentItem?.status == .failed {
                    self.suspend("This transport cannot be played. Check session state or request compatible H.264/AAC transport.")
                }
                if !self.paused, self.player.currentItem?.status == .readyToPlay,
                   self.player.currentItem?.duration.seconds.isFinite == true,
                   self.player.currentTime().seconds >= self.player.currentItem!.duration.seconds {
                    await self.close(reason: "ended")
                    return
                }
                self.ticks += 1
                if self.ticks % 15 == 0 { await self.heartbeat() }
            }
        }
    }

    func heartbeat() async {
        guard started, !busy, !blocked, !closed else { return }
        guard sequence < UInt32.max else { await close(reason: "sequence_exhausted"); return }
        await report(.heartbeat)
        if !paused, !blocked { await report(.progress) }
    }

    func toggle() async {
        guard !busy, !blocked, !closed, session != nil else { return }
        guard sequence < UInt32.max else { await close(reason: "sequence_exhausted"); return }
        if paused {
            let kind: PlaybackEventKind = started ? .resumed : .started
            started = true
            await report(kind, intendedPaused: false)
            guard !blocked, !closed else { return }
            paused = false
            player.play()
            message = ""
        } else {
            player.pause()
            paused = true
            await report(.paused, intendedPaused: true)
        }
    }

    func handleRemotePlayPause() async { await toggle() }

    func handleScenePhase(_ phase: ScenePhase) async {
        recordDiagnostic("scene", phase: phase)
        if phase != .active { await close(reason: "background") }
    }

    func seek(by seconds: Int64) async {
        guard !busy, !blocked, !closed, installedRevision != 0 else { return }
        capturePosition()
        await seek(to: position + seconds * 1000)
    }

    func seek(to milliseconds: Int64, diagnostic: Int? = nil) async {
        guard !busy, !blocked, !closed, installedRevision != 0 else { return }
        pendingSeekDiagnostic = nil
        let target = max(0, min(duration > 0 ? duration : 922_337_203_685_477, milliseconds))
        // Seeking before an HLS transport's origin requires a new gateway timeline.
        if target < offset {
            recordSeekDispatch(diagnostic, target: target, media: nil)
            await update(choices: nil, fallback: nil, target: target)
            return
        }
        busy = true
        let current = generation
        let mediaTarget = PlaybackTimeline.transport(position: target, offset: offset)
        recordSeekDispatch(diagnostic, target: target, media: mediaTarget * 1000)
        await seekMedia(mediaTarget)
        busy = false
        guard !closed, generation == current else { return }
        position = target
        if ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1", let diagnostic {
            pendingSeekDiagnostic = (diagnostic, current)
        }
        if started { await report(.seeked) }
    }

    private func report(_ kind: PlaybackEventKind, intendedPaused: Bool? = nil) async {
        guard let value = session, !closed, !blocked, !busy, sequence < UInt32.max else { return }
        capturePosition()
        sequence += 1
        let request = ReportPlaybackEventRequest(sessionId: value.sessionId, controlCapability: value.controlCapability,
            transportRevision: value.transportRevision, sequence: sequence, kind: kind,
            positionMs: String(position), paused: intendedPaused ?? paused, muted: player.isMuted, volume: Double(player.volume))
        busy = true
        do {
            let response = try await client.reportPlaybackEvent(request: request)
            guard let next = response.session else { throw CocoaError(.coderReadCorrupt) }
            try accept(next)
            guard !closed else { busy = false; return }
            guard usable(next), response.reportOutcome == .confirmed else {
                personalUncertain = true
                suspend("Personal playback reporting is not confirmed. Check session state; reports will not replay automatically.")
                busy = false
                return
            }
        } catch {
            personalUncertain = true
            if !closed {
                suspend("Playback reporting is uncertain. Check session state or stop; no report is automatically replayed.")
                await reconcileAfterFailure()
            }
        }
        busy = false
    }

    func check() async {
        guard let value = session, !busy else { return }
        busy = true
        do {
            let response = try await client.getPlaybackSession(request: GetPlaybackSessionRequest(sessionId: value.sessionId, controlCapability: value.controlCapability))
            guard let next = response.session else { throw CocoaError(.coderReadCorrupt) }
            try accept(next)
            guard !closed else {
                message = next.state == .stopped && !next.cleanupPending ? "Session stopped. Earlier uncertain personal reports remain unknown." : "Provider cleanup or final reporting is not confirmed."
                busy = false
                return
            }
            if usable(next), next.transport != nil {
                try await install(next)
                blocked = false
                message = paused ? "Session checked. Press Play to continue." : "Session checked."
            } else { suspend("Session remains unavailable or uncertain. Stop playback; a check does not prove a personal reporting outcome.") }
        } catch { message = "Session state could not be checked. No playback was replayed." }
        busy = false
    }

    private func reconcileAfterFailure() async {
        guard let value = session else { return }
        if let response = try? await client.getPlaybackSession(request: GetPlaybackSessionRequest(sessionId: value.sessionId, controlCapability: value.controlCapability)),
           let next = response.session {
            try? accept(next)
        }
    }

    func choose(_ track: PlaybackTrack?) async {
        let choices = PlaybackTrackChoices(audioTrackId: track?.kind == .audio ? track?.id : nil,
            subtitleTrackId: track?.kind == .subtitle ? track?.id : nil, subtitlesOff: track == nil)
        await update(choices: choices, fallback: nil)
    }

    func compatibleTransport() async {
        guard !fallbackUsed else { return }
        fallbackUsed = true
        await update(choices: nil, fallback: true)
    }

    func toggleMute() {
        muted.toggle()
        player.isMuted = muted
    }

    func adjustVolume(by delta: Float) {
        recordDiagnostic("volume.begin")
        volume = min(1, max(0, volume + delta))
        player.volume = volume
        recordDiagnostic("volume.end")
    }

    private func update(choices: PlaybackTrackChoices?, fallback: Bool?, target: Int64? = nil) async {
        guard let value = session, !busy, !closed, !personalUncertain, updateRequest == nil else { return }
        capturePosition()
        player.pause()
        paused = true
        let request = UpdatePlaybackSessionRequest(sessionId: value.sessionId, controlCapability: value.controlCapability,
            requestId: UUID().uuidString.lowercased(), expectedTransportRevision: value.transportRevision,
            positionMs: String(target ?? position), trackChoices: choices, compatibleTranscodeFallback: fallback)
        updateRequest = request
        await retryTransport()
    }

    func retryTransport() async {
        guard let request = updateRequest, !busy, !closed else { return }
        busy = true
        do {
            let response = try await client.updatePlaybackSession(request: request)
            guard let next = response.session else { throw CocoaError(.coderReadCorrupt) }
            try accept(next)
            guard !closed else { busy = false; return }
            try await install(next)
            if usable(next) { updateRequest = nil }
        } catch {
            suspend("Transport outcome is uncertain. Check session state or explicitly retry the identical transport request.")
            await reconcileAfterFailure()
        }
        busy = false
    }

    var canRetryTransport: Bool { updateRequest != nil }
    var canFallback: Bool { !fallbackUsed && session != nil && !closed && !personalUncertain && updateRequest == nil }

    func close(reason: String = "player_exit") async {
        guard !closed else { return }
        capturePosition()
        finalPosition = position
        exitReason = reason
        closed = true
        generation += 1
        player.pause()
        player.currentItem?.cancelPendingSeeks()
        player.currentItem?.asset.cancelLoading()
        player.replaceCurrentItem(with: nil)
        #if !os(macOS)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
        subtitleCues = []
        loop?.cancel()
        loop = nil
        // Detach immediately, then let any already dispatched control operation settle.
        while busy { await withCheckedContinuation { idleWaiters.append($0) } }
        await sendStop()
    }

    func retryStop() async {
        guard closed, stopRequest != nil else { return }
        await sendStop(retry: true)
    }

    private func sendStop(retry: Bool = false) async {
        guard let value = session, !busy, !stopAttempted || retry else { return }
        if stopRequest == nil {
            stopRequest = StopPlaybackSessionRequest(sessionId: value.sessionId, controlCapability: value.controlCapability,
                requestId: UUID().uuidString.lowercased(), finalSequence: sequence,
                transportRevision: value.transportRevision, positionMs: String(finalPosition ?? position), stopReason: exitReason)
        }
        stopAttempted = true
        busy = true
        do {
            let response = try await client.stopPlaybackSession(request: stopRequest!)
            guard let next = response.session else { throw CocoaError(.coderReadCorrupt) }
            try accept(next)
            if next.cleanupPending { message = "Playback stopped locally; provider cleanup is pending." }
            else if next.state == .stopped && next.reportOutcome == .confirmed { message = "Playback stopped." }
            else { message = "Playback stopped locally; the final personal report is not confirmed. Check session state or retry the identical Stop request." }
            if personalUncertain { message += " An earlier personal-report outcome remains unknown." }
        } catch {
            message = "Playback stopped locally. Server cleanup is unconfirmed. Retry Stop uses the same final position and request."
            await reconcileAfterFailure()
        }
        busy = false
    }
}

struct NativePlaybackView: View {
    let playback: NativePlayback
    let title: String
    let exit: () -> Void
    @Environment(\.scenePhase) private var phase

    var body: some View {
        NativePlaybackContent(playback: playback, title: title, exit: exit,
            renderMedia: renderInlineMedia)
            .task { await playback.prepareOnPresentation() }
            .onChange(of: phase) { _, value in
                Task { await playback.handleScenePhase(value) }
            }
            .onDisappear {
                #if os(iOS)
                // The presenter owns true-unmount cleanup while fullscreen.
                if playback.presentation.fullscreen { return }
                #endif
                Task { await playback.close(reason: "unmount") }
            }
            #if os(iOS)
            .background(PlaybackFullscreenPresenter(playback: playback, title: title, exit: exit))
            #elseif os(macOS)
            .background(PlaybackWindowAttachment(presentation: playback.presentation))
            #endif
    }

    private var renderInlineMedia: Bool {
        #if os(iOS)
        !playback.presentation.fullscreen
        #else
        true
        #endif
    }
}

enum NativePlayerMenu: String { case volume, tracks, session }
enum NativePlayerTelevisionInput { case navigation, select, keyboard }

enum NativePlayerControl: String, CaseIterable {
    case play, backward, forward, mute, volume, tracks, session, fullscreen
}

struct NativePlayerVisibility: Equatable {
    var paused = false
    var busy = false
    var blocked = false
    var closed = false
    var notice = false
    var scrubbing = false
    var menuOpen = false
    var controlFocused = false
    var holdsVisible: Bool { paused || busy || blocked || closed || notice || scrubbing || menuOpen || controlFocused }
}

struct NativePlayerChromeState {
    private(set) var visible = true
    private(set) var menu: NativePlayerMenu?
    private(set) var interaction = 0
    private(set) var revealFocus = "play"
    private(set) var televisionControlEngaged = false
    private var returnFocus: String?
    var menuAnchor: String? { menu == nil ? nil : returnFocus }
    var televisionStageFocusable: Bool { !visible && menu == nil }

    mutating func reveal() { visible = true; interaction += 1 }
    mutating func holdVisible() { visible = true }
    mutating func televisionInput(_ input: NativePlayerTelevisionInput, focus: String?) {
        guard visible, focus != nil, input != .select || focus != "stage" else { return }
        televisionControlEngaged = true
        reveal()
    }
    mutating func revealFromTelevisionStage(navigation: Bool) {
        if !visible || !navigation { releaseTelevisionFocus() }
        reveal()
    }
    mutating func releaseTelevisionFocus() { televisionControlEngaged = false }
    mutating func rememberFocus(_ value: String?) {
        guard let value, value != "stage", !value.hasPrefix("menu."), !value.hasPrefix("track.") else { return }
        revealFocus = value
    }
    mutating func open(_ value: NativePlayerMenu, anchor: String) {
        menu = value
        returnFocus = anchor
        reveal()
    }
    mutating func dismissMenu() -> String? {
        guard menu != nil else { return nil }
        menu = nil
        reveal()
        return returnFocus
    }
    static func focusPinsControls(television: Bool, keyboardInput: Bool, voiceOver: Bool, focus: String?,
                                  accessibilityFocus: String? = nil, televisionControlEngaged: Bool = false) -> Bool {
        let control = focus != nil && focus != "stage"
        let accessibleControl = accessibilityFocus != nil && accessibilityFocus != "stage"
        return ((keyboardInput || (television && televisionControlEngaged)) && control) || (voiceOver && (control || accessibleControl))
    }
    func canExpire(interaction expected: Int, visibility: NativePlayerVisibility) -> Bool {
        expected == interaction && menu == nil && !visibility.holdsVisible
    }
    @discardableResult
    mutating func expire(interaction expected: Int, visibility: NativePlayerVisibility) -> Bool {
        guard canExpire(interaction: expected, visibility: visibility) else { return false }
        releaseTelevisionFocus()
        visible = false
        return true
    }
}

enum NativePlayerIdleTimer {
    static let delay: Duration = .seconds(3)
    static func wait(for delay: Duration = delay) async -> Bool {
        do { try await Task.sleep(for: delay); return !Task.isCancelled }
        catch { return false }
    }
}

struct NativePlayerStageLayout {
    static let sessionMenus: [NativePlayerMenu] = [.tracks, .volume]
    let size: CGSize
    let television: Bool
    var target: CGFloat { television ? 72 : 44 }
    var gap: CGFloat { television ? 10 : 4 }
    var compactHeight: Bool { size.height < (television ? 400 : 280) }
    var minimalHeight: Bool { size.height < (television ? 260 : 190) }
    var inset: CGFloat { compactHeight ? 4 : min(television ? 48 : 20, max(8, min(size.width, size.height) * 0.04)) }
    var footerHeight: CGFloat {
        if television && minimalHeight { return target + 8 }
        return target + (television ? 32 : (minimalHeight ? 44 : 62)) + inset * 2 + (compactHeight ? 8 : 20)
    }
    var controls: [NativePlayerControl] {
        let slots = Int(max(0, size.width - inset * 2 + gap) / (target + gap))
        let full: [NativePlayerControl] = television
            ? [.play, .backward, .forward, .mute, .volume, .tracks, .session]
            : NativePlayerControl.allCases
        if slots >= full.count { return full }
        if slots >= 6 { return television ? [.play, .backward, .forward, .mute, .tracks, .session] : [.play, .backward, .forward, .mute, .session, .fullscreen] }
        if slots >= 4 { return [.play, .mute, .tracks, .session] }
        if slots >= 3 { return [.play, .tracks, .session] }
        return slots >= 2 ? [.play, .session] : [.session]
    }
    var toolbarWidth: CGFloat { CGFloat(controls.count) * target + CGFloat(max(0, controls.count - 1)) * gap }
    func restoredFocus(_ saved: String, canSeek: Bool = true) -> String {
        if saved == "back" || (!television && saved == "position" && canSeek) || controls.contains(where: { $0.rawValue == saved }) { return saved }
        return controls.contains(.play) ? "play" : "session"
    }
    func popup(for menu: NativePlayerMenu, anchor: String? = nil) -> CGRect {
        let width = min(television ? 520 : 340, max(0, size.width - inset * 2))
        let height = min(television ? 500 : 360, max(0, size.height - inset * 2))
        let control = NativePlayerControl(rawValue: anchor ?? menu.rawValue) ?? .session
        let index = controls.firstIndex(of: control) ?? controls.firstIndex(of: .session) ?? 0
        let anchorX = (size.width - toolbarWidth) / 2 + CGFloat(index) * (target + gap) + target / 2
        let x = min(max(inset, anchorX - width / 2), max(inset, size.width - inset - width))
        let y = max(inset, size.height - footerHeight - 8 - height)
        return CGRect(x: x, y: min(y, max(inset, size.height - inset - height)), width: width, height: height)
    }
}

struct NativePlayerMenuReturn {
    struct Request: Equatable {
        let generation: Int
        let anchor: String
        let interaction: Int
        let originFocus: String?
        let originAccessibilityFocus: String?
    }
    private(set) var generation = 0
    private(set) var pending: Request?
    private(set) var issued = false
    private(set) var issuedTarget: String?
    private var issuedRequest: Request?
    var diagnosticIssuedGeneration: Int? { issuedRequest?.generation }

    mutating func cancel() {
        generation += 1
        pending = nil
        issued = false
        issuedTarget = nil
        issuedRequest = nil
    }

    mutating func schedule(anchor: String, interaction: Int, focus: String?, accessibilityFocus: String?) {
        cancel()
        pending = Request(generation: generation, anchor: anchor, interaction: interaction,
                          originFocus: focus, originAccessibilityFocus: accessibilityFocus)
    }

    mutating func nativeFocusChanged(_ value: String?, interaction: Int, visible: Bool,
                                    menuOpen: Bool, closed: Bool, active: Bool) -> Bool {
        if acknowledges(value, interaction: interaction, visible: visible, menuOpen: menuOpen, closed: closed, active: active) {
            pending = nil
            return true
        }
        let returning = preservesEngagement(interaction: interaction, visible: visible, menuOpen: menuOpen, closed: closed, active: active)
        if pending != nil && !returning { cancel() }
        return returning
    }

    mutating func accessibilityChanged(_ value: String?, interaction: Int, visible: Bool,
                                      menuOpen: Bool, closed: Bool, active: Bool) -> Bool {
        let selfAcknowledged = acknowledges(value, interaction: interaction, visible: visible,
            menuOpen: menuOpen, closed: closed, active: active)
        if selfAcknowledged { pending = nil }
        else if value != nil { cancel() }
        return selfAcknowledged
    }

    private func acknowledges(_ value: String?, interaction: Int, visible: Bool,
                              menuOpen: Bool, closed: Bool, active: Bool) -> Bool {
        guard let issuedRequest else { return false }
        return issued && value != nil && value == issuedTarget && issuedRequest.generation == generation &&
            issuedRequest.interaction == interaction && visible && !menuOpen && !closed && active
    }

    func preservesEngagement(interaction: Int, visible: Bool, menuOpen: Bool, closed: Bool, active: Bool) -> Bool {
        guard let pending else { return false }
        return pending.generation == generation && pending.interaction == interaction &&
            visible && !menuOpen && !closed && active
    }

    func target(_ request: Request?, layout: NativePlayerStageLayout, interaction: Int,
                visible: Bool, menuOpen: Bool, closed: Bool, active: Bool,
                focus _: String?, accessibilityFocus: String?) -> String? {
        guard let request, request == pending, request.generation == generation,
              request.interaction == interaction, visible, !menuOpen, !closed, active,
              accessibilityFocus == nil || accessibilityFocus == request.originAccessibilityFocus ||
                (issued && accessibilityFocus == issuedTarget) else { return nil }
        return layout.restoredFocus(request.anchor)
    }

    func registeredTarget(layout: NativePlayerStageLayout, interaction: Int, visible: Bool,
                          menuOpen: Bool, closed: Bool, active: Bool, accessibilityFocus: String?) -> String? {
        guard issued, let issuedRequest, let issuedTarget,
              issuedRequest.generation == generation, issuedRequest.interaction == interaction,
              visible, !menuOpen, !closed, active,
              accessibilityFocus == nil || accessibilityFocus == issuedRequest.originAccessibilityFocus ||
                accessibilityFocus == issuedTarget else { return nil }
        return layout.restoredFocus(issuedTarget)
    }

    mutating func claim(_ request: Request?, layout: NativePlayerStageLayout, interaction: Int,
                        visible: Bool, menuOpen: Bool, closed: Bool, active: Bool,
                        focus: String?, accessibilityFocus: String?) -> String? {
        guard !issued, let target = target(request, layout: layout, interaction: interaction,
            visible: visible, menuOpen: menuOpen, closed: closed, active: active,
            focus: focus, accessibilityFocus: accessibilityFocus) else { return nil }
        issued = true
        issuedTarget = target
        issuedRequest = request
        return target
    }
}

final class NativePlayerFocusTrace {
    enum Event: String {
        case dismissBefore, dismissAfter, scheduleBefore, scheduleAfter, toolbarAppear, claimBefore, claimAfter
        case preferenceEligible, defaultPreference
        case assignmentBefore, assignmentAfter, nativeBefore, nativeAfter, accessibilityBefore, accessibilityAfter
        case inputBefore, inputAfter, hideBefore, hideAfter, closedBefore, closedAfter
        case disappearBefore, disappearAfter, menuBefore, menuAfter, exitBefore, exitAfter
        case interactionBefore, interactionAfter, preferred, closedAppearanceBefore, closedAppearanceAfter
    }
    enum Outcome: String { case observed, accepted, rejected, preserved, cancelled }
    enum Control: String {
        case back, play, forward, backward, mute, volume, tracks, session, stage
        case menuClose = "menu.close"
        case other, missing
        init(_ value: String?) { self = value.flatMap(Self.init(rawValue:)) ?? (value == nil ? .missing : .other) }
    }
    enum Eligibility: String {
        case eligible, inactive, hidden, menuOpen, closed, capturedGeneration, capturedInteraction
        case missingRequest, staleRequest, staleInteraction, accessibilityMismatch, alreadyIssued
    }
    struct State: Equatable {
        let generation: Int
        let interaction: Int
        let requestGeneration: Int?
        let pendingGeneration: Int?
        let issuedGeneration: Int?
        let capturedGeneration: Int?
        let capturedInteraction: Int?
        let anchor: Control
        let focusState: Control
        let accessibilityState: Control
        let callbackValue: Control
        let target: Control
        let pending: Bool
        let issued: Bool
        let visible: Bool
        let menuOpen: Bool
        let closed: Bool
        let active: Bool
        let eligibility: Eligibility
        var json: [String: Any] {
            ["generation": generation, "interaction": interaction,
             "request_generation": requestGeneration as Any? ?? NSNull(),
             "pending_generation": pendingGeneration as Any? ?? NSNull(),
             "issued_generation": issuedGeneration as Any? ?? NSNull(),
             "captured_generation": capturedGeneration as Any? ?? NSNull(),
             "captured_interaction": capturedInteraction as Any? ?? NSNull(),
             "anchor": anchor.rawValue, "focus_state": focusState.rawValue,
             "accessibility_state": accessibilityState.rawValue, "callback_value": callbackValue.rawValue,
             "target": target.rawValue, "pending": pending, "issued": issued, "visible": visible,
             "menu_open": menuOpen, "closed": closed, "active": active, "eligibility": eligibility.rawValue,
             "eligible": eligibility == .eligible]
        }
    }
    let enabled: Bool
    private(set) var records: [[String: Any]] = []
    private var armed = false
    private var lastPreferred: State?
    private var lastDefault: State?
    private let clock: () -> Double
    private let sink: (String) -> Void

    static func isEnabled(environment: [String: String], television: Bool) -> Bool {
        television && environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1" &&
            environment["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] == "1"
    }
    convenience init() {
        #if os(tvOS)
        let television = true
        #else
        let television = false
        #endif
        self.init(enabled: Self.isEnabled(environment: ProcessInfo.processInfo.environment, television: television))
    }
    init(enabled: Bool, clock: @escaping () -> Double = { ProcessInfo.processInfo.systemUptime },
         sink: @escaping (String) -> Void = { print($0) }) {
        self.enabled = enabled
        self.clock = clock
        self.sink = sink
    }
    func record(_ event: Event, outcome: Outcome = .observed, state: State) {
        guard enabled, records.count < 128 else { return }
        guard [state.generation, state.interaction].allSatisfy({ $0 >= 0 }),
              [state.requestGeneration, state.pendingGeneration, state.issuedGeneration,
               state.capturedGeneration, state.capturedInteraction].compactMap({ $0 }).allSatisfy({ $0 >= 0 }) else { return }
        if event == .dismissBefore && state.menuOpen { armed = true }
        if event == .scheduleBefore { armed = true }
        guard armed || event == .menuBefore || event == .menuAfter else { return }
        if event == .preferred {
            guard state != lastPreferred else { return }
            lastPreferred = state
        }
        if event == .defaultPreference {
            guard state != lastDefault else { return }
            lastDefault = state
        }
        let time = clock()
        guard time.isFinite, time >= 0 else { return }
        let record: [String: Any] = ["schema_version": 1, "sequence": records.count + 1,
            "event": event.rawValue, "outcome": outcome.rawValue, "time": time,
            "capped": records.count == 127, "state": state.json]
        guard let data = try? JSONSerialization.data(withJSONObject: record, options: [.sortedKeys]),
              let text = String(data: data, encoding: .utf8) else { return }
        records.append(record)
        sink("ScreenroomPlayerFocusTrace " + text)
    }
}

final class NativePlayerNativeItemTrace {
    enum Event: String { case arm, attached, detached, updated, failed, localSample }
    enum Kind: String { case view, other, none }
    enum Relation: String { case owned, other, unknown, missing }
    enum FrameSpace: String { case window, missingContainer, invalid, unowned, missing }
    struct Frame {
        let x: Double
        let y: Double
        let width: Double
        let height: Double
        init?(_ rect: CGRect) {
            let values = [rect.origin.x, rect.origin.y, rect.size.width, rect.size.height].map(Double.init)
            guard values.allSatisfy({ $0.isFinite && abs($0) <= 1_000_000 }),
                  values[2] >= 0, values[3] >= 0 else { return nil }
            x = values[0]; y = values[1]; width = values[2]; height = values[3]
        }
        var json: [String: Double] { ["x": x, "y": y, "width": width, "height": height] }
    }
    struct Item {
        let ordinal: Int?
        let kind: Kind
        let control: NativePlayerFocusTrace.Control
        let relation: Relation
        let canBecomeFocused: Bool?
        let frameSpace: FrameSpace
        let frame: Frame?
        static let missing = Item(ordinal: nil, kind: .none, control: .missing, relation: .missing,
                                  canBecomeFocused: nil, frameSpace: .missing, frame: nil)
        var json: [String: Any] {
            ["present": kind != .none, "ordinal": ordinal as Any? ?? NSNull(), "kind": kind.rawValue,
             "control": control.rawValue, "relation": relation.rawValue,
             "can_become_focused": canBecomeFocused as Any? ?? NSNull(),
             "frame_space": frameSpace.rawValue, "frame": frame?.json as Any? ?? NSNull()]
        }
    }
    struct Snapshot {
        let window: Int
        let system: Int?
        let keyWindow: Bool
        let viewport: Frame?
        let context: Bool
        let previous: Item
        let next: Item
        let current: Item
        let anchor: Item
        var valid: Bool {
            guard (1...128).contains(window), system.map({ (1...128).contains($0) }) ?? true else { return false }
            for item in [previous, next, current, anchor] {
                if let ordinal = item.ordinal, !(1...128).contains(ordinal) { return false }
                if (item.frameSpace == .window) != (item.frame != nil) { return false }
                if item.kind == .none {
                    guard item.ordinal == nil, item.control == .missing, item.relation == .missing,
                          item.canBecomeFocused == nil, item.frameSpace == .missing else { return false }
                } else if item.control == .missing || item.relation == .missing { return false }
                if item.frame != nil && item.relation != .owned { return false }
            }
            return true
        }
        var json: [String: Any] {
            ["window": window, "system": system as Any? ?? NSNull(), "key_window": keyWindow,
             "viewport": viewport?.json as Any? ?? NSNull(), "context_present": context,
             "previous": previous.json, "next": next.json, "current": current.json, "anchor": anchor.json]
        }
    }
    private final class Entry {
        weak var object: AnyObject?
        init(_ object: AnyObject) { self.object = object }
    }
    let enabled: Bool
    private(set) var armed = false
    private(set) var anchorCaptured = false
    private(set) weak var windowObject: AnyObject?
    private(set) weak var anchorObject: AnyObject?
    private(set) var records: [[String: Any]] = []
    private var entries: [Entry] = []
    private var invalidDropped = 0
    private var ignoredNotifications = 0
    private let clock: () -> Double
    private let sink: (String) -> Void
    var canRecord: Bool { enabled && armed && windowObject != nil && records.count < 128 }
    convenience init() {
        #if os(tvOS)
        let television = true
        #else
        let television = false
        #endif
        self.init(enabled: NativePlayerFocusTrace.isEnabled(
            environment: ProcessInfo.processInfo.environment, television: television))
    }
    init(enabled: Bool, clock: @escaping () -> Double = { ProcessInfo.processInfo.systemUptime },
         sink: @escaping (String) -> Void = { print($0) }) {
        self.enabled = enabled; self.clock = clock; self.sink = sink
    }
    func bind(_ window: AnyObject) { if enabled { windowObject = window } }
    func unbind() { windowObject = nil; anchorObject = nil }
    func arm(anchor: AnyObject?) -> Bool {
        guard enabled, !armed, windowObject != nil else { return false }
        armed = true
        anchorObject = anchor
        anchorCaptured = anchor != nil
        return true
    }
    func ordinal(_ object: AnyObject?) -> Int? {
        guard enabled, let object else { return nil }
        if let index = entries.firstIndex(where: { $0.object === object }) { return index + 1 }
        guard entries.count < 128 else { return nil }
        entries.append(Entry(object))
        return entries.count
    }
    static func acceptsNotification(systemMatches: Bool?, previous: Relation, next: Relation, current: Relation) -> Bool {
        guard systemMatches != false else { return false }
        if previous == .owned || next == .owned { return true }
        guard systemMatches == true, previous != .other, next != .other, current != .other else { return false }
        return current == .owned || [previous, next, current].allSatisfy({ $0 == .missing })
    }
    func ignored() { if canRecord { ignoredNotifications = min(1_000_000, ignoredNotifications + 1) } }
    static func control(_ identifier: String?) -> NativePlayerFocusTrace.Control {
        guard let identifier, identifier.utf8.prefix(65).count <= 64 else { return .other }
        let raw = identifier.hasPrefix("playback.") ? String(identifier.dropFirst(9)) : identifier
        let mapped = NativePlayerFocusTrace.Control(raw)
        return mapped == .missing ? .other : mapped
    }
    func record(_ event: Event, generation: Int, interaction: Int, snapshot: () -> Snapshot?) {
        guard canRecord else { return }
        let time = clock()
        guard generation >= 0, interaction >= 0, generation <= 1_000_000, interaction <= 1_000_000,
              time.isFinite, time >= 0, time <= 1_000_000_000_000, let snapshot = snapshot(), snapshot.valid,
              snapshot.anchor.kind == .none || anchorCaptured else {
            invalidDropped = min(1_000_000, invalidDropped + 1)
            return
        }
        let record: [String: Any] = ["schema_version": 1, "sequence": records.count + 1,
            "event": event.rawValue, "time": time, "generation": generation, "interaction": interaction,
            "state_source": "latestProductHook",
            "capped": records.count == 127, "invalid_dropped": invalidDropped,
            "notifications_ignored": ignoredNotifications, "anchor_captured": anchorCaptured,
            "snapshot": snapshot.json]
        guard let data = try? JSONSerialization.data(withJSONObject: record, options: [.sortedKeys]),
              let text = String(data: data, encoding: .utf8) else { return }
        records.append(record)
        sink("ScreenroomPlayerNativeItemTrace " + text)
    }
}

private struct NativePlayerIdleToken: Equatable {
    let interaction: Int
    let visibility: NativePlayerVisibility
}

private struct NativePlayerButtonStyle: ButtonStyle {
    let focused: Bool
    var selected = false
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.white.opacity(enabled ? 1 : 0.35))
            .background(RoundedRectangle(cornerRadius: 10).fill(.white.opacity(configuration.isPressed ? 0.3 : focused ? 0.22 : selected ? 0.14 : 0.04)))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(.white.opacity(focused ? 0.95 : 0), lineWidth: 2))
    }
}

private struct NativePlaybackContent: View {
    let playback: NativePlayback
    let title: String
    let exit: () -> Void
    let renderMedia: Bool
    @State private var chrome = NativePlayerChromeState()
    @State private var scrubPosition = 0.0
    @State private var scrubbing = false
    @State private var keyboardInput = false
    @FocusState private var focusedControl: String?
    @AccessibilityFocusState(for: .voiceOver) private var accessibleControl: String?
    @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
    #if os(tvOS)
    @Namespace private var toolbarFocusScope
    @State private var menuReturn = NativePlayerMenuReturn()
    @State private var televisionActive = false
    @State private var focusTrace = NativePlayerFocusTrace()
    @State private var nativeItemObservation = NativePlayerNativeItemObservation()
    #endif
    private var scrubBounds: PlaybackScrubBounds { PlaybackScrubBounds(duration: playback.duration) }
    private var unavailable: Bool { playback.busy || playback.blocked || playback.closed }
    private var hasNotice: Bool {
        !playback.subtitleWarning.isEmpty ||
            (!playback.message.isEmpty && playback.message != "Session checked." && !playback.message.hasPrefix("Ready."))
    }
    private var visibility: NativePlayerVisibility {
        NativePlayerVisibility(paused: playback.paused, busy: playback.busy, blocked: playback.blocked,
            closed: playback.closed, notice: hasNotice, scrubbing: scrubbing, menuOpen: chrome.menu != nil,
            controlFocused: NativePlayerChromeState.focusPinsControls(television: television,
                keyboardInput: keyboardInput, voiceOver: voiceOver, focus: focusedControl,
                accessibilityFocus: accessibleControl, televisionControlEngaged: chrome.televisionControlEngaged))
    }

    var body: some View {
        GeometryReader { geometry in
            let layout = NativePlayerStageLayout(size: geometry.size, television: television)
            ZStack {
                Color.black
                Group {
                    if renderMedia { NativeVideoSurface(player: playback.player, firstFrame: playback.firstFrameReady) }
                    else { Color.black }
                }
                .frame(width: geometry.size.width, height: geometry.size.height)
                .accessibilityHidden(true).allowsHitTesting(false)
                stageInput(layout).disabled(chrome.menu != nil).accessibilityHidden(chrome.menu != nil)
                if !playback.subtitle.isEmpty {
                    VStack {
                        Spacer(minLength: 0)
                        Text(playback.subtitle)
                            .font(television ? .title3 : .body).multilineTextAlignment(.center)
                            .foregroundStyle(.white).padding(8).background(.black.opacity(0.85))
                            .frame(maxWidth: min(900, geometry.size.width * 0.88))
                            .accessibilityIdentifier("playback.subtitles")
                    }
                    .padding(.bottom, chrome.visible ? layout.footerHeight + 12 : max(24, layout.inset))
                    .allowsHitTesting(false)
                }
                if playback.busy || hasNotice || playback.blocked || playback.closed {
                    statusOverlay(width: geometry.size.width)
                }
                Group {
                    if chrome.visible {
                        #if os(tvOS)
                        let generation = menuReturn.generation
                        let interaction = chrome.interaction
                        televisionToolbar(layout)
                            .focusScope(toolbarFocusScope)
                            .defaultFocus($focusedControl, defaultTelevisionFocus(layout), priority: .automatic)
                            .onChange(of: menuReturn.pending) { _, request in
                                guard let request else { return }
                                restoreToolbarFocus(request, generation: request.generation,
                                    interaction: request.interaction, layout: layout)
                            }
                            .onChange(of: chrome.menu) { _, menu in
                                if menu == nil && playback.closed {
                                    restoreClosedAccessibilityFocus(generation: generation, interaction: interaction)
                                }
                            }
                        #else
                        VStack(spacing: 0) {
                            header(layout)
                            Spacer(minLength: 0)
                            footer(layout)
                        }.transition(.opacity)
                        #endif
                    }
                }
                .disabled(chrome.menu != nil)
                .allowsHitTesting(chrome.visible && chrome.menu == nil)
                .accessibilityHidden(!chrome.visible || chrome.menu != nil)
                .animation(.easeOut(duration: 0.2), value: chrome.visible)
                if let menu = chrome.menu {
                    Color.black.opacity(0.2).contentShape(Rectangle())
                        .onTapGesture { dismissMenu() }
                        .accessibilityLabel("Dismiss playback menu")
                        .accessibilityAddTraits(.isButton)
                        .accessibilityAction { dismissMenu() }
                    let popup = layout.popup(for: menu, anchor: chrome.menuAnchor)
                    floatingMenu(menu, layout: layout)
                        .frame(width: popup.width, height: popup.height)
                        .position(x: popup.midX, y: popup.midY)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .clipped()
            #if os(tvOS)
            .onAppear {
                focusedControl = layout.restoredFocus(chrome.revealFocus, canSeek: scrubBounds.canSeek)
            }
            #endif
        }
        .background(Color.black.ignoresSafeArea())
        .preferredColorScheme(.dark)
        .onAppear {
            scrubPosition = scrubBounds.clamp(Double(playback.position))
            #if os(tvOS)
            televisionActive = true
            #endif
            #if os(macOS)
            focusedControl = "stage"
            #endif
        }
        .onChange(of: playback.position) { _, value in
            if !scrubbing { scrubPosition = scrubBounds.clamp(Double(value)) }
        }
        .onChange(of: playback.duration) { _, _ in
            scrubPosition = scrubBounds.clamp(scrubbing ? scrubPosition : Double(playback.position))
        }
        .onChange(of: focusedControl) { _, value in
            #if os(tvOS)
            traceFocus(.nativeBefore, value: value)
            let returning = menuReturn.nativeFocusChanged(value, interaction: chrome.interaction,
                visible: chrome.visible, menuOpen: chrome.menu != nil, closed: playback.closed, active: televisionActive)
            if !returning {
                if value == "stage" || value == nil { chrome.releaseTelevisionFocus() }
                chrome.rememberFocus(value)
            } else if value == menuReturn.issuedTarget { chrome.rememberFocus(value) }
            traceFocus(.nativeAfter, outcome: returning ? .preserved : .observed, value: value)
            #else
            if value == nil || value == "stage" { chrome.releaseTelevisionFocus() }
            chrome.rememberFocus(value)
            if value != nil && value != "stage" { chrome.reveal() }
            #endif
        }
        .onChange(of: accessibleControl) { _, value in
            #if os(tvOS)
            traceFocus(.accessibilityBefore, value: value)
            let selfAcknowledged = menuReturn.accessibilityChanged(value, interaction: chrome.interaction,
                visible: chrome.visible, menuOpen: chrome.menu != nil, closed: playback.closed, active: televisionActive)
            if !selfAcknowledged && value != nil && value != "stage" { chrome.reveal() }
            traceFocus(.accessibilityAfter, outcome: selfAcknowledged ? .preserved : .observed, value: value)
            #else
            if value != nil && value != "stage" { chrome.reveal() }
            #endif
        }
        .task(id: NativePlayerIdleToken(interaction: chrome.interaction, visibility: visibility)) {
            if visibility.holdsVisible { chrome.holdVisible(); return }
            guard chrome.visible else { return }
            let interaction = chrome.interaction
            guard await NativePlayerIdleTimer.wait(),
                  chrome.canExpire(interaction: interaction, visibility: visibility) else { return }
            #if os(macOS)
            focusedControl = "stage"
            await Task.yield()
            #endif
            guard !Task.isCancelled else { return }
            #if os(tvOS)
            if chrome.expire(interaction: interaction, visibility: visibility) {
                focusedControl = "stage"
            }
            #else
            chrome.expire(interaction: interaction, visibility: visibility)
            #endif
        }
        #if os(macOS)
        .onContinuousHover { phase in
            if case .active = phase { chrome.releaseTelevisionFocus(); interact(keyboard: false) }
        }
        .onKeyPress(phases: .down) { press in
            interact(keyboard: true)
            guard chrome.menu == nil else { return .ignored }
            switch press.key {
            case .space where focusedControl == nil || focusedControl == "stage" || focusedControl == "play":
                Task { await playback.toggle() }; return .handled
            case .leftArrow where focusedControl != "position":
                Task { await playback.seek(by: -30) }; return .handled
            case .rightArrow where focusedControl != "position":
                Task { await playback.seek(by: 30) }; return .handled
            case "f":
                guard !playback.busy && (!playback.closed || playback.presentation.fullscreen) else { return .handled }
                playback.presentation.toggle(); return .handled
            default: return .ignored
            }
        }
        #endif
        #if os(tvOS) || os(macOS)
        .onExitCommand {
            #if os(tvOS)
            traceFocus(.exitBefore)
            menuReturn.cancel()
            traceFocus(.exitAfter, outcome: .cancelled)
            #endif
            if dismissMenu() { return }
            #if os(macOS)
            if playback.presentation.fullscreen { playback.presentation.toggle(); return }
            #else
            if focusedControl != "stage" && !visibility.paused && !visibility.busy && !visibility.blocked && !visibility.closed && !visibility.notice {
                chrome.releaseTelevisionFocus()
                focusedControl = "stage"
                chrome.expire(interaction: chrome.interaction, visibility: NativePlayerVisibility())
                return
            }
            #endif
            Task { await playback.close(); exit() }
        }
        #endif
        #if os(tvOS)
        .onChange(of: chrome.visible) { _, visible in
            traceFocus(.hideBefore)
            if !visible { menuReturn.cancel() }
            traceFocus(.hideAfter, outcome: visible ? .observed : .cancelled)
        }
        .onChange(of: playback.closed) { _, _ in
            traceFocus(.closedBefore)
            menuReturn.cancel()
            traceFocus(.closedAfter, outcome: .cancelled)
        }
        .onDisappear {
            traceFocus(.disappearBefore)
            televisionActive = false
            menuReturn.cancel()
            traceFocus(.disappearAfter, outcome: .cancelled)
        }
        .background(NativePlayerTelevisionInputObserver(nativeItems: nativeItemObservation.trace.enabled ? nativeItemObservation : nil) { input in
            traceFocus(.inputBefore)
            menuReturn.cancel()
            chrome.televisionInput(input, focus: focusedControl)
            traceFocus(.inputAfter, outcome: .cancelled)
        })
        .onPlayPauseCommand {
            interact(keyboard: false)
            Task { await playback.handleRemotePlayPause() }
        }
        #endif
    }

    #if os(tvOS)
    private func traceFocus(_ event: NativePlayerFocusTrace.Event, outcome: NativePlayerFocusTrace.Outcome = .observed,
                            request: NativePlayerMenuReturn.Request? = nil, capturedGeneration: Int? = nil,
                            capturedInteraction: Int? = nil, anchor: String? = nil, value: String? = nil, target: String? = nil) {
        guard focusTrace.enabled else { return }
        nativeItemObservation.observe(event, generation: menuReturn.generation, interaction: chrome.interaction, anchor: anchor)
        let request = capturedGeneration != nil ? request : (request ?? menuReturn.pending)
        let eligibility: NativePlayerFocusTrace.Eligibility
        if !televisionActive { eligibility = .inactive }
        else if !chrome.visible { eligibility = .hidden }
        else if chrome.menu != nil { eligibility = .menuOpen }
        else if playback.closed { eligibility = .closed }
        else if let capturedGeneration, capturedGeneration != menuReturn.generation { eligibility = .capturedGeneration }
        else if let capturedInteraction, capturedInteraction != chrome.interaction { eligibility = .capturedInteraction }
        else if request == nil { eligibility = .missingRequest }
        else if request != menuReturn.pending || request?.generation != menuReturn.generation { eligibility = .staleRequest }
        else if request?.interaction != chrome.interaction { eligibility = .staleInteraction }
        else if accessibleControl != nil && accessibleControl != request?.originAccessibilityFocus &&
                    !(menuReturn.issued && accessibleControl == menuReturn.issuedTarget) { eligibility = .accessibilityMismatch }
        else if menuReturn.issued { eligibility = .alreadyIssued }
        else { eligibility = .eligible }
        focusTrace.record(event, outcome: outcome, state: .init(generation: menuReturn.generation,
            interaction: chrome.interaction, requestGeneration: request?.generation,
            pendingGeneration: menuReturn.pending?.generation, issuedGeneration: menuReturn.diagnosticIssuedGeneration,
            capturedGeneration: capturedGeneration, capturedInteraction: capturedInteraction,
            anchor: .init(anchor ?? request?.anchor ?? menuReturn.issuedTarget), focusState: .init(focusedControl),
            accessibilityState: .init(accessibleControl), callbackValue: .init(value), target: .init(target),
            pending: menuReturn.pending != nil, issued: menuReturn.issued, visible: chrome.visible,
            menuOpen: chrome.menu != nil, closed: playback.closed, active: televisionActive, eligibility: eligibility))
    }

    private func televisionToolbar(_ layout: NativePlayerStageLayout) -> some View {
        VStack(spacing: 0) {
            header(layout)
            Spacer(minLength: 0)
            footer(layout)
        }.transition(.opacity)
    }

    private func restoreClosedAccessibilityFocus(generation: Int, interaction: Int) {
        guard televisionActive, chrome.visible, chrome.menu == nil, playback.closed, voiceOver,
              generation == menuReturn.generation, interaction == chrome.interaction else { return }
        accessibleControl = "back"
    }

    private func preferredTelevisionFocus(_ layout: NativePlayerStageLayout) -> String? {
        if playback.closed && chrome.visible && chrome.menu == nil {
            traceFocus(.preferred, outcome: .accepted, target: "back")
            return "back"
        }
        let target = menuReturn.target(menuReturn.pending, layout: layout, interaction: chrome.interaction,
            visible: chrome.visible, menuOpen: chrome.menu != nil, closed: playback.closed,
            active: televisionActive, focus: focusedControl, accessibilityFocus: accessibleControl)
        traceFocus(.preferred, outcome: target == nil ? .rejected : .accepted, target: target)
        return target
    }

    private func defaultTelevisionFocus(_ layout: NativePlayerStageLayout) -> String? {
        let target: String?
        if !televisionActive || !chrome.visible || chrome.menu != nil {
            target = nil
        } else if playback.closed {
            target = "back"
        } else {
            target = menuReturn.registeredTarget(layout: layout, interaction: chrome.interaction,
                visible: chrome.visible, menuOpen: false, closed: false, active: televisionActive,
                accessibilityFocus: accessibleControl) ?? preferredTelevisionFocus(layout) ??
                layout.restoredFocus(focusedControl ?? chrome.revealFocus)
        }
        traceFocus(.defaultPreference, outcome: target == nil ? .rejected : .accepted, target: target)
        return target
    }

    private func restoreToolbarFocus(_ request: NativePlayerMenuReturn.Request?, generation: Int,
                                     interaction: Int, layout: NativePlayerStageLayout) {
        traceFocus(.preferenceEligible, request: request, capturedGeneration: generation, capturedInteraction: interaction)
        guard televisionActive, chrome.visible, chrome.menu == nil,
              generation == menuReturn.generation, interaction == chrome.interaction else { return }
        traceFocus(.claimBefore, request: request, capturedGeneration: generation, capturedInteraction: interaction)
        guard let target = menuReturn.claim(request, layout: layout, interaction: chrome.interaction,
            visible: chrome.visible, menuOpen: false, closed: playback.closed, active: televisionActive,
            focus: focusedControl, accessibilityFocus: accessibleControl) else {
            traceFocus(.claimAfter, outcome: .rejected, request: request, capturedGeneration: generation, capturedInteraction: interaction)
            return
        }
        traceFocus(.claimAfter, outcome: .accepted, request: request, capturedGeneration: generation, capturedInteraction: interaction, target: target)
        if voiceOver { accessibleControl = target }
    }
    #endif

    private var television: Bool {
        #if os(tvOS)
        true
        #else
        false
        #endif
    }

    private func stageInput(_ layout: NativePlayerStageLayout) -> some View {
        Color.clear.contentShape(Rectangle())
            .onTapGesture {
                #if os(tvOS)
                chrome.revealFromTelevisionStage(navigation: false)
                focusedControl = layout.restoredFocus(chrome.revealFocus, canSeek: scrubBounds.canSeek)
                #else
                chrome.releaseTelevisionFocus()
                interact(keyboard: false)
                #endif
            }
            .accessibilityLabel("Show playback controls")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction {
                chrome.reveal()
                let target = layout.restoredFocus(chrome.revealFocus, canSeek: scrubBounds.canSeek)
                focusedControl = target
                accessibleControl = target
            }
            .accessibilityFocused($accessibleControl, equals: "stage")
            #if os(tvOS)
            .focusable(chrome.televisionStageFocusable)
            #elseif os(macOS)
            .focusable()
            #endif
            #if os(macOS) || os(tvOS)
            .focusEffectDisabled()
            .focused($focusedControl, equals: "stage")
            #endif
            #if os(tvOS)
            .onMoveCommand { _ in
                chrome.revealFromTelevisionStage(navigation: true)
                focusedControl = layout.restoredFocus(chrome.revealFocus, canSeek: scrubBounds.canSeek)
            }
            #endif
    }

    private func header(_ layout: NativePlayerStageLayout) -> some View {
        HStack(spacing: 12) {
            icon("Back to details", symbol: "chevron.left", id: "back", layout: layout) {
                Task { await playback.close(); exit() }
            }
            Text(title).font(television ? .title3 : .headline).lineLimit(1).truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityFocused($accessibleControl, equals: "title")
        }
        .padding(layout.inset).padding(.bottom, layout.compactHeight ? 0 : 24)
        .background(LinearGradient(colors: [.black.opacity(0.88), .black.opacity(0.55), .clear], startPoint: .top, endPoint: .bottom))
        #if os(tvOS)
        .focusSection()
        #endif
    }

    private func footer(_ layout: NativePlayerStageLayout) -> some View {
        VStack(spacing: television ? (layout.minimalHeight ? 4 : 12) : 6) {
            #if !os(tvOS)
            Slider(value: Binding(get: { scrubBounds.clamp(scrubPosition) }, set: {
                playback.recordSliderReceived($0)
                scrubPosition = scrubBounds.clamp($0)
            }),
                   in: scrubBounds.range, step: scrubBounds.step) { editing in
                interact(keyboard: keyboardInput)
                scrubbing = editing
                if !editing && scrubBounds.canSeek {
                    let target = scrubBounds.milliseconds(scrubPosition)
                    scrubPosition = Double(target)
                    let diagnostic = playback.recordSliderEnded(target: target)
                    Task { await playback.seek(to: target, diagnostic: diagnostic) }
                }
            }
            .tint(.white).frame(minHeight: 44)
            .focused($focusedControl, equals: "position")
            .accessibilityFocused($accessibleControl, equals: "position")
            .accessibilityLabel("Playback position")
            .accessibilityValue(time(scrubBounds.milliseconds(scrubPosition)))
            .accessibilityIdentifier("playback.position")
            .disabled(unavailable || !scrubBounds.canSeek)
            #else
            ProgressView(value: Double(min(max(0, playback.position), max(1, playback.duration))), total: Double(max(1, playback.duration)))
                .tint(.white).frame(height: 4).accessibilityLabel("Playback position")
                .accessibilityValue(time(playback.position))
                .accessibilityFocused($accessibleControl, equals: "position")
            #endif
            if !layout.minimalHeight {
                HStack {
                    Text(time(scrubbing ? scrubBounds.milliseconds(scrubPosition) : playback.position))
                    Spacer()
                    Text(playback.duration > 0 ? time(playback.duration) : "—")
                }.font(television ? .callout.monospacedDigit() : .caption.monospacedDigit()).foregroundStyle(.white.opacity(0.8))
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("playback.time")
                    .accessibilityFocused($accessibleControl, equals: "time")
            }
            HStack(spacing: layout.gap) {
                ForEach(layout.controls, id: \.self) { control in toolbarControl(control, layout: layout) }
            }.frame(maxWidth: .infinity)
        }
        .padding(.horizontal, layout.inset)
        .padding(.vertical, television && layout.minimalHeight ? 0 : layout.inset)
        .padding(.top, layout.compactHeight ? 0 : 20)
        .background(LinearGradient(colors: [.clear, .black.opacity(0.6), .black.opacity(0.92)], startPoint: .top, endPoint: .bottom))
        #if os(tvOS)
        .focusSection()
        #endif
    }

    @ViewBuilder private func toolbarControl(_ control: NativePlayerControl, layout: NativePlayerStageLayout) -> some View {
        switch control {
        case .play:
            icon(playback.paused ? "Play" : "Pause", symbol: playback.paused ? "play.fill" : "pause.fill", id: control.rawValue, layout: layout) {
                Task { await playback.toggle() }
            }.disabled(unavailable)
        case .backward:
            icon("Back 30 seconds", symbol: "gobackward.30", id: control.rawValue, layout: layout) {
                Task { await playback.seek(by: -30) }
            }.disabled(unavailable)
        case .forward:
            icon("Forward 30 seconds", symbol: "goforward.30", id: control.rawValue, layout: layout) {
                Task { await playback.seek(by: 30) }
            }.disabled(unavailable)
        case .mute:
            icon(playback.muted ? "Unmute" : "Mute", symbol: playback.muted ? "speaker.slash" : "speaker.wave.2", id: control.rawValue, layout: layout) {
                playback.toggleMute()
            }.disabled(unavailable)
        case .volume:
            icon("Volume", symbol: "speaker.wave.2.circle", id: control.rawValue, layout: layout, selected: chrome.menu == .volume) { openMenu(.volume) }
        case .tracks:
            icon("Audio and subtitles", symbol: "captions.bubble", id: control.rawValue, layout: layout, selected: chrome.menu == .tracks) { openMenu(.tracks) }
        case .session:
            icon("Playback settings and session", symbol: "gearshape", id: control.rawValue, layout: layout, selected: chrome.menu == .session) { openMenu(.session) }
        case .fullscreen:
            icon(playback.presentation.fullscreen ? "Exit fullscreen" : "Fullscreen",
                 symbol: playback.presentation.fullscreen ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right",
                 id: control.rawValue, layout: layout) { playback.presentation.toggle() }
                .disabled(playback.busy || (playback.closed && !playback.presentation.fullscreen))
        }
    }

    private func icon(_ label: String, symbol: String, id: String, layout: NativePlayerStageLayout, selected: Bool = false,
                      action: @escaping () -> Void) -> some View {
        Button {
            interact(keyboard: keyboardInput)
            action()
        } label: {
            Image(systemName: symbol).font(.system(size: television ? 27 : 19, weight: .regular))
                .frame(width: layout.target, height: layout.target).contentShape(Rectangle())
        }
        .buttonStyle(NativePlayerButtonStyle(focused: focusedControl == id, selected: selected))
        .focused($focusedControl, equals: id)
        .accessibilityFocused($accessibleControl, equals: id)
        .accessibilityLabel(label)
        .accessibilityValue(id == "session" ? playback.diagnosticValue ?? (selected ? "Selected" : "") : (selected ? "Selected" : ""))
        .accessibilityIdentifier("playback.\(id)")
        #if os(tvOS)
        .prefersDefaultFocus(preferredTelevisionFocus(layout) == id, in: toolbarFocusScope)
        #endif
        #if os(macOS)
        .help(label)
        #endif
    }

    private func floatingMenu(_ menu: NativePlayerMenu, layout: NativePlayerStageLayout) -> some View {
        VStack(spacing: 0) {
            HStack {
                Text(menu == .volume ? "Volume" : menu == .tracks ? "Audio and subtitles" : "Playback settings")
                    .font(television ? .headline : .subheadline.weight(.semibold)).lineLimit(1)
                Spacer(minLength: 4)
                icon("Dismiss playback menu", symbol: "xmark", id: "menu.close", layout: layout) { dismissMenu() }
            }.padding(.horizontal, 8).padding(.top, 4)
            Divider().overlay(.white.opacity(0.2))
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    if menu == .volume {
                        Text("Volume \(Int((playback.volume * 100).rounded()))%").font(.callout).padding(8)
                            .accessibilityIdentifier("playback.volume.readout")
                        menuRow("Quieter", symbol: "minus", id: "volume.down", layout: layout) { playback.adjustVolume(by: -0.1) }.disabled(playback.busy)
                        menuRow("Louder", symbol: "plus", id: "volume.up", layout: layout) { playback.adjustVolume(by: 0.1) }.disabled(playback.busy)
                        menuRow(playback.muted ? "Unmute" : "Mute", symbol: playback.muted ? "speaker.wave.2" : "speaker.slash",
                                id: "volume.mute", layout: layout) { playback.toggleMute() }.disabled(unavailable)
                    } else if menu == .tracks {
                        tracksMenu(layout)
                    } else {
                        sessionMenu(layout)
                    }
                }.padding(8)
            }
        }
        .foregroundStyle(.white)
        .background(.black.opacity(0.94), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.white.opacity(0.25), lineWidth: 1))
        .accessibilityIdentifier("playback.menu.\(menu.rawValue)")
        #if os(tvOS)
        .focusSection()
        #endif
    }

    private func menuRow(_ label: String, symbol: String, id: String, layout: NativePlayerStageLayout,
                          selected: Bool = false, action: @escaping () -> Void) -> some View {
        Button {
            if id.hasPrefix("volume.") { playback.recordDiagnostic("menu.action.\(id)") }
            interact(keyboard: keyboardInput)
            action()
        } label: {
            HStack(spacing: 12) {
                Image(systemName: symbol).font(.system(size: television ? 24 : 18)).frame(width: 28)
                Text(label).font(television ? .callout : .body).multilineTextAlignment(.leading).lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if selected { Image(systemName: "checkmark") }
            }.padding(.horizontal, 12).padding(.vertical, 8).frame(minHeight: layout.target)
                .frame(maxWidth: .infinity).contentShape(Rectangle())
        }
        .buttonStyle(NativePlayerButtonStyle(focused: focusedControl == id, selected: selected))
        .focused($focusedControl, equals: id)
        .accessibilityFocused($accessibleControl, equals: id)
        .accessibilityLabel(label).accessibilityValue(selected ? "Selected" : "")
        .background {
            if id.hasPrefix("volume."), ProcessInfo.processInfo.environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1" {
                NativePlaybackControlDiagnostics(playback: playback, control: id)
            }
        }
    }

    @ViewBuilder private func tracksMenu(_ layout: NativePlayerStageLayout) -> some View {
        if playback.session?.tracks.isEmpty != false {
            Text("No track choices are available.").foregroundStyle(.white.opacity(0.7)).padding(8)
        }
        menuRow("Subtitles Off", symbol: "captions.bubble", id: "tracks.off", layout: layout,
                selected: playback.session?.tracks.contains(where: { $0.kind == .subtitle && $0.selected }) == false) {
            Task { await playback.choose(nil) }
        }.disabled(unavailable || playback.session == nil)
        ForEach(playback.session?.tracks ?? [], id: \.id) { track in
            let available = track.available && track.subtitleMode != .unavailable
            menuRow("\(track.kind == .audio ? "Audio" : "Subtitle"): \(track.displayName)\(available ? "" : " — unavailable")",
                    symbol: track.kind == .audio ? "waveform" : "captions.bubble", id: "track.\(track.id)", layout: layout, selected: track.selected) {
                Task { await playback.choose(track) }
            }.disabled(unavailable || !available)
        }
    }

    @ViewBuilder private func sessionMenu(_ layout: NativePlayerStageLayout) -> some View {
        if !layout.controls.contains(.play) {
            menuRow(playback.paused ? "Play" : "Pause", symbol: playback.paused ? "play.fill" : "pause.fill", id: "session.play", layout: layout) {
                Task { await playback.toggle() }
            }.disabled(unavailable)
        }
        ForEach(NativePlayerStageLayout.sessionMenus, id: \.self) { menu in
            menuRow(menu == .tracks ? "Audio and subtitles" : "Volume",
                    symbol: menu == .tracks ? "captions.bubble" : "speaker.wave.2", id: "session.\(menu.rawValue)", layout: layout) {
                openMenu(menu, anchor: "session")
            }
        }
        if !layout.controls.contains(.backward) {
            menuRow("Back 30 seconds", symbol: "gobackward.30", id: "session.backward", layout: layout) {
                Task { await playback.seek(by: -30) }
            }.disabled(unavailable)
            menuRow("Forward 30 seconds", symbol: "goforward.30", id: "session.forward", layout: layout) {
                Task { await playback.seek(by: 30) }
            }.disabled(unavailable)
        }
        #if !os(tvOS)
        if !layout.controls.contains(.fullscreen) {
            menuRow(playback.presentation.fullscreen ? "Exit fullscreen" : "Fullscreen",
                    symbol: "arrow.up.left.and.arrow.down.right", id: "session.fullscreen", layout: layout) { playback.presentation.toggle() }
                .disabled(playback.busy || (playback.closed && !playback.presentation.fullscreen))
        }
        #endif
        if playback.session == nil && !playback.closed {
            menuRow("Retry identical preparation", symbol: "arrow.clockwise", id: "session.create", layout: layout) {
                Task { await playback.start() }
            }.disabled(playback.busy)
        }
        if playback.session != nil {
            menuRow("Check session state", symbol: "checkmark.shield", id: "session.check", layout: layout) {
                Task { await playback.check() }
            }.disabled(playback.busy)
            if playback.canRetryTransport {
                menuRow("Retry identical transport request", symbol: "arrow.clockwise", id: "session.retry", layout: layout) {
                    Task { await playback.retryTransport() }
                }.disabled(playback.busy)
            }
            if playback.canFallback {
                menuRow("Request compatible H.264/AAC transport", symbol: "arrow.triangle.2.circlepath", id: "session.fallback", layout: layout) {
                    Task { await playback.compatibleTransport() }
                }.disabled(playback.busy)
            }
            menuRow(playback.closed ? "Retry identical Stop" : "Stop playback", symbol: "stop.fill", id: "session.stop", layout: layout) {
                Task {
                    if playback.closed { await playback.retryStop() }
                    else { await playback.close() }
                }
            }.disabled(playback.busy)
        }
        if !playback.message.isEmpty { Text(playback.message).font(.caption).foregroundStyle(.white.opacity(0.8)).padding(8) }
        if !playback.subtitleWarning.isEmpty { Text(playback.subtitleWarning).font(.caption).foregroundStyle(.white.opacity(0.8)).padding(8) }
    }

    private func statusOverlay(width: CGFloat) -> some View {
        VStack(spacing: 12) {
            if playback.busy { ProgressView().tint(.white).accessibilityLabel("Contacting playback gateway") }
            if !playback.message.isEmpty && hasNotice {
                Text(playback.message).lineLimit(3).accessibilityIdentifier("playback.status")
            }
            if !playback.subtitleWarning.isEmpty { Text(playback.subtitleWarning).lineLimit(2) }
            if playback.blocked || playback.closed { Text("Session options are under Playback settings.").font(.caption) }
        }
        .font(television ? .callout : .body).multilineTextAlignment(.center)
        .padding(16).frame(maxWidth: max(0, min(560, width - 32)))
        .foregroundStyle(.white).background(.black.opacity(0.8), in: RoundedRectangle(cornerRadius: 12))
        .allowsHitTesting(false)
    }

    private func interact(keyboard: Bool) {
        #if os(tvOS)
        traceFocus(.interactionBefore)
        menuReturn.cancel()
        traceFocus(.interactionAfter, outcome: .cancelled)
        #endif
        keyboardInput = keyboard
        chrome.reveal()
    }

    private func openMenu(_ menu: NativePlayerMenu, anchor: String? = nil) {
        #if os(tvOS)
        traceFocus(.menuBefore, anchor: anchor ?? menu.rawValue)
        menuReturn.cancel()
        #endif
        chrome.open(menu, anchor: anchor ?? menu.rawValue)
        focusedControl = "menu.close"
        if voiceOver { accessibleControl = "menu.close" }
        #if os(tvOS)
        traceFocus(.menuAfter, outcome: .cancelled, anchor: anchor ?? menu.rawValue)
        #endif
    }

    @discardableResult private func dismissMenu() -> Bool {
        #if os(tvOS)
        traceFocus(.dismissBefore, anchor: chrome.menuAnchor)
        #endif
        guard let anchor = chrome.dismissMenu() else {
            #if os(tvOS)
            traceFocus(.dismissAfter, outcome: .rejected)
            #endif
            return false
        }
        #if os(tvOS)
        traceFocus(.dismissAfter, outcome: .accepted, anchor: anchor)
        if playback.closed {
            traceFocus(.closedBefore)
            menuReturn.cancel()
            traceFocus(.closedAfter, outcome: .cancelled)
        }
        else {
            traceFocus(.scheduleBefore, anchor: anchor)
            menuReturn.schedule(anchor: anchor, interaction: chrome.interaction,
                                focus: focusedControl, accessibilityFocus: accessibleControl)
            traceFocus(.scheduleAfter, anchor: anchor)
        }
        #else
        focusedControl = anchor
        if voiceOver { accessibleControl = anchor }
        #endif
        return true
    }

    private func time(_ ms: Int64) -> String {
        let seconds = ms / 1000
        return "\(seconds / 60):\(String(format: "%02lld", seconds % 60))"
    }

}

private struct NativePlaybackControlDiagnostics: View {
    let playback: NativePlayback
    let control: String
    @Environment(\.isEnabled) private var enabled
    @Environment(\.scenePhase) private var phase

    var body: some View {
        Color.clear.frame(width: 0, height: 0).accessibilityHidden(true).allowsHitTesting(false)
            .onAppear { record("appear") }
            .onChange(of: enabled) { _, _ in record("enabled") }
            .onChange(of: phase) { _, _ in record("scene") }
    }

    private func record(_ event: String) {
        playback.recordDiagnostic("control.\(control).\(event)", enabled: enabled, phase: phase)
    }
}

#if os(tvOS)
@MainActor
private final class NativePlayerNativeItemObservation: NSObject {
    let trace: NativePlayerNativeItemTrace
    private nonisolated let enabled: Bool
    private var generation = 0
    private var interaction = 0
    private var installed = false
    private var ownedWindow: UIWindow? { trace.windowObject as? UIWindow }

    override init() {
        let trace = NativePlayerNativeItemTrace()
        self.trace = trace
        enabled = trace.enabled
        super.init()
    }
    deinit {
        if enabled { NotificationCenter.default.removeObserver(self) }
    }
    func attach(to window: UIWindow?) {
        guard trace.enabled, ownedWindow !== window else { return }
        detach()
        guard let window else { return }
        trace.bind(window)
        NotificationCenter.default.addObserver(self, selector: #selector(updated(_:)),
            name: UIFocusSystem.didUpdateNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(failed(_:)),
            name: UIFocusSystem.movementDidFailNotification, object: nil)
        installed = true
        sample(.attached)
    }
    func detach() {
        guard trace.enabled else { return }
        sample(.detached)
        if installed {
            NotificationCenter.default.removeObserver(self)
            installed = false
        }
        trace.unbind()
    }
    func observe(_ event: NativePlayerFocusTrace.Event, generation: Int, interaction: Int, anchor: String?) {
        guard trace.enabled else { return }
        self.generation = generation
        self.interaction = interaction
        if event == .menuBefore, anchor == "volume", !trace.armed, let window = ownedWindow {
            let item = UIFocusSystem.focusSystem(for: window)?.focusedItem
            if trace.arm(anchor: item) { sample(.arm) }
        } else if event == .nativeAfter || event == .dismissAfter {
            sample(.localSample)
        }
    }
    @objc private nonisolated func updated(_ notification: Notification) { deliver(notification, event: .updated) }
    @objc private nonisolated func failed(_ notification: Notification) { deliver(notification, event: .failed) }
    private nonisolated func deliver(_ notification: Notification, event: NativePlayerNativeItemTrace.Event) {
        guard Thread.isMainThread else { return }
        let context = notification.userInfo?[UIFocusSystem.focusUpdateContextUserInfoKey] as? UIFocusUpdateContext
        let notifier = notification.object as? UIFocusSystem
        MainActor.assumeIsolated { receive(context: context, notifier: notifier, event: event) }
    }
    private func receive(context: UIFocusUpdateContext?, notifier: UIFocusSystem?, event: NativePlayerNativeItemTrace.Event) {
        guard Thread.isMainThread, trace.canRecord, let window = ownedWindow else { return }
        let system = UIFocusSystem.focusSystem(for: window)
        let previous = context?.previouslyFocusedItem
        let next = context?.nextFocusedItem
        let current = system?.focusedItem
        let matches = system.flatMap { owned in notifier.map { $0 === owned } }
        guard NativePlayerNativeItemTrace.acceptsNotification(systemMatches: matches,
            previous: relation(previous, window: window), next: relation(next, window: window),
            current: relation(current, window: window)) else {
            trace.ignored()
            return
        }
        trace.record(event, generation: generation, interaction: interaction) {
            self.snapshot(window: window, system: system, previous: previous, next: next,
                          current: current, context: context != nil)
        }
    }
    private func sample(_ event: NativePlayerNativeItemTrace.Event) {
        guard trace.canRecord, let window = ownedWindow else { return }
        trace.record(event, generation: generation, interaction: interaction) {
            let system = UIFocusSystem.focusSystem(for: window)
            return self.snapshot(window: window, system: system, previous: nil, next: nil,
                                 current: system?.focusedItem, context: false)
        }
    }
    private func snapshot(window: UIWindow, system: UIFocusSystem?, previous: (any UIFocusItem)?,
                          next: (any UIFocusItem)?, current: (any UIFocusItem)?, context: Bool) -> NativePlayerNativeItemTrace.Snapshot? {
        guard let ordinal = trace.ordinal(window) else { return nil }
        return .init(window: ordinal, system: trace.ordinal(system), keyWindow: window.isKeyWindow,
            viewport: .init(window.bounds), context: context,
            previous: item(previous, window: window), next: item(next, window: window),
            current: item(current, window: window),
            anchor: item(trace.anchorObject as? any UIFocusItem, window: window))
    }
    private func relation(_ item: (any UIFocusItem)?, window: UIWindow) -> NativePlayerNativeItemTrace.Relation {
        guard let item else { return .missing }
        var environment: (any UIFocusEnvironment)? = item
        var visited: Set<ObjectIdentifier> = []
        for _ in 0..<32 {
            guard let current = environment, visited.insert(ObjectIdentifier(current)).inserted else { return .unknown }
            if let candidate = current as? UIWindow { return candidate === window ? .owned : .other }
            if let candidate = (current as? UIView)?.window { return candidate === window ? .owned : .other }
            environment = current.parentFocusEnvironment
        }
        return .unknown
    }
    private func item(_ item: (any UIFocusItem)?, window: UIWindow) -> NativePlayerNativeItemTrace.Item {
        guard let item else { return .missing }
        let owner = relation(item, window: window)
        if owner == .other {
            return .init(ordinal: nil, kind: .other, control: .other, relation: .other,
                         canBecomeFocused: nil, frameSpace: .unowned, frame: nil)
        }
        let identifier = owner == .owned ? (item as? any UIAccessibilityIdentification)?.accessibilityIdentifier : nil
        let control = NativePlayerNativeItemTrace.control(identifier)
        var frame: NativePlayerNativeItemTrace.Frame?
        var space: NativePlayerNativeItemTrace.FrameSpace = .unowned
        if owner == .owned {
            if let container = item.focusItemContainer {
                frame = .init(window.convert(item.frame, from: container.coordinateSpace))
                space = frame == nil ? .invalid : .window
            } else { space = .missingContainer }
        }
        return .init(ordinal: trace.ordinal(item), kind: item is UIView ? .view : .other,
            control: owner == .owned ? control : .other, relation: owner,
            canBecomeFocused: item.canBecomeFocused, frameSpace: space, frame: frame)
    }
}

private struct NativePlayerTelevisionInputObserver: UIViewRepresentable {
    let nativeItems: NativePlayerNativeItemObservation?
    let input: (NativePlayerTelevisionInput) -> Void
    func makeUIView(context: Context) -> NativePlayerTelevisionInputView { NativePlayerTelevisionInputView(nativeItems: nativeItems, input: input) }
    func updateUIView(_ view: NativePlayerTelevisionInputView, context: Context) {
        view.input = input
        view.observeNativeItems(nativeItems)
    }
    static func dismantleUIView(_ view: NativePlayerTelevisionInputView, coordinator: ()) { view.detach() }
}

private final class NativePlayerTelevisionInputView: UIView, UIGestureRecognizerDelegate {
    private weak var nativeItems: NativePlayerNativeItemObservation?
    var input: (NativePlayerTelevisionInput) -> Void
    private weak var observedWindow: UIWindow?
    private lazy var observer: UIGestureRecognizer = {
        let gesture = UIGestureRecognizer(target: nil, action: nil)
        gesture.delegate = self
        gesture.allowedPressTypes = [UIPress.PressType.upArrow, .downArrow, .leftArrow, .rightArrow, .select, .menu, .playPause, .pageUp, .pageDown]
            .map { NSNumber(value: $0.rawValue) }
        gesture.cancelsTouchesInView = false
        gesture.delaysTouchesBegan = false
        gesture.delaysTouchesEnded = false
        return gesture
    }()
    init(nativeItems: NativePlayerNativeItemObservation?, input: @escaping (NativePlayerTelevisionInput) -> Void) {
        self.nativeItems = nativeItems
        self.input = input
        super.init(frame: .zero)
        isUserInteractionEnabled = false
    }
    required init?(coder: NSCoder) { nil }
    func observeNativeItems(_ value: NativePlayerNativeItemObservation?) {
        guard nativeItems !== value else { return }
        nativeItems?.detach()
        nativeItems = value
        nativeItems?.attach(to: observedWindow)
    }
    override func didMoveToWindow() {
        super.didMoveToWindow()
        detach()
        window?.addGestureRecognizer(observer)
        observedWindow = window
        nativeItems?.attach(to: window)
    }
    func detach() {
        observedWindow?.removeGestureRecognizer(observer)
        nativeItems?.detach()
        observedWindow = nil
    }
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive event: UIEvent) -> Bool {
        if let event = event as? UIPressesEvent {
            for press in event.allPresses where press.phase == .began {
                if press.key != nil { input(.keyboard) }
                else {
                    switch press.type {
                    case .upArrow, .downArrow, .leftArrow, .rightArrow: input(.navigation)
                    case .select: input(.select)
                    default: break
                    }
                }
            }
        }
        // Observe human input without recognizing, delaying, or consuming it.
        return false
    }
}
#endif

#if os(iOS)
private struct PlaybackFullscreenPresenter: UIViewControllerRepresentable {
    let playback: NativePlayback
    let title: String
    let exit: () -> Void
    func makeUIViewController(context: Context) -> PlaybackFullscreenHost {
        PlaybackFullscreenHost(playback: playback, title: title, exit: exit)
    }
    func updateUIViewController(_ controller: PlaybackFullscreenHost, context: Context) { controller.synchronize() }
    static func dismantleUIViewController(_ controller: PlaybackFullscreenHost, coordinator: ()) {
        controller.tearDown()
        Task { await controller.playback.close(reason: "unmount") }
    }
}

private final class PlaybackFullscreenHost: UIViewController {
    let playback: NativePlayback
    let titleText: String
    let exit: () -> Void
    private var transitioning = false
    private var removed = false
    init(playback: NativePlayback, title: String, exit: @escaping () -> Void) {
        self.playback = playback
        titleText = title
        self.exit = exit
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { nil }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        synchronize()
    }
    func synchronize() {
        guard !removed, !transitioning, viewIfLoaded?.window != nil else { return }
        if playback.presentation.fullscreen && presentedViewController == nil && !playback.closed {
            let content = NativePlaybackContent(playback: playback, title: titleText, exit: exit, renderMedia: true)
                .statusBarHidden(true).persistentSystemOverlays(.hidden)
            let controller = UIHostingController(rootView: content)
            controller.modalPresentationStyle = .overFullScreen
            controller.modalPresentationCapturesStatusBarAppearance = true
            controller.isModalInPresentation = true
            transitioning = true
            present(controller, animated: true) { [weak self] in
                self?.transitioning = false
                self?.synchronize()
            }
        } else if !playback.presentation.fullscreen && presentedViewController != nil {
            transitioning = true
            dismiss(animated: true) { [weak self] in
                self?.transitioning = false
                self?.synchronize()
            }
        }
    }
    func tearDown() {
        removed = true
        dismiss(animated: false)
    }
}
#endif

@MainActor
private final class NativeVideoObservation {
    let firstFrame: @MainActor () -> Void
    var observation: NSKeyValueObservation?
    init(_ firstFrame: @escaping @MainActor () -> Void) { self.firstFrame = firstFrame }
}

#if os(macOS)
private struct PlaybackWindowAttachment: NSViewRepresentable {
    let presentation: NativePlaybackPresentation
    func makeNSView(context: Context) -> PlaybackWindowObserver { PlaybackWindowObserver(presentation: presentation) }
    func updateNSView(_ view: PlaybackWindowObserver, context: Context) {}
    static func dismantleNSView(_ view: PlaybackWindowObserver, coordinator: ()) { view.detach() }
}

private final class PlaybackWindowObserver: NSView {
    let presentation: NativePlaybackPresentation
    private var observers: [NSObjectProtocol] = []
    init(presentation: NativePlaybackPresentation) {
        self.presentation = presentation
        super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { nil }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        detach()
        guard let window else { return }
        presentation.window = window
        presentation.setFullscreen(window.styleMask.contains(.fullScreen))
        for name in [NSWindow.didEnterFullScreenNotification, NSWindow.didExitFullScreenNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self, let window = self.window else { return }
                    self.presentation.setFullscreen(window.styleMask.contains(.fullScreen))
                }
            })
        }
    }
    func detach() {
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers = []
        presentation.window = nil
    }
}

private struct NativeVideoSurface: NSViewRepresentable {
    let player: AVPlayer
    let firstFrame: @MainActor () -> Void
    func makeCoordinator() -> NativeVideoObservation { NativeVideoObservation(firstFrame) }
    func makeNSView(context: Context) -> AVPlayerView {
        let view = AVPlayerView()
        view.controlsStyle = .none
        view.videoGravity = .resizeAspect
        view.allowsPictureInPicturePlayback = false
        view.player = player
        let coordinator = context.coordinator
        coordinator.observation = view.observe(\.isReadyForDisplay, options: [.initial, .new]) { [weak coordinator] _, change in
            guard change.newValue == true else { return }
            Task { @MainActor [weak coordinator] in coordinator?.firstFrame() }
        }
        return view
    }
    func updateNSView(_ view: AVPlayerView, context: Context) { view.player = player }
    static func dismantleNSView(_ view: AVPlayerView, coordinator: NativeVideoObservation) {
        coordinator.observation = nil
        view.player = nil
    }
}
#else
private struct NativeVideoSurface: UIViewControllerRepresentable {
    let player: AVPlayer
    let firstFrame: @MainActor () -> Void
    func makeCoordinator() -> NativeVideoObservation { NativeVideoObservation(firstFrame) }
    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        controller.showsPlaybackControls = false
        controller.videoGravity = .resizeAspect
        controller.allowsPictureInPicturePlayback = false
        controller.player = player
        let coordinator = context.coordinator
        coordinator.observation = controller.observe(\.isReadyForDisplay, options: [.initial, .new]) { [weak coordinator] _, change in
            guard change.newValue == true else { return }
            Task { @MainActor [weak coordinator] in coordinator?.firstFrame() }
        }
        return controller
    }
    func updateUIViewController(_ controller: AVPlayerViewController, context: Context) { controller.player = player }
    static func dismantleUIViewController(_ controller: AVPlayerViewController, coordinator: NativeVideoObservation) {
        coordinator.observation = nil
        controller.player = nil
    }
}
#endif
