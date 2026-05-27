# Atlas

**Atlas is a developer platform for building location-aware, real-time, transactional apps.** Drop it into a mobility project and you get auth, geospatial queries, payments, and an event bus without configuring any of it yourself.

The platform exposes four namespaces that mirror four backend services:

* `atlas.auth` for JWT identity with optional geospatial claims
* `atlas.geo` for PostGIS-backed nearby search, route scoring, and geofencing
* `atlas.payments` for wallets, idempotent transactions, and settlement
* `atlas.events` for a protobuf-encoded Kafka event bus

A developer configures everything by dropping an `atlas.toml` in their project root, runs `atlas deploy`, and starts calling the SDK in their language of choice.

## Why this project exists

Most consumer mobility apps re-solve the same hard problems in isolation: identity, geospatial search, trust scoring, real-time messaging, and payments. Atlas is a study in what those problems look like when you build them once, as services, and ship them as a plug-and-play platform.

It is intentionally polyglot and event-driven. The systems work is the point.

## Status

In active development. The repository currently contains:

* A working Rust CLI (`atlas`) with config parsing, validation, deploy, status, logs, and key management
* gRPC contracts for every internal service (`proto/`)
* Kafka event schemas (`proto/events.proto`)
* Per-schema SQL migrations including PostGIS extensions
* Local development environment via `docker-compose`

Backend services and the language SDKs (TypeScript, Dart, Rust) are being built in ordered phases.

## Technical scope

| Area | Technology |
|---|---|
| Backend services | Rust (Axum, Tonic) and Kotlin (Ktor) |
| Developer CLI | Rust (clap, reqwest, tokio) |
| Inter-service RPC | gRPC with shared `.proto` contracts |
| Event bus | Apache Kafka with protobuf-encoded payloads |
| Database | PostgreSQL 15 with PostGIS (per-service schemas, single instance) |
| SDKs | TypeScript, Dart, Rust (planned) |
| Infrastructure | Terraform, Kubernetes on GKE, Docker Compose for local dev |
| Observability | Prometheus metrics, structured JSON logging |
| CI/CD | GitHub Actions |

## Developer surface

A new project starts with a single config file:

```toml
[project]
name        = "my-mobility-app"
api_key     = "atl_live_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "production"
region      = "us-central1"

[services]
auth     = true
geo      = true
payments = true
events   = true
```

Then the CLI takes over:

```bash
atlas validate    # parse and validate atlas.toml
atlas deploy      # provision services with the Atlas control plane
atlas status      # show current service health
atlas logs auth   # tail logs for a given service
atlas keys list   # manage API keys
```

The CLI defaults to an in-memory mock transport so it is fully usable while the control plane is being built. Pass `--live` once the backend ships.

## Architecture

```
                    +-----------------+
       HTTPS        |   API Gateway   |   Rust / Axum
   ---------------->|   (port 8080)   |   JWT validation, REST to gRPC fanout
                    +-------+---------+
                            |
        +-------------------+--------------------+
        v                   v                    v
  +-----------+      +------------+      +------------+
  |   auth    |      |    geo     |      |  payments  |
  | (Kotlin)  |      |   (Rust)   |      |  (Kotlin)  |
  +-----+-----+      +-----+------+      +-----+------+
        |                  |                   |
        +------------------+-------------------+
                           v
                  +-----------------+
                  |     Kafka       |
                  +-------+---------+
                          |
        +-----------------+-----------------+
        v                 v                 v
  +-----------+    +------------+    +------------+
  | location  |    |   safety   |    |    fare    |
  | consumer  |    |  consumer  |    |  consumer  |
  +-----------+    +------------+    +------------+
```

Six services, three consumers. Every service owns one database schema. Synchronous communication uses gRPC, asynchronous communication uses Kafka.

## Repo layout

```
atlas/
├── cli/                # Rust developer CLI (produces the `atlas` binary)
├── proto/              # gRPC contracts and Kafka event schemas
├── migrations/         # Numbered SQL files, applied in order
├── services/           # Backend services (planned)
├── consumers/          # Kafka consumers (planned)
├── infra/
│   ├── k8s/            # Kubernetes manifests including Kafka topics
│   └── terraform/      # GCP and GKE provisioning (planned)
├── atlas.toml.example  # Sample developer config
└── docker-compose.yml  # Local dev environment
```

## Author

Naing Lynn
