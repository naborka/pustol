//! Changing the room and the rules, and what becomes of the bookings already taken.

mod common;

use pustol_db::Error;
use pustol_domain::draft::{StaffDraft, TableDraft};

use common::{bar_with, config_with, default_bar, draft_of, fresh_account, guest_booking, morning, numbered, staff_booking, store, table, thursday, utc};

#[tokio::test]
async fn saving_a_proposal_that_changes_nothing_leaves_everything_alone() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let saved = store
        .save_settings(bar, &draft_of(&config), morning())
        .await
        .expect("saved");
    assert!(saved.reconciliation.is_empty());
    assert_eq!(saved.above_cap, 0);
    assert_eq!(*saved.config, *config);
    assert_eq!(saved.next_table_number, 16);
}

#[tokio::test]
async fn renaming_the_bar_takes_effect_and_survives_a_reload() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    draft.name = "Бар «Чердак»".to_owned();
    draft.address = "Кнез Михаилова 1, Белград".to_owned();
    store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");

    let reloaded = store.config(bar).await.expect("loads");
    assert_eq!(reloaded.name, "Бар «Чердак»");
    assert_eq!(reloaded.address, "Кнез Михаилова 1, Белград");
}

#[tokio::test]
async fn adding_a_table_gives_it_the_next_number_and_makes_it_bookable() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    draft.tables.push(TableDraft::New {
        seats: 4,
        zone: "Зал".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");

    assert_eq!(saved.next_table_number, 17);
    let added = saved
        .config
        .active_tables()
        .find(|table| table.number == 16)
        .expect("the sixteenth table exists");
    assert_eq!(added.seats, 4);
    assert_eq!(store.next_table_number(bar).await.expect("reads"), 17);
}

#[tokio::test]
async fn a_removed_table_is_retired_and_keeps_its_number_for_good() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let last = numbered(&config.tables, 15).clone();

    let mut draft = draft_of(&config);
    draft
        .tables
        .retain(|entry| !matches!(entry, TableDraft::Existing { id, .. } if *id == last.id.0));
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");
    assert_eq!(saved.config.active_tables().count(), 14);
    assert_eq!(
        saved.next_table_number, 16,
        "the retired fifteenth still owns its number"
    );

    // Add a table: it must be sixteen, never a second fifteen.
    let mut draft = draft_of(&saved.config);
    draft.tables.push(TableDraft::New {
        seats: 2,
        zone: "Бар".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");
    let numbers: Vec<i32> = saved.config.tables.iter().map(|table| table.number).collect();
    assert_eq!(numbers.iter().filter(|number| **number == 15).count(), 1);
    assert!(numbers.contains(&16));
}

#[tokio::test]
async fn retiring_a_table_moves_the_party_sitting_at_it() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let account = fresh_account("Ксения");
    store.identify(bar, &account, morning()).await.expect("ok");
    let seated = store
        .create_booking(&guest_booking(bar, &account, 1200, 6), morning())
        .await
        .expect("free");
    assert_eq!(seated.record.table_number, Some(11));

    let mut draft = draft_of(&config);
    draft.tables.retain(
        |entry| !matches!(entry, TableDraft::Existing { id, .. } if *id == numbered(&config.tables, 11).id.0),
    );
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");

    assert_eq!(saved.reconciliation.outcome.orphaned, Vec::new());
    assert_eq!(saved.reconciliation.outcome.moved.len(), 1);
    assert_eq!(saved.reconciliation.outcome.moved[0].to_number, 12);

    let after = store
        .booking_of_guest(bar, account.id, morning())
        .await
        .expect("reads")
        .expect("still booked");
    assert_eq!(after.table_number, Some(12));
}

#[tokio::test]
async fn shrinking_the_room_until_a_party_no_longer_fits_leaves_them_without_a_table() {
    let store = store().await;
    // Two six-tops and nothing else, both holding a party of six at the same time.
    let (bar, config) = bar_with(
        &store,
        config_with(
            vec![table(11, 6, "Зал"), table(12, 6, "Зал")],
            "anna_mgr",
        ),
    )
    .await;
    store
        .create_booking(&staff_booking(bar, "Игорь", 1200, 6), morning())
        .await
        .expect("free");
    store
        .create_booking(&staff_booking(bar, "Тимур", 1200, 6), morning())
        .await
        .expect("free");

    // Shrink one of them: its party has nowhere to go.
    let mut draft = draft_of(&config);
    for entry in &mut draft.tables {
        if let TableDraft::Existing { id, seats, .. } = entry
            && *id == numbered(&config.tables, 12).id.0
        {
            *seats = 2;
        }
    }
    // The cap has to come down with the room, or the configuration is illegal.
    draft.max_party = 6;
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");

    assert_eq!(saved.reconciliation.outcome.moved, Vec::new());
    assert_eq!(saved.reconciliation.outcome.orphaned.len(), 1);

    let shift = store.shift(bar, thursday()).await.expect("reads");
    let orphan = shift
        .bookings
        .iter()
        .find(|record| record.table_number.is_none())
        .expect("one booking has no table");
    assert_eq!(orphan.guest_name, "Тимур");
    assert!(
        shift.bookings.iter().any(|record| record.table_number == Some(11)),
        "the other party keeps its table"
    );
}

#[tokio::test]
async fn a_party_left_without_a_table_is_seated_the_moment_the_room_can_take_them() {
    let store = store().await;
    let (bar, config) = bar_with(
        &store,
        config_with(
            vec![table(11, 6, "Зал"), table(12, 6, "Зал")],
            "anna_mgr",
        ),
    )
    .await;
    store
        .create_booking(&staff_booking(bar, "Игорь", 1200, 6), morning())
        .await
        .expect("free");
    let stranded = store
        .create_booking(&staff_booking(bar, "Тимур", 1200, 6), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&config);
    for entry in &mut draft.tables {
        if let TableDraft::Existing { id, seats, .. } = entry
            && *id == numbered(&config.tables, 12).id.0
        {
            *seats = 2;
        }
    }
    draft.max_party = 6;
    store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");

    // Asking again changes nothing while the room is still too small, and says so plainly.
    let still_stuck = store
        .reconcile_shift(bar, thursday(), morning())
        .await
        .expect("the attempt itself succeeds");
    assert_eq!(still_stuck.outcome.moved, Vec::new());
    assert_eq!(still_stuck.outcome.orphaned, vec![stranded.record.booking.id]);

    // Give the room a six-top back. Nobody has to remember to press anything: the party the bar
    // still owes a table is seated as part of applying the change.
    let mut draft = draft_of(&store.config(bar).await.expect("loads"));
    draft.tables.push(TableDraft::New {
        seats: 6,
        zone: "Зал".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");
    assert_eq!(saved.reconciliation.outcome.orphaned, Vec::new());
    assert_eq!(saved.reconciliation.outcome.moved.len(), 1);
    assert_eq!(saved.reconciliation.outcome.moved[0].booking, stranded.record.booking.id);
    assert_eq!(saved.reconciliation.outcome.moved[0].to_number, 13);

    let seated = store
        .shift(bar, thursday())
        .await
        .expect("reads")
        .bookings
        .into_iter()
        .find(|record| record.booking.id == stranded.record.booking.id)
        .expect("still on the shift");
    assert_eq!(seated.table_number, Some(13));
}

#[tokio::test]
async fn asking_a_sound_shift_to_reconcile_moves_nobody() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    store
        .create_booking(&staff_booking(bar, "Марк", 1200, 4), morning())
        .await
        .expect("free");
    let outcome = store
        .reconcile_shift(bar, thursday(), morning())
        .await
        .expect("nothing to do");
    assert!(outcome.is_empty());
}

#[tokio::test]
async fn cancelling_a_booking_hands_the_freed_table_to_a_party_that_was_owed_one() {
    let store = store().await;
    // Two six-tops, both full, then one shrinks so its party is stranded.
    let (bar, config) = bar_with(
        &store,
        config_with(vec![table(11, 6, "Зал"), table(12, 6, "Зал")], "anna_mgr"),
    )
    .await;
    let keeping = store
        .create_booking(&staff_booking(bar, "Игорь", 1200, 6), morning())
        .await
        .expect("free");
    let stranded = store
        .create_booking(&staff_booking(bar, "Тимур", 1200, 6), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&config);
    for entry in &mut draft.tables {
        if let TableDraft::Existing { id, seats, .. } = entry
            && *id == numbered(&config.tables, 12).id.0
        {
            *seats = 2;
        }
    }
    draft.max_party = 6;
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");
    assert_eq!(saved.reconciliation.outcome.orphaned, vec![stranded.record.booking.id]);

    // The party holding the surviving six-top cancels. Nobody has to notice: the table goes
    // straight to the party the bar still owes one.
    let cancelled = store
        .cancel_booking(
            bar,
            keeping.record.booking.id,
            Some("Технические проблемы в баре"),
            Some(common::cancellation_wording),
            morning(),
        )
        .await
        .expect("cancelled");
    assert_eq!(cancelled.reconciliation.outcome.moved.len(), 1);
    assert_eq!(
        cancelled.reconciliation.outcome.moved[0].booking,
        stranded.record.booking.id
    );
    assert_eq!(cancelled.reconciliation.outcome.moved[0].to_number, 11);
}

#[tokio::test]
async fn closing_a_day_that_has_bookings_on_it_is_refused_with_the_bookings_named() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    store
        .create_booking(&staff_booking(bar, "Олег", 1200, 4), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&config);
    // Thursday, index 4 counting from Sunday.
    draft.week[4].closed = true;
    let refused = store
        .save_settings(bar, &draft, morning())
        .await
        .expect_err("a promised booking cannot be shut out");
    match refused {
        Error::WouldStrandBookings(conflicts) => assert_eq!(conflicts.len(), 1),
        other => panic!("got {other:?}"),
    }

    // Nothing was written.
    assert!(!store.config(bar).await.expect("loads").week.all()[4].closed);
}

#[tokio::test]
async fn a_day_with_no_bookings_left_on_it_can_be_closed() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    draft.week[4].closed = true;
    store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");
    assert!(store.config(bar).await.expect("loads").week.all()[4].closed);
}

