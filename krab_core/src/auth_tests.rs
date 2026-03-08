#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serial_test::serial;
    use serde_json::{json, Value};
    use std::sync::{Mutex, OnceLock};
    use tower::ServiceExt; 
    
    use crate::http::{RuntimeState, apply_common_http_layers};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        match ENV_LOCK.get_or_init(|| Mutex::new(())).lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn reset_auth_env() {
        for key in [
            "KRAB_AUTH_MODE",
            "KRAB_JWT_SECRET",
            "KRAB_JWT_KEYS_JSON",
            "KRAB_JWT_PROVIDERS_JSON",
            "KRAB_OIDC_ISSUER",
            "KRAB_OIDC_AUDIENCE",
            "KRAB_AUTH_REQUIRED_SCOPES",
            "KRAB_AUTH_REQUIRED_ROLES",
            "KRAB_AUTH_REQUIRED_CLAIMS_JSON",
            "KRAB_AUTH_ROUTE_POLICIES_JSON",
            "KRAB_AUTH_REQUIRE_TENANT_CLAIM",
            "KRAB_AUTH_REQUIRE_TENANT_MATCH",
            "KRAB_JWT_REQUIRE_KID",
        ] {
            std::env::remove_var(key);
        }
    }

    fn test_app() -> Router {
        let state = RuntimeState::new();
        // `apply_common_http_layers` requires state to implement `HasRuntimeState`.
        // Use a lightweight wrapper for tests so we can provide `RuntimeState`
        // without changing production state types.
        #[derive(Clone)]
        struct TestState {
            runtime: RuntimeState,
        }
        
        impl crate::http::HasRuntimeState for TestState {
            fn runtime_state(&self) -> &RuntimeState {
                &self.runtime
            }
        }
        
        let state = TestState { runtime: state };

        let app = Router::new()
            .route("/protected", axum::routing::get(|| async { "ok" }))
            .route("/api/admin/audit", axum::routing::get(|| async { "admin" }))
            .route("/api/tenants/{tenant_id}/users", axum::routing::get(|| async { "tenant" }));
            
        apply_common_http_layers(app, state.clone()).with_state(state)
    }

    fn generate_token(claims: Value) -> String {
        let key = b"secret";
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(key),
        )
        .unwrap()
    }

    fn generate_token_with_kid(kid: &str, claims: Value, secret: &[u8]) -> String {
        let header = Header {
            kid: Some(kid.to_string()),
            ..Default::default()
        };
        encode(&header, &claims, &EncodingKey::from_secret(secret)).unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_expired_token() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_JWT_SECRET", "secret");

        let app = test_app();
        let claims = json!({
            "sub": "user",
            "exp": 1000000000 // Past timestamp
        });
        let token = generate_token(claims);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_wrong_issuer() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_JWT_SECRET", "secret");
        std::env::set_var("KRAB_OIDC_ISSUER", "correct-issuer");

        let app = test_app();
        let claims = json!({
            "sub": "user",
            "iss": "wrong-issuer",
            "exp": 9999999999i64
        });
        let token = generate_token(claims);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_wrong_audience() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_JWT_SECRET", "secret");
        std::env::set_var("KRAB_OIDC_AUDIENCE", "correct-aud");

        let app = test_app();
        let claims = json!({
            "sub": "user",
            "aud": "wrong-aud",
            "exp": 9999999999i64
        });
        let token = generate_token(claims);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_missing_required_scope() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_JWT_SECRET", "secret");
        std::env::set_var("KRAB_AUTH_REQUIRED_SCOPES", "read:data");

        let app = test_app();
        let claims = json!({
            "sub": "user",
            "scope": "other:scope",
            "exp": 9999999999i64
        });
        let token = generate_token(claims);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_revoked_key() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        // We set keys JSON where 'kid1' is valid but we sign with 'kid2' which is not in the set
        std::env::set_var("KRAB_JWT_KEYS_JSON", r#"{"kid1": "secret1"}"#);

        let app = test_app();
        
        let claims = json!({
            "sub": "user",
            "exp": 9999999999i64
        });
        
        // Sign with a secret that corresponds to kid2, but kid2 is not in the trusted set
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(b"secret2"),
        )
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should fail because kid2 is not found in loaded keys (effectively revoked/unknown)
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_provider_jwks_selection() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var(
            "KRAB_JWT_PROVIDERS_JSON",
            r#"[
                {"name":"provider-a","issuer":"iss-a","audience":"aud-a","keys":{"kid-a":"secret-a"}},
                {"name":"provider-b","issuer":"iss-b","audience":"aud-b","keys":{"kid-b":"secret-b"}}
            ]"#,
        );

        let app = test_app();
        let claims = json!({
            "sub": "user",
            "iss": "iss-b",
            "aud": "aud-b",
            "exp": 9999999999i64
        });
        let token = generate_token_with_kid("kid-b", claims, b"secret-b");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[serial]
    async fn test_tenant_path_mismatch_is_denied() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_JWT_SECRET", "secret");
        std::env::set_var("KRAB_AUTH_REQUIRE_TENANT_MATCH", "true");

        let app = test_app();
        let claims = json!({
            "sub": "user",
            "tenant_id": "tenant-a",
            "exp": 9999999999i64
        });
        let token = generate_token(claims);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/tenants/tenant-b/users")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn test_route_policy_requires_composed_scope() {
        let _guard = env_lock();
        reset_auth_env();
        std::env::set_var("KRAB_AUTH_MODE", "jwt");
        std::env::set_var("KRAB_JWT_SECRET", "secret");
        std::env::set_var(
            "KRAB_AUTH_ROUTE_POLICIES_JSON",
            r#"[{"prefix":"/api/admin","all_scopes":["audit.read"]}]"#,
        );

        let app = test_app();
        let claims = json!({
            "sub": "user",
            "scope": "users.read",
            "exp": 9999999999i64
        });
        let token = generate_token(claims);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/admin/audit")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
