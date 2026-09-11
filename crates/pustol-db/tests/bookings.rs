//! Taking bookings against a real `PostgreSQL`, including the race the whole design exists to lose
//! safely.

mod common;

use pustol_db::bookings::Attendance;
use pustol_db::{BookingSource, Error};
use pustol_domain::{BookingStatus, SlotAvailability, TableId};
use uuid::Uuid;

use common::{at, config_with, default_bar, default_config, fresh_account, guest_booking, morning, staff_booking, store, table, thursday, utc};

#[tokio::test]
async fn a_bar_round_trips_through_storage_unchanged() {
    let store = store().await;
    let written = default_config();
    let (bar, _) = common::bar_with(&store, written.clone()).await;
    let loaded = store.config(bar).await.expect("loads");
    assert_eq!(*loaded, written);
}

#[tokio::test]
async fn a_guest_is_seated_at_the_smallest_table_that_fits() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let account = fresh_account("Алексей");
    store
        .identify(bar, &account, morning())
        .await
        .expect("identified");

    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("a table is free");
    assert_eq!(created.record.table_number, Some(1));
    assert_eq!(created.record.source, BookingSource::App);
    assert_eq!(created.record.booking.status, BookingStatus::Confirmed);
    assert_eq!(created.replaced, None);
    assert_eq!(
        created.record.booking.window.minutes(),
        i64::from(config.turn_minutes)
    );
}

#[tokio::test]
async fn a_booking_takes_its_slot_out_of_the_picker() {
    let store = store().await;
    // One two-top, so the only slot for a couple at 20:00 disappears once it is taken.
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let account = fresh_account("Вера");
    store.identify(bar, &account, morning()).await.expect("ok");

    store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    let slots = store
        .availability(bar, thursday(), 2, morning(), None)
        .await
        .expect("reads").slots;
    let at_eight = slots
        .iter()
        .find(|slot| slot.start_minutes == 1200)
        .expect("20:00 is offered");
    assert_eq!(at_eight.availability, SlotAvailability::Taken);

    // The turn is two hours, so everything that would overlap 20:00–22:00 is gone with it: the
    // three half-hours before and the three after.
    let taken: Vec<i32> = slots
        .iter()
        .filter(|slot| slot.availability == SlotAvailability::Taken)
        .map(|slot| slot.start_minutes)
        .collect();
    assert_eq!(taken, vec![1110, 1140, 1170, 1200, 1230, 1260, 1290]);

    // And the turns that merely touch it are still sellable, which is the whole point of a
    // half-open window: the party leaving at 22:00 does not cost the bar the 22:00 sitting.
    for edge in [1080, 1320] {
        assert!(
            slots
                .iter()
                .find(|slot| slot.start_minutes == edge)
                .is_some_and(|slot| slot.availability.is_free()),
            "the sitting at {edge} minutes only touches the booking"
        );
    }
}

#[tokio::test]
async fn a_time_that_no_longer_has_a_table_is_refused_by_name() {
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let first = fresh_account("Вера");
    let second = fresh_account("Марина");
    store.identify(bar, &first, morning()).await.expect("ok");
    store.identify(bar, &second, morning()).await.expect("ok");

    store
        .create_booking(&guest_booking(bar, &first, 1200, 2), morning())
        .await
        .expect("free");
    let refused = store
        .create_booking(&guest_booking(bar, &second, 1200, 2), morning())
        .await
        .expect_err("the only table is taken");
    assert!(
        matches!(refused, Error::NoTableFree { party_size: 2 }),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn a_time_that_has_gone_is_refused_as_past_rather_than_as_unavailable() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Егор");
    store.identify(bar, &account, morning()).await.expect("ok");

    // 22:00 Belgrade, asking for a table at 20:00.
    let late = utc(2026, 7, 30, 20, 0);
    let refused = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), late)
        .await
        .expect_err("the evening has moved on");
    assert!(matches!(refused, Error::InThePast), "got {refused:?}");
}

