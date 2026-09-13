//! The staff side, over HTTP.

mod common;

use common::{Caller, config_with, draft_from, harness, harness_at, morning, table};

/// The manager, recognised by the username the fixture bar invited.
async fn manager(app: &common::Harness) -> Caller {
    let manager = Caller::manager();
    let session = app.get("/api/session", &manager).await;
    assert_eq!(
        session.expect_ok()["is_staff"], true,
        "the invited username claims its seat on first sight"
    );
    manager
}

#[tokio::test]
async fn an_invited_username_is_recognised_on_its_first_visit() {
    let app = harness().await;
    let _ = manager(&app).await;
}

const SETTINGS: &str = "/api/admin/settings?service_date=2026-07-30";
const SHIFT: &str = "/api/admin/shift?service_date=2026-07-30";

#[tokio::test]
async fn a_payload_signed_before_a_seat_was_offered_does_not_claim_it() {
    // A payload is accepted for an hour, and the username in it is whatever the account was called
    // when Telegram signed. The name may have been somebody else's by the time the seat was offered.
    let app = harness().await;
    let staff = manager(&app).await;
    let newcomer = Caller::new("Паша");
    let signed_before = newcomer.credentials(morning());

    let invited = app.at(morning() + chrono::TimeDelta::minutes(10));
    let settings = invited.get(SETTINGS, &staff).await.expect_ok().clone();
    let mut draft = draft_from(&settings);
    draft["staff"]
        .as_array_mut()
        .expect("staff")
        .push(serde_json::json!({ "username": newcomer.username }));
    invited.send("PUT", SETTINGS, &staff, draft).await.expect_ok();

    let answer = invited.send_raw("GET", SHIFT, Some(&signed_before), None).await;
    assert_eq!(answer.status, axum::http::StatusCode::FORBIDDEN, "{}", answer.body);
    let seat = invited.get(SETTINGS, &staff).await.expect_ok()["staff"]
        .as_array()
        .expect("staff")
        .iter()
        .find(|member| member["username"] == serde_json::json!(newcomer.username))
        .expect("invited")
        .clone();
    assert_eq!(seat["bound"], false, "{seat}");

    app.at(morning() + chrono::TimeDelta::minutes(11))
        .get(SHIFT, &newcomer)
        .await
        .expect_ok();
}

#[tokio::test]
async fn a_save_from_settings_another_manager_has_changed_since_is_refused_and_theirs_stands() {
    let app = harness().await;
    let staff = manager(&app).await;
    let settings = app.get(SETTINGS, &staff).await.expect_ok().clone();
    assert!(settings["version"].is_string(), "{settings}");

    let mut renamed = draft_from(&settings);
    renamed["name"] = serde_json::json!("Бар «Чердак»");
    let mut capped = draft_from(&settings);
    capped["max_party"] = serde_json::json!(4);

    let saved = app.send("PUT", SETTINGS, &staff, renamed).await.expect_ok().clone();
    let refused = app.send("PUT", SETTINGS, &staff, capped).await;
    assert_eq!(refused.status, axum::http::StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(refused.error_code(), Some("settings_changed"));

    let current = app.get(SETTINGS, &staff).await.expect_ok().clone();
    assert_eq!(current["name"], "Бар «Чердак»", "the other manager's change stands");
    assert_eq!(current["max_party"], 6, "nothing of the refused save was written");
    assert_eq!(current["version"], saved["settings"]["version"]);
    assert_ne!(current["version"], settings["version"]);

    let mut fresh = draft_from(&current);
    fresh["max_party"] = serde_json::json!(4);
    let saved = app.send("PUT", SETTINGS, &staff, fresh).await.expect_ok().clone();
    assert_eq!(saved["settings"]["max_party"], 4);
    assert_eq!(saved["settings"]["name"], "Бар «Чердак»");
}

#[tokio::test]
async fn a_stranger_who_takes_over_an_invited_username_gets_nothing() {
    // A Telegram username can be released and claimed by somebody else. Once a seat carries a
    // numeric account it is that person's, and the username is only a label.
    let app = harness().await;
    let real = manager(&app).await;
    let squatter = Caller {
        id: real.id + 1_000_000,
        first_name: "Не Анна".to_owned(),
        username: Some("anna_mgr".to_owned()),
    };
    let session = app.get("/api/session", &squatter).await;
    assert_eq!(session.expect_ok()["is_staff"], false);
    assert_eq!(
        app.get("/api/admin/shift?service_date=2026-07-30", &squatter)
            .await
            .status,
        axum::http::StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn the_shift_view_carries_the_room_the_bookings_and_the_stats() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Алексей");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 4 }),
    )
    .await
    .expect_ok();

    let body = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();

    assert_eq!(body["service_date"], "2026-07-30");
    assert_eq!(body["hours"]["open_minutes"], 600);
    assert_eq!(body["tables"].as_array().expect("tables").len(), 15);
    assert_eq!(body["bookings"].as_array().expect("bookings").len(), 1);
    assert_eq!(body["stats"]["bookings"], 1);
    assert_eq!(body["stats"]["guests"], 4);
    // Early morning: the whole room is free, and the "now" line is on today's shift.
    assert_eq!(body["stats"]["free_now"], 15);
    assert!(body["now_minutes"].is_number());

    let booking = &body["bookings"][0];
    assert_eq!(booking["guest_name"], "Алексей");
    assert_eq!(booking["table_number"], 5, "the smallest table that fits");
    assert_eq!(booking["source"], "app");
    assert_eq!(booking["reachable_by_bot"], true);
    assert_eq!(booking["status"], "confirmed");

    // The bar's own lists travel with the shift so the sheets need no second request.
    assert_eq!(
        body["cancel_reasons"].as_array().expect("reasons").len(),
        2
    );
    assert_eq!(
        body["message_templates"].as_array().expect("messages").len(),
        2
    );
}

#[tokio::test]
async fn free_now_is_blank_on_a_shift_that_is_not_running() {
    let app = harness().await;
    let staff = manager(&app).await;
    let body = app
        .get("/api/admin/shift?service_date=2026-08-05", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        body["stats"]["free_now"],
        serde_json::Value::Null,
        "\"free now\" has no meaning on next Wednesday, and a number would be a lie"
    );
    assert_eq!(body["now_minutes"], serde_json::Value::Null);
}

#[tokio::test]
async fn staff_take_a_booking_at_the_door_and_the_bot_has_no_chat_with_that_guest() {
    let app = harness().await;
    let staff = manager(&app).await;
    let created = app
        .post(
            "/api/admin/bookings",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "start_minutes": 1290,
                "party_size": 3,
                "guest_name": "  Полина  "
            }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(created["booking"]["guest_name"], "Полина", "trimmed on the way in");
    assert_eq!(created["booking"]["source"], "staff");
    assert_eq!(created["booking"]["reachable_by_bot"], false);

    let refused = app
        .post(
            &format!("/api/admin/bookings/{}/message", created["booking"]["id"].as_str().unwrap()),
            &staff,
            serde_json::json!({ "text": "Ваш стол готов, ждём вас!" }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("no_bot_chat"));
}

#[tokio::test]
async fn a_booking_needs_a_name_to_call_out() {
    let app = harness().await;
    let staff = manager(&app).await;
    let refused = app
        .post(
            "/api/admin/bookings",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "start_minutes": 1200,
                "party_size": 2,
                "guest_name": "   "
            }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("blank_guest_name"));
}

#[tokio::test]
async fn staff_are_not_bound_by_the_guest_booking_horizon() {
    let app = harness().await;
    let staff = manager(&app).await;
    app.post(
        "/api/admin/bookings",
        &staff,
        serde_json::json!({
            "service_date": "2026-09-15",
            "start_minutes": 1200,
            "party_size": 2,
            "guest_name": "Телефонный гость"
        }),
    )
    .await
    .expect_ok();
}

#[tokio::test]
async fn staff_record_whether_a_party_turned_up() {
    let app = harness().await;
    let staff = manager(&app).await;
    let created = app
        .post(
            "/api/admin/bookings",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "start_minutes": 1200,
                "party_size": 2,
                "guest_name": "Марина"
            }),
        )
        .await
        .expect_ok()
        .clone();
    let id = created["booking"]["id"].as_str().expect("an id");

    for (attendance, previous) in [("arrived", "confirmed"), ("no_show", "arrived"), ("confirmed", "no_show")] {
        let updated = app
            .send(
                "PATCH",
                &format!("/api/admin/bookings/{id}/attendance"),
                &staff,
                serde_json::json!({ "attendance": attendance }),
            )
            .await;
        assert_eq!(updated.expect_ok()["booking"]["status"], attendance);
        assert_eq!(updated.expect_ok()["previous"], previous, "what undo goes back to");
    }

    // A status endpoint that could also cancel would let a mis-tap free a table with no reason
    // attached, so the value is not accepted at all.
    let refused = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "cancelled" }),
        )
        .await;
    assert_eq!(refused.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn undo_learns_what_the_booking_was_from_the_server_not_from_a_screen_that_missed_a_change() {
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let id = booked(&app, &staff, 1_200, "Марина").await;
    let attendance = format!("/api/admin/bookings/{id}/attendance");
    let mark = |status: &'static str| {
        let app = &app;
        let staff = &staff;
        let attendance = attendance.clone();
        async move {
            app.send("PATCH", &attendance, staff, serde_json::json!({ "attendance": status }))
                .await
                .expect_ok()
                .clone()
        }
    };

    // The bar screen saw them arrive. The door screen then marked them as not coming, and the bar
    // screen, still showing «Пришли», marks them gone.
    mark("arrived").await;
    mark("no_show").await;
    let gone = mark("left").await;

    assert_eq!(gone["previous"], "no_show", "{gone}");
}

