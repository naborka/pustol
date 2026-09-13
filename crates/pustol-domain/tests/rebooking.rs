//! What a guest's screen may offer to do with a booking they hold.

mod common;

use chrono::{NaiveDate, TimeDelta};
use pustol_domain::allocator::{Booking, BookingStatus};
use pustol_domain::config::{BarConfig, DayHours, WeekSchedule};
use pustol_domain::rebooking::{Rebooking, refused_on};
use pustol_domain::service_day::ServiceDay;
use pustol_domain::slots::has_arrival_after;

use common::{booking, default_config, force, thursday, utc};

/// The fixture bar, open 10:00 to 02:00, with this turn and time step.
fn grid(turn_minutes: i32, slot_step_minutes: i32) -> BarConfig {
    BarConfig {
        week: WeekSchedule::uniform(DayHours {
            open_minutes: 600,
            close_minutes: 1560,
            closed: false,
        }),
        turn_minutes,
        slot_step_minutes,
        ..default_config()
    }
}

/// A party due at midnight that staff marked as not coming before it was due, so the table is held
/// for them through the grace period.
fn held_no_show(turn_minutes: i32) -> Booking {
    let due = booking(1, thursday(), 1440, 2, None, turn_minutes);
    Booking {
        status: BookingStatus::NoShow,
        released_at: Some(due.window.start() + TimeDelta::minutes(15)),
        ..due
    }
}

#[test]
fn a_held_no_show_is_not_offered_a_move_when_its_evening_has_no_arrival_time_left() {
    // Last arrival by the hours is 00:30 (close less a ninety-minute turn), but the hourly grid's
    // last arrival is midnight. At 00:10 the grid has nothing left, whatever closing less a turn says.
    let config = force(grid(90, 60));
    let absent = held_no_show(90);
    let now = utc(2026, 7, 30, 22, 10);

    assert_eq!(absent.rebooking(now), Some(Rebooking::SameEvening), "still held");
    assert!(!has_arrival_after(&config, thursday(), now));
    assert_eq!(absent.rebooking_on_offer(std::slice::from_ref(&absent), &config, now), None);
}

#[test]
fn a_held_no_show_is_offered_a_move_while_its_evening_still_has_an_arrival_time() {
    let config = force(grid(90, 30));
    let absent = held_no_show(90);
    let now = utc(2026, 7, 30, 22, 10);

    assert!(has_arrival_after(&config, thursday(), now), "00:30 is still ahead");
    assert_eq!(
        absent.rebooking_on_offer(std::slice::from_ref(&absent), &config, now),
        Some(Rebooking::SameEvening)
    );
}

#[test]
fn a_plan_is_offered_a_move_to_any_evening_whatever_its_own_evening_has_left() {
    let config = force(grid(90, 60));
    let plan = booking(2, thursday(), 1440, 2, None, 90);
    let now = utc(2026, 7, 30, 21, 59);

    assert_eq!(
        plan.rebooking_on_offer(std::slice::from_ref(&plan), &config, now),
        Some(Rebooking::AnyEvening)
    );
}

#[test]
fn a_booking_holds_its_evening_exactly_when_a_new_booking_on_it_is_refused() {
    // Half past eight on Thursday, with two-hour sittings from eight.
    let during = utc(2026, 7, 30, 18, 30);
    let eight = |sequence, status| Booking {
        status,
        ..booking(sequence, thursday(), 1200, 2, None, 120)
    };
    let cases = [
        ("at the table", eight(1, BookingStatus::Arrived), true),
        ("under way, unmarked", eight(2, BookingStatus::Confirmed), true),
        (
            "a plan for ten",
            booking(3, thursday(), 1320, 2, None, 120),
            false,
        ),
        (
            "not coming at ten, still held",
            Booking {
                status: BookingStatus::NoShow,
                released_at: Some(utc(2026, 7, 30, 20, 15)),
                ..booking(4, thursday(), 1320, 2, None, 120)
            },
            false,
        ),
        (
            "gone home",
            Booking {
                released_at: Some(utc(2026, 7, 30, 18, 10)),
                ..eight(5, BookingStatus::Left)
            },
            false,
        ),
    ];
    for (what, held, holds) in cases {
        assert_eq!(held.holds_evening(during), holds, "{what}");
        assert_eq!(
            refused_on(std::slice::from_ref(&held), thursday(), during),
            holds,
            "{what}"
        );
    }
}

