//! The guest side, over HTTP.

mod common;

use common::{Caller, config_with, harness, harness_at, morning, table, utc};

#[tokio::test]
async fn a_request_with_no_credentials_is_refused() {
    let app = harness().await;
    let answer = app.get_anonymously("/api/session").await;
    assert_eq!(answer.status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(answer.error_code(), Some("no_credentials"));
}

#[tokio::test]
async fn a_payload_signed_by_another_token_is_refused() {
    // Without this the account number in the payload would be a claim anybody could make, and
    // taking over another guest's booking would be a text edit.
    let app = harness().await;
    let answer = app
        .get_with_forged_credentials("/api/session", &Caller::new("Самозванец"))
        .await;
    assert_eq!(answer.status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(answer.error_code(), Some("not_telegram"));
}

#[tokio::test]
async fn a_payload_signed_hours_ago_is_refused_as_expired_rather_than_as_forged() {
    // The app can fix "expired" by relaunching. It can do nothing about "forged", so the two must
    // not look the same.
    let app = harness_at(
        morning() + chrono::TimeDelta::hours(3),
        config_with(common::default_tables()),
    )
    .await;
    let stale = Caller::new("Вера").credentials(morning());
    let answer = app
        .send_raw("GET", "/api/session", Some(&stale), None)
        .await;
    assert_eq!(answer.status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(answer.error_code(), Some("session_expired"));
}

#[tokio::test]
async fn the_first_screen_arrives_in_one_request() {
    let app = harness().await;
    let guest = Caller::new("Алексей");
    let body = app.get("/api/session", &guest).await.expect_ok().clone();

    assert_eq!(body["user"]["id"], guest.id);
    assert_eq!(body["user"]["first_name"], "Алексей");
    assert_eq!(body["is_staff"], false);
    assert_eq!(body["booking"], serde_json::Value::Null);
    assert_eq!(body["bar"]["name"], "Бар «Подвал»");
    assert_eq!(body["bar"]["address"], "Дечанска 12, Белград");
    assert_eq!(body["bar"]["max_party"], 6);
    assert_eq!(body["bar"]["today"], "2026-07-30");
    assert_eq!(body["bar"]["today_hours"]["open_minutes"], 600);
    assert_eq!(body["bar"]["today_hours"]["close_minutes"], 1560);
    assert_eq!(body["bar"]["last_arrival_minutes"], 1440);
    assert_eq!(body["reminders"]["should_ask"], true);
    assert_eq!(
        body["bookable_days"],
        serde_json::json!(["2026-07-30", "2026-07-31", "2026-08-01", "2026-08-02"])
    );
}

#[tokio::test]
async fn the_picker_offers_every_arrival_time_with_a_reason() {
    let app = harness().await;
    let guest = Caller::new("Вера");
    let body = app
        .get(
            "/api/availability?service_date=2026-07-30&party_size=2",
            &guest,
        )
        .await
        .expect_ok()
        .clone();

    let slots = body["slots"].as_array().expect("a list of slots");
    assert_eq!(slots.len(), 29);
    assert_eq!(slots[0]["start_minutes"], 600);
    assert_eq!(slots[0]["state"], "free");
    assert_eq!(slots[0]["evening"], false);
    assert_eq!(slots[28]["start_minutes"], 1440);
    assert_eq!(slots[28]["evening"], true);
    assert_eq!(body["free_count"], 29);
    assert_eq!(body["turn_minutes"], 120);
    // A guest is never told which table they would be given.
    assert!(!body.to_string().contains("table"));
}

#[tokio::test]
async fn a_guest_books_a_table_and_sees_it_on_the_next_visit() {
    let app = harness().await;
    let guest = Caller::new("Алексей");
    let taken = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(taken["booking"]["start_minutes"], 1200);
    assert_eq!(taken["booking"]["end_minutes"], 1320);
    assert_eq!(taken["booking"]["party_size"], 2);
    assert_eq!(taken["booking"]["status"], "confirmed");
    assert_eq!(taken["replaced"], serde_json::Value::Null);

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["booking"]["id"], taken["booking"]["id"]);
}

#[tokio::test]
async fn booking_again_replaces_the_earlier_booking() {
    let app = harness().await;
    let guest = Caller::new("Лиза");
    let first = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()
        .clone();
    let second = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1320, "party_size": 4 }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(second["replaced"], first["booking"]["id"]);
}

#[tokio::test]
async fn a_time_somebody_else_has_taken_is_refused_with_a_code_the_app_can_act_on() {
    let app = harness_at(morning(), config_with(vec![table(1, 2, "Бар")])).await;
    let first = Caller::new("Вера");
    let second = Caller::new("Марина");
    app.post(
        "/api/booking",
        &first,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let refused = app
        .post(
            "/api/booking",
            &second,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await;
    assert_eq!(refused.status, axum::http::StatusCode::CONFLICT);
    assert_eq!(refused.error_code(), Some("no_table_free"));
}

#[tokio::test]
async fn a_time_that_has_gone_is_refused_as_past() {
    let app = harness_at(utc(2026, 7, 30, 20, 0), config_with(common::default_tables())).await;
    let guest = Caller::new("Егор");
    let refused = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("in_the_past"));
}

#[tokio::test]
async fn a_party_above_the_cap_is_refused_with_the_cap_in_the_detail() {
    let app = harness().await;
    let guest = Caller::new("Артур");
    let refused = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 9 }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("party_too_large"));
    assert_eq!(refused.body["error"]["detail"]["max_party"], 6);
}

