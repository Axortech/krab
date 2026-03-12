use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

#[derive(Parser)]
#[command(name = "krab")]
#[command(about = "Krab Framework CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new krab project
    New {
        /// Name of the project
        name: String,
    },
    /// Build the full stack application
    Build {
        /// Build in release mode
        #[arg(long)]
        release: bool,
        /// Selective rebuild target
        #[arg(long, value_enum, default_value_t = BuildTarget::All)]
        target: BuildTarget,
        /// Emit richer command diagnostics
        #[arg(long)]
        diagnostics: bool,
    },
    /// Run frontend dev workflow (build + run server)
    Dev {
        /// Build in release mode
        #[arg(long)]
        release: bool,
        /// Rebuild + restart frontend server automatically on file changes
        #[arg(long)]
        watch: bool,
        /// Polling interval in milliseconds while watching
        #[arg(long, default_value_t = 800)]
        poll_ms: u64,
        /// Debounce window in milliseconds before rebuild after file changes
        #[arg(long, default_value_t = 250)]
        settle_ms: u64,
    },
    /// Watch mode (build + restart frontend on changes)
    Watch {
        /// Build in release mode
        #[arg(long)]
        release: bool,
        /// Polling interval in milliseconds
        #[arg(long, default_value_t = 800)]
        poll_ms: u64,
        /// Debounce window in milliseconds before rebuild after file changes
        #[arg(long, default_value_t = 250)]
        settle_ms: u64,
    },
    /// Generate developer workflow documentation
    Docs {
        /// Output file path
        #[arg(long, default_value = "plans/07_dev_workflow.md")]
        out: PathBuf,
    },
    /// Bootstrap full local stack in one command (build + orchestrator)
    Bootstrap {
        /// Build artifacts in release mode before starting stack
        #[arg(long)]
        release: bool,
        /// Skip build step and start orchestrator immediately
        #[arg(long)]
        skip_build: bool,
    },
    /// Validate common environment settings used by services
    EnvCheck {
        /// Fail with non-zero exit when warnings are found
        #[arg(long)]
        strict: bool,
    },
    /// Run API contract checks used by CI
    Contract {
        #[command(subcommand)]
        action: ContractAction,
    },
    /// Run database governance checks used by CI
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Generate resources
    Gen {
        #[command(subcommand)]
        resource: GenResource,
    },
}

#[derive(Subcommand)]
enum ContractAction {
    /// Run contract checks and schema snapshots
    Check {
        /// Emit richer command diagnostics
        #[arg(long)]
        diagnostics: bool,
    },
    /// Run protocol parity and resolver checks
    ProtocolCheck {
        /// Emit richer command diagnostics
        #[arg(long)]
        diagnostics: bool,
    },
}

#[derive(Subcommand)]
enum DbAction {
    /// Run migration lifecycle checks
    Lifecycle {
        /// Emit richer command diagnostics
        #[arg(long)]
        diagnostics: bool,
    },
    /// Run rollback simulation checks
    Rollback {
        /// Emit richer command diagnostics
        #[arg(long)]
        diagnostics: bool,
    },
    /// Run migration drift-detection checks
    Drift {
        /// Emit richer command diagnostics
        #[arg(long)]
        diagnostics: bool,
    },
    /// Run rollback rehearsal and capture evidence
    Rehearsal {
        /// Path for evidence output
        #[arg(long, default_value = "rollback-rehearsal-evidence.txt")]
        out: PathBuf,
        /// Emit richer command diagnostics
        #[arg(long)]
        diagnostics: bool,
    },
}

#[derive(Subcommand)]
enum GenResource {
    /// Generate a new microservice
    Service {
        /// Name of the service (e.g., service_payment)
        name: String,
        /// Type of API to generate
        #[arg(long, value_enum)]
        r#type: ServiceType,
        /// Exposure mode (single/multi)
        #[arg(long, value_enum, default_value_t = ExposureMode::Single)]
        exposure_mode: ExposureMode,
        /// Protocol set for multi mode (CSV)
        #[arg(long, value_delimiter = ',')]
        protocols: Option<Vec<ServiceType>>,
        /// Deployment topology for generated services
        #[arg(long, value_enum, default_value_t = Topology::SingleService)]
        topology: Topology,
    },
    /// Generate a new component
    Component {
        /// Name of the component
        name: String,
    },
    /// Generate a new route
    Route {
        /// Name of the route
        name: String,
    },
    /// Generate a new server function
    ServerFunction {
        /// Name of the server function
        name: String,
    },
}

