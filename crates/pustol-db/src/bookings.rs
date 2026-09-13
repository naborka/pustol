//! Taking bookings, moving them, and closing tables.

use chrono::{DateTime, Utc};
use pustol_domain::allocator::{Assignment, Booking, BookingId};
use pustol_domain::config::ValidConfig;
use pustol_domain::rebooking::{self, HoldingConflict};
use pustol_domain::schedule::{BarTable, TableId};
use pustol_domain::service_day::ServiceDay;
use pustol_domain::slots::{self, Slot, SlotAvailability};
use pustol_domain::reconcile::{Request as ReconcileRequest, reconcile};
use pustol_domain::{BookingStatus, Interval, Reconciliation, bookable_days, minutes_within};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::bar::load_config;
use crate::error::{Error, Result};
use crate::evening::{Evening, read_evening};
use crate::ids::{BarId, TelegramUserId};
use crate::records::{
    BlockRecord, BlockRow, BookingRecord, BookingRow, BookingSource, StoredStatus, blocks_of,
    bookings_of,
};
use crate::{Store, lock_bar, notifications};

/// The columns every block read needs, joined to the table for its printed number.
///
/// A macro rather than a constant because `sqlx` accepts only `&'static str` query text — a
/// deliberate guard against interpolated SQL. Expanding literals keeps the column list in one
/// place while every query stays a compile-time constant.
macro_rules! block_columns {
    () => {
        "block.table_id, t.number as table_number, block.service_date, block.reason
         from table_block block join bar_table t on t.id = block.table_id"
    };
}

/// The columns every booking read needs, joined to the table for its printed number and to the
/// guest's account for whether the bot can reach them. A macro for the reason `block_columns` is.
macro_rules! booking_columns {
    () => {
        "b.id, b.table_id, t.number as table_number, t.zone as table_zone, b.service_date,
         b.starts_at, b.ends_at, b.left_at, b.party_size, b.guest_name, b.guest_username,
         b.telegram_user_id, coalesce(u.can_receive_messages, false) as reachable_by_bot,
         b.status, b.source, b.note, b.cancel_reason
         from booking b left join bar_table t on t.id = b.table_id
         left join telegram_user u on u.id = b.telegram_user_id"
    };
}

/// The longest note staff may write on a booking, matching the database's own check.
///
/// A note is read at a glance on a row in a list; anything longer is a conversation, and a
/// conversation belongs in the chat with the guest.
pub const NOTE_MAX_CHARS: usize = 120;

/// A status staff can set by hand.
///
/// Cancellation is deliberately absent. Releasing a table needs a reason to give the guest and a
/// moment to record, so it goes through [`Store::cancel_booking`]; a status endpoint that could
/// also cancel would let a mis-tap free a table with no explanation attached.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attendance {
    /// Still expected.
    Confirmed,
    /// At the table.
    Arrived,
    /// Never came. The table goes back into the pool once the grace period has run out.
    NoShow,
    /// Came, sat, and went home early. The table is free from this minute.
    Left,
}

impl Attendance {
    /// The minute the table goes back into the pool, or `None` when it stays held.
    ///
    /// Each of the two settled statuses answers a different question about *when*. A party that
    /// has left gave the table back at the moment staff pressed the button. A party that never
    /// came gave it back when the bar stopped holding it for them — the end of the grace period —
    /// which is not the same as the moment a bartender got round to noticing. Releasing at "now"
    /// would take the table from somebody who is merely five minutes late and about to walk in.
    ///
    /// The result is clamped into the promised window because that is the only range a booking
    /// can hold: releasing before it began holds nothing, and releasing after it ended releases
    /// nothing.
    #[must_use]
    pub fn released_at(
        self,
        window: Interval,
        grace_minutes: i32,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let moment = match self {
            Self::Confirmed | Self::Arrived => return None,
            Self::Left => now,
            Self::NoShow => now.max(
                window.start() + chrono::Duration::minutes(i64::from(grace_minutes)),
            ),
        };
        Some(moment.clamp(window.start(), window.end()))
    }

    /// The status a booking has once this attendance is recorded.
    #[must_use]
    pub const fn status(self) -> BookingStatus {
        match self {
            Self::Confirmed => BookingStatus::Confirmed,
            Self::Arrived => BookingStatus::Arrived,
            Self::NoShow => BookingStatus::NoShow,
            Self::Left => BookingStatus::Left,
        }
    }

    /// The attendance a booking's status records, or `None` for a cancelled booking, which has none.
    #[must_use]
    pub const fn of(status: BookingStatus) -> Option<Self> {
        match status {
            BookingStatus::Confirmed => Some(Self::Confirmed),
            BookingStatus::Arrived => Some(Self::Arrived),
            BookingStatus::NoShow => Some(Self::NoShow),
            BookingStatus::Left => Some(Self::Left),
            BookingStatus::Cancelled => None,
        }
    }
}

impl From<Attendance> for StoredStatus {
    fn from(attendance: Attendance) -> Self {
        match attendance {
            Attendance::Confirmed => Self::Confirmed,
            Attendance::Arrived => Self::Arrived,
            Attendance::NoShow => Self::NoShow,
            Attendance::Left => Self::Left,
        }
    }
}

/// Who is asking for a table.
#[derive(Clone, Debug)]
pub enum Channel {
    /// A guest, in the Mini App. Bound by the booking horizon, and bound by
    /// [`pustol_domain::rebooking`]: booking again replaces what it replaces, and is refused an
    /// evening the guest already holds.
    Guest {
        user: TelegramUserId,
        name: String,
        username: Option<String>,
        /// The bookings the guest's app said this one replaces, which is what the guest agreed to.
        /// The booking is taken only if it replaces exactly these, compared as a set.
        replacing: Vec<BookingId>,
    },
    /// Staff, taking a booking by telephone or at the door. Not bound by the horizon: a bar takes
    /// a booking for next month over the phone without arguing about it.
    ///
    /// `table` is the one staff chose; `None` asks the room. It lives here, not on the request,
    /// so a guest booking has no field for it at all.
    Staff {
        guest_name: String,
        table: Option<TableId>,
    },
}

impl Channel {
    /// The account a guest's booking is taken for, `None` for one staff take.
    const fn guest(&self) -> Option<TelegramUserId> {
        match self {
            Self::Guest { user, .. } => Some(*user),
            Self::Staff { .. } => None,
        }
    }
}

/// A request for a table.
#[derive(Clone, Debug)]
pub struct NewBooking {
    pub bar: BarId,
    pub service_day: ServiceDay,
    /// Wall-clock minutes into the shift — what the guest tapped.
    pub start_minutes: i32,
    pub party_size: i32,
    pub channel: Channel,
    /// How to word the reminder, or `None` for a booking with no account behind it.
    ///
    /// A function rather than a finished string: the wording needs the bar's name and the window
    /// the booking actually got, and both are only known inside the transaction that took it.
    /// Handing over the words instead of the text keeps the guest-facing copy with the caller and
    /// keeps the single derivation of the window here.
    pub reminder: Option<ReminderWording>,
}

/// Words a reminder from the configuration in force and the window promised.
pub type ReminderWording = fn(&ValidConfig, Interval, i32) -> String;

/// Words the notice a guest gets when staff cancel their booking.
pub type CancellationWording = fn(&ValidConfig, &BookingRecord, &str) -> String;

/// Words the notice for a moved booking, from the booking as it was and as it now is.
pub type MoveWording = fn(&ValidConfig, &BookingRecord, &BookingRecord) -> String;

/// What the bot says when a booking moves: the notice now, and the reminder that would otherwise
/// still name the old hour. Together, so a move cannot remember one and forget the other.
#[derive(Clone, Copy, Debug)]
pub struct MoveWords {
    pub notice: MoveWording,
    pub reminder: ReminderWording,
}