#[tokio::test]
async fn a_booking_that_has_already_finished_never_blocks_a_settings_change() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    store
        .create_booking(&staff_booking(bar, "Вадим", 1200, 4), morning())
        .await
        .expect("free");

    // The following afternoon: Thursday's evening is over and cannot be moved.
    let later = utc(2026, 7, 31, 12, 0);
    let mut draft = draft_of(&config);
    draft.week[4].closed = true;
    store
        .save_settings(bar, &draft, later)
        .await
        .expect("history does not hold the settings hostage");
}

#[tokio::test]
async fn moving_opening_hours_past_a_live_booking_is_refused() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    // Arrives at 11:00, so an opening at 17:00 would shut them out.
    store
        .create_booking(&staff_booking(bar, "Никита", 660, 4), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&config);
    draft.week[4].open_minutes = 1020;
    let refused = store
        .save_settings(bar, &draft, morning())
        .await
        .expect_err("refused");
    assert!(
        matches!(refused, Error::WouldStrandBookings(_)),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn lengthening_the_turn_does_not_retroactively_extend_anyones_table() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    // Two parties back to back on what will be the same table.
    let first = store
        .create_booking(&staff_booking(bar, "Саша", 1200, 2), morning())
        .await
        .expect("free");
    let second = store
        .create_booking(&staff_booking(bar, "Лиза", 1320, 2), morning())
        .await
        .expect("free");
    assert_eq!(first.record.table_number, Some(1));
    assert_eq!(second.record.table_number, Some(1));

    let mut draft = draft_of(&config);
    draft.turn_minutes = 240;
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("nobody already seated is affected");
    assert!(saved.reconciliation.is_empty());

    // Both bookings keep the two hours they were promised.
    let shift = store.shift(bar, thursday()).await.expect("reads");
    assert!(
        shift
            .bookings
            .iter()
            .all(|record| record.booking.window.minutes() == 120)
    );
    // The next guest gets the new length.
    let next = store
        .create_booking(&staff_booking(bar, "Юля", 1200, 4), morning())
        .await
        .expect("free");
    assert_eq!(next.record.booking.window.minutes(), 240);
}

