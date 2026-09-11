//! The shapes on the wire.
//!
//! Written out by hand rather than derived from the domain types. Two reasons, and both matter.
//! A guest is never told which table they were given — the bar assigns tables and moves them, and a
//! number on a guest's screen becomes a number they argue about — so the guest projection cannot be
//! the same type the allocator uses. And the admin projection carries staff usernames, which must
//! be structurally incapable of reaching a guest's response.

use chrono::NaiveDate;
use pustol_db::bookings::Attendance;
use pustol_db::identity::{ReminderStanding, Viewer};
use pustol_db::bookings::Reseated;
use pustol_db::records::{BookingRecord, BookingSource};
use pustol_domain::config::{DayHours, LIMITS, ValidConfig};
use pustol_domain::slots::{PartOfDay, Slot, SlotAvailability};
use pustol_domain::{BookingStatus, ServiceDay, minutes_within};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A booking as its own guest sees it: when, how many, and nothing about the furniture.
#[derive(Debug, Serialize)]
pub struct GuestBooking {
    pub id: Uuid,
    pub service_date: NaiveDate,
    pub start_minutes: i32,
    pub end_minutes: i32,
    pub party_size: i32,
    pub status: Status,
}

impl GuestBooking {
    pub fn of(record: &BookingRecord, config: &ValidConfig) -> Self {
        let day = record.booking.service_day;
        Self {
            id: record.booking.id.0,
            service_date: day.date(),
            start_minutes: minutes_within(day, record.booking.window.start(), config.timezone),
            end_minutes: minutes_within(day, record.booking.window.end(), config.timezone),
            party_size: record.booking.party_size,
            status: record.booking.status.into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Confirmed,
    Arrived,
    NoShow,
    Left,
    Cancelled,
}

impl From<BookingStatus> for Status {
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

/// One weekday's hours.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Hours {
    pub open_minutes: i32,
    pub close_minutes: i32,
    pub closed: bool,
}

impl From<DayHours> for Hours {
    fn from(hours: DayHours) -> Self {
        Self {
            open_minutes: hours.open_minutes,
            close_minutes: hours.close_minutes,
            closed: hours.closed,
        }
    }
}

/// What a guest is told about the bar.
#[derive(Debug, Serialize)]
pub struct BarView {
    pub name: String,
    pub address: String,
    pub timezone: String,
    pub max_party: i32,
    pub grace_minutes: i32,
    pub remind_hours: i32,
    pub turn_minutes: i32,
    pub slot_step_minutes: i32,
    /// The shift that is running, or about to.
    pub today: NaiveDate,
    pub today_hours: Hours,
    /// The last wall-clock minute a party may arrive today, absent on a day off.
    pub last_arrival_minutes: Option<i32>,
    /// The bar's own clock, in wall-clock minutes into today's shift.
    ///
    /// Sent rather than read off the device, because the phone in the guest's hand may be in a
    /// different timezone from the bar and is under nobody's control. "Открыт до 02:00" is a claim
    /// about the bar, so it is answered by the bar.
    pub now_minutes: i32,
}

impl BarView {
    pub fn of(config: &ValidConfig, today: ServiceDay, now: chrono::DateTime<chrono::Utc>) -> Self {
        let hours = config.week.for_service_day(today);
        Self {
            name: config.name.clone(),
            address: config.address.clone(),
            timezone: config.timezone.name().to_owned(),
            max_party: config.max_party,
            grace_minutes: config.grace_minutes,
            remind_hours: config.remind_hours,
            turn_minutes: config.turn_minutes,
            slot_step_minutes: config.slot_step_minutes,
            today: today.date(),
            today_hours: hours.into(),
            last_arrival_minutes: config.last_arrival_minutes(today.weekday()),
            now_minutes: minutes_within(today, now, config.timezone),
        }
    }
}

/// Everything the app needs to draw its first screen, in one request.
#[derive(Debug, Serialize)]
pub struct Session {
    pub user: UserView,
    pub is_staff: bool,
    pub reminders: RemindersView,
    pub bar: BarView,
    pub booking: Option<GuestBooking>,
    pub bookable_days: Vec<NaiveDate>,
    /// The earliest arrival time tonight still has, absent when it has none.
    ///
    /// The home screen's one honest sentence about this evening — "Сегодня свободно с 21:30" —
    /// answered here so the first screen still costs one request.
    pub today_free_from_minutes: Option<i32>,
    /// The party size that sentence speaks for, and the size the picker opens on.
    ///
    /// Sent rather than agreed by comment. A promise has to be about a definite party, and if the
    /// two ends picked their own number the card would promise a time the very next screen did not
    /// keep.
    pub today_free_for_party: i32,
}

#[derive(Debug, Serialize)]
pub struct UserView {
    pub id: i64,
    pub first_name: String,
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RemindersView {
    pub opted_in: bool,
    /// Whether the bot has been found unable to reach this account.
    pub deliverable: bool,
    /// Whether the app should offer to set reminders up.
    pub should_ask: bool,
}

impl RemindersView {
    pub fn of(viewer: &Viewer) -> Self {
        Self::of_standing(viewer.reminders)
    }

    pub fn of_standing(standing: ReminderStanding) -> Self {
        Self {
            opted_in: standing.opted_in,
            deliverable: standing.deliverable,
            should_ask: standing.should_ask(),
        }
    }
}

/// Why a slot can or cannot be taken.
///
/// A wall-clock time the clocks jumped over is absent from the list altogether rather than shown as
/// unavailable: a greyed-out 02:30 that never existed is not information, it is confusion.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlotState {
    Free,
    Taken,
    Past,
}

#[derive(Debug, Serialize)]
pub struct SlotView {
    pub start_minutes: i32,
    pub state: SlotState,
    pub evening: bool,
}

#[derive(Debug, Serialize)]
pub struct Availability {
    pub service_date: NaiveDate,
    pub party_size: i32,
    pub turn_minutes: i32,
    pub slots: Vec<SlotView>,
    /// How many of them can actually be taken — the picker's "free windows" line.
    pub free_count: usize,
}

/// One chip on the guest's day rail.
///
/// A day says what it holds before it is tapped, because a day that turns out to be empty *after*
/// a tap has cost the guest a screen to find out. The rail is every day of the booking horizon,
/// shut ones included: a rail that silently dropped them would be a different length every week.
#[derive(Debug, Serialize)]
pub struct DayOffer {
    pub service_date: NaiveDate,
    pub closed: bool,
    /// The earliest arrival time still free for this party, absent when the day holds none.
    pub free_from_minutes: Option<i32>,
}

/// The whole rail, for one party size.
#[derive(Debug, Serialize)]
pub struct DayRail {
    pub party_size: i32,
    pub days: Vec<DayOffer>,
}

impl Availability {
    pub fn of(
        day: ServiceDay,
        party_size: i32,
        config: &ValidConfig,
        slots: &[Slot],
    ) -> Self {
        let offered: Vec<SlotView> = slots
            .iter()
            .filter(|slot| slot.availability.is_offerable())
            .map(|slot| SlotView {
                start_minutes: slot.start_minutes,
                state: match slot.availability {
                    SlotAvailability::Free { .. } => SlotState::Free,
                    // A time the clock change jumped over is filtered out above and never reaches a
                    // client. It is matched rather than left to a catch-all so that adding a
                    // variant to the domain is a compile error here rather than a silent default.
                    SlotAvailability::Taken | SlotAvailability::Nonexistent => SlotState::Taken,
                    SlotAvailability::Past => SlotState::Past,
                },
                evening: slot.part_of_day == PartOfDay::Evening,
            })
            .collect();
        let free_count = offered
            .iter()
            .filter(|slot| slot.state == SlotState::Free)
            .count();
        Self {
            service_date: day.date(),
            party_size,
            turn_minutes: config.turn_minutes,
            slots: offered,
            free_count,
        }
    }
}

/// A booking as staff see it: everything, including the table and who the guest is.
#[derive(Debug, Serialize)]
pub struct ShiftBooking {
    pub id: Uuid,
    pub table_id: Option<Uuid>,
    pub table_number: Option<i32>,
    pub table_zone: Option<String>,
    pub start_minutes: i32,
    /// The end of the window promised to the guest. What "Когда 21:00 — 23:00" reads from, and
    /// never shortened by what happened on the night.
    pub end_minutes: i32,
    /// The minute the table went back into the pool, absent while the booking still holds it.
    ///
    /// The screen's half of the one occupancy rule: the block on the timeline stops here, the
    /// status line says "Ушли в 21:20", and the shift's occupancy figure counts up to here. A
    /// screen that drew the promised window instead would show a table as busy that the server
    /// has already sold to somebody else.
    pub released_minutes: Option<i32>,
    pub party_size: i32,
    pub guest_name: String,
    pub guest_username: Option<String>,
    pub status: Status,
    pub source: Source,
    /// What staff wrote on this booking. Staff-facing only: nothing sends it anywhere.
    pub note: Option<String>,
    /// Whether the bot could ever message this guest. False for a booking taken at the door, which
    /// has no Telegram account behind it at all.
    pub reachable_by_bot: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    App,
    Staff,
    Walk,
}

impl From<BookingSource> for Source {
    fn from(source: BookingSource) -> Self {
        match source {
            BookingSource::App => Self::App,
            BookingSource::Staff => Self::Staff,
            BookingSource::Walk => Self::Walk,
        }
    }
}

impl ShiftBooking {
    pub fn of(record: &BookingRecord, config: &ValidConfig) -> Self {
        let day = record.booking.service_day;
        Self {
            id: record.booking.id.0,
            table_id: record.booking.table_id.map(|table| table.0),
            table_number: record.table_number,
            table_zone: record.table_zone.clone(),
            start_minutes: minutes_within(day, record.booking.window.start(), config.timezone),
            end_minutes: minutes_within(day, record.booking.window.end(), config.timezone),
            released_minutes: record
                .booking
                .released_at
                .map(|released| minutes_within(day, released, config.timezone)),
            party_size: record.booking.party_size,
            guest_name: record.guest_name.clone(),
            guest_username: record.guest_username.clone(),
            status: record.booking.status.into(),
            source: record.source.into(),
            note: record.note.clone(),
            reachable_by_bot: record.has_telegram_account(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ShiftTable {
    pub id: Uuid,
    pub number: i32,
    pub seats: i32,
    pub zone: String,
    /// The reason it is shut tonight, absent when it is in service.
    pub blocked_because: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ShiftStats {
    pub bookings: usize,
    pub guests: i32,
    /// Tables free at this moment, absent for a shift that is not the one running: "free now" has
    /// no meaning on next Tuesday, and an invented number is worse than a blank.
    pub free_now: Option<usize>,
}

/// One row of the staff day sheet.
#[derive(Debug, Serialize)]
pub struct ShiftDay {
    pub service_date: NaiveDate,
    pub closed: bool,
    pub bookings: usize,
}

#[derive(Debug, Serialize)]
pub struct ShiftView {
    pub service_date: NaiveDate,
    pub hours: Hours,
    pub tables: Vec<ShiftTable>,
    pub bookings: Vec<ShiftBooking>,
    pub stats: ShiftStats,
    /// Where to draw the "now" line, absent for a shift that is not running.
    pub now_minutes: Option<i32>,
    /// The largest party the room could seat this minute, absent when none fits — and absent on
    /// any shift but the one running, where "now" means nothing.
    ///
    /// Answered here, by the allocator, rather than inferred from a count of free tables: seven
    /// free two-tops do not seat the four people at the door, and a bartender who is sent to
    /// another view to find that out has been failed by the one he was on.
    pub largest_party_seatable_now: Option<i32>,
    /// Every day staff can reach from here, with what is on. Longer than the guest's horizon on
    /// purpose: a telephone booking for next month is not a thing to argue about.
    pub days: Vec<ShiftDay>,
    /// How far ahead guests may book, so the day sheet can say where their horizon ends.
    pub guest_horizon_days: i32,
    pub cancel_reasons: Vec<String>,
    pub message_templates: Vec<String>,
}

/// What a change to the room did to the bookings on it.
#[derive(Debug, Serialize)]
pub struct ReconciliationView {
    pub moved: Vec<MovedBooking>,
    pub orphaned: Vec<StrandedBooking>,
}

#[derive(Debug, Serialize)]
pub struct MovedBooking {
    pub booking_id: Uuid,
    pub guest_name: String,
    pub to_number: i32,
}

#[derive(Debug, Serialize)]
pub struct StrandedBooking {
    pub booking_id: Uuid,
    pub guest_name: String,
}

impl ReconciliationView {
    /// Attaches guest names, which the report needs and the domain deliberately does not carry.
    ///
    /// The records come out of the transaction that reconciled, so a name here always describes the
    /// state that was reconciled rather than whatever a later read would have found.
    pub fn of(reseated: &Reseated) -> Self {
        let outcome = &reseated.outcome;
        let name_of = |id: pustol_domain::BookingId| {
            reseated
                .affected
                .iter()
                .find(|record| record.booking.id == id)
                .map_or_else(String::new, |record| record.guest_name.clone())
        };
        Self {
            moved: outcome
                .moved
                .iter()
                .map(|moved| MovedBooking {
                    booking_id: moved.booking.0,
                    guest_name: name_of(moved.booking),
                    to_number: moved.to_number,
                })
                .collect(),
            orphaned: outcome
                .orphaned
                .iter()
                .map(|orphan| StrandedBooking {
                    booking_id: orphan.0,
                    guest_name: name_of(*orphan),
                })
                .collect(),
        }
    }
}

// ---- requests ---------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BookingRequest {
    pub service_date: NaiveDate,
    pub start_minutes: i32,
    pub party_size: i32,
}

#[derive(Debug, Deserialize)]
pub struct StaffBookingRequest {
    pub service_date: NaiveDate,
    pub start_minutes: i32,
    pub party_size: i32,
    pub guest_name: String,
}

#[derive(Debug, Deserialize)]
pub struct AvailabilityQuery {
    pub service_date: NaiveDate,
    pub party_size: i32,
}

#[derive(Debug, Deserialize)]
pub struct ShiftQuery {
    pub service_date: NaiveDate,
}

#[derive(Debug, Deserialize)]
pub struct AttendanceRequest {
    pub attendance: Attendance,
}

#[derive(Debug, Deserialize)]
pub struct NoteRequest {
    /// What staff want to remember, or `null` to rub it out.
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WalkInRequest {
    pub service_date: NaiveDate,
    pub party_size: i32,
}

#[derive(Debug, Deserialize)]
pub struct DayRailQuery {
    pub party_size: i32,
}

#[derive(Debug, Deserialize)]
pub struct CancelRequest {
    /// One of the bar's configured reasons. Absent when a guest cancels their own booking: they
    /// know why, and nothing is sent to them.
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    /// Must be one of the bar's configured messages. Free text would turn a borrowed staff account
    /// into a way to send anything to every guest who ever booked.
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct BlockRequest {
    pub service_date: NaiveDate,
    pub table_ids: Vec<Uuid>,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct UnblockRequest {
    pub service_date: NaiveDate,
    pub table_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ReconcileRequest {
    pub service_date: NaiveDate,
}

// ---- settings ---------------------------------------------------------------------------------

/// The settings screen's current state, and the bounds every control has to respect.
///
/// The bounds travel with the settings so the screen can grey out a stepper without a round trip,
/// and so widening a limit needs no change here.
#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub name: String,
    pub address: String,
    pub timezone: String,
    /// Indexed from Sunday, matching `chrono`'s numbering.
    pub week: Vec<Hours>,
    pub zones: Vec<String>,
    pub tables: Vec<SettingsTable>,
    pub turn_minutes: i32,
    pub slot_step_minutes: i32,
    pub max_party: i32,
    pub horizon_days: i32,
    pub remind_hours: i32,
    pub grace_minutes: i32,
    pub message_templates: Vec<String>,
    pub cancel_reasons: Vec<String>,
    pub staff: Vec<StaffView>,
    pub next_table_number: i32,
    pub limits: LimitsView,
}

#[derive(Debug, Serialize)]
pub struct SettingsTable {
    pub id: Uuid,
    pub number: i32,
    pub seats: i32,
    pub zone: String,
    /// How many bookings sit at this table on the shift being viewed — the badge that stops staff
    /// deleting a table somebody is about to sit at.
    pub bookings_today: usize,
}

#[derive(Debug, Serialize)]
pub struct StaffView {
    pub username: String,
    /// Whether this invitation has been claimed by an account yet.
    pub bound: bool,
}

#[derive(Debug, Serialize)]
pub struct BoundsView {
    pub min: i32,
    pub max: i32,
}

#[derive(Debug, Serialize)]
pub struct LimitsView {
    pub open_minutes: BoundsView,
    pub close_minutes: BoundsView,
    pub turn_minutes: BoundsView,
    pub max_party: BoundsView,
    pub horizon_days: BoundsView,
    pub remind_hours: BoundsView,
    pub grace_minutes: BoundsView,
    pub seats: BoundsView,
    pub slot_step_minutes: Vec<i32>,
}

impl LimitsView {
    pub fn current() -> Self {
        let bounds = |bounds: pustol_domain::Bounds| BoundsView {
            min: bounds.min,
            max: bounds.max,
        };
        Self {
            open_minutes: bounds(LIMITS.open_minutes),
            close_minutes: bounds(LIMITS.close_minutes),
            turn_minutes: bounds(LIMITS.turn_minutes),
            max_party: bounds(LIMITS.max_party),
            horizon_days: bounds(LIMITS.horizon_days),
            remind_hours: bounds(LIMITS.remind_hours),
            grace_minutes: bounds(LIMITS.grace_minutes),
            seats: bounds(LIMITS.seats),
            slot_step_minutes: LIMITS.slot_step_minutes.to_vec(),
        }
    }
}

/// What a settings save did.
#[derive(Debug, Serialize)]
pub struct SavedSettingsView {
    pub settings: SettingsView,
    pub reconciliation: ReconciliationView,
    /// Live bookings for parties above the new cap. They keep their tables; staff are told so that a
    /// cap lowered by accident is visible at once.
    pub above_cap: usize,
}
