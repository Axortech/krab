# Phase C — Frontend Protocol-Aware Integration

> **Goal**: make `service_frontend` discover downstream service capabilities at runtime, resolve the preferred protocol per policy, and issue downstream calls accordingly.

---

## C-0  Current implementation notes

| Item | Current state in `service_frontend/src/main.rs` |
|---|---|
| HTTP client | `reqwest::Client` stored in `AppState`. Used for probing auth/users health. |
| Downstream URLs | `auth_base_url`, `users_base_url` strings from env. |
| Downstream calls | Hard-coded HTTP REST calls to `/api/v1/auth/status`, `/ready`. No protocol selection. |
| RPC-like routes | Serves `/rpc/now`, `/rpc/version`, `/data/dashboard` as local server functions — not downstream RPC calls. |
| Caching | ISR cache + distributed store. |

---

## C-1  New module: `service_frontend/src/protocol_client.rs`

### C-1.1  Capability discovery client

```rust
use krab_core::protocol::{ServiceCapabilities, ProtocolKind, ProtocolSelectionPolicy, resolve_protocol};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};

/// Cached capability entry with TTL.
struct CachedCapability {
    caps: ServiceCapabilities,
    fetched_at: Instant,
}

/// Protocol-aware downstream client.
/// Discovers capabilities, caches them, resolves protocol, and calls the correct adapter.
pub struct ProtocolAwareClient {
    http: Client,
    /// service_name → base_url for capability discovery.
    service_urls: HashMap<String, String>,
    /// Cached capabilities per service.
    cache: Arc<RwLock<HashMap<String, CachedCapability>>>,
    /// How long to cache capability responses.
    cache_ttl: Duration,
    /// Frontend's own selection policy.
    policy: ProtocolSelectionPolicy,
}

impl ProtocolAwareClient {
    pub fn new(
        http: Client,
        service_urls: HashMap<String, String>,
        policy: ProtocolSelectionPolicy,
        cache_ttl: Duration,
    ) -> Self {
        Self {
            http,
            service_urls,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl,
            policy,
        }
    }

    /// Fetch (or return cached) capabilities for a service.
    pub async fn capabilities(&self, service: &str) -> anyhow::Result<ServiceCapabilities> {
        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(service) {
                if entry.fetched_at.elapsed() < self.cache_ttl {
                    return Ok(entry.caps.clone());
                }
            }
        }

        // Fetch from service
        let base_url = self.service_urls.get(service)
            .ok_or_else(|| anyhow::anyhow!("unknown service: {}", service))?;

        let url = format!("{}/api/v1/capabilities", base_url);
        let resp = self.http.get(&url)
            .timeout(Duration::from_secs(3))
            .send()
            .await?;

        let caps: ServiceCapabilities = resp.json().await?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(service.to_string(), CachedCapability {
                caps: caps.clone(),
                fetched_at: Instant::now(),
            });
        }

        Ok(caps)
    }

    /// Resolve which protocol to use for a given operation on a given service.
    pub async fn resolve(
        &self,
        service: &str,
        operation: &str,
        client_pref: Option<ProtocolKind>,
        tenant: Option<&str>,
    ) -> anyhow::Result<ProtocolKind> {
        let caps = self.capabilities(service).await?;
        resolve_protocol(operation, client_pref, &caps, &self.policy, tenant)
            .map_err(|e| anyhow::anyhow!("{:?}", e))
    }

    /// Make a downstream call using the resolved protocol.
    pub async fn call(
        &self,
        service: &str,
        operation: &str,
        client_pref: Option<ProtocolKind>,
        tenant: Option<&str>,
        payload: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let protocol = self.resolve(service, operation, client_pref, tenant).await?;
        let caps = self.capabilities(service).await?;
        let base_url = self.service_urls.get(service)
            .ok_or_else(|| anyhow::anyhow!("unknown service: {}", service))?;

        match protocol {
            ProtocolKind::Rest => self.call_rest(base_url, operation, payload).await,
            ProtocolKind::Graphql => self.call_graphql(base_url, operation, payload).await,
            ProtocolKind::Rpc => self.call_rpc(base_url, operation, payload).await,
        }
    }

    async fn call_rest(
        &self, base_url: &str, operation: &str, _payload: Option<serde_json::Value>
    ) -> anyhow::Result<serde_json::Value> {
        // Map operation to REST route
        let route = match operation {
            "users.getMe" => "/api/v1/users/me",
            "auth.status" => "/api/v1/auth/status",
            _ => return Err(anyhow::anyhow!("no REST mapping for {}", operation)),
        };
        let url = format!("{}{}", base_url, route);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.json().await?)
    }

    async fn call_graphql(
        &self, base_url: &str, operation: &str, _payload: Option<serde_json::Value>
    ) -> anyhow::Result<serde_json::Value> {
        let query = match operation {
            "users.getMe" => r#"{ me { id username } }"#,
            _ => return Err(anyhow::anyhow!("no GraphQL mapping for {}", operation)),
        };
        let url = format!("{}/api/v1/graphql", base_url);
        let body = serde_json::json!({ "query": query });
        let resp = self.http.post(&url)
            .json(&body)
            .send()
            .await?;
        Ok(resp.json().await?)
    }

    async fn call_rpc(
        &self, base_url: &str, operation: &str, params: Option<serde_json::Value>
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/api/v1/rpc", base_url);
        let body = serde_json::json!({
            "method": operation,
            "params": params.unwrap_or(serde_json::Value::Null),
            "id": 1
        });
        let resp = self.http.post(&url)
            .json(&body)
            .send()
            .await?;
        let rpc_resp: serde_json::Value = resp.json().await?;
        if let Some(error) = rpc_resp.get("error") {
            return Err(anyhow::anyhow!("RPC error: {}", error));
        }
        Ok(rpc_resp.get("result").cloned().unwrap_or(serde_json::Value::Null))
    }
}
```

