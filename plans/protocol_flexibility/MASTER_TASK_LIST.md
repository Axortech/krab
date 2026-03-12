# Protocol Flexibility — Master Task List

> Track all implementation tasks across phases A–E.
> Mark: `[ ]` = pending, `[/]` = in progress, `[x]` = done.

## Execution Rule

- Do not stop until all tasks in this file are completed.
- Verify implementation after each edit.
- Ask the user for required decisions or missing input before proceeding when needed.
- Update relevant documentation after implementation changes.

---

## Phase A — Core Primitives ([02_phase_a_core_primitives.md](./02_phase_a_core_primitives.md))

### A-1  New module `krab_core/src/protocol.rs`
- [x] Define `ProtocolKind` enum with `parse()` and `as_str()` — [§A-1.1](./02_phase_a_core_primitives.md#a-11--types)
- [x] Define `ExposureMode`, `DeploymentTopology` enums — [§A-1.1](./02_phase_a_core_primitives.md#a-11--types)
- [x] Define `ServiceCapabilities` struct — [§A-1.1](./02_phase_a_core_primitives.md#a-11--types)
- [x] Define `ProtocolPolicy` struct with operation restrictions (ex: auth lifecycle REST-only) — [§A-1.1](./02_phase_a_core_primitives.md#a-11--types)
- [x] Define `ProtocolConfig` struct — [§A-1.1](./02_phase_a_core_primitives.md#a-11--types)
- [x] Implement `ProtocolConfig::from_env()` — read all `KRAB_PROTOCOL_*` env vars — [§A-1.2](./02_phase_a_core_primitives.md#a-12--protocolconfig-construction-from-env)
- [x] Implement `ProtocolConfig::validate()` with split-topology + exposure invariants — [§A-1.2](./02_phase_a_core_primitives.md#a-12--protocolconfig-construction-from-env)
- [x] Implement parser for `KRAB_PROTOCOL_TOPOLOGY=single_service|split_services`
- [x] Implement parser for service-local `KRAB_PROTOCOL_ENABLED`
- [x] Implement hard default to explicit protocol endpoints (no runtime header switching)
- [x] Implement `capabilities_handler()` for `GET /api/capabilities` — [§A-1.4](./02_phase_a_core_primitives.md#a-14--capability-endpoint-contract-helper)

### A-2  Extend `krab_core/src/http.rs`
- [x] Add protocol attribution helper based on explicit endpoint namespace (`/api/v1/users/*`, `/api/v1/graphql`, `/api/v1/rpc`)
- [x] Add middleware to attach resolved protocol from route family into request extensions
- [x] Extend `tracing_middleware` with `krab.protocol` span attribute — [§A-2.3](./02_phase_a_core_primitives.md#a-23--add-protocol-label-to-tracingmetrics)
- [x] Extend `metrics_middleware` / Prometheus with `protocol` label — [§A-2.3](./02_phase_a_core_primitives.md#a-23--add-protocol-label-to-tracingmetrics)
- [x] Add guard that runtime protocol-switch headers are disabled by default

### A-3  Extend `krab_core/src/service.rs`
- [x] Add `protocol: Option<ProtocolConfig>` to `ServiceConfig` — [§A-3](./02_phase_a_core_primitives.md#a-3--extend-krab_coresrcservicers)

### A-4  Wire into `krab_core/src/lib.rs`
- [x] Add `pub mod protocol;` (unconditional) — [§A-6](./02_phase_a_core_primitives.md#a-6--wire-changes-summary)
- [x] Add `#[cfg(all(feature = "rest", test))] mod protocol_tests;` — [§A-6](./02_phase_a_core_primitives.md#a-6--wire-changes-summary)

### A-5  Unit tests `krab_core/src/protocol_tests.rs`
- [x] `test_topology_parse_single_service` — [§A-4](./02_phase_a_core_primitives.md#a-4--new-file-krab_coresrcprotocol_testsrs)
- [x] `test_topology_parse_split_services`
- [x] `test_config_validation_default_in_enabled`
- [x] `test_config_validation_single_mode_one_protocol`
- [x] `test_parse_protocol_kind_case_insensitive`
- [x] `test_parse_protocol_kind_invalid`
- [x] `test_route_family_resolves_protocol_rest`
- [x] `test_route_family_resolves_protocol_graphql`
- [x] `test_route_family_resolves_protocol_rpc`
- [x] `test_runtime_switch_header_rejected_by_default`

### A-6  Acceptance
- [x] `cargo test -p krab_core --features rest -- protocol` passes — [§A-8](./02_phase_a_core_primitives.md#a-8--acceptance-gates-for-phase-a)
- [x] `cargo build -p krab_core` (no features) compiles
- [x] `cargo build -p krab_core --features rest,graphql,grpc` compiles
- [x] Existing `service_auth` + `service_users` tests unchanged and passing

---

## Phase B — Users Service Pilot ([03_phase_b_users_pilot.md](./03_phase_b_users_pilot.md))

### B-1  Domain layer extraction (shared across protocol services)
- [x] Create `service_users/src/domain/mod.rs` — [§B-1.1](./03_phase_b_users_pilot.md#b-11--new-directory-layout)
- [x] Create `domain/models.rs` (`UserModel`, `UserProfile`) — [§B-1.2](./03_phase_b_users_pilot.md#b-12--domain-layer-domain)
- [x] Create `domain/errors.rs` (`DomainError`) — [§B-1.2](./03_phase_b_users_pilot.md#b-12--domain-layer-domain)
- [x] Create `domain/service.rs` (`UserDomainService` trait + `UserDomainServiceImpl`) — [§B-1.2](./03_phase_b_users_pilot.md#b-12--domain-layer-domain)
- [x] Move existing `PostgresUserRepository` / `SqliteUserRepository` into `db/` sub-module — [§B-1.1](./03_phase_b_users_pilot.md#b-11--new-directory-layout)
- [x] Ensure domain module is consumable from `users-rest`, `users-graphql`, and `users-rpc` binaries

### B-2  Protocol adapters
- [x] Create `adapters/mod.rs` — [§B-1.1](./03_phase_b_users_pilot.md#b-11--new-directory-layout)
- [x] Implement `adapters/rest.rs` — REST handlers calling domain service — [§B-1.3](./03_phase_b_users_pilot.md#b-13--rest-adapter-adaptersrestrs)
- [x] Implement `adapters/graphql.rs` — migrate `UserQuery` to use domain service — [§B-1.4](./03_phase_b_users_pilot.md#b-14--graphql-adapter-adaptersgraphqlrs)
- [x] Implement `adapters/rpc.rs` — JSON-RPC dispatcher calling domain service — [§B-1.5](./03_phase_b_users_pilot.md#b-15--rpc-adapter-adaptersrpcrs)

### B-3  Split-service topology pilot (`users-rest`, `users-graphql`, `users-rpc`)
- [x] Create protocol-specific service targets (or binaries) for `users-rest`, `users-graphql`, `users-rpc`
- [x] Compile `krab_core` into each protocol-specific service
- [x] Configure independent bind addresses/ports per protocol service
- [x] Keep non-conflicting public namespaces:
  - [x] `/api/v1/users/*` for REST
  - [x] `/api/v1/graphql` for GraphQL
  - [x] `/api/v1/rpc` for RPC
- [x] Add API Gateway upstream mapping example and validation for all three users services

### B-4  Router assembly
- [x] Refactor `build_app()` to read `ProtocolConfig` and mount only enabled adapters — [§B-2](./03_phase_b_users_pilot.md#b-2--mainrs-router-assembly-protocol-mode-aware)
- [x] Add `GET /api/v1/capabilities` endpoint — [§B-3](./03_phase_b_users_pilot.md#b-3--capabilities-endpoint-response)
- [x] Create `capabilities.rs` to construct `ServiceCapabilities` — [§B-1.1](./03_phase_b_users_pilot.md#b-11--new-directory-layout)

### B-5  Parity tests
- [x] `parity_get_me_rest_equals_graphql` — [§B-4](./03_phase_b_users_pilot.md#b-4--parity-tests)
- [x] `parity_get_me_rpc_equals_graphql`
- [x] `parity_tenant_required_all_protocols`
- [x] `parity_auth_required_all_protocols`

### B-6  Contract preservation
- [x] GraphQL schema snapshot test still passes — [§B-7](./03_phase_b_users_pilot.md#b-7--acceptance-gates-for-phase-b)
- [x] Existing `service_users` tests pass with zero regressions

### B-7  Acceptance
- [x] `service_users` starts in `single` mode with each protocol — [§B-7](./03_phase_b_users_pilot.md#b-7--acceptance-gates-for-phase-b)
- [x] `service_users` starts in `multi` mode with `rest,graphql,rpc`
- [x] Split deployment starts as independent services (`users-rest`, `users-graphql`, `users-rpc`) behind gateway
- [x] Parity tests pass
- [x] Admin RBAC consistent across modes

---

## Phase C — Frontend Integration ([04_phase_c_frontend_integration.md](./04_phase_c_frontend_integration.md))

### C-1  Protocol-aware client
- [x] Create `service_frontend/src/protocol_client.rs` — [§C-1](./04_phase_c_frontend_integration.md#c-1--new-module-service_frontendsrcprotocol_clientrs)
- [x] Implement `ProtocolAwareClient` struct — [§C-1.1](./04_phase_c_frontend_integration.md#c-11--capability-discovery-client)
- [x] Implement `capabilities()` with TTL cache — [§C-1.1](./04_phase_c_frontend_integration.md#c-11--capability-discovery-client)
- [x] Implement explicit endpoint routing helper (no default runtime header switching)
- [x] Implement `call()` with REST/GraphQL/RPC transport methods against explicit endpoints — [§C-1.1](./04_phase_c_frontend_integration.md#c-11--capability-discovery-client)
- [x] Implement `call_with_fallback()` — [§C-1.2](./04_phase_c_frontend_integration.md#c-12--fallback-behavior)

### C-2  Frontend integration
- [x] Extend `AppState` with `protocol_client` — [§C-2.1](./04_phase_c_frontend_integration.md#c-21--appstate-extension)
- [x] Bootstrap `ProtocolAwareClient` at startup — [§C-2.2](./04_phase_c_frontend_integration.md#c-22--bootstrap-protocol-client-at-startup)
- [x] Keep ops probes on direct REST — [§C-2.3](./04_phase_c_frontend_integration.md#c-23--ops-probes-stay-on-rest)

### C-3  Split-topology support
- [x] Parse `KRAB_PROTOCOL_SPLIT_TARGETS_JSON` — [§C-3](./04_phase_c_frontend_integration.md#c-3--split-topology-support)
- [x] Route calls to per-protocol URLs when split topology is active
- [x] Route through API Gateway single domain when external mode is enabled

### C-4  Testing
- [x] `test_capability_discovery_and_caching` — [§C-4](./04_phase_c_frontend_integration.md#c-4--testing)
- [x] `test_call_rest_via_explicit_namespace`
- [x] `test_call_graphql_via_explicit_namespace`
- [x] `test_call_rpc_via_explicit_namespace`
- [x] `test_fallback_on_primary_failure`
- [x] `test_ops_probes_are_always_rest`

### C-5  Acceptance
- [x] Frontend discovers capabilities from running `service_users` — [§C-5](./04_phase_c_frontend_integration.md#c-5--acceptance-gates-for-phase-c)
- [x] Downstream calls use resolved protocol
- [x] Ops probes work if capability endpoint is down
- [x] Fallback works when primary adapter is down

---

## Phase D — Auth Guardrails ([05_phase_d_auth_guardrails.md](./05_phase_d_auth_guardrails.md))

### D-1  Capability endpoint
- [x] Add `auth_capabilities_handler()` — [§D-1](./05_phase_d_auth_guardrails.md#d-1--capability-endpoint-for-auth)
- [x] Mount at `GET /api/v1/auth/capabilities` — [§D-1](./05_phase_d_auth_guardrails.md#d-1--capability-endpoint-for-auth)
- [x] Add `/api/v1/auth/capabilities` to auth middleware open paths — [§D-1](./05_phase_d_auth_guardrails.md#d-1--capability-endpoint-for-auth)

### D-2  Security lockdown
- [x] Verify hard REST-only restrictions for login/refresh/revoke/jwks — [§D-2](./05_phase_d_auth_guardrails.md#d-2--hard-restrictions-security-sensitive-operations)
- [x] Verify restrictions hold identically under split-topology deployment
- [x] Document conditions for future non-REST surfaces — [§D-3](./05_phase_d_auth_guardrails.md#d-3--optional-limited-rpcgraphql-read-operations-future-gated-by-security-review)

### D-3  Testing
- [x] `test_capabilities_endpoint_returns_rest_only` — [§D-5](./05_phase_d_auth_guardrails.md#d-5--testing)
- [x] `test_existing_lifecycle_unaffected`
- [x] All existing `service_auth` tests pass

### D-4  Acceptance
- [x] `GET /api/v1/auth/capabilities` returns correct response — [§D-6](./05_phase_d_auth_guardrails.md#d-6--acceptance-gates-for-phase-d)
- [x] Auth lifecycle unchanged

---

## Phase E — Governance, CI, CLI ([06_phase_e_governance_ci_cli.md](./06_phase_e_governance_ci_cli.md))

### E-1  Governance docs
- [x] Add §5 "Protocol Parity and Exposure Mode Policy" to `api_governance.md` — [§E-1](./06_phase_e_governance_ci_cli.md#e-1--governance-doc-updates-plansapi_governancemd)
  - [x] Parity matrix required rule
  - [x] Behavioral equivalence test rule
  - [x] Protocol change classification table
  - [x] Deprecation by protocol surface rule
  - [x] Observability labels rule
  - [x] Exposure mode change policy

### E-2  CLI updates
- [x] Add `--exposure-mode` arg to `gen service` — [§E-2.1](./06_phase_e_governance_ci_cli.md#e-21--extended-gen-service-command)
- [x] Add `--protocols` arg (CSV) — [§E-2.1](./06_phase_e_governance_ci_cli.md#e-21--extended-gen-service-command)
- [x] Add `--topology` arg — [§E-2.1](./06_phase_e_governance_ci_cli.md#e-21--extended-gen-service-command)
- [x] Implement `multi` mode template generation (domain + adapters dirs) — [§E-2.2](./06_phase_e_governance_ci_cli.md#e-22--template-generation-behavior)
- [x] Implement `split_services` topology scaffolding — [§E-2.2](./06_phase_e_governance_ci_cli.md#e-22--template-generation-behavior)
- [x] Scaffold protocol-specific directories:
  - [x] `users-rest/adapters/rest + domain`
  - [x] `users-graphql/adapters/graphql + domain`
  - [x] `users-rpc/adapters/rpc + domain`
- [x] Ensure generated split services all include `krab_core` and shared domain dependency guidance

### E-3  CI pipeline
- [x] Add protocol matrix job (mode × protocol) — [§E-3.1](./06_phase_e_governance_ci_cli.md#e-31--protocol-matrix-tests)
- [x] Add parity suite as release gate — [§E-3.2](./06_phase_e_governance_ci_cli.md#e-32--parity-suite-as-release-gate)
- [x] Add `krab contract protocol-check` CLI command — [§E-3.3](./06_phase_e_governance_ci_cli.md#e-33--new-cli-command-krab-contract-protocol-check)
- [x] Add split-topology gateway route conflict check gate
- [x] Add protocol-specific microservice version compatibility gate

### E-4  Configuration
- [x] Add `KRAB_PROTOCOL_*` vars to `.env.example` — [§E-4](./06_phase_e_governance_ci_cli.md#e-4--environment-template-updates-envexample)
- [x] Document `KRAB_PROTOCOL_TOPOLOGY=split_services` usage in deployment docs
- [x] Document per-service `KRAB_PROTOCOL_ENABLED=rest|graphql|rpc`
- [x] Document per-service `KRAB_PROTOCOL_EXPOSURE_MODE=single|multi`

### E-5  Observability
- [x] Extend Prometheus metrics with `protocol` dimension — [§E-5.1](./06_phase_e_governance_ci_cli.md#e-51--prometheus-metrics-additions)
- [x] Extend tracing with `krab.protocol`, `krab.operation`, `krab.selection_source` — [§E-5.2](./06_phase_e_governance_ci_cli.md#e-52--tracing-attributes)
- [x] Add protocol-segmented dashboard views — [§E-5.3](./06_phase_e_governance_ci_cli.md#e-53--dashboard-additions-grafana--monitoring-config)
- [x] Add protocol-specific service labels (`users-rest`, `users-graphql`, `users-rpc`) to dashboards and alerts

### E-6  Documentation
- [x] Create `docs/protocol_flexibility.md` — [§E-6.1](./06_phase_e_governance_ci_cli.md#e-61--docsprotocol_flexibilitymd-new-public-facing-doc)
- [x] Update `docs/API.md` with capability endpoints — [§E-6.2](./06_phase_e_governance_ci_cli.md#e-62--api-reference-updates)
- [x] Add CHANGELOG.md entry — [§E-6.3](./06_phase_e_governance_ci_cli.md#e-63--changelogmd-entry)

### E-7  Acceptance
- [x] Governance doc includes protocol parity rules — [§E-7](./06_phase_e_governance_ci_cli.md#e-7--acceptance-gates-for-phase-e)
- [x] CLI `gen service --exposure-mode multi` generates correct structure
- [x] CI protocol matrix passes
- [x] `.env.example` includes `KRAB_PROTOCOL_*` vars

---

## Rollback & Safety ([07_rollback_and_safety.md](./07_rollback_and_safety.md))

- [x] Document per-phase rollback procedures — [§2](./07_rollback_and_safety.md#2--per-phase-rollback-procedures)
- [x] Implement emergency disable switches — [§3](./07_rollback_and_safety.md#3--emergency-disable-switches)
- [x] Configure monitoring thresholds — [§4](./07_rollback_and_safety.md#4--monitoring-for-rollback-triggers)

---

## Summary

| Phase | Tasks | Status |
|---|---|---|
| **A** Core Primitives | 28 | ✅ Completed |
| **B** Users Pilot | 24 | ✅ Completed |
| **C** Frontend Integration | 17 | ✅ Completed |
| **D** Auth Guardrails | 9 | ✅ Completed |
| **E** Governance/CI/CLI | 27 | ✅ Completed |
| **Rollback** | 3 | ✅ Completed |
| **Total** | **108** | |
