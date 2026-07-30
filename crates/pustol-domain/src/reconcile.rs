//! Putting the bookings back where they belong after the room changes underneath them.
//!
//! Closing the terrace, shrinking a six-top to a four-top and retiring a table are all the same
//! event as far as the bookings are concerned: some seatings that were sound a moment ago are not
//! any more. Handling them once, here, is what stops three subtly different reassignment loops
//! from growing in three places and disagreeing about the awkward cases.
//!
//! Inventory changes are applied and the bookings reconciled, never refused. The settings screen
//! describes the room as it now physically is — the terrace really is under water — and a system
//! that refuses to record that fact just gets worked around.

use chrono::{DateTime, Utc};

use crate::allocator::{self, Booking, BookingId, TableBlock, seating_is_sound};
use crate::schedule::{BarTable, TableId};

/// A booking that had to be given a different table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reseating {
    pub booking: BookingId,
    pub from: Option<TableId>,
    pub to: TableId,
    /// The number staff will read out, carried so the report needs no second lookup.
    pub to_number: i32,
}

/// What reconciliation decided. The caller persists it; nothing here writes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Reconciliation {
    pub moved: Vec<Reseating>,
    /// Bookings left with no table at all. They are still live, still owed a table, and the
    /// admin screen shows them until somebody settles the debt. Leaving them pointing at a
    /// closed table instead would render them on that table's row, which is a lie.
    pub orphaned: Vec<BookingId>,
}

impl Reconciliation {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.moved.is_empty() && self.orphaned.is_empty()
    }
}

/// Everything reconciliation is allowed to look at.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// The room as it now is, retirements and resizes included.
    pub tables: &'a [BarTable],
    /// Every live booking that could be affected or could stand in the way of a move.
    pub bookings: &'a [Booking],
    /// Blocks as they now are, the newly added ones included.
    pub blocks: &'a [TableBlock],
    pub now: DateTime<Utc>,
}

/// Re-seats every booking the room can no longer honour, and reports what it could not place.
///
/// Only bookings that have not finished yet are touched. A booking from last Friday cannot be
/// moved — it already happened — and rewriting it would corrupt the shift history that staff rely
/// on to settle disputes.
///
/// Candidates are handled in arrival order so that the outcome is reproducible: the same room
/// change replayed on the same bookings always produces the same report, whatever order the
/// caller happened to load rows in.
#[must_use]
pub fn reconcile(request: &Request<'_>) -> Reconciliation {
    let mut working: Vec<Booking> = request
        .bookings
        .iter()
        .filter(|booking| booking.status.holds_a_table())
        .cloned()
        .collect();

    // Sorted on the arrival time already in hand, rather than on a key that looks the booking up
    // again for every comparison.
    let mut candidates: Vec<(DateTime<Utc>, BookingId)> = working
        .iter()
        .filter(|booking| booking.window.end() > request.now)
        .filter(|booking| !seating_is_sound(booking, request.tables, request.blocks))
        .map(|booking| (booking.window.start(), booking.id))
        .collect();
    candidates.sort_unstable();

    let mut outcome = Reconciliation::default();
    for (_, id) in candidates {
        let booking = find(&working, id).expect("candidates come from the working set").clone();
        let previous = booking.table_id;
        let assignment = allocator::assign(&allocator::Request {
            party_size: booking.party_size,
            window: booking.window,
            service_day: booking.service_day,
            tables: request.tables,
            bookings: &working,
            blocks: request.blocks,
            ignoring: Some(id),
        });

        let seat = assignment.map(|found| found.table_id);
        if let Some(booking) = working.iter_mut().find(|booking| booking.id == id) {
            booking.table_id = seat;
        }
        match assignment {
            Some(found) => outcome.moved.push(Reseating {
                booking: id,
                from: previous,
                to: found.table_id,
                to_number: found.number,
            }),
            None => outcome.orphaned.push(id),
        }
    }
    outcome
}

fn find(bookings: &[Booking], id: BookingId) -> Option<&Booking> {
    bookings.iter().find(|booking| booking.id == id)
}
