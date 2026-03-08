# Operational Runbook

## Service health endpoints

- Auth: `/health`, `/ready`
- Users: `/health`, `/ready`
- Frontend: `/health`, `/ready`

## Incident first steps

1. Check service smoke workflow: [`.github/workflows/service-smoke.yaml`](../../.github/workflows/service-smoke.yaml)
2. Check release criteria: [`RELEASE_POLICY.md`](../../RELEASE_POLICY.md)
3. Check on-call playbook: [`plans/oncall_playbook.md`](../../plans/oncall_playbook.md)

## DB governance

- Lifecycle checks and rollback process are defined in:
  - [`plans/db_rollback_runbook.md`](../../plans/db_rollback_runbook.md)
  - [`krab_core/src/db.rs`](../../krab_core/src/db.rs)