#[tokio::test]
async fn a_shift_beyond_the_horizon_is_refused() {
    let app = harness().await;
    let guest = Caller::new("Мила");
    let refused = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-09-01", "start_minutes": 1200, "party_size": 2 }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("shift_not_bookable"));
}

#[tokio::test]
async fn a_guest_gives_their_table_back() {
    let app = harness().await;
    let guest = Caller::new("Полина");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let cancelled = app
        .send("DELETE", "/api/booking", &guest, serde_json::Value::Null)
        .await;
    assert_eq!(cancelled.expect_ok()["status"], "cancelled");
    assert_eq!(
        app.get("/api/session", &guest).await.expect_ok()["booking"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn cancelling_when_there_is_nothing_to_cancel_says_so() {
    let app = harness().await;
    let guest = Caller::new("Настя");
    let answer = app
        .send("DELETE", "/api/booking", &guest, serde_json::Value::Null)
        .await;
    assert_eq!(answer.status, axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_guest_changing_their_booking_still_sees_their_own_time_as_free() {
    let app = harness_at(morning(), config_with(vec![table(1, 2, "Бар")])).await;
    let guest = Caller::new("Тимур");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let body = app
        .get(
            "/api/availability?service_date=2026-07-30&party_size=2",
            &guest,
        )
        .await
        .expect_ok()
        .clone();
    let at_eight = body["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|slot| slot["start_minutes"] == 1200)
        .expect("20:00 is offered");
    assert_eq!(
        at_eight["state"], "free",
        "a guest must be able to keep their time while changing the party size"
    );
}

#[tokio::test]
async fn the_reminder_prompt_is_asked_once_and_then_settled() {
    let app = harness().await;
    let guest = Caller::new("Соня");
    assert_eq!(
        app.get("/api/session", &guest).await.expect_ok()["reminders"]["should_ask"],
        true
    );

    let dismissed = app
        .post("/api/reminders/dismiss", &guest, serde_json::Value::Null)
        .await;
    assert_eq!(dismissed.expect_ok()["should_ask"], false);
    assert_eq!(dismissed.expect_ok()["opted_in"], false);

    let opted = app
        .post("/api/reminders/opt-in", &guest, serde_json::Value::Null)
        .await;
    assert_eq!(opted.expect_ok()["opted_in"], true);
    assert_eq!(opted.expect_ok()["should_ask"], false);
}

#[tokio::test]
async fn a_guest_cannot_reach_the_admin_side() {
    let app = harness().await;
    let guest = Caller::new("Прохожий");
    for path in [
        "/api/admin/shift?service_date=2026-07-30",
        "/api/admin/settings?service_date=2026-07-30",
    ] {
        let answer = app.get(path, &guest).await;
        assert_eq!(
            answer.status,
            axum::http::StatusCode::FORBIDDEN,
            "{path} was reachable by a guest"
        );
    }
}

#[tokio::test]
async fn a_guest_with_no_username_can_still_book() {
    // Telegram lets an account have no username at all, and such a guest must not be a special case
    // anywhere.
    let app = harness().await;
    let guest = Caller::anonymous("Инкогнито");
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["user"]["username"], serde_json::Value::Null);
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();
}

#[tokio::test]
async fn a_day_the_bar_is_shut_is_not_offered_and_cannot_be_booked() {
    let mut config = config_with(common::default_tables());
    // Friday, index 5 counting from Sunday.
    config.week = config.week.with(
        chrono::Weekday::Fri,
        pustol_domain::DayHours {
            open_minutes: 600,
            close_minutes: 1560,
            closed: true,
        },
    );
    let app = harness_at(morning(), config).await;
    let guest = Caller::new("Ксения");

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(
        session["bookable_days"],
        serde_json::json!(["2026-07-30", "2026-08-01", "2026-08-02"]),
        "the closed Friday is left out but still counts against the horizon"
    );

    assert!(
        app.get(
            "/api/availability?service_date=2026-07-31&party_size=2",
            &guest
        )
        .await
        .expect_ok()["slots"]
            .as_array()
            .expect("slots")
            .is_empty()
    );

    let refused = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({ "service_date": "2026-07-31", "start_minutes": 1200, "party_size": 2 }),
        )
        .await;
    assert_eq!(refused.error_code(), Some("shift_not_bookable"));
}

#[tokio::test]
async fn at_one_in_the_morning_tonight_still_means_the_evening_in_progress() {
    // 01:00 Belgrade on the Friday is 23:00 UTC on the Thursday, and the bar shuts at 02:00.
    let app = harness_at(utc(2026, 7, 30, 23, 0), config_with(common::default_tables())).await;
    let guest = Caller::new("Данила");
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(
        session["bar"]["today"], "2026-07-30",
        "the running shift is the one that opened yesterday evening"
    );
}

#[tokio::test]
async fn health_needs_no_credentials() {
    let app = harness().await;
    let answer = app.get_anonymously("/health").await;
    assert!(answer.status.is_success());
}

#[tokio::test]
async fn an_unknown_api_path_fails_the_way_the_api_fails() {
    // The process also serves the app's static build, and its fallback answers HTML. An endpoint
    // that has been renamed must not reach it: `api.ts` reads a code out of a JSON body, and a
    // 404 page arriving instead turns a typo into a parse error nobody can read.
    let app = harness().await;
    for path in ["/api/no-such-endpoint", "/api/admin/no-such-endpoint"] {
        let answer = app.get_anonymously(path).await;
        assert_eq!(answer.status, axum::http::StatusCode::NOT_FOUND, "{path}");
        assert_eq!(answer.error_code(), Some("not_found"), "{path}");
    }
}
