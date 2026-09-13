//! The shapes on the wire.
//!
//! Written out by hand rather than derived from the domain types. Two reasons, and both matter.
//! A guest is never told which table they were given — the bar assigns tables and moves them, and a
//! number on a guest's screen becomes a number they argue about — so the guest projection cannot be
//! the same type the allocator uses. And the admin projection carries staff usernames, which must
//! be structurally incapable of reaching a guest's response.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use pustol_db::bookings::Attendance;
use pustol_db::bookings::Reseated;
use pustol_db::evening::Evening;
use pustol_db::identity::{ReminderStanding, Viewer};
use pustol_db::records::{BookingRecord, BookingSource, blocks_of, bookings_of};
use pustol_domain::config::{Bounds, DayHours, LIMITS, ListLimits, TextLimits, ValidConfig};
use pustol_domain::slots::{PartOfDay, Slot, SlotAvailability};
use pustol_domain::{Booking, BookingStatus, Rebooking, ServiceDay, TableId, minutes_within};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// Raw request date, read only through [`Self::day`], so every path refuses bad dates same way;
/// `PostgreSQL` failed `-5000-01-01` as server fault.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct ServiceDate(String);

impl ServiceDate {
    /// # Errors
    ///
    /// `invalid_date` when text is not a date or fails [`in_calendar`].
    pub fn day(&self) -> ApiResult<ServiceDay> {
        self.0
            .parse::<NaiveDate>()
            .ok()
            .filter(|date| in_calendar(*date))
            .map(ServiceDay::new)
            .ok_or_else(|| {
                ApiError::bad_request(
                    "invalid_date",
                    format!("{:?} is not a date in the years 1 to 9999", self.0),
                )
            })
    }
}

/// Years 1 to 9999: dates people write, all storable.
pub fn in_calendar(date: NaiveDate) -> bool {
    (1..=9999).contains(&date.year())
}

/// A booking as its own guest sees it: when, how many, and nothing about the furniture.
#[derive(Debug, Serialize)]
pub struct GuestBooking {
    pub id: Uuid,
    pub service_date: NaiveDate,
    pub start_minutes: i32,
    pub end_minutes: i32,
    pub party_size: i32,
    pub status: Status,
    /// By bar clock, not phone clock.
    pub started: bool,
    /// Absent when no replacing booking possible now: no bookable evening with arrival time left
    /// that guest's other bookings do not hold. Plan moves to any such evening, no-show only to
    /// its own. App shows «Перенести» exactly when present, same rule as booking endpoint.
    pub rebooking_replaces: Option<RebookingView>,
    /// New booking on this evening refused because of it; absent `rebooking_replaces` never
    /// implies this.
    pub holds_evening: bool,
}

