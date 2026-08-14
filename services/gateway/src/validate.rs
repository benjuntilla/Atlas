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
