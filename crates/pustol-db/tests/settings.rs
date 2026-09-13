//! Changing the room and the rules, and what becomes of the bookings already taken.

mod common;

use pustol_db::Error;
use pustol_domain::draft::{StaffDraft, TableDraft};

use common::{bar_with, config_with, signed, default_bar, draft_of, fresh_account, guest_booking, morning, numbered, staff_booking, store, table, thursday, utc};

#[tokio::test]
async fn saving_a_proposal_that_changes_nothing_leaves_everything_alone() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let saved = store
        .save_settings(bar, &draft_of(&store, bar).await, thursday(), morning())
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
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    draft.name = "Бар «Чердак»".to_owned();
    draft.address = "Кнез Михаилова 1, Белград".to_owned();
    store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");

    let reloaded = store.config(bar).await.expect("loads");
    assert_eq!(reloaded.name, "Бар «Чердак»");
    assert_eq!(reloaded.address, "Кнез Михаилова 1, Белград");
}

#[tokio::test]
async fn adding_a_table_gives_it_the_next_number_and_makes_it_bookable() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    draft.tables.push(TableDraft {
        id: uuid::Uuid::new_v4(),
        seats: 4,
        zone: "Зал".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
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

    let mut draft = draft_of(&store, bar).await;
    draft
        .tables
        .retain(|entry| entry.id != last.id.0);
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");
    assert_eq!(saved.config.active_tables().count(), 14);
    assert_eq!(
        saved.next_table_number, 16,
        "the retired fifteenth still owns its number"
    );

    // Add a table: it must be sixteen, never a second fifteen.
    let mut draft = draft_of(&store, bar).await;
    draft.tables.push(TableDraft {
        id: uuid::Uuid::new_v4(),
        seats: 2,
        zone: "Бар".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
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
    store.identify(bar, &account, signed(morning()), morning()).await.expect("ok");
    let seated = store
        .create_booking(&guest_booking(bar, &account, 1200, 6), morning())
        .await
        .expect("free");
    assert_eq!(seated.record.table_number, Some(11));

    let mut draft = draft_of(&store, bar).await;
    draft.tables.retain(
        |entry| entry.id != numbered(&config.tables, 11).id.0,
    );
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");

    assert_eq!(saved.reconciliation.outcome.orphaned, Vec::new());
    assert_eq!(saved.reconciliation.outcome.moved.len(), 1);
    assert_eq!(saved.reconciliation.outcome.moved[0].to_number, 12);

    let after = store
        .bookings_of_guest(bar, account.id, morning())
        .await
        .expect("reads");
    assert_eq!(after.len(), 1, "still booked");
    assert_eq!(after[0].table_number, Some(12));
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
    let mut draft = draft_of(&store, bar).await;
    for entry in &mut draft.tables {
        if entry.id == numbered(&config.tables, 12).id.0 {
            entry.seats = 2;
        }
    }
    // The cap has to come down with the room, or the configuration is illegal.
    draft.max_party = 6;
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");

    assert_eq!(saved.reconciliation.outcome.moved, Vec::new());
    assert_eq!(saved.reconciliation.outcome.orphaned.len(), 1);

    let shift = store.evening(bar, thursday(), common::morning()).await.expect("reads");
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

    let mut draft = draft_of(&store, bar).await;
    for entry in &mut draft.tables {
        if entry.id == numbered(&config.tables, 12).id.0 {
            entry.seats = 2;
        }
    }
    draft.max_party = 6;
    store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");

    // Asking again changes nothing while the room is still too small, and says so plainly.
    let still_stuck = store
        .reconcile_shift(bar, thursday(), morning())
        .await
        .expect("the attempt itself succeeds");
    assert_eq!(still_stuck.reconciliation.outcome.moved, Vec::new());
    assert_eq!(still_stuck.reconciliation.outcome.orphaned, vec![stranded.record.booking.id]);

    // Give the room a six-top back. Nobody has to remember to press anything: the party the bar
    // still owes a table is seated as part of applying the change.
    let mut draft = draft_of(&store, bar).await;
    draft.tables.push(TableDraft {
        id: uuid::Uuid::new_v4(),
        seats: 6,
        zone: "Зал".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");
    assert_eq!(saved.reconciliation.outcome.orphaned, Vec::new());
    assert_eq!(saved.reconciliation.outcome.moved.len(), 1);
    assert_eq!(saved.reconciliation.outcome.moved[0].booking, stranded.record.booking.id);
    assert_eq!(saved.reconciliation.outcome.moved[0].to_number, 13);

    let seated = store
        .evening(bar, thursday(), common::morning())
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
    assert!(outcome.reconciliation.is_empty());
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

    let mut draft = draft_of(&store, bar).await;
    for entry in &mut draft.tables {
        if entry.id == numbered(&config.tables, 12).id.0 {
            entry.seats = 2;
        }
    }
    draft.max_party = 6;
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
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
    let (bar, _) = default_bar(&store).await;
    store
        .create_booking(&staff_booking(bar, "Олег", 1200, 4), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&store, bar).await;
    // Thursday, index 4 counting from Sunday.
    draft.week[4].closed = true;
    let refused = store
        .save_settings(bar, &draft, thursday(), morning())
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
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    draft.week[4].closed = true;
    store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");
    assert!(store.config(bar).await.expect("loads").week.all()[4].closed);
}

#[tokio::test]
async fn a_booking_that_has_already_finished_never_blocks_a_settings_change() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    store
        .create_booking(&staff_booking(bar, "Вадим", 1200, 4), morning())
        .await
        .expect("free");

    // The following afternoon: Thursday's evening is over and cannot be moved.
    let later = utc(2026, 7, 31, 12, 0);
    let mut draft = draft_of(&store, bar).await;
    draft.week[4].closed = true;
    store
        .save_settings(bar, &draft, thursday(), later)
        .await
        .expect("history does not hold the settings hostage");
}

#[tokio::test]
async fn moving_opening_hours_past_a_live_booking_is_refused() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    // Arrives at 11:00, so an opening at 17:00 would shut them out.
    store
        .create_booking(&staff_booking(bar, "Никита", 660, 4), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&store, bar).await;
    draft.week[4].open_minutes = 1020;
    let refused = store
        .save_settings(bar, &draft, thursday(), morning())
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
    let (bar, _) = default_bar(&store).await;
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

    let mut draft = draft_of(&store, bar).await;
    draft.turn_minutes = 240;
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("nobody already seated is affected");
    assert!(saved.reconciliation.is_empty());

    // Both bookings keep the two hours they were promised.
    let shift = store.evening(bar, thursday(), common::morning()).await.expect("reads");
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
    let (bar, _) = default_bar(&store).await;
    store
        .create_booking(&staff_booking(bar, "Артур", 1200, 6), morning())
        .await
        .expect("free");
    store
        .create_booking(&staff_booking(bar, "Настя", 1200, 2), morning())
        .await
        .expect("free");

    let mut draft = draft_of(&store, bar).await;
    draft.max_party = 4;
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
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
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    draft.max_party = 10;
    let refused = store
        .save_settings(bar, &draft, thursday(), morning())
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
    let (first_bar, _) = default_bar(&store).await;
    let (_, other_config) = default_bar(&store).await;

    let mut draft = draft_of(&store, first_bar).await;
    draft.tables.push(TableDraft {
        id: other_config.tables[0].id.0,
        seats: 4,
        zone: "Зал".to_owned(),
    });
    let refused = store
        .save_settings(first_bar, &draft, thursday(), morning())
        .await
        .expect_err("that table is not in this room");
    assert!(
        matches!(refused, Error::UnusableProposal(_)),
        "got {refused:?}"
    );
}

#[tokio::test]
async fn a_table_the_app_adds_keeps_the_identity_the_app_gave_it() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let named = uuid::Uuid::new_v4();
    let mut draft = draft_of(&store, bar).await;
    draft.tables.push(TableDraft {
        id: named,
        seats: 4,
        zone: "Зал".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");
    let added = saved
        .config
        .active_tables()
        .find(|table| table.id.0 == named)
        .expect("stored under the id it was sent with");
    assert_eq!(added.number, 16);
}

#[tokio::test]
async fn the_same_proposal_saved_twice_adds_its_table_once() {
    // The answer to the first save was lost, and the app sends the same rows again, made from the
    // settings as they now are.
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let named = uuid::Uuid::new_v4();
    let mut draft = draft_of(&store, bar).await;
    draft.tables.push(TableDraft {
        id: named,
        seats: 4,
        zone: "Зал".to_owned(),
    });
    let first = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");
    draft.version = first.version;
    let second = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved again");

    assert_eq!(second.config.tables.len(), 16, "one table was added, not two");
    assert_eq!(second.next_table_number, 17);
    let tables: Vec<i64> =
        sqlx::query_scalar("select count(*) from bar_table where bar_id = $1")
            .bind(bar)
            .fetch_all(store.pool())
            .await
            .expect("counted");
    assert_eq!(tables, vec![16]);
}

#[tokio::test]
async fn a_retired_table_named_again_comes_back_under_its_own_number() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let last = numbered(&config.tables, 15).clone();
    let mut draft = draft_of(&store, bar).await;
    draft.tables.retain(|entry| entry.id != last.id.0);
    store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("retired");

    let mut draft = draft_of(&store, bar).await;
    draft.tables.push(TableDraft {
        id: last.id.0,
        seats: 4,
        zone: "Терраса".to_owned(),
    });
    let saved = store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("revived");
    let back = saved
        .config
        .active_tables()
        .find(|table| table.id == last.id)
        .expect("in the room again");
    assert_eq!(back.number, 15);
    assert_eq!(back.seats, 4);
    assert_eq!(saved.config.tables.len(), 15, "nothing new was made");
}

#[tokio::test]
async fn the_last_admin_cannot_be_removed() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    draft.staff.clear();
    let refused = store
        .save_settings(bar, &draft, thursday(), morning())
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
        .identify(bar, &anna, signed(morning()), morning())
        .await
        .expect("identified");
    assert!(viewer.is_staff, "the invited username claims its seat");

    // A save that only adds somebody else must not clear the binding already made.
    let mut draft = draft_of(&store, bar).await;
    draft.staff.push(StaffDraft {
        username: "pavel_bar".to_owned(),
    });
    store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");

    let after = store
        .identify(bar, &anna, signed(morning()), morning())
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
            .identify(bar, &anna, signed(morning()), morning())
            .await
            .expect("identified")
            .is_staff
    );

    let mut squatter = fresh_account("Не Анна");
    squatter.username = Some("anna_mgr".to_owned());
    let viewer = store
        .identify(bar, &squatter, signed(morning()), morning())
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
            .identify(bar, &stranger, signed(morning()), morning())
            .await
            .expect("identified")
            .is_staff
    );
}

