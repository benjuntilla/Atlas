CREATE TABLE geo.locations (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id     UUID NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    position    GEOMETRY(Point, 4326) NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- TTL enforced by location-consumer; default 24h gives the consumer
    -- a window to run without dropping data on lazy clients.
    expires_at  TIMESTAMPTZ NOT NULL DEFAULT (NOW() + INTERVAL '24 hours')
);

CREATE INDEX idx_geo_locations_position ON geo.locations USING GIST(position);
CREATE INDEX idx_geo_locations_user_id  ON geo.locations(user_id);
CREATE INDEX idx_geo_locations_expires  ON geo.locations(expires_at);

CREATE TABLE geo.safety_ratings (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    segment_geom  GEOMETRY(LineString, 4326) NOT NULL,
    elo_score     DOUBLE PRECISION NOT NULL DEFAULT 1500.0,
    vote_count    INTEGER NOT NULL DEFAULT 0,
    last_computed TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_safety_ratings_geom ON geo.safety_ratings USING GIST(segment_geom);

-- Required by the ELO recompute job in location-consumer (Phase 8).
-- Missing from the original spec; added here so the recompute SQL can run.
CREATE TABLE geo.safety_votes (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id    UUID NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    position   GEOMETRY(Point, 4326) NOT NULL,
    vote       TEXT NOT NULL CHECK (vote IN ('safe', 'unsafe')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_safety_votes_position ON geo.safety_votes USING GIST(position);
CREATE INDEX idx_safety_votes_created  ON geo.safety_votes(created_at);
