use anyhow::{Context as _, Result};
use async_graphql::{EmptyMutation, EmptySubscription, Schema};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use async_trait::async_trait;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use krab_core::config::KrabConfig;
use krab_core::db::{
    detect_migration_drift, enforce_migration_governance, enforce_promotion_policy,
    migration_failure_policy_from_env, run_versioned_migrations, DbConfig, DbPool, Migration,
    MigrationGovernanceConfig, PromotionConfig,
};
use krab_core::http::AuthContext;
use krab_core::http::{
    apply_common_http_layers, health, metrics, metrics_prometheus, readiness_with_dependencies,
    DependencyStatus, HasReadinessDependencies, HasRuntimeState, RuntimeState,
};
use krab_core::protocol::{
    DeploymentTopology, ExposureMode, ProtocolConfig, ProtocolKind, ServiceCapabilities,
};
use krab_core::repository::UserRepository;
use krab_core::service::{ApiService, ServiceConfig};
use krab_core::telemetry::init_tracing;
use serde::Serialize;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

mod adapters;
mod capabilities;
mod db;
mod domain;

use crate::domain::service::{UserDomainService, UserDomainServiceImpl};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DbDriver {
    Postgres,
    Sqlite,
}

impl DbDriver {
    fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "postgres" => Ok(Self::Postgres),
            "sqlite" => Ok(Self::Sqlite),
            other => anyhow::bail!(
                "unsupported KRAB_DB_DRIVER='{}'; supported values are postgres|sqlite",
                other
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlDialect {
    Postgres,
    Sqlite,
}

impl SqlDialect {
    fn bind_param(self, one_based_index: usize) -> String {
        match self {
            Self::Postgres => format!("${}", one_based_index),
            Self::Sqlite => "?".to_string(),
        }
    }

    fn users_me_tenant_scoped_sql(self) -> String {
        let p1 = self.bind_param(1);
        format!(
            "SELECT id, username FROM users WHERE tenant_id = {} ORDER BY created_at ASC LIMIT 1",
            p1
        )
    }
}

fn redact_db_url_credentials(url: &str) -> String {
    if let Some(scheme_sep) = url.find("://") {
        let authority_start = scheme_sep + 3;
        if let Some(at_pos_rel) = url[authority_start..].find('@') {
            let at_pos = authority_start + at_pos_rel;
            let mut redacted = String::with_capacity(url.len());
            redacted.push_str(&url[..authority_start]);
            redacted.push_str("***:***");
            redacted.push('@');
            redacted.push_str(&url[at_pos + 1..]);
            return redacted;
        }
    }
    url.to_string()
}

fn resolve_db_driver() -> Result<DbDriver> {
    let raw = std::env::var("KRAB_DB_DRIVER").unwrap_or_else(|_| "sqlite".to_string());
    DbDriver::parse(&raw)
}

#[derive(Clone)]
enum UsersDbPool {
    Postgres(DbPool),
    Sqlite(SqlitePool),
}

impl UsersDbPool {
    fn dependency_name(&self) -> &'static str {
        match self {
            Self::Postgres(_) => "postgres",
            Self::Sqlite(_) => "sqlite",
        }
    }

    fn try_acquire_available(&self) -> bool {
        match self {
            Self::Postgres(pool) => pool.try_acquire().is_some(),
            Self::Sqlite(pool) => pool.try_acquire().is_some(),
        }
    }
}

fn build_user_repository(driver: DbDriver, pool: &UsersDbPool) -> Result<Arc<dyn UserRepository>> {
    match (driver, pool) {
        (DbDriver::Postgres, UsersDbPool::Postgres(pool)) => Ok(Arc::new(
            db::postgres::PostgresUserRepository::new(pool.clone()),
        )),
        (DbDriver::Sqlite, UsersDbPool::Sqlite(pool)) => Ok(Arc::new(
            db::sqlite::SqliteUserRepository::new(pool.clone()),
        )),
        (DbDriver::Postgres, UsersDbPool::Sqlite(_)) => {
            anyhow::bail!("database driver/pool mismatch: postgres driver requires postgres pool")
        }
        (DbDriver::Sqlite, UsersDbPool::Postgres(_)) => {
            anyhow::bail!("database driver/pool mismatch: sqlite driver requires sqlite pool")
        }
    }
}

struct UsersService {
    config: ServiceConfig,
    pool: UsersDbPool,
    domain: Arc<dyn UserDomainService>,
}

fn users_service_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            name: "create_users",
            sql: "CREATE TABLE IF NOT EXISTS users (id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, email TEXT NOT NULL UNIQUE, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now())",
            rollback_sql: Some("DROP TABLE IF EXISTS users"),
            critical: true,
            destructive: false,
        },
        Migration {
            version: 2,
            name: "create_users_created_at_index",
            sql: "CREATE INDEX IF NOT EXISTS idx_users_created_at ON users(created_at)",
            rollback_sql: Some("DROP INDEX IF EXISTS idx_users_created_at"),
            critical: false,
            destructive: false,
        },
        Migration {
            version: 3,
            name: "create_user_profiles",
            sql: "CREATE TABLE IF NOT EXISTS user_profiles (user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE, display_name TEXT, bio TEXT, avatar_url TEXT, updated_at TIMESTAMPTZ NOT NULL DEFAULT now())",
            rollback_sql: Some("DROP TABLE IF EXISTS user_profiles"),
            critical: false,
            destructive: false,
        },
        Migration {
            version: 4,
            name: "create_user_audit_log",
            sql: "CREATE TABLE IF NOT EXISTS user_audit_log (id BIGSERIAL PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE, action TEXT NOT NULL, actor_sub TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now())",
            rollback_sql: Some("DROP TABLE IF EXISTS user_audit_log"),
            critical: false,
            destructive: false,
        },
        Migration {
            version: 5,
            name: "create_user_audit_log_created_at_index",
            sql: "CREATE INDEX IF NOT EXISTS idx_user_audit_log_created_at ON user_audit_log(created_at)",
            rollback_sql: Some("DROP INDEX IF EXISTS idx_user_audit_log_created_at"),
            critical: false,
            destructive: false,
        },
        Migration {
            version: 6,
            name: "add_tenant_id_to_users",
            sql: "ALTER TABLE users ADD COLUMN IF NOT EXISTS tenant_id TEXT",
            rollback_sql: Some("ALTER TABLE users DROP COLUMN IF EXISTS tenant_id"),
            critical: true,
            destructive: false,
        },
        Migration {
            version: 7,
            name: "create_users_tenant_id_index",
            sql: "CREATE INDEX IF NOT EXISTS idx_users_tenant_id ON users(tenant_id)",
            rollback_sql: Some("DROP INDEX IF EXISTS idx_users_tenant_id"),
            critical: false,
            destructive: false,
        },
    ]
}