async fn cancelled_notices(app: &common::Harness, booking: &str) -> i64 {
    sqlx::query_scalar(
        "select count(*) from notification where booking_id = $1::uuid and kind = 'cancelled'",
    )
    .bind(booking)
    .fetch_one(app.store.pool())
    .await
    .expect("counted")
}

#[tokio::test]
async fn an_evening_that_is_over_cannot_be_cancelled_and_nobody_is_told_it_was() {
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let guest = Caller::new("Вера");
    let booking = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let attendance = format!("/api/admin/bookings/{booking}/attendance");
    let sitting = app.at(common::utc(2026, 7, 30, 18, 30));
    for status in ["arrived", "left"] {
        sitting
            .send("PATCH", &attendance, &staff, serde_json::json!({ "attendance": status }))
            .await
            .expect_ok();
    }

    for later in [common::utc(2026, 7, 30, 18, 45), common::utc(2026, 7, 30, 20, 30)] {
        let refused = app
            .at(later)
            .post(
                &format!("/api/admin/bookings/{booking}/cancel"),
                &staff,
                serde_json::json!({ "reason": "Частное мероприятие" }),
            )
            .await;
        assert_eq!(refused.status, axum::http::StatusCode::CONFLICT, "{later}: {}", refused.body);
        assert_eq!(refused.error_code(), Some("booking_finished"), "{later}");
    }
    assert_eq!(cancelled_notices(&app, &booking).await, 0, "no notice about an evening that happened");
}

