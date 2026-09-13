//! The single predicate for "is this configuration legal", exercised rule by rule.

mod common;

use chrono::Weekday;
use pustol_domain::config::{
    BarConfig, ConfigError, LIMITS, Setting, StaffMember, WeekSchedule, is_telegram_username,
    parties_above_cap, schedule_conflicts,
};
use pustol_domain::service_day::ServiceDay;
use pustol_domain::{DayHours, Interval, ScheduleConflict};

use common::{
    BELGRADE, DEFAULT_HOURS, booking, date, default_config, force, grid, numbered, seated, table,
    thursday, utc, zone,
};

#[test]
fn the_prototypes_own_configuration_is_legal() {
    assert_eq!(default_config().validate(), Vec::new());
}

#[test]
fn a_bar_needs_a_name_and_an_address() {
    let mut config = default_config();
    config.name = "   ".to_owned();
    config.address = String::new();
    let errors = config.validate();
    assert!(errors.contains(&ConfigError::BlankName));
    assert!(errors.contains(&ConfigError::BlankAddress));
}

#[test]
fn opening_and_closing_times_stay_within_the_offered_range() {
    let mut config = default_config();
    config.week = config.week.with(
        Weekday::Fri,
        DayHours {
            open_minutes: LIMITS.open_minutes.min - 1,
            close_minutes: LIMITS.close_minutes.max + 1,
            closed: false,
        },
    );
    let errors = config.validate();
    assert!(errors.contains(&ConfigError::OpenOutOfRange {
        weekday: Weekday::Fri,
        minutes: LIMITS.open_minutes.min - 1,
    }));
    assert!(errors.contains(&ConfigError::CloseOutOfRange {
        weekday: Weekday::Fri,
        minutes: LIMITS.close_minutes.max + 1,
    }));
}

#[test]
fn the_bar_cannot_be_shut_every_day_of_the_week() {
    let mut config = default_config();
    config.week = WeekSchedule::uniform(DayHours {
        closed: true,
        ..DEFAULT_HOURS
    });
    assert!(config.validate().contains(&ConfigError::EveryDayClosed));
}

#[test]
fn a_shift_must_be_long_enough_to_hold_one_booking() {
    let mut config = default_config();
    // A four hour turn will not fit inside a three hour Monday.
    config.turn_minutes = 240;
    config.week = config.week.with(
        Weekday::Mon,
        DayHours {
            open_minutes: 1020,
            close_minutes: 1200,
            closed: false,
        },
    );
    assert!(
        config
            .validate()
            .contains(&ConfigError::ShiftShorterThanTurn {
                weekday: Weekday::Mon,
                shift_minutes: 180,
                turn_minutes: 240,
            })
    );
}

#[test]
fn a_shift_exactly_one_turn_long_is_legal_and_offers_a_single_arrival() {
    let mut config = default_config();
    config.turn_minutes = 240;
    config.week = config.week.with(
        Weekday::Mon,
        DayHours {
            open_minutes: 1020,
            close_minutes: 1260,
            closed: false,
        },
    );
    assert_eq!(config.validate(), Vec::new());
    assert_eq!(force(config).last_arrival_minutes(Weekday::Mon), Some(1020));
}

#[test]
fn a_closed_day_is_not_asked_to_fit_a_turn() {
    let mut config = default_config();
    config.turn_minutes = 240;
    config.week = config.week.with(
        Weekday::Mon,
        DayHours {
            open_minutes: 1020,
            close_minutes: 1200,
            closed: true,
        },
    );
    assert_eq!(config.validate(), Vec::new());
    assert_eq!(force(config).last_arrival_minutes(Weekday::Mon), None);
}

#[test]
fn hours_and_turns_at_the_ends_of_the_integers_are_refused_rather_than_overflowed() {
    // Save body carries any integers; `i32` subtraction would overflow.
    let cases = [
        (600, i32::MIN, 1),
        (i32::MAX, i32::MIN, 120),
        (600, i32::MAX, i32::MIN),
        (i32::MIN, i32::MAX, i32::MAX),
    ];
    for (open_minutes, close_minutes, turn_minutes) in cases {
        let mut config = default_config();
        config.turn_minutes = turn_minutes;
        config.week = config.week.with(
            Weekday::Fri,
            DayHours {
                open_minutes,
                close_minutes,
                closed: false,
            },
        );
        let errors = config.validate();
        assert!(
            errors.contains(&ConfigError::CloseOutOfRange {
                weekday: Weekday::Fri,
                minutes: close_minutes,
            }),
            "{errors:?}"
        );
        assert_eq!(
            errors.iter().any(|error| matches!(
                error,
                ConfigError::SettingOutOfRange {
                    setting: Setting::TurnMinutes,
                    ..
                }
            )),
            !LIMITS.turn_minutes.contains(turn_minutes),
            "{errors:?}"
        );
    }

    let mut config = default_config();
    config.week = config.week.with(
        Weekday::Fri,
        DayHours {
            open_minutes: i32::MAX,
            close_minutes: i32::MIN,
            closed: false,
        },
    );
    assert!(
        config
            .validate()
            .contains(&ConfigError::ShiftShorterThanTurn {
                weekday: Weekday::Fri,
                shift_minutes: i64::from(i32::MIN) - i64::from(i32::MAX),
                turn_minutes: 120,
            }),
        "{:?}",
        config.validate()
    );
}

