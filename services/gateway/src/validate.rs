//! Input validation that runs before any upstream call.
//!
//! The backends validate too — geo-engine clamps radius and limit,
//! payments rejects non-positive amounts. This layer is not a substitute
//! for that; it exists so obviously-bad input costs one HTTP round trip
//! instead of an HTTP round trip plus a gRPC round trip, and so the error
//! message is phrased in terms of the REST field the caller actually sent.
//!
//! Bounds here are deliberately identical to the ones in
//! `services/geo-engine/src/service.rs`. If those change, change these.

use crate::error::ApiError;

/// Matches `MAX_RADIUS_M` in geo-engine.
pub const MAX_RADIUS_M: f64 = 50_000.0;
/// Matches `MAX_NEARBY_LIMIT` in geo-engine.
pub const MAX_NEARBY_LIMIT: u32 = 100;
/// Matches `MAX_ROUTE_CANDIDATES` in geo-engine.
pub const MAX_ROUTE_CANDIDATES: usize = 10;

/// Most points one route candidate may carry.
///
/// There was no upper bound at all, only a minimum of two. Every point
/// becomes a vertex of a PostGIS LineString that `ST_DWithin` is then run
/// against, so cost grows with the vertex count: measured against the real
/// database, 1,000 points scored in ~46ms and 50,000 in ~553ms. Ten
/// candidates of that size is five seconds of database time bought with
/// one request, and the default quota allows six hundred a minute.
///
/// 2,000 is generously above any real route — a turn-by-turn polyline for
/// a cross-city trip is a few hundred points — while keeping the worst
/// case per request in the tens of milliseconds.
pub const MAX_ROUTE_POINTS: usize = 2_000;

/// Check one route candidate's point count.
///
/// A free function rather than inline in the handler so it can be tested
/// without standing up a gateway: the handler sits behind the `AuthUser`
/// extractor, which needs a live auth-service, and a bound that can only
/// be exercised through a full stack tends not to be exercised.
pub fn route_points(route_id: &str, count: usize) -> Result<(), ApiError> {
    if count < 2 {
        return Err(ApiError::BadRequest(format!(
            "route '{route_id}' needs at least 2 points"
        )));
    }
    if count > MAX_ROUTE_POINTS {
        // Rejected rather than truncated: silently scoring a different
        // route than the caller asked about would be a wrong answer
        // presented as a right one.
        return Err(ApiError::BadRequest(format!(
            "route '{route_id}' has {count} points; the maximum is {MAX_ROUTE_POINTS}"
        )));
    }
    Ok(())
}

/// Reject NaN/infinity and out-of-range coordinates.
///
/// NaN matters more than it looks: it survives serde_json as a valid f64
/// only via non-standard input, but an infinity would sail through
/// protobuf into `ST_MakePoint` and produce a PostGIS error surfacing as
/// a 500. Catching it here keeps it a 400.
pub fn lat_lng(lat: f64, lng: f64) -> Result<(), ApiError> {
    if !lat.is_finite() || !lng.is_finite() {
        return Err(ApiError::BadRequest(
            "lat and lng must be finite numbers".to_string(),
        ));
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(ApiError::BadRequest(
            "lat must be between -90 and 90".to_string(),
        ));
    }
    if !(-180.0..=180.0).contains(&lng) {
        return Err(ApiError::BadRequest(
            "lng must be between -180 and 180".to_string(),
        ));
    }
    Ok(())
}

pub fn radius_m(r: f64) -> Result<f64, ApiError> {
    if !r.is_finite() || r <= 0.0 {
        return Err(ApiError::BadRequest("radius_m must be > 0".to_string()));
    }
    if r > MAX_RADIUS_M {
        return Err(ApiError::BadRequest(format!(
            "radius_m must be <= {MAX_RADIUS_M}"
        )));
    }
    Ok(r)
}

/// 0 or absent means "let the backend apply its default"; anything over
/// the cap is clamped rather than rejected, matching geo-engine.
pub fn nearby_limit(limit: Option<u32>) -> u32 {
    match limit {
        None | Some(0) => 0,
        Some(n) => n.min(MAX_NEARBY_LIMIT),
    }
}

pub fn amount_cents(amount: i64) -> Result<i64, ApiError> {
    if amount <= 0 {
        return Err(ApiError::BadRequest("amount_cents must be > 0".to_string()));
    }
    Ok(amount)
}

/// The payments schema has `idempotency_key TEXT UNIQUE NOT NULL`, so an
/// empty key would let two distinct transactions collide on the first
/// insert and then fail every subsequent one. Require a real value.
pub fn idempotency_key(key: &str) -> Result<&str, ApiError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest(
            "idempotency_key is required (send it in the body or the Idempotency-Key header)"
                .to_string(),
        ));
    }
    if trimmed.len() > 255 {
        return Err(ApiError::BadRequest(
            "idempotency_key must be at most 255 characters".to_string(),
        ));
    }
    Ok(trimmed)
}

