-- Password reset and email verification.
--
-- Until now a user who forgot their password had no route back into their
-- account, and Atlas had no way to know whether an address belonged to the
-- person who typed it. Both are the same primitive: a single-use secret
-- mailed to an address, redeemable once, within a window.
--
-- # One table, two purposes
--
-- Password reset and email verification differ only in what redeeming the
-- token does. Two tables would mean two copies of issue/lookup/expire/
-- consume — four chances for the halves to drift, and it is always the
-- security-relevant half that drifts.

CREATE TABLE auth.verification_tokens (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES control.projects(id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    purpose    TEXT NOT NULL CHECK (purpose IN ('password_reset', 'email_verification')),

    -- SHA-256 of the token, never the token itself — the same rule the
    -- API keys in control.api_keys follow. A database dump, a backup, or
    -- a stray log line must not hand someone the ability to take over an
    -- account. The plaintext exists only in the email.
    token_hash TEXT NOT NULL UNIQUE,

    expires_at TIMESTAMPTZ NOT NULL,

    -- Set on redemption rather than deleting the row. A used token that
    -- still exists can be recognised as used; a deleted one is
    -- indistinguishable from one that never existed, and "your link has
    -- already been used" is a materially better answer than "invalid
    -- link" for a user who double-clicked.
    used_at    TIMESTAMPTZ,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Redemption looks a token up by its hash, which the UNIQUE above already
-- indexes. This one serves the other direction: "does this user have a
-- live token of this purpose already?", which is how a re-request reuses
-- or supersedes rather than minting an unbounded pile.
CREATE INDEX idx_verification_tokens_user_purpose
    ON auth.verification_tokens(project_id, user_id, purpose, expires_at DESC);

-- Sweeping expired tokens is by expiry across all tenants, so this index
-- deliberately leads with expires_at rather than project_id.
CREATE INDEX idx_verification_tokens_expiry
    ON auth.verification_tokens(expires_at)
    WHERE used_at IS NULL;

-- NULL means unverified. A timestamp rather than a boolean because "when"
-- is the question asked in every support conversation and every audit,
-- and a boolean cannot be widened into one later without a backfill that
-- has already lost the information.
ALTER TABLE auth.users ADD COLUMN email_verified_at TIMESTAMPTZ;

COMMENT ON COLUMN auth.users.email_verified_at IS
    'When the address was confirmed. NULL means unverified. Atlas does not '
    'itself gate login on this — whether an unverified user may act is the '
    'application''s policy, so the API reports the fact and lets the '
    'developer decide.';
