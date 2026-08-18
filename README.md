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
* Three **Kafka consumers**: location (retention), safety (geofence alerts), fare (settlement)
* A **control plane** (Rust/Axum) backing the CLI: accounts, project provisioning, API keys, live status, and an audit trail
* gRPC contracts for every internal service (`proto/`)
* Kafka event schemas (`proto/events.proto`)
* Per-schema SQL migrations including PostGIS extensions
* Local development environment via `docker-compose`
* CI on GitHub Actions: fmt + clippy + tests for Rust, Gradle build for Kotlin, and a job that applies every migration against a real PostGIS instance
* A **migration runner** (`atlas-migrate`) with recorded history, so schema changes can be applied to a running database
* **Kubernetes manifests** for the whole platform — Deployments, HPAs, PDBs, NetworkPolicies, Ingress — validated in CI against real API schemas
* **Container images** for all nine workloads, built and pushed to GHCR on every push
* **Terraform** for GCP: VPC, GKE with Dataplane V2, private Cloud SQL with PostGIS, Secret Manager, Workload Identity
* A **TypeScript SDK** (`sdks/typescript`) with zero runtime dependencies, covering the whole gateway surface

Still to come: the Dart and Rust SDKs.

### Database migrations

Migrations used to be mounted into the Postgres container at
`/docker-entrypoint-initdb.d`. That directory runs **only on a database's
first boot**, which meant a new migration was silently skipped by anyone
who already had a volume, nothing recorded what had been applied, and there
was no way at all to change the schema of a running database.

`tools/migrator` replaces it. It records applied versions in
`_sqlx_migrations`, applies only what is pending, wraps each migration in a
transaction, and verifies checksums so an applied file cannot be edited out
from under a deployed environment.

```bash
docker compose up -d          # migrator runs first; services wait for it
docker compose run --rm migrator status   # applied vs pending
```

Services never migrate on startup — with several replicas booting at once
that is a race, and a failed migration would crash-loop every pod instead
of failing one Job. In Kubernetes it is a `Job`, and each service blocks on
an init container running `atlas-migrate status --check`.

**If you have a volume from before this landed**, its schema came from the
old initdb path and has no recorded history. Adopt it once:

```bash
docker compose run --rm migrator baseline --through 40 \
    --i-understand-this-skips-sql
```

`--through` has no default on purpose: the tool cannot verify which
migrations a legacy database actually received, and guessing would silently
skip real schema changes. Or just start clean with `docker compose down -v`.

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
| SDKs | TypeScript (shipped); Dart and Rust planned |
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

The CLI still defaults to an in-memory mock transport, so it works with no
backend at all. Pass `--live` to talk to a real control plane.

### Going live

The control plane runs on 8081, which is the CLI's default base URL. Start
it, then mint the account key that `atlas.toml` needs — this is the one
bootstrap step, because every other route requires a key and `atlas deploy`
cannot create the account that would hold one:

```bash
docker compose up -d postgres control-plane

curl -sX POST http://localhost:8081/v1/accounts \
  -H 'content-type: application/json' \
  -d '{"email":"you@example.com"}'
# => {"account_id":"...","api_key":"atl_dev_...","prefix":"atl_dev_...", ...}
```

Paste that `api_key` into `atlas.toml` as `project.api_key` — it is shown
once and is not recoverable — then:

```bash
atlas deploy --live      # creates the project; idempotent on re-run
atlas status --live      # live gRPC health probes + real gateway metrics
atlas keys create ci --expiry 90d --live
atlas logs --live        # the project's audit trail
```

`POST /v1/accounts` is the only unauthenticated write in the platform and
has no email verification or rate limit. Keep the control plane on a
private network until it does.

### What `atlas status` actually measures

`healthy` is a live probe on every call — a gRPC `Health/Check` against
auth, geo, and payments, and a TCP connect to Kafka for `events`. Nothing
is cached, so a service that dies is `DOWN` on the next invocation.

The three numeric columns are parsed from the gateway's Prometheus
endpoint, which is the only place per-request data exists. Two honest
caveats: `requests_24h` is really "since the gateway process started"
(a true 24-hour window needs a time-series database to difference the
counter, which is Prometheus' job, not this service's), and `events` has
no gateway routes so its counters are always zero — only its health means
anything.

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
| POST | `/v1/payments/deposits` | Bearer | `payments.Deposit` |
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

### Payments and the placeholder provider

`atlas.payments` runs against `FakePaymentProvider`, which approves every
charge and mints `fake_*` references. That placeholder covers the **provider
network call only**. Everything around it is real and exercised:
idempotency, the pending-then-capture ordering that keeps a crash
recoverable, the transactional outbox, the ledger updates, and webhook
signature verification.

Deposits are the only way money enters the platform — before them wallets
sat at zero and settlement refused to move funds that were not there, so no
transaction could ever complete.

