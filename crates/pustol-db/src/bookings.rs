//! Taking bookings, moving them, and closing tables.

use chrono::{DateTime, Utc};
use pustol_domain::allocator::{Assignment, Booking, BookingId, TableBlock};
use pustol_domain::config::ValidConfig;
use pustol_domain::rebooking::{self, HoldingConflict};
use pustol_domain::reconcile::{Request as ReconcileRequest, reconcile};
use pustol_domain::schedule::{BarTable, TableId};
use pustol_domain::service_day::ServiceDay;
use pustol_domain::slots::{self, SlotAvailability};
use pustol_domain::{
    BlockReason, BookingStatus, GuestName, Interval, Reconciliation, bookable_days, longer_than,
    minutes_within,
};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::bar::load_config;
use crate::error::{Error, Result};
use crate::evening::{Evening, ShiftRows, evening_around, read_evening, read_shift};
use crate::ids::{BarId, TelegramUserId};
use crate::records::{
    BlockRecord, BlockRow, BookingRecord, BookingRow, BookingSource, StoredStatus, blocks_of,
    bookings_of,
};
use crate::{Store, lock_bar, notifications};

/// Block columns, joined to table for printed number.
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

/// Booking columns, joined to table and to guest account for `reachable_by_bot`. Macro for same
/// reason as `block_columns`.
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
            Self::NoShow => {
                now.max(window.start() + chrono::Duration::minutes(i64::from(grace_minutes)))
            }
        };
        Some(moment.clamp(window.start(), window.end()))
    }

    #[must_use]
    pub fn status(self) -> BookingStatus {
        BookingStatus::from(StoredStatus::from(self))
    }

    /// `None` for cancelled.
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
    /// Guest in Mini App. Bound by booking horizon and [`pustol_domain::rebooking`].
    Guest {
        user: TelegramUserId,
        name: String,
        username: Option<String>,
        /// Bookings guest's app said this one replaces; guest agreed to that. Taken only if it
        /// replaces exactly these, as set.
        replacing: Vec<BookingId>,
    },
    /// Staff, taking a booking by telephone or at the door. Not bound by the horizon: a bar takes
    /// a booking for next month over the phone without arguing about it.
    ///
    /// `table` is the one staff chose; `None` asks the room. It lives here, not on the request,
    /// so a guest booking has no field for it at all.
    Staff {
        guest_name: GuestName,
        table: Option<TableId>,
    },
}

impl Channel {
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

/// Words moved-booking notice from booking before and after move.
pub type MoveWording = fn(&ValidConfig, &BookingRecord, &BookingRecord) -> String;

/// What the bot says when a booking moves: the notice now, and the reminder that would otherwise
/// still name the old hour. Together, so a move cannot remember one and forget the other.
#[derive(Clone, Copy, Debug)]
pub struct MoveWords {
    pub notice: MoveWording,
    pub reminder: ReminderWording,
}

/// Move target, named in full as one act.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveTo {
    /// Wall-clock arrival. Booking's own when only table or party changes.
    pub start_minutes: i32,
    /// `None` asks room for best fit.
    pub table: Option<TableId>,
    /// `None` keeps party size.
    pub party_size: Option<i32>,
}

/// A booking that now exists.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreatedBooking {
    pub record: BookingRecord,
    /// Guest's earlier bookings cancelled for this one, soonest first.
    pub replaced: Vec<BookingId>,
    /// Guest's bookings still holding table after this take, this one included, soonest first.
    /// Empty without guest. Rebooking decisions depend on them, so read like `evening`.
    pub guest_bookings: Vec<BookingRecord>,
    /// Evening as take left it, with config it was taken under.
    ///
    /// Read in taking transaction before commit, not by caller later: second read could see
    /// concurrent change or settings save, and could fail after booking was taken.
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
    /// Attendance just before change, read in same transaction. Undo returns here, not to what
    /// screen last saw.
    pub previous: Attendance,
    /// Parties the released table let the room seat. Empty when nothing moved.
    pub reconciliation: Reseated,
    /// Evening as change left it, read before commit.
    pub evening: Evening,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NoteWritten {
    pub record: BookingRecord,
    /// Evening as note left it, read before commit.
    pub evening: Evening,
}

