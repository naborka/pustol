//! Closing tables, and the queue of things the bot has to say.

mod common;

use chrono::Duration;
use pustol_db::identity::ReminderChoice;
use pustol_db::notifications::NotificationKind;

use common::{bar_with, config_with, default_bar, fresh_account, guest_booking, morning, numbered, staff_booking, store, table, thursday, utc};

#[tokio::test]
async fn closing_a_table_moves_the_party_sitting_at_it() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let seated = store
        .create_booking(&staff_booking(bar, "Анна К.", 1200, 2), morning())
        .await
        .expect("free");
    assert_eq!(seated.record.table_number, Some(1));

    let outcome = store
        .block_tables(
            bar,
            thursday(),
            &[numbered(&config.tables, 1).id],
            "Сломан / залит",
            None,
            morning(),
        )
        .await
        .expect("closed");
    assert_eq!(outcome.outcome.orphaned, Vec::new());
    assert_eq!(outcome.outcome.moved.len(), 1);
    assert_eq!(outcome.outcome.moved[0].to_number, 2);

    let shift = store.shift(bar, thursday()).await.expect("reads");
    assert_eq!(shift.blocks.len(), 1);
    assert_eq!(shift.bookings[0].table_number, Some(2));
}

#[tokio::test]
async fn closing_a_table_takes_it_out_of_the_picker_for_that_shift_only() {
    let store = store().await;
    let (bar, config) = bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    store
        .block_tables(
            bar,
            thursday(),
            &[numbered(&config.tables, 1).id],
            "Дождь",
            None,
            morning(),
        )
        .await
        .expect("closed");

    let tonight = store
        .availability(bar, thursday(), 2, morning(), None)
        .await
        .expect("reads").slots;
    assert!(
        tonight
            .iter()
            .all(|slot| slot.availability == pustol_domain::SlotAvailability::Taken),
        "the only table is shut tonight"
    );

    let tomorrow = store
        .availability(
            bar,
            thursday().checked_add_days(1).expect("in range"),
            2,
            morning(),
            None,
        )
        .await
        .expect("reads").slots;
    assert!(
        tomorrow.iter().any(|slot| slot.availability.is_free()),
        "a table shut tonight is open again tomorrow"
    );
}

#[tokio::test]
async fn closing_a_whole_zone_reseats_what_it_can_and_strands_the_rest() {
    let store = store().await;
    // Every six-top full at the same time, one of them on the terrace.
    let (bar, config) = bar_with(
        &store,
        config_with(
            vec![
                table(11, 6, "Зал"),
                table(12, 6, "Зал"),
                table(15, 6, "Терраса"),
            ],
            "anna_mgr",
        ),
    )
    .await;
    for name in ["Игорь", "Тимур", "Ксения"] {
        store
            .create_booking(&staff_booking(bar, name, 1200, 6), morning())
            .await
            .expect("free");
    }

    let terrace: Vec<_> = config
        .tables
        .iter()
        .filter(|table| table.zone.as_str() == "Терраса")
        .map(|table| table.id)
        .collect();
    let outcome = store
        .block_tables(bar, thursday(), &terrace, "Дождь", None, morning())
        .await
        .expect("closed");

    assert_eq!(outcome.outcome.moved, Vec::new(), "the room is full");
    assert_eq!(outcome.outcome.orphaned.len(), 1);

    let shift = store.shift(bar, thursday()).await.expect("reads");
    assert_eq!(
        shift
            .bookings
            .iter()
            .filter(|record| record.table_number.is_none())
            .count(),
        1,
        "one party is left owed a table"
    );
}

#[tokio::test]
async fn opening_a_table_again_seats_the_party_that_was_left_without_one() {
    let store = store().await;
    let (bar, config) = bar_with(
        &store,
        config_with(vec![table(1, 2, "Бар")], "anna_mgr"),
    )
    .await;
    let stranded = store
        .create_booking(&staff_booking(bar, "Павел", 1200, 2), morning())
        .await
        .expect("free");

    let only_table = numbered(&config.tables, 1).id;
    let closed = store
        .block_tables(bar, thursday(), &[only_table], "Сломан", None, morning())
        .await
        .expect("closed");
    assert_eq!(closed.outcome.orphaned, vec![stranded.record.booking.id]);

    let reopened = store
        .unblock_tables(bar, thursday(), &[only_table], morning())
        .await
        .expect("opened");
    assert_eq!(reopened.outcome.orphaned, Vec::new());
    assert_eq!(reopened.outcome.moved.len(), 1);
    assert_eq!(reopened.outcome.moved[0].to_number, 1);

    let shift = store.shift(bar, thursday()).await.expect("reads");
    assert!(shift.blocks.is_empty());
    assert_eq!(shift.bookings[0].table_number, Some(1));
}

