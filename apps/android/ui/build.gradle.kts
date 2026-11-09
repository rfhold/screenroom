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
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}

val prepareClientCore by tasks.registering(Exec::class) {
    commandLine("bash", rootProject.file("scripts/build-core.sh"))
}
tasks.named("preBuild").configure { dependsOn(prepareClientCore) }
