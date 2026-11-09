package org.screenroom.ui

import org.junit.Assert.*
import org.junit.Test
import org.screenroom.core.*

class CatalogPresentationTest {
    @Test fun providerIdentityStatusAndLinkAreNotInvented() {
        val rating = Rating("archive", 4.5, 5.0, null, "date")
        assertEquals(ProviderPresentation("Archive · 4.5/5", "https://archive.example/item"), ProviderReference("Archive", "Available", "https://archive.example/item").presentation(rating, "4.5/5"))
        assertEquals(ProviderPresentation("Library: Ready", null), ProviderReference("Library", "Ready", "").presentation(rating, "4.5/5"))
        assertEquals(ProviderPresentation("TMDB · 4.5/5", "https://example.com"), ProviderReference("TMDB", "Listed", "https://example.com").presentation(rating.copy(provider = "tmdb"), "4.5/5"))
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
        val item = Item("opaque", "Server title", "Movie", null, 2020u, "Server subtitle", "Server overview", listOf("tag"), listOf("#123abc", "#abcdef"), "date", 4u, rights, rights, null, rating, runtime, listOf(provider), 20u, listOf("Server provenance"), "metadata-date")
        val title = item.ui()
        assertSame(item, title.record)
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

    @Test fun globalInfoPreservesUnvisitedArtworkAndOrderedNotices() {
        val logo = Artwork("brand", "https://mock.example/art/brand.png", "Server brand", "Creator", "License", "https://source.example", "https://license.example", "logo")
        val unvisited = logo.copy(id = "unvisited", kind = "portrait")
        val notices = listOf(CatalogNotice("z", "legal", "Last alphabetically", "First on server", "", ""), CatalogNotice("a", "art", "Second", "Second on server", "Source", "https://source.example"))
        val info = CatalogInfo(listOf(logo, unvisited), notices, "catalog-date")
        assertEquals(listOf("brand", "unvisited"), info.artwork.map { it.id })
        assertEquals(listOf("z", "a"), info.notices.map { it.id })
        assertEquals("brand", info.artwork.single { it.kind == "logo" }.id)
        assertEquals("catalog-date", info.retrievedAt)
    }
}
