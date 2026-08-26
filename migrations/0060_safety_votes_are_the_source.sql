-- Make safety scores mean something.
--
-- `GetNearby` has always returned a `safety_score`, and it has always been
-- 1500.0 for every user on the platform. The number came from
-- `COALESCE(AVG(sr.elo_score), 1500.0)` over `geo.safety_ratings` — a
-- table that no code has ever written a row to. There was no API to cast
-- a vote, nothing produced the recompute event, and nothing consumed it.
-- The score was a constant wearing a query's clothes.
--
-- # Votes are the source of truth
--
-- `geo.safety_ratings` scored SEGMENTS: rows of `GEOMETRY(LineString)`
-- with an `elo_score`. Nothing ever produced those segments either, so
-- the design had two unrooted halves rather than one. This drops it and
-- derives scores directly from `geo.safety_votes`, which the new
-- `CastSafetyVote` RPC writes.
--
-- Dropping a table is normally a destructive migration and this one is
-- not: `safety_ratings` has never had a writer in any released build, so
-- it is empty everywhere by construction. Leaving an empty table that
-- nothing reads and nothing writes is worse than removing it — it is a
-- trap for the next person, who reasonably assumes it means something.
--
-- # Why the score is not ELO
--
-- The old column was called `elo_score`, and ELO is the wrong instrument.
-- ELO rates competitors from PAIRWISE outcomes: A played B, A won. Safety
-- votes are absolute judgements about one place, with no opponent and no
-- match. Running ELO over them produces numbers that move but do not
-- mean anything.
--
-- What the function below computes is a Bayesian-smoothed net sentiment:
-- the balance of safe against unsafe votes, pulled toward neutral in
-- proportion to how little evidence there is. Two votes cannot swing a
-- location to an extreme; two hundred can.

-- A place nobody has voted on scores exactly neutral, which is what
-- callers already expect from the constant this replaces.
CREATE OR REPLACE FUNCTION geo.safety_score(safe_votes BIGINT, unsafe_votes BIGINT)
RETURNS DOUBLE PRECISION
LANGUAGE SQL
IMMUTABLE
PARALLEL SAFE
AS $$
    -- PRIOR_STRENGTH = 5. With no votes the fraction is 0/5 and the score
    -- is the neutral 1500. The first few votes move it a little; the
    -- denominator grows with the evidence, so a location with 200 votes
    -- reaches nearly the full ±500 while one with 2 barely moves.
    --
    -- Bounds are 1000..2000 and both are asymptotic: no amount of
    -- agreement produces a score outside them, so a caller can treat the
    -- range as fixed.
    SELECT 1500.0 + 500.0 * (
        (safe_votes - unsafe_votes)::double precision
        / (safe_votes + unsafe_votes + 5)::double precision
    );
$$;

COMMENT ON FUNCTION geo.safety_score(BIGINT, BIGINT) IS
    'Bayesian-smoothed net sentiment in 1000..2000, neutral 1500. Not ELO: '
    'votes are absolute judgements, not pairwise outcomes.';

-- One row per vote. A user may vote repeatedly — opinions change, and a
-- place at 2am is not the place at 2pm — and the aggregation takes each
-- user's MOST RECENT vote in an area, so re-voting corrects rather than
-- accumulates. That is also what stops one enthusiastic user from
-- outweighing everyone else.
CREATE INDEX idx_safety_votes_project_user_recent
    ON geo.safety_votes(project_id, user_id, created_at DESC);

DROP TABLE geo.safety_ratings;