### C-1.2  Fallback behavior

```rust
impl ProtocolAwareClient {
    /// Call with fallback: if the resolved protocol fails, try the default.
    pub async fn call_with_fallback(
        &self,
        service: &str,
        operation: &str,
        client_pref: Option<ProtocolKind>,
        tenant: Option<&str>,
        payload: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        match self.call(service, operation, client_pref, tenant, payload.clone()).await {
            Ok(result) => Ok(result),
            Err(primary_err) => {
                tracing::warn!(
                    service = service,
                    operation = operation,
                    error = %primary_err,
                    "protocol_call_failed_trying_fallback"
                );
                // Fallback: call service default protocol
                let caps = self.capabilities(service).await?;
                let default = caps.default_protocol;
                match default {
                    ProtocolKind::Rest => self.call_rest(
                        self.service_urls.get(service).unwrap(), operation, payload
                    ).await,
                    ProtocolKind::Graphql => self.call_graphql(
                        self.service_urls.get(service).unwrap(), operation, payload
                    ).await,
                    ProtocolKind::Rpc => self.call_rpc(
                        self.service_urls.get(service).unwrap(), operation, payload
                    ).await,
                }
            }
        }
    }
}
```

---

## C-2  Integrate into `service_frontend/src/main.rs`

### C-2.1  AppState extension

```rust
struct AppState {
    runtime: RuntimeState,
    http_client: Client,
    auth_base_url: String,
    users_base_url: String,
    isr_cache: IsrCache,
    isr_revalidating: Arc<tokio::sync::Mutex<HashSet<String>>>,
    hmr_rx: tokio::sync::watch::Receiver<u64>,
    // NEW
    protocol_client: Arc<ProtocolAwareClient>,
}
```

### C-2.2  Bootstrap protocol client at startup

```rust
let protocol_client = Arc::new(ProtocolAwareClient::new(
    http_client.clone(),
    HashMap::from([
        ("auth".to_string(), auth_base_url.clone()),
        ("users".to_string(), users_base_url.clone()),
    ]),
    ProtocolSelectionPolicy::default(), // or load from env
    Duration::from_secs(60), // cache capability responses for 60s
));
```

### C-2.3  Ops probes stay on REST

`/ready`, `/health`, `/metrics` calls to downstream services remain direct HTTP REST to avoid dependency on capability discovery for liveness checks.

---

## C-3  Split-topology support

When `KRAB_PROTOCOL_TOPOLOGY=split_services`, the frontend needs separate URLs per protocol:

```
KRAB_PROTOCOL_SPLIT_TARGETS_JSON={
  "users": {
    "rest": "http://users-rest:3002",
    "graphql": "http://users-graphql:3002",
    "rpc": "http://users-rpc:3002"
  }
}
```

`ProtocolAwareClient::call_rest/call_graphql/call_rpc` will pick the correct base URL from the split-targets map when available.

---

## C-4  Testing

| Test | Description |
|---|---|
| `test_capability_discovery_and_caching` | Mock HTTP server returns capabilities; second call within TTL returns cached. |
| `test_resolve_and_call_rest` | Resolves to REST, makes GET request. |
| `test_resolve_and_call_graphql` | Resolves to GraphQL, makes POST request with query body. |
| `test_resolve_and_call_rpc` | Resolves to RPC, makes POST request with JSON-RPC body. |
| `test_fallback_on_primary_failure` | Primary protocol call fails, falls back to default. |
| `test_ops_probes_are_always_rest` | Health/ready probes bypass protocol client. |

---

## C-5  Acceptance gates for Phase C

- [ ] Frontend starts and discovers capabilities from running `service_users`.
- [ ] Frontend can make downstream calls using whichever protocol is resolved.
- [ ] Ops probes (`/ready`, `/health`) still work if capability endpoint is unavailable.
- [ ] Capability cache respects TTL.
- [ ] Fallback works when primary protocol adapter is down.
