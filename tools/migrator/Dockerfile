# The migration image. Runs to completion and exits — as a Kubernetes Job
# before a rollout, or a one-shot service in docker-compose.
#
# The SQL is embedded in the binary by `sqlx::migrate!` at compile time, so
# the runtime image ships no .sql files and the schema cannot drift from
# the binary that applies it.

FROM rust:1.95 AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p atlas-migrator

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/atlas-migrate /usr/local/bin/atlas-migrate
# No EXPOSE: this listens on nothing. It runs, applies, and exits.
ENTRYPOINT ["/usr/local/bin/atlas-migrate"]
CMD ["run"]
