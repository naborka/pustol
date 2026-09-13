//! What a fresh Telegram payload is exchanged for.
//!
//! Telegram hands the app its signed payload once, when it opens, and never again while it stays
//! open. The payload is accepted for an hour, because it travels in the launch URL and is copied
//! wherever that URL is. A shift is longer than an hour, so the first request trades a fresh payload
//! for a session: the same account, signed by this server, ending a day after Telegram signed the
//! payload. The session lives in the app's memory and in a header, and nowhere a URL goes.
//!
//! Written as `claims.signature`, both hex. The signature covers the claims exactly as sent, so there
//! is no second encoding to disagree with.

use chrono::{DateTime, Utc};
use hmac::{KeyInit, Mac};
use subtle::ConstantTimeEq;

use crate::init_data::{BotToken, HmacSha256, TelegramUser, VerifyError};

/// A session this server issued, verified.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    pub user: TelegramUser,
    pub expires_at: DateTime<Utc>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Claims {
    user: TelegramUser,
    /// When the session ends, as a unix timestamp.
    exp: i64,
}

/// Signs a session for `user` that ends at `expires_at`.
#[must_use]
pub fn issue(user: &TelegramUser, expires_at: DateTime<Utc>, token: &BotToken) -> String {
    let claims = Claims {
        user: user.clone(),
        exp: expires_at.timestamp(),
    };
    let claims = hex::encode(serde_json::to_vec(&claims).expect("claims are plain data"));
    let signature = hex::encode(sign(&claims, token));
    format!("{claims}.{signature}")
}

/// Verifies a session and returns who it is for.
pub fn verify_session(
    text: &str,
    token: &BotToken,
    now: DateTime<Utc>,
) -> Result<Session, VerifyError> {
    let (claims, signature) = text.split_once('.').ok_or(VerifyError::BadSignature)?;
    let signature = hex::decode(signature).map_err(|_| VerifyError::BadSignature)?;
    if !bool::from(sign(claims, token).as_slice().ct_eq(&signature)) {
        return Err(VerifyError::BadSignature);
    }
    // Signed by this server, so a claim that does not read is a bug here rather than an attack.
    let claims: Claims = hex::decode(claims)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| VerifyError::MalformedUser("the session's claims do not read".to_owned()))?;
    let expires_at = DateTime::from_timestamp(claims.exp, 0)
        .ok_or_else(|| VerifyError::MalformedAuthDate(claims.exp.to_string()))?;
    if now >= expires_at {
        return Err(VerifyError::SessionEnded);
    }
    Ok(Session {
        user: claims.user,
        expires_at,
    })
}

fn sign(claims: &str, token: &BotToken) -> [u8; 32] {
    let mut mac =
        HmacSha256::new_from_slice(&token.session_key).expect("hmac accepts any key length");
    mac.update(claims.as_bytes());
    mac.finalize().into_bytes().into()
}