#[derive(Clone, ValueEnum, Debug, PartialEq, Eq)]
enum ServiceType {
    Rest,
    Graphql,
    Rpc,
    Grpc,
}

#[derive(Clone, ValueEnum, Debug, PartialEq, Eq)]
enum ExposureMode {
    Single,
    Multi,
}

#[derive(Clone, ValueEnum, Debug, PartialEq, Eq)]
enum Topology {
    SingleService,
    SplitServices,
}

#[derive(Clone, ValueEnum, Debug)]
enum BuildTarget {
    All,
    Frontend,
    Client,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Build {
            release,
            target,
            diagnostics,
        } => {
            build_project(*release, target, *diagnostics)?;
        }
        Commands::Dev {
            release,
            watch,
            poll_ms,
            settle_ms,
        } => {
            if *watch {
                watch_project(*release, *poll_ms, *settle_ms)?;
            } else {
                dev_project(*release)?;
            }
        }
        Commands::Watch {
            release,
            poll_ms,
            settle_ms,
        } => {
            watch_project(*release, *poll_ms, *settle_ms)?;
        }
        Commands::Docs { out } => {
            generate_docs(out)?;
        }
        Commands::Bootstrap {
            release,
            skip_build,
        } => {
            bootstrap_local_stack(*release, *skip_build)?;
        }
        Commands::EnvCheck { strict } => {
            validate_environment(*strict)?;
        }
        Commands::Contract { action } => match action {
            ContractAction::Check { diagnostics } => run_contract_checks(*diagnostics)?,
            ContractAction::ProtocolCheck { diagnostics } => {
                run_protocol_contract_checks(*diagnostics)?
            }
        },
        Commands::Db { action } => match action {
            DbAction::Lifecycle { diagnostics } => run_db_lifecycle_check(*diagnostics)?,
            DbAction::Rollback { diagnostics } => run_db_rollback_check(*diagnostics)?,
            DbAction::Drift { diagnostics } => run_db_drift_check(*diagnostics)?,
            DbAction::Rehearsal { out, diagnostics } => {
                run_db_rollback_rehearsal(out, *diagnostics)?
            }
        },
        Commands::Gen { resource } => match resource {
            GenResource::Service {
                name,
                r#type,
                exposure_mode,
                protocols,
                topology,
            } => {
                generate_service(name, r#type, exposure_mode, protocols, topology)?;
            }
            GenResource::Component { name } => {
                generate_component(name)?;
            }
            GenResource::Route { name } => {
                generate_route(name)?;
            }
            GenResource::ServerFunction { name } => {
                generate_server_function(name)?;
            }
        },
        Commands::New { name } => {
            generate_project(name)?;
        }
    }

    Ok(())
}

fn generate_service(
    name: &str,
    service_type: &ServiceType,
    exposure_mode: &ExposureMode,
    protocols: &Option<Vec<ServiceType>>,
    topology: &Topology,
) -> Result<()> {
    println!(
        "🦀 Generating service '{}' of type {:?} (mode={:?}, topology={:?})...",
        name, service_type, exposure_mode, topology
    );

    let selected_protocols = resolve_protocols(service_type, exposure_mode, protocols)?;

    if *topology == Topology::SplitServices {
        return generate_split_service_topology(name, &selected_protocols);
    }

    let path = PathBuf::from(name);
    if path.exists() {
        anyhow::bail!("Directory '{}' already exists", name);
    }

    // Create directory
    fs::create_dir(&path).context("Failed to create service directory")?;

    // Create Cargo.toml
    let mut feature_names: Vec<&str> = Vec::new();
    for proto in &selected_protocols {
        let feature = match proto {
            ServiceType::Rest => Some("rest"),
            ServiceType::Graphql => Some("graphql"),
            ServiceType::Rpc => Some("rest"),
            ServiceType::Grpc => Some("grpc"),
        };
        if let Some(feature) = feature {
            if !feature_names.contains(&feature) {
                feature_names.push(feature);
            }
        }
    }
    if feature_names.is_empty() {
        feature_names.push("rest");
    }

    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = {{ version = "1.0", features = ["full"] }}
krab_core = {{ path = "../krab_core", features = [{}] }}
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
serde = {{ version = "1.0", features = ["derive"] }}
"#,
        name,
        feature_names
            .iter()
            .map(|f| format!("\"{}\"", f))
            .collect::<Vec<String>>()
            .join(", ")
    );

    fs::write(path.join("Cargo.toml"), cargo_toml)?;

    // Create src directory
    fs::create_dir(path.join("src"))?;

    // Create src/main.rs stub
    let mut main_rs = r#"use anyhow::Result;