#[tokio::test]
async fn a_party_still_at_their_table_can_be_cancelled() {
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let guest = Caller::new("Вера");
    let booking = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let sitting = app.at(common::utc(2026, 7, 30, 18, 30));
    sitting
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{booking}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "arrived" }),
        )
        .await
        .expect_ok();

    let cancelled = sitting
        .post(
            &format!("/api/admin/bookings/{booking}/cancel"),
            &staff,
            serde_json::json!({ "reason": "Технические проблемы в баре" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(cancelled["booking"]["status"], "cancelled");
    assert_eq!(cancelled_notices(&app, &booking).await, 1);
}

#[tokio::test]
async fn staff_cancel_with_one_of_the_bars_reasons_and_the_guest_is_told() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Вера");
    let booking = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let cancelled = app
        .post(
            &format!("/api/admin/bookings/{booking}/cancel"),
            &staff,
            serde_json::json!({ "reason": "Частное мероприятие" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(cancelled["booking"]["status"], "cancelled");
    assert_eq!(cancelled["guest_notified"], true);

    // And the guest sees it gone.
    assert_eq!(
        app.get("/api/session", &guest).await.expect_ok()["bookings"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn a_reason_the_bar_never_configured_is_refused() {
    // Free text here would turn a borrowed staff account into a way to send anything to every guest
    // who has ever booked.
    let app = harness().await;
    let staff = manager(&app).await;
    let created = app
        .post(
            "/api/admin/bookings",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "start_minutes": 1200,
                "party_size": 2,
                "guest_name": "Марк"
            }),
        )
        .await
        .expect_ok()
        .clone();
    let refused = app
        .post(
            &format!("/api/admin/bookings/{}/cancel", created["booking"]["id"].as_str().unwrap()),
            &staff,
            serde_json::json!({ "reason": "перейдите по ссылке http://example.invalid" }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("unknown_cancel_reason"));
}

#[tokio::test]
async fn a_message_the_bar_never_configured_is_refused() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Катя");
    let booking = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let refused = app
        .post(
            &format!("/api/admin/bookings/{booking}/message"),
            &staff,
            serde_json::json!({ "text": "купите наш курс" }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("unknown_message"));

    let queued = app
        .post(
            &format!("/api/admin/bookings/{booking}/message"),
            &staff,
            serde_json::json!({ "text": "Ваш стол готов, ждём вас!" }),
        )
        .await;
    assert_eq!(queued.expect_ok()["queued"], true);
}

#[tokio::test]
async fn closing_a_table_moves_its_party_and_names_them_in_the_report() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Анна К.");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let first_table = shift["tables"][0]["id"].as_str().expect("an id").to_owned();

    let report = app
        .post(
            "/api/admin/blocks",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "table_ids": [first_table],
                "reason": "Сломан / залит"
            }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(report["reconciliation"]["moved"].as_array().expect("moved").len(), 1);
    assert_eq!(report["reconciliation"]["moved"][0]["guest_name"], "Анна К.");
    assert_eq!(report["reconciliation"]["moved"][0]["to_number"], 2);
    assert!(report["reconciliation"]["orphaned"].as_array().expect("orphaned").is_empty());

    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(shift["tables"][0]["blocked_because"], "Сломан / залит");
    assert_eq!(shift["stats"]["free_now"], 14);
}

#[tokio::test]
async fn closing_a_table_needs_a_reason() {
    let app = harness().await;
    let staff = manager(&app).await;
    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let refused = app
        .post(
            "/api/admin/blocks",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "table_ids": [shift["tables"][0]["id"]],
                "reason": "   "
            }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("missing_block_reason"));
}

#[tokio::test]
async fn a_party_the_room_cannot_take_is_reported_and_seated_when_a_table_reopens() {
    let app = harness_at(morning(), config_with(vec![table(1, 2, "Бар")])).await;
    let staff = manager(&app).await;
    let guest = Caller::new("Павел");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let only_table = shift["tables"][0]["id"].as_str().expect("an id").to_owned();

    let closed = app
        .post(
            "/api/admin/blocks",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "table_ids": [only_table],
                "reason": "Дождь"
            }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(closed["reconciliation"]["orphaned"].as_array().expect("orphaned").len(), 1);
    assert_eq!(closed["reconciliation"]["orphaned"][0]["guest_name"], "Павел");

    // The guest is never told, and their booking still reads as confirmed to them.
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["bookings"][0]["status"], "confirmed");

    let reopened = app
        .send(
            "DELETE",
            "/api/admin/blocks",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "table_ids": [only_table] }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(reopened["reconciliation"]["moved"].as_array().expect("moved").len(), 1);
    assert_eq!(reopened["reconciliation"]["moved"][0]["guest_name"], "Павел");
}

#[tokio::test]
async fn asking_the_room_to_try_again_says_plainly_when_there_is_still_nowhere() {
    let app = harness_at(morning(), config_with(vec![table(1, 2, "Бар")])).await;
    let staff = manager(&app).await;
    let guest = Caller::new("Тимур");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();
    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    app.post(
        "/api/admin/blocks",
        &staff,
        serde_json::json!({
            "service_date": "2026-07-30",
            "table_ids": [shift["tables"][0]["id"]],
            "reason": "Дождь"
        }),
    )
    .await
    .expect_ok();

    let retried = app
        .post(
            "/api/admin/shift/reconcile",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30" }),
        )
        .await
        .expect_ok()
        .clone();
    assert!(retried["reconciliation"]["moved"].as_array().expect("moved").is_empty());
    assert_eq!(retried["reconciliation"]["orphaned"][0]["guest_name"], "Тимур");
}

// ---- settings ---------------------------------------------------------------------------------

#[tokio::test]
async fn the_settings_screen_arrives_with_the_bounds_every_control_must_respect() {
    let app = harness().await;
    let staff = manager(&app).await;
    let body = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();

    assert_eq!(body["name"], "Бар «Подвал»");
    assert_eq!(body["tables"].as_array().expect("tables").len(), 15);
    assert_eq!(body["next_table_number"], 16);
    assert_eq!(body["week"].as_array().expect("week").len(), 7);
    assert_eq!(body["limits"]["turn_minutes"]["min"], 60);
    assert_eq!(body["limits"]["turn_minutes"]["max"], 240);
    assert_eq!(body["limits"]["seats"]["max"], 12);
    assert_eq!(body["limits"]["slot_step_minutes"], serde_json::json!([15, 30, 60]));
    assert_eq!(body["staff"][0]["username"], "anna_mgr");
    assert_eq!(body["staff"][0]["bound"], true, "claimed on first sight");
    assert_eq!(
        body["service_date"], "2026-07-30",
        "the evening each table's booking count is for"
    );
}

#[tokio::test]
async fn saving_the_settings_unchanged_changes_nothing() {
    let app = harness().await;
    let staff = manager(&app).await;
    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let saved = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft_from(&settings),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(saved["above_cap"], 0);
    assert!(saved["reconciliation"]["moved"].as_array().expect("moved").is_empty());
    assert_eq!(saved["settings"]["name"], settings["name"]);
    assert_eq!(saved["settings"]["service_date"], "2026-07-30");
}

#[tokio::test]
async fn adding_a_table_takes_the_next_number_and_leaves_retired_numbers_alone() {
    let app = harness().await;
    let staff = manager(&app).await;
    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();

    // Remove the fifteenth and add one: the new table must be sixteen.
    let mut draft = draft_from(&settings);
    let tables = draft["tables"].as_array_mut().expect("tables");
    tables.pop();
    tables.push(serde_json::json!({ "id": uuid::Uuid::new_v4(), "seats": 6, "zone": "Зал" }));

    let saved = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft,
        )
        .await
        .expect_ok()
        .clone();
    let numbers: Vec<i64> = saved["settings"]["tables"]
        .as_array()
        .expect("tables")
        .iter()
        .map(|table| table["number"].as_i64().expect("a number"))
        .collect();
    assert!(!numbers.contains(&15), "the fifteenth was retired");
    assert!(numbers.contains(&16));
    assert_eq!(saved["settings"]["next_table_number"], 17);
}

#[tokio::test]
async fn closing_a_day_that_has_bookings_is_refused_and_names_them() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Олег");
    let booking = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let mut draft = draft_from(&settings);
    // Thursday, index 4 counting from Sunday.
    draft["week"][4]["closed"] = serde_json::json!(true);

    let refused = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft,
        )
        .await;
    assert_eq!(refused.status, axum::http::StatusCode::CONFLICT);
    assert_eq!(refused.error_code(), Some("would_strand_bookings"));
    assert_eq!(
        refused.body["error"]["detail"]["conflicts"][0]["booking_id"],
        booking
    );
}

