# Service: Users (`service_users`)

## Role

[`service_users/src/main.rs`](../../service_users/src/main.rs) serves user-domain APIs and GraphQL contracts with tenant-aware data access.

## Responsibilities

- GraphQL user queries/mutations
- tenant-scoped data access rules
- DB driver support (`postgres` / `sqlite`)
- health/readiness/metrics endpoints

## API and contract surface

Operational routes:

- `/health`
- `/ready`
- `/metrics`
- `/metrics/prometheus`

Versioned API surface:

- GraphQL endpoint under `/api/v1`
- admin/tenant-sensitive paths enforced by auth + context checks

Contract assets and checks:

- baseline schema: [`service_users/contracts/graphql_schema_v1.graphql`](../../service_users/contracts/graphql_schema_v1.graphql)
- contract tests in [`service_users/src/main.rs`](../../service_users/src/main.rs)

## Contract assets

- GraphQL baseline schema: [`service_users/contracts/graphql_schema_v1.graphql`](../../service_users/contracts/graphql_schema_v1.graphql)

## Operational notes

- runtime smoke in CI validates `/health` and `/ready`
- readiness reflects dependency availability

## Database behavior

- Supports `postgres` and `sqlite` driver modes.
- Readiness reports dependency state based on pool availability.
- Migration/bootstrap behavior differs by driver and environment policy.

## Extension checklist

When changing GraphQL contracts:

1. update schema baseline intentionally.
2. ensure contract tests are adjusted and pass.
3. validate auth/tenant invariants remain enforced.

## Related references

- [`docs/database.md`](../database.md)
- [21_ci_cd_and_governance.md](21_ci_cd_and_governance.md)
