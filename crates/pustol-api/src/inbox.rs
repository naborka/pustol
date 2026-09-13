//! Answers what guests send bot. Long polling, not webhook: no public address, secret or
//! registration, so laptop and production run same single process.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use pustol_db::{BarId, Store, TelegramUserId};
use pustol_telegram::updates::{CallbackQuery, Message, UPDATE_RETENTION};
use pustol_telegram::{Bot, SendError, Update, messages};
use uuid::Uuid;

use crate::callbacks::{self, Callback};
use crate::state::Clock;

const LONG_POLL: Duration = Duration::from_secs(25);

/// After Telegram or database unreachable.
const AFTER_FAILURE: Duration = Duration::from_secs(5);

/// Telegram refuses while another process polls same bot or webhook set; neither clears in seconds.
const AFTER_REFUSAL: Duration = Duration::from_mins(1);

/// Retries before failing update is let go; updates answered in order, so it blocks all behind it.
pub const ANSWER_RETRIES: i32 = 5;

const SWEEP_EVERY: TimeDelta = TimeDelta::hours(1);

#[derive(Clone, Debug)]
pub struct Inbox {
    store: Store,
    bot: Bot,
    bar: BarId,
    clock: Clock,
    /// Fresh per inbox: restarted process never retakes claim it may have answered. Clones share.
    owner: Uuid,
    /// Unix seconds of last claim sweep; clones share.
    last_sweep: Arc<AtomicI64>,
}

