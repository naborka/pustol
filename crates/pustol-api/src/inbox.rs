//! Answering what guests send the bot.
//!
//! The outbox is the bot talking; this is the bot listening. Long polling rather than a webhook: it
//! needs no public address, no secret and no registration with Telegram, so it runs the same on a
//! laptop as in production, and the one process stays the only thing that has to be running.

use std::time::Duration;

use pustol_db::{BarId, Store, TelegramUserId};
use pustol_telegram::updates::{CallbackQuery, Message};
use pustol_telegram::{Bot, SendError, Update, messages};

use crate::callbacks::{self, Callback};
use crate::state::Clock;

/// How long one request waits for something to arrive before asking again.
const LONG_POLL: Duration = Duration::from_secs(25);

/// How long to wait after Telegram could not be reached.
const AFTER_FAILURE: Duration = Duration::from_secs(5);

/// How long to wait after Telegram refused to hand updates over.
///
/// It refuses when another process polls the same bot or a webhook is set for it. Neither goes away
/// in seconds, and asking every five would fill the log with the same line.
const AFTER_REFUSAL: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct Inbox {
    pub store: Store,
    pub bot: Bot,
    pub bar: BarId,
    pub clock: Clock,
}

impl Inbox {
    /// Runs until the process is asked to stop.
    ///
    /// Only the wait for Telegram is cut short by a stop. An update already in hand is answered in
    /// full, so a guest's tap is never left cancelled but unanswered.
    pub async fn run(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut offset = None;
        loop {
            let polled = tokio::select! {
                polled = self.bot.get_updates(offset, LONG_POLL) => polled,
                _ = shutdown.changed() => return,
            };
            let pause = match polled {
                Ok(updates) => {
                    offset = self.handle_all(updates, offset).await;
                    continue;
                }
                Err(error) => {
                    tracing::error!(%error, "could not read what was sent to the bot");
                    if error.is_worth_retrying() {
                        AFTER_FAILURE
                    } else {
                        AFTER_REFUSAL
                    }
                }
            };
            tokio::select! {
                () = tokio::time::sleep(pause) => {}
                _ = shutdown.changed() => return,
            }
        }
    }

    /// Fetches once and answers everything fetched. Returns the offset to ask from next.
    pub async fn poll_once(&self, offset: Option<i64>) -> Result<Option<i64>, SendError> {
        let updates = self.bot.get_updates(offset, LONG_POLL).await?;
        Ok(self.handle_all(updates, offset).await)
    }

    /// Answers each update and moves the offset past it, whether or not answering worked: an update
    /// that fails every time must not be fetched again for ever.
    async fn handle_all(&self, updates: Vec<Update>, offset: Option<i64>) -> Option<i64> {
        let mut next = offset;
        for update in updates {
            let after = update.update_id + 1;
            self.handle(update).await;
            next = Some(next.map_or(after, |current| current.max(after)));
        }
        next
    }

    /// Answers one update. Failures are logged; there is nobody else to tell.
    pub async fn handle(&self, update: Update) {
        if let Some(query) = update.callback_query {
            self.answer_tap(query).await;
        } else if let Some(message) = update.message {
            self.answer_message(message).await;
        }
    }

    async fn answer_tap(&self, query: CallbackQuery) {
        let asked = query.data.as_deref().and_then(callbacks::parse);
        let (reply, done) = match asked {
            Some(Callback::CancelBooking(booking)) => {
                match self
                    .store
                    .cancel_reminded_booking(
                        self.bar,
                        TelegramUserId(query.from.id),
                        booking,
                        self.clock.now(),
                    )
                    .await
                {
                    Ok(_) => (messages::CANCELLED_FROM_REMINDER, true),
                    Err(pustol_db::Error::NotFound { .. }) => (messages::NO_LONGER_ACTIVE, true),
                    Err(error) => {
                        tracing::error!(%error, "could not cancel a booking from its reminder");
                        (messages::COULD_NOT_CANCEL, false)
                    }
                }
            }
            None => (messages::NO_LONGER_ACTIVE, true),
        };

        if let Err(error) = self.bot.answer_callback_query(&query.id, reply).await {
            tracing::warn!(%error, "could not answer a tap");
        }
        // Left in place when cancelling failed, so the guest can try again.
        if done
            && let Some(message) = &query.message
            && let Err(error) = self
                .bot
                .remove_buttons(message.chat.id, message.message_id)
                .await
        {
            tracing::warn!(%error, "could not take a spent button away");
        }
    }

    async fn answer_message(&self, message: Message) {
        if !message.chat.is_private() {
            return;
        }
        let config = match self.store.config(self.bar).await {
            Ok(config) => config,
            Err(error) => {
                tracing::error!(%error, "could not read the bar to answer a message");
                return;
            }
        };
        let reply = match message.text.as_deref().and_then(start_payload) {
            Some(payload) => {
                // Starting the bot is the one thing that makes a guest reachable again after they
                // blocked it, and it is evidence in exactly the way a delivery is.
                if let Some(from) = &message.from
                    && let Err(error) = self.store.set_reachable(TelegramUserId(from.id), true).await
                {
                    tracing::warn!(%error, "could not record that a guest started the bot");
                }
                if payload == "reminders" {
                    messages::reminders_on(config.remind_hours)
                } else {
                    messages::welcome(&config.name)
                }
            }
            None => messages::nobody_reads_this(&config.name),
        };
        if let Err(error) = self.bot.send_message(message.chat.id, &reply, &[]).await {
            tracing::warn!(%error, "could not answer a message");
        }
    }
}

/// The deep-link payload of a `/start` command, empty when there is none.
fn start_payload(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("/start")?;
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix(' ').map(str::trim)
}
