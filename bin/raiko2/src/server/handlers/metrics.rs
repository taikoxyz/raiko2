use axum::{
    http::{StatusCode, header},
    response::IntoResponse,
};

use crate::server::telemetry;

pub async fn metrics() -> impl IntoResponse {
    match telemetry::render() {
        Ok((content_type, body)) => {
            (StatusCode::OK, [(header::CONTENT_TYPE, content_type)], body).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "failed to render prometheus metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to render metrics".to_string(),
            )
                .into_response()
        }
    }
}