use krab_core::service::{ApiService, ServiceConfig};
use async_trait::async_trait;

struct Service;

#[async_trait]
impl ApiService for Service {
    async fn start(&self) -> Result<()> {
        println!("Service started!");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    println!("exposure_mode=__EXPOSURE_MODE__");
    println!("protocols=__PROTOCOLS__");
    let service = Service;
    service.start().await
}
"#
    .to_string();
    let exposure_mode_value = match exposure_mode {
        ExposureMode::Single => "single",
        ExposureMode::Multi => "multi",
    };
    let protocols_value = selected_protocols
        .iter()
        .map(protocol_label)
        .collect::<Vec<&str>>()
        .join(",");
    main_rs = main_rs
        .replace("__EXPOSURE_MODE__", exposure_mode_value)
        .replace("__PROTOCOLS__", &protocols_value);
    fs::write(path.join("src/main.rs"), main_rs)?;

    if *exposure_mode == ExposureMode::Multi {
        generate_multi_mode_layout(&path, &selected_protocols)?;
    }

    println!("✅ Service '{}' created successfully!", name);
    println!(
        "👉 Add '{}' to your workspace Cargo.toml members list.",
        name
    );

    Ok(())
}

fn resolve_protocols(
    service_type: &ServiceType,
    exposure_mode: &ExposureMode,
    protocols: &Option<Vec<ServiceType>>,
) -> Result<Vec<ServiceType>> {
    let mut selected = if *exposure_mode == ExposureMode::Single {
        vec![service_type.clone()]
    } else {
        protocols
            .clone()
            .unwrap_or_else(|| vec![service_type.clone()])
    };

    if selected.is_empty() {
        selected.push(service_type.clone());
    }

    let mut deduped = Vec::new();
    for p in selected {
        if !deduped.contains(&p) {
            deduped.push(p);
        }
    }
    Ok(deduped)
}

fn protocol_label(service_type: &ServiceType) -> &'static str {
    match service_type {
        ServiceType::Rest => "rest",
        ServiceType::Graphql => "graphql",
        ServiceType::Rpc => "rpc",
        ServiceType::Grpc => "grpc",
    }
}

fn generate_multi_mode_layout(path: &Path, selected_protocols: &[ServiceType]) -> Result<()> {
    let domain_dir = path.join("src/domain");
    let adapters_dir = path.join("src/adapters");
    fs::create_dir_all(&domain_dir)?;
    fs::create_dir_all(&adapters_dir)?;

    fs::write(
        path.join("src/capabilities.rs"),
        "pub fn build_capabilities() {}\n",
    )?;
    fs::write(
        path.join("src/domain/mod.rs"),
        "pub mod models;\npub mod service;\n",
    )?;
    fs::write(
        path.join("src/domain/models.rs"),
        "#[derive(Debug, Clone)]\npub struct DomainModel;\n",
    )?;
    fs::write(
        path.join("src/domain/service.rs"),
        "pub trait DomainService: Send + Sync {}\n",
    )?;

    let mut mod_rs = String::new();
    for protocol in selected_protocols {
        let label = protocol_label(protocol);
        let module_name = label.replace('-', "_");
        mod_rs.push_str(&format!("pub mod {};\n", module_name));
        fs::write(
            adapters_dir.join(format!("{}.rs", module_name)),
            format!("pub fn mount_{}() {{}}\n", module_name),
        )?;
    }
    if mod_rs.is_empty() {
        mod_rs.push_str("pub mod rest;\n");
        fs::write(adapters_dir.join("rest.rs"), "pub fn mount_rest() {}\n")?;
    }
    fs::write(adapters_dir.join("mod.rs"), mod_rs)?;
    Ok(())
}

