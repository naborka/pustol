//! Subset of Telegram update shape this system reads.
//!
//! Unreadable part is dropped, never fails batch: batch is confirmed by highest id, so one bad
//! update would be refetched for ever and block all after it.

use chrono::TimeDelta;
use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer};

/// How long claim on update id proves update was handled.
///
/// Telegram keeps unfetched update one day, and after quiet week restarts ids from random number.
/// Claim younger than day names update Telegram may resend; older one may collide with new update.
pub const UPDATE_RETENTION: TimeDelta = TimeDelta::days(1);

#[derive(Clone, Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<Message>,
    #[serde(default, deserialize_with = "lenient")]
    pub callback_query: Option<CallbackQuery>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub chat: Chat,
    #[serde(default)]
    pub from: Option<Sender>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
}

impl Chat {
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.kind == "private"
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Sender {
    pub id: i64,
}

/// Tap on inline button under bot message.
#[derive(Clone, Debug, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: Sender,
    /// Telegram names message by chat and id even when bot can no longer read it.
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<MessageRef>,
    /// Untrusted: modified client can send anything.
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MessageRef {
    pub message_id: i64,
    pub chat: Chat,
}

fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}
