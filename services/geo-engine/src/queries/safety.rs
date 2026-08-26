//! Safety votes, and the scores derived from them.
//!
//! # What a score is
//!
//! A vote is one user's judgement about one place: safe or unsafe. A score
//! is the Bayesian-smoothed balance of those votes near a point, computed
//! by `geo.safety_score` (migration 0060) and bounded to 1000..2000 with
//! 1500 meaning neutral — which is also what a place nobody has voted on
//! scores.
//!
//! It is deliberately not ELO despite what the dropped column was called.
//! ELO rates competitors from pairwise outcomes; these votes have no
//! opponent and no match, so ELO over them would produce numbers that move
//! without meaning anything.
//!
//! # One vote per user per area
//!
//! Aggregation takes each user's MOST RECENT vote within the radius, not
//! every vote they have ever cast. Re-voting corrects rather than
//! accumulates — opinions change, and a street at 2am is not the same
//! street at 2pm — and it stops one user from outweighing everyone else by
//! voting repeatedly.

use sqlx::PgPool;
use uuid::Uuid;

/// How far around a point votes count toward its score, in meters.
///
/// 200m is roughly a city block or two: close enough that the votes are
/// about the same place, wide enough that a handful of votes on one corner
/// still say something about the corner beside it.
pub const SCORE_RADIUS_M: f64 = 200.0;

/// A vote's verdict. Mirrors the `vote` CHECK constraint on
/// `geo.safety_votes`, which is the reason this is not a bare string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Safe,
    Unsafe,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Safe => "safe",
            Verdict::Unsafe => "unsafe",
        }
    }
}

/// A score and the evidence behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub score: f64,
    /// Distinct users who have voted in the area.
    pub vote_count: i64,
}

impl Score {
    /// What an area with no votes scores.
    pub const NEUTRAL: Score = Score {
        score: 1500.0,
        vote_count: 0,
    };
}

/// Record a vote and return the area's score including it.
///
/// Insert and read happen in one transaction so the returned score always
/// reflects the vote just cast. Without that, two users voting at once
/// could each be told a number that was already stale.
pub async fn cast_vote(
    pool: &PgPool,
    project_id: Uuid,
    user_id: Uuid,
    lat: f64,
    lng: f64,
    verdict: Verdict,
) -> Result<Score, sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO geo.safety_votes (project_id, user_id, position, vote)
        VALUES ($1, $2, ST_SetSRID(ST_MakePoint($3, $4), 4326), $5)
        "#,
    )
    .bind(project_id)
    .bind(user_id)
    .bind(lng)
    .bind(lat)
    .bind(verdict.as_str())
    .execute(&mut *tx)
    .await?;

    let row: (f64, i64) = sqlx::query_as(SCORE_AT_POINT)
        .bind(project_id)
        .bind(lng)
        .bind(lat)
        .bind(SCORE_RADIUS_M)
        .fetch_one(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(Score {
        score: row.0,
        vote_count: row.1,
    })
}

/// Score one point. Used by `CastSafetyVote`'s response and by tests;
/// `nearby` inlines the same aggregation as a lateral join so it can score
/// many points in one round trip.
pub async fn score_at(
    pool: &PgPool,
    project_id: Uuid,
    lat: f64,
    lng: f64,
) -> Result<Score, sqlx::Error> {
    let row: (f64, i64) = sqlx::query_as(SCORE_AT_POINT)
        .bind(project_id)
        .bind(lng)
        .bind(lat)
        .bind(SCORE_RADIUS_M)
        .fetch_one(pool)
        .await?;
    Ok(Score {
        score: row.0,
        vote_count: row.1,
    })
}

/// `DISTINCT ON (user_id) ... ORDER BY created_at DESC` is what collapses
/// each voter to their latest opinion before counting.
const SCORE_AT_POINT: &str = r#"
    WITH latest AS (
        SELECT DISTINCT ON (sv.user_id) sv.user_id, sv.vote
        FROM geo.safety_votes sv
        WHERE sv.project_id = $1
          AND ST_DWithin(
              sv.position::geography,
              ST_SetSRID(ST_MakePoint($2, $3), 4326)::geography,
              $4
          )
        ORDER BY sv.user_id, sv.created_at DESC
    )
    SELECT
        geo.safety_score(
            COUNT(*) FILTER (WHERE vote = 'safe'),
            COUNT(*) FILTER (WHERE vote = 'unsafe')
        ) AS score,
        COUNT(*)::bigint AS vote_count
    FROM latest
"#;

/// Score every point on a route line, as one number.
///
/// Averaging per-point scores would let a single well-voted corner drag a
/// long route; counting all votes along the line together treats the route
/// as one place, which is what a caller comparing two routes is asking
/// about.
pub async fn score_route_line(
    pool: &PgPool,
    project_id: Uuid,
    points: &[(f64, f64)],
) -> Result<Score, sqlx::Error> {
    let payload: Vec<serde_json::Value> = points
        .iter()
        .map(|(lat, lng)| serde_json::json!({ "lat": lat, "lng": lng }))
        .collect();

    let row: (f64, i64) = sqlx::query_as(
        r#"
        WITH route_segment AS (
            SELECT ST_MakeLine(
                ARRAY(
                    SELECT ST_SetSRID(
                        ST_MakePoint((p->>'lng')::float8, (p->>'lat')::float8),
                        4326
                    )::geometry
                    FROM jsonb_array_elements($1::jsonb) p
                )
            ) AS geom
        ),
        latest AS (
            SELECT DISTINCT ON (sv.user_id) sv.user_id, sv.vote
            FROM geo.safety_votes sv, route_segment rs
            WHERE sv.project_id = $2
              -- 30m of a line, not 200m of a point: a route is already a
              -- long object, so a wide radius would sweep in votes about
              -- streets it never touches.
              AND ST_DWithin(sv.position::geography, rs.geom::geography, 30)
            ORDER BY sv.user_id, sv.created_at DESC
        )
        SELECT
            geo.safety_score(
                COUNT(*) FILTER (WHERE vote = 'safe'),
                COUNT(*) FILTER (WHERE vote = 'unsafe')
            ) AS score,
            COUNT(*)::bigint AS vote_count
        FROM latest
        "#,
    )
    .bind(serde_json::Value::Array(payload))
    .bind(project_id)
    .fetch_one(pool)
    .await?;

    Ok(Score {
        score: row.0,
        vote_count: row.1,
    })
}
