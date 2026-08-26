# Multi-stage build, same shape as the geo-engine image.
#
# Unlike geo-engine this does NOT need cmake: the gateway has no rdkafka
# dependency (it never touches Kafka), so protoc is the only build tool
# beyond the Rust toolchain.

FROM rust:1.95 AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
# Copy the whole workspace: Cargo workspace member paths are relative to
# the workspace root, and build.rs reads from /proto.
COPY . .
RUN cargo build --release -p atlas-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/atlas-gateway /usr/local/bin/atlas-gateway
# 8080 public REST, 9090 Prometheus scrape (never publish 9090 publicly).
EXPOSE 8080 9090
ENTRYPOINT ["/usr/local/bin/atlas-gateway"]
