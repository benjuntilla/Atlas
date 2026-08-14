# Atlas fare-consumer container. Multi-stage so the runtime image does not
# include Gradle or the Kotlin compiler. Build context must be the repo root
# because consumers/fare-consumer/build.gradle.kts pulls .proto files from
# $rootDir/proto.

FROM gradle:8.10.2-jdk21 AS builder
WORKDIR /build

# Build configuration first so dependency resolution caches across
# source-only changes. Only this module's build file is copied — the other
# projects named in settings.gradle.kts configure as empty ones, which is
# the same arrangement the auth and payments images already use.
COPY settings.gradle.kts build.gradle.kts gradle.properties* ./
COPY gradle ./gradle
COPY consumers/fare-consumer/build.gradle.kts ./consumers/fare-consumer/build.gradle.kts

RUN gradle --no-daemon :consumers:fare-consumer:help -q || true

COPY proto ./proto
COPY consumers/fare-consumer/src ./consumers/fare-consumer/src

RUN gradle --no-daemon :consumers:fare-consumer:shadowJar

# --- runtime stage -----------------------------------------------------------
FROM eclipse-temurin:21-jre AS runtime
WORKDIR /app
COPY --from=builder /build/consumers/fare-consumer/build/libs/fare-consumer-all.jar /app/fare-consumer.jar

# HTTP only: /metrics and /healthz. No gRPC server — this process is a
# client of payments-service, not a server.
EXPOSE 8054

ENTRYPOINT ["java", "-jar", "/app/fare-consumer.jar"]
