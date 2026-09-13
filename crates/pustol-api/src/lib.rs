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

/// The largest request body this API reads, in bytes: room for the largest settings save the limits
/// allow.
///
/// A settings save is the biggest thing anybody sends, and every legal one has to fit, or a bar whose
/// settings grew to the limits could never save them again. Counted from [`LIMITS`], each character
/// as wide as JSON ever writes one a text may hold, so widening a limit widens this with it. Without
/// a bound, one request can make the process allocate until it dies.
///
/// Enforced by the extractor that reads the body, so a body over it is refused as `body_invalid`
/// JSON like every other body refusal, whether or not the request said how long it was.
fn max_body_bytes() -> usize {
    /// The most bytes JSON writes one character of a text in: `\ud83c\udf7a`, a character past U+FFFF
    /// escaped as its pair of surrogates.
    const WIDEST_CHARACTER: usize = 12;
    /// Room around one entry of a list: quotes, keys, an identity, a number, punctuation.
    const ENTRY: usize = 128;
    /// Room for everything else a save carries: the week, the numbers, the version, the timezone and
    /// the contact.
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

/// Who may show the app in a frame: this origin and Telegram's web clients.
///
/// Not `X-Frame-Options: DENY`, which would break the app inside web.telegram.org.
const FRAME_ANCESTORS: HeaderValue =
    HeaderValue::from_static("frame-ancestors 'self' https://web.telegram.org https://*.telegram.org");

/// Builds the whole process: the API, and the app's build when there is one to serve.
///
/// Composed in one place so that what every answer carries — compression, the headers below, the
/// access log — covers the API and the app's files alike, rather than whichever was wired first.
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
    // Routes win over a fallback, so `/health` and everything under `/api` keep answering as the
    // API however the app's build is laid out.
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

/// A path under `/api` that exists, asked with a method it does not take — usually an app opened
/// before that method was removed, which can only be told to reopen when the refusal carries a code.
async fn wrong_method() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "this path does not take that method",
    )
}
