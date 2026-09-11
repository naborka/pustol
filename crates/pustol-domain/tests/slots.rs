//! Every offered time has a real table behind it.

mod common;

use chrono::{NaiveDate, Weekday};
use pustol_domain::config::DayHours;
use pustol_domain::service_day::ServiceDay;
use pustol_domain::slots::{
    PartOfDay, Query, SlotAvailability, first_free_minutes, horizon_days, slot_list,
};
use pustol_domain::{WeekSchedule, bookable_days};

use common::{
    DEFAULT_HOURS, at, block, booking, default_config, force, in_force, numbered, table, thursday,
    utc, zone,
};

/// Early morning on the Thursday, before any of the fixture bookings.
fn morning() -> chrono::DateTime<chrono::Utc> {
    utc(2026, 7, 30, 6, 0)
}

/// A query against an empty room for a couple, at `now`.
macro_rules! couple_at {
    ($config:expr, $day:expr, $now:expr) => {
        Query {
            config: &$config,
            service_day: $day,
            party_size: 2,
            bookings: &[],
            blocks: &[],
            now: $now,
            ignoring: None,
        }
    };
}

#[test]
fn a_day_off_offers_nothing() {
    let mut draft = default_config();
    draft.week = draft.week.with(
        Weekday::Thu,
        DayHours {
            closed: true,
            ..DEFAULT_HOURS
        },
    );
    let config = force(draft);
    assert!(slot_list(&couple_at!(config, thursday(), morning())).is_empty());
}

#[test]
fn slots_run_from_opening_to_closing_minus_one_turn_in_configured_steps() {
    let config = in_force();
    let slots = slot_list(&couple_at!(config, thursday(), morning()));
    let minutes: Vec<i32> = slots.iter().map(|slot| slot.start_minutes).collect();
    assert_eq!(minutes.first(), Some(&600));
    assert_eq!(
        minutes.last(),
        Some(&1440),
        "closing 26:00 less a two hour turn"
    );
    assert_eq!(minutes.len(), 29);
    assert!(minutes.windows(2).all(|pair| pair[1] - pair[0] == 30));
}

#[test]
fn no_offered_booking_runs_past_closing_time() {
    let mut draft = default_config();
    draft.turn_minutes = 90;
    let config = force(draft);
    let slots = slot_list(&couple_at!(config, thursday(), morning()));
    let close = at(thursday(), DEFAULT_HOURS.close_minutes);
    assert!(!slots.is_empty());
    assert!(
        slots
            .iter()
            .filter_map(|slot| slot.window)
            .all(|window| window.end() <= close)
    );
}

#[test]
fn a_free_slot_names_the_table_it_would_use() {
    let config = in_force();
    let slots = slot_list(&couple_at!(config, thursday(), morning()));
    let two_top = numbered(&config.tables, 1);
    assert_eq!(
        slots[0].availability,
        SlotAvailability::Free { table: two_top.id },
        "the smallest table that fits, deterministically"
    );
}

#[test]
fn a_slot_is_taken_once_every_table_that_fits_is_busy() {
    let config = in_force();
    let bookings: Vec<_> = config
        .tables
        .iter()
        .enumerate()
        .map(|(index, table)| {
            booking(
                u128::try_from(index).expect("small"),
                thursday(),
                1200,
                2,
                Some(table),
                config.turn_minutes,
            )
        })
        .collect();
    let slots = slot_list(&Query {
        bookings: &bookings,
        ..couple_at!(config, thursday(), morning())
    });
    let at_eight = slots
        .iter()
        .find(|slot| slot.start_minutes == 1200)
        .expect("20:00 is offered");
    assert_eq!(at_eight.availability, SlotAvailability::Taken);
}

#[test]
fn closing_tables_removes_them_from_the_picker() {
    let config = in_force();
    let blocks: Vec<_> = config
        .tables
        .iter()
        .filter(|table| table.seats == 2)
        .map(|table| block(thursday(), table))
        .collect();
    let slots = slot_list(&Query {
        blocks: &blocks,
        ..couple_at!(config, thursday(), morning())
    });
    // With the bar's two-tops shut, a couple is offered the smallest remaining table.
    let four_top = numbered(&config.tables, 5);
    assert_eq!(
        slots[0].availability,
        SlotAvailability::Free { table: four_top.id }
    );
}