type UsersSchema = Schema<adapters::graphql::UserQuery, EmptyMutation, EmptySubscription>;

#[derive(Clone)]
struct AppState {
    schema: UsersSchema,
    pool: UsersDbPool,
    runtime: RuntimeState,
    domain: Arc<dyn UserDomainService>,
    protocol_config: ProtocolConfig,
    capabilities: ServiceCapabilities,
}

#[derive(Serialize)]
struct StatusPayload {
    status: &'static str,
}

async fn root() -> &'static str {
    "Users Service Online"
}

impl HasReadinessDependencies for AppState {
    fn readiness_dependencies(&self) -> Vec<DependencyStatus> {
        let db_ready = self.pool.try_acquire_available();
        vec![DependencyStatus {
            name: self.pool.dependency_name(),
            ready: db_ready,
            critical: true,
            latency_ms: None,
            detail: Some(if db_ready {
                "connection-pool-available".to_string()
            } else {
                "connection-pool-unavailable".to_string()
            }),
        }]
    }
}

async fn graphql_handler(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<AuthContext>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let request = req.into_inner().data(auth_ctx);
    let response = state.schema.execute(request).await;
    GraphQLResponse::from(response)
}

async fn capabilities_handler(State(state): State<AppState>) -> Json<ServiceCapabilities> {
    Json(state.capabilities.clone())
}

fn has_admin_entitlement(auth: &AuthContext) -> bool {
    let admin_scope =
        std::env::var("KRAB_AUTH_ADMIN_SCOPE").unwrap_or_else(|_| "admin".to_string());
    let admin_role = std::env::var("KRAB_AUTH_ADMIN_ROLE").unwrap_or_else(|_| "admin".to_string());
    auth.scopes.iter().any(|s| s == &admin_scope) || auth.roles.iter().any(|r| r == &admin_role)
}

async fn admin_audit_handler(
    Extension(auth_ctx): Extension<AuthContext>,
) -> (StatusCode, Json<StatusPayload>) {
    if !has_admin_entitlement(&auth_ctx) {
        return (
            StatusCode::FORBIDDEN,
            Json(StatusPayload {
                status: "forbidden",
            }),
        );
    }

    (StatusCode::OK, Json(StatusPayload { status: "admin_ok" }))
}

