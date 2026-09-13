//! The outbox for everything the bot says to guests.
//!
//! The bot lives outside the transaction that decides to send something, so the decision is
//! committed here and delivery is attempted afterwards. A crash between the two loses at worst a
//! reminder, never a booking, and delivery is retried rather than assumed.

use chrono::{DateTime, TimeDelta, Utc};
use pustol_domain::allocator::BookingId;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::Store;
use crate::error::Result;
use crate::ids::{BarId, TelegramUserId};

/// How long a claimed message is kept from other workers while it is being delivered.
///
/// Claiming takes a row lock that ends with the claiming statement, so the lock alone cannot stop a
/// second worker from sending a message the first is still waiting on Telegram about. Pushing the
/// message out of the due window for longer than a delivery can take does. A worker that dies
/// mid-delivery leaves the message to come back once this has passed.
pub const CLAIM_LEASE: TimeDelta = TimeDelta::minutes(5);

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
    /// The moment this claim keeps the message from other workers until, exactly as stored.
    ///
    /// Together with `attempts` it names this claim and no other: a reminder rewritten while it
    /// was out for delivery has a new moment and no attempts, and a later claim has more attempts.
    /// A delivery result is recorded only while both still match, so the outcome of sending the old
    /// words can never settle, delay or give up the new ones.
    pub lease: DateTime<Utc>,
}

/// The condition that the claim a delivery result is for is still the message's: the same row, lease
/// and attempt, not yet settled. Binds `$1` to `$3`, which [`of_claim`] fills.
macro_rules! still_claimed {
    () => {
        " where id = $1 and scheduled_for = $2 and attempts = $3
            and sent_at is null and gave_up_at is null"
    };
}

