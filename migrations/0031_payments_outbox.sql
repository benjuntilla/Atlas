-- Transactional outbox for exactly-once-effective Kafka publishing.
--
-- On every transaction state change, the payments-service writes a FareEvent
-- payload to this table in the SAME jdbc transaction as the transactions-row
-- update. A background dispatcher claims rows with FOR UPDATE SKIP LOCKED,
-- publishes to Kafka, and marks them dispatched. Guarantee: if the
-- transaction row exists, its event will eventually reach Kafka.

CREATE TABLE payments.outbox (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    transaction_id  UUID REFERENCES payments.transactions(id) ON DELETE CASCADE,
    topic           TEXT NOT NULL DEFAULT 'atlas.fare.events',
    payload         BYTEA NOT NULL,   -- protobuf-encoded atlas.events.FareEvent
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    dispatched_at   TIMESTAMPTZ,
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT
);

-- Partial index keeps the dispatcher's "find pending rows" scan narrow even
-- as dispatched rows accumulate over time.
CREATE INDEX idx_outbox_pending
    ON payments.outbox(created_at)
    WHERE dispatched_at IS NULL;

-- Columns the payments-service needs that were not in 0030_payments.sql:
--   provider_ref          - the reference returned by PaymentProvider.authorize,
--                           passed back to capture/refund.
--   idempotency_args_hash - hash of (from,to,amount,ride_id) so a reused
--                           idempotency key with DIFFERENT args is rejected
--                           rather than silently returning the wrong tx.
ALTER TABLE payments.transactions
    ADD COLUMN provider_ref          TEXT,
    ADD COLUMN idempotency_args_hash TEXT;
