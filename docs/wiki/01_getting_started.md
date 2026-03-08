# Getting Started

## Audience

Developers who want to install Krab locally, run services, and start building features.

## Prerequisites

- Rust toolchain (stable)
- Cargo
- Git
- PostgreSQL (or SQLite for lightweight local runs)

## Install / Clone

```bash
git clone <your-krab-repo-url>
cd krab
```

Copy environment template:

```bash
cp .env.example .env
```

Edit `.env` for your local setup (DB/auth values).

## Build workspace

```bash
cargo check --workspace
```

## Choose database mode

### PostgreSQL mode (default)

Set `KRAB_DB_DRIVER=postgres` and a valid `DATABASE_URL` in `.env`.

### SQLite mode (quick local)

Set:

```bash
KRAB_DB_DRIVER=sqlite
DATABASE_URL=sqlite://krab_users.sqlite?mode=rwc
```

## Run key services

- Auth service: `cargo run --bin service_auth`
- Users service: `cargo run --bin service_users`
- Frontend service: `cargo run --bin service_frontend`

## Create a new service

Use the CLI to scaffold a new service:

```bash
# REST service (default)
cargo run -p krab_cli -- gen service my_service --type rest

# GraphQL service
cargo run -p krab_cli -- gen service my_graphql_service --type graphql
```

## Run orchestrator

Use [`krab.toml`](../../krab.toml) and run:

```bash
cargo run --bin krab_orchestrator
```

## Verify local runtime

Check service health endpoints:

```bash
curl -sf http://127.0.0.1:3001/health
curl -sf http://127.0.0.1:3002/health
curl -sf http://127.0.0.1:3000/health
```

Open frontend:

- `http://127.0.0.1:3002`

## Next steps

1. Read [02_architecture_map.md](02_architecture_map.md)
2. Follow [03_dev_workflows.md](03_dev_workflows.md)
3. Review service-specific pages:
   - [30_service_auth.md](30_service_auth.md)
   - [31_service_users.md](31_service_users.md)
   - [32_service_frontend.md](32_service_frontend.md)