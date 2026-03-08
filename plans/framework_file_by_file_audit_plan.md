# Krab Framework — File-by-File Audit Plan

Source alignment:

- [README.md](../README.md)
- [plans/03_roadmap.md](./03_roadmap.md)
- [plans/08_production_readiness.md](./08_production_readiness.md)

## 1) Audit objective

Execute a complete, file-by-file technical audit of the Krab framework to verify:

1. architecture integrity,
2. security and operational hardening controls,
3. API and data contract stability,
4. SSR/islands correctness,
5. build/test/release reproducibility,
6. documentation-to-implementation consistency.

## 2) Scope and inventory rules

### In scope (full review)

- Workspace manifests and policy docs (`Cargo.toml`, release/security docs, CI workflows).
- All first-party crate sources under:
  - `krab_core/`
  - `krab_macros/`
  - `krab_client/`
  - `krab_server/`
  - `krab_cli/`
  - `krab_orchestrator/`
  - `service_auth/`
  - `service_users/`
  - `service_frontend/`
- `docs/`, `plans/`, and `scripts/` as governance/ops artifacts.

### Out of scope (unless evidence required)

- Build artifacts (`target/`) except when referenced to validate reproducibility or prior failures.
- Generated static HTML snapshots under `service_frontend/public/__ssg/` (spot-check only unless mismatch is detected).

## 3) Audit methodology (file-by-file protocol)

For every audited file, record one audit row with:

- file path,
- classification (runtime, build, policy, test, docs, script),
- risk tier (critical/high/medium/low),
- checks applied,
- findings,
- evidence pointer (line references, command output, or test artifact),
- remediation recommendation,
- owner,
- status (`pass`, `needs-fix`, `accepted-risk`, `n/a`).

### Severity model

- **Critical**: security break, data corruption/loss path, auth bypass, or release blocker.
- **High**: contract breakage risk, reliability defect, migration safety gap, or major observability blind spot.
- **Medium**: correctness/performance concerns with bounded blast radius.
- **Low**: maintainability/documentation drift without immediate runtime risk.

## 4) Audit phases

### Phase A — Baseline and control-plane verification

Goal: confirm workspace architecture and quality gates before code-level deep dive.

Checklist:

1. Validate workspace membership and dependency pinning consistency.
2. Verify CI gates for formatting, linting, tests, docs, dependency security, API contracts, DB lifecycle, and NFT.
3. Cross-check policy docs against enforced behavior in CI/workflows.

Deliverable: baseline control-plane report with any gate-policy drift.

### Phase B — Core framework internals (`krab_core`, `krab_server`, `krab_macros`, `krab_client`)

Goal: prove framework-level primitives are safe, deterministic, and aligned with architecture promises.

Checklist:

1. HTTP middleware chain correctness (auth, CORS, request-id, trace, limits).
2. Error envelope consistency and propagation behavior.
3. Resilience primitives (timeouts/retries/fallback paths) and failure semantics.
4. SSR stream/render and style scoping correctness.
5. Islands/hydration macro and client runtime contract compatibility.
6. Signal/thread-safety assumptions validated against docs.

Deliverable: framework internals audit matrix + prioritized defects.

### Phase C — Service implementations (`service_auth`, `service_users`, `service_frontend`)

Goal: verify each service correctly consumes framework primitives and enforces production constraints.

Checklist:

1. Endpoint surface parity with documented contracts.
2. AuthN/AuthZ policy enforcement (issuer/audience/role/scope/rate-limits/revocation).
3. Input validation and error behavior under negative paths.
4. DB behavior: migrations, transaction boundaries, rollback expectations.
5. Frontend SSR fallback behavior under API degradation.
6. Metrics/health/readiness consistency and trace correlation.

Deliverable: per-service findings list with remediation owners.

### Phase D — Tooling and orchestration (`krab_cli`, `krab_orchestrator`, scripts)

Goal: ensure developer and operational tooling is safe, deterministic, and non-destructive by default.

Checklist:

1. Bootstrap/scaffolding correctness and template hygiene.
2. Orchestrator startup/shutdown semantics and failure handling.
3. Script safety (idempotency, argument validation, error handling).
4. Local-to-CI behavior parity.

Deliverable: tooling reliability report + quick wins.

### Phase E — Governance, docs, and release readiness closure

Goal: close the loop between implementation and stated standards.

Checklist:

1. `README`, docs, and plans reflect current code behavior.
2. Release/changelog policy consistency.
3. On-call/SLO/rollback runbooks map to real system behavior.
4. Residual risk register updated with owner and review cadence.

Deliverable: final audit dossier + signoff recommendation.

## 5) File review order (execution sequence)

1. Root governance files (`README.md`, workspace `Cargo.toml`, release/security docs, workflows).
2. `krab_core` (highest shared blast radius).
3. `krab_server` + `krab_macros` + `krab_client`.
4. Service crates (`service_auth`, `service_users`, `service_frontend`).
5. `krab_orchestrator` + `krab_cli` + `scripts`.
6. `docs/` and `plans/` for consistency reconciliation.

## 6) Standard checks applied to every code file

1. **Correctness**: control flow, edge cases, error handling, panic boundaries.
2. **Security**: secret handling, auth checks, injection risks, unsafe usage, trust boundaries.
3. **Reliability**: retry/timeout/circuit behavior, shutdown handling, backpressure.
4. **Observability**: logs/metrics/traces cardinality and diagnostic quality.
5. **Performance**: allocations/cloning/hot-path inefficiencies.
6. **Maintainability**: cohesion, naming, testability, dead code, TODO debt.
7. **Contract compliance**: API/schema/config compatibility and backward-compatibility posture.

## 7) Evidence and artifact model

Audit outputs will be maintained as:

- File-level checklist table (append-only during execution).
- Finding log with severity and remediation owner.
- Verification appendix (commands run, test outcomes, lint/doc/security gates).
- Drift list for doc-vs-code mismatches.

## 8) Exit criteria for complete audit

Audit is complete when:

1. every in-scope file has an audit row,
2. every high/critical issue has an owner + remediation plan,
3. unresolved risks are explicitly accepted with rationale,
4. documentation and governance drifts are recorded,
5. final summary includes go/no-go recommendation and follow-up sequence.

## 9) Expected deliverables

1. Master file-by-file audit ledger.
2. Prioritized findings report (critical/high/medium/low).
3. Remediation roadmap grouped by crate/service.
4. Governance consistency report (docs/policies vs implementation).
5. Executive summary with release-readiness conclusion.
