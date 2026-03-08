use anyhow::Result;
use config::Config;
use krab_core::resilience::CircuitBreaker;
use krab_core::telemetry::init_tracing;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::process::Command;
use tracing::{error, info, warn};

#[derive(Debug, Deserialize)]
struct KrabConfig {
    services: HashMap<String, ServiceDefinition>,
    #[serde(default)]
    watch: Option<WatchConfig>,
}

#[derive(Debug, Deserialize)]
struct ServiceDefinition {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    watch: bool,
    #[serde(default = "default_true")]
    restart_on_exit: bool,
    #[serde(default = "default_restart_backoff_ms")]
    restart_backoff_ms: u64,
    #[serde(default = "default_max_restart_attempts")]
    max_restart_attempts: u32,
    #[serde(default)]
    healthcheck_url: Option<String>,
    #[serde(default = "default_healthcheck_timeout_ms")]
    healthcheck_timeout_ms: u64,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    startup_dependencies: Vec<String>,
    #[serde(default)]
    restart_policy: Option<RestartPolicyConfig>,
    #[serde(default)]
    healthcheck: Option<HealthProbeConfig>,
}

#[derive(Debug, Deserialize)]
struct RestartPolicyConfig {
    #[serde(default = "default_true")]
    on_exit: bool,
    #[serde(default = "default_restart_backoff_ms")]
    backoff_ms: u64,
    #[serde(default = "default_max_restart_attempts")]
    max_attempts: u32,
}

#[derive(Debug, Deserialize)]
struct HealthProbeConfig {
    url: String,
    #[serde(default = "default_healthcheck_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_healthcheck_retries")]
    retries: u8,
    #[serde(default = "default_healthcheck_interval_ms")]
    interval_ms: u64,
}

#[derive(Debug, Deserialize, Clone)]
struct WatchConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_poll_ms")]
    poll_ms: u64,
    #[serde(default)]
    paths: Vec<String>,
}

fn default_poll_ms() -> u64 {
    1000
}

fn default_true() -> bool {
    true
}

fn default_restart_backoff_ms() -> u64 {
    500
}

fn default_max_restart_attempts() -> u32 {
    5
}

fn default_healthcheck_timeout_ms() -> u64 {
    1200
}

fn default_healthcheck_retries() -> u8 {
    10
}

fn default_healthcheck_interval_ms() -> u64 {
    250
}

impl ServiceDefinition {
    fn effective_restart_on_exit(&self) -> bool {
        self.restart_policy
            .as_ref()
            .map(|p| p.on_exit)
            .unwrap_or(self.restart_on_exit)
    }

    fn effective_restart_backoff_ms(&self) -> u64 {
        self.restart_policy
            .as_ref()
            .map(|p| p.backoff_ms)
            .unwrap_or(self.restart_backoff_ms)
    }

    fn effective_max_restart_attempts(&self) -> u32 {
        self.restart_policy
            .as_ref()
            .map(|p| p.max_attempts)
            .unwrap_or(self.max_restart_attempts)
    }

    fn effective_healthcheck_url(&self) -> Option<&str> {
        self.healthcheck
            .as_ref()
            .map(|h| h.url.as_str())
            .or(self.healthcheck_url.as_deref())
    }

    fn effective_healthcheck_timeout_ms(&self) -> u64 {
        self.healthcheck
            .as_ref()
            .map(|h| h.timeout_ms)
            .unwrap_or(self.healthcheck_timeout_ms)
    }

    fn effective_healthcheck_retries(&self) -> u8 {
        self.healthcheck
            .as_ref()
            .map(|h| h.retries)
            .unwrap_or(default_healthcheck_retries())
    }