#[test]
fn every_numeric_rule_is_bounded() {
    for (setting, apply) in numeric_settings() {
        let mut too_low = default_config();
        let mut too_high = default_config();
        let bounds = bounds_of(setting);
        apply(&mut too_low, bounds.min - 1);
        apply(&mut too_high, bounds.max + 1);
        for (config, value) in [(too_low, bounds.min - 1), (too_high, bounds.max + 1)] {
            assert!(
                config.validate().contains(&ConfigError::SettingOutOfRange {
                    setting,
                    value,
                    bounds,
                }),
                "{setting:?} accepted {value}"
            );
        }
    }
}

/// A setting and the way to write it, so the bounds test can walk every one of them.
type SettingWriter = fn(&mut BarConfig, i32);

fn numeric_settings() -> Vec<(Setting, SettingWriter)> {
    vec![
        (Setting::TurnMinutes, |config, value| {
            config.turn_minutes = value;
        }),
        (Setting::MaxParty, |config, value| {
            config.max_party = value;
        }),
        (Setting::HorizonDays, |config, value| {
            config.horizon_days = value;
        }),
        (Setting::RemindHours, |config, value| {
            config.remind_hours = value;
        }),
        (Setting::GraceMinutes, |config, value| {
            config.grace_minutes = value;
        }),
    ]
}

fn bounds_of(setting: Setting) -> pustol_domain::Bounds {
    match setting {
        Setting::TurnMinutes => LIMITS.turn_minutes,
        Setting::MaxParty => LIMITS.max_party,
        Setting::HorizonDays => LIMITS.horizon_days,
        Setting::RemindHours => LIMITS.remind_hours,
        Setting::GraceMinutes => LIMITS.grace_minutes,
    }
}

#[test]
fn only_the_offered_time_steps_are_accepted() {
    let mut config = default_config();
    config.slot_step_minutes = 20;
    assert!(
        config
            .validate()
            .contains(&ConfigError::SlotStepNotOffered { minutes: 20 })
    );
    for step in LIMITS.slot_step_minutes {
        let mut config = default_config();
        config.slot_step_minutes = *step;
        assert_eq!(config.validate(), Vec::new(), "step {step} was refused");
    }
}

#[test]
fn a_party_cap_above_the_largest_table_is_refused() {
    let mut config = default_config();
    config.max_party = 8;
    assert!(
        config
            .validate()
            .contains(&ConfigError::MaxPartyExceedsLargestTable {
                max_party: 8,
                largest_table_seats: 6,
            })
    );
}

#[test]
fn retiring_the_last_large_table_invalidates_a_cap_that_relied_on_it() {
    let mut config = default_config();
    for table in &mut config.tables {
        if table.seats == 6 {
            table.retired = true;
        }
    }
    assert!(
        config
            .validate()
            .contains(&ConfigError::MaxPartyExceedsLargestTable {
                max_party: 6,
                largest_table_seats: 4,
            }),
        "a retired table cannot justify a party cap"
    );
}

#[test]
fn a_room_needs_at_least_one_live_table() {
    let mut config = default_config();
    for table in &mut config.tables {
        table.retired = true;
    }
    assert!(config.validate().contains(&ConfigError::NoTables));
}

#[test]
fn table_numbers_are_unique_even_across_retired_tables() {
    let mut config = default_config();
    let mut duplicate = table(1, 4, "Зал");
    duplicate.retired = true;
    config.tables.push(duplicate);
    assert!(
        config
            .validate()
            .contains(&ConfigError::DuplicateTableNumber { number: 1 })
    );
}

#[test]
fn a_table_seats_a_sensible_number_of_people() {
    let mut config = default_config();
    config.tables[0].seats = LIMITS.seats.max + 1;
    assert!(config.validate().contains(&ConfigError::SeatsOutOfRange {
        number: config.tables[0].number,
        seats: LIMITS.seats.max + 1,
    }));
}

#[test]
fn a_table_must_stand_in_a_zone_the_bar_has() {
    let mut config = default_config();
    config.tables[0].zone = zone("Подвал");
    assert!(config.validate().contains(&ConfigError::UnknownZone {
        number: config.tables[0].number,
        zone: zone("Подвал"),
    }));
}

#[test]
fn zones_are_a_non_empty_list_without_repeats() {
    let mut config = default_config();
    config.zones.push(zone("Зал"));
    assert!(config.validate().contains(&ConfigError::DuplicateZone {
        zone: zone("Зал")
    }));

    let mut config = default_config();
    config.zones.clear();
    let errors = config.validate();
    assert!(errors.contains(&ConfigError::NoZones));
}

