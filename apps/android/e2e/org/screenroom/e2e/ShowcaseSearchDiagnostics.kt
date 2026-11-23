package org.screenroom.e2e

import android.app.Instrumentation
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.ComposeTestRule
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

internal class ShowcaseSearchDiagnostics(
    private val rule: ComposeTestRule,
    private val instrumentation: Instrumentation,
    private val directory: File,
    private val title: String,
) {
    private var sequence = 0
    private val labels = setOf("Title only", "Text (titles only)", "Text (titles and metadata)", "Fuzzy titles",
        "Semantic", "Movies", "Shows", "Episodes", "All", "Search titles", "Search settings", "Load more",
        "Expand sidebar", "Play", "Resume", title)

    fun capabilities(methods: List<String>, types: List<String>, sources: List<String>) {
        directory.mkdirs()
        // Record only protocol vocabulary, never private source identifiers.
        File(directory, "capabilities.json").writeText(JSONObject()
            .put("methods", JSONArray(methods.filter { it in setOf("title_only", "text", "fuzzy", "semantic") }))
            .put("types", JSONArray(types.filter { it.lowercase() in setOf("movie", "series", "episode") }))
            .put("has_all_source", "all" in sources).toString(2))
    }

    fun snapshot(stage: String) {
        directory.mkdirs()
        val result = JSONObject().put("stage", stage).put("sequence", sequence)
        for (unmerged in listOf(false, true)) {
            val nodes = JSONArray()
            fun walk(node: SemanticsNode, depth: Int) {
                if (depth > 30 || nodes.length() >= 1500) return
                val config = node.config
                nodes.put(JSONObject().put("depth", depth)
                    .put("role", config.getOrNull(SemanticsProperties.Role)?.toString())
                    .put("tag", config.getOrNull(SemanticsProperties.TestTag)?.takeIf {
                        it in setOf("content", "sidebar", "home-scope", "search-query", "search-effective-methods", "detail-header")
                    })
                    .put("descriptions", JSONArray(config.getOrNull(SemanticsProperties.ContentDescription).orEmpty().filter { it in labels }))
                    .put("toggle", config.getOrNull(SemanticsProperties.ToggleableState)?.toString())
                    .put("disabled", config.getOrNull(SemanticsProperties.Disabled) != null)
                    .put("selected", config.getOrNull(SemanticsProperties.Selected))
                    .put("bounds", node.boundsInRoot.toString()))
                node.children.forEach { walk(it, depth + 1) }
            }
            rule.onAllNodes(isRoot(), useUnmergedTree = unmerged).fetchSemanticsNodes().forEach { walk(it, 0) }
            result.put(if (unmerged) "unmerged" else "merged", nodes)
        }
        val ax = JSONArray()
        fun walkAx(node: AccessibilityNodeInfo?, depth: Int) {
            if (node == null || depth > 30 || ax.length() >= 1500) return
            val bounds = android.graphics.Rect()
            node.getBoundsInScreen(bounds)
            ax.put(JSONObject().put("depth", depth).put("class", node.className?.toString())
                .put("description", node.contentDescription?.toString()?.takeIf { it in labels })
                .put("enabled", node.isEnabled).put("clickable", node.isClickable)
                .put("checkable", node.isCheckable).put("checked", node.isChecked)
                .put("visible", node.isVisibleToUser).put("bounds", bounds.toShortString()))
            for (index in 0 until node.childCount) walkAx(node.getChild(index), depth + 1)
        }
        walkAx(instrumentation.uiAutomation.rootInActiveWindow, 0)
        result.put("accessibility", ax)
        File(directory, "${sequence++.toString().padStart(3, '0')}-${stage.replace(Regex("[^a-zA-Z0-9_-]"), "_")}.json")
            .writeText(result.toString(2))
    }
}
