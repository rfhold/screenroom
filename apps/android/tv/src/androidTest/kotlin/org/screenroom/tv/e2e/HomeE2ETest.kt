package org.screenroom.tv.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.screenroom.e2e.assertHappyPathAndCapture
import org.screenroom.e2e.assertRegression
import org.screenroom.e2e.assertRetry
import org.screenroom.tv.MainActivity

@RunWith(AndroidJUnit4::class)
class HomeE2ETest {
    @get:Rule val compose = org.screenroom.e2e.coreComposeRule(MainActivity::class.java, "android-tv")

    @Test fun happyPath() = compose.assertHappyPathAndCapture(compose.activity)
    @Test fun regression() = compose.assertRegression(compose.activity)
    @Test fun retry() = compose.assertRetry(compose.activity)
}
