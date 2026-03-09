use anyhow::{Context, Result};
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Environment {
    Dev,
    Staging,
    Prod,
    Unknown(String),
}

pub fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn read_env_or_file(name: &str) -> Result<Option<String>> {
    if let Some(value) = env_non_empty(name) {
        return Ok(Some(value));
    }

    let file_var = format!("{name}_FILE");
    if let Some(path) = env_non_empty(&file_var) {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {} from file path '{}'", file_var, path))?;
        let value = raw.trim().to_string();
        anyhow::ensure!(
            !value.is_empty(),
            "{} points to empty file '{}'",
            file_var,
            path
        );
        return Ok(Some(value));
    }

    Ok(None)
}

impl Environment {
    pub fn from_env() -> Self {
        match std::env::var("KRAB_ENVIRONMENT") {
            Ok(v) if v.eq_ignore_ascii_case("dev") => Self::Dev,
            Ok(v) if v.eq_ignore_ascii_case("staging") => Self::Staging,
            Ok(v) if v.eq_ignore_ascii_case("prod") || v.eq_ignore_ascii_case("production") => {
                Self::Prod
            }
            Ok(v) => Self::Unknown(v),
            Err(_) => Self::Dev,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Dev => "dev",
            Self::Staging => "staging",
            Self::Prod => "prod",
            Self::Unknown(v) => v.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub auth_mode: String,
    pub service_auth_scope: String,
    pub rate_limit_capacity: u64,
    pub rate_limit_refill_per_sec: u64,
    /// Allowed CORS origins. Empty means allow all (`*`). Comma-separated in env var `KRAB_CORS_ORIGINS`.
    pub cors_origins: Vec<String>,
}

impl HttpConfig {
    pub fn from_env() -> Self {
        let cors_origins = std::env::var("KRAB_CORS_ORIGINS")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Self {
            auth_mode: std::env::var("KRAB_AUTH_MODE").unwrap_or_else(|_| "jwt".to_string()),
            service_auth_scope: std::env::var("KRAB_SERVICE_AUTH_SCOPE")
                .unwrap_or_else(|_| "service:internal".to_string()),
            rate_limit_capacity: std::env::var("KRAB_RATE_LIMIT_CAPACITY")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(120),
            rate_limit_refill_per_sec: std::env::var("KRAB_RATE_LIMIT_REFILL_PER_SEC")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60),
            cors_origins,
        }
    }
}

/// Unified application configuration loaded from environment variables.
///
/// Call [`KrabConfig::from_env`] once at startup; pass the result (or
/// specific sub-configs) through the dependency graph instead of reading
/// `std::env::var` ad-hoc in individual modules.
#[derive(Debug, Clone)]
pub struct KrabConfig {
    pub environment: Environment,
    pub service_name: String,
    pub host: String,
    pub port: u16,
    pub log_filter: String,
    pub http: HttpConfig,
}

impl KrabConfig {
    /// Load all configuration from environment variables with typed defaults.
    pub fn from_env(default_service_name: &str, default_port: u16) -> Self {
        Self {
            environment: Environment::from_env(),
            service_name: std::env::var("KRAB_SERVICE_NAME")
                .unwrap_or_else(|_| default_service_name.to_string()),
            host: std::env::var("KRAB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: std::env::var("KRAB_PORT")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(default_port),
            log_filter: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
            http: HttpConfig::from_env(),
        }
    }

    /// Validate security-critical configuration at startup.
    ///
    /// In `Dev` environments all checks are skipped. In `Staging`, `Prod`, or any
    /// unrecognised environment, required secrets must be present or this returns
    /// an error — callers should propagate the error and abort the process.
    pub fn validate(&self) -> anyhow::Result<()> {
        let has_non_empty = |name: &str| !std::env::var(name).unwrap_or_default().trim().is_empty();

        match self.environment {
            Environment::Dev => return Ok(()),
            Environment::Staging | Environment::Prod | Environment::Unknown(_) => {}
        }

        let auth_mode = self.http.auth_mode.as_str();
        if auth_mode.eq_ignore_ascii_case("static") {
            anyhow::bail!(
                "KRAB_AUTH_MODE=static is not allowed in '{}' environment; use KRAB_AUTH_MODE=jwt (or oidc) with provider-based validation",
                self.environment.as_str()
            );
        } else if auth_mode.eq_ignore_ascii_case("jwt") || auth_mode.eq_ignore_ascii_case("oidc") {
            let has_provider_json = has_non_empty("KRAB_JWT_PROVIDERS_JSON");
            let has_provider_json_file = has_non_empty("KRAB_JWT_PROVIDERS_JSON_FILE");
            let has_provider_json_vault_ref = has_non_empty("KRAB_JWT_PROVIDERS_JSON_VAULT_REF");
            let token = std::env::var("KRAB_BEARER_TOKEN").unwrap_or_default();
            if !token.trim().is_empty() {
                anyhow::bail!(
                    "KRAB_BEARER_TOKEN must be unset/empty in '{}' environment when KRAB_AUTH_MODE={} \
                     (static bearer tokens are not allowed outside dev)",
                    self.environment.as_str(),
                    auth_mode
                );
            }

            let has_keys = has_non_empty("KRAB_JWT_KEYS_JSON");
            let has_secret = has_non_empty("KRAB_JWT_SECRET");
            let has_keys_file = has_non_empty("KRAB_JWT_KEYS_JSON_FILE");
            let has_secret_file = has_non_empty("KRAB_JWT_SECRET_FILE");
            let has_keys_vault_ref = has_non_empty("KRAB_JWT_KEYS_JSON_VAULT_REF");
            let has_secret_vault_ref = has_non_empty("KRAB_JWT_SECRET_VAULT_REF");
            let has_issuer = has_non_empty("KRAB_OIDC_ISSUER");
            let has_audience = has_non_empty("KRAB_OIDC_AUDIENCE");

            let has_secure_secret_source =
                has_keys_file || has_secret_file || has_keys_vault_ref || has_secret_vault_ref;

            if (has_keys || has_secret) && !has_secure_secret_source {
                anyhow::bail!(
                    "In '{}' environment, inline KRAB_JWT_SECRET/KRAB_JWT_KEYS_JSON is forbidden; use *_FILE or *_VAULT_REF secret sourcing",
                    self.environment.as_str()
                );
            }

            let has_provider_bundle =
                has_provider_json || has_provider_json_file || has_provider_json_vault_ref;
            let has_fallback_provider_tuple =
                (has_keys || has_secret || has_keys_file || has_secret_file)
                    && has_issuer
                    && has_audience;

            if !has_provider_bundle && !has_fallback_provider_tuple {
                anyhow::bail!(
                    "JWT/OIDC provider configuration required in '{}' environment; set KRAB_JWT_PROVIDERS_JSON \
                     or provide KRAB_OIDC_ISSUER + KRAB_OIDC_AUDIENCE + secure secret sourcing via \
                     KRAB_JWT_SECRET_FILE/KRAB_JWT_KEYS_JSON_FILE (or *_VAULT_REF)",
                    self.environment.as_str()
                );
            }
        } else {
            anyhow::bail!(
                "Unsupported KRAB_AUTH_MODE='{}' in '{}' environment; use jwt or oidc",
                auth_mode,
                self.environment.as_str()
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        match ENV_LOCK.get_or_init(|| Mutex::new(())).lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn clear_auth_env() {
        for key in [
            "KRAB_ENVIRONMENT",
            "KRAB_AUTH_MODE",
            "KRAB_BEARER_TOKEN",
            "KRAB_JWT_SECRET",
            "KRAB_JWT_SECRET_FILE",
            "KRAB_JWT_SECRET_VAULT_REF",
            "KRAB_JWT_KEYS_JSON",
            "KRAB_JWT_KEYS_JSON_FILE",
            "KRAB_JWT_KEYS_JSON_VAULT_REF",
            "KRAB_JWT_PROVIDERS_JSON",
            "KRAB_JWT_PROVIDERS_JSON_FILE",
            "KRAB_JWT_PROVIDERS_JSON_VAULT_REF",
            "KRAB_OIDC_ISSUER",
            "KRAB_OIDC_AUDIENCE",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    #[serial]
    fn validate_rejects_static_in_non_local_env() {
        let _guard = env_lock();
        clear_auth_env();
        std::env::set_var("KRAB_ENVIRONMENT", "staging");
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "token");

        let cfg = KrabConfig::from_env("users", 3002);
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("KRAB_AUTH_MODE=static is not allowed"));
    }

    #[test]
    #[serial]
    fn validate_requires_provider_configuration_in_non_local_jwt_mode() {
        let _guard = env_lock();
        clear_auth_env();
        std::env::set_var("KRAB_ENVIRONMENT", "prod");
        std::env::set_var("KRAB_AUTH_MODE", "jwt");

        let cfg = KrabConfig::from_env("users", 3002);
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("JWT/OIDC provider configuration required"));
    }

    #[test]
    #[serial]
    fn validate_accepts_static_mode_in_dev() {
        let _guard = env_lock();
        clear_auth_env();
        std::env::set_var("KRAB_ENVIRONMENT", "dev");
        std::env::set_var("KRAB_AUTH_MODE", "static");
        std::env::set_var("KRAB_BEARER_TOKEN", "token");

        let cfg = KrabConfig::from_env("users", 3002);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    #[serial]
    fn validate_accepts_oidc_tuple_in_non_local_env() {
        let _guard = env_lock();
        clear_auth_env();
        std::env::set_var("KRAB_ENVIRONMENT", "staging");
        std::env::set_var("KRAB_AUTH_MODE", "oidc");
        std::env::set_var("KRAB_OIDC_ISSUER", "https://issuer.example.com");
        std::env::set_var("KRAB_OIDC_AUDIENCE", "krab-api");
        std::env::set_var("KRAB_JWT_SECRET_FILE", "/run/secrets/krab_jwt_secret");

        let cfg = KrabConfig::from_env("users", 3002);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    #[serial]
    fn validate_rejects_inline_secret_in_non_local_env() {
        let _guard = env_lock();
        clear_auth_env();
        std::env::set_var("KRAB_ENVIRONMENT", "prod");
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_OIDC_ISSUER", "https://issuer.example.com");
        std::env::set_var("KRAB_OIDC_AUDIENCE", "krab-api");
        std::env::set_var("KRAB_JWT_SECRET", "secret-inline");

        let cfg = KrabConfig::from_env("users", 3002);
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("inline KRAB_JWT_SECRET/KRAB_JWT_KEYS_JSON is forbidden"));
    }
}
