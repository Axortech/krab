use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;

use tracing::{debug, info, warn};

use crate::protocol::{ProtocolConfig, ProtocolKind};
#[cfg(feature = "redis-store")]
use crate::store::RedisStore;
use crate::store::{DistributedStore, MemoryStore};

// --- API Contract & Error Model ---

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            request_id: None,
            trace_id: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

// Implement IntoResponse for ApiError to make it easy to return from Axum handlers
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.code.as_str() {
            "UNAUTHORIZED" => StatusCode::UNAUTHORIZED,
            "FORBIDDEN" => StatusCode::FORBIDDEN,
            "NOT_FOUND" => StatusCode::NOT_FOUND,
            "BAD_REQUEST" | "VALIDATION_ERROR" => StatusCode::BAD_REQUEST,
            "CONFLICT" => StatusCode::CONFLICT,
            "TOO_MANY_REQUESTS" => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // Note: Request ID propagation usually happens via middleware attaching it to the response headers.
        // If we want it in the body, we'd need to extract it here from extensions if possible,
        // but IntoResponse doesn't have access to the request parts easily.
        // For now, we rely on the response header `x-request-id` which is added by middleware.
        // The `request_id` field in the body is optional and can be populated if the error is created
        // in a context where the ID is known.

        (status, Json(self)).into_response()
    }
}

#[derive(Clone)]
pub struct RuntimeState {
    pub request_count: Arc<AtomicU64>,
    pub inflight_requests: Arc<AtomicU64>,
    pub auth_failures_total: Arc<AtomicU64>,
    pub response_2xx_total: Arc<AtomicU64>,
    pub response_4xx_total: Arc<AtomicU64>,
    pub response_5xx_total: Arc<AtomicU64>,
    pub started_at: Instant,
    pub store: Arc<dyn DistributedStore>,
    // Latency histogram buckets (count per bucket)
    // Buckets: 10ms, 50ms, 100ms, 200ms, 500ms, 1s, 2s, 5s
    pub latency_buckets: Arc<[AtomicU64; 8]>,
    // Dependency health gauge (1=ready, 0=not ready)
    pub readiness_status: Arc<std::sync::atomic::AtomicBool>,
    // Fields initialised once from HttpConfig at startup to avoid per-request env-var reads.
    pub rate_limit_capacity: f64,
    pub rate_limit_refill_per_sec: f64,
    /// Explicitly allowed CORS origins.
    pub cors_origins: Vec<String>,
    /// If true, CORS falls back to wildcard behavior (`*`) when no explicit origin list is set.
    pub cors_allow_any_origin: bool,
    /// If true, trust forwarded proxy headers (`x-forwarded-for` / `x-real-ip`) for client IP extraction.
    pub trust_proxy_headers: bool,
    /// If true, request rate limiting fails open when the backing store is unavailable.
    pub rate_limit_fail_open: bool,
    pub auth_mode: String,
    pub service_auth_scope: String,
    /// Paths that are allowed without authentication (in addition to default health/metrics).
    /// Supports exact match or prefix match if ending in `*`.
    pub public_paths: Vec<String>,
    /// Total HTTP requests segmented by resolved protocol.
    /// Buckets: rest, graphql, rpc, unknown.
    pub protocol_request_totals: Arc<[AtomicU64; 4]>,
    /// Total HTTP responses segmented by status class and resolved protocol.
    /// Slot index = class_index * 4 + protocol_index.
    /// class_index: 0=2xx, 1=4xx, 2=5xx; protocol_index: rest, graphql, rpc, unknown.
    pub response_class_protocol_totals: Arc<[AtomicU64; 12]>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeState {
    pub fn new() -> Self {
        let http_cfg = crate::config::HttpConfig::from_env();

        let store: Arc<dyn DistributedStore> = {
            #[cfg(feature = "redis-store")]
            {
                if let Ok(redis_url) = std::env::var("KRAB_REDIS_URL") {
                    if !redis_url.trim().is_empty() {
                        match RedisStore::from_url(redis_url.trim()) {
                            Ok(redis) => Arc::new(redis),
                            Err(err) => {
                                warn!(error = %err, "failed_to_initialize_redis_store_falling_back_to_memory");
                                Arc::new(MemoryStore::new())
                            }
                        }
                    } else {
                        Arc::new(MemoryStore::new())
                    }
                } else {
                    Arc::new(MemoryStore::new())
                }
            }

            #[cfg(not(feature = "redis-store"))]
            {
                Arc::new(MemoryStore::new())
            }
        };

        Self {
            request_count: Arc::new(AtomicU64::new(0)),
            inflight_requests: Arc::new(AtomicU64::new(0)),
            auth_failures_total: Arc::new(AtomicU64::new(0)),
            response_2xx_total: Arc::new(AtomicU64::new(0)),
            response_4xx_total: Arc::new(AtomicU64::new(0)),
            response_5xx_total: Arc::new(AtomicU64::new(0)),
            started_at: Instant::now(),
            store,
            latency_buckets: Arc::new([
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ]),
            readiness_status: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            rate_limit_capacity: http_cfg.rate_limit_capacity as f64,
            rate_limit_refill_per_sec: http_cfg.rate_limit_refill_per_sec as f64,
            cors_origins: http_cfg.cors_origins,
            cors_allow_any_origin: http_cfg.cors_allow_any_origin,
            trust_proxy_headers: http_cfg.trust_proxy_headers,
            rate_limit_fail_open: http_cfg.rate_limit_fail_open,
            auth_mode: http_cfg.auth_mode,
            service_auth_scope: http_cfg.service_auth_scope,
            public_paths: std::env::var("KRAB_AUTH_PUBLIC_PATHS")
                .ok()
                .map(|v| parse_csv_set(&v))
                .unwrap_or_default(),
            protocol_request_totals: Arc::new([
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ]),
            response_class_protocol_totals: Arc::new([
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ]),
        }
    }
}

fn current_window_epoch(window_secs: u64) -> u64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe_window = window_secs.max(1);
    secs / safe_window
}

pub trait HasRuntimeState {
    fn runtime_state(&self) -> &RuntimeState;
}

pub trait HasReadinessDependencies {
    fn readiness_dependencies(&self) -> Vec<DependencyStatus>;
}

#[derive(Serialize)]
pub struct MetricsPayload {
    pub requests_total: u64,
    pub inflight_requests: u64,
    pub auth_failures_total: u64,
    pub response_2xx_total: u64,
    pub response_4xx_total: u64,
    pub response_5xx_total: u64,
    pub uptime_seconds: u64,
    pub protocol_rest_total: u64,
    pub protocol_graphql_total: u64,
    pub protocol_rpc_total: u64,
    pub protocol_unknown_total: u64,
}

#[derive(Serialize, Clone)]
pub struct DependencyStatus {
    pub name: &'static str,
    pub ready: bool,
    pub critical: bool,
    pub latency_ms: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Serialize)]
pub struct ReadinessPayload {
    pub status: &'static str,
    pub uptime_seconds: u64,
    pub dependencies: Vec<DependencyStatus>,
}

#[derive(Serialize)]
pub struct StatusPayload {
    pub status: &'static str,
}

pub async fn health() -> Json<StatusPayload> {
    Json(StatusPayload { status: "ok" })
}

pub async fn readiness() -> Json<StatusPayload> {
    Json(StatusPayload { status: "ready" })
}

pub async fn readiness_with_dependencies<S>(
    State(state): State<S>,
) -> (StatusCode, Json<ReadinessPayload>)
where
    S: HasReadinessDependencies + HasRuntimeState,
{
    let dependencies = state.readiness_dependencies();
    let has_critical_failure = dependencies.iter().any(|d| d.critical && !d.ready);
    let has_non_critical_failure = dependencies.iter().any(|d| !d.critical && !d.ready);
    let status = if has_critical_failure {
        "not_ready"
    } else if has_non_critical_failure {
        "degraded"
    } else {
        "ready"
    };
    let code = if has_critical_failure {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };

    // Update readiness gauge
    if has_critical_failure {
        state
            .runtime_state()
            .readiness_status
            .store(false, Ordering::Relaxed);
    } else {
        state
            .runtime_state()
            .readiness_status
            .store(true, Ordering::Relaxed);
    }

    let uptime_seconds = state.runtime_state().started_at.elapsed().as_secs();

    (
        code,
        Json(ReadinessPayload {
            status,
            uptime_seconds,
            dependencies,
        }),
    )
}

