//! Authentication in extractors: handler type only built from signed Telegram payload or session.
//!
//! Authorisation separate: roster checked by numeric id every request, never by username, which
//! owner can release for stranger to claim.

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

/// Telegram never expires `initData` and it leaks with launch URL (screenshots, proxy logs,
/// history); this bound stops leaked copy working forever. App swaps it for session at once.
pub const MAX_INIT_DATA_AGE: TimeDelta = TimeDelta::hours(1);

/// Counted from payload signing time. Exceeds longest allowed bar day (08:00 to 04:00), so no
/// re-login mid-shift. Session lives in app memory; whoever reads it there reads fresh payload too.
pub const SESSION_LIFETIME: TimeDelta = TimeDelta::hours(24);

/// The scheme Telegram Mini App backends conventionally use.
const PAYLOAD_SCHEME: &str = "tma ";

const SESSION_SCHEME: &str = "session ";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proof {
    /// Profile as of `signed_at`, up to [`MAX_INIT_DATA_AGE`] old.
    Telegram { signed_at: DateTime<Utc> },
    /// Server-issued; profile up to [`SESSION_LIFETIME`] old.
    Session,
}

/// Identity only, no permissions; see [`Staff`].
#[derive(Clone, Debug)]
pub struct Authenticated {
    user: TelegramUser,
    proof: Proof,
    expires_at: DateTime<Utc>,
}

impl Authenticated {
    #[must_use]
    pub const fn user_id(&self) -> TelegramUserId {
        TelegramUserId(self.user.id)
    }

    /// Only payload may rewrite stored profile or claim staff seat by username; session never.
    ///
    /// # Errors
    ///
    /// Database failure.
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

    /// Ends when current proof ends, so reissued sessions never outlive original payload.
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

/// Refuses what account row cannot hold: id not positive, blank first name, U+0000 in any string.
///
/// `trim` strips every Unicode space, more than row check, so database check never fires first.
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
