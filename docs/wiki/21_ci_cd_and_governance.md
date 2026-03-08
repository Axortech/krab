# CI/CD and Governance

## Core workflows

- [`.github/workflows/ops-hardening.yaml`](../../.github/workflows/ops-hardening.yaml)
- [`.github/workflows/dependency-security.yaml`](../../.github/workflows/dependency-security.yaml)
- [`.github/workflows/api-contract.yaml`](../../.github/workflows/api-contract.yaml)
- [`.github/workflows/db-lifecycle.yaml`](../../.github/workflows/db-lifecycle.yaml)
- [`.github/workflows/service-smoke.yaml`](../../.github/workflows/service-smoke.yaml)

## Workflow intent and gate semantics

### Ops hardening

- blocks on format/lint/test/doc/dependency checks.
- ensures baseline engineering quality before merge.

### API contract

- validates API behavior through stable contract-oriented checks.
- intended to prevent accidental envelope or route contract drift.

### DB lifecycle

- validates migration lifecycle, rollback, and drift checks.
- intended to enforce migration governance policy under CI.

### Service smoke

- validates runtime health endpoints for key services.
- includes users runtime readiness checks (not compile-only).

## Governance artifacts

- Release policy: [`RELEASE_POLICY.md`](../../RELEASE_POLICY.md)
- Dependency policy: [`deny.toml`](../../deny.toml)
- Audit reports: [`audit/`](../../audit/)

## Policy consistency model

- Release policy language in [`RELEASE_POLICY.md`](../../RELEASE_POLICY.md) should match enforced CI gates.
- Dependency policy in [`deny.toml`](../../deny.toml) should align with release criteria statements.
- Governance docs should be updated whenever a gate's behavior changes.

## Gate expectations

- formatting/lint/tests must pass
- contract and DB lifecycle checks must pass
- service smoke must pass including users runtime readiness

## Promotion policy

Use release channels and criteria from [`RELEASE_POLICY.md`](../../RELEASE_POLICY.md) as source of truth.

## Change management checklist

When modifying workflows:

1. update this doc + relevant wiki pages.
2. ensure local reproducibility of changed gate commands.
3. confirm no gate regression in PR runs.
