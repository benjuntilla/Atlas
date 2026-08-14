//! Real status for `atlas status`.
//!
//! # Where each field comes from
//!
//! `healthy` is a live probe, not a stored flag: a gRPC `Health/Check`
//! against auth, geo, and payments, and a TCP reachability check against
//! the Kafka brokers for `events` (the event bus has no gRPC health
//! service to ask).
//!
//! The three numeric fields are parsed out of the gateway's Prometheus
//! endpoint, which is the only place in the platform that sees per-request
//! data. That is genuinely measured traffic — but note two honest limits:
//!
//!   * **`requests_24h` is not a 24-hour window.** `atlas_gateway_requests_total`
//!     is a counter that resets when the gateway process restarts, so this
//!     is "requests since the gateway started". A real 24h figure needs a
//!     time-series database to subtract `counter[now] - counter[now-24h]`;
//!     Prometheus itself is that database, and the control plane is not
//!     going to reimplement it. The field keeps the CLI's name because
//!     renaming it would break the contract in `cli/src/api.rs`.
//!   * **`events` has no request data.** It is the Kafka bus, not a
//!     gateway route, so its counters are zero and only `healthy` is
//!     meaningful.
//!
//! Everything here degrades rather than fails: if the gateway is
//! unreachable the usage numbers come back zero and health still reports.

use std::collections::HashMap;
use std::time::Duration;

/// The four namespaces `atlas.toml` can enable, in the order the CLI
/// renders them.
pub const SERVICES: [&str; 4] = ["auth", "geo", "payments", "events"];

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ServiceUsage {
    pub requests: u64,
    /// Requests that returned 5xx. A 401 from `/v1/auth/me` is a normal
    /// outcome for a caller with a stale token, not a platform error, so
    /// 4xx deliberately does not count here.
    pub errors: u64,
    pub p95_ms: f64,
}

impl ServiceUsage {
    pub fn error_rate(&self) -> f64 {
        if self.requests == 0 {
            0.0
        } else {
            self.errors as f64 / self.requests as f64
        }
    }
}

/// Map a gateway route template onto the namespace that owns it.
///
/// The label is the matched path (`/v1/geo/geofences/:id`), so prefix
/// matching is stable regardless of path parameters. `/healthz` and the
/// `unmatched` bucket belong to no namespace.
pub fn service_for_path(path: &str) -> Option<&'static str> {
    if path.starts_with("/v1/auth") {
        Some("auth")
    } else if path.starts_with("/v1/geo") {
        Some("geo")
    } else if path.starts_with("/v1/payments") {
        Some("payments")
    } else {
        None
    }
}

/// Pull `key="value"` pairs out of a Prometheus label set.
///
/// Deliberately simple: Atlas label values are route templates, HTTP
/// methods, status codes, and quantiles, none of which contain a comma or
/// an escaped quote. A general Prometheus parser would be a dependency
/// and a lot of code for input we control end to end.
fn parse_labels(labels: &str) -> HashMap<&str, &str> {
    labels
        .split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((k.trim(), v.trim().trim_matches('"')))
        })
        .collect()
}

/// Split `name{labels} value` into its three parts.
fn split_sample(line: &str) -> Option<(&str, HashMap<&str, &str>, f64)> {
    let (head, value) = line.rsplit_once(' ')?;
    let value: f64 = value.trim().parse().ok()?;
    match head.split_once('{') {
        Some((name, rest)) => {
            let labels = rest.strip_suffix('}')?;
            Some((name.trim(), parse_labels(labels), value))
        }
        None => Some((head.trim(), HashMap::new(), value)),
    }
}

/// Aggregate a gateway `/metrics` body into per-namespace usage.
pub fn parse_gateway_metrics(text: &str) -> HashMap<&'static str, ServiceUsage> {
    let mut out: HashMap<&'static str, ServiceUsage> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, labels, value)) = split_sample(line) else {
            continue;
        };
        let Some(path) = labels.get("path") else {
            continue;
        };
        let Some(service) = service_for_path(path) else {
            continue;
        };
        let entry = out.entry(service).or_default();

        match name {
            "atlas_gateway_requests_total" => {
                let count = value as u64;
                entry.requests += count;
                let is_server_error = labels
                    .get("status")
                    .and_then(|s| s.parse::<u16>().ok())
                    .is_some_and(|s| s >= 500);
                if is_server_error {
                    entry.errors += count;
                }
            }
            "atlas_gateway_request_duration_ms" => {
                // The summary emits one line per quantile; we want 0.95.
                // Several routes map to one namespace, so take the worst
                // rather than averaging quantiles, which is not a
                // mathematically meaningful operation anyway.
                if labels.get("quantile") == Some(&"0.95") && value > entry.p95_ms {
                    entry.p95_ms = value;
                }
            }
            _ => {}
        }
    }

    out
}

