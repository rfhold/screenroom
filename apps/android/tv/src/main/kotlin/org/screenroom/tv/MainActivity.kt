package org.screenroom.tv

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.BackHandler
import org.screenroom.ui.HomeScreen
import org.screenroom.ui.ScreenroomTheme

class MainActivity : ComponentActivity() {
    private var navigateBack: () -> Boolean = { false }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            ScreenroomTheme {
                BackHandler { if (!navigateBack()) finish() }
                HomeScreen(television = true, endpoint = intent.getStringExtra("SCREENROOM_API_ENDPOINT") ?: "http://127.0.0.1:50051", profile = intent.getStringExtra("SCREENROOM_MOCK_PROFILE") ?: "android-tv", debug = BuildConfig.DEBUG, registerBack = { navigateBack = it })
            }
        }
    }
}