#[tokio::test]
async fn an_account_remembered_from_a_session_claims_no_seat_and_rewrites_nothing() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let known = fresh_account("Анна");
    store.identify(bar, &known, signed(morning()), morning()).await.expect("identified");

    let mut remembered = known.clone();
    remembered.username = Some("anna_mgr".to_owned());
    remembered.first_name = "Старое имя".to_owned();
    let viewer = store
        .recognise(bar, &remembered, morning())
        .await
        .expect("recognised");

    assert!(!viewer.is_staff, "a username kept in a session proves nothing about who holds it now");
    assert_eq!(viewer.account, known, "the account as stored, not as remembered");
    let bound = store
        .config(bar)
        .await
        .expect("loads")
        .staff
        .iter()
        .find(|member| member.username == "anna_mgr")
        .and_then(|member| member.telegram_user_id);
    assert_eq!(bound, None);
}

#[tokio::test]
async fn an_account_first_seen_through_a_session_is_recorded_as_given() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Гость");
    let viewer = store
        .recognise(bar, &account, morning())
        .await
        .expect("recognised");
    assert_eq!(viewer.account, account);
    assert!(viewer.reminders.should_ask());
}

#[tokio::test]
async fn choosing_reminders_does_not_rewrite_a_known_account() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let known = fresh_account("Анна");
    store.identify(bar, &known, signed(morning()), morning()).await.expect("identified");

    let mut stale = known.clone();
    stale.username = Some("old_name".to_owned());
    store
        .choose_reminders(&stale, pustol_db::identity::ReminderChoice::OptIn, morning())
        .await
        .expect("chosen");

    let viewer = store
        .recognise(bar, &known, morning())
        .await
        .expect("recognised");
    assert_eq!(viewer.account, known);
    assert!(viewer.reminders.opted_in);
}