/// Where a booking should now be, when, and for how many — named in full, as one act.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveTo {
    /// The wall-clock arrival. The booking's own, when only the table or the party changes.
    pub start_minutes: i32,
    /// The table staff chose; `None` asks the room for its own best fit.
    pub table: Option<TableId>,
    /// How many are coming now; `None` keeps the party it was.
    pub party_size: Option<i32>,
}

/// A booking that now exists.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreatedBooking {
    pub record: BookingRecord,
    /// The guest's earlier bookings cancelled to make room for this one, soonest first.
    pub replaced: Vec<BookingId>,
    /// Every booking of the guest whose table is still held for them, as taking this one left them, this
    /// one among them, soonest first. Empty for a booking with no guest behind it.
    ///
    /// What booking again would do to this booking depends on them, so they are read by the transaction
    /// that took it, for the reason `evening` is.
    pub guest_bookings: Vec<BookingRecord>,
    /// The evening the booking is on as taking it left it, with the configuration it was taken under.
    ///
    /// Read by the transaction that took the booking, before it committed, rather than left for the
    /// caller to read again: a second read could observe a colleague's change or a settings save that
    /// happened in between, and could fail after the booking had been taken.
    pub evening: Evening,
}

/// The name a party with no booking is filed under.
///
/// Not a placeholder for a name somebody forgot to type: there is no name, because nobody booked.
/// Calling it anything else would put a fiction into the shift history.
pub const WALK_IN_NAME: &str = "Без брони";

/// An attendance change, and whatever the table it freed let the room put right.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AttendanceRecorded {
    pub record: BookingRecord,
    /// What the booking recorded just before this change, read in the transaction that made it.
    /// Undo goes back to this, not to whatever a screen last saw.
    pub previous: Attendance,
    /// Parties the released table let the room seat. Empty when nothing moved.
    pub reconciliation: Reseated,
    /// The evening as this change left it, read before it committed.
    pub evening: Evening,
}

/// A note written or rubbed out, and the evening it is on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NoteWritten {
    pub record: BookingRecord,
    /// The evening as the note left it, read before it committed.
    pub evening: Evening,
}

/// A booking that has been released, and whatever the freed table let the room fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CancelledBooking {
    pub record: BookingRecord,
    /// Bookings the freed table allowed to be seated.
    pub reconciliation: Reseated,
    /// Whether the guest will be told: a notice went into the outbox, and the bot can reach them.
    ///
    /// Decided here rather than by the caller: it depends on a reason, an account and whether that
    /// account can be reached, all facts this transaction holds. The notice is queued for an account
    /// the bot has found it cannot reach all the same — that finding is only what the last delivery
    /// learned, and the guest may let the bot back in — but nobody is told the guest knows.
    pub guest_notified: bool,
    /// The evening the booking was on, as the cancellation left it, read before it committed.
    pub evening: Evening,
}

/// A booking that now sits somewhere else, or at some other time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MovedBooking {
    pub record: BookingRecord,
    /// Whatever the table they left allowed the room to settle.
    pub reconciliation: Reseated,
    /// Whether the guest will hear about it. Only a time change is theirs to hear about, and only
    /// when the bot can reach them; the notice is queued either way, as a cancellation's is.
    pub guest_notified: bool,
    /// The evening as the move left it, read before it committed.
    pub evening: Evening,
}

/// A reconciliation together with the bookings it touched.
///
/// The domain deals in identities because allocation must not be able to see who a guest is. The
/// report staff read has to say "Тимур", so the records are carried out of the transaction that
/// already had them — rather than re-read afterwards, when a concurrent change could make the
/// names describe a different state than the one that was reconciled.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Reseated {
    pub outcome: Reconciliation,
    pub affected: Vec<BookingRecord>,
}

impl Reseated {
    /// Whether the room turned out to need no changes at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outcome.is_empty()
    }
}

/// What one day holds for one party size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DayOffer {
    pub day: ServiceDay,
    pub closed: bool,
    /// The earliest arrival time still free, absent when the day holds none.
    pub free_from_minutes: Option<i32>,
    /// The guest already holds this evening with a booking booking again cannot replace, so a
    /// booking here is refused whatever time is free.
    pub booked: bool,
}

/// Tables taken out of service, and whatever that made the room re-seat.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClosedTables {
    /// The tables this call closed, in the order asked. One already shut is not among them.
    pub closed: Vec<TableId>,
    pub reconciliation: Reseated,
    /// The evening as the closure left it, read before it committed.
    pub evening: Evening,
}

/// Tables put back into service, and whatever that let the room seat.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReopenedTables {
    /// The closures this call removed, in the order asked. A table that was not shut is not among
    /// them.
    pub reopened: Vec<ReopenedTable>,
    pub reconciliation: Reseated,
    /// The evening as the reopening left it, read before it committed.
    pub evening: Evening,
}

/// A closure that was removed, with the reason it had been given.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReopenedTable {
    pub table_id: TableId,
    pub reason: String,
}

/// Arrival times, and the configuration they were computed from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AvailabilityReading {
    pub config: ValidConfig,
    pub slots: Vec<Slot>,
}

/// What asking the room again settled, and the evening as it then stood.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReconciledShift {
    pub reconciliation: Reseated,
    /// The evening as the attempt left it, read before it committed.
    pub evening: Evening,
}

impl Store {
    /// Arrival times for a party on one shift, each with the reason it can or cannot be taken.
    ///
    /// No lock: availability is advice, true at the moment it was read. The guarantee that two
    /// guests cannot both act on it lives in [`Self::create_booking`] and, beneath that, in the
    /// exclusion constraint.
    ///
    /// `ignoring` is set aside: a booking being moved, or the ones a guest's booking on `day` would
    /// replace, which [`pustol_domain::rebooking::replaced_on`] names.
    pub async fn availability(
        &self,
        bar: BarId,
        day: ServiceDay,
        party_size: i32,
        now: DateTime<Utc>,
        ignoring: &[BookingId],
    ) -> Result<AvailabilityReading> {
        let mut connection = self.pool().acquire().await?;
        let config = load_config(&mut connection, bar).await?;
        let bookings = load_window(&mut connection, bar, day).await?;
        let blocks = load_blocks(&mut connection, bar, day).await?;
        let slots = slots::slot_list(&slots::Query {
            config: &config,
            service_day: day,
            party_size,
            bookings: &bookings_of(&bookings),
            blocks: &blocks_of(&blocks),
            now,
            ignoring,
        });
        Ok(AvailabilityReading { config, slots })
    }

    /// Shifts a guest may choose from.
    pub async fn bookable_days(&self, bar: BarId, now: DateTime<Utc>) -> Result<Vec<ServiceDay>> {
        let config = self.config(bar).await?;
        Ok(bookable_days(&config, config.current_service_day(now)))
    }

    /// What each of `days` holds for a party of this size.
    ///
    /// One read of the whole span rather than one per day: a thirty-day rail asked day by day
    /// would be sixty round trips to answer one screen, and the answers could disagree with each
    /// other because a booking taken between two of them would be in one and not the next.
    ///
    /// `guest` is every booking the guest asking holds, empty for nobody in particular. Each day sets
    /// aside exactly the ones a booking on that day would replace, so a guest's own plan never makes
    /// an evening look full to them, and says whether that day is one they are refused.
    pub async fn day_offers(
        &self,
        bar: BarId,
        config: &ValidConfig,
        days: &[ServiceDay],
        party_size: i32,
        now: DateTime<Utc>,
        guest: &[Booking],
    ) -> Result<Vec<DayOffer>> {
        let (Some(first), Some(last)) = (days.first(), days.last()) else {
            return Ok(Vec::new());
        };
        let mut connection = self.pool().acquire().await?;
        // A window can outlast midnight in both directions, so the span reaches one shift either
        // side of the range being answered — the same neighbourhood the allocator always needs.
        let from = first.checked_sub_days(1).unwrap_or(*first).date();
        let to = last.checked_add_days(1).unwrap_or(*last).date();
        let bookings = bookings_of(&load_between(&mut connection, bar, from, to).await?);
        let blocks = blocks_of(&load_blocks_between(&mut connection, bar, from, to).await?);

        Ok(days
            .iter()
            .map(|day| DayOffer {
                day: *day,
                closed: config.week.for_service_day(*day).closed,
                free_from_minutes: slots::first_free_minutes(&slots::Query {
                    config,
                    service_day: *day,
                    party_size,
                    bookings: &bookings,
                    blocks: &blocks,
                    now,
                    ignoring: &rebooking::replaced_on(guest, *day, now),
                }),
                booked: rebooking::refused_on(guest, *day, now),
            })
            .collect())
    }

