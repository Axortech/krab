# Phase B — Users Service Multi-Protocol Pilot

> **Goal**: refactor `service_users` into a domain-core + protocol-adapter architecture, making it the first service that runs in REST-only, GraphQL-only, RPC-only, or multi-protocol mode.

---

## B-0  Current implementation notes

| Item | Current state in `service_users/src/main.rs` |
|---|---|
| GraphQL schema | `UserQuery` with `me` resolver. Schema: `Schema<UserQuery, EmptyMutation, EmptySubscription>`. |
| REST endpoints | `GET /api/v1/users/me` (stub returning `{"status":"ok"}`), `GET /api/v1/admin/audit` (admin-gated). |
| Repository layer | `PostgresUserRepository`, `SqliteUserRepository` behind `UserRepository` trait. `find_first_by_tenant()`. |
| Authentication | Uses `AuthContext` from `krab_core::http`. GraphQL handler receives it via `Extension<AuthContext>`. |
| Admin RBAC | `admin_rbac_middleware` checks for admin scope/role before `/admin/*` routes. |
| DB migrations | 7 versioned migrations, governance + drift detection. |
| Contract tests | GraphQL schema snapshot + auth-requirement tests. |

---

## B-1  Restructure service into domain + adapters

### B-1.1  New directory layout

```
service_users/
├── Cargo.toml
├── contracts/
│   └── graphql_schema_v1.graphql   (existing)
└── src/
    ├── main.rs                     (bootstrap + router assembly)
    ├── domain/
    │   ├── mod.rs
    │   ├── models.rs               (User, UserProfile, …)
    │   ├── service.rs              (UserDomainService trait + impl)
    │   └── errors.rs               (domain-layer error enum)
    ├── adapters/
    │   ├── mod.rs
    │   ├── rest.rs                 (axum REST handlers)
    │   ├── graphql.rs              (async-graphql resolvers)
    │   └── rpc.rs                  (JSON-RPC / tonic handlers)
    ├── db/
    │   ├── mod.rs
    │   ├── postgres.rs             (existing PostgresUserRepository)
    │   └── sqlite.rs               (existing SqliteUserRepository)
    └── capabilities.rs             (ServiceCapabilities construction)
```

### B-1.2  Domain layer (`domain/`)

#### `domain/models.rs`

```rust
#[derive(Debug, Clone)]
pub struct UserModel {
    pub id: String,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct UserProfile {
    pub user_id: String,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
}
```

#### `domain/errors.rs`

```rust
#[derive(Debug)]
pub enum DomainError {
    TenantRequired,
    NotFound,
    Unauthorized,
    Internal(String),
}
```

#### `domain/service.rs`

```rust
use async_trait::async_trait;
use super::models::UserModel;
use super::errors::DomainError;

/// Protocol-agnostic domain interface.
/// All protocol adapters call these methods — business logic lives here.
#[async_trait]
pub trait UserDomainService: Send + Sync {
    /// Get the "me" user for the authenticated tenant.
    async fn get_me(&self, tenant_id: &str) -> Result<UserModel, DomainError>;

    /// (future) Get user by ID.  
    async fn get_user_by_id(&self, id: &str, tenant_id: &str) -> Result<UserModel, DomainError>;
}

/// Concrete implementation backed by UserRepository.
pub struct UserDomainServiceImpl {
    repo: Arc<dyn UserRepository>,
}

#[async_trait]
impl UserDomainService for UserDomainServiceImpl {
    async fn get_me(&self, tenant_id: &str) -> Result<UserModel, DomainError> {
        let record = self.repo
            .find_first_by_tenant(tenant_id)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))?;

        match record {
            Some(r) => Ok(UserModel { id: r.id, username: r.username }),
            None => Ok(UserModel {
                id: "1".to_string(),
                username: "krab_user".to_string(),
            }),
        }
    }

    async fn get_user_by_id(&self, id: &str, tenant_id: &str) -> Result<UserModel, DomainError> {
        // placeholder — extend when DB supports single-user lookup
        Err(DomainError::NotFound)
    }
}
```

### B-1.3  REST adapter (`adapters/rest.rs`)

