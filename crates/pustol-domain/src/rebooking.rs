//! What booking again does to the bookings a guest already holds.
//!
//! **The one rule for rebooking.** Which bookings a new one cancels, which evenings a guest is
//! refused a second table on, which of their own bookings every offer made to them sets aside, and
//! what their screen says booking again would do, are all this module. Asked in two places, the
//! picker would offer a time the booking endpoint then refused, or the endpoint would cancel a
//! booking the picker had counted as staying.

use chrono::{DateTime, Utc};

use crate::allocator::{Booking, BookingId, BookingStatus};
use crate::config::ValidConfig;
use crate::service_day::ServiceDay;
use crate::slots::{guest_may_book, has_arrival_after};

/// Which new booking of the same guest replaces a booking they hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rebooking {
    /// A plan that has not started: the guest changed their mind, and a booking on any evening is
    /// the new plan.
    AnyEvening,
    /// A no-show whose table the bar still holds through the grace period: a booking on the same
    /// evening takes its place. Another evening leaves it alone, because it is that evening's
    /// record of who did not come, not a plan the guest still has.
    SameEvening,
}

impl Booking {
    /// What a new booking by this booking's guest does to it at `now`, or `None` when nothing
    /// replaces it.
    ///
    /// A party at its table, or a plan already under way, is never replaced: cancelling it because
    /// the guest tapped Book again would take the table out from under them. A booking whose table
    /// is no longer held has nothing left to replace.
    #[must_use]
    pub fn rebooking(&self, now: DateTime<Utc>) -> Option<Rebooking> {
        if self.has_finished(now) {
            return None;
        }
        match self.status {
            BookingStatus::Confirmed if !self.has_started(now) => Some(Rebooking::AnyEvening),
            BookingStatus::NoShow => Some(Rebooking::SameEvening),
            BookingStatus::Confirmed
            | BookingStatus::Arrived
            | BookingStatus::Left
            | BookingStatus::Cancelled => None,
        }
    }

    /// What the guest's screen offers booking again to do to this booking at `now`, or `None` when
    /// it offers nothing.
    ///
    /// [`Self::rebooking`], except for a no-show on an evening no booking can now be made on: only a
    /// booking on that evening takes its place, so it is offered only while guests may book that
    /// evening and its grid has an arrival time left. Offering «Перенести» otherwise would promise a
    /// move the endpoint refuses.
    ///
    /// A plan is offered whatever its own evening, a lowered horizon included: a booking on any
    /// evening replaces it, and the app names what a booking replaces from this answer.
    #[must_use]
    pub fn rebooking_on_offer(&self, config: &ValidConfig, now: DateTime<Utc>) -> Option<Rebooking> {
        self.rebooking(now).filter(|rebooking| match rebooking {
            Rebooking::AnyEvening => true,
            Rebooking::SameEvening => {
                guest_may_book(config, self.service_day, now)
                    && has_arrival_after(config, self.service_day, now)
            }
        })
    }

    /// Whether this booking holds its evening at `now`: its table is still held for the guest and
    /// no new booking of theirs replaces it, so a booking on that evening is refused.
    ///
    /// **The one rule for "this evening is already yours".** [`refused_on`] is this question asked of
    /// every booking a guest holds, so the guest's card and the endpoint cannot disagree about it.
    #[must_use]
    pub fn holds_evening(&self, now: DateTime<Utc>) -> bool {
        !self.has_finished(now) && self.rebooking(now).is_none()
    }

    /// Whether this booking is a plan at `now`: confirmed, and its time not yet come.
    #[must_use]
    pub fn is_plan(&self, now: DateTime<Utc>) -> bool {
        self.rebooking(now) == Some(Rebooking::AnyEvening)
    }

    fn replaced_by_booking_on(&self, day: ServiceDay, now: DateTime<Utc>) -> bool {
        match self.rebooking(now) {
            Some(Rebooking::AnyEvening) => true,
            Some(Rebooking::SameEvening) => self.service_day == day,
            None => false,
        }
    }
}

/// The bookings of one guest that a new booking of theirs on `day` replaces, soonest first.
///
/// `guest` is every booking that guest holds; the rule decides which of them count.
#[must_use]
pub fn replaced_on(guest: &[Booking], day: ServiceDay, now: DateTime<Utc>) -> Vec<BookingId> {
    let mut replaced: Vec<&Booking> = guest
        .iter()
        .filter(|booking| booking.replaced_by_booking_on(day, now))
        .collect();
    replaced.sort_unstable_by_key(|booking| (booking.window.start(), booking.id));
    replaced.into_iter().map(|booking| booking.id).collect()
}