    /// Takes a booking, or explains why it cannot.
    ///
    /// The picker and this method ask the very same function which times are free, so a guest is
    /// never refused a slot the app had just shown as available — except by losing a race, which
    /// is reported as its own error so the app can say "somebody just took it" rather than
    /// something vague.
    ///
    /// Refused for the first reason it cannot be taken, widest first: the party, the evening, the time,
    /// the table. Only a booking that could otherwise be taken is held to what the guest's app promised
    /// it replaces, and refused as changed when that promise is wrong. Refused as changed first, a guest
    /// whose time has gone read their bookings again, found nothing to name, and was refused again.
    pub async fn create_booking(
        &self,
        request: &NewBooking,
        now: DateTime<Utc>,
    ) -> Result<CreatedBooking> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, request.bar).await?;
        let config = load_config(&mut transaction, request.bar).await?;

        check_party_size(request.party_size, &config)?;
        check_shift_is_offered(request, &config, now)?;

        // What booking again does to the guest's bookings is `rebooking` and nothing else, asked of them
        // as this transaction reads them under the bar's lock.
        let user = request.channel.guest();
        let mine = bookings_of(&running_bookings_of(&mut transaction, request.bar, user, now).await?);
        if rebooking::refused_on(&mine, request.service_day, now) {
            return Err(Error::AlreadyBookedThisShift);
        }
        let replacing = rebooking::replaced_on(&mine, request.service_day, now);

        let bookings = load_window(&mut transaction, request.bar, request.service_day).await?;
        let blocks = load_blocks(&mut transaction, request.bar, request.service_day).await?;
        let live = bookings_of(&bookings);
        let closed = blocks_of(&blocks);
        let asking = slots::Query {
            config: &config,
            service_day: request.service_day,
            party_size: request.party_size,
            bookings: &live,
            blocks: &closed,
            now,
            ignoring: &replacing,
        };
        let window = window_at(&asking, request.start_minutes)?;

        let (source, name, username, chosen) = match &request.channel {
            Channel::Guest { name, username, .. } => {
                (BookingSource::App, name.clone(), username.clone(), None)
            }
            Channel::Staff { guest_name, table } => {
                (BookingSource::Staff, guest_name.clone(), None, *table)
            }
        };
        let free = pustol_domain::free_tables(&asking.request(window));
        let table = seat_of(&free, chosen, request.party_size)?.table_id;
        check_promise(&request.channel, &replacing)?;

        // Cancelled in the same transaction the booking is taken in, so there is no instant in which the
        // guest holds two or none.
        let replaced = cancel_replaced(&mut transaction, request.bar, &mine, &replacing, now).await?;
        let id = insert_booking(
            &mut transaction,
            &Written {
                bar: request.bar,
                table,
                service_day: request.service_day,
                window,
                party_size: request.party_size,
                guest_name: &name,
                guest_username: username.as_deref(),
                user,
                source,
                status: StoredStatus::Confirmed,
            },
        )
        .await?;

        if let (Some(wording), Some(user)) = (request.reminder, user) {
            notifications::plan_reminder(
                &mut transaction,
                request.bar,
                id,
                user,
                &wording(&config, window, request.party_size),
                window.start() - chrono::Duration::hours(i64::from(config.remind_hours)),
                now,
            )
            .await?;
        }

        // A replaced booking's table is capacity appearing on its evening, and capacity appearing
        // is offered to whoever that evening could not seat.
        let mut evenings: Vec<ServiceDay> = replaced.iter().map(|(_, day)| *day).collect();
        evenings.sort_unstable();
        evenings.dedup();
        for day in evenings {
            reconcile_shift(&mut transaction, request.bar, &config, day, now).await?;
        }

        let record = fetch_booking(&mut transaction, request.bar, id).await?;
        let guest_bookings = running_bookings_of(&mut transaction, request.bar, user, now).await?;
        let evening =
            read_evening(&mut transaction, request.bar, config, request.service_day, now).await?;
        transaction.commit().await?;
        Ok(CreatedBooking {
            record,
            replaced: replaced.into_iter().map(|(id, _)| id).collect(),
            guest_bookings,
            evening,
        })
    }

    /// Every booking of the guest that is still running, soonest first.
    ///
    /// A guest sitting at their table still sees it, and a plan for another evening beside it. A
    /// guest whose table has gone back into the pool does not — they went home, or they never came
    /// and the bar stopped waiting — because what a guest holds is a table being held for them, and
    /// once that ends there is nothing to move and nothing to give back.
    ///
    /// When that is, is [`pustol_domain::Booking::has_finished`] and nothing else.
    pub async fn bookings_of_guest(
        &self,
        bar: BarId,
        user: TelegramUserId,
        now: DateTime<Utc>,
    ) -> Result<Vec<BookingRecord>> {
        let mut connection = self.pool().acquire().await?;
        running_bookings_of_guest(&mut connection, bar, user, now).await
    }

    /// Records whether a party turned up, and gives their table back when they are done with it.
    ///
    /// One transaction, because the three things it does are one fact about the room: the status
    /// changes, the table is released or taken back, and whoever the room could not seat is given
    /// another chance at it. Setting the status in one statement and reconciling in another would
    /// leave an instant in which a table is visibly free and a party is visibly stranded.
    ///
    /// Going back to `confirmed` or `arrived` clears the release, which is what makes undo exact:
    /// the room returns to the arrangement it had, rather than to one that merely looks like it. It
    /// is refused when the table has gone to another party since, and when it would leave the guest
    /// holding what no guest may hold.
    pub async fn set_attendance(
        &self,
        bar: BarId,
        booking: BookingId,
        attendance: Attendance,
        now: DateTime<Utc>,
    ) -> Result<AttendanceRecorded> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let current = fetch_booking(&mut transaction, bar, booking).await?;
        let previous =
            Attendance::of(current.booking.status).ok_or(Error::NotFound { entity: "booking" })?;
        let released_at =
            attendance.released_at(current.booking.window, config.grace_minutes, now);
        check_table_free(
            &mut transaction,
            bar,
            &Booking {
                status: attendance.status(),
                released_at,
                ..current.booking.clone()
            },
        )
        .await?;

        sqlx::query(
            "update booking set status = $3, left_at = $4
             where bar_id = $1 and id = $2 and status <> 'cancelled'",
        )
        .bind(bar)
        .bind(booking.0)
        .bind(StoredStatus::from(attendance))
        .bind(released_at)
        .execute(&mut *transaction)
        .await
        .map_err(Error::from_write)?;

        check_holdings(&mut transaction, bar, current.telegram_user_id, booking, now).await?;
        let day = current.booking.service_day;
        let reconciliation = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        let record = fetch_booking(&mut transaction, bar, booking).await?;
        let evening = read_evening(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(AttendanceRecorded {
            record,
            previous,
            reconciliation,
            evening,
        })
    }

    /// Writes what staff want to remember about a booking, or rubs it out.
    ///
    /// Staff-facing by construction: nothing sends a note anywhere, and the guest projection has
    /// no field to put one in.
    ///
    /// Under the bar's lock like every other change to a booking, so the evening in the answer,
    /// read before the note commits, is the room exactly as this write left it.
    pub async fn set_note(
        &self,
        bar: BarId,
        booking: BookingId,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<NoteWritten> {
        let trimmed = note.map(str::trim).filter(|text| !text.is_empty());
        if trimmed.is_some_and(|text| text.chars().count() > NOTE_MAX_CHARS) {
            return Err(Error::NoteTooLong {
                limit: NOTE_MAX_CHARS,
            });
        }
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let updated = sqlx::query(
            "update booking set note = $3
             where bar_id = $1 and id = $2 and status <> 'cancelled'",
        )
        .bind(bar)
        .bind(booking.0)
        .bind(trimmed)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(Error::NotFound { entity: "booking" });
        }
        let record = fetch_booking(&mut transaction, bar, booking).await?;
        let evening =
            read_evening(&mut transaction, bar, config, record.booking.service_day, now).await?;
        transaction.commit().await?;
        Ok(NoteWritten { record, evening })
    }

    /// Moves a booking to another table, another time, or both.
    ///
    /// One act, naming both in full: patching one field is how you end up with one changed and
    /// nobody sure which was meant. The guest hears about it only when the time changed — they
    /// were never told a table number.
    ///
    /// A booking is one row with one table, so the new table must be free for the whole window.
    /// Table 3 until nine and table 9 after is two rows, and two rows is a different schema.
    pub async fn move_booking(
        &self,
        bar: BarId,
        booking: BookingId,
        to: MoveTo,
        words: Option<MoveWords>,
        now: DateTime<Utc>,
    ) -> Result<MovedBooking> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let current = fetch_unfinished(&mut transaction, bar, booking, now).await?;

        // The cap is asked only of a party that changes: a booking taken before the cap was
        // lowered keeps its size, and must still be movable to another table or time.
        let party_size = to.party_size.unwrap_or(current.booking.party_size);
        if party_size != current.booking.party_size {
            check_party_size(party_size, &config)?;
        }

        let day = current.booking.service_day;
        let bookings = load_window(&mut transaction, bar, day).await?;
        let blocks = load_blocks(&mut transaction, bar, day).await?;
        let live = bookings_of(&bookings);
        let closed = blocks_of(&blocks);
        let was = current.booking.window;
        let moving = [booking];
        let asking = slots::Query {
            config: &config,
            service_day: day,
            party_size,
            bookings: &live,
            blocks: &closed,
            now,
            ignoring: &moving,
        };
        let window = if to.start_minutes == minutes_within(day, was.start(), config.timezone) {
            was
        } else {
            if current.booking.has_started(now) {
                return Err(Error::BookingHasStarted);
            }
            window_at(&asking, to.start_minutes)?
        };
        let free = pustol_domain::free_tables(&asking.request(window));
        let seat = seat_of(&free, to.table, party_size)?;

        // What staff recorded about the party — at the table, not coming, gone — is about the time
        // they were expected. At a new time nothing has happened yet, so the booking is a plan again;
        // keeping a release from the old window would also leave it outside the new one.
        sqlx::query(
            "update booking set table_id = $3, starts_at = $4, ends_at = $5, party_size = $6,
                    status = case when $7 then 'confirmed'::booking_status else status end,
                    left_at = case when $7 then null else left_at end
             where bar_id = $1 and id = $2 and status <> 'cancelled'",
        )
        .bind(bar)
        .bind(booking.0)
        .bind(seat.table_id.0)
        .bind(window.start())
        .bind(window.end())
        .bind(party_size)
        .bind(window != was)
        .execute(&mut *transaction)
        .await
        .map_err(Error::from_write)?;

        check_holdings(&mut transaction, bar, current.telegram_user_id, booking, now).await?;
        // The table they left is capacity appearing, and capacity appearing is offered to whoever
        // the room could not seat — the same rule a cancellation and a party going home follow.
        let reconciliation = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        let record = fetch_booking(&mut transaction, bar, booking).await?;
        let mut notice_queued = false;
        if let (Some(recipient), Some(words)) = (current.telegram_user_id, words) {
            // Only a new time is news: the guest never saw a table number, and a change of size
            // is one they asked for. The reminder names both the hour and the party, so it follows
            // either.
            if window != was {
                notifications::enqueue(
                    &mut transaction,
                    bar,
                    booking,
                    recipient,
                    notifications::NotificationKind::Moved,
                    &(words.notice)(&config, &current, &record),
                    now,
                )
                .await?;
                notice_queued = true;
            }
            if window != was || party_size != current.booking.party_size {
                notifications::plan_reminder(
                    &mut transaction,
                    bar,
                    booking,
                    recipient,
                    &(words.reminder)(&config, window, party_size),
                    window.start() - chrono::Duration::hours(i64::from(config.remind_hours)),
                    now,
                )
                .await?;
            }
        }
        let guest_notified = notice_queued && record.reachable_by_bot;
        let evening = read_evening(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(MovedBooking {
            record,
            reconciliation,
            guest_notified,
            evening,
        })
    }

    /// Seats a party that walked in, at the minute they sat down.
    ///
    /// Not put through the slot list, and deliberately: a slot list answers "when may somebody
    /// arrive", and every one of its answers is either in the future or refused as past. A party
    /// standing at the door is arriving *now*, which is a time no grid contains. Flooring them
    /// onto the grid instead would draw the table as occupied from a minute nobody sat down, and
    /// would hide a table freed mid-step from the very offer that is looking for one.
    ///
    /// It is the same allocator underneath, over the same window the party will actually hold, so
    /// the table this takes is the table the shift's own "who fits" line promised.
    ///
    /// `table` is the one staff chose — the bartender can see the room and the allocator cannot.
    /// Checked against the room's own list, in the transaction that writes. `None` asks the
    /// room to choose.
    ///
    /// What the room offers is [`pustol_domain::walk_in`], the offer the shift draws its tables free
    /// for a walk-in and its "who fits" line from: a turn cut short at closing, and a table free for all
    /// of it. Refused as not the running shift on any other day, and on this one while it is not open.
    pub async fn seat_walk_in(
        &self,
        bar: BarId,
        day: ServiceDay,
        party_size: i32,
        table: Option<TableId>,
        now: DateTime<Utc>,
    ) -> Result<CreatedBooking> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        check_party_size(party_size, &config)?;
        let bookings = bookings_of(&load_window(&mut transaction, bar, day).await?);
        let blocks = blocks_of(&load_blocks(&mut transaction, bar, day).await?);
        let offer = pustol_domain::walk_in(&config, day, now, &bookings, &blocks).ok_or(
            Error::NotTheRunningShift {
                service_day: day.date(),
            },
        )?;
        let seat = seat_of(&offer.tables_for(party_size), table, party_size)?;
        let window = offer.window;

        let id = insert_booking(
            &mut transaction,
            &Written {
                bar,
                table: seat.table_id,
                service_day: day,
                window,
                party_size,
                guest_name: WALK_IN_NAME,
                guest_username: None,
                user: None,
                source: BookingSource::Walk,
                status: StoredStatus::Arrived,
            },
        )
        .await?;

        let record = fetch_booking(&mut transaction, bar, id).await?;
        let evening = read_evening(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(CreatedBooking {
            record,
            replaced: Vec::new(),
            guest_bookings: Vec::new(),
            evening,
        })
    }

    /// Releases a table and records why.
    ///
    /// The freed table is offered straight back to anyone the room could not seat: a cancellation
    /// is the commonest way capacity appears, and an orphan is a live booking still owed a table.
    /// Leaving it stranded next to an empty table would keep a promise the bar can now honour in a
    /// state the bar cannot serve.
    ///
    /// A pending reminder is settled in the same transaction. The worker also refuses to send a
    /// reminder for a booking that no longer holds a table, which is what actually guarantees the
    /// guest is not reminded about a cancelled evening; settling here keeps the queue from filling
    /// with rows that will never be sent.
    pub async fn cancel_booking(
        &self,
        bar: BarId,
        booking: BookingId,
        reason: Option<&str>,
        notice: Option<CancellationWording>,
        now: DateTime<Utc>,
    ) -> Result<CancelledBooking> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let cancelled =
            cancel_locked(&mut transaction, bar, config, booking, reason, notice, now).await?;
        transaction.commit().await?;
        Ok(cancelled)
    }

    /// A guest gives back one booking of theirs.
    ///
    /// Somebody else's booking is not found, whatever state it is in: a guest learns nothing about a
    /// booking that is not theirs. Their own is refused as over once its table is no longer held,
    /// exactly as it would be for staff.
    ///
    /// Checked and released in one transaction: between a separate lookup and a cancel, staff could
    /// have cancelled the same booking, and the guest would be told their cancellation failed when
    /// in truth the table is already free.
    pub async fn cancel_booking_of_guest(
        &self,
        bar: BarId,
        user: TelegramUserId,
        booking: BookingId,
        now: DateTime<Utc>,
    ) -> Result<CancelledBooking> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let record = fetch_booking(&mut transaction, bar, booking).await?;
        if record.telegram_user_id != Some(user) {
            return Err(Error::NotFound { entity: "booking" });
        }
        let cancelled =
            cancel_locked(&mut transaction, bar, config, booking, None, None, now).await?;
        transaction.commit().await?;
        Ok(cancelled)
    }

    /// A guest gives back the booking a reminder named, from the button under that reminder.
    ///
    /// Only while it is still a plan: a reminder can be tapped long after it arrived, and the party
    /// may by then be sitting at the table it would release.
    pub async fn cancel_reminded_booking(
        &self,
        bar: BarId,
        user: TelegramUserId,
        booking: BookingId,
        now: DateTime<Utc>,
    ) -> Result<CancelledBooking> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let mine = running_bookings_of_guest(&mut transaction, bar, user, now)
            .await?
            .into_iter()
            .find(|record| {
                record.booking.id == booking
                    && record.booking.status == pustol_domain::BookingStatus::Confirmed
            })
            .ok_or(Error::NotFound { entity: "booking" })?;
        let cancelled =
            cancel_locked(&mut transaction, bar, config, mine.booking.id, None, None, now).await?;
        transaction.commit().await?;
        Ok(cancelled)
    }

    /// Asks the room, one more time, to seat everything it owes a table.
    ///
    /// The staff-facing "find a table" action. It is the same reconciliation that runs whenever
    /// the room changes, offered as a button because staff sometimes know something has freed up
    /// before the system does — and because being told plainly that there is still nowhere to put
    /// a party is itself the answer they need.
    pub async fn reconcile_shift(
        &self,
        bar: BarId,
        day: ServiceDay,
        now: DateTime<Utc>,
    ) -> Result<ReconciledShift> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let reconciliation = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        let evening = read_evening(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(ReconciledShift {
            reconciliation,
            evening,
        })
    }

    /// Takes tables out of service for a shift and re-seats whoever was sitting at them.
    pub async fn block_tables(
        &self,
        bar: BarId,
        day: ServiceDay,
        tables: &[TableId],
        reason: &str,
        by: Option<TelegramUserId>,
        now: DateTime<Utc>,
    ) -> Result<ClosedTables> {
        let changed = self
            .change_blocks(bar, day, tables, &[], Some((reason, by)), now)
            .await?;
        Ok(ClosedTables {
            closed: changed.closed,
            reconciliation: changed.reconciliation,
            evening: changed.evening,
        })
    }

    /// Puts tables back into service. Reconciliation runs afterwards because a freed table may be
    /// exactly what an orphaned booking has been waiting for.
    pub async fn unblock_tables(
        &self,
        bar: BarId,
        day: ServiceDay,
        tables: &[TableId],
        now: DateTime<Utc>,
    ) -> Result<ReopenedTables> {
        let changed = self.change_blocks(bar, day, &[], tables, None, now).await?;
        Ok(ReopenedTables {
            reopened: changed.reopened,
            reconciliation: changed.reconciliation,
            evening: changed.evening,
        })
    }

    /// Closing and opening tables are the same operation with the arrow reversed: change what is
    /// in service, then let reconciliation put every booking where it now belongs.
    ///
    /// What changed is read from the rows the statements actually wrote, not from what was asked:
    /// a table closed twice, or opened when it was never shut, is not news to report.
    ///
    /// Every table named has to be one of this room's live tables, or nothing is written. An
    /// identity no table has reached the database as a key it refused, and one of another bar's
    /// tables was stored against this bar.
    async fn change_blocks(
        &self,
        bar: BarId,
        day: ServiceDay,
        add: &[TableId],
        remove: &[TableId],
        reason: Option<(&str, Option<TelegramUserId>)>,
        now: DateTime<Utc>,
    ) -> Result<BlocksChanged> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        if add
            .iter()
            .chain(remove)
            .any(|asked| !config.active_tables().any(|table| table.id == *asked))
        {
            return Err(Error::NotFound { entity: "table" });
        }

        let mut reopened = Vec::new();
        if !remove.is_empty() {
            let rows = sqlx::query(
                "delete from table_block
                 where bar_id = $1 and service_date = $2 and table_id = any($3::uuid[])
                 returning table_id, reason",
            )
            .bind(bar)
            .bind(day.date())
            .bind(remove.iter().map(|table| table.0).collect::<Vec<_>>())
            .fetch_all(&mut *transaction)
            .await?;
            let removed: Vec<ReopenedTable> = rows
                .iter()
                .map(|row| {
                    Ok(ReopenedTable {
                        table_id: TableId(row.try_get("table_id")?),
                        reason: row.try_get("reason")?,
                    })
                })
                .collect::<Result<_>>()?;
            reopened = in_order_asked(remove, removed, |table| table.table_id);
        }
        let mut closed = Vec::new();
        if !add.is_empty() {
            let (text, by) = reason.ok_or(Error::MissingBlockReason)?;
            let rows = sqlx::query(
                "insert into table_block (bar_id, table_id, service_date, reason, created_by)
                 select $1, table_id, $2, $3, $4 from unnest($5::uuid[]) as table_id
                 on conflict (table_id, service_date) do nothing
                 returning table_id",
            )
            .bind(bar)
            .bind(day.date())
            .bind(text)
            .bind(by.map(|user| user.0))
            .bind(add.iter().map(|table| table.0).collect::<Vec<_>>())
            .fetch_all(&mut *transaction)
            .await?;
            let inserted: Vec<TableId> = rows
                .iter()
                .map(|row| Ok(TableId(row.try_get("table_id")?)))
                .collect::<Result<_>>()?;
            closed = in_order_asked(add, inserted, |table| *table);
        }

        let reconciliation = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        let evening = read_evening(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(BlocksChanged {
            closed,
            reopened,
            reconciliation,
            evening,
        })
    }
}

