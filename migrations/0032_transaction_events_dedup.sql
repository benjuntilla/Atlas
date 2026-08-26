-- Dedup key for the fare-consumer's audit log.
--
-- payments.transaction_events was created in 0030 "for fare-consumer" but
-- nothing has ever written to it. Now that the consumer exists, it needs a
-- way to not write the same row twice: Kafka delivery is at-least-once, so
-- a rebalance or a crash between acting on an event and committing its
-- offset replays that event. The money operations tolerate this already
-- (SettleTransaction on a settled transaction returns success without
-- moving anything), but an audit log that silently doubles its rows is
-- worse than useless — it is misleading.
--
-- event_key is a hash over the identifying fields of a FareEvent
-- (ride_id, transaction_id, event_type, occurred_at). Inserting with
-- ON CONFLICT DO NOTHING makes a replayed event a no-op.
--
-- Added in three steps rather than as a single NOT NULL column so this
-- migration is safe against a database that somehow does have rows —
-- ADD COLUMN ... NOT NULL with no default fails outright when it does.

ALTER TABLE payments.transaction_events
    ADD COLUMN event_key TEXT;

-- Backfill any pre-existing row with something unique to itself, so the
-- NOT NULL below can be applied. In practice this updates zero rows.
UPDATE payments.transaction_events
SET event_key = 'legacy:' || id::text
WHERE event_key IS NULL;

ALTER TABLE payments.transaction_events
    ALTER COLUMN event_key SET NOT NULL;

CREATE UNIQUE INDEX idx_transaction_events_key
    ON payments.transaction_events(event_key);
