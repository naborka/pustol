//! Day-long session traded for fresh Telegram payload.
//!
//! Telegram gives signed payload only once, at app open. Payload rides launch URL, so accepted one
//! hour only; shift is longer. Session is kept in app memory and header, never in URL.
//!
//! Format `claims.signature`, both hex. Signature covers claims bytes as sent: no second encoding.

use chrono::{DateTime, Utc};
use hmac::{KeyInit, Mac};
use subtle::ConstantTimeEq;

use crate::init_data::{BotToken, HmacSha256, TelegramUser, VerifyError};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    pub user: TelegramUser,
    pub expires_at: DateTime<Utc>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Claims {
    user: TelegramUser,
    /// Unix timestamp.
    exp: i64,
}

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

/// # Errors
///
/// `BadSignature` when not signed by this token, `SessionEnded` at or past expiry, `MalformedUser`
/// or `MalformedAuthDate` when claims do not parse.
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
    // Signature valid, so unreadable claims mean bug here, not attack.
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
