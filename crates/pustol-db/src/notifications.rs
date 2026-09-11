//! The outbox for everything the bot says to guests.
//!
//! The bot lives outside the transaction that decides to send something, so the decision is
//! committed here and delivery is attempted afterwards. A crash between the two loses at worst a
//! reminder, never a booking, and delivery is retried rather than assumed.

use chrono::{DateTime, Utc};
use pustol_domain::allocator::BookingId;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::error::Result;
use crate::ids::{BarId, TelegramUserId};
use crate::Store;

/// What a message is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, sqlx::Type, serde::Serialize)]
#[sqlx(type_name = "notification_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// Sent some hours before the booking, with a way to cancel.
    Reminder,
    /// Sent when staff cancel a booking, carrying the reason they chose.
    Cancelled,
    /// One of the bar's own messages, sent by staff from the guest's card.
    StaffMessage,
    /// Sent when staff move a booking to another time, naming the new one.
    Moved,
}

/// A message ready to go out.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PendingNotification {
    pub id: Uuid,
    pub bar: BarId,
    pub booking: BookingId,
    pub recipient: TelegramUserId,
    pub kind: NotificationKind,
    pub body: String,
    /// How many times delivery has now been tried, this attempt included.
    pub attempts: i32,
}

impl Store {
    /// Queues one of the bar's own messages, chosen by staff from a guest's card.
    ///
    /// The text has to be one the bar configured. Accepting free text here would make a borrowed
    /// staff account a way to send anything to every guest who has ever booked, and checking it in
    /// the handler instead would leave the rule for every future caller to remember.
    pub async fn send_template(
        &self,
        bar: BarId,
        booking: BookingId,
        recipient: TelegramUserId,
        text: &str,
        now: DateTime<Utc>,
    ) -> Result<Uuid> {
        let mut connection = self.pool().acquire().await?;
        let config = crate::bar::load_config(&mut connection, bar).await?;
        if !config.message_templates.iter().any(|offered| offered == text) {
            return Err(crate::Error::UnknownMessage);
        }
        drop(connection);

        let mut transaction = self.pool().begin().await?;
        let id = enqueue(
            &mut transaction,
            bar,
            booking,
            recipient,
            NotificationKind::StaffMessage,
            text,
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(id)
    }

    /// Takes up to `limit` messages that are due, marking each as attempted.
    ///
    /// `for update skip locked` is what lets several workers drain the same queue without either
    /// blocking on each other or sending the same message twice.
    ///
    /// A reminder is withheld unless the guest asked for reminders, the bot is believed able to
    /// reach them, the booking still holds a table and the booking has not already started. The
    /// booking-status test here is the authoritative one: cancelling settles the pending reminder
    /// too, but only this check is immune to a cancellation that commits a moment later.
    pub async fn claim_due(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<PendingNotification>> {
        let rows = sqlx::query(
            "with due as (
                 select n.id
                 from notification n
                 join booking b on b.id = n.booking_id
                 join telegram_user u on u.id = n.telegram_user_id
                 where n.sent_at is null and n.gave_up_at is null and n.scheduled_for <= $1
                   and (
                       n.kind <> 'reminder'
                       or (
                           b.status <> 'cancelled'
                           and b.starts_at > $1
                           and u.reminders_opted_in
                           and u.can_receive_messages
                       )
                   )
                 order by n.scheduled_for
                 limit $2
                 for update of n skip locked
             )
             update notification set attempts = attempts + 1
             where id in (select id from due)
             returning id, bar_id, booking_id, telegram_user_id, kind, body, attempts",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(PendingNotification {
                    id: row.try_get("id")?,
                    bar: BarId(row.try_get("bar_id")?),
                    booking: BookingId(row.try_get("booking_id")?),
                    recipient: TelegramUserId(row.try_get("telegram_user_id")?),
                    kind: row.try_get("kind")?,
                    body: row.try_get("body")?,
                    attempts: row.try_get("attempts")?,
                })
            })
            .collect()
    }

    /// Records a delivered message.
    pub async fn mark_sent(&self, id: Uuid, now: DateTime<Utc>) -> Result<()> {
        sqlx::query("update notification set sent_at = $2, last_error = null where id = $1")
            .bind(id)
            .bind(now)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Records a failed attempt, and when to try again.
    pub async fn defer(&self, id: Uuid, retry_at: DateTime<Utc>, error: &str) -> Result<()> {
        sqlx::query(
            "update notification set scheduled_for = $2, last_error = $3 where id = $1",
        )
        .bind(id)
        .bind(retry_at)
        .bind(error)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Stops trying. Used for refusals that will not change however often they are retried — a
    /// blocked bot, a deleted account — and for a message that has exhausted its attempts.
    pub async fn give_up(&self, id: Uuid, now: DateTime<Utc>, error: &str) -> Result<()> {
        sqlx::query("update notification set gave_up_at = $2, last_error = $3 where id = $1")
            .bind(id)
            .bind(now)
            .bind(error)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

/// Puts a message in the outbox, on whichever transaction the caller is already in.
///
/// Taking a connection rather than the pool is what lets a message be queued in the same
/// transaction as the decision to send it: a notice that survived a rolled-back cancellation would
/// tell a guest their evening is off when it is not.
pub(crate) async fn enqueue(
    connection: &mut PgConnection,
    bar: BarId,
    booking: BookingId,
    recipient: TelegramUserId,
    kind: NotificationKind,
    body: &str,
    now: DateTime<Utc>,
) -> Result<Uuid> {
    let row = sqlx::query(
        "insert into notification (bar_id, booking_id, telegram_user_id, kind, body, scheduled_for)
         values ($1, $2, $3, $4, $5, $6) returning id",
    )
    .bind(bar)
    .bind(booking.0)
    .bind(recipient.0)
    .bind(kind)
    .bind(body)
    .bind(now)
    .fetch_one(connection)
    .await?;
    Ok(row.try_get("id")?)
}

/// Queues the reminder for a new booking.
///
/// A reminder whose moment has already passed is not queued at all: telling somebody three hours
/// in advance about a table they booked ten minutes ago is noise, not a service. Whether the guest
/// wants reminders is deliberately *not* checked here — that is settled when the message is about
/// to go out, so a guest who opts in after booking still gets one.
pub(crate) async fn enqueue_reminder(
    connection: &mut PgConnection,
    bar: BarId,
    booking: BookingId,
    recipient: TelegramUserId,
    body: &str,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<()> {
    if scheduled_for <= now {
        return Ok(());
    }
    sqlx::query(
        "insert into notification (bar_id, booking_id, telegram_user_id, kind, body, scheduled_for)
         values ($1, $2, $3, 'reminder', $4, $5)
         on conflict (booking_id) where kind = 'reminder' do nothing",
    )
    .bind(bar)
    .bind(booking.0)
    .bind(recipient.0)
    .bind(body)
    .bind(scheduled_for)
    .execute(connection)
    .await?;
    Ok(())
}

/// Moves a pending reminder onto a booking's new window.
///
/// The body names an hour, so a reminder left behind would contradict the notice just sent.
/// Rewritten rather than replaced: the queue holds at most one reminder per booking. One whose
/// new moment has already gone is given up.
pub(crate) async fn reschedule_reminder(
    connection: &mut PgConnection,
    booking: BookingId,
    body: &str,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<()> {
    if scheduled_for <= now {
        return abandon_reminder(connection, booking, "the booking moved past its reminder").await;
    }
    sqlx::query(
        "update notification set body = $2, scheduled_for = $3
         where booking_id = $1 and kind = 'reminder' and sent_at is null and gave_up_at is null",
    )
    .bind(booking.0)
    .bind(body)
    .bind(scheduled_for)
    .execute(connection)
    .await?;
    Ok(())
}

/// Settles the pending reminder of a booking that is no longer happening.
///
/// Hygiene rather than correctness: [`Store::claim_due`] would refuse to send it anyway. Without
/// this the queue accumulates rows that can never be delivered and never be cleaned up.
pub(crate) async fn abandon_reminder(
    connection: &mut PgConnection,
    booking: BookingId,
    why: &str,
) -> Result<()> {
    sqlx::query(
        "update notification set gave_up_at = now(), last_error = $2
         where booking_id = $1 and kind = 'reminder' and sent_at is null and gave_up_at is null",
    )
    .bind(booking.0)
    .bind(why)
    .execute(connection)
    .await?;
    Ok(())
}