#[tokio::test]
async fn a_time_off_the_configured_step_is_not_an_arrival_time_at_all() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Настя");
    store.identify(bar, &account, morning()).await.expect("ok");

    let refused = store
        .create_booking(&guest_booking(bar, &account, 1205, 2), morning())
        .await
        .expect_err("20:05 is not on a half-hour step");
    assert!(
        matches!(refused, Error::NotAnArrivalTime { minutes: 1205 }),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn an_arrival_after_the_last_one_the_shift_allows_is_refused() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Глеб");
    store.identify(bar, &account, morning()).await.expect("ok");

    // Closing is 02:00 and a turn is two hours, so the last arrival is midnight.
    let refused = store
        .create_booking(&guest_booking(bar, &account, 1470, 2), morning())
        .await
        .expect_err("00:30 leaves only ninety minutes before closing");
    assert!(
        matches!(refused, Error::NotAnArrivalTime { minutes: 1470 }),
        "got {refused:?}"
    );
    store
        .create_booking(&guest_booking(bar, &account, 1440, 2), morning())
        .await
        .expect("midnight exactly is allowed");
}

#[tokio::test]
async fn a_party_above_the_bars_limit_is_refused_with_the_limit_named() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Артур");
    store.identify(bar, &account, morning()).await.expect("ok");

    let refused = store
        .create_booking(&guest_booking(bar, &account, 1200, 8), morning())
        .await
        .expect_err("above the cap");
    assert!(
        matches!(
            refused,
            Error::PartyTooLarge {
                party_size: 8,
                max_party: 6
            }
        ),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn a_guest_cannot_book_beyond_the_horizon_but_staff_can() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Мила");
    store.identify(bar, &account, morning()).await.expect("ok");

    // The horizon is four days: Thursday through Sunday.
    let next_week = thursday().checked_add_days(9).expect("in range");
    let mut request = guest_booking(bar, &account, 1200, 2);
    request.service_day = next_week;

    let refused = store
        .create_booking(&request, morning())
        .await
        .expect_err("outside the horizon");
    assert!(
        matches!(refused, Error::ShiftNotBookable { .. }),
        "got {refused:?}"
    );

    // A bar takes a telephone booking for next month without arguing about it.
    let mut by_staff = staff_booking(bar, "Мила", 1200, 2);
    by_staff.service_day = next_week;
    let created = store
        .create_booking(&by_staff, morning())
        .await
        .expect("staff are not bound by the horizon");
    assert_eq!(created.record.source, BookingSource::Staff);
    assert!(
        !created.record.has_telegram_account(),
        "a staff-entered guest has no account, so the bot has no chat with them"
    );
}

#[tokio::test]
async fn booking_again_replaces_the_booking_the_guest_had_not_yet_started() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Лиза");
    store.identify(bar, &account, morning()).await.expect("ok");

    let first = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    let second = store
        .create_booking(&guest_booking(bar, &account, 1320, 4), morning())
        .await
        .expect("free");

    assert_eq!(second.replaced, Some(first.record.booking.id));
    let mine = store
        .booking_of_guest(bar, account.id, morning())
        .await
        .expect("reads")
        .expect("one booking");
    assert_eq!(mine.booking.id, second.record.booking.id);
    assert_eq!(mine.booking.party_size, 4);

    // The released table is available again.
    let slots = store
        .availability(bar, thursday(), 2, morning(), None)
        .await
        .expect("reads").slots;
    assert!(
        slots
            .iter()
            .find(|slot| slot.start_minutes == 1200)
            .is_some_and(|slot| slot.availability.is_free())
    );
}

#[tokio::test]
async fn booking_again_does_not_take_the_table_from_a_guest_already_sitting_at_it() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Тимур");
    store.identify(bar, &account, morning()).await.expect("ok");

    let seated = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    store
        .set_attendance(bar, seated.record.booking.id, Attendance::Arrived, morning())
        .await
        .expect("they turned up");

    // Half past eight in the evening: they are at the table. A booking for another night must not
    // silently cancel the seating in progress.
    let evening = utc(2026, 7, 30, 18, 30);
    let mut tomorrow = guest_booking(bar, &account, 1200, 2);
    tomorrow.service_day = thursday().checked_add_days(1).expect("in range");
    let created = store
        .create_booking(&tomorrow, evening)
        .await
        .expect("a different shift");
    assert_eq!(
        created.replaced, None,
        "a seating in progress is never cancelled by a later booking"
    );

    let still_seated = store
        .shift(bar, thursday())
        .await
        .expect("reads")
        .bookings
        .into_iter()
        .find(|record| record.booking.id == seated.record.booking.id)
        .expect("still on the shift");
    assert_eq!(still_seated.booking.status, BookingStatus::Arrived);
}