#[tokio::test]
async fn closing_the_same_table_twice_on_one_shift_is_harmless() {
    let store = store().await;
    let (bar, config) = default_bar(&store).await;
    let target = numbered(&config.tables, 3).id;
    store
        .block_tables(bar, thursday(), &[target], "Дождь", None, morning())
        .await
        .expect("closed");
    store
        .block_tables(bar, thursday(), &[target], "Сломан", None, morning())
        .await
        .expect("closing again is not an error");

    let shift = store.shift(bar, thursday()).await.expect("reads");
    assert_eq!(shift.blocks.len(), 1);
    assert_eq!(
        shift.blocks[0].reason, "Дождь",
        "the first reason stands rather than being silently rewritten"
    );
}

#[tokio::test]
async fn a_reminder_is_queued_for_a_guest_booking_and_withheld_until_they_ask_for_it() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Алексей");
    store.identify(bar, &account, morning()).await.expect("ok");
    store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    // The booking starts at 18:00 UTC; the reminder is due three hours before.
    let due = utc(2026, 7, 30, 15, 0);
    assert!(
        store
            .claim_due(10, due)
            .await
            .expect("reads")
            .is_empty(),
        "nothing goes out until the guest has asked to be reminded"
    );

    store
        .choose_reminders(&account, ReminderChoice::OptIn, morning())
        .await
        .expect("opted in");
    let claimed = store.claim_due(10, due).await.expect("reads");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].kind, NotificationKind::Reminder);
    assert_eq!(claimed[0].recipient, account.id);
    assert_eq!(claimed[0].attempts, 1);
}

#[tokio::test]
async fn a_claimed_message_is_not_handed_to_a_second_worker() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Вера");
    store.identify(bar, &account, morning()).await.expect("ok");
    store.choose_reminders(&account, ReminderChoice::OptIn, morning()).await.expect("ok");
    store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    let due = utc(2026, 7, 30, 15, 0);
    let first = store.claim_due(10, due).await.expect("reads");
    assert_eq!(first.len(), 1);
    store
        .mark_sent(first[0].id, due)
        .await
        .expect("recorded");
    assert!(
        store.claim_due(10, due).await.expect("reads").is_empty(),
        "a delivered message is never offered again"
    );
}

#[tokio::test]
async fn a_reminder_is_not_due_before_its_hour() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Марк");
    store.identify(bar, &account, morning()).await.expect("ok");
    store.choose_reminders(&account, ReminderChoice::OptIn, morning()).await.expect("ok");
    store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    let too_early = utc(2026, 7, 30, 14, 59);
    assert!(store.claim_due(10, too_early).await.expect("reads").is_empty());
}

#[tokio::test]
async fn cancelling_a_booking_stops_its_reminder() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Мила");
    store.identify(bar, &account, morning()).await.expect("ok");
    store.choose_reminders(&account, ReminderChoice::OptIn, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    store
        .cancel_booking(
            bar,
            created.record.booking.id,
            Some("Технические проблемы в баре"),
            Some(common::cancellation_wording),
            morning(),
        )
        .await
        .expect("cancelled");

    let due = utc(2026, 7, 30, 15, 0);
    let claimed = store.claim_due(10, due).await.expect("reads");
    assert!(
        claimed
            .iter()
            .all(|message| message.kind != NotificationKind::Reminder),
        "nobody is reminded about an evening that is no longer happening"
    );
    assert_eq!(
        claimed
            .iter()
            .filter(|message| message.kind == NotificationKind::Cancelled)
            .count(),
        1,
        "but the guest is told once that it is off"
    );
}

#[tokio::test]
async fn rebooking_stops_the_reminder_for_the_booking_it_replaced() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Лиза");
    store.identify(bar, &account, morning()).await.expect("ok");
    store.choose_reminders(&account, ReminderChoice::OptIn, morning()).await.expect("ok");
    let first = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    let second = store
        .create_booking(&guest_booking(bar, &account, 1320, 2), morning())
        .await
        .expect("free");
    assert_eq!(second.replaced, Some(first.record.booking.id));

    // 16:00 UTC is three hours before the *new* booking and an hour after the old one's reminder
    // would have been due.
    let due = utc(2026, 7, 30, 17, 0);
    let claimed = store.claim_due(10, due).await.expect("reads");
    assert_eq!(claimed.len(), 1, "only the booking that still stands");
    assert_eq!(claimed[0].booking, second.record.booking.id);
}

