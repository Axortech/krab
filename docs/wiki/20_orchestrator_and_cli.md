# Orchestrator and CLI

## Orchestrator (`krab_orchestrator`)

Main file: [`krab_orchestrator/src/main.rs`](../../krab_orchestrator/src/main.rs)

### Responsibilities

- Load [`krab.toml`](../../krab.toml)
- Resolve startup order from dependencies
- Spawn and supervise services
- Restart on failure using configured policy
- Run health checks after start/restart

### Supervision loop semantics

- startup order resolved from dependency graph before initial spawn.
- each service child is tracked by name + process handle.
- exited processes are evaluated against effective restart policy.
- restart attempts are capped per service.
- health checks run after initial start and every restart attempt.

### Policy layering

Service policy can come from legacy flat fields or nested policy blocks in [`krab.toml`](../../krab.toml). Effective getters in orchestrator normalize both forms.

### Config schema highlights

- `depends_on`, `startup_dependencies`
- `restart_policy` (`on_exit`, `backoff_ms`, `max_attempts`)
- `healthcheck` (`url`, `timeout_ms`, `retries`, `interval_ms`)

## CLI (`krab_cli`)

Purpose: stable operational commands and developer tooling.

Primary entrypoint: [`krab_cli/src/main.rs`](../../krab_cli/src/main.rs)

Command groups include:

- Service scaffolding (`gen service`)
- API contract gates
- DB governance checks (drift, rollback, policy)
- environment/setup helpers

### Service Generation

The CLI provides scaffolding for creating new services with specific API types.

```bash
krab gen service <NAME> --type <TYPE>
```

Supported types:
- `rest` (Default): Scaffolds an Axum-based REST service.
- `graphql`: Scaffolds an Async-GraphQL service.
- `grpc`: Scaffolds a Tonic-based gRPC service. **Note**: This sets up dependencies but requires manual `.proto` file creation and `build.rs` configuration.

Use in CI for stable interfaces instead of brittle grep/test-name coupling.

### CI-oriented command usage

- contract checks should be executed through CLI contract action wiring.
- DB lifecycle/rollback/drift checks should be executed through CLI DB action wiring.
- CI pipelines should avoid ad-hoc command duplication when stable CLI wrappers exist.