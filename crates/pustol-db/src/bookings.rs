//! Taking bookings, moving them, and closing tables.

use chrono::{DateTime, Utc};
use pustol_domain::allocator::BookingId;
use pustol_domain::config::ValidConfig;
use pustol_domain::schedule::TableId;
use pustol_domain::service_day::ServiceDay;
use pustol_domain::slots::{self, Slot, SlotAvailability};
use pustol_domain::reconcile::{Request as ReconcileRequest, reconcile};
use pustol_domain::{Interval, Reconciliation, bookable_days};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::bar::load_config;
use crate::error::{Error, Result};
use crate::ids::{BarId, TelegramUserId};
use crate::records::{
    BlockRecord, BlockRow, BookingRecord, BookingRow, BookingSource, StoredStatus, blocks_of,
    bookings_of,
};
use crate::{Store, lock_bar, notifications};

/// The columns every booking read needs, joined to the table for its printed number.
///
/// A macro rather than a constant because `sqlx` accepts only `&'static str` query text — a
/// deliberate guard against interpolated SQL. Expanding literals keeps the column list in one
/// place while every query stays a compile-time constant.
/// The columns every block read needs, joined to the table for its printed number.
macro_rules! block_columns {
    () => {
        "block.table_id, t.number as table_number, block.service_date, block.reason
         from table_block block join bar_table t on t.id = block.table_id"
    };
}

macro_rules! booking_columns {
    () => {
        "b.id, b.table_id, t.number as table_number, t.zone as table_zone, b.service_date,
         b.starts_at, b.ends_at, b.party_size, b.guest_name, b.guest_username,
         b.telegram_user_id, b.status, b.source, b.cancel_reason
         from booking b left join bar_table t on t.id = b.table_id"
    };
}

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
    /// Never came. The table stays theirs until somebody cancels it.
    NoShow,
}

impl From<Attendance> for StoredStatus {
    fn from(attendance: Attendance) -> Self {
        match attendance {
            Attendance::Confirmed => Self::Confirmed,
            Attendance::Arrived => Self::Arrived,
            Attendance::NoShow => Self::NoShow,
        }
    }
}

/// Who is asking for a table.
#[derive(Clone, Debug)]
pub enum Channel {
    /// A guest, in the Mini App. Bound by the booking horizon, and holding at most one booking
    /// that has not yet started.
    Guest {
        user: TelegramUserId,
        name: String,
        username: Option<String>,
    },
    /// Staff, taking a booking by telephone or at the door. Not bound by the horizon: a bar takes
    /// a booking for next month over the phone without arguing about it.
    Staff { guest_name: String },
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

/// A booking that now exists.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreatedBooking {
    pub record: BookingRecord,
    /// The guest's previous booking, cancelled to make room for this one.
    pub replaced: Option<BookingId>,
    /// The configuration the booking was taken under.
    ///
    /// Returned rather than left for the caller to read again: this is the one the transaction
    /// actually used, and a second read could observe a settings save that happened in between.
    pub config: ValidConfig,
}

/// A booking that has been released, and whatever the freed table let the room fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CancelledBooking {
    pub record: BookingRecord,
    /// Bookings the freed table allowed to be seated.
    pub reconciliation: Reseated,
    pub config: ValidConfig,
    /// Whether a notice went into the outbox for the guest.
    ///
    /// Decided here rather than by the caller: it depends on there being both a reason and an
    /// account to send it to, and both are facts this transaction holds.
    pub guest_notified: bool,
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

/// Arrival times, and the configuration they were computed from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AvailabilityReading {
    pub config: ValidConfig,
    pub slots: Vec<Slot>,
}

/// One shift as the admin screen needs it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Shift {
    pub bookings: Vec<BookingRecord>,
    pub blocks: Vec<BlockRecord>,
}