#[test]
fn times_that_have_already_passed_are_never_offered_as_free() {
    let config = in_force();
    // Half past nine in the evening in Belgrade is 19:30 UTC.
    let slots = slot_list(&couple_at!(config, thursday(), utc(2026, 7, 30, 19, 30)));
    let past: Vec<i32> = slots
        .iter()
        .filter(|slot| slot.availability == SlotAvailability::Past)
        .map(|slot| slot.start_minutes)
        .collect();
    assert_eq!(past.first(), Some(&600));
    assert_eq!(past.last(), Some(&1290), "21:30 has just gone");
    assert!(
        slots
            .iter()
            .find(|slot| slot.start_minutes == 1320)
            .is_some_and(|slot| slot.availability.is_free()),
        "22:00 is still ahead"
    );
}

#[test]
fn the_picker_splits_the_daytime_from_the_evening_at_five() {
    let config = in_force();
    let slots = slot_list(&couple_at!(config, thursday(), morning()));
    let last_daytime = slots
        .iter()
        .filter(|slot| slot.part_of_day == PartOfDay::Day)
        .map(|slot| slot.start_minutes)
        .max();
    let first_evening = slots
        .iter()
        .filter(|slot| slot.part_of_day == PartOfDay::Evening)
        .map(|slot| slot.start_minutes)
        .min();
    assert_eq!(last_daytime, Some(990));
    assert_eq!(first_evening, Some(1020));
}

#[test]
fn a_wall_clock_time_the_spring_change_skips_is_not_offered() {
    // A bar shutting at 04:00 with a one hour turn takes arrivals up to 03:00. On the shift that
    // opens the evening before the clocks go forward, 02:00 and 02:30 never happen.
    let mut draft = default_config();
    draft.turn_minutes = 60;
    draft.week = WeekSchedule::uniform(DayHours {
        open_minutes: 600,
        close_minutes: 1680,
        closed: false,
    });
    let config = force(draft);
    let eve_of_change = ServiceDay::new(NaiveDate::from_ymd_opt(2026, 3, 28).expect("valid date"));
    let slots = slot_list(&couple_at!(config, eve_of_change, utc(2026, 3, 28, 0, 0)));
    let availability = |minutes: i32| {
        slots
            .iter()
            .find(|slot| slot.start_minutes == minutes)
            .map(|slot| slot.availability)
    };
    assert!(
        availability(1500).is_some_and(SlotAvailability::is_free),
        "01:00 exists"
    );
    assert_eq!(
        availability(1560),
        Some(SlotAvailability::Nonexistent),
        "02:00 is skipped"
    );
    assert_eq!(
        availability(1590),
        Some(SlotAvailability::Nonexistent),
        "02:30 is skipped"
    );
    assert!(
        availability(1620).is_some_and(SlotAvailability::is_free),
        "03:00 exists"
    );
    assert!(
        !SlotAvailability::Nonexistent.is_offerable(),
        "a time that does not happen is not shown at all"
    );
}

#[test]
fn a_party_larger_than_the_room_finds_every_slot_taken() {
    let config = in_force();
    let slots = slot_list(&Query {
        party_size: config.largest_table_seats() + 1,
        ..couple_at!(config, thursday(), morning())
    });
    assert!(!slots.is_empty());
    assert!(
        slots
            .iter()
            .all(|slot| slot.availability == SlotAvailability::Taken)
    );
}

#[test]
fn a_guest_changing_their_own_booking_still_sees_their_current_time_as_free() {
    let config = in_force();
    // Fill every table that could seat a couple at 20:00, one of them being the guest's own.
    let bookings: Vec<_> = config
        .tables
        .iter()
        .enumerate()
        .map(|(index, table)| {
            booking(
                u128::try_from(index).expect("small"),
                thursday(),
                1200,
                2,
                Some(table),
                config.turn_minutes,
            )
        })
        .collect();
    let mine = bookings[0].id;

    let taken = slot_list(&Query {
        bookings: &bookings,
        ..couple_at!(config, thursday(), morning())
    });
    assert_eq!(
        taken
            .iter()
            .find(|slot| slot.start_minutes == 1200)
            .map(|slot| slot.availability),
        Some(SlotAvailability::Taken)
    );

    let with_mine_ignored = slot_list(&Query {
        bookings: &bookings,
        ignoring: Some(mine),
        ..couple_at!(config, thursday(), morning())
    });
    assert!(
        with_mine_ignored
            .iter()
            .find(|slot| slot.start_minutes == 1200)
            .is_some_and(|slot| slot.availability.is_free()),
        "a guest must be able to keep their own time while changing party size"
    );
}