#[tokio::test]
async fn editing_the_guest_messages_and_cancellation_reasons_replaces_the_lists() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    draft.message_templates = vec!["Стол готов".to_owned(), "Держим ещё 15 минут".to_owned()];
    draft.cancel_reasons = vec!["Дождь".to_owned()];
    store
        .save_settings(bar, &draft, thursday(), morning())
        .await
        .expect("saved");

    let reloaded = store.config(bar).await.expect("loads");
    assert_eq!(reloaded.message_templates.len(), 2);
    assert_eq!(reloaded.cancel_reasons, vec!["Дождь".to_owned()]);
}

#[tokio::test]
async fn a_blank_guest_message_is_refused() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    draft.message_templates = vec!["   ".to_owned()];
    let refused = store
        .save_settings(bar, &draft, thursday(), morning())
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
    let (bar, _) = default_bar(&store).await;
    let mut draft = draft_of(&store, bar).await;
    for day in &mut draft.week {
        day.open_minutes = 1020;
        day.close_minutes = 1440;
    }
    store
        .save_settings(bar, &draft, thursday(), morning())
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

#[tokio::test]
async fn a_contact_for_guests_is_saved_cleared_and_checked() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;

    let mut draft = draft_of(&store, bar).await;
    draft.contact = "  @podval_bar ".to_owned();
    store.save_settings(bar, &draft, thursday(), morning()).await.expect("saved");
    assert_eq!(store.config(bar).await.expect("loads").contact.as_deref(), Some("@podval_bar"));

    draft.version = store.settings(bar).await.expect("reads").version;
    draft.contact = String::new();
    store.save_settings(bar, &draft, thursday(), morning()).await.expect("saved");
    assert_eq!(store.config(bar).await.expect("loads").contact, None, "blank means none");

    draft.version = store.settings(bar).await.expect("reads").version;
    draft.contact = "звоните".to_owned();
    let refused = store.save_settings(bar, &draft, thursday(), morning()).await;
    assert!(matches!(refused, Err(Error::ProposedConfigInvalid(_))), "{refused:?}");
}

