//! Stored rows, and the mapping between them and the domain's value types.
//!
//! The domain's [`Booking`] carries only what allocation is allowed to look at. A record adds the
//! facts the screens need — who the guest is, how the booking arrived, which table number to
//! print — by composition, so no amount of feature growth up here can smuggle a guest's name into
//! a decision about who gets a table.

use chrono::{DateTime, NaiveDate, Utc};
use pustol_domain::allocator::{Booking, BookingId, BookingStatus, TableBlock};
use pustol_domain::schedule::TableId;
use pustol_domain::service_day::{Interval, ServiceDay};

use crate::error::{Error, Result};
use crate::ids::TelegramUserId;

/// How a booking reached the bar.
///
/// Not merely descriptive: a booking taken by staff has no Telegram account behind it, which is
/// what makes "the bot has no chat with this guest" a fact about the data rather than a flag
/// somebody has to keep in step.
#[derive(Clone, Copy, PartialEq, Eq, Debug, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(type_name = "booking_source", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BookingSource {
    /// The guest booked it themselves in the Mini App.
    App,
    /// Staff entered it: by phone, or at the door.
    Staff,
    /// Nobody booked it. Staff sat a party that walked in, at the minute they sat down.
    Walk,
}

/// Storage's mirror of [`BookingStatus`].
///
/// A separate enum on purpose. The conversions below are exhaustive, so adding a status to the
/// domain fails to compile here until the storage side has been considered — which is the point
/// of having a boundary at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, sqlx::Type)]
#[sqlx(type_name = "booking_status", rename_all = "snake_case")]
pub enum StoredStatus {
    Confirmed,
    Arrived,
    NoShow,
    Left,
    Cancelled,
}

impl From<BookingStatus> for StoredStatus {
    fn from(status: BookingStatus) -> Self {
        match status {
            BookingStatus::Confirmed => Self::Confirmed,
            BookingStatus::Arrived => Self::Arrived,
            BookingStatus::NoShow => Self::NoShow,
            BookingStatus::Left => Self::Left,
            BookingStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl From<StoredStatus> for BookingStatus {
    fn from(status: StoredStatus) -> Self {
        match status {
            StoredStatus::Confirmed => Self::Confirmed,
            StoredStatus::Arrived => Self::Arrived,
            StoredStatus::NoShow => Self::NoShow,
            StoredStatus::Left => Self::Left,
            StoredStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// A booking as stored, straight off the wire from `PostgreSQL`.
#[derive(Debug)]
pub(crate) struct BookingRow {
    pub id: uuid::Uuid,
    pub table_id: Option<uuid::Uuid>,
    pub table_number: Option<i32>,
    pub table_zone: Option<String>,
    pub service_date: NaiveDate,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
    pub party_size: i32,
    pub guest_name: String,
    pub guest_username: Option<String>,
    pub telegram_user_id: Option<i64>,
    pub status: StoredStatus,
    pub source: BookingSource,
    pub note: Option<String>,
    pub cancel_reason: Option<String>,
}

/// A booking with everything the screens need to draw it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BookingRecord {
    /// The allocation-relevant core, shared with the domain.
    pub booking: Booking,
    /// The printed number of the assigned table, absent for an orphan.
    pub table_number: Option<i32>,
    pub table_zone: Option<String>,
    pub guest_name: String,
    pub guest_username: Option<String>,
    pub telegram_user_id: Option<TelegramUserId>,
    pub source: BookingSource,
    /// What staff wrote on this booking: "День рождения", "У окна". Never sent to the guest.
    pub note: Option<String>,
    pub cancel_reason: Option<String>,
}

impl BookingRecord {
    /// Whether the bot could conceivably message this guest: staff-entered bookings have no
    /// account behind them at all.
    #[must_use]
    pub fn has_telegram_account(&self) -> bool {
        self.telegram_user_id.is_some()
    }
}

impl TryFrom<BookingRow> for BookingRecord {
    type Error = Error;

    fn try_from(row: BookingRow) -> Result<Self> {
        Ok(Self {
            booking: Booking {
                id: BookingId(row.id),
                table_id: row.table_id.map(TableId),
                service_day: ServiceDay::new(row.service_date),
                window: Interval::new(row.starts_at, row.ends_at)?,
                released_at: row.left_at,
                party_size: row.party_size,
                status: row.status.into(),
            },
            table_number: row.table_number,
            table_zone: row.table_zone,
            guest_name: row.guest_name,
            guest_username: row.guest_username,
            telegram_user_id: row.telegram_user_id.map(TelegramUserId),
            source: row.source,
            note: row.note,
            cancel_reason: row.cancel_reason,
        })
    }
}

/// A table taken out of service, with the reason staff gave.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlockRecord {
    pub block: TableBlock,
    pub table_number: i32,
    pub reason: String,
}

#[derive(Debug)]
pub(crate) struct BlockRow {
    pub table_id: uuid::Uuid,
    pub table_number: i32,
    pub service_date: NaiveDate,
    pub reason: String,
}

impl From<BlockRow> for BlockRecord {
    fn from(row: BlockRow) -> Self {
        Self {
            block: TableBlock {
                table_id: TableId(row.table_id),
                service_day: ServiceDay::new(row.service_date),
            },
            table_number: row.table_number,
            reason: row.reason,
        }
    }
}

/// The domain views of a set of records, for handing to the allocator.
///
/// Public because the API asks the allocator its own questions — "who fits right now" is one — and
/// a second hand-rolled projection up there would be a second chance to leave something out.
#[must_use]
pub fn bookings_of(records: &[BookingRecord]) -> Vec<Booking> {
    records.iter().map(|record| record.booking.clone()).collect()
}

#[must_use]
pub fn blocks_of(records: &[BlockRecord]) -> Vec<TableBlock> {
    records.iter().map(|record| record.block.clone()).collect()
}
