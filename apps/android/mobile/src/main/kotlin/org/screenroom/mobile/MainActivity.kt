package org.screenroom.mobile

import android.os.Bundle
import android.graphics.Color
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.compose.BackHandler
import androidx.activity.enableEdgeToEdge
import org.screenroom.ui.DEFAULT_API_ENDPOINT
import org.screenroom.ui.HomeScreen
import org.screenroom.ui.ScreenroomTheme
import org.screenroom.ui.initializeAppTelemetry

class MainActivity : ComponentActivity() {
    private var navigateBack: () -> Boolean = { false }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (!BuildConfig.DEBUG) initializeAppTelemetry("android", BuildConfig.TELEMETRY_ENVIRONMENT)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        setContent {
            ScreenroomTheme {
                BackHandler { if (!navigateBack()) finish() }
                HomeScreen(endpoint = intent.getStringExtra("SCREENROOM_API_ENDPOINT") ?: DEFAULT_API_ENDPOINT, userId = intent.getStringExtra("SCREENROOM_USER_ID") ?: "", settingsNamespace = intent.getStringExtra("SCREENROOM_E2E_SETTINGS_NAMESPACE") ?: "default", debug = BuildConfig.DEBUG, registerBack = { navigateBack = it })
            }
        }
    }
}
