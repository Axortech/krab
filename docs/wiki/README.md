# Krab Developer Wiki

This folder is the internal developer wiki for day-to-day engineering work.

## Start Here

- [00_home.md](docs/wiki/00_home.md)
- [01_getting_started.md](docs/wiki/01_getting_started.md)
- [02_architecture_map.md](docs/wiki/02_architecture_map.md)
- [03_dev_workflows.md](docs/wiki/03_dev_workflows.md)
- [04_operational_runbook.md](docs/wiki/04_operational_runbook.md)

## Reading Paths

### Path A — New framework contributor (2–3 hours)

1. [00_home.md](docs/wiki/00_home.md)
2. [01_getting_started.md](docs/wiki/01_getting_started.md)
3. [02_architecture_map.md](docs/wiki/02_architecture_map.md)
4. [10_core_runtime.md](docs/wiki/10_core_runtime.md)
5. [03_dev_workflows.md](docs/wiki/03_dev_workflows.md)

### Path B — Runtime internals deep dive (half day)

1. [10_core_runtime.md](docs/wiki/10_core_runtime.md)
2. [11_server_runtime.md](docs/wiki/11_server_runtime.md)
3. [12_client_hydration.md](docs/wiki/12_client_hydration.md)
4. [13_macro_system.md](docs/wiki/13_macro_system.md)
5. [32_service_frontend.md](docs/wiki/32_service_frontend.md)

### Path C — Platform / release owner

1. [20_orchestrator_and_cli.md](docs/wiki/20_orchestrator_and_cli.md)
2. [21_ci_cd_and_governance.md](docs/wiki/21_ci_cd_and_governance.md)
3. [04_operational_runbook.md](docs/wiki/04_operational_runbook.md)
4. [40_troubleshooting_playbook.md](docs/wiki/40_troubleshooting_playbook.md)

## Detailed Framework Documentation

### Core Runtime

- [10_core_runtime.md](docs/wiki/10_core_runtime.md)
- [11_server_runtime.md](docs/wiki/11_server_runtime.md)
- [12_client_hydration.md](docs/wiki/12_client_hydration.md)
- [13_macro_system.md](docs/wiki/13_macro_system.md)

### Platform & Governance

- [20_orchestrator_and_cli.md](docs/wiki/20_orchestrator_and_cli.md)
- [21_ci_cd_and_governance.md](docs/wiki/21_ci_cd_and_governance.md)

### Services

- [30_service_auth.md](docs/wiki/30_service_auth.md)
- [31_service_users.md](docs/wiki/31_service_users.md)
- [32_service_frontend.md](docs/wiki/32_service_frontend.md)

### Operations

- [40_troubleshooting_playbook.md](docs/wiki/40_troubleshooting_playbook.md)

## Documentation Coverage Matrix

| Area | Deep doc |
|---|---|
| Core config/middleware/db/resilience | [10_core_runtime.md](docs/wiki/10_core_runtime.md) |
| Hyper/Tower server internals | [11_server_runtime.md](docs/wiki/11_server_runtime.md) |
| WASM hydration/reconciliation | [12_client_hydration.md](docs/wiki/12_client_hydration.md) |
| Procedural macro generation paths | [13_macro_system.md](docs/wiki/13_macro_system.md) |
| Orchestrator policies and CLI gates | [20_orchestrator_and_cli.md](docs/wiki/20_orchestrator_and_cli.md) |
| CI/CD and release governance | [21_ci_cd_and_governance.md](docs/wiki/21_ci_cd_and_governance.md) |
| Auth service runtime/contracts | [30_service_auth.md](docs/wiki/30_service_auth.md) |
| Users service runtime/contracts | [31_service_users.md](docs/wiki/31_service_users.md) |
| Frontend service SSR/cache/islands | [32_service_frontend.md](docs/wiki/32_service_frontend.md) |
| Incidents/debug/triage | [40_troubleshooting_playbook.md](docs/wiki/40_troubleshooting_playbook.md) |

## Canonical References

- API reference: [docs/API.md](docs/API.md)
- Security model: [docs/security.md](docs/security.md)
- Database guide: [docs/database.md](docs/database.md)
- Deployment guide: [docs/deployment.md](docs/deployment.md)
