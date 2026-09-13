//! When a shift stops seating, on every kind of night the clocks can make.
//!
//! Each answer is held, minute by minute, to the wall itself: where it reads, one minute after another,
//! rather than to the function that computes it.

mod common;

use chrono::{DateTime, Datelike, NaiveDate, TimeDelta, Utc};
use pustol_domain::allocator::{Booking, BookingStatus, Request, free_tables, walk_in};
use pustol_domain::config::{BarConfig, DayHours, ValidConfig, WeekSchedule, schedule_conflicts};
use pustol_domain::schedule::BarTable;
use pustol_domain::service_day::{Interval, ServiceDay, minutes_within, resolve};

use common::{BELGRADE, block, booking, default_config, force, table, utc};

const MINUTE: TimeDelta = TimeDelta::minutes(1);

fn night(year: i32, month: u32, day: u32) -> ServiceDay {
    ServiceDay::new(NaiveDate::from_ymd_opt(year, month, day).expect("valid date"))
}

/// The fixture bar open 10:00 to `close_minutes` every day, with this turn and time step.
fn bar(close_minutes: i32, turn_minutes: i32, slot_step_minutes: i32) -> ValidConfig {
    force(BarConfig {
        week: WeekSchedule::uniform(DayHours {
            open_minutes: 600,
            close_minutes,
            closed: false,
        }),
        turn_minutes,
        slot_step_minutes,
        ..default_config()
    })
}

/// Every minute from 20:00 UTC on `day` to 05:00 UTC the next morning: past midnight on the wall, and
/// past the latest any shift the limits allow can run.
fn minutes_of_the_night(day: ServiceDay) -> impl Iterator<Item = DateTime<Utc>> {
    let date = day.date();
    let dusk = utc(2026, date.month(), date.day(), 20, 0);
    (0..9 * 60).map(move |minute| dusk + TimeDelta::minutes(minute))
}

/// Where the wall stands at `instant`, in minutes into `day`.
fn wall(day: ServiceDay, instant: DateTime<Utc>) -> i32 {
    minutes_within(day, instant, BELGRADE)
}

/// The last minute of the night at which the wall comes up to `close_minutes` from a minute before it.
fn closing_by_the_wall(day: ServiceDay, close_minutes: i32) -> DateTime<Utc> {
    minutes_of_the_night(day)
        .filter(|now| wall(day, *now - MINUTE) < close_minutes && close_minutes <= wall(day, *now))
        .last()
        .expect("every night comes up to closing")
}

/// Every sitting the grid sells on `day`: each arrival time from opening, a step apart, up to closing
/// less a turn, that the clocks do not skip.
fn sittings(config: &ValidConfig, day: ServiceDay) -> Vec<Interval> {
    let hours = config.week.for_service_day(day);
    let step = usize::try_from(config.slot_step_minutes).expect("a positive step");
    (hours.open_minutes..=hours.close_minutes - config.turn_minutes)
        .step_by(step)
        .filter_map(|minutes| resolve(day, minutes, BELGRADE).ok())
        .map(|arrives| Interval::from_duration(arrives, config.turn_minutes).expect("a positive turn"))
        .collect()
}

fn seated(day: ServiceDay, window: Interval) -> Booking {
    Booking {
        window,
        status: BookingStatus::Arrived,
        ..booking(1, day, 600, 2, None, 60)
    }
}