#[test]
fn the_day_strip_skips_days_off_and_stops_at_the_horizon() {
    let mut draft = default_config();
    draft.horizon_days = 4;
    // Thursday 30 July 2026, so the strip covers Thu, Fri, Sat, Sun.
    draft.week = draft.week.with(
        Weekday::Sat,
        DayHours {
            closed: true,
            ..DEFAULT_HOURS
        },
    );
    let config = force(draft);
    assert_eq!(
        bookable_days(&config, thursday()),
        vec![
            thursday(),
            thursday().checked_add_days(1).unwrap(),
            thursday().checked_add_days(3).unwrap(),
        ],
        "the closed Saturday is left out but still counts against the horizon"
    );
}

#[test]
fn the_day_rail_is_exactly_as_long_as_the_horizon_the_manager_set() {
    // The rail a guest scrolls is the horizon itself, whatever the week does. A closed Saturday
    // is a chip that says `выходной`, not a gap that makes the rail a different length each week.
    for length in [1, 4, 30] {
        let mut draft = default_config();
        draft.horizon_days = length;
        draft.week = draft.week.with(
            Weekday::Sat,
            DayHours {
                closed: true,
                ..DEFAULT_HOURS
            },
        );
        let config = force(draft);
        let rail = horizon_days(&config, thursday());
        assert_eq!(
            i32::try_from(rail.len()).expect("short rail"),
            length,
            "horizon of {length} days"
        );
        assert_eq!(rail.first(), Some(&thursday()));
    }
}

#[test]
fn a_one_day_horizon_reaches_today_and_nothing_else() {
    let mut draft = default_config();
    draft.horizon_days = 1;
    let config = force(draft);
    assert_eq!(horizon_days(&config, thursday()), vec![thursday()]);
    assert_eq!(bookable_days(&config, thursday()), vec![thursday()]);
}

#[test]
fn a_chip_says_the_first_time_that_day_still_has() {
    let config = in_force();
    assert_eq!(
        first_free_minutes(&couple_at!(config, thursday(), morning())),
        Some(DEFAULT_HOURS.open_minutes),
        "an empty room offers its first arrival time"
    );
}

#[test]
fn a_chip_offers_nothing_on_a_day_the_week_schedule_closes() {
    let mut draft = default_config();
    draft.week = draft.week.with(
        Weekday::Thu,
        DayHours {
            closed: true,
            ..DEFAULT_HOURS
        },
    );
    let config = force(draft);
    assert_eq!(
        first_free_minutes(&couple_at!(config, thursday(), morning())),
        None
    );
}

#[test]
fn a_chip_offers_nothing_once_the_party_is_larger_than_the_room() {
    let mut draft = default_config();
    draft.tables = vec![table(1, 2, "Бар")];
    draft.max_party = 2;
    let config = force(draft);
    let query = Query {
        party_size: 4,
        ..couple_at!(config, thursday(), morning())
    };
    assert_eq!(first_free_minutes(&query), None);
}

#[test]
fn a_chip_skips_the_hours_already_sold_and_names_the_first_that_is_not() {
    let mut draft = default_config();
    draft.tables = vec![table(1, 2, "Бар")];
    draft.max_party = 2;
    let config = force(draft);
    let taken = booking(1, thursday(), 600, 2, Some(&config.tables[0]), 120);
    let query = Query {
        bookings: std::slice::from_ref(&taken),
        ..couple_at!(config, thursday(), morning())
    };
    assert_eq!(
        first_free_minutes(&query),
        Some(720),
        "the only table is busy 10:00–12:00, so the first free arrival is 12:00"
    );
}

#[test]
fn slot_generation_follows_each_weekday_rather_than_one_pair_of_hours() {
    let mut draft = default_config();
    draft.week = draft.week.with(
        Weekday::Fri,
        DayHours {
            open_minutes: 1_080,
            close_minutes: 1_560,
            closed: false,
        },
    );
    let config = force(draft);
    let friday = thursday().checked_add_days(1).expect("in range");
    assert_eq!(
        first_free_minutes(&couple_at!(config, friday, morning())),
        Some(1_080),
        "Friday opens at 18:00 even though Thursday opens at 10:00"
    );
    assert_eq!(
        first_free_minutes(&couple_at!(config, thursday(), morning())),
        Some(600)
    );
}

#[test]
fn a_zone_name_is_part_of_the_room_description() {
    assert!(in_force().zones.contains(&zone("Терраса")));
}
