# API Protocol Flexibility Plan (REST + GraphQL + RPC by Need)

## 1) Current Status (As-Is)

This section reflects the current implementation state before proposing changes.

### 1.1 Service-level protocol split is mostly static today

- `service_auth` exposes fixed REST endpoints mounted in router code:
  - `POST /api/v1/auth/login`
  - `POST /api/v1/auth/refresh`
  - `POST /api/v1/auth/revoke`
  - `GET /api/v1/auth/jwks`
  - `GET /api/v1/auth/status`
  - plus ops routes (`/health`, `/ready`, `/metrics`, `/metrics/prometheus`)
- `service_users` exposes GraphQL at `POST /api/v1/graphql` plus selected REST admin/status routes.
- Route registration is compile-time and explicit in each service `build_app(...)`.

### 1.2 Core crate supports protocol features

- `krab_core` has optional features for `rest`, `graphql`, and `grpc` (RPC transport foundation).
- Protocol capability is enabled at build/package level.
- Multi-service deployment guidance is not yet explicit enough.

### 1.3 CLI protocol choice is generation-time today

- `krab_cli` supports scaffolding service protocol choices.
- It does not yet define standard split-service scaffolding across REST/GraphQL/RPC deployables.

### 1.4 Governance/docs state

- API governance already documents REST versioning and GraphQL schema evolution.
- Public API reference documents the current split (auth mostly REST, users GraphQL-first).
- Missing piece: explicit split-topology routing, parity, and release governance across protocol-specific services.

### 1.5 Summary diagnosis

The system is protocol-capable but lacks a fully explicit multi-service protocol deployment contract.

---

## 2) Problem Statement

Krab must support protocol choice by need while allowing protocol-specific deployables without conflicts:

- REST for ecosystem-compatible command and integration flows.
- GraphQL for query flexibility.
- RPC for low-overhead action/query contracts.

When these are split into separate microservices, teams need guaranteed parity, stable endpoint names, shared policy enforcement, and clear gateway routing under one domain.

---

## 3) Goals and Non-Goals

### 3.1 Goals

1. Enable both single-service multi-adapter and split protocol-specific microservice topologies.
2. Preserve **Domain Core + Protocol Adapter** architecture as the implementation baseline.
3. Prevent endpoint collisions through explicit protocol namespaces and API Gateway routing.
4. Keep authn/authz, rate-limit, audit, and policy behavior consistent across all protocol services.
5. Enforce protocol parity testing and protocol-labeled observability.
6. Maintain backward-compatible rollout with explicit migration governance.

### 3.2 Non-Goals

1. No immediate removal of existing stable endpoints.
2. No unbounded dynamic protocol switching behavior in runtime request headers.
3. No weakening of auth lifecycle restrictions for convenience.

---

## 4) Target Architecture (To-Be)

### 4.1 Design principle: Domain Core + Protocol Adapters

Each service (or protocol-specific deployable) must keep:

1. **Domain/Application layer**: business logic, validation, authorization policy.
2. **Protocol adapters**:
   - REST handlers mapping HTTP payloads to domain commands/queries.
   - GraphQL resolvers mapping schema operations to the same domain commands/queries.
   - RPC handlers mapping methods/actions to the same domain commands/queries.

This remains the required architecture in both monolithic and split-service topologies.

### 4.2 Multi-service protocol topology

Krab supports the following deployment shapes:

1. **Single service, multi-adapter**
   - Example: one `users` process exposes REST + GraphQL + RPC adapters.

2. **Split protocol-specific services**
   - `users-rest` (REST)
   - `users-graphql` (GraphQL)
   - `users-rpc` (RPC)

For split topology, each service may run on separate ports or hosts, for example:

- `users-rest`: `users-rest.internal:8081`
- `users-graphql`: `users-graphql.internal:8082`
- `users-rpc`: `users-rpc.internal:8083`

An API Gateway must unify these under one external domain (for example `api.krab.local`) so clients consume stable public routes.

### 4.3 Endpoint structure clarity and conflict prevention

External route namespaces are fixed and non-conflicting:

- `/api/v1/auth/*` → auth service REST endpoints
- `/api/v1/users/*` → users REST endpoints
- `/api/v1/graphql` → GraphQL services
- `/api/v1/rpc` → RPC services

Rules:

1. REST business routes remain resource-scoped (`/api/v1/auth/*`, `/api/v1/users/*`).
2. GraphQL and RPC remain protocol-scoped singleton ingress paths.
3. No service may register conflicting public routes outside its namespace.
4. Ops endpoints remain service-local HTTP routes (`/health`, `/ready`, `/metrics`).

API Gateway upstream routing example:

- `POST /api/v1/users/*` → upstream `users-rest`
- `POST /api/v1/graphql` with `service=users` operation context → upstream `users-graphql`
- `POST /api/v1/rpc` with `service=users` method namespace → upstream `users-rpc`
- `POST /api/v1/auth/*` → upstream `service_auth`