#[tokio::test]
async fn lowering_the_party_cap_reports_the_bookings_above_it_without_refusing() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    store
        .create_booking(&staff_booking(bar, "Артур", 1200, 6), morning())
        .await
        .expect("free");
    store
        .create_booking(&staff_booking(bar, "Настя", 1200, 2), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&config);
    draft.max_party = 4;
    let saved = store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");
    assert_eq!(saved.above_cap, 1);
    assert!(
        saved.reconciliation.is_empty(),
        "a party already seated keeps its table"
    );
}

#[tokio::test]
async fn a_party_cap_no_table_can_seat_is_refused_as_illegal() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    draft.max_party = 10;
    let refused = store
        .save_settings(bar, &draft, morning())
        .await
        .expect_err("no table seats ten");
    match refused {
        Error::ProposedConfigInvalid(errors) => assert!(errors.iter().any(|error| matches!(
            error,
            pustol_domain::ConfigError::MaxPartyExceedsLargestTable { .. }
        ))),
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn a_proposal_naming_a_table_from_another_bar_cannot_even_be_interpreted() {
    let store = store().await;
    let (first_bar, first_config) = default_bar(&store).await;
    let (_, other_config) = default_bar(&store).await;

    let mut draft = draft_of(&first_config);
    draft.tables.push(TableDraft::Existing {
        id: other_config.tables[0].id.0,
        seats: 4,
        zone: "Зал".to_owned(),
    });
    let refused = store
        .save_settings(first_bar, &draft, morning())
        .await
        .expect_err("that table is not in this room");
    assert!(
        matches!(refused, Error::UnusableProposal(_)),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn the_last_admin_cannot_be_removed() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    draft.staff.clear();
    let refused = store
        .save_settings(bar, &draft, morning())
        .await
        .expect_err("that would lock everyone out");
    match refused {
        Error::ProposedConfigInvalid(errors) => {
            assert!(errors.contains(&pustol_domain::ConfigError::NoStaff));
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn a_bound_admin_keeps_their_seat_through_a_settings_save() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let mut anna = fresh_account("Анна");
    anna.username = Some("anna_mgr".to_owned());
    let viewer = store
        .identify(bar, &anna, morning())
        .await
        .expect("identified");
    assert!(viewer.is_staff, "the invited username claims its seat");

    // A save that only adds somebody else must not clear the binding already made.
    let mut draft = draft_of(&store.config(bar).await.expect("loads"));
    draft.staff.push(StaffDraft {
        username: "pavel_bar".to_owned(),
    });
    store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");

    let after = store
        .identify(bar, &anna, morning())
        .await
        .expect("identified");
    assert!(after.is_staff);
    let bound = store
        .config(bar)
        .await
        .expect("loads")
        .staff
        .iter()
        .find(|member| member.username == "anna_mgr")
        .and_then(|member| member.telegram_user_id);
    assert_eq!(bound, Some(anna.id.0));
}

#[tokio::test]
async fn a_stranger_who_takes_over_an_invited_username_gets_nothing() {
    // A Telegram username can be released and claimed by somebody else. Once a seat carries a
    // numeric id, that seat is that person's and the username is only a label.
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let mut anna = fresh_account("Анна");
    anna.username = Some("anna_mgr".to_owned());
    assert!(
        store
            .identify(bar, &anna, morning())
            .await
            .expect("identified")
            .is_staff
    );

    let mut squatter = fresh_account("Не Анна");
    squatter.username = Some("anna_mgr".to_owned());
    let viewer = store
        .identify(bar, &squatter, morning())
        .await
        .expect("identified");
    assert!(
        !viewer.is_staff,
        "the seat is already claimed by a numeric account"
    );
}

#[tokio::test]
async fn somebody_never_invited_is_not_staff() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let stranger = fresh_account("Прохожий");
    assert!(
        !store
            .identify(bar, &stranger, morning())
            .await
            .expect("identified")
            .is_staff
    );
}

#[tokio::test]
async fn editing_the_guest_messages_and_cancellation_reasons_replaces_the_lists() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    draft.message_templates = vec!["Стол готов".to_owned(), "Держим ещё 15 минут".to_owned()];
    draft.cancel_reasons = vec!["Дождь".to_owned()];
    store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");

    let reloaded = store.config(bar).await.expect("loads");
    assert_eq!(reloaded.message_templates.len(), 2);
    assert_eq!(reloaded.cancel_reasons, vec!["Дождь".to_owned()]);
}

#[tokio::test]
async fn a_blank_guest_message_is_refused() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    draft.message_templates = vec!["   ".to_owned()];
    let refused = store
        .save_settings(bar, &draft, morning())
        .await
        .expect_err("an empty message would be sent to a guest");
    assert!(
        matches!(refused, Error::ProposedConfigInvalid(_)),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn applying_one_days_hours_to_the_whole_week_is_just_a_proposal_like_any_other() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let mut draft = draft_of(&config);
    for day in &mut draft.week {
        day.open_minutes = 1020;
        day.close_minutes = 1440;
    }
    store
        .save_settings(bar, &draft, morning())
        .await
        .expect("saved");
    let reloaded = store.config(bar).await.expect("loads");
    assert!(
        reloaded
            .week
            .all()
            .iter()
            .all(|hours| hours.open_minutes == 1020 && hours.close_minutes == 1440)
    );
}
