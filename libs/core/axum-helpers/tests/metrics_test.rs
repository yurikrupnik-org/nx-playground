#![allow(clippy::unwrap_used)]

//! End-to-end smoke test: a tracked request shows up in the `/metrics` render.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum_helpers::{init_metrics, metrics_router, track_metrics};
use tower::ServiceExt; // for `oneshot`

#[tokio::test]
async fn metrics_endpoint_renders_tracked_requests() {
    // Install the process-global Prometheus recorder (only this test does so).
    let handle = init_metrics().expect("install recorder");

    let app = Router::new()
        .route("/ping", get(|| async { "pong" }))
        // RED metrics for /ping; /metrics is merged after, so it is not self-counted.
        .layer(axum::middleware::from_fn(track_metrics))
        .merge(metrics_router(handle));

    // Drive one tracked request through the middleware.
    let res = app
        .clone()
        .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Scrape the exposition endpoint.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).expect("utf8 metrics");

    // RED counter present, labelled with the matched route template and status.
    assert!(
        text.contains("http_requests_total"),
        "counter missing:\n{text}"
    );
    assert!(
        text.contains("path=\"/ping\""),
        "matched-path label missing:\n{text}"
    );
    assert!(
        text.contains("status=\"200\""),
        "status label missing:\n{text}"
    );
    // Latency histogram present (bucketed via init_metrics).
    assert!(
        text.contains("http_request_duration_seconds"),
        "histogram missing:\n{text}"
    );
    // The scrape endpoint itself was not tracked.
    assert!(
        !text.contains("path=\"/metrics\""),
        "/metrics should not be self-counted:\n{text}"
    );
}
