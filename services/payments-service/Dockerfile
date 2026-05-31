# Atlas payments-service container. Multi-stage so the runtime image does not
# include Gradle, the Kotlin compiler, or build cache. Build context must be
# the repo root because services/payments-service/build.gradle.kts pulls .proto
# files from $rootDir/proto.

FROM gradle:8.10.2-jdk21 AS builder
WORKDIR /build

# Copy build configuration first so dependency resolution caches across
# source-only changes.
COPY settings.gradle.kts build.gradle.kts gradle.properties* ./
COPY gradle ./gradle
COPY services/payments-service/build.gradle.kts ./services/payments-service/build.gradle.kts

# Warm the dependency cache without requiring source files to exist yet.
RUN gradle --no-daemon :services:payments-service:help -q || true

# Now copy sources and the shared proto/ directory.
COPY proto ./proto
COPY services/payments-service/src ./services/payments-service/src

RUN gradle --no-daemon :services:payments-service:shadowJar

# --- runtime stage -----------------------------------------------------------
FROM eclipse-temurin:21-jre AS runtime
WORKDIR /app
COPY --from=builder /build/services/payments-service/build/libs/payments-service-all.jar /app/payments-service.jar

EXPOSE 50053 8053

# Logback writes to stdout via the bundled logback.xml; nothing extra needed.
ENTRYPOINT ["java", "-jar", "/app/payments-service.jar"]