### 4.4 Shared library usage and performance isolation

- `krab_core` is compiled into each protocol-specific service binary.
- Split services do **not** share runtime memory; they are process-isolated.
- Runtime performance of `users-rest` is not degraded by `users-graphql` or `users-rpc` except through shared infrastructure limits (CPU quotas, DB contention, network).
- Shared domain logic remains versioned and tested centrally to prevent behavioral drift.

### 4.5 Runtime protocol selection simplification

Primary model: **explicit protocol endpoints**.

- Clients select protocol by calling explicit route families.
- Runtime protocol override headers/queries are removed by default.
- Header-based protocol switching is permitted only for tightly controlled internal gateway experiments and must be disabled by default.

### 4.6 Security and auth invariants

All protocol services/adapters must enforce identical controls:

- Same authentication middleware semantics.
- Same tenant extraction and authorization policy engine.
- Same rate-limiting policy class.
- Same audit logging and trace identity propagation.

Policy restrictions apply per service and operation:

- Auth lifecycle operations (`login`, `refresh`, `revoke`) remain REST-only unless re-approved via formal security review.

### 4.7 Exposure modes per service

Per service deployment must support:

1. **single** exposure mode:
   - exactly one of REST, GraphQL, RPC enabled.
2. **multi** exposure mode:
   - any explicit subset enabled (REST+GraphQL, GraphQL+RPC, REST+RPC, REST+GraphQL+RPC).

Exposure mode is configured independently per service whether topology is `single_service` or `split_services`.

---

## 5) Governance Extensions Required

Add the following governance rules:

1. **Parity Matrix Required**
   - For equivalent operations, document REST ↔ GraphQL ↔ RPC mapping and known intentional gaps.

2. **Behavioral Equivalence Tests**
   - Equivalent operations across protocol services must match semantics, auth outcomes, and error taxonomy.

3. **Protocol-Specific Version Governance**
   - Each protocol-specific service (`users-rest`, `users-graphql`, `users-rpc`) can release independently.
   - Domain contract version must remain synchronized or explicitly compatibility-mapped.
   - Removing a protocol surface for an externally consumed operation is a breaking change.

4. **Deprecation by Protocol Surface**
   - Deprecations must identify impacted protocol surface and replacement timeline.

5. **Security Policy Inheritance**
   - Policy bundles are shared from one source of truth and consumed by all protocol services.

6. **Exposure Mode Change Policy**
   - Switching from multi to single mode for externally used operations requires major version governance and migration notice.

---

## 6) Service-by-Service Migration Plan

### 6.1 Auth service (`service_auth`)

- Keep auth lifecycle as REST source of truth.
- Optional non-sensitive GraphQL/RPC read surfaces require explicit security review.
- Maintain REST-only policy lock for token issuance lifecycle until formally revised.

### 6.2 Users services (`users-rest`, `users-graphql`, `users-rpc`)

Phased strategy:

- **Phase A:** stabilize domain core contracts.
- **Phase B:** expose REST and GraphQL parity set.
- **Phase C:** add RPC parity set for selected stable operations.
- **Phase D:** enforce split-service parity CI gates before release.

### 6.3 Frontend service (`service_frontend`)

- Use explicit downstream endpoints through gateway namespaces.
- Avoid runtime protocol negotiation logic unless required for controlled internal routing.
- Keep operational probes on HTTP REST ops endpoints.

---

## 7) Configuration Contract (Proposed)

### 7.1 Required protocol topology keys

- `KRAB_PROTOCOL_TOPOLOGY=single_service|split_services`
- `KRAB_PROTOCOL_ENABLED=rest|graphql|rpc` (service-local enabled set; delimiter policy documented by parser)
- `KRAB_PROTOCOL_EXPOSURE_MODE=single|multi`

### 7.2 Additional policy keys

- `KRAB_PROTOCOL_DEFAULT=rest|graphql|rpc`
- `KRAB_PROTOCOL_ALLOWED=rest,graphql,rpc`
- `KRAB_PROTOCOL_STRICT_PARITY=true|false`
- `KRAB_PROTOCOL_RESTRICTED_OPS_JSON={...}`
- `KRAB_PROTOCOL_SPLIT_TARGETS_JSON={...}`

### 7.3 Split-service configuration example

`users-rest`:

- `KRAB_PROTOCOL_TOPOLOGY=split_services`
- `KRAB_PROTOCOL_ENABLED=rest`
- `KRAB_PROTOCOL_EXPOSURE_MODE=single`

`users-graphql`:

- `KRAB_PROTOCOL_TOPOLOGY=split_services`
- `KRAB_PROTOCOL_ENABLED=graphql`
- `KRAB_PROTOCOL_EXPOSURE_MODE=single`

`users-rpc`:

- `KRAB_PROTOCOL_TOPOLOGY=split_services`
- `KRAB_PROTOCOL_ENABLED=rpc`
- `KRAB_PROTOCOL_EXPOSURE_MODE=single`

