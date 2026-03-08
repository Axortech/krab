# Architecture Map

## Core layers

- Shared primitives and middleware: [`krab_core/src/lib.rs`](../../krab_core/src/lib.rs)
- HTTP stack and middleware: [`krab_core/src/http.rs`](../../krab_core/src/http.rs)
- DB governance and migrations: [`krab_core/src/db.rs`](../../krab_core/src/db.rs)

## Runtime modules

- SSR/server runtime: [`krab_server/src/lib.rs`](../../krab_server/src/lib.rs)
- WASM client runtime: [`krab_client/src/lib.rs`](../../krab_client/src/lib.rs)
- Macro generation: [`krab_macros/src/lib.rs`](../../krab_macros/src/lib.rs)
- Service orchestration: [`krab_orchestrator/src/main.rs`](../../krab_orchestrator/src/main.rs)

## Services

- Authentication: [`service_auth/src/main.rs`](../../service_auth/src/main.rs)
- Users API: [`service_users/src/main.rs`](../../service_users/src/main.rs)
- Frontend: [`service_frontend/src/main.rs`](../../service_frontend/src/main.rs)