/// Scrape the gateway. Returns empty usage rather than an error when the
/// gateway is down — `atlas status` should still report health.
pub async fn fetch_gateway_metrics(
    http: &reqwest::Client,
    url: &str,
) -> HashMap<&'static str, ServiceUsage> {
    match http.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(body) => parse_gateway_metrics(&body),
            Err(e) => {
                tracing::warn!(error = %e, url, "gateway metrics body unreadable");
                HashMap::new()
            }
        },
        Ok(resp) => {
            tracing::warn!(status = %resp.status(), url, "gateway metrics scrape failed");
            HashMap::new()
        }
        Err(e) => {
            tracing::warn!(error = %e, url, "gateway metrics unreachable");
            HashMap::new()
        }
    }
}

/// Standard gRPC health check. `false` on any failure — unreachable,
/// timeout, or NOT_SERVING all mean the same thing to a developer looking
/// at `atlas status`.
pub async fn grpc_healthy(addr: &str, timeout: Duration) -> bool {
    use tonic_health::pb::health_check_response::ServingStatus;
    use tonic_health::pb::health_client::HealthClient;
    use tonic_health::pb::HealthCheckRequest;

    let endpoint = match tonic::transport::Endpoint::from_shared(addr.to_string()) {
        Ok(e) => e.connect_timeout(timeout).timeout(timeout),
        Err(e) => {
            tracing::warn!(error = %e, addr, "invalid gRPC address");
            return false;
        }
    };

    let channel = match endpoint.connect().await {
        Ok(c) => c,
        Err(_) => return false,
    };

    let mut client = HealthClient::new(channel);
    // Empty service name asks about the server as a whole, which is what
    // every Atlas service marks SERVING at startup.
    match client
        .check(HealthCheckRequest {
            service: String::new(),
        })
        .await
    {
        Ok(resp) => resp.into_inner().status == ServingStatus::Serving as i32,
        Err(_) => false,
    }
}