#[test]
fn guest_messages_and_cancellation_reasons_are_present_and_not_blank() {
    let mut config = default_config();
    config.message_templates.clear();
    config.cancel_reasons.clear();
    let errors = config.validate();
    assert!(errors.contains(&ConfigError::NoMessageTemplates));
    assert!(errors.contains(&ConfigError::NoCancelReasons));

    let mut config = default_config();
    config.message_templates.push("   ".to_owned());
    config.cancel_reasons.push(String::new());
    let errors = config.validate();
    assert!(errors.contains(&ConfigError::BlankMessageTemplate));
    assert!(errors.contains(&ConfigError::BlankCancelReason));
}

#[test]
fn the_last_admin_cannot_be_removed() {
    let mut config = default_config();
    config.staff.clear();
    assert!(config.validate().contains(&ConfigError::NoStaff));
}

#[test]
fn admin_usernames_are_telegram_shaped_and_listed_once() {
    let mut config = default_config();
    config.staff.push(StaffMember {
        username: "ANNA_MGR".to_owned(),
        telegram_user_id: None,
    });
    assert!(
        config
            .validate()
            .contains(&ConfigError::DuplicateStaffUsername {
                username: "ANNA_MGR".to_owned(),
            }),
        "usernames differing only in case are the same person"
    );

    let mut config = default_config();
    config.staff.push(StaffMember {
        username: "9lives".to_owned(),
        telegram_user_id: None,
    });
    assert!(
        config
            .validate()
            .contains(&ConfigError::MalformedStaffUsername {
                username: "9lives".to_owned(),
            })
    );
}

#[test]
fn telegram_username_shape_follows_the_published_rules() {
    assert!(is_telegram_username("anna_mgr"));
    assert!(is_telegram_username("a1234"));
    assert!(
        !is_telegram_username("anna"),
        "four characters is too short"
    );
    assert!(!is_telegram_username(&"a".repeat(33)), "over thirty two");
    assert!(!is_telegram_username("_anna"), "must start with a letter");
    assert!(!is_telegram_username("anna-mgr"), "hyphen is not allowed");
    assert!(!is_telegram_username("анна_менеджер"), "ascii only");
}

#[test]
fn every_reason_a_configuration_is_illegal_is_reported_at_once() {
    let mut config = default_config();
    config.name = String::new();
    config.staff.clear();
    config.max_party = LIMITS.max_party.max + 1;
    let errors = config.validate();
    assert!(errors.len() >= 3, "expected several errors, got {errors:?}");
    assert!(errors.contains(&ConfigError::BlankName));
    assert!(errors.contains(&ConfigError::NoStaff));
}

// ---- retroactive conflicts -------------------------------------------------------------------

#[test]
fn closing_a_day_that_already_has_bookings_is_a_conflict() {
    let config = default_config();
    let tables = config.tables.clone();
    let evening = booking(
        1,
        thursday(),
        1200,
        2,
        Some(numbered(&tables, 1)),
        config.turn_minutes,
    );
    let closed = {
        let mut next = config.clone();
        next.week = next.week.with(
            Weekday::Thu,
            DayHours {
                closed: true,
                ..DEFAULT_HOURS
            },
        );
        next
    };
    assert_eq!(
        schedule_conflicts(
            &force(closed),
            std::slice::from_ref(&evening),
            utc(2026, 7, 30, 10, 0)
        ),
        vec![ScheduleConflict::DayBecameClosed {
            booking: evening.id,
            service_day: thursday(),
        }]
    );
}

#[test]
fn shortening_the_evening_past_a_live_booking_is_a_conflict() {
    let config = default_config();
    let tables = config.tables.clone();
    // Arrives 23:00, leaves 01:00.
    let late = booking(
        2,
        thursday(),
        1380,
        2,
        Some(numbered(&tables, 1)),
        config.turn_minutes,
    );
    let earlier_close = {
        let mut next = config.clone();
        next.week = next.week.with(
            Weekday::Thu,
            DayHours {
                close_minutes: 1440,
                ..DEFAULT_HOURS
            },
        );
        next
    };
    assert_eq!(
        schedule_conflicts(
            &force(earlier_close),
            std::slice::from_ref(&late),
            utc(2026, 7, 30, 10, 0)
        ),
        vec![ScheduleConflict::OutsideOpeningHours {
            booking: late.id,
            service_day: thursday(),
            start_minutes: 1380,
            end_minutes: 1500,
            open_minutes: 600,
            close_minutes: 1440,
        }]
    );
}