/// A rule about what one guest may hold, broken by one of their bookings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HoldingConflict {
    /// Another booking of theirs still holds a table on the same evening.
    SameEvening,
    /// The booking is a plan, and so is another booking of theirs.
    AnotherPlan,
}

/// **What a guest may hold.** At most one running booking on any evening, and at most one plan.
///
/// Answers which of the two rules `booking` breaks among `guest` — every booking that guest holds —
/// at `now`. Booking again keeps both by replacing; this is the statement of them that any other
/// change to a guest's booking, such as staff restoring or moving one, is checked against.
#[must_use]
pub fn holding_conflict(
    guest: &[Booking],
    booking: BookingId,
    now: DateTime<Utc>,
) -> Option<HoldingConflict> {
    let this = guest
        .iter()
        .find(|held| held.id == booking && !held.has_finished(now))?;
    let mut others = guest
        .iter()
        .filter(|held| held.id != booking && !held.has_finished(now));
    if others.clone().any(|other| other.service_day == this.service_day) {
        return Some(HoldingConflict::SameEvening);
    }
    (this.is_plan(now) && others.any(|other| other.is_plan(now)))
        .then_some(HoldingConflict::AnotherPlan)
}

/// Whether a new booking of this guest on `day` is refused, because one of their bookings on that
/// evening [holds it](Booking::holds_evening).
#[must_use]
pub fn refused_on(guest: &[Booking], day: ServiceDay, now: DateTime<Utc>) -> bool {
    guest
        .iter()
        .any(|booking| booking.service_day == day && booking.holds_evening(now))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use uuid::Uuid;

    use super::*;
    use crate::allocator::{Booking, BookingId, BookingStatus};
    use crate::service_day::{Interval, ServiceDay};

    fn thursday() -> ServiceDay {
        ServiceDay::new(NaiveDate::from_ymd_opt(2026, 7, 30).expect("valid date"))
    }

    fn friday() -> ServiceDay {
        thursday().checked_add_days(1).expect("in range")
    }

    fn at(day: ServiceDay, hour: u32) -> DateTime<Utc> {
        Utc.from_utc_datetime(&day.date().and_hms_opt(hour, 0, 0).expect("valid time"))
    }

    /// A two-hour booking from 18:00 UTC on `day`.
    fn booking(sequence: u128, day: ServiceDay, status: BookingStatus) -> Booking {
        let start = at(day, 18);
        Booking {
            id: BookingId(Uuid::from_u128(sequence)),
            table_id: None,
            service_day: day,
            window: Interval::from_duration(start, 120).expect("positive"),
            released_at: None,
            party_size: 2,
            status,
        }
    }

    fn no_show_held_until(sequence: u128, day: ServiceDay, until: DateTime<Utc>) -> Booking {
        Booking {
            released_at: Some(until),
            ..booking(sequence, day, BookingStatus::NoShow)
        }
    }

    #[test]
    fn a_plan_that_has_not_started_is_replaced_by_a_booking_on_any_evening() {
        let plan = booking(1, thursday(), BookingStatus::Confirmed);
        let before = at(thursday(), 17);
        assert_eq!(plan.rebooking(before), Some(Rebooking::AnyEvening));
        assert_eq!(
            replaced_on(std::slice::from_ref(&plan), thursday(), before),
            vec![plan.id]
        );
        assert_eq!(
            replaced_on(std::slice::from_ref(&plan), friday(), before),
            vec![plan.id]
        );
        assert!(!refused_on(std::slice::from_ref(&plan), thursday(), before));
    }

    #[test]
    fn a_no_show_whose_table_is_still_held_is_replaced_only_on_its_own_evening() {
        // Marked before the party was due, so the bar holds the table through the grace period.
        let absent = no_show_held_until(
            1,
            thursday(),
            at(thursday(), 18) + chrono::Duration::minutes(15),
        );
        let phoned = at(thursday(), 17);
        assert_eq!(absent.rebooking(phoned), Some(Rebooking::SameEvening));
        assert_eq!(
            replaced_on(std::slice::from_ref(&absent), thursday(), phoned),
            vec![absent.id]
        );
        assert_eq!(
            replaced_on(std::slice::from_ref(&absent), friday(), phoned),
            Vec::<BookingId>::new(),
            "a booking for Friday leaves Thursday's record of the evening alone"
        );
        assert!(!refused_on(
            std::slice::from_ref(&absent),
            thursday(),
            phoned
        ));
        assert!(!refused_on(std::slice::from_ref(&absent), friday(), phoned));
    }

    #[test]
    fn a_seated_party_or_a_plan_under_way_is_never_replaced_and_holds_its_evening() {
        let during = at(thursday(), 19);
        for status in [BookingStatus::Arrived, BookingStatus::Confirmed] {
            let running = booking(1, thursday(), status);
            assert_eq!(running.rebooking(during), None, "{status:?}");
            assert!(replaced_on(std::slice::from_ref(&running), thursday(), during).is_empty());
            assert!(replaced_on(std::slice::from_ref(&running), friday(), during).is_empty());
            assert!(
                refused_on(std::slice::from_ref(&running), thursday(), during),
                "{status:?}"
            );
            assert!(
                !refused_on(std::slice::from_ref(&running), friday(), during),
                "{status:?}: another evening is still free to book"
            );
        }
    }

    #[test]
    fn a_booking_whose_table_is_no_longer_held_neither_is_replaced_nor_holds_anything() {
        let over = at(thursday(), 21);
        let finished = [
            booking(1, thursday(), BookingStatus::Confirmed),
            no_show_held_until(
                2,
                thursday(),
                at(thursday(), 18) + chrono::Duration::minutes(15),
            ),
            Booking {
                released_at: Some(at(thursday(), 19)),
                ..booking(3, thursday(), BookingStatus::Left)
            },
            booking(4, thursday(), BookingStatus::Cancelled),
        ];
        for record in &finished {
            assert_eq!(record.rebooking(over), None, "{:?}", record.status);
        }
        assert!(replaced_on(&finished, thursday(), over).is_empty());
        assert!(!refused_on(&finished, thursday(), over));
    }

    #[test]
    fn a_guest_holds_at_most_one_plan_and_one_booking_an_evening() {
        let now = at(thursday(), 10);
        let saturday = thursday().checked_add_days(2).expect("in range");
        let thursday_plan = booking(1, thursday(), BookingStatus::Confirmed);
        let friday_plan = booking(2, friday(), BookingStatus::Confirmed);
        assert_eq!(
            holding_conflict(&[thursday_plan.clone(), friday_plan.clone()], thursday_plan.id, now),
            Some(HoldingConflict::AnotherPlan)
        );

        let second_thursday = Booking {
            window: Interval::from_duration(at(thursday(), 21), 60).expect("positive"),
            ..booking(3, thursday(), BookingStatus::Arrived)
        };
        assert_eq!(
            holding_conflict(&[thursday_plan.clone(), second_thursday], thursday_plan.id, now),
            Some(HoldingConflict::SameEvening),
            "two on one evening, whatever else"
        );

        let seated = at(thursday(), 19);
        let at_the_table = booking(4, thursday(), BookingStatus::Arrived);
        assert_eq!(
            holding_conflict(&[at_the_table, friday_plan.clone()], friday_plan.id, seated),
            None,
            "a table tonight and a plan for Friday is what a guest may hold"
        );

        let over = booking(5, thursday(), BookingStatus::Confirmed);
        assert_eq!(
            holding_conflict(
                &[over, friday_plan.clone()],
                friday_plan.id,
                at(thursday(), 21)
            ),
            None,
            "an evening that is over holds nothing"
        );
        let unrelated = booking(6, saturday, BookingStatus::Confirmed);
        assert_eq!(
            holding_conflict(std::slice::from_ref(&unrelated), unrelated.id, now),
            None
        );
    }

    #[test]
    fn what_a_booking_replaces_is_named_soonest_first_whatever_order_it_was_read_in() {
        let now = at(thursday(), 10);
        let saturday = thursday().checked_add_days(2).expect("in range");
        let guest = [
            booking(1, saturday, BookingStatus::Confirmed),
            booking(2, thursday(), BookingStatus::Confirmed),
            booking(3, friday(), BookingStatus::Confirmed),
        ];
        assert_eq!(
            replaced_on(&guest, friday(), now),
            vec![guest[1].id, guest[2].id, guest[0].id]
        );
    }
}