impl Store {
    /// Queues one of the bar's own messages, chosen by staff from a guest's card.
    ///
    /// The text has to be one the bar configured. Accepting free text here would make a borrowed
    /// staff account a way to send anything to every guest who has ever booked, and checking it in
    /// the handler instead would leave the rule for every future caller to remember. Checked in
    /// the transaction that queues it, under the bar's lock a settings save also holds, so a
    /// template removed a moment ago cannot slip through.
    ///
    /// Only to a guest the bot can reach: a booking with an account behind it, which the bot has not
    /// found it cannot write to. Staff are told the message went, and one that can never arrive would
    /// tell them the guest knows what the guest does not, when somebody has to call instead.
    pub async fn send_template(
        &self,
        bar: BarId,
        booking: BookingId,
        text: &str,
        now: DateTime<Utc>,
    ) -> Result<Uuid> {
        let mut transaction = self.pool().begin().await?;
        crate::lock_bar(&mut transaction, bar).await?;
        let config = crate::bar::load_config(&mut transaction, bar).await?;
        let record = crate::bookings::fetch_booking(&mut transaction, bar, booking).await?;
        let Some(recipient) = record.telegram_user_id.filter(|_| record.reachable_by_bot) else {
            return Err(crate::Error::NoBotChat);
        };
        if !config
            .message_templates
            .iter()
            .any(|offered| offered == text)
        {
            return Err(crate::Error::UnknownMessage);
        }
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

    /// Takes up to `limit` messages that are due, marking each as attempted and leased.
    ///
    /// `for update skip locked` keeps two workers from claiming the same row at the same instant;
    /// [`CLAIM_LEASE`] keeps the second from claiming it while the first is still delivering.
    ///
    /// A reminder is withheld unless the guest asked for reminders, the bot is believed able to
    /// reach them and the booking still holds a table. Messages that can never be worth sending —
    /// a reminder, cancellation or move notice about an evening that is over, a reminder for a
    /// booking that was cancelled or has begun — are settled first, so they stop sitting in front
    /// of the ones that can. A staff message is never settled by the clock: it is about whatever
    /// staff chose to say, and staff were told it was sent.
    pub async fn claim_due(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<PendingNotification>> {
        // One statement: every part sees the same snapshot, so the messages it settles are set aside
        // from the ones it claims by identity rather than by what the settling wrote.
        let rows = sqlx::query(
            "with settled as (
                 update notification n
                 set gave_up_at = $1,
                     last_error = case when b.ends_at <= $1 then 'the evening is over'
                                       else 'the booking is no longer upcoming' end
                 from booking b
                 where b.id = n.booking_id
                   and n.sent_at is null and n.gave_up_at is null and n.scheduled_for <= $1
                   and n.kind in ('reminder', 'cancelled', 'moved')
                   and (b.ends_at <= $1
                        or (n.kind = 'reminder' and (b.status = 'cancelled' or b.starts_at <= $1)))
                 returning n.id
             ),
             due as (
                 select n.id
                 from notification n
                 join booking b on b.id = n.booking_id
                 join telegram_user u on u.id = n.telegram_user_id
                 where n.sent_at is null and n.gave_up_at is null and n.scheduled_for <= $1
                   and n.id not in (select id from settled)
                   and (
                       n.kind <> 'reminder'
                       or (b.table_id is not null and u.reminders_opted_in and u.can_receive_messages)
                   )
                 order by n.scheduled_for
                 limit $2
                 for update of n skip locked
             )
             update notification set attempts = attempts + 1, scheduled_for = $3
             where id in (select id from due)
             returning id, bar_id, booking_id, telegram_user_id, kind, body, attempts,
                       scheduled_for",
        )
        .bind(now)
        .bind(limit)
        .bind(now + CLAIM_LEASE)
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
                    lease: row.try_get("scheduled_for")?,
                })
            })
            .collect()
    }

    /// Records a delivered message.
    pub async fn mark_sent(&self, claimed: &PendingNotification, now: DateTime<Utc>) -> Result<()> {
        of_claim(
            concat!(
                "update notification set sent_at = $4, last_error = null",
                still_claimed!()
            ),
            claimed,
        )
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Records a failed attempt, and when to try again.
    pub async fn defer(
        &self,
        claimed: &PendingNotification,
        retry_at: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        of_claim(
            concat!(
                "update notification set scheduled_for = $4, last_error = $5",
                still_claimed!()
            ),
            claimed,
        )
        .bind(retry_at)
        .bind(error)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Waits as long as Telegram asked, without counting the wait as a failed attempt.
    ///
    /// Being told to slow down says nothing about whether this message can be delivered, so it
    /// must not bring the message closer to being given up on.
    pub async fn postpone(
        &self,
        claimed: &PendingNotification,
        retry_at: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        of_claim(
            concat!(
                "update notification
                 set scheduled_for = $4, last_error = $5, attempts = greatest(attempts - 1, 0)",
                still_claimed!()
            ),
            claimed,
        )
        .bind(retry_at)
        .bind(error)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Stops trying. Used for refusals that will not change however often they are retried — a
    /// blocked bot, a deleted account — and for a message that has exhausted its attempts.
    pub async fn give_up(
        &self,
        claimed: &PendingNotification,
        now: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        of_claim(
            concat!(
                "update notification set gave_up_at = $4, last_error = $5",
                still_claimed!()
            ),
            claimed,
        )
        .bind(now)
        .bind(error)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

/// `sql`, with the identity of `claimed` bound to `$1`, `$2` and `$3`.
fn of_claim<'q>(
    sql: &'static str,
    claimed: &'q PendingNotification,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    sqlx::query(sql)
        .bind(claimed.id)
        .bind(claimed.lease)
        .bind(claimed.attempts)
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

/// The reminder a booking should have, for the window it now has.
///
/// One function for taking a booking and for moving one, because they ask the same question: the
/// reminder follows the window the booking has now, however it came to have it.
///
/// A reminder whose moment has already passed is not kept: telling somebody three hours in advance
/// about a table they booked ten minutes ago is noise. One already delivered is left alone. Whether
/// the guest wants reminders is deliberately *not* checked here — that is settled when the message
/// is about to go out, so a guest who opts in after booking still gets one.
pub(crate) async fn plan_reminder(
    connection: &mut PgConnection,
    bar: BarId,
    booking: BookingId,
    recipient: TelegramUserId,
    body: &str,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<()> {
    if scheduled_for <= now {
        return abandon_reminder(connection, booking, "the reminder's moment has passed").await;
    }
    sqlx::query(
        "insert into notification (bar_id, booking_id, telegram_user_id, kind, body, scheduled_for)
         values ($1, $2, $3, 'reminder', $4, $5)
         on conflict (booking_id) where kind = 'reminder' do update
           set body = excluded.body, scheduled_for = excluded.scheduled_for,
               gave_up_at = null, last_error = null, attempts = 0
           where notification.sent_at is null",
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