#[test]
fn opening_later_than_a_live_booking_is_a_conflict() {
    let config = default_config();
    let tables = config.tables.clone();
    let lunch = booking(
        3,
        thursday(),
        660,
        2,
        Some(numbered(&tables, 1)),
        config.turn_minutes,
    );
    let later_open = {
        let mut next = config.clone();
        next.week = next.week.with(
            Weekday::Thu,
            DayHours {
                open_minutes: 1020,
                ..DEFAULT_HOURS
            },
        );
        next
    };
    let conflicts = schedule_conflicts(
        &force(later_open),
        std::slice::from_ref(&lunch),
        utc(2026, 7, 30, 6, 0),
    );
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].booking(), lunch.id);
}

#[test]
fn bookings_that_have_already_finished_never_block_a_settings_change() {
    // Without this, one busy Friday would freeze the opening hours for good: history cannot be
    // moved or cancelled, so counting it as a conflict makes the setting unchangeable.
    let config = default_config();
    let tables = config.tables.clone();
    let last_week = booking(
        4,
        ServiceDay::new(chrono::NaiveDate::from_ymd_opt(2026, 7, 23).expect("valid date")),
        1200,
        2,
        Some(numbered(&tables, 1)),
        config.turn_minutes,
    );
    let closed = {
        let mut next = config.clone();
        next.week = next.week.with(
            Weekday::Thu,
            DayHours {
                closed: true,
                ..DEFAULT_HOURS
            },
        );
        next
    };
    assert_eq!(
        schedule_conflicts(
            &force(closed),
            std::slice::from_ref(&last_week),
            utc(2026, 7, 30, 10, 0)
        ),
        Vec::new()
    );
}

#[test]
fn a_cancelled_booking_never_blocks_a_settings_change() {
    let config = default_config();
    let tables = config.tables.clone();
    let mut cancelled = booking(
        5,
        thursday(),
        1200,
        2,
        Some(numbered(&tables, 1)),
        config.turn_minutes,
    );
    cancelled.status = pustol_domain::BookingStatus::Cancelled;
    let closed = {
        let mut next = config.clone();
        next.week = next.week.with(
            Weekday::Thu,
            DayHours {
                closed: true,
                ..DEFAULT_HOURS
            },
        );
        next
    };
    assert_eq!(
        schedule_conflicts(
            &force(closed),
            std::slice::from_ref(&cancelled),
            utc(2026, 7, 30, 10, 0)
        ),
        Vec::new()
    );
}

#[test]
fn a_booking_that_still_fits_the_new_hours_is_not_a_conflict() {
    let config = default_config();
    let tables = config.tables.clone();
    let evening = booking(
        6,
        thursday(),
        1200,
        2,
        Some(numbered(&tables, 1)),
        config.turn_minutes,
    );
    let earlier_close = {
        let mut next = config.clone();
        next.week = next.week.with(
            Weekday::Thu,
            DayHours {
                close_minutes: 1440,
                ..DEFAULT_HOURS
            },
        );
        next
    };
    assert_eq!(
        schedule_conflicts(
            &force(earlier_close),
            std::slice::from_ref(&evening),
            utc(2026, 7, 30, 10, 0)
        ),
        Vec::new()
    );
}

#[test]
fn changing_the_turn_length_cannot_strand_an_existing_booking() {
    // Every booking carries the window it was taken for, so a longer default turn applies to the
    // next guest rather than retroactively extending the ones already seated. There is nothing
    // for the conflict check to find, and no guest is told after the fact that their table is
    // now needed for two hours longer than they agreed to.
    let config = default_config();
    let tables = config.tables.clone();
    let back_to_back = vec![
        booking(
            7,
            thursday(),
            1200,
            2,
            Some(numbered(&tables, 1)),
            config.turn_minutes,
        ),
        booking(
            8,
            thursday(),
            1320,
            2,
            Some(numbered(&tables, 1)),
            config.turn_minutes,
        ),
    ];
    let longer_turn = BarConfig {
        turn_minutes: 240,
        ..config
    };
    assert_eq!(longer_turn.validate(), Vec::new());
    assert_eq!(
        schedule_conflicts(&force(longer_turn), &back_to_back, utc(2026, 7, 30, 10, 0)),
        Vec::new()
    );
}

#[test]
fn lowering_the_party_cap_reports_the_bookings_it_would_have_refused() {
    let config = default_config();
    let tables = config.tables.clone();
    let large = booking(
        9,
        thursday(),
        1200,
        6,
        Some(numbered(&tables, 11)),
        config.turn_minutes,
    );
    let small = booking(
        10,
        thursday(),
        1200,
        2,
        Some(numbered(&tables, 1)),
        config.turn_minutes,
    );
    let lowered = BarConfig {
        max_party: 4,
        ..config
    };
    assert_eq!(
        parties_above_cap(
            &force(lowered),
            &[large.clone(), small],
            utc(2026, 7, 30, 10, 0)
        ),
        vec![large.id]
    );
}