#[tokio::test]
async fn a_party_cap_no_table_can_seat_is_refused_with_the_reason_spelled_out() {
    let app = harness().await;
    let staff = manager(&app).await;
    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let mut draft = draft_from(&settings);
    draft["max_party"] = serde_json::json!(10);

    let refused = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft,
        )
        .await;
    assert_eq!(
        refused.status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(refused.error_code(), Some("settings_invalid"));
    let reasons = refused.body["error"]["detail"]["reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .map(|reason| reason.as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(reasons.contains("largest table"), "got {reasons}");
}

#[tokio::test]
async fn removing_the_last_admin_is_refused() {
    let app = harness().await;
    let staff = manager(&app).await;
    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let mut draft = draft_from(&settings);
    draft["staff"] = serde_json::json!([]);

    let refused = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft,
        )
        .await;
    assert_eq!(refused.error_code(), Some("settings_invalid"));
}

#[tokio::test]
async fn a_proposal_naming_a_table_of_another_bar_is_refused_as_unreadable() {
    let app = harness().await;
    let staff = manager(&app).await;
    let other = pustol_domain::ValidConfig::new(config_with(common::default_tables()))
        .expect("legal");
    app.store
        .create_bar(&other, morning())
        .await
        .expect("another bar on the same database");
    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let mut draft = draft_from(&settings);
    draft["tables"].as_array_mut().expect("tables").push(serde_json::json!({
        "id": other.tables[0].id.0,
        "seats": 4,
        "zone": "Зал"
    }));

    let refused = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft,
        )
        .await;
    assert_eq!(refused.status, axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(refused.error_code(), Some("settings_unreadable"));
}

#[tokio::test]
async fn lengthening_the_turn_leaves_the_bookings_already_taken_alone() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Саша");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let mut draft = draft_from(&settings);
    draft["turn_minutes"] = serde_json::json!(240);
    app.send(
        "PUT",
        "/api/admin/settings?service_date=2026-07-30",
        &staff,
        draft,
    )
    .await
    .expect_ok();

    // The guest keeps the two hours they were promised.
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["bookings"][0]["start_minutes"], 1200);
    assert_eq!(session["bookings"][0]["end_minutes"], 1320);
    // The next guest gets the new length.
    let next = Caller::new("Юля");
    let taken = app
        .post(
            "/api/booking",
            &next,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 4 }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(taken["booking"]["end_minutes"], 1440);
}

#[tokio::test]
async fn lowering_the_party_cap_reports_the_bookings_above_it_without_refusing() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Артур");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 6 }),
    )
    .await
    .expect_ok();

    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let mut draft = draft_from(&settings);
    draft["max_party"] = serde_json::json!(4);
    let saved = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft,
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(saved["above_cap"], 1);
}

#[tokio::test]
async fn retiring_a_table_moves_its_party_and_reports_it_by_name() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Ксения");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 6 }),
    )
    .await
    .expect_ok();

    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    // The party of six is on table 11, the first six-top.
    let seated = settings["tables"]
        .as_array()
        .expect("tables")
        .iter()
        .find(|table| table["number"] == 11)
        .expect("a six-top")
        .clone();
    assert_eq!(seated["bookings_today"], 1, "the badge staff see before deleting");

    let mut draft = draft_from(&settings);
    draft["tables"]
        .as_array_mut()
        .expect("tables")
        .retain(|table| table["id"] != seated["id"]);
    let saved = app
        .send(
            "PUT",
            "/api/admin/settings?service_date=2026-07-30",
            &staff,
            draft,
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(saved["reconciliation"]["moved"][0]["guest_name"], "Ксения");
    assert_eq!(saved["reconciliation"]["moved"][0]["to_number"], 12);
}

#[tokio::test]
async fn the_shift_says_who_could_be_seated_right_now() {
    // Eight in the evening, an empty room whose largest table seats six.
    let app = harness_at(common::utc(2026, 7, 30, 18, 0), config_with(vec![
        table(1, 2, "Бар"),
        table(2, 6, "Зал"),
    ]))
    .await;
    let staff = manager(&app).await;

    let body = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(body["largest_party_seatable_now"], 6);

    // Seat six of them and only the two-top is left.
    app.post(
        "/api/admin/walkins",
        &staff,
        serde_json::json!({ "service_date": "2026-07-30", "party_size": 6 }),
    )
    .await
    .expect_ok();
    let body = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(body["largest_party_seatable_now"], 2);
    assert_eq!(body["stats"]["free_now"], 1);
}

#[tokio::test]
async fn a_walk_in_is_seated_at_the_minute_they_sat_down() {
    let app = harness_at(
        common::utc(2026, 7, 30, 18, 7),
        config_with(vec![table(1, 2, "Бар"), table(2, 6, "Зал")]),
    )
    .await;
    let staff = manager(&app).await;

    let seated = app
        .post(
            "/api/admin/walkins",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "party_size": 2 }),
        )
        .await
        .expect_ok()
        .clone();

    assert_eq!(seated["booking"]["table_number"], 1, "the smallest table that fits");
    assert_eq!(seated["booking"]["source"], "walk");
    assert_eq!(seated["booking"]["status"], "arrived");
    assert_eq!(seated["booking"]["guest_name"], "Без брони");
    assert_eq!(
        seated["booking"]["start_minutes"], 1_207,
        "20:07 local, not floored onto the half-hour grid"
    );
    assert_eq!(seated["booking"]["reachable_by_bot"], false);
}

/// Takes a booking by telephone on the fixture Thursday and answers with its identifier.
async fn booked(app: &common::Harness, staff: &Caller, minutes: i64, name: &str) -> String {
    app.post(
        "/api/admin/bookings",
        staff,
        serde_json::json!({
            "service_date": "2026-07-30",
            "start_minutes": minutes,
            "party_size": 2,
            "guest_name": name,
        }),
    )
    .await
    .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an identifier")
        .to_owned()
}

