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
use pustol_db::identity::{Signature, TelegramAccount, Viewer};
use pustol_db::ids::TelegramUserId;
use pustol_telegram::init_data::{CLOCK_SKEW, TelegramUser};
use pustol_telegram::session::{issue, verify_session};
use pustol_telegram::{BotToken, verify};

use crate::body::nul_refused;
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

/// How a caller proved who they are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proof {
    /// A payload Telegram signed at `signed_at`, within [`MAX_INIT_DATA_AGE`]: its profile is as it
    /// was then, which may be that long ago.
    Telegram { signed_at: DateTime<Utc> },
    /// A session this server issued: its profile is as it was when Telegram signed, up to
    /// [`SESSION_LIFETIME`] ago.
    Session,
}

/// A caller whose identity Telegram signed, directly or through a session.
///
/// Says nothing about what they may do. That is [`Staff`]'s job.
#[derive(Clone, Debug)]
pub struct Authenticated {
    user: TelegramUser,
    proof: Proof,
    /// When the proof this caller holds stops being accepted.
    expires_at: DateTime<Utc>,
}

impl Authenticated {
    #[must_use]
    pub const fn user_id(&self) -> TelegramUserId {
        TelegramUserId(self.user.id)
    }

    /// The account as storage knows it, and what it may do.
    ///
    /// The one way a handler learns about the caller beyond their id. Only a payload may rewrite the
    /// stored profile or claim a staff seat by username, and only as of when Telegram signed it; a
    /// session does neither.
    pub async fn viewer(&self, state: &AppState) -> Result<Viewer, ApiError> {
        let account = TelegramAccount {
            id: self.user_id(),
            username: self.user.username.clone(),
            first_name: self.user.first_name.clone(),
            last_name: self.user.last_name.clone(),
            language_code: self.user.language_code.clone(),
        };
        let now = state.now();
        Ok(match self.proof {
            Proof::Telegram { signed_at } => {
                let signature = Signature {
                    stamped_at: signed_at,
                    clock_skew: CLOCK_SKEW,
                };
                state
                    .store
                    .identify(state.bar, &account, signature, now)
                    .await?
            }
            Proof::Session => state.store.recognise(state.bar, &account, now).await?,
        })
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
            refuse_unstorable(&verified.user)?;
            return Ok(Self {
                user: verified.user,
                proof: Proof::Telegram {
                    signed_at: verified.auth_date,
                },
                expires_at: verified.auth_date + SESSION_LIFETIME,
            });
        }
        if let Some(session) = header.strip_prefix(SESSION_SCHEME) {
            let verified = verify_session(session, state.bot_token(), state.now())?;
            refuse_unstorable(&verified.user)?;
            return Ok(Self {
                user: verified.user,
                proof: Proof::Session,
                expires_at: verified.expires_at,
            });
        }
        Err(ApiError::unauthorised(
            "no_credentials",
            "expected an Authorization header of the form `tma <initData>` or `session <token>`",
        ))
    }
}

/// Refuses a profile the account row cannot hold, as `text_invalid`: an id that is not positive, a
/// first name that is blank, or U+0000 in any of its strings, which `PostgreSQL` text cannot hold.
///
/// Blank is judged by every Unicode space, more than the row's own check trims, so the row's check
/// is never the one that refuses.
fn refuse_unstorable(user: &TelegramUser) -> Result<(), ApiError> {
    if user.id <= 0 {
        return Err(ApiError::bad_request(
            "text_invalid",
            "the Telegram profile's id is not a positive number",
        ));
    }
    if user.first_name.trim().is_empty() {
        return Err(ApiError::bad_request(
            "text_invalid",
            "the Telegram profile's first name is blank",
        ));
    }
    let texts = [
        Some(&user.first_name),
        user.last_name.as_ref(),
        user.username.as_ref(),
        user.language_code.as_ref(),
        user.photo_url.as_ref(),
    ];
    if texts.into_iter().flatten().any(|text| text.contains('\0')) {
        return Err(nul_refused("the Telegram profile"));
    }
    Ok(())
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
        let viewer = authenticated.viewer(state).await?;
        if !viewer.is_staff {
            return Err(ApiError::forbidden(
                "this account is not on the bar's admin list",
            ));
        }
        Ok(Self { viewer })
    }
}
