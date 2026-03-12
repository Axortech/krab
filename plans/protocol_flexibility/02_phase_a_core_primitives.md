# Phase A — Core Primitives (krab_core protocol layer)

> **Goal**: introduce the foundational runtime protocol types, policy resolver, HTTP header parsing, and capability-endpoint helper into `krab_core`, without changing any existing service behavior.

---

## A-0  Current implementation notes

| File | What exists today | Impact |
|---|---|---|
| `krab_core/Cargo.toml` | Feature flags: `rest`, `graphql`, `grpc`. Each gates optional deps (`axum`, `async-graphql`, `tonic`). | New `protocol.rs` must be available regardless of enabled features — it is feature-agnostic. |
| `krab_core/src/lib.rs` | Conditionally exposes `http`, `store`, `server_fn` under `#[cfg(feature="rest")]`. No `protocol` module. | Add `pub mod protocol;` **unconditionally**. |
| `krab_core/src/http.rs` | 1725 lines. `RuntimeState`, middleware stack (`apply_common_http_layers`), auth middleware with hard-coded path list, `AuthContext`, `ApiError`. | Extend `RuntimeState` to optionally carry `ServiceCapabilities`. Add protocol header parsing and tracing attribute injection. |
| `krab_core/src/service.rs` | `ApiService` trait (`start()`) + `ServiceConfig { name, host, port }`. | `ServiceConfig` gains optional `ProtocolConfig`. |

---

## A-1  New file: `krab_core/src/protocol.rs`

### A-1.1  Types

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Supported API transport protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolKind {
    Rest,
    Graphql,
    Rpc,
}

impl ProtocolKind {
    /// Parse from a case-insensitive string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rest" => Some(Self::Rest),
            "graphql" => Some(Self::Graphql),
            "rpc" | "grpc" => Some(Self::Rpc),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Graphql => "graphql",
            Self::Rpc => "rpc",
        }
    }
}

/// How many protocol adapters the service exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExposureMode {
    /// Exactly one protocol adapter.
    Single,
    /// Two or more protocol adapters.
    Multi,
}

/// Deployment shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentTopology {
    /// One process serves all adapters.
    SingleService,
    /// Each protocol gets its own microservice binary.
    SplitServices,
}

/// Advertised protocol capabilities for a service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceCapabilities {
    pub service: String,
    pub default_protocol: ProtocolKind,
    pub supported_protocols: Vec<ProtocolKind>,
    /// Maps protocol → base route, e.g. Rest → "/api/v1/users".
    pub protocol_routes: HashMap<ProtocolKind, String>,
    pub allow_client_override: bool,
}

/// Policy constraints on protocol selection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProtocolSelectionPolicy {
    /// Operation name → allowed protocols.
    /// If an operation is listed here, *only* those protocols are valid.
    pub restricted_operations: HashMap<String, Vec<ProtocolKind>>,
    /// Tenant ID → allowed protocols override.
    pub tenant_overrides: HashMap<String, Vec<ProtocolKind>>,
}

/// Full runtime protocol config loaded from env at startup.
#[derive(Debug, Clone)]
pub struct ProtocolConfig {
    pub exposure_mode: ExposureMode,
    pub enabled_protocols: Vec<ProtocolKind>,
    pub default_protocol: ProtocolKind,
    pub allow_client_override: bool,
    pub topology: DeploymentTopology,
    pub selection_policy: ProtocolSelectionPolicy,
}
```

### A-1.2  ProtocolConfig construction from env

```rust
impl ProtocolConfig {
    /// Build from environment variables.
    /// Env vars consumed:
    ///   KRAB_PROTOCOL_EXPOSURE_MODE    = single | multi
    ///   KRAB_PROTOCOL_ENABLED          = rest,graphql,rpc  (CSV)
    ///   KRAB_PROTOCOL_DEFAULT          = rest | graphql | rpc
    ///   KRAB_PROTOCOL_ALLOW_CLIENT_OVERRIDE = true | false
    ///   KRAB_PROTOCOL_TOPOLOGY         = single_service | split_services
    ///   KRAB_PROTOCOL_RESTRICTED_OPS_JSON = { "op": ["rest"] }
    ///   KRAB_PROTOCOL_TENANT_OVERRIDES_JSON = { "tenant-a": ["graphql"] }
    pub fn from_env() -> Self { /* … see implementation below */ }

