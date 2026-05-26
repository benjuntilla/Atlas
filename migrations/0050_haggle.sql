CREATE TABLE haggle.rides (
    id               UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    rider_id         UUID NOT NULL REFERENCES auth.users(id),
    driver_id        UUID REFERENCES auth.users(id),
    pickup           GEOMETRY(Point, 4326) NOT NULL,
    dropoff          GEOMETRY(Point, 4326) NOT NULL,
    status           TEXT NOT NULL DEFAULT 'negotiating'
                     CHECK (status IN ('negotiating', 'accepted', 'in_progress',
                                       'completed', 'cancelled')),
    final_fare_cents BIGINT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at     TIMESTAMPTZ
);

CREATE INDEX idx_haggle_rides_status ON haggle.rides(status);
CREATE INDEX idx_haggle_rides_pickup ON haggle.rides USING GIST(pickup);
CREATE INDEX idx_haggle_rides_rider  ON haggle.rides(rider_id);
CREATE INDEX idx_haggle_rides_driver ON haggle.rides(driver_id);

CREATE TABLE haggle.bids (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    ride_id      UUID NOT NULL REFERENCES haggle.rides(id) ON DELETE CASCADE,
    bidder_id    UUID NOT NULL REFERENCES auth.users(id),
    role         TEXT NOT NULL CHECK (role IN ('rider', 'driver')),
    amount_cents BIGINT NOT NULL,
    -- Bids transition from pending -> accepted or pending -> superseded
    -- when a competing bid wins or the ride moves out of negotiating.
    status       TEXT NOT NULL DEFAULT 'pending'
                 CHECK (status IN ('pending', 'accepted', 'superseded', 'expired')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at   TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_haggle_bids_ride_id ON haggle.bids(ride_id);
CREATE INDEX idx_haggle_bids_status  ON haggle.bids(status);
