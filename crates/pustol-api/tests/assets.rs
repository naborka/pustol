//! Serving the built Mini App from the API process.
//!
//! The app and the API answer on one origin. Proxying was one way to get that; serving the build
//! from the process that owns the API is the other, and it is the one that cannot be misconfigured
//! into two origins, because there is only ever one listener.
//!
//! These tests need no database: the fixture is a directory shaped like a Next static export.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use pustol_api::assets::{Assets, AssetsError};
use tower::ServiceExt;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn app() -> Router {
    Assets::open(fixture("app"))
        .expect("the fixture is a built app")
        .into_router()
}

struct Served {
    status: StatusCode,
    headers: header::HeaderMap,
    body: Vec<u8>,
}

impl Served {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn header(&self, name: header::HeaderName) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }
}

async fn get(path: &str) -> Served {
    get_with(path, &[]).await
}

async fn get_with(path: &str, headers: &[(header::HeaderName, &str)]) -> Served {
    let mut request = Request::builder().uri(path);
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    let response = app()
        .oneshot(request.body(Body::empty()).expect("a request"))
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes()
        .to_vec();
    Served {
        status,
        headers,
        body,
    }
}

#[test]
fn a_directory_that_is_not_there_is_refused() {
    // The likely misconfiguration is a path that does not exist — a stale environment variable, a
    // build stage that produced nothing. Refusing at start-up turns it into a deploy that fails
    // instead of a bar whose staff find a white screen.
    let error = Assets::open(fixture("no-such-directory")).expect_err("refused");
    assert!(matches!(error, AssetsError::NotADirectory(_)), "{error}");
}

#[test]
fn a_directory_with_no_index_is_refused() {
    // A directory that exists but holds no app is the same failure wearing a disguise: every
    // request would answer 404 and the process would call itself healthy.
    let error = Assets::open(fixture("not-an-app")).expect_err("refused");
    assert!(matches!(error, AssetsError::NoIndex(_)), "{error}");
}

#[tokio::test]
async fn the_root_serves_the_index() {
    let served = get("/").await;
    assert_eq!(served.status, StatusCode::OK);
    assert!(served.text().contains("the mini app"), "{}", served.text());
    let content_type = served.header(header::CONTENT_TYPE).unwrap_or_default();
    assert!(
        content_type.starts_with("text/html"),
        "the index has to arrive as a document, not as a download: {content_type}"
    );
}

#[tokio::test]
async fn the_index_is_revalidated_on_every_launch() {
    // The one that matters. `index.html` names content-hashed chunks; a cached copy outlives the
    // deploy that deleted those chunks, and the guest gets a white screen inside a WebView they
    // cannot force-refresh. Revalidation costs one conditional request and removes the class.
    let served = get("/").await;
    assert_eq!(served.header(header::CACHE_CONTROL), Some("no-cache"));
}

#[tokio::test]
async fn hashed_assets_are_cached_for_a_year() {
    // The complement: the filename changes whenever the bytes do, so re-fetching them is waste on
    // a phone that is already on a bad connection.
    let served = get("/_next/static/chunk.deadbeef.js").await;
    assert_eq!(served.status, StatusCode::OK);
    assert_eq!(
        served.header(header::CACHE_CONTROL),
        Some("public, max-age=31536000, immutable")
    );
}

#[tokio::test]
async fn a_hashed_asset_that_is_not_there_is_not_cached_at_all() {
    // A request that arrives while a deploy is half-done can ask for a chunk this machine does not
    // have yet. Answering "gone, and remember that" would leave a client the next deploy cannot
    // reach. Absence is a fact about this moment, so it carries the policy for things that change.
    let served = get("/_next/static/chunk.neverexisted.js").await;
    assert_eq!(served.status, StatusCode::NOT_FOUND);
    assert_eq!(served.header(header::CACHE_CONTROL), Some("no-cache"));
}

#[tokio::test]
async fn a_precompressed_asset_is_served_as_it_was_built() {
    // Compression happens once, in the image build, rather than per request on a shared vCPU.
    // Without this the `.gz` files the build produces would be dead weight.
    let served = get_with(
        "/_next/static/chunk.deadbeef.js",
        &[(header::ACCEPT_ENCODING, "gzip")],
    )
    .await;
    assert_eq!(served.status, StatusCode::OK);
    assert_eq!(served.header(header::CONTENT_ENCODING), Some("gzip"));
    assert_eq!(
        &served.body[..2],
        &[0x1f, 0x8b],
        "the body should be the gzip member itself"
    );
}

#[tokio::test]
async fn a_client_that_cannot_decompress_still_gets_the_asset() {
    let served = get("/_next/static/chunk.deadbeef.js").await;
    assert_eq!(served.status, StatusCode::OK);
    assert_eq!(served.header(header::CONTENT_ENCODING), None);
    assert!(
        served.text().contains("the plain chunk"),
        "{}",
        served.text()
    );
}

#[tokio::test]
async fn an_unknown_path_answers_with_the_not_found_page_and_a_404() {
    // Not a 200. A crawler, a mistyped link and a stale bookmark all deserve the truth, and a soft
    // 404 is how a search engine ends up indexing an error page.
    let served = get("/nothing/here").await;
    assert_eq!(served.status, StatusCode::NOT_FOUND);
    assert!(served.text().contains("no such page"), "{}", served.text());
}

#[tokio::test]
async fn the_not_found_page_is_not_cached() {
    let served = get("/nothing/here").await;
    assert_eq!(served.header(header::CACHE_CONTROL), Some("no-cache"));
}

#[tokio::test]
async fn a_path_climbing_out_of_the_root_is_refused() {
    // The whole filesystem of the container is behind this handler, including the environment the
    // process was started with.
    for path in [
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/_next/static/../../../Cargo.toml",
        "/_next/static/%2e%2e%2f%2e%2e%2f%2e%2e%2fCargo.toml",
    ] {
        let served = get(path).await;
        assert!(
            !served.text().contains("[package]"),
            "{path} escaped the root: {}",
            served.text()
        );
    }
}

#[tokio::test]
async fn a_directory_is_not_listed() {
    // A listing of `_next/static` tells anybody the exact build a deployment is running.
    let served = get("/_next/static").await;
    assert_ne!(served.status, StatusCode::OK);
    assert!(
        !served.text().contains("chunk.deadbeef"),
        "{}",
        served.text()
    );
}