impl GuestBooking {
    /// `guest`: every booking that guest holds.
    pub fn of(
        record: &BookingRecord,
        guest: &[Booking],
        config: &ValidConfig,
        now: DateTime<Utc>,
    ) -> Self {
        let day = record.booking.service_day;
        Self {
            id: record.booking.id.0,
            service_date: day.date(),
            start_minutes: minutes_within(day, record.booking.window.start(), config.timezone),
            end_minutes: minutes_within(day, record.booking.window.end(), config.timezone),
            party_size: record.booking.party_size,
            status: record.booking.status.into(),
            started: record.booking.has_started(now),
            rebooking_replaces: record
                .booking
                .rebooking_on_offer(guest, config, now)
                .map(Into::into),
            holds_evening: record.booking.holds_evening(now),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RebookingView {
    AnyEvening,
    SameEvening,
}

impl From<Rebooking> for RebookingView {
    fn from(rebooking: Rebooking) -> Self {
        match rebooking {
            Rebooking::AnyEvening => Self::AnyEvening,
            Rebooking::SameEvening => Self::SameEvening,
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
    /// Same open and close walk-in endpoint uses. Decided on instants, not wall minutes: when
    /// clocks go back, wall hour repeats and wall comparison called bar shut while door seated.
    pub open_now: bool,
    /// Wall minutes into shift; absent once opened and on day off. Feeds «Откроется в …».
    pub opens_at_minutes: Option<i32>,
    pub contact: Option<ContactView>,
}

#[derive(Debug, Serialize)]
pub struct ContactView {
    pub label: String,
    pub url: String,
}

impl ContactView {
    pub fn of(config: &ValidConfig) -> Option<Self> {
        config.contact().map(|contact| Self {
            label: contact.label(),
            url: contact.url(),
        })
    }
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
            open_now: config.is_open(today, now),
            opens_at_minutes: config
                .opening_ahead(today, now)
                .map(|opens| minutes_within(today, opens, config.timezone)),
            contact: ContactView::of(config),
        }
    }
}

/// Everything the app needs to draw its first screen, in one request.
#[derive(Debug, Serialize)]
pub struct Session {
    /// Replaces Telegram payload in later requests; payload expires after one hour.
    pub session_token: String,
    pub user: UserView,
    pub is_staff: bool,
    pub reminders: RemindersView,
    pub bar: BarView,
    /// Bookings still holding a table, soonest first: current seat plus plan for another evening.
    pub bookings: Vec<GuestBooking>,
    pub bookable_days: Vec<NaiveDate>,
    /// The earliest arrival time tonight still has, absent when it has none.
    ///
    /// The home screen's one honest sentence about this evening — "Сегодня свободно с 21:30" —
    /// answered here so the first screen still costs one request. Guest's own bookings that
    /// tonight's booking would replace are set aside.
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
pub struct Availability<S = SlotView> {
    pub service_date: NaiveDate,
    pub party_size: i32,
    pub turn_minutes: i32,
    pub slots: Vec<S>,
    /// How many of them can actually be taken — the picker's "free windows" line.
    pub free_count: usize,
}

#[derive(Debug, Serialize)]
pub struct GuestAvailability {
    #[serde(flatten)]
    pub offer: Availability,
    /// Soonest first; picker button promises these, booking echoes them as `replacing`.
    pub replacing: Vec<Uuid>,
    /// Refused because guest's booking holds evening.
    pub booked: bool,
}

#[derive(Debug, Serialize)]
pub struct StaffSlotView {
    #[serde(flatten)]
    pub slot: SlotView,
    /// Every free table whatever party size, smallest first, so sheet can show too-small ones.
    pub free_table_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct StaffAvailability {
    #[serde(flatten)]
    pub offer: Availability<StaffSlotView>,
    /// Tables free for moved booking's stored window, even when that time is off grid.
    pub kept_free_table_ids: Option<Vec<Uuid>>,
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
    /// Guest holds evening with booking rebooking cannot replace; new booking refused.
    pub booked: bool,
}

/// The whole rail, for one party size.
#[derive(Debug, Serialize)]
pub struct DayRail {
    pub party_size: i32,
    pub days: Vec<DayOffer>,
}

impl<S> Availability<S> {
    pub fn drawn(
        day: ServiceDay,
        party_size: i32,
        config: &ValidConfig,
        slots: &[Slot],
        draw: impl Fn(&Slot, SlotView) -> S,
    ) -> Self {
        let offered = slots.iter().filter(|slot| slot.availability.is_offerable());
        let free_count = offered
            .clone()
            .filter(|slot| slot.availability.is_free())
            .count();
        Self {
            service_date: day.date(),
            party_size,
            turn_minutes: config.turn_minutes,
            slots: offered.map(|slot| draw(slot, SlotView::of(slot))).collect(),
            free_count,
        }
    }
}

impl Availability {
    pub fn of(day: ServiceDay, party_size: i32, config: &ValidConfig, slots: &[Slot]) -> Self {
        Self::drawn(day, party_size, config, slots, |_, view| view)
    }
}

impl SlotView {
    fn of(slot: &Slot) -> Self {
        Self {
            start_minutes: slot.start_minutes,
            state: match slot.availability {
                SlotAvailability::Free => SlotState::Free,
                // `Nonexistent` filtered out before drawing. Named, not catch-all: new variant
                // must fail compile.
                SlotAvailability::Taken | SlotAvailability::Nonexistent => SlotState::Taken,
                SlotAvailability::Past => SlotState::Past,
            },
            evening: slot.part_of_day == PartOfDay::Evening,
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
    /// False for door booking without Telegram account or guest bot cannot reach: staff must call.
    pub reachable_by_bot: bool,
    /// By server clock.
    pub started: bool,
    /// No longer holds table; same server rule that refuses move or cancel.
    pub finished: bool,
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
    pub fn of(record: &BookingRecord, config: &ValidConfig, now: DateTime<Utc>) -> Self {
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
            reachable_by_bot: record.reachable_by_bot,
            started: record.booking.has_started(now),
            finished: record.booking.has_finished(now),
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
    /// Absent unless running shift. Counted in instants: when clocks go back, party sitting from
    /// first 02:40 to second reads as empty in wall minutes.
    pub seated_now: Option<i32>,
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
    /// Room version at read. Answers arrive out of order; screen ignores evening older than shown.
    pub version: i64,
    /// Running shift by bar clock, whatever day shown; phone may sit in other timezone.
    pub today: NaiveDate,
    pub hours: Hours,
    pub tables: Vec<ShiftTable>,
    pub bookings: Vec<ShiftBooking>,
    pub stats: ShiftStats,
    /// Where to draw the "now" line, absent for a shift that is not running.
    pub now_minutes: Option<i32>,
    /// Absent when none fits or [`Self::walk_in_until_minutes`] absent. Uses walk-in endpoint
    /// window, so never promises table door then refuses.
    ///
    /// Answered here, by the allocator, rather than inferred from a count of free tables: seven
    /// free two-tops do not seat the four people at the door, and a bartender who is sent to
    /// another view to find that out has been failed by the one he was on.
    pub largest_party_seatable_now: Option<i32>,
    /// End of walk-in window, wall minutes into shift. Absent unless running shift and open. Server
    /// computes it: phone wall-minute math breaks on clock-change nights.
    pub walk_in_until_minutes: Option<i32>,
    /// Same list walk-in endpoint seats from, any size; empty when
    /// [`Self::walk_in_until_minutes`] absent.
    pub walk_in_free_table_ids: Vec<Uuid>,
    /// Every day staff can reach from here, with what is on. Longer than the guest's horizon on
    /// purpose: a telephone booking for next month is not a thing to argue about.
    pub days: Vec<ShiftDay>,
    /// How far ahead guests may book, so the day sheet can say where their horizon ends.
    pub guest_horizon_days: i32,
    pub cancel_reasons: Vec<String>,
    pub message_templates: Vec<String>,
}

/// Party that left or never came does not hold table staff see standing empty.
fn holds_table_at(record: &BookingRecord, now: DateTime<Utc>) -> bool {
    record
        .booking
        .occupancy()
        .is_some_and(|held| held.contains(now))
}

impl ShiftView {
    /// One drawing for `GET /shift` and every write answer, so rules never diverge.
    pub fn of(evening: &Evening) -> Self {
        let Evening {
            now,
            day,
            today,
            config,
            bookings,
            blocks,
            days,
            version,
        } = evening;
        let (now, day) = (*now, *day);

        let tables: Vec<ShiftTable> = config
            .active_tables()
            .map(|table| ShiftTable {
                id: table.id.0,
                number: table.number,
                seats: table.seats,
                zone: table.zone.as_str().to_owned(),
                blocked_because: blocks
                    .iter()
                    .find(|block| {
                        block.block.table_id == table.id && block.block.service_day == day
                    })
                    .map(|block| block.reason.clone()),
            })
            .collect();

        // "Free now", the now-line and "who fits" are only meaningful on the shift that is actually
        // running. On any other day an invented number would be worse than a blank.
        let is_running = *today == day;
        let free_now = is_running.then(|| {
            tables
                .iter()
                .filter(|table| table.blocked_because.is_none())
                .filter(|table| {
                    !bookings.iter().any(|record| {
                        record.booking.table_id == Some(TableId(table.id))
                            && holds_table_at(record, now)
                    })
                })
                .count()
        });
        let seated_now = is_running.then(|| {
            bookings
                .iter()
                .filter(|record| {
                    matches!(
                        record.booking.status,
                        BookingStatus::Arrived | BookingStatus::Left
                    ) && holds_table_at(record, now)
                })
                .map(|record| record.booking.party_size)
                .sum()
        });
        let now_minutes = is_running.then(|| minutes_within(day, now, config.timezone));
        let (live, closed) = (bookings_of(bookings), blocks_of(blocks));
        let walk_in = pustol_domain::walk_in(config, day, now, &live, &closed);
        let walk_in_until_minutes = walk_in
            .as_ref()
            .map(|offer| minutes_within(day, offer.window.end(), config.timezone));
        let largest_party_seatable_now = walk_in
            .as_ref()
            .and_then(|offer| offer.largest_party(config.max_party));
        let walk_in_free_table_ids = walk_in.map_or_else(Vec::new, |offer| {
            offer.tables.iter().map(|table| table.id.0).collect()
        });

        Self {
            service_date: day.date(),
            version: *version,
            today: today.date(),
            hours: config.week.for_service_day(day).into(),
            tables,
            bookings: bookings
                .iter()
                .map(|record| ShiftBooking::of(record, config, now))
                .collect(),
            stats: ShiftStats {
                bookings: bookings.len(),
                guests: bookings
                    .iter()
                    .map(|record| record.booking.party_size)
                    .sum(),
                free_now,
                seated_now,
            },
            now_minutes,
            largest_party_seatable_now,
            walk_in_until_minutes,
            walk_in_free_table_ids,
            days: days
                .iter()
                .map(|count| ShiftDay {
                    service_date: count.day.date(),
                    closed: config.week.for_service_day(count.day).closed,
                    bookings: count.bookings,
                })
                .collect(),
            guest_horizon_days: config.horizon_days,
            cancel_reasons: config.cancel_reasons.clone(),
            message_templates: config.message_templates.clone(),
        }
    }
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
    pub service_date: ServiceDate,
    pub start_minutes: i32,
    pub party_size: i32,
    /// What «Перенести» promised; `booking_changed` unless exact match. Absent means none, so
    /// old app is refused wherever booking would replace something.
    #[serde(default)]
    pub replacing: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct StaffBookingRequest {
    pub service_date: ServiceDate,
    pub start_minutes: i32,
    pub party_size: i32,
    pub guest_name: String,
    /// The table staff chose. Absent asks the room to choose.
    #[serde(default)]
    pub table_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct AvailabilityQuery {
    pub service_date: ServiceDate,
    pub party_size: i32,
    /// A booking being moved, which must not block its own time.
    #[serde(default)]
    pub ignoring: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ShiftQuery {
    pub service_date: ServiceDate,
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

/// Where a booking now sits, and when. Both named in full, so there is no reading in which the
/// caller meant one and the server changed the other.
#[derive(Debug, Deserialize)]
pub struct MoveRequest {
    pub start_minutes: i32,
    /// The table staff chose. Absent asks the room to choose, as everywhere else.
    #[serde(default)]
    pub table_id: Option<Uuid>,
    /// Absent keeps party size.
    #[serde(default)]
    pub party_size: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct WalkInRequest {
    pub service_date: ServiceDate,
    pub party_size: i32,
    /// The table staff chose while looking at the room. Absent asks the room to choose, which is
    /// the same best fit every other booking gets.
    #[serde(default)]
    pub table_id: Option<Uuid>,
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
    pub service_date: ServiceDate,
    pub table_ids: Vec<Uuid>,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct UnblockRequest {
    pub service_date: ServiceDate,
    pub table_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ReconcileRequest {
    pub service_date: ServiceDate,
}

// ---- settings ---------------------------------------------------------------------------------

/// The settings screen's current state, and the bounds every control has to respect.
///
/// The bounds travel with the settings so the screen can grey out a stepper without a round trip,
/// and so widening a limit needs no change here.
#[derive(Debug, Serialize)]
pub struct SettingsView {
    /// Bumped every write; save echoes it and is refused if settings changed since.
    pub version: i64,
    pub name: String,
    pub address: String,
    /// As typed; empty when none.
    pub contact: String,
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
}

#[derive(Debug, Serialize)]
pub struct StaffView {
    pub username: String,
    /// Whether this invitation has been claimed by an account yet.
    pub bound: bool,
}

#[derive(Debug, Serialize)]
pub struct LimitsView {
    pub open_minutes: Bounds,
    pub close_minutes: Bounds,
    pub turn_minutes: Bounds,
    pub max_party: Bounds,
    pub horizon_days: Bounds,
    pub remind_hours: Bounds,
    pub grace_minutes: Bounds,
    pub seats: Bounds,
    pub slot_step_minutes: &'static [i32],
    pub text: TextLimits,
    /// Screen hides «Добавить» at bound.
    pub lists: ListLimits,
}

impl LimitsView {
    pub const fn current() -> Self {
        Self {
            open_minutes: LIMITS.open_minutes,
            close_minutes: LIMITS.close_minutes,
            turn_minutes: LIMITS.turn_minutes,
            max_party: LIMITS.max_party,
            horizon_days: LIMITS.horizon_days,
            remind_hours: LIMITS.remind_hours,
            grace_minutes: LIMITS.grace_minutes,
            seats: LIMITS.seats,
            slot_step_minutes: LIMITS.slot_step_minutes,
            text: LIMITS.text,
            lists: LIMITS.lists,
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
