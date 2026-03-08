# Krab Framework Planning Documentation

This directory contains architectural planning and operational reference documents for the Krab full-stack Rust framework.

## Architecture and Roadmap

1. [**Vision & Philosophy**](./01_vision_and_philosophy.md) — Why Krab exists; core values (DX First, Performance by Default, End-to-End Type Safety); differentiators.
2. [**Architecture Design**](./02_architecture_design.md) — Technical deep-dive into the Server, Client, and Build systems; Islands Architecture; file-system routing; data loading patterns.
3. [**Implementation Roadmap**](./03_roadmap.md) — Phase 0 roadmap, governance, epic breakdown, and risk log.
4. [**Package & Plugin Management**](./04_package_management.md) — Pure Rust strategy; Cargo-based configuration; asset pipeline.
5. [**Performance & Efficiency Plan**](./05_performance_and_efficiency.md) — Build-time optimizations; zero-copy server architecture; Islands efficiency.
6. [**Production Readiness**](./08_production_readiness.md) — Production checklist and gate definitions.

## Developer Workflow

7. [**Dev Workflow**](./07_dev_workflow.md) — Local development workflow, watch mode, and tooling.
8. [**Environment Template**](./environment_template.md) — Full environment variable reference with validation rules.

## Operations

9. [**On-Call Playbook**](./oncall_playbook.md) — Incident response procedures and alert actions.
10. [**DB Rollback Runbook**](./db_rollback_runbook.md) — Database rollback procedures and disaster recovery.
11. [**SLO & Alerting Policy**](./slo_alerts.md) — Service Level Objectives, burn-rate alerts, and thresholds.
12. [**API Contract Governance**](./api_governance.md) — Versioning, schema, and change policy.
13. [**Service Dashboard**](./service_dashboard.json.md) — Grafana dashboard template and alert rule reference.
14. [**Load Test Artifacts**](./load_test_artifacts/README.md) — NFT evidence repository, thresholds, and CI contract.
