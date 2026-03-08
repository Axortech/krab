# Client Hydration (`krab_client`)

## Purpose

[`krab_client`](../../krab_client/src/lib.rs) hydrates server-rendered islands in the browser via WASM.

## Hydration execution flow

1. Entry point: [`hydrate()`](../../krab_client/src/lib.rs)
2. Discover islands via `data-island` attributes.
3. Resolve island factory from inventory.
4. Build virtual node and reconcile with existing DOM via [`hydrate_recursive()`](../../krab_client/src/lib.rs)

Reconciliation model:

- returns consumed node counts (`u32`) to keep parent-child alignment during traversal.
- applies mismatch strategy: replace/append with warnings rather than panic.

## Resilience controls

- Boundary panic capture: [`catch_unwind(AssertUnwindSafe(..))`](../../krab_client/src/lib.rs)
- Structured diagnostics helper: [`log_hydration_diagnostic()`](../../krab_client/src/lib.rs)
- Fallible DOM construction: [`create_dom_node()`](../../krab_client/src/lib.rs) returns `Option<WebNode>`
- Graceful fallback rendering for failed islands.

Failure handling policy:

- unknown/malformed islands are skipped or boundary-marked.
- DOM op failures log diagnostics and continue where possible.
- fallback alert content prevents silent dead UI.

## Observability hooks

- `data-krab-boundary`
- `data-krab-boundary-state`
- Console diagnostics on mismatch/fallback paths

## Extension guidance

When changing hydration internals:

1. keep all browser API calls fallible.
2. preserve boundary-level isolation (avoid global hydration abort).
3. keep diagnostics structured and stable for triage.
4. re-run targeted wasm/client checks.