/// The race the exclusion constraint and the advisory lock exist for.
#[tokio::test]
async fn only_one_of_many_guests_racing_for_the_last_table_gets_it() {
    let store = store().await;
    // Exactly one table that can seat a couple.
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;

    let mut accounts = Vec::new();
    for index in 0..8 {
        let account = fresh_account(&format!("Гость {index}"));
        store.identify(bar, &account, morning()).await.expect("ok");
        accounts.push(account);
    }

    let attempts = accounts.into_iter().map(|account| {
        let store = store.clone();
        tokio::spawn(async move {
            store
                .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
                .await
        })
    });
    let outcomes = futures_lite(attempts).await;

    let winners: Vec<_> = outcomes.iter().filter(|outcome| outcome.is_ok()).collect();
    assert_eq!(winners.len(), 1, "exactly one guest gets the table");
    for outcome in &outcomes {
        if let Err(error) = outcome {
            assert!(
                matches!(
                    error,
                    Error::NoTableFree { .. } | Error::TableTakenConcurrently
                ),
                "a loser must be told the table is gone, got {error:?}"
            );
        }
    }

    let live = store.shift(bar, thursday()).await.expect("reads").bookings;
    assert_eq!(live.len(), 1, "and the room holds exactly one booking");
}

/// Awaits a set of spawned tasks, in order, without pulling in a futures crate for one test.
async fn futures_lite<T>(
    handles: impl IntoIterator<Item = tokio::task::JoinHandle<T>>,
) -> Vec<T> {
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.expect("task did not panic"));
    }
    results
}

#[tokio::test]
async fn a_no_show_holds_its_table_through_the_grace_period_and_no_longer() {
    // Attendance still cannot cancel — `Attendance` has no cancelled variant, so that stays a
    // compile-time guarantee. What it does now is release the table at the moment the bar stopped
    // holding it, which is the end of the grace period and not the moment a bartender got round
    // to pressing the button.
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let account = fresh_account("Марина");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    let absent = store
        .set_attendance(bar, created.record.booking.id, Attendance::NoShow, morning())
        .await
        .expect("recorded");
    assert_eq!(absent.record.booking.status, BookingStatus::NoShow);
    assert_eq!(
        absent.record.booking.released_at,
        Some(at(thursday(), 1215)),
        "the fifteen minute grace period, counted from the booking rather than from the tap"
    );

    let slots = store
        .availability(bar, thursday(), 2, morning(), None)
        .await
        .expect("reads")
        .slots;
    let availability_at = |minutes: i32| {
        slots
            .iter()
            .find(|slot| slot.start_minutes == minutes)
            .map(|slot| slot.availability)
    };
    assert_eq!(
        availability_at(1200),
        Some(SlotAvailability::Taken),
        "the quarter hour the bar was still holding the table is not for sale"
    );
    assert!(
        availability_at(1230).is_some_and(SlotAvailability::is_free),
        "and from then on the table is back in the pool"
    );
}

#[tokio::test]
async fn a_party_that_leaves_gives_its_table_back_from_that_minute() {
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let account = fresh_account("Саша");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    // Twenty past nine local: an hour and twenty minutes into a two hour turn.
    let left_at = at(thursday(), 1280);
    let gone = store
        .set_attendance(bar, created.record.booking.id, Attendance::Left, left_at)
        .await
        .expect("recorded");
    assert_eq!(gone.record.booking.status, BookingStatus::Left);
    assert_eq!(gone.record.booking.released_at, Some(left_at));
    assert_eq!(
        gone.record.booking.occupancy().map(pustol_domain::Interval::end),
        Some(left_at),
        "the one occupancy rule: the table is theirs up to the minute they left"
    );

    let free_from = |minutes: i32| {
        let store = &store;
        async move {
            store
                .availability(bar, thursday(), 2, at(thursday(), 600), None)
                .await
                .expect("reads")
                .slots
                .iter()
                .find(|slot| slot.start_minutes == minutes)
                .map(|slot| slot.availability)
        }
    };
    assert!(
        free_from(1290).await.is_some_and(SlotAvailability::is_free),
        "the table a guest can see standing empty is offered to the next guest"
    );
    assert_eq!(free_from(1230).await, Some(SlotAvailability::Taken));
}

