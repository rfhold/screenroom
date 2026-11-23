import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const apple = join(dirname(fileURLToPath(import.meta.url)), '..');
const source = readFileSync(join(apple, 'E2ETests/PlayerScreenshots.swift'), 'utf8');
const start = source.indexOf('enum PlaybackCapturePolicy {');
const end = source.indexOf('\nfinal class PlayerScreenshots:');
assert.ok(start >= 0 && end > start);
const nativeSource = readFileSync(join(apple, 'Sources/Home/NativePlayback.swift'), 'utf8');
assert.equal((nativeSource.match(/accessibilityIdentifier\("playback\.volume\.readout"\)/g) ?? []).length, 1);
assert.equal((nativeSource.match(/accessibilityIdentifier\("playback\.time"\)/g) ?? []).length, 1);
assert.ok(nativeSource.includes('.accessibilityElement(children: .combine)\n                    .accessibilityIdentifier("playback.time")\n                    .accessibilityFocused($accessibleControl, equals: "time")'));
const clockObserver = source.slice(source.indexOf('@MainActor private func absoluteClock('), source.indexOf('@MainActor private func absoluteSeekSnapshot('));
assert.ok(clockObserver.includes('app.descendants(matching: .any).matching(identifier: PlaybackCapturePolicy.clockIdentifier).allElementsBoundByIndex'));
assert.ok(clockObserver.includes('PlaybackCapturePolicy.identifiedClockPosition('));
assert.ok(clockObserver.includes('guard clocks.count == 1 else { return (clocks.count, []) }'));
assert.ok(clockObserver.includes('role: $0.elementType.rawValue, label: $0.label, value: $0.value'));
for (const unsafe of ['.staticTexts', '.sliders', '.firstMatch', 'boundBy:', 'String(describing:', 'String(reflecting:', 'toNormalizedSliderPosition:', 'activate(', 'openMenu(', 'reveal(']) {
    assert.ok(!clockObserver.includes(unsafe), `Clock observation must not use ${unsafe}`);
}
assert.ok(!source.includes('matching(NSPredicate(format: "label MATCHES %@", PlaybackCapturePolicy.timelinePairPattern))'),
    'Every combined-clock observation uses the same exact identifier, not a role-specific text search');
assert.ok(source.includes('app.descendants(matching: .any).matching(identifier: "playback.volume.readout")'));
assert.ok(source.includes('func testNosferatuVolumeControls() throws {\n        try runJourney(volumeOnly: true)'));
const journey = source.slice(source.indexOf('    @MainActor\n    private func runJourney('), source.indexOf('    @MainActor private func absoluteClock('));
const statusObserver = source.slice(source.indexOf('    @MainActor private func stopFields('), source.indexOf('    @MainActor private func wait('));
assert.ok(statusObserver.includes('app.descendants(matching: .any).matching(identifier: PlaybackCapturePolicy.stopIdentifier).allElementsBoundByIndex'));
assert.ok(statusObserver.includes('guard matches.count == 1 else { return (matches.count, []) }'));
assert.ok(statusObserver.includes('role: $0.elementType.rawValue, label: $0.label, value: $0.value'));
assert.ok(statusObserver.includes('let snapshot = self.stopFields(in: app)'));
assert.ok(statusObserver.includes('let snapshot = stopFields(in: app)'));
assert.ok(statusObserver.includes('guard ready, snapshot.count == 1, !progress else { return nil }'));
for (const unsafe of ['.staticTexts', '.firstMatch', 'boundBy:', 'String(describing:', 'String(reflecting:', 'activate(', 'openMenu(', 'closeMenu(', 'reveal(', 'playback.closed', 'Task {']) {
    assert.ok(!statusObserver.includes(unsafe), `Stop observation must not use ${unsafe}`);
}
const newStopBlock = `            let outcome = try XCTUnwrap(waitForStoppedOutcome(in: app), "Stop must expose one complete known local outcome with no gateway progress")
            cleanupEvidence = outcome.cleanupEvidence
`;
const originalStopBlock = `            let status = app.staticTexts["playback.status"]
            XCTAssertTrue(wait(status, "label CONTAINS 'stopped'"), "Stop must reach a visible settled outcome")
            if status.label.contains("provider cleanup is pending") {
                cleanupEvidence = "Stopped locally; visible status reports provider cleanup pending."
            } else if status.label == "Playback stopped." {
                cleanupEvidence = "Visible status reports Playback stopped without a cleanup warning."
            } else {
                cleanupEvidence = "Stopped locally; visible status reports an unconfirmed or uncertain cleanup/personal-report outcome."
            }
`;
assert.equal(journey.split(newStopBlock).length, 2, 'Only the shared Stop observation/classification changes');
assert.equal((journey.match(/activate\(stop, in: app\)/g) ?? []).length, 1);
assert.ok(source.includes('attachment.name = "playback-stop-fields.json"'));
for (const message of [
    'Playback stopped.',
    'Playback stopped locally; provider cleanup is pending.',
    'Playback stopped locally; the final personal report is not confirmed. Check session state or retry the identical Stop request.',
    'Playback stopped locally. Server cleanup is unconfirmed. Retry Stop uses the same final position and request.',
    ' An earlier personal-report outcome remains unknown.',
]) {
    assert.ok(nativeSource.includes(`"${message}"`), 'Stop classifier accepts only current fixed production messages');
}
assert.ok(journey.includes('if !volumeOnly {\n            let hidden ='));
assert.ok(journey.includes('if !volumeOnly {\n            for (id, control)'));
assert.ok(journey.includes('            #endif\n            }\n            openMenu("volume", in: app)'));
const originalJourney = journey
    .replace('        if remoteFocusOnly { app.launchEnvironment["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] = "1" }\n', '')
    .replace(', remoteFocusOnly: Bool = false', '')
    .replace(' && !remoteFocusOnly', '')
    .replace(',\n              !remoteFocusOnly || env["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] == "1"', '')
    .replaceAll(' || remoteFocusOnly', '')
    .replace('        remoteFocusReceiptsEnabled = remoteFocusOnly\n', '')
    .replace('        if remoteFocusOnly { XCTAssertEqual(profile, "tvos") }\n', '')
    .replace('        if remoteFocusOnly {\n            try observeTelevisionFocus(app)\n            return\n        }\n', '')
    .replace(newStopBlock, originalStopBlock)
    .replace('        clockReceiptsEnabled = volumeOnly || preparationOnly || absoluteSeekOnly\n', '')
    .replace('            if !volumeOnly {\n            let hidden =', '            let hidden =')
    .replace('            }\n            reveal(app)\n            activate(play, in: app)', '            reveal(app)\n            activate(play, in: app)')
    .replace('            if volumeOnly { attachClockFields("volume-paused", in: app) }\n', '')
    .replace('            if !volumeOnly {\n            for (id, control)', '            for (id, control)')
    .replace('            #endif\n            }\n            openMenu("volume", in: app)', '            #endif\n            openMenu("volume", in: app)')
    .replace('Focused actual volume/mute reproduction', 'Focused actual volume/seek/mute reproduction');
assert.equal(createHash('sha256').update(originalJourney).digest('hex'),
    '354e9814a466fae8621cde5df92e7a3b9532865c0039a1bb3a1cff801ce432b4',
    'Removing only the approved Stop observer and prior mode/receipt delta restores the entire original journey byte-for-byte');
assert.ok(source.includes('attachClockFields("failure", in: app)'));
assert.ok(source.includes('receipt["clock_fields"] = clockFieldsReceipt(in: app)'));
for (const guard of [
    '(!volumeOnly && !preparationOnly && !absoluteSeekOnly && !remoteFocusOnly) || env["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1"',
    '!absoluteSeekOnly || env["SCREENROOM_E2E_ABSOLUTE_SEEK_DIAGNOSTIC"] == "1"',
    '!remoteFocusOnly || env["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] == "1"',
]) {
    assert.ok(source.indexOf(guard) >= 0 && source.indexOf(guard) < source.indexOf('let app = XCUIApplication()'));
}
assert.ok(source.includes('func testNosferatuAbsoluteSeek() throws {\n        try runJourney(volumeOnly: false, absoluteSeekOnly: true)'));
assert.ok(source.includes('func testNosferatuTelevisionFocus() throws {\n        try runJourney(volumeOnly: false, remoteFocusOnly: true)'));
assert.ok(source.includes('if remoteFocusOnly {\n            try observeTelevisionFocus(app)\n            return\n        }'));
assert.ok(nativeSource.includes('var televisionStageFocusable: Bool { !visible && menu == nil }'));
assert.ok(nativeSource.includes('.focusable(chrome.televisionStageFocusable)'));
assert.ok(nativeSource.includes('if chrome.expire(interaction: interaction, visibility: visibility) {\n                focusedControl = "stage"'));
assert.equal((nativeSource.match(/\.focusSection\(\)/g) ?? []).length, 3, 'TV header/footer connect native focus regions; menus retain their section');
assert.ok(nativeSource.includes('.focusScope(toolbarFocusScope)\n                            .defaultFocus($focusedControl, defaultTelevisionFocus(layout), priority: .automatic)'));
assert.ok(nativeSource.includes('.onChange(of: menuReturn.pending) { _, request in'));
const toolbarRegion = nativeSource.slice(nativeSource.indexOf('                Group {\n                    if chrome.visible {'),
    nativeSource.indexOf('                if let menu = chrome.menu {'));
assert.equal((toolbarRegion.match(/televisionToolbar\(layout\)/g) ?? []).length, 1,
    'One stable toolbar focus environment survives modal open and close');
assert.ok(!toolbarRegion.includes('.onAppear') && !toolbarRegion.includes('if chrome.menu == nil'));
assert.ok(nativeSource.includes('.prefersDefaultFocus(preferredTelevisionFocus(layout) == id, in: toolbarFocusScope)'));
assert.ok(nativeSource.includes('generation == menuReturn.generation, interaction == chrome.interaction'));
assert.ok(nativeSource.includes('guard let target = menuReturn.claim(request, layout: layout'));
const returnPolicy = nativeSource.slice(nativeSource.indexOf('struct NativePlayerMenuReturn {'),
    nativeSource.indexOf('private struct NativePlayerIdleToken:'));
assert.ok(returnPolicy.includes('request == pending, request.generation == generation'));
assert.ok(returnPolicy.includes('request.interaction == interaction, visible, !menuOpen, !closed, active'));
assert.ok(returnPolicy.includes('return layout.restoredFocus(request.anchor)'));
assert.ok(returnPolicy.includes('guard !issued, let target = target('));
assert.ok(returnPolicy.includes('(issued && accessibilityFocus == issuedTarget)'));
assert.ok(returnPolicy.includes('issuedRequest.generation == generation'));
assert.ok(returnPolicy.includes('issuedRequest.interaction == interaction'));
assert.ok(returnPolicy.includes('if selfAcknowledged { pending = nil }'));
assert.ok(returnPolicy.includes('else if value != nil { cancel() }'));
assert.ok(nativeSource.includes('if !selfAcknowledged && value != nil && value != "stage" { chrome.reveal() }'));
assert.ok(nativeSource.includes('let returning = menuReturn.nativeFocusChanged(value, interaction: chrome.interaction'));
assert.ok(!returnPolicy.includes('focus == nil || focus == request.originFocus'),
    'Raw native fallback focus cannot invalidate an otherwise current return');
const televisionHandoff = nativeSource.slice(nativeSource.indexOf('    private func televisionToolbar('),
    nativeSource.indexOf('    private var television:'));
for (const bypass of ['Task {', 'Task.sleep', 'DispatchQueue', 'asyncAfter', 'Timer', 'UIFocusSystem', 'setNeedsFocusUpdate', 'playback.close(', 'playback.toggle(']) {
    assert.ok(!televisionHandoff.includes(bypass) && !returnPolicy.includes(bypass),
        `Native declarative handoff cannot add ${bypass}`);
}
assert.ok(!televisionHandoff.includes('focusedControl =') && !televisionHandoff.includes('resetFocus('),
    'The native default-focus declaration replaces every immediate menu-return setter/reset');
