-- Geofences are a first-class platform feature owned by the geo schema.
-- Replaces the dropped wayfinder.geofences table — geofences now belong
-- to a user_id directly (not a route), matching the pure-platform framing
-- where the developer decides how to use them.

CREATE TABLE geo.geofences (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id     UUID NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    label       TEXT,
    center      GEOMETRY(Point, 4326) NOT NULL,
    radius_m    DOUBLE PRECISION NOT NULL DEFAULT 50.0,
    active      BOOLEAN NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- GIST index supports ST_DWithin for the trigger-check path.
CREATE INDEX idx_geo_geofences_center ON geo.geofences USING GIST(center);

-- Partial index keeps the active-only ListGeofences path narrow even when
-- soft-deleted rows accumulate.
CREATE INDEX idx_geo_geofences_user_active
    ON geo.geofences(user_id) WHERE active = TRUE;
