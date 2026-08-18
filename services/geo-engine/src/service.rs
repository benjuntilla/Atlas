//! tonic service implementation. This is where proto types meet business
//! logic. Every handler:
//!   1. Validates inputs (UUID parsing, bounds checks).
//!   2. Calls into `queries::*` for the DB hit.
//!   3. Translates results back to proto messages.
//!   4. Records latency / counters.
//!
//! Trust model: this service trusts the `user_id` and `project_id` strings
//! in request bodies. The gateway validates the JWT and resolves the API
//! key, then injects both; neither has a field in any REST DTO, so a
//! client cannot supply either. We never re-validate against auth-service
//! on the hot path.
//!
//! What this service DOES enforce is that every query is scoped by the
//! project it was handed. `nearby` in particular searches by position
//! rather than by user id, so without that scoping it would return every
//! Atlas user near a point regardless of which customer asked.

use chrono::{TimeZone, Utc};
use sqlx::PgPool;
use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::{info, warn};
use uuid::Uuid;

use crate::kafka::LocationProducer;
use crate::pb::events::LocationUpdateEvent;
use crate::pb::geo::{
    geo_engine_server::GeoEngine, CreateGeofenceRequest, DeleteGeofenceRequest,
    DeleteGeofenceResponse, Geofence, GeofenceCheckRequest, GeofenceCheckResponse,
    ListGeofencesRequest, ListGeofencesResponse, LocationAck, LocationUpdate, NearbyRequest,
    NearbyResponse, NearbyUser, RouteScoreRequest, RouteScoreResponse, ScoredRoute,
};
use crate::queries;

// Defaults / caps applied in code (not the proto) so they are easy to
// audit and change without a proto bump.
const DEFAULT_NEARBY_LIMIT: u32 = 20;
const MAX_NEARBY_LIMIT: u32 = 100;
const MAX_RADIUS_M: f64 = 50_000.0; // 50 km — generous upper bound
const MAX_ROUTE_CANDIDATES: usize = 10;

pub struct GeoEngineImpl {
    pool: PgPool,
    producer: LocationProducer,
}

impl GeoEngineImpl {
    pub fn new(pool: PgPool, producer: LocationProducer) -> Self {
        Self { pool, producer }
    }
}

fn parse_user_id(s: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(s).map_err(|_| Status::invalid_argument("user_id is not a valid UUID"))
}

/// Parse the project the gateway injected.
///
/// Strict on purpose: this side of the boundary trusts its callers, so an
/// empty project_id cannot have come from a client — only from a bug in
/// the gateway or a service calling this one directly. Defaulting would
/// turn that bug into silent cross-tenant reads and writes; failing turns
/// it into an error someone has to look at.
fn parse_project_id(s: &str) -> Result<Uuid, Status> {
    if s.is_empty() {
        return Err(Status::invalid_argument(
            "project_id is required; the gateway must inject it",
        ));
    }
    Uuid::parse_str(s).map_err(|_| Status::invalid_argument("project_id is not a valid UUID"))
}

fn clamp_radius(r: f64) -> Result<f64, Status> {
    if !r.is_finite() || r <= 0.0 {
        return Err(Status::invalid_argument("radius_m must be > 0"));
    }
    Ok(r.min(MAX_RADIUS_M))
}

fn clamp_limit(limit: u32) -> i64 {
    let l = if limit == 0 {
        DEFAULT_NEARBY_LIMIT
    } else {
        limit.min(MAX_NEARBY_LIMIT)
    };
    l as i64
}

#[tonic::async_trait]
impl GeoEngine for GeoEngineImpl {
    async fn update_location(
        &self,
        req: Request<LocationUpdate>,
    ) -> Result<Response<LocationAck>, Status> {
        let r = req.into_inner();
        let project_id = parse_project_id(&r.project_id)?;
        let user_id = parse_user_id(&r.user_id)?;
        // Trust the proto-supplied timestamp but fall back to "now" when
        // the client sends 0 (common on first ping before time is set).
        let recorded_at = if r.recorded_at > 0 {
            Utc.timestamp_opt(r.recorded_at, 0)
                .single()
                .ok_or_else(|| Status::invalid_argument("recorded_at out of range"))?
        } else {
            Utc::now()
        };

        queries::locations::insert_location(
            &self.pool,
            project_id,
            user_id,
            r.lat,
            r.lng,
            recorded_at,
        )
        .await
        .map_err(|e| {
            warn!(error = %e, "insert_location failed");
            Status::internal("failed to record location")
        })?;

        // Fire-and-forget Kafka enqueue. The row is in Postgres regardless.
        self.producer.enqueue(&LocationUpdateEvent {
            user_id: r.user_id.clone(),
            lat: r.lat,
            lng: r.lng,
            recorded_at: recorded_at.timestamp(),
        });
        metrics::counter!("atlas_location_updates_total").increment(1);

        Ok(Response::new(LocationAck { ok: true }))
    }