/// What one change to the closures did, before it is told to the caller as a closing or an opening.
struct BlocksChanged {
    closed: Vec<TableId>,
    reopened: Vec<ReopenedTable>,
    reconciliation: Reseated,
    evening: Evening,
}

/// `changed` in the order its tables were `asked` for, since a statement returns rows in no order.
fn in_order_asked<T>(
    asked: &[TableId],
    mut changed: Vec<T>,
    table: impl Fn(&T) -> TableId,
) -> Vec<T> {
    changed.sort_by_key(|item| asked.iter().position(|id| *id == table(item)));
    changed
}

/// The table a party of `party_size` takes: the one staff chose, or the room's own pick when nobody did.
///
/// `free` is every table the room could seat the party at, in the order it takes them. A chosen table
/// is accepted exactly when it is among them, so choosing is as safe as being allocated.
fn seat_of(free: &[&BarTable], chosen: Option<TableId>, party_size: i32) -> Result<Assignment> {
    match chosen {
        Some(id) => free
            .iter()
            .copied()
            .find(|candidate| candidate.id == id)
            .map(Assignment::of)
            .ok_or(Error::ChosenTableNotFree),
        None => free
            .first()
            .copied()
            .map(Assignment::of)
            .ok_or(Error::NoTableFree { party_size }),
    }
}