/// Offers seats under these usernames at `at`, keeping everybody already on the roster.
async fn invite(
    store: &pustol_db::Store,
    bar: pustol_db::BarId,
    usernames: &[&str],
    at: chrono::DateTime<chrono::Utc>,
) {
    let mut draft = draft_of(store, bar).await;
    for username in usernames {
        draft.staff.push(StaffDraft {
            username: (*username).to_owned(),
        });
    }
    store.save_settings(bar, &draft, thursday(), at).await.expect("saved");
}

async fn holder_of(store: &pustol_db::Store, bar: pustol_db::BarId, username: &str) -> Option<i64> {
    store
        .config(bar)
        .await
        .expect("loads")
        .staff
        .iter()
        .find(|member| member.username == username)
        .and_then(|member| member.telegram_user_id)
}

#[tokio::test]
async fn a_payload_stamped_within_the_clock_skew_after_an_invitation_claims_nothing() {
    // Telegram stamps a payload by its own clock. One stamped thirty seconds after the seat was
    // offered, by a clock that may be a minute ahead, may have been signed before the offer, under a
    // name that was somebody else's then.
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let offered = morning() + chrono::TimeDelta::hours(1);
    invite(&store, bar, &["pavel_bar"], offered).await;
    let mut pavel = fresh_account("Павел");
    pavel.username = Some("pavel_bar".to_owned());
    let skew = chrono::TimeDelta::minutes(1);

    let close = offered + chrono::TimeDelta::seconds(30);
    let viewer = store
        .identify(
            bar,
            &pavel,
            pustol_db::identity::Signature {
                stamped_at: close,
                clock_skew: skew,
            },
            close,
        )
        .await
        .expect("identified");
    assert!(!viewer.is_staff);
    assert_eq!(holder_of(&store, bar, "pavel_bar").await, None);

    let clear = offered + chrono::TimeDelta::seconds(61);
    let viewer = store
        .identify(
            bar,
            &pavel,
            pustol_db::identity::Signature {
                stamped_at: clear,
                clock_skew: skew,
            },
            clear,
        )
        .await
        .expect("identified");
    assert!(
        viewer.is_staff,
        "signed after the offer on any clock within the skew"
    );
    assert_eq!(holder_of(&store, bar, "pavel_bar").await, Some(pavel.id.0));
}

#[tokio::test]
async fn an_older_payload_claims_nothing_under_a_name_the_account_has_moved_on_from() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    invite(&store, bar, &["pavel_bar"], morning()).await;
    let mut renamed = fresh_account("Павел");
    renamed.username = Some("renamed_since".to_owned());
    store
        .identify(
            bar,
            &renamed,
            signed(morning() + chrono::TimeDelta::minutes(10)),
            morning() + chrono::TimeDelta::minutes(10),
        )
        .await
        .expect("identified");

    let mut before = renamed.clone();
    before.username = Some("pavel_bar".to_owned());
    let viewer = store
        .identify(
            bar,
            &before,
            signed(morning() + chrono::TimeDelta::minutes(5)),
            morning() + chrono::TimeDelta::minutes(10),
        )
        .await
        .expect("identified");

    assert!(
        !viewer.is_staff,
        "the server already knows the account is not called that any more"
    );
    assert_eq!(viewer.account.username.as_deref(), Some("renamed_since"));
    assert_eq!(holder_of(&store, bar, "pavel_bar").await, None);
}

