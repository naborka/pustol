//! Shift screen evening, read as one snapshot of the room.

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

/// Staff day sheet reach: widest guest horizon bar may ever set, so staff always see at least as
/// far as guests, and a month out as docs promise.
pub const STAFF_REACH_DAYS: i32 = LIMITS.horizon_days.max;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Evening {
    /// Moment read for; judges "now", "started" and "over".
    pub now: DateTime<Utc>,
    pub day: ServiceDay,
    /// Shift running at `now`, by `config`.
    pub today: ServiceDay,
    pub config: ValidConfig,
    /// Cancelled ones left out.
    pub bookings: Vec<BookingRecord>,
    /// Tables shut on this evening only.
    pub blocks: Vec<BlockRecord>,
    /// Staff day sheet counts, from `today` onwards.
    pub days: Vec<DayCount>,
    /// Room version. Any later change to booking, closure, table or bar reads higher.
    pub version: i64,
}

/// Staff day sheet row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DayCount {
    pub day: ServiceDay,
    pub bookings: usize,
}

/// Cancelled bookings left out; order as [`load_shift`] and [`load_blocks_on`] give.
pub(crate) struct ShiftRows {
    pub(crate) day: ServiceDay,
    pub(crate) bookings: Vec<BookingRecord>,
    pub(crate) blocks: Vec<BlockRecord>,
}

impl Store {
    /// Read on one snapshot: version read after rest would call older room newer, and screen
    /// keeping newest evening would keep that one.
    ///
    /// # Errors
    ///
    /// `NotFound` for unknown bar, stored config refusals, database errors.
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

/// Every room write calls this, or [`evening_around`], before commit under bar lock: answer is room
/// exactly as write left it, and read failure rolls write back.
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
