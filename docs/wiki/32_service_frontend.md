# Service: Frontend (`service_frontend`)

## Role

[`service_frontend/src/main.rs`](../../service_frontend/src/main.rs) serves SSR pages, island hydration bootstrapping, API endpoints, and cache middleware.

## Responsibilities

- server-side page rendering
- static/data endpoint handling
- island hydration bootstrapping scripts
- cache management with explicit authority model

## Route and runtime surface

Operational routes:

- `/health`
- `/ready`
- `/metrics`
- `/metrics/prometheus`

Application routes include:

- SSR pages (home/about/greet/blog/...)
- data and API helper endpoints
- hydration bootstrapping script integration

## Cache model

- `CacheAuthority::Isr` for selected page routes
- `CacheAuthority::Distributed` for selected API/static payload routes
- separation implemented in [`cache_authority()`](../../service_frontend/src/main.rs)

Behavioral contract:

- ISR routes should not be served from distributed cache path.
- distributed cache routes should avoid ISR revalidation semantics.
- cache/debug headers should communicate path decisions (`x-cache`, `x-isr-state`).

## Operational notes

- exposes `/health`, `/ready`, and metrics endpoints
- CI smoke validates runtime availability

## Extension checklist

When adding/changing routes:

1. classify route cache authority explicitly.
2. add/adjust middleware tests for expected headers and freshness behavior.
3. confirm hydration scripts remain aligned with island registration contracts.

## Related references

- [12_client_hydration.md](12_client_hydration.md)
- [21_ci_cd_and_governance.md](21_ci_cd_and_governance.md)
