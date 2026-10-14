plugins { kotlin("jvm"); `java-library` }
kotlin {
    jvmToolchain(21)
    compilerOptions.jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
}
java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}
dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
    api("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    api("org.commonmark:commonmark:0.24.0")
    api("org.commonmark:commonmark-ext-gfm-tables:0.24.0")
    api("org.commonmark:commonmark-ext-gfm-strikethrough:0.24.0")
    api("org.commonmark:commonmark-ext-autolink:0.24.0")
    api("org.commonmark:commonmark-ext-task-list-items:0.24.0")
    testImplementation(kotlin("test-junit"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}
sourceSets.test {
    resources.srcDir("../../../contracts")
    resources.srcDir("../../ios/WispScience/DemoData")
}
