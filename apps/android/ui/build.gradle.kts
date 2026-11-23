plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "org.screenroom.ui"
    compileSdk = 37
    defaultConfig { minSdk = 26 }
    sourceSets["main"].kotlin.srcDir("../../../target/client-core/bindings")
    sourceSets["main"].jniLibs.srcDir("../../../target/client-core/android/jniLibs")
    buildFeatures { compose = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("androidx.media3:media3-exoplayer:1.9.0")
    implementation("androidx.media3:media3-exoplayer-hls:1.9.0")
    implementation("androidx.media3:media3-ui:1.9.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.core:core-ktx:1.17.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}

val prepareClientCore by tasks.registering(Exec::class) {
    commandLine("bash", rootProject.file("scripts/build-core.sh"))
}
tasks.named("preBuild").configure { dependsOn(prepareClientCore) }
