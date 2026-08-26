# Multi-stage build, same shape as the geo-engine image.
#
# cmake is required because rdkafka builds librdkafka from source;
# protobuf-compiler is required because build.rs runs prost-build.

FROM rust:1.95 AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p atlas-safety-consumer

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/atlas-safety-consumer /usr/local/bin/atlas-safety-consumer
EXPOSE 9090
ENTRYPOINT ["/usr/local/bin/atlas-safety-consumer"]