    fn effective_healthcheck_interval_ms(&self) -> u64 {
        self.healthcheck
            .as_ref()
            .map(|h| h.interval_ms)
            .unwrap_or(default_healthcheck_interval_ms())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing("krab_orchestrator");

    info!("Starting Krab Orchestrator...");

    let settings = Config::builder()
        .add_source(config::File::with_name("krab"))
        .build();

    match settings {
        Ok(settings) => match settings.try_deserialize::<KrabConfig>() {
            Ok(config) => {
                info!("Loaded configuration for services: {:?}", config.services.keys());
                run_supervisor(config).await?;
            }
            Err(e) => {
                error!("Failed to parse krab.toml: {}", e);
            }
        },
        Err(e) => {
            error!("Failed to load krab.toml: {}", e);
            info!("Ensure krab.toml exists in the current directory.");
        }
    }

    Ok(())
}

async fn run_supervisor(config: KrabConfig) -> Result<()> {
    let mut children = HashMap::<String, tokio::process::Child>::new();
    let mut restart_attempts = HashMap::<String, u32>::new();

    let startup_order = resolve_startup_order(&config.services)?;
    info!(order = ?startup_order, "startup_order_resolved");

    for name in startup_order {
        let Some(service) = config.services.get(&name) else {
            continue;
        };
        match spawn_service(&name, service).await {
            Ok(child) => {
                children.insert(name.clone(), child);
                restart_attempts.insert(name.clone(), 0);
                if let Err(err) = wait_for_service_health(&name, service).await {
                    warn!(service = %name, error = %err, "healthcheck_after_start_failed");
                }
            }
            Err(err) => {
                error!(service = %name, error = %err, "service_spawn_failed");
            }
        }
    }

    let watch_cfg = config.watch.clone().unwrap_or(WatchConfig {
        enabled: false,
        poll_ms: default_poll_ms(),
        paths: vec![],
    });

    if !watch_cfg.enabled {
        info!("Watch mode disabled; enabling exit supervision loop.");
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("shutdown_signal_received");
                    shutdown_children(&mut children).await;
                    return Ok(());
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    supervise_exited_children(&config, &mut children, &mut restart_attempts).await;
                }
            }
        }
    }

    let mut fingerprint = watch_fingerprint(&watch_cfg.paths)?;
    info!(poll_ms = watch_cfg.poll_ms, "watch_mode_enabled");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown_signal_received");
                shutdown_children(&mut children).await;
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(watch_cfg.poll_ms)) => {
                supervise_exited_children(&config, &mut children, &mut restart_attempts).await;

                let current = watch_fingerprint(&watch_cfg.paths)?;
                if current == fingerprint {
                    continue;
                }

                fingerprint = current;
                info!("Source changes detected. Restarting watched services...");

                for (name, child) in children.iter_mut() {
                    if !config.services.get(name).map(|s| s.watch).unwrap_or(false) {
                        continue;
                    }
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }

                for (name, service) in &config.services {
                    if !service.watch {
                        continue;
                    }

                    match spawn_service(name, service).await {
                        Ok(child) => {
                            children.insert(name.clone(), child);
                            restart_attempts.insert(name.clone(), 0);
                            info!(service = %name, "service_restarted");
                            if let Err(err) = wait_for_service_health(name, service).await {
                                warn!(service = %name, error = %err, "healthcheck_after_restart_failed");
                            }
                        }
                        Err(err) => {
                            error!(service = %name, error = %err, "service_restart_failed");
                        }
                    }
                }
            }
        }
    }
}

async fn supervise_exited_children(
    config: &KrabConfig,
    children: &mut HashMap<String, tokio::process::Child>,
    restart_attempts: &mut HashMap<String, u32>,
) {
    let names: Vec<String> = children.keys().cloned().collect();
    for name in names {
        let Some(child) = children.get_mut(&name) else {
            continue;
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                warn!(service = %name, status = %status, code = ?status.code(), "service_exited");
                let service = match config.services.get(&name) {
                    Some(service) => service,
                    None => continue,
                };

                if !service.effective_restart_on_exit() {
                    continue;
                }

                let attempts = restart_attempts.get(&name).copied().unwrap_or(0);
                if attempts >= service.effective_max_restart_attempts() {
                    error!(
                        service = %name,
                        attempts,
                        max_attempts = service.effective_max_restart_attempts(),
                        "service_restart_limit_reached"
                    );
                    continue;
                }

                tokio::time::sleep(Duration::from_millis(service.effective_restart_backoff_ms())).await;
                match spawn_service(&name, service).await {
                    Ok(new_child) => {
                        children.insert(name.clone(), new_child);
                        let next_attempt = attempts + 1;
                        restart_attempts.insert(name.clone(), next_attempt);
                        info!(service = %name, attempt = next_attempt, "service_auto_restarted");
                        if let Err(err) = wait_for_service_health(&name, service).await {
                            warn!(service = %name, error = %err, "healthcheck_after_auto_restart_failed");
                        }
                    }
                    Err(err) => {
                        error!(service = %name, error = %err, "service_auto_restart_failed");
                    }
                }
            }
            Ok(None) => {}
            Err(err) => {
                error!(service = %name, error = %err, "service_try_wait_failed");
            }
        }
    }
}

