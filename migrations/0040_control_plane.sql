-- Control plane: the backend the `atlas` CLI and the dashboard talk to.
--
-- The schema is created here rather than in 0001 because 0001 only runs on
-- a database's first boot; adding it there would leave every existing
-- volume without it.
CREATE SCHEMA IF NOT EXISTS control;

-- An account owns projects and keys. Created by POST /v1/accounts, which
-- is the only unauthenticated write in the control plane — it is the
-- bootstrap that mints the first key a developer pastes into atlas.toml.
CREATE TABLE control.accounts (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email      TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Project names are globally unique, not per-account: they appear in the
-- endpoint URL (https://api.atlas.dev/v1/<name>), so two accounts cannot
-- both own "checkout". The CLI addresses projects by name alone, which
-- only works if the name is unambiguous.
CREATE TABLE control.projects (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    account_id  UUID NOT NULL REFERENCES control.accounts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL UNIQUE,
    region      TEXT NOT NULL,
    environment TEXT NOT NULL
                CHECK (environment IN ('development', 'staging', 'production')),
    endpoint    TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_control_projects_account ON control.projects(account_id);

-- API keys.
--
-- `key_hash` is the SHA-256 of the full key and is what we look up on.
-- The plaintext key is returned exactly once, at creation, and is never
-- recoverable afterwards — same contract as every other platform's keys.
--
-- A NULL project_id means an account-scoped key: it can create projects
-- and manage every project in its account. That is the key `atlas deploy`
-- uses, because at deploy time the project may not exist yet. A non-NULL
-- project_id scopes the key to exactly one project.
CREATE TABLE control.api_keys (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    account_id   UUID NOT NULL REFERENCES control.accounts(id) ON DELETE CASCADE,
    project_id   UUID REFERENCES control.projects(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    -- Display form: scheme plus the first 4 secret chars, e.g. atl_live_9f3c.
    -- Not unique on its own; uniqueness lives on key_hash.
    key_prefix   TEXT NOT NULL,
    key_hash     TEXT NOT NULL UNIQUE,
    status       TEXT NOT NULL DEFAULT 'active'
                 CHECK (status IN ('active', 'revoked')),
    expires_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ
);

-- Every authenticated request hashes the bearer token and looks it up
-- here, so this index is on the hottest path in the service.
CREATE UNIQUE INDEX idx_control_api_keys_hash ON control.api_keys(key_hash);
CREATE INDEX idx_control_api_keys_project ON control.api_keys(project_id);
CREATE INDEX idx_control_api_keys_account ON control.api_keys(account_id);
-- `atlas keys revoke <prefix>` addresses keys by prefix within a project.
CREATE INDEX idx_control_api_keys_prefix ON control.api_keys(project_id, key_prefix);

-- Which of the four namespaces a project has turned on. Rewritten on every
-- deploy, so it always reflects the last applied atlas.toml.
CREATE TABLE control.project_services (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id     UUID NOT NULL REFERENCES control.projects(id) ON DELETE CASCADE,
    service        TEXT NOT NULL
                   CHECK (service IN ('auth', 'geo', 'payments', 'events')),
    enabled        BOOLEAN NOT NULL DEFAULT TRUE,
    status         TEXT NOT NULL DEFAULT 'ok'
                   CHECK (status IN ('ok', 'skipped', 'failed')),
    detail         TEXT,
    provisioned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, service)
);

-- Deploy history. `atlas deploy` is idempotent by project name, so this is
-- how you tell a re-deploy from a first provision.
CREATE TABLE control.deployments (
    id                 UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id         UUID NOT NULL REFERENCES control.projects(id) ON DELETE CASCADE,
    services_requested TEXT[] NOT NULL,
    status             TEXT NOT NULL DEFAULT 'ok',
    elapsed_ms         BIGINT NOT NULL DEFAULT 0,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_control_deployments_project ON control.deployments(project_id, created_at DESC);

-- Audit trail, and the backing store for `atlas logs`.
--
-- This is deliberately NOT application log shipping. It records what the
-- control plane itself did — deploys, key issuance, key revocation — which
-- is real, attributable data it owns. Streaming stdout from auth-service
-- or geo-engine needs a log aggregator (Loki, Cloud Logging) sitting in
-- front of the cluster; that is infrastructure, not a table.
CREATE TABLE control.audit_events (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id      UUID REFERENCES control.projects(id) ON DELETE CASCADE,
    account_id      UUID NOT NULL REFERENCES control.accounts(id) ON DELETE CASCADE,
    -- Which key performed the action, by prefix. Never the key itself.
    actor_key_prefix TEXT,
    service         TEXT NOT NULL DEFAULT 'control-plane',
    level           TEXT NOT NULL DEFAULT 'info'
                    CHECK (level IN ('info', 'warn', 'error')),
    action          TEXT NOT NULL,
    message         TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_control_audit_project ON control.audit_events(project_id, created_at DESC);
CREATE INDEX idx_control_audit_service ON control.audit_events(project_id, service, created_at DESC);
