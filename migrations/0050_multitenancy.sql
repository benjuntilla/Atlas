-- Multi-tenancy, phase 1 of 2 (expand): every data-plane row belongs to a
-- project.
--
-- Until now the control plane was the only part of Atlas that knew tenants
-- existed. `control.projects` recorded that a developer had created
-- "checkout", but `auth.users` was a single flat table: one global user
-- namespace, one global wallet namespace, one global set of geofences. A
-- `nearby` query returned every user in the database regardless of which
-- customer's app asked. That is fine for one application and disqualifying
-- for a platform.
--
-- This migration adds `project_id` to every table whose rows belong to one
-- tenant, and — more importantly — fixes the three global uniqueness
-- constraints that made multiple tenants impossible rather than merely
-- unscoped:
--
--   auth.users.email                     was UNIQUE across the platform, so
--                                        two customers could not both have a
--                                        user named alice@example.com.
--   payments.transactions.idempotency_key was UNIQUE across the platform, so
--                                        two customers using "order-1" would
--                                        collide, and the second would be
--                                        handed the FIRST one's transaction.
--   payments.transaction_events.event_key same, for the audit log.
--
-- The idempotency one is the dangerous member of that set: a collision there
-- does not fail loudly, it silently returns someone else's payment.
--
-- # Why NOT NULL DEFAULT rather than NOT NULL
--
-- Every column here lands NOT NULL with a default pointing at a bootstrap
-- project, so existing rows have a home and existing INSERT statements keep
-- working unchanged. That makes this migration deployable on its own, ahead
-- of the services that will start setting project_id explicitly, instead of
-- requiring a flag-day where schema and every service change together.
--
-- The default is scaffolding, not the design. A default tenant is a footgun:
-- an INSERT that forgets project_id silently lands in the bootstrap tenant
-- instead of failing. Migration 0051 (contract) drops these defaults once
-- every writer sets the column explicitly, and that is when the column starts
-- actually enforcing anything.

-- Lets a GIST index put project_id in front of a geometry, so the tenant
-- filter and the spatial predicate are served by one index instead of the
-- planner scanning every tenant's geometry and filtering afterwards.
CREATE EXTENSION IF NOT EXISTS btree_gist;

-- ---------------------------------------------------------------------------
-- Bootstrap tenant
-- ---------------------------------------------------------------------------
-- Fixed UUIDs rather than generated ones: the backfill default below has to
-- name this row literally, and services and tests need to be able to refer to
-- it without querying for it first.
INSERT INTO control.accounts (id, email)
VALUES ('00000000-0000-0000-0000-000000000001', 'bootstrap@atlas.local')
ON CONFLICT DO NOTHING;

INSERT INTO control.projects (id, account_id, name, region, environment, endpoint)
VALUES (
    '00000000-0000-0000-0000-000000000002',
    '00000000-0000-0000-0000-000000000001',
    'bootstrap',
    'local',
    'development',
    ''
)
ON CONFLICT DO NOTHING;

-- ---------------------------------------------------------------------------
-- auth
-- ---------------------------------------------------------------------------
ALTER TABLE auth.users
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

-- The constraint this replaces is what made two tenants impossible.
ALTER TABLE auth.users DROP CONSTRAINT users_email_key;
ALTER TABLE auth.users
    ADD CONSTRAINT users_project_email_key UNIQUE (project_id, email);

-- auth.sessions deliberately has no project_id: it is only ever read by
-- user_id, user ids are UUIDs so they cannot collide across tenants, and the
-- row cascades away with its user. A denormalised copy would be one more
-- thing to keep true for no query that needs it.

-- ---------------------------------------------------------------------------
-- geo
-- ---------------------------------------------------------------------------
ALTER TABLE geo.locations
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

ALTER TABLE geo.geofences
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

ALTER TABLE geo.safety_votes
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

-- safety_ratings has no user_id — it scores road segments, which are shared
-- physical geography. It still gets a project_id, because the SCORE is not
-- shared: it is derived from one tenant's users' votes, and letting tenant A's
-- votes move the number tenant B reads would leak behaviour across the
-- boundary in both directions.
ALTER TABLE geo.safety_ratings
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

