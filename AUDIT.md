# Project Audit Report

**Project:** Krab  
**Audit Type:** Standard Engineering & Operational Audit  
**Audit Date (UTC):** 2026-03-13  
**Audited Scope:** Workspace crates, CI/CD, runtime security controls, observability, deployment artifacts

---

## 1) Executive Summary

Krab shows strong architecture and governance intent (multi-workflow CI, startup configuration validation, security middleware, and container hardening defaults).  
Local quality gates are now green (fmt/clippy/tests), and targeted hardening checks completed.  
However, release readiness remains blocked by one critical gap:

- Dependency audit tooling is still unavailable locally (`cargo-deny` not installed), so the advisory/license gate is unverified.

**Current Release Readiness:** **Not ready for production release** until dependency audit tooling is installed and the gate passes.

---

## 2) Audit Scope & Methodology

### In-scope
- Rust workspace structure and dependency policy
- Build/test/lint/doc quality gates
- CI/CD workflows and policy gates
- Runtime security and configuration enforcement
- Monitoring/alerting compatibility and operational readiness

### Method
- Static review of source and configuration
- Local command execution for quality signals
- Cross-check of runtime metrics contract vs alert/dashboard queries
- Risk grading by severity (Critical/High/Medium/Low)

---

## 3) Quality Gate Results

| Gate | Status | Result |
|---|---|---|
| `cargo fmt --all --check` | ✅ Passed | Clean formatting |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ Passed | No clippy violations |
| `cargo test --workspace` | ✅ Passed | All tests green |
| `cargo doc --workspace --no-deps` | ❌ Not rechecked | Not run in this audit pass |
| `cargo deny --all-features check advisories licenses bans` | ❌ Not runnable locally | `cargo-deny` not installed in audit environment |

---
## 4) Findings Register

### Critical

#### F-01: Monitoring contract mismatch (alerts/dashboard vs emitted metrics)
- **Severity:** Critical
- **Status:** âœ… Remediated (2026-03-12)
- **Resolution Summary:**
  - Runtime now emits canonical series used by alerting/dashboard queries.
  - Added compatibility aliases (`krab_response_5xx_total`, `krab_request_duration_ms_*`) to avoid immediate observability breakage during migration.
  - Added per-protocol error metric emission (`krab_http_responses_by_protocol_total{class,protocol}`) for protocol parity alerting.

### High

#### F-02: Formatting gate failure
- **Severity:** High
- **Status:** ✅ Remediated (2026-03-13)
- **Resolution Summary:** Formatting pass run; gate now green.

#### F-03: Clippy hard-fail in frontend build script
- **Severity:** High
- **Status:** ✅ Remediated (2026-03-13)
- **Resolution Summary:** Replaced collapsible `str::replace` calls; clippy now passes.

#### F-04: Workspace test failure in server route behavior
- **Severity:** High
- **Status:** ✅ Remediated (2026-03-13)
- **Resolution Summary:** Router now treats non-root trailing slash as non-match; test gate green.

#### F-08: Dependency audit gate cannot be executed locally
- **Severity:** High
- **Impact:** Advisory/license policy gate is unverified; release readiness blocked.
- **Recommendation:** Install `cargo-deny` and run the dependency audit gate.

### Medium

#### F-05: DB driver documentation/implementation inconsistency
- **Severity:** Medium
- **Impact:** Operator confusion and misconfiguration risk.
- **Details:** Environment template mentions MySQL as available/planned; active service implementation supports `postgres|sqlite`.
- **Recommendation:** Harmonize docs and env templates with actual supported drivers (or implement feature-gated MySQL support).

#### F-06: CORS default allows all origins when unset
- **Severity:** Medium
- **Status:** âœ… Remediated (2026-03-12)
- **Resolution Summary:**
  - Startup now fails in `staging`/`prod` when `KRAB_CORS_ORIGINS` is empty.
  - Dev-only wildcard fallback remains available.

#### F-07: Rustdoc warning debt
- **Severity:** Medium
- **Impact:** Documentation quality debt and future hard-fail risk under stricter docs policy.
- **Recommendation:** Fix invalid doc tag formatting in protocol docs.

---

## 5) Positive Controls Observed

- Security-oriented startup validation for non-dev environments
- HTTP hardening middleware (security headers, CSRF checks, request-id handling)
- Strong CI workflow coverage: ops hardening, dependency security, API contract, DB lifecycle, E2E, NFT, WASM size, smoke tests
- Container runtime hardening settings in compose (drop caps, no-new-privileges, read-only root fs, pid limits)

---

## 6) Prioritized Remediation Plan

### P0 (Immediate)
1. Install `cargo-deny` and run the dependency audit gate.

### P1 (Near-term)
1. Resolve DB driver contract mismatch between docs and implementation.
2. Verify hardened auth/rate-limit/proxy policies in staging canary.

### P2 (Backlog)
1. Eliminate rustdoc warning debt.
2. Standardize local developer bootstrap to include policy tooling (`cargo-deny`, etc.).

---

## 7) Exit Criteria for â€œRelease Readyâ€

Release readiness can be declared when all are true:

- `cargo fmt --all --check` passes
- `cargo clippy --workspace --all-targets -- -D warnings` passes
- `cargo test --workspace` passes
- Metrics contract is reconciled and verified in alert/dashboards
- Dependency policy scan is runnable and passing in CI and reproducible locally

---

## 8) Audit Status

**Overall Status:** **Open (dependency audit gate missing)**  
**Next Milestone:** Unblock dependency audit and re-run full gate suite.

---

## 9) Hardening Remediation Update (2026-03-13)

### Implemented Controls

- Metrics contract alignment across runtime + Prometheus alerts + Grafana queries.
- Staging/prod CORS enforcement at startup (`KRAB_CORS_ORIGINS` required).
- Trusted proxy control for client-IP extraction (`KRAB_TRUST_PROXY_HEADERS`, default `false`).
- Configurable rate-limit store failure mode (`KRAB_RATE_LIMIT_FAIL_OPEN`).
- JWT verification algorithm allowlist (`KRAB_JWT_ALLOWED_ALGS`, default `HS256`).
- Orchestrator watch-mode scaling via event-based file watch with polling fallback.
- Orchestrator graceful shutdown with timeout and exit capture.
- ISR lock-poison recovery (removed panic-on-poison behavior).
- Removed unsafe `Send/Sync` impls for server-function registration.
- Security/ops docs updated.

### Targeted Performance + Security Test Results

Source: `plans/load_test_artifacts/targeted_hardening_results.json` (timestamp `2026-03-12T19:55:08Z`)

- **Load check** (`GET /health` against `service_auth`):
  - samples=120, success=119, errors=1
  - p50=5.25ms, p95=9.01ms, p99=13.04ms, mean=5.67ms
- **Input-fuzzing probe** (`POST /api/v1/auth/login` with randomized payloads):
  - samples=120, `4xx`=120, `5xx`=0, transport errors=0
  - Interpretation: malformed/randomized input was rejected without server-side crash responses.

### Dependency Audit Tooling Result

- `cargo deny --all-features check advisories licenses bans` remains **not runnable locally** (`cargo deny` subcommand missing in this environment).



