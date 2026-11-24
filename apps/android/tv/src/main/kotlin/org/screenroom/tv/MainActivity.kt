package org.screenroom.tv

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.BackHandler
import org.screenroom.ui.DEFAULT_API_ENDPOINT
import org.screenroom.ui.HomeScreen
import org.screenroom.ui.ScreenroomTheme
import org.screenroom.ui.initializeAppTelemetry

class MainActivity : ComponentActivity() {
    private var navigateBack: () -> Boolean = { false }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (!BuildConfig.DEBUG) initializeAppTelemetry("android-tv", BuildConfig.TELEMETRY_ENVIRONMENT)
        setContent {
            ScreenroomTheme {
                BackHandler { if (!navigateBack()) finish() }
                HomeScreen(television = true, endpoint = intent.getStringExtra("SCREENROOM_API_ENDPOINT") ?: DEFAULT_API_ENDPOINT, userId = intent.getStringExtra("SCREENROOM_USER_ID") ?: "", settingsNamespace = intent.getStringExtra("SCREENROOM_E2E_SETTINGS_NAMESPACE") ?: "default", debug = BuildConfig.DEBUG, registerBack = { navigateBack = it })
            }
        }
    }
}
