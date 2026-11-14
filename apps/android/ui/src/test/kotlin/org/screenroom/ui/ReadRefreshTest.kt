package org.screenroom.ui

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.awt.image.BufferedImage
import javax.imageio.ImageIO
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class ReadRefreshTest {
    @Test fun refreshRetriesPendingSearchAndArtwork() = runTest {
        val refresh = ReadRefresh()
        val queries = mutableListOf<String>()
        var artRequests = 0
        var cancellations = 0
        var result: String? = null
        var decoded: java.awt.image.BufferedImage? = null
        val imageBytes = ByteArrayOutputStream().also {
            ImageIO.write(BufferedImage(2, 3, BufferedImage.TYPE_INT_RGB), "png", it)
        }.toByteArray()
        val search = launch {
            refresh.repeatOnRefresh {
                queries += "query"
                if (queries.size == 1) try { awaitCancellation() } finally { cancellations++ }
                result = "result"
            }
        }
        val artwork = launch {
            refresh.repeatOnRefresh {
                artRequests++
                if (artRequests == 1) try { awaitCancellation() } finally { cancellations++ }
                decoded = ImageIO.read(ByteArrayInputStream(imageBytes))
            }
        }
        runCurrent()
        assertEquals(listOf("query"), queries)
        assertEquals(1, artRequests)
        refresh.advance()
        runCurrent()
        assertEquals(2, cancellations)
        assertEquals(listOf("query", "query"), queries)
        assertEquals("result", result)
        assertEquals(2, artRequests)
        assertNotNull(decoded)
        assertEquals(2, decoded!!.width)
        assertEquals(3, decoded!!.height)
        search.cancelAndJoin(); artwork.cancelAndJoin()
        refresh.advance(); runCurrent()
        assertEquals(2, artRequests)
        assertEquals(2, queries.size)
    }

    @Test fun nativeCancellationDoesNotDetachRefreshOwner() = runTest {
        val refresh = ReadRefresh()
        var calls = 0
        val owner = launch {
            refresh.repeatOnRefresh {
                calls++
                if (calls == 1) throw CancellationException("Native read invalidated")
            }
        }
        runCurrent()
        refresh.advance(); runCurrent()
        assertEquals(2, calls)
        owner.cancelAndJoin()
    }
}
