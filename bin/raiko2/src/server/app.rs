//! HTTP app wiring.

use axum::{Router, extract::DefaultBodyLimit, http::Request};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::{HttpMakeClassifier, MakeSpan, TraceLayer};
use tracing::Span;

use super::AppState;
use super::routes;

const API_BODY_LIMIT_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug)]
struct RequestSpan;

impl<B> MakeSpan<B> for RequestSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        tracing::info_span!(
            "request",
            method = %request.method(),
            path = %request.uri().path(),
        )
    }
}

fn http_trace_layer() -> TraceLayer<HttpMakeClassifier, RequestSpan> {
    TraceLayer::new_for_http().make_span_with(RequestSpan)
}

pub fn build_router(state: AppState) -> Router {
    build_router_with_api_routes(state, routes::api_routes())
}

#[cfg(all(test, feature = "fixture-server"))]
pub fn build_router_with_legacy_v3_for_tests(state: AppState) -> Router {
    build_router_with_api_routes(state, routes::api_routes_with_legacy_v3_for_tests())
}

fn build_router_with_api_routes(state: AppState, api_routes: Router<AppState>) -> Router {
    Router::new()
        .merge(api_routes)
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES))
        .layer(http_trace_layer())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::post,
    };
    use tower::ServiceExt;
    use tracing_subscriber::EnvFilter;

    use super::*;

    #[derive(Clone, Default)]
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    impl LogBuffer {
        fn output(&self) -> String {
            let bytes = self.0.lock().expect("lock log buffer").clone();
            String::from_utf8(bytes).expect("log output is UTF-8")
        }
    }

    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("lock log buffer")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
        type Writer = LogWriter;

        fn make_writer(&'a self) -> Self::Writer {
            LogWriter(Arc::clone(&self.0))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failure_log_includes_method_and_path_without_query() {
        let logs = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("info"))
            .without_time()
            .with_ansi(false)
            .with_writer(logs.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let app = Router::new()
            .route(
                "/unavailable",
                post(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            )
            .layer(http_trace_layer());
        let request = Request::builder()
            .method("POST")
            .uri("/unavailable?token=secret")
            .body(Body::empty())
            .expect("build request");

        let response = app.oneshot(request).await.expect("dispatch request");
        let output = logs.output();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            output.contains("request{method=POST path=/unavailable}"),
            "{output}"
        );
        assert!(output.contains("response failed"), "{output}");
        assert!(
            output.contains("Status code: 503 Service Unavailable"),
            "{output}"
        );
        assert!(!output.contains("token=secret"), "{output}");
    }
}
