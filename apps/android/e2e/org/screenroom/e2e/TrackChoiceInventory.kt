package org.screenroom.e2e

fun showcaseTrackChoiceSemantics(selected: Boolean?, heading: Boolean): Boolean =
    selected != null && !heading