impl Store {
    /// Arrival times for a party on one shift, each with the reason it can or cannot be taken.
    ///
    /// No lock: availability is advice, true at the moment it was read. The guarantee that two
    /// guests cannot both act on it lives in [`Self::create_booking`] and, beneath that, in the
    /// exclusion constraint.
    pub async fn availability(
        &self,
        bar: BarId,
        day: ServiceDay,
        party_size: i32,
        now: DateTime<Utc>,
        ignoring: Option<BookingId>,
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

    /// Takes a booking, or explains why it cannot.
    ///
    /// The picker and this method ask the very same function which times are free, so a guest is
    /// never refused a slot the app had just shown as available — except by losing a race, which
    /// is reported as its own error so the app can say "somebody just took it" rather than
    /// something vague.
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

        // A guest holds one booking that has not yet started. Booking again replaces it, in the
        // same transaction, so there is no instant in which they hold two or none.
        let replaced = match &request.channel {
            Channel::Guest { user, .. } => {
                cancel_not_yet_started(&mut transaction, request.bar, *user, now).await?
            }
            Channel::Staff { .. } => None,
        };

        let bookings = load_window(&mut transaction, request.bar, request.service_day).await?;
        let blocks = load_blocks(&mut transaction, request.bar, request.service_day).await?;
        let Seat { table, window } = choose_seat(
            request,
            &config,
            &bookings_of(&bookings),
            &blocks_of(&blocks),
            now,
        )?;

        let (source, name, username, user) = match &request.channel {
            Channel::Guest {
                user,
                name,
                username,
            } => (
                BookingSource::App,
                name.clone(),
                username.clone(),
                Some(*user),
            ),
            Channel::Staff { guest_name } => {
                (BookingSource::Staff, guest_name.clone(), None, None)
            }
        };

        let id: Uuid = sqlx::query(
            "insert into booking (bar_id, table_id, service_date, starts_at, ends_at, party_size,
                                  guest_name, guest_username, telegram_user_id, source)
             values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) returning id",
        )
        .bind(request.bar)
        .bind(table.0)
        .bind(request.service_day.date())
        .bind(window.start())
        .bind(window.end())
        .bind(request.party_size)
        .bind(&name)
        .bind(&username)
        .bind(user.map(|user| user.0))
        .bind(source)
        .fetch_one(&mut *transaction)
        .await
        .map_err(Error::from_write)
        .map(|row| row.get("id"))?;

        if let (Some(wording), Some(user)) = (request.reminder, user) {
            notifications::enqueue_reminder(
                &mut transaction,
                request.bar,
                BookingId(id),
                user,
                &wording(&config, window, request.party_size),
                window.start() - chrono::Duration::hours(i64::from(config.remind_hours)),
                now,
            )
            .await?;
        }

        let record = fetch_booking(&mut transaction, request.bar, BookingId(id)).await?;
        transaction.commit().await?;
        Ok(CreatedBooking {
            record,
            replaced,
            config,
        })
    }

    /// The guest's booking, if they have one that has not finished.
    ///
    /// A guest who is already at their table still sees it, which is what the home screen shows
    /// them; only a finished or cancelled booking disappears.
    pub async fn booking_of_guest(
        &self,
        bar: BarId,
        user: TelegramUserId,
        now: DateTime<Utc>,
    ) -> Result<Option<BookingRecord>> {
        let row = sqlx::query(concat!(
            "select ",
            booking_columns!(),
            " where b.bar_id = $1 and b.telegram_user_id = $2
                and b.status <> 'cancelled' and b.ends_at > $3
              order by b.starts_at limit 1"
        ))
        .bind(bar)
        .bind(user.0)
        .bind(now)
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| BookingRecord::try_from(row_into(&row)?)).transpose()
    }

    /// Everything on one shift: the bookings and the tables that are shut.
    pub async fn shift(&self, bar: BarId, day: ServiceDay) -> Result<Shift> {
        let mut connection = self.pool().acquire().await?;
        Ok(Shift {
            bookings: load_shift(&mut connection, bar, day).await?,
            blocks: load_blocks_on(&mut connection, bar, day).await?,
        })
    }

    /// Records whether a party turned up.
    pub async fn set_attendance(
        &self,
        bar: BarId,
        booking: BookingId,
        attendance: Attendance,
    ) -> Result<BookingRecord> {
        let updated = sqlx::query(
            "update booking set status = $3
             where bar_id = $1 and id = $2 and status <> 'cancelled'",
        )
        .bind(bar)
        .bind(booking.0)
        .bind(StoredStatus::from(attendance))
        .execute(self.pool())
        .await?;
        if updated.rows_affected() == 0 {
            return Err(Error::NotFound { entity: "booking" });
        }
        let mut connection = self.pool().acquire().await?;
        fetch_booking(&mut connection, bar, booking).await
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
        // Checked here, against the configuration this transaction read, rather than in whichever
        // handler happens to call: a rule about what reaches a guest should not depend on every
        // future caller remembering it.
        if let Some(reason) = reason
            && !config.cancel_reasons.iter().any(|offered| offered == reason)
        {
            return Err(Error::UnknownCancelReason);
        }
        let updated = sqlx::query(
            "update booking set status = 'cancelled', cancelled_at = $4, cancel_reason = $3
             where bar_id = $1 and id = $2 and status <> 'cancelled'",
        )
        .bind(bar)
        .bind(booking.0)
        .bind(reason)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(Error::NotFound { entity: "booking" });
        }
        notifications::abandon_reminder(&mut transaction, booking).await?;
        let record = fetch_booking(&mut transaction, bar, booking).await?;

        // Queued in the same transaction as the cancellation: a notice that survives a crash the
        // cancellation did not would tell a guest their table is gone when it is not.
        let mut guest_notified = false;
        if let (Some(recipient), Some(reason), Some(wording)) =
            (record.telegram_user_id, reason, notice)
        {
            notifications::enqueue(
                &mut transaction,
                bar,
                booking,
                recipient,
                notifications::NotificationKind::Cancelled,
                &wording(&config, &record, reason),
                now,
            )
            .await?;
            guest_notified = true;
        }

        let reconciliation = reconcile_shift(
            &mut transaction,
            bar,
            &config,
            record.booking.service_day,
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(CancelledBooking {
            record,
            reconciliation,
            config,
            guest_notified,
        })
    }

    /// A guest gives back whichever booking of theirs has not finished.
    ///
    /// Finding it and releasing it in one transaction rather than two: between a separate lookup and
    /// a cancel, staff could have cancelled the same booking, and the guest would be told their
    /// cancellation failed when in truth the table is already free.
    pub async fn cancel_booking_of_guest(
        &self,
        bar: BarId,
        user: TelegramUserId,
        now: DateTime<Utc>,
    ) -> Result<CancelledBooking> {
        let mine = self
            .booking_of_guest(bar, user, now)
            .await?
            .ok_or(Error::NotFound { entity: "booking" })?;
        self.cancel_booking(bar, mine.booking.id, None, None, now)
            .await
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
    ) -> Result<Reseated> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;
        let config = load_config(&mut transaction, bar).await?;
        let outcome = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        transaction.commit().await?;
        Ok(outcome)
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
    ) -> Result<Reseated> {
        self.change_blocks(bar, day, tables, &[], Some((reason, by)), now)
            .await
    }

    /// Puts tables back into service. Reconciliation runs afterwards because a freed table may be
    /// exactly what an orphaned booking has been waiting for.
    pub async fn unblock_tables(
        &self,
        bar: BarId,
        day: ServiceDay,
        tables: &[TableId],
        now: DateTime<Utc>,
    ) -> Result<Reseated> {
        self.change_blocks(bar, day, &[], tables, None, now).await
    }

    /// Closing and opening tables are the same operation with the arrow reversed: change what is
    /// in service, then let reconciliation put every booking where it now belongs.
    async fn change_blocks(
        &self,
        bar: BarId,
        day: ServiceDay,
        add: &[TableId],
        remove: &[TableId],
        reason: Option<(&str, Option<TelegramUserId>)>,
        now: DateTime<Utc>,
    ) -> Result<Reseated> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;

        if !remove.is_empty() {
            sqlx::query(
                "delete from table_block
                 where bar_id = $1 and service_date = $2 and table_id = any($3::uuid[])",
            )
            .bind(bar)
            .bind(day.date())
            .bind(remove.iter().map(|table| table.0).collect::<Vec<_>>())
            .execute(&mut *transaction)
            .await?;
        }
        if !add.is_empty() {
            let (text, by) = reason.ok_or(Error::MissingBlockReason)?;
            sqlx::query(
                "insert into table_block (bar_id, table_id, service_date, reason, created_by)
                 select $1, table_id, $2, $3, $4 from unnest($5::uuid[]) as table_id
                 on conflict (table_id, service_date) do nothing",
            )
            .bind(bar)
            .bind(day.date())
            .bind(text)
            .bind(by.map(|user| user.0))
            .bind(add.iter().map(|table| table.0).collect::<Vec<_>>())
            .execute(&mut *transaction)
            .await?;
        }

        let config = load_config(&mut transaction, bar).await?;
        let outcome = reconcile_shift(&mut transaction, bar, &config, day, now).await?;
        transaction.commit().await?;
        Ok(outcome)
    }
}