fn generate_split_service_topology(name: &str, selected_protocols: &[ServiceType]) -> Result<()> {
    let domain_name = format!("{}-domain", name);
    let domain_path = PathBuf::from(&domain_name);
    if domain_path.exists() {
        anyhow::bail!("Directory '{}' already exists", domain_name);
    }

    fs::create_dir(&domain_path)?;
    fs::create_dir(domain_path.join("src"))?;
    fs::write(
        domain_path.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            domain_name
        ),
    )?;
    fs::write(
        domain_path.join("src/lib.rs"),
        "pub fn shared_domain_marker() -> &'static str { \"shared\" }\n",
    )?;

    let mut created = Vec::new();
    for protocol in selected_protocols {
        let label = protocol_label(protocol);
        let crate_name = format!("{}-{}", name, label);
        let crate_path = PathBuf::from(&crate_name);
        if crate_path.exists() {
            anyhow::bail!("Directory '{}' already exists", crate_name);
        }
        fs::create_dir(&crate_path)?;
        fs::create_dir_all(crate_path.join(format!("src/adapters/{}", label)))?;
        fs::create_dir_all(crate_path.join("src/domain"))?;
        fs::write(
            crate_path.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{} = {{ path = \"../{}\" }}\nkrab_core = {{ path = \"../krab_core\", features = [\"rest\"] }}\n",
                crate_name, domain_name, domain_name
            ),
        )?;
        fs::write(
            crate_path.join("src/main.rs"),
            format!("fn main() {{ println!(\"{} adapter service\"); }}\n", label),
        )?;
        fs::write(crate_path.join("src/domain/mod.rs"), "pub use crate::*;\n")?;
        fs::write(
            crate_path.join(format!("src/adapters/{}/mod.rs", label)),
            format!("pub fn mount_{}() {{}}\n", label.replace('-', "_")),
        )?;
        fs::write(
            crate_path.join("src/adapters/mod.rs"),
            format!("pub mod {};\n", label.replace('-', "_")),
        )?;
        created.push(crate_name);
    }

    println!(
        "✅ Split topology generated with shared domain crate: {}",
        domain_name
    );
    println!("👉 Generated protocol crates: {}", created.join(", "));
    Ok(())
}