#[tokio::test]
async fn staff_move_a_booking_to_the_table_and_the_time_they_choose() {
    // The allocator gave them the first four-top. The room the allocator cannot see says the
    // corner, and the guest telephoned to ask for half an hour later.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 4, "Бар"), table(2, 4, "Зал")]),
    )
    .await;
    let staff = manager(&app).await;
    let id = booked(&app, &staff, 1_200, "Глеб").await;
    let corner = table_id(&app, &staff, 2).await;

    let moved = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/move"),
            &staff,
            serde_json::json!({ "start_minutes": 1_320, "table_id": corner }),
        )
        .await
        .expect_ok()
        .clone();

    assert_eq!(moved["booking"]["table_number"], 2);
    assert_eq!(moved["booking"]["start_minutes"], 1_320);
    assert_eq!(moved["booking"]["end_minutes"], 1_440);
    assert_eq!(
        moved["guest_notified"], false,
        "a booking taken over the telephone has no account to write to"
    );

    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(shift["bookings"][0]["table_number"], 2);
    assert_eq!(shift["bookings"][0]["start_minutes"], 1_320);
}

#[tokio::test]
async fn the_times_offered_for_a_move_do_not_count_the_booking_being_moved() {
    // One table, one booking. Moving it by half an hour must not mean giving up its table first
    // and hoping: the times staff are offered are the times with this booking set aside.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 4, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let id = booked(&app, &staff, 1_200, "Вера").await;

    let state = |body: &serde_json::Value, minutes: i64| {
        body["slots"]
            .as_array()
            .expect("slots")
            .iter()
            .find(|slot| slot["start_minutes"] == minutes)
            .map(|slot| slot["state"].as_str().expect("a state").to_owned())
    };

    let plain = app
        .get(
            "/api/admin/availability?service_date=2026-07-30&party_size=2",
            &staff,
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(state(&plain, 1_200).as_deref(), Some("taken"));

    let moving = app
        .get(
            &format!("/api/admin/availability?service_date=2026-07-30&party_size=2&ignoring={id}"),
            &staff,
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(state(&moving, 1_200).as_deref(), Some("free"));
    assert_eq!(state(&moving, 1_320).as_deref(), Some("free"));
}

#[tokio::test]
async fn a_booking_that_has_started_keeps_its_time_and_can_still_change_table() {
    // 20:30 Belgrade: the 20:00 booking is under way. Its window is history now, and history is
    // not rewritten; where they sit for the rest of it still is.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 4, "Бар"), table(2, 4, "Зал")]),
    )
    .await;
    let staff = manager(&app).await;
    let id = booked(&app, &staff, 1_200, "Тимур").await;
    let corner = table_id(&app, &staff, 2).await;
    let under_way = app.at(common::utc(2026, 7, 30, 18, 30));

    let refused = under_way
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/move"),
            &staff,
            serde_json::json!({ "start_minutes": 1_320, "table_id": corner }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("booking_started"));

    let moved = under_way
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/move"),
            &staff,
            serde_json::json!({ "start_minutes": 1_200, "table_id": corner }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(moved["booking"]["table_number"], 2);
}

#[tokio::test]
async fn staff_seat_a_walk_in_at_the_table_they_picked_themselves() {
    // The room would offer the two-top. A bartender who can see the couple asking for the corner
    // puts them at the six-top instead, and the shift then reads the way the room looks.
    let app = harness_at(
        common::utc(2026, 7, 30, 18, 7),
        config_with(vec![table(1, 2, "Бар"), table(2, 6, "Зал")]),
    )
    .await;
    let staff = manager(&app).await;
    let six_top = table_id(&app, &staff, 2).await;

    let seated = app
        .post(
            "/api/admin/walkins",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "party_size": 2,
                "table_id": six_top,
            }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(seated["booking"]["table_number"], 2);

    let refused = app
        .post(
            "/api/admin/walkins",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30",
                "party_size": 2,
                "table_id": six_top,
            }),
        )
        .await;
    assert_eq!(
        refused.error_code(),
        Some("chosen_table_not_free"),
        "the table they are looking at is gone, which is not the same as the room being full"
    );
}

/// The identifier the shift gives for a printed table number, so a test names tables the way staff
/// do rather than carrying a UUID through a fixture.
async fn table_id(app: &common::Harness, staff: &Caller, number: i64) -> String {
    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", staff)
        .await
        .expect_ok()
        .clone();
    shift["tables"]
        .as_array()
        .expect("the shift lists its tables")
        .iter()
        .find(|table| table["number"] == number)
        .unwrap_or_else(|| panic!("no table {number} in the room"))["id"]
        .as_str()
        .expect("an identifier")
        .to_owned()
}

