//! gRPC health-check service. K8s liveness/readiness probes hit this.
//!
//! We register the GeoEngine service name as SERVING immediately on
//! startup. If the DB or Kafka becomes unreachable later, the service
//! continues to report SERVING — readiness gating of upstream traffic
//! is a Phase 14 (k8s) concern, not Phase 3.

use tonic_health::server::HealthReporter;

pub async fn mark_serving(reporter: &mut HealthReporter) {
    // Empty service name marks the overall server as serving, which is
    // what the standard probe checks. We additionally register the
    // generated GeoEngine descriptor so per-service probes work too.
    reporter
        .set_serving::<crate::pb::geo::geo_engine_server::GeoEngineServer<crate::service::GeoEngineImpl>>()
        .await;
}
