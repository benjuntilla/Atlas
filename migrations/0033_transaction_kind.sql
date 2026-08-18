-- Distinguish how money entered or left a transaction.
--
-- Until now every row in payments.transactions was a wallet-to-wallet
-- transfer, and there was no way for money to enter the system at all:
-- wallets start at zero and settle refuses to move funds a wallet does
-- not have, so every real transaction failed with insufficient funds.
--
-- A deposit is `from_wallet IS NULL` — money arriving from outside via a
-- payment provider. That could be inferred from the null, but inferring
-- intent from the absence of a value is how you end up with a reporting
-- query that silently misclassifies a row. The column states it.
--
-- 'withdrawal' is in the CHECK but not yet implemented: payouts are the
-- mirror of deposits and will need the same treatment. Listing it now
-- means adding them later does not require a second migration to widen
-- the constraint.
ALTER TABLE payments.transactions
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'transfer'
        CHECK (kind IN ('transfer', 'deposit', 'withdrawal'));

-- Reporting reads by kind ("how much was topped up this week"), and the
-- reconciliation sweep looks for pending deposits specifically.
CREATE INDEX idx_transactions_kind ON payments.transactions(kind, created_at DESC);

-- Pending deposits are the reconciliation target: a crash between
-- capturing the provider charge and crediting the wallet leaves one here
-- with a provider_ref, which is exactly what a sweeper or the provider
-- webhook needs to find.
CREATE INDEX idx_transactions_pending_deposits
    ON payments.transactions(created_at)
    WHERE kind = 'deposit' AND status = 'pending';
