# Pure Rust Package & Plugin Management Strategy

## 1. Philosophy: "Cargo is All You Need"
The prevailing complexity in modern web development stems from managing two distinct ecosystems: Rust (Cargo) for the backend and Node.js (npm/yarn/pnpm) for the frontend. Krab eliminates this dichotomy. 

**The Golden Rule:** If it can't be done via `Cargo.toml` or a Rust binary, it's not in the core workflow.

## 2. Removing Node.js: The Asset Strategy
We replace the typical Webpack/Vite/PostCSS pipeline with a highly optimized, parallelized Rust-native build system integrated directly into the Krab CLI.

### A. CSS Processing (`lightningcss`)
Instead of PostCSS or Sass binaries, Krab integrates **[lightningcss](https://github.com/parcel-bundler/lightningcss)** (formerly `parcel-css`).
- **Why?** It is written in Rust, extremely fast, and handles minification, vendor prefixing, and syntax lowering (nesting, custom media queries) out of the box.
- **Workflow:**
    1.  Krab CLI watches `.css` files.
    2.  On change, it runs `lightningcss` via library bindings (no external process).
    3.  Outputs optimized CSS to the `dist` folder.
- **Sass Support?** Optional support via **[grass](https://github.com/connorskees/grass)**, a pure Rust Sass compiler, enabled via a feature flag.

### B. JavaScript & Frontend Libraries
Without NPM, how do we handle external JS libraries (e.g., Leaflet, generic analytics)?
- **Philosophy:** Logic belongs in Rust (WASM). JavaScript is for glue code or legacy libraries only.
- **ES Modules & Import Maps:** Krab targets modern browsers. We do not bundle JavaScript libraries into massive blobs.
    - User puts JS files in `assets/js`.
    - Krab generates an [Import Map](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/script/type/importmap) pointing to these files or public CDNs (e.g., `esm.sh` or `unpkg`).
- **"Vendoring":** If a user wants to host dependencies locally, they can define them in `Cargo.toml` metadata (see below), and Krab CLI will download and vendor them into `dist/vendor` at build time.

### C. Icons & Assets
- **Icons:** A `krab-icons` crate (similar to `icondata`) providing compile-time optimized SVG icons as Rust components. No runtime fetching or SVG parsing.
- **Images:** Image optimization using the **[image](https://github.com/image-rs/image)** crate to generate WebP/AVIF variants at build time.

## 3. Configuration: `Cargo.toml` Metadata
We avoid `krab.json` or `krab.config.js`. All configuration lives in `Cargo.toml` under `[package.metadata.krab]`.

```toml
[package]
name = "my-krab-app"
version = "0.1.0"

# Core dependencies
[dependencies]
krab = "0.1"
krab-plugin-auth = "0.1"

# Krab Configuration
[package.metadata.krab]
# Server settings
port = 3000
host = "127.0.0.1"

# Asset Configuration
[package.metadata.krab.assets]
css_entry = "styles/main.css"
minify = true
# Optional: Vendor JS libraries without NPM
vendor_js = [
    { name = "chartjs", url = "https://cdn.jsdelivr.net/npm/chart.js", version = "4.4.0" }
]

# Plugin Configuration (Type-safe mapping)
[package.metadata.krab.plugins.auth]
provider = "google"
redirect_url = "/auth/callback"
```

## 4. The Plugin System
Plugins in Krab are just **Rust Crates**. There is no complex "Plugin API" that requires runtime loading of dynamic libraries. We leverage Rust's compilation model and the Trait system.

### A. The `KrabPlugin` Trait
Plugins implement a trait that allows them to hook into the application lifecycle (Server Start, Request Handling, HTML Injection).

```rust
// In the plugin crate (e.g., krab-plugin-analytics)
pub struct AnalyticsPlugin {
    pub tracking_id: String,
}

impl KrabPlugin for AnalyticsPlugin {
    fn on_app_init(&self, config: &mut AppConfig) {
        // Modify global config
    }

    fn inject_head(&self) -> String {
        format!("<script src='https://analytics.com?id={}'></script>", self.tracking_id)
    }
}
```

### B. Registration
Users register plugins in their main application entry point (usually `src/main.rs`).

```rust
#[tokio::main]
async fn main() {
    let app = Krab::new()
        .register(krab_plugin_analytics::new("UA-12345"))
        .register(krab_plugin_auth::new());

    app.run().await;
}
```

### C. Build-Time Plugins (Macros)
For plugins that need to generate code (e.g., inspecting routes, generating sitemaps), we use **Procedural Macros**.
- Example: `#[derive(KrabRoute)]` or `krab::generate_sitemap!()`.
- Since plugins are standard crates, they can include a `build.rs` or proc-macros to run logic at compile time.

## 5. Summary of Changes to Architecture
1.  **Deletion of Node.js Subsystem:** The architecture diagram will no longer show a parallel Node process for bundling.
2.  **CLI Responsibilities:** The `krab` CLI takes over the role of "Bundler". It orchestrates `cargo build` for the backend, `cargo build --target wasm32...` for the frontend, and `lightningcss` for styles.
3.  **Single Binary Output:** The final production build produces a single binary + a static `assets` folder. No `node_modules`.

## 6. Risk Assessment
- **Tailwind CSS:** The official Tailwind CLI is a binary, but integration is loose. 
    - *Mitigation:* We will support a "utility-first" engine written in Rust (like `railwind`) or allow the user to provide a path to the Tailwind CLI binary if they strictly need official compatibility. Default recommendation is standard CSS/Modules powered by `lightningcss`.
- **Complex JS Ecosystem:** React/Vue libraries won't work easily.
    - *Mitigation:* This is a feature, not a bug. We are promoting a Rust/WASM ecosystem. Wrappers will emerge for essential libs (Leaflet, etc.).