/// Refuses to let `changed` hold its table while another party holds it.
///
/// Every other way a booking comes to hold a table asks the allocator first. Setting a booking back
/// to a status that holds its table again does not choose a table, so it asks here, of the same
/// occupancy rule. The exclusion constraint would refuse the write as well, but as a race lost to
/// another booking, which under the bar's lock this can never be.
async fn check_table_free(
    connection: &mut PgConnection,
    bar: BarId,
    changed: &Booking,
) -> Result<()> {
    let (Some(table), Some(held)) = (changed.table_id, changed.occupancy()) else {
        return Ok(());
    };
    let taken = load_window(connection, bar, changed.service_day)
        .await?
        .iter()
        .any(|other| other.booking.id != changed.id && other.booking.holds(table, held));
    if taken {
        Err(Error::TableTaken)
    } else {
        Ok(())
    }
}

/// Everything a booking row is made of.
///
/// One shape, so the two ways a booking comes into existence — somebody chose a time, or somebody
/// walked in — write the same columns. A second `insert into booking` elsewhere would be a second
/// chance to forget one.
struct Written<'a> {
    bar: BarId,
    table: TableId,
    service_day: ServiceDay,
    window: Interval,
    party_size: i32,
    guest_name: &'a str,
    guest_username: Option<&'a str>,
    user: Option<TelegramUserId>,
    source: BookingSource,
    status: StoredStatus,
}

