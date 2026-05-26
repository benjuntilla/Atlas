-- Enable PostGIS and uuid generation once at the instance level.
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Per-service schemas. Each service owns exactly one schema.
-- Cross-schema reads must be explicit, never hidden inside a service.
CREATE SCHEMA IF NOT EXISTS auth;
CREATE SCHEMA IF NOT EXISTS geo;
CREATE SCHEMA IF NOT EXISTS payments;
CREATE SCHEMA IF NOT EXISTS wayfinder;
CREATE SCHEMA IF NOT EXISTS haggle;