/// A booking that has been released, and whatever the freed table let the room fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CancelledBooking {
    pub record: BookingRecord,
    /// Bookings the freed table allowed to be seated.
    pub reconciliation: Reseated,
    /// Notice queued and bot can reach guest.
    ///
    /// Decided here: needs reason, account and reachability, all held by this transaction. Notice
    /// still queued for unreachable account, since reachability is only what last delivery learned
    /// and guest may unblock bot; but nobody is told guest knows.
    pub guest_notified: bool,
    /// Evening as cancellation left it, read before commit.
    pub evening: Evening,
}

/// A booking that now sits somewhere else, or at some other time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MovedBooking {
    pub record: BookingRecord,
    /// Whatever the table they left allowed the room to settle.
    pub reconciliation: Reseated,
    /// Only time change is news, and only when bot can reach guest. Notice queued either way, like
    /// cancellation's.
    pub guest_notified: bool,
    /// Evening as move left it, read before commit.
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
    /// Guest holds this evening with booking rebooking cannot replace, so booking here refused
    /// whatever time is free.
    pub booked: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClosedTables {
    /// In order asked. Tables already shut excluded.
    pub closed: Vec<TableId>,
    pub reconciliation: Reseated,
    /// Evening as closure left it, read before commit.
    pub evening: Evening,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReopenedTables {
    /// Removed closures, in order asked. Tables not shut excluded.
    pub reopened: Vec<ReopenedTable>,
    pub reconciliation: Reseated,
    /// Evening as reopening left it, read before commit.
    pub evening: Evening,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReopenedTable {
    pub table_id: TableId,
    pub reason: String,
}

/// One evening as allocation sees it, with config in force.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Room {
    pub config: ValidConfig,
    pub day: ServiceDay,
    /// Live bookings, neighbour evenings included: window can outlast midnight.
    bookings: Vec<Booking>,
    /// Closures, neighbour evenings included.
    blocks: Vec<TableBlock>,
}

impl Room {
    /// Slot picker query over this room, `ignoring` set aside.
    #[must_use]
    pub fn query<'a>(
        &'a self,
        party_size: i32,
        now: DateTime<Utc>,
        ignoring: &'a [BookingId],
    ) -> slots::Query<'a> {
        slots::Query {
            config: &self.config,
            service_day: self.day,
            party_size,
            bookings: &self.bookings,
            blocks: &self.blocks,
            now,
            ignoring,
        }
    }

    #[must_use]
    pub fn booking(&self, id: BookingId) -> Option<&Booking> {
        self.bookings.iter().find(|booking| booking.id == id)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReconciledShift {
    pub reconciliation: Reseated,
    /// Evening as attempt left it, read before commit.
    pub evening: Evening,
}

impl Store {
    /// Room on one shift, for asking arrival times and free tables.
    ///
    /// No lock: availability is advice, true at the moment it was read. The guarantee that two
    /// guests cannot both act on it lives in [`Self::create_booking`] and, beneath that, in the
    /// exclusion constraint.
    ///
    /// # Errors
    ///
    /// Bar not found, stored config unusable, database failure.
    pub async fn room(&self, bar: BarId, day: ServiceDay) -> Result<Room> {
        let mut connection = self.pool().acquire().await?;
        let config = load_config(&mut connection, bar).await?;
        let (bookings, blocks) = load_room(&mut connection, bar, day).await?;
        Ok(Room {
            config,
            day,
            bookings,
            blocks,
        })
    }

    /// Shifts a guest may choose from.
    ///
    /// # Errors
    ///
    /// Bar not found, stored config unusable, database failure.
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
    /// `guest`: every booking of asking guest, empty for nobody. Each day ignores exactly those a
    /// booking that day would replace, so guest's own plan never makes evening look full, and
    /// `booked` marks days guest is refused.
    ///
    /// # Errors
    ///
    /// Database failure.
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
    /// Refusal order, widest first: party, evening, time, table. Promised replacements checked
    /// last, only for booking otherwise takeable.
    ///
    /// # Errors
    ///
    /// `PartyTooLarge`, `ShiftNotBookable`, `AlreadyBookedThisShift`, `NotAnArrivalTime`,
    /// `InThePast`, `NoTableFree`, `ChosenTableNotFree`, `BookingChanged`,
    /// `TableTakenConcurrently`, or database failure.
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

        // Rebooking effect decided by `rebooking` only, over rows read under bar lock.
        let user = request.channel.guest();
        let held = running_bookings_of(&mut transaction, request.bar, user, now).await?;
        let mine = bookings_of(&held);
        if rebooking::refused_on(&mine, request.service_day, now) {
            return Err(Error::AlreadyBookedThisShift);
        }
        let replacing = rebooking::replaced_on(&mine, request.service_day, now);

        let (live, closed) = load_room(&mut transaction, request.bar, request.service_day).await?;
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
                (BookingSource::App, name.as_str(), username.as_deref(), None)
            }
            Channel::Staff { guest_name, table } => {
                (BookingSource::Staff, guest_name.as_str(), None, *table)
            }
        };
        let free = pustol_domain::free_tables(&asking.request(window));
        let table = seat_of(&free, chosen, request.party_size)?.table_id;
        check_promise(&request.channel, &replacing)?;

        // Same transaction as take: guest never holds two or none.
        let replaced =
            cancel_replaced(&mut transaction, request.bar, &mine, &replacing, now).await?;
        let id = insert_booking(
            &mut transaction,
            &Written {
                bar: request.bar,
                table,
                service_day: request.service_day,
                window,
                party_size: request.party_size,
                guest_name: name,
                guest_username: username,
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

        // Replaced booking frees capacity on its evening; offer it to that evening's orphans.
        let mut evenings: Vec<ServiceDay> = replaced.iter().map(|(_, day)| *day).collect();
        evenings.sort_unstable();
        evenings.dedup();
        let mut room_moved = false;
        for day in evenings {
            let reconciled =
                reconcile_shift(&mut transaction, request.bar, &config, day, now).await?;
            room_moved |= !reconciled.reseated.is_empty();
        }

        let record = fetch_booking(&mut transaction, request.bar, id).await?;
        let guest_bookings = if room_moved {
            running_bookings_of(&mut transaction, request.bar, user, now).await?
        } else {
            held_after(held, &replacing, user.map(|_| record.clone()))
        };
        let evening = read_evening(
            &mut transaction,
            request.bar,
            config,
            request.service_day,
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(CreatedBooking {
            record,
            replaced: replaced.into_iter().map(|(id, _)| id).collect(),
            guest_bookings,
            evening,
        })
    }

    /// Guest's running bookings, soonest first.
    ///
    /// Seated guest still sees booking, plus plan for another evening. Booking whose table went
    /// back to pool (went home, or no-show past grace) excluded: nothing left to move or give
    /// back. Rule is [`pustol_domain::Booking::has_finished`] only.
    ///
    /// # Errors
    ///
    /// Database failure.
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
    /// the room returns to the arrangement it had, rather than to one that merely looks like it.
    ///
    /// # Errors
    ///
    /// `NotFound` for missing or cancelled booking, `TableTaken` when table went to another party,
    /// `AlreadyBookedThisShift` or `GuestHasAnotherPlan` when guest would hold too much, or
    /// database failure.
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
        let released_at = attendance.released_at(current.booking.window, config.grace_minutes, now);
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

        check_holdings(
            &mut transaction,
            bar,
            current.telegram_user_id,
            booking,
            now,
        )
        .await?;
        let day = current.booking.service_day;
        let reconciled = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        let record = reconciled.booking(&mut transaction, bar, booking).await?;
        let (reconciliation, evening) = reconciled
            .evening(&mut transaction, bar, config, now)
            .await?;
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
    /// Under bar lock like every booking change, so returned evening is room exactly as this write
    /// left it.
    ///
    /// # Errors
    ///
    /// `NoteTooLong`, `NotFound` for missing or cancelled booking, or database failure.
    pub async fn set_note(
        &self,
        bar: BarId,
        booking: BookingId,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<NoteWritten> {
        let trimmed = note.map(str::trim).filter(|text| !text.is_empty());
        if trimmed.is_some_and(|text| longer_than(text, NOTE_MAX_CHARS)) {
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
        let evening = read_evening(
            &mut transaction,
            bar,
            config,
            record.booking.service_day,
            now,
        )
        .await?;
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
    ///
    /// # Errors
    ///
    /// `NotFound`, `BookingHasFinished`, `BookingHasStarted` on time change, `PartyTooLarge`,
    /// slot or table refusals as in [`Self::create_booking`], `AlreadyBookedThisShift`,
    /// `GuestHasAnotherPlan`, or database failure.
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

        // Cap checked only on size change: booking taken before cap lowered stays movable.
        let party_size = to.party_size.unwrap_or(current.booking.party_size);
        if party_size != current.booking.party_size {
            check_party_size(party_size, &config)?;
        }

        let day = current.booking.service_day;
        let (live, closed) = load_room(&mut transaction, bar, day).await?;
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

        // Recorded attendance belongs to old time. New time makes booking plan again; old release
        // would also fall outside new window.
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

        check_holdings(
            &mut transaction,
            bar,
            current.telegram_user_id,
            booking,
            now,
        )
        .await?;
        // The table they left is capacity appearing, and capacity appearing is offered to whoever
        // the room could not seat — the same rule a cancellation and a party going home follow.
        let reconciled = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        let record = reconciled.booking(&mut transaction, bar, booking).await?;
        let mut notice_queued = false;
        if let (Some(recipient), Some(words)) = (current.telegram_user_id, words) {
            // Only new time is news: guest never saw table number, and size change is their own
            // request. Reminder names hour and party, so follows either.
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
        let (reconciliation, evening) = reconciled
            .evening(&mut transaction, bar, config, now)
            .await?;
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
    /// Offer is [`pustol_domain::walk_in`], same one shift screen draws walk-in tables and
    /// "who fits" from: turn cut short at closing, table free for all of it.
    ///
    /// # Errors
    ///
    /// `NotTheRunningShift` on any other day or while shift not open, `PartyTooLarge`,
    /// `ChosenTableNotFree`, `NoTableFree`, `TableTakenConcurrently`, or database failure.
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
        let (bookings, blocks) = load_room(&mut transaction, bar, day).await?;
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
    ///
    /// # Errors
    ///
    /// `UnknownCancelReason`, `NotFound`, `BookingHasFinished`, or database failure.
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

    /// Guest cancels one own booking.
    ///
    /// Another guest's booking is `NotFound` in any state: guest learns nothing about it. Own
    /// booking refused as finished once table no longer held, same as staff.
    ///
    /// Check and release in one transaction: separate lookup could race staff cancel, and guest
    /// would hear of failure while table is already free.
    ///
    /// # Errors
    ///
    /// `NotFound`, `BookingHasFinished`, or database failure.
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

    /// Guest cancels booking from button under its reminder.
    ///
    /// Only while still `confirmed`: reminder tap can come late, party may already sit at table.
    ///
    /// # Errors
    ///
    /// `NotFound` unless guest's running confirmed booking, or database failure.
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
        let cancelled = cancel_locked(
            &mut transaction,
            bar,
            config,
            mine.booking.id,
            None,
            None,
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(cancelled)
    }

    /// Asks the room, one more time, to seat everything it owes a table.
    ///
    /// The staff-facing "find a table" action. It is the same reconciliation that runs whenever
    /// the room changes, offered as a button because staff sometimes know something has freed up
    /// before the system does — and because being told plainly that there is still nowhere to put
    /// a party is itself the answer they need.
    ///
    /// # Errors
    ///
    /// Bar not found, stored config unusable, `TableTakenConcurrently`, or database failure.
    pub async fn reconcile_shift(
        &self,
        bar: BarId,
        day: ServiceDay,
        now: DateTime<Utc>,
    ) -> Result<ReconciledShift> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let (reconciliation, evening) =
            reconcile_and_read(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(ReconciledShift {
            reconciliation,
            evening,
        })
    }

    /// Takes tables out of service for a shift and re-seats whoever was sitting at them.
    ///
    /// Reports rows written, not tables asked: table closed twice is not news.
    ///
    /// # Errors
    ///
    /// `NotFound` when any table is not live in this room (nothing written), or database failure.
    pub async fn block_tables(
        &self,
        bar: BarId,
        day: ServiceDay,
        tables: &[TableId],
        reason: &BlockReason,
        by: Option<TelegramUserId>,
        now: DateTime<Utc>,
    ) -> Result<ClosedTables> {
        let (mut transaction, config) = self.changing_tables(bar, tables).await?;
        let mut closed = Vec::new();
        if !tables.is_empty() {
            let rows = sqlx::query(
                "insert into table_block (bar_id, table_id, service_date, reason, created_by)
                 select $1, table_id, $2, $3, $4 from unnest($5::uuid[]) as table_id
                 on conflict (table_id, service_date) do nothing
                 returning table_id",
            )
            .bind(bar)
            .bind(day.date())
            .bind(reason.as_str())
            .bind(by.map(|user| user.0))
            .bind(tables.iter().map(|table| table.0).collect::<Vec<_>>())
            .fetch_all(&mut *transaction)
            .await?;
            let inserted: Vec<TableId> = rows
                .iter()
                .map(|row| Ok(TableId(row.try_get("table_id")?)))
                .collect::<Result<_>>()?;
            closed = in_order_asked(tables, inserted, |table| *table);
        }
        let (reconciliation, evening) =
            reconcile_and_read(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(ClosedTables {
            closed,
            reconciliation,
            evening,
        })
    }

    /// Puts tables back into service. Reconciliation runs afterwards because a freed table may be
    /// exactly what an orphaned booking has been waiting for.
    ///
    /// Reports rows removed: table never shut is not news.
    ///
    /// # Errors
    ///
    /// `NotFound` when any table is not live in this room (nothing written), or database failure.
    pub async fn unblock_tables(
        &self,
        bar: BarId,
        day: ServiceDay,
        tables: &[TableId],
        now: DateTime<Utc>,
    ) -> Result<ReopenedTables> {
        let (mut transaction, config) = self.changing_tables(bar, tables).await?;
        let mut reopened = Vec::new();
        if !tables.is_empty() {
            let rows = sqlx::query(
                "delete from table_block
                 where bar_id = $1 and service_date = $2 and table_id = any($3::uuid[])
                 returning table_id, reason",
            )
            .bind(bar)
            .bind(day.date())
            .bind(tables.iter().map(|table| table.0).collect::<Vec<_>>())
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
            reopened = in_order_asked(tables, removed, |table| table.table_id);
        }
        let (reconciliation, evening) =
            reconcile_and_read(&mut transaction, bar, config, day, now).await?;
        transaction.commit().await?;
        Ok(ReopenedTables {
            reopened,
            reconciliation,
            evening,
        })
    }

    /// Locked transaction and its config for changing closures of `tables`. `NotFound` unless all
    /// are live tables of this room: unknown id fails foreign key, other bar's table would be
    /// stored against this bar.
    async fn changing_tables(
        &self,
        bar: BarId,
        tables: &[TableId],
    ) -> Result<(sqlx::Transaction<'static, sqlx::Postgres>, ValidConfig)> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        if tables
            .iter()
            .any(|asked| !config.active_tables().any(|table| table.id == *asked))
        {
            return Err(Error::NotFound { entity: "table" });
        }
        Ok((transaction, config))
    }
}

/// Sorts `changed` by `asked` order: statement returns rows unordered.
fn in_order_asked<T>(
    asked: &[TableId],
    mut changed: Vec<T>,
    table: impl Fn(&T) -> TableId,
) -> Vec<T> {
    changed.sort_by_key(|item| asked.iter().position(|id| *id == table(item)));
    changed
}

/// Staff choice, else room's first pick. `free` is every table room could seat party at, in pick
/// order; choice accepted only when among them, so choosing is as safe as allocation.
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

/// Refuses `changed` holding its table while another party holds it.
///
/// Status restore picks no table, so allocator never asked; same occupancy rule asked here.
/// Exclusion constraint would refuse too, but as lost race, impossible under bar lock.
async fn check_table_free(
    connection: &mut PgConnection,
    bar: BarId,
    changed: &Booking,
) -> Result<()> {
    let (Some(table), Some(held)) = (changed.table_id, changed.occupancy()) else {
        return Ok(());
    };
    let window = bookings_of(&load_window(connection, bar, changed.service_day).await?);
    if pustol_domain::free_during(table, held, &window, &[changed.id]) {
        Ok(())
    } else {
        Err(Error::TableTaken)
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
async fn insert_booking(connection: &mut PgConnection, written: &Written<'_>) -> Result<BookingId> {
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

/// Domain bookings and closures around `day`, neighbour evenings included.
async fn load_room(
    connection: &mut PgConnection,
    bar: BarId,
    day: ServiceDay,
) -> Result<(Vec<Booking>, Vec<TableBlock>)> {
    let bookings = bookings_of(&load_window(&mut *connection, bar, day).await?);
    let blocks = blocks_of(&load_blocks(connection, bar, day).await?);
    Ok((bookings, blocks))
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
/// Single reassignment loop behind every room change and staff retry, so callers never disagree
/// about awkward cases.
pub(crate) async fn reconcile_shift(
    connection: &mut PgConnection,
    bar: BarId,
    config: &ValidConfig,
    day: ServiceDay,
    now: DateTime<Utc>,
) -> Result<Reconciled> {
    let bookings = load_window(&mut *connection, bar, day).await?;
    let blocks = load_blocks(&mut *connection, bar, day).await?;
    let outcome = reconcile(&ReconcileRequest {
        tables: &config.tables,
        bookings: &bookings_of(&bookings),
        blocks: &blocks_of(&blocks),
        now,
    });
    if outcome.is_empty() {
        return Ok(Reconciled {
            day,
            reseated: Reseated {
                outcome,
                affected: Vec::new(),
            },
            unmoved: Some((bookings, blocks)),
        });
    }
    persist_reconciliation(&mut *connection, &outcome).await?;
    Ok(Reconciled {
        day,
        reseated: with_affected(outcome, bookings),
        unmoved: None,
    })
}

pub(crate) struct Reconciled {
    day: ServiceDay,
    pub(crate) reseated: Reseated,
    /// Rows reconciliation read, kept only when it moved nothing, so they still match database.
    unmoved: Option<(Vec<BookingRecord>, Vec<BlockRecord>)>,
}

impl Reconciled {
    /// From rows in hand when present, else read again.
    async fn booking(
        &self,
        connection: &mut PgConnection,
        bar: BarId,
        booking: BookingId,
    ) -> Result<BookingRecord> {
        let held = self
            .unmoved
            .as_ref()
            .and_then(|(bookings, _)| bookings.iter().find(|record| record.booking.id == booking));
        match held {
            Some(record) => Ok(record.clone()),
            None => fetch_booking(connection, bar, booking).await,
        }
    }

    async fn evening(
        self,
        connection: &mut PgConnection,
        bar: BarId,
        config: ValidConfig,
        now: DateTime<Utc>,
    ) -> Result<(Reseated, Evening)> {
        let day = self.day;
        let rows = match self.unmoved {
            Some((bookings, blocks)) => ShiftRows {
                day,
                bookings: bookings
                    .into_iter()
                    .filter(|record| record.booking.service_day == day)
                    .collect(),
                blocks: blocks
                    .into_iter()
                    .filter(|block| block.block.service_day == day)
                    .collect(),
            },
            None => read_shift(&mut *connection, bar, day).await?,
        };
        let evening = evening_around(connection, bar, config, rows, now).await?;
        Ok((self.reseated, evening))
    }
}

async fn reconcile_and_read(
    connection: &mut PgConnection,
    bar: BarId,
    config: ValidConfig,
    day: ServiceDay,
    now: DateTime<Utc>,
) -> Result<(Reseated, Evening)> {
    let reconciled = reconcile_shift(&mut *connection, bar, &config, day, now).await?;
    reconciled.evening(connection, bar, config, now).await
}

/// Guest's running bookings after take when room moved nothing: `held` minus `replaced`, plus
/// `taken`, soonest first.
fn held_after(
    held: Vec<BookingRecord>,
    replaced: &[BookingId],
    taken: Option<BookingRecord>,
) -> Vec<BookingRecord> {
    let mut running: Vec<BookingRecord> = held
        .into_iter()
        .filter(|record| !replaced.contains(&record.booking.id))
        .chain(taken)
        .collect();
    running.sort_by_key(|record| record.booking.window.start());
    running
}

/// Keeps the records a reconciliation touched, discarding the rest of the window.
pub(crate) fn with_affected(outcome: Reconciliation, loaded: Vec<BookingRecord>) -> Reseated {
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

/// Cancels `replacing` among `mine`; returns each with evening it was on.
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

/// Guest's bookings whose table is still held, soonest first.
///
/// SQL only narrows candidates; [`pustol_domain::Booking::has_finished`] decides, so SQL never
/// grows its own rule.
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

/// Refuses guest booking whose app promised other replacements than `replacing`: not what guest
/// agreed to.
fn check_promise(channel: &Channel, replacing: &[BookingId]) -> Result<()> {
    match channel {
        Channel::Guest {
            replacing: promised,
            ..
        } if !same_bookings(replacing, promised) => Err(Error::BookingChanged),
        Channel::Guest { .. } | Channel::Staff { .. } => Ok(()),
    }
}

/// Set equality: order and repeats ignored.
fn same_bookings(one: &[BookingId], other: &[BookingId]) -> bool {
    let set = |ids: &[BookingId]| {
        ids.iter()
            .map(|id| id.0)
            .collect::<std::collections::BTreeSet<_>>()
    };
    set(one) == set(other)
}

/// Refuses change to `booking` that leaves its guest holding what no guest may hold.
///
/// Asks [`pustol_domain::rebooking::holding_conflict`] after write, before commit, under bar lock,
/// not index: whether booking still runs depends on clock. No account, or record fixed after its
/// evening ended, never refused.
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

/// Caller holds bar lock and read `config` in same transaction.
async fn cancel_locked(
    connection: &mut PgConnection,
    bar: BarId,
    config: ValidConfig,
    booking: BookingId,
    reason: Option<&str>,
    notice: Option<CancellationWording>,
    now: DateTime<Utc>,
) -> Result<CancelledBooking> {
    // Checked here against this transaction's config, not in handlers: guest-facing rule must not
    // rely on every caller remembering it.
    if let Some(reason) = reason
        && !config
            .cancel_reasons
            .iter()
            .any(|offered| offered == reason)
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

    // Same transaction as cancel: notice must never survive crash that undid cancellation.
    let mut notice_queued = false;
    if let (Some(recipient), Some(reason), Some(wording)) =
        (record.telegram_user_id, reason, notice)
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
    let guest_notified = notice_queued && record.reachable_by_bot;
    let (reconciliation, evening) = reconcile_and_read(connection, bar, config, day, now).await?;
    Ok(CancelledBooking {
        record,
        reconciliation,
        guest_notified,
        evening,
    })
}

/// Live booking whose table is still held, read under bar lock.
///
/// Party gone home or no-show past grace is record, not plan: nothing to move or give back. Rule is
/// [`pustol_domain::Booking::has_finished`] only.
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
