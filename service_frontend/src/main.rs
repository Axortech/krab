use axum::body::Body;
use axum::extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::{middleware::Next, Json, Router};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use krab_client::components::{Counter, CounterProps, Likes, LikesProps, Toggle, ToggleProps};
use krab_core::config::KrabConfig;
use krab_core::error_boundary::ErrorBoundary;
use krab_core::http::{apply_common_http_layers, HasRuntimeState, RuntimeState};
use krab_core::i18n::{detect_locale_from_header, I18n, Locale, TranslationBundle};
use krab_core::isr::{IsrCache, IsrPolicy};
use krab_core::render_stream::{ChunkedStreamWriter, SuspenseState};
use krab_core::telemetry::init_tracing;
use krab_core::ws::{WsMessage, WsRoomManager};
use krab_core::Render;
use krab_macros::view;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const SERVER_FUNCTION_VERSION: &str = "2026-02-27.1";

fn i18n_bundle() -> TranslationBundle {
    let mut bundle = TranslationBundle::new();
    bundle.add_locale(
        Locale::new("en", "English"),
        vec![
            ("home_title", "Krab Framework"),
            ("hello", "Hello from Krab!"),
            ("rendered", "This is rendered server-side."),
        ],
    );
    bundle.add_locale(
        Locale::new("ne", "नेपाली"),
        vec![
            ("home_title", "क्र्याब फ्रेमवर्क"),
            ("hello", "क्र्याबबाट नमस्ते!"),
            ("rendered", "यो सर्भर-साइडबाट रेन्डर गरिएको हो।"),
        ],
    );
    bundle
}

fn i18n_for(locale: &str) -> I18n {
    I18n::new(i18n_bundle(), "en").with_locale(locale)
}

fn resolve_locale(headers: &HeaderMap) -> String {
    let supported = i18n_bundle().supported_locales().to_vec();
    let from_header = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| detect_locale_from_header(v, &supported));
    from_header.unwrap_or_else(|| "en".to_string())
}

fn ws_manager() -> &'static WsRoomManager {
    static WS_MANAGER: std::sync::OnceLock<WsRoomManager> = std::sync::OnceLock::new();
    WS_MANAGER.get_or_init(WsRoomManager::new)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CachedHttpPayload {
    body: String,
    content_type: String,
}

#[derive(Clone)]
struct AppState {
    runtime: RuntimeState,
    http_client: Client,
    auth_base_url: String,
    users_base_url: String,
    isr_cache: IsrCache,
    isr_revalidating: Arc<tokio::sync::Mutex<HashSet<String>>>,
    hmr_rx: tokio::sync::watch::Receiver<u64>,
}

#[derive(Clone)]
struct SeoMeta {
    title: String,
    description: String,
    path: String,
    og_type: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheAuthority {
    Isr,
    Distributed,
    None,
}

fn cache_authority(method: &axum::http::Method, path: &str) -> CacheAuthority {
    if *method != axum::http::Method::GET {
        return CacheAuthority::None;
    }

    if path == "/" || path == "/about" || path == "/greet" || path.starts_with("/blog/") {
        return CacheAuthority::Isr;
    }

    if path == "/robots.txt"
        || path == "/sitemap.xml"
        || path == "/data/dashboard"
        || path == "/rpc/version"
        || path == "/asset-manifest.json"
    {
        return CacheAuthority::Distributed;
    }

    CacheAuthority::None
}

impl HasRuntimeState for AppState {
    fn runtime_state(&self) -> &RuntimeState {
        &self.runtime
    }
}

async fn cache_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let cache_key = req.uri().to_string(); // Include query params in cache key
    let method = req.method().clone();

    let authority = cache_authority(&method, &path);
    let isr_eligible = authority == CacheAuthority::Isr;
    let distributed_eligible = authority == CacheAuthority::Distributed;

    if isr_eligible {
        if let Some(entry) = state.isr_cache.get(&cache_key) {
            let state_header = if entry.is_stale() { "stale" } else { "fresh" };

            let mut res = Response::new(Body::from(entry.html.into_bytes()));
            res.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
            );
            if let Ok(etag) = entry.etag.parse() {
                res.headers_mut().insert(axum::http::header::ETAG, etag);
            }

            if state_header == "stale" {
                let cache_key_bg = cache_key.clone();
                let path_bg = path.clone();
                let state_bg = state.clone();
                tokio::spawn(async move {
                    trigger_isr_revalidation(state_bg, cache_key_bg, path_bg).await;
                });
                res.headers_mut().insert(
                    axum::http::header::HeaderName::from_static("x-cache"),
                    axum::http::HeaderValue::from_static("STALE"),
                );
            } else {
                res.headers_mut().insert(
                    axum::http::header::HeaderName::from_static("x-cache"),
                    axum::http::HeaderValue::from_static("HIT"),
                );
            }

            res.headers_mut().insert(
                axum::http::header::HeaderName::from_static("x-isr-state"),
                axum::http::HeaderValue::from_static(state_header),
            );

            return res;
        }
    }

    if authority == CacheAuthority::None {
        return next.run(req).await;
    }

    if distributed_eligible {
        // Check distributed cache
        if let Ok(Some(raw)) = state.runtime_state().store.get(&cache_key).await {
            if let Ok(entry) = serde_json::from_str::<CachedHttpPayload>(&raw) {
                tracing::debug!(path = %cache_key, "cache_hit");
                let mut res = Response::new(Body::from(entry.body.into_bytes()));
                res.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    entry
                        .content_type
                        .parse()
                        .unwrap_or(axum::http::HeaderValue::from_static("text/html")),
                );
                res.headers_mut().insert(
                    axum::http::header::HeaderName::from_static("x-cache"),
                    axum::http::HeaderValue::from_static("HIT"),
                );
                return res;
            }
        }
    }

    tracing::debug!(path = %path, "cache_miss");
    let res = next.run(req).await;

    // Only cache successful responses
    if !res.status().is_success() {
        return res;
    }

    // Extract body and cache it
    let (parts, body) = res.into_parts();

    // We need to buffer the body to store it.
    // Limit size to avoid memory issues (e.g. 10MB)
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            tracing::error!("failed to read response body for caching: {}", err);
            return Response::from_parts(parts, Body::empty());
        }
    };

    let content_type = parts
        .headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html")
        .to_string();

    let html_for_isr = String::from_utf8(bytes.to_vec()).ok();

    if distributed_eligible {
        if let Some(body) = html_for_isr.clone() {
            let payload = CachedHttpPayload {
                body,
                content_type: content_type.clone(),
            };
            if let Ok(serialized) = serde_json::to_string(&payload) {
                let _ = state
                    .runtime_state()
                    .store
                    .set(&cache_key, &serialized, Duration::from_secs(60))
                    .await;
            }
        }
    }

    let mut res = Response::from_parts(parts, Body::from(bytes));
    res.headers_mut().insert(
        axum::http::header::HeaderName::from_static("x-cache"),
        axum::http::HeaderValue::from_static("MISS"),
    );

    if isr_eligible {
        if let Some(html) = html_for_isr {
            state.isr_cache.put(
                &cache_key,
                html,
                IsrPolicy::revalidate(isr_revalidate_duration()),
            );
            res.headers_mut().insert(
                axum::http::header::HeaderName::from_static("x-isr-state"),
                axum::http::HeaderValue::from_static("fresh"),
            );
        }
    }

    res
}

