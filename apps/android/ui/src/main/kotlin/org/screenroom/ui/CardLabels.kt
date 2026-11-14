package org.screenroom.ui

internal data class CardLabels(val primary: String, val number: String?, val subtitle: String) {
    val heading get() = listOfNotNull(primary.takeIf { it.isNotEmpty() }, number).joinToString(" · ")
}

internal fun episodeSeasonContext(kind: String, parent: String?, displayedKind: String?, displayedId: String?): Boolean =
    kind == "Episode" && displayedKind == "Season" && !displayedId.isNullOrEmpty() && parent == displayedId

internal fun cardLabels(kind: String, title: String, meta: String, year: UInt, seasonContext: Boolean = false): CardLabels {
    if (kind != "Episode") return CardLabels(title, null, if (kind == "Movie") year.toString() else meta)
    val match = Regex("^(?:(.+) · )?S(\\d+)\\s*E(\\d+)(?: · .*)?$").matchEntire(meta.trim())
        ?: return CardLabels("Episode", null, title)
    val episode = "E${match.groupValues[3].padStart(2, '0')}"
    return CardLabels(if (seasonContext) "" else match.groupValues[1],
        if (seasonContext) episode else "S${match.groupValues[2].padStart(2, '0')}:$episode", title)
}

internal fun progressError(code: String): String = if (code == "progress_unavailable")
    "Continue Watching is unavailable. Jellyfin progress could not be read. Other catalog rows remain available."
else "This row could not be loaded. Other catalog rows remain available."