    /// Validate invariants. Call at startup; panic on invalid config.
    pub fn validate(&self) -> Result<(), Vec<String>> { /* … */ }
}
```

**Validation rules** (at startup, hard-fail):

1. `default_protocol` ∈ `enabled_protocols`.
2. If `exposure_mode == Single`, then `enabled_protocols.len() == 1`.
3. Every protocol mentioned in `restricted_operations` values must be in `enabled_protocols`.
4. If `topology == SplitServices`, a split-targets map env var must be present.

### A-1.3  Protocol selection resolver

```rust
#[derive(Debug)]
pub enum ProtocolSelectionError {
    /// The requested protocol is not supported by this service.
    ProtocolNotSupported(ProtocolKind),
    /// The operation is restricted to other protocols.
    OperationRestricted {
        operation: String,
        allowed: Vec<ProtocolKind>,
    },
    /// Client override is not permitted for this service.
    ClientOverrideDisabled,
}

/// Deterministic protocol resolution.
/// Priority:
///   1. Operation restriction (hard).
///   2. Tenant override.
///   3. Client preference (if allowed).
///   4. Service default.
pub fn resolve_protocol(
    operation: &str,
    client_pref: Option<ProtocolKind>,
    caps: &ServiceCapabilities,
    policy: &ProtocolSelectionPolicy,
    tenant: Option<&str>,
) -> Result<ProtocolKind, ProtocolSelectionError> {
    // 1. Operation restriction
    if let Some(allowed) = policy.restricted_operations.get(operation) {
        if let Some(pref) = client_pref {
            if allowed.contains(&pref) {
                return Ok(pref);
            }
        }
        return allowed
            .first()
            .copied()
            .ok_or(ProtocolSelectionError::OperationRestricted {
                operation: operation.to_string(),
                allowed: allowed.clone(),
            });
    }

    // 2. Tenant override
    if let Some(tid) = tenant {
        if let Some(tenant_protos) = policy.tenant_overrides.get(tid) {
            if let Some(pref) = client_pref {
                if tenant_protos.contains(&pref) {
                    return Ok(pref);
                }
            }
            if let Some(first) = tenant_protos.first().copied() {
                return Ok(first);
            }
        }
    }

    // 3. Client preference
    if let Some(pref) = client_pref {
        if !caps.allow_client_override {
            return Err(ProtocolSelectionError::ClientOverrideDisabled);
        }
        if caps.supported_protocols.contains(&pref) {
            return Ok(pref);
        }
        return Err(ProtocolSelectionError::ProtocolNotSupported(pref));
    }

    // 4. Service default
    Ok(caps.default_protocol)
}
```

### A-1.4  Capability endpoint helper

```rust
use axum::{Json, extract::State};

/// Handler that any service can mount at `GET /api/capabilities`.
/// Requires `ServiceCapabilities` in state.
pub async fn capabilities_handler<S>(
    State(caps): State<ServiceCapabilities>,
) -> Json<ServiceCapabilities> {
    Json(caps)
}
```

---

## A-2  Extend `krab_core/src/http.rs`

### A-2.1  Parse `x-krab-protocol` header

Add a new small middleware or utility used by `apply_common_http_layers`:

```rust
/// Extract client protocol preference from the inbound request.
/// Looks at:
///   1. Header `x-krab-protocol`
///   2. Query param `protocol`  (lower priority)
/// Returns None if neither is present.
pub fn extract_protocol_preference(req: &Request<Body>) -> Option<ProtocolKind> {
    // header first
    if let Some(hv) = req.headers().get("x-krab-protocol") {
        if let Ok(s) = hv.to_str() {
            if let Some(pk) = ProtocolKind::parse(s) {
                return Some(pk);
            }
        }
    }
    // query param fallback
    if let Some(query) = req.uri().query() {
        for pair in query.split('&') {
            if let Some(val) = pair.strip_prefix("protocol=") {
                if let Some(pk) = ProtocolKind::parse(val) {
                    return Some(pk);
                }
            }
        }
    }
    None
}
```

### A-2.2  Inject resolved protocol into request extensions

The resolved `ProtocolKind` will be inserted into `req.extensions_mut()` so downstream handlers and tracing middleware can use it:

```rust
/// Middleware: resolve protocol and attach to request extensions.
async fn protocol_resolution_middleware<S>(
    State(state): State<S>,
    mut req: Request<Body>,
    next: Next,
) -> Response
where
    S: HasProtocolConfig + Clone + Send + Sync + 'static,
{
    let pref = extract_protocol_preference(&req);
    let resolved = pref.unwrap_or(state.protocol_config().default_protocol);
    req.extensions_mut().insert(resolved);
    next.run(req).await
}
```

### A-2.3  Add protocol label to tracing/metrics

In the existing `tracing_middleware`, extend the span with:

```rust
tracing::info_span!(
    "http_request",
    // existing fields …
    krab.protocol = %req.extensions().get::<ProtocolKind>()
        .map(|p| p.as_str())
        .unwrap_or("unknown"),
);
```

In `metrics_middleware`, tag the `protocol` label into the Prometheus format string. Add counter dimension `protocol="rest|graphql|rpc|unknown"`.

### A-2.4  New response header `x-krab-protocol`

After resolution, set response header `x-krab-protocol` to the resolved value so clients can confirm which protocol was used.

---

## A-3  Extend `krab_core/src/service.rs`

Add optional `ProtocolConfig` to `ServiceConfig`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    /// Protocol configuration. `None` = legacy single-protocol mode.
    #[serde(default)]
    pub protocol: Option<crate::protocol::ProtocolConfig>,
}
```

