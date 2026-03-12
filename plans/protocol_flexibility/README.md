# Protocol Flexibility Implementation Suite (REST + GraphQL + RPC)

This folder contains the detailed implementation expansion for protocol-flexibility work.

## Scope

- Per-service user choice of API style: REST-only, GraphQL-only, RPC-only, or multi-protocol.
- Support both topology styles:
  - single service exposing multiple adapters
  - split protocol-specific microservices
- Keep security controls and operational behavior consistent.

## Documents

### Baseline

1. [`00_current_state_acknowledgement.md`](./00_current_state_acknowledgement.md)
   — Concrete current baseline from the codebase. Notes what exists and what is missing.

2. [`01_detailed_implementation_blueprint.md`](./01_detailed_implementation_blueprint.md)
   — High-level end-to-end design, subsystem-by-subsystem execution plan.

### Expanded Phase Plans (implementation-ready)

3. [`02_phase_a_core_primitives.md`](./02_phase_a_core_primitives.md)
   — **Phase A**: `krab_core/src/protocol.rs` types, resolver, config, HTTP header parsing, capability endpoint helper. Unit test plan. File-level touch map.

4. [`03_phase_b_users_pilot.md`](./03_phase_b_users_pilot.md)
   — **Phase B**: `service_users` refactor into domain + adapter architecture. REST/GraphQL/RPC adapter code. Parity test suite. Mode-aware router assembly.

5. [`04_phase_c_frontend_integration.md`](./04_phase_c_frontend_integration.md)
   — **Phase C**: `service_frontend` protocol-aware downstream client. Capability discovery with caching. Protocol resolution. Fallback behavior. Split-topology support.

6. [`05_phase_d_auth_guardrails.md`](./05_phase_d_auth_guardrails.md)
   — **Phase D**: `service_auth` capability endpoint. Hard restriction of security-sensitive operations to REST. Conditions for future non-REST read surface.

7. [`06_phase_e_governance_ci_cli.md`](./06_phase_e_governance_ci_cli.md)
   — **Phase E**: Governance doc updates, CI protocol matrix, parity release gates, CLI multi-protocol scaffolding, env template, observability, documentation.

### Safety & Operations

8. [`07_rollback_and_safety.md`](./07_rollback_and_safety.md)
   — Per-phase rollback procedures, emergency disable switches, monitoring thresholds.

### Tracking

9. [`MASTER_TASK_LIST.md`](./MASTER_TASK_LIST.md)
   — **92 tasks** across all phases with checkboxes and cross-references to each plan section.

## Relationship to previous plan

The high-level plan in [`../api_protocol_flexibility_plan.md`](../api_protocol_flexibility_plan.md) remains the policy/vision layer.

This folder is the execution layer.
