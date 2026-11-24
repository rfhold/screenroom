plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "org.screenroom.tv"
    compileSdk = 37

    defaultConfig {
        applicationId = "tv.screenroom.tv"
        minSdk = 26
        targetSdk = 37
        versionCode = providers.gradleProperty("screenroomBuild").orElse("1").get().toInt()
        val telemetryEnvironment = providers.gradleProperty("screenroomTelemetryEnvironment").orElse("production").get()
        require(telemetryEnvironment in setOf("local", "preview", "production"))
        buildConfigField("String", "TELEMETRY_ENVIRONMENT", "\"$telemetryEnvironment\"")
        versionName = providers.gradleProperty("screenroomVersion").orElse("0.1.0").get()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildFeatures { compose = true; buildConfig = true }
    sourceSets.getByName("androidTest").kotlin.directories.add("../e2e")
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":ui"))
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.foundation:foundation")
    androidTestImplementation(platform("androidx.compose:compose-bom:2026.09.00"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    androidTestImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}
