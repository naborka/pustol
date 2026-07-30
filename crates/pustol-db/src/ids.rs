//! Identities that belong to storage rather than to the reservation rules.

use std::fmt;

use uuid::Uuid;

/// A bar. The domain never needs one — its rules are about a single room at a time — so it lives
/// here, where rows have to be told apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(transparent)]
#[serde(transparent)]
pub struct BarId(pub Uuid);

impl fmt::Display for BarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Telegram's own immutable account number.
///
/// Every authorisation decision is made against this and never against a username, which its
/// owner can release for somebody else to claim.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(transparent)]
#[serde(transparent)]
pub struct TelegramUserId(pub i64);

impl fmt::Display for TelegramUserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