    async fn get_nearby(
        &self,
        req: Request<NearbyRequest>,
    ) -> Result<Response<NearbyResponse>, Status> {
        let r = req.into_inner();
        let project_id = parse_project_id(&r.project_id)?;
        let requester = parse_user_id(&r.requester_user_id)?;
        let radius = clamp_radius(r.radius_m)?;
        let limit = clamp_limit(r.limit);
        let role = if r.role.is_empty() {
            "unknown".to_string()
        } else {
            r.role.clone()
        };

        let start = Instant::now();
        let rows = queries::locations::nearby(
            &self.pool, project_id, r.lat, r.lng, radius, requester, limit,
        )
        .await
        .map_err(|e| {
            warn!(error = %e, "nearby query failed");
            Status::internal("nearby query failed")
        })?;
        metrics::histogram!(
            "atlas_geo_nearby_query_duration_ms",
            "role" => role.clone(),
        )
        .record(start.elapsed().as_secs_f64() * 1000.0);

        let users = rows
            .into_iter()
            .map(|row| NearbyUser {
                user_id: row.user_id.to_string(),
                lat: row.lat,
                lng: row.lng,
                distance_m: row.distance_m,
                safety_score: row.safety_score,
            })
            .collect();

        Ok(Response::new(NearbyResponse { users }))
    }

    async fn score_route(
        &self,
        req: Request<RouteScoreRequest>,
    ) -> Result<Response<RouteScoreResponse>, Status> {
        let r = req.into_inner();
        let project_id = parse_project_id(&r.project_id)?;
        if r.candidates.is_empty() {
            return Err(Status::invalid_argument("at least one candidate required"));
        }
        if r.candidates.len() > MAX_ROUTE_CANDIDATES {
            return Err(Status::invalid_argument(format!(
                "at most {MAX_ROUTE_CANDIDATES} candidates per request"
            )));
        }
        // Validate each candidate has at least 2 points (a single point is
        // not a route).
        for c in &r.candidates {
            if c.points.len() < 2 {
                return Err(Status::invalid_argument(
                    "each route candidate needs at least 2 points",
                ));
            }
        }

        let start = Instant::now();
        let candidate_count_bucket = match r.candidates.len() {
            1 => "1",
            2..=3 => "2-3",
            4..=5 => "4-5",
            _ => "6+",
        };

        let mut scored = Vec::with_capacity(r.candidates.len());
        for c in r.candidates {
            let points: Vec<(f64, f64)> = c.points.iter().map(|p| (p.lat, p.lng)).collect();
            let score = queries::routes::score_route(&self.pool, project_id, &points)
                .await
                .map_err(|e| {
                    warn!(error = %e, "score_route query failed");
                    Status::internal("route scoring failed")
                })?;
            scored.push(ScoredRoute {
                route_id: c.route_id,
                score,
            });
        }
        metrics::histogram!(
            "atlas_geo_score_route_duration_ms",
            "candidate_count" => candidate_count_bucket,
        )
        .record(start.elapsed().as_secs_f64() * 1000.0);

        // Best = highest ELO. Stable: first candidate wins on ties.
        let best = scored
            .iter()
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("non-empty after validation");

        Ok(Response::new(RouteScoreResponse {
            best_route_id: best.route_id.clone(),
            score: best.score,
            all_scores: scored,
        }))
    }

    async fn trigger_geofence_check(
        &self,
        req: Request<GeofenceCheckRequest>,
    ) -> Result<Response<GeofenceCheckResponse>, Status> {
        let r = req.into_inner();
        let project_id = parse_project_id(&r.project_id)?;
        let user_id = parse_user_id(&r.user_id)?;
        let start = Instant::now();
        let ids =
            queries::geofences::check_membership(&self.pool, project_id, user_id, r.lat, r.lng)
                .await
                .map_err(|e| {
                    warn!(error = %e, "geofence membership check failed");
                    Status::internal("geofence check failed")
                })?;
        metrics::histogram!("atlas_geo_geofence_check_duration_ms")
            .record(start.elapsed().as_secs_f64() * 1000.0);

        Ok(Response::new(GeofenceCheckResponse {
            triggered: !ids.is_empty(),
            geofence_ids: ids.into_iter().map(|id| id.to_string()).collect(),
        }))
    }

