# Phase D — Auth Service Guardrails

> **Goal**: add capability endpoint to `service_auth`, hard-lock sensitive operations to REST, and optionally expose non-sensitive read operations on other protocols after security review.

---

## D-0  Current implementation notes

| Item | Current state in `service_auth/src/main.rs` |
|---|---|
| Endpoints | `POST /api/v1/auth/login`, `POST /api/v1/auth/refresh`, `POST /api/v1/auth/revoke`, `GET /api/v1/auth/jwks`, `GET /api/v1/auth/status`. |
| Auth mode | Configurable via `KRAB_AUTH_MODE` (jwt/oidc/static). |
| Security | Token issuance uses key ring, refresh rotation with one-time-use tracking, revocation via store. |
| Readiness | `HasReadinessDependencies` impl with `auth_runtime` check. |
| Tests | Integration tests for health, auth lifecycle, rate limiting, Prometheus metrics. |

---

## D-1  Capability endpoint for auth

### Route: `GET /api/v1/auth/capabilities`

```json
{
  "service": "auth",
  "default_protocol": "rest",
  "supported_protocols": ["rest"],
  "protocol_routes": {
    "rest": "/api/v1/auth"
  },
  "allow_client_override": false,
  "policy": {
    "restricted_operations": {
      "auth.login": ["rest"],
      "auth.refresh": ["rest"],
      "auth.revoke": ["rest"],
      "auth.jwks": ["rest"],
      "auth.status": ["rest"]
    }
  }
}
```

### Implementation in `service_auth/src/main.rs`:

```rust
async fn auth_capabilities_handler() -> Json<serde_json::Value> {
    Json(json!({
        "service": "auth",
        "default_protocol": "rest",
        "supported_protocols": ["rest"],
        "protocol_routes": { "rest": "/api/v1/auth" },
        "allow_client_override": false,
        "policy": {
            "restricted_operations": {
                "auth.login": ["rest"],
                "auth.refresh": ["rest"],
                "auth.revoke": ["rest"],
                "auth.jwks": ["rest"],
                "auth.status": ["rest"]
            }
        }
    }))
}
```

Add to `build_app`:
```rust
.route("/auth/capabilities", get(auth_capabilities_handler))
```

Add `/api/v1/auth/capabilities` to the auth middleware open paths list.

---

## D-2  Hard restrictions: security-sensitive operations

These operations **MUST** remain REST-only. This is a non-negotiable policy lock:

| Operation | Route | Protocol Lock | Rationale |
|---|---|---|---|
| `auth.login` | `POST /api/v1/auth/login` | REST only | Token issuance is security-critical; HTTP headers/cookies are the standard transport. |
| `auth.refresh` | `POST /api/v1/auth/refresh` | REST only | Rotation tracking requires strict request/response envelope. |
| `auth.revoke` | `POST /api/v1/auth/revoke` | REST only | Side-effect operation; must be idempotent and auditable in REST form. |
| `auth.jwks` | `GET /api/v1/auth/jwks` | REST only | JWKS is an industry-standard REST endpoint format. |

---

## D-3  Optional limited RPC/GraphQL read operations (future, gated by security review)

After Phase D is stable, a **separate security review** may approve adding:

| Operation | Potential GraphQL | Potential RPC |
|---|---|---|
| `auth.status` | `query { authStatus { status active_kid key_count auth_mode } }` | `auth.getStatus` |
| `auth.introspect` | `query { tokenIntrospect(token: "...") { active sub exp } }` | `auth.introspect` |

**Conditions for approval**:
1. Read-only — no state mutation.
2. No credential material in request or response.
3. Same auth middleware applied.
4. Audit logged.
5. Parity test suite must exist before enablement.

Until explicitly approved, auth remains REST-only.

---

## D-4  Service_auth changes summary

| File | Change |
|---|---|
| `service_auth/src/main.rs` | Add `auth_capabilities_handler`. Add route. Add path to open-auth list. |
| `service_auth/src/main.rs` | Import `ProtocolConfig` from `krab_core::protocol`. Read protocol env to validate at startup (but auth ignores multi-mode — it is always REST). |

No new files needed. Auth remains a single-file service.

---

## D-5  Testing

| Test | Description |
|---|---|
| `test_capabilities_endpoint_returns_rest_only` | `GET /api/v1/auth/capabilities` returns the expected JSON with `supported_protocols: ["rest"]`. |
| `test_login_via_non_rest_rejected` | If a GraphQL or RPC endpoint were somehow mounted (future), login operations must be rejected. |
| `test_existing_lifecycle_unaffected` | All existing auth tests still pass. |

---

## D-6  Acceptance gates for Phase D

- [ ] `GET /api/v1/auth/capabilities` returns correct response.
- [ ] All existing `service_auth` tests still pass.
- [ ] Auth lifecycle operations (login/refresh/revoke) remain REST-only and function correctly.
- [ ] No new protocol adapters are exposed on auth until explicit security review approval.