pub async fn metrics<S>(State(state): State<S>) -> Json<MetricsPayload>
where
    S: HasRuntimeState,
{
    let runtime = state.runtime_state();
    let protocol_totals = &runtime.protocol_request_totals;
    Json(MetricsPayload {
        requests_total: runtime.request_count.load(Ordering::Relaxed),
        inflight_requests: runtime.inflight_requests.load(Ordering::Relaxed),
        auth_failures_total: runtime.auth_failures_total.load(Ordering::Relaxed),
        response_2xx_total: runtime.response_2xx_total.load(Ordering::Relaxed),
        response_4xx_total: runtime.response_4xx_total.load(Ordering::Relaxed),
        response_5xx_total: runtime.response_5xx_total.load(Ordering::Relaxed),
        uptime_seconds: runtime.started_at.elapsed().as_secs(),
        protocol_rest_total: protocol_totals[0].load(Ordering::Relaxed),
        protocol_graphql_total: protocol_totals[1].load(Ordering::Relaxed),
        protocol_rpc_total: protocol_totals[2].load(Ordering::Relaxed),
        protocol_unknown_total: protocol_totals[3].load(Ordering::Relaxed),
    })
}

pub async fn metrics_prometheus<S>(State(state): State<S>) -> Response
where
    S: HasRuntimeState,
{
    let runtime = state.runtime_state();
    let requests_total = runtime.request_count.load(Ordering::Relaxed);
    let inflight_requests = runtime.inflight_requests.load(Ordering::Relaxed);
    let auth_failures_total = runtime.auth_failures_total.load(Ordering::Relaxed);
    let response_2xx_total = runtime.response_2xx_total.load(Ordering::Relaxed);
    let response_4xx_total = runtime.response_4xx_total.load(Ordering::Relaxed);
    let response_5xx_total = runtime.response_5xx_total.load(Ordering::Relaxed);
    let uptime_seconds = runtime.started_at.elapsed().as_secs();
    let protocol_rest_total = runtime.protocol_request_totals[0].load(Ordering::Relaxed);
    let protocol_graphql_total = runtime.protocol_request_totals[1].load(Ordering::Relaxed);
    let protocol_rpc_total = runtime.protocol_request_totals[2].load(Ordering::Relaxed);
    let protocol_unknown_total = runtime.protocol_request_totals[3].load(Ordering::Relaxed);

    // Readiness Gauge (1 = ready, 0 = not ready)
    let readiness_status = match runtime.readiness_status.load(Ordering::Relaxed) {
        true => 1,
        false => 0,
    };

    let b0 = runtime.latency_buckets[0].load(Ordering::Relaxed);
    let b1 = b0 + runtime.latency_buckets[1].load(Ordering::Relaxed);
    let b2 = b1 + runtime.latency_buckets[2].load(Ordering::Relaxed);
    let b3 = b2 + runtime.latency_buckets[3].load(Ordering::Relaxed);
    let b4 = b3 + runtime.latency_buckets[4].load(Ordering::Relaxed);
    let b5 = b4 + runtime.latency_buckets[5].load(Ordering::Relaxed);
    let b6 = b5 + runtime.latency_buckets[6].load(Ordering::Relaxed);
    let b_inf = requests_total;

    let protocols = ["rest", "graphql", "rpc", "unknown"];
    let mut body = String::new();
    let _ = writeln!(
        body,
        "# HELP krab_requests_total Total HTTP requests handled\n# TYPE krab_requests_total counter\nkrab_requests_total {}",
        requests_total
    );
    let _ = writeln!(
        body,
        "# HELP krab_http_requests_by_protocol_total Total HTTP requests by resolved protocol\n# TYPE krab_http_requests_by_protocol_total counter\nkrab_http_requests_by_protocol_total{{protocol=\"rest\"}} {}\nkrab_http_requests_by_protocol_total{{protocol=\"graphql\"}} {}\nkrab_http_requests_by_protocol_total{{protocol=\"rpc\"}} {}\nkrab_http_requests_by_protocol_total{{protocol=\"unknown\"}} {}",
        protocol_rest_total,
        protocol_graphql_total,
        protocol_rpc_total,
        protocol_unknown_total
    );

    let _ = writeln!(
        body,
        "# HELP krab_http_responses_by_protocol_total Total HTTP responses by class and resolved protocol\n# TYPE krab_http_responses_by_protocol_total counter"
    );
    for (class_index, class_label) in ["2xx", "4xx", "5xx"].iter().enumerate() {
        for (protocol_index, protocol_label) in protocols.iter().enumerate() {
            let slot = response_metric_slot(class_index, protocol_index);
            let count = runtime.response_class_protocol_totals[slot].load(Ordering::Relaxed);
            let _ = writeln!(
                body,
                "krab_http_responses_by_protocol_total{{class=\"{}\",protocol=\"{}\"}} {}",
                class_label, protocol_label, count
            );
        }
    }

    let _ = writeln!(
        body,
        "# HELP krab_inflight_requests Current inflight HTTP requests\n# TYPE krab_inflight_requests gauge\nkrab_inflight_requests {}\n# HELP krab_auth_failures_total Total authentication/authorization failures\n# TYPE krab_auth_failures_total counter\nkrab_auth_failures_total {}",
        inflight_requests,
        auth_failures_total
    );
    let _ = writeln!(
        body,
        "# HELP krab_http_responses_total Total HTTP responses by class\n# TYPE krab_http_responses_total counter\nkrab_http_responses_total{{class=\"2xx\"}} {}\nkrab_http_responses_total{{class=\"4xx\"}} {}\nkrab_http_responses_total{{class=\"5xx\"}} {}",
        response_2xx_total,
        response_4xx_total,
        response_5xx_total
    );
    let _ = writeln!(
        body,
        "# HELP krab_response_2xx_total Backward-compatible alias for 2xx responses\n# TYPE krab_response_2xx_total counter\nkrab_response_2xx_total {}\n# HELP krab_response_4xx_total Backward-compatible alias for 4xx responses\n# TYPE krab_response_4xx_total counter\nkrab_response_4xx_total {}\n# HELP krab_response_5xx_total Backward-compatible alias for 5xx responses\n# TYPE krab_response_5xx_total counter\nkrab_response_5xx_total {}",
        response_2xx_total,
        response_4xx_total,
        response_5xx_total
    );
    let _ = writeln!(
        body,
        "# HELP krab_uptime_seconds Process uptime in seconds\n# TYPE krab_uptime_seconds gauge\nkrab_uptime_seconds {}\n# HELP krab_dependency_up Service readiness status (1=up, 0=down)\n# TYPE krab_dependency_up gauge\nkrab_dependency_up {}",
        uptime_seconds,
        readiness_status
    );
    let _ = writeln!(
        body,
        "# HELP krab_http_request_duration_seconds HTTP request latency histogram\n# TYPE krab_http_request_duration_seconds histogram\nkrab_http_request_duration_seconds_bucket{{le=\"0.01\"}} {}\nkrab_http_request_duration_seconds_bucket{{le=\"0.05\"}} {}\nkrab_http_request_duration_seconds_bucket{{le=\"0.1\"}} {}\nkrab_http_request_duration_seconds_bucket{{le=\"0.2\"}} {}\nkrab_http_request_duration_seconds_bucket{{le=\"0.5\"}} {}\nkrab_http_request_duration_seconds_bucket{{le=\"1.0\"}} {}\nkrab_http_request_duration_seconds_bucket{{le=\"2.0\"}} {}\nkrab_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {}\nkrab_http_request_duration_seconds_count {}",
        b0,
        b1,
        b2,
        b3,
        b4,
        b5,
        b6,
        b_inf,
        requests_total
    );
    let _ = writeln!(
        body,
        "# HELP krab_request_duration_ms Backward-compatible alias for request latency histogram in milliseconds\n# TYPE krab_request_duration_ms histogram\nkrab_request_duration_ms_bucket{{le=\"10\"}} {}\nkrab_request_duration_ms_bucket{{le=\"50\"}} {}\nkrab_request_duration_ms_bucket{{le=\"100\"}} {}\nkrab_request_duration_ms_bucket{{le=\"200\"}} {}\nkrab_request_duration_ms_bucket{{le=\"500\"}} {}\nkrab_request_duration_ms_bucket{{le=\"1000\"}} {}\nkrab_request_duration_ms_bucket{{le=\"2000\"}} {}\nkrab_request_duration_ms_bucket{{le=\"+Inf\"}} {}\nkrab_request_duration_ms_count {}",
        b0,
        b1,
        b2,
        b3,
        b4,
        b5,
        b6,
        b_inf,
        requests_total
    );

    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        CONTENT_TYPE,
        "text/plain; version=0.0.4; charset=utf-8".parse().unwrap(),
    );
    response
}

