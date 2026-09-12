//! Who is asking.
//!
//! Authentication is cryptographic and happens in an extractor: a handler cannot run without a
//! payload Telegram signed, or a session one was exchanged for, because the type it needs cannot be
//! built any other way.
//!
//! Authorisation is a separate step, deliberately. It asks the database, by numeric account id,
//! whether this person is on the bar's admin roster — never the username in the payload, which its
//! owner can release for a stranger to claim. A session changes nothing about that: it proves who is
//! asking, and the roster is read on every request.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use chrono::{DateTime, TimeDelta, Utc};
use pustol_db::identity::{TelegramAccount, Viewer};
use pustol_db::ids::TelegramUserId;
use pustol_telegram::init_data::TelegramUser;
use pustol_telegram::session::{issue, verify_session};
use pustol_telegram::{BotToken, verify};

use crate::error::ApiError;
use crate::state::AppState;

/// How long a signed payload stays usable.
///
/// Telegram never expires `initData`, and it travels in the URL the app is launched with, so it is
/// copied wherever that URL goes — a shared screenshot, a proxy log, a browser history. This bound
/// is what stops such a copy authenticating its owner for ever. The app trades the payload for a
/// session on its first request, so an hour costs nobody anything.
pub const MAX_INIT_DATA_AGE: TimeDelta = TimeDelta::hours(1);

/// How long a session lasts, counted from when Telegram signed the payload it came from.
///
/// Longer than the longest bar day the settings allow — open at 08:00, closed at 04:00 — so a shift
/// never ends in "open the app again". A session is kept in the app's memory and sent in a header;
/// anything that can read it there can read a fresh payload just as well.
pub const SESSION_LIFETIME: TimeDelta = TimeDelta::hours(24);

/// The scheme Telegram Mini App backends conventionally use.
const PAYLOAD_SCHEME: &str = "tma ";

/// The scheme for a session this server issued.
const SESSION_SCHEME: &str = "session ";

/// A caller whose identity Telegram signed, directly or through a session.
///
/// Says nothing about what they may do. That is [`Staff`]'s job.
#[derive(Clone, Debug)]
pub struct Authenticated {
    pub user: TelegramUser,
    /// When the proof this caller holds stops being accepted.
    pub expires_at: DateTime<Utc>,
}

impl Authenticated {
    #[must_use]
    pub const fn user_id(&self) -> TelegramUserId {
        TelegramUserId(self.user.id)
    }

    /// The account, in the shape storage records.
    #[must_use]
    pub fn account(&self) -> TelegramAccount {
        TelegramAccount {
            id: self.user_id(),
            username: self.user.username.clone(),
            first_name: self.user.first_name.clone(),
            last_name: self.user.last_name.clone(),
            language_code: self.user.language_code.clone(),
        }
    }

    /// A session for this caller, ending when their current proof does.
    ///
    /// Issued again from a session it comes out the same, so no chain of sessions outlives the
    /// payload the first one was exchanged for.
    #[must_use]
    pub fn session(&self, token: &BotToken) -> String {
        issue(&self.user, self.expires_at, token)
    }
}

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                ApiError::unauthorised("no_credentials", "this request carries no Telegram payload")
            })?;

        if let Some(payload) = header.strip_prefix(PAYLOAD_SCHEME) {
            let verified = verify(payload, state.bot_token(), state.now(), MAX_INIT_DATA_AGE)?;
            return Ok(Self {
                expires_at: verified.auth_date + SESSION_LIFETIME,
                user: verified.user,
            });
        }
        if let Some(session) = header.strip_prefix(SESSION_SCHEME) {
            let verified = verify_session(session, state.bot_token(), state.now())?;
            return Ok(Self {
                user: verified.user,
                expires_at: verified.expires_at,
            });
        }
        Err(ApiError::unauthorised(
            "no_credentials",
            "expected an Authorization header of the form `tma <initData>` or `session <token>`",
        ))
    }
}

/// A caller who is on the bar's admin roster.
///
/// Handlers that must not be reachable by a guest take this instead of [`Authenticated`], so the
/// check is made by the type signature rather than by a line somebody could forget to write.
#[derive(Clone, Debug)]
pub struct Staff {
    pub viewer: Viewer,
}

impl FromRequestParts<AppState> for Staff {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let authenticated = Authenticated::from_request_parts(parts, state).await?;
        // Recording the account here is what lets an invitation take effect on somebody's first
        // visit rather than after a cache expires.
        let viewer = state
            .store
            .identify(state.bar, &authenticated.account(), state.now())
            .await?;
        if !viewer.is_staff {
            return Err(ApiError::forbidden(
                "this account is not on the bar's admin list",
            ));
        }
        Ok(Self { viewer })
    }
}