#[test]
fn a_held_no_show_on_an_evening_the_guest_may_no_longer_book_is_not_offered_a_move() {
    // Booked for Sunday under a four-day horizon; the manager lowered it to two, and staff marked the
    // party as not coming before it was due. Only a booking on Sunday takes its place, and Sunday is
    // refused to guests now. A plan on Sunday is still replaced by a booking on any evening, so the
    // screen still says so: the app names what it replaces from this.
    let config = force(BarConfig {
        horizon_days: 2,
        ..grid(120, 30)
    });
    let sunday = thursday().checked_add_days(3).expect("in range");
    let plan = booking(1, sunday, 1200, 2, None, 120);
    let absent = Booking {
        status: BookingStatus::NoShow,
        released_at: Some(plan.window.start() + TimeDelta::minutes(15)),
        ..booking(2, sunday, 1200, 2, None, 120)
    };
    let morning = utc(2026, 7, 30, 6, 0);

    assert_eq!(absent.rebooking(morning), Some(Rebooking::SameEvening), "still held");
    assert!(has_arrival_after(&config, sunday, morning));
    assert_eq!(absent.rebooking_on_offer(std::slice::from_ref(&absent), &config, morning), None);
    assert_eq!(
        plan.rebooking_on_offer(std::slice::from_ref(&plan), &config, morning),
        Some(Rebooking::AnyEvening)
    );
}

#[test]
fn a_plan_is_offered_a_move_only_while_an_evening_is_left_that_the_guest_could_book() {
    // Twenty past eight on Thursday. The guest's eight o'clock table has begun and holds Thursday, and
    // they have a plan for Sunday. Under a four-day horizon Friday and Saturday are there to move it to.
    // Under a horizon of one only Thursday may be booked, and Thursday is theirs already.
    let during = utc(2026, 7, 30, 18, 20);
    let horizon = |horizon_days| force(BarConfig { horizon_days, ..grid(120, 30) });
    let sunday = thursday().checked_add_days(3).expect("in range");
    let tonight = Booking {
        status: BookingStatus::Arrived,
        ..booking(1, thursday(), 1200, 2, None, 120)
    };
    let plan = booking(2, sunday, 1200, 2, None, 120);
    let guest = [tonight, plan.clone()];

    assert_eq!(plan.rebooking_on_offer(&guest, &horizon(4), during), Some(Rebooking::AnyEvening));
    assert_eq!(plan.rebooking_on_offer(&guest, &horizon(1), during), None);
    assert_eq!(
        plan.rebooking_on_offer(std::slice::from_ref(&plan), &horizon(1), during),
        Some(Rebooking::AnyEvening),
        "Thursday is free to them and has arrival times left"
    );

    // Ten past midnight on an hourly grid of ninety-minute sittings: Thursday has no arrival time left.
    let late = force(BarConfig { horizon_days: 1, ..grid(90, 60) });
    let after_midnight = utc(2026, 7, 30, 22, 10);
    assert_eq!(late.current_service_day(after_midnight), thursday());
    assert_eq!(plan.rebooking_on_offer(std::slice::from_ref(&plan), &late, after_midnight), None);
}

#[test]
fn an_arrival_time_the_clocks_skip_is_not_one_left() {
    // Saturday 28 March 2026 in Belgrade: 02:00 and 02:30 on Sunday never happen, and they are the
    // grid's last two arrivals. At 01:45 closing less a turn is still ahead; no arrival is.
    let mut config = grid(60, 30);
    config.week = WeekSchedule::uniform(DayHours {
        open_minutes: 600,
        close_minutes: 1650,
        closed: false,
    });
    let config = force(config);
    let saturday = ServiceDay::new(NaiveDate::from_ymd_opt(2026, 3, 28).expect("valid date"));

    assert!(has_arrival_after(&config, saturday, utc(2026, 3, 29, 0, 20)));
    assert!(!has_arrival_after(&config, saturday, utc(2026, 3, 29, 0, 45)));
}
