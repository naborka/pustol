//! What a guest's screen may offer to do with a booking they hold.

mod common;

use chrono::{NaiveDate, TimeDelta};
use pustol_domain::allocator::{Booking, BookingStatus};
use pustol_domain::config::{BarConfig, DayHours, WeekSchedule};
use pustol_domain::rebooking::Rebooking;
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
    assert_eq!(absent.rebooking_on_offer(&config, now), None);
}

#[test]
fn a_held_no_show_is_offered_a_move_while_its_evening_still_has_an_arrival_time() {
    let config = force(grid(90, 30));
    let absent = held_no_show(90);
    let now = utc(2026, 7, 30, 22, 10);

    assert!(has_arrival_after(&config, thursday(), now), "00:30 is still ahead");
    assert_eq!(
        absent.rebooking_on_offer(&config, now),
        Some(Rebooking::SameEvening)
    );
}

#[test]
fn a_plan_is_offered_a_move_to_any_evening_whatever_its_own_evening_has_left() {
    let config = force(grid(90, 60));
    let plan = booking(2, thursday(), 1440, 2, None, 90);
    let now = utc(2026, 7, 30, 21, 59);

    assert_eq!(
        plan.rebooking_on_offer(&config, now),
        Some(Rebooking::AnyEvening)
    );
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