#[tokio::test]
async fn a_reminder_whose_moment_has_already_passed_is_never_queued() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Егор");
    store.identify(bar, &account, morning()).await.expect("ok");
    store.choose_reminders(&account, ReminderChoice::OptIn, morning()).await.expect("ok");

    // Booking at 20:00 Belgrade made at 19:00 Belgrade: a three-hour warning is meaningless.
    let late = utc(2026, 7, 30, 17, 0);
    store
        .create_booking(&guest_booking(bar, &account, 1200, 2), late)
        .await
        .expect("free");
    assert!(
        store
            .claim_due(10, late + Duration::hours(1))
            .await
            .expect("reads")
            .is_empty()
    );
}

#[tokio::test]
async fn a_reminder_is_withheld_from_a_guest_the_bot_cannot_reach() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Настя");
    store.identify(bar, &account, morning()).await.expect("ok");
    store.choose_reminders(&account, ReminderChoice::OptIn, morning()).await.expect("ok");
    store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    store
        .set_reachable(account.id, false)
        .await
        .expect("the bot was refused");
    let due = utc(2026, 7, 30, 15, 0);
    assert!(store.claim_due(10, due).await.expect("reads").is_empty());

    store.set_reachable(account.id, true).await.expect("and again");
    assert_eq!(store.claim_due(10, due).await.expect("reads").len(), 1);
}

#[tokio::test]
async fn a_staff_message_goes_out_regardless_of_the_reminder_preference() {
    // The bar's own messages are sent because a member of staff pressed send. They are not
    // reminders and are not governed by the reminder opt-in.
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Катя");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");

    store
        .send_template(
            bar,
            created.record.booking.id,
            account.id,
            "Ваш стол готов, ждём вас!",
            morning(),
        )
        .await
        .expect("queued");

    let claimed = store.claim_due(10, morning()).await.expect("reads");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].kind, NotificationKind::StaffMessage);
    assert_eq!(claimed[0].body, "Ваш стол готов, ждём вас!");
}

#[tokio::test]
async fn a_deferred_message_comes_back_when_its_retry_falls_due() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Соня");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    let id = store
        .send_template(
            bar,
            created.record.booking.id,
            account.id,
            "Опаздываете? Держим стол ещё 15 минут.",
            morning(),
        )
        .await
        .expect("queued");

    let claimed = store.claim_due(10, morning()).await.expect("reads");
    assert_eq!(claimed.len(), 1);
    let retry_at = morning() + Duration::minutes(5);
    store
        .defer(id, retry_at, "429 too many requests")
        .await
        .expect("deferred");

    assert!(store.claim_due(10, morning()).await.expect("reads").is_empty());
    let again = store.claim_due(10, retry_at).await.expect("reads");
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].attempts, 2, "attempts accumulate across retries");
}

#[tokio::test]
async fn a_message_given_up_on_is_never_offered_again() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Данила");
    store.identify(bar, &account, morning()).await.expect("ok");
    let created = store
        .create_booking(&guest_booking(bar, &account, 1200, 2), morning())
        .await
        .expect("free");
    let id = store
        .send_template(
            bar,
            created.record.booking.id,
            account.id,
            "Ваш стол готов, ждём вас!",
            morning(),
        )
        .await
        .expect("queued");

    store
        .claim_due(10, morning())
        .await
        .expect("reads");
    store
        .give_up(id, morning(), "403 bot was blocked by the user")
        .await
        .expect("gave up");
    assert!(
        store
            .claim_due(10, morning() + Duration::days(1))
            .await
            .expect("reads")
            .is_empty()
    );
}

#[tokio::test]
async fn the_reminder_prompt_is_shown_once_and_then_left_alone() {
    let store = store().await;
    let (bar, _) = default_bar(&store).await;
    let account = fresh_account("Полина");
    let viewer = store
        .identify(bar, &account, morning())
        .await
        .expect("identified");
    assert!(viewer.reminders.should_ask());

    store
        .choose_reminders(&account, ReminderChoice::NotNow, morning())
        .await
        .expect("not now");
    let after = store
        .identify(bar, &account, morning())
        .await
        .expect("identified");
    assert!(!after.reminders.should_ask());
    assert!(!after.reminders.opted_in);

    store.choose_reminders(&account, ReminderChoice::OptIn, morning()).await.expect("ok");
    let opted = store
        .identify(bar, &account, morning())
        .await
        .expect("identified");
    assert!(opted.reminders.opted_in);
    assert!(!opted.reminders.should_ask());
}
