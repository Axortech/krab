# Phase E — Governance, CI Hardening, and CLI Updates

> **Goal**: finalize governance docs, enforce parity and policy gates in CI, extend CLI scaffolding for multi-protocol services, and publish migration notes.

---

## E-0  Current implementation notes

| Item | Current state |
|---|---|
| `plans/api_governance.md` | Covers REST versioning, GraphQL schema evolution, deprecation timeline, error model. No protocol parity or exposure mode change rules. |
| `krab_cli gen service` | Generates single-protocol service stub with `--type rest\|graphql\|grpc`. No multi-adapter or topology options. |
| CI | `krab_cli contract check` runs `krab_core` API tests + `service_auth`/`service_users` contract tests. No protocol matrix. |
| Docs | `docs/API.md` (if it exists) documents current REST/GraphQL endpoints. No capability endpoint docs. |
| `.env.example` | 3899 bytes. Does not include `KRAB_PROTOCOL_*` variables. |

---

## E-1  Governance doc updates (`plans/api_governance.md`)

Add a new section **"5. Protocol Parity and Exposure Mode Policy"**:

### 5.1 Parity Matrix Required
- For any operation exposed on multiple protocols, a parity matrix document must exist in `plans/` listing:
  - REST endpoint ↔ GraphQL query/mutation ↔ RPC method
  - Known behavioral gaps (if any)
  - Which adapter is the "source of record"

### 5.2 Behavioral Equivalence Tests
- For dual/multi-exposed operations, response semantics and authorization outcomes must match.
- Parity tests are mandatory CI gates — failures block release.

### 5.3 Protocol Change Classification
| Change | Classification |
|---|---|
| Adding a new protocol adapter for an existing operation | Minor |
| Removing protocol support for an operation | **Major** + sunset policy |
| Switching from multi-mode to single-mode | **Breaking** unless no external consumers exist |

### 5.4 Deprecation by Protocol Surface
- Deprecation notices must specify the protocol surface explicitly.
  - Example: "REST field `user.email` deprecated in v1.3; use GraphQL field `User.emailAddress`."

### 5.5 Observability Labels
- All metrics, traces, and logs MUST include `protocol` dimension.
- Suggested labels: `protocol="rest|graphql|rpc"`, `operation="users.getMe"`, `selection_source="policy|tenant|client|default"`.

### 5.6 Exposure Mode Change Policy
- Switching from multi → single mode is treated as a breaking change.
- Requires migration notice and sunset timeline (minimum 6 months per existing deprecation policy).

---

## E-2  CLI updates (`krab_cli`)

### E-2.1  Extended `gen service` command

```
krab gen service payment \
  --type rest \
  --exposure-mode multi \
  --protocols rest,graphql \
  --topology single_service
```

New CLI arguments:

| Argument | Values | Default | Description |
|---|---|---|---|
| `--exposure-mode` | `single`, `multi` | `single` | Whether to scaffold one or multiple adapters. |
| `--protocols` | CSV of `rest,graphql,rpc` | Same as `--type` | Which adapters to generate. Only valid when `--exposure-mode=multi`. |
| `--topology` | `single_service`, `split_services` | `single_service` | Deployment shape. |

### E-2.2  Template generation behavior

**`single` mode** (current behavior, preserved):
- Scaffolds only one `src/main.rs` with the selected protocol's routes.

**`multi` mode** (new):
1. Generate directory structure:
   ```
   service_{name}/
   ├── Cargo.toml
   └── src/
       ├── main.rs
       ├── domain/
       │   ├── mod.rs
       │   ├── models.rs
       │   └── service.rs
       ├── adapters/
       │   ├── mod.rs
       │   ├── rest.rs    (if "rest" in --protocols)
       │   ├── graphql.rs (if "graphql" in --protocols)
       │   └── rpc.rs     (if "rpc" in --protocols)
       └── capabilities.rs
   ```
2. `main.rs` wires adapters based on `ProtocolConfig.from_env()`.
3. `capabilities.rs` constructs `ServiceCapabilities` from enabled protocols.
4. `Cargo.toml` includes feature flags matching selected protocols.

**`split_services` topology** (new, advanced):
- Generates multiple sibling crate directories sharing a common domain crate:
  ```
  service_{name}_domain/    (shared domain logic)
  service_{name}_rest/      (REST adapter binary)
  service_{name}_graphql/   (GraphQL adapter binary)
  service_{name}_rpc/       (RPC adapter binary)
  ```

### E-2.3  CLI code changes in `krab_cli/src/main.rs`

```rust
#[derive(Subcommand)]
enum GenResource {
    Service {
        name: String,
        #[arg(long, value_enum)]
        r#type: ServiceType,
        #[arg(long, value_enum, default_value_t = ExposureMode::Single)]
        exposure_mode: ExposureMode,
        #[arg(long, value_delimiter = ',')]
        protocols: Option<Vec<ServiceType>>,
        #[arg(long, value_enum, default_value_t = Topology::SingleService)]
        topology: Topology,
    },
    // ... existing variants
}

#[derive(Clone, ValueEnum, Debug)]
enum ExposureMode {
    Single,
    Multi,
}

#[derive(Clone, ValueEnum, Debug)]
enum Topology {
    SingleService,
    SplitServices,
}
```

