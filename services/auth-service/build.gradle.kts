// Atlas auth-service.
//
// Phase 2A scope:
//   - Password hashing (bcrypt via jbcrypt)
//   - JWT signing and verification (HS256 via jose4j) with atlas:* claims
//   - User/session repositories (interface + in-memory + Exposed implementations)
//   - AuthService business logic: register, authenticate
//   - Unit tests
//
// Phase 2B will add:
//   - gRPC server (Ktor or grpc-kotlin-stub) on port 50051
//   - Kafka producer for atlas.auth.tokens
//   - grpc.health.v1.Health
//   - Docker compose end-to-end smoke test

plugins {
    alias(libs.plugins.kotlin.jvm)
}

kotlin {
    jvmToolchain(21)
}

dependencies {
    implementation(libs.exposed.core)
    implementation(libs.exposed.jdbc)
    implementation(libs.exposed.java.time)
    implementation(libs.hikari)
    implementation(libs.postgres)
    implementation(libs.jose4j)
    implementation(libs.jbcrypt)
    implementation(libs.slf4j.api)
    implementation(libs.logback.classic)
    implementation(libs.logstash.encoder)

    testImplementation(libs.kotlin.test)
    testImplementation(libs.junit.jupiter)
    testRuntimeOnly(libs.junit.platform.launcher)
}
