//! One evening as the shift screen draws it, read as one moment of the room.

use chrono::{DateTime, NaiveDate, Utc};
use pustol_domain::config::ValidConfig;
use pustol_domain::service_day::ServiceDay;
use pustol_domain::{LIMITS, days_from};
use sqlx::{PgConnection, Row};

use crate::Store;
use crate::bar::load_config;
use crate::bookings::{load_blocks_on, load_shift};
use crate::error::Result;
use crate::ids::BarId;
use crate::records::{BlockRecord, BookingRecord};

/// How far ahead the staff day sheet reaches.
///
/// The widest booking horizon the bar could ever set for guests, so staff can always see at least
/// as far as the guests they are answering the phone for — and, as the docs promise, a month out.
pub const STAFF_REACH_DAYS: i32 = LIMITS.horizon_days.max;

/// One evening, and everything the shift screen draws around it, as one moment of the room.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Evening {
    /// The moment it was read for, which "now", "started" and "over" are judged by.
    pub now: DateTime<Utc>,
    pub day: ServiceDay,
    /// The shift running at `now`, by `config`.
    pub today: ServiceDay,
    /// The configuration in force when it was read.
    pub config: ValidConfig,
    /// The evening's bookings, cancelled ones left out.
    pub bookings: Vec<BookingRecord>,
    /// The tables shut on this evening and no other.
    pub blocks: Vec<BlockRecord>,
    /// How many bookings each day of the staff day sheet holds, from `today` onwards.
    pub days: Vec<DayCount>,
    /// How far the bar's room had moved on when this was read. Any later change to a booking, a
    /// closure, a table or the bar reads higher.
    pub version: i64,
}

/// One row of the staff day sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DayCount {
    pub day: ServiceDay,
    pub bookings: usize,
}

/// One evening's bookings, cancelled ones left out, and its closures, as [`load_shift`] and
/// [`load_blocks_on`] order them.
pub(crate) struct ShiftRows {
    pub(crate) day: ServiceDay,
    pub(crate) bookings: Vec<BookingRecord>,
    pub(crate) blocks: Vec<BlockRecord>,
}

impl Store {
    /// One evening as it stands at `now`.
    ///
    /// Read on one snapshot, so the bookings, the closures, the day sheet and the version describe
    /// the same moment. A version read a moment after the rest would call an older room newer than
    /// it is, and a screen keeping the newest evening would keep that one.
    pub async fn evening(
        &self,
        bar: BarId,
        day: ServiceDay,
        now: DateTime<Utc>,
    ) -> Result<Evening> {
        let mut snapshot = self.snapshot().await?;
        let config = load_config(&mut snapshot, bar).await?;
        let evening = read_evening(&mut snapshot, bar, config, day, now).await?;
        snapshot.commit().await?;
        Ok(evening)
    }
}

/// Reads one evening on a transaction that has already read the configuration in force.
///
/// Every write that changes the room calls this, or [`evening_around`] with rows it already holds,
/// before it commits, under the bar's lock: nothing else changes the room in between, so the answer
/// is the room exactly as that write left it, and a failure to read it takes the write back with it.
pub(crate) async fn read_evening(
    connection: &mut PgConnection,
    bar: BarId,
    config: ValidConfig,
    day: ServiceDay,
    now: DateTime<Utc>,
) -> Result<Evening> {
    let rows = read_shift(&mut *connection, bar, day).await?;
    evening_around(connection, bar, config, rows, now).await
}

/// `day`'s bookings and closures.
pub(crate) async fn read_shift(
    connection: &mut PgConnection,
    bar: BarId,
    day: ServiceDay,
) -> Result<ShiftRows> {
    Ok(ShiftRows {
        day,
        bookings: load_shift(&mut *connection, bar, day).await?,
        blocks: load_blocks_on(&mut *connection, bar, day).await?,
    })
}

/// The evening `rows` are, with the day sheet and the version read around them.
pub(crate) async fn evening_around(
    connection: &mut PgConnection,
    bar: BarId,
    config: ValidConfig,
    rows: ShiftRows,
    now: DateTime<Utc>,
) -> Result<Evening> {
    let today = config.current_service_day(now);
    let (days, version) =
        day_counts_and_version(connection, bar, &days_from(today, STAFF_REACH_DAYS)).await?;
    Ok(Evening {
        now,
        day: rows.day,
        today,
        config,
        bookings: rows.bookings,
        blocks: rows.blocks,
        days,
        version,
    })
}

/// How many bookings sit on each of `days`, and the room's version, in one round trip.
async fn day_counts_and_version(
    connection: &mut PgConnection,
    bar: BarId,
    days: &[ServiceDay],
) -> Result<(Vec<DayCount>, i64)> {
    let rows = sqlx::query(
        "select v.version, counted.service_date, counted.total
         from room_version v
         left join (
             select service_date, count(*) as total from booking
             where bar_id = $1 and status <> 'cancelled' and service_date between $2 and $3
             group by service_date
         ) counted on true
         where v.bar_id = $1",
    )
    .bind(bar)
    .bind(days.first().map(|day| day.date()))
    .bind(days.last().map(|day| day.date()))
    .fetch_all(connection)
    .await?;

    let version = rows
        .first()
        .ok_or(sqlx::Error::RowNotFound)?
        .try_get("version")?;
    let mut counted = std::collections::HashMap::<NaiveDate, i64>::with_capacity(rows.len());
    for row in &rows {
        if let (Some(date), Some(total)) = (
            row.try_get::<Option<NaiveDate>, _>("service_date")?,
            row.try_get::<Option<i64>, _>("total")?,
        ) {
            counted.insert(date, total);
        }
    }
    let days = days
        .iter()
        .map(|day| DayCount {
            day: *day,
            bookings: usize::try_from(counted.get(&day.date()).copied().unwrap_or(0)).unwrap_or(0),
        })
        .collect();
    Ok((days, version))
}
