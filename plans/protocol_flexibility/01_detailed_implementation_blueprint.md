# Detailed Implementation Blueprint (REST + GraphQL + RPC)

This blueprint translates strategy into implementation work packages, based on the current baseline captured in [`00_current_state_acknowledgement.md`](./00_current_state_acknowledgement.md).

---

## 1) Implementation goals

1. Introduce **runtime protocol selection** while preserving existing route compatibility.
2. Enable per-service modes:
   - REST-only
   - GraphQL-only
   - RPC-only
   - Multi-protocol (any subset of REST/GraphQL/RPC)
3. Support deployment topologies:
   - single service process with multiple adapters
   - split protocol-specific microservices
4. Keep auth/security/tenant policy behavior identical across protocol adapters.

---

## 2) Current codebase touch map (acknowledged baseline)

### 2.1 Existing files to extend

- `krab_core/src/http.rs`
  - central middleware and HTTP utility behavior.
- `krab_core/Cargo.toml`
  - protocol feature declarations already present.
- `krab_cli/src/main.rs`
  - current generator-time protocol enum and scaffolding flow.
- `service_auth/src/main.rs`
  - REST auth lifecycle routes and startup policy checks.
- `service_users/src/main.rs`
  - GraphQL-first API and selected REST endpoints.
- `service_frontend/src/main.rs`
  - downstream status/readiness calls and route surfaces.

### 2.2 New files/modules to introduce (proposed)

- `krab_core/src/protocol.rs`
  - protocol enums, capability model, mode/topology model, resolver logic.
- `krab_core/src/protocol_tests.rs`
  - unit tests for selection and policy precedence.
- `docs/protocol_flexibility.md` (optional public-facing contract doc later)
  - runtime capability and selection behavior for integrators.

---

## 3) Core runtime implementation (krab_core)

### 3.1 Data model and policy primitives

Add core types in `krab_core/src/protocol.rs`:

```rust
enum ProtocolKind { Rest, Graphql, Rpc }
enum ExposureMode { Single, Multi }
enum DeploymentTopology { SingleService, SplitServices }

struct ServiceCapabilities {
    service: String,
    default_protocol: ProtocolKind,
    supported_protocols: Vec<ProtocolKind>,
    protocol_routes: HashMap<ProtocolKind, String>,
    allow_client_override: bool,
}

struct ProtocolSelectionPolicy {
    restricted_operations: HashMap<String, Vec<ProtocolKind>>,
    tenant_overrides: HashMap<String, Vec<ProtocolKind>>,
}
```

### 3.2 Selection resolver

Add a deterministic resolver API:

```rust
fn resolve_protocol(
    operation: &str,
    client_pref: Option<ProtocolKind>,
    caps: &ServiceCapabilities,
    policy: &ProtocolSelectionPolicy,
    tenant: Option<&str>,
) -> Result<ProtocolKind, ProtocolSelectionError>
```

Resolution order (must remain stable):

1. operation restriction,
2. tenant override,
3. client preference (if allowed),
4. service default.

### 3.3 HTTP integration points

Extend shared HTTP layer usage in `krab_core/src/http.rs`:

- parse `x-krab-protocol` header and validate into `ProtocolKind`.
- attach resolved protocol into request extensions for downstream handlers.
- include resolved protocol in metrics labels and tracing attributes.

### 3.4 Capability endpoint contract helper

Provide helper to emit standardized JSON for `GET /api/capabilities` from any service.

---

## 4) CLI implementation (krab_cli)

### 4.1 Extend service generation model

Current enum in `krab_cli/src/main.rs` already supports protocol-at-generation (`rest`, `graphql`, `grpc`).

Expand generator inputs:

- `--exposure-mode single|multi`
- `--protocols rest,graphql,rpc`
- `--topology single_service|split_services`

### 4.2 Template generation behavior

For `single` mode:

- scaffold only selected adapter and route registration.

For `multi` mode:

- scaffold adapter module split:
  - `src/domain/`
  - `src/adapters/rest.rs`
  - `src/adapters/graphql.rs`
  - `src/adapters/rpc.rs`
- scaffold capabilities endpoint.
- scaffold protocol selection wiring.

For `split_services` topology:

- scaffold optional sibling service packages sharing a generated domain crate.

---

## 5) Service implementation details

### 5.1 service_auth

Current state: REST auth lifecycle is explicit and security-sensitive.

Implementation plan:

1. Add `GET /api/capabilities` response declaring:
   - supported protocols (likely REST for sensitive operations),
   - operation restrictions for login/refresh/revoke.
2. Keep auth lifecycle operation restrictions hard-coded in policy layer.
3. Optionally add RPC only for non-sensitive status/read methods after security review.