pub fn apply_common_http_layers<S>(router: Router<S>, state: S) -> Router<S>
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    router
        .layer(middleware::from_fn(security_headers_middleware))
        .layer(middleware::from_fn(api_version_header_middleware))
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(1024 * 1024 * 2)) // 2MB global request limit
        // Idempotency: We don't have a shared store for idempotency keys yet (e.g. Redis).
        // For now, we rely on the client to send unique Request-IDs and the services to handle duplicate logic if critical.
        // A full idempotency middleware would require a persistent store to track keys and responses.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            csrf_protection_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            service_auth_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            global_rate_limit_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cors_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            tracing_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            request_id_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            metrics_middleware::<S>,
        ))
        .layer(middleware::from_fn_with_state(
            state,
            protocol_resolution_middleware::<S>,
        ))
}

async fn api_version_header_middleware(req: Request<Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        HeaderName::from_static("x-krab-api-version"),
        HeaderValue::from_static("1"),
    );
    response
}

async fn protocol_resolution_middleware<S>(
    State(_state): State<S>,
    mut req: Request<Body>,
    next: Next,
) -> Response
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let config = ProtocolConfig::from_env();
    if runtime_switch_header_rejected_by_default(&req, &config) {
        return ApiError::new(
            "BAD_REQUEST",
            "runtime protocol switching via x-krab-protocol is disabled",
        )
        .into_response();
    }

    let resolved = match resolve_protocol_for_request(&req, &config) {
        Ok(protocol) => protocol,
        Err(err) => return err.into_response(),
    };

    req.extensions_mut().insert(resolved);

    let mut response = next.run(req).await;
    let header_value = HeaderValue::from_str(resolved.as_str())
        .unwrap_or_else(|_| HeaderValue::from_static("unknown"));
    response
        .headers_mut()
        .insert(HeaderName::from_static("x-krab-protocol"), header_value);
    response
}

fn extract_client_ip(req: &Request<Body>, trust_proxy_headers: bool) -> String {
    if trust_proxy_headers {
        if let Some(value) = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok())
        {
            if let Some(first) = value.split(',').next() {
                let ip = first.trim();
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }

        if let Some(value) = req.headers().get("x-real-ip").and_then(|h| h.to_str().ok()) {
            let ip = value.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }

    "unknown".to_string()
}

async fn global_rate_limit_middleware<S>(
    State(state): State<S>,
    req: Request<Body>,
    next: Next,
) -> Response
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let (client_ip, capacity, refill_per_second, fail_open) = {
        let r = state.runtime_state();
        (
            extract_client_ip(&req, r.trust_proxy_headers),
            r.rate_limit_capacity,
            r.rate_limit_refill_per_sec,
            r.rate_limit_fail_open,
        )
    };

    let window_secs = ((capacity / refill_per_second.max(1.0)).ceil() as u64).clamp(1, 300);
    let window_epoch = current_window_epoch(window_secs);
    let key = format!("rate:ip:{}:{}", client_ip, window_epoch);

    let allowed = match state.runtime_state().store.incr(&key, 1).await {
        Ok(count) => {
            if count == 1 {
                let _ = state
                    .runtime_state()
                    .store
                    .expire(&key, Duration::from_secs(window_secs + 2))
                    .await;
            }
            (count as f64) <= capacity
        }
        Err(err) => {
            if fail_open {
                warn!(error = %err, key = %key, mode = "open", "rate_limit_store_error_policy_applied");
                true
            } else {
                warn!(error = %err, key = %key, mode = "closed", "rate_limit_store_error_policy_applied");
                false
            }
        }
    };

    if !allowed {
        warn!(
            client_ip = %client_ip,
            capacity,
            refill_per_second,
            limiter = "global_per_ip_token_bucket",
            "global_ip_rate_limiter_triggered"
        );
        return ApiError::new("TOO_MANY_REQUESTS", "global per-ip rate limit exceeded")
            .into_response();
    }

    next.run(req).await
}

async fn security_headers_middleware(req: Request<Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    headers.insert(
        STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static("default-src 'self'; script-src 'self' 'wasm-unsafe-eval'"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), camera=(), microphone=()"),
    );

    response
}

fn csrf_protection_enabled() -> bool {
    bool_env("KRAB_CSRF_ENABLED", false) || bool_env("KRAB_AUTH_COOKIE_SESSION_ENABLED", false)
}

fn is_unsafe_http_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn csrf_cookie_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get("cookie")?.to_str().ok()?;
    raw.split(';').map(str::trim).find_map(|pair| {
        pair.strip_prefix("krab_csrf_token=")
            .map(ToString::to_string)
    })
}

fn csrf_header_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-csrf-token")
        .and_then(|h| h.to_str().ok())
        .map(ToString::to_string)
}