#[test]
fn the_latest_arrival_is_always_closing_time_minus_one_turn() {
    let mut config = default_config();
    config.turn_minutes = 90;
    assert_eq!(
        force(config.clone()).last_arrival_minutes(Weekday::Thu),
        Some(1470)
    );
    config.turn_minutes = 240;
    assert_eq!(force(config).last_arrival_minutes(Weekday::Thu), Some(1320));
}

#[test]
fn the_bar_reports_the_shape_of_its_room() {
    let config = force(default_config());
    assert_eq!(config.largest_table_seats(), 6);
    assert_eq!(config.total_seats(), 4 * 2 + 6 * 4 + 3 * 6 + 4 + 6);
    assert_eq!(config.timezone, BELGRADE);
}

#[test]
fn a_configuration_cannot_be_put_in_force_until_it_is_legal() {
    use pustol_domain::config::ValidConfig;

    let mut draft = default_config();
    draft.staff.clear();
    let errors = ValidConfig::new(draft).expect_err("an admin-less bar must be refused");
    assert!(errors.contains(&ConfigError::NoStaff));

    let legal = default_config();
    let in_force = ValidConfig::new(legal.clone()).expect("the fixture bar is legal");
    assert_eq!(in_force.name, legal.name, "reading through to the proposal");
    assert_eq!(
        in_force.into_inner(),
        legal,
        "and back out again for editing"
    );
}

// ---- which shift is running -------------------------------------------------------------------

#[test]
fn after_midnight_the_running_shift_is_still_yesterdays() {
    // The fixture bar opens at 10:00 and shuts at 02:00. At 01:00 on Friday morning it is still
    // working Thursday's shift, and a guest tapping "tonight" means the evening they are sitting in.
    let config = force(default_config());
    // 01:00 Belgrade on 31 July is 23:00 UTC on 30 July.
    assert_eq!(
        config.current_service_day(utc(2026, 7, 30, 23, 0)),
        thursday()
    );
}

#[test]
fn once_the_bar_has_shut_the_running_shift_is_todays() {
    let config = force(default_config());
    // 03:00 Belgrade on 31 July, an hour after closing.
    assert_eq!(
        config.current_service_day(utc(2026, 7, 31, 1, 0)),
        thursday().checked_add_days(1).unwrap()
    );
}

#[test]
fn before_opening_the_running_shift_is_todays_not_yesterdays() {
    let config = force(default_config());
    // 09:00 Belgrade on 31 July: yesterday is over, today has not begun.
    assert_eq!(
        config.current_service_day(utc(2026, 7, 31, 7, 0)),
        thursday().checked_add_days(1).unwrap()
    );
}

#[test]
fn a_bar_that_shuts_before_midnight_never_borrows_yesterdays_shift() {
    let mut draft = default_config();
    draft.week = WeekSchedule::uniform(DayHours {
        open_minutes: 600,
        close_minutes: 1380,
        closed: false,
    });
    let config = force(draft);
    // 00:30 Belgrade on 31 July, well after an 23:00 close.
    assert_eq!(
        config.current_service_day(utc(2026, 7, 30, 22, 30)),
        thursday().checked_add_days(1).unwrap()
    );
}

#[test]
fn a_day_off_yesterday_cannot_be_the_running_shift() {
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
        config.current_service_day(utc(2026, 7, 30, 23, 0)),
        thursday().checked_add_days(1).unwrap()
    );
}

/// Two-hour sittings, 30-minute step.
fn closing_at(close_minutes: i32) -> pustol_domain::config::ValidConfig {
    force(grid(close_minutes, 120, 30))
}

#[test]
fn on_the_night_the_clocks_go_back_the_shift_runs_until_the_wall_last_reads_closing() {
    // Sat 24 Oct 2026 closes 02:30. Sunday 03:00 clocks go back to 02:00, so 02:30 happens at 00:30Z
    // and 01:30Z. Open until last reading; once Sunday runs, never back to Saturday.
    let config = closing_at(1590);
    let saturday = date(2026, 10, 24);
    let sunday = date(2026, 10, 25);
    let read = [
        (utc(2026, 10, 25, 0, 15), saturday),
        (utc(2026, 10, 25, 0, 31), saturday),
        (utc(2026, 10, 25, 1, 15), saturday),
        (utc(2026, 10, 25, 1, 29), saturday),
        (utc(2026, 10, 25, 1, 30), sunday),
        (utc(2026, 10, 25, 1, 31), sunday),
        (utc(2026, 10, 25, 2, 30), sunday),
    ];
    for (now, expected) in read {
        assert_eq!(config.current_service_day(now), expected, "at {now}");
    }
}

#[test]
fn on_the_night_the_clocks_go_forward_the_shift_runs_until_its_last_sitting_ends() {
    // Sat 28 Mar 2026 closes 03:00; Sunday 02:00 clocks jump to 03:00. Last sitting arrives 01:00
    // (00:00Z), holds table two real hours, to 02:00Z.
    let config = closing_at(1620);
    let saturday = date(2026, 3, 28);
    let read = [
        (utc(2026, 3, 29, 0, 59), saturday),
        (utc(2026, 3, 29, 1, 0), saturday),
        (utc(2026, 3, 29, 1, 59), saturday),
        (utc(2026, 3, 29, 2, 0), date(2026, 3, 29)),
    ];
    for (now, expected) in read {
        assert_eq!(config.current_service_day(now), expected, "at {now}");
    }
}

