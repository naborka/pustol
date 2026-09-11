//! The staff side, over HTTP.

mod common;

use common::{Caller, config_with, harness, harness_at, morning, table};

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
    assert_eq!(created["guest_name"], "Полина", "trimmed on the way in");
    assert_eq!(created["source"], "staff");
    assert_eq!(created["reachable_by_bot"], false);

    let refused = app
        .post(
            &format!("/api/admin/bookings/{}/message", created["id"].as_str().unwrap()),
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
    let id = created["id"].as_str().expect("an id");

    for (attendance, expected) in [("arrived", "arrived"), ("no_show", "no_show"), ("confirmed", "confirmed")] {
        let updated = app
            .send(
                "PATCH",
                &format!("/api/admin/bookings/{id}/attendance"),
                &staff,
                serde_json::json!({ "attendance": attendance }),
            )
            .await;
        assert_eq!(updated.expect_ok()["status"], expected);
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
        app.get("/api/session", &guest).await.expect_ok()["booking"],
        serde_json::Value::Null
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
            &format!("/api/admin/bookings/{}/cancel", created["id"].as_str().unwrap()),
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
    assert_eq!(report["moved"].as_array().expect("moved").len(), 1);
    assert_eq!(report["moved"][0]["guest_name"], "Анна К.");
    assert_eq!(report["moved"][0]["to_number"], 2);
    assert!(report["orphaned"].as_array().expect("orphaned").is_empty());

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
    assert_eq!(closed["orphaned"].as_array().expect("orphaned").len(), 1);
    assert_eq!(closed["orphaned"][0]["guest_name"], "Павел");

    // The guest is never told, and their booking still reads as confirmed to them.
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["booking"]["status"], "confirmed");

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
    assert_eq!(reopened["moved"].as_array().expect("moved").len(), 1);
    assert_eq!(reopened["moved"][0]["guest_name"], "Павел");
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
    assert!(retried["moved"].as_array().expect("moved").is_empty());
    assert_eq!(retried["orphaned"][0]["guest_name"], "Тимур");
}

// ---- settings ---------------------------------------------------------------------------------

/// The settings screen's payload turned back into the proposal the screen would send.
fn draft_from(settings: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": settings["name"],
        "address": settings["address"],
        "timezone": settings["timezone"],
        "week": settings["week"],
        "zones": settings["zones"],
        "tables": settings["tables"]
            .as_array()
            .expect("tables")
            .iter()
            .map(|table| serde_json::json!({
                "kind": "existing",
                "id": table["id"],
                "seats": table["seats"],
                "zone": table["zone"],
            }))
            .collect::<Vec<_>>(),
        "turn_minutes": settings["turn_minutes"],
        "slot_step_minutes": settings["slot_step_minutes"],
        "max_party": settings["max_party"],
        "horizon_days": settings["horizon_days"],
        "remind_hours": settings["remind_hours"],
        "grace_minutes": settings["grace_minutes"],
        "message_templates": settings["message_templates"],
        "cancel_reasons": settings["cancel_reasons"],
        "staff": settings["staff"]
            .as_array()
            .expect("staff")
            .iter()
            .map(|member| serde_json::json!({ "username": member["username"] }))
            .collect::<Vec<_>>(),
    })
}

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
    tables.push(serde_json::json!({ "kind": "new", "seats": 6, "zone": "Зал" }));

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
async fn a_proposal_naming_a_table_that_does_not_exist_is_refused_as_unreadable() {
    let app = harness().await;
    let staff = manager(&app).await;
    let settings = app
        .get("/api/admin/settings?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let mut draft = draft_from(&settings);
    draft["tables"].as_array_mut().expect("tables").push(serde_json::json!({
        "kind": "existing",
        "id": "11111111-2222-3333-4444-555555555555",
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
    assert_eq!(session["booking"]["start_minutes"], 1200);
    assert_eq!(session["booking"]["end_minutes"], 1320);
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

    assert_eq!(seated["table_number"], 1, "the smallest table that fits");
    assert_eq!(seated["source"], "walk");
    assert_eq!(seated["status"], "arrived");
    assert_eq!(seated["guest_name"], "Без брони");
    assert_eq!(
        seated["start_minutes"], 1_207,
        "20:07 local, not floored onto the half-hour grid"
    );
    assert_eq!(seated["reachable_by_bot"], false);
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
    let id = seated["id"].as_str().expect("an identifier").to_owned();

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
    assert_eq!(gone["status"], "left");
    assert_eq!(
        gone["released_minutes"], 1_200,
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
    assert!(back["released_minutes"].is_null());
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
    assert_eq!(noted["note"], "День рождения");

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert!(
        session["booking"].get("note").is_none(),
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
    assert!(rubbed["note"].is_null());

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
