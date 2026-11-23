#!/usr/bin/env python3
import copy
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zlib

sys = os.sys
sys.dont_write_bytecode = True
SCRIPT = Path(__file__).with_name("playback-showcase.py")
spec = importlib.util.spec_from_file_location("showcase", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def image():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\0\0\0\0\xff")) + chunk(b"IEND", b"")


def manifest(slug):
    keys = ["playing", "paused", "idle", "seek-before", "seek-after", "volume-0", "volume-50", "volume-100", "mute", "tracks", "subtitles-off", "settings", "fullscreen", "loading", "error", "stop", "cleanup"]
    states = [{"id": key, "label": key, "status": "blocked", "viewport": "unknown", "reason": "Natural media failure", "observation": "Not captured"} for key in keys]
    states[keys.index("error")] = {"id": "error", "label": "Media error", "status": "captured", "viewport": "1x1", "observation": "Actual visible media error", "image": f"{slug}/error.png"}
    return {"schema_version": 1, "journey": "playback", "backend_instance_id": "test-instance", "item": {"id": "explicit-item", "title": "Nosferatu", "year": 1922}, "platforms": [{"slug": slug, "runtime": "isolated test fixture", "outcome": "blocked", "states": states}]}


class ContractTests(unittest.TestCase):
    def setUp(self):
        os.environ.update(SCREENROOM_BACKEND_INSTANCE_ID="test-instance", SCREENROOM_SHOWCASE_ITEM_ID="explicit-item", SCREENROOM_SHOWCASE_TITLE="Nosferatu", SCREENROOM_SHOWCASE_YEAR="1922")

    def test_actual_gateway_media_item_with_pinned_media3_classifier(self):
        android = SCRIPT.parent.parent
        env = dict(os.environ, JAVA_HOME="/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home",
                   ANDROID_HOME="/Users/rfhold/Library/Android/sdk")
        subprocess.run([str(android / "gradlew"), "--quiet", "--no-daemon", "-x:ui:prepareClientCore",
                        ":ui:testDebugUnitTest", "--tests", "org.screenroom.ui.GatewayMediaItemTest"],
                       cwd=android, env=env, check=True, timeout=120)
        player = (android / "e2e/org/screenroom/e2e/PlayerShowcase.kt").read_text()
        focused = player[player.index("if (playbackOnly) {"):player.index('if (blocked) row("error"')]
        self.assertIn("after > before", focused)
        self.assertIn('onNodeWithContentDescription("Pause").assertIsEnabled()', focused)
        self.assertIn("stopFocusedPlayback()", focused)
        self.assertIn('"back_to_details", true', focused)
        self.assertNotIn('row(', focused)
        self.assertIn('return', focused)
        runner = SCRIPT.with_name("e2e.sh").read_text()
        self.assertNotIn("showcasePlaybackOnly", runner)

    def test_paths_and_foreign_inputs(self):
        document = manifest("android")
        self.assertEqual(module.validate(document, ["android"]), ["android/error.png"])
        for field in ("backend_instance_id", "journey", "schema_version"):
            bad = copy.deepcopy(document)
            bad[field] = "foreign"
            with self.assertRaises(AssertionError):
                module.validate(bad, ["android"])
        with self.assertRaises(AssertionError):
            module.validate(document, ["android-tv"])
        bad = copy.deepcopy(document)
        bad["item"]["id"] = "other"
        with self.assertRaises(AssertionError):
            module.validate(bad, ["android"])

    def test_unsafe_duplicate_and_missing_evidence(self):
        for path in ("../error.png", "/android/error.png", "android/../error.png", "android/error.png/extra", "android\\error.png"):
            bad = manifest("android")
            bad["platforms"][0]["states"][14]["image"] = path
            with self.assertRaises(AssertionError):
                module.validate(bad, ["android"])
        bad = manifest("android")
        bad["platforms"][0]["states"].append(copy.deepcopy(bad["platforms"][0]["states"][0]))
        with self.assertRaises(AssertionError):
            module.validate(bad, ["android"])
        bad = manifest("android")
        bad["platforms"][0]["states"][14].update(status="blocked", reason="no error image")
        del bad["platforms"][0]["states"][14]["image"]
        with self.assertRaises(AssertionError):
            module.validate(bad, ["android"])

    def test_late_media_error_preserves_prior_observed_playing(self):
        document = manifest("android")
        document["platforms"][0]["states"][0] = {
            "id": "playing", "label": "Playing before track change", "status": "captured",
            "viewport": "1x1", "observation": "Position advanced before later natural media failure",
            "image": "android/playing.png",
        }
        self.assertEqual(module.validate(document, ["android"]), ["android/playing.png", "android/error.png"])

    def test_selector_mapping_and_no_browse_capture_reuse(self):
        runner = SCRIPT.with_name("e2e.sh").read_text()
        self.assertIn("methods=(playbackShowcase)", runner)
        self.assertIn('if [ "$journey" = playback ]; then flows=(); fi', runner)
        self.assertIn("INSTRUMENTATION_STATUS: test=playbackShowcase", runner)
        self.assertIn("INSTRUMENTATION_STATUS_CODE: -[34]", runner)
        source = SCRIPT.parent.parent / "e2e/org/screenroom/e2e/PlayerShowcase.kt"
        self.assertIn('arg("SCREENROOM_BACKEND_INSTANCE_ID")', source.read_text())
        for profile in ("mobile", "tv"):
            wrapper = SCRIPT.parent.parent / profile / f"src/androidTest/kotlin/org/screenroom/{profile}/e2e/HomeE2ETest.kt"
            self.assertIn("@Test fun playbackShowcase() = compose.assertPlaybackShowcase(compose.activity)", wrapper.read_text())

    def test_search_options_require_unique_checkbox_in_content(self):
        player = (SCRIPT.parent.parent / "e2e/org/screenroom/e2e/PlayerShowcase.kt").read_text()
        preflight = player[player.index('waitUntil(15_000) { exists(hasTestTag("home-scope")) }'):player.index("val card = onNode(cardMatcher)")]
        self.assertIn("SemanticsMatcher.expectValue(SemanticsProperties.Role, androidx.compose.ui.semantics.Role.Checkbox)", preflight)
        self.assertIn('val searchContent = hasAnyAncestor(hasTestTag("content"))', preflight)
        self.assertIn("val matcher = hasContentDescription(label) and isToggleable() and checkboxRole and searchContent", preflight)
        self.assertIn("onAllNodes(matcher).assertCountEquals(1)", preflight)
        self.assertIn('setSearchOption("Title only", true)', preflight)
        self.assertIn("setSearchOption(label, enabled)", preflight)
        self.assertIn('setSearchOption("All", true)', preflight)
        self.assertIn('clickNavigation("Search titles", "sidebar")', preflight)
        self.assertIn('clickNavigation("Search settings", "content")', preflight)
        self.assertIn('clickNavigation("Load more", "content")', preflight)
        self.assertIn('choice.performScrollTo().assertIsDisplayed()', preflight)
        self.assertIn('showcaseSearchOptionSettled(nodes.map', preflight)
        self.assertIn('choice.assertIsOnOrOff(enabled)', preflight)
        self.assertLess(preflight.index('setSearchOption("Title only", true)'), preflight.index('"Text (titles only)" to true'))
        self.assertIn('diagnostics.snapshot("$searchStage-before")', preflight)
        self.assertIn('diagnostics.snapshot("$searchStage-settled")', preflight)
        self.assertIn("hasContentDescription(label) and hasClickAction() and buttonRole and hasAnyAncestor(hasTestTag(scope))", preflight)
        self.assertNotIn("onNodeWithContentDescription(", preflight)

        def matches(node, label):
            return node["description"] == label and node["toggleable"] and node["role"] == "Checkbox" and node["scope"] == "content"

        for label in ("Movies", "Shows", "Episodes", "Title only", "Text (titles only)", "Fuzzy titles", "All"):
            option = dict(description=label, role="Checkbox", toggleable=True, scope="content")
            fixtures = [
                dict(description=label, role="Button", toggleable=False, scope="sidebar"),
                option,
                dict(description=label, role="Checkbox", toggleable=True, scope="sidebar"),
                dict(description=label, role="Button", toggleable=True, scope="content"),
                dict(description=label, role="Checkbox", toggleable=False, scope="content"),
            ]
            self.assertEqual([node for node in fixtures if matches(node, label)], [option])
            self.assertEqual(sum(matches(node, label) for node in fixtures + [dict(option)]), 2,
                             "A duplicate actual option must remain ambiguous, not choose first/nth")

    def test_focused_search_stops_before_play_and_diagnostics_are_bounded(self):
        sources = SCRIPT.parent.parent / "e2e/org/screenroom/e2e"
        player = (sources / "PlayerShowcase.kt").read_text()
        focused = player[player.index('if (searchOnly) {'):player.index('searchStage = "playback"')]
        self.assertIn('put("play_invoked", false)', focused)
        self.assertIn("return", focused)
        self.assertNotIn("performClick", focused)
        self.assertLess(player.index('searchStage = "playback"'), player.index('click(hasText("Play") or hasText("Resume"))'))
        self.assertIn('client.getSearchCapabilities()', player)
        self.assertIn('diagnostics.capabilities(capabilities.methods, capabilities.types, capabilities.sources)', player)
        self.assertIn('diagnostics.snapshot("failure-$searchStage")', player)
        self.assertIn('throw failure', player)
        diagnostics = (sources / "ShowcaseSearchDiagnostics.kt").read_text()
        self.assertIn("depth > 30 || nodes.length() >= 1500", diagnostics)
        self.assertIn("depth > 30 || ax.length() >= 1500", diagnostics)
        self.assertIn('.filter { it in labels }', diagnostics)
        self.assertIn('.takeIf { it in labels }', diagnostics)
        self.assertNotIn("takeScreenshot", diagnostics)
        for profile in ("mobile", "tv"):
            wrapper = (SCRIPT.parent.parent / f"{profile}/src/androidTest/kotlin/org/screenroom/{profile}/e2e/HomeE2ETest.kt").read_text()
            self.assertIn("@Test fun showcaseSearchOnly() = compose.assertShowcaseSearch(compose.activity)", wrapper)

    def test_real_kotlin_track_selection_gate_offline(self):
        cache = Path(os.environ.get("GRADLE_USER_HOME", Path.home() / ".gradle")) / "caches/modules-2/files-2.1"

        def jar(group, artifact):
            matches = sorted((cache / group / artifact).glob("*/*/*.jar"))
            self.assertTrue(matches, f"Run the normal Android compile first; cached {artifact} is required (no downloads)")
            return matches[-1]

        compiler = jar("org.jetbrains.kotlin", "kotlin-compiler-embeddable")
        stdlib = jar("org.jetbrains.kotlin", "kotlin-stdlib")
        annotations = jar("org.jetbrains", "annotations")
        dependencies = [compiler, stdlib, annotations]
        for artifact in ("kotlin-build-tools-api", "kotlin-script-runtime", "kotlin-reflect", "kotlin-daemon-embeddable"):
            dependencies.append(jar("org.jetbrains.kotlin", artifact))
        dependencies.append(jar("org.jetbrains.kotlinx", "kotlinx-coroutines-core-jvm"))
        java = Path(os.environ.get("JAVA_HOME", "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home")) / "bin/java"
        helper = SCRIPT.parent.parent / "e2e/org/screenroom/e2e/TrackSelectionEvidence.kt"
        player = helper.with_name("PlayerShowcase.kt").read_text()
        self.assertIn("if (!alreadySelected) node.performClick()", player)
        self.assertIn("evidence = selectionEvidence(node)", player)
        self.assertIn('transportUnconfirmed = text("Transport change is unconfirmed.")', player)
        self.assertNotIn("off.performScrollTo().assertIsEnabled().performClick()", player)
        self.assertIn("hasTestTag(selectedCard.selector)", player)
        self.assertIn("onAllNodes(cardMatcher).assertCountEquals(1)", player)
        self.assertIn("node.viewIdResourceName == selectedCard.selector && node.isClickable", player)
        self.assertIn("resourceMatches(instrumentation.uiAutomation.rootInActiveWindow) == 1", player)
        self.assertNotIn("hasContentDescription(selectedCard.description)", player)
        self.assertIn("SearchMethods(titleOnly = true, text = true, fuzzy = true, semantic = false)", player)
        self.assertIn('"Text (titles only)" to true, "Fuzzy titles" to true', player)
        self.assertIn("result.items.map(::searchCard)", player)
        self.assertIn("item.progressPercent?.toInt()", player)
        self.assertNotIn("description(it) == description(selected)", player)
        search_helper = helper.with_name("ShowcaseSearchSelection.kt")
        inventory_helper = helper.with_name("TrackChoiceInventory.kt")
        self.assertIn("if (!showcaseTrackChoiceSemantics(", player)
        self.assertIn("node.config.getOrNull(SemanticsProperties.Selected)", player)
        self.assertIn("node.config.contains(SemanticsProperties.Heading)", player)
        self.assertIn('captureChoice("subtitles-off", "Off")', player)
        self.assertIn('for ((index, choice) in choices.withIndex())', player)
        self.assertIn('captureChoice("track-choice-${index + 1}", name)', player)
        self.assertIn('else row("track-choice-${index + 1}", "unavailable"', player)
        playback_ui = (SCRIPT.parent.parent / "ui/src/main/kotlin/org/screenroom/ui/PlaybackScreen.kt").read_text()
        sections = playback_ui[playback_ui.index("private fun TrackMenuChoices("):playback_ui.index("private fun MenuChoice(")]
        self.assertIn('DropdownMenuItem(text = { PlayerText(if (kind == PlaybackTrackKind.AUDIO) "Audio" else "Subtitles") }, onClick = {}, enabled = false)', sections)
        menu_choice = playback_ui[playback_ui.index("private fun MenuChoice("):playback_ui.index("private tailrec fun Context.activity")]
        self.assertIn(".semantics { this.selected = selected }", menu_choice)
        production = (SCRIPT.parent.parent / "ui/src/main/kotlin/org/screenroom/ui/HomeScreen.kt").read_text()
        start = production.index("private fun catalogItemTestTag(")
        production_encoder = production[start:production.index("\n\n@OptIn", start)]
        card_source = production[production.index("private fun PosterCard("):production.index("private fun PosterGrid(")]
        self.assertIn(".testTag(catalogItemTestTag(item.id))", card_source)
        self.assertIn("testTagsAsResourceId = true; contentDescription = item.accessibility(seasonContext)", card_source)
        self.assertIn(".clickable(role = Role.Button, onClick = open)", card_source)
        self.assertIn("Modifier.clearAndSetSemantics {}", card_source)
        with tempfile.TemporaryDirectory(prefix="selection-gate-", dir=SCRIPT.parent.parent / "build") as temp:
            root = Path(temp)
            test = root / "GateContract.kt"
            test.write_text("""
package org.screenroom.e2e

fun main() {
    fun rejected(block: () -> Unit) {
        var failed = false
        try { block() } catch (_: IllegalStateException) { failed = true }
        check(failed) { "Unsafe showcase selector was accepted" }
    }
    for (requested in listOf(false, true)) {
        check(!showcaseSearchOptionSettled(emptyList(), requested))
        check(!showcaseSearchOptionSettled(listOf(null), requested))
        check(!showcaseSearchOptionSettled(listOf(!requested), requested))
        check(showcaseSearchOptionSettled(listOf(requested), requested))
        rejected { showcaseSearchOptionSettled(listOf(requested, requested), requested) }
        rejected { showcaseSearchOptionSettled(listOf(requested, !requested), requested) }
    }
    println("Actual search toggle gate rejects missing, indeterminate, stale and ambiguous states")
    val title = "Nosferatu"
    val detail = ShowcaseSearchCard("explicit-id", title, "1922",
        showcaseCardDescription(title, "movie", "GetItem metadata", false, 3))
    val search = ShowcaseSearchCard("explicit-id", title, "1922",
        showcaseCardDescription(title, "Movie", "Actual search metadata", true, 27))
    check(detail.description != search.description)
    check(selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(search)) === search)
    check(search.description == "Nosferatu, Movie, Actual search metadata, Watched, Playback progress 27%")
    val wrongIdSameTitle = search.copy(id = "other-id")
    rejected { selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(wrongIdSameTitle)) }
    rejected { selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(wrongIdSameTitle.copy(year = "2024"))) }
    rejected { selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(search, search)) }
    rejected { selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(search.copy(year = "2024"))) }
    rejected { selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(search.copy(title = "Other"))) }
    rejected { selectShowcaseSearchCard("explicit-id", title, "1922", detail.copy(id = "other-id"), listOf(search)) }
    check(search.description == wrongIdSameTitle.description)
    check(search.selector != wrongIdSameTitle.selector)
    check(selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(search, wrongIdSameTitle)) === search)
    val distinctRemake = wrongIdSameTitle.copy(year = "2024", description = "Nosferatu, Movie, 2024")
    check(selectShowcaseSearchCard("explicit-id", title, "1922", detail, listOf(distinctRemake, search)) === search)
    val ids = listOf("jellyfin:source:item", "jellyfin_source_item", "a:b", "a_b", "a/b", "é", "é", "片🎬", "a%3Ab")
    val selectors = ids.map(::showcaseItemTestTag)
    check(selectors.distinct().size == ids.size)
    for (id in ids) {
        val tag = showcaseItemTestTag(id)
        check(tag == catalogItemTestTag(id)) { "Capture and actual production card encodings differ" }
        check(tag.matches(Regex("screenroom_item_[0-9a-f]+")))
        val decoded = tag.removePrefix("screenroom_item_").chunked(2).map { it.toInt(16).toByte() }.toByteArray().toString(Charsets.UTF_8)
        check(decoded == id)
    }
    check(showcaseItemTestTag("a:b") == "screenroom_item_613a62")
    check(search.copy(id = "another-id").selector != search.selector)
    println("Stable-ID/search-presentation/unique UI-resource selector regressions passed")
    data class MenuRow(val label: String, val selected: Boolean?, val enabled: Boolean, val heading: Boolean = false)
    val menuRows = listOf(
        MenuRow("Audio", null, false),
        MenuRow("English audio", true, true),
        MenuRow("Unavailable audio", false, false),
        MenuRow("Subtitles", null, false),
        MenuRow("Off", true, true),
        MenuRow("English subtitles", false, true),
        MenuRow("Unavailable subtitles", false, false),
        MenuRow("Semantic heading", false, false, true),
        MenuRow("Audio", false, false),
        MenuRow("Subtitles", false, true)
    )
    val inventory = menuRows.filter { showcaseTrackChoiceSemantics(it.selected, it.heading) }
    check(inventory.map { it.label } == listOf("English audio", "Unavailable audio", "Off",
        "English subtitles", "Unavailable subtitles", "Audio", "Subtitles"))
    check(inventory.filterNot { it.enabled }.map { it.label } ==
        listOf("Unavailable audio", "Unavailable subtitles", "Audio"))
    for (selected in listOf(false, true)) {
        check(showcaseTrackChoiceSemantics(selected, false))
        check(!showcaseTrackChoiceSemantics(selected, true))
    }
    check(!showcaseTrackChoiceSemantics(null, false))
    check(!showcaseTrackChoiceSemantics(null, true))
    println("Actual inventory policy excludes sections/headings, preserves disabled choices, Off and order without label filtering")
    for (mask in 0 until 64) {
        fun flag(bit: Int) = mask and (1 shl bit) != 0
        val actual = trackSelectionEvidence(flag(0), flag(1), flag(2), flag(3), flag(4), flag(5))
        val expected = when {
            flag(5) -> TrackSelectionEvidence.UNCONFIRMED
            flag(4) -> TrackSelectionEvidence.MEDIA_BLOCKED
            flag(0) && flag(1) && !flag(2) && !flag(3) -> TrackSelectionEvidence.SETTLED
            else -> TrackSelectionEvidence.WAIT
        }
        check(actual == expected) { "Selection evidence mismatch for mask=$mask: $actual" }
    }
    // Old selected flags cannot authorize a capture while an update disables controls.
    check(trackSelectionEvidence(true, false, false, false, false, false) == TrackSelectionEvidence.WAIT)
    // A transport receipt alone cannot authorize a capture while replacement media loads.
    check(trackSelectionEvidence(true, true, false, true, false, false) == TrackSelectionEvidence.WAIT)
    // An unchanged settled selection needs no new update.
    check(trackSelectionEvidence(true, true, false, false, false, false) == TrackSelectionEvidence.SETTLED)
    println("64 selection-state combinations and stale-selected/loading/current regressions passed")
}
""" + "\n" + production_encoder + "\n")
            subprocess.run([str(java), "-cp", os.pathsep.join(map(str, dependencies)),
                            "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler", "-no-stdlib", "-no-reflect",
                            "-jvm-target", "17", "-classpath", os.pathsep.join(map(str, [stdlib, annotations])),
                             "-d", str(root / "classes"), str(helper), str(search_helper), str(inventory_helper), str(test)], check=True, timeout=60)
            subprocess.run([str(java), "-cp", os.pathsep.join(map(str, [root / "classes", stdlib])),
                            "org.screenroom.e2e.GateContractKt"], check=True, timeout=15)

    def test_both_profiles_gate_promotion_and_png_integrity(self):
        with tempfile.TemporaryDirectory(prefix="android-showcase-", dir=SCRIPT.parent.parent / "build") as temp:
            root = Path(temp)
            target = root / "playback"
            target.mkdir()
            (target / "sentinel").write_text("prior")
            stage = root / "playback-stage-test"
            stage.mkdir()
            for slug in ("android", "android-tv"):
                (stage / slug).mkdir()
                (stage / slug / "error.png").write_bytes(image())
                (stage / f"{slug}.json").write_text(json.dumps(manifest(slug)))
            self.assertEqual(module.png(stage / "android/error.png"), "1x1")
            (stage / "android-tv/error.png").write_bytes(b"not a png")
            result = subprocess.run([str(Path(os.sys.executable)), str(SCRIPT), "promote", str(stage), "android", "android-tv"], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((target / "sentinel").read_text(), "prior")
            self.assertFalse((target / "manifest.json").exists())
            (stage / "android-tv/error.png").write_bytes(image())
            subprocess.run([os.sys.executable, str(SCRIPT), "promote", str(stage), "android", "android-tv"], check=True)
            self.assertEqual([p["slug"] for p in json.loads((target / "manifest.json").read_text())["platforms"]], ["android", "android-tv"])
            self.assertFalse(stage.exists())
            self.assertFalse((target / "sentinel").exists())


if __name__ == "__main__":
    unittest.main()