#[test]
fn the_shift_runs_while_its_last_sitting_holds_its_table_and_until_the_wall_last_reads_closing() {
    // Shift stops at later of last sitting end and last wall reading of closing; later only when
    // clocks go back.
    let minute = chrono::TimeDelta::minutes(1);
    for (close_minutes, day, stops) in [
        (1560, thursday(), utc(2026, 7, 31, 0, 0)),
        (1590, date(2026, 10, 24), utc(2026, 10, 25, 1, 30)),
        (1620, date(2026, 3, 28), utc(2026, 3, 29, 2, 0)),
    ] {
        let config = closing_at(close_minutes);
        let last = booking(
            1,
            day,
            close_minutes - config.turn_minutes,
            2,
            None,
            config.turn_minutes,
        );
        let what = format!("{close_minutes} on {day:?}");
        let next = day.checked_add_days(1).expect("in range");

        assert!(last.window.end() <= stops, "{what}");
        assert!(!last.has_finished(last.window.end() - minute), "{what}");
        assert_eq!(
            config.current_service_day(last.window.end() - minute),
            day,
            "{what}"
        );
        assert_eq!(config.current_service_day(stops - minute), day, "{what}");
        assert_eq!(config.current_service_day(stops), next, "{what}");
    }
}

#[test]
fn on_the_night_the_clocks_skip_the_last_arrival_the_shift_ends_with_the_last_sitting_there_is() {
    // Sat 28 Mar 2026 closes 03:00, one-hour sittings every half hour. Latest allowed arrival 02:00
    // never happens: clocks jump at 01:00Z, wall reads closing at once. Last grid sitting arrives
    // 01:30, over at 01:30Z; shift runs exactly that long.
    let config = force(grid(1620, 60, 30));
    let saturday = date(2026, 3, 28);
    let last = booking(1, saturday, 1530, 2, None, 60);

    assert_eq!(last.window.end(), utc(2026, 3, 29, 1, 30));
    assert_eq!(
        config.current_service_day(utc(2026, 3, 29, 1, 29)),
        saturday
    );
    assert_eq!(
        config.current_service_day(utc(2026, 3, 29, 1, 30)),
        date(2026, 3, 29)
    );
}

#[test]
fn a_party_seated_now_holds_its_table_no_later_than_its_shift_runs() {
    // Thursday closes 02:00, two-hour sittings, stops at 00:00Z. Party seated 01:30 held until then:
    // to 03:30 would run into Friday, whose screen reads only Friday bookings and calls table free.
    let config = force(default_config());
    let friday = thursday().checked_add_days(1).expect("in range");

    let evening = config
        .walk_in_window(thursday(), utc(2026, 7, 30, 18, 0))
        .expect("running");
    assert_eq!(evening.start(), utc(2026, 7, 30, 18, 0));
    assert_eq!(
        evening.end(),
        utc(2026, 7, 30, 20, 0),
        "a whole turn while the shift has one"
    );

    let late = config
        .walk_in_window(thursday(), utc(2026, 7, 30, 23, 30))
        .expect("still running");
    assert_eq!(late.start(), utc(2026, 7, 30, 23, 30));
    assert_eq!(late.end(), utc(2026, 7, 31, 0, 0));

    let after = utc(2026, 7, 31, 0, 30);
    assert_eq!(config.current_service_day(after), friday);
    assert_eq!(
        config.walk_in_window(thursday(), after),
        None,
        "Thursday is over"
    );
    assert_eq!(
        config.walk_in_window(friday, after),
        None,
        "Friday is the running shift, and has not opened"
    );
    let opening = utc(2026, 7, 31, 8, 0);
    assert_eq!(
        config.walk_in_window(friday, opening - chrono::TimeDelta::minutes(1)),
        None
    );
    assert_eq!(
        config.walk_in_window(friday, opening).map(Interval::end),
        Some(utc(2026, 7, 31, 10, 0))
    );
}

#[test]
fn on_the_night_the_clocks_go_forward_a_party_seated_now_holds_its_table_until_the_wall_reads_closing()
 {
    // Sat 28 Mar 2026 closes 03:30, one-hour sittings. Clocks jump 02:00 to 03:00 at 01:00Z; wall
    // reads 03:30 half hour later. Party seated at jump holds table that half hour, within hours.
    let config = force(grid(1650, 60, 30));
    let saturday = date(2026, 3, 28);
    let jump = utc(2026, 3, 29, 1, 0);

    let window = config.walk_in_window(saturday, jump).expect("still open");
    assert_eq!(
        (window.start(), window.end()),
        (jump, utc(2026, 3, 29, 1, 30))
    );
    assert_eq!(
        config.walk_in_window(saturday, utc(2026, 3, 29, 1, 30)),
        None
    );
    assert_eq!(
        schedule_conflicts(&config, &[seated(saturday, window)], jump),
        Vec::new()
    );
}

