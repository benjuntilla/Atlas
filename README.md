# Atlas

**Atlas is a real-time urban mobility and safety platform.** It acts as a
shared trust layer for two consumer products: **Wayfinder**, a safety-aware
walking navigation app, and **Haggle**, a fare-negotiation rideshare app.
Both run on top of the same identity, location, and safety graph.

The core idea: when a user opens Haggle to book a ride, the platform already
knows their location from Wayfinder, has already scored the surrounding
streets using a shared ELO-based safety rating, and settles the fare through
the same payments rail. One identity, one location context, one safety graph.

## Why this project exists

Most consumer mobility apps re-solve the same hard problems in isolation —
identity, geospatial search, trust scoring, real-time messaging, payments —
and don't share signal across products. Atlas is a study in what those
problems look like when you build them once, as services, and let multiple
product surfaces consume them.

It is intentionally polyglot and event-driven: the systems work was the
point, not the product.

## Technical scope

| Area | Technology |
|---|---|
| Backend services | **Rust** (Axum, Tonic) and **Kotlin** (Ktor) |
| Inter-service RPC | **gRPC** with shared `.proto` contracts |
| Event bus | **Apache Kafka** with protobuf-encoded payloads |
| Database | **PostgreSQL 15** with **PostGIS** (per-service schemas, single instance) |
| Clients | **React + TypeScript + Mapbox GL JS**, **Flutter + Dart + Riverpod** |
| Infrastructure | **Terraform**, **Kubernetes** (GKE), **Docker Compose** for local dev |
| Observability | **Prometheus** metrics, structured JSON logging, distributed tracing |
| CI/CD | **GitHub Actions** |

## Architecture at a glance

```
                    ┌─────────────────┐
       HTTPS        │   API Gateway   │   Rust / Axum
   ────────────────▶│   (port 8080)   │   JWT validation, REST → gRPC fanout
                    └────────┬────────┘
                             │ gRPC
        ┌────────────────────┼────────────────────┐
        ▼                    ▼                    ▼
  ┌───────────┐       ┌────────────┐       ┌────────────┐
  │   auth    │       │    geo     │       │  payments  │
  │ (Kotlin)  │       │   (Rust)   │       │  (Kotlin)  │
  └─────┬─────┘       └──────┬─────┘       └─────┬──────┘
        │                    │                   │
        └────────────────────┼───────────────────┘
                             ▼
                    ┌─────────────────┐
                    │     Kafka       │
                    └────────┬────────┘
                             │
        ┌────────────────────┼────────────────────┐
        ▼                    ▼                    ▼
  ┌───────────┐       ┌────────────┐       ┌────────────┐
  │ location  │       │   safety   │       │    fare    │
  │ consumer  │       │  consumer  │       │  consumer  │
  └───────────┘       └────────────┘       └────────────┘

  Products that consume the platform:
    Wayfinder (Rust backend + React web client)
    Haggle    (Kotlin backend + Flutter mobile client)
```

Six services, three consumers, two client apps. Every service owns one
database schema. Cross-service communication happens through gRPC for
synchronous calls and Kafka for asynchronous events.

## Status

In active development. The repository currently contains the gRPC
contracts, Kafka event schemas, database migrations, and the local
development environment. Service implementations are being built in
ordered phases.

## Author

Naing Lynn — building Atlas to demonstrate distributed systems work across
Rust, Kotlin, and cloud infrastructure.