async fn admin_rbac_middleware(req: Request, next: Next) -> Result<Response, StatusCode> {
    let authorized = req
        .extensions()
        .get::<AuthContext>()
        .map(has_admin_entitlement)
        .unwrap_or(false);

    if !authorized {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
}

impl HasRuntimeState for AppState {
    fn runtime_state(&self) -> &RuntimeState {
        &self.runtime
    }
}

fn build_app(state: AppState) -> Router {
    let domain = state.domain.clone();
    let proto_cfg = state.protocol_config.clone();

    let admin_api = Router::new()
        .route("/audit", get(admin_audit_handler))
        .route_layer(middleware::from_fn(admin_rbac_middleware));

    let mut api = Router::new();

    if proto_cfg.enabled_protocols.contains(&ProtocolKind::Graphql) {
        api = api.route("/graphql", post(graphql_handler));
    }
    if proto_cfg.enabled_protocols.contains(&ProtocolKind::Rest) {
        api = api.merge(adapters::rest::rest_router(domain.clone()).with_state(()));
    }
    if proto_cfg.enabled_protocols.contains(&ProtocolKind::Rpc) {
        api = api.merge(adapters::rpc::rpc_router(domain.clone()).with_state(()));
    }

    api = api
        .nest("/admin", admin_api)
        .route("/capabilities", get(capabilities_handler));

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/ready", get(readiness_with_dependencies::<AppState>))
        .route("/metrics", get(metrics::<AppState>))
        .route("/metrics/prometheus", get(metrics_prometheus::<AppState>))
        .nest("/api/v1", api);

    apply_common_http_layers(app, state.clone()).with_state(state)
}

#[async_trait]
impl ApiService for UsersService {
    async fn start(&self) -> Result<()> {
        let schema = adapters::graphql::build_schema(self.domain.clone());
        let protocol_config = self
            .config
            .protocol
            .clone()
            .unwrap_or_else(ProtocolConfig::from_env);
        let capabilities = capabilities::build_capabilities(&protocol_config);

        let state = AppState {
            schema,
            pool: self.pool.clone(),
            runtime: RuntimeState::new(),
            domain: self.domain.clone(),
            protocol_config,
            capabilities,
        };

        let app = build_app(state);

        let addr = format!("{}:{}", self.config.host, self.config.port)
            .parse::<SocketAddr>()
            .context("invalid users service bind address")?;

        info!(
            service = %self.config.name,
            host = %self.config.host,
            port = self.config.port,
            %addr,
            "service_listening"
        );
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .context("failed to bind users service listener")?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("users service server exited with error")?;
        info!(service = %self.config.name, "service_shutdown_complete");
        Ok(())
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!(service = "users", "service_shutdown_signal_received");
}

async fn bootstrap_users_service() -> Result<UsersService> {
    let cfg = KrabConfig::from_env("users", 3002);
    cfg.validate().context("startup config validation failed")?;
    let mut protocol_config = ProtocolConfig::from_env();

    // Backward-compatible users default when protocol env vars are unset.
    if std::env::var("KRAB_PROTOCOL_EXPOSURE_MODE").is_err()
        && std::env::var("KRAB_PROTOCOL_ENABLED").is_err()
        && std::env::var("KRAB_PROTOCOL_ENABLED_USERS").is_err()
    {
        protocol_config.exposure_mode = ExposureMode::Multi;
        protocol_config.enabled_protocols = vec![ProtocolKind::Graphql, ProtocolKind::Rest];
        protocol_config.default_protocol = ProtocolKind::Graphql;
        protocol_config.topology = DeploymentTopology::SingleService;
    }

    protocol_config
        .validate()
        .map_err(|errs| anyhow::anyhow!("invalid protocol configuration: {}", errs.join("; ")))?;

    let config = ServiceConfig {
        name: cfg.service_name.clone(),
        host: cfg.host.clone(),
        port: cfg.port,
        protocol: Some(protocol_config.clone()),
    };

    let db_driver = resolve_db_driver()?;
    let default_db_url = match db_driver {
        DbDriver::Postgres => "postgres://postgres@localhost:5432/krab_users",
        DbDriver::Sqlite => "sqlite://krab_users.sqlite?mode=rwc",
    };
    let db_cfg = DbConfig::from_env(default_db_url);
    info!(
        db_driver = ?db_driver,
        db_url = %redact_db_url_credentials(&db_cfg.url),
        db_max_connections = db_cfg.max_connections,
        db_min_connections = db_cfg.min_connections,
        db_acquire_timeout_secs = db_cfg.acquire_timeout.as_secs(),
        db_max_lifetime_secs = db_cfg.max_lifetime.as_secs(),
        db_idle_timeout_secs = db_cfg.idle_timeout.as_secs(),
        db_connect_retries = db_cfg.connect_retries,
        db_connect_retry_delay_ms = db_cfg.connect_retry_delay.as_millis() as u64,
        "db_pool_config_resolved"
    );

    let (pool, user_repo) = match db_driver {
        DbDriver::Postgres => {
            let pool = krab_core::db::connect_with_config(&db_cfg)
                .await
                .context("failed to connect to users database")?;

            let promotion = PromotionConfig::from_env();
            enforce_promotion_policy(&pool, &promotion)
                .await
                .context("failed to enforce migration promotion policy")?;

            let governance = MigrationGovernanceConfig::from_env();
            enforce_migration_governance(&pool, &governance)
                .await
                .context("failed to enforce migration governance policy")?;

            anyhow::ensure!(
                promotion.allow_apply,
                "DB_MIGRATION_ALLOW_APPLY is false; refusing to run automatic migrations"
            );

            let report = run_versioned_migrations(
                &pool,
                &users_service_migrations(),
                migration_failure_policy_from_env(),
            )
            .await
            .context("failed to run users migrations")?;
            info!(
                applied = ?report.applied_versions,
                skipped = ?report.skipped_versions,
                "users_migrations_applied"
            );

            let drift = detect_migration_drift(&pool, &users_service_migrations())
                .await
                .context("failed to detect users migration drift")?;
            info!(
                missing = ?drift.missing_versions,
                unexpected = ?drift.unexpected_versions,
                checksum_mismatches = ?drift.checksum_mismatches,
                environment = %promotion.environment,
                "users_migration_drift_report"
            );

            let wrapped = UsersDbPool::Postgres(pool);
            let repo = build_user_repository(DbDriver::Postgres, &wrapped)?;
            (wrapped, repo)
        }
        DbDriver::Sqlite => {
            let pool = SqlitePoolOptions::new()
                .max_connections(db_cfg.max_connections)
                .min_connections(db_cfg.min_connections)
                .acquire_timeout(db_cfg.acquire_timeout)
                .max_lifetime(db_cfg.max_lifetime)
                .idle_timeout(db_cfg.idle_timeout)
                .connect(&db_cfg.url)
                .await
                .context("failed to connect to users sqlite database")?;

            sqlx::query(
                "CREATE TABLE IF NOT EXISTS users (
                    id TEXT PRIMARY KEY,
                    username TEXT NOT NULL UNIQUE,
                    email TEXT NOT NULL UNIQUE,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    tenant_id TEXT NULL
                )",
            )
            .execute(&pool)
            .await
            .context("failed to bootstrap sqlite users schema")?;

            sqlx::query("CREATE INDEX IF NOT EXISTS idx_users_created_at ON users(created_at)")
                .execute(&pool)
                .await
                .context("failed to create sqlite users index")?;

            sqlx::query("CREATE INDEX IF NOT EXISTS idx_users_tenant_id ON users(tenant_id)")
                .execute(&pool)
                .await
                .context("failed to create sqlite tenant index")?;

            info!("sqlite_users_schema_bootstrapped");

            let wrapped = UsersDbPool::Sqlite(pool);
            let repo = build_user_repository(DbDriver::Sqlite, &wrapped)?;
            (wrapped, repo)
        }
    };

    Ok(UsersService {
        config,
        pool,
        domain: Arc::new(UserDomainServiceImpl::new(user_repo)),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitUsersTarget {
    Rest,
    Graphql,
    Rpc,
}

impl SplitUsersTarget {
    fn protocol(self) -> ProtocolKind {
        match self {
            Self::Rest => ProtocolKind::Rest,
            Self::Graphql => ProtocolKind::Graphql,
            Self::Rpc => ProtocolKind::Rpc,
        }
    }

    fn service_name(self) -> &'static str {
        match self {
            Self::Rest => "users-rest",
            Self::Graphql => "users-graphql",
            Self::Rpc => "users-rpc",
        }
    }

    fn service_name_env(self) -> &'static str {
        match self {
            Self::Rest => "KRAB_USERS_REST_SERVICE_NAME",
            Self::Graphql => "KRAB_USERS_GRAPHQL_SERVICE_NAME",
            Self::Rpc => "KRAB_USERS_RPC_SERVICE_NAME",
        }
    }

    fn host_env(self) -> &'static str {
        match self {
            Self::Rest => "KRAB_USERS_REST_HOST",
            Self::Graphql => "KRAB_USERS_GRAPHQL_HOST",
            Self::Rpc => "KRAB_USERS_RPC_HOST",
        }
    }

    fn port_env(self) -> &'static str {
        match self {
            Self::Rest => "KRAB_USERS_REST_PORT",
            Self::Graphql => "KRAB_USERS_GRAPHQL_PORT",
            Self::Rpc => "KRAB_USERS_RPC_PORT",
        }
    }

    fn default_port(self) -> u16 {
        match self {
            Self::Rest => 3101,
            Self::Graphql => 3102,
            Self::Rpc => 3103,
        }
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn configure_split_target_env(target: SplitUsersTarget) {
    let protocol = target.protocol().as_str();
    std::env::set_var("KRAB_PROTOCOL_EXPOSURE_MODE", "single");
    std::env::set_var("KRAB_PROTOCOL_ENABLED", protocol);
    std::env::set_var("KRAB_PROTOCOL_DEFAULT", protocol);
    std::env::set_var("KRAB_PROTOCOL_TOPOLOGY", "split_services");

    let service_name = env_non_empty(target.service_name_env())
        .unwrap_or_else(|| target.service_name().to_string());
    std::env::set_var("KRAB_SERVICE_NAME", service_name);

    if let Some(host) = env_non_empty(target.host_env()) {
        std::env::set_var("KRAB_HOST", host);
    }

    let port =
        env_non_empty(target.port_env()).unwrap_or_else(|| target.default_port().to_string());
    std::env::set_var("KRAB_PORT", port);
}

pub async fn run_default() -> Result<()> {
    init_tracing("service_users");
    let service = bootstrap_users_service().await?;
    service.start().await
}

pub async fn run_split_target(target: SplitUsersTarget) -> Result<()> {
    configure_split_target_env(target);
    init_tracing(target.service_name());
    let service = bootstrap_users_service().await?;
    service.start().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::graphql::UserQuery;
    use crate::db::postgres::PostgresUserRepository;
    use crate::db::sqlite::SqliteUserRepository;
    use crate::domain::service::UserDomainServiceImpl;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::StatusCode;
    use serde_json::Value;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use std::sync::OnceLock;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::oneshot;
    use tower::util::ServiceExt;

    const GRAPHQL_SCHEMA_BASELINE: &str = include_str!("../contracts/graphql_schema_v1.graphql");
    const GATEWAY_UPSTREAMS_BASELINE: &str =
        include_str!("../contracts/users_gateway_upstreams_v1.json");

    fn normalize_schema(schema: &str) -> String {
        schema.chars().filter(|c| !c.is_whitespace()).collect()
    }

    static ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static tokio::sync::Mutex<()> {
        ENV_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    async fn test_state() -> AppState {
        let pool = krab_core::db::connect("postgres://postgres@localhost:5432/krab_users")
            .await
            .ok();

        let pool = if let Some(pool) = pool {
            pool
        } else {
            // fallback to lazy connect if database is unavailable in CI/local
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://postgres@localhost:5432/krab_users")
                .unwrap()
        };

        let user_repo =
            Arc::new(PostgresUserRepository::new(pool.clone())) as Arc<dyn UserRepository>;
        let domain = Arc::new(UserDomainServiceImpl::new(user_repo));
        let schema = adapters::graphql::build_schema(domain.clone());
        let protocol_config = ProtocolConfig {
            exposure_mode: ExposureMode::Multi,
            enabled_protocols: vec![ProtocolKind::Graphql, ProtocolKind::Rest, ProtocolKind::Rpc],
            default_protocol: ProtocolKind::Graphql,
            topology: DeploymentTopology::SingleService,
            policy: Default::default(),
            allow_runtime_switch_header: false,
        };
        let capabilities = capabilities::build_capabilities(&protocol_config);

        AppState {
            schema,
            pool: UsersDbPool::Postgres(pool),
            runtime: RuntimeState::new(),
            domain,
            protocol_config,
            capabilities,
        }
    }

    fn protocol_config_single(protocol: ProtocolKind) -> ProtocolConfig {
        ProtocolConfig {
            exposure_mode: ExposureMode::Single,
            enabled_protocols: vec![protocol],
            default_protocol: protocol,
            topology: DeploymentTopology::SplitServices,
            policy: Default::default(),
            allow_runtime_switch_header: false,
        }
    }

    fn protocol_config_multi_all() -> ProtocolConfig {
        ProtocolConfig {
            exposure_mode: ExposureMode::Multi,
            enabled_protocols: vec![ProtocolKind::Graphql, ProtocolKind::Rest, ProtocolKind::Rpc],
            default_protocol: ProtocolKind::Graphql,
            topology: DeploymentTopology::SingleService,
            policy: Default::default(),
            allow_runtime_switch_header: false,
        }
    }

    async fn parity_state_with_protocol(protocol_config: ProtocolConfig) -> AppState {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                tenant_id TEXT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO users (id, username, email, created_at, updated_at, tenant_id)
             VALUES ('u1', 'tenant_user', 'tenant@example.com', datetime('now'), datetime('now'), 'tenant-a')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let user_repo =
            Arc::new(SqliteUserRepository::new(pool.clone())) as Arc<dyn UserRepository>;
        let domain = Arc::new(UserDomainServiceImpl::new(user_repo));
        let schema = adapters::graphql::build_schema(domain.clone());
        let capabilities = capabilities::build_capabilities(&protocol_config);

        AppState {
            schema,
            pool: UsersDbPool::Sqlite(pool),
            runtime: RuntimeState::new(),
            domain,
            protocol_config,
            capabilities,
        }
    }

    async fn parity_state() -> AppState {
        parity_state_with_protocol(protocol_config_multi_all()).await
    }

    fn unavailable_postgres_pool() -> DbPool {
        PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(50))
            .connect_lazy("postgres://postgres@127.0.0.1:1/krab_users")
            .unwrap()
    }

    fn unavailable_sqlite_pool() -> SqlitePool {
        SqlitePoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(50))
            .connect_lazy("sqlite::memory:")
            .unwrap()
    }

    fn require_ok<T, E: std::fmt::Display>(result: std::result::Result<T, E>, context: &str) -> T {
        match result {
            Ok(value) => value,
            Err(err) => panic!("{}: {}", context, err),
        }
    }

    #[test]
    fn contract_db_driver_parse_accepts_supported_values() {
        assert!(matches!(
            DbDriver::parse("postgres"),
            Ok(DbDriver::Postgres)
        ));
        assert!(matches!(DbDriver::parse("sqlite"), Ok(DbDriver::Sqlite)));
        assert!(matches!(
            DbDriver::parse("  PoStGrEs "),
            Ok(DbDriver::Postgres)
        ));
    }

    #[test]
    fn contract_db_driver_parse_rejects_unknown_value() {
        let err = DbDriver::parse("oracle").unwrap_err();
        assert!(err
            .to_string()
            .contains("supported values are postgres|sqlite"));
    }

    #[tokio::test]
    async fn contract_repository_factory_supports_sqlite_and_rejects_mismatch() {
        let sqlite_pool = UsersDbPool::Sqlite(unavailable_sqlite_pool());
        let sqlite_repo = build_user_repository(DbDriver::Sqlite, &sqlite_pool);
        assert!(sqlite_repo.is_ok());

        let postgres_pool = UsersDbPool::Postgres(unavailable_postgres_pool());
        let mismatch = build_user_repository(DbDriver::Sqlite, &postgres_pool);
        assert!(
            mismatch.is_err(),
            "expected sqlite driver with postgres pool to fail"
        );
        let mismatch_err = require_ok(
            mismatch.err().ok_or("missing mismatch error"),
            "repository mismatch should return error",
        );
        assert!(mismatch_err.to_string().contains("driver/pool mismatch"));
    }

    async fn degraded_state() -> AppState {
        let pool = unavailable_postgres_pool();
        let user_repo =
            Arc::new(PostgresUserRepository::new(pool.clone())) as Arc<dyn UserRepository>;
        let domain = Arc::new(UserDomainServiceImpl::new(user_repo));
        let schema = adapters::graphql::build_schema(domain.clone());
        let protocol_config = ProtocolConfig {
            exposure_mode: ExposureMode::Multi,
            enabled_protocols: vec![ProtocolKind::Graphql, ProtocolKind::Rest, ProtocolKind::Rpc],
            default_protocol: ProtocolKind::Graphql,
            topology: DeploymentTopology::SingleService,
            policy: Default::default(),
            allow_runtime_switch_header: false,
        };
        let capabilities = capabilities::build_capabilities(&protocol_config);

        AppState {
            schema,
            pool: UsersDbPool::Postgres(pool),
            runtime: RuntimeState::new(),
            domain,
            protocol_config,
            capabilities,
        }
    }

    #[tokio::test]
    async fn integration_health_endpoint() {
        let app = build_app(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn contract_graphql_requires_auth() {
        let app = build_app(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn contract_graphql_schema_matches_baseline_snapshot() {
        let schema = Schema::build(UserQuery, EmptyMutation, EmptySubscription)
            .finish()
            .sdl();

        assert_eq!(
            normalize_schema(&schema),
            normalize_schema(GRAPHQL_SCHEMA_BASELINE),
            "GraphQL schema drift detected. Update service_users/contracts/graphql_schema_v1.graphql only after intentional contract review."
        );
    }

    #[tokio::test]
    async fn contract_graphql_rejects_missing_tenant_claim() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let app = build_app(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: Value = serde_json::from_slice(&bytes).unwrap();
        let first_error_message = payload
            .get("errors")
            .and_then(|errors| errors.as_array())
            .and_then(|errors| errors.first())
            .and_then(|err| err.get("message"))
            .and_then(|msg| msg.as_str());

        assert_eq!(first_error_message, Some("tenant context is required"));
    }

    #[test]
    fn invariant_graphql_me_query_is_tenant_scoped() {
        let postgres = SqlDialect::Postgres.users_me_tenant_scoped_sql();
        let sqlite = SqlDialect::Sqlite.users_me_tenant_scoped_sql();
        assert!(postgres.contains("WHERE tenant_id = $1"));
        assert!(sqlite.contains("WHERE tenant_id = ?"));
        assert!(!postgres.contains("FROM users ORDER BY"));
    }

    #[test]
    fn contract_gateway_upstream_mapping_users_split_services() {
        let mapping: Value = serde_json::from_str(GATEWAY_UPSTREAMS_BASELINE)
            .expect("gateway upstream mapping must be valid JSON");
        let routes = mapping
            .get("routes")
            .and_then(Value::as_array)
            .expect("routes array is required");
        assert_eq!(routes.len(), 3, "expected exactly three users split routes");

        let mut has_rest = false;
        let mut has_graphql = false;
        let mut has_rpc = false;

        for route in routes {
            let upstream = route
                .get("upstream")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let path_prefix = route.get("path_prefix").and_then(Value::as_str);
            let path_exact = route.get("path_exact").and_then(Value::as_str);

            match (upstream, path_prefix, path_exact) {
                ("users-rest", Some("/api/v1/users/"), None) => has_rest = true,
                ("users-graphql", None, Some("/api/v1/graphql")) => has_graphql = true,
                ("users-rpc", None, Some("/api/v1/rpc")) => has_rpc = true,
                _ => panic!("unexpected route mapping entry: {route}"),
            }
        }

        assert!(has_rest, "missing users-rest mapping");
        assert!(has_graphql, "missing users-graphql mapping");
        assert!(has_rpc, "missing users-rpc mapping");
    }

    #[tokio::test]
    async fn acceptance_single_mode_graphql_only_routes_graphql() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let app = build_app(
            parity_state_with_protocol(protocol_config_single(ProtocolKind::Graphql)).await,
        );

        let graphql = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(graphql.status(), StatusCode::OK);

        let rest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/users/me")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rest.status(), StatusCode::NOT_FOUND);

        let rpc = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rpc")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"method":"users.getMe","params":{},"id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn acceptance_single_mode_rest_only_routes_rest() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let app =
            build_app(parity_state_with_protocol(protocol_config_single(ProtocolKind::Rest)).await);

        let rest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/users/me")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rest.status(), StatusCode::BAD_REQUEST);

        let graphql = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(graphql.status(), StatusCode::NOT_FOUND);

        let rpc = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rpc")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"method":"users.getMe","params":{},"id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn acceptance_single_mode_rpc_only_routes_rpc() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let app =
            build_app(parity_state_with_protocol(protocol_config_single(ProtocolKind::Rpc)).await);

        let rpc = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rpc")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"method":"users.getMe","params":{},"id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.status(), StatusCode::OK);

        let rest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/users/me")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rest.status(), StatusCode::NOT_FOUND);

        let graphql = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(graphql.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn acceptance_multi_mode_exposes_all_protocol_routes() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let app = build_app(parity_state_with_protocol(protocol_config_multi_all()).await);

        let rest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/users/me")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rest.status(), StatusCode::BAD_REQUEST);

        let graphql = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(graphql.status(), StatusCode::OK);

        let rpc = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rpc")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"method":"users.getMe","params":{},"id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn acceptance_admin_rbac_consistent_across_modes() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let configs = vec![
            protocol_config_single(ProtocolKind::Rest),
            protocol_config_single(ProtocolKind::Graphql),
            protocol_config_single(ProtocolKind::Rpc),
            protocol_config_multi_all(),
        ];

        for protocol_config in configs {
            let app = build_app(parity_state_with_protocol(protocol_config).await);

            let missing_auth = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/api/v1/admin/audit")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                missing_auth.status(),
                StatusCode::UNAUTHORIZED,
                "admin route should require auth in all protocol modes"
            );

            let static_auth = app
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/api/v1/admin/audit")
                        .header("authorization", "Bearer test-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                static_auth.status(),
                StatusCode::FORBIDDEN,
                "static token without entitlement should be forbidden in all protocol modes"
            );
        }
    }

    #[test]
    fn acceptance_split_target_defaults_are_distinct() {
        assert_eq!(SplitUsersTarget::Rest.protocol(), ProtocolKind::Rest);
        assert_eq!(SplitUsersTarget::Graphql.protocol(), ProtocolKind::Graphql);
        assert_eq!(SplitUsersTarget::Rpc.protocol(), ProtocolKind::Rpc);

        assert_eq!(SplitUsersTarget::Rest.default_port(), 3101);
        assert_eq!(SplitUsersTarget::Graphql.default_port(), 3102);
        assert_eq!(SplitUsersTarget::Rpc.default_port(), 3103);
    }

    #[tokio::test]
    async fn contract_admin_endpoint_requires_auth() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let app = build_app(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/admin/audit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn contract_admin_endpoint_denies_static_token_without_admin_entitlement() {
        let _env_guard = env_lock().lock().await;
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "test-token");

        let app = build_app(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/admin/audit")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn contract_metrics_prometheus_exposed() {
        let app = build_app(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics/prometheus")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("krab_requests_total"));
        assert!(body.contains("krab_uptime_seconds"));
    }

    #[tokio::test]
    async fn contract_ready_reports_dependency_set() {
        let app = build_app(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: Value = serde_json::from_slice(&bytes).unwrap();
        let deps = payload
            .get("dependencies")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(deps
            .iter()
            .any(|d| d.get("name") == Some(&Value::String("postgres".to_string()))));
    }

    #[tokio::test]
    async fn fault_injection_db_outage_drives_readiness_not_ready() {
        let app = build_app(degraded_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            payload.get("status").and_then(|v| v.as_str()),
            Some("not_ready")
        );
    }

    #[tokio::test]
    async fn e2e_network_health_over_tcp() {
        let state = test_state().await;
        let app = build_app(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel::<()>();

        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await
                .unwrap();
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8(buf).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("{\"status\":\"ok\"}"));

        let _ = tx.send(());
        let _ = server.await;
    }

    fn base_env() {
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_OIDC_ISSUER", "tests");
        std::env::set_var("KRAB_OIDC_AUDIENCE", "service_users");
        std::env::set_var("KRAB_JWT_SECRET", "test-secret");
        std::env::set_var("KRAB_AUTH_REQUIRE_TENANT_MATCH", "false");
        std::env::set_var("KRAB_PROTOCOL_EXPOSURE_MODE", "multi");
        std::env::set_var("KRAB_PROTOCOL_ENABLED", "rest,graphql,rpc");
        std::env::set_var("KRAB_PROTOCOL_DEFAULT", "graphql");
        std::env::set_var("KRAB_PROTOCOL_TOPOLOGY", "single_service");
    }

    const TOKEN_WITH_TENANT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1MSIsImlzcyI6InRlc3RzIiwiYXVkIjoic2VydmljZV91c2VycyIsImV4cCI6NDEwMjQ0NDgwMCwidGlkIjoidGVuYW50LWEifQ.DSOpi_2AU_CYR6FJbWti7qF-0aAoeRNUpZ6nxbZToD8";
    const TOKEN_NO_TENANT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1MSIsImlzcyI6InRlc3RzIiwiYXVkIjoic2VydmljZV91c2VycyIsImV4cCI6NDEwMjQ0NDgwMH0.pwlx48u_ld6bjvVdo6Jz-Gc7rUMrIkhoOgzBJ9olYkI";

    #[tokio::test]
    async fn parity_get_me_rest_equals_graphql() {
        let _env_guard = env_lock().lock().await;
        base_env();
        let app = build_app(parity_state().await);

        let rest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/users/me")
                    .header("authorization", format!("Bearer {TOKEN_WITH_TENANT}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rest.status(), StatusCode::OK);
        let rest_body = to_bytes(rest.into_body(), usize::MAX).await.unwrap();
        let rest_json: Value = serde_json::from_slice(&rest_body).unwrap();

        let gql = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", format!("Bearer {TOKEN_WITH_TENANT}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gql.status(), StatusCode::OK);
        let gql_body = to_bytes(gql.into_body(), usize::MAX).await.unwrap();
        let gql_json: Value = serde_json::from_slice(&gql_body).unwrap();

        assert_eq!(
            rest_json.get("id").and_then(Value::as_str),
            gql_json
                .get("data")
                .and_then(|v| v.get("me"))
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
        );
        assert_eq!(
            rest_json.get("username").and_then(Value::as_str),
            gql_json
                .get("data")
                .and_then(|v| v.get("me"))
                .and_then(|v| v.get("username"))
                .and_then(Value::as_str)
        );
    }

    #[tokio::test]
    async fn parity_get_me_rpc_equals_graphql() {
        let _env_guard = env_lock().lock().await;
        base_env();
        let app = build_app(parity_state().await);

        let rpc = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rpc")
                    .header("authorization", format!("Bearer {TOKEN_WITH_TENANT}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"method":"users.getMe","params":{},"id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.status(), StatusCode::OK);
        let rpc_body = to_bytes(rpc.into_body(), usize::MAX).await.unwrap();
        let rpc_json: Value = serde_json::from_slice(&rpc_body).unwrap();

        let gql = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", format!("Bearer {TOKEN_WITH_TENANT}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gql.status(), StatusCode::OK);
        let gql_body = to_bytes(gql.into_body(), usize::MAX).await.unwrap();
        let gql_json: Value = serde_json::from_slice(&gql_body).unwrap();

        assert_eq!(
            rpc_json
                .get("result")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str),
            gql_json
                .get("data")
                .and_then(|v| v.get("me"))
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
        );
        assert_eq!(
            rpc_json
                .get("result")
                .and_then(|v| v.get("username"))
                .and_then(Value::as_str),
            gql_json
                .get("data")
                .and_then(|v| v.get("me"))
                .and_then(|v| v.get("username"))
                .and_then(Value::as_str)
        );
    }

    #[tokio::test]
    async fn parity_tenant_required_all_protocols() {
        let _env_guard = env_lock().lock().await;
        base_env();
        let app = build_app(parity_state().await);

        let rest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/users/me")
                    .header("authorization", format!("Bearer {TOKEN_NO_TENANT}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rest.status(), StatusCode::BAD_REQUEST);

        let gql = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("authorization", format!("Bearer {TOKEN_NO_TENANT}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gql.status(), StatusCode::OK);
        let gql_body = to_bytes(gql.into_body(), usize::MAX).await.unwrap();
        let gql_json: Value = serde_json::from_slice(&gql_body).unwrap();
        assert_eq!(
            gql_json
                .get("errors")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str),
            Some("tenant context is required")
        );

        let rpc = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rpc")
                    .header("authorization", format!("Bearer {TOKEN_NO_TENANT}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"method":"users.getMe","params":{},"id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.status(), StatusCode::OK);
        let rpc_body = to_bytes(rpc.into_body(), usize::MAX).await.unwrap();
        let rpc_json: Value = serde_json::from_slice(&rpc_body).unwrap();
        assert_eq!(
            rpc_json
                .get("error")
                .and_then(|v| v.get("message"))
                .and_then(Value::as_str),
            Some("tenant context required")
        );
    }

    #[tokio::test]
    async fn parity_auth_required_all_protocols() {
        let _env_guard = env_lock().lock().await;
        base_env();
        let app = build_app(parity_state().await);

        let rest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/users/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rest.status(), StatusCode::UNAUTHORIZED);

        let gql = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/graphql")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"{ me { id username } }"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gql.status(), StatusCode::UNAUTHORIZED);

        let rpc = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/rpc")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"method":"users.getMe","params":{},"id":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rpc.status(), StatusCode::UNAUTHORIZED);
    }
}
