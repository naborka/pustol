//! What every answer carries, whichever part of the process gave it.

mod common;

use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, header};
use pustol_api::Assets;
use tower::ServiceExt;

use common::harness;

fn built_app() -> Assets {
    Assets::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app"))
        .expect("the fixture is a built app")
}

async fn headers_of(router: Router, path: &str, accept_encoding: Option<&str>) -> HeaderMap {
    let mut request = Request::builder().uri(path);
    if let Some(encoding) = accept_encoding {
        request = request.header(header::ACCEPT_ENCODING, encoding);
    }
    router
        .oneshot(request.body(Body::empty()).expect("a request"))
        .await
        .expect("the router answers")
        .headers()
        .clone()
}

#[tokio::test]
async fn every_answer_forbids_sniffing_referrers_and_framing_outside_telegram() {
    let app = harness().await;
    let router = app.serving(built_app());
    for path in ["/", "/no-such-page", "/health", "/api/session"] {
        let headers = headers_of(router.clone(), path, None).await;
        assert_eq!(headers.get("x-content-type-options").map(axum::http::HeaderValue::as_bytes), Some(&b"nosniff"[..]), "{path}");
        assert_eq!(headers.get("referrer-policy").map(axum::http::HeaderValue::as_bytes), Some(&b"no-referrer"[..]), "{path}");
        let policy = headers
            .get("content-security-policy")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(
            policy.contains("frame-ancestors") && policy.contains("https://web.telegram.org"),
            "{path}: a page any site can frame can be dressed up as something else, got {policy:?}"
        );
    }
}

#[tokio::test]
async fn the_app_travels_compressed_to_a_phone_that_accepts_it() {
    let app = harness().await;
    let router = app.serving(built_app());
    let headers = headers_of(router, "/", Some("gzip")).await;
    assert_eq!(
        headers.get(header::CONTENT_ENCODING).map(axum::http::HeaderValue::as_bytes),
        Some(&b"gzip"[..])
    );
}