fn generate_component(name: &str) -> Result<()> {
    println!("🦀 Generating component '{}'...", name);
    let path = PathBuf::from(format!("src/components/{}.rs", name.to_lowercase()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = format!(
        r#"use krab_core::prelude::*;

#[component]
pub fn {}() -> impl IntoView {{
    view! {{
        <div class="{}">
            "We are crabs"
        </div>
    }}
}}
"#,
        name,
        name.to_lowercase()
    );
    fs::write(&path, content)?;
    println!("✅ Component '{}' created at {:?}", name, path);
    Ok(())
}

fn generate_route(name: &str) -> Result<()> {
    println!("🦀 Generating route '{}'...", name);
    let path = PathBuf::from(format!("src/routes/{}.rs", name.to_lowercase()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = format!(
        r#"use krab_core::prelude::*;

#[route(path = "/{}")]
pub fn {}() -> impl IntoView {{
    view! {{
        <div>
            "Route: {}"
        </div>
    }}
}}
"#,
        name.to_lowercase(),
        name,
        name
    );
    fs::write(&path, content)?;
    println!("✅ Route '{}' created at {:?}", name, path);
    Ok(())
}

fn generate_server_function(name: &str) -> Result<()> {
    println!("🦀 Generating server function '{}'...", name);
    let path = PathBuf::from(format!("src/server_functions/{}.rs", name.to_lowercase()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = format!(
        r#"use krab_core::prelude::*;
use serde::{{Serialize, Deserialize}};

#[server(endpoint = "/api/{}")]
pub async fn {}() -> Result<String, ServerFnError> {{
    Ok("Hello from server".to_string())
}}
"#,
        name.to_lowercase(),
        name
    );
    fs::write(&path, content)?;
    println!("✅ Server function '{}' created at {:?}", name, path);
    Ok(())
}

fn generate_project(name: &str) -> Result<()> {
    println!("🦀 Scaffolding new Krab project '{}'...", name);
    let path = PathBuf::from(name);
    if path.exists() {
        anyhow::bail!("Directory '{}' already exists", name);
    }
    fs::create_dir(&path)?;
    fs::create_dir(path.join("src"))?;

    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
krab_core = "0.1.0"
tokio = {{ version = "1.0", features = ["full"] }}
"#,
        name
    );

    fs::write(path.join("Cargo.toml"), cargo_toml)?;

    let main_rs = r#"use krab_core::prelude::*;

fn main() {
    println!("Hello, Krab!");
}
"#;
    fs::write(path.join("src/main.rs"), main_rs)?;
    println!("✅ Project '{}' created successfully!", name);
    Ok(())
}

fn build_project(release: bool, target: &BuildTarget, diagnostics: bool) -> Result<()> {
    println!("🦀 Building Krab Project...");

    if matches!(target, BuildTarget::All | BuildTarget::Frontend) {
        println!("   > Building Frontend Service...");
        let mut server_cmd = Command::new("cargo");
        server_cmd.arg("build").arg("--bin").arg("service_frontend");
        if release {
            server_cmd.arg("--release");
        }
        run_command_logged(
            "cargo build --bin service_frontend",
            &mut server_cmd,
            diagnostics,
        )?;
    }

    if matches!(target, BuildTarget::All | BuildTarget::Client) {
        println!("   > Building Client (WASM)...");
        let mut client_cmd = Command::new("cargo");
        client_cmd
            .arg("build")
            .arg("-p")
            .arg("krab_client")
            .arg("--target")
            .arg("wasm32-unknown-unknown");

        if release {
            client_cmd.arg("--release");
        }

        run_command_logged(
            "cargo build -p krab_client --target wasm32-unknown-unknown",
            &mut client_cmd,
            diagnostics,
        )?;

        // Run wasm-bindgen
        println!("   > Generatings JS Bindings...");
        let target_dir = PathBuf::from("target/wasm32-unknown-unknown");
        let mode = if release { "release" } else { "debug" };
        let wasm_path = target_dir.join(mode).join("krab_client.wasm");
        let out_dir = PathBuf::from("dist");

        if !wasm_path.exists() {
            anyhow::bail!("WASM file not found at: {:?}", wasm_path);
        }

        if !out_dir.exists() {
            std::fs::create_dir_all(&out_dir).context("Failed to create dist directory")?;
        }

        let mut bindgen_cmd = Command::new("wasm-bindgen");
        bindgen_cmd
            .arg(&wasm_path)
            .arg("--out-dir")
            .arg(&out_dir)
            .arg("--target")
            .arg("web")
            .arg("--no-typescript");

        match bindgen_cmd.status() {
            Ok(status) => {
                if !status.success() {
                    anyhow::bail!("wasm-bindgen failed. Make sure it is installed.");
                }
            }
            Err(_) => {
                println!("⚠️ wasm-bindgen not found. Skipping JS generation.");
            }
        }

        fingerprint_assets(&out_dir)?;
    }

    println!("✅ Build Complete! Assets are in ./dist");
    Ok(())
}

fn dev_project(release: bool) -> Result<()> {
    println!("🧪 Running Dev Workflow...");
    build_project(release, &BuildTarget::All, false)?;

    println!("   > Starting service_frontend...");
    let status = Command::new("cargo")
        .arg("run")
        .arg("--bin")
        .arg("service_frontend")
        .status()
        .context("Failed to run service_frontend")?;

    if !status.success() {
        anyhow::bail!("service_frontend exited with non-zero status");
    }

    Ok(())
}

fn watch_project(release: bool, poll_ms: u64, settle_ms: u64) -> Result<()> {
    println!("👀 Starting watch workflow (HMR-style restart)...");
    println!("   > Poll interval: {}ms", poll_ms);
    println!("   > Debounce settle: {}ms", settle_ms);

    let mut baseline = collect_file_fingerprints()?;
    build_project(release, &BuildTarget::All, false)?;

    let mut child = spawn_frontend(release)?;
    let mut pending_change_since: Option<std::time::Instant> = None;

    loop {
        std::thread::sleep(Duration::from_millis(poll_ms));

        if let Some(status) = child
            .try_wait()
            .context("Failed checking frontend process state")?
        {
            eprintln!("⚠️ service_frontend exited ({status}). Restarting...");
            child = spawn_frontend(release)?;
        }

        let next = collect_file_fingerprints()?;
        if next == baseline {
            pending_change_since = None;
            continue;
        }

        if pending_change_since.is_none() {
            pending_change_since = Some(std::time::Instant::now());
            continue;
        }

        if pending_change_since
            .map(|t| t.elapsed() < Duration::from_millis(settle_ms))
            .unwrap_or(false)
        {
            continue;
        }

        // Analyze what changed
        let mut client_changed = false;
        let mut server_changed = false;
        let mut public_changed = false;

        for (path, hash) in &next {
            if baseline.get(path) != Some(hash) {
                let path_str = path.to_string_lossy();
                if path_str.contains("krab_client") {
                    client_changed = true;
                } else if path_str.contains("service_frontend") && !path_str.contains("public") {
                    server_changed = true;
                } else if path_str.contains("public") {
                    public_changed = true;
                } else if path_str.contains("krab_core") || path_str.contains("krab_macros") {
                    client_changed = true;
                    server_changed = true;
                }
            }
        }
        for path in baseline.keys() {
            if !next.contains_key(path) {
                let path_str = path.to_string_lossy();
                if path_str.contains("krab_client") {
                    client_changed = true;
                } else if path_str.contains("service_frontend") && !path_str.contains("public") {
                    server_changed = true;
                } else if path_str.contains("public") {
                    public_changed = true;
                } else if path_str.contains("krab_core") || path_str.contains("krab_macros") {
                    client_changed = true;
                    server_changed = true;
                }
            }
        }

        println!("   > Change detected (settled). Rebuilding...");
        if client_changed && !server_changed {
            println!("   > ⚡ Partial invalidation: Client only");
            if let Err(err) = build_project(release, &BuildTarget::Client, false) {
                eprintln!("⚠️ Client rebuild failed: {err}");
                baseline = next;
                pending_change_since = None;
                continue;
            }
        } else if server_changed {
            println!("   > ⚡ Partial invalidation: Server/Full");
            let _ = child.kill();
            let _ = child.wait();

            let target = if client_changed {
                BuildTarget::All
            } else {
                BuildTarget::Frontend
            };
            if let Err(err) = build_project(release, &target, false) {
                eprintln!("⚠️ Rebuild failed: {err}");
                baseline = next;
                pending_change_since = None;
                continue;
            }
            child = spawn_frontend(release)?;
        } else if public_changed {
            println!("   > ⚡ Partial invalidation: Public assets only (No rebuild)");
        }

        // Write HMR signal file
        let dist_dir = PathBuf::from("dist");
        if dist_dir.exists() {
            let _ = fs::write(
                dist_dir.join(".hmr_signal"),
                format!(
                    "{}",
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_millis()
                ),
            );
        }

        baseline = next;
        pending_change_since = None;
    }
}

fn run_command_logged(label: &str, cmd: &mut Command, diagnostics: bool) -> Result<()> {
    if diagnostics {
        println!("   > Running: {label}");
    }
    let started = std::time::Instant::now();
    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute command: {label}"))?;
    if diagnostics {
        println!(
            "   > Finished: {label} (status: {}, elapsed: {}ms)",
            status,
            started.elapsed().as_millis()
        );
    }
    if !status.success() {
        anyhow::bail!("Command failed: {label}");
    }
    Ok(())
}

fn bootstrap_local_stack(release: bool, skip_build: bool) -> Result<()> {
    println!("🚀 Bootstrapping local Krab stack...");
    if !skip_build {
        build_project(release, &BuildTarget::All, true)?;
    }

    let status = Command::new("cargo")
        .arg("run")
        .arg("--bin")
        .arg("krab_orchestrator")
        .status()
        .context("Failed to run krab_orchestrator")?;

    if !status.success() {
        anyhow::bail!("krab_orchestrator exited with non-zero status");
    }
    Ok(())
}

fn validate_environment(strict: bool) -> Result<()> {
    let mut warnings = Vec::new();

    let auth_mode = std::env::var("KRAB_AUTH_MODE").unwrap_or_else(|_| "jwt".to_string());
    if auth_mode.eq_ignore_ascii_case("jwt") || auth_mode.eq_ignore_ascii_case("oidc") {
        if std::env::var("KRAB_OIDC_ISSUER").is_err() {
            warnings.push("KRAB_OIDC_ISSUER is required when KRAB_AUTH_MODE=jwt".to_string());
        }
        if std::env::var("KRAB_OIDC_AUDIENCE").is_err() {
            warnings.push("KRAB_OIDC_AUDIENCE is required when KRAB_AUTH_MODE=jwt".to_string());
        }
    } else if auth_mode.eq_ignore_ascii_case("static") {
        let env_name = std::env::var("KRAB_ENVIRONMENT").unwrap_or_else(|_| "dev".to_string());
        if !env_name.eq_ignore_ascii_case("local") && !env_name.eq_ignore_ascii_case("dev") {
            warnings.push(
                "KRAB_AUTH_MODE=static is forbidden outside local/dev; use jwt or oidc".to_string(),
            );
        }
    } else {
        warnings.push(format!(
            "Unsupported KRAB_AUTH_MODE='{}'; expected static|jwt|oidc",
            auth_mode
        ));
    }

    let env_name = std::env::var("KRAB_ENVIRONMENT").unwrap_or_else(|_| "dev".to_string());
    if !["local", "dev", "staging", "prod"].contains(&env_name.as_str()) {
        warnings.push(format!(
            "KRAB_ENVIRONMENT should be one of local|dev|staging|prod, found: {}",
            env_name
        ));
    }

    if warnings.is_empty() {
        println!("✅ Environment validation passed");
        return Ok(());
    }

    for warning in &warnings {
        eprintln!("⚠️ {warning}");
    }

    if strict {
        anyhow::bail!("Environment validation failed in strict mode");
    }

    Ok(())
}

fn run_contract_checks(diagnostics: bool) -> Result<()> {
    println!("📜 Running API contract checks...");

    run_command_logged(
        "krab_core API envelope contract tests",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("krab_core")
            .arg("--features")
            .arg("rest")
            .arg("api_tests"),
        diagnostics,
    )?;

    run_command_logged(
        "service_auth contract tests",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("service_auth")
            .arg("contract_"),
        diagnostics,
    )?;

    run_command_logged(
        "service_users contract tests",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("service_users")
            .arg("contract_"),
        diagnostics,
    )?;

    println!("✅ API contract checks passed");
    Ok(())
}

fn run_protocol_contract_checks(diagnostics: bool) -> Result<()> {
    println!("🧪 Running protocol contract checks...");

    run_command_logged(
        "service_users parity tests",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("service_users")
            .arg("parity_"),
        diagnostics,
    )?;

    run_command_logged(
        "krab_core protocol tests",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("krab_core")
            .arg("--features")
            .arg("rest")
            .arg("protocol"),
        diagnostics,
    )?;

    run_split_topology_gateway_conflict_check()?;
    run_protocol_version_compatibility_check()?;

    println!("✅ Protocol contract checks passed");
    Ok(())
}

fn run_split_topology_gateway_conflict_check() -> Result<()> {
    let mapping_path =
        PathBuf::from("services/service_users/contracts/users_gateway_upstreams_v1.json");
    let raw = fs::read_to_string(&mapping_path)
        .with_context(|| format!("failed reading {}", mapping_path.display()))?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid json in {}", mapping_path.display()))?;

    let routes = parsed
        .get("routes")
        .or_else(|| parsed.get("upstreams"))
        .and_then(|v| v.as_array())
        .context("gateway contract missing 'routes' (or legacy 'upstreams') array")?;

    let mut seen = std::collections::BTreeSet::new();
    for route in routes {
        let upstream = route
            .get("upstream")
            .or_else(|| route.get("service"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let path_prefix = route
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let path_exact = route
            .get("path_exact")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let key = format!("upstream={upstream}|prefix={path_prefix}|exact={path_exact}");
        if !seen.insert(key.clone()) {
            anyhow::bail!("duplicate upstream mapping detected: {key}");
        }
    }
    Ok(())
}

fn run_protocol_version_compatibility_check() -> Result<()> {
    for bin in [
        "services/service_users/src/bin/users_rest.rs",
        "services/service_users/src/bin/users_graphql.rs",
        "services/service_users/src/bin/users_rpc.rs",
    ] {
        if !PathBuf::from(bin).exists() {
            anyhow::bail!("missing expected protocol split binary source: {bin}");
        }
    }
    Ok(())
}

fn run_db_lifecycle_check(diagnostics: bool) -> Result<()> {
    run_command_logged(
        "db migration lifecycle checks",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("krab_core")
            .arg("--features")
            .arg("db rest")
            .arg("test_migration_lifecycle"),
        diagnostics,
    )
}

fn run_db_rollback_check(diagnostics: bool) -> Result<()> {
    run_command_logged(
        "db rollback checks",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("krab_core")
            .arg("--features")
            .arg("db rest")
            .arg("test_migration_rollback"),
        diagnostics,
    )
}

fn run_db_drift_check(diagnostics: bool) -> Result<()> {
    run_command_logged(
        "db drift checks",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("krab_core")
            .arg("--features")
            .arg("db rest")
            .arg("test_drift_detection"),
        diagnostics,
    )
}

fn run_db_rollback_rehearsal(out: &PathBuf, diagnostics: bool) -> Result<()> {
    run_command_logged(
        "db rollback rehearsal test",
        Command::new("cargo")
            .arg("test")
            .arg("--package")
            .arg("krab_core")
            .arg("--features")
            .arg("db rest")
            .arg("test_migration_rollback")
            .arg("--")
            .arg("--nocapture"),
        diagnostics,
    )?;

    let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "local".to_string());
    let sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_string());
    let git_ref = std::env::var("GITHUB_REF").unwrap_or_else(|_| "local".to_string());
    let timestamp = chrono_like_utc_now();

    let mut evidence = String::new();
    evidence.push_str("rollback_rehearsal: ok\n");
    evidence.push_str(&format!("run_id: {}\n", run_id));
    evidence.push_str(&format!("sha: {}\n", sha));
    evidence.push_str(&format!("ref: {}\n", git_ref));
    evidence.push_str(&format!("timestamp: {}\n", timestamp));

    fs::write(out, evidence)
        .with_context(|| format!("Failed to write rollback evidence to {}", out.display()))?;

    println!("✅ Wrote rollback rehearsal evidence to {}", out.display());
    Ok(())
}

fn chrono_like_utc_now() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0));
    format!("{}", now.as_secs())
}

fn spawn_frontend(release: bool) -> Result<std::process::Child> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run").arg("--bin").arg("service_frontend");
    if release {
        cmd.arg("--release");
    }
    cmd.spawn()
        .context("Failed to start service_frontend process")
}

fn collect_file_fingerprints() -> Result<std::collections::HashMap<PathBuf, u64>> {
    let mut files = Vec::new();
    for root in [
        "crates/framework/krab_core/src",
        "services/service_frontend/src",
        "crates/framework/krab_client/src",
        "crates/framework/krab_macros/src",
        "services/service_frontend/public",
    ] {
        collect_files_recursive(PathBuf::from(root).as_path(), &mut files)?;
    }

    let mut map = std::collections::HashMap::new();
    for file in files {
        let mut hasher = DefaultHasher::new();
        if let Ok(meta) = fs::metadata(&file) {
            if let Ok(modified) = meta.modified() {
                let millis = modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                millis.hash(&mut hasher);
            }
            meta.len().hash(&mut hasher);
        }
        map.insert(file, hasher.finish());
    }
    Ok(map)
}

fn collect_files_recursive(path: &std::path::Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(path).with_context(|| format!("Failed to read directory {path:?}"))? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect_files_recursive(&p, out)?;
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if ["rs", "html", "css", "js", "json", "wasm"].contains(&ext) {
                out.push(p);
            }
        }
    }

    Ok(())
}

