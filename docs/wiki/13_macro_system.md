# Macro System (`krab_macros`)

## Purpose

[`krab_macros`](../../krab_macros/src/lib.rs) contains compile-time transforms for Krab view syntax and island generation.

## Main macros

- [`view!`](../../krab_macros/src/lib.rs): declarative HTML-like syntax to `krab_core::Node`
- [`#[island]`](../../krab_macros/src/lib.rs): generates server/client dual behavior and hydration registration

## Island generation model

- Server path wraps rendered output with `data-island` and serialized `data-props`.
- Web path registers a hydrator in inventory for runtime island resolution.

Generated shape summary:

1. Original function is split into internal implementation + cfg-gated wrappers.
2. Server wrapper serializes props and emits island metadata attributes.
3. Client wrapper executes local render path.
4. Hydration factory function is registered for runtime lookup.

## Safety behavior

- Prop decode handled with fallible `match` in generated hydrator (no panic on malformed props).
- Decode failures return a boundary-marked fallback node and emit diagnostics.

Contract invariants:

- island name in markup must match registered inventory entry.
- prop schema drift must degrade locally, never crash whole hydrate pass.
- fallback node must be deterministic for operational visibility.

## Extension guidance

When extending macro output:

1. keep generated code panic-free for decode and boundary paths.
2. maintain cfg split correctness (`web` vs non-`web`).
3. preserve backward-compatible attribute conventions unless coordinated.
4. update docs/tests for generated AST shape changes.

## Related runtime

- Hydration consumer: [`krab_client/src/lib.rs`](../../krab_client/src/lib.rs)
