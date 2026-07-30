//! Draining the outbox.
//!
//! The bot lives outside the transaction that decided to send something, so delivery is a separate
//! job that retries. Its correctness rests on three things: only claiming what is due, honouring
//! the delay Telegram asks for, and knowing which failures are worth repeating.

use std::time::Duration;

use chrono::TimeDelta;
use pustol_db::Store;
use pustol_db::notifications::{NotificationKind, PendingNotification};
use pustol_telegram::{Bot, CallbackButton, SendError};

use crate::state::Clock;

/// How many messages one pass takes.
const BATCH: i64 = 20;

/// How many deliveries are in flight at once.
///
/// A batch is delivered concurrently so that one slow or rate-limited message does not hold up the
/// rest — a sequential loop would make the whole pass as slow as its worst request. Five stays an
/// order of magnitude below what Telegram accepts, so the bound is about being a good citizen
/// rather than about throughput.
const IN_FLIGHT: usize = 5;

/// How long to wait between passes when there was nothing to do.
const IDLE_PAUSE: Duration = Duration::from_secs(20);

/// How long to wait after a transient failure Telegram did not put a number on.
const DEFAULT_BACKOFF: TimeDelta = TimeDelta::minutes(2);

/// After this many attempts a message is abandoned.
///
/// Without a ceiling a message Telegram keeps refusing for a reason nobody anticipated is retried
/// for ever, and a queue that never drains hides every later message behind it.
const MAX_ATTEMPTS: i32 = 6;

/// The callback the reminder's button sends back.
///
/// A guest who can cancel in one tap does, and the bar gets the table back — which is the entire
/// argument for reminding anybody about anything.
pub const CANCEL_CALLBACK: &str = "cancel_booking";

/// Runs until the process is asked to stop.
pub async fn run(store: Store, bot: Bot, clock: Clock, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    loop {
        let sent = match drain_once(&store, &bot, &clock).await {
            Ok(count) => count,
            Err(error) => {
                tracing::error!(%error, "could not read the outbox");
                0
            }
        };
        // Only pause when there was nothing to do: a full batch means there is probably more.
        if sent == 0 {
            tokio::select! {
                () = tokio::time::sleep(IDLE_PAUSE) => {}
                _ = shutdown.changed() => return,
            }
        } else if *shutdown.borrow() {
            return;
        }
    }
}

/// Takes one batch and tries to deliver it. Returns how many were attempted.
pub async fn drain_once(
    store: &Store,
    bot: &Bot,
    clock: &Clock,
) -> Result<usize, pustol_db::Error> {
    let due = store.claim_due(BATCH, clock.now()).await?;
    let attempted = due.len();

    let mut in_flight = tokio::task::JoinSet::new();
    let mut queued = due.into_iter();
    loop {
        while in_flight.len() < IN_FLIGHT {
            let Some(message) = queued.next() else { break };
            let (store, bot, clock) = (store.clone(), bot.clone(), clock.clone());
            in_flight.spawn(async move { deliver(&store, &bot, &clock, &message).await });
        }
        let Some(finished) = in_flight.join_next().await else {
            break;
        };
        // A panicked delivery must not take the worker down with it: the message stays claimed with
        // its attempt counted, and the next pass picks it up.
        match finished {
            Ok(Err(error)) => tracing::error!(%error, "could not record a delivery"),
            Err(error) => tracing::error!(%error, "a delivery panicked"),
            Ok(Ok(())) => {}
        }
    }
    Ok(attempted)
}

async fn deliver(
    store: &Store,
    bot: &Bot,
    clock: &Clock,
    message: &PendingNotification,
) -> Result<(), pustol_db::Error> {
    let buttons = reminder_buttons(message);
    match bot
        .send_message(message.recipient.0, &message.body, &buttons)
        .await
    {
        Ok(()) => {
            store.mark_sent(message.id, clock.now()).await?;
            // A delivery is the only positive evidence there is that the bot can reach this guest.
            store.set_reachable(message.recipient, true).await?;
        }
        Err(failure) if failure.means_unreachable() => {
            // Remembered, so the app stops promising reminders it cannot deliver, and so the queue
            // does not keep trying an account that has blocked the bot.
            store.set_reachable(message.recipient, false).await?;
            store
                .give_up(message.id, clock.now(), &failure.to_string())
                .await?;
        }
        Err(failure) if failure.is_worth_retrying() => {
            if message.attempts >= MAX_ATTEMPTS {
                store
                    .give_up(
                        message.id,
                        clock.now(),
                        &format!("gave up after {} attempts: {failure}", message.attempts),
                    )
                    .await?;
            } else {
                let wait = match &failure {
                    SendError::RateLimited {
                        retry_after_seconds,
                    } => TimeDelta::seconds(*retry_after_seconds),
                    _ => DEFAULT_BACKOFF,
                };
                store
                    .defer(message.id, clock.now() + wait, &failure.to_string())
                    .await?;
            }
        }
        Err(failure) => {
            // Refused on its merits. Repeating the same request repeats the same refusal.
            store
                .give_up(message.id, clock.now(), &failure.to_string())
                .await?;
        }
    }
    Ok(())
}

fn reminder_buttons(message: &PendingNotification) -> Vec<CallbackButton> {
    if message.kind == NotificationKind::Reminder {
        vec![CallbackButton {
            text: "Не смогу прийти".to_owned(),
            callback_data: format!("{CANCEL_CALLBACK}:{}", message.booking.0),
        }]
    } else {
        Vec::new()
    }
}
