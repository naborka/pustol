//! The times a party can actually be seated.
//!
//! Every slot offered has been through the allocator: a free slot means a specific table was
//! found for this exact party at this exact time. Nothing is inferred from a seat count, so a
//! guest is never shown a time that the booking endpoint would then refuse — the picker and the
//! endpoint ask the same question of the same function.

use chrono::{DateTime, Utc};

use crate::allocator::{self, Booking, TableBlock};
use crate::config::{EVENING_FROM_MINUTES, ValidConfig};
use crate::schedule::TableId;
use crate::service_day::{Interval, ServiceDay, resolve};

/// Why a slot can or cannot be taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotAvailability {
    /// A table was found. Which one is an internal detail: guests are never shown a table
    /// number, and staff see the assignment only once the booking exists.
    Free { table: TableId },
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
        matches!(self, Self::Free { .. })
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
    /// A booking being moved, which must not block its own slot.
    pub ignoring: Option<crate::allocator::BookingId>,
}

/// Every arrival time on `service_day`, each with the reason it can or cannot be taken.
///
/// The last slot is closing time minus one turn, so a booking never runs past the moment the
/// lights go off. On a day off the list is empty; there is nothing to offer and no need for a
/// special case anywhere downstream.
#[must_use]
pub fn slot_list(query: &Query<'_>) -> Vec<Slot> {
    let hours = query.config.week.for_service_day(query.service_day);
    if hours.closed {
        return Vec::new();
    }
    // A validated config guarantees a positive step, so this loop always terminates.
    let step = query.config.slot_step_minutes;
    let last_arrival = hours.close_minutes - query.config.turn_minutes;

    let mut slots = Vec::new();
    let mut minutes = hours.open_minutes;
    while minutes <= last_arrival {
        slots.push(evaluate(query, minutes));
        minutes += step;
    }
    slots
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

    // Against a validated config the only way this can fail is a wall-clock time the spring
    // clock change jumps over: the minute offset is bounded by closing time and the turn is
    // positive, so neither of the other failures is reachable.
    let Ok(window) = resolve(query.service_day, start_minutes, query.config.timezone)
        .and_then(|start| Interval::from_duration(start, query.config.turn_minutes))
    else {
        return slot(None, SlotAvailability::Nonexistent);
    };

    if window.start() <= query.now {
        return slot(Some(window), SlotAvailability::Past);
    }

    let assignment = allocator::assign(&allocator::Request {
        party_size: query.party_size,
        window,
        service_day: query.service_day,
        tables: &query.config.tables,
        bookings: query.bookings,
        blocks: query.blocks,
        ignoring: query.ignoring,
    });

    match assignment {
        Some(found) => slot(Some(window), SlotAvailability::Free { table: found.table_id }),
        None => slot(Some(window), SlotAvailability::Taken),
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

/// The earliest arrival time still free for this party, or `None` when the day holds none.
///
/// What one chip on the day rail says about itself: `с 21:30`, or `мест нет`. Evaluated in order
/// and stopped at the first free slot, because a rail thirty days long would otherwise run the
/// allocator over every slot of every day to answer a question that is settled by the first.
#[must_use]
pub fn first_free_minutes(query: &Query<'_>) -> Option<i32> {
    let hours = query.config.week.for_service_day(query.service_day);
    if hours.closed {
        return None;
    }
    let step = query.config.slot_step_minutes;
    let last_arrival = hours.close_minutes - query.config.turn_minutes;
    let mut minutes = hours.open_minutes;
    while minutes <= last_arrival {
        if evaluate(query, minutes).availability.is_free() {
            return Some(minutes);
        }
        minutes += step;
    }
    None
}
