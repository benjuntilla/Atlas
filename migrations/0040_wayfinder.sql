CREATE TABLE wayfinder.routes (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id      UUID NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    origin       GEOMETRY(Point, 4326) NOT NULL,
    destination  GEOMETRY(Point, 4326) NOT NULL,
    chosen_path  GEOMETRY(LineString, 4326),
    safety_score DOUBLE PRECISION,
    started_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    arrived_at   TIMESTAMPTZ
);

CREATE INDEX idx_wayfinder_routes_user_active
    ON wayfinder.routes(user_id) WHERE arrived_at IS NULL;

CREATE TABLE wayfinder.geofences (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    route_id     UUID NOT NULL REFERENCES wayfinder.routes(id) ON DELETE CASCADE,
    center       GEOMETRY(Point, 4326) NOT NULL,
    radius_m     DOUBLE PRECISION NOT NULL DEFAULT 50.0,
    triggered    BOOLEAN NOT NULL DEFAULT FALSE,
    triggered_at TIMESTAMPTZ
);

CREATE INDEX idx_wayfinder_geofences_center   ON wayfinder.geofences USING GIST(center);
-- Required by safety-consumer JOIN on route_id; missing from the original spec.
CREATE INDEX idx_wayfinder_geofences_route_id ON wayfinder.geofences(route_id);
CREATE INDEX idx_wayfinder_geofences_active
    ON wayfinder.geofences(route_id) WHERE triggered = FALSE;