async fn csrf_protection_middleware<S>(
    State(_state): State<S>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode>
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    if !csrf_protection_enabled() {
        return Ok(next.run(req).await);
    }

    if !is_unsafe_http_method(req.method()) {
        return Ok(next.run(req).await);
    }

    // Only enforce when cookie transport is present (session-style browser auth).
    if req.headers().get("cookie").is_none() {
        return Ok(next.run(req).await);
    }

    let cookie_token = csrf_cookie_token(req.headers()).unwrap_or_default();
    let header_token = csrf_header_token(req.headers()).unwrap_or_default();

    if cookie_token.is_empty() || header_token.is_empty() || cookie_token != header_token {
        warn!("csrf_token_validation_failed");
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
}

async fn auth_middleware<S>(
    State(state): State<S>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode>
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let path = req.uri().path();
    // Debug log to diagnose why /api/status might be failing match
    // tracing::info!(path = %path, "auth_middleware_checking_path");

    let open = path == "/"
        || path == "/health"
        || path == "/ready"
        || path == "/contact"
        || path == "/api/contact"
        || path == "/api/v1/auth/login"
        || path == "/api/v1/auth/refresh"
        || path == "/api/v1/auth/revoke"
        || path == "/api/v1/auth/jwks"
        || path == "/api/v1/auth/capabilities"
        || path == "/api/v1/auth/status"
        || path == "/api/status"
        || path == "/metrics"
        || path == "/metrics/prometheus"
        || path == "/data/dashboard"
        || path == "/rpc/version"
        || path == "/rpc/now"
        || path == "/asset-manifest.json"
        || path.starts_with("/blog/")
        || path.starts_with("/pkg/");

    let is_public = state.runtime_state().public_paths.iter().any(|p| {
        if let Some(prefix) = p.strip_suffix('*') {
            path.starts_with(prefix)
        } else {
            path == p
        }
    });

    if open || is_public {
        return Ok(next.run(req).await);
    }

    // Explicit deny-by-default for any other path if no auth method is configured or valid

    let mode = state.runtime_state().auth_mode.clone();
    let authorized = if mode.eq_ignore_ascii_case("jwt") || mode.eq_ignore_ascii_case("oidc") {
        authorize_with_jwt(&req, path)
    } else {
        authorize_with_static_bearer(&req)
    };

    match authorized {
        Ok(ctx) => {
            if is_admin_api_path(path) && !has_admin_entitlement(&ctx) {
                return Err(StatusCode::FORBIDDEN);
            }
            req.extensions_mut().insert(ctx);
            Ok(next.run(req).await)
        }
        Err(code) => {
            let rs = state.runtime_state();
            rs.auth_failures_total.fetch_add(1, Ordering::Relaxed);

            let client_ip = extract_client_ip(&req, rs.trust_proxy_headers);
            let auth_window_secs = 60_u64;
            let auth_window = current_window_epoch(auth_window_secs);
            let auth_key = format!("auth:fail:{client_ip}:{auth_window}");

            let failures = match rs.store.incr(&auth_key, 1).await {
                Ok(n) => n,
                Err(err) => {
                    warn!(
                        error = %err,
                        client_ip = %client_ip,
                        "auth_failure_store_error_failing_closed"
                    );
                    return Err(StatusCode::TOO_MANY_REQUESTS);
                }
            };
            if failures == 1 {
                let _ = rs
                    .store
                    .expire(&auth_key, Duration::from_secs(auth_window_secs + 2))
                    .await;
            }

            if failures > 100 {
                warn!(
                    failures_in_window = failures,
                    window_seconds = 60,
                    limiter_scope = "per_ip_distributed_auth_failures",
                    client_ip = %client_ip,
                    "auth_failure_rate_limiter_triggered"
                );
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }

            Err(code)
        }
    }
}

fn is_internal_service_path(path: &str) -> bool {
    path.starts_with("/internal") || path.starts_with("/api/internal")
}

async fn service_auth_middleware<S>(
    State(state): State<S>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode>
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let path = req.uri().path().to_string();
    if !is_internal_service_path(&path) {
        return Ok(next.run(req).await);
    }

    let expected_scope = state.runtime_state().service_auth_scope.clone();

    let has_scope = req
        .extensions()
        .get::<AuthContext>()
        .map(|ctx| ctx.scopes.iter().any(|s| s == &expected_scope))
        .unwrap_or(false);

    if !has_scope {
        warn!(
            path = %path,
            required_scope = %expected_scope,
            "service_auth_scope_validation_failed"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    debug!(
        path = %path,
        required_scope = %expected_scope,
        "service_auth_scope_validation_passed"
    );

    Ok(next.run(req).await)
}

fn authorize_with_static_bearer(req: &Request<Body>) -> Result<AuthContext, StatusCode> {
    let expected = std::env::var("KRAB_BEARER_TOKEN").map_err(|_| {
        warn!("KRAB_BEARER_TOKEN is not configured for static auth mode");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    if expected.trim().is_empty() {
        warn!("KRAB_BEARER_TOKEN is empty and cannot be used for static auth mode");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let expected = format!("Bearer {expected}");

    let authorized = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .map(|v| constant_time_eq(v.as_bytes(), expected.as_bytes()))
        .unwrap_or(false);

    if !authorized {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(AuthContext {
        subject: Some("static-token-client".to_string()),
        issuer: Some("krab.static".to_string()),
        provider: Some("static".to_string()),
        tenant_id: None,
        scopes: vec![],
        roles: vec![],
    })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff: usize = left.len() ^ right.len();

    for i in 0..max_len {
        let l = left.get(i).copied().unwrap_or(0);
        let r = right.get(i).copied().unwrap_or(0);
        diff |= (l ^ r) as usize;
    }

    diff == 0
}

fn request_id_value_from_headers(headers: &axum::http::HeaderMap) -> (HeaderValue, &'static str) {
    match headers.get("x-request-id").cloned() {
        Some(existing) => (existing, "inbound"),
        None => {
            let generated = uuid::Uuid::new_v4().to_string();
            let value = HeaderValue::from_str(&generated)
                .unwrap_or_else(|_| HeaderValue::from_static("request-id-invalid"));
            (value, "uuid_v4")
        }
    }
}

fn authorize_with_jwt(req: &Request<Body>, path: &str) -> Result<AuthContext, StatusCode> {
    let bearer = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let header = decode_header(bearer).map_err(|_| StatusCode::UNAUTHORIZED)?;

    let allowed_algs = load_allowed_jwt_algorithms();
    if !allowed_algs.contains(&header.alg) {
        warn!(alg = ?header.alg, "jwt_algorithm_not_allowed");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let providers = load_jwt_providers();
    let mut accepted: Option<(JwtClaims, String)> = None;
    for provider in providers {
        let selected_key = match select_key(&provider.keys, header.kid.as_deref()) {
            Some(k) => k,
            None => continue,
        };

        let mut validation = Validation::new(header.alg);
        validation.algorithms = allowed_algs.clone();
        validation.validate_exp = true;
        validation.validate_aud = false;
        validation.leeway = 60; // 60 seconds clock skew tolerance
        let token_data = match decode::<JwtClaims>(
            bearer,
            &DecodingKey::from_secret(selected_key.as_bytes()),
            &validation,
        ) {
            Ok(data) => data,
            Err(_) => continue,
        };

        if validate_provider_claims(&token_data.claims, &provider).is_err() {
            continue;
        }

        accepted = Some((
            token_data.claims,
            provider.name.unwrap_or_else(|| "provider".to_string()),
        ));
        break;
    }

    let (claims, provider_name) = accepted.ok_or(StatusCode::UNAUTHORIZED)?;

    let scopes = scopes_from_claims(&claims);
    let roles = roles_from_claims(&claims);
    let tenant_id = tenant_from_claims(&claims);
    enforce_claim_policy(path, &claims, tenant_id.as_deref(), &scopes, &roles)?;

    Ok(AuthContext {
        subject: claims.sub,
        issuer: claims.iss,
        provider: Some(provider_name),
        tenant_id,
        scopes,
        roles,
    })
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub provider: Option<String>,
    pub tenant_id: Option<String>,
    pub scopes: Vec<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JwtClaims {
    sub: Option<String>,
    iss: Option<String>,
    aud: Option<Value>,
    exp: Option<i64>,
    tid: Option<String>,
    tenant_id: Option<String>,
    scope: Option<String>,
    scp: Option<Value>,
    roles: Option<Vec<String>>,
    role: Option<String>,
    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct JwtProviderConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    audience: Option<String>,
    keys: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    required_claims: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct RoutePolicy {
    prefix: String,
    #[serde(default)]
    all_scopes: Vec<String>,
    #[serde(default)]
    any_scopes: Vec<String>,
    #[serde(default)]
    all_roles: Vec<String>,
    #[serde(default)]
    any_roles: Vec<String>,
    #[serde(default)]
    allow_subjects: Vec<String>,
    #[serde(default)]
    require_tenant_match: bool,
}

fn load_rotation_keys() -> std::collections::BTreeMap<String, String> {
    if let Ok(json) = std::env::var("KRAB_JWT_KEYS_JSON") {
        if let Ok(parsed) =
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(&json)
        {
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    let secret = std::env::var("KRAB_JWT_SECRET").ok().and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let Some(secret) = secret else {
        warn!(
            "No JWT signing key configured; set KRAB_JWT_SECRET or KRAB_JWT_KEYS_JSON for jwt/oidc mode"
        );
        return std::collections::BTreeMap::new();
    };

    let mut keys = std::collections::BTreeMap::new();
    keys.insert("default".to_string(), secret);
    keys
}

fn load_jwt_providers() -> Vec<JwtProviderConfig> {
    if let Ok(raw) = std::env::var("KRAB_JWT_PROVIDERS_JSON") {
        if let Ok(mut providers) = serde_json::from_str::<Vec<JwtProviderConfig>>(&raw) {
            providers.retain(|p| !p.keys.is_empty());
            if !providers.is_empty() {
                return providers;
            }
        }
    }

    vec![JwtProviderConfig {
        name: Some("default".to_string()),
        issuer: std::env::var("KRAB_OIDC_ISSUER").ok(),
        audience: std::env::var("KRAB_OIDC_AUDIENCE").ok(),
        keys: load_rotation_keys(),
        required_claims: std::collections::BTreeMap::new(),
    }]
}

fn parse_jwt_algorithm(name: &str) -> Option<jsonwebtoken::Algorithm> {
    match name.trim().to_uppercase().as_str() {
        "HS256" => Some(jsonwebtoken::Algorithm::HS256),
        "HS384" => Some(jsonwebtoken::Algorithm::HS384),
        "HS512" => Some(jsonwebtoken::Algorithm::HS512),
        "RS256" => Some(jsonwebtoken::Algorithm::RS256),
        "RS384" => Some(jsonwebtoken::Algorithm::RS384),
        "RS512" => Some(jsonwebtoken::Algorithm::RS512),
        "ES256" => Some(jsonwebtoken::Algorithm::ES256),
        "ES384" => Some(jsonwebtoken::Algorithm::ES384),
        "PS256" => Some(jsonwebtoken::Algorithm::PS256),
        "PS384" => Some(jsonwebtoken::Algorithm::PS384),
        "PS512" => Some(jsonwebtoken::Algorithm::PS512),
        "EDDSA" => Some(jsonwebtoken::Algorithm::EdDSA),
        _ => None,
    }
}

fn load_allowed_jwt_algorithms() -> Vec<jsonwebtoken::Algorithm> {
    let mut parsed = std::env::var("KRAB_JWT_ALLOWED_ALGS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .filter_map(parse_jwt_algorithm)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if parsed.is_empty() {
        parsed.push(jsonwebtoken::Algorithm::HS256);
    }
    parsed
}

fn require_kid() -> bool {
    std::env::var("KRAB_JWT_REQUIRE_KID")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn select_key<'a>(
    keys: &'a std::collections::BTreeMap<String, String>,
    kid: Option<&str>,
) -> Option<&'a String> {
    match kid {
        Some(k) => keys.get(k),
        None if require_kid() => None,
        None => keys.get("default").or_else(|| keys.values().next()),
    }
}

fn tenant_from_claims(claims: &JwtClaims) -> Option<String> {
    claims
        .tenant_id
        .clone()
        .or_else(|| claims.tid.clone())
        .or_else(|| {
            claims
                .extra
                .get("tenant_id")
                .and_then(|v| v.as_str().map(ToString::to_string))
        })
        .or_else(|| {
            claims
                .extra
                .get("tid")
                .and_then(|v| v.as_str().map(ToString::to_string))
        })
}

fn tenant_from_path(path: &str) -> Option<&str> {
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    while let Some(segment) = parts.next() {
        if segment == "tenants" {
            return parts.next();
        }
    }
    None
}

fn load_route_policies() -> Vec<RoutePolicy> {
    std::env::var("KRAB_AUTH_ROUTE_POLICIES_JSON")
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<RoutePolicy>>(&raw).ok())
        .unwrap_or_default()
}

fn bool_env(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

fn validate_provider_claims(
    claims: &JwtClaims,
    provider: &JwtProviderConfig,
) -> Result<(), StatusCode> {
    if let Some(expected_issuer) = provider.issuer.as_deref() {
        if claims.iss.as_deref() != Some(expected_issuer) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    if let Some(expected_audience) = provider.audience.as_ref() {
        let aud_ok = match claims.aud.as_ref() {
            Some(Value::String(aud)) => aud == expected_audience,
            Some(Value::Array(values)) => values
                .iter()
                .any(|v| v.as_str() == Some(expected_audience.as_str())),
            _ => false,
        };
        if !aud_ok {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    if !provider.required_claims.is_empty() {
        let claims_json = serde_json::to_value(claims).map_err(|_| StatusCode::UNAUTHORIZED)?;
        let claims_obj = claims_json.as_object().ok_or(StatusCode::UNAUTHORIZED)?;
        for (k, v) in &provider.required_claims {
            let found = claims_obj
                .get(k)
                .or_else(|| claims.extra.get(k))
                .ok_or(StatusCode::UNAUTHORIZED)?;
            if found != v {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    Ok(())
}

fn scopes_from_claims(claims: &JwtClaims) -> Vec<String> {
    if let Some(scope) = &claims.scope {
        return scope.split_whitespace().map(|s| s.to_string()).collect();
    }

    match claims.scp.as_ref() {
        Some(Value::String(scope)) => scope.split_whitespace().map(|s| s.to_string()).collect(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    }
}

fn roles_from_claims(claims: &JwtClaims) -> Vec<String> {
    if let Some(roles) = &claims.roles {
        return roles.clone();
    }
    if let Some(role) = &claims.role {
        return vec![role.clone()];
    }
    vec![]
}

fn enforce_claim_policy(
    path: &str,
    claims: &JwtClaims,
    tenant_id: Option<&str>,
    scopes: &[String],
    roles: &[String],
) -> Result<(), StatusCode> {
    let required_scopes = std::env::var("KRAB_AUTH_REQUIRED_SCOPES")
        .ok()
        .map(|v| parse_csv_set(&v))
        .unwrap_or_default();
    for scope in required_scopes {
        if !scopes.iter().any(|s| s == &scope) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    let required_roles = std::env::var("KRAB_AUTH_REQUIRED_ROLES")
        .ok()
        .map(|v| parse_csv_set(&v))
        .unwrap_or_default();
    for role in required_roles {
        if !roles.iter().any(|r| r == &role) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    if is_admin_api_path(path) {
        let admin_scope =
            std::env::var("KRAB_AUTH_ADMIN_SCOPE").unwrap_or_else(|_| "admin".to_string());
        let admin_role =
            std::env::var("KRAB_AUTH_ADMIN_ROLE").unwrap_or_else(|_| "admin".to_string());
        if !scopes.iter().any(|s| s == &admin_scope) && !roles.iter().any(|r| r == &admin_role) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    if bool_env("KRAB_AUTH_REQUIRE_TENANT_CLAIM", false) && tenant_id.is_none() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    if bool_env("KRAB_AUTH_REQUIRE_TENANT_MATCH", true) {
        if let Some(path_tenant) = tenant_from_path(path) {
            if tenant_id != Some(path_tenant) {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    for policy in load_route_policies()
        .into_iter()
        .filter(|p| path.starts_with(&p.prefix))
    {
        for scope in &policy.all_scopes {
            if !scopes.iter().any(|s| s == scope) {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
        if !policy.any_scopes.is_empty()
            && !policy
                .any_scopes
                .iter()
                .any(|s| scopes.iter().any(|actual| actual == s))
        {
            return Err(StatusCode::UNAUTHORIZED);
        }

        for role in &policy.all_roles {
            if !roles.iter().any(|r| r == role) {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
        if !policy.any_roles.is_empty()
            && !policy
                .any_roles
                .iter()
                .any(|r| roles.iter().any(|actual| actual == r))
        {
            return Err(StatusCode::UNAUTHORIZED);
        }

        if !policy.allow_subjects.is_empty() {
            let subject = claims.sub.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
            if !policy.allow_subjects.iter().any(|s| s == subject) {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }

        if policy.require_tenant_match {
            let path_tenant = tenant_from_path(path).ok_or(StatusCode::UNAUTHORIZED)?;
            if tenant_id != Some(path_tenant) {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    if let Ok(required_claims) = std::env::var("KRAB_AUTH_REQUIRED_CLAIMS_JSON") {
        let required: std::collections::BTreeMap<String, Value> =
            serde_json::from_str(&required_claims).map_err(|_| StatusCode::UNAUTHORIZED)?;
        let claims_json = serde_json::to_value(claims).map_err(|_| StatusCode::UNAUTHORIZED)?;
        let claims_obj = claims_json.as_object().ok_or(StatusCode::UNAUTHORIZED)?;
        for (k, v) in required {
            let found = claims_obj
                .get(&k)
                .or_else(|| claims.extra.get(&k))
                .ok_or(StatusCode::UNAUTHORIZED)?;
            if found != &v {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    Ok(())
}

fn parse_csv_set(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn request_tenant_hint(req: &Request<Body>) -> Option<String> {
    if let Some(ctx) = req.extensions().get::<AuthContext>() {
        if let Some(tenant_id) = &ctx.tenant_id {
            let trimmed = tenant_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    if let Some(value) = req.headers().get("x-krab-tenant-id") {
        if let Ok(raw) = value.to_str() {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    let query = req.uri().query()?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        if key.eq_ignore_ascii_case("tenant_id") || key.eq_ignore_ascii_case("tid") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

fn policy_allowed_protocols(
    method: &Method,
    path: &str,
    tenant_id: Option<&str>,
    config: &ProtocolConfig,
) -> Vec<ProtocolKind> {
    let mut allowed = vec![ProtocolKind::Rest, ProtocolKind::Graphql, ProtocolKind::Rpc];
    let operation = operation_label(method, path);

    if let Some(restricted) = config.policy.restricted_operations.get(operation) {
        if !restricted.is_empty() {
            allowed.retain(|candidate| restricted.contains(candidate));
        }
    }

    if let Some(tenant_id) = tenant_id {
        if let Some(tenant_allowed) = config.policy.tenant_overrides.get(tenant_id) {
            if !tenant_allowed.is_empty() {
                allowed.retain(|candidate| tenant_allowed.contains(candidate));
            }
        }
    }

    allowed
}

#[allow(clippy::result_large_err)]
fn resolve_protocol_for_request(
    req: &Request<Body>,
    config: &ProtocolConfig,
) -> Result<ProtocolKind, ApiError> {
    let method = req.method();
    let path = req.uri().path();
    let operation = operation_label(method, path);
    let tenant_id = request_tenant_hint(req);
    let allowed = policy_allowed_protocols(method, path, tenant_id.as_deref(), config);

    if allowed.is_empty() {
        return Err(ApiError::new(
            "BAD_REQUEST",
            "protocol policy produced no allowed protocols for this request",
        )
        .with_details(serde_json::json!({
            "operation": operation,
            "tenant_id": tenant_id,
        })));
    }

    if let Some(route_protocol) = route_family_protocol(path) {
        if allowed.contains(&route_protocol) {
            return Ok(route_protocol);
        }

        return Err(ApiError::new(
            "BAD_REQUEST",
            "route family protocol is disallowed by protocol policy",
        )
        .with_details(serde_json::json!({
            "operation": operation,
            "tenant_id": tenant_id,
            "candidate_protocol": route_protocol.as_str(),
            "allowed_protocols": allowed.iter().map(ProtocolKind::as_str).collect::<Vec<_>>(),
        })));
    }

    if config.allow_runtime_switch_header {
        if let Some(client_preference) = extract_protocol_preference(req) {
            if allowed.contains(&client_preference) {
                return Ok(client_preference);
            }

            return Err(ApiError::new(
                "BAD_REQUEST",
                "client protocol preference is disallowed by protocol policy",
            )
            .with_details(serde_json::json!({
                "operation": operation,
                "tenant_id": tenant_id,
                "candidate_protocol": client_preference.as_str(),
                "allowed_protocols": allowed.iter().map(ProtocolKind::as_str).collect::<Vec<_>>(),
            })));
        }
    }

    if allowed.contains(&config.default_protocol) {
        return Ok(config.default_protocol);
    }

    Ok(allowed[0])
}

pub fn extract_protocol_preference(req: &Request<Body>) -> Option<ProtocolKind> {
    if let Some(value) = req.headers().get("x-krab-protocol") {
        if let Ok(raw) = value.to_str() {
            if let Some(protocol) = ProtocolKind::parse(raw) {
                return Some(protocol);
            }
        }
    }

    let query = req.uri().query()?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        if key.eq_ignore_ascii_case("protocol") {
            if let Some(protocol) = ProtocolKind::parse(value) {
                return Some(protocol);
            }
        }
    }
    None
}

pub fn route_family_protocol(path: &str) -> Option<ProtocolKind> {
    if path == "/api/v1/graphql" || path.starts_with("/api/v1/graphql/") {
        return Some(ProtocolKind::Graphql);
    }
    if path == "/api/v1/rpc" || path.starts_with("/api/v1/rpc/") {
        return Some(ProtocolKind::Rpc);
    }
    if path == "/api/v1/users" || path.starts_with("/api/v1/users/") {
        return Some(ProtocolKind::Rest);
    }
    None
}

pub fn runtime_switch_header_rejected_by_default(
    req: &Request<Body>,
    config: &ProtocolConfig,
) -> bool {
    req.headers().contains_key("x-krab-protocol") && !config.allow_runtime_switch_header
}

fn resolved_protocol(req: &Request<Body>) -> Option<ProtocolKind> {
    req.extensions().get::<ProtocolKind>().copied()
}

fn protocol_label(req: &Request<Body>) -> &'static str {
    resolved_protocol(req)
        .map(|protocol| protocol.as_str())
        .unwrap_or("unknown")
}

fn operation_label(method: &Method, path: &str) -> &'static str {
    match (method, path) {
        (&Method::GET, "/api/v1/users/me") => "users.getMe",
        (&Method::POST, "/api/v1/graphql") => "users.graphql",
        (&Method::POST, "/api/v1/rpc") => "users.rpc",
        (&Method::POST, "/api/v1/auth/login") => "auth.login",
        (&Method::POST, "/api/v1/auth/refresh") => "auth.refresh",
        (&Method::POST, "/api/v1/auth/revoke") => "auth.revoke",
        (&Method::GET, "/api/v1/auth/jwks") => "auth.jwks",
        (&Method::GET, "/api/v1/auth/status") => "auth.status",
        (&Method::GET, "/api/v1/auth/capabilities") => "auth.capabilities",
        _ => "http.request",
    }
}

fn selection_source_label(req: &Request<Body>, path: &str) -> &'static str {
    if req.headers().contains_key("x-krab-protocol") {
        return "client";
    }
    if route_family_protocol(path).is_some() {
        return "policy";
    }
    "default"
}

fn protocol_metric_index(protocol: Option<ProtocolKind>) -> usize {
    match protocol {
        Some(ProtocolKind::Rest) => 0,
        Some(ProtocolKind::Graphql) => 1,
        Some(ProtocolKind::Rpc) => 2,
        None => 3,
    }
}

fn response_status_class_index(status_code: u16) -> Option<usize> {
    if (200..300).contains(&status_code) {
        Some(0)
    } else if (400..500).contains(&status_code) {
        Some(1)
    } else if (500..600).contains(&status_code) {
        Some(2)
    } else {
        None
    }
}

fn response_metric_slot(class_index: usize, protocol_index: usize) -> usize {
    class_index * 4 + protocol_index
}

fn is_admin_api_path(path: &str) -> bool {
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    if parts.next() != Some("api") {
        return false;
    }

    match parts.next() {
        Some("admin") => true,
        Some(version)
            if version.starts_with('v') && version[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            parts.next() == Some("admin")
        }
        _ => false,
    }
}

fn has_admin_entitlement(ctx: &AuthContext) -> bool {
    let admin_scope =
        std::env::var("KRAB_AUTH_ADMIN_SCOPE").unwrap_or_else(|_| "admin".to_string());
    let admin_role = std::env::var("KRAB_AUTH_ADMIN_ROLE").unwrap_or_else(|_| "admin".to_string());
    ctx.scopes.iter().any(|s| s == &admin_scope) || ctx.roles.iter().any(|r| r == &admin_role)
}

async fn request_id_middleware<S>(_state: State<S>, mut req: Request<Body>, next: Next) -> Response
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let (request_id_value, strategy) = request_id_value_from_headers(req.headers());
    req.headers_mut()
        .insert("x-request-id", request_id_value.clone());

    let mut response = next.run(req).await;

    response
        .headers_mut()
        .insert("x-request-id", request_id_value.clone());

    debug!(
        request_id = %request_id_value.to_str().unwrap_or("non-utf8"),
        strategy = %strategy,
        "request_id_attached"
    );

    response
}

async fn metrics_middleware<S>(State(state): State<S>, req: Request<Body>, next: Next) -> Response
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let runtime = state.runtime_state();
    let protocol_index = protocol_metric_index(resolved_protocol(&req));

    runtime.request_count.fetch_add(1, Ordering::Relaxed);
    runtime.protocol_request_totals[protocol_index].fetch_add(1, Ordering::Relaxed);
    runtime.inflight_requests.fetch_add(1, Ordering::Relaxed);

    let response = next.run(req).await;

    runtime.inflight_requests.fetch_sub(1, Ordering::Relaxed);

    let code = response.status().as_u16();
    if let Some(class_index) = response_status_class_index(code) {
        match class_index {
            0 => {
                runtime.response_2xx_total.fetch_add(1, Ordering::Relaxed);
            }
            1 => {
                runtime.response_4xx_total.fetch_add(1, Ordering::Relaxed);
            }
            2 => {
                runtime.response_5xx_total.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }

        let slot = response_metric_slot(class_index, protocol_index);
        runtime.response_class_protocol_totals[slot].fetch_add(1, Ordering::Relaxed);
    }

    response
}

/// Returns the value to use for `Access-Control-Allow-Origin`, or `None` if the request
/// origin is not whitelisted.
fn compute_cors_origin<'a>(
    request_origin: &'a str,
    allowed: &[String],
    allow_any_origin: bool,
) -> Option<&'a str> {
    if request_origin.is_empty() {
        return None;
    }
    if allow_any_origin && allowed.is_empty() {
        return Some("*");
    }
    if allowed.iter().any(|o| o == "*") {
        return Some("*");
    }
    if allowed.iter().any(|o| o == request_origin) {
        return Some(request_origin);
    }
    None
}

fn cors_allow_methods_value() -> &'static str {
    "GET,POST,OPTIONS"
}

fn cors_allow_headers_value() -> &'static str {
    "authorization,content-type,x-request-id,x-trace-id"
}

fn cors_preflight_response(origin: &str) -> Option<Response> {
    let origin_header = HeaderValue::from_str(origin).ok()?;
    let methods_header = HeaderValue::from_str(cors_allow_methods_value()).ok()?;
    let headers_header = HeaderValue::from_str(cors_allow_headers_value()).ok()?;

    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = StatusCode::NO_CONTENT;
    resp.headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin_header);
    resp.headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_METHODS, methods_header);
    resp.headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_HEADERS, headers_header);
    Some(resp)
}

fn append_cors_headers(resp: &mut Response, origin: &str) -> bool {
    let origin_header = match HeaderValue::from_str(origin) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let methods_header = match HeaderValue::from_str(cors_allow_methods_value()) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let headers_header = match HeaderValue::from_str(cors_allow_headers_value()) {
        Ok(value) => value,
        Err(_) => return false,
    };

    resp.headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin_header);
    resp.headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_METHODS, methods_header);
    resp.headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_HEADERS, headers_header);
    true
}

async fn cors_middleware<S>(State(state): State<S>, req: Request<Body>, next: Next) -> Response
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let request_origin = req
        .headers()
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();

    let runtime = state.runtime_state();
    let allowed_origin = compute_cors_origin(
        &request_origin,
        &runtime.cors_origins,
        runtime.cors_allow_any_origin,
    )
    .map(|s| s.to_string());

    if req.method() == Method::OPTIONS {
        match allowed_origin {
            Some(origin) => {
                if let Some(resp) = cors_preflight_response(&origin) {
                    return resp;
                }

                let mut resp = Response::new(Body::empty());
                *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                return resp;
            }
            None => {
                // Origin not whitelisted — reject preflight.
                let mut resp = Response::new(Body::empty());
                *resp.status_mut() = StatusCode::FORBIDDEN;
                return resp;
            }
        }
    }

    let mut resp = next.run(req).await;
    if let Some(origin) = allowed_origin {
        if !append_cors_headers(&mut resp, &origin) {
            *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        }
    }
    resp
}

async fn tracing_middleware<S>(State(state): State<S>, req: Request<Body>, next: Next) -> Response
where
    S: Clone + Send + Sync + 'static + HasRuntimeState,
{
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let protocol = protocol_label(&req);
    let operation = operation_label(&method, &path);
    let selection_source = selection_source_label(&req, &path);
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    if request_id == "unknown" {
        debug!("tracing_missing_request_id_in_inbound_headers");
    }

    let start = Instant::now();
    let resp = next.run(req).await;
    let status = resp.status();
    let elapsed = start.elapsed();
    let elapsed_ms = elapsed.as_millis();

    // Update latency buckets (from State)
    let buckets = &state.runtime_state().latency_buckets;
    if elapsed_ms <= 10 {
        buckets[0].fetch_add(1, Ordering::Relaxed);
    } else if elapsed_ms <= 50 {
        buckets[1].fetch_add(1, Ordering::Relaxed);
    } else if elapsed_ms <= 100 {
        buckets[2].fetch_add(1, Ordering::Relaxed);
    } else if elapsed_ms <= 200 {
        buckets[3].fetch_add(1, Ordering::Relaxed);
    } else if elapsed_ms <= 500 {
        buckets[4].fetch_add(1, Ordering::Relaxed);
    } else if elapsed_ms <= 1000 {
        buckets[5].fetch_add(1, Ordering::Relaxed);
    } else if elapsed_ms <= 2000 {
        buckets[6].fetch_add(1, Ordering::Relaxed);
    } else {
        buckets[7].fetch_add(1, Ordering::Relaxed);
    }

    // Standardize tracing attributes (OpenTelemetry conventions)
    info!(
        event = "http_request_complete",
        http.method = %method,
        http.route = %path,
        http.status_code = %status.as_u16(),
        http.request_id = %request_id,
        krab.protocol = %protocol,
        krab.operation = %operation,
        krab.selection_source = %selection_source,
        duration_ms = elapsed_ms,
        "request_complete"
    );
    resp
}

// ── Cross-service propagation ─────────────────────────────────────────────────

/// Headers that must be forwarded on every outbound service-to-service call
/// to maintain request-id and trace correlation across the topology.
///
/// # Usage
///
/// ```rust,ignore
/// use krab_core::http::PropagationHeaders;
///
/// // Inside an Axum handler, extract the inbound headers:
/// let prop = PropagationHeaders::from_request_headers(req.headers());
///
/// // Then inject them into every outbound reqwest / hyper request:
/// let client = reqwest::Client::new();
/// let mut builder = client.get("http://service_users:3002/api/v1/graphql");
/// builder = prop.inject(builder);
/// let resp = builder.send().await?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct PropagationHeaders {
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
}

impl PropagationHeaders {
    pub const REQUEST_ID_HEADER: &'static str = "x-request-id";
    pub const TRACE_ID_HEADER: &'static str = "x-trace-id";

    /// Extract propagation headers from an inbound request's header map.
    pub fn from_request_headers(headers: &axum::http::HeaderMap) -> Self {
        Self {
            request_id: headers
                .get(Self::REQUEST_ID_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string),
            trace_id: headers
                .get(Self::TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string),
        }
    }

    /// Return a `Vec` of `(header-name, value)` pairs suitable for injecting
    /// into any HTTP client builder that accepts raw header pairs.
    pub fn as_header_pairs(&self) -> Vec<(&'static str, &str)> {
        let mut pairs = Vec::new();
        if let Some(id) = &self.request_id {
            pairs.push((Self::REQUEST_ID_HEADER, id.as_str()));
        }
        if let Some(tid) = &self.trace_id {
            pairs.push((Self::TRACE_ID_HEADER, tid.as_str()));
        }
        pairs
    }

    /// Inject into an [`axum::http::HeaderMap`] (useful when building
    /// outbound `hyper` / `reqwest` requests from an Axum handler).
    pub fn inject_into_headers(&self, headers: &mut axum::http::HeaderMap) {
        if let Some(id) = &self.request_id {
            if let Ok(v) = axum::http::HeaderValue::from_str(id) {
                headers.insert(
                    axum::http::HeaderName::from_static(Self::REQUEST_ID_HEADER),
                    v,
                );
            }
        }
        if let Some(tid) = &self.trace_id {
            if let Ok(v) = axum::http::HeaderValue::from_str(tid) {
                headers.insert(
                    axum::http::HeaderName::from_static(Self::TRACE_ID_HEADER),
                    v,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;

    #[test]
    fn scope_claim_string_is_split() {
        let claims = JwtClaims {
            sub: Some("u1".to_string()),
            iss: Some("issuer".to_string()),
            aud: None,
            exp: None,
            tid: None,
            tenant_id: None,
            scope: Some("users.read users.write".to_string()),
            scp: None,
            roles: None,
            role: None,
            extra: Default::default(),
        };

        let scopes = scopes_from_claims(&claims);
        assert_eq!(
            scopes,
            vec!["users.read".to_string(), "users.write".to_string()]
        );
    }

    #[test]
    fn role_claim_falls_back_to_single_role() {
        let claims = JwtClaims {
            sub: Some("u1".to_string()),
            iss: Some("issuer".to_string()),
            aud: None,
            exp: None,
            tid: None,
            tenant_id: None,
            scope: None,
            scp: None,
            roles: None,
            role: Some("admin".to_string()),
            extra: Default::default(),
        };

        let roles = roles_from_claims(&claims);
        assert_eq!(roles, vec!["admin".to_string()]);
    }

    #[test]
    fn tenant_path_extraction_works() {
        assert_eq!(tenant_from_path("/api/tenants/t1/users"), Some("t1"));
        assert_eq!(tenant_from_path("/api/users/me"), None);
    }

    #[test]
    fn tenant_claim_falls_back_to_tid() {
        let claims = JwtClaims {
            sub: Some("u1".to_string()),
            iss: Some("issuer".to_string()),
            aud: None,
            exp: None,
            tid: Some("tenant-a".to_string()),
            tenant_id: None,
            scope: None,
            scp: None,
            roles: None,
            role: None,
            extra: Default::default(),
        };

        assert_eq!(tenant_from_claims(&claims).as_deref(), Some("tenant-a"));
    }

    #[derive(Clone)]
    struct TestState {
        runtime: RuntimeState,
        dependencies: Vec<DependencyStatus>,
    }

    impl HasRuntimeState for TestState {
        fn runtime_state(&self) -> &RuntimeState {
            &self.runtime
        }
    }

    impl HasReadinessDependencies for TestState {
        fn readiness_dependencies(&self) -> Vec<DependencyStatus> {
            self.dependencies.clone()
        }
    }

    #[tokio::test]
    async fn readiness_non_critical_failure_is_degraded_but_available() {
        let state = TestState {
            runtime: RuntimeState::new(),
            dependencies: vec![DependencyStatus {
                name: "cache",
                ready: false,
                critical: false,
                latency_ms: Some(250),
                detail: Some("cache-timeout".to_string()),
            }],
        };

        let (status, Json(payload)) = readiness_with_dependencies::<TestState>(State(state)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload.status, "degraded");
    }

    #[test]
    fn propagation_headers_round_trip() {
        let mut map = axum::http::HeaderMap::new();
        map.insert(
            axum::http::HeaderName::from_static("x-request-id"),
            axum::http::HeaderValue::from_static("req-abc"),
        );
        map.insert(
            axum::http::HeaderName::from_static("x-trace-id"),
            axum::http::HeaderValue::from_static("trace-xyz"),
        );

        let prop = PropagationHeaders::from_request_headers(&map);
        assert_eq!(prop.request_id.as_deref(), Some("req-abc"));
        assert_eq!(prop.trace_id.as_deref(), Some("trace-xyz"));

        let mut out = axum::http::HeaderMap::new();
        prop.inject_into_headers(&mut out);
        assert_eq!(
            out.get("x-request-id").and_then(|v| v.to_str().ok()),
            Some("req-abc")
        );
        assert_eq!(
            out.get("x-trace-id").and_then(|v| v.to_str().ok()),
            Some("trace-xyz")
        );
    }

    #[test]
    fn propagation_headers_missing_fields_produce_empty_pairs() {
        let map = axum::http::HeaderMap::new();
        let prop = PropagationHeaders::from_request_headers(&map);
        assert!(prop.request_id.is_none());
        assert!(prop.trace_id.is_none());
        assert!(prop.as_header_pairs().is_empty());
    }

    #[test]
    fn cors_allow_headers_is_strict_allowlist() {
        assert_eq!(
            cors_allow_headers_value(),
            "authorization,content-type,x-request-id,x-trace-id"
        );
    }

    #[test]
    fn cors_allow_methods_is_reduced_surface() {
        assert_eq!(cors_allow_methods_value(), "GET,POST,OPTIONS");
    }

    #[test]
    fn csrf_cookie_token_extraction_works() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            axum::http::HeaderValue::from_static("foo=1; krab_csrf_token=abc123; bar=2"),
        );

        assert_eq!(csrf_cookie_token(&headers).as_deref(), Some("abc123"));
    }

    #[test]
    fn csrf_header_token_extraction_works() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::HeaderName::from_static("x-csrf-token"),
            axum::http::HeaderValue::from_static("abc123"),
        );

        assert_eq!(csrf_header_token(&headers).as_deref(), Some("abc123"));
    }

    #[test]
    fn unsafe_method_detection_is_strict() {
        assert!(is_unsafe_http_method(&Method::POST));
        assert!(is_unsafe_http_method(&Method::PUT));
        assert!(is_unsafe_http_method(&Method::PATCH));
        assert!(is_unsafe_http_method(&Method::DELETE));
        assert!(!is_unsafe_http_method(&Method::GET));
        assert!(!is_unsafe_http_method(&Method::HEAD));
        assert!(!is_unsafe_http_method(&Method::OPTIONS));
    }

    #[test]
    fn constant_time_eq_requires_exact_match() {
        assert!(constant_time_eq(b"Bearer abc", b"Bearer abc"));
        assert!(!constant_time_eq(b"Bearer abc", b"Bearer abd"));
        assert!(!constant_time_eq(b"Bearer abc", b"Bearer abcx"));
    }

    #[test]
    fn request_id_generation_preserves_inbound_header_when_present() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::HeaderName::from_static("x-request-id"),
            axum::http::HeaderValue::from_static("req-123"),
        );

        let (value, strategy) = request_id_value_from_headers(&headers);
        assert_eq!(strategy, "inbound");
        assert_eq!(value.to_str().ok(), Some("req-123"));
    }

    #[test]
    fn request_id_generation_creates_uuid_when_absent() {
        let headers = axum::http::HeaderMap::new();
        let (value, strategy) = request_id_value_from_headers(&headers);
        assert_eq!(strategy, "uuid_v4");
        assert!(value.to_str().ok().is_some());
    }

    fn protocol_config_for_tests() -> ProtocolConfig {
        ProtocolConfig {
            exposure_mode: crate::protocol::ExposureMode::Multi,
            enabled_protocols: vec![ProtocolKind::Rest, ProtocolKind::Graphql, ProtocolKind::Rpc],
            default_protocol: ProtocolKind::Graphql,
            topology: crate::protocol::DeploymentTopology::SingleService,
            policy: crate::protocol::ProtocolPolicy::default(),
            allow_runtime_switch_header: false,
        }
    }

    #[test]
    fn protocol_resolution_rejects_route_family_disallowed_by_operation_policy() {
        let mut config = protocol_config_for_tests();
        config.policy.restricted_operations.insert(
            "users.graphql".to_string(),
            vec![ProtocolKind::Rest, ProtocolKind::Rpc],
        );

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/graphql")
            .body(Body::empty())
            .unwrap();

        let err = resolve_protocol_for_request(&req, &config)
            .expect_err("graphql route should be rejected when operation policy disallows graphql");
        assert_eq!(err.code, "BAD_REQUEST");
        assert!(err.message.contains("route family protocol"));
    }

    #[test]
    fn protocol_resolution_applies_tenant_override_for_default_selection() {
        let mut config = protocol_config_for_tests();
        config
            .policy
            .tenant_overrides
            .insert("tenant-a".to_string(), vec![ProtocolKind::Rest]);

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/unknown")
            .header("x-krab-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let resolved = resolve_protocol_for_request(&req, &config)
            .expect("tenant override should force rest for unknown route");
        assert_eq!(resolved, ProtocolKind::Rest);
    }

    #[test]
    fn protocol_resolution_rejects_client_preference_when_disallowed_by_tenant_override() {
        let mut config = protocol_config_for_tests();
        config.allow_runtime_switch_header = true;
        config
            .policy
            .tenant_overrides
            .insert("tenant-a".to_string(), vec![ProtocolKind::Rest]);

        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/unknown")
            .header("x-krab-protocol", "graphql")
            .header("x-krab-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let err = resolve_protocol_for_request(&req, &config)
            .expect_err("client preference should be rejected by tenant override");
        assert_eq!(err.code, "BAD_REQUEST");
        assert!(err.message.contains("client protocol preference"));
    }

    #[test]
    fn protocol_resolution_keeps_route_family_priority_without_policy_restrictions() {
        let config = ProtocolConfig {
            exposure_mode: crate::protocol::ExposureMode::Single,
            enabled_protocols: vec![ProtocolKind::Rest],
            default_protocol: ProtocolKind::Rest,
            topology: crate::protocol::DeploymentTopology::SingleService,
            policy: crate::protocol::ProtocolPolicy::default(),
            allow_runtime_switch_header: false,
        };

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/graphql")
            .body(Body::empty())
            .unwrap();

        let resolved = resolve_protocol_for_request(&req, &config)
            .expect("explicit graphql route should resolve to graphql without policy restrictions");
        assert_eq!(resolved, ProtocolKind::Graphql);
    }
}