#[test]
fn nobody_is_seated_now_once_the_wall_has_read_closing_though_the_last_sitting_runs_on() {
    // Sat 28 Mar 2026 closes 03:00, two-hour sittings. Wall reads 03:00 at 01:00Z jump; last sitting
    // (01:00) holds table to 02:00Z. At 01:30Z Saturday still runs; nobody new seated either day.
    let config = closing_at(1620);
    let (saturday, sunday) = (date(2026, 3, 28), date(2026, 3, 29));
    let now = utc(2026, 3, 29, 1, 30);

    assert_eq!(config.current_service_day(now), saturday);
    assert_eq!(config.walk_in_window(saturday, now), None);
    assert_eq!(config.walk_in_window(sunday, now), None);
    assert_eq!(
        config
            .walk_in_window(saturday, utc(2026, 3, 29, 0, 59))
            .map(Interval::end),
        Some(utc(2026, 3, 29, 1, 0))
    );
}

#[test]
fn on_the_night_the_clocks_go_back_a_party_is_seated_until_the_wall_last_reads_closing() {
    // Sat 24 Oct 2026 closes 03:00. At 01:00Z wall goes back from 03:00 to 02:00, reads 03:00 only at
    // 02:00Z. At 01:30Z reads 02:30 second time: Saturday seats until 02:00Z; Sunday not begun.
    let config = closing_at(1620);
    let (saturday, sunday) = (date(2026, 10, 24), date(2026, 10, 25));
    let second_pass = utc(2026, 10, 25, 1, 30);

    assert_eq!(config.current_service_day(second_pass), saturday);
    assert_eq!(
        config
            .walk_in_window(saturday, second_pass)
            .map(Interval::end),
        Some(utc(2026, 10, 25, 2, 0))
    );
    assert_eq!(config.walk_in_window(sunday, second_pass), None);

    // Seated on first pass through repeated hour: holds table longer than wall says, still within
    // hours.
    let first_pass = utc(2026, 10, 25, 0, 30);
    let early = config
        .walk_in_window(saturday, first_pass)
        .expect("still open");
    assert_eq!(early.end(), utc(2026, 10, 25, 2, 0));
    assert_eq!(
        schedule_conflicts(&config, &[seated(saturday, early)], first_pass),
        Vec::new()
    );
}

#[test]
fn nobody_is_seated_now_on_a_day_off() {
    let mut draft = default_config();
    draft.week = draft.week.with(
        Weekday::Thu,
        DayHours {
            closed: true,
            ..DEFAULT_HOURS
        },
    );
    assert_eq!(
        force(draft).walk_in_window(thursday(), utc(2026, 7, 30, 18, 0)),
        None
    );
}

#[test]
fn the_last_arrival_on_the_spring_clock_change_is_not_a_conflict_with_the_hours_that_sold_it() {
    // Sat 28 Mar 2026: Sunday 02:00 clocks jump to 03:00. Two-hour booking at 01:00 ends 04:00 on
    // wall, past 03:00 closing, yet grid offered 01:00 as last arrival. Conflict here would refuse
    // every save that week.
    let saturday = ServiceDay::new(chrono::NaiveDate::from_ymd_opt(2026, 3, 28).expect("valid"));
    let mut config = common::default_config();
    config.week = WeekSchedule::uniform(DayHours {
        open_minutes: 600,
        close_minutes: 1620,
        closed: false,
    });
    let last = booking(1, saturday, 1500, 2, Some(table(1, 2, "Бар")).as_ref(), 120);
    assert_eq!(
        schedule_conflicts(
            &force(config),
            std::slice::from_ref(&last),
            utc(2026, 3, 28, 10, 0)
        ),
        Vec::new()
    );
}

#[test]
fn texts_the_bar_writes_stay_short_enough_to_send_and_to_show() {
    // Template longer than Telegram carries fails every send; huge bar name breaks every screen.
    let mut config = default_config();
    config.name = "б".repeat(LIMITS.text.name + 1);
    config.address = "в".repeat(LIMITS.text.address + 1);
    config.message_templates = vec!["а".repeat(LIMITS.text.message + 1)];
    config.cancel_reasons = vec!["г".repeat(LIMITS.text.reason + 1)];
    let errors = config.validate();
    assert!(errors.contains(&ConfigError::NameTooLong {
        limit: LIMITS.text.name
    }));
    assert!(errors.contains(&ConfigError::AddressTooLong {
        limit: LIMITS.text.address
    }));
    assert!(errors.contains(&ConfigError::MessageTemplateTooLong {
        limit: LIMITS.text.message
    }));
    assert!(errors.contains(&ConfigError::CancelReasonTooLong {
        limit: LIMITS.text.reason
    }));

    let mut at_the_limit = default_config();
    at_the_limit.name = "🍺".repeat(LIMITS.text.name);
    at_the_limit.message_templates = vec!["а".repeat(LIMITS.text.message)];
    assert_eq!(
        at_the_limit.validate(),
        Vec::new(),
        "counted in characters, not bytes"
    );
}

