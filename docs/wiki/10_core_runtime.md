# Core Runtime (`krab_core`)

## Scope and responsibility

[`krab_core`](../../krab_core/src/lib.rs) is the platform contract layer for all runtime crates and services. It defines:

- configuration loading and validation semantics
- shared HTTP middleware and response contracts
- resilience primitives and telemetry bootstrap
- database governance and migration controls
- signal/reactivity primitives used by rendering layers

Think of this crate as the "operating system" for the framework.

---

## Module-by-module behavior

### 1) Configuration and environment policy

Primary file: [`krab_core/src/config.rs`](../../krab_core/src/config.rs)

Responsibilities:

- Resolve environment defaults for service name/host/port.
- Parse HTTP/security-related config (CORS origins, auth mode, limits).
- Enforce production safety rules (secret sourcing and non-dev constraints).

Invariants:

- Non-dev environments should fail fast on insecure or missing secret sources.
- Config parsing should be deterministic and typed.

Extension pattern:

1. Add new field to config struct.
2. Add parsing default in `from_env`.
3. Add explicit validation in `validate`.
4. Add unit tests for both valid and invalid paths.

### 2) HTTP runtime and middleware composition

Primary file: [`krab_core/src/http.rs`](../../krab_core/src/http.rs)

Responsibilities:

- Build common middleware layers applied by services.
- Provide health/readiness/metrics handlers and response formats.
- Enforce auth + security headers + request tracing conventions.

Critical middleware expectations:

- Auth checks should fail closed on invalid credentials.
- Header parsing paths should never panic.
- CORS behavior should map invalid configuration to controlled HTTP responses.

Operational outputs:

- `/health`, `/ready`, `/metrics`, `/metrics/prometheus`
- request identifiers propagated via headers/log fields

### 3) Database connectivity and governance

Primary file: [`krab_core/src/db.rs`](../../krab_core/src/db.rs)

Responsibilities:

- DB pool construction from typed config.
- migration execution and checksum validation.
- drift detection and rollback rehearsal helpers.
- promotion policy and governance audit records.

Governance invariants:

- migration drift must be detectable and reportable.
- promotion ordering should be policy-driven, not implicit.
- rollback flows must be testable and reproducible in CI.

### 4) Resilience primitives

Primary file: [`krab_core/src/resilience.rs`](../../krab_core/src/resilience.rs)

Responsibilities:

- Circuit breaker behavior for transient dependency failures.
- retry/backoff helper behavior for external calls.

Usage model:

- Services and orchestrator should use shared primitives instead of ad-hoc loops.

### 5) Telemetry bootstrap

Primary file: [`krab_core/src/telemetry.rs`](../../krab_core/src/telemetry.rs)

Responsibilities:

- tracing initialization and baseline telemetry dimensions.
- consistent service naming/environment labels.

### 6) Signal/reactivity primitives

Primary file: [`krab_core/src/signal.rs`](../../krab_core/src/signal.rs)

Responsibilities:

- reactive signal creation and effect execution model used by render/hydration logic.

---

## Integration boundaries

Used directly by:

- [`service_auth/src/main.rs`](../../service_auth/src/main.rs)
- [`service_users/src/main.rs`](../../service_users/src/main.rs)
- [`service_frontend/src/main.rs`](../../service_frontend/src/main.rs)
- [`krab_orchestrator/src/main.rs`](../../krab_orchestrator/src/main.rs)

Depends conceptually with:

- runtime server layer: [11_server_runtime.md](11_server_runtime.md)
- governance policy: [21_ci_cd_and_governance.md](21_ci_cd_and_governance.md)

---

## Change safety checklist

Before merging any `krab_core` change:

1. Run workspace checks and relevant target crate checks.
2. Validate service startup and readiness behavior still passes.
3. Confirm CI gates that rely on contract/DB policy still pass.
4. Update docs if behavior/contract changed.

## Related docs

- [`docs/security.md`](../security.md)
- [`docs/database.md`](../database.md)
- [21_ci_cd_and_governance.md](21_ci_cd_and_governance.md)