#[tokio::test]
async fn seating_somebody_now_is_refused_on_an_evening_that_is_not_tonight() {
    let app = harness_at(
        common::utc(2026, 7, 30, 18, 0),
        config_with(vec![table(1, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let refused = app
        .post(
            "/api/admin/walkins",
            &staff,
            serde_json::json!({ "service_date": "2026-08-01", "party_size": 2 }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("not_the_running_shift"));
}

#[tokio::test]
async fn a_party_that_leaves_hands_its_table_back_to_the_room_at_once() {
    let app = harness_at(
        common::utc(2026, 7, 30, 18, 0),
        config_with(vec![table(1, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let seated = app
        .post(
            "/api/admin/walkins",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "party_size": 2 }),
        )
        .await
        .expect_ok()
        .clone();
    let id = seated["booking"]["id"].as_str().expect("an identifier").to_owned();

    let before = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(before["stats"]["free_now"], 0);
    assert!(before["largest_party_seatable_now"].is_null());

    let gone = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "left" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(gone["booking"]["status"], "left");
    assert_eq!(
        gone["booking"]["released_minutes"], 1_200,
        "the table goes back into the pool at the minute they left"
    );

    let after = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(after["stats"]["free_now"], 1);
    assert_eq!(after["largest_party_seatable_now"], 2);

    // Undo: the room goes back to exactly where it was.
    let back = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "arrived" }),
        )
        .await
        .expect_ok()
        .clone();
    assert!(back["booking"]["released_minutes"].is_null());
    let restored = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(restored["stats"]["free_now"], 0);
    assert!(restored["largest_party_seatable_now"].is_null());
}

#[tokio::test]
async fn a_note_belongs_to_the_shift_and_never_reaches_the_guest() {
    let app = harness().await;
    let staff = manager(&app).await;
    let guest = Caller::new("Тимур");
    let booked = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()
        .clone();
    let id = booked["booking"]["id"].as_str().expect("an id").to_owned();

    let noted = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/note"),
            &staff,
            serde_json::json!({ "note": "День рождения" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(noted["booking"]["note"], "День рождения");

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert!(
        session["bookings"][0].get("note").is_none(),
        "a guest's own booking has no field a note could travel in"
    );

    let rubbed = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/note"),
            &staff,
            serde_json::json!({ "note": null }),
        )
        .await
        .expect_ok()
        .clone();
    assert!(rubbed["booking"]["note"].is_null());

    let refused = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/note"),
            &staff,
            serde_json::json!({ "note": "я".repeat(121) }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("note_too_long"));
}

#[tokio::test]
async fn the_shift_reaches_a_month_ahead_whatever_the_guest_horizon_is() {
    let app = harness().await;
    let staff = manager(&app).await;
    let body = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();

    let days = body["days"].as_array().expect("a day sheet");
    assert_eq!(days.len(), 30);
    assert_eq!(days[0]["service_date"], "2026-07-30");
    assert_eq!(days[29]["service_date"], "2026-08-28");
    assert_eq!(
        body["guest_horizon_days"], 4,
        "so the sheet can say where the guest's own horizon ends"
    );
}

#[tokio::test]
async fn every_admin_route_refuses_a_request_with_no_credentials() {
    let app = harness().await;
    for path in [
        "/api/admin/shift?service_date=2026-07-30",
        "/api/admin/settings?service_date=2026-07-30",
        "/api/admin/availability?service_date=2026-07-30&party_size=2",
    ] {
        assert_eq!(
            app.get_anonymously(path).await.status,
            axum::http::StatusCode::UNAUTHORIZED,
            "{path} answered without credentials"
        );
    }
}

#[tokio::test]
async fn staff_grow_a_party_over_the_telephone_without_cancelling_anything() {
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 4, "Зал")]),
    )
    .await;
    let staff = manager(&app).await;
    let id = booked(&app, &staff, 1_200, "Глеб").await;

    let grown = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/move"),
            &staff,
            serde_json::json!({ "start_minutes": 1_200, "table_id": null, "party_size": 4 }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(grown["booking"]["party_size"], 4);
    assert_eq!(grown["booking"]["table_number"], 2);
    assert_eq!(grown["booking"]["status"], "confirmed", "the booking stands; nothing was cancelled");
}

/// The booking with this id in a shift the API answered with, if it is there.
fn in_shift<'a>(shift: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    shift["bookings"]
        .as_array()
        .expect("the shift lists its bookings")
        .iter()
        .find(|booking| booking["id"] == id)
}

#[tokio::test]
async fn every_write_to_a_booking_answers_with_the_evening_as_it_now_stands() {
    // A screen that reloads the shift after every tap shows the room as it was between the write and
    // the read, and as nobody saw it; the answer to the write is the room the write left.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let second = table_id(&app, &staff, 2).await;

    let created = app
        .post(
            "/api/admin/bookings",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2, "guest_name": "Глеб" }),
        )
        .await
        .expect_ok()
        .clone();
    let id = created["booking"]["id"].as_str().expect("an id").to_owned();
    assert_eq!(created["shift"]["service_date"], "2026-07-30");
    assert!(in_shift(&created["shift"], &id).is_some(), "{created}");

    let noted = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/note"),
            &staff,
            serde_json::json!({ "note": "У окна" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        in_shift(&noted["shift"], &id).expect("on the shift")["note"],
        "У окна"
    );

    let marked = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "arrived" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        in_shift(&marked["shift"], &id).expect("on the shift")["status"],
        "arrived"
    );

    let moved = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/move"),
            &staff,
            serde_json::json!({ "start_minutes": 1200, "table_id": second }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        in_shift(&moved["shift"], &id).expect("on the shift")["table_number"],
        2
    );

    let cancelled = app
        .post(
            &format!("/api/admin/bookings/{id}/cancel"),
            &staff,
            serde_json::json!({ "reason": "Частное мероприятие" }),
        )
        .await
        .expect_ok()
        .clone();
    assert!(
        in_shift(&cancelled["shift"], &id).is_none(),
        "a cancelled booking is not part of the evening"
    );
}

#[tokio::test]
async fn every_write_to_the_room_answers_with_the_evening_as_it_now_stands() {
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 4, "Зал")]),
    )
    .await;
    let staff = manager(&app).await;
    let second = table_id(&app, &staff, 2).await;

    let seated = app
        .post(
            "/api/admin/walkins",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "party_size": 2 }),
        )
        .await
        .expect_ok()
        .clone();
    let walk_in = seated["booking"]["id"].as_str().expect("an id").to_owned();
    assert!(in_shift(&seated["shift"], &walk_in).is_some(), "{seated}");

    let closed = app
        .post(
            "/api/admin/blocks",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "table_ids": [second], "reason": "Дождь" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(closed["closed"], serde_json::json!([second]));
    assert_eq!(closed["shift"]["tables"][1]["blocked_because"], "Дождь");

    let reopened = app
        .send(
            "DELETE",
            "/api/admin/blocks",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "table_ids": [second] }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        reopened["reopened"],
        serde_json::json!([{ "table_id": second, "reason": "Дождь" }])
    );
    assert!(reopened["shift"]["tables"][1]["blocked_because"].is_null());

    let retried = app
        .post(
            "/api/admin/shift/reconcile",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(retried["shift"]["service_date"], "2026-07-30");
    assert!(in_shift(&retried["shift"], &walk_in).is_some());
}

#[tokio::test]
async fn the_shift_says_by_the_bars_own_clock_what_has_started_what_is_over_and_which_day_is_today()
{
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let early = booked(&app, &staff, 1_200, "Вера").await;
    let late = booked(&app, &staff, 1_320, "Глеб").await;

    // Half past eight in Belgrade.
    let evening = app.at(common::utc(2026, 7, 30, 18, 30));
    let shift = evening.get(SHIFT, &staff).await.expect_ok().clone();
    assert_eq!(shift["today"], "2026-07-30");
    let vera = in_shift(&shift, &early).expect("on the shift");
    assert_eq!(
        (vera["started"].clone(), vera["finished"].clone()),
        (serde_json::json!(true), serde_json::json!(false))
    );
    let gleb = in_shift(&shift, &late).expect("on the shift");
    assert_eq!(
        (gleb["started"].clone(), gleb["finished"].clone()),
        (serde_json::json!(false), serde_json::json!(false))
    );

    let gone = evening
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{early}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "left" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        gone["booking"]["finished"], true,
        "gone home is over, whatever was promised"
    );
    assert_eq!(
        in_shift(&gone["shift"], &early).expect("on the shift")["finished"],
        true
    );

    let next_week = evening
        .get("/api/admin/shift?service_date=2026-08-05", &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        next_week["today"], "2026-07-30",
        "the day on screen is not the day it is"
    );
    let one_in_the_morning = app
        .at(common::utc(2026, 7, 30, 23, 0))
        .get(SHIFT, &staff)
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        one_in_the_morning["today"], "2026-07-30",
        "the evening still running"
    );
}

