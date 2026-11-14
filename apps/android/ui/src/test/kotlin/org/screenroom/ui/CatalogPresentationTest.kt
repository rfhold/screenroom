package org.screenroom.ui

import org.junit.Assert.*
import org.junit.Test
import org.screenroom.core.*

class CatalogPresentationTest {
    @Test fun providerIdentityStatusAndLinkAreNotInvented() {
        val rating = Rating("archive", 4.5, 5.0, null, "date")
        assertEquals(ProviderPresentation("Archive · 4.5/5", "https://archive.example/item"), ProviderReference("Archive", "Available", "https://archive.example/item").presentation(rating, "4.5/5"))
        assertEquals(ProviderPresentation("Library: Ready", null), ProviderReference("Library", "Ready", "").presentation(rating, "4.5/5"))
        assertEquals(ProviderPresentation("Provider · 4.5/5", "https://example.com"), ProviderReference("Provider", "Listed", "https://example.com").presentation(rating.copy(provider = "provider"), "4.5/5"))
        assertNull(ProviderReference("Other", "Linked", "javascript:alert(1)").presentation(null, "—").url)
    }
    @Test fun libraryIdentityAndOrderDoNotDependOnEnglishTitles() {
        val libraries = listOf(MediaLibrary("collection-9", "Cinéma"), MediaLibrary("shelf-2", "Archive"))
        assertEquals("collection-9", libraries.resolveLibrary("")!!.id)
        assertEquals("Archive", libraries.resolveLibrary("shelf-2")!!.title)
        assertNull(emptyList<MediaLibrary>().resolveLibrary("movies"))
    }

    @Test fun colorsUseServerPaletteAndNeutralFallbackOnly() {
        assertEquals(0xff123abc, presentationColor("#123abc", 0xff202020))
        assertEquals(0xff202020, presentationColor(null, 0xff202020))
        assertEquals(0xff555555, presentationColor("not-a-color", 0xff555555))
    }

    @Test fun capabilitiesLimitAvailableDefaults() {
        val caps = SearchCapabilities(listOf("fuzzy"), listOf("Episode"), listOf("archive"), "Server description", 27u)
        assertEquals(Methods(false, true, false), caps.defaultMethods())
        assertEquals(setOf("Episode"), caps.allowedTypes(SearchTypes(true, true, true)))
        assertEquals(emptySet<String>(), caps.allowedTypes(null))
        assertEquals("Archive", sourceLabel(caps.sources.single()))
        assertEquals("Server description", caps.semanticDescription)
        assertEquals(27u, caps.maximumQueryLength)
    }

    @Test fun titleOnlyNeverEnablesAnUnavailableMethod() {
        val fuzzyOnly = SearchCapabilities(listOf("fuzzy", "title_only", "semantic"), listOf("Movie"), listOf("all"), "Description", 160u)
        val next = fuzzyOnly.toggleTitleOnly(Methods(false, false, true))
        assertEquals(Methods(false, true, false, true), next)
        assertEquals(Methods(false, true, false, false), fuzzyOnly.toggleTitleOnly(next))
        val semanticOnly = fuzzyOnly.copy(methods = listOf("semantic", "title_only"))
        assertFalse(semanticOnly.canUseTitleOnly())
        assertEquals(Methods(false, false, true), semanticOnly.toggleTitleOnly(Methods(false, false, true)))
    }

    @Test fun itemMapperRetainsEveryServerRecord() {
        val rights = Rights("Metadata", "Server rights", "https://archive.example/rights", "https://archive.example/license", "Creator")
        val rating = Rating("Archive", 4.5, 5.0, null, "server-rating-date")
        val runtime = org.screenroom.core.Runtime(90u, 80u, 100u, "Edition varies", "1m 30s")
        val provider = ProviderReference("Archive", "Available", "https://archive.example/item")
        val poster = Artwork("poster", "https://archive.example/poster.png", "Poster", "Poster artist", "Poster CC BY", "https://archive.example/poster-source", "https://archive.example/poster-license", "poster")
        val item = Item("opaque", "Server title", "Movie", null, 2020u, "Server subtitle", "Server overview", listOf("tag"), listOf("#123abc", "#abcdef"), "date", 4u, rights, rights, poster, rating, runtime, listOf(provider), 20u, listOf("Server provenance"), "metadata-date", null)
        val title = item.ui()
        listOf(null, false, true, false, null, true).forEach { watched ->
            val presentation = item.copy(watched = watched, progressPercent = 100u).ui()
            assertEquals(watched == true, presentation.watched)
            assertEquals(watched == true, presentation.cardAccessibility.contains("Watched"))
            assertTrue(presentation.playbackAccessibility.contains("Playback progress 100%"))
            assertSame(poster, presentation.artwork)
            assertEquals(item.id, presentation.id)
        }
        assertEquals("Watched", item.copy(watched = true, progressPercent = null).ui().playbackAccessibility)
        assertSame(item, title.record)
        assertSame(poster, title.artwork)
        assertNotEquals(title.rights!!.attribution, title.artwork!!.creator)
        assertNotEquals(title.rights!!.licenseUrl, title.artwork!!.licenseUrl)
        assertEquals("https://archive.example/poster-source", title.artwork!!.source)
        assertEquals(0xff123abc, title.ink)
        assertEquals(0xffabcdef, title.light)
        assertEquals(rating, title.record.rating)
        assertEquals(runtime, title.record.runtime)
        assertEquals(listOf(provider), title.record.providerReferences)
        assertEquals(rights, title.record.metadataRights)
        assertEquals("metadata-date", title.record.metadataRetrievedAt)
        assertEquals(listOf("Server provenance"), title.record.provenanceNotes)
        assertTrue(validLink(provider.url))
        listOf("", "javascript:alert(1)", "https://user:secret@example.com", "not a URL").forEach { assertFalse(validLink(it)) }
    }

    @Test fun sourceSelectionReconcilesAgainstCapabilitiesBeforeRequests() {
        val caps = SearchCapabilities(listOf("text"), listOf("Movie"), listOf("all", "jellyfin"), "", 160u)
        assertEquals("all", caps.allowedSource("tmdb"))
        assertEquals("all", caps.allowedSource("removed-source"))
        assertEquals("jellyfin", caps.allowedSource("jellyfin"))
        assertEquals("jellyfin", caps.copy(sources = listOf("jellyfin")).allowedSource("all"))
        assertEquals("", caps.copy(sources = emptyList()).allowedSource("tmdb"))
        for (provider in listOf("Imdb", "Tmdb")) {
            assertEquals(ProviderPresentation("$provider: source_supplied", "https://example.test/item"),
                ProviderReference(provider, "source_supplied", "https://example.test/item").presentation(null, "—"))
        }
    }
}