/// Writes the row, letting the exclusion constraint have the last word on who got the table.
async fn insert_booking(
    connection: &mut PgConnection,
    written: &Written<'_>,
) -> Result<BookingId> {
    sqlx::query(
        "insert into booking (bar_id, table_id, service_date, starts_at, ends_at, party_size,
                              guest_name, guest_username, telegram_user_id, source, status)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) returning id",
    )
    .bind(written.bar)
    .bind(written.table.0)
    .bind(written.service_day.date())
    .bind(written.window.start())
    .bind(written.window.end())
    .bind(written.party_size)
    .bind(written.guest_name)
    .bind(written.guest_username)
    .bind(written.user.map(|user| user.0))
    .bind(written.source)
    .bind(written.status)
    .fetch_one(&mut *connection)
    .await
    .map_err(Error::from_write)
    .map(|row| BookingId(row.get("id")))
}

fn check_party_size(party_size: i32, config: &ValidConfig) -> Result<()> {
    if party_size < 1 || party_size > config.max_party {
        return Err(Error::PartyTooLarge {
            party_size,
            max_party: config.max_party,
        });
    }
    Ok(())
}

/// Guests book within the horizon the bar publishes; staff are not bound by it, because a bar
/// takes a telephone booking for next month without arguing about it.
fn check_shift_is_offered(
    request: &NewBooking,
    config: &ValidConfig,
    now: DateTime<Utc>,
) -> Result<()> {
    if !matches!(request.channel, Channel::Guest { .. }) {
        return Ok(());
    }
    if slots::guest_may_book(config, request.service_day, now) {
        Ok(())
    } else {
        Err(Error::ShiftNotBookable {
            service_day: request.service_day,
        })
    }
}

/// The window an arrival time would get, or the reason it cannot be taken.
///
/// The picker's own function, so nobody is refused a time the app had just offered. Taking the
/// picker's query means a booking being moved asks with itself set aside.
fn window_at(query: &slots::Query<'_>, start_minutes: i32) -> Result<Interval> {
    let unavailable = || Error::NotAnArrivalTime {
        minutes: start_minutes,
    };
    let slot = slots::slot_at(query, start_minutes).ok_or_else(unavailable)?;

    match slot.availability {
        SlotAvailability::Free => slot.window.ok_or_else(unavailable),
        SlotAvailability::Taken => Err(Error::NoTableFree {
            party_size: query.party_size,
        }),
        SlotAvailability::Past => Err(Error::InThePast),
        SlotAvailability::Nonexistent => Err(unavailable()),
    }
}

impl Store {
    /// Reads specific bookings, for reports that need to name the guests involved.
    ///
    /// Reconciliation deals in identities because allocation must not be able to see who a guest
    /// is; the report staff read has to say "Тимур", so the names are fetched afterwards.
    pub async fn bookings_by_id(
        &self,
        bar: BarId,
        ids: &[BookingId],
    ) -> Result<Vec<BookingRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(concat!(
            "select ",
            booking_columns!(),
            " where b.bar_id = $1 and b.id = any($2::uuid[])"
        ))
        .bind(bar)
        .bind(ids.iter().map(|id| id.0).collect::<Vec<_>>())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| BookingRecord::try_from(row_into(&row)?))
            .collect()
    }
}

/// Bookings on `day` — what the admin screen draws.
pub(crate) async fn load_shift(
    connection: &mut PgConnection,
    bar: BarId,
    day: ServiceDay,
) -> Result<Vec<BookingRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        booking_columns!(),
        " where b.bar_id = $1 and b.service_date = $2 and b.status <> 'cancelled'
          order by b.starts_at, b.id"
    ))
    .bind(bar)
    .bind(day.date())
    .fetch_all(connection)
    .await?;
    rows.into_iter()
        .map(|row| BookingRecord::try_from(row_into(&row)?))
        .collect()
}

/// Bookings that could possibly conflict with something on `day`.
///
/// The neighbouring shifts are included because a window can outlast midnight, and the allocator
/// judges overlap on absolute time rather than on which shift a booking is filed under.
pub(crate) async fn load_window(
    connection: &mut PgConnection,
    bar: BarId,
    day: ServiceDay,
) -> Result<Vec<BookingRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        booking_columns!(),
        " where b.bar_id = $1 and b.status <> 'cancelled'
            and b.service_date between ($2::date - 1) and ($2::date + 1)
          order by b.starts_at, b.id"
    ))
    .bind(bar)
    .bind(day.date())
    .fetch_all(connection)
    .await?;
    rows.into_iter()
        .map(|row| BookingRecord::try_from(row_into(&row)?))
        .collect()
}

/// Bookings anywhere in a span of shifts, for a screen that asks about many days at once.
///
/// The guest's day rail is the only caller: [`load_window`] answers the allocator's question about
/// one evening and its neighbours, and running it thirty times would be thirty round trips whose
/// answers could disagree with one another.
pub(crate) async fn load_between(
    connection: &mut PgConnection,
    bar: BarId,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> Result<Vec<BookingRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        booking_columns!(),
        " where b.bar_id = $1 and b.status <> 'cancelled'
            and b.service_date between $2 and $3
          order by b.starts_at, b.id"
    ))
    .bind(bar)
    .bind(from)
    .bind(to)
    .fetch_all(connection)
    .await?;
    rows.into_iter()
        .map(|row| BookingRecord::try_from(row_into(&row)?))
        .collect()
}

/// Blocks anywhere in a span of shifts. The rail's companion to [`load_between`].
pub(crate) async fn load_blocks_between(
    connection: &mut PgConnection,
    bar: BarId,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> Result<Vec<BlockRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        block_columns!(),
        " where block.bar_id = $1 and block.service_date between $2 and $3
          order by block.service_date, t.number"
    ))
    .bind(bar)
    .bind(from)
    .bind(to)
    .fetch_all(connection)
    .await?;
    rows.iter().map(block_into).collect()
}

