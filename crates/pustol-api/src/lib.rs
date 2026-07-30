//! The HTTP layer.
//!
//! Deliberately thin. Every decision that could be wrong lives in `pustol-domain`, and every
//! decision that has to be transactional lives in `pustol-db`; this crate authenticates the caller,
//! translates JSON into those calls and translates the result back. A handler long enough to hide a
//! rule in is a handler that has taken work from a layer that could test it properly.

pub mod assets;
pub mod auth;
pub mod dto;
pub mod error;
pub mod routes;
pub mod state;
pub mod worker;

use axum::Router;
use axum::routing::get;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

pub use assets::Assets;
pub use state::{AppState, Clock};

use crate::error::ApiError;

/// Largest request this API will read.
///
/// A settings save is the biggest thing anybody sends and it is a few kilobytes. Without a bound,
/// one request can make the process allocate until it dies.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Builds the router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .nest("/api", routes::guest::routes().fallback(no_such_endpoint))
        .nest(
            "/api/admin",
            routes::admin::routes().fallback(no_such_endpoint),
        )
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

/// A path under `/api` that no handler claims.
///
/// The same process serves the app's static build, whose fallback answers HTML. Without this, a
/// renamed endpoint would reach that fallback and `api.ts` — which reads a code out of a JSON body
/// — would report a parse error instead of the 404 that happened.
async fn no_such_endpoint() -> ApiError {
    ApiError::not_found("endpoint")
}
