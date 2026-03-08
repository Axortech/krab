# Troubleshooting Playbook

## Triage matrix

| Symptom | Likely layer | First file to inspect |
|---|---|---|
| Hydration fallback alert | Client/macros | [`krab_client/src/lib.rs`](../../krab_client/src/lib.rs) |
| Route returns unexpected 404/500 | Server/router | [`krab_server/src/lib.rs`](../../krab_server/src/lib.rs) |
| Ready fails but health passes | Service dependency | [`service_users/src/main.rs`](../../service_users/src/main.rs) |
| Contract gate CI fails | CLI/workflow | [`krab_cli/src/main.rs`](../../krab_cli/src/main.rs) |
| DB lifecycle gate fails | DB governance | [`krab_core/src/db.rs`](../../krab_core/src/db.rs) |

## Hydration issues

Symptoms:

- UI islands not interactive
- boundary fallback alert rendered

Checks:

1. Inspect browser console for hydration diagnostics from [`krab_client/src/lib.rs`](../../krab_client/src/lib.rs).
2. Confirm island props are valid JSON and schema-compatible.
3. Verify island registration path in [`krab_macros/src/lib.rs`](../../krab_macros/src/lib.rs).

## Service startup failures

Checks:

1. Validate environment via [`.env.example`](../../.env.example).
2. Confirm DB connectivity and driver settings.
3. Check orchestrator healthcheck and restart policy in [`krab.toml`](../../krab.toml).

Deep checks:

- confirm dependency graph in orchestrator startup order.
- inspect restart-attempt cap behavior and backoff timing.
- verify endpoint-specific readiness dependencies are actually reachable.

## CI failures

Checks:

1. Identify failing workflow in [`.github/workflows/`](../../.github/workflows/).
2. Re-run equivalent local command (check/test/deny/contract command).
3. Review policy constraints in [`RELEASE_POLICY.md`](../../RELEASE_POLICY.md) and [`deny.toml`](../../deny.toml).

Workflow-specific hints:

- API contract: ensure schema/contract snapshots were intentionally changed.
- DB lifecycle: check migration ordering, checksum, and rehearsal artifacts.
- service-smoke: verify startup env and readiness loops for users/auth/frontend.

## Escalation artifacts

- service logs with request IDs
- failing command output
- relevant workflow URL/run id

## Recovery procedure template

1. isolate failing layer from matrix above.
2. reproduce with minimal local command.
3. collect logs with request IDs and timestamps.
4. apply targeted fix + regression test.
5. re-run affected CI workflow(s) and document closure.