/// Every live booking that has not finished yet.
///
/// The set a settings change is judged against, and the set reconciliation may move. A booking
/// that has already ended is history: it cannot be moved, and counting it would hold the bar's
/// own settings hostage to its past.
pub(crate) async fn load_unfinished(
    connection: &mut PgConnection,
    bar: BarId,
    now: DateTime<Utc>,
) -> Result<Vec<BookingRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        booking_columns!(),
        " where b.bar_id = $1 and b.status <> 'cancelled' and b.ends_at > $2
          order by b.starts_at, b.id"
    ))
    .bind(bar)
    .bind(now)
    .fetch_all(connection)
    .await?;
    rows.into_iter()
        .map(|row| BookingRecord::try_from(row_into(&row)?))
        .collect()
}

/// Blocks that could bear on anything in the window around `day`.
///
/// The neighbouring shifts are included because reconciliation looks at bookings from them, and a
/// booking's soundness is judged against the blocks of *its own* shift. This is the set allocation
/// and reconciliation need — and deliberately not the set a screen should show, which is
/// [`load_blocks_on`]: a view that listed a neighbouring day's closures would be describing an
/// evening that is not on screen.
pub(crate) async fn load_blocks(
    connection: &mut PgConnection,
    bar: BarId,
    day: ServiceDay,
) -> Result<Vec<BlockRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        block_columns!(),
        " where block.bar_id = $1
            and block.service_date between ($2::date - 1) and ($2::date + 1)
          order by t.number"
    ))
    .bind(bar)
    .bind(day.date())
    .fetch_all(connection)
    .await?;
    rows.iter().map(block_into).collect()
}

/// Blocks on `day` and no other, for anything that draws a shift.
pub(crate) async fn load_blocks_on(
    connection: &mut PgConnection,
    bar: BarId,
    day: ServiceDay,
) -> Result<Vec<BlockRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        block_columns!(),
        " where block.bar_id = $1 and block.service_date = $2
          order by t.number"
    ))
    .bind(bar)
    .bind(day.date())
    .fetch_all(connection)
    .await?;
    rows.iter().map(block_into).collect()
}

async fn load_blocks_from(
    connection: &mut PgConnection,
    bar: BarId,
    from: chrono::NaiveDate,
) -> Result<Vec<BlockRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        block_columns!(),
        " where block.bar_id = $1 and block.service_date >= $2
          order by block.service_date, t.number"
    ))
    .bind(bar)
    .bind(from)
    .fetch_all(connection)
    .await?;
    rows.iter().map(block_into).collect()
}

/// Reads a block row column by column, beside the query that selects them.
fn block_into(row: &sqlx::postgres::PgRow) -> Result<BlockRecord> {
    Ok(BlockRecord::from(BlockRow {
        table_id: row.try_get("table_id")?,
        table_number: row.try_get("table_number")?,
        service_date: row.try_get("service_date")?,
        reason: row.try_get("reason")?,
    }))
}

/// Re-seats everything on one shift that the room can no longer honour, and seats anything it
/// now can.
///
/// The single implementation behind closing a table, opening one, cancelling a booking and the
/// staff-facing retry. Four call sites that each grew their own reassignment loop would be four
/// chances to disagree about the awkward cases.
pub(crate) async fn reconcile_shift(
    connection: &mut PgConnection,
    bar: BarId,
    config: &ValidConfig,
    day: ServiceDay,
    now: DateTime<Utc>,
) -> Result<Reseated> {
    let bookings = load_window(&mut *connection, bar, day).await?;
    let blocks = load_blocks(&mut *connection, bar, day).await?;
    let outcome = reconcile(&ReconcileRequest {
        tables: &config.tables,
        bookings: &bookings_of(&bookings),
        blocks: &blocks_of(&blocks),
        now,
    });
    persist_reconciliation(&mut *connection, &outcome).await?;
    Ok(with_affected(outcome, bookings))
}

/// Keeps the records a reconciliation touched, discarding the rest of the window.
pub(crate) fn with_affected(
    outcome: Reconciliation,
    loaded: Vec<BookingRecord>,
) -> Reseated {
    let touched: Vec<BookingId> = outcome
        .moved
        .iter()
        .map(|moved| moved.booking)
        .chain(outcome.orphaned.iter().copied())
        .collect();
    let affected = loaded
        .into_iter()
        .filter(|record| touched.contains(&record.booking.id))
        .collect();
    Reseated { outcome, affected }
}

/// Re-seats everything the room can no longer honour, from now onwards.
pub(crate) async fn reconcile_from(
    connection: &mut PgConnection,
    bar: BarId,
    config: &ValidConfig,
    now: DateTime<Utc>,
) -> Result<Reseated> {
    let bookings = load_unfinished(&mut *connection, bar, now).await?;
    let from = config
        .current_service_day(now)
        .checked_sub_days(1)
        .unwrap_or_else(|| config.current_service_day(now));
    let blocks = load_blocks_from(&mut *connection, bar, from.date()).await?;
    let outcome = reconcile(&ReconcileRequest {
        tables: &config.tables,
        bookings: &bookings_of(&bookings),
        blocks: &blocks_of(&blocks),
        now,
    });
    persist_reconciliation(&mut *connection, &outcome).await?;
    Ok(with_affected(outcome, bookings))
}

/// Writes a reconciliation, in two passes.
///
/// Every affected booking releases its table first, and only then takes its new one. Applying a
/// permutation one row at a time would otherwise trip the exclusion constraint halfway through —
/// moving a party onto a table whose current occupant has not moved off it yet — even though the
/// final arrangement is perfectly sound. The intermediate state is a set of orphans, which hold
/// nothing and cannot collide with anything.
pub(crate) async fn persist_reconciliation(
    connection: &mut PgConnection,
    outcome: &Reconciliation,
) -> Result<()> {
    if outcome.is_empty() {
        return Ok(());
    }
    let releasing: Vec<Uuid> = outcome
        .moved
        .iter()
        .map(|moved| moved.booking.0)
        .chain(outcome.orphaned.iter().map(|orphan| orphan.0))
        .collect();
    sqlx::query("update booking set table_id = null where id = any($1::uuid[])")
        .bind(&releasing)
        .execute(&mut *connection)
        .await?;

    if !outcome.moved.is_empty() {
        let ids: Vec<Uuid> = outcome.moved.iter().map(|moved| moved.booking.0).collect();
        let tables: Vec<Uuid> = outcome.moved.iter().map(|moved| moved.to.0).collect();
        sqlx::query(
            "update booking b set table_id = seating.table_id
             from unnest($1::uuid[], $2::uuid[]) as seating(id, table_id)
             where b.id = seating.id",
        )
        .bind(ids)
        .bind(tables)
        .execute(&mut *connection)
        .await
        .map_err(Error::from_write)?;
    }
    Ok(())
}

/// Cancels `replacing`, the bookings among `mine` a guest's new booking replaces, soonest first, and
/// names the evening each was on.
async fn cancel_replaced(
    connection: &mut PgConnection,
    bar: BarId,
    mine: &[Booking],
    replacing: &[BookingId],
    now: DateTime<Utc>,
) -> Result<Vec<(BookingId, ServiceDay)>> {
    let replaced: Vec<(BookingId, ServiceDay)> = replacing
        .iter()
        .filter_map(|id| {
            mine.iter()
                .find(|booking| booking.id == *id)
                .map(|booking| (*id, booking.service_day))
        })
        .collect();
    if replaced.is_empty() {
        return Ok(replaced);
    }
    sqlx::query(
        "update booking set status = 'cancelled', cancelled_at = $3
         where bar_id = $1 and id = any($2::uuid[])",
    )
    .bind(bar)
    .bind(replaced.iter().map(|(id, _)| id.0).collect::<Vec<_>>())
    .bind(now)
    .execute(&mut *connection)
    .await?;
    for (id, _) in &replaced {
        notifications::abandon_reminder(&mut *connection, *id, "the guest replaced this booking")
            .await?;
    }
    Ok(replaced)
}