    async fn create_geofence(
        &self,
        req: Request<CreateGeofenceRequest>,
    ) -> Result<Response<Geofence>, Status> {
        let r = req.into_inner();
        let project_id = parse_project_id(&r.project_id)?;
        let user_id = parse_user_id(&r.user_id)?;
        if !r.radius_m.is_finite() || r.radius_m <= 0.0 || r.radius_m > MAX_RADIUS_M {
            return Err(Status::invalid_argument(
                "radius_m must be > 0 and <= 50000",
            ));
        }
        let row = queries::geofences::create(
            &self.pool,
            project_id,
            user_id,
            &r.label,
            r.center_lat,
            r.center_lng,
            r.radius_m,
        )
        .await
        .map_err(|e| {
            warn!(error = %e, "create_geofence failed");
            // FK violation = unknown user_id; everything else is internal.
            if let sqlx::Error::Database(dbe) = &e {
                if dbe.constraint().is_some() {
                    return Status::failed_precondition("user_id does not exist");
                }
            }
            Status::internal("create_geofence failed")
        })?;

        info!(geofence_id = %row.id, user_id = %row.user_id, "geofence created");
        Ok(Response::new(to_proto(row)))
    }

    async fn list_geofences(
        &self,
        req: Request<ListGeofencesRequest>,
    ) -> Result<Response<ListGeofencesResponse>, Status> {
        let r = req.into_inner();
        let project_id = parse_project_id(&r.project_id)?;
        let user_id = parse_user_id(&r.user_id)?;
        let rows = queries::geofences::list(&self.pool, project_id, user_id, r.active_only)
            .await
            .map_err(|e| {
                warn!(error = %e, "list_geofences failed");
                Status::internal("list_geofences failed")
            })?;
        Ok(Response::new(ListGeofencesResponse {
            geofences: rows.into_iter().map(to_proto).collect(),
        }))
    }

    async fn delete_geofence(
        &self,
        req: Request<DeleteGeofenceRequest>,
    ) -> Result<Response<DeleteGeofenceResponse>, Status> {
        let r = req.into_inner();
        let project_id = parse_project_id(&r.project_id)?;
        let user_id = parse_user_id(&r.user_id)?;
        let geofence_id = Uuid::parse_str(&r.geofence_id)
            .map_err(|_| Status::invalid_argument("geofence_id is not a valid UUID"))?;
        let deleted = queries::geofences::deactivate(&self.pool, project_id, user_id, geofence_id)
            .await
            .map_err(|e| {
                warn!(error = %e, "delete_geofence failed");
                Status::internal("delete_geofence failed")
            })?;
        Ok(Response::new(DeleteGeofenceResponse { deleted }))
    }
}

fn to_proto(row: queries::geofences::GeofenceRow) -> Geofence {
    Geofence {
        id: row.id.to_string(),
        user_id: row.user_id.to_string(),
        label: row.label,
        center_lat: row.center_lat,
        center_lng: row.center_lng,
        radius_m: row.radius_m,
        active: row.active,
        created_at: row.created_at.timestamp(),
    }
}

#[cfg(test)]
mod tests {
    //! Pure unit tests for input validation helpers. No DB or Kafka.
    //! These run on every `cargo test` (no docker-compose required).
    use super::*;

    #[test]
    fn parse_user_id_accepts_uuid() {
        let id = parse_user_id("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn parse_user_id_rejects_garbage() {
        assert!(parse_user_id("not-a-uuid").is_err());
        assert!(parse_user_id("").is_err());
    }

    #[test]
    fn clamp_radius_rejects_nonpositive_and_nan() {
        assert!(clamp_radius(0.0).is_err());
        assert!(clamp_radius(-1.0).is_err());
        assert!(clamp_radius(f64::NAN).is_err());
        assert!(clamp_radius(f64::INFINITY).is_err());
    }

    #[test]
    fn clamp_radius_caps_at_max() {
        assert_eq!(clamp_radius(100.0).unwrap(), 100.0);
        assert_eq!(clamp_radius(MAX_RADIUS_M).unwrap(), MAX_RADIUS_M);
        assert_eq!(clamp_radius(MAX_RADIUS_M + 1.0).unwrap(), MAX_RADIUS_M);
        assert_eq!(clamp_radius(999_999.0).unwrap(), MAX_RADIUS_M);
    }

    #[test]
    fn clamp_limit_zero_becomes_default() {
        assert_eq!(clamp_limit(0), DEFAULT_NEARBY_LIMIT as i64);
    }

    #[test]
    fn clamp_limit_caps_at_max() {
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(MAX_NEARBY_LIMIT), MAX_NEARBY_LIMIT as i64);
        assert_eq!(clamp_limit(MAX_NEARBY_LIMIT + 1), MAX_NEARBY_LIMIT as i64);
        assert_eq!(clamp_limit(u32::MAX), MAX_NEARBY_LIMIT as i64);
    }
}