Auth restriction example:

- `KRAB_PROTOCOL_RESTRICTED_OPS_JSON={"auth.login":["rest"],"auth.refresh":["rest"],"auth.revoke":["rest"]}`

---

## 8) Testing Strategy

### 8.1 Contract tests

- REST schema/shape snapshots.
- GraphQL schema snapshots and drift checks.
- RPC contract compatibility checks.

### 8.2 Parity tests (mandatory)

- Same seed data and identity context across protocols.
- Execute equivalent REST/GraphQL/RPC operations.
- Assert semantic equality of successful responses and authorization outcomes.
- Assert stable protocol-specific error mapping to shared domain error classes.

### 8.3 Topology tests

- Route conflict checks for `/api/v1/auth/*`, `/api/v1/users/*`, `/api/v1/graphql`, `/api/v1/rpc`.
- API Gateway upstream routing validation for split services.
- Service isolation checks (one protocol service degraded does not crash peers).

### 8.4 Policy tests

- Restricted operation rejection in non-approved protocols.
- Exposure mode tests for single and multi configurations.

---

## 9) Observability and Operations

Protocol-aware telemetry is required in all topologies:

- `http_requests_total{service,operation,protocol,status_class}`
- `request_latency_ms{service,operation,protocol}`
- `auth_failures_total{service,operation,protocol,reason}`
- trace attributes: `krab.protocol`, `krab.operation`, `krab.service_instance`
- logs: include protocol, service name, operation, policy decision

Dashboards and alerts must support:

1. Per-protocol SLO comparisons.
2. Cross-service parity anomaly detection.
3. Split-service release health view by protocol.

---

## 10) Rollout Plan

### Phase 0 — Governance and architecture baseline

- Ratify this amended plan.
- Update API governance and release policy with split-service protocol rules.

### Phase 1 — Core and config primitives

- Implement topology and exposure-mode parsing.
- Implement shared policy bundle loading across protocol services.
- Implement protocol-labeled telemetry defaults.

### Phase 2 — Users split-service pilot

- Deploy `users-rest`, `users-graphql`, `users-rpc` behind one gateway domain.
- Validate explicit endpoint namespace model.
- Enforce parity and route conflict tests in CI.

### Phase 3 — CLI developer experience

- Extend CLI scaffolding to generate split-service layout when requested.

Example scaffold:

```text
users-rest/
    adapters/rest/
    domain/
users-graphql/
    adapters/graphql/
    domain/
users-rpc/
    adapters/rpc/
    domain/
```

- Scaffold build/deploy manifests independently for each protocol service.
- Reuse shared domain crate and `krab_core` dependency in every generated service.

### Phase 4 — Auth policy hardening

- Keep auth lifecycle REST-only.
- Allow extension only after explicit security and parity review.

---

## 11) Risks and Mitigations

1. **Risk:** Endpoint collisions in split topology.
   - **Mitigation:** fixed public namespaces + gateway route validation tests.

2. **Risk:** Cross-protocol semantic drift.
   - **Mitigation:** shared domain core + mandatory parity CI gates.

3. **Risk:** Security inconsistency.
   - **Mitigation:** centralized policy bundle, shared middleware semantics, and release checklist gates.

4. **Risk:** Operational complexity.
   - **Mitigation:** protocol-labeled telemetry, phased rollout, and per-service ownership boundaries.

5. **Risk:** Shared-library performance concerns.
   - **Mitigation:** compile `krab_core` into each service binary; process-level isolation avoids cross-service runtime contention.

---

## 12) Acceptance Criteria

This plan is considered implemented when:

1. At least one domain (`users`) runs as split protocol-specific services (`users-rest`, `users-graphql`, `users-rpc`) behind one gateway domain.
2. Public endpoint namespaces are non-conflicting and enforced.
3. `krab_core` is compiled and used in every protocol service.
4. Parity tests across REST/GraphQL/RPC are part of CI gates.
5. Protocol-labeled metrics, traces, and logs are available in operations dashboards.
6. Release governance supports protocol-specific microservice versioning and compatibility policy.
7. Auth lifecycle restriction policy remains enforced as REST-only unless formally changed.
8. Runtime protocol switching headers are disabled by default in favor of explicit endpoints.

---

## 13) Immediate Next Actions

1. Update governance docs with split-service protocol release/version rules.
2. Add gateway routing policy examples for `/api/v1/auth/*`, `/api/v1/users/*`, `/api/v1/graphql`, `/api/v1/rpc`.
3. Define and document service-level protocol config parser behavior for `KRAB_PROTOCOL_TOPOLOGY`, `KRAB_PROTOCOL_ENABLED`, and exposure mode.
4. Extend CLI scaffolding for split-service generation with shared domain crate conventions.
5. Add parity + conflict + policy test templates to CI and service starter repositories.

