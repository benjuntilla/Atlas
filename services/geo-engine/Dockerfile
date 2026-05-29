# Multi-stage build keeps the runtime image lean (~80MB) by leaving the
# Rust toolchain in the builder layer. librdkafka is built from source by
# rdkafka's build script — that needs cmake and a C++ compiler in the
# builder. The runtime only needs ca-certificates and libgcc/libstdc++
# which debian:bookworm-slim already provides.

FROM rust:1.95 AS build
WORKDIR /src
# Install librdkafka build deps. cmake is the big one; rest are tiny.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
# Copy the whole workspace because Cargo workspace builds need the full
# tree (member paths in Cargo.toml are relative to the workspace root).
COPY . .
RUN cargo build --release -p atlas-geo-engine

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/atlas-geo-engine /usr/local/bin/atlas-geo-engine
EXPOSE 50052 9090
ENTRYPOINT ["/usr/local/bin/atlas-geo-engine"]
