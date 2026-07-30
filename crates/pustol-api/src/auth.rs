//! Who is asking.
//!
//! Authentication is cryptographic and happens in an extractor: a handler cannot run without a
//! payload Telegram signed, because the type it needs cannot be built any other way.
//!
//! Authorisation is a separate step, deliberately. It asks the database, by numeric account id,
//! whether this person is on the bar's admin roster — never the username in the payload, which its
//! owner can release for a stranger to claim.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use chrono::TimeDelta;
use pustol_db::identity::{TelegramAccount, Viewer};
use pustol_db::ids::TelegramUserId;
use pustol_telegram::{InitData, verify};

use crate::error::ApiError;
use crate::state::AppState;

/// How long a signed payload stays usable.
///
/// Telegram never expires `initData`, so this bound is the only thing that stops a payload captured
/// once — a shared screenshot, a proxy log, a browser history — from authenticating its owner for
/// ever. The Mini App refreshes it on every launch, so an hour costs a guest nothing.
pub const MAX_INIT_DATA_AGE: TimeDelta = TimeDelta::hours(1);

/// The scheme Telegram Mini App backends conventionally use.
const SCHEME: &str = "tma ";

/// A caller whose payload Telegram signed.
///
/// Says nothing about what they may do. That is [`Staff`]'s job.
#[derive(Clone, Debug)]
pub struct Authenticated {
    pub init_data: InitData,
}

impl Authenticated {
    #[must_use]
    pub const fn user_id(&self) -> TelegramUserId {
        TelegramUserId(self.init_data.user.id)
    }

    /// The account, in the shape storage records.
    #[must_use]
    pub fn account(&self) -> TelegramAccount {
        TelegramAccount {
            id: self.user_id(),
            username: self.init_data.user.username.clone(),
            first_name: self.init_data.user.first_name.clone(),
            last_name: self.init_data.user.last_name.clone(),
            language_code: self.init_data.user.language_code.clone(),
        }
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
        let init_data = header.strip_prefix(SCHEME).ok_or_else(|| {
            ApiError::unauthorised(
                "no_credentials",
                "expected an Authorization header of the form `tma <initData>`",
            )
        })?;

        let init_data = verify(
            init_data,
            state.bot_token(),
            state.now(),
            MAX_INIT_DATA_AGE,
        )?;
        Ok(Self { init_data })
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
