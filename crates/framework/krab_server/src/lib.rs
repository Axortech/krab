use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower::util::service_fn;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;

use std::future::Future;
use std::pin::Pin;

use hyper::header::HeaderValue;
use mime_guess::MimeGuess;

// Type alias for route parameters
pub type Params = HashMap<String, String>;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub trait IntoResponse {
    fn into_response(self) -> Response<Full<Bytes>>;
}

impl IntoResponse for String {
    fn into_response(self) -> Response<Full<Bytes>> {
        let mut res = Response::new(Full::new(Bytes::from(self)));
        res.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html"),
        );
        res
    }
}

impl IntoResponse for &str {
    fn into_response(self) -> Response<Full<Bytes>> {
        self.to_string().into_response()
    }
}

impl IntoResponse for Response<Full<Bytes>> {
    fn into_response(self) -> Response<Full<Bytes>> {
        self
    }
}

// Handler type: takes Params and returns Future of Response
pub type Handler = Box<dyn Fn(Params) -> BoxFuture<Response<Full<Bytes>>> + Send + Sync>;

struct TrieNode {
    children: HashMap<String, TrieNode>,
    dynamic_child: Option<(String, Box<TrieNode>)>, // (param_name, next_node)
    handler: Option<Handler>,
}

impl TrieNode {
    fn new() -> Self {
        Self {
            children: HashMap::new(),
            dynamic_child: None,
            handler: None,
        }
    }
}

pub struct Router {
    root: TrieNode,
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Router {
    pub fn new() -> Self {
        Self {
            root: TrieNode::new(),
        }
    }

    pub fn add_route<F, Fut, R>(&mut self, path: &str, handler: F)
    where
        F: Fn(Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        let mut current = &mut self.root;
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        for segment in segments {
            if let Some(stripped) = segment.strip_prefix(':') {
                let param_name = stripped.to_string();
                if current.dynamic_child.is_none() {
                    current.dynamic_child = Some((param_name.clone(), Box::new(TrieNode::new())));
                }
                if let Some((_, node)) = current.dynamic_child.as_mut() {
                    current = node;
                } else {
                    return;
                }
            } else {
                current = current
                    .children
                    .entry(segment.to_string())
                    .or_insert(TrieNode::new());
            }
        }

        current.handler = Some(Box::new(move |params| {
            let fut = handler(params);
            Box::pin(async move { fut.await.into_response() })
        }));
    }

    pub fn handle(&self, path: &str) -> Option<BoxFuture<Response<Full<Bytes>>>> {
        if path.len() > 1 && path.ends_with('/') {
            return None;
        }

        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current = &self.root;
        let mut params = Params::new();

        for segment in segments {
            if let Some(node) = current.children.get(segment) {
                current = node;
            } else if let Some((param_name, node)) = &current.dynamic_child {
                params.insert(param_name.clone(), segment.to_string());
                current = node;
            } else {
                return None;
            }
        }

        current.handler.as_ref().map(|handler| handler(params))
    }
}

pub struct Server {
    router: Arc<Router>,
    static_dir: Option<PathBuf>,
    static_cache_control: Arc<String>,
    not_found_page: Arc<String>,
    error_page: Arc<String>,
}

impl Server {
    pub fn new(router: Router) -> Self {
        Self {
            router: Arc::new(router),
            static_dir: None,
            static_cache_control: Arc::new("public, max-age=31536000, immutable".to_string()),
            not_found_page: Arc::new("<h1>404 Not Found</h1>".to_string()),
            error_page: Arc::new("<h1>500 Internal Server Error</h1>".to_string()),
        }
    }

    pub fn with_static(mut self, path: impl Into<PathBuf>) -> Self {
        self.static_dir = Some(path.into());
        self
    }

    pub fn with_static_cache_control(mut self, value: impl Into<String>) -> Self {
        self.static_cache_control = Arc::new(value.into());
        self
    }

    pub fn with_not_found_page(mut self, html: impl Into<String>) -> Self {
        self.not_found_page = Arc::new(html.into());
        self
    }

    pub fn with_error_page(mut self, html: impl Into<String>) -> Self {
        self.error_page = Arc::new(html.into());
        self
    }