#[test]
fn every_list_the_bar_keeps_is_bounded_and_retired_tables_do_not_count() {
    use pustol_domain::config::BarList;
    use pustol_domain::schedule::BarTable;

    let lists = LIMITS.lists;
    let number = |n: usize| i32::try_from(n).expect("a small table number");
    let mut over = default_config();
    over.message_templates = vec!["Ждём вас".to_owned(); lists.message_templates + 1];
    over.cancel_reasons = vec!["Дождь".to_owned(); lists.cancel_reasons + 1];
    over.zones
        .extend((over.zones.len()..=lists.zones).map(|n| zone(&format!("Зона {n}"))));
    over.staff = (0..=lists.staff)
        .map(|n| StaffMember {
            username: format!("staff_{n:03}"),
            telegram_user_id: None,
        })
        .collect();
    over.tables = (1..=lists.tables + 1)
        .map(|n| table(number(n), 6, "Зал"))
        .collect();

    let errors = over.validate();
    for (list, limit) in [
        (BarList::MessageTemplates, lists.message_templates),
        (BarList::CancelReasons, lists.cancel_reasons),
        (BarList::Zones, lists.zones),
        (BarList::Staff, lists.staff),
        (BarList::Tables, lists.tables),
    ] {
        assert!(
            errors.contains(&ConfigError::ListTooLong { list, limit }),
            "{list:?}: {errors:?}"
        );
    }
    assert_eq!(errors.len(), 5, "{errors:?}");

    let mut at_the_bounds = over;
    at_the_bounds.message_templates.pop();
    at_the_bounds.cancel_reasons.pop();
    at_the_bounds.zones.pop();
    at_the_bounds.staff.pop();
    at_the_bounds.tables.pop();
    at_the_bounds
        .tables
        .extend((lists.tables + 2..lists.tables + 7).map(|n| BarTable {
            retired: true,
            ..table(number(n), 6, "Зал")
        }));
    assert_eq!(
        at_the_bounds.validate(),
        Vec::new(),
        "retired tables are the room's history, not its size"
    );
}

#[test]
fn a_zone_name_stays_short_enough_to_show() {
    let mut config = default_config();
    config.zones.push(zone(&"🍺".repeat(LIMITS.text.zone + 1)));
    assert!(config.validate().contains(&ConfigError::ZoneNameTooLong {
        limit: LIMITS.text.zone
    }));
    config.zones.pop();
    config.zones.push(zone(&"🍺".repeat(LIMITS.text.zone)));
    assert_eq!(
        config.validate(),
        Vec::new(),
        "counted in characters, not bytes"
    );
}

#[test]
fn a_contact_is_a_phone_number_or_a_telegram_username_and_nothing_else() {
    use pustol_domain::config::Contact;

    let phone = Contact::parse(" +381 (11) 123-45-67 ").expect("a phone number");
    assert_eq!(phone.label(), "+381 (11) 123-45-67");
    assert_eq!(phone.url(), "tel:+381111234567");

    let account = Contact::parse("@podval_bar").expect("a username");
    assert_eq!(account.label(), "@podval_bar");
    assert_eq!(account.url(), "https://t.me/podval_bar");
    assert_eq!(
        Contact::parse("podval_bar"),
        Some(account),
        "the @ is optional"
    );

    for nonsense in [
        "",
        "позвоните",
        "12",
        "+1+2345678",
        "https://evil.example",
        "@ab",
    ] {
        assert_eq!(Contact::parse(nonsense), None, "{nonsense:?}");
    }
}

#[test]
fn a_contact_username_is_bounded_by_telegram_and_a_phone_by_one_line() {
    use pustol_domain::config::Contact;

    let longest = format!("a{}", "b".repeat(31));
    assert_eq!(
        Contact::parse(&format!("@{longest}")),
        Some(Contact::Telegram { username: longest }),
        "the @ is not part of the username"
    );

    let phone = |length: usize| format!("+381{}111234567", " ".repeat(length - 13));
    assert!(Contact::parse(&phone(32)).is_some());
    assert_eq!(Contact::parse(&phone(33)), None);
}

#[test]
fn a_bar_with_an_unreadable_contact_is_not_legal_and_one_without_a_contact_is() {
    let mut config = default_config();
    config.contact = Some("звоните в дверь".to_owned());
    assert!(config.validate().contains(&ConfigError::MalformedContact));
    config.contact = None;
    assert_eq!(config.validate(), Vec::new());
}
