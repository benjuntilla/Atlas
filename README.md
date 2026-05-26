# Atlas

Real-time urban mobility and safety platform. Atlas is a shared trust layer
that Wayfinder (safety navigation) and Haggle (fare negotiation) both run on
top of. One identity, one location context, one safety graph.

> **Phase 1 only.** The repository currently contains the monorepo scaffold,
> gRPC contracts, Kafka event schemas, SQL migrations, and the local
> docker-compose environment. Service implementations land in subsequent
> phases.

## Repo layout

```
atlas/
├── proto/              # gRPC contracts + Kafka event schemas (events.proto)
├── migrations/         # Numbered SQL files, applied in order
├── services/           # gateway, geo-engine, auth, payments, wayfinder, haggle
├── consumers/          # location, fare, safety Kafka consumers
├── clients/            # wayfinder-web (React), haggle-mobile (Flutter)
├── infra/
│   ├── k8s/            # Kubernetes manifests, including kafka-topics.yaml
│   └── terraform/      # GCP / GKE provisioning
└── docker-compose.yml  # Local dev environment
```

## Local setup (Phase 1)

```bash
# Bring up Postgres + Kafka. Migrations apply on first boot via
# the docker-entrypoint-initdb.d mount.
docker compose up -d postgres kafka

# Watch migrations apply.
docker compose logs postgres | grep -i 'CREATE\|migration'

# Verify schemas exist.
psql postgres://atlas:atlas_dev@localhost:5432/atlas -c '\dn'
# Expected: auth, geo, payments, wayfinder, haggle (plus public)

# Verify PostGIS is enabled and tables landed.
psql postgres://atlas:atlas_dev@localhost:5432/atlas -c \
  "SELECT schemaname, tablename FROM pg_tables
   WHERE schemaname IN ('auth','geo','payments','wayfinder','haggle')
   ORDER BY schemaname, tablename;"
```

Postgres data is persisted in the `atlas_pg_data` named volume. To start
from a clean slate (re-run all migrations):

```bash
docker compose down -v
docker compose up -d postgres
```

## Architecture (planned)

| Service | Language | Port | Owns schema | Phase |
|---|---|---|---|---|
| gateway | Rust (Axum) | 8080 | — | 5 |
| auth-service | Kotlin (Ktor) | 50051 | `auth` | 2 |
| geo-engine | Rust (Axum + tonic) | 50052 | `geo` | 3 |
| payments-service | Kotlin (Ktor) | 50053 | `payments` | 4 |
| wayfinder | Rust (Axum) | 50054 | `wayfinder` | 7 |
| haggle | Kotlin (Ktor) | 50055 | `haggle` | 8 |
| location-consumer | Rust | — | reads `geo`, `wayfinder` | 6 |
| fare-consumer | Kotlin | — | writes `payments` | 6 |
| safety-consumer | Rust | — | reads `wayfinder`, `geo` | 6 |

## Kafka topics

All payloads are protobuf-encoded. Schemas live in [`proto/events.proto`](proto/events.proto).

| Topic | Producer | Consumers |
|---|---|---|
| `atlas.location.updates` | geo-engine | location-consumer, safety-consumer |
| `atlas.fare.events` | haggle, payments-service | fare-consumer |
| `atlas.auth.tokens` | auth-service | gateway (cache invalidation) |
| `atlas.safety.alerts` | safety-consumer | wayfinder |
| `atlas.elo.recompute` | location-consumer | geo-engine |

## Build order

1. **Phase 1 — Foundations** *(done)* — scaffold, protos, migrations, docker-compose
2. Phase 2 — Auth service (Kotlin)
3. Phase 3 — Geo engine (Rust)
4. Phase 4 — Payments service (Kotlin)
5. Phase 5 — API gateway (Rust)
6. Phase 6 — Kafka consumers
7. Phase 7 — Wayfinder backend
8. Phase 8 — Haggle backend
9. Phase 9 — Terraform + GKE
10. Phase 10 — Kubernetes manifests
11. Phase 11 — CI/CD
12. Phase 12 — Observability
13. Phase 13 — README expansion + load test results
