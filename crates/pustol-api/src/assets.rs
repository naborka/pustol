//! Serving the app's static build.
//!
//! The Mini App and the API answer on one origin. A Telegram Mini App runs inside a `WebView` whose
//! page came from this host, so a second origin would need CORS, would put the API's hostname in
//! the client bundle, and would break the moment a network blocked the second name.
//!
//! Proxying the API behind the app's server was one way to get that. Serving the build from the
//! process that *is* the API is the other, and it is the one where two origins have no
//! representation: there is a single listener, so there is nothing to configure inconsistently.
//!
//! The app is a static export — every route is prerendered and nothing is decided per request —
//! so what is left is a file server with two cache policies and an honest 404.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::set_status::SetStatus;

/// Where the build's content-hashed files live, relative to the root.
const HASHED: &str = "/_next/static";

/// What a build names a file it is willing to serve as the app itself.
const INDEX: &str = "index.html";

/// The page for a path the build does not have.
const NOT_FOUND: &str = "404.html";

/// What a name whose bytes can change under it must carry.
///
/// `no-cache` permits storing and requires revalidating. `index.html` points at content-hashed
/// chunks, and a cached copy of it outlives the deploy that deleted them — leaving a white screen
/// inside a `WebView` the guest cannot force-refresh.
const REVALIDATE: HeaderValue = HeaderValue::from_static("no-cache");

/// What a content-hashed name may carry, when there was something behind it.
const FOR_EVER: HeaderValue = HeaderValue::from_static("public, max-age=31536000, immutable");

/// A directory that has been shown to hold a built app.
///
/// The check happens once, when this is constructed, so that pointing the server at the wrong
/// directory is a refusal to boot rather than a bar whose staff find a white screen.
#[derive(Clone, Debug)]
pub struct Assets {
    root: PathBuf,
}

/// Why a directory is not a built app.
#[derive(Debug, thiserror::Error)]
pub enum AssetsError {
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    #[error("{} has no {INDEX}, so it is not a built app", .0.display())]
    NoIndex(PathBuf),
}

impl Assets {
    /// Checks that `root` is a built app.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, AssetsError> {
        let root = root.into();
        if !root.is_dir() {
            return Err(AssetsError::NotADirectory(root));
        }
        if !root.join(INDEX).is_file() {
            return Err(AssetsError::NoIndex(root));
        }
        Ok(Self { root })
    }

    /// The routes that serve it.
    pub fn into_router(self) -> Router {
        // Two policies, because the build has two kinds of file and caching them alike breaks one
        // of them.
        let hashed = Router::new()
            .nest_service(HASHED, files(self.root.join("_next").join("static")))
            .layer(axum::middleware::map_response(cache_a_hashed_name));

        let rest = Router::new()
            .fallback_service(files(&self.root).not_found_service(SetStatus::new(
                ServeFile::new(self.root.join(NOT_FOUND)),
                // `ServeFile` answers 200 for a file it found, which for this file would be a soft
                // 404: the truth is that the path does not exist.
                StatusCode::NOT_FOUND,
            )))
            .layer(SetResponseHeaderLayer::overriding(
                CACHE_CONTROL,
                REVALIDATE,
            ));

        hashed.merge(rest)
    }
}

fn files(root: impl AsRef<Path>) -> ServeDir {
    ServeDir::new(root)
}

/// How long a client may keep what a content-hashed name gave it.
///
/// A hashed name is immutable because its bytes decide it — but only once there are bytes. "There
/// is no such file" is a fact about this moment, and a request that arrives while a deploy is half
/// done would otherwise have its 404 remembered for a year by a client the next deploy cannot
/// reach.
async fn cache_a_hashed_name(mut response: Response) -> Response {
    let policy = if response.status().is_success() {
        FOR_EVER
    } else {
        REVALIDATE
    };
    response.headers_mut().insert(CACHE_CONTROL, policy);
    response
}