/// TCP reachability, used for Kafka. Proves a broker is accepting
/// connections; it does not prove the cluster has a leader for every
/// partition, which would need an admin client.
pub async fn tcp_reachable(brokers: &str, timeout: Duration) -> bool {
    let Some(first) = brokers.split(',').next() else {
        return false;
    };
    let addr = first.trim();
    if addr.is_empty() {
        return false;
    }
    matches!(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim from a running `atlas-gateway` on its /metrics
    /// port after three requests: one 200 on /healthz, one 401 on
    /// /v1/auth/me, and one 404 on an unmatched path. Using real output
    /// rather than a hand-written fixture is the point — it pins the
    /// exposition format that metrics-exporter-prometheus actually emits.
    const REAL_GATEWAY_METRICS: &str = r#"
# TYPE atlas_gateway_errors_total counter
atlas_gateway_errors_total{code="unauthenticated"} 1

# TYPE atlas_gateway_requests_total counter
atlas_gateway_requests_total{method="GET",path="/v1/auth/me",status="401"} 1
atlas_gateway_requests_total{method="GET",path="/healthz",status="200"} 1
atlas_gateway_requests_total{method="GET",path="unmatched",status="404"} 1

# TYPE atlas_gateway_request_duration_ms summary
atlas_gateway_request_duration_ms{method="GET",path="unmatched",quantile="0"} 0.014416
atlas_gateway_request_duration_ms{method="GET",path="unmatched",quantile="0.95"} 0.014414797164503987
atlas_gateway_request_duration_ms{method="GET",path="unmatched",quantile="1"} 0.014416
atlas_gateway_request_duration_ms_sum{method="GET",path="unmatched"} 0.014416
atlas_gateway_request_duration_ms_count{method="GET",path="unmatched"} 1
atlas_gateway_request_duration_ms{method="GET",path="/v1/auth/me",quantile="0"} 0.10353000000000001
atlas_gateway_request_duration_ms{method="GET",path="/v1/auth/me",quantile="0.95"} 0.10352936232388359
atlas_gateway_request_duration_ms{method="GET",path="/v1/auth/me",quantile="1"} 0.10353000000000001
atlas_gateway_request_duration_ms_sum{method="GET",path="/v1/auth/me"} 0.10353000000000001
atlas_gateway_request_duration_ms_count{method="GET",path="/v1/auth/me"} 1
atlas_gateway_request_duration_ms{method="GET",path="/healthz",quantile="0.95"} 0.04759155187327729
"#;

    #[test]
    fn parses_real_gateway_output() {
        let usage = parse_gateway_metrics(REAL_GATEWAY_METRICS);

        let auth = usage.get("auth").expect("auth namespace present");
        assert_eq!(auth.requests, 1);
        // 401 is a client outcome, not a platform error.
        assert_eq!(auth.errors, 0);
        assert_eq!(auth.error_rate(), 0.0);
        assert!((auth.p95_ms - 0.10352936232388359).abs() < 1e-12);

        // /healthz and the unmatched bucket belong to no namespace and
        // must not inflate anyone's counters.
        assert!(!usage.contains_key("geo"));
        assert!(!usage.contains_key("payments"));
    }

    #[test]
    fn path_mapping_covers_every_gateway_namespace() {
        assert_eq!(service_for_path("/v1/auth/login"), Some("auth"));
        assert_eq!(service_for_path("/v1/geo/nearby"), Some("geo"));
        assert_eq!(
            service_for_path("/v1/geo/geofences/:id"),
            Some("geo"),
            "route templates with params must still map"
        );
        assert_eq!(
            service_for_path("/v1/payments/transactions"),
            Some("payments")
        );
        assert_eq!(service_for_path("/healthz"), None);
        assert_eq!(service_for_path("unmatched"), None);
        assert_eq!(service_for_path("/v1/unknown"), None);
    }

    #[test]
    fn server_errors_drive_the_error_rate() {
        let text = r#"
atlas_gateway_requests_total{method="GET",path="/v1/geo/nearby",status="200"} 97
atlas_gateway_requests_total{method="GET",path="/v1/geo/nearby",status="400"} 2
atlas_gateway_requests_total{method="GET",path="/v1/geo/nearby",status="503"} 1
"#;
        let usage = parse_gateway_metrics(text);
        let geo = usage.get("geo").unwrap();
        assert_eq!(geo.requests, 100);
        assert_eq!(geo.errors, 1, "only the 503 counts");
        assert!((geo.error_rate() - 0.01).abs() < 1e-9);
    }

    #[test]
    fn p95_takes_the_worst_route_in_a_namespace() {
        let text = r#"
atlas_gateway_request_duration_ms{method="GET",path="/v1/geo/nearby",quantile="0.95"} 41.5
atlas_gateway_request_duration_ms{method="POST",path="/v1/geo/routes/score",quantile="0.95"} 88.2
atlas_gateway_request_duration_ms{method="GET",path="/v1/geo/nearby",quantile="0.99"} 210.0
"#;
        let usage = parse_gateway_metrics(text);
        let geo = usage.get("geo").unwrap();
        assert!(
            (geo.p95_ms - 88.2).abs() < 1e-9,
            "expected the slowest 0.95 quantile, got {}",
            geo.p95_ms
        );
    }

    #[test]
    fn error_rate_of_an_idle_service_is_zero_not_nan() {
        let usage = ServiceUsage::default();
        assert_eq!(usage.error_rate(), 0.0);
        assert!(usage.error_rate().is_finite());
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let text = r#"
this is not a metric
atlas_gateway_requests_total{method="GET",path="/v1/auth/me",status="200"} notanumber
atlas_gateway_requests_total{unterminated="x" 5
atlas_gateway_requests_total{method="GET",path="/v1/auth/me",status="200"} 7
"#;
        let usage = parse_gateway_metrics(text);
        assert_eq!(usage.get("auth").unwrap().requests, 7);
    }

    #[tokio::test]
    async fn tcp_reachable_is_false_for_a_dead_port() {
        assert!(!tcp_reachable("127.0.0.1:59404", Duration::from_millis(200)).await);
        assert!(!tcp_reachable("", Duration::from_millis(200)).await);
    }

    #[tokio::test]
    async fn grpc_health_is_false_for_a_dead_backend() {
        assert!(!grpc_healthy("http://127.0.0.1:59405", Duration::from_millis(200)).await);
        assert!(!grpc_healthy("not a url", Duration::from_millis(200)).await);
    }
}
