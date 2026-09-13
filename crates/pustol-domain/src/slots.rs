//! The times a party can actually be seated.
//!
//! Every slot offered has been through the allocator: a free slot means a specific table was
//! found for this exact party at this exact time. Nothing is inferred from a seat count, so a
//! guest is never shown a time that the booking endpoint would then refuse — the picker and the
//! endpoint ask the same question of the same function.

use chrono::{DateTime, Utc};

use crate::allocator::{self, Booking, TableBlock};
use crate::config::{EVENING_FROM_MINUTES, ValidConfig};
use crate::schedule::BarTable;
use crate::service_day::{Interval, ServiceDay, resolve};

/// Why a slot can or cannot be taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotAvailability {
    /// A table was found. Which one is settled when the booking is written, from the same list.
    Free,
    /// Every table that could seat this party is busy or closed.
    Taken,
    /// The time has already passed. Offering it would let a guest book the past.
    Past,
    /// This wall-clock time does not exist on this date, because the clocks jumped over it.
    Nonexistent,
}

impl SlotAvailability {
    #[must_use]
    pub const fn is_free(self) -> bool {
        matches!(self, Self::Free)
    }

    /// Whether the slot should be shown to a guest at all.
    ///
    /// A taken slot is shown greyed out, because seeing the evening fill up is information. A
    /// time that does not exist is not information, it is confusion.
    #[must_use]
    pub const fn is_offerable(self) -> bool {
        !matches!(self, Self::Nonexistent)
    }
}

/// Whether a slot belongs to the daytime or the evening block of the picker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PartOfDay {
    Day,
    Evening,
}

/// One candidate arrival time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    /// Wall-clock minutes from the start of the shift — what the guest reads on the button.
    pub start_minutes: i32,
    /// The absolute window the booking would occupy.
    pub window: Option<Interval>,
    pub part_of_day: PartOfDay,
    pub availability: SlotAvailability,
}

/// Everything slot generation is allowed to look at.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    pub config: &'a ValidConfig,
    pub service_day: ServiceDay,
    pub party_size: i32,
    /// Bookings that could conflict, from this shift and its neighbours.
    pub bookings: &'a [Booking],
    pub blocks: &'a [TableBlock],
    /// The instant the guest is looking at the picker.
    pub now: DateTime<Utc>,
    /// Bookings set aside: one being moved, which must not block its own slot, or the ones a guest's
    /// booking again would replace.
    pub ignoring: &'a [crate::allocator::BookingId],
}

impl<'a> Query<'a> {
    /// The allocation question this query asks about one window — the same one the grid asks, so
    /// a caller that seats a booking cannot be looking at a different room from the picker.
    #[must_use]
    pub fn request(&self, window: Interval) -> allocator::Request<'a> {
        allocator::Request {
            party_size: self.party_size,
            window,
            service_day: self.service_day,
            tables: &self.config.tables,
            bookings: self.bookings,
            blocks: self.blocks,
            ignoring: self.ignoring,
        }
    }
}

/// Every arrival time on `service_day`, each with the reason it can or cannot be taken.
///
/// The last slot is closing time minus one turn, so a booking never runs past the moment the
/// lights go off. On a day off the list is empty; there is nothing to offer and no need for a
/// special case anywhere downstream.
#[must_use]
pub fn slot_list(query: &Query<'_>) -> Vec<Slot> {
    arrival_minutes(query.config, query.service_day)
        .map(|minutes| evaluate(query, minutes))
        .collect()
}

/// One arrival time, judged exactly as the grid judges it, or `None` when the shift has no such
/// time at all. The same `evaluate` — so taking a booking cannot disagree with the picker that
/// offered it — without running the allocator over the other forty-odd slots to answer about one.
#[must_use]
pub fn slot_at(query: &Query<'_>, start_minutes: i32) -> Option<Slot> {
    arrival_minutes(query.config, query.service_day)
        .find(|minutes| *minutes == start_minutes)
        .map(|minutes| evaluate(query, minutes))
}

/// Every table free for `slot`'s window whatever the party, as [`allocator::open_tables_for`] answers
/// this query about it, whatever the slot's state; empty for a slot the clocks jump over.
#[must_use]
pub fn open_tables_at<'a>(query: &Query<'a>, slot: &Slot) -> Vec<&'a BarTable> {
    slot.window.map_or_else(Vec::new, |window| {
        allocator::open_tables_for(&query.request(window))
    })
}

/// Whether `day`'s grid still has an arrival time after `now`: one that happens, and that the grid
/// would not call past.
///
/// Asked of the grid itself, never of closing time less a turn. A step that does not divide the
/// shift ends the grid earlier than that, and the clocks can skip its last arrivals; either way a
/// guest told they could still move to tonight would find no time to move to.
#[must_use]
pub fn has_arrival_after(config: &ValidConfig, day: ServiceDay, now: DateTime<Utc>) -> bool {
    arrival_minutes(config, day)
        .any(|minutes| matches!(timing(config, day, minutes, now), Timing::Ahead(_)))
}

