//! # Server Functions (`#[server]`)
//!
//! Server functions allow you to write async functions that run on the server
//! and can be called transparently from the client (WASM) via RPC.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use krab_macros::server;
//! use krab_core::server_fn::ServerFnError;
//!
//! #[server]
//! pub async fn get_user(id: String) -> Result<User, ServerFnError> {
//!     db::find_user(&id).await.map_err(|e| ServerFnError::new(e.to_string()))
//! }
//! ```
//!
//! On the server, this keeps the function as-is and generates an Axum handler.
//! On the client (WASM), the body is replaced with a `fetch` call to `/api/rpc/get_user`.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

// ── Error Type ──────────────────────────────────────────────────────────────

/// Error type returned by server functions.
///
/// Implements `Serialize`/`Deserialize` for wire transport, and converts
/// into an Axum response when used on the server side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerFnError {
    /// Human-readable error message.
    pub message: String,
    /// HTTP status code for the error response.
    #[serde(default = "default_status")]
    pub status_code: u16,
}

fn default_status() -> u16 {
    500
}

impl std::fmt::Display for ServerFnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ServerFnError({}): {}", self.status_code, self.message)
    }
}

impl std::error::Error for ServerFnError {}

impl ServerFnError {
    /// Create a new server error with HTTP 500.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: 500,
        }
    }

    /// Create a bad request error (HTTP 400).
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: 400,
        }
    }

    /// Create an unauthorized error (HTTP 401).
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: 401,
        }
    }

    /// Create a forbidden error (HTTP 403).
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: 403,
        }
    }

    /// Create a not found error (HTTP 404).
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: 404,
        }
    }

    /// Create a conflict error (HTTP 409).
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: 409,
        }
    }
}

impl From<serde_json::Error> for ServerFnError {
    fn from(err: serde_json::Error) -> Self {
        Self::bad_request(format!("serialization error: {}", err))
    }
}

impl From<anyhow::Error> for ServerFnError {
    fn from(err: anyhow::Error) -> Self {
        Self::new(err.to_string())
    }
}

// ── Axum Integration (server side) ──────────────────────────────────────────

#[cfg(feature = "rest")]
impl axum::response::IntoResponse for ServerFnError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.status_code)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::json!({
            "error": self.message,
            "status_code": self.status_code,
        });
        (status, axum::Json(body)).into_response()
    }
}

// ── Registration ────────────────────────────────────────────────────────────

/// Type alias for server function handler.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A registered server function entry point.
pub struct ServerFnRegistration {
    /// Function name (snake_case).
    pub name: &'static str,
    /// URL path for this function (e.g., `/api/rpc/get_user`).
    pub url: &'static str,
    /// Handler that accepts JSON args and returns an Axum response (can be streaming).
    #[cfg(feature = "rest")]
    pub handler: fn(serde_json::Value) -> BoxFuture<axum::response::Response>,
}

#[cfg(not(feature = "rest"))]
pub struct ServerFnRegistration {
    pub name: &'static str,
    pub url: &'static str,
}

// ServerFnRegistration is Send + Sync because all fields are:
// - &'static str: Send + Sync
// - fn pointer: Send + Sync
unsafe impl Send for ServerFnRegistration {}
unsafe impl Sync for ServerFnRegistration {}

/// Build an Axum router from a list of server function registrations.
///
/// Each registration is mounted at its declared URL as a POST endpoint.
///
/// # Example
///
/// ```rust,ignore
/// use krab_core::server_fn::server_fn_router;
///
/// let rpc_routes = server_fn_router(&[
///     get_user_registration(),
///     list_items_registration(),
/// ]);
///
/// let app = Router::new()
///     .merge(rpc_routes)
///     .route("/health", get(health));
/// ```
#[cfg(feature = "rest")]
pub fn server_fn_router(registrations: &'static [ServerFnRegistration]) -> axum::Router {
    use axum::routing::post;

    let mut router = axum::Router::new();
    for reg in registrations {
        let handler = reg.handler;
        router = router.route(
            reg.url,
            post(
                move |axum::Json(args): axum::Json<serde_json::Value>| async move {
                    handler(args).await
                },
            ),
        );
    }
    router
}

/// Build a catch-all RPC dispatcher that routes `/api/rpc/:fn_name` to
/// matching registrations.
///
/// This is an alternative to `server_fn_router` for simpler wiring.
#[cfg(feature = "rest")]
pub fn server_fn_dispatch_router(registrations: &'static [ServerFnRegistration]) -> axum::Router {
    router_with_dispatch(registrations)
}

#[cfg(feature = "rest")]
fn router_with_dispatch(registrations: &'static [ServerFnRegistration]) -> axum::Router {
    use axum::extract::Path;
    use axum::response::IntoResponse;

    axum::Router::new().route(
        "/api/rpc/{fn_name}",
        axum::routing::post(
            move |Path(fn_name): Path<String>,
                  axum::Json(args): axum::Json<serde_json::Value>| async move {
                for reg in registrations {
                    if reg.name == fn_name {
                        return (reg.handler)(args).await;
                    }
                }
                ServerFnError::not_found(format!("server function '{}' not found", fn_name))
                    .into_response()
            },
        ),
    )
}