async fn shutdown_children(children: &mut HashMap<String, tokio::process::Child>) {
    for (name, child) in children.iter_mut() {
        let _ = child.kill().await;
        let _ = child.wait().await;
        info!(service = %name, "service_stopped");
    }
}

async fn spawn_service(name: &str, service: &ServiceDefinition) -> Result<tokio::process::Child> {
    let mut cmd = Command::new(&service.command);
    cmd.args(&service.args).envs(&service.env);
    if let Some(cwd) = &service.cwd {
        cmd.current_dir(cwd);
    }

    let child = cmd.spawn()?;
    info!(service = %name, pid = ?child.id(), "service_started");
    Ok(child)
}

async fn wait_for_service_health(name: &str, service: &ServiceDefinition) -> Result<()> {
    let Some(url) = service.effective_healthcheck_url() else {
        return Ok(());
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(service.effective_healthcheck_timeout_ms()))
        .build()?;

    let mut circuit = CircuitBreaker::new(3, Duration::from_secs(2), 1);

    for _ in 0..service.effective_healthcheck_retries() {
        if !circuit.allow_request() {
            warn!(service = %name, url = %url, circuit_state = ?circuit.state(), "service_health_probe_blocked_by_circuit");
            tokio::time::sleep(Duration::from_millis(service.effective_healthcheck_interval_ms())).await;
            continue;
        }

        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                circuit.record_success();
                info!(service = %name, url = %url, "service_healthy");
                return Ok(());
            }
            Ok(resp) => {
                circuit.record_failure();
                warn!(service = %name, url = %url, status = %resp.status(), "service_unhealthy_response");
            }
            Err(err) => {
                circuit.record_failure();
                warn!(service = %name, url = %url, error = %err, "service_health_probe_failed");
            }
        }
        tokio::time::sleep(Duration::from_millis(service.effective_healthcheck_interval_ms())).await;
    }

    anyhow::bail!("service '{}' failed health check at {}", name, url)
}

fn resolve_startup_order(services: &HashMap<String, ServiceDefinition>) -> Result<Vec<String>> {
    fn visit(
        node: &str,
        services: &HashMap<String, ServiceDefinition>,
        temporary: &mut HashMap<String, bool>,
        permanent: &mut HashMap<String, bool>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if permanent.get(node).copied().unwrap_or(false) {
            return Ok(());
        }
        if temporary.get(node).copied().unwrap_or(false) {
            anyhow::bail!("dependency cycle detected at service '{}'", node);
        }

        temporary.insert(node.to_string(), true);
        let service = services
            .get(node)
            .ok_or_else(|| anyhow::anyhow!("service '{}' not found", node))?;

        for dep in service.depends_on.iter().chain(service.startup_dependencies.iter()) {
            if !services.contains_key(dep) {
                anyhow::bail!("service '{}' depends on unknown service '{}'", node, dep);
            }
            visit(dep, services, temporary, permanent, order)?;
        }

        temporary.insert(node.to_string(), false);
        permanent.insert(node.to_string(), true);
        if !order.iter().any(|s| s == node) {
            order.push(node.to_string());
        }
        Ok(())
    }

    let mut order = Vec::new();
    let mut temporary = HashMap::new();
    let mut permanent = HashMap::new();

    let mut names: Vec<String> = services.keys().cloned().collect();
    names.sort();
    for name in names {
        visit(&name, services, &mut temporary, &mut permanent, &mut order)?;
    }

    Ok(order)
}

fn watch_fingerprint(paths: &[String]) -> Result<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut files = Vec::new();
    if paths.is_empty() {
        collect_recursive(Path::new("service_auth/src"), &mut files)?;
        collect_recursive(Path::new("service_users/src"), &mut files)?;
        collect_recursive(Path::new("service_frontend/src"), &mut files)?;
        collect_recursive(Path::new("krab_client/src"), &mut files)?;
    } else {
        for p in paths {
            collect_recursive(Path::new(p), &mut files)?;
        }
    }

    files.sort();
    let mut hasher = DefaultHasher::new();
    for file in files {
        file.hash(&mut hasher);
        if let Ok(meta) = std::fs::metadata(&file) {
            if let Ok(modified) = meta.modified() {
                modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .hash(&mut hasher);
            }
            meta.len().hash(&mut hasher);
        }
    }

    Ok(hasher.finish())
}

fn collect_recursive(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect_recursive(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}