---

## A-4  New file: `krab_core/src/protocol_tests.rs`

Unit tests covering:

| Test | Asserts |
|---|---|
| `test_resolve_default_when_no_preference` | Returns `caps.default_protocol` when no client pref, no op restriction, no tenant override. |
| `test_resolve_client_preference_accepted` | Client pref is returned when `allow_client_override == true` and protocol is in `supported_protocols`. |
| `test_resolve_client_preference_denied_by_override_flag` | Returns `ClientOverrideDisabled` error when `allow_client_override == false`. |
| `test_resolve_client_preference_unsupported` | Returns `ProtocolNotSupported` error when client requests a protocol not in `supported_protocols`. |
| `test_resolve_operation_restriction_overrides_client` | Even if client prefers `graphql`, `operation_restricted["login"] = ["rest"]` forces `rest`. |
| `test_resolve_tenant_override` | Tenant override narrows available protocols. |
| `test_resolve_priority_order` | Operation restriction > tenant > client > default. |
| `test_config_validation_default_in_enabled` | Fails if default is not in enabled set. |
| `test_config_validation_single_mode_one_protocol` | Fails if `Single` mode has ≠ 1 enabled. |
| `test_parse_protocol_kind_case_insensitive` | `"REST"`, `"Graphql"`, `"rpc"`, `"GRPC"` all parse correctly. |
| `test_parse_protocol_kind_invalid` | `"soap"`, `""`, `"xml"` return `None`. |
| `test_extract_protocol_from_header` | Extracts from `x-krab-protocol` header. |
| `test_extract_protocol_from_query` | Extracts from `?protocol=rest` query param. |
| `test_header_takes_priority_over_query` | Header wins when both are present. |

Declare in `lib.rs`:
```rust
#[cfg(all(feature = "rest", test))]
mod protocol_tests;
```

---

## A-5  krab_core Cargo.toml changes

The `protocol` module uses only `serde`, `serde_json`, and `std` — all already available. No new deps needed.

---

## A-6  Wire changes summary

| File | Change |
|---|---|
| `krab_core/src/lib.rs` | Add `pub mod protocol;` (unconditional). Add `#[cfg(all(feature = "rest", test))] mod protocol_tests;` |
| `krab_core/src/protocol.rs` | **[NEW]** Types + resolver + config loader + capability handler. |
| `krab_core/src/protocol_tests.rs` | **[NEW]** Unit tests. |
| `krab_core/src/http.rs` | Add `extract_protocol_preference()`, `protocol_resolution_middleware`, extend tracing/metrics labels, add response header. |
| `krab_core/src/service.rs` | Add `protocol: Option<ProtocolConfig>` to `ServiceConfig`. |
| `krab_core/Cargo.toml` | No new dependencies. |

---

## A-7  Rollback safety

- All new code is additive; no existing behavior changes.
- `protocol` field on `ServiceConfig` is `Option` — defaults to `None`, making existing services unaffected.
- Protocol middleware is only wired when `ServiceCapabilities` is present in state.
- The new module compiles on all feature combinations because it does not depend on `axum`, `async-graphql`, or `tonic` directly (capability handler requires `rest` feature, guarded by `#[cfg(feature = "rest")]`).

---

## A-8  Acceptance gates for Phase A

- [ ] `cargo test -p krab_core --features rest -- protocol` passes all A-4 tests.
- [ ] `cargo build -p krab_core` (no features) compiles — protocol module has no feature deps.
- [ ] `cargo build -p krab_core --features rest,graphql,grpc` compiles.
- [ ] Existing `service_auth` and `service_users` tests still pass with zero code changes to those crates.