fn generate_docs(out: &PathBuf) -> Result<()> {
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create documentation directory {parent:?}"))?;
        }
    }

    let mut command_matrix = BTreeMap::new();
    command_matrix.insert(
        "krab build [--release]",
        "Build frontend + WASM + bindgen + fingerprinted assets",
    );
    command_matrix.insert(
        "krab dev [--release]",
        "Build once and run service_frontend",
    );
    command_matrix.insert(
        "krab dev --watch [--release] [--poll-ms <n>] [--settle-ms <n>]",
        "HMR-style loop: debounce file changes, rebuild, restart frontend",
    );
    command_matrix.insert(
        "krab watch [--release] [--poll-ms <n>] [--settle-ms <n>]",
        "Alias dedicated to watch workflow",
    );
    command_matrix.insert(
        "krab docs [--out <path>]",
        "Regenerate this developer workflow document",
    );

    let mut rows = String::new();
    for (cmd, desc) in command_matrix {
        rows.push_str(&format!("| `{}` | {} |\n", cmd, desc));
    }

    let content = format!(
        "# Dev Workflow and Build Outputs\n\n## CLI Commands\n\n| Command | Description |\n|---|---|\n{}\n## Asset Fingerprinting\n\nThe CLI computes deterministic hashes and writes `dist/assets.json`.\n\n## Watch/HMR Workflow\n\n`krab dev --watch` (or `krab watch`) performs incremental change detection over Rust/frontend sources, rebuilds artifacts, and restarts `service_frontend`.\n\n## Orchestrator Integration\n\nPair with `krab_orchestrator` for multi-service restarts from `krab.toml`.\n",
        rows
    );

    fs::write(out, content).with_context(|| format!("Failed to write docs file {out:?}"))?;
    println!("✅ Wrote workflow docs to {}", out.display());
    Ok(())
}

fn fingerprint_assets(out_dir: &Path) -> Result<()> {
    let mut manifest_entries = Vec::new();

    for file_name in ["krab_client.js", "krab_client_bg.wasm"] {
        let input = out_dir.join(file_name);
        if !input.exists() {
            continue;
        }

        let bytes = fs::read(&input).with_context(|| format!("Failed to read {:?}", input))?;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let digest = format!("{:016x}", hasher.finish());

        let ext = input
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        let stem = input
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("asset");

        let output_name = format!("{}.{}.{}", stem, &digest[..8], ext);
        let output = out_dir.join(&output_name);
        fs::copy(&input, &output)
            .with_context(|| format!("Failed to copy {:?} -> {:?}", input, output))?;

        manifest_entries.push(format!(
            "\"{}\":{{\"source\":\"{}\",\"fingerprinted\":\"{}\"}}",
            file_name, file_name, output_name
        ));
    }

    let manifest = format!("{{{}}}\n", manifest_entries.join(","));
    fs::write(out_dir.join("assets.json"), manifest).context("Failed to write assets.json")?;

    Ok(())
}
