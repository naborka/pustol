//! Answering what guests send the bot.
//!
//! The outbox is the bot talking; this is the bot listening. Long polling rather than a webhook: it
//! needs no public address, no secret and no registration with Telegram, so it runs the same on a
//! laptop as in production, and the one process stays the only thing that has to be running.

use std::time::Duration;

use pustol_db::{BarId, Store, TelegramUserId};
use pustol_telegram::updates::{CallbackQuery, Message, UPDATE_RETENTION};
use pustol_telegram::{Bot, SendError, Update, messages};

use crate::callbacks::{self, Callback};
use crate::state::Clock;

/// How long one request waits for something to arrive before asking again.
const LONG_POLL: Duration = Duration::from_secs(25);

/// How long to wait after Telegram or the database could not be reached.
const AFTER_FAILURE: Duration = Duration::from_secs(5);

/// How long to wait after Telegram refused to hand updates over.
///
/// It refuses when another process polls the same bot or a webhook is set for it. Neither goes away
/// in seconds, and asking every five would fill the log with the same line.
const AFTER_REFUSAL: Duration = Duration::from_mins(1);

#[derive(Clone, Debug)]
pub struct Inbox {
    pub store: Store,
    pub bot: Bot,
    pub bar: BarId,
    pub clock: Clock,
}

/// Why an update could not be taken in hand.
#[derive(Debug, thiserror::Error)]
pub enum ClaimError {
    #[error("the bot token names no bot, so no update of it can be claimed")]
    NoBot,
    #[error("could not claim an update: {0}")]
    Store(#[from] pustol_db::Error),
}

/// Why one poll of the inbox did not finish.
#[derive(Debug, thiserror::Error)]
pub enum PollError {
    #[error(transparent)]
    Telegram(#[from] SendError),
    #[error(transparent)]
    Claim(#[from] ClaimError),
}

impl Inbox {
    /// Runs until the process is asked to stop.
    ///
    /// Only the wait for Telegram is cut short by a stop. Updates already in hand are settled in
    /// full, so a guest's tap is never left cancelled but unanswered, and before stopping Telegram is
    /// told what was settled. An update that cannot be claimed stops the batch where it is: it is
    /// fetched again once the wait after a failure is over.
    pub async fn run(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut offset = None;
        let mut confirmed = offset;
        loop {
            let polled = tokio::select! {
                polled = self.bot.get_updates(offset, LONG_POLL) => polled,
                _ = shutdown.changed() => break,
            };
            let pause = match polled {
                Ok(updates) => {
                    confirmed = offset;
                    let (settled, stopped) = self.settle(updates, offset).await;
                    offset = settled;
                    match stopped {
                        Ok(()) => continue,
                        Err(error) => {
                            tracing::error!(%error, "could not take an update in hand");
                            AFTER_FAILURE
                        }
                    }
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
                _ = shutdown.changed() => break,
            }
        }
        if let Some(settled) = offset
            && offset > confirmed
            && let Err(error) = self.confirm(settled).await
        {
            tracing::warn!(%error, "could not tell Telegram which updates were settled");
        }
    }

    /// Tells Telegram every update before `offset` is done, without waiting for more.
    ///
    /// What comes back is dropped: it has not been answered, so it is left for the next poll.
    pub async fn confirm(&self, offset: i64) -> Result<(), SendError> {
        self.bot.get_updates(Some(offset), Duration::ZERO).await.map(drop)
    }

    /// Fetches once from `offset`, settles everything fetched, and gives the offset past it.
    pub async fn poll_once(&self, offset: Option<i64>) -> Result<Option<i64>, PollError> {
        let updates = self.bot.get_updates(offset, LONG_POLL).await?;
        let (settled, stopped) = self.settle(updates, offset).await;
        stopped?;
        Ok(settled)
    }

    /// Settles each update in order and gives the offset past the last one settled.
    ///
    /// An update is settled once it is claimed and answered, or found claimed by somebody else, and
    /// the offset moves past it either way: an update whose answer fails every time must not be
    /// fetched again for ever. The first update that cannot be claimed stops the batch before it,
    /// with the error.
    async fn settle(
        &self,
        updates: Vec<Update>,
        mut offset: Option<i64>,
    ) -> (Option<i64>, Result<(), ClaimError>) {
        let fetched = !updates.is_empty();
        for update in updates {
            let after = update.update_id + 1;
            if let Err(error) = self.handle(update).await {
                return (offset, Err(error));
            }
            offset = Some(offset.map_or(after, |current| current.max(after)));
        }
        if fetched {
            self.forget_old_claims().await;
        }
        (offset, Ok(()))
    }

    /// Answers one update if this process is the one that claims it, and says whether it was.
    ///
    /// At most once, not at least once. The claim is written before the answer, so two processes, or
    /// one process fetching again what it already answered, never answer twice. A crash between the
    /// claim and the answer leaves that one update unanswered; for a tap that means a spinner that
    /// gives up, and the guest taps again, which Telegram sends as a new update. Answering twice
    /// instead would cancel twice and say so twice, which nobody can take back.
    ///
    /// Failures to answer are logged; there is nobody else to tell.
    pub async fn handle(&self, update: Update) -> Result<bool, ClaimError> {
        let bot = self.bot.id().ok_or(ClaimError::NoBot)?;
        let now = self.clock.now();
        if !self
            .store
            .claim_update(bot, update.update_id, now, now - UPDATE_RETENTION)
            .await?
        {
            return Ok(false);
        }
        if let Some(query) = update.callback_query {
            self.answer_tap(query).await;
        } else if let Some(message) = update.message {
            self.answer_message(message).await;
        }
        Ok(true)
    }

    /// Clears away claims too old to mean anything. A claim that stays behind costs a row and
    /// silences nothing, since claiming takes an old claim over.
    async fn forget_old_claims(&self) {
        let Some(bot) = self.bot.id() else {
            return;
        };
        if let Err(error) = self
            .store
            .forget_update_claims(bot, self.clock.now() - UPDATE_RETENTION)
            .await
        {
            tracing::warn!(%error, "could not clear away old update claims");
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
        let contact = config
            .contact
            .as_deref()
            .and_then(pustol_domain::config::Contact::parse)
            .map(|contact| contact.label());
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
                    messages::welcome(&config.name, contact.as_deref())
                }
            }
            None => messages::nobody_reads_this(&config.name, contact.as_deref()),
        };
        if let Err(error) = self.bot.send_message(message.chat.id, &reply, &[]).await {
            tracing::warn!(%error, "could not answer a message");
        }
    }
}

/// The deep-link payload of a `/start` command, empty when there is none.
///
/// A client that picks the command from a list addresses it to the bot by name, `/start@PodvalBot`,
/// and a payload follows the name the same way it follows the bare command.
fn start_payload(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("/start")?;
    let rest = match rest.strip_prefix('@') {
        Some(addressed) => {
            let name = addressed
                .find(char::is_whitespace)
                .unwrap_or(addressed.len());
            if name == 0 {
                return None;
            }
            &addressed[name..]
        }
        None => rest,
    };
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix(' ').map(str::trim)
}