#[tokio::test]
async fn moving_a_booking_marked_as_not_coming_before_its_time_to_a_later_time_makes_it_a_plan_again()
 {
    // Booked for eight, marked as not coming at seven, moved to ten. Keeping the release from the
    // old window put it outside the new one, and the database refused the write.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 4, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let id = booked(&app, &staff, 1_200, "Рита").await;
    let phoned = app.at(common::utc(2026, 7, 30, 17, 0));
    phoned
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "no_show" }),
        )
        .await
        .expect_ok();

    let moved = phoned
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{id}/move"),
            &staff,
            serde_json::json!({ "start_minutes": 1_320 }),
        )
        .await;
    assert_eq!(moved.status, axum::http::StatusCode::OK, "{}", moved.body);
    assert_eq!(moved.body["booking"]["status"], "confirmed");
    assert!(moved.body["booking"]["released_minutes"].is_null());
    assert_eq!(moved.body["booking"]["start_minutes"], 1_320);
}

#[tokio::test]
async fn closing_a_table_already_shut_or_opening_one_never_shut_is_not_reported_as_done() {
    let app = harness().await;
    let staff = manager(&app).await;
    let first = table_id(&app, &staff, 1).await;
    let second = table_id(&app, &staff, 2).await;
    let third = table_id(&app, &staff, 3).await;
    app.post("/api/admin/blocks", &staff, serde_json::json!({ "service_date": "2026-07-30", "table_ids": [first], "reason": "Дождь" }))
        .await
        .expect_ok();

    let closed = app
        .post("/api/admin/blocks", &staff, serde_json::json!({ "service_date": "2026-07-30", "table_ids": [first, second], "reason": "Сломан" }))
        .await
        .expect_ok()
        .clone();
    assert_eq!(closed["closed"], serde_json::json!([second]));

    let reopened = app
        .send(
            "DELETE",
            "/api/admin/blocks",
            &staff,
            serde_json::json!({ "service_date": "2026-07-30", "table_ids": [first, third] }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        reopened["reopened"],
        serde_json::json!([{ "table_id": first, "reason": "Дождь" }])
    );
}

#[tokio::test]
async fn a_payload_stamped_within_a_minute_of_the_invitation_does_not_claim_the_seat() {
    // Telegram's clock may be a minute ahead of this one, so a payload stamped thirty seconds after
    // the seat was offered may have been signed before it, under a name that was not yet invited.
    let app = harness().await;
    let staff = manager(&app).await;
    let newcomer = Caller::new("Паша");
    let offered = morning() + chrono::TimeDelta::minutes(10);
    let invited = app.at(offered);
    let settings = invited.get(SETTINGS, &staff).await.expect_ok().clone();
    let mut draft = draft_from(&settings);
    draft["staff"]
        .as_array_mut()
        .expect("staff")
        .push(serde_json::json!({ "username": newcomer.username }));
    invited
        .send("PUT", SETTINGS, &staff, draft)
        .await
        .expect_ok();

    let close = app.at(offered + chrono::TimeDelta::seconds(30));
    assert_eq!(
        close.get(SHIFT, &newcomer).await.status,
        axum::http::StatusCode::FORBIDDEN
    );

    app.at(offered + chrono::TimeDelta::minutes(1))
        .get(SHIFT, &newcomer)
        .await
        .expect_ok();
}

#[tokio::test]
async fn text_holding_a_nul_character_is_refused_and_nothing_is_written() {
    // PostgreSQL text cannot hold U+0000. It used to reach the database and come back as a fault.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let id = booked(&app, &staff, 1_200, "Глеб").await;
    let first = table_id(&app, &staff, 1).await;
    let settings = app.get(SETTINGS, &staff).await.expect_ok().clone();
    let mut renamed = draft_from(&settings);
    renamed["name"] = serde_json::json!("Бар\u{0}");

    for (method, path, body) in [
        (
            "PATCH",
            format!("/api/admin/bookings/{id}/note"),
            serde_json::json!({ "note": "У окна\u{0}" }),
        ),
        (
            "POST",
            "/api/admin/bookings".to_owned(),
            serde_json::json!({
                "service_date": "2026-07-30", "start_minutes": 1_320, "party_size": 2,
                "guest_name": "Пётр\u{0}"
            }),
        ),
        (
            "POST",
            "/api/admin/blocks".to_owned(),
            serde_json::json!({
                "service_date": "2026-07-30", "table_ids": [first], "reason": "Сломан\u{0}"
            }),
        ),
        ("PUT", SETTINGS.to_owned(), renamed),
    ] {
        let refused = app.send(method, &path, &staff, body).await;
        assert_eq!(
            refused.status,
            axum::http::StatusCode::BAD_REQUEST,
            "{path}: {}",
            refused.body
        );
        assert_eq!(refused.error_code(), Some("text_invalid"), "{path}");
    }

    let shift = app.get(SHIFT, &staff).await.expect_ok().clone();
    assert_eq!(shift["bookings"].as_array().expect("bookings").len(), 1, "{shift}");
    assert!(shift["bookings"][0]["note"].is_null());
    assert!(shift["tables"][0]["blocked_because"].is_null());
    assert_eq!(
        app.get(SETTINGS, &staff).await.expect_ok()["name"],
        "Бар «Подвал»"
    );
}

#[tokio::test]
async fn undoing_a_departure_once_the_table_has_gone_to_somebody_else_says_the_table_is_taken() {
    let app = harness_at(
        common::utc(2026, 7, 30, 18, 0),
        config_with(vec![table(1, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let walk_in = serde_json::json!({ "service_date": "2026-07-30", "party_size": 2 });
    let first = app
        .post("/api/admin/walkins", &staff, walk_in.clone())
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let attendance = format!("/api/admin/bookings/{first}/attendance");
    let later = app.at(common::utc(2026, 7, 30, 18, 30));
    later
        .send("PATCH", &attendance, &staff, serde_json::json!({ "attendance": "left" }))
        .await
        .expect_ok();
    later
        .post("/api/admin/walkins", &staff, walk_in)
        .await
        .expect_ok();

    let refused = later
        .send("PATCH", &attendance, &staff, serde_json::json!({ "attendance": "arrived" }))
        .await;

    assert_eq!(refused.status, axum::http::StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(refused.error_code(), Some("table_taken"));
    let shift = later.get(SHIFT, &staff).await.expect_ok().clone();
    assert_eq!(
        in_shift(&shift, &first).expect("on the shift")["status"],
        "left",
        "nothing changed"
    );
}

#[tokio::test]
async fn a_guest_the_bot_cannot_reach_is_shown_so_and_never_reported_as_told() {
    // The bot found it cannot write to them. A notice is still queued, in case they let the bot back
    // in before it goes, but staff are not told the guest knows: they are the ones who must call.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let mut ids = Vec::new();
    for guest in [Caller::new("Вера"), Caller::new("Марк")] {
        let id = app
            .post(
                "/api/booking",
                &guest,
                serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1_200, "party_size": 2 }),
            )
            .await
            .expect_ok()["booking"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        app.store
            .set_reachable(pustol_db::TelegramUserId(guest.id), false)
            .await
            .expect("recorded");
        ids.push(id);
    }

    let shift = app.get(SHIFT, &staff).await.expect_ok().clone();
    for id in &ids {
        assert_eq!(
            in_shift(&shift, id).expect("on the shift")["reachable_by_bot"],
            false,
            "{shift}"
        );
    }
    let moved = app
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{}/move", ids[0]),
            &staff,
            serde_json::json!({ "start_minutes": 1_320 }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(moved["guest_notified"], false, "{moved}");
    let cancelled = app
        .post(
            &format!("/api/admin/bookings/{}/cancel", ids[1]),
            &staff,
            serde_json::json!({ "reason": "Частное мероприятие" }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(cancelled["guest_notified"], false, "{cancelled}");
    assert_eq!(cancelled_notices(&app, &ids[1]).await, 1, "queued all the same");
}

#[tokio::test]
async fn a_write_whose_evening_cannot_be_read_writes_nothing_and_trying_again_writes_it_once() {
    // The evening in the answer is read inside the write's own transaction. A failure reading it
    // takes the write back with it, so staff are never told a booking failed that was in fact taken,
    // and trying again cannot take it twice.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар")]),
    )
    .await;
    let staff = manager(&app).await;
    let count = || {
        let pool = app.store.pool().clone();
        let bar = app.bar;
        async move {
            sqlx::query_scalar::<_, i64>("select count(*) from booking where bar_id = $1")
                .bind(bar)
                .fetch_one(&pool)
                .await
                .expect("counted")
        }
    };
    let body = serde_json::json!({
        "service_date": "2026-07-30", "start_minutes": 1_200, "party_size": 2, "guest_name": "Глеб"
    });
    // The row the evening's version is read from is gone: the booking writes, the evening cannot be
    // read.
    sqlx::query("delete from room_version where bar_id = $1")
        .bind(app.bar)
        .execute(app.store.pool())
        .await
        .expect("deleted");

    let failed = app.post("/api/admin/bookings", &staff, body.clone()).await;
    assert!(failed.status.is_server_error(), "{} {}", failed.status, failed.body);
    assert_eq!(count().await, 0, "nothing was written");

    sqlx::query("insert into room_version (bar_id, version) values ($1, 1)")
        .bind(app.bar)
        .execute(app.store.pool())
        .await
        .expect("restored");
    app.post("/api/admin/bookings", &staff, body).await.expect_ok();
    assert_eq!(count().await, 1);
}

/// The version of the evening the shift screen would draw now.
async fn version(app: &common::Harness, staff: &Caller) -> i64 {
    app.get(SHIFT, staff).await.expect_ok()["version"]
        .as_i64()
        .expect("a version")
}

#[tokio::test]
async fn every_change_to_the_room_moves_the_evening_version_forward() {
    // A screen applies an evening only when it is not older than the one it shows, whichever answer
    // arrives first. That holds only if every change, by anybody on any path, moves the version.
    let app = harness_at(
        common::utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар"), table(3, 4, "Зал")]),
    )
    .await;
    let staff = manager(&app).await;
    let guest = Caller::new("Вера");
    let evening = "2026-07-30";
    let mut seen = vec![("the start", version(&app, &staff).await)];

    let plan = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-31", "start_minutes": 1_200, "party_size": 2 }),
        )
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    seen.push(("a guest booking", version(&app, &staff).await));

    let created = app
        .post(
            "/api/admin/bookings",
            &staff,
            serde_json::json!({
                "service_date": evening, "start_minutes": 1_200, "party_size": 2, "guest_name": "Глеб"
            }),
        )
        .await
        .expect_ok()
        .clone();
    seen.push(("a staff booking", version(&app, &staff).await));
    assert_eq!(
        created["shift"]["version"],
        seen.last().expect("seen").1,
        "the answer carries the version the write left"
    );
    let id = created["booking"]["id"].as_str().expect("an id").to_owned();

    sqlx::query("update booking set table_id = null where id = $1::uuid")
        .bind(&id)
        .execute(app.store.pool())
        .await
        .expect("written");
    seen.push(("a table taken away by hand", version(&app, &staff).await));

    let booking = format!("/api/admin/bookings/{id}");
    let second = table_id(&app, &staff, 2).await;
    let third = table_id(&app, &staff, 3).await;
    let settings = app.get(SETTINGS, &staff).await.expect_ok().clone();
    let staff_writes = [
        ("a reconcile", "POST", "/api/admin/shift/reconcile".to_owned(), serde_json::json!({ "service_date": evening })),
        ("a note", "PATCH", format!("{booking}/note"), serde_json::json!({ "note": "У окна" })),
        ("an attendance", "PATCH", format!("{booking}/attendance"), serde_json::json!({ "attendance": "arrived" })),
        ("a move", "PATCH", format!("{booking}/move"), serde_json::json!({ "start_minutes": 1_200, "table_id": second })),
        ("a closed table", "POST", "/api/admin/blocks".to_owned(), serde_json::json!({ "service_date": evening, "table_ids": [third], "reason": "Дождь" })),
        ("a reopened table", "DELETE", "/api/admin/blocks".to_owned(), serde_json::json!({ "service_date": evening, "table_ids": [third] })),
        ("a walk-in", "POST", "/api/admin/walkins".to_owned(), serde_json::json!({ "service_date": evening, "party_size": 2 })),
        ("a cancellation", "POST", format!("{booking}/cancel"), serde_json::json!({ "reason": "Частное мероприятие" })),
        ("a settings save", "PUT", SETTINGS.to_owned(), draft_from(&settings)),
    ];
    for (label, method, path, body) in staff_writes {
        app.send(method, &path, &staff, body).await.expect_ok();
        seen.push((label, version(&app, &staff).await));
    }
    app.send("DELETE", &format!("/api/bookings/{plan}"), &guest, serde_json::Value::Null)
        .await
        .expect_ok();
    seen.push(("a guest giving a table back", version(&app, &staff).await));

    for pair in seen.windows(2) {
        assert!(
            pair[1].1 > pair[0].1,
            "{} did not move the version past {}: {seen:?}",
            pair[1].0,
            pair[0].0
        );
    }
}