    pub async fn run(
        self,
        addr: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(addr).await?;
        println!("Listening on http://{}", addr);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let router = self.router.clone();
            let static_dir = self.static_dir.clone();
            let static_cache_control = self.static_cache_control.clone();
            let not_found_page = self.not_found_page.clone();
            let error_page = self.error_page.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    handle_request(
                        req,
                        router.clone(),
                        static_dir.clone(),
                        static_cache_control.clone(),
                        not_found_page.clone(),
                        error_page.clone(),
                    )
                });

                let service = ServiceBuilder::new()
                    .layer(TraceLayer::new_for_http())
                    .service(service);

                let hyper_service = TowerToHyperService::new(service);

                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, hyper_service)
                    .await
                {
                    eprintln!("Error serving connection: {:?}", err);
                }
            });
        }
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    router: Arc<Router>,
    static_dir: Option<PathBuf>,
    static_cache_control: Arc<String>,
    not_found_page: Arc<String>,
    error_page: Arc<String>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path();

    if let Some(dir) = &static_dir {
        if path.starts_with("/pkg/") {
            let Some(relative_path) = path.strip_prefix("/pkg/") else {
                return Ok(build_not_found_response(not_found_page.as_ref()));
            };
            let relative_path = relative_path.trim_start_matches('/');

            if !relative_path.is_empty() {
                if let Some(file_path) = resolve_static_pkg_path(dir, relative_path) {
                    match tokio::fs::read(&file_path).await {
                        Ok(content) => {
                            let mime = static_mime_for(path);

                            let mut response = Response::new(Full::new(Bytes::from(content)));
                            if !set_header_from_str(
                                &mut response,
                                hyper::header::CONTENT_TYPE,
                                &mime,
                            ) {
                                return Ok(build_internal_error_response(error_page.as_ref()));
                            }

                            set_header_from_static(
                                &mut response,
                                hyper::header::X_CONTENT_TYPE_OPTIONS,
                                "nosniff",
                            );

                            if !set_header_from_str(
                                &mut response,
                                hyper::header::CACHE_CONTROL,
                                static_cache_control.as_ref(),
                            ) {
                                return Ok(build_internal_error_response(error_page.as_ref()));
                            }
                            return Ok(response);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => {
                            return Ok(build_internal_error_response(error_page.as_ref()));
                        }
                    }
                }
            }
        }
    }

    match router.handle(path) {
        Some(fut) => Ok(fut.await),
        None => Ok(build_not_found_response(not_found_page.as_ref())),
    }
}

fn set_header_from_static(
    response: &mut Response<Full<Bytes>>,
    header: hyper::header::HeaderName,
    value: &'static str,
) {
    response
        .headers_mut()
        .insert(header, HeaderValue::from_static(value));
}