### 5.2 service_users

Current state: GraphQL-first; best candidate for multi-protocol pilot.

Implementation plan:

1. Extract domain operations behind trait interfaces:
   - `get_me`, `get_profile`, etc.
2. Ensure existing GraphQL resolver uses extracted domain trait.
3. Add REST adapter mapping same domain operations.
4. Add RPC adapter mapping same domain operations.
5. Add `GET /api/capabilities` with real routes and defaults.

### 5.3 service_frontend

Current state: direct downstream HTTP calls to auth/users health/status.

Implementation plan:

1. Add downstream protocol-aware client helper:
   - fetch capabilities,
   - resolve protocol via shared policy,
   - call REST/GraphQL/RPC transport.
2. Keep operational probes (`/ready`/`/health`) on HTTP REST for reliability.
3. Add fallback behavior when selected protocol unavailable (policy-controlled).

---

## 6) Configuration and bootstrap changes

### 6.1 Environment variables

Standardize per-service protocol config:

- `KRAB_PROTOCOL_EXPOSURE_MODE=single|multi`
- `KRAB_PROTOCOL_ENABLED=rest,graphql,rpc`
- `KRAB_PROTOCOL_DEFAULT=rest|graphql|rpc`
- `KRAB_PROTOCOL_ALLOW_CLIENT_OVERRIDE=true|false`
- `KRAB_PROTOCOL_TOPOLOGY=single_service|split_services`
- `KRAB_PROTOCOL_RESTRICTED_OPS_JSON={...}`
- `KRAB_PROTOCOL_SPLIT_TARGETS_JSON={...}`

### 6.2 Validation rules

At startup:

1. `default` must be in enabled set.
2. `single` mode must have exactly one enabled protocol.
3. `split_services` requires explicit target map for enabled protocols.
4. restricted operations must not reference disabled protocols.

---

## 7) Governance and docs implementation

### 7.1 Governance doc updates

Update `plans/api_governance.md` with:

- protocol parity requirements,
- exposure mode change classification,
- protocol-specific deprecation policy.

### 7.2 API reference updates

Update `docs/API.md` to include:

- capability endpoint,
- protocol selection semantics,
- operation-level restrictions for auth-sensitive flows.

---

## 8) Testing implementation plan

### 8.1 Core tests

- resolver precedence tests,
- invalid configuration tests,
- request header parsing tests.

### 8.2 Service tests (users pilot)

- GraphQL vs REST vs RPC semantic parity tests.
- authorization parity tests across protocols.
- tenant scope parity tests.

### 8.3 Negative policy tests

- client preference denied by policy,
- operation restricted to specific protocol,
- disabled protocol access returns deterministic error envelope.

### 8.4 CI gates

Add protocol matrix in CI:

- `mode=single` with each protocol,
- `mode=multi` with selected combinations,
- parity suite mandatory for release branches.

---

## 9) Observability implementation

### 9.1 Metrics

Add/extend metrics labels with `protocol`:

- request totals,
- auth failures,
- latency histograms,
- response-class counters.

### 9.2 Tracing

Attach attributes to spans:

- `krab.protocol`,
- `krab.operation`,
- `krab.selection_source` (`policy|tenant|client|default`).

### 9.3 Dashboard changes

Add protocol-segmented views for latency/error/rate per service and per operation.

---

## 10) Phased execution plan

### Phase A — Core primitives

- add `protocol.rs` + resolver + tests,
- wire parsing and trace/metric tagging.

### Phase B — Users pilot adapters

- extract domain trait,
- wire REST + GraphQL + RPC adapters,
- expose capabilities endpoint.

### Phase C — Frontend integration

- protocol-aware downstream client,
- capability discovery cache + fallback rules.

### Phase D — Auth guardrails

- capability endpoint for auth,
- hard restrictions for sensitive operations,
- optional limited RPC read operation review.

### Phase E — Governance/CI hardening

- finalize docs,
- enforce parity and policy gates in CI,
- publish migration notes.

---

## 11) Rollback and safety model

1. Keep existing endpoints active behind compatibility flags.
2. Allow per-service forced default protocol if resolver issues occur.
3. Support disable switches for protocol adapters without full redeploy where possible.
4. Treat parity failures as release blockers, not runtime surprises.

---

## 12) Acceptance checklist (implementation)

1. Users service runs successfully in REST-only, GraphQL-only, RPC-only, and multi mode.
2. One shared domain implementation drives all enabled adapters.
3. Auth-sensitive operations remain policy-locked.
4. Frontend can choose protocol using capabilities + policy.
5. CI enforces parity and policy matrix.
6. Observability clearly reports protocol-specific behavior.