assert.ok(nativeSource.includes('televisionActive = false\n            menuReturn.cancel()'));
assert.ok(nativeSource.includes('if !visible { menuReturn.cancel() }'));
assert.ok(nativeSource.includes('.onChange(of: playback.closed) { _, _ in\n            traceFocus(.closedBefore)\n            menuReturn.cancel()'));
const nativeInputDeclaration = '.background(NativePlayerTelevisionInputObserver(nativeItems: nativeItemObservation.trace.enabled ? nativeItemObservation : nil) { input in\n';
assert.ok(nativeSource.includes(nativeInputDeclaration + '            traceFocus(.inputBefore)\n            menuReturn.cancel()'));
assert.ok(nativeSource.includes('.onExitCommand {\n            #if os(tvOS)\n            traceFocus(.exitBefore)\n            menuReturn.cancel()'));
const traceType = nativeSource.slice(nativeSource.indexOf('final class NativePlayerFocusTrace {'),
    nativeSource.indexOf('private struct NativePlayerIdleToken:'));
for (const forbidden of ['@Observable', 'ObservableObject', '@Published', '@State', 'Task', 'Timer',
    'focusedControl =', 'resetFocus(', 'FileManager', 'UserDefaults', 'String(describing:', 'String(reflecting:']) {
    assert.ok(!traceType.includes(forbidden), `Bounded inert focus trace cannot use ${forbidden}`);
}
assert.ok(traceType.includes('records.count < 128'));
assert.ok(traceType.includes('event == .scheduleBefore { armed = true }'));
assert.ok(traceType.includes('state != lastPreferred'));
assert.ok(traceType.includes('environment["SCREENROOM_PLAYBACK_DIAGNOSTICS"] == "1"'));
assert.ok(traceType.includes('environment["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] == "1"'));
const nativeItemObserver = nativeSource.slice(nativeSource.indexOf('private final class NativePlayerNativeItemObservation:'),
    nativeSource.indexOf('private struct NativePlayerTelevisionInputObserver:'));
assert.ok(nativeItemObserver.includes('UIFocusSystem.didUpdateNotification'));
assert.ok(nativeItemObserver.includes('UIFocusSystem.movementDidFailNotification'));
assert.ok(nativeItemObserver.includes('UIFocusSystem.focusUpdateContextUserInfoKey'));
assert.ok(nativeItemObserver.includes('context?.previouslyFocusedItem'));
assert.ok(nativeItemObserver.includes('context?.nextFocusedItem'));
assert.ok(nativeItemObserver.includes('let current = system?.focusedItem'));
assert.ok(nativeItemObserver.includes('NativePlayerNativeItemTrace.acceptsNotification('));
assert.ok(nativeItemObserver.includes('let matches = system.flatMap { owned in notifier.map { $0 === owned } }'));
assert.ok(nativeItemObserver.includes('window.convert(item.frame, from: container.coordinateSpace)'));
assert.ok(nativeItemObserver.includes('current.parentFocusEnvironment'));
assert.ok(nativeItemObserver.includes('anchor == "volume", !trace.armed, let window = ownedWindow'));
assert.ok(nativeItemObserver.includes('trace.arm(anchor: item)'));
assert.ok(nativeItemObserver.includes('guard Thread.isMainThread, trace.canRecord, let window = ownedWindow'));
assert.ok(nativeItemObserver.includes('guard Thread.isMainThread else { return }'));
assert.ok(nativeItemObserver.includes('MainActor.assumeIsolated { receive(context: context, notifier: notifier, event: event) }'));
assert.ok(nativeItemObserver.includes('guard trace.enabled, ownedWindow !== window else { return }'));
for (const forbidden of ['requestFocusUpdate', 'setNeedsFocusUpdate', 'updateFocusIfNeeded', 'resetFocus(',
    'focusedControl', 'accessibleControl', 'defaultFocus(', 'prefersDefaultFocus(', 'Task', 'Timer',
    'DispatchQueue', 'becomeFirstResponder', 'String(describing:', 'String(reflecting:', 'value(forKey:', 'perform(']) {
    assert.ok(!nativeItemObserver.includes(forbidden), `Native-item observation cannot perform ${forbidden}`);
}
assert.ok(nativeSource.includes('private weak var nativeItems: NativePlayerNativeItemObservation?'));
assert.ok(nativeSource.includes('nativeItems?.attach(to: window)'));
assert.ok(nativeSource.includes('nativeItems?.detach()'));
assert.ok(traceType.includes('private(set) weak var windowObject: AnyObject?'));
assert.ok(traceType.includes('private(set) weak var anchorObject: AnyObject?'));
assert.ok(traceType.includes('weak var object: AnyObject?'));
assert.ok(traceType.includes('guard canRecord else { return }'));
assert.ok(traceType.includes('guard entries.count < 128 else { return nil }'));
assert.ok(traceType.includes('sink("ScreenroomPlayerNativeItemTrace " + text)'));
assert.ok(source.includes('if remoteFocusOnly { app.launchEnvironment["SCREENROOM_E2E_REMOTE_FOCUS_DIAGNOSTIC"] = "1" }'));
assert.ok(source.includes('if remoteFocusReceiptsEnabled { receipt["observation_uptime"] = ProcessInfo.processInfo.systemUptime }'));
const restoredObservation = source.slice(source.indexOf('    @MainActor private func observeRestoredToolbarFocus('),
    source.indexOf('    @MainActor private func televisionMenuButton('));
assert.ok(restoredObservation.includes('targets.count == 1 && focused.count == 1'));
assert.ok(restoredObservation.includes('targets[0].hasFocus && focused[0].identifier == identifier'));
assert.equal((restoredObservation.match(/targets\[0\]\.isEnabled/g) ?? []).length, 2,
    'Both the wait predicate and fresh final observation require an enabled target');
assert.ok(restoredObservation.includes('remoteFocusReceipt(targets[0], focused: focused, failure: !ready)'));
for (const action of ['XCUIRemote', 'activate(', 'focus(', '.tap(', '.click(', '.firstMatch', '.setValue(', 'focusedControl =']) {
    assert.ok(!restoredObservation.includes(action), `Post-modal observation must not issue ${action}`);
}
const remoteFocus = source.slice(source.indexOf('    #if os(tvOS)\n    @MainActor private func focus('));
const televisionProbe = source.slice(source.indexOf('    @MainActor private func observeTelevisionFocus('),
    source.indexOf('    @MainActor private func stopFields('));