/// Pull the token out of an `Authorization: Bearer <token>` header value.
///
/// The scheme is matched case-insensitively because RFC 7235 says auth
/// schemes are case-insensitive and real clients send "bearer".
pub fn bearer_token(header: &str) -> Result<&str, ApiError> {
    let (scheme, token) = header
        .split_once(' ')
        .ok_or_else(|| ApiError::Unauthorized("expected 'Authorization: Bearer <token>'".into()))?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(ApiError::Unauthorized(
            "authorization scheme must be Bearer".to_string(),
        ));
    }
    let token = token.trim();
    if token.is_empty() {
        return Err(ApiError::Unauthorized("bearer token is empty".to_string()));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_bad_request(e: ApiError) -> bool {
        matches!(e, ApiError::BadRequest(_))
    }
    fn is_unauthorized(e: ApiError) -> bool {
        matches!(e, ApiError::Unauthorized(_))
    }

    #[test]
    fn lat_lng_accepts_valid_coordinates() {
        assert!(lat_lng(0.0, 0.0).is_ok());
        assert!(lat_lng(90.0, 180.0).is_ok());
        assert!(lat_lng(-90.0, -180.0).is_ok());
        assert!(lat_lng(37.7749, -122.4194).is_ok());
    }

    #[test]
    fn lat_lng_rejects_out_of_range() {
        assert!(is_bad_request(lat_lng(90.1, 0.0).unwrap_err()));
        assert!(is_bad_request(lat_lng(-90.1, 0.0).unwrap_err()));
        assert!(is_bad_request(lat_lng(0.0, 180.1).unwrap_err()));
        assert!(is_bad_request(lat_lng(0.0, -180.1).unwrap_err()));
    }

    #[test]
    fn lat_lng_rejects_nan_and_infinity() {
        assert!(is_bad_request(lat_lng(f64::NAN, 0.0).unwrap_err()));
        assert!(is_bad_request(lat_lng(0.0, f64::NAN).unwrap_err()));
        assert!(is_bad_request(lat_lng(f64::INFINITY, 0.0).unwrap_err()));
        assert!(is_bad_request(lat_lng(0.0, f64::NEG_INFINITY).unwrap_err()));
    }

    #[test]
    fn radius_bounds_match_geo_engine() {
        assert_eq!(radius_m(100.0).unwrap(), 100.0);
        assert_eq!(radius_m(MAX_RADIUS_M).unwrap(), MAX_RADIUS_M);
        assert!(is_bad_request(radius_m(0.0).unwrap_err()));
        assert!(is_bad_request(radius_m(-1.0).unwrap_err()));
        assert!(is_bad_request(radius_m(MAX_RADIUS_M + 1.0).unwrap_err()));
        assert!(is_bad_request(radius_m(f64::NAN).unwrap_err()));
    }

    #[test]
    fn nearby_limit_clamps_rather_than_rejects() {
        assert_eq!(nearby_limit(None), 0);
        assert_eq!(nearby_limit(Some(0)), 0);
        assert_eq!(nearby_limit(Some(20)), 20);
        assert_eq!(nearby_limit(Some(MAX_NEARBY_LIMIT)), MAX_NEARBY_LIMIT);
        assert_eq!(nearby_limit(Some(u32::MAX)), MAX_NEARBY_LIMIT);
    }

    #[test]
    fn route_points_bounds_both_ends() {
        assert!(route_points("r", 2).is_ok());
        assert!(route_points("r", MAX_ROUTE_POINTS).is_ok());
        // A realistic turn-by-turn polyline is a few hundred points, so
        // the bound must not reject one.
        assert!(route_points("r", 500).is_ok());

        assert!(is_bad_request(route_points("r", 0).unwrap_err()));
        assert!(is_bad_request(route_points("r", 1).unwrap_err()));
        assert!(is_bad_request(
            route_points("r", MAX_ROUTE_POINTS + 1).unwrap_err()
        ));
    }

    /// The message names the route and both numbers, because the caller
    /// sent up to ten candidates and needs to know which one to shorten.
    #[test]
    fn route_points_error_identifies_the_candidate() {
        let err = route_points("scenic-route", 50_000).unwrap_err();
        let rendered = format!("{err:?}");
        assert!(rendered.contains("scenic-route"), "{rendered}");
        assert!(rendered.contains("50000"), "{rendered}");
    }

    #[test]
    fn amount_must_be_positive() {
        assert_eq!(amount_cents(1).unwrap(), 1);
        assert!(is_bad_request(amount_cents(0).unwrap_err()));
        assert!(is_bad_request(amount_cents(-500).unwrap_err()));
    }

    #[test]
    fn idempotency_key_is_trimmed_and_required() {
        assert_eq!(idempotency_key("  abc  ").unwrap(), "abc");
        assert!(is_bad_request(idempotency_key("").unwrap_err()));
        assert!(is_bad_request(idempotency_key("   ").unwrap_err()));
        assert!(is_bad_request(
            idempotency_key(&"x".repeat(256)).unwrap_err()
        ));
    }

    #[test]
    fn bearer_parsing_is_scheme_insensitive() {
        assert_eq!(bearer_token("Bearer abc.def.ghi").unwrap(), "abc.def.ghi");
        assert_eq!(bearer_token("bearer abc.def.ghi").unwrap(), "abc.def.ghi");
        assert_eq!(bearer_token("BEARER abc.def.ghi").unwrap(), "abc.def.ghi");
    }

    #[test]
    fn bearer_parsing_rejects_malformed_headers() {
        assert!(is_unauthorized(bearer_token("abc.def.ghi").unwrap_err()));
        assert!(is_unauthorized(
            bearer_token("Basic dXNlcjpwdw==").unwrap_err()
        ));
        assert!(is_unauthorized(bearer_token("Bearer ").unwrap_err()));
        assert!(is_unauthorized(bearer_token("Bearer    ").unwrap_err()));
    }
}