/// The table and window a request resolves to.
struct Seat {
    table: TableId,
    window: pustol_domain::Interval,
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
    if bookable_days(config, config.current_service_day(now)).contains(&request.service_day) {
        Ok(())
    } else {
        Err(Error::ShiftNotBookable {
            service_day: request.service_day,
        })
    }
}

/// Asks the picker's own function which table this arrival time would use.
///
/// Going through [`slots::slot_list`] rather than calling the allocator directly is what
/// guarantees the guest is never refused a time the app had just offered: the two cannot disagree,
/// because they are the same computation. It also settles for free every reason a time might be
/// unavailable — outside opening hours, off the time step, already gone, or a wall-clock time the
/// clock change skipped.
fn choose_seat(
    request: &NewBooking,
    config: &ValidConfig,
    bookings: &[pustol_domain::Booking],
    blocks: &[pustol_domain::TableBlock],
    now: DateTime<Utc>,
) -> Result<Seat> {
    let unavailable = || Error::NotAnArrivalTime {
        minutes: request.start_minutes,
    };
    let offered = slots::slot_list(&slots::Query {
        config,
        service_day: request.service_day,
        party_size: request.party_size,
        bookings,
        blocks,
        now,
        ignoring: None,
    });
    let slot = offered
        .iter()
        .find(|slot| slot.start_minutes == request.start_minutes)
        .ok_or_else(unavailable)?;

    let table = match slot.availability {
        SlotAvailability::Free { table } => table,
        SlotAvailability::Taken => {
            return Err(Error::NoTableFree {
                party_size: request.party_size,
            });
        }
        SlotAvailability::Past => return Err(Error::InThePast),
        SlotAvailability::Nonexistent => return Err(unavailable()),
    };
    Ok(Seat {
        table,
        window: slot.window.ok_or_else(unavailable)?,
    })
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

/// Cancels the guest's booking that has not started yet, if there is one.
///
/// Only `confirmed` and only before it starts: a guest already at their table has a seating in
/// progress, and silently cancelling it because they tapped Book again would take the table out
/// from under them.
async fn cancel_not_yet_started(
    connection: &mut PgConnection,
    bar: BarId,
    user: TelegramUserId,
    now: DateTime<Utc>,
) -> Result<Option<BookingId>> {
    let row = sqlx::query(
        "update booking set status = 'cancelled', cancelled_at = $3
         where bar_id = $1 and telegram_user_id = $2 and status = 'confirmed' and starts_at > $3
         returning id",
    )
    .bind(bar)
    .bind(user.0)
    .bind(now)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let id = BookingId(row.get("id"));
    notifications::abandon_reminder(&mut *connection, id).await?;
    Ok(Some(id))
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
        party_size: row.try_get("party_size")?,
        guest_name: row.try_get("guest_name")?,
        guest_username: row.try_get("guest_username")?,
        telegram_user_id: row.try_get("telegram_user_id")?,
        status: row.try_get("status")?,
        source: row.try_get("source")?,
        cancel_reason: row.try_get("cancel_reason")?,
    })
}