---

## E-3  CI pipeline additions

### E-3.1  Protocol matrix tests

Add to CI configuration (GitHub Actions example):

```yaml
jobs:
  protocol-matrix:
    strategy:
      matrix:
        mode: [single, multi]
        protocol: [rest, graphql, rpc]
    env:
      KRAB_PROTOCOL_EXPOSURE_MODE: ${{ matrix.mode }}
      KRAB_PROTOCOL_ENABLED: ${{ matrix.mode == 'multi' && 'rest,graphql,rpc' || matrix.protocol }}
      KRAB_PROTOCOL_DEFAULT: ${{ matrix.protocol }}
    steps:
      - run: cargo test -p service_users
```

### E-3.2  Parity suite as release gate

```yaml
  parity-tests:
    steps:
      - run: cargo test -p service_users -- parity_
      - run: cargo test -p krab_core --features rest -- protocol
```

Parity test failures block merges to `main` and release branches.

### E-3.3  New CLI command: `krab contract protocol-check`

```rust
ContractAction::ProtocolCheck { diagnostics } => {
    run_command_logged(
        "protocol parity tests",
        Command::new("cargo")
            .arg("test")
            .arg("--package").arg("service_users")
            .arg("parity_"),
        diagnostics,
    )?;
    run_command_logged(
        "protocol resolver tests",
        Command::new("cargo")
            .arg("test")
            .arg("--package").arg("krab_core")
            .arg("--features").arg("rest")
            .arg("protocol"),
        diagnostics,
    )?;
}
```

---

## E-4  Environment template updates (`.env.example`)

Add the following block:

```env
# === Protocol Flexibility ===
# KRAB_PROTOCOL_EXPOSURE_MODE=single|multi       # Default: single
# KRAB_PROTOCOL_ENABLED=rest,graphql,rpc          # CSV, default: depends on service
# KRAB_PROTOCOL_DEFAULT=rest|graphql|rpc          # Must be in ENABLED set
# KRAB_PROTOCOL_ALLOW_CLIENT_OVERRIDE=true|false  # Default: false
# KRAB_PROTOCOL_TOPOLOGY=single_service|split_services  # Default: single_service
# KRAB_PROTOCOL_RESTRICTED_OPS_JSON={}            # JSON: operation → [protocols]
# KRAB_PROTOCOL_TENANT_OVERRIDES_JSON={}          # JSON: tenant_id → [protocols]
# KRAB_PROTOCOL_SPLIT_TARGETS_JSON={}             # JSON: service → { protocol → url }
```

---

## E-5  Observability: dashboard and alerting updates

### E-5.1  Prometheus metrics additions

Extend the `metrics_prometheus` handler format string to include protocol dimensions:

```
krab_http_requests_total{protocol="rest"} ...
krab_http_requests_total{protocol="graphql"} ...
krab_http_requests_total{protocol="rpc"} ...
krab_auth_failures_total{protocol="rest"} ...
```

### E-5.2  Tracing attributes

Extend tracing spans with:
```
krab.protocol = "rest|graphql|rpc"
krab.operation = "users.getMe"
krab.selection_source = "policy|tenant|client|default"
```

### E-5.3  Dashboard additions (Grafana / monitoring config)

Add to `monitoring/` or `plans/service_dashboard.json.md`:
- Protocol-segmented latency histograms
- Protocol-segmented error rate panels
- Protocol-segmented request rate panels
- Parity drift alert (response differences between protocols for same operation)

---

## E-6  Documentation updates

### E-6.1  `docs/protocol_flexibility.md` (new public-facing doc)

Contents:
1. Overview of protocol selection model
2. Capability endpoint contract (`GET /api/capabilities`)
3. Client preference header (`x-krab-protocol`)
4. Resolution priority order
5. Configuration variables
6. Migration guide for existing integrators

### E-6.2  API reference updates

Update `docs/API.md` (or create if missing) to include:
- Capability endpoint documentation per service
- Protocol selection semantics
- Operation-level restrictions (auth lifecycle on REST-only)

### E-6.3  CHANGELOG.md entry

Add entry for the protocol flexibility feature under the appropriate version.

---

## E-7  Acceptance gates for Phase E

- [ ] `plans/api_governance.md` includes protocol parity rules.
- [ ] `krab gen service --exposure-mode multi --protocols rest,graphql` generates correct directory structure.
- [ ] CI protocol matrix runs and passes.
- [ ] Parity tests are mandatory release gates.
- [ ] `.env.example` includes all `KRAB_PROTOCOL_*` variables.
- [ ] `krab contract protocol-check` runs parity + resolver tests.
- [ ] `docs/protocol_flexibility.md` exists and documents the full contract.
- [ ] Prometheus metrics include `protocol` dimension.
- [ ] CHANGELOG.md has an entry for protocol flexibility.