impl Inbox {
    pub fn new(store: Store, bot: Bot, bar: BarId, clock: Clock) -> Self {
        Self {
            store,
            bot,
            bar,
            clock,
            owner: Uuid::new_v4(),
            last_sweep: Arc::new(AtomicI64::new(i64::MIN)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    #[error("the bot token names no bot, so no update of it can be claimed")]
    NoBot,
    #[error("could not take an update in hand: {0}")]
    Claim(pustol_db::Error),
    #[error(
        "could not read what the answer to update {update_id} needs, on attempt {attempt}: {error}"
    )]
    Answer {
        update_id: i64,
        /// Counts this attempt.
        attempt: i32,
        error: pustol_db::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PollError {
    #[error(transparent)]
    Telegram(#[from] SendError),
    #[error(transparent)]
    Handle(#[from] HandleError),
}

impl Inbox {
    /// Stop cuts only Telegram wait; updates in hand settle fully, so no tap cancelled unanswered.
    ///
    /// Before exit, confirms offset if it differs from last confirmed, lower included: Telegram may
    /// restart id numbering. Unsettled update halts batch; refetched after failure pause.
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
                            tracing::error!(%error, "could not settle an update");
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
            && offset != confirmed
            && let Err(error) = self.confirm(settled).await
        {
            tracing::warn!(%error, "could not tell Telegram which updates were settled");
        }
    }

    /// Marks updates before `offset` done, no wait. Returned updates dropped, left for next poll.
    ///
    /// # Errors
    ///
    /// Telegram request failed or refused.
    pub async fn confirm(&self, offset: i64) -> Result<(), SendError> {
        self.bot
            .get_updates(Some(offset), Duration::ZERO)
            .await
            .map(drop)
    }

    /// Fetches once, settles all, returns offset past last settled.
    ///
    /// # Errors
    ///
    /// Telegram fetch failed, or update could not be settled.
    pub async fn poll_once(&self, offset: Option<i64>) -> Result<Option<i64>, PollError> {
        let updates = self.bot.get_updates(offset, LONG_POLL).await?;
        let (settled, stopped) = self.settle(updates, offset).await;
        stopped?;
        Ok(settled)
    }

    /// Settled means answered, claimed by other, or let go after [`ANSWER_RETRIES`] failed retries.
    /// First unsettled update halts batch. Claim failure never lets go: database down, would drop
    /// every update behind.
    ///
    /// Offset past last settled, not highest id seen: after quiet week Telegram restarts ids from
    /// random number, and old higher offset would refetch same update forever.
    async fn settle(
        &self,
        updates: Vec<Update>,
        mut offset: Option<i64>,
    ) -> (Option<i64>, Result<(), HandleError>) {
        let fetched = !updates.is_empty();
        for update in updates {
            let id = update.update_id;
            match self.handle(update).await {
                Ok(_) => {}
                Err(HandleError::Answer {
                    update_id,
                    attempt,
                    error,
                }) if attempt > ANSWER_RETRIES => {
                    tracing::error!(
                        %error,
                        update_id,
                        retries = ANSWER_RETRIES,
                        "could not answer an update, and let it go"
                    );
                }
                Err(error) => return (offset, Err(error)),
            }
            offset = Some(id + 1);
        }
        if fetched {
            self.forget_old_claims().await;
        }
        (offset, Ok(()))
    }

    /// Returns whether this process claimed update.
    ///
    /// At most once: claim written before answer, so no double cancel or double message. Crash
    /// between leaves update unanswered; guest taps again, Telegram sends new update.
    ///
    /// # Errors
    ///
    /// [`HandleError::NoBot`] when token names no bot. [`HandleError::Claim`] when claim write
    /// failed or its reply lost. [`HandleError::Answer`] when reading answer data failed before
    /// anything sent; owner retakes claim on refetch. Failures after sending only logged.
    pub async fn handle(&self, update: Update) -> Result<bool, HandleError> {
        let bot = self.bot.id().ok_or(HandleError::NoBot)?;
        let now = self.clock.now();
        let Some(attempt) = self
            .store
            .claim_update(
                bot,
                update.update_id,
                self.owner,
                now,
                now - UPDATE_RETENTION,
            )
            .await
            .map_err(HandleError::Claim)?
        else {
            return Ok(false);
        };
        if let Some(query) = update.callback_query {
            self.answer_tap(query).await;
        } else if let Some(message) = update.message {
            self.answer_message(message)
                .await
                .map_err(|error| HandleError::Answer {
                    update_id: update.update_id,
                    attempt,
                    error,
                })?;
        }
        Ok(true)
    }

    /// Missed sweep harmless: stale claim costs one row, claiming takes it over.
    async fn forget_old_claims(&self) {
        let Some(bot) = self.bot.id() else {
            return;
        };
        let now = self.clock.now();
        if !self.sweep_due(now) {
            return;
        }
        if let Err(error) = self
            .store
            .forget_update_claims(bot, now - UPDATE_RETENTION)
            .await
        {
            tracing::warn!(%error, "could not clear away old update claims");
        }
    }

    /// Records sweep as made when due.
    fn sweep_due(&self, now: DateTime<Utc>) -> bool {
        let now = now.timestamp();
        self.last_sweep
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
                (now.saturating_sub(last) >= SWEEP_EVERY.num_seconds()).then_some(now)
            })
            .is_ok()
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
        // Keep buttons when cancel failed, so guest can retry.
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

    async fn answer_message(&self, message: Message) -> Result<(), pustol_db::Error> {
        if !message.chat.is_private() {
            return Ok(());
        }
        let config = self.store.config(self.bar).await?;
        let contact = config.contact().map(|contact| contact.label());
        let reply = match message.text.as_deref().and_then(start_payload) {
            Some(payload) => {
                // `/start` after block proves reachable, same as delivery.
                if let Some(from) = &message.from
                    && let Err(error) = self
                        .store
                        .set_reachable(TelegramUserId(from.id), true)
                        .await
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
        Ok(())
    }
}

/// Deep-link payload, empty when none. Bot name in `/start@PodvalBot` ignored: only private chats
/// answered, where every message reaches this bot.
fn start_payload(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("/start")?;
    let (command, payload) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    (command.is_empty() || command.starts_with('@')).then(|| payload.trim())
}
