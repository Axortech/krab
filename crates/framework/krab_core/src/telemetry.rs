use tracing_subscriber::EnvFilter;

/// Standard fields emitted on every service startup log event.
/// Keeps the shape consistent across all services so log aggregators
/// and dashboards can rely on stable field names.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub service: String,
    pub version: String,
    pub environment: String,
    /// Stable per-process identifier (hostname + PID by default).
    pub instance_id: String,
}

impl TelemetryConfig {
    /// Build from environment, using `service_name` and the crate version
    /// of the *calling* binary (passed as `version`).
    pub fn from_env(service_name: &str, version: &str) -> Self {
        let environment = std::env::var("KRAB_ENVIRONMENT").unwrap_or_else(|_| "dev".to_string());
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        let pid = std::process::id();
        Self {
            service: std::env::var("KRAB_SERVICE_NAME")
                .unwrap_or_else(|_| service_name.to_string()),
            version: version.to_string(),
            environment,
            instance_id: format!("{}.{}", hostname, pid),
        }
    }
}

/// Initialise JSON structured tracing and emit a canonical `service_started`
/// event with the standardised field set.
///
/// All services **must** call this before emitting any other log events so that
/// the `service`, `version`, `environment`, and `instance_id` fields appear in
/// every structured log line.
pub fn init_tracing(service_name: &str) {
    let cfg = TelemetryConfig::from_env(service_name, env!("CARGO_PKG_VERSION"));
    init_tracing_with_config(&cfg);
}

/// Variant for callers that have already resolved [`TelemetryConfig`].
pub fn init_tracing_with_config(cfg: &TelemetryConfig) {
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| EnvFilter::try_new(s).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_env_filter(filter)
        .with_current_span(false)
        .with_target(true)
        .init();

    tracing::info!(
        service = %cfg.service,
        version = %cfg.version,
        environment = %cfg.environment,
        instance_id = %cfg.instance_id,
        "service_started"
    );
}