Swapping in a real processor means implementing `PaymentProvider` and
setting `PAYMENT_PROVIDER`. Nothing above the interface changes. An unknown
value fails at startup rather than silently falling back to the fake, which
in production would approve charges against money never collected.

**Do not point this at real money until** a provider is implemented, the
webhook handler reconciles against `payments.transactions`, and a sweep
exists for deposits left pending by a crash between capture and credit
(migration 0033 indexes exactly those rows).

Four RPCs are deliberately unrouted: `auth.IssueToken` and
`payments.DrainOutbox` (both marked internal in their `.proto`), plus
`payments.SettleTransaction` and `payments.RefundTransaction`, which take
a bare `transaction_id` with no ownership signal the gateway could check.
Settlement is meant to be driven by the Phase 6 fare-consumer reacting to
ride lifecycle events, not by a client call.

## Event flow

Synchronous calls go through the gateway; everything asynchronous goes
through Kafka. Each topic has exactly one producer.

| Topic | Produced by | Consumed by | What the consumer does |
|---|---|---|---|
| `atlas.location.updates` | geo-engine | location-consumer | Enforces the 24h retention window on `geo.locations` |
| | | safety-consumer | Diffs geofence membership and emits crossings |
| `atlas.safety.alerts` | safety-consumer | *(developer's app)* | Geofence entry/exit, for the app to act on |
| `atlas.fare.events` | payments (outbox) | fare-consumer | Settles on `RIDE_COMPLETED`, refunds on `RIDE_CANCELLED` |
| `atlas.auth.tokens` | auth-service | auth-service | Cache invalidation fanout on revocation |
| `atlas.elo.recompute` | — | — | Not yet wired; see below |

Two things are worth knowing about this table.

**Settlement is event-driven, not an API call.** `POST
/v1/payments/transactions` creates a pending transaction and payments
writes a `RIDE_ACCEPTED` event through its outbox. The money moves later,
when the application publishes `RIDE_COMPLETED` or `RIDE_CANCELLED` and
fare-consumer calls `SettleTransaction` / `RefundTransaction`. This is why
the gateway exposes no settle endpoint — see the note in the HTTP API
section.

**fare-consumer ignores payments' own events.** Payments publishes
`TRANSACTION_SETTLED` and `TRANSACTION_REFUNDED` onto the same topic
fare-consumer reads. Those are acknowledgements, not instructions; acting
on them would settle in response to having settled, forever. They are
recorded in the audit log and nothing else.

`atlas.elo.recompute` and the `geo.safety_votes` table are still
unconnected: there is no API for a user to cast a safety vote, so nothing
produces the event and the ELO scores in `safety_ratings` never change.
`GetNearby` therefore returns the neutral 1500.0 for every user. That is a
known gap, not an oversight in this phase.

## SDK

`sdks/typescript` wraps the HTTP API above. No runtime dependencies — it
uses the platform `fetch`.

```ts
import { AtlasClient, AtlasError } from '@atlas/sdk';

const atlas = new AtlasClient({ baseUrl: 'https://api.atlas.dev' });
await atlas.auth.login({ email, password });   // token stored on the client

const { users } = await atlas.geo.nearby({ lat, lng, radiusM: 500 });
const { balanceCents } = await atlas.payments.wallet();
```

Two things it inherits from the API design. No method takes a user id —
identity comes from the token, mirroring the gateway's own rule — and
`payments.createTransaction` always sends an idempotency key, which is
what makes it the only POST the transport will retry. Errors arrive as
`AtlasError` carrying the stable `code` from the envelope.

See `sdks/typescript/README.md` for the retry policy and the full surface.

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
│   ├── control-plane/  # Projects, API keys, status (Rust/Axum)
│   ├── auth-service/   # Identity and JWTs (Kotlin)
│   ├── geo-engine/     # PostGIS queries (Rust)
│   └── payments-service/ # Wallets and transactions (Kotlin)
├── consumers/
│   ├── location-consumer/ # Location retention (Rust)
│   ├── safety-consumer/   # Geofence alerts (Rust)
│   └── fare-consumer/     # Settlement (Kotlin)
├── tools/
│   └── migrator/       # Schema migration runner (Rust)
├── sdks/
│   └── typescript/     # @atlas/sdk — TypeScript client
├── .github/workflows/  # CI
├── infra/
│   ├── k8s/
│   │   ├── base/       # Deployments, HPAs, PDBs, NetworkPolicies, Ingress
│   │   ├── overlays/   # dev (1 replica) and prod (zone spread)
│   │   └── kafka-topics.yaml  # Strimzi KafkaTopic CRDs
│   └── terraform/      # VPC, GKE, Cloud SQL, Secret Manager, IAM
├── atlas.toml.example  # Sample developer config
└── docker-compose.yml  # Local dev environment
```

## Author

Naing Lynn