fn static_mime_for(path: &str) -> String {
    MimeGuess::from_path(path)
        .first_raw()
        .map(normalize_static_mime)
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn normalize_static_mime(mime: &str) -> String {
    if mime == "text/html" {
        "text/html; charset=utf-8".to_string()
    } else {
        mime.to_string()
    }
}

fn set_header_from_str(
    response: &mut Response<Full<Bytes>>,
    header: hyper::header::HeaderName,
    value: &str,
) -> bool {
    match HeaderValue::from_str(value) {
        Ok(parsed) => {
            response.headers_mut().insert(header, parsed);
            true
        }
        Err(_) => false,
    }
}

fn build_not_found_response(not_found_page: &str) -> Response<Full<Bytes>> {
    let mut not_found = Response::new(Full::new(Bytes::from(not_found_page.to_owned())));
    *not_found.status_mut() = StatusCode::NOT_FOUND;
    set_header_from_static(
        &mut not_found,
        hyper::header::CONTENT_TYPE,
        "text/html; charset=utf-8",
    );
    not_found
}

fn build_internal_error_response(error_page: &str) -> Response<Full<Bytes>> {
    let mut internal_error = Response::new(Full::new(Bytes::from(error_page.to_owned())));
    *internal_error.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    set_header_from_static(
        &mut internal_error,
        hyper::header::CONTENT_TYPE,
        "text/html; charset=utf-8",
    );
    internal_error
}

fn resolve_static_pkg_path(static_root: &Path, requested_relative_path: &str) -> Option<PathBuf> {
    let requested = Path::new(requested_relative_path);
    if requested.is_absolute() {
        return None;
    }

    if requested.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }

    let canonical_root = std::fs::canonicalize(static_root).ok()?;
    let candidate = canonical_root.join(requested);
    let canonical_candidate = std::fs::canonicalize(candidate).ok()?;

    if canonical_candidate.starts_with(&canonical_root) {
        Some(canonical_candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    fn make_router() -> Router {
        let mut r = Router::new();
        r.add_route("/", |_| async { "root".to_string() });
        r.add_route("/about", |_| async { "about".to_string() });
        r.add_route("/blog/:slug", |params: Params| async move {
            params.get("slug").cloned().unwrap_or_default()
        });
        r.add_route("/a/b/c", |_| async { "deep".to_string() });
        r
    }

    async fn body_string(resp: Response<Full<Bytes>>) -> String {
        let bytes = resp
            .into_body()
            .collect()
            .await
            .expect("body collect failed")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap_or_default()
    }

    #[tokio::test]
    async fn root_route_resolves() {
        let r = make_router();
        let fut = r.handle("/").expect("root route not found");
        assert_eq!(body_string(fut.await).await, "root");
    }

    #[tokio::test]
    async fn static_route_resolves() {
        let r = make_router();
        assert!(r.handle("/about").is_some());
    }

    #[tokio::test]
    async fn unknown_route_returns_none() {
        let r = make_router();
        assert!(r.handle("/not-found").is_none());
    }

    #[tokio::test]
    async fn dynamic_route_extracts_param() {
        let r = make_router();
        let fut = r.handle("/blog/hello-world").expect("blog route not found");
        assert_eq!(body_string(fut.await).await, "hello-world");
    }

    #[tokio::test]
    async fn deep_static_route_resolves() {
        let r = make_router();
        assert!(r.handle("/a/b/c").is_some());
        assert!(r.handle("/a/b").is_none());
    }

    #[tokio::test]
    async fn trailing_slash_does_not_match_without_route() {
        let r = make_router();
        // /about/ has an extra empty segment — no matching route registered
        assert!(r.handle("/about/").is_none());
    }

    #[test]
    fn string_into_response_sets_html_content_type() {
        let resp = "hello".into_response();
        assert_eq!(
            resp.headers()
                .get(hyper::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/html")
        );
    }

    #[test]
    fn resolve_static_pkg_path_rejects_parent_dir_traversal() {
        let root = std::env::temp_dir().join("krab_server_static_root_reject");
        std::fs::create_dir_all(&root).expect("failed to create static root");

        let resolved = resolve_static_pkg_path(&root, "../secret.txt");
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_static_pkg_path_accepts_file_inside_root() {
        let root = std::env::temp_dir().join("krab_server_static_root_accept");
        std::fs::create_dir_all(&root).expect("failed to create static root");
        let file = root.join("app.js");
        std::fs::write(&file, "console.log('ok');").expect("failed to create static file");

        let resolved = resolve_static_pkg_path(&root, "app.js").expect("expected in-root file");
        let canonical_root = std::fs::canonicalize(&root).expect("failed to canonicalize root");
        assert!(resolved.starts_with(&canonical_root));
    }

    #[test]
    fn static_mime_for_uses_lookup_and_safe_default() {
        assert_eq!(
            static_mime_for("/pkg/app.js"),
            "text/javascript".to_string()
        );
        assert_eq!(static_mime_for("/pkg/site.css"), "text/css".to_string());
        assert_eq!(
            static_mime_for("/pkg/index.html"),
            "text/html; charset=utf-8".to_string()
        );
        assert_eq!(
            static_mime_for("/pkg/module.wasm"),
            "application/wasm".to_string()
        );
        assert_eq!(
            static_mime_for("/pkg/blob.unknownext"),
            "application/octet-stream".to_string()
        );
    }
}
