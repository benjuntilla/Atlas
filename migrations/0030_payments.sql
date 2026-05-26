CREATE TABLE payments.wallets (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id       UUID NOT NULL UNIQUE REFERENCES auth.users(id) ON DELETE CASCADE,
    balance_cents BIGINT NOT NULL DEFAULT 0,
    currency      TEXT NOT NULL DEFAULT 'USD',
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE payments.transactions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    from_wallet     UUID REFERENCES payments.wallets(id),
    to_wallet       UUID REFERENCES payments.wallets(id),
    amount_cents    BIGINT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'settled', 'failed', 'refunded')),
    idempotency_key TEXT UNIQUE NOT NULL,
    ride_id         UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at      TIMESTAMPTZ
);

CREATE INDEX idx_transactions_status ON payments.transactions(status);
CREATE INDEX idx_transactions_ride   ON payments.transactions(ride_id);

-- Audit log for fare-consumer (Phase 8). Missing from the original spec.
CREATE TABLE payments.transaction_events (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    transaction_id UUID REFERENCES payments.transactions(id) ON DELETE CASCADE,
    ride_id        UUID,
    event_type     TEXT NOT NULL,
    payload        JSONB NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_transaction_events_tx   ON payments.transaction_events(transaction_id);
CREATE INDEX idx_transaction_events_ride ON payments.transaction_events(ride_id);
