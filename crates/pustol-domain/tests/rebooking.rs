mod common;

use chrono::{NaiveDate, TimeDelta};
use pustol_domain::allocator::{Booking, BookingStatus};
use pustol_domain::config::{BarConfig, DayHours, WeekSchedule};
use pustol_domain::rebooking::{Rebooking, refused_on};
use pustol_domain::service_day::ServiceDay;
use pustol_domain::slots::has_arrival_after;

use common::{booking, force, grid, thursday, utc};

/// Due midnight, marked no-show before due, so table held through grace.
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
    // Hours say last arrival 00:30 (close less 90-minute turn); hourly grid's last is midnight.
    // At 00:10 grid has nothing left.
    let config = force(grid(1560, 90, 60));
    let absent = held_no_show(90);
    let now = utc(2026, 7, 30, 22, 10);

    assert_eq!(
        absent.rebooking(now),
        Some(Rebooking::SameEvening),
        "still held"
    );
    assert!(!has_arrival_after(&config, thursday(), now));
    assert_eq!(
        absent.rebooking_on_offer(std::slice::from_ref(&absent), &config, now),
        None
    );
}

#[test]
fn a_held_no_show_is_offered_a_move_while_its_evening_still_has_an_arrival_time() {
    let config = force(grid(1560, 90, 30));
    let absent = held_no_show(90);
    let now = utc(2026, 7, 30, 22, 10);

    assert!(
        has_arrival_after(&config, thursday(), now),
        "00:30 is still ahead"
    );
    assert_eq!(
        absent.rebooking_on_offer(std::slice::from_ref(&absent), &config, now),
        Some(Rebooking::SameEvening)
    );
}

#[test]
fn a_plan_is_offered_a_move_to_any_evening_whatever_its_own_evening_has_left() {
    let config = force(grid(1560, 90, 60));
    let plan = booking(2, thursday(), 1440, 2, None, 90);
    let now = utc(2026, 7, 30, 21, 59);

    assert_eq!(
        plan.rebooking_on_offer(std::slice::from_ref(&plan), &config, now),
        Some(Rebooking::AnyEvening)
    );
}

#[test]
fn a_booking_holds_its_evening_exactly_when_a_new_booking_on_it_is_refused() {
    // Thursday 20:30, two-hour sittings from 20:00.
    let during = utc(2026, 7, 30, 18, 30);
    let eight = |sequence, status| Booking {
        status,
        ..booking(sequence, thursday(), 1200, 2, None, 120)
    };
    let cases = [
        ("at the table", eight(1, BookingStatus::Arrived), true),
        (
            "under way, unmarked",
            eight(2, BookingStatus::Confirmed),
            true,
        ),
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
    // Sunday booked under four-day horizon, horizon lowered to two, no-show marked before due. Only
    // Sunday booking replaces it, and Sunday now refused. Sunday plan still replaced by any evening;
    // app names what it replaces from this.
    let config = force(BarConfig {
        horizon_days: 2,
        ..grid(1560, 120, 30)
    });
    let sunday = thursday().checked_add_days(3).expect("in range");
    let plan = booking(1, sunday, 1200, 2, None, 120);
    let absent = Booking {
        status: BookingStatus::NoShow,
        released_at: Some(plan.window.start() + TimeDelta::minutes(15)),
        ..booking(2, sunday, 1200, 2, None, 120)
    };
    let morning = utc(2026, 7, 30, 6, 0);

    assert_eq!(
        absent.rebooking(morning),
        Some(Rebooking::SameEvening),
        "still held"
    );
    assert!(has_arrival_after(&config, sunday, morning));
    assert_eq!(
        absent.rebooking_on_offer(std::slice::from_ref(&absent), &config, morning),
        None
    );
    assert_eq!(
        plan.rebooking_on_offer(std::slice::from_ref(&plan), &config, morning),
        Some(Rebooking::AnyEvening)
    );
}

#[test]
fn a_plan_is_offered_a_move_only_while_an_evening_is_left_that_the_guest_could_book() {
    // Thursday 20:20. Guest's 20:00 table started, holds Thursday; plan for Sunday. Horizon four:
    // Friday and Saturday open for move. Horizon one: only Thursday, already theirs.
    let during = utc(2026, 7, 30, 18, 20);
    let horizon = |horizon_days| {
        force(BarConfig {
            horizon_days,
            ..grid(1560, 120, 30)
        })
    };
    let sunday = thursday().checked_add_days(3).expect("in range");
    let tonight = Booking {
        status: BookingStatus::Arrived,
        ..booking(1, thursday(), 1200, 2, None, 120)
    };
    let plan = booking(2, sunday, 1200, 2, None, 120);
    let guest = [tonight, plan.clone()];

    assert_eq!(
        plan.rebooking_on_offer(&guest, &horizon(4), during),
        Some(Rebooking::AnyEvening)
    );
    assert_eq!(plan.rebooking_on_offer(&guest, &horizon(1), during), None);
    assert_eq!(
        plan.rebooking_on_offer(std::slice::from_ref(&plan), &horizon(1), during),
        Some(Rebooking::AnyEvening),
        "Thursday is free to them and has arrival times left"
    );

    // 00:10, hourly grid, 90-minute sittings: Thursday has no arrival left.
    let late = force(BarConfig {
        horizon_days: 1,
        ..grid(1560, 90, 60)
    });
    let after_midnight = utc(2026, 7, 30, 22, 10);
    assert_eq!(late.current_service_day(after_midnight), thursday());
    assert_eq!(
        plan.rebooking_on_offer(std::slice::from_ref(&plan), &late, after_midnight),
        None
    );
}

#[test]
fn an_arrival_time_the_clocks_skip_is_not_one_left() {
    // Sat 28 Mar 2026, Belgrade: Sunday 02:00 and 02:30 never happen, and are grid's last two
    // arrivals. At 01:45 closing less turn still ahead; no arrival is.
    let mut config = grid(1560, 60, 30);
    config.week = WeekSchedule::uniform(DayHours {
        open_minutes: 600,
        close_minutes: 1650,
        closed: false,
    });
    let config = force(config);
    let saturday = ServiceDay::new(NaiveDate::from_ymd_opt(2026, 3, 28).expect("valid date"));

    assert!(has_arrival_after(
        &config,
        saturday,
        utc(2026, 3, 29, 0, 20)
    ));
    assert!(!has_arrival_after(
        &config,
        saturday,
        utc(2026, 3, 29, 0, 45)
    ));
}
