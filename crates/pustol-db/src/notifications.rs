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

/// How long claimed message stays hidden from other workers during delivery.
///
/// Claim row lock ends with claim statement, so lock alone cannot stop second worker resending while
/// first waits on Telegram; pushing message out of due window longer than delivery takes does.
/// Message of dead worker returns after lease.
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
    /// Lease end, exactly as stored.
    ///
    /// With `attempts` identifies this claim only: reminder rewritten mid-delivery gets new moment
    /// and zero attempts; later claim has more attempts. Result recorded only while both match, so
    /// outcome of old words never settles, delays or gives up new ones.
    pub lease: DateTime<Utc>,
}

/// SQL filter: row, lease and attempt still match claim, not settled. Uses `$1` to `$3`, bound by
/// [`of_claim`].
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
    /// queuing transaction under bar lock settings save also holds, so just-removed template cannot
    /// slip through.
    ///
    /// Only to guest bot can reach: booking has account, not marked unreachable. Staff are told
    /// message went; undeliverable one would mislead them when someone must call instead.
    ///
    /// # Errors
    ///
    /// `NoBotChat`, `UnknownMessage`, `NotFound` for unknown bar or booking, database errors.
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
    /// `for update skip locked` stops two workers claiming same row at same instant; [`CLAIM_LEASE`]
    /// stops second claiming it while first still delivers.
    ///
    /// A reminder is withheld unless the guest asked for reminders, the bot is believed able to
    /// reach them and booking still holds table. Messages never worth sending (reminder,
    /// cancellation or move notice for evening already over; reminder for booking cancelled or
    /// begun) settled first, so they stop blocking the rest. Staff message never settled by clock:
    /// staff chose it and were told it was sent.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn claim_due(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<PendingNotification>> {
        // One statement, one snapshot: settled rows excluded from claim by id, not by what settling
        // wrote.
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

    /// Waits as long as Telegram asked; wait not counted as failed attempt, since rate limit says
    /// nothing about deliverability.
    ///
    /// # Errors
    ///
    /// Database errors.
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

/// `sql` with claim identity of `claimed` bound to `$1`, `$2`, `$3`.
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

/// Fits reminder to booking's current window; shared by taking and moving booking.
///
/// Reminder whose moment passed not kept: three-hour notice for table booked ten minutes ago is
/// noise. Delivered one left alone. Guest consent deliberately *not* checked here: settled at send
/// time, so guest opting in after booking still gets one.
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