#[tokio::test]
async fn undoing_a_departure_puts_the_table_back_exactly_as_it_was() {
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let account = fresh_account("Глеб");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    let held = created.record.booking.occupancy();

    let left_at = at(thursday(), 1280);
    store
        .set_attendance(bar, created.record.booking.id, Attendance::Left, left_at)
        .await
        .expect("recorded");
    let back = store
        .set_attendance(bar, created.record.booking.id, Attendance::Arrived, left_at)
        .await
        .expect("undone");

    assert_eq!(back.record.booking.status, BookingStatus::Arrived);
    assert_eq!(back.record.booking.released_at, None);
    assert_eq!(
        back.record.booking.occupancy(),
        held,
        "undo restores the range the booking held, not something that merely resembles it"
    );
}

#[tokio::test]
async fn a_table_given_back_early_seats_a_party_that_had_none() {
    // The room fixes itself: closing the only table that fits leaves a party stranded, and the
    // next table to come free is offered to them without anybody pressing anything.
    let store = store().await;
    let (bar, config) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let first = fresh_account("Аня");
    let second = fresh_account("Борис");
    for account in [&first, &second] {
        store.identify(bar, account, morning()).await.expect("ok");
    }
    // Eight o'clock at the first table, nine at the second: the second party cannot move to the
    // first table while the first party is still sitting at it.
    let sitting = store
        .create_booking(&guest_booking(bar, &first, 1200, 2), morning())
        .await
        .expect("free");
    let stranded = store
        .create_booking(&guest_booking(bar, &second, 1260, 2), morning())
        .await
        .expect("the other table");

    // Close the second table: its party has nowhere to go, because the first table is taken.
    let closed = store
        .block_tables(
            bar,
            thursday(),
            &[config.tables[1].id],
            "Дождь",
            None,
            morning(),
        )
        .await
        .expect("closes");
    assert_eq!(closed.outcome.orphaned, vec![stranded.record.booking.id]);

    // The party at the first table leaves early. Nobody asks for a reconciliation.
    let left_at = at(thursday(), 1260);
    let recorded = store
        .set_attendance(bar, sitting.record.booking.id, Attendance::Left, left_at)
        .await
        .expect("recorded");
    assert_eq!(
        recorded
            .reconciliation
            .outcome
            .moved
            .iter()
            .map(|moved| moved.booking)
            .collect::<Vec<_>>(),
        vec![stranded.record.booking.id],
        "the table they gave back is offered straight to whoever was owed one"
    );
}

#[tokio::test]
async fn a_walk_in_takes_the_table_the_shift_said_would_fit() {
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар"), table(2, 6, "Зал")], "anna_mgr"),
    )
    .await;
    // Half past eight on the Thursday evening: the shift is running.
    let evening = at(thursday(), 1230);
    let seated = store
        .seat_walk_in(bar, thursday(), 2, None, evening)
        .await
        .expect("a table fits");

    assert_eq!(seated.record.table_number, Some(1), "smallest that fits");
    assert_eq!(seated.record.source, BookingSource::Walk);
    assert_eq!(seated.record.booking.status, BookingStatus::Arrived);
    assert_eq!(seated.record.guest_name, "Без брони");
    assert_eq!(
        seated.record.booking.window.start(),
        evening,
        "seated at the minute they sat down, not at the nearest slot"
    );
    assert_eq!(seated.record.booking.window.minutes(), 120);
}

#[tokio::test]
async fn a_walk_in_sits_where_staff_put_them_rather_than_where_the_room_would() {
    let store = store().await;
    let six_top = table(2, 6, "Зал");
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар"), six_top.clone()], "anna_mgr"),
    )
    .await;
    let evening = at(thursday(), 1230);

    let seated = store
        .seat_walk_in(bar, thursday(), 2, Some(six_top.id), evening)
        .await
        .expect("the six top is free");

    assert_eq!(
        seated.record.table_number,
        Some(2),
        "a couple at the six top because somebody looked at the room and decided so"
    );
}

