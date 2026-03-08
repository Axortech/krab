# Service: Auth (`service_auth`)

## Role

[`service_auth/src/main.rs`](../../service_auth/src/main.rs) provides authentication and token lifecycle endpoints.

## API surface

Public root/ops routes:

- `/health`
- `/ready`
- `/metrics`
- `/metrics/prometheus`

Versioned auth routes (`/api/v1`):

- `POST /auth/login`
- `POST /auth/refresh`
- `POST /auth/revoke`
- `GET /auth/jwks`
- `GET /auth/status`
- protected contract probe route(s)

## Responsibilities

- credential verification/login flows
- token issuance/refresh/revoke lifecycle
- health/readiness/metrics endpoints

## Runtime security behavior

- refresh token replay detection and revoke semantics
- token-use validation for refresh/revoke flows
- key-ring based JWT verification behavior
- secret sourcing enforcement in non-dev environments

## Configuration surface (high level)

- auth mode and bearer token settings
- JWT key material / key-ring controls
- login bootstrap credential sources (`*_FILE`/vault variants)

Use [`docs/security.md`](../security.md) and [`.env.example`](../../.env.example) as canonical value references.

## Operational notes

- configure auth mode via environment
- ensure secrets are provided via secure sourcing in non-dev

## Related references

- [`docs/security.md`](../security.md)
- [`RELEASE_POLICY.md`](../../RELEASE_POLICY.md)
