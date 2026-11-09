package org.screenroom.ui

internal data class RatingPresentation(val score: String, val disclosure: String)

internal fun ratingPresentation(rating: org.screenroom.core.Rating?, metadataRetrievedAt: String): RatingPresentation {
    if (rating == null) return RatingPresentation("—", "Rating snapshot not supplied. Metadata snapshot · ${metadataRetrievedAt.ifBlank { "Not supplied" }}.")
    val scale = java.math.BigDecimal.valueOf(rating.scale).stripTrailingZeros().toPlainString()
    val score = if (rating.votes == 0uL) "Unrated" else String.format(java.util.Locale.US, "%.1f/%s", rating.value, scale)
    val count = rating.votes?.let { "$it votes" } ?: "Vote count unavailable"
    return RatingPresentation(score, "${rating.provider} user rating: $score · $count. Snapshot · ${rating.retrievedAt.take(10)} · Not live.")
}