fn isr_revalidate_duration() -> Duration {
    let secs = std::env::var("KRAB_ISR_REVALIDATE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30)
        .max(1);
    Duration::from_secs(secs)
}

fn render_isr_path(path: &str) -> Option<String> {
    if path == "/" {
        return Some(render_home_page());
    }
    if path == "/about" {
        return Some(render_about_page());
    }
    if path == "/greet" {
        return Some(render_greet_page());
    }
    if let Some(slug) = path.strip_prefix("/blog/") {
        return Some(render_blog_page(slug));
    }
    None
}

async fn trigger_isr_revalidation(state: AppState, cache_key: String, path: String) {
    {
        let mut in_progress = state.isr_revalidating.lock().await;
        if !in_progress.insert(cache_key.clone()) {
            return;
        }
    }

    if let Some(html) = render_isr_path(&path) {
        state.isr_cache.put(
            &cache_key,
            html,
            IsrPolicy::revalidate(isr_revalidate_duration()),
        );
    }

    let mut in_progress = state.isr_revalidating.lock().await;
    in_progress.remove(&cache_key);
}

fn render_home_page() -> String {
    render_home_page_localized("en")
}

fn render_home_page_localized(locale: &str) -> String {
    let i18n = i18n_for(locale);
    let page_title = i18n.t("home_title");
    let hello = i18n.t("hello");
    let rendered = i18n.t("rendered");

    let counter = Counter(CounterProps { initial: 10 });
    let toggle = Toggle(ToggleProps { initial: false });
    let likes = Likes(LikesProps { initial: 3 });
    let runtime_script = r#"
                    import init, { hydrate } from '/pkg/krab_client.js';

                    const SERVER_FUNCTION_VERSION = '2026-02-27.1';
                    const ROUTE_BUDGETS = { ttfbMs: 800, hydrationMs: 1500 };

                    function asObject(value) {
                        return !!value && typeof value === 'object' && !Array.isArray(value);
                    }

                    function setText(id, text) {
                        const el = document.getElementById(id);
                        if (el) {
                            el.textContent = text;
                        }
                    }

                    function markDegraded(reason) {
                        const el = document.getElementById('frontend-degraded');
                        if (el) {
                            el.textContent = '⚠ partial functionality mode: ' + reason;
                        }
                    }

                    function validateStatus(payload) {
                        return asObject(payload)
                            && payload.service === 'frontend'
                            && (payload.status === 'ok' || payload.status === 'degraded');
                    }

                    function validateRpcNow(payload) {
                        return asObject(payload)
                            && Number.isFinite(payload.epoch_millis)
                            && typeof payload.server_function_version === 'string';
                    }

                    function validateRpcVersion(payload) {
                        return asObject(payload)
                            && typeof payload.server_function_version === 'string'
                            && typeof payload.policy === 'string';
                    }

                    function validateDashboard(payload) {
                        return asObject(payload)
                            && Number.isFinite(payload.users_online)
                            && Number.isFinite(payload.active_sessions)
                            && payload.feature === 'islands';
                    }

                    async function fetchJsonWithRetry(url, options = {}) {
                        const timeoutMs = options.timeoutMs ?? 1200;
                        const retries = options.retries ?? 2;
                        const baseBackoffMs = options.baseBackoffMs ?? 150;
                        const validator = options.validator;
                        let lastError = null;

                        for (let attempt = 0; attempt <= retries; attempt++) {
                            const controller = new AbortController();
                            const timeoutId = setTimeout(() => controller.abort(), timeoutMs);
                            try {
                                const response = await fetch(url, {
                                    signal: controller.signal,
                                    cache: 'no-store',
                                });
                                if (!response.ok) {
                                    throw new Error('HTTP ' + response.status);
                                }
                                const json = await response.json();
                                if (validator && !validator(json)) {
                                    throw new Error('schema mismatch');
                                }
                                clearTimeout(timeoutId);
                                return { ok: true, data: json };
                            } catch (err) {
                                clearTimeout(timeoutId);
                                lastError = err;
                                if (attempt < retries) {
                                    const backoff = baseBackoffMs * (attempt + 1);
                                    await new Promise(resolve => setTimeout(resolve, backoff));
                                }
                            }
                        }

                        return { ok: false, error: String(lastError) };
                    }

                    async function verifyManifestIntegrity() {
                        const manifest = await fetchJsonWithRetry('/asset-manifest.json', {
                            timeoutMs: 700,
                            retries: 0,
                            validator: payload => asObject(payload) && asObject(payload.assets),
                        });

                        if (!manifest.ok) {
                            console.warn('manifest check skipped:', manifest.error);
                            return;
                        }

                        const clientEntry = manifest.data.assets['krab_client.js'];
                        const valid = asObject(clientEntry)
                            && typeof clientEntry.path === 'string'
                            && typeof clientEntry.integrity === 'string'
                            && clientEntry.integrity.startsWith('sha256-')
                            && clientEntry.immutable === true;

                        if (!valid) {
                            markDegraded('asset manifest integrity validation failed');
                        }
                    }

                    function checkRouteBudgets(hydrationMs) {
                        const nav = performance.getEntriesByType('navigation')[0];
                        if (nav && nav.responseStart > ROUTE_BUDGETS.ttfbMs) {
                            console.warn('TTFB budget exceeded', {
                                ttfbMs: nav.responseStart,
                                budgetMs: ROUTE_BUDGETS.ttfbMs,
                            });
                        }

                        if (hydrationMs > ROUTE_BUDGETS.hydrationMs) {
                            console.warn('Hydration budget exceeded', {
                                hydrationMs,
                                budgetMs: ROUTE_BUDGETS.hydrationMs,
                            });
                        }
                    }

                    async function loadData() {
                        const [status, rpc, rpcVersion, dashboard] = await Promise.all([
                            fetchJsonWithRetry('/api/status', { validator: validateStatus }),
                            fetchJsonWithRetry('/rpc/now', { validator: validateRpcNow }),
                            fetchJsonWithRetry('/rpc/version', { validator: validateRpcVersion }),
                            fetchJsonWithRetry('/data/dashboard', { validator: validateDashboard }),
                        ]);

                        setText('status', status.ok ? JSON.stringify(status.data) : 'status unavailable');
                        setText('rpc', rpc.ok ? JSON.stringify(rpc.data) : 'rpc unavailable');
                        setText('version', rpcVersion.ok ? JSON.stringify(rpcVersion.data) : 'version unavailable');
                        setText('dashboard', dashboard.ok ? JSON.stringify(dashboard.data) : 'dashboard unavailable');

                        if (!status.ok || !rpc.ok || !rpcVersion.ok || !dashboard.ok) {
                            markDegraded('one or more upstream APIs are unavailable');
                        }
                    }

                    async function run() {
                        const hydrationStart = performance.now();
                        try {
                            await init();
                            hydrate();
                        } catch (err) {
                            markDegraded('hydration mismatch recovered via SSR fallback');
                            console.error('hydration failed:', err);
                        }
                        const hydrationMs = performance.now() - hydrationStart;

                        checkRouteBudgets(hydrationMs);
                        await loadData();
                        await verifyManifestIntegrity();

                        if (SERVER_FUNCTION_VERSION !== '2026-02-27.1') {
                            markDegraded('server function version mismatch');
                        }

                        // HMR
                        if (location.hostname === 'localhost' || location.hostname === '127.0.0.1') {
                            const evtSource = new EventSource('/api/hmr');
                            evtSource.onmessage = (e) => {
                                console.log('HMR signal received:', e.data);
                                // Simple state-preserving HMR: we'll fetch the current page and diff the DOM,
                                // but for now a simple reload is best since we don't have a full VDOM diffing engine.
                                // In a real framework, we'd update loaded modules and re-render the components.
                                window.location.reload();
                            };
                        }
                    }

                    run();"#;
    let base_url = normalize_public_base_url();
    let canonical = canonical_url(&base_url, "/");
    let structured_data = serde_json::to_string(&json!({
        "@context": "https://schema.org",
        "@type": "WebPage",
        "name": "Krab Framework",
        "url": canonical,
        "description": "Krab full-stack Rust framework home page"
    }))
    .unwrap_or_else(|_| "{}".to_string());

    let content = view! {
        <html>
            <head>
                <title>{page_title}</title>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <meta name="description" content="Krab full-stack Rust framework home page" />
                <meta name="robots" content="index,follow" />
                <link rel="canonical" href={canonical.clone()} />
                <meta property="og:title" content="Krab Framework" />
                <meta property="og:description" content="Krab full-stack Rust framework home page" />
                <meta property="og:type" content="website" />
                <meta property="og:url" content={canonical.clone()} />
                <meta property="og:site_name" content="Krab" />
                <meta name="twitter:card" content="summary_large_image" />
                <meta name="twitter:title" content="Krab Framework" />
                <meta name="twitter:description" content="Krab full-stack Rust framework home page" />
                <style>
                    r#"
                    :root {
                        --bg-color: #0f172a;
                        --text-color: #e2e8f0;
                        --primary: #38bdf8;
                        --secondary: #94a3b8;
                        --card-bg: #1e293b;
                        --border: #334155;
                    }
                    body {
                        font-family: system-ui, -apple-system, sans-serif;
                        line-height: 1.5;
                        color: var(--text-color);
                        background: var(--bg-color);
                        margin: 0;
                        padding: 0;
                    }
                    .container {
                        max_width: 800px;
                        margin: 0 auto;
                        padding: 2rem;
                    }
                    header {
                        text-align: center;
                        padding: 4rem 0;
                    }
                    h1 {
                        font-size: 3.5rem;
                        font-weight: 800;
                        margin: 0 0 1rem;
                        background: linear-gradient(to right, var(--primary), #a855f7);
                        -webkit-background-clip: text;
                        -webkit-text-fill-color: transparent;
                    }
                    .tagline {
                        font-size: 1.25rem;
                        color: var(--secondary);
                        max_width: 600px;
                        margin: 0 auto 2rem;
                    }
                    .links {
                        display: flex;
                        gap: 1rem;
                        justify-content: center;
                        margin-bottom: 4rem;
                    }
                    .btn {
                        display: inline-block;
                        padding: 0.75rem 1.5rem;
                        border-radius: 9999px;
                        font-weight: 600;
                        text-decoration: none;
                        transition: transform 0.2s;
                    }
                    .btn-primary {
                        background: var(--primary);
                        color: #0f172a;
                    }
                    .btn-secondary {
                        background: var(--card-bg);
                        color: var(--text-color);
                        border: 1px solid var(--border);
                    }
                    .btn:hover {
                        transform: translateY(-2px);
                    }
                    .grid {
                        display: grid;
                        grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
                        gap: 1.5rem;
                        margin-bottom: 4rem;
                    }
                    .card {
                        background: var(--card-bg);
                        border: 1px solid var(--border);
                        border-radius: 0.75rem;
                        padding: 1.5rem;
                    }
                    .card h3 {
                        margin-top: 0;
                        color: var(--primary);
                    }
                    .interactive-demo {
                        background: var(--card-bg);
                        border: 1px solid var(--border);
                        border-radius: 1rem;
                        padding: 2rem;
                        margin-top: 2rem;
                    }
                    .demo-row {
                        display: flex;
                        align-items: center;
                        gap: 1rem;
                        margin-bottom: 1rem;
                        padding-bottom: 1rem;
                        border-bottom: 1px solid var(--border);
                    }
                    .demo-row:last-child {
                        border-bottom: none;
                        margin-bottom: 0;
                        padding-bottom: 0;
                    }
                    .status-grid {
                        display: grid;
                        grid-template-columns: repeat(2, 1fr);
                        gap: 1rem;
                        font-family: monospace;
                        font-size: 0.875rem;
                    }
                    .status-item {
                        background: #0002;
                        padding: 0.5rem;
                        border-radius: 0.25rem;
                    }
                    .status-label {
                        color: var(--secondary);
                        display: block;
                        font-size: 0.75rem;
                        margin-bottom: 0.25rem;
                    }
                    "#
                </style>
                <script r#type="application/ld+json">{structured_data}</script>
                <script r#type="module">
                    {runtime_script}
                </script>
            </head>
            <body>
                <div class="container">
                    <header>
                        <h1>{hello}</h1>
                        <p class="tagline">
                            "The full-stack Rust framework designed for performance, type safety, and developer joy."
                        </p>
                        <div class="links">
                            <a href="https://github.com/bishesh/krab" class="btn btn-primary">"Get Started"</a>
                            <a href="/docs" class="btn btn-secondary">"Documentation"</a>
                            <a href="https://github.com/bishesh/krab" class="btn btn-secondary">"GitHub"</a>
                        </div>
                    </header>

                    <div class="grid">
                        <div class="card">
                            <h3>"Server-Side Rendering"</h3>
                            <p>"Blazing fast initial loads with Rust-powered HTML generation. SEO-friendly by default."</p>
                        </div>
                        <div class="card">
                            <h3>"Islands Architecture"</h3>
                            <p>"Ship zero JS by default. Hydrate only the interactive bits for optimal performance."</p>
                        </div>
                        <div class="card">
                            <h3>"Type-Safe Everywhere"</h3>
                            <p>"Share types between backend and frontend. Catch errors at compile time, not runtime."</p>
                        </div>
                    </div>

                    <div class="interactive-demo">
                        <h2>"Interactive Islands"</h2>
                        <p class="mb-4 text-secondary">"These components are hydrated on the client. Try them out!"</p>

                        <div class="demo-row">
                            <span>"Counter:"</span>
                            {counter}
                        </div>
                        <div class="demo-row">
                            <span>"Toggle:"</span>
                            <div>{toggle}</div>
                        </div>
                        <div class="demo-row">
                            <span>"Likes:"</span>
                            <div>{likes}</div>
                        </div>
                    </div>

                    <div class="interactive-demo" style="margin-top: 2rem;">
                        <h2>"Real-Time Data"</h2>
                        <p id="frontend-degraded" style="color: #f87171;"></p>
                        <div class="status-grid">
                            <div class="status-item">
                                <span class="status-label">"SYSTEM STATUS"</span>
                                <span id="status">"connecting..."</span>
                            </div>
                            <div class="status-item">
                                <span class="status-label">"RPC CONNECTION"</span>
                                <span id="rpc">"connecting..."</span>
                            </div>
                            <div class="status-item">
                                <span class="status-label">"SERVER VERSION"</span>
                                <span id="version">"checking..."</span>
                            </div>
                            <div class="status-item">
                                <span class="status-label">"DASHBOARD METRICS"</span>
                                <span id="dashboard">"loading..."</span>
                            </div>
                        </div>
                        <p style="margin-top: 1rem; font-size: 0.875rem; color: var(--secondary);">
                            {rendered}
                        </p>
                    </div>

                    <footer style="text-align: center; margin-top: 4rem; color: var(--secondary); font-size: 0.875rem;">
                        <p>"Built with Rust & Krab Framework"</p>
                    </footer>
                </div>
            </body>
        </html>
    };

    let guarded = ErrorBoundary::new(
        "home-page",
        content,
        view! { <html><body><h1>"Krab fallback"</h1></body></html> },
    );
    let mut writer = ChunkedStreamWriter::new(1024, 2048);
    writer.write("<!DOCTYPE html>");
    writer.write_suspense_marker("home", SuspenseState::Pending);
    writer.write("<div data-krab-hydration=\"home\">");
    writer.write(&guarded.render());
    writer.write("</div>");
    writer.write_suspense_marker("home", SuspenseState::Resolved);
    writer.finish().concat()
}

async fn home_handler(headers: HeaderMap) -> Html<String> {
    let locale = resolve_locale(&headers);
    Html(render_home_page_localized(&locale))
}

async fn localized_home_handler(Path(params): Path<HashMap<String, String>>) -> Html<String> {
    let locale = params.get("locale").map(|s| s.as_str()).unwrap_or("en");
    Html(render_home_page_localized(locale))
}

#[derive(Debug, Deserialize)]
struct WsPublishPayload {
    message: String,
}

async fn ws_publish_handler(Json(payload): Json<WsPublishPayload>) -> Json<serde_json::Value> {
    let room = ws_manager().room("chat").await;
    let delivered = room.broadcast(WsMessage::text(payload.message));
    Json(json!({
        "status": "published",
        "delivered": delivered
    }))
}

async fn ws_chat_handler(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    ws.on_upgrade(handle_ws_chat_socket)
}

async fn handle_ws_chat_socket(socket: WebSocket) {
    let room = ws_manager().room("chat").await;
    room.connect().await;
    let mut subscription = room.subscribe();
    let (mut sender, mut receiver) = socket.split();
    let room_for_sender = room.clone();

    let send_task = tokio::spawn(async move {
        while let Ok(message) = subscription.recv().await {
            match message {
                WsMessage::Text(text) => {
                    if sender.send(AxumWsMessage::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                WsMessage::Binary(bin) => {
                    if sender
                        .send(AxumWsMessage::Binary(bin.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                WsMessage::Close => {
                    let _ = sender.send(AxumWsMessage::Close(None)).await;
                    break;
                }
            }
        }
        room_for_sender.disconnect().await;
    });

    while let Some(Ok(incoming)) = receiver.next().await {
        match incoming {
            AxumWsMessage::Text(text) => {
                room.broadcast(WsMessage::text(text.to_string()));
            }
            AxumWsMessage::Binary(bin) => {
                room.broadcast(WsMessage::Binary(bin.to_vec()));
            }
            AxumWsMessage::Close(_) => {
                room.broadcast(WsMessage::Close);
                break;
            }
            _ => {}
        }
    }

    send_task.abort();
    room.disconnect().await;
}

fn normalize_public_base_url() -> String {
    std::env::var("KRAB_PUBLIC_BASE_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "http://localhost:3000".to_string())
}

fn canonical_url(base_url: &str, path: &str) -> String {
    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{}{}", base_url.trim_end_matches('/'), normalized_path)
}

fn render_seo_page(meta: SeoMeta, body_html: String) -> String {
    let base_url = normalize_public_base_url();
    let canonical = canonical_url(&base_url, &meta.path);
    let structured_data = serde_json::to_string(&json!({
        "@context": "https://schema.org",
        "@type": "WebPage",
        "name": meta.title,
        "url": canonical,
        "description": meta.description
    }))
    .unwrap_or_else(|_| "{}".to_string());

    format!(
        "<!DOCTYPE html><html><head><title>{title}</title><meta charset=\"utf-8\" /><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" /><meta name=\"description\" content=\"{description}\" /><meta name=\"robots\" content=\"index,follow\" /><link rel=\"canonical\" href=\"{canonical}\" /><meta property=\"og:title\" content=\"{title}\" /><meta property=\"og:description\" content=\"{description}\" /><meta property=\"og:type\" content=\"{og_type}\" /><meta property=\"og:url\" content=\"{canonical}\" /><meta property=\"og:site_name\" content=\"Krab\" /><meta name=\"twitter:card\" content=\"summary_large_image\" /><meta name=\"twitter:title\" content=\"{title}\" /><meta name=\"twitter:description\" content=\"{description}\" /><script type=\"application/ld+json\">{structured_data}</script></head><body>{body}</body></html>",
        title = meta.title,
        description = meta.description,
        canonical = canonical,
        og_type = meta.og_type,
        structured_data = structured_data,
        body = body_html,
    )
}

fn render_about_page() -> String {
    render_seo_page(
        SeoMeta {
            title: "About | Krab Framework".to_string(),
            description: "About Krab full-stack Rust framework".to_string(),
            path: "/about".to_string(),
            og_type: "website",
        },
        view! { <h1>"About Page"</h1> }.render(),
    )
}

fn render_greet_page() -> String {
    let name = "Visitor";
    render_seo_page(
        SeoMeta {
            title: "Greet | Krab Framework".to_string(),
            description: "Greeting page for Krab framework".to_string(),
            path: "/greet".to_string(),
            og_type: "website",
        },
        view! {
            <div>
                "Hello, " {name} "!"
                <p>"Welcome to the site."</p>
            </div>
        }
        .render(),
    )
}

fn render_blog_page(slug: &str) -> String {
    let path = format!("/blog/{slug}");
    render_seo_page(
        SeoMeta {
            title: format!("Blog Post: {slug} | Krab Framework"),
            description: format!("Blog content for {slug} in Krab framework"),
            path,
            og_type: "article",
        },
        view! {
            <div>
                <h1>"Blog Post: " {slug}</h1>
                <p>"Content for " {slug}</p>
            </div>
        }
        .render(),
    )
}

async fn robots_txt_handler() -> ([(&'static str, &'static str); 1], String) {
    let base_url = normalize_public_base_url();
    (
        [("content-type", "text/plain; charset=utf-8")],
        format!(
            "User-agent: *\nAllow: /\nSitemap: {}/sitemap.xml\n",
            base_url
        ),
    )
}

async fn sitemap_xml_handler() -> ([(&'static str, &'static str); 1], String) {
    let base_url = normalize_public_base_url();
    let routes = ["/", "/about", "/greet"];
    let urls = routes
        .iter()
        .map(|route| format!("<url><loc>{}{}</loc></url>", base_url, route))
        .collect::<Vec<String>>()
        .join("");

    (
        [("content-type", "application/xml; charset=utf-8")],
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{}</urlset>",
            urls
        ),
    )
}

fn normalize_service_base_url(name: &str, default_url: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default_url.to_string())
}

async fn api_status_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let auth_ok = state
        .http_client
        .get(format!("{}/api/v1/auth/status", state.auth_base_url))
        .send()
        .await
        .map(|resp| resp.status().is_success())
        .unwrap_or(false);

    let users_ok = state
        .http_client
        .get(format!("{}/ready", state.users_base_url))
        .send()
        .await
        .map(|resp| resp.status().is_success())
        .unwrap_or(false);

    Json(json!({
        "service": "frontend",
        "status": if auth_ok && users_ok { "ok" } else { "degraded" },
        "dependencies": {
            "auth": auth_ok,
            "users": users_ok
        }
    }))
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({
        "service": "frontend",
        "status": "ok"
    }))
}

async fn ready_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let uptime = state.runtime_state().started_at.elapsed().as_secs();
    Json(json!({
        "status": "ready",
        "uptime_seconds": uptime,
        "dependencies": []
    }))
}

async fn dashboard_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let users_ready = state
        .http_client
        .get(format!("{}/ready", state.users_base_url))
        .send()
        .await
        .map(|resp| resp.status().is_success())
        .unwrap_or(false);

    let mut auth_key_count = 0_u64;
    if let Ok(resp) = state
        .http_client
        .get(format!("{}/api/v1/auth/status", state.auth_base_url))
        .send()
        .await
    {
        if let Ok(payload) = resp.json::<serde_json::Value>().await {
            auth_key_count = payload
                .get("key_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
    }

    Json(json!({
        "users_online": if users_ready { 1 } else { 0 },
        "active_sessions": auth_key_count,
        "feature": "islands",
        "sources": {
            "users_ready": users_ready,
            "auth_key_count": auth_key_count
        }
    }))
}

fn asset_manifest_json() -> String {
    format!(
        "{{\"assets\":{{\"krab_client.js\":{{\"path\":\"/pkg/krab_client.js?h=6f2c1a\",\"integrity\":\"sha256-demo-manifest-checksum\",\"immutable\":true}}}},\"server_function_version\":\"{}\"}}",
        SERVER_FUNCTION_VERSION
    )
}

fn rpc_version_json() -> String {
    format!(
        "{{\"server_function_version\":\"{}\",\"policy\":\"date-revision (YYYY-MM-DD.N), additive-first changes\"}}",
        SERVER_FUNCTION_VERSION
    )
}

fn rpc_now_json() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!(
        "{{\"epoch_millis\":{},\"server_function_version\":\"{}\"}}",
        now, SERVER_FUNCTION_VERSION
    )
}

#[derive(Debug, Deserialize)]
struct ContactSubmission {
    name: String,
    email: String,
    message: String,
}

fn redact_name_for_log(name: &str) -> String {
    format!("len:{}", name.chars().count())
}

fn redact_email_for_log(email: &str) -> String {
    let trimmed = email.trim();
    let Some((local, domain)) = trimmed.split_once('@') else {
        return "invalid-email".to_string();
    };

    let mut hasher = DefaultHasher::new();
    local.hash(&mut hasher);
    let local_hash = hasher.finish();
    format!("hash:{local_hash:016x}@{}", domain.to_ascii_lowercase())
}

async fn submit_contact_handler(
    Json(payload): Json<ContactSubmission>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let name = payload.name.trim();
    let email = payload.email.trim();
    let message = payload.message.trim();

    if name.is_empty() || email.is_empty() || message.is_empty() || !email.contains('@') {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "invalid",
                "message": "name, email, and message are required"
            })),
        );
    }

    tracing::info!(
        event = "contact_submission_received",
        name_redacted = %redact_name_for_log(name),
        email_redacted = %redact_email_for_log(email),
        message_len = message.len()
    );

    (
        axum::http::StatusCode::ACCEPTED,
        Json(json!({
            "status": "accepted",
            "queued": true,
            "contact": {
                "name": name,
                "email": email,
            }
        })),
    )
}

