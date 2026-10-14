plugins {
    id("com.android.application")
    kotlin("android")
    kotlin("plugin.compose")
}
// The APK ships with each desktop release and carries the workspace version,
// so the tag, the APK and the host it connects to always name the same release.
val wispVersion: String = rootDir.resolve("../../Cargo.toml").readLines().map(String::trim)
    .dropWhile { it != "[workspace.package]" }.drop(1).takeWhile { !it.startsWith("[") }
    .firstNotNullOfOrNull { Regex("""^version\s*=\s*"(\d+\.\d+\.\d+)"$""").find(it)?.groupValues?.get(1) }
    ?: error("workspace.package.version not found in the repository Cargo.toml")
val wispVersionCode: Int = wispVersion.split(".").map(String::toInt).let { (major, minor, patch) ->
    require(minor < 1000 && patch < 1000) { "versionCode needs minor and patch below 1000: $wispVersion" }
    major * 1_000_000 + minor * 1_000 + patch
}
// Release signing comes only from the environment (CI secrets or a local shell).
// Without it assembleRelease still builds, unsigned; no key material is committed.
val releaseKeystore: String? = providers.environmentVariable("WISP_ANDROID_KEYSTORE").orNull?.takeIf(String::isNotBlank)
fun signingEnv(name: String): String =
    providers.environmentVariable(name).orNull?.takeIf(String::isNotBlank)
        ?: error("$name is required when WISP_ANDROID_KEYSTORE is set")
android {
    namespace = "science.wisp.android"
    compileSdk = 36
    buildToolsVersion = "36.1.0"
    defaultConfig {
        applicationId = "science.wisp.android"
        minSdk = 26
        targetSdk = 36
        versionCode = wispVersionCode
        versionName = wispVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    signingConfigs {
        releaseKeystore?.let { keystore -> create("release") {
            storeFile = file(keystore)
            storePassword = signingEnv("WISP_ANDROID_KEYSTORE_PASSWORD")
            keyAlias = signingEnv("WISP_ANDROID_KEY_ALIAS")
            keyPassword = signingEnv("WISP_ANDROID_KEY_PASSWORD")
        } }
    }
    buildTypes {
        release {
            // R8 stays off until shrinking is verified against the protocol and Compose code.
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures { compose = true }
    // One synthetic dataset for both mobile clients; no copied or drifting fixtures.
    sourceSets["main"].assets.srcDir("../../ios/WispScience/DemoData")
    packaging.resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
}
kotlin {
    jvmToolchain(21)
    compilerOptions.jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
}
dependencies {
    implementation(project(":remote"))
    implementation(platform("androidx.compose:compose-bom:2025.04.01"))
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")
    testImplementation(kotlin("test-junit"))
    androidTestImplementation(platform("androidx.compose:compose-bom:2025.04.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
