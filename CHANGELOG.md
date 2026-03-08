# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/) and [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-03-06

### Added
- Multi-service NFT gate with single/scale (`N=1` vs `N=3`) validation.
- SLO burn-rate alert linkage and on-call mapping.
- Rustdoc CI gate and docs publish workflow.
- Centralized `read_env_or_file` secret sourcing utility in `krab_core::config`.
- `env_non_empty` helper for safe non-empty environment variable reads.
- SQLite database driver support (`KRAB_DB_DRIVER=sqlite`) with full schema bootstrap.
- Feature maturity closure items:
  - Route-level middleware chaining for file-based routes.
  - Incremental Static Regeneration (ISR) stale-while-revalidate integration in frontend cache flow.
  - i18n locale detection + localized home rendering (Accept-Language + locale-prefixed route).
  - WebSocket ergonomic layer service integration (`/api/ws/chat`, `/api/ws/publish`).
- Comprehensive publish-ready documentation suite:
  - `docs/security.md` — Security architecture, secret management, threat model.
  - `docs/database.md` — Database architecture, multi-driver support, migration governance.
  - `docs/deployment.md` — Deployment guide for Docker, Kubernetes, and self-hosted.
  - Rewritten `README.md` with full architecture, feature, and configuration reference.

### Changed
- Load-test thresholds and trend artifacts expanded to service-level tracking.
- `config` crate upgraded from `0.13` to `0.14` in `krab_core` and `krab_orchestrator` (eliminates `yaml-rust` unmaintained advisory).
- `DATABASE_URL` now supports `DATABASE_URL_FILE` secret sourcing via `read_env_or_file` pattern.
- `service_users` database backend changed from MySQL to SQLite as the alternative to PostgreSQL.
- `sqlx` configured with `default-features = false` to minimize dependency surface.

### Removed
- MySQL database driver and all associated scaffolding code (`MySqlUserRepository`, `MySqlPool`, MySQL schema bootstrap).
- `rsa` crate entirely removed from dependency tree (was pulled in transitively by `sqlx-mysql`).
- `yaml-rust` crate removed from dependency tree (was pulled in by `config` 0.13).
- `RUSTSEC-2023-0071` removed from `deny.toml` ignore list (vulnerability no longer present).
- `RUSTSEC-2024-0320` removed from `deny.toml` ignore list (vulnerability no longer present).
- `deny.toml` `ignore` array emptied — zero advisory exceptions.

### Security
- `sqlx` moved to `0.8.x` in workspace services.
- Non-local auth startup now rejects insecure/default JWT/bootstrap credentials.
- Production secret sourcing enforced via `*_FILE` / `*_VAULT_REF` pattern.
- `cargo deny --all-features check advisories licenses bans` passes with zero ignores.
- All RUSTSEC advisories resolved at the crate level (not suppressed).

### Fixed
- `deny.toml` syntax errors corrected for `cargo-deny` compatibility (`unsound`, `yanked`, `unmaintained` values).
- Deprecated `copyleft` key removed from `deny.toml` `[licenses]` section.