#[tokio::test]
async fn a_table_staff_cannot_have_is_refused_by_its_own_name() {
    let store = store().await;
    let two_top = table(1, 2, "Бар");
    let closed = table(2, 4, "Зал");
    let taken = table(3, 4, "Зал");
    let (bar, _) = common::bar_with(
        &store,
        config_with(
            vec![two_top.clone(), closed.clone(), taken.clone()],
            "anna_mgr",
        ),
    )
    .await;
    let evening = at(thursday(), 1230);
    store
        .block_tables(bar, thursday(), &[closed.id], "Дождь", None, evening)
        .await
        .expect("closes");
    store
        .seat_walk_in(bar, thursday(), 4, Some(taken.id), evening)
        .await
        .expect("free until now");

    for (table_id, why) in [
        (two_top.id, "too small for four"),
        (closed.id, "closed for the evening"),
        (taken.id, "somebody is sitting there"),
        (TableId(Uuid::nil()), "not a table this bar has"),
    ] {
        let refused = store
            .seat_walk_in(bar, thursday(), 4, Some(table_id), evening)
            .await;
        assert!(
            matches!(refused, Err(Error::ChosenTableNotFree)),
            "{why}, got {refused:?}"
        );
    }
}

#[tokio::test]
async fn a_walk_in_is_refused_on_a_shift_that_is_not_running() {
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let outcome = store
        .seat_walk_in(
            bar,
            thursday().checked_add_days(2).expect("in range"),
            2,
            None,
            at(thursday(), 1230),
        )
        .await;
    assert!(
        matches!(outcome, Err(Error::NotTheRunningShift { .. })),
        "there is no now on next Saturday, got {outcome:?}"
    );
}

/// The same race as the booking one, run at the door: eight bartenders, one table.
#[tokio::test]
async fn only_one_of_many_walk_ins_racing_for_the_last_table_gets_it() {
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let evening = at(thursday(), 1230);

    let attempts = (0..8).map(|_| {
        let store = store.clone();
        tokio::spawn(async move { store.seat_walk_in(bar, thursday(), 2, None, evening).await })
    });
    let outcomes = futures_lite(attempts).await;

    let winners: Vec<_> = outcomes.iter().filter(|outcome| outcome.is_ok()).collect();
    assert_eq!(winners.len(), 1, "exactly one party gets the table");
    for outcome in &outcomes {
        if let Err(error) = outcome {
            assert!(
                matches!(
                    error,
                    Error::NoTableFree { .. } | Error::TableTakenConcurrently
                ),
                "a loser must be told the table is gone, got {error:?}"
            );
        }
    }
    let live = store.shift(bar, thursday()).await.expect("reads").bookings;
    assert_eq!(live.len(), 1);
}

