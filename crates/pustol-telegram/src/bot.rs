//! Talking to guests through the bot.
//!
//! The bot is the only way the bar reaches a guest who is not looking at the app. Telegram gives
//! no way to ask whether a chat exists, so the outcome of a send is the only evidence there is —
//! which is why the failures here are classified by whether retrying could ever help.

use serde::Serialize;

use crate::init_data::BotToken;

/// Where an outgoing message failed, and whether trying again could help.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    /// The guest has blocked the bot, deleted their account, or never started a chat. No number of
    /// retries changes any of those, and the queue must stop rather than grind.
    #[error("telegram will not deliver to this account: {description}")]
    Unreachable { description: String },
    /// Telegram asked for a pause. The delay it named is honoured rather than guessed at.
    #[error("telegram asked to wait {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: i64 },
    /// Something transient: a network blip, a 5xx.
    #[error("telegram could not be reached: {0}")]
    Transient(String),
    /// A request Telegram refused on its merits — a malformed message, a bad token. Retrying sends
    /// the same broken request again, so it is terminal, but it is a bug rather than a fact about
    /// the guest.
    #[error("telegram refused the request: {description}")]
    Refused { description: String },
}

impl SendError {
    /// Whether trying again later could plausibly succeed.
    #[must_use]
    pub const fn is_worth_retrying(&self) -> bool {
        matches!(self, Self::RateLimited { .. } | Self::Transient(_))
    }

    /// Whether this says the guest cannot be reached at all, which the bar should remember.
    #[must_use]
    pub const fn means_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable { .. })
    }
}

/// A button under a message.
#[derive(Clone, Debug, Serialize)]
pub struct CallbackButton {
    pub text: String,
    pub callback_data: String,
}

/// The bot API, as much of it as this system needs.
#[derive(Clone, Debug)]
pub struct Bot {
    client: reqwest::Client,
    token: BotToken,
    base_url: String,
}

impl Bot {
    /// Builds a client against Telegram.
    pub fn new(token: BotToken, client: reqwest::Client) -> Self {
        Self {
            client,
            token,
            base_url: "https://api.telegram.org".to_owned(),
        }
    }

    /// Points the client at another host, so the send path can be tested end to end against a stub
    /// rather than mocked out and assumed.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Sends a message, optionally with buttons under it.
    ///
    /// The reminder carries a "cannot make it" button, which is the whole point of reminding
    /// somebody: a guest who can cancel in one tap does, and the bar gets the table back.
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        buttons: &[CallbackButton],
    ) -> Result<(), SendError> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "disable_notification": false,
        });
        if !buttons.is_empty() {
            body["reply_markup"] = serde_json::json!({
                "inline_keyboard": [buttons],
            });
        }

        let response = self
            .client
            .post(format!(
                "{}/bot{}/sendMessage",
                self.base_url,
                self.token.expose()
            ))
            .json(&body)
            .send()
            .await
            .map_err(|error| SendError::Transient(error.to_string()))?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }

        let payload: ApiError = response.json().await.unwrap_or_else(|_| ApiError {
            description: format!("HTTP {status}"),
            parameters: None,
        });
        Err(classify(status, &payload))
    }
}

#[derive(Debug, serde::Deserialize)]
struct ApiError {
    #[serde(default)]
    description: String,
    #[serde(default)]
    parameters: Option<ApiErrorParameters>,
}

#[derive(Debug, serde::Deserialize)]
struct ApiErrorParameters {
    #[serde(default)]
    retry_after: Option<i64>,
}

/// Phrases Telegram uses when an account cannot be reached at all.
const UNREACHABLE: [&str; 6] = [
    "bot was blocked by the user",
    "user is deactivated",
    "chat not found",
    "bot can't initiate conversation with a user",
    "the group chat was deleted",
    "forbidden",
];

/// Sorts a Telegram refusal into something the queue can act on.
///
/// Telegram signals most of what matters in the description text rather than in the status code, so
/// the phrases it uses are matched explicitly. An unrecognised 4xx is treated as refused rather
/// than transient: retrying a request Telegram has already rejected on its merits only repeats it.
fn classify(status: reqwest::StatusCode, payload: &ApiError) -> SendError {
    let description = payload.description.clone();
    let lowered = description.to_lowercase();

    if let Some(seconds) = payload.parameters.as_ref().and_then(|it| it.retry_after) {
        return SendError::RateLimited {
            retry_after_seconds: seconds,
        };
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return SendError::RateLimited {
            retry_after_seconds: 30,
        };
    }
    if UNREACHABLE.iter().any(|phrase| lowered.contains(phrase)) {
        return SendError::Unreachable { description };
    }
    if status.is_server_error() {
        return SendError::Transient(description);
    }
    SendError::Refused { description }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(description: &str, retry_after: Option<i64>) -> ApiError {
        ApiError {
            description: description.to_owned(),
            parameters: retry_after.map(|retry_after| ApiErrorParameters {
                retry_after: Some(retry_after),
            }),
        }
    }

    #[test]
    fn a_blocked_bot_is_permanent_and_worth_remembering() {
        let failure = classify(
            reqwest::StatusCode::FORBIDDEN,
            &error("Forbidden: bot was blocked by the user", None),
        );
        assert!(failure.means_unreachable());
        assert!(!failure.is_worth_retrying());
    }

    #[test]
    fn a_guest_who_never_started_the_bot_is_unreachable_rather_than_broken() {
        let failure = classify(
            reqwest::StatusCode::FORBIDDEN,
            &error(
                "Forbidden: bot can't initiate conversation with a user",
                None,
            ),
        );
        assert!(failure.means_unreachable());
    }

    #[test]
    fn a_deactivated_account_is_unreachable() {
        let failure = classify(
            reqwest::StatusCode::FORBIDDEN,
            &error("Forbidden: user is deactivated", None),
        );
        assert!(failure.means_unreachable());
    }

    #[test]
    fn a_rate_limit_honours_the_delay_telegram_named() {
        let failure = classify(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            &error("Too Many Requests: retry after 42", Some(42)),
        );
        assert!(matches!(
            failure,
            SendError::RateLimited {
                retry_after_seconds: 42
            }
        ));
        assert!(failure.is_worth_retrying());
        assert!(!failure.means_unreachable());
    }

    #[test]
    fn a_rate_limit_without_a_named_delay_still_waits() {
        let failure = classify(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            &error("Too Many Requests", None),
        );
        assert!(matches!(failure, SendError::RateLimited { .. }));
    }

    #[test]
    fn a_server_fault_is_transient() {
        let failure = classify(
            reqwest::StatusCode::BAD_GATEWAY,
            &error("Bad Gateway", None),
        );
        assert!(failure.is_worth_retrying());
        assert!(!failure.means_unreachable());
    }

    #[test]
    fn an_unrecognised_refusal_is_terminal_but_is_not_blamed_on_the_guest() {
        let failure = classify(
            reqwest::StatusCode::BAD_REQUEST,
            &error("Bad Request: message text is empty", None),
        );
        assert!(!failure.is_worth_retrying());
        assert!(
            !failure.means_unreachable(),
            "a broken request must not mark a reachable guest unreachable"
        );
        assert!(matches!(failure, SendError::Refused { .. }));
    }
}