```rust
use axum::{extract::Extension, Json, routing::get, Router};
use krab_core::http::AuthContext;
use crate::domain::service::UserDomainService;
use std::sync::Arc;

pub fn rest_router(domain: Arc<dyn UserDomainService>) -> Router {
    Router::new()
        .route("/users/me", get(get_me_handler))
        .with_state(domain)
}

async fn get_me_handler(
    Extension(auth): Extension<AuthContext>,
    State(domain): State<Arc<dyn UserDomainService>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let tenant_id = auth.tenant_id.as_deref()
        .ok_or((StatusCode::BAD_REQUEST, Json(ApiError::new("BAD_REQUEST", "tenant context required"))))?;

    let user = domain.get_me(tenant_id).await
        .map_err(|e| domain_error_to_rest(e))?;

    Ok(Json(json!({ "id": user.id, "username": user.username })))
}

fn domain_error_to_rest(err: DomainError) -> (StatusCode, Json<ApiError>) {
    match err {
        DomainError::TenantRequired => (StatusCode::BAD_REQUEST, Json(ApiError::new("BAD_REQUEST", "tenant required"))),
        DomainError::NotFound => (StatusCode::NOT_FOUND, Json(ApiError::new("NOT_FOUND", "user not found"))),
        DomainError::Unauthorized => (StatusCode::FORBIDDEN, Json(ApiError::new("FORBIDDEN", "access denied"))),
        DomainError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError::new("INTERNAL", msg))),
    }
}
```

### B-1.4  GraphQL adapter (`adapters/graphql.rs`)

Move existing `UserQuery` resolver here, but back it with the domain service:

```rust
use async_graphql::{Context, Object, SimpleObject, Schema, EmptyMutation, EmptySubscription};
use crate::domain::service::UserDomainService;
use std::sync::Arc;

#[derive(SimpleObject)]
pub struct User {
    pub id: String,
    pub username: String,
}

pub struct UserQuery;

#[Object]
impl UserQuery {
    async fn me(&self, ctx: &Context<'_>) -> async_graphql::Result<User> {
        let domain = ctx.data::<Arc<dyn UserDomainService>>()?;
        let auth = ctx.data::<AuthContext>()?;

        let tenant_id = auth.tenant_id.as_deref()
            .ok_or_else(|| async_graphql::Error::new("tenant context is required"))?;

        let model = domain.get_me(tenant_id).await
            .map_err(|e| async_graphql::Error::new(format!("{:?}", e)))?;

        Ok(User { id: model.id, username: model.username })
    }
}

pub type UsersSchema = Schema<UserQuery, EmptyMutation, EmptySubscription>;

pub fn build_schema(domain: Arc<dyn UserDomainService>) -> UsersSchema {
    Schema::build(UserQuery, EmptyMutation, EmptySubscription)
        .data(domain)
        .finish()
}
```

### B-1.5  RPC adapter (`adapters/rpc.rs`)

JSON-RPC style (lightweight, no `.proto` file needed initially):

```rust
use axum::{Json, routing::post, Router, extract::State};
use krab_core::http::AuthContext;
use crate::domain::service::UserDomainService;

#[derive(Deserialize)]
struct RpcRequest {
    method: String,
    params: serde_json::Value,
    id: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct RpcResponse {
    result: Option<serde_json::Value>,
    error: Option<RpcError>,
    id: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

pub fn rpc_router(domain: Arc<dyn UserDomainService>) -> Router {
    Router::new()
        .route("/rpc", post(rpc_dispatch))
        .with_state(domain)
}

async fn rpc_dispatch(
    Extension(auth): Extension<AuthContext>,
    State(domain): State<Arc<dyn UserDomainService>>,
    Json(req): Json<RpcRequest>,
) -> Json<RpcResponse> {
    match req.method.as_str() {
        "users.getMe" => {
            let tenant_id = match auth.tenant_id.as_deref() {
                Some(t) => t,
                None => return rpc_error(-32602, "tenant context required", req.id),
            };
            match domain.get_me(tenant_id).await {
                Ok(user) => Json(RpcResponse {
                    result: Some(json!({ "id": user.id, "username": user.username })),
                    error: None,
                    id: req.id,
                }),
                Err(e) => rpc_error(-32000, &format!("{:?}", e), req.id),
            }
        }
        _ => rpc_error(-32601, "method not found", req.id),
    }
}

fn rpc_error(code: i32, message: &str, id: Option<serde_json::Value>) -> Json<RpcResponse> {
    Json(RpcResponse {
        result: None,
        error: Some(RpcError { code, message: message.to_string() }),
        id,
    })
}
```

