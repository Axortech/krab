#[cfg(test)]
mod tests {
    use crate::http::ApiError;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::json;

    #[test]
    fn test_error_envelope_structure() {
        let error = ApiError::new("TEST_ERROR", "Something went wrong")
            .with_details(json!({ "field": "value" }));

        assert_eq!(error.code, "TEST_ERROR");
        assert_eq!(error.message, "Something went wrong");
        assert!(error.details.is_some());
    }

    #[test]
    fn test_error_status_mapping() {
        let error = ApiError::new("UNAUTHORIZED", "Access denied");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let error = ApiError::new("NOT_FOUND", "Resource missing");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let error = ApiError::new("UNKNOWN_CODE", "System error");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
