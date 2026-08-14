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
* Three backend services: **auth** (Kotlin), **geo** (Rust), **payments** (Kotlin)
* An **API gateway** (Rust/Axum) that terminates public HTTP, validates JWTs, and fans out to all three over gRPC
* gRPC contracts for every internal service (`proto/`)
* Kafka event schemas (`proto/events.proto`)
* Per-schema SQL migrations including PostGIS extensions
* Local development environment via `docker-compose`
* CI on GitHub Actions: fmt + clippy + tests for Rust, Gradle build for Kotlin, and a job that applies every migration against a real PostGIS instance

Still to come: the Kafka consumers (Phase 6), the control plane the CLI's
`--live` mode talks to (Phase 7), and the language SDKs.

### Building from source

Two system packages are required before `cargo build` will work, because
every Rust service runs `tonic-build` in its `build.rs`:

```bash
apt-get install -y protobuf-compiler cmake   # protoc for codegen, cmake for librdkafka
```

The Kotlin build needs only a JDK 21 — `protobuf-gradle-plugin` resolves
`protoc` from Maven rather than the system.

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

## HTTP API

The gateway is the only service exposed publicly. Everything behind it
speaks gRPC on a private network.

| Method | Path | Auth | Backend RPC |
|---|---|---|---|
| POST | `/v1/auth/register` | — | `auth.Register` |
| POST | `/v1/auth/login` | — | `auth.Authenticate` |
| POST | `/v1/auth/logout` | Bearer | `auth.RevokeToken` |
| GET | `/v1/auth/me` | Bearer | `auth.ValidateToken` |
| POST | `/v1/geo/locations` | Bearer | `geo.UpdateLocation` |
| GET | `/v1/geo/nearby` | Bearer | `geo.GetNearby` |
| POST | `/v1/geo/routes/score` | Bearer | `geo.ScoreRoute` |
| POST | `/v1/geo/geofences` | Bearer | `geo.CreateGeofence` |
| GET | `/v1/geo/geofences` | Bearer | `geo.ListGeofences` |
| DELETE | `/v1/geo/geofences/:id` | Bearer | `geo.DeleteGeofence` |
| POST | `/v1/geo/geofences/check` | Bearer | `geo.TriggerGeofenceCheck` |
| GET | `/v1/payments/wallet` | Bearer | `payments.GetWalletBalance` |
| POST | `/v1/payments/transactions` | Bearer | `payments.InitiateTransaction` |
| GET | `/healthz`, `/readyz` | — | — |

Authenticate with `Authorization: Bearer <token>` from `/v1/auth/login`.
Errors use one envelope, with a stable `code` for SDKs to branch on:

```json
{ "error": { "code": "invalid_argument", "message": "radius_m must be > 0" } }
```

**The identity rule:** no request body on this API has a `user_id` field.
Every `user_id` sent to a backend comes from the validated token, so a
caller can only ever read and write its own data. The backends trust that
guarantee — geo-engine takes `user_id` from the request body without
re-checking it — which is why the gateway must be the only route in.

Four RPCs are deliberately unrouted: `auth.IssueToken` and
`payments.DrainOutbox` (both marked internal in their `.proto`), plus
`payments.SettleTransaction` and `payments.RefundTransaction`, which take
a bare `transaction_id` with no ownership signal the gateway could check.
Settlement is meant to be driven by the Phase 6 fare-consumer reacting to
ride lifecycle events, not by a client call.

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
├── services/
│   ├── gateway/        # Public REST edge (Rust/Axum)
│   ├── auth-service/   # Identity and JWTs (Kotlin)
│   ├── geo-engine/     # PostGIS queries (Rust)
│   └── payments-service/ # Wallets and transactions (Kotlin)
├── consumers/          # Kafka consumers (planned)
├── .github/workflows/  # CI
├── infra/
│   ├── k8s/            # Kubernetes manifests including Kafka topics
│   └── terraform/      # GCP and GKE provisioning (planned)
├── atlas.toml.example  # Sample developer config
└── docker-compose.yml  # Local dev environment
```

## Author

Naing Lynn