/// Every wall-clock minute a party may arrive at on this shift, in order.
///
/// The one definition of "when the grid runs from and to": opening time, in the configured step,
/// stopping one turn before closing so no booking runs past the moment the lights go off. Empty on
/// a day off, which is what makes every caller need no special case for one.
///
/// Written once because every caller walks it — the whole grid, one slot of it, the first free slot
/// on it, and whether any of it is left — and copies of the same loop are chances to disagree about
/// the last arrival time.
fn arrival_minutes(config: &ValidConfig, day: ServiceDay) -> impl Iterator<Item = i32> {
    let hours = config.week.for_service_day(day);
    // A validated config guarantees a positive step, so this range is always finite.
    let step = config.slot_step_minutes;
    let last_arrival = hours.close_minutes - config.turn_minutes;
    let open = hours.open_minutes;
    let closed = hours.closed;
    std::iter::successors(
        (!closed && open <= last_arrival).then_some(open),
        move |minutes| {
            let next = minutes + step;
            (next <= last_arrival).then_some(next)
        },
    )
}

/// Where one arrival time stands against the clock, before anybody asks which table it would get.
enum Timing {
    /// The clocks jump over it.
    Nonexistent,
    Past(Interval),
    Ahead(Interval),
}

fn timing(config: &ValidConfig, day: ServiceDay, start_minutes: i32, now: DateTime<Utc>) -> Timing {
    match window_of(config, day, start_minutes) {
        None => Timing::Nonexistent,
        Some(window) if window.start() <= now => Timing::Past(window),
        Some(window) => Timing::Ahead(window),
    }
}

/// The window a party arriving `start_minutes` into `day` holds, or `None` when the clocks jump over
/// that time.
fn window_of(config: &ValidConfig, day: ServiceDay, start_minutes: i32) -> Option<Interval> {
    // Against a validated config the only way this can fail is a wall-clock time the spring
    // clock change jumps over: the minute offset is bounded by closing time and the turn is
    // positive, so neither of the other failures is reachable.
    resolve(day, start_minutes, config.timezone)
        .and_then(|start| Interval::from_duration(start, config.turn_minutes))
        .ok()
}

/// The window of the last sitting `day`'s grid has: its last arrival time that happens. `None` on a
/// day off, or when the clocks jump over every arrival.
///
/// Asked of the grid itself, like [`has_arrival_after`]: closing less a turn is not always an arrival
/// the grid has, nor one that happens.
pub(crate) fn last_sitting(config: &ValidConfig, day: ServiceDay) -> Option<Interval> {
    arrival_minutes(config, day)
        .filter_map(|minutes| window_of(config, day, minutes))
        .last()
}

fn evaluate(query: &Query<'_>, start_minutes: i32) -> Slot {
    let part_of_day = if start_minutes >= EVENING_FROM_MINUTES {
        PartOfDay::Evening
    } else {
        PartOfDay::Day
    };
    let slot = |window, availability| Slot {
        start_minutes,
        window,
        part_of_day,
        availability,
    };

    match timing(query.config, query.service_day, start_minutes, query.now) {
        Timing::Nonexistent => slot(None, SlotAvailability::Nonexistent),
        Timing::Past(window) => slot(Some(window), SlotAvailability::Past),
        Timing::Ahead(window) => match allocator::assign(&query.request(window)) {
            Some(_) => slot(Some(window), SlotAvailability::Free),
            None => slot(Some(window), SlotAvailability::Taken),
        },
    }
}

/// Every service day inside the bar's booking horizon, nearest first, closed days included.
///
/// The guest's day rail is exactly this long, because the horizon is a number the manager sets and
/// a rail that silently dropped the Mondays the bar is shut would be a different length on
/// different weeks — and would leave a guest wondering where Monday went. A closed day is shown
/// and says `выходной`; it is answered, not hidden.
#[must_use]
pub fn horizon_days(config: &ValidConfig, today: ServiceDay) -> Vec<ServiceDay> {
    days_from(today, config.horizon_days)
}

/// `count` service days from `from`, nearest first.
#[must_use]
pub fn days_from(from: ServiceDay, count: i32) -> Vec<ServiceDay> {
    let count = u64::try_from(count).unwrap_or(0);
    (0..count)
        .filter_map(|offset| from.checked_add_days(offset))
        .collect()
}

/// Service days a guest may actually book, nearest first.
///
/// Days the bar is shut are left out: this is the set a booking request is checked against, and a
/// day the bar is closed is not one of them. It is deliberately *not* what the day rail is drawn
/// from — see [`horizon_days`].
#[must_use]
pub fn bookable_days(config: &ValidConfig, today: ServiceDay) -> Vec<ServiceDay> {
    horizon_days(config, today)
        .into_iter()
        .filter(|day| !config.week.for_service_day(*day).closed)
        .collect()
}

/// Whether a guest may book `day` at `now`: whether it is one of the [`bookable_days`] counted from
/// the shift running then.
///
/// The question the booking endpoint refuses by and every offer made to a guest is held to, so the
/// screen never offers a move to an evening the endpoint then refuses.
#[must_use]
pub fn guest_may_book(config: &ValidConfig, day: ServiceDay, now: DateTime<Utc>) -> bool {
    bookable_days(config, config.current_service_day(now)).contains(&day)
}

/// The earliest arrival time still free for this party, or `None` when the day holds none.
///
/// What one chip on the day rail says about itself: `с 21:30`, or `мест нет`. Evaluated in order
/// and stopped at the first free slot, because a rail thirty days long would otherwise run the
/// allocator over every slot of every day to answer a question that is settled by the first.
#[must_use]
pub fn first_free_minutes(query: &Query<'_>) -> Option<i32> {
    arrival_minutes(query.config, query.service_day)
        .find(|minutes| evaluate(query, *minutes).availability.is_free())
}
