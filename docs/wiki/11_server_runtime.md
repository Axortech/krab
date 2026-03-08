# Server Runtime (`krab_server`)

## Purpose and scope

[`krab_server`](../../krab_server/src/lib.rs) provides the low-level server/router/runtime primitives for SSR and static asset serving.

---

## Request lifecycle

1. TCP accept loop in [`Server`](../../krab_server/src/lib.rs)
2. Request routed through [`handle_request()`](../../krab_server/src/lib.rs)
3. Static `/pkg/*` handling (if configured)
4. Router trie dispatch for app routes
5. Controlled 404/500 fallback responses

---

## Key components

- Route trie and dynamic parameter matching: [`Router`](../../krab_server/src/lib.rs)
- Service runner and connection handling: [`Server`](../../krab_server/src/lib.rs)
- Request dispatch and static serving: [`handle_request()`](../../krab_server/src/lib.rs)
- Static path safety checks: [`resolve_static_pkg_path()`](../../krab_server/src/lib.rs)

## Routing internals

- routes are inserted into a trie with static and dynamic segments.
- dynamic segment extraction is accumulated into `Params`.
- unresolved paths return `None` from router and map to controlled 404.

## Reliability behavior

- Fallible header parsing via [`set_header_from_str()`](../../krab_server/src/lib.rs)
- MIME detection via [`static_mime_for()`](../../krab_server/src/lib.rs)
- Safe fallback responses: [`build_not_found_response()`](../../krab_server/src/lib.rs), [`build_internal_error_response()`](../../krab_server/src/lib.rs)

Failure mode policy:

- invalid dynamic header values should map to 500, not panic.
- unreadable static files map to 404/500 based on IO class.
- handler errors should remain contained to request scope.

## Security behavior

- Path traversal protection in static handler.
- `X-Content-Type-Options: nosniff` for static responses.

## Extension guidelines

When adding new server capabilities:

1. keep all header mutation paths fallible.
2. avoid introducing `unwrap`/`expect` in request hot paths.
3. preserve traversal guard semantics for any new static namespaces.
4. add regression tests in [`krab_server/src/lib.rs`](../../krab_server/src/lib.rs).
