package org.screenroom.mobile.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.screenroom.e2e.assertHappyPathAndCapture
import org.screenroom.e2e.assertPlaybackShowcase
import org.screenroom.e2e.assertShowcaseSearch
import org.screenroom.e2e.assertShowcasePlayback
import org.screenroom.e2e.assertRegression
import org.screenroom.e2e.assertRetry
import org.screenroom.e2e.assertSettingsPersistence
import org.screenroom.e2e.assertCorrectionEdges
import org.screenroom.e2e.coreComposeRule
import org.screenroom.mobile.MainActivity

@RunWith(AndroidJUnit4::class)
class HomeE2ETest {
    @get:Rule val compose = coreComposeRule(MainActivity::class.java, "android-mobile")
    @Test fun happyPath() = compose.assertHappyPathAndCapture(compose.activity)
    @Test fun playbackShowcase() = compose.assertPlaybackShowcase(compose.activity)
    @Test fun showcaseSearchOnly() = compose.assertShowcaseSearch(compose.activity)
    @Test fun showcasePlaybackOnly() = compose.assertShowcasePlayback(compose.activity)
    @Test fun regression() = compose.assertRegression(compose.activity)
    @Test fun retry() = compose.assertRetry(compose.activity)
    @Test fun settingsPersistence() = compose.assertSettingsPersistence(compose.activity)
    @Test fun correctionEdges() = compose.assertCorrectionEdges(compose.activity)
}