#[tokio::test]
async fn a_note_is_written_rubbed_out_and_never_longer_than_a_row() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Тимур");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    let id = created.record.booking.id;
    assert_eq!(created.record.note, None);

    let noted = store
        .set_note(bar, id, Some("  День рождения  "))
        .await
        .expect("written");
    assert_eq!(noted.note.as_deref(), Some("День рождения"));

    let blanked = store.set_note(bar, id, Some("   ")).await.expect("rubbed out");
    assert_eq!(blanked.note, None, "whitespace is not a note");

    let refused = store.set_note(bar, id, Some(&"я".repeat(121))).await;
    assert!(
        matches!(refused, Err(Error::NoteTooLong { .. })),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn cancelling_frees_the_table_and_records_the_reason_given() {
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let account = fresh_account("Полина");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    let cancelled = store
        .cancel_booking(
            bar,
            created.record.booking.id,
            Some("Технические проблемы в баре"),
            Some(common::cancellation_wording),
            morning(),
        )
        .await
        .expect("cancelled");
    assert_eq!(cancelled.record.booking.status, BookingStatus::Cancelled);
    assert_eq!(
        cancelled.record.cancel_reason.as_deref(),
        Some("Технические проблемы в баре")
    );
    assert!(cancelled.reconciliation.is_empty(), "nobody was waiting");

    let slots = store
        .availability(bar, thursday(), 2, morning(), None)
        .await
        .expect("reads").slots;
    assert!(
        slots
            .iter()
            .find(|slot| slot.start_minutes == 1200)
            .is_some_and(|slot| slot.availability.is_free())
    );
    assert_eq!(
        store
            .booking_of_guest(bar, account.id, morning())
            .await
            .expect("reads"),
        None
    );
}

#[tokio::test]
async fn a_booking_is_the_guests_for_exactly_as_long_as_it_holds_their_table() {
    // The one occupancy rule, asked of the guest's own screen. What a guest has is a table being
    // held for them; the moment it goes back into the pool — they went home, or they never came
    // and the bar stopped waiting — the evening is over and there is nothing left to move.
    let store = store().await;
    let (bar, _) = default_bar(&store).await;

    for (attendance, settled, still_theirs, over) in [
        // Left at half past nine, an hour before the window they were promised ran out.
        (Attendance::Left, 1290, 1289, 1290),
        // Never came. The bar holds the table through the fifteen-minute grace and no longer.
        (Attendance::NoShow, 1205, 1214, 1215),
    ] {
        let account = fresh_account("Полина");
        store.identify(bar, &account, morning()).await.expect("ok");
        let created = store
            .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
            .await
            .expect("free");
        store
            .set_attendance(
                bar,
                created.record.booking.id,
                attendance,
                at(thursday(), settled),
            )
            .await
            .expect("recorded");

        assert!(
            store
                .booking_of_guest(bar, account.id, at(thursday(), still_theirs))
                .await
                .expect("reads")
                .is_some(),
            "{attendance:?}: the table is still being held at {still_theirs}"
        );
        assert_eq!(
            store
                .booking_of_guest(bar, account.id, at(thursday(), over))
                .await
                .expect("reads"),
            None,
            "{attendance:?}: the table went back into the pool at {over}"
        );
        assert!(
            matches!(
                store
                    .cancel_booking_of_guest(bar, account.id, at(thursday(), over))
                    .await,
                Err(Error::NotFound { .. })
            ),
            "{attendance:?}: an evening that happened is not a booking to give back"
        );
    }
}

#[tokio::test]
async fn leaving_tonight_hands_the_guest_back_the_evening_they_booked_next() {
    // A guest may hold two at once: the table they are sitting at, and a booking for another
    // evening taken while sitting at it. When tonight ends, what they have is the other one — so
    // the question "which booking is mine" cannot be answered by the earliest row and a filter
    // after it.
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Ксения");
    store.identify(bar, &account, morning()).await.expect("ok");

    let tonight = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    let sat_down = at(thursday(), 1200);
    store
        .set_attendance(bar, tonight.record.booking.id, Attendance::Arrived, sat_down)
        .await
        .expect("recorded");

    let mut next = guest_booking(bar, &account, 1200, 2);
    next.service_day = thursday().checked_add_days(2).expect("in range");
    let saturday = store
        .create_booking(&next, sat_down)
        .await
        .expect("a table on Saturday");

    assert_eq!(
        store
            .booking_of_guest(bar, account.id, at(thursday(), 1250))
            .await
            .expect("reads")
            .map(|record| record.booking.id),
        Some(tonight.record.booking.id),
        "while they are sitting, what they have is the table they are at"
    );

    let went_home = at(thursday(), 1290);
    store
        .set_attendance(bar, tonight.record.booking.id, Attendance::Left, went_home)
        .await
        .expect("recorded");
    assert_eq!(
        store
            .booking_of_guest(bar, account.id, went_home)
            .await
            .expect("reads")
            .map(|record| record.booking.id),
        Some(saturday.record.booking.id),
        "tonight is over, and Saturday is still theirs"
    );
}

#[tokio::test]
async fn cancelling_twice_is_refused_rather_than_silently_repeated() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Юля");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    store
        .cancel_booking(bar, created.record.booking.id, None, None, morning())
        .await
        .expect("cancelled");
    let again = store
        .cancel_booking(bar, created.record.booking.id, None, None, morning())
        .await
        .expect_err("already cancelled");
    assert!(
        matches!(again, Error::NotFound { entity: "booking" }),
        "got {again:?}"
    );
}

