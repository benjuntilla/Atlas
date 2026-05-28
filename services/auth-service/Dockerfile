# Atlas auth-service container. Multi-stage so the runtime image does not
# include Gradle, the Kotlin compiler, or build cache. Build context must be
# the repo root because services/auth-service/build.gradle.kts pulls .proto
# files from $rootDir/proto.

FROM gradle:8.10.2-jdk21 AS builder
WORKDIR /build

# Copy build configuration first so dependency resolution caches across
# source-only changes.
COPY settings.gradle.kts build.gradle.kts gradle.properties* ./
COPY gradle ./gradle
COPY services/auth-service/build.gradle.kts ./services/auth-service/build.gradle.kts

# Warm the dependency cache. The `help` task touches every classpath without
# requiring source files to exist yet.
RUN gradle --no-daemon :services:auth-service:help -q || true

# Now copy sources and the shared proto/ directory.
COPY proto ./proto
COPY services/auth-service/src ./services/auth-service/src

RUN gradle --no-daemon :services:auth-service:shadowJar

# --- runtime stage -----------------------------------------------------------
FROM eclipse-temurin:21-jre AS runtime
WORKDIR /app
COPY --from=builder /build/services/auth-service/build/libs/auth-service-all.jar /app/auth-service.jar

EXPOSE 50051

# Logback writes to stdout via the bundled logback.xml; nothing extra needed.
ENTRYPOINT ["java", "-jar", "/app/auth-service.jar"]
