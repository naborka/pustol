//! What guests send the bot, in as much of Telegram's shape as this system reads.
//!
//! Everything else in an update is ignored rather than modelled, and a part that cannot be read is
//! dropped rather than failing the batch it came in: the batch is confirmed by its highest id, so a
//! single unreadable update would otherwise be fetched again for ever and wedge every one behind it.

use serde::de::{DeserializeOwned, Deserializer};
use serde::Deserialize;

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
    /// Whether this is somebody's own chat with the bot, rather than a group it was added to.
    #[must_use]
    pub fn is_private(&self) -> bool {
        self.kind == "private"
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Sender {
    pub id: i64,
}

/// A tap on a button under one of the bot's messages.
#[derive(Clone, Debug, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: Sender,
    /// The message the button was under. Telegram still names it by chat and id when the bot can
    /// no longer read it.
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<MessageRef>,
    /// Whatever the client sent. A modified client can send anything here.
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
