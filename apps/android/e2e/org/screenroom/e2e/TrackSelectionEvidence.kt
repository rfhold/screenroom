package org.screenroom.e2e

internal enum class TrackSelectionEvidence { WAIT, SETTLED, MEDIA_BLOCKED, UNCONFIRMED }

internal fun trackSelectionEvidence(
    selected: Boolean,
    controlsEnabled: Boolean,
    preparing: Boolean,
    loading: Boolean,
    mediaError: Boolean,
    transportUnconfirmed: Boolean,
): TrackSelectionEvidence = when {
    transportUnconfirmed -> TrackSelectionEvidence.UNCONFIRMED
    mediaError -> TrackSelectionEvidence.MEDIA_BLOCKED
    selected && controlsEnabled && !preparing && !loading -> TrackSelectionEvidence.SETTLED
    else -> TrackSelectionEvidence.WAIT
}
