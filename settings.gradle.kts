/*
 * Root Gradle settings for all Kotlin services in Atlas.
 *
 * Subprojects are included as each phase lands. Phase 1 ships the settings
 * file with no includes so a future `./gradlew` invocation resolves cleanly.
 */

rootProject.name = "atlas"

// Phase 2
// include("services:auth-service")
// Phase 4
// include("services:payments-service")
// Phase 6
// include("consumers:fare-consumer")

pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.PREFER_PROJECT)
    repositories {
        mavenCentral()
    }
}