---

## B-2  Main.rs router assembly (protocol-mode aware)

The `build_app()` function reads `ProtocolConfig` from env and assembles only the enabled adapters:

```rust
fn build_app(state: AppState) -> Router {
    let domain: Arc<dyn UserDomainService> = state.domain.clone();
    let proto_cfg = state.protocol_config.clone();

    let mut api = Router::new();

    // Mount adapters based on enabled protocols
    if proto_cfg.enabled_protocols.contains(&ProtocolKind::Graphql) {
        let schema = adapters::graphql::build_schema(domain.clone());
        api = api.route("/graphql", post(graphql_handler).with_state(schema));
    }

    if proto_cfg.enabled_protocols.contains(&ProtocolKind::Rest) {
        api = api.merge(adapters::rest::rest_router(domain.clone()));
    }

    if proto_cfg.enabled_protocols.contains(&ProtocolKind::Rpc) {
        api = api.merge(adapters::rpc::rpc_router(domain.clone()));
    }

    // Admin routes (always REST)
    let admin_api = Router::new()
        .route("/audit", get(admin_audit_handler))
        .route_layer(middleware::from_fn(admin_rbac_middleware));
    api = api.nest("/admin", admin_api);

    // Capabilities endpoint
    let capabilities = build_capabilities(&proto_cfg);
    api = api.route("/capabilities", get(capabilities_handler).with_state(capabilities));

    // Ops routes (always REST)
    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/ready", get(readiness_with_dependencies::<AppState>))
        .route("/metrics", get(metrics::<AppState>))
        .route("/metrics/prometheus", get(metrics_prometheus::<AppState>))
        .nest("/api/v1", api);

    apply_common_http_layers(app, state.clone()).with_state(state)
}
```

---

## B-3  Capabilities endpoint response

```json
{
  "service": "users",
  "default_protocol": "graphql",
  "supported_protocols": ["rest", "graphql", "rpc"],
  "protocol_routes": {
    "rest": "/api/v1/users",
    "graphql": "/api/v1/graphql",
    "rpc": "/api/v1/rpc"
  },
  "allow_client_override": true
}
```

---

## B-4  Parity tests

New test module `service_users/src/parity_tests.rs`:

| Test | Description |
|---|---|
| `parity_get_me_rest_equals_graphql` | Given same auth + tenant, REST `GET /api/v1/users/me` and GraphQL `{ me { id username } }` return identical `id` and `username`. |
| `parity_get_me_rpc_equals_graphql` | Same, but RPC `users.getMe` vs GraphQL. |
| `parity_tenant_required_all_protocols` | Missing tenant returns error on all 3 protocols. |
| `parity_auth_required_all_protocols` | No auth token → 401 on all 3 protocols. |

These tests ensure behavioral equivalence across all adapters.

---

## B-5  Service_users Cargo.toml changes

| Dependency | Reason |
|---|---|
| `serde_json` | Already present. Used in RPC adapter. |
| `tonic` (optional) | Only if later we upgrade from JSON-RPC to gRPC. Not required for Phase B. |

No new dependencies needed for JSON-RPC adapter.

---

## B-6  Migration notes

- **Zero breaking change**: existing `POST /api/v1/graphql` and `GET /api/v1/users/me` continue to work.
- When `KRAB_PROTOCOL_EXPOSURE_MODE` is unset, default to `multi` with `enabled = graphql,rest` to preserve backward compatibility.
- Domain extraction is a refactor, not a behavior change — existing GraphQL snapshot test must still pass.

---

## B-7  Acceptance gates for Phase B

- [ ] `service_users` starts in `single` mode with `protocol=graphql` — only GraphQL routes respond; REST and RPC return 404.
- [ ] `service_users` starts in `single` mode with `protocol=rest` — only REST routes respond.
- [ ] `service_users` starts in `single` mode with `protocol=rpc` — only RPC routes respond.
- [ ] `service_users` starts in `multi` mode with `enabled=rest,graphql,rpc` — all three respond.
- [ ] Parity tests pass.
- [ ] GraphQL schema snapshot test still passes.
- [ ] `GET /api/v1/capabilities` returns correct response.
- [ ] Operations with `AuthContext` behave identically across protocols.
- [ ] Admin RBAC works the same regardless of protocol mode.