#[test]
fn today_walk_ins_their_ends_what_is_finished_and_the_hours_agree_minute_by_minute_on_every_kind_of_night()
 {
    let nights = [
        ("autumn", night(2026, 10, 24)),
        ("spring", night(2026, 3, 28)),
        ("ordinary", night(2026, 7, 30)),
    ];
    for (kind, day) in nights {
        let next = day.checked_add_days(1).expect("in range");
        for close_minutes in (1530..=1680).step_by(15) {
            let closes = closing_by_the_wall(day, close_minutes);
            for turn_minutes in [60, 90, 120, 240] {
                for step in [15, 30, 60] {
                    let config = bar(close_minutes, turn_minutes, step);
                    let what = format!("{kind}, closing {close_minutes}, turn {turn_minutes}, step {step}");
                    let grid = sittings(&config, day);
                    let runs_until = grid
                        .iter()
                        .map(|sitting| sitting.end())
                        .fold(closes, DateTime::max);
                    let morning = utc(2026, day.date().month(), day.date().day(), 6, 0);
                    for sitting in &grid {
                        assert_eq!(
                            schedule_conflicts(&config, &[seated(day, *sitting)], morning),
                            Vec::new(),
                            "{what}: the sitting at {}",
                            sitting.start()
                        );
                        assert_eq!(
                            config.current_service_day(sitting.end() - MINUTE),
                            day,
                            "{what}: a sitting is unfinished only while its shift runs"
                        );
                    }

                    for now in minutes_of_the_night(day) {
                        let running = if now < runs_until { day } else { next };
                        assert_eq!(config.current_service_day(now), running, "{what}: today at {now}");
                        assert_eq!(config.walk_in_window(next, now), None, "{what}: tomorrow at {now}");

                        let window = config.walk_in_window(day, now);
                        assert_eq!(
                            walk_in(&config, day, now, &[], &[]).map(|offer| offer.window),
                            window,
                            "{what}: the door at {now}"
                        );
                        assert_eq!(window.is_some(), now < closes, "{what}: walk-ins at {now}");
                        assert_eq!(config.is_open(day, now), now < closes, "{what}: open at {now}");
                        let Some(window) = window else {
                            continue;
                        };
                        let turn_ends = now + TimeDelta::minutes(i64::from(turn_minutes));
                        assert_eq!(
                            (window.start(), window.end()),
                            (now, turn_ends.min(closes)),
                            "{what}: a walk-in at {now}"
                        );
                        assert_eq!(
                            config.current_service_day(window.end() - MINUTE),
                            day,
                            "{what}: a walk-in at {now} is unfinished only while its shift runs"
                        );
                        assert_eq!(
                            schedule_conflicts(&config, &[seated(day, window)], now),
                            Vec::new(),
                            "{what}: a walk-in at {now}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn on_the_autumn_night_every_closing_minute_closes_when_the_wall_last_comes_up_to_it() {
    // Every minute a bar may close at, the repeated hour's first and last among them, read against the
    // wall minute by minute. The turn and step only move when the last sitting ends, which the test
    // above crosses with fewer closings.
    let saturday = night(2026, 10, 24);
    let sunday = night(2026, 10, 25);
    for close_minutes in 1530..=1680 {
        let closes = closing_by_the_wall(saturday, close_minutes);
        let config = bar(close_minutes, 60, 60);
        let runs_until = sittings(&config, saturday)
            .iter()
            .map(|sitting| sitting.end())
            .fold(closes, DateTime::max);
        for now in minutes_of_the_night(saturday) {
            let what = format!("closing {close_minutes} at {now}");
            assert_eq!(config.is_open(saturday, now), now < closes, "{what}");
            assert_eq!(
                config.walk_in_window(saturday, now).map(Interval::end),
                (now < closes).then(|| (now + TimeDelta::hours(1)).min(closes)),
                "{what}"
            );
            let running = if now < runs_until { saturday } else { sunday };
            assert_eq!(config.current_service_day(now), running, "{what}");
        }
    }
}

#[test]
fn a_walk_in_is_offered_exactly_the_tables_it_would_be_seated_at_minute_by_minute_on_the_autumn_night() {
    // Saturday 24 October 2026 closes at 04:00, with one-hour sittings. Both two-tops are booked at the
    // first 02:30, from 00:30Z to 01:30Z, and the four-top is shut for the night. At 00:40Z the wall reads
    // 02:40 for the first time, and a party seated then would sit into both bookings.
    let saturday = night(2026, 10, 24);
    let config = force(BarConfig {
        tables: vec![table(1, 2, "Бар"), table(2, 2, "Бар"), table(3, 4, "Зал")],
        max_party: 4,
        ..(*bar(1680, 60, 30)).clone()
    });
    let bookings = [
        booking(1, saturday, 1590, 2, Some(&config.tables[0]), 60),
        booking(2, saturday, 1590, 2, Some(&config.tables[1]), 60),
    ];
    let blocks = [block(saturday, &config.tables[2])];
    let numbers = |tables: &[&BarTable]| tables.iter().map(|table| table.number).collect::<Vec<_>>();

    let probe = walk_in(&config, saturday, utc(2026, 10, 25, 0, 40), &bookings, &blocks).expect("open");
    assert_eq!(numbers(&probe.tables), Vec::<i32>::new());
    let after = walk_in(&config, saturday, utc(2026, 10, 25, 1, 30), &bookings, &blocks).expect("open");
    assert_eq!(numbers(&after.tables), vec![1, 2]);

    for now in minutes_of_the_night(saturday) {
        let offer = walk_in(&config, saturday, now, &bookings, &blocks);
        assert_eq!(offer.as_ref().map(|offer| offer.window), config.walk_in_window(saturday, now), "at {now}");
        let Some(offer) = offer else {
            continue;
        };
        for party_size in 1..=config.max_party {
            let seated_at = free_tables(&Request {
                party_size,
                window: offer.window,
                service_day: saturday,
                tables: &config.tables,
                bookings: &bookings,
                blocks: &blocks,
                ignoring: &[],
            });
            assert_eq!(numbers(&offer.tables_for(party_size)), numbers(&seated_at), "{party_size} at {now}");
            assert!(
                offer.tables_for(party_size).iter().all(|table| offer.tables.contains(table)),
                "{party_size} at {now}"
            );
        }
    }
}

#[test]
fn on_the_night_the_clocks_go_back_a_bar_closing_at_two_closes_the_first_time_the_wall_reads_two() {
    // Saturday 24 October 2026 closes at 02:00, which the wall reaches from 01:59 at 00:00Z. At 01:00Z it
    // falls back from 02:59 to 02:00, which is not closing again: a walk-in used to be seated at 00:30Z
    // until 01:00Z, arriving at 02:30 and leaving at 02:00.
    let config = bar(1560, 120, 30);
    let saturday = night(2026, 10, 24);

    assert_eq!(config.walk_in_window(saturday, utc(2026, 10, 24, 23, 59)).map(Interval::end), Some(utc(2026, 10, 25, 0, 0)));
    assert_eq!(config.walk_in_window(saturday, utc(2026, 10, 25, 0, 0)), None);
    assert_eq!(config.walk_in_window(saturday, utc(2026, 10, 25, 0, 30)), None);
    assert_eq!(config.current_service_day(utc(2026, 10, 25, 0, 0)), night(2026, 10, 25));
}
