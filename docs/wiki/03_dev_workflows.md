# Development Workflows

## Common checks

Run full workspace check:

```bash
cargo check --workspace
```

Run targeted checks:

```bash
cargo check -p krab_client -p krab_macros -p krab_server
```

## CI workflow references

- Service smoke: [`.github/workflows/service-smoke.yaml`](../../.github/workflows/service-smoke.yaml)
- API contract: [`.github/workflows/api-contract.yaml`](../../.github/workflows/api-contract.yaml)
- DB lifecycle: [`.github/workflows/db-lifecycle.yaml`](../../.github/workflows/db-lifecycle.yaml)

## Security and policy

- Release policy: [`RELEASE_POLICY.md`](../../RELEASE_POLICY.md)
- Dependency governance: [`deny.toml`](../../deny.toml)

## Scaffolding

Generate new resources using the CLI:

```bash
# Generate a service
cargo run -p krab_cli -- gen service <NAME> --type <rest|graphql|grpc>

# Generate a component (for frontend)
cargo run -p krab_cli -- gen component <NAME>

# Generate a route (for frontend)
cargo run -p krab_cli -- gen route <NAME>
```