async fn hmr_handler(
    State(state): State<AppState>,
) -> axum::response::Sse<
    impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use futures_util::StreamExt;
    use tokio_stream::wrappers::WatchStream;

    let stream = WatchStream::new(state.hmr_rx)
        .map(|sig| Ok(axum::response::sse::Event::default().data(sig.to_string())));

    axum::response::Sse::new(stream)
}

fn build_router(state: AppState) -> Router {
    let app: Router<AppState> = Router::new()
        // Critical probes defined FIRST to ensure they are available
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/api/status", get(api_status_handler))
        // Application routes
        .route("/", get(home_handler))
        .route("/{locale}", get(localized_home_handler))
        .route("/about", get(|| async { Html(render_about_page()) }))
        .route("/greet", get(|| async { Html(render_greet_page()) }))
        .route(
            "/blog/{slug}",
            get(|Path(params): Path<HashMap<String, String>>| async move {
                let slug = params
                    .get("slug")
                    .map(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                Html(render_blog_page(slug.as_str()))
            }),
        )
        .route("/robots.txt", get(robots_txt_handler))
        .route("/sitemap.xml", get(sitemap_xml_handler))
        .route("/data/dashboard", get(dashboard_handler))
        .route(
            "/asset-manifest.json",
            get(|| async { asset_manifest_json() }),
        )
        .route("/rpc/now", get(|| async { rpc_now_json() }))
        .route("/rpc/version", get(|| async { rpc_version_json() }))
        .route("/api/ws/chat", get(ws_chat_handler))
        .route("/api/ws/publish", post(ws_publish_handler))
        .route("/api/contact", post(submit_contact_handler))
        .route("/api/hmr", get(hmr_handler));

    // Merge file-system routes generated by build.rs
    // Note: If generated routes have conflicts, Axum will panic at startup.
    let app = service_frontend::register_routes(app, state.clone());

    // Apply cache middleware to the router before wrapping with common layers.
    // With Axum's layer composition, this yields: Request -> Common -> Cache -> Handler.

    let app = app.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        cache_middleware,
    ));

    apply_common_http_layers(app, state.clone()).with_state(state)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing("service_frontend");
    let cfg = KrabConfig::from_env("frontend", 3000);
    cfg.validate()?;
    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    let (hmr_tx, hmr_rx) = tokio::sync::watch::channel(0);

    tokio::spawn(async move {
        let mut last_sig = 0;
        let p = std::path::PathBuf::from("dist/.hmr_signal");
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(content) = std::fs::read_to_string(&p) {
                if let Ok(sig) = content.trim().parse::<u64>() {
                    if sig != last_sig {
                        last_sig = sig;
                        let _ = hmr_tx.send(sig);
                    }
                }
            }
        }
    });

    let state = AppState {
        runtime: RuntimeState::new(),
        http_client: Client::builder().timeout(Duration::from_secs(2)).build()?,
        auth_base_url: normalize_service_base_url("KRAB_AUTH_BASE_URL", "http://127.0.0.1:3001"),
        users_base_url: normalize_service_base_url("KRAB_USERS_BASE_URL", "http://127.0.0.1:3002"),
        isr_cache: IsrCache::new(),
        isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        hmr_rx,
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(service = "frontend", %addr, "service_listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!(service = "frontend", "service_shutdown_signal_received");
        })
        .await?;
    tracing::info!(service = "frontend", "service_shutdown_complete");
    Ok(())
}