#[tokio::test]
async fn a_guest_of_one_bar_is_invisible_to_another() {
    let store = store().await;
    let (first_bar, _) = default_bar(&store).await;
    let (second_bar, _) = default_bar(&store).await;
    let account = fresh_account("Ксения");
    store
        .identify(first_bar, &account, morning())
        .await
        .expect("ok");

    store
        .create_booking(&guest_booking(first_bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    assert_eq!(
        store
            .booking_of_guest(second_bar, account.id, morning())
            .await
            .expect("reads"),
        None
    );
    assert!(
        store
            .shift(second_bar, thursday())
            .await
            .expect("reads")
            .bookings
            .is_empty()
    );
}

#[tokio::test]
async fn the_shift_view_reports_the_bookings_and_the_tables_that_are_shut() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let account = fresh_account("Соня");
    store.identify(bar, &account, morning()).await.expect("ok");
    store
        .create_booking(&guest_booking(bar, &account, 1200, 4), morning())
        .await
        .expect("free");

    let terrace: Vec<_> = config
        .tables
        .iter()
        .filter(|table| table.zone.as_str() == "Терраса")
        .map(|table| table.id)
        .collect();
    store
        .block_tables(bar, thursday(), &terrace, "Дождь", None, morning())
        .await
        .expect("closed");

    let shift = store.shift(bar, thursday()).await.expect("reads");
    assert_eq!(shift.bookings.len(), 1);
    assert_eq!(shift.blocks.len(), 2);
    assert!(shift.blocks.iter().all(|block| block.reason == "Дождь"));
    assert_eq!(
        shift
            .blocks
            .iter()
            .map(|block| block.table_number)
            .collect::<Vec<_>>(),
        vec![14, 15]
    );
}

#[tokio::test]
async fn a_booking_at_one_in_the_morning_belongs_to_the_evening_it_started_in() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let account = fresh_account("Егор");
    store.identify(bar, &account, morning()).await.expect("ok");

    // Midnight is minute 1440 of the Thursday shift: the last arrival before a 02:00 close.
    let created = store
        .create_booking(&guest_booking(bar, &account, 1440, 2), morning())
        .await
        .expect("free");
    assert_eq!(created.record.booking.service_day, thursday());
    // 00:00 Belgrade on the Friday is 22:00 UTC on the Thursday.
    assert_eq!(
        created.record.booking.window.start(),
        utc(2026, 7, 30, 22, 0)
    );
    assert!(
        store
            .shift(bar, thursday())
            .await
            .expect("reads")
            .bookings
            .iter()
            .any(|record| record.booking.id == created.record.booking.id),
        "it appears on Thursday's shift, not Friday's"
    );

    // And at one in the morning the app still considers Thursday the running shift.
    assert_eq!(
        config.current_service_day(utc(2026, 7, 30, 23, 0)),
        thursday()
    );
}

#[tokio::test]
async fn a_late_booking_does_not_haunt_the_following_shift() {
    // The limits make a genuine cross-midnight collision unreachable — the earliest a shift may
    // open is 08:00 and the latest a booking may end is 04:00 — but availability is still computed
    // by loading the neighbouring shifts and judging overlap on absolute time, so that the
    // guarantee does not quietly depend on that arithmetic staying true. What must be visible here
    // is the absence of a phantom conflict: yesterday's late booking blocks nothing today.
    let store = store().await;
    let (bar, _) = common::bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let first = fresh_account("Роман");
    let second = fresh_account("Данила");
    store.identify(bar, &first, morning()).await.expect("ok");
    store.identify(bar, &second, morning()).await.expect("ok");

    // Arrives at midnight on the Thursday shift, leaves at 02:00 on the Friday morning.
    let late = store
        .create_booking(&guest_booking(bar, &first, 1440, 2), morning())
        .await
        .expect("free");
    assert_eq!(late.record.booking.service_day, thursday());
    assert_eq!(late.record.booking.window.end(), utc(2026, 7, 31, 0, 0));

    // Friday opens at 10:00, eight hours after that table is free again.
    let friday = thursday().checked_add_days(1).expect("in range");
    let slots = store
        .availability(bar, friday, 2, morning(), None)
        .await
        .expect("reads").slots;
    assert!(
        slots.iter().all(|slot| slot.availability.is_free()),
        "no slot on the following shift is held by the late booking"
    );

    let mut friday_evening = guest_booking(bar, &second, 1200, 2);
    friday_evening.service_day = friday;
    store
        .create_booking(&friday_evening, morning())
        .await
        .expect("the one table is free on Friday");
}
