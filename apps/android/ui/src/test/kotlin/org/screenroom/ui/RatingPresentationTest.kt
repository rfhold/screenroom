package org.screenroom.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class RatingPresentationTest {
    private val retrievedAt = "2026-11-07T12:00:00Z"

    @Test fun unknownCountPreservesAvailableRating() {
        val result = ratingPresentation(org.screenroom.core.Rating("TMDB", 6.1, 10.0, null, retrievedAt), retrievedAt)
        assertEquals("6.1/10", result.score)
        assertEquals("TMDB user rating: 6.1/10 · Vote count unavailable. Snapshot · 2026-11-07 · Not live.", result.disclosure)
    }

    @Test fun explicitZeroCountIsUnrated() {
        val result = ratingPresentation(org.screenroom.core.Rating("TMDB", 6.1, 10.0, 0uL, retrievedAt), retrievedAt)
        assertEquals("Unrated", result.score)
        assertEquals("TMDB user rating: Unrated · 0 votes. Snapshot · 2026-11-07 · Not live.", result.disclosure)
    }

    @Test fun absentRatingDoesNotInventSnapshot() {
        run {
            val result = ratingPresentation(null, retrievedAt)
            assertEquals("—", result.score)
            assertEquals("Rating snapshot not supplied. Metadata snapshot · $retrievedAt.", result.disclosure)
        }
    }

    @Test fun providerScaleAndLargeVoteCountComeFromServer() {
        val result = ratingPresentation(org.screenroom.core.Rating("Archive", 4.5, 5.0, 5_000_000_000uL, retrievedAt), retrievedAt)
        assertEquals("4.5/5", result.score)
        assertEquals("Archive user rating: 4.5/5 · 5000000000 votes. Snapshot · 2026-11-07 · Not live.", result.disclosure)
    }
}
