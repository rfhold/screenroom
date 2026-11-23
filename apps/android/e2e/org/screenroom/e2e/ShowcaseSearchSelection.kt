package org.screenroom.e2e

internal data class ShowcaseSearchCard(val id: String, val title: String, val year: String, val description: String) {
    val selector get() = showcaseItemTestTag(id)
}

// Matches the clickable card's merged test tag and raw accessibility resource ID.
// UiAutomator uses By.res(tag) / UiSelector().resourceId(tag), without a package prefix.
internal fun showcaseItemTestTag(id: String): String = "screenroom_item_" +
    id.toByteArray(Charsets.UTF_8).joinToString("") { (it.toInt() and 0xff).toString(16).padStart(2, '0') }

internal fun showcaseCardDescription(title: String, kind: String, subtitle: String, watched: Boolean, progress: Int?): String =
    listOf(title, kind.replaceFirstChar { it.uppercase() }, subtitle,
        listOfNotNull("Watched".takeIf { watched }, progress?.let { "Playback progress $it%" }).joinToString(", "))
        .filter(String::isNotEmpty).joinToString(", ")

internal fun showcaseSearchOptionSettled(states: List<Boolean?>, requested: Boolean): Boolean {
    check(states.size <= 1) { "Ambiguous search option after input" }
    return states.size == 1 && states.single() == requested
}

internal fun selectShowcaseSearchCard(
    id: String,
    title: String,
    year: String,
    detail: ShowcaseSearchCard,
    results: List<ShowcaseSearchCard>,
): ShowcaseSearchCard {
    check(detail.id == id && detail.title == title && detail.year == year) { "Explicit showcase selector differs from catalog item" }
    val matches = results.filter { it.id == id }
    check(matches.size == 1) { "Explicit item ID must occur exactly once in bounded title search" }
    val selected = matches.single()
    check(selected.title == title && selected.year == year) { "Search item identity differs from explicit selector" }
    check(results.none { it.id != id && it.selector == selected.selector }) { "Explicit item has an ambiguous UI identity selector" }
    return selected
}
