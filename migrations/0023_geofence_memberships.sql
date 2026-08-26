-- Current geofence membership, maintained by the safety-consumer.
--
-- `geo.geofences` says which fences exist; this says which ones each user
-- is inside *right now*. The safety-consumer needs that because a
-- geofence alert is about a TRANSITION, not a state: entering a fence is
-- an event, being inside one is not. Without somewhere to remember the
-- previous membership set, every location ping inside a fence would
-- re-emit GEOFENCE_ENTERED forever.
--
-- Why a table rather than in-memory state in the consumer:
--   1. It survives restarts and rebalances. In-memory state would replay
--      a storm of spurious ENTERED alerts every deploy.
--   2. It makes the consumer horizontally scalable — two instances
--      handling different partitions share one view of the world.
--   3. It makes reprocessing idempotent. Re-handling an already-applied
--      location ping computes an empty diff and emits nothing, which is
--      what makes at-least-once Kafka delivery safe here.
--
-- The composite primary key is the dedup mechanism: a user can only be
-- inside a given fence once.

CREATE TABLE geo.geofence_memberships (
    user_id     UUID NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    geofence_id UUID NOT NULL REFERENCES geo.geofences(id) ON DELETE CASCADE,
    entered_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, geofence_id)
);

-- The consumer's hot path reads every membership for one user per ping.
CREATE INDEX idx_geofence_memberships_user ON geo.geofence_memberships(user_id);
