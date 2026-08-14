# Multi-stage build, same shape as the gateway image.
#
# No cmake: the control plane has no rdkafka dependency — it checks Kafka
# reachability with a plain TCP connect rather than a client. protoc is
# still needed because tonic-health pulls in prost-generated code.

FROM rust:1.95 AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
# Cargo workspace member paths are relative to the workspace root.
COPY . .
RUN cargo build --release -p atlas-control-plane

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/atlas-control-plane /usr/local/bin/atlas-control-plane
# 8081 REST (the CLI's default base URL), 9090 Prometheus scrape.
EXPOSE 8081 9090
ENTRYPOINT ["/usr/local/bin/atlas-control-plane"]