// ── Client-Side Call (WASM) ─────────────────────────────────────────────────

/// Call a server function from the client (WASM) via fetch.
///
/// This is used by the `#[server]` macro in the WASM client stub.
#[cfg(target_arch = "wasm32")]
pub async fn call_server_fn<A: Serialize, T: serde::de::DeserializeOwned>(
    url: &str,
    args: &A,
) -> Result<T, ServerFnError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| ServerFnError::new("no window object"))?;
    let body = serde_json::to_string(args).map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut opts = web_sys::RequestInit::new();
    opts.method("POST");
    opts.body(Some(&wasm_bindgen::JsValue::from_str(&body)));

    let request = web_sys::Request::new_with_str_and_init(url, &opts)
        .map_err(|_| ServerFnError::new("failed to create request"))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|_| ServerFnError::new("failed to set content-type"))?;

    // Propagate CSRF token if present
    if let Some(document) = window.document() {
        if let Ok(cookie) =
            js_sys::Reflect::get(&document, &wasm_bindgen::JsValue::from_str("cookie"))
        {
            let cookie_str = cookie.as_string().unwrap_or_default();
            for part in cookie_str.split(';') {
                let trimmed = part.trim();
                if let Some(token) = trimmed.strip_prefix("csrf_token=") {
                    let _ = request.headers().set("x-csrf-token", token);
                }
            }
        }
    }

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|_| ServerFnError::new("fetch failed"))?;

    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| ServerFnError::new("response cast failed"))?;

    let text = JsFuture::from(
        resp.text()
            .map_err(|_| ServerFnError::new("failed to read response body"))?,
    )
    .await
    .map_err(|_| ServerFnError::new("failed to await response text"))?;

    let text_str = text
        .as_string()
        .ok_or_else(|| ServerFnError::new("response is not a string"))?;

    if resp.ok() {
        serde_json::from_str(&text_str).map_err(|e| ServerFnError::new(e.to_string()))
    } else {
        // Try to parse server error
        if let Ok(err) = serde_json::from_str::<ServerFnError>(&text_str) {
            Err(err)
        } else {
            Err(ServerFnError {
                message: text_str,
                status_code: resp.status(),
            })
        }
    }
}

/// Placeholder for non-WASM, non-server contexts (e.g., tests).
#[cfg(all(not(target_arch = "wasm32"), not(feature = "rest")))]
pub async fn call_server_fn<A: Serialize, T: serde::de::DeserializeOwned>(
    _url: &str,
    _args: &A,
) -> Result<T, ServerFnError> {
    Err(ServerFnError::new(
        "call_server_fn is only available in WASM or with the 'rest' feature",
    ))
}

// ── Convenience Macro ───────────────────────────────────────────────────────

/// Macro to collect server function registrations into a static slice.
///
/// # Example
///
/// ```rust,ignore
/// use krab_core::collect_server_fns;
///
/// // Each server function generates a `{name}_registration` function
/// static SERVER_FNS: &[krab_core::server_fn::ServerFnRegistration] =
///     &collect_server_fns![get_user, list_items, create_item];
/// ```
#[macro_export]
macro_rules! collect_server_fns {
    ($($fn_name:ident),* $(,)?) => {
        [
            $($crate::server_fn::ServerFnRegistration {
                name: stringify!($fn_name),
                url: concat!("/api/rpc/", stringify!($fn_name)),
                handler: paste::paste! { [<__ $fn_name _handler>] },
            }),*
        ]
    };
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_fn_error_display() {
        let err = ServerFnError::new("something went wrong");
        assert_eq!(err.to_string(), "ServerFnError(500): something went wrong");
        assert_eq!(err.status_code, 500);
    }

    #[test]
    fn server_fn_error_constructors() {
        assert_eq!(ServerFnError::bad_request("bad").status_code, 400);
        assert_eq!(ServerFnError::unauthorized("no").status_code, 401);
        assert_eq!(ServerFnError::forbidden("denied").status_code, 403);
        assert_eq!(ServerFnError::not_found("gone").status_code, 404);
        assert_eq!(ServerFnError::conflict("dup").status_code, 409);
    }

    #[test]
    fn server_fn_error_serialization() {
        let err = ServerFnError::bad_request("invalid input");
        let json = serde_json::to_string(&err).unwrap();
        let parsed: ServerFnError = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.message, "invalid input");
        assert_eq!(parsed.status_code, 400);
    }

    #[test]
    fn server_fn_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something failed");
        let err: ServerFnError = anyhow_err.into();
        assert_eq!(err.status_code, 500);
        assert!(err.message.contains("something failed"));
    }

    #[test]
    fn server_fn_error_from_serde() {
        let serde_err = serde_json::from_str::<String>("not valid json").unwrap_err();
        let err: ServerFnError = serde_err.into();
        assert_eq!(err.status_code, 400);
        assert!(err.message.contains("serialization error"));
    }
}
