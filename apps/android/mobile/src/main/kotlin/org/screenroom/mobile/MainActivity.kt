package org.screenroom.mobile

import android.os.Bundle
import android.graphics.Color
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.compose.BackHandler
import androidx.activity.enableEdgeToEdge
import org.screenroom.ui.HomeScreen
import org.screenroom.ui.ScreenroomTheme

class MainActivity : ComponentActivity() {
    private var navigateBack: () -> Boolean = { false }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        setContent {
            ScreenroomTheme {
                BackHandler { if (!navigateBack()) finish() }
                HomeScreen(endpoint = intent.getStringExtra("SCREENROOM_API_ENDPOINT") ?: "http://127.0.0.1:50051", profile = intent.getStringExtra("SCREENROOM_MOCK_PROFILE") ?: "android-mobile", debug = BuildConfig.DEBUG, registerBack = { navigateBack = it })
            }
        }
    }
}