-- geo.geofence_memberships is keyed (user_id, geofence_id) and read by
-- user_id only; both sides are already tenant-scoped UUIDs, so it needs no
-- column of its own.

-- Composite spatial indexes: project first, geography second.
--
-- The single-tenant versions in 0022 are still correct but no longer
-- sufficient — with them the planner finds every nearby point on the planet
-- and then discards the ones belonging to other tenants, which is both slower
-- and exactly the shape of query you do not want load-bearing.
CREATE INDEX idx_geo_locations_project_position_geog
    ON geo.locations USING GIST (project_id, (position::geography));

CREATE INDEX idx_geo_geofences_project_center_geog
    ON geo.geofences USING GIST (project_id, (center::geography));

CREATE INDEX idx_safety_ratings_project_geom_geog
    ON geo.safety_ratings USING GIST (project_id, (segment_geom::geography));

CREATE INDEX idx_safety_votes_project_position_geog
    ON geo.safety_votes USING GIST (project_id, (position::geography));

-- ListGeofences filters by user within a project.
CREATE INDEX idx_geo_geofences_project_user_active
    ON geo.geofences(project_id, user_id) WHERE active = TRUE;

-- ---------------------------------------------------------------------------
-- payments
-- ---------------------------------------------------------------------------
ALTER TABLE payments.wallets
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

-- Referenced by the composite foreign keys on transactions below. A plain
-- UNIQUE(id) is implied by the primary key; this pair is what lets another
-- table say "this wallet, in this project".
ALTER TABLE payments.wallets
    ADD CONSTRAINT wallets_id_project_key UNIQUE (id, project_id);

ALTER TABLE payments.transactions
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

-- Scope idempotency to the tenant. Two customers using "order-1" is normal
-- and must not collide; before this, the second caller was handed the first
-- caller's transaction row.
ALTER TABLE payments.transactions DROP CONSTRAINT transactions_idempotency_key_key;
ALTER TABLE payments.transactions
    ADD CONSTRAINT transactions_project_idempotency_key
        UNIQUE (project_id, idempotency_key);

-- Make a cross-tenant transfer impossible in the database rather than merely
-- unreachable through the API.
--
-- Application code is where scoping bugs live: one query that forgets its
-- WHERE clause moves money between tenants. These composite keys mean the
-- wallet on each end must belong to the same project as the transaction, so
-- the mistake fails at COMMIT instead of succeeding quietly.
--
-- MATCH SIMPLE (the default) skips the check when any referencing column is
-- NULL, which is what makes deposits still work: a deposit is
-- `from_wallet IS NULL`, money arriving from outside, with no wallet to scope.
ALTER TABLE payments.transactions DROP CONSTRAINT transactions_from_wallet_fkey;
ALTER TABLE payments.transactions DROP CONSTRAINT transactions_to_wallet_fkey;
ALTER TABLE payments.transactions
    ADD CONSTRAINT transactions_from_wallet_project_fkey
        FOREIGN KEY (from_wallet, project_id)
        REFERENCES payments.wallets(id, project_id);
ALTER TABLE payments.transactions
    ADD CONSTRAINT transactions_to_wallet_project_fkey
        FOREIGN KEY (to_wallet, project_id)
        REFERENCES payments.wallets(id, project_id);

ALTER TABLE payments.transaction_events
    ADD COLUMN project_id UUID NOT NULL
        DEFAULT '00000000-0000-0000-0000-000000000002'
        REFERENCES control.projects(id) ON DELETE CASCADE;

-- The dedup key is a hash of fields the caller supplies (ride_id,
-- transaction_id, event_type, occurred_at). ride_id in particular is opaque
-- to Atlas, so two tenants can legitimately produce the same event_key, and
-- globally-unique dedup would drop the second tenant's audit row on the
-- floor.
DROP INDEX payments.idx_transaction_events_key;
CREATE UNIQUE INDEX idx_transaction_events_project_key
    ON payments.transaction_events(project_id, event_key);

-- The reconciliation sweep and the reporting reads are both per-tenant.
CREATE INDEX idx_transactions_project_kind
    ON payments.transactions(project_id, kind, created_at DESC);

-- payments.outbox has no project_id: the dispatcher drains it globally by
-- age, never per tenant, and each row cascades from its transaction.
