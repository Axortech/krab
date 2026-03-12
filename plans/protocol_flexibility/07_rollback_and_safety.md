# Rollback and Safety Model

> **Goal**: ensure that every phase can be rolled back safely without breaking existing services or client integrations.

---

## 1  General safety principles

1. **Additive-first**: all new code is added alongside existing code, not replacing it.
2. **Feature-gated**: new protocol behavior is opt-in via `KRAB_PROTOCOL_*` env vars. When unset, services behave exactly as they do today.
3. **Compatibility flags**: existing endpoints remain active and functional throughout migration.
4. **Parity-as-blocker**: parity test failures block releases, not runtime behavior.

---

## 2  Per-phase rollback procedures

### Phase A (Core Primitives)

| Risk | Rollback step |
|---|---|
| New `protocol.rs` breaks compilation | Remove `pub mod protocol;` from `lib.rs`. Compile succeeds immediately. |
| HTTP middleware changes break requests | The protocol middleware is only added to the middleware stack when `ServiceCapabilities` is in app state. Existing services don't set this — they are unaffected. |
| `ServiceConfig` change breaks deserialization | `protocol` field is `Option` with `#[serde(default)]` — missing field deserializes as `None`. |

### Phase B (Users Pilot)

| Risk | Rollback step |
|---|---|
| Domain extraction breaks GraphQL behavior | Revert `service_users/src/` to single `main.rs`. The `UserQuery` resolver code is preserved and can be reverted. |
| REST/RPC adapters return wrong data | Disable by setting `KRAB_PROTOCOL_EXPOSURE_MODE=single` + `KRAB_PROTOCOL_DEFAULT=graphql`. Only GraphQL routes mount. |
| Parity test failures | Non-passing adapters are disabled in production config while issues are investigated. |

### Phase C (Frontend Integration)

| Risk | Rollback step |
|---|---|
| Capability discovery fails | `ProtocolAwareClient` fallback logic calls the service default. If all else fails, frontend still makes direct HTTP calls (existing behavior). |
| Cache poisoning in capability cache | Set `cache_ttl` to 0 or restart frontend — cache is in-memory. |
| Split-topology routing errors | Remove `KRAB_PROTOCOL_SPLIT_TARGETS_JSON`. Client falls back to single base URL per service. |

### Phase D (Auth Guardrails)

| Risk | Rollback step |
|---|---|
| Capability endpoint leaks sensitive info | The endpoint returns only protocol metadata, never credentials or tokens. If needed, simply remove the route from `build_app`. |
| Auth operations accidentally exposed on non-REST | Not possible — auth adapter code is never scaffolded for GraphQL/RPC in Phase D. Hard restriction is in policy, not just config. |

### Phase E (Governance/CI/CLI)

| Risk | Rollback step |
|---|---|
| CI protocol matrix too slow | Run matrix only on release branches, not on every PR. |
| CLI generates broken multi-protocol scaffolds | Revert `krab_cli` generator changes. Existing `--type` flag continues to work as before. |
| Documentation inconsistency | Treat as non-blocking. Update docs independently. |

---

## 3  Emergency disable switches

| Switch | Effect |
|---|---|
| Unset all `KRAB_PROTOCOL_*` env vars | All services behave exactly as pre-protocol-flexibility baseline. |
| `KRAB_PROTOCOL_EXPOSURE_MODE=single` | Forces a single adapter. Set `KRAB_PROTOCOL_DEFAULT` to the known-good protocol. |
| `KRAB_PROTOCOL_ALLOW_CLIENT_OVERRIDE=false` | Prevents any client from requesting a non-default protocol. |
| Feature flag removal in `Cargo.toml` | Remove `graphql` or `grpc` feature from a service's dependency on `krab_core` to completely eliminate protocol adapter at compile time. |

---

## 4  Monitoring for rollback triggers

| Signal | Threshold | Action |
|---|---|---|
| Parity test failure in CI | Any failure | Block release, investigate. |
| Error rate spike on new protocol adapter | > 2× baseline | Auto-disable adapter, page on-call. |
| Latency regression on new adapter | > 3× p99 baseline | Alert, investigate possible rollback. |
| Capability endpoint 5xx rate | > 5% | Fallback to hardcoded default protocol in clients. |

---

## 5  Data migration considerations

- **No schema migrations required** for protocol flexibility. All changes are in application code and configuration.
- If split-topology is deployed with separate databases per protocol service, data synchronization becomes a concern — but this is explicitly discouraged in the plan (shared domain, shared data store is the recommendation).