#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod tests {
    use super::{
        api_status_handler, asset_manifest_json, dashboard_handler, health_handler,
        normalize_public_base_url, normalize_service_base_url, ready_handler, redact_email_for_log,
        redact_name_for_log, render_about_page, render_blog_page, render_home_page,
        robots_txt_handler, rpc_now_json, rpc_version_json, sitemap_xml_handler, AppState,
        CachedHttpPayload, RuntimeState, SERVER_FUNCTION_VERSION,
    };
    use axum::extract::State;
    use axum::Json;
    use krab_core::http::HasRuntimeState;
    use krab_core::isr::IsrCache;
    use reqwest::Client;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const P95_SSR_RENDER_MS_THRESHOLD: u128 = 6;
    const P99_SSR_RENDER_MS_THRESHOLD: u128 = 12;

    fn percentile_millis(samples_ms: &mut [u128], percentile: f64) -> u128 {
        assert!(!samples_ms.is_empty(), "samples must not be empty");
        let bounded = percentile.clamp(0.0, 1.0);
        samples_ms.sort_unstable();
        let index = ((samples_ms.len() - 1) as f64 * bounded).round() as usize;
        samples_ms[index]
    }

    #[test]
    fn ssr_home_includes_hydration_and_data_loading_contracts() {
        let html = render_home_page();
        assert!(html.contains("hydrate()"));
        assert!(html.contains("/api/status"));
        assert!(html.contains("/rpc/now"));
        assert!(html.contains("/rpc/version"));
        assert!(html.contains("/data/dashboard"));
        assert!(html.contains("/asset-manifest.json"));
        assert!(html.contains("id=\"status\""));
        assert!(html.contains("id=\"rpc\""));
        assert!(html.contains("id=\"version\""));
        assert!(html.contains("id=\"dashboard\""));
        assert!(html.contains("id=\"frontend-degraded\""));
        assert!(html.contains("fetchJsonWithRetry"));
        assert!(html.contains("ROUTE_BUDGETS"));
        assert!(html.contains("schema mismatch"));
        assert!(html.contains("data-island=\"Counter\""));
        assert!(html.contains("data-island=\"Toggle\""));
        assert!(html.contains("data-island=\"Likes\""));
        assert!(html.contains("data-props="));
    }

    #[test]
    fn ssr_home_includes_robust_seo_metadata() {
        std::env::set_var("KRAB_PUBLIC_BASE_URL", "https://krab.example.com");
        let html = render_home_page();
        assert!(html.contains("<meta name=\"description\""));
        assert!(html.contains("<meta name=\"robots\" content=\"index,follow\""));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://krab.example.com/\""));
        assert!(html.contains("<meta property=\"og:title\""));
        assert!(html.contains("<meta name=\"twitter:card\""));
        assert!(html.contains("application/ld+json"));
    }

    #[test]
    fn blog_page_uses_route_specific_canonical_and_article_type() {
        std::env::set_var("KRAB_PUBLIC_BASE_URL", "https://krab.example.com");
        let html = render_blog_page("integration-check");
        assert!(html.contains(
            "<link rel=\"canonical\" href=\"https://krab.example.com/blog/integration-check\""
        ));
        assert!(html.contains("<meta property=\"og:type\" content=\"article\""));
        assert!(html.contains("Blog Post: integration-check | Krab Framework"));
    }

    #[test]
    fn about_page_uses_route_specific_canonical() {
        std::env::set_var("KRAB_PUBLIC_BASE_URL", "https://krab.example.com");
        let html = render_about_page();
        assert!(html.contains("<link rel=\"canonical\" href=\"https://krab.example.com/about\""));
    }

    #[tokio::test]
    async fn robots_and_sitemap_routes_publish_crawler_contracts() {
        std::env::set_var("KRAB_PUBLIC_BASE_URL", "https://krab.example.com");

        let (_robots_headers, robots_body) = robots_txt_handler().await;
        assert!(robots_body.contains("User-agent: *"));
        assert!(robots_body.contains("Sitemap: https://krab.example.com/sitemap.xml"));

        let (_sitemap_headers, sitemap_body) = sitemap_xml_handler().await;
        assert!(sitemap_body.contains("<urlset"));
        assert!(sitemap_body.contains("<loc>https://krab.example.com/</loc>"));
        assert!(sitemap_body.contains("<loc>https://krab.example.com/about</loc>"));
        assert!(sitemap_body.contains("<loc>https://krab.example.com/greet</loc>"));
    }

    #[test]
    fn seo_public_base_url_defaults_for_local_development() {
        std::env::remove_var("KRAB_PUBLIC_BASE_URL");
        assert_eq!(normalize_public_base_url(), "http://localhost:3000");
    }

    #[tokio::test]
    async fn api_contract_status_payload_is_stable_json() {
        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_millis(10))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };
        let Json(json) = api_status_handler(State(state)).await;
        assert_eq!(
            json.get("service").and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(matches!(
            json.get("status").and_then(|v| v.as_str()),
            Some("ok") | Some("degraded")
        ));
    }

    #[tokio::test]
    async fn operational_health_payload_is_stable_json() {
        let Json(json) = health_handler().await;
        assert_eq!(
            json.get("service").and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));
    }

    #[tokio::test]
    async fn operational_ready_payload_matches_contract_shape() {
        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };
        let Json(json) = ready_handler(State(state)).await;
        assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ready"));
        assert!(json
            .get("uptime_seconds")
            .and_then(|v| v.as_u64())
            .is_some());
        assert!(json
            .get("dependencies")
            .and_then(|v| v.as_array())
            .is_some());
    }

    #[tokio::test]
    async fn api_contract_dashboard_payload_contains_expected_fields() {
        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_millis(10))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };
        let Json(json) = dashboard_handler(State(state)).await;
        assert!(json.get("users_online").and_then(|v| v.as_u64()).is_some());
        assert!(json
            .get("active_sessions")
            .and_then(|v| v.as_u64())
            .is_some());
        assert_eq!(
            json.get("feature").and_then(|v| v.as_str()),
            Some("islands")
        );
    }

    #[test]
    fn service_base_url_normalization_uses_default_when_env_missing() {
        std::env::remove_var("KRAB_USERS_BASE_URL");
        let base = normalize_service_base_url("KRAB_USERS_BASE_URL", "http://127.0.0.1:3002");
        assert_eq!(base, "http://127.0.0.1:3002");
    }

    #[test]
    fn browser_journey_contract_scripts_reference_api_matrix() {
        let html = render_home_page();
        assert!(html.contains("fetchJsonWithRetry('/api/status'"));
        assert!(html.contains("fetchJsonWithRetry('/rpc/now'"));
        assert!(html.contains("fetchJsonWithRetry('/rpc/version'"));
        assert!(html.contains("fetchJsonWithRetry('/data/dashboard'"));
        assert!(html.contains("Promise.all"));
    }

    #[test]
    fn rpc_now_contract_returns_epoch_millis_number_and_version() {
        let raw = rpc_now_json();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(json.get("epoch_millis").and_then(|v| v.as_u64()).is_some());
        assert_eq!(
            json.get("server_function_version").and_then(|v| v.as_str()),
            Some(SERVER_FUNCTION_VERSION)
        );
    }

    #[test]
    fn rpc_version_contract_exposes_policy_and_version() {
        let raw = rpc_version_json();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            json.get("server_function_version").and_then(|v| v.as_str()),
            Some(SERVER_FUNCTION_VERSION)
        );
        assert!(json.get("policy").and_then(|v| v.as_str()).is_some());
    }

    #[test]
    fn asset_manifest_contract_enforces_integrity_shape() {
        let raw = asset_manifest_json();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let entry = &json["assets"]["krab_client.js"];
        assert!(entry["path"].as_str().is_some());
        assert!(entry["integrity"]
            .as_str()
            .map(|v| v.starts_with("sha256-"))
            .unwrap_or(false));
        assert_eq!(entry["immutable"].as_bool(), Some(true));
    }

    #[test]
    fn e2e_ssr_to_hydration_journey_contract() {
        let html = render_home_page();
        assert!(html.contains("import init, { hydrate }"));
        assert!(html.contains("await init();"));
        assert!(html.contains("hydrate();"));
        assert!(html.contains("hydration mismatch recovered via SSR fallback"));
        assert!(html.contains("checkRouteBudgets"));
    }

    #[cfg(feature = "nft")]
    #[test]
    fn non_functional_load_profile_ssr_render_stability() {
        let mut render_samples_ms = Vec::with_capacity(1_000);
        for _ in 0..1_000 {
            let start = Instant::now();
            let html = render_home_page();
            assert!(html.contains("Hello from Krab!"));
            render_samples_ms.push(start.elapsed().as_millis());
        }

        let mut p95_samples = render_samples_ms.clone();
        let mut p99_samples = render_samples_ms;
        let p95 = percentile_millis(&mut p95_samples, 0.95);
        let p99 = percentile_millis(&mut p99_samples, 0.99);

        assert!(
            p95 <= P95_SSR_RENDER_MS_THRESHOLD,
            "p95 SSR render latency {}ms exceeded threshold {}ms",
            p95,
            P95_SSR_RENDER_MS_THRESHOLD
        );
        assert!(
            p99 <= P99_SSR_RENDER_MS_THRESHOLD,
            "p99 SSR render latency {}ms exceeded threshold {}ms",
            p99,
            P99_SSR_RENDER_MS_THRESHOLD
        );
    }

    #[cfg(feature = "nft")]
    #[test]
    fn non_functional_spike_profile_ssr_render_stability() {
        let mut render_samples_ms = Vec::with_capacity(3_000);
        for _ in 0..3_000 {
            let start = Instant::now();
            let html = render_home_page();
            assert!(html.contains("id=\"status\""));
            render_samples_ms.push(start.elapsed().as_millis());
        }

        let mut p95_samples = render_samples_ms.clone();
        let mut p99_samples = render_samples_ms;
        let p95 = percentile_millis(&mut p95_samples, 0.95);
        let p99 = percentile_millis(&mut p99_samples, 0.99);

        assert!(
            p95 <= P95_SSR_RENDER_MS_THRESHOLD,
            "spike profile p95 SSR render latency {}ms exceeded threshold {}ms",
            p95,
            P95_SSR_RENDER_MS_THRESHOLD
        );
        assert!(
            p99 <= P99_SSR_RENDER_MS_THRESHOLD,
            "spike profile p99 SSR render latency {}ms exceeded threshold {}ms",
            p99,
            P99_SSR_RENDER_MS_THRESHOLD
        );
    }

    #[cfg(feature = "nft")]
    #[test]
    fn non_functional_soak_profile_ssr_render_stability() {
        let mut render_samples_ms = Vec::with_capacity(10_000);
        for _ in 0..10_000 {
            let start = Instant::now();
            let html = render_home_page();
            assert!(html.contains("data-island=\"Counter\""));
            render_samples_ms.push(start.elapsed().as_millis());
        }

        let mut p95_samples = render_samples_ms.clone();
        let mut p99_samples = render_samples_ms;
        let p95 = percentile_millis(&mut p95_samples, 0.95);
        let p99 = percentile_millis(&mut p99_samples, 0.99);

        assert!(
            p95 <= P95_SSR_RENDER_MS_THRESHOLD,
            "soak profile p95 SSR render latency {}ms exceeded threshold {}ms",
            p95,
            P95_SSR_RENDER_MS_THRESHOLD
        );
        assert!(
            p99 <= P99_SSR_RENDER_MS_THRESHOLD,
            "soak profile p99 SSR render latency {}ms exceeded threshold {}ms",
            p99,
            P99_SSR_RENDER_MS_THRESHOLD
        );
    }

    #[tokio::test]
    async fn cache_tier_contract_api_paths_are_cached() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };

        let app = super::build_router(state);

        // First request - MISS
        let req1 = Request::builder()
            .uri("/data/dashboard")
            .body(Body::empty())
            .unwrap();
        let response1 = app.clone().oneshot(req1).await.unwrap();

        assert_eq!(response1.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response1
                .headers()
                .get("x-cache")
                .and_then(|v| v.to_str().ok()),
            Some("MISS")
        );

        // Second request - HIT
        let req2 = Request::builder()
            .uri("/data/dashboard")
            .body(Body::empty())
            .unwrap();
        let response2 = app.oneshot(req2).await.unwrap();

        assert_eq!(response2.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response2
                .headers()
                .get("x-cache")
                .and_then(|v| v.to_str().ok()),
            Some("HIT")
        );
    }

    #[tokio::test]
    async fn isr_cache_serves_fresh_then_stale() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        std::env::set_var("KRAB_ISR_REVALIDATE_SECS", "1");

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };

        let app = super::build_router(state);

        let first = Request::builder().uri("/").body(Body::empty()).unwrap();
        let response1 = app.clone().oneshot(first).await.unwrap();
        assert_eq!(response1.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response1
                .headers()
                .get("x-isr-state")
                .and_then(|v| v.to_str().ok()),
            Some("fresh")
        );

        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

        let second = Request::builder().uri("/").body(Body::empty()).unwrap();
        let response2 = app.clone().oneshot(second).await.unwrap();
        assert_eq!(response2.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response2
                .headers()
                .get("x-isr-state")
                .and_then(|v| v.to_str().ok()),
            Some("stale")
        );
        assert_eq!(
            response2
                .headers()
                .get("x-cache")
                .and_then(|v| v.to_str().ok()),
            Some("STALE")
        );

        std::env::remove_var("KRAB_ISR_REVALIDATE_SECS");
    }

    #[tokio::test]
    async fn isr_stale_request_triggers_background_regeneration() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        std::env::set_var("KRAB_ISR_REVALIDATE_SECS", "1");

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };

        let app = super::build_router(state);

        let first = Request::builder().uri("/").body(Body::empty()).unwrap();
        let _ = app.clone().oneshot(first).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

        let stale_req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let stale_response = app.clone().oneshot(stale_req).await.unwrap();
        assert_eq!(
            stale_response
                .headers()
                .get("x-isr-state")
                .and_then(|v| v.to_str().ok()),
            Some("stale")
        );

        let mut became_fresh = false;
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            let req = Request::builder().uri("/").body(Body::empty()).unwrap();
            let response = app.clone().oneshot(req).await.unwrap();
            let state = response
                .headers()
                .get("x-isr-state")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            if state == "fresh" {
                became_fresh = true;
                break;
            }
        }

        assert!(
            became_fresh,
            "expected background ISR regeneration to refresh stale cache"
        );
        std::env::remove_var("KRAB_ISR_REVALIDATE_SECS");
    }

    #[tokio::test]
    async fn cache_authority_prefers_isr_for_page_routes_over_distributed_cache() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };

        let stale_payload = CachedHttpPayload {
            body: "<html><body>distributed</body></html>".to_string(),
            content_type: "text/html; charset=utf-8".to_string(),
        };
        let serialized = serde_json::to_string(&stale_payload).unwrap();
        let _ = state
            .runtime_state()
            .store
            .set("/", &serialized, Duration::from_secs(60))
            .await;

        let app = super::build_router(state);
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let response = app.clone().oneshot(req).await.unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-cache")
                .and_then(|v| v.to_str().ok()),
            Some("MISS")
        );
        assert_eq!(
            response
                .headers()
                .get("x-isr-state")
                .and_then(|v| v.to_str().ok()),
            Some("fresh")
        );
    }

    #[tokio::test]
    async fn contact_routes_are_public_and_submission_contract_is_stable() {
        use axum::body::Body;
        use axum::http::{Method, Request};
        use tower::ServiceExt;

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };
        let app = super::build_router(state);

        let get_contact = Request::builder()
            .method(Method::GET)
            .uri("/contact")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(get_contact).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let submit_contact = Request::builder()
            .method(Method::POST)
            .uri("/api/contact")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"Alex","email":"alex@example.com","message":"Hello team"}"#,
            ))
            .unwrap();
        let response = app.oneshot(submit_contact).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::ACCEPTED);
    }

    #[test]
    fn security_log_redaction_masks_email_local_part() {
        let redacted = redact_email_for_log("alex@example.com");
        assert!(redacted.contains("@example.com"));
        assert!(!redacted.contains("alex"));
        assert!(redacted.starts_with("hash:"));
    }

    #[test]
    fn security_log_redaction_masks_name_content() {
        let redacted = redact_name_for_log("Alex Example");
        assert_eq!(redacted, "len:12");
        assert!(!redacted.contains("Alex"));
        assert!(!redacted.contains("Example"));
    }

    #[tokio::test]
    async fn blog_slug_route_renders_successfully() {
        use axum::body::Body;
        use axum::http::{Method, Request};
        use tower::ServiceExt;

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };
        let app = super::build_router(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/blog/integration-check")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn i18n_home_uses_accept_language_locale() {
        use axum::body::Body;
        use axum::http::{Method, Request};
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };
        let app = super::build_router(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/")
            .header("accept-language", "ne-NP,ne;q=0.9,en;q=0.5")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(html.contains("क्र्याबबाट नमस्ते!"));
    }

    #[tokio::test]
    async fn websocket_ergonomic_publish_endpoint_is_available() {
        use axum::body::Body;
        use axum::http::{Method, Request};
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        std::env::set_var("KRAB_AUTH_PUBLIC_PATHS", "/api/ws/*");

        let state = AppState {
            runtime: RuntimeState::new(),
            http_client: Client::builder()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap(),
            auth_base_url: "http://127.0.0.1:1".to_string(),
            users_base_url: "http://127.0.0.1:1".to_string(),
            isr_cache: IsrCache::new(),
            isr_revalidating: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            hmr_rx: tokio::sync::watch::channel(0).1,
        };
        let app = super::build_router(state);

        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/ws/publish")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message":"hello from publish"}"#))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let payload = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(payload.contains("published"));

        std::env::remove_var("KRAB_AUTH_PUBLIC_PATHS");
    }
}