#[tokio::test]
async fn a_payload_of_the_same_second_as_the_stored_profile_rewrites_it_only_when_it_says_the_same() {
    // Telegram stamps whole seconds. The account was renamed within the second, and the payload
    // still carrying the old name is not known to be the older one; it must not claim a seat under a
    // name the server has already seen the account give up.
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    invite(&store, bar, &["pavel_bar"], morning()).await;
    let second = morning() + chrono::TimeDelta::minutes(10);
    let mut renamed = fresh_account("Павел");
    renamed.username = Some("renamed_v".to_owned());
    store
        .identify(bar, &renamed, signed(second), second)
        .await
        .expect("identified");

    let mut same_second = renamed.clone();
    same_second.username = Some("pavel_bar".to_owned());
    let viewer = store
        .identify(bar, &same_second, signed(second), second)
        .await
        .expect("identified");
    assert!(!viewer.is_staff);
    assert_eq!(viewer.account.username.as_deref(), Some("renamed_v"));
    assert_eq!(holder_of(&store, bar, "pavel_bar").await, None);

    let again = store
        .identify(bar, &renamed, signed(second), second)
        .await
        .expect("the very same payload is still accepted");
    assert_eq!(again.account.username.as_deref(), Some("renamed_v"));

    let later = second + chrono::TimeDelta::seconds(1);
    let viewer = store
        .identify(bar, &same_second, signed(later), later)
        .await
        .expect("identified");
    assert!(viewer.is_staff, "a strictly newer payload rewrites as ever");
    assert_eq!(holder_of(&store, bar, "pavel_bar").await, Some(renamed.id.0));
}

#[tokio::test]
async fn an_account_that_holds_a_seat_claims_no_second_one() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let mut anna = fresh_account("Анна");
    anna.username = Some("anna_mgr".to_owned());
    store
        .identify(bar, &anna, signed(morning()), morning())
        .await
        .expect("identified");
    invite(&store, bar, &["pavel_bar"], morning()).await;

    anna.username = Some("pavel_bar".to_owned());
    let later = morning() + chrono::TimeDelta::minutes(10);
    let viewer = store
        .identify(bar, &anna, signed(later), later)
        .await
        .expect("a second name is not an error");

    assert!(viewer.is_staff);
    assert_eq!(holder_of(&store, bar, "anna_mgr").await, Some(anna.id.0));
    assert_eq!(
        holder_of(&store, bar, "pavel_bar").await,
        None,
        "pavel_bar is still Павел's to claim"
    );
}

#[tokio::test]
async fn two_payloads_of_one_account_claiming_two_seats_at_once_bind_one_and_fail_neither() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let rounds = 8;
    let names: Vec<(String, String)> = (0..rounds)
        .map(|round| (format!("seat_{round}_a"), format!("seat_{round}_b")))
        .collect();
    let all: Vec<&str> = names
        .iter()
        .flat_map(|(a, b)| [a.as_str(), b.as_str()])
        .collect();
    invite(&store, bar, &all, morning()).await;
    let later = morning() + chrono::TimeDelta::minutes(10);

    let mut attempts = Vec::new();
    let mut accounts = Vec::new();
    for (a, b) in &names {
        let account = fresh_account("Двое");
        accounts.push(account.id.0);
        for username in [a, b] {
            let mut payload = account.clone();
            payload.username = Some(username.clone());
            let store = store.clone();
            attempts.push(tokio::spawn(async move {
                store.identify(bar, &payload, signed(later), later).await
            }));
        }
    }
    for attempt in attempts {
        attempt
            .await
            .expect("did not panic")
            .expect("neither payload fails");
    }

    let staff = store.config(bar).await.expect("loads").staff.clone();
    for account in accounts {
        assert_eq!(
            staff
                .iter()
                .filter(|member| member.telegram_user_id == Some(account))
                .count(),
            1,
            "one account, one seat"
        );
    }
}