assert.ok(televisionProbe.includes('app.scrollViews.matching(identifier: "playback.menu.volume").allElementsBoundByIndex'));
assert.ok(televisionProbe.includes('menus.count == 1 ? menus[0] : nil'));
assert.ok(televisionProbe.includes('menu.buttons.matching(NSPredicate(format: "label IN %@", labels as NSArray)).allElementsBoundByIndex'));
assert.ok(televisionProbe.includes('PlaybackCapturePolicy.uniqueMenuButtonIndex('));
assert.ok(!televisionProbe.includes('app.buttons["Mute"]'));
assert.ok(!televisionProbe.includes('focus(app.buttons[label]'));
for (const unsafe of ['.firstMatch', '.element(boundBy:', 'coordinate(', 'tap(', 'click(', 'setValue(']) {
    assert.ok(!televisionProbe.includes(unsafe), `Menu navigation must retain unique scoped leaf selection, not ${unsafe}`);
}
assert.ok(remoteFocus.includes('for _ in 0..<30'));
assert.ok(remoteFocus.includes('guard matches.count == 1 else'));
assert.ok(remoteFocus.includes('if element.hasFocus { return true }'));
assert.ok(remoteFocus.includes('remoteFocusReceipt(element, focused: finalFocus, failure: !ready)'));
assert.ok(!remoteFocus.includes('focused: [], failure:'), 'Failure receipts must read actual focus, not invent an empty result');
assert.ok(remoteFocus.includes('XCUIRemote.shared.press('));
assert.ok(remoteFocus.includes('PlaybackCapturePolicy.remoteDirection(target: element.frame, focused: focused.frame)'));
assert.ok(!remoteFocus.includes('abs(dx) > 100'), 'Separate rows require vertical remote navigation before horizontal alignment');
for (const bypass of ['.tap(', '.click(', '.coordinate(', '.setValue(', 'performAccessibilityAction', 'focusedControl =', 'String(describing:', 'String(reflecting:']) {
    assert.ok(!remoteFocus.includes(bypass), `Remote focus cannot use ${bypass}`);
}
const focusedRemote = source.slice(source.indexOf('@MainActor private func observeTelevisionFocus('), source.indexOf('@MainActor private func stopFields('));
assert.equal((focusedRemote.match(/step: "stop", in: app\)/g) ?? []).length, 1);
assert.equal((focusedRemote.match(/televisionProofPress\(\.select, button: "select", step: "back"/g) ?? []).length, 1);
assert.ok(focusedRemote.includes('XCUIRemote.shared.press(button)'));
assert.ok(focusedRemote.includes('buttons[0].isHittable'));
assert.ok(focusedRemote.includes('No empty-focus search') || focusedRemote.includes('no empty-focus search'));
assert.ok(focusedRemote.includes('PlaybackCapturePolicy.immediateRight('));
const focusedBody = focusedRemote.slice(0, focusedRemote.indexOf('    #if os(tvOS)\n    @MainActor private func televisionProofMenuFocus'));
assert.ok(!focusedBody.includes('observeRestoredToolbarFocus('));
for (const step of ['volume-return', 'tracks-return']) {
    assert.equal((focusedBody.match(new RegExp(`televisionProofPress\\(\\.select, button: "select", step: "${step}"`, 'g')) ?? []).length, 1);
}
assert.ok(!focusedRemote.includes('playback-manifest-success') && !focusedRemote.includes('seek(to:') && !focusedRemote.includes('adjustVolume('));
const absoluteObserver = source.slice(source.indexOf('@MainActor private func observeAbsoluteSeek('), source.indexOf('@MainActor private func observePreparation('));
assert.equal((absoluteObserver.match(/\.adjust\(toNormalizedSliderPosition:/g) ?? []).length, 1);
assert.ok(source.includes('if absoluteSeekOnly {\n            observeAbsoluteSeek(app)\n            return\n        }'));
assert.ok(!absoluteObserver.includes('playback-manifest-success') && !absoluteObserver.includes('naturalError('));
assert.ok(absoluteObserver.indexOf('attachAbsoluteSeek("cleanup"') < absoluteObserver.indexOf('XCTAssertTrue(issues.isEmpty'));
const normalAbsoluteBlock = source.slice(source.indexOf('            #if !os(tvOS)\n            let position = app.sliders["playback.position"]'), source.indexOf('            openMenu("volume", in: app)'));
assert.ok(normalAbsoluteBlock.includes('PlaybackCapturePolicy.absoluteSeekPlan(before)'));
assert.equal((normalAbsoluteBlock.match(/\.adjust\(toNormalizedSliderPosition:/g) ?? []).length, 1);
assert.equal((normalAbsoluteBlock.match(/PlaybackCapturePolicy\.absoluteSeekSettled\(/g) ?? []).length, 3);
assert.ok(!normalAbsoluteBlock.includes('toNormalizedSliderPosition: 0.2') && !normalAbsoluteBlock.includes('timelineReading('));
assert.ok(absoluteObserver.includes('PlaybackCapturePolicy.seekCorrelationComplete(after["seek_correlation"] as? [String: Any])'));
assert.ok(source.includes('receipt["seek_correlation"] = PlaybackCapturePolicy.seekCorrelationReceipt('));
assert.equal((nativeSource.match(/readPosition\?\(\)/g) ?? []).length, 1, 'Correlation reuses the existing position read');
assert.equal((nativeSource.match(/await seekMedia\(/g) ?? []).length, 2, 'Correlation adds no media seek');
const seekRecorders = nativeSource.slice(nativeSource.indexOf('    func recordSliderReceived('), nativeSource.indexOf('    func recordDiagnostic('));
for (const unsafe of ['Task {', 'await ', 'client.', 'seekMedia(', 'currentTime(', 'readPosition']) {
    assert.ok(!seekRecorders.includes(unsafe), `Correlation recorders must not add ${unsafe}`);
}
assert.ok(nativeSource.includes('playback.recordSliderReceived($0)') && nativeSource.includes('let diagnostic = playback.recordSliderEnded(target: target)'));
assert.ok(nativeSource.includes('Task { await playback.seek(to: target, diagnostic: diagnostic) }'));
const namespaceGuard = source.indexOf('let settingsNamespace = try XCTUnwrap(PlaybackCapturePolicy.settingsNamespace(runID: runID),');
assert.ok(namespaceGuard >= 0 && namespaceGuard < source.indexOf('let app = XCUIApplication()'),
    'An invalid selected namespace must throw a test failure before any app creation');
assert.ok(source.includes('app.launchEnvironment["SCREENROOM_E2E_SETTINGS_NAMESPACE"] = settingsNamespace'));
assert.ok(!source.includes('"playback-showcase-\\(runID)"'));
const namespacePolicy = source.slice(source.indexOf('    static func settingsNamespace('), source.indexOf('    static func seekCorrelationReceipt('));
for (const effect of ['XCUIApplication', 'FileManager', 'UserDefaults', 'launch(', 'XCTSkip']) {
    assert.ok(!namespacePolicy.includes(effect), `Namespace rejection must not perform ${effect}`);
}
assert.ok(source.includes('func testNosferatuPreparation() throws {\n        try runJourney(volumeOnly: false, preparationOnly: true)'));
assert.ok(source.includes('if preparationOnly {\n            observePreparation(app)\n            return\n        }'));
const preparationObserver = source.slice(source.indexOf('@MainActor private func observePreparation('), source.indexOf('@MainActor private func advancingTimeline('));
assert.ok(!preparationObserver.includes('playback-manifest-success') && !preparationObserver.includes('naturalError('));
assert.ok(preparationObserver.indexOf('playback-preparation-cleanup.txt') < preparationObserver.indexOf('XCTAssertTrue(issues.isEmpty'));
const diagnosticMapping = nativeSource.slice(nativeSource.indexOf('enum NativePlaybackPreparationDiagnostic {'), nativeSource.indexOf('@MainActor @Observable\nfinal class NativePlayback {'));
for (const unsafe of ['String(describing:', 'String(reflecting:', 'localizedDescription', 'errorDescription', 'mediaUrl', 'controlCapability']) {
    assert.ok(!diagnosticMapping.includes(unsafe), `Preparation diagnostics must not expose ${unsafe}`);
}
const focusedExit = source.indexOf('attachment.name = "playback-focused-volume-complete.txt"');
assert.ok(focusedExit > 0 && focusedExit < source.indexOf('let manifest: [String: Any]'));
assert.ok(source.slice(focusedExit, source.indexOf('let manifest: [String: Any]')).includes('return'));
const diagnostic = source.slice(source.indexOf('@MainActor private func volumeDiagnostic('), source.indexOf('\n    private func missing('));
for (const mutation of ['activate(', 'openMenu(', 'closeMenu(', 'reveal(', '.tap(', '.click(']) {
    assert.ok(!diagnostic.includes(mutation), `Diagnostic observation must not perform ${mutation}`);
}
assert.ok(nativeSource.includes('@Environment(\\.isEnabled) private var enabled'));
assert.ok(nativeSource.includes('playback.recordDiagnostic("control.\\(control).\\(event)", enabled: enabled, phase: phase)'));
assert.ok(nativeSource.includes('recordDiagnostic("volume.begin")') && nativeSource.includes('recordDiagnostic("volume.end")'));
const fullscreenObservation = source.slice(source.indexOf('@MainActor private func fullscreenControl('), source.indexOf('@MainActor private func exposeFullscreenControl('));
const fullscreenWait = source.slice(source.indexOf('@MainActor private func waitForFullscreen('), source.indexOf('\n    #endif', source.indexOf('@MainActor private func waitForFullscreen(')));
for (const body of [fullscreenObservation, fullscreenWait]) {
    assert.ok(body.length > 0);
    for (const mutation of ['activate(', 'openMenu(', 'closeMenu(', 'reveal(', '.tap(', '.click(']) {
        assert.ok(!body.includes(mutation), `Fullscreen observation must not perform ${mutation}`);
    }
}
// Failed run 20261116T025811Z-EzhTBX, exported Session log lines 14233–14239.
const failedVolumeAX = `App element pid: 37181
  Window
    Button
      Group
        AX element pid: 37192, token: SJEAAD2RAABvY29jKwAAAAAAAAA=`;
const extractedReadouts = failedVolumeAX.split('\n').filter(line => line.includes("identifier: 'playback.volume.readout'"));
assert.equal(extractedReadouts.length, 0, 'The failed sparse AX tree supplies no volume readout; never invent its role/label/value');
const root = join(apple, 'build/showcase-tooling');
mkdirSync(root, { recursive: true });
const directory = mkdtempSync(join(root, 'control-contract-'));
try {
    const file = join(directory, 'main.swift');
    const nativePolicies = nativeSource.slice(nativeSource.indexOf('enum NativePlayerMenu:'),
        nativeSource.indexOf('private struct NativePlayerIdleToken:'));
    const callbackStart = nativeSource.indexOf('        .onChange(of: accessibleControl) { _, value in\n            #if os(tvOS)\n');
    assert.ok(callbackStart >= 0);
    const callbackBody = nativeSource.slice(
        nativeSource.indexOf('            traceFocus(.accessibilityBefore,', callbackStart),
        nativeSource.indexOf('            #else\n', callbackStart));
    assert.ok(callbackBody.includes('menuReturn.accessibilityChanged(value, interaction: chrome.interaction'));
    const nativeCallbackStart = nativeSource.indexOf('        .onChange(of: focusedControl) { _, value in\n            #if os(tvOS)\n');
    const nativeCallbackBody = nativeSource.slice(nativeSource.indexOf('            traceFocus(.nativeBefore,', nativeCallbackStart),
        nativeSource.indexOf('            #else\n', nativeCallbackStart));
    assert.ok(nativeCallbackStart >= 0 && nativeCallbackBody.includes('menuReturn.nativeFocusChanged('));
    assert.ok(!nativeCallbackBody.includes('chrome.reveal()'),
        'Native observations never impersonate physical input by advancing interaction');
    const inputStart = nativeSource.indexOf(nativeInputDeclaration);
    const inputBody = nativeSource.slice(nativeSource.indexOf('            traceFocus(.inputBefore)', inputStart),
        nativeSource.indexOf('        })', inputStart));
    assert.ok(inputStart >= 0 && inputBody.includes('chrome.televisionInput(input, focus: focusedControl)'));
    const effectStart = nativeSource.indexOf('    private func restoreToolbarFocus(');
    const effectBody = nativeSource.slice(nativeSource.indexOf('{', effectStart) + 1,
        nativeSource.indexOf('    }\n    #endif', effectStart));
    assert.ok(effectStart >= 0 && effectBody.includes('menuReturn.claim(') && !effectBody.includes('focusedControl ='));
    const traceHelper = nativeSource.slice(nativeSource.indexOf('    private func traceFocus('),
        nativeSource.indexOf('    private func televisionToolbar(')).replace('    private func', '    func')
        .replace('        nativeItemObservation.observe(event, generation: menuReturn.generation, interaction: chrome.interaction, anchor: anchor)\n', '');
    const preferredStart = nativeSource.indexOf('    private func preferredTelevisionFocus(');
    const closedStart = nativeSource.indexOf('    private func restoreClosedAccessibilityFocus(');
    const closedBody = nativeSource.slice(nativeSource.indexOf(' {\n', closedStart) + 3, preferredStart - 1).trimEnd();
    const defaultStart = nativeSource.indexOf('    private func defaultTelevisionFocus(');
    const preferredBody = nativeSource.slice(nativeSource.indexOf(' {\n', preferredStart) + 3, defaultStart - 1).trimEnd();
    const defaultBody = nativeSource.slice(nativeSource.indexOf(' {\n', defaultStart) + 3, effectStart - 1).trimEnd()
        .replaceAll('preferredTelevisionFocus(', 'actualPreferredFocus(');
    const dismissStart = nativeSource.indexOf('    @discardableResult private func dismissMenu()');
    const dismissBody = nativeSource.slice(nativeSource.indexOf(' {\n', dismissStart) + 3,
        nativeSource.indexOf('    private func time(', dismissStart)).trimEnd();
    let tvActive = true;
    const tvParents = [];
    const dismissTV = dismissBody.split('\n').filter(line => {
        if (/^\s*#if os\(tvOS\)/.test(line)) { tvParents.push(tvActive); return false; }
        if (/^\s*#else/.test(line)) { tvActive = tvParents.at(-1) && !tvActive; return false; }
        if (/^\s*#endif/.test(line)) { tvActive = tvParents.pop(); return false; }
        return tvActive;
    }).join('\n');
    writeFileSync(file, 'import Foundation\nimport CoreGraphics\n' + nativePolicies + source.slice(start, end) + `
 func check(_ value: Bool, _ message: String) { precondition(value, message) }
 var menuReturn = NativePlayerMenuReturn()
 var chrome = NativePlayerChromeState()
 var playback = NativePlayerVisibility()
  var televisionActive = true
  var focusTrace = NativePlayerFocusTrace(enabled: false)
  ${traceHelper}
  var voiceOver = false
  var focusAssignments = 0
  var nativeResets = 0
  var focusedControl: String? = nil { didSet { focusAssignments += 1 } }
  var accessibleControl: String?
  let toolbarFocusScope = 0
  func resetFocus(in scope: Int) { nativeResets += 1 }
 let returnLayout = NativePlayerStageLayout(size: CGSize(width: 1920, height: 1080), television: true)
  func actualAccessibilityCallback(_ value: String?) {
      accessibleControl = value
  ${callbackBody}
  }
  func actualNativeCallback(_ value: String?) {
      let assignments = focusAssignments
      focusedControl = value
      focusAssignments = assignments
  ${nativeCallbackBody}
  }
  func actualInputCallback(_ input: NativePlayerTelevisionInput) {
  ${inputBody}
  }
  func actualToolbarEffect(_ request: NativePlayerMenuReturn.Request?, generation: Int,
                          interaction: Int, layout: NativePlayerStageLayout) {
  ${effectBody}
  }
  func actualPreferredFocus(_ layout: NativePlayerStageLayout) -> String? {
  ${preferredBody}
  func actualDefaultFocus(_ layout: NativePlayerStageLayout) -> String? {
  ${defaultBody}
  func actualClosedAccessibility(generation: Int, interaction: Int) {
  ${closedBody}
  func actualMenuDismiss() -> Bool {
  ${dismissTV}
  func preparePendingReturn() -> NativePlayerMenuReturn.Request {
      chrome = NativePlayerChromeState()
      playback = NativePlayerVisibility()
      televisionActive = true
      voiceOver = false
      menuReturn = NativePlayerMenuReturn()
      focusedControl = "menu.close"
      accessibleControl = nil
      focusAssignments = 0
      nativeResets = 0
      menuReturn.schedule(anchor: "volume", interaction: chrome.interaction, focus: "menu.close", accessibilityFocus: nil)
      return menuReturn.pending!
  }
  func prepareClaimedReturn() -> NativePlayerMenuReturn.Request {
      let request = preparePendingReturn()
      actualToolbarEffect(request, generation: request.generation, interaction: request.interaction, layout: returnLayout)
       check(actualDefaultFocus(returnLayout) == "volume" && focusAssignments == 0 && nativeResets == 0,
           "The eligible registration offers one native default without an immediate setter")
      return request
 }
 func preferredReturn(_ request: NativePlayerMenuReturn.Request?, accessibilityFocus: String?) -> String? {
     menuReturn.target(request, layout: returnLayout, interaction: chrome.interaction,
         visible: chrome.visible, menuOpen: chrome.menu != nil, closed: playback.closed, active: televisionActive,
         focus: nil, accessibilityFocus: accessibilityFocus)
 }
  let fallbackReturn = preparePendingReturn()
  focusedControl = "back"
  focusAssignments = 0
  let fallbackInteraction = chrome.interaction
  actualNativeCallback("back")
  check(menuReturn.pending == fallbackReturn && chrome.interaction == fallbackInteraction,
        "Automatic Back before toolbar appearance is observation, not human intent")
  actualToolbarEffect(fallbackReturn, generation: fallbackReturn.generation,
      interaction: fallbackReturn.interaction, layout: returnLayout)
   check(focusedControl == "back" && actualDefaultFocus(returnLayout) == "volume" && focusAssignments == 0 && nativeResets == 0,
         "A current registration offers Volume despite provisional Back; only the engine can complete")
  actualNativeCallback("volume")
  check(menuReturn.pending == nil && chrome.interaction == fallbackInteraction,
        "Native target acknowledgement completes without a counterfeit interaction")
  actualAccessibilityCallback("volume")
  actualToolbarEffect(fallbackReturn, generation: fallbackReturn.generation,
      interaction: fallbackReturn.interaction, layout: returnLayout)
   check(chrome.interaction == fallbackInteraction && focusAssignments == 0 && actualDefaultFocus(returnLayout) == "volume",
        "Late own VoiceOver acknowledgement and appearance cannot steal focus again")
   _ = prepareClaimedReturn()
   let claimedInteraction = chrome.interaction
  actualAccessibilityCallback("volume")
  check(chrome.interaction == claimedInteraction && menuReturn.pending == nil,
        "The actual extracted callback must not advance interaction for its own VoiceOver acknowledgement")
  actualNativeCallback("volume")
   check(chrome.interaction == claimedInteraction && focusAssignments == 0,
         "VoiceOver-first then native acknowledgement also completes once")
   let observedRequest = prepareClaimedReturn()
   let observedInteraction = chrome.interaction
   actualNativeCallback(nil)
   actualNativeCallback("back")
   check(menuReturn.pending == observedRequest && menuReturn.issued &&
         actualDefaultFocus(returnLayout) == "volume" && focusedControl == "back" &&
         chrome.interaction == observedInteraction && focusAssignments == 0 && nativeResets == 0,
         "Actual traced claim -> nil -> Back preserves the native preference, not a retry or false completion")
   actualToolbarEffect(observedRequest, generation: observedRequest.generation,
       interaction: observedRequest.interaction, layout: returnLayout)
   check(focusAssignments == 0 && actualDefaultFocus(returnLayout) == "volume",
         "Repeated view evaluation cannot issue another imperative focus request")
  let humanRightReturn = preparePendingReturn()
  actualInputCallback(.navigation)
  let humanInteraction = chrome.interaction
  actualNativeCallback("tracks")
  actualToolbarEffect(humanRightReturn, generation: humanRightReturn.generation,
      interaction: humanRightReturn.interaction, layout: returnLayout)
  check(menuReturn.pending == nil && focusAssignments == 0 && chrome.interaction == humanInteraction &&
        humanInteraction != humanRightReturn.interaction,
        "Actual remote input invalidates the generation before focus or toolbar appearance")
 let humanVoiceOverReturn = prepareClaimedReturn()
 actualAccessibilityCallback("tracks")
  check(menuReturn.pending == nil && chrome.interaction != humanVoiceOverReturn.interaction &&
        preferredReturn(humanVoiceOverReturn, accessibilityFocus: "tracks") == nil,
        "Unrelated VoiceOver focus cancels and retains the normal human reveal")
  let beforeAppearanceVoiceOver = preparePendingReturn()
  actualAccessibilityCallback("tracks")
  actualToolbarEffect(beforeAppearanceVoiceOver, generation: beforeAppearanceVoiceOver.generation,
      interaction: beforeAppearanceVoiceOver.interaction, layout: returnLayout)
  check(menuReturn.pending == nil && focusAssignments == 0,
        "Unrelated human VoiceOver before appearance prevents every obsolete anchor assignment")
  for boundary in ["interaction", "menu", "hide", "closed", "teardown", "new-generation"] {
      let obsolete = preparePendingReturn()
      switch boundary {
      case "interaction": chrome.reveal()
      case "menu": menuReturn.cancel(); chrome.open(.tracks, anchor: "tracks")
      case "hide": menuReturn.cancel(); _ = chrome.expire(interaction: chrome.interaction, visibility: NativePlayerVisibility())
      case "closed": menuReturn.cancel(); playback.closed = true
      case "teardown": televisionActive = false; menuReturn.cancel()
      default: menuReturn.schedule(anchor: "tracks", interaction: chrome.interaction, focus: "menu.close", accessibilityFocus: nil)
      }
      actualNativeCallback("back")
      actualToolbarEffect(obsolete, generation: obsolete.generation, interaction: obsolete.interaction, layout: returnLayout)
      check(focusAssignments == 0, "A stale eligible-toolbar callback cannot assign an anchor after " + boundary)
  }
  let oldMenu = preparePendingReturn()
  menuReturn.schedule(anchor: "tracks", interaction: chrome.interaction, focus: "menu.close", accessibilityFocus: nil)
  let newMenu = menuReturn.pending!
  actualNativeCallback("back")
  actualToolbarEffect(oldMenu, generation: oldMenu.generation, interaction: oldMenu.interaction, layout: returnLayout)
  actualToolbarEffect(newMenu, generation: newMenu.generation, interaction: newMenu.interaction, layout: returnLayout)
   check(focusAssignments == 0 && actualDefaultFocus(returnLayout) == "tracks",
        "Automatic fallback cannot resurrect menu A or cancel the newer menu B intent")
  let compactRequest = preparePendingReturn()
  let compactLayout = NativePlayerStageLayout(size: CGSize(width: 120, height: 150), television: true)
  actualToolbarEffect(compactRequest, generation: compactRequest.generation,
      interaction: compactRequest.interaction, layout: compactLayout)
   check(focusAssignments == 0 && actualDefaultFocus(compactLayout) == "session",
        "Only a genuinely absent menu anchor uses semantic compact-layout fallback")
  _ = preparePendingReturn()
  menuReturn.cancel()
   let initialInteraction = chrome.interaction
  actualNativeCallback("play")
   check(chrome.interaction == initialInteraction && !chrome.televisionControlEngaged,
         "Initial automatic control focus creates no human engagement or idle deadline refresh")
   _ = preparePendingReturn()
   playback.closed = true
   menuReturn.cancel()
   voiceOver = true
   let closedGeneration = menuReturn.generation
   let closedInteraction = chrome.interaction
   actualClosedAccessibility(generation: closedGeneration, interaction: closedInteraction)
   check(accessibleControl == "back" && actualDefaultFocus(returnLayout) == "back" && focusAssignments == 0,
         "Closed-owner Back uses the native declaration and preserves its existing VoiceOver destination")
   accessibleControl = "tracks"
   actualInputCallback(.navigation)
   actualClosedAccessibility(generation: closedGeneration, interaction: closedInteraction)
   check(accessibleControl == "tracks", "A stale closed-owner accessibility callback cannot override new human input")
  for boundary in ["interaction", "menu", "hide", "closed", "teardown", "new-generation"] {
     let obsoleteReturn = prepareClaimedReturn()
     switch boundary {
     case "interaction": chrome.reveal()
     case "menu": chrome.open(.tracks, anchor: "tracks")
     case "hide": _ = chrome.expire(interaction: chrome.interaction, visibility: NativePlayerVisibility())
     case "closed": playback.closed = true
     case "teardown": televisionActive = false
     default: menuReturn.schedule(anchor: "tracks", interaction: chrome.interaction, focus: "menu.close", accessibilityFocus: nil)
     }
     actualAccessibilityCallback("volume")
     check(menuReturn.pending == nil && preferredReturn(obsoleteReturn, accessibilityFocus: "volume") == nil,
           "Stale self-acknowledgement cannot survive " + boundary)
 }
   print("Extracted callbacks and native default registration: observed nil/Back preserves one preference; no setter/retry; human/VoiceOver/stale boundaries cancel")
  func tracedCallbackSequence(_ enabled: Bool) -> [String] {
      _ = preparePendingReturn()
      menuReturn.cancel()
      chrome.open(.volume, anchor: "volume")
      focusedControl = "menu.close"
      focusAssignments = 0
      var time = 1.0
      focusTrace = NativePlayerFocusTrace(enabled: enabled, clock: { time += 1; return time }, sink: { _ in })
      check(actualMenuDismiss(), "The exact product dismiss callback schedules its trace without fixture request injection")
      let request = menuReturn.pending!
      actualNativeCallback("back")
      check(actualPreferredFocus(returnLayout) == "volume", "The actual preferred-target getter records the pending anchor")
       actualToolbarEffect(request, generation: request.generation, interaction: request.interaction, layout: returnLayout)
       actualNativeCallback(nil)
       actualNativeCallback("back")
       check(actualDefaultFocus(returnLayout) == "volume", "Default focus remains registered after the actual failed order")
       actualNativeCallback("volume")
       check(actualPreferredFocus(returnLayout) == nil, "The actual getter records preference after local acknowledgement, not AX completion")
       check(actualDefaultFocus(returnLayout) == "volume", "Local acknowledgement must not withdraw the registered native preference")
      actualAccessibilityCallback("volume")
      check(focusTrace.records.isEmpty == !enabled, "The actual callbacks emit only under the explicit diagnostic gate")
      if enabled {
          let events = focusTrace.records.compactMap { $0["event"] as? String }
           check(events.filter { $0 == "claimAfter" }.count == 1 &&
                 !events.contains("assignmentBefore") && !events.contains("assignmentAfter"),
                 "One accepted preference claim, no imperative setter or retry")
           let defaults = focusTrace.records.filter { $0["event"] as? String == "defaultPreference" }
           check(defaults.contains { record in
               let state = record["state"] as! [String: Any]
               return state["target"] as? String == "volume" && state["focus_state"] as? String == "back"
           }, "Actual local Back plus offered native Volume is explicit, not an AX completion claim")
          if ProcessInfo.processInfo.environment["SCREENROOM_FOCUS_TRACE_FIXTURE"] == "1" {
              let data = try! JSONSerialization.data(withJSONObject: focusTrace.records, options: [.sortedKeys])
              print("NativePlayerFocusTraceFixture " + String(data: data, encoding: .utf8)!)
          }
      }
      return [String(chrome.interaction), String(focusAssignments), String(nativeResets),
              String(menuReturn.pending == nil), focusedControl ?? "missing", menuReturn.issuedTarget ?? "missing"]
  }
  check(tracedCallbackSequence(false) == tracedCallbackSequence(true),
        "Diagnostic emission does not alter callback state, request counts or product focus assignments")
  focusTrace = NativePlayerFocusTrace(enabled: false)
  print("Actual instrumented callback sequence: bounded product trace preserves behavioral results; local FocusState is not independent AX completion")
 let muteLabels: Set<String> = ["Mute", "Unmute"]
// Current AX: disabled toolbar Mute plus enabled menu Mute. Global ambiguity must fail.
let actualGlobalMute = [(label: "Mute", enabled: false), (label: "Mute", enabled: true)]
let toolbarNames = ["play", "backward", "forward", "mute", "volume", "tracks", "session"]
let actualToolbarFixture = toolbarNames.enumerated().map { index, name in
    PlaybackCapturePolicy.ToolbarItem(control: name, enabled: true,
        frame: CGRect(x: 678 + Double(index) * 82, y: 909.5, width: 72, height: 72))
}
check(PlaybackCapturePolicy.immediateRight(from: "volume", to: "tracks", before: actualToolbarFixture, current: actualToolbarFixture) &&
      PlaybackCapturePolicy.immediateRight(from: "tracks", to: "session", before: actualToolbarFixture, current: actualToolbarFixture),
      "Actual toolbar geometry supports only the immediate eligible right neighbor")
check(!PlaybackCapturePolicy.immediateRight(from: "volume", to: "session", before: actualToolbarFixture, current: actualToolbarFixture),
      "A skipped eligible neighbor cannot authorize one right step")
for mutation in 0..<8 {
    var row = actualToolbarFixture
    switch mutation {
    case 0: row.removeLast()
    case 1: row[6] = row[5]
    case 2: row[6] = .init(control: "other", enabled: true, frame: row[6].frame)
    case 3: row[5] = .init(control: "tracks", enabled: false, frame: row[5].frame)
    case 4: row[5] = .init(control: "tracks", enabled: true, frame: CGRect(x: 1088, y: 600, width: 72, height: 72))
    case 5: row[5] = .init(control: "tracks", enabled: true, frame: .infinite)
    case 6: row[5] = .init(control: "tracks", enabled: true, frame: CGRect(x: CGFloat.nan, y: 909.5, width: 72, height: 72))
    default: row[5] = .init(control: "tracks", enabled: true, frame: row[4].frame)
    }
    check(!PlaybackCapturePolicy.immediateRight(from: "volume", to: "tracks", before: row, current: row),
          "Missing/duplicate/unknown/disabled/non-row/invalid/overlapping toolbar rejects")
}
var changedToolbar = actualToolbarFixture
changedToolbar[5] = .init(control: "tracks", enabled: true, frame: CGRect(x: 1089, y: 909.5, width: 72, height: 72))
check(!PlaybackCapturePolicy.immediateRight(from: "volume", to: "tracks", before: actualToolbarFixture, current: changedToolbar),
      "A changed reference inventory cannot authorize input")
check(PlaybackCapturePolicy.uniqueMenuButtonIndex(actualGlobalMute, labels: muteLabels) == nil,
      "The actual global two-button query cannot choose a first or enabled fallback")
check(PlaybackCapturePolicy.uniqueMenuButtonIndex([(label: "Mute", enabled: true)], labels: muteLabels) == 0,
      "The actual volume-menu Button leaf is the sole enabled Mute target")
for label in ["Quieter", "Louder", "Mute", "Unmute"] {
    check(PlaybackCapturePolicy.uniqueMenuButtonIndex([(label: label, enabled: true)], labels: [label]) == 0,
          "Unique scoped enabled Button leaves preserve normal remote focus")
    check(PlaybackCapturePolicy.uniqueMenuButtonIndex([(label: label, enabled: false)], labels: [label]) == nil &&
          PlaybackCapturePolicy.uniqueMenuButtonIndex([(label: label, enabled: true), (label: label, enabled: true)], labels: [label]) == nil,
          "Disabled or duplicated menu targets remain failures")
}
check(PlaybackCapturePolicy.uniqueMenuButtonIndex([], labels: muteLabels) == nil &&
      PlaybackCapturePolicy.uniqueMenuButtonIndex([(label: "other", enabled: true)], labels: muteLabels) == nil &&
      PlaybackCapturePolicy.uniqueMenuButtonIndex([(label: "Mute", enabled: true), (label: "Unmute", enabled: true)], labels: muteLabels) == nil,
      "Missing, wrong, or conflicting leaves never select an ancestor or arbitrary match")
// Exact current tvOS AX frames; button geometry supplies direction, never a pointer action.
let actualTVPlay = CGRect(x: 678, y: 909.5, width: 72, height: 72)
let actualTVForward = CGRect(x: 842, y: 909.5, width: 72, height: 72)
let actualTVBack = CGRect(x: 118.5, y: 98.5, width: 72, height: 72)
let actualTVStage = CGRect(x: 80, y: 60, width: 1760, height: 960)
check(PlaybackCapturePolicy.remoteDirection(target: actualTVForward, focused: actualTVPlay) == .right &&
      PlaybackCapturePolicy.remoteDirection(target: actualTVPlay, focused: actualTVForward) == .left,
      "Actual same-row controls use real horizontal remote navigation")
check(PlaybackCapturePolicy.remoteDirection(target: actualTVBack, focused: actualTVPlay) == .up &&
      PlaybackCapturePolicy.remoteDirection(target: actualTVPlay, focused: actualTVBack) == .down,
      "Actual header Back is above the toolbar; the old X-first Left rule cannot define this route")
check(PlaybackCapturePolicy.remoteDirection(target: actualTVPlay, focused: actualTVStage) == .left,
      "Hidden stage uses a real first reveal-only direction; it never activates Play")
let syntheticMenuClose = CGRect(x: 1200, y: 400, width: 48, height: 48)
let syntheticMenuRow = CGRect(x: 900, y: 470, width: 400, height: 60)
check(PlaybackCapturePolicy.remoteDirection(target: syntheticMenuRow, focused: syntheticMenuClose) == .down &&
      PlaybackCapturePolicy.remoteDirection(target: syntheticMenuClose, focused: syntheticMenuRow) == .up,
      "Synthetic wide menu rows use vertical remote input despite different horizontal centers")
check(PlaybackCapturePolicy.remoteDirection(target: actualTVPlay, focused: actualTVPlay) == nil,
      "Coincident unknown focus geometry does not invent a direction")
for invalid in [CGRect.zero, CGRect.null, CGRect.infinite,
                CGRect(x: CGFloat.nan, y: 0, width: 72, height: 72),
                CGRect(x: 0, y: CGFloat.infinity, width: 72, height: 72),
                CGRect(x: 0, y: 0, width: -1, height: 72)] {
    check(PlaybackCapturePolicy.remoteDirection(target: invalid, focused: actualTVPlay) == nil &&
          PlaybackCapturePolicy.remoteDirection(target: actualTVPlay, focused: invalid) == nil,
          "Invalid geometry remains a failure, not a coordinate guess")
}
func stopField(_ label: Any? = nil, value: Any? = nil, role: UInt = 48, identifier: String = "playback.status") -> PlaybackCapturePolicy.StopField {
    .init(identifier: identifier, role: role, label: label, value: value)
}
let stoppedMessage = "Playback stopped."
// Actual frame supplies the complete text; status.value Swift type remains unverified until runtime.
let pendingMessage = "Playback stopped locally; provider cleanup is pending."
let personalMessage = "Playback stopped locally; the final personal report is not confirmed. Check session state or retry the identical Stop request."
let serverMessage = "Playback stopped locally. Server cleanup is unconfirmed. Retry Stop uses the same final position and request."
let personalSuffix = " An earlier personal-report outcome remains unknown."
let stopMessages: [(String, PlaybackCapturePolicy.StopOutcome)] = [
    (stoppedMessage, .stopped), (pendingMessage, .providerPending),
    (personalMessage, .personalUnconfirmed), (serverMessage, .serverUnconfirmed)
]
for (message, outcome) in stopMessages {
    for role: UInt in [0, 1, 48, 999] {
        for field in [stopField(message, role: role), stopField(value: message, role: role),
                      stopField("", value: message, role: role), stopField(message, value: "", role: role),
                      stopField(message, value: message, role: role)] {
            let actual = PlaybackCapturePolicy.identifiedStopOutcome([field])
            check(actual == .init(outcome: outcome, earlierPersonalReportUnknown: false),
                  "Exactly one complete known String label or value supplies a role-independent local Stop outcome")
        }
    }
    if outcome != .serverUnconfirmed {
        let actual = PlaybackCapturePolicy.identifiedStopOutcome([stopField(value: message + personalSuffix)])
        check(actual == .init(outcome: outcome, earlierPersonalReportUnknown: true) &&
              actual!.cleanupEvidence.contains("earlier personal-report outcome remains unknown"),
              "The explicit accepted-response suffix retains earlier personal uncertainty")
        check(PlaybackCapturePolicy.identifiedStopOutcome([stopField(message, value: message + personalSuffix)]) == nil,
              "Conflicting earlier-report flags cannot collapse to the same base outcome")
    }
}
check(PlaybackCapturePolicy.stopObservation(serverMessage + personalSuffix) == nil,
      "Do not invent a server-error suffix combination absent from source")
check(PlaybackCapturePolicy.stopObservation(stoppedMessage)!.cleanupEvidence.contains("not independently confirmed"),
      "Even a no-warning visible stopped outcome does not independently confirm provider cleanup")
check(PlaybackCapturePolicy.stopObservation(pendingMessage)!.cleanupEvidence == "Stopped locally; visible status reports provider cleanup pending.",
      "Pending never becomes confirmed cleanup")
for unknown in ["Playback stopped l...", "Playback stopped l…", "Playback stopped…", "Playback stopped locally",
                pendingMessage + " ", " " + pendingMessage, pendingMessage + personalSuffix + personalSuffix,
                "stopped", "Preparing playback…", "https://private.invalid/?cap=secret", "", String(repeating: "x", count: 10000)] {
    check(PlaybackCapturePolicy.stopObservation(unknown) == nil, "Only complete exact source Stop messages decode")
    check(PlaybackCapturePolicy.identifiedStopOutcome([stopField(value: unknown)]) == nil, "Unknown text is not evidence")
    if !unknown.isEmpty {
        check(PlaybackCapturePolicy.identifiedStopOutcome([stopField(unknown, value: pendingMessage)]) == nil &&
              PlaybackCapturePolicy.identifiedStopOutcome([stopField(pendingMessage, value: unknown)]) == nil,
              "A truncated or unknown nonempty field cannot hide behind a recognized other field")
    }
}
for invalid: Any in [NSNumber(value: 1), NSNumber(value: true), NSNumber(value: Double.nan),
                     NSNumber(value: Double.infinity), ["Playback stopped."], ["url": "https://private.invalid"]] {
    check(PlaybackCapturePolicy.identifiedStopOutcome([stopField(value: invalid)]) == nil &&
          PlaybackCapturePolicy.identifiedStopOutcome([stopField(invalid, value: pendingMessage)]) == nil &&
          PlaybackCapturePolicy.identifiedStopOutcome([stopField(pendingMessage, value: invalid)]) == nil,
          "No numeric coercion or nonstring field can authorize a Stop capture")
}
check(PlaybackCapturePolicy.identifiedStopOutcome([]) == nil &&
      PlaybackCapturePolicy.identifiedStopOutcome([stopField()]) == nil &&
      PlaybackCapturePolicy.identifiedStopOutcome([stopField(value: pendingMessage, identifier: "other.status")]) == nil &&
      PlaybackCapturePolicy.identifiedStopOutcome([stopField(value: pendingMessage), stopField(value: pendingMessage)]) == nil &&
      PlaybackCapturePolicy.identifiedStopOutcome([stopField(pendingMessage, value: stoppedMessage)]) == nil &&
      PlaybackCapturePolicy.identifiedStopOutcome([stopField(personalMessage, value: serverMessage)]) == nil,
      "Absent, duplicate, wrong-identifier and conflicting Stop evidence fails closed")
let safeStopReceipt = PlaybackCapturePolicy.stopFieldsReceipt(count: 1, fields: [
    stopField("https://private.invalid/?cap=secret", value: pendingMessage)
], gatewayProgress: false)
let safeStopJSON = String(data: try! JSONSerialization.data(withJSONObject: safeStopReceipt), encoding: .utf8)!
check(!safeStopJSON.contains("private.invalid") && !safeStopJSON.contains("cap=secret") &&
      !safeStopJSON.contains(pendingMessage) && safeStopReceipt["observation"] is NSNull,
      "Safe receipts contain only fixed field types/outcomes, never arbitrary messages or URLs")
let safeStopFields = safeStopReceipt["fields"] as! [[String: Any]]
check(safeStopFields[0]["label_type"] as? String == "string" &&
      safeStopFields[0]["value_type"] as? String == "string", "Actual field classification remains explicit")
let duplicateStopReceipt = PlaybackCapturePolicy.stopFieldsReceipt(count: 2, fields: [
    stopField(value: pendingMessage), stopField(value: stoppedMessage)
], gatewayProgress: true)
check(duplicateStopReceipt["observation"] is NSNull &&
      (duplicateStopReceipt["fields"] as! [[String: Any]]).count == 1,
      "Duplicate receipts remain bounded and cannot claim an observed outcome")
let namespaceUUID = "5C8BB2F7-7F76-44E9-976F-2CDE2EB2599D"
let actualAbsoluteRunID = "absolute-seek-" + namespaceUUID
check(("playback-showcase-" + actualAbsoluteRunID).utf8.count == 68, "Actual old focused namespace exceeds the product limit")
check(PlaybackCapturePolicy.settingsNamespace(runID: actualAbsoluteRunID) == "showcase-" + actualAbsoluteRunID &&
      PlaybackCapturePolicy.settingsNamespace(runID: actualAbsoluteRunID)!.utf8.count == 59,
      "New capture-only prefix preserves the complete actual run ID in 59 bytes")
for (prefix, bytes) in [("", 45), ("focused-", 53), ("preparation-", 57), ("absolute-seek-", 59)] {
    let runID = prefix + namespaceUUID
    let namespace = PlaybackCapturePolicy.settingsNamespace(runID: runID)
    check(namespace == "showcase-" + runID && namespace!.utf8.count == bytes, "All current Apple runner prefixes preserve full unique run IDs")
}
let ascii64 = String(repeating: "a", count: 55)
let ascii65 = String(repeating: "a", count: 56)
let unicode64 = "a" + String(repeating: "é", count: 27)
let unicode66 = "a" + String(repeating: "é", count: 28)
for runID in [ascii64, unicode64] {
    let namespace = PlaybackCapturePolicy.settingsNamespace(runID: runID)
    check(namespace == "showcase-" + runID && namespace!.utf8.count == 64, "Exactly 64 UTF-8 bytes remains valid without truncation")
}
check(("showcase-" + ascii65).utf8.count == 65 && ("showcase-" + unicode66).utf8.count == 66, "Invalid fixtures exercise byte counts, not grapheme counts")
for runID in ["", ascii65, unicode66, String(repeating: "a", count: 100000), String(repeating: "é", count: 100000)] {
    check(PlaybackCapturePolicy.settingsNamespace(runID: runID) == nil, "Empty or oversized selected namespaces fail the pre-app unwrap barrier")
}
func encodedCorrelation(_ root: [String: Any]) -> String { String(data: try! JSONSerialization.data(withJSONObject: root), encoding: .utf8)! }
let correlationRecords: [[String: Any]] = [
    ["operation": 1, "stage": "slider-ended", "slider_received_ms": 90000.75, "logical_target_ms": 90000, "transport_offset_ms": 10000],
    ["operation": 1, "stage": "seek-dispatch", "logical_target_ms": 90000, "transport_offset_ms": 10000, "media_requested_ms": 80000],
    ["operation": 1, "stage": "seek-completion", "logical_target_ms": 90000, "transport_offset_ms": 10000, "media_requested_ms": 80000,
     "media_actual_ms": 87000, "logical_actual_ms": 97000]
]
let correlation: [String: Any] = ["schema_version": 1, "records": correlationRecords]
check(PlaybackCapturePolicy.seekCorrelationComplete(PlaybackCapturePolicy.seekCorrelationReceipt(encodedCorrelation(correlation))),
      "Safe three-stage receipt identifies one operation without claiming an exact target or responsible layer")
for count in [1, 2] {
    let partial = PlaybackCapturePolicy.seekCorrelationReceipt(encodedCorrelation(["schema_version": 1, "records": Array(correlationRecords.prefix(count))]))
    check(partial != nil && !PlaybackCapturePolicy.seekCorrelationComplete(partial), "Partial correlation is safe evidence, not a completed trace")
}
var updateRecords = Array(correlationRecords.prefix(2))
updateRecords[1] = ["operation": 1, "stage": "timeline-update", "logical_target_ms": 5000, "transport_offset_ms": 10000]
let timelineUpdate = PlaybackCapturePolicy.seekCorrelationReceipt(encodedCorrelation(["schema_version": 1, "records": updateRecords]))
check(timelineUpdate != nil && !PlaybackCapturePolicy.seekCorrelationComplete(timelineUpdate), "Gateway timeline branch does not pretend a direct media completion")
for invalid: Any? in [nil, "not-json", "ScreenroomPlaybackPreparation phase=done", encodedCorrelation(["schema_version": true, "records": correlationRecords]),
                     encodedCorrelation(["schema_version": 2, "records": correlationRecords]),
                     encodedCorrelation(["schema_version": 1, "records": correlationRecords, "url": "https://private.invalid/?cap=secret"]),
                     encodedCorrelation(correlation).replacingOccurrences(of: "80000", with: "1e999")] {
    check(PlaybackCapturePolicy.seekCorrelationReceipt(invalid) == nil, "Unknown, unsafe, nonfinite, and foreign schema evidence must be rejected")
}
for (key, value): (String, Any) in [("operation", 0), ("operation", 9), ("operation", true), ("logical_target_ms", -1),
                                   ("transport_offset_ms", 922337203685478), ("slider_received_ms", true),
                                   ("stage", "private-error"), ("media_url", "https://private.invalid/?cap=secret")] {
    var records = correlationRecords
    records[0][key] = value
    check(PlaybackCapturePolicy.seekCorrelationReceipt(encodedCorrelation(["schema_version": 1, "records": records])) == nil,
          "Stage and finite numeric allowlists reject untrusted fields")
}
check(PlaybackCapturePolicy.seekCorrelationReceipt(encodedCorrelation(["schema_version": 1, "records": correlationRecords + [correlationRecords[2]]])) == nil,
      "A repeated completion is not a second valid observation")
var secondOperation = correlationRecords
for index in secondOperation.indices { secondOperation[index]["operation"] = 2 }
let multiple = PlaybackCapturePolicy.seekCorrelationReceipt(encodedCorrelation(["schema_version": 1, "records": correlationRecords + secondOperation]))
check(multiple != nil && !PlaybackCapturePolicy.seekCorrelationComplete(multiple), "Multiple actual operations remain visible and cannot satisfy one-gesture proof")
let originalNearTwenty = PlaybackCapturePolicy.clockPosition("18:04, 88:40")!
let exactTwenty = PlaybackCapturePolicy.clockPosition("17:44 / 88:40")!
func clockField(_ label: Any? = nil, value: Any? = nil, role: UInt = 0, identifier: String = "playback.time") -> PlaybackCapturePolicy.ClockField {
    .init(identifier: identifier, role: role, label: label, value: value)
}
// Roles and Swift field types are synthetic; latest actual AX supplies value TEXT, not a Swift type receipt.
let clockFixtures: [(role: String, identifier: String, label: String)] = [
    ("Group", "playback.time", "16:54, 88:40"),
    ("StaticText", "playback.time", "16:54 / 88:40"),
    ("Other", "playback.time", "16:54 88:40")
]
for fixture in clockFixtures {
    let actual = PlaybackCapturePolicy.identifiedClockPosition([clockField(fixture.label, identifier: fixture.identifier)])
    check(actual == .init(elapsed: 1014, duration: 5320), "An exact identified clock is role-independent: " + fixture.role)
}
check(PlaybackCapturePolicy.identifiedClockPosition([clockField("79:33, 88:40")]) == .init(elapsed: 4773, duration: 5320),
      "Current iPhone displayed clock values retain their explicit seconds and 5320-second duration")
check(PlaybackCapturePolicy.identifiedClockPosition([clockField("16:49, 88:40")]) == .init(elapsed: 1009, duration: 5320),
      "The actual iPhone post-seek display is not its ideal 20-percent request")
let rejectedIdentifiedClocks: [[(identifier: String, label: String)]] = [
    [], [("wrong.time", "16:54 / 88:40")],
    [("playback.time", "16:54 / 88:40"), ("playback.time", "16:54 / 88:40")],
    [("playback.time", "16:54 / 88:40"), ("playback.time", "17:44 / 88:40")],
    [("playback.time", "")], [("playback.time", "16:54")], [("playback.time", "88:40")],
    [("playback.time", "1014487")], [("playback.time", "1014 / 5320")],
    [("playback.time", "20% / 88:40")], [("playback.time", "-1:00 / 88:40")],
    [("playback.time", "NaN / 88:40")], [("playback.time", "Infinity / 88:40")],
    [("playback.time", "16:54 / 0:00")], [("playback.time", "88:41 / 88:40")],
    [("playback.time", "16:60 / 88:40")], [("playback.time", "16:54 / —")],
    [("playback.time", "16:54 / 88:40 extra")]
]
for fixture in rejectedIdentifiedClocks {
    check(PlaybackCapturePolicy.identifiedClockPosition(fixture.map { clockField($0.label, identifier: $0.identifier) }) == nil,
          "Missing, mismatched, duplicate, ambiguous, invalid and unknown-unit clocks never authorize input")
}
let latestMacAXValueText = "17:11, 88:40"
let latestMacClock = PlaybackCapturePolicy.ClockPosition(elapsed: 1031, duration: 5320)
for role: UInt in [0, 1, 48, 999] {
    for label: Any? in [nil, "", "Playback time", latestMacAXValueText] {
        check(PlaybackCapturePolicy.identifiedClockPosition([clockField(label, value: latestMacAXValueText, role: role)]) == latestMacClock,
              "Actual Mac AX value TEXT parses as explicit seconds with synthetic String type and role-independent fixtures")
    }
}
check(PlaybackCapturePolicy.identifiedClockPosition([clockField("17:11 / 88:40", value: latestMacAXValueText)]) == latestMacClock,
      "Two valid fields must match elapsed/duration exactly, regardless of delimiter")
check(PlaybackCapturePolicy.identifiedClockPosition([clockField(latestMacAXValueText, value: "Playback time")]) == latestMacClock,
      "One explicit label clock remains valid; an unparsed String field supplies no normalized numeric evidence")
for field in [
    clockField("17:12 / 88:40", value: latestMacAXValueText),
    clockField("17:11 / 88:41", value: latestMacAXValueText),
    clockField(latestMacAXValueText, value: NSNumber(value: 1031000)),
    clockField(value: NSNumber(value: 1031000)), clockField(value: NSNumber(value: Double.nan)),
    clockField(value: NSNumber(value: Double.infinity)), clockField(value: NSNumber(value: true)),
    clockField(NSNumber(value: 1031), value: latestMacAXValueText),
    clockField(value: ["private": "secret"]), clockField(),
    clockField("", value: "20%"), clockField("", value: "1031000"),
    clockField("", value: "1031 / 5320"), clockField("", value: "NaN / 88:40"),
    clockField("", value: "17:11 / —"), clockField("", value: "17:11 / 0:00"),
    clockField("", value: "88:41 / 88:40"), clockField("", value: "17:11 / 88:40 extra"),
    clockField(value: String(repeating: "9", count: 100000)),
    clockField(value: latestMacAXValueText, identifier: "wrong.time")
] {
    check(PlaybackCapturePolicy.identifiedClockPosition([field]) == nil,
          "Conflicts, nonstring fields, unknown units, malformed clocks and wrong identifiers reject input")
}
let actualTextSyntheticType = clockField("", value: latestMacAXValueText, role: 48)
check(PlaybackCapturePolicy.identifiedClockPosition([actualTextSyntheticType, actualTextSyntheticType]) == nil,
      "Even identical duplicate value clocks reject input")
check(PlaybackCapturePolicy.identifiedClockPosition([]) == nil, "Missing exact identifier rejects input")
for field in [actualTextSyntheticType, clockField("private-label-secret", value: "private-value-secret"),
              clockField(value: NSNumber(value: Double.nan)), clockField(value: ["secret": "private-data"])] {
    let receipt = PlaybackCapturePolicy.clockFieldsReceipt(count: 1, fields: [field])
    let json = String(data: try! JSONSerialization.data(withJSONObject: receipt), encoding: .utf8)!
    check(!json.contains("private-") && !json.contains("secret") && !json.contains("NaN"),
          "Clock receipts contain fixed types and parsed finite clock numbers, never raw arbitrary fields")
}
let fieldReceipt = PlaybackCapturePolicy.clockFieldsReceipt(count: 1, fields: [actualTextSyntheticType])
let receiptFields = fieldReceipt["fields"] as! [[String: Any]]
check(receiptFields.count == 1 && receiptFields[0]["label_type"] as? String == "string" &&
      receiptFields[0]["value_type"] as? String == "string" && receiptFields[0]["role"] as? UInt == 48,
      "Synthetic typed fixture proves safe role/label-type/value-type receipt shape, not runtime Swift String evidence")
check((fieldReceipt["clock"] as? [String: Double]) == ["elapsed_seconds": 1031, "duration_seconds": 5320],
      "Safe receipt retains explicit actual clock numeric units")
for (value, type) in [(nil as Any?, "missing"), (NSNumber(value: 1) as Any?, "number"), (["private": "secret"] as Any?, "other")] {
    let typed = PlaybackCapturePolicy.clockFieldsReceipt(count: 1, fields: [clockField(value: value)])
    check((typed["fields"] as! [[String: Any]])[0]["value_type"] as? String == type,
          "Safe receipt distinguishes absent, numeric and unsupported actual value types without reflection or coercion")
}
let freshAfter = PlaybackCapturePolicy.identifiedClockPosition([clockField("", value: "35:28 / 88:40")])
check(PlaybackCapturePolicy.absoluteSeekSettled(before: latestMacClock, after: freshAfter,
    playEnabled: true, playLabel: "Play", gatewayProgress: false),
    "A fresh actual String observation retains the strict changed comparable paused timeline proof")
check(!PlaybackCapturePolicy.absoluteSeekSettled(before: latestMacClock,
    after: PlaybackCapturePolicy.identifiedClockPosition([clockField("", value: latestMacAXValueText)]),
    playEnabled: true, playLabel: "Play", gatewayProgress: false), "A stale unchanged value cannot prove seek settlement")
let duplicateReceipt = PlaybackCapturePolicy.clockFieldsReceipt(count: 2, fields: [actualTextSyntheticType, actualTextSyntheticType])
check(duplicateReceipt["match_count"] as? Int == 2 && duplicateReceipt["clock"] is NSNull &&
      (duplicateReceipt["fields"] as! [[String: Any]]).count <= 1, "Duplicate receipt stays bounded and cannot authorize a clock")
check(PlaybackCapturePolicy.uniqueClockPosition(["18:04 / 88:40"]) == originalNearTwenty, "One exact clock label is valid evidence")
for labels in [[], ["18:04 / 88:40", "18:04 / 88:40"], ["18:04 / 88:40", "35:28 / 88:40"], ["1084 / 5320"]] {
    check(PlaybackCapturePolicy.uniqueClockPosition(labels) == nil, "Absent, duplicate, ambiguous, or unknown-unit labels prohibit normalization")
}
let actualFocusedBefore = PlaybackCapturePolicy.clockPosition("21:02 / 88:40")!
let actualFocusedAfter = PlaybackCapturePolicy.clockPosition("40:34 / 88:40")!
check(PlaybackCapturePolicy.absoluteSeekPlan(actualFocusedBefore)?.normalizedTarget == 0.4, "Actual completed focused run selected the separated 40-percent request")
check(PlaybackCapturePolicy.absoluteSeekPlan(actualFocusedBefore)?.targetSeconds == 2128 && actualFocusedAfter.elapsed == 2434,
      "Actual focused seek changed the timeline but did not land at its ideal target; never conflate these claims")
check(PlaybackCapturePolicy.absoluteSeekSettled(before: actualFocusedBefore, after: actualFocusedAfter,
    playEnabled: true, playLabel: "Play", gatewayProgress: false), "Actual focused receipt satisfies changed settled pause, not exact landing")
check(originalNearTwenty.elapsed == 1084 && originalNearTwenty.duration == 5320, "Actual failed-frame clock must retain its units and values")
for clock in [originalNearTwenty, exactTwenty, PlaybackCapturePolicy.ClockPosition(elapsed: 1000, duration: 5320)] {
    check(PlaybackCapturePolicy.absoluteSeekPlan(clock)?.normalizedTarget == 0.4, "Near 20 percent selects one separated 40-percent target")
}
for clock in [PlaybackCapturePolicy.ClockPosition(elapsed: 2128, duration: 5320),
              PlaybackCapturePolicy.ClockPosition(elapsed: 0, duration: 5320)] {
    check(PlaybackCapturePolicy.absoluteSeekPlan(clock)?.normalizedTarget == 0.2, "40 percent and zero elapsed select 20 percent")
}
for elapsed in stride(from: 0.0, through: 5320.0, by: 5.0) {
    let plan = PlaybackCapturePolicy.absoluteSeekPlan(.init(elapsed: elapsed, duration: 5320))!
    check(abs(plan.normalizedTarget - elapsed / 5320) >= 0.1, "Every valid clock must select at least 10 percent separation")
    check(abs(plan.targetSeconds - elapsed) >= 2, "Clock precision requires at least two seconds separation")
}
for invalid: PlaybackCapturePolicy.ClockPosition? in [
    nil, .init(elapsed: 0, duration: 0), .init(elapsed: 0, duration: .infinity),
    .init(elapsed: .nan, duration: 5320), .init(elapsed: -1, duration: 5320),
    .init(elapsed: 5321, duration: 5320), .init(elapsed: 0, duration: -1),
    .init(elapsed: 0, duration: 1)
] {
    check(PlaybackCapturePolicy.absoluteSeekPlan(invalid) == nil, "Unknown, nonfinite, invalid, and insufficient-resolution clocks prohibit slider input")
}
check(PlaybackCapturePolicy.clockPosition(elapsed: .native(1084), duration: .seconds(5320)) == nil, "Native values have no proven clock unit")
check(PlaybackCapturePolicy.clockPosition(elapsed: .seconds(1084), duration: .native(5320)) == nil, "Native duration must not authorize normalization")
check(PlaybackCapturePolicy.clockPosition(elapsed: .percent(20), duration: .seconds(5320)) == nil, "Percent value is not elapsed seconds")
for label: String? in [nil, "", "1084 5320", "20% / 88:40", "18:04 / —", "NaN / 88:40", "18:60 / 88:40", "18:04 / 0:00", "18:04 88:40 secret"] {
    check(PlaybackCapturePolicy.clockPosition(label) == nil, "Only an exact actual elapsed/duration clock pair authorizes normalization")
}
let seekAfter = PlaybackCapturePolicy.ClockPosition(elapsed: 2128, duration: 5320)
check(PlaybackCapturePolicy.absoluteSeekSettled(before: originalNearTwenty, after: seekAfter, playEnabled: true, playLabel: "Play", gatewayProgress: false),
      "Changed comparable clock plus enabled Play and no gateway progress proves settled pause")
for (after, enabled, label, progress) in [
    (originalNearTwenty, true, "Play", false), (seekAfter, false, "Play", false),
    (seekAfter, true, "Pause", false), (seekAfter, true, "Play", true),
    (PlaybackCapturePolicy.ClockPosition(elapsed: 2128, duration: 5321), true, "Play", false)
] {
    check(!PlaybackCapturePolicy.absoluteSeekSettled(before: originalNearTwenty, after: after, playEnabled: enabled, playLabel: label, gatewayProgress: progress),
          "Unchanged, disabled, nonpaused, busy, and changed-duration states never pass")
}
let secretSeekText = "private-body https://private.invalid/?cap=secret profile=private item=private"
let redactedSeek = PlaybackCapturePolicy.absoluteSeekReceipt(
    clock: nil, sliderValue: secretSeekText, requested: nil, playExists: true, playEnabled: false,
    playLabel: secretSeekText, sliderEnabled: false, gatewayProgress: true, status: secretSeekText, preparation: secretSeekText)
let redactedSeekText = String(data: try JSONSerialization.data(withJSONObject: redactedSeek, options: [.sortedKeys]), encoding: .utf8)!
check(redactedSeek["clock"] is NSNull && redactedSeek["slider_clock_seconds"] is NSNull, "Unknown clock evidence must remain unknown")
check(PlaybackCapturePolicy.seekStatus("Playback stopped. " + secretSeekText) == "redacted-unrecognized",
      "A recognized status prefix must not admit private suffix text or falsely claim a stopped state")
for forbidden in ["private-body", "https", "cap=", "profile=", "item="] {
    check(!redactedSeekText.contains(forbidden), "Receipts must not reflect arbitrary status, AX, or preparation text")
}
let seekPlan = PlaybackCapturePolicy.absoluteSeekPlan(originalNearTwenty)!
let invalidSeek = PlaybackCapturePolicy.absoluteSeekReceipt(clock: .init(elapsed: .nan, duration: .infinity),
    sliderValue: Double.nan, requested: .init(before: originalNearTwenty, normalizedTarget: .infinity),
    playExists: false, playEnabled: false, playLabel: nil, sliderEnabled: false, gatewayProgress: true, status: nil, preparation: nil)
check(invalidSeek["clock"] is NSNull && invalidSeek["request"] is NSNull, "Invalid numeric clock or request data must not become receipt evidence")
check(JSONSerialization.isValidJSONObject(invalidSeek), "Invalid numeric inputs must still produce valid safe JSON")
let safeSeek = PlaybackCapturePolicy.absoluteSeekReceipt(clock: originalNearTwenty, sliderValue: "18:04", requested: seekPlan,
    playExists: true, playEnabled: true, playLabel: "Play", sliderEnabled: true, gatewayProgress: false,
    status: nil, preparation: "ScreenroomPlaybackPreparation phase=done stage=done code=none blocked=false closed=false session=true")
check(safeSeek["slider_clock_seconds"] as? Double == 1084, "A clock-formatted slider value retains seconds without numeric-unit inference")
check((safeSeek["request"] as? [String: Double])?["target_seconds"] == 2128, "Retain the exact selected normalized and clock targets")
for native: Any in [NSNumber(value: 1084), "1084", "20%", Double.nan] {
    let receipt = PlaybackCapturePolicy.absoluteSeekReceipt(clock: nil, sliderValue: native, requested: nil,
        playExists: false, playEnabled: false, playLabel: nil, sliderEnabled: false, gatewayProgress: false, status: nil, preparation: nil)
    check(receipt["slider_clock_seconds"] is NSNull && receipt["clock"] is NSNull, "Native, percent, or nonfinite values must not become clock seconds")
}
let safePreparation = "ScreenroomPlaybackPreparation phase=failure stage=create code=upstream_unavailable blocked=true closed=false session=false"
check(PlaybackCapturePolicy.timelineAdvanced(.seconds(1), .seconds(2)), "Actual playback must advance")
check(!PlaybackCapturePolicy.timelineAdvanced(.seconds(2), .seconds(1)), "Backward movement is not playback advancement")
check(!PlaybackCapturePolicy.timelineAdvanced(.seconds(1), .percent(2)), "Incomparable units do not prove advancement")
check(PlaybackCapturePolicy.preparationReceipt(safePreparation)?["code"] == "upstream_unavailable", "Read the actual allowlisted preparation code")
for code in ["playback_authorization_unavailable", "playback_conflict", "operation_conflict", "upstream_unavailable", "untyped", "unknown"] {
    check(PlaybackCapturePolicy.preparationReceipt(safePreparation.replacingOccurrences(of: "upstream_unavailable", with: code))?["code"] == code,
          "Each known candidate must retain its own code without inferring the old failure cause")
}
for invalid in [
    safePreparation + " url=https://private.invalid/",
    safePreparation.replacingOccurrences(of: "code=upstream_unavailable", with: "code=private_body"),
    safePreparation.replacingOccurrences(of: "stage=create", with: "stage=private_body"),
    safePreparation.replacingOccurrences(of: "blocked=true", with: "blocked=1"),
    safePreparation.replacingOccurrences(of: "blocked=true", with: "code=upstream_unavailable"),
    safePreparation.replacingOccurrences(of: "phase=failure", with: "phase=done"),
    "private error body", ""
] {
    check(PlaybackCapturePolicy.preparationReceipt(invalid) == nil, "Reject arbitrary, duplicate, or inconsistent diagnostic metadata")
}
check(PlaybackCapturePolicy.preparationReceipt("ScreenroomPlaybackPreparation phase=done stage=done code=none blocked=false closed=false session=true")?["phase"] == "done",
      "A successful current preparation must remain distinct from an earlier error")
// Actual failed run 20261116T033114Z-ldBnWE, test.log1070: Exit fullscreen before any fullscreen request.
let alreadyFull = PlaybackCapturePolicy.fullscreenPlan(label: "Exit fullscreen", enabled: true)!
check(alreadyFull.map(\\.toggle) == [false, true], "Already-fullscreen state needs exactly one exit action")
check(alreadyFull.map(\\.entered) == [true, false], "Capture both current fullscreen and final windowed states")
check(alreadyFull[0].observation.contains("before any fullscreen request") && alreadyFull[0].observation.contains("not an entry transition"),
      "A current-state capture must not claim an entry transition")
let initiallyWindowed = PlaybackCapturePolicy.fullscreenPlan(label: "Fullscreen", enabled: true)!
check(initiallyWindowed.map(\\.toggle) == [true, true], "Windowed state needs one enter and one exit action")
check(initiallyWindowed[0].observation.contains("after one enter toggle"), "Entry claim requires the enter action")
for label: String? in [nil, "", "Unknown", "Enter Full Screen"] {
    check(PlaybackCapturePolicy.fullscreenPlan(label: label, enabled: true) == nil, "Unknown or missing initial state must fail")
}
check(PlaybackCapturePolicy.fullscreenPlan(label: "Fullscreen", enabled: false) == nil, "Disabled initial state must fail")
check(PlaybackCapturePolicy.fullscreenPlan(label: "Exit fullscreen", enabled: false) == nil, "Disabled current fullscreen must fail")
let ambiguous = [(identifier: "playback.fullscreen", label: "Exit fullscreen", hittable: true),
                 (identifier: "playback.fullscreen", label: "Exit fullscreen", hittable: true)]
let ambiguousIndex = PlaybackCapturePolicy.fullscreenChoice(ambiguous, menu: nil)
check(PlaybackCapturePolicy.fullscreenPlan(label: ambiguousIndex.map { ambiguous[$0].label }, enabled: true) == nil,
      "Ambiguous real controls cannot authorize a fullscreen plan")
// Actual failed run session log line 87034: app-owned direct control before the successful native toggle.
let beforeFullscreen = [(identifier: "playback.fullscreen", label: "Fullscreen", hittable: true)]
check(PlaybackCapturePolicy.fullscreenChoice(beforeFullscreen, menu: nil) == 0, "Select the exact app-owned direct control")
check(PlaybackCapturePolicy.fullscreenChoice([], menu: nil) == nil, "A transient absent snapshot requires more observation, never menu navigation")
// Actual failed run line 88929: the old helper opened playback.menu.tracks during state observation.
check(PlaybackCapturePolicy.fullscreenChoice([], menu: "playback.menu.tracks") == nil, "Wrong tracks menu supplies no fullscreen evidence")
let generic = [(identifier: "", label: "Exit fullscreen", hittable: true)]
check(PlaybackCapturePolicy.fullscreenChoice(generic, menu: nil) == nil, "Generic labels outside the explicit session menu cannot prove app state")
check(PlaybackCapturePolicy.fullscreenChoice(generic, menu: "playback.menu.tracks") == nil, "Wrong-menu generic labels cannot masquerade as fullscreen")
check(PlaybackCapturePolicy.fullscreenChoice(generic, menu: "playback.menu.session") == 0, "A unique explicit session-menu control is supported")
check(PlaybackCapturePolicy.fullscreenChoice(beforeFullscreen + beforeFullscreen, menu: nil) == nil, "Ambiguous app-owned controls must not toggle")
check(PlaybackCapturePolicy.fullscreenChoice([(identifier: "playback.fullscreen", label: "Fullscreen", hittable: false)], menu: nil) == nil,
      "A stale noninteractive window control cannot authorize a toggle")
let failedAXReadouts: [(String, Any?)] = [${extractedReadouts.map(() => '("", nil)').join(',')}]
check(PlaybackCapturePolicy.uniqueVolumeReadout(failedAXReadouts) == nil, "Actual failed-run sparse AX evidence cannot authorize a volume capture")
// Synthetic label/value surface variants for the new identifier, not claims about the failed snapshot.
for target in [0, 50, 100] {
    let text = "Volume \\(target)%"
    check(PlaybackCapturePolicy.uniqueVolumeReadout([(text, nil)]) == target, "Label-backed readout")
    check(PlaybackCapturePolicy.uniqueVolumeReadout([("", text)]) == target, "Value-backed native readout")
    check(PlaybackCapturePolicy.uniqueVolumeReadout([(text, text)]) == target, "Consistent dual AX surface")
}
check(PlaybackCapturePolicy.uniqueVolumeReadout([("Volume 0%", "Volume 100%")]) == nil, "Conflicting readout evidence must fail")
check(PlaybackCapturePolicy.uniqueVolumeReadout([("Volume 50%", nil), ("Volume 50%", nil)]) == nil, "Duplicate readout identity must fail")
check(PlaybackCapturePolicy.uniqueVolumeReadout([("Volume", NSNumber(value: 100))]) == nil, "Untyped numeric value cannot invent volume units")
check(PlaybackCapturePolicy.uniqueVolumeReadout([("", nil)]) == nil, "Absent AX text must fail")
for enabled in [false, true] {
    for visible in [false, true] {
        for busy in [false, true] {
            check(PlaybackCapturePolicy.volumeSettled(actual: 50, target: 50, previous: 40, controlsEnabled: enabled,
                                                      menuVisible: visible, transportBusy: busy) == (enabled && visible && !busy),
                  "Volume captures require settled enabled controls, visible menu and actual target")
        }
    }
}
check(!PlaybackCapturePolicy.volumeSettled(actual: nil, target: 50, previous: 40, controlsEnabled: true, menuVisible: true, transportBusy: false),
      "No missing readout can satisfy capture evidence")
check(!PlaybackCapturePolicy.volumeSettled(actual: 40, target: nil, previous: 40, controlsEnabled: true, menuVisible: true, transportBusy: false),
      "A click without changed volume cannot satisfy evidence")
check(!PlaybackCapturePolicy.volumeSettled(actual: 40, target: 50, previous: nil, controlsEnabled: true, menuVisible: true, transportBusy: false),
      "A wrong endpoint cannot satisfy evidence")
check(PlaybackCapturePolicy.timelineValue(NSNumber(value: 0.25)) == .native(0.25), "macOS native NSNumber slider values")
check(PlaybackCapturePolicy.timelineValue(NSNumber(value: 0)) == .native(0), "Zero is readable position evidence")
check(PlaybackCapturePolicy.timelineValue("0.25") == .native(0.25), "Numeric AX strings retain numeric units")
check(PlaybackCapturePolicy.timelineValue("25%") == .percent(25), "Percentage AX strings")
check(PlaybackCapturePolicy.timelineValue("1:02:03") == .seconds(3723), "Readable time AX values")
check(PlaybackCapturePolicy.elapsedTime("0:05, 1:34:50") == .seconds(5), "Combined elapsed/duration footer")
check(PlaybackCapturePolicy.elapsedTime("1:02 / 1:34:50") == .seconds(62), "Readable time pair separator")
check(PlaybackCapturePolicy.elapsedTime("1:02 1:34:50") == .seconds(62), "Whitespace-combined footer")
for value: Any? in [nil, NSNumber(value: true), NSNumber(value: Double.nan), NSNumber(value: Double.infinity),
                    NSNumber(value: -1), "", "unknown", "1:99", "10%%", "101%"] {
    check(PlaybackCapturePolicy.timelineValue(value) == nil, "Missing or invalid actual position must fail, never count as movement")
}
check(PlaybackCapturePolicy.elapsedTime("Movie 1922") == nil, "Unrelated labels are not timeline evidence")
check(!PlaybackCapturePolicy.timelineChanged(.native(0.25), .native(0.25)), "A tap without a changed native value proves nothing")
check(PlaybackCapturePolicy.timelineChanged(.native(0.25), .native(0.3)), "Changed native values prove actual movement")
check(PlaybackCapturePolicy.timelineChanged(.seconds(5), .seconds(35)), "Changed elapsed time proves movement")
check(!PlaybackCapturePolicy.timelineChanged(.native(0.25), .seconds(35)), "Different AX units cannot manufacture movement")
for owned in [false, true] {
    for attempted in [false, true] {
        check(PlaybackCapturePolicy.normalExitAllowed(owned: owned, attempted: attempted) == (owned && !attempted),
              "Teardown exits only owned sessions and never repeats an uncertain Back request")
    }
}
check(!PlaybackCapturePolicy.trackNeedsSelection(selected: true), "Already-selected choices must not send requests")
check(PlaybackCapturePolicy.trackNeedsSelection(selected: false), "Changed choices require a normal UI request")
for selected in [false, true] {
    for enabled in [false, true] {
        for busy in [false, true] {
            let actual = PlaybackCapturePolicy.trackSettled(selected: selected, enabled: enabled, transportBusy: busy)
            check(actual == (selected && enabled && !busy), "Stale selection or disabled/busy controls cannot settle")
        }
    }
}
let snapshots = [(true, false, true), (true, false, false), (false, true, false), (true, true, false)]
let firstSettled = snapshots.firstIndex {
    PlaybackCapturePolicy.trackSettled(selected: $0.0, enabled: $0.1, transportBusy: $0.2)
}
check(firstSettled == 3, "Only the final enabled, transport-settled requested selection is capture evidence")
for invalid in ["Volume -1%", "Volume 101%", "Volume unknown%", "50%", "Volume 50"] {
    check(PlaybackCapturePolicy.volumePercent(invalid) == nil, "Reject malformed/out-of-range UI evidence")
}
var current = 100
var counts: [Int] = []
for target in [0, 50, 100] {
    var presses = 0
    while let action = PlaybackCapturePolicy.volumeAction(current: current, target: target) {
        check(presses < 12, "Bounded normal volume navigation")
        current = max(0, min(100, current + (action == "Louder" ? 10 : -10)))
        check(PlaybackCapturePolicy.volumePercent("Volume \\(current)%") == current, "Parse actual displayed percentage")
        presses += 1
    }
    check(current == target, "Reach each explicit volume state")
    check(PlaybackCapturePolicy.volumeAction(current: current, target: target) == nil, "Never click after reaching target")
    counts.append(presses)
}
check(counts == [10, 5, 5], "Capture minimum, midpoint and maximum rather than one-step samples")
print("Playback capture policy: strict complete Stop status fields/outcomes/redaction, absolute-seek clock/separation/settlement/redaction, pure fullscreen locator, timeline evidence, owned single-exit, track settlement, volume 0/50/100 passed")
`);
    const result = spawnSync('swift', [file], { encoding: 'utf8', timeout: 120000 });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    process.stdout.write(result.stdout);
} finally {
    rmSync(directory, { recursive: true });
}