/// The guest's bookings whose table is still held for them, soonest first.
///
/// When a booking stops running is [`pustol_domain::Booking::has_finished`] and nothing else. The
/// query only narrows to the rows that could still be running; restating the rule in SQL would give
/// this reading an opinion of its own.
async fn running_bookings_of_guest(
    connection: &mut PgConnection,
    bar: BarId,
    user: TelegramUserId,
    now: DateTime<Utc>,
) -> Result<Vec<BookingRecord>> {
    let rows = sqlx::query(concat!(
        "select ",
        booking_columns!(),
        " where b.bar_id = $1 and b.telegram_user_id = $2
            and b.status <> 'cancelled' and b.ends_at > $3
          order by b.starts_at"
    ))
    .bind(bar)
    .bind(user.0)
    .bind(now)
    .fetch_all(connection)
    .await?;
    let mut running = Vec::new();
    for row in &rows {
        let record = BookingRecord::try_from(row_into(row)?)?;
        if !record.booking.has_finished(now) {
            running.push(record);
        }
    }
    Ok(running)
}

/// [`running_bookings_of_guest`] for `user`, and none for a booking with no guest behind it.
async fn running_bookings_of(
    connection: &mut PgConnection,
    bar: BarId,
    user: Option<TelegramUserId>,
    now: DateTime<Utc>,
) -> Result<Vec<BookingRecord>> {
    match user {
        Some(user) => running_bookings_of_guest(connection, bar, user, now).await,
        None => Ok(Vec::new()),
    }
}

/// Refuses a guest's booking whose app promised it replaces other bookings than `replacing`, the ones
/// it does: whatever else it is, it is not the booking the guest agreed to.
fn check_promise(channel: &Channel, replacing: &[BookingId]) -> Result<()> {
    match channel {
        Channel::Guest {
            replacing: promised,
            ..
        } if !same_bookings(replacing, promised) => Err(Error::BookingChanged),
        Channel::Guest { .. } | Channel::Staff { .. } => Ok(()),
    }
}

/// Whether two lists name the same bookings, whatever their order and however often.
fn same_bookings(one: &[BookingId], other: &[BookingId]) -> bool {
    let set = |ids: &[BookingId]| {
        ids.iter()
            .map(|id| id.0)
            .collect::<std::collections::BTreeSet<_>>()
    };
    set(one) == set(other)
}

/// Refuses a change to `booking` that leaves its guest holding what no guest may hold.
///
/// Asked of [`pustol_domain::rebooking::holding_conflict`] once the change is written and before it
/// commits, under the bar's lock rather than by an index: whether a booking still runs depends on the
/// clock. A booking with no account behind it has no guest the rule is about, and a record corrected
/// after its evening is over holds nothing, so neither is ever refused.
async fn check_holdings(
    connection: &mut PgConnection,
    bar: BarId,
    user: Option<TelegramUserId>,
    booking: BookingId,
    now: DateTime<Utc>,
) -> Result<()> {
    let Some(user) = user else {
        return Ok(());
    };
    let mine = bookings_of(&running_bookings_of_guest(connection, bar, user, now).await?);
    match rebooking::holding_conflict(&mine, booking, now) {
        Some(HoldingConflict::SameEvening) => Err(Error::AlreadyBookedThisShift),
        Some(HoldingConflict::AnotherPlan) => Err(Error::GuestHasAnotherPlan),
        None => Ok(()),
    }
}

/// Cancels a booking inside a transaction that already holds the bar's lock and read `config`.
async fn cancel_locked(
    connection: &mut PgConnection,
    bar: BarId,
    config: ValidConfig,
    booking: BookingId,
    reason: Option<&str>,
    notice: Option<CancellationWording>,
    now: DateTime<Utc>,
) -> Result<CancelledBooking> {
    // Checked here, against the configuration this transaction read, rather than in whichever
    // handler happens to call: a rule about what reaches a guest should not depend on every
    // future caller remembering it.
    if let Some(reason) = reason
        && !config.cancel_reasons.iter().any(|offered| offered == reason)
    {
        return Err(Error::UnknownCancelReason);
    }
    fetch_unfinished(&mut *connection, bar, booking, now).await?;
    sqlx::query(
        "update booking set status = 'cancelled', cancelled_at = $4, cancel_reason = $3
         where bar_id = $1 and id = $2",
    )
    .bind(bar)
    .bind(booking.0)
    .bind(reason)
    .bind(now)
    .execute(&mut *connection)
    .await?;
    notifications::abandon_reminder(&mut *connection, booking, "the booking was cancelled").await?;
    let record = fetch_booking(&mut *connection, bar, booking).await?;

    // Queued in the same transaction as the cancellation: a notice that survives a crash the
    // cancellation did not would tell a guest their table is gone when it is not.
    let mut notice_queued = false;
    if let (Some(recipient), Some(reason), Some(wording)) = (record.telegram_user_id, reason, notice)
    {
        notifications::enqueue(
            &mut *connection,
            bar,
            booking,
            recipient,
            notifications::NotificationKind::Cancelled,
            &wording(&config, &record, reason),
            now,
        )
        .await?;
        notice_queued = true;
    }

    let day = record.booking.service_day;
    let reconciliation = reconcile_shift(&mut *connection, bar, &config, day, now).await?;
    let guest_notified = notice_queued && record.reachable_by_bot;
    let evening = read_evening(&mut *connection, bar, config, day, now).await?;
    Ok(CancelledBooking {
        record,
        reconciliation,
        guest_notified,
        evening,
    })
}

/// A booking that is still a plan, read under the bar's lock.
///
/// A party that went home or never came is the record of an evening, not a plan: nothing about it
/// can be moved or given back, and a guest told otherwise would be told about an evening that
/// happened. When that is, is [`pustol_domain::Booking::has_finished`] and nothing else.
async fn fetch_unfinished(
    connection: &mut PgConnection,
    bar: BarId,
    booking: BookingId,
    now: DateTime<Utc>,
) -> Result<BookingRecord> {
    let record = fetch_booking(connection, bar, booking).await?;
    if !record.booking.status.is_live() {
        return Err(Error::NotFound { entity: "booking" });
    }
    if record.booking.has_finished(now) {
        return Err(Error::BookingHasFinished);
    }
    Ok(record)
}

pub(crate) async fn fetch_booking(
    connection: &mut PgConnection,
    bar: BarId,
    booking: BookingId,
) -> Result<BookingRecord> {
    let row = sqlx::query(concat!(
        "select ",
        booking_columns!(),
        " where b.bar_id = $1 and b.id = $2"
    ))
    .bind(bar)
    .bind(booking.0)
    .fetch_optional(connection)
    .await?
    .ok_or(Error::NotFound { entity: "booking" })?;
    BookingRecord::try_from(row_into(&row)?)
}

/// Reads a booking row column by column.
///
/// Written out rather than derived so that a column renamed in a migration fails here, next to
/// the query that selects it, instead of somewhere further downstream.
fn row_into(row: &sqlx::postgres::PgRow) -> Result<BookingRow> {
    Ok(BookingRow {
        id: row.try_get("id")?,
        table_id: row.try_get("table_id")?,
        table_number: row.try_get("table_number")?,
        table_zone: row.try_get("table_zone")?,
        service_date: row.try_get("service_date")?,
        starts_at: row.try_get("starts_at")?,
        ends_at: row.try_get("ends_at")?,
        left_at: row.try_get("left_at")?,
        party_size: row.try_get("party_size")?,
        guest_name: row.try_get("guest_name")?,
        guest_username: row.try_get("guest_username")?,
        telegram_user_id: row.try_get("telegram_user_id")?,
        reachable_by_bot: row.try_get("reachable_by_bot")?,
        status: row.try_get("status")?,
        source: row.try_get("source")?,
        note: row.try_get("note")?,
        cancel_reason: row.try_get("cancel_reason")?,
    })
}
