//! The HTTP layer.
//!
//! Deliberately thin. Every decision that could be wrong lives in `pustol-domain`, and every
//! decision that has to be transactional lives in `pustol-db`; this crate authenticates the caller,
//! translates JSON into those calls and translates the result back. A handler long enough to hide a
//! rule in is a handler that has taken work from a layer that could test it properly.

pub mod assets;
pub mod auth;
pub mod body;
pub mod boot;
pub mod callbacks;
pub mod dto;
pub mod error;
pub mod inbox;
pub mod params;
pub mod routes;
pub mod state;
pub mod worker;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, header};
use axum::routing::{any, get};
use pustol_domain::LIMITS;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};

pub use assets::Assets;
pub use boot::{bind_address, interrupt_signal};
pub use state::{AppState, Clock};

use crate::error::ApiError;

/// Fits largest legal settings save, derived from [`LIMITS`] at worst-case JSON width; else a bar
/// at limits could never save again. Unbounded body lets one request exhaust memory.
fn max_body_bytes() -> usize {
    /// Worst JSON escape of one character: `\ud83c\udf7a`, surrogate pair past U+FFFF.
    const WIDEST_CHARACTER: usize = 12;
    /// Per list entry overhead: quotes, keys, id, number, punctuation.
    const ENTRY: usize = 128;
    /// Week, numbers, version, timezone, contact.
    const REST: usize = 16 * 1024;
    let (text, lists) = (LIMITS.text, LIMITS.lists);
    let username = usize::try_from(LIMITS.staff_username_length.max).unwrap_or(usize::MAX);
    let entries = |count: usize, characters: usize| count * (characters * WIDEST_CHARACTER + ENTRY);
    REST + (text.name + text.address) * WIDEST_CHARACTER
        + entries(lists.message_templates, text.message)
        + entries(lists.cancel_reasons, text.reason)
        + entries(lists.zones, text.zone)
        + entries(lists.tables, text.zone)
        + entries(lists.staff, username)
}

/// Not `X-Frame-Options: DENY`: app must load in frame inside web.telegram.org.
const FRAME_ANCESTORS: HeaderValue = HeaderValue::from_static(
    "frame-ancestors 'self' https://web.telegram.org https://*.telegram.org",
);

/// API plus app build when given; one place so compression, headers, access log cover both.
pub fn router(state: AppState, assets: Option<Assets>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/api", any(no_such_endpoint))
        .route("/api/", any(no_such_endpoint))
        .route("/api/admin", any(no_such_endpoint))
        .route("/api/admin/", any(no_such_endpoint))
        .nest(
            "/api",
            routes::guest::routes()
                .fallback(no_such_endpoint)
                .method_not_allowed_fallback(wrong_method),
        )
        .nest(
            "/api/admin",
            routes::admin::routes()
                .fallback(no_such_endpoint)
                .method_not_allowed_fallback(wrong_method),
        )
        .layer(DefaultBodyLimit::max(max_body_bytes()))
        .with_state(state);
    // Routes beat fallback: `/health` and `/api` stay API whatever app build contains.
    let app = match assets {
        Some(assets) => api.fallback_service(assets.into_router()),
        None => api,
    };
    app.layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            FRAME_ANCESTORS,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(tracing::Level::INFO))
                .on_response(DefaultOnResponse::new().level(tracing::Level::INFO)),
        )
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

/// JSON code lets stale app, opened before method removal, tell user to reopen.
async fn wrong_method() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "this path does not take that method",
    )
}
