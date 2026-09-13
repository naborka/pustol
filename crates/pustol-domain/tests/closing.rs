//! When shift stops seating, on every clock-change night. Checked minute by minute against wall
//! reading, not against function under test.

mod common;

use chrono::{DateTime, Datelike, TimeDelta, Utc};
use pustol_domain::allocator::{Request, free_tables, walk_in};
use pustol_domain::config::{BarConfig, ValidConfig, schedule_conflicts};
use pustol_domain::schedule::BarTable;
use pustol_domain::service_day::{Interval, ServiceDay, minutes_within, resolve};

use common::{BELGRADE, block, booking, date, force, grid, seated, table, utc};

const MINUTE: TimeDelta = TimeDelta::minutes(1);

/// 20:00Z to 05:00Z next morning: past midnight, past latest shift limits allow.
fn minutes_of_the_night(day: ServiceDay) -> impl Iterator<Item = DateTime<Utc>> {
    let date = day.date();
    let dusk = utc(2026, date.month(), date.day(), 20, 0);
    (0..9 * 60).map(move |minute| dusk + TimeDelta::minutes(minute))
}

fn wall(day: ServiceDay, instant: DateTime<Utc>) -> i32 {
    minutes_within(day, instant, BELGRADE)
}

/// Last minute wall comes up to `close_minutes` from minute before.
fn closing_by_the_wall(day: ServiceDay, close_minutes: i32) -> DateTime<Utc> {
    minutes_of_the_night(day)
        .filter(|now| wall(day, *now - MINUTE) < close_minutes && close_minutes <= wall(day, *now))
        .last()
        .expect("every night comes up to closing")
}

/// Grid arrivals from opening, step apart, to closing less turn, skipped ones dropped.
fn sittings(config: &ValidConfig, day: ServiceDay) -> Vec<Interval> {
    let hours = config.week.for_service_day(day);
    let step = usize::try_from(config.slot_step_minutes).expect("a positive step");
    (hours.open_minutes..=hours.close_minutes - config.turn_minutes)
        .step_by(step)
        .filter_map(|minutes| resolve(day, minutes, BELGRADE).ok())
        .map(|arrives| {
            Interval::from_duration(arrives, config.turn_minutes).expect("a positive turn")
        })
        .collect()
}

#[test]
fn today_walk_ins_their_ends_what_is_finished_and_the_hours_agree_minute_by_minute_on_every_kind_of_night()
 {
    let nights = [
        ("autumn", date(2026, 10, 24)),
        ("spring", date(2026, 3, 28)),
        ("ordinary", date(2026, 7, 30)),
    ];
    for (kind, day) in nights {
        let next = day.checked_add_days(1).expect("in range");
        for close_minutes in (1530..=1680).step_by(15) {
            let closes = closing_by_the_wall(day, close_minutes);
            for turn_minutes in [60, 90, 120, 240] {
                for step in [15, 30, 60] {
                    let config = force(grid(close_minutes, turn_minutes, step));
                    let what = format!(
                        "{kind}, closing {close_minutes}, turn {turn_minutes}, step {step}"
                    );
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
                        assert_eq!(
                            config.current_service_day(now),
                            running,
                            "{what}: today at {now}"
                        );
                        assert_eq!(
                            config.walk_in_window(next, now),
                            None,
                            "{what}: tomorrow at {now}"
                        );

                        let window = config.walk_in_window(day, now);
                        assert_eq!(
                            walk_in(&config, day, now, &[], &[]).map(|offer| offer.window),
                            window,
                            "{what}: the door at {now}"
                        );
                        assert_eq!(window.is_some(), now < closes, "{what}: walk-ins at {now}");
                        assert_eq!(
                            config.is_open(day, now),
                            now < closes,
                            "{what}: open at {now}"
                        );
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
    // Every closing minute, repeated hour's first and last included. Turn and step only move last
    // sitting end, which test above crosses with fewer closings.
    let saturday = date(2026, 10, 24);
    let sunday = date(2026, 10, 25);
    for close_minutes in 1530..=1680 {
        let closes = closing_by_the_wall(saturday, close_minutes);
        let config = force(grid(close_minutes, 60, 60));
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
fn a_walk_in_is_offered_exactly_the_tables_it_would_be_seated_at_minute_by_minute_on_the_autumn_night()
 {
    // Sat 24 Oct 2026 closes 04:00, one-hour sittings. Both two-tops booked at first 02:30 (00:30Z to
    // 01:30Z); four-top shut. At 00:40Z wall first reads 02:40; party seated then overlaps both.
    let saturday = date(2026, 10, 24);
    let config = force(BarConfig {
        tables: vec![table(1, 2, "Бар"), table(2, 2, "Бар"), table(3, 4, "Зал")],
        max_party: 4,
        ..grid(1680, 60, 30)
    });
    let bookings = [
        booking(1, saturday, 1590, 2, Some(&config.tables[0]), 60),
        booking(2, saturday, 1590, 2, Some(&config.tables[1]), 60),
    ];
    let blocks = [block(saturday, &config.tables[2])];
    let numbers =
        |tables: &[&BarTable]| tables.iter().map(|table| table.number).collect::<Vec<_>>();

    let probe = walk_in(
        &config,
        saturday,
        utc(2026, 10, 25, 0, 40),
        &bookings,
        &blocks,
    )
    .expect("open");
    assert_eq!(numbers(&probe.tables), Vec::<i32>::new());
    let after = walk_in(
        &config,
        saturday,
        utc(2026, 10, 25, 1, 30),
        &bookings,
        &blocks,
    )
    .expect("open");
    assert_eq!(numbers(&after.tables), vec![1, 2]);

    for now in minutes_of_the_night(saturday) {
        let offer = walk_in(&config, saturday, now, &bookings, &blocks);
        assert_eq!(
            offer.as_ref().map(|offer| offer.window),
            config.walk_in_window(saturday, now),
            "at {now}"
        );
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
            assert_eq!(
                numbers(&offer.tables_for(party_size)),
                numbers(&seated_at),
                "{party_size} at {now}"
            );
            assert!(
                offer
                    .tables_for(party_size)
                    .iter()
                    .all(|table| offer.tables.contains(table)),
                "{party_size} at {now}"
            );
        }
    }
}

#[test]
fn on_the_night_the_clocks_go_back_a_bar_closing_at_two_closes_the_first_time_the_wall_reads_two() {
    // Sat 24 Oct 2026 closes 02:00, reached from 01:59 at 00:00Z. At 01:00Z wall falls back from 02:59
    // to 02:00: not closing again.
    let config = force(grid(1560, 120, 30));
    let saturday = date(2026, 10, 24);

    assert_eq!(
        config
            .walk_in_window(saturday, utc(2026, 10, 24, 23, 59))
            .map(Interval::end),
        Some(utc(2026, 10, 25, 0, 0))
    );
    assert_eq!(
        config.walk_in_window(saturday, utc(2026, 10, 25, 0, 0)),
        None
    );
    assert_eq!(
        config.walk_in_window(saturday, utc(2026, 10, 25, 0, 30)),
        None
    );
    assert_eq!(
        config.current_service_day(utc(2026, 10, 25, 0, 0)),
        date(2026, 10, 25)
    );
}
