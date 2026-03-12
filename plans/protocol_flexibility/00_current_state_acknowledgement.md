# Current State Acknowledgement (Implementation Baseline)

This document captures the exact baseline we are planning from, so implementation work is grounded in what already exists.

## 1. Confirmed baseline

### 1.1 Protocol support in core crates

- `krab_core` already exposes optional feature flags for:
  - `rest`
  - `graphql`
  - `grpc` (to be used as RPC transport foundation)
- This means protocol capabilities are currently decided at build/package time.

### 1.2 Service-level API reality today

- `service_auth` is REST-first and explicitly mounts auth lifecycle endpoints in router code.
- `service_users` is GraphQL-first (`/api/v1/graphql`) with selected REST endpoints.
- `service_frontend` mostly consumes downstream HTTP routes for readiness/status and also exposes `/rpc/*` utility endpoints, but there is no general protocol negotiation contract.

### 1.3 CLI scaffolding behavior today

- `krab_cli` supports generation-time service type choices (`rest`, `graphql`, `grpc`).
- The generator does not yet scaffold multi-adapter services or protocol negotiation primitives.

### 1.4 Runtime selection gap

Current system behavior is **service-static**:

- protocol is chosen by service implementation and build features,
- not dynamically resolved per service policy + user/client preference.

### 1.5 Documentation changes already made

- High-level strategy exists in [`../api_protocol_flexibility_plan.md`](../api_protocol_flexibility_plan.md).
- Scope is now REST + GraphQL + RPC (SOAP removed).

---

## 2. Constraints we must preserve

1. Existing stable routes must continue to work during migration.
2. Auth-sensitive operations must remain policy-restricted (REST-only where mandated).
3. Ops routes (`/health`, `/ready`, `/metrics`) stay operationally consistent.
4. Protocol choice must not create authorization drift.

---

## 3. Required deltas from baseline

To move from current baseline to target state, we need:

1. runtime protocol capability declaration per service,
2. protocol selection policy resolver,
3. service adapter patterns (REST + GraphQL + RPC) over a shared domain layer,
4. CLI support for single or multi-protocol scaffolding,
5. parity tests + protocol-tagged observability,
6. deployment support for both single-service multi-adapter and split protocol-specific services.

---

## 4. Definition of done for baseline-to-target transition

Transition is complete when at least one service (users) can run in:

- REST-only,
- GraphQL-only,
- RPC-only,
- or multi-protocol mode,

without changing domain logic, while passing parity/security/observability gates.

