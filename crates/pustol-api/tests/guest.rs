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
    assert_eq!(body["bookings"], serde_json::json!([]));
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
    assert_eq!(taken["replaced"], serde_json::json!([]));
    assert_eq!(taken["booking"]["started"], false);
    assert_eq!(taken["booking"]["rebooking_replaces"], "any_evening");

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["bookings"][0]["id"], taken["booking"]["id"]);
    assert_eq!(session["bookings"].as_array().expect("bookings").len(), 1);
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
            serde_json::json!({
                "service_date": "2026-07-30", "start_minutes": 1320, "party_size": 4,
                "replacing": [first["booking"]["id"]]
            }),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(second["replaced"], serde_json::json!([first["booking"]["id"]]));
}

#[tokio::test]
async fn a_body_holding_a_nul_character_anywhere_is_refused_as_text_the_bar_cannot_keep() {
    // Keys included: an unknown key is otherwise ignored, and it is still text in a request that
    // storage would have to be trusted to refuse.
    let app = harness().await;
    let guest = Caller::new("Вера");
    let refused = app
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({
                "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2,
                "replacing": [], "comment\u{0}": "ключ"
            }),
        )
        .await;
    assert_eq!(refused.status, axum::http::StatusCode::BAD_REQUEST, "{}", refused.body);
    assert_eq!(refused.error_code(), Some("text_invalid"));
    assert_eq!(
        app.get("/api/session", &guest).await.expect_ok()["bookings"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn a_body_that_is_not_json_of_the_right_shape_is_refused_with_a_code_the_app_can_read() {
    // The statuses are axum's own. The body used to be a line of plain text, which the app, reading a
    // code out of JSON, reported as a failure to parse.
    let app = harness().await;
    let guest = Caller::new("Вера");
    let cases = [
        ("application/json", "{\"service_date\": ", axum::http::StatusCode::BAD_REQUEST),
        ("text/plain", "{}", axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (
            "application/json",
            "{\"service_date\": 5}",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (content_type, body, status) in cases {
        let answer = app
            .send_text("POST", "/api/booking", &guest, content_type, body)
            .await;
        assert_eq!(answer.status, status, "{content_type} {body}: {}", answer.body);
        assert_eq!(answer.error_code(), Some("body_invalid"), "{content_type} {body}: {}", answer.body);
        assert!(
            answer.body["error"]["message"].as_str().is_some_and(|message| !message.is_empty()),
            "{}",
            answer.body
        );
    }
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
    let app = harness_at(
        utc(2026, 7, 30, 20, 0),
        config_with(common::default_tables()),
    )
    .await;
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

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    let id = session["bookings"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let cancelled = app
        .send(
            "DELETE",
            &format!("/api/bookings/{id}"),
            &guest,
            serde_json::Value::Null,
        )
        .await;
    assert_eq!(cancelled.expect_ok()["status"], "cancelled");
    assert_eq!(cancelled.expect_ok()["id"], serde_json::json!(id));
    assert_eq!(
        app.get("/api/session", &guest).await.expect_ok()["bookings"],
        serde_json::json!([])
    );
    assert_eq!(
        app.send("DELETE", "/api/booking", &guest, serde_json::Value::Null)
            .await
            .status,
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        "there is no longer a booking without an id to give back"
    );
}

#[tokio::test]
async fn a_guest_who_has_gone_home_has_nothing_left_to_move_or_cancel() {
    // Booked at six, sat at eight, left at half past nine — an hour before the two-hour window
    // they were promised runs out. The bar's evening is over for them, and the home screen has to
    // say so: a card reading «Стол ваш» over «Перенести» and «Отменить» is the app telling
    // somebody who is already walking home that they still have a table.
    let evening = harness_at(
        utc(2026, 7, 30, 16, 0),
        config_with(vec![table(1, 2, "Бар")]),
    )
    .await;
    let guest = Caller::new("Полина");
    let staff = Caller::manager();

    let taken = evening
        .post(
            "/api/booking",
            &guest,
            serde_json::json!({
                "service_date": "2026-07-30",
                "start_minutes": 1200,
                "party_size": 2,
            }),
        )
        .await
        .expect_ok()
        .clone();
    let id = taken["booking"]["id"]
        .as_str()
        .expect("an identifier")
        .to_owned();
    let attendance = format!("/api/admin/bookings/{id}/attendance");

    let sitting = evening.at(utc(2026, 7, 30, 18, 0));
    sitting
        .send(
            "PATCH",
            &attendance,
            &staff,
            serde_json::json!({ "attendance": "arrived" }),
        )
        .await
        .expect_ok();
    let seated = sitting
        .get("/api/session", &guest)
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        seated["bookings"][0]["id"],
        serde_json::Value::String(id.clone()),
        "a guest at their table still has their booking"
    );
    assert_eq!(seated["bookings"][0]["started"], true);
    assert_eq!(
        seated["bookings"][0]["rebooking_replaces"],
        serde_json::Value::Null,
        "booking again never takes the table they are sitting at"
    );

    let gone = evening.at(utc(2026, 7, 30, 19, 30));
    gone.send(
        "PATCH",
        &attendance,
        &staff,
        serde_json::json!({ "attendance": "left" }),
    )
    .await
    .expect_ok();
    assert_eq!(
        gone.get("/api/session", &guest).await.expect_ok()["bookings"],
        serde_json::json!([])
    );
    let refused = gone
        .send(
            "DELETE",
            &format!("/api/bookings/{id}"),
            &guest,
            serde_json::Value::Null,
        )
        .await;
    assert_eq!(
        refused.status,
        axum::http::StatusCode::CONFLICT,
        "{}",
        refused.body
    );
    assert_eq!(
        refused.error_code(),
        Some("booking_finished"),
        "an evening that happened is not a booking to give back"
    );
}

#[tokio::test]
async fn cancelling_a_booking_that_is_not_the_guests_to_give_back_says_there_is_none() {
    let app = harness().await;
    let guest = Caller::new("Настя");
    let other = Caller::new("Вера");
    let theirs = app
        .post(
            "/api/booking",
            &other,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    for path in [
        format!("/api/bookings/{}", uuid::Uuid::new_v4()),
        format!("/api/bookings/{theirs}"),
    ] {
        let answer = app
            .send("DELETE", &path, &guest, serde_json::Value::Null)
            .await;
        assert_eq!(answer.status, axum::http::StatusCode::NOT_FOUND, "{path}");
        assert_eq!(answer.error_code(), Some("not_found"), "{path}");
    }
    assert_eq!(
        app.get("/api/session", &other).await.expect_ok()["bookings"][0]["status"],
        "confirmed",
        "somebody else's booking is untouched"
    );
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
    let app = harness_at(
        utc(2026, 7, 30, 23, 0),
        config_with(common::default_tables()),
    )
    .await;
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
async fn health_answers_when_the_database_is_gone() {
    // Liveness, not readiness. Infra's probe is `curl -fsS http://localhost:8080/health` and it
    // must not wait on Postgres. A check that talked to the database would fail a running
    // process during a blip and restart it while it still had work.
    let app = harness().await;
    app.store.pool().close().await;
    let answer = app.get_anonymously("/health").await;
    assert!(
        answer.status.is_success(),
        "health checked the database: {} {}",
        answer.status,
        answer.body
    );
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

#[tokio::test]
async fn the_day_rail_is_the_horizon_itself_and_every_chip_says_what_it_holds() {
    // Four days from a Thursday, with the Saturday shut. The rail is four chips long, not three:
    // a day a guest cannot have is a day the rail has to answer, not one it may quietly drop.
    let mut config = config_with(vec![table(1, 2, "Бар")]);
    config.horizon_days = 4;
    config.week = config.week.with(
        chrono::Weekday::Sat,
        pustol_domain::DayHours {
            closed: true,
            ..config.week.on(chrono::Weekday::Sat)
        },
    );
    let app = harness_at(morning(), config).await;
    let guest = Caller::new("Алексей");

    let body = app
        .get("/api/days?party_size=2", &guest)
        .await
        .expect_ok()
        .clone();
    let days = body["days"].as_array().expect("a rail");
    assert_eq!(days.len(), 4);
    assert_eq!(days[0]["service_date"], "2026-07-30");
    assert_eq!(days[0]["closed"], false);
    assert_eq!(days[0]["booked"], false, "a guest holding nothing is refused nothing");
    assert_eq!(days[0]["free_from_minutes"], 600);
    assert_eq!(days[2]["service_date"], "2026-08-01");
    assert_eq!(days[2]["closed"], true);
    assert!(
        days[2]["free_from_minutes"].is_null(),
        "a day off offers nothing to anybody"
    );
}

#[tokio::test]
async fn a_horizon_of_one_day_reaches_tonight_and_nowhere_else() {
    let mut config = config_with(vec![table(1, 2, "Бар")]);
    config.horizon_days = 1;
    let app = harness_at(morning(), config).await;
    let guest = Caller::new("Вера");
    let body = app
        .get("/api/days?party_size=2", &guest)
        .await
        .expect_ok()
        .clone();
    assert_eq!(body["days"].as_array().expect("a rail").len(), 1);
    assert_eq!(body["days"][0]["service_date"], "2026-07-30");
}

#[tokio::test]
async fn a_day_with_nothing_left_says_so_before_it_is_tapped() {
    // One table, one couple, and the only table busy from opening to closing.
    let mut config = config_with(vec![table(1, 2, "Бар")]);
    config.horizon_days = 2;
    config.turn_minutes = 240;
    config.week = pustol_domain::WeekSchedule::uniform(pustol_domain::DayHours {
        open_minutes: 1_080,
        close_minutes: 1_320,
        closed: false,
    });
    let app = harness_at(morning(), config).await;
    let guest = Caller::new("Тимур");
    let other = Caller::new("Глеб");

    app.post(
        "/api/booking",
        &other,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1080, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let body = app
        .get("/api/days?party_size=2", &guest)
        .await
        .expect_ok()
        .clone();
    assert!(
        body["days"][0]["free_from_minutes"].is_null(),
        "the evening is sold out and the chip says so"
    );
    assert_eq!(body["days"][1]["free_from_minutes"], 1_080);

    // And the home screen learns the same thing from the same function.
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert!(session["today_free_from_minutes"].is_null());
}

/// One table, busy from opening to closing for whoever holds it.
fn one_table_one_sitting() -> pustol_domain::BarConfig {
    let mut config = config_with(vec![table(1, 2, "Бар")]);
    config.horizon_days = 2;
    config.turn_minutes = 240;
    config.week = pustol_domain::WeekSchedule::uniform(pustol_domain::DayHours {
        open_minutes: 1_080,
        close_minutes: 1_320,
        closed: false,
    });
    config
}

#[tokio::test]
async fn a_guest_holding_tonights_only_table_is_not_told_tonight_is_full() {
    // Booking again replaces their own booking, so it frees exactly the time it holds. The card
    // that says tonight is sold out, shown to the one guest who could take it, reads as the app
    // having lost their table the moment they cancel it.
    let app = harness_at(morning(), one_table_one_sitting()).await;
    let guest = Caller::new("Тимур");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1080, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["today_free_from_minutes"], 1_080, "{session}");
    assert!(
        app.get("/api/session", &Caller::new("Глеб")).await.expect_ok()["today_free_from_minutes"]
            .is_null(),
        "for anybody else the evening is still sold out"
    );
}

#[tokio::test]
async fn the_day_rail_does_not_count_the_guests_own_booking_against_them() {
    let app = harness_at(morning(), one_table_one_sitting()).await;
    let guest = Caller::new("Тимур");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1080, "party_size": 2 }),
    )
    .await
    .expect_ok();

    let rail = app.get("/api/days?party_size=2", &guest).await.expect_ok().clone();
    assert_eq!(rail["days"][0]["free_from_minutes"], 1_080, "{rail}");
}

#[tokio::test]
async fn the_first_screen_says_when_tonight_opens_up() {
    let app = harness().await;
    let guest = Caller::new("Алексей");
    let body = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(body["today_free_from_minutes"], 600);
    assert_eq!(
        body["today_free_for_party"], 2,
        "the party that sentence speaks for, sent rather than agreed by comment"
    );
    assert_eq!(
        body["bar"]["now_minutes"], 480,
        "eight in the morning, in the bar's own timezone rather than the phone's"
    );
}

async fn staff_mark(app: &common::Harness, staff: &Caller, id: &str, attendance: &str) {
    app.send(
        "PATCH",
        &format!("/api/admin/bookings/{id}/attendance"),
        staff,
        serde_json::json!({ "attendance": attendance }),
    )
    .await
    .expect_ok();
}

fn book_at(start_minutes: i32) -> serde_json::Value {
    serde_json::json!({ "service_date": "2026-07-30", "start_minutes": start_minutes, "party_size": 2 })
}

/// A booking for two that expects to replace exactly the bookings `replacing` names.
fn book_replacing(service_date: &str, start_minutes: i32, replacing: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "service_date": service_date, "start_minutes": start_minutes, "party_size": 2,
        "replacing": replacing
    })
}

#[tokio::test]
async fn a_guest_who_has_gone_home_can_book_again_the_same_night() {
    let app = harness().await;
    let guest = Caller::new("Жора");
    let id = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let later = app.at(utc(2026, 7, 30, 18, 30));
    let staff = Caller::manager();
    staff_mark(&later, &staff, &id, "arrived").await;
    staff_mark(&later, &staff, &id, "left").await;

    later.post("/api/booking", &guest, book_at(1320)).await.expect_ok();
}

#[tokio::test]
async fn a_guest_already_at_their_table_is_told_they_have_tonight_rather_than_an_error() {
    let app = harness().await;
    let guest = Caller::new("Лёва");
    let id = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let later = app.at(utc(2026, 7, 30, 18, 30));
    staff_mark(&later, &Caller::manager(), &id, "arrived").await;

    let answer = later.post("/api/booking", &guest, book_at(1320)).await;
    assert_eq!(answer.status, axum::http::StatusCode::CONFLICT);
    assert_eq!(answer.error_code(), Some("already_booked_tonight"));
}

#[tokio::test]
async fn an_evening_nobody_marked_as_over_does_not_stop_the_guest_booking_again() {
    for marked in [None, Some("arrived")] {
        let app = harness().await;
        let guest = Caller::new("Сева");
        let id = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        if let Some(attendance) = marked {
            staff_mark(&app.at(utc(2026, 7, 30, 18, 30)), &Caller::manager(), &id, attendance).await;
        }

        // 20:00 to 22:00 Belgrade is over at 22:30, whatever staff did or did not press.
        let after = app.at(utc(2026, 7, 30, 20, 30));
        let session = after.get("/api/session", &guest).await.expect_ok().clone();
        assert_eq!(session["bookings"], serde_json::json!([]), "{marked:?}: the app shows no booking");
        after.post("/api/booking", &guest, book_at(1380)).await.expect_ok();
    }
}

/// The bookings staff see on the fixture Thursday under this guest's username.
async fn live_bookings_of(app: &common::Harness, staff: &Caller, guest: &Caller) -> Vec<serde_json::Value> {
    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", staff)
        .await
        .expect_ok()
        .clone();
    shift["bookings"]
        .as_array()
        .expect("bookings")
        .iter()
        .filter(|booking| booking["guest_username"] == serde_json::json!(guest.username))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_guest_whose_table_is_still_held_through_the_grace_period_books_again_in_its_place() {
    let app = harness().await;
    let guest = Caller::new("Рита");
    let id = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let late = app.at(utc(2026, 7, 30, 18, 5));
    let staff = Caller::manager();
    staff_mark(&late, &staff, &id, "no_show").await;

    let answer = late
        .post("/api/booking", &guest, book_replacing("2026-07-30", 1320, &[&id]))
        .await;
    assert_eq!(answer.expect_ok()["replaced"], serde_json::json!([id]));
    let live = live_bookings_of(&late, &staff, &guest).await;
    assert_eq!(live.len(), 1, "one table held for them tonight, not two: {live:?}");
    assert_eq!(live[0]["start_minutes"], 1320);
}

#[tokio::test]
async fn a_guest_marked_as_not_coming_before_their_time_can_book_a_later_one() {
    // They telephoned at seven to say eight is off. Staff pressed «Не пришли» there and then, and
    // the bar holds the table until a quarter past eight whatever happens.
    let app = harness().await;
    let guest = Caller::new("Стас");
    let id = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let phoned = app.at(utc(2026, 7, 30, 17, 0));
    staff_mark(&phoned, &Caller::manager(), &id, "no_show").await;

    let answer = phoned
        .post("/api/booking", &guest, book_replacing("2026-07-30", 1320, &[&id]))
        .await;
    assert_eq!(answer.expect_ok()["replaced"], serde_json::json!([id]));
}

#[tokio::test]
async fn staff_cannot_restore_a_no_show_that_would_give_the_guest_two_tables_tonight() {
    let app = harness().await;
    let guest = Caller::new("Нина");
    let first = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let staff = Caller::manager();
    let later = app.at(utc(2026, 7, 30, 18, 20));
    staff_mark(&later, &staff, &first, "no_show").await;
    later.post("/api/booking", &guest, book_at(1320)).await.expect_ok();

    let answer = later
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{first}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "confirmed" }),
        )
        .await;
    assert_eq!(answer.status, axum::http::StatusCode::CONFLICT, "{}", answer.body);
    assert_eq!(answer.error_code(), Some("already_booked_tonight"));
}

#[tokio::test]
async fn a_party_marked_gone_cannot_be_held_again_while_the_guest_has_another_table() {
    let app = harness().await;
    let guest = Caller::new("Оля");
    let first = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let staff = Caller::manager();
    let early = app.at(utc(2026, 7, 30, 18, 5));
    staff_mark(&early, &staff, &first, "arrived").await;
    staff_mark(&early, &staff, &first, "left").await;
    early.post("/api/booking", &guest, book_at(1320)).await.expect_ok();

    // A no-show is held until the grace period ends, a quarter past.
    let answer = early
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{first}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "no_show" }),
        )
        .await;
    assert_eq!(answer.status, axum::http::StatusCode::CONFLICT, "{}", answer.body);
    assert_eq!(answer.error_code(), Some("already_booked_tonight"));
}

#[tokio::test]
async fn staff_can_correct_an_evening_that_is_over_while_the_guest_sits_at_another_table() {
    let app = harness().await;
    let guest = Caller::new("Гена");
    let first = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let staff = Caller::manager();
    let later = app.at(utc(2026, 7, 30, 18, 20));
    staff_mark(&later, &staff, &first, "no_show").await;
    later.post("/api/booking", &guest, book_at(1320)).await.expect_ok();

    // They did come after all. Recording it holds nothing: that booking ended at 22:00.
    staff_mark(&app.at(utc(2026, 7, 30, 20, 30)), &staff, &first, "arrived").await;
}

/// A guest's plan for Thursday at eight, marked as not coming at seven when they telephoned, and the
/// plan for Friday they then made; with the staff who marked it.
async fn not_coming_tonight_with_a_plan_for_friday(
    app: &common::Harness,
    guest: &Caller,
) -> (common::Harness, Caller, String) {
    let tonight = app.post("/api/booking", guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let phoned = app.at(utc(2026, 7, 30, 17, 0));
    let staff = Caller::manager();
    staff_mark(&phoned, &staff, &tonight, "no_show").await;
    let friday = phoned
        .post("/api/booking", guest, book_replacing("2026-07-31", 1200, &[]))
        .await
        .expect_ok()
        .clone();
    assert_eq!(friday["replaced"], serde_json::json!([]), "{friday}");
    (phoned, staff, tonight)
}

#[tokio::test]
async fn staff_cannot_make_a_no_show_a_plan_again_while_the_guest_holds_another_plan() {
    let app = harness().await;
    let guest = Caller::new("Рома");
    let (phoned, staff, tonight) = not_coming_tonight_with_a_plan_for_friday(&app, &guest).await;

    let answer = phoned
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{tonight}/attendance"),
            &staff,
            serde_json::json!({ "attendance": "confirmed" }),
        )
        .await;

    assert_eq!(answer.status, axum::http::StatusCode::CONFLICT, "{}", answer.body);
    assert_eq!(answer.error_code(), Some("guest_has_another_plan"));
    let live = live_bookings_of(&phoned, &staff, &guest).await;
    assert_eq!(live[0]["status"], "no_show", "nothing changed: {live:?}");
}

#[tokio::test]
async fn staff_cannot_move_a_no_show_to_a_new_time_while_the_guest_holds_another_plan() {
    let app = harness().await;
    let guest = Caller::new("Рома");
    let (phoned, staff, tonight) = not_coming_tonight_with_a_plan_for_friday(&app, &guest).await;

    let answer = phoned
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{tonight}/move"),
            &staff,
            serde_json::json!({ "start_minutes": 1320 }),
        )
        .await;

    assert_eq!(answer.status, axum::http::StatusCode::CONFLICT, "{}", answer.body);
    assert_eq!(answer.error_code(), Some("guest_has_another_plan"));
    let live = live_bookings_of(&phoned, &staff, &guest).await;
    assert_eq!(
        (live[0]["status"].clone(), live[0]["start_minutes"].clone()),
        (serde_json::json!("no_show"), serde_json::json!(1200)),
        "nothing changed: {live:?}"
    );
}

#[tokio::test]
async fn booking_again_without_naming_the_plan_it_would_replace_is_refused_and_nothing_changes() {
    let app = harness().await;
    let guest = Caller::new("Дина");
    let plan = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let refused = app
        .post("/api/booking", &guest, book_replacing("2026-07-31", 1200, &[]))
        .await;

    assert_eq!(refused.status, axum::http::StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(refused.error_code(), Some("booking_changed"));
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(ids_of(&session["bookings"]), vec![plan]);
}

#[tokio::test]
async fn a_promise_read_before_the_plan_began_is_refused_once_it_has_begun() {
    // At 19:59 the app reads that booking again replaces the eight o'clock plan and says
    // «Перенести». The guest taps at 20:01: the plan is under way and nothing replaces it any more,
    // so the booking would not be the move the button promised.
    let app = harness().await;
    let guest = Caller::new("Лев");
    let plan = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let read = app
        .at(utc(2026, 7, 30, 17, 59))
        .get("/api/session", &guest)
        .await
        .expect_ok()
        .clone();
    assert_eq!(read["bookings"][0]["rebooking_replaces"], "any_evening", "{read}");

    let tapped = app.at(utc(2026, 7, 30, 18, 1));
    for body in [
        book_replacing("2026-07-31", 1200, &[&plan]),
        book_replacing("2026-07-30", 1320, &[&plan]),
    ] {
        let refused = tapped.post("/api/booking", &guest, body.clone()).await;
        assert_eq!(refused.status, axum::http::StatusCode::CONFLICT, "{body}: {}", refused.body);
        assert_eq!(refused.error_code(), Some("booking_changed"), "{body}");
    }
    let session = tapped.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(ids_of(&session["bookings"]), vec![plan], "no second booking");
}

#[tokio::test]
async fn rebooking_another_evening_gives_the_table_left_behind_to_a_party_without_one() {
    let app = harness_at(
        morning(),
        config_with(vec![table(1, 2, "Бар"), table(2, 2, "Бар")]),
    )
    .await;
    let guest = Caller::new("Ира");
    let thursday = app.post("/api/booking", &guest, book_at(1200)).await.expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let staff = Caller::manager();
    let phoned = app
        .post(
            "/api/admin/bookings",
            &staff,
            serde_json::json!({
                "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2,
                "guest_name": "Пётр"
            }),
        )
        .await
        .expect_ok()
        .clone();
    app.post(
        "/api/admin/blocks",
        &staff,
        serde_json::json!({
            "service_date": "2026-07-30",
            "table_ids": [phoned["booking"]["table_id"]],
            "reason": "Сломан"
        }),
    )
    .await
    .expect_ok();

    app.post(
        "/api/booking",
        &guest,
        book_replacing("2026-07-31", 1200, &[&thursday]),
    )
    .await
    .expect_ok();

    let shift = app
        .get("/api/admin/shift?service_date=2026-07-30", &staff)
        .await
        .expect_ok()
        .clone();
    let peter = shift["bookings"]
        .as_array()
        .expect("bookings")
        .iter()
        .find(|booking| booking["guest_name"] == "Пётр")
        .expect("still booked");
    assert!(!peter["table_id"].is_null(), "the table Ира gave back should seat Пётр: {peter}");
}

#[tokio::test]
async fn the_first_screen_says_how_to_reach_a_person_at_the_bar() {
    let mut config = config_with(common::default_tables());
    config.contact = Some("+381 11 123 45 67".to_owned());
    let app = harness_at(morning(), config).await;
    let body = app.get("/api/session", &Caller::new("Юля")).await.expect_ok().clone();
    assert_eq!(body["bar"]["contact"]["label"], "+381 11 123 45 67");
    assert_eq!(body["bar"]["contact"]["url"], "tel:+381111234567");
}

#[tokio::test]
async fn a_bar_without_a_contact_says_so_rather_than_inventing_one() {
    let app = harness().await;
    let body = app.get("/api/session", &Caller::new("Юля")).await.expect_ok().clone();
    assert!(body["bar"]["contact"].is_null());
}

fn book_on(service_date: &str, start_minutes: i32) -> serde_json::Value {
    serde_json::json!({ "service_date": service_date, "start_minutes": start_minutes, "party_size": 2 })
}

/// The ids of the bookings in a list the API answered with.
fn ids_of(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .expect("a list")
        .iter()
        .map(|booking| booking["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[tokio::test]
async fn a_guest_whose_table_is_held_tonight_books_another_evening_and_tonight_stays_as_it_was() {
    let app = harness().await;
    let guest = Caller::new("Стас");
    let staff = Caller::manager();
    let tonight = app
        .post("/api/booking", &guest, book_at(1200))
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    // Seven in the evening: they telephoned to say eight is off, and the table is held until a
    // quarter past.
    let phoned = app.at(utc(2026, 7, 30, 17, 0));
    staff_mark(&phoned, &staff, &tonight, "no_show").await;

    let friday = phoned
        .post("/api/booking", &guest, book_on("2026-07-31", 1200))
        .await
        .expect_ok()
        .clone();
    assert_eq!(friday["replaced"], serde_json::json!([]), "{friday}");

    let thursday = live_bookings_of(&phoned, &staff, &guest).await;
    assert_eq!(thursday.len(), 1);
    assert_eq!(
        thursday[0]["status"], "no_show",
        "Thursday still says who did not come"
    );

    let session = phoned.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(
        ids_of(&session["bookings"]),
        vec![
            tonight.clone(),
            friday["booking"]["id"].as_str().expect("an id").to_owned()
        ]
    );
    assert_eq!(session["bookings"][0]["rebooking_replaces"], "same_evening");
    assert_eq!(session["bookings"][1]["rebooking_replaces"], "any_evening");
}

#[tokio::test]
async fn a_no_show_is_offered_a_move_to_tonight_only_while_tonight_has_an_arrival_time_left() {
    // Ninety-minute sittings: by the hours the last arrival is 00:30. On an hourly grid it is
    // midnight, so at ten past there is nothing to move to; on a half-hourly grid 00:30 is still
    // ahead.
    for (step, offered) in [
        (60, serde_json::Value::Null),
        (30, serde_json::json!("same_evening")),
    ] {
        let mut config = config_with(common::default_tables());
        config.turn_minutes = 90;
        config.slot_step_minutes = step;
        let app = harness_at(morning(), config).await;
        let guest = Caller::new("Стас");
        let id = app
            .post("/api/booking", &guest, book_at(1440))
            .await
            .expect_ok()["booking"]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        staff_mark(
            &app.at(utc(2026, 7, 30, 21, 0)),
            &Caller::manager(),
            &id,
            "no_show",
        )
        .await;

        let session = app
            .at(utc(2026, 7, 30, 22, 10))
            .get("/api/session", &guest)
            .await
            .expect_ok()
            .clone();
        assert_eq!(ids_of(&session["bookings"]), vec![id], "step {step}: {session}");
        assert_eq!(
            session["bookings"][0]["rebooking_replaces"], offered,
            "step {step}: {session}"
        );
    }
}

/// A guest seated at tonight's only table, with a plan for the same table on Friday.
async fn seated_tonight_with_a_plan_for_friday(
    app: &common::Harness,
    guest: &Caller,
) -> (common::Harness, String, String) {
    let tonight = app
        .post("/api/booking", guest, book_on("2026-07-30", 1080))
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    staff_mark(
        &app.at(utc(2026, 7, 30, 16, 0)),
        &Caller::manager(),
        &tonight,
        "arrived",
    )
    .await;
    let seated = app.at(utc(2026, 7, 30, 16, 10));
    let friday = seated
        .post("/api/booking", guest, book_on("2026-07-31", 1080))
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        friday["replaced"],
        serde_json::json!([]),
        "a party at its table is never replaced"
    );
    let friday = friday["booking"]["id"].as_str().expect("an id").to_owned();
    (seated, tonight, friday)
}

#[tokio::test]
async fn a_seated_guest_is_refused_tonight_and_told_so_before_they_tap_it() {
    let app = harness_at(morning(), one_table_one_sitting()).await;
    let guest = Caller::new("Лёва");
    let (seated, _, friday) = seated_tonight_with_a_plan_for_friday(&app, &guest).await;

    // The app says what booking tonight would replace — their Friday plan — and tonight is still
    // refused, because they are sitting at its table.
    let refused = seated
        .post(
            "/api/booking",
            &guest,
            book_replacing("2026-07-30", 1080, &[&friday]),
        )
        .await;
    assert_eq!(
        refused.status,
        axum::http::StatusCode::CONFLICT,
        "{}",
        refused.body
    );
    assert_eq!(refused.error_code(), Some("already_booked_tonight"));

    let rail = seated
        .get("/api/days?party_size=2", &guest)
        .await
        .expect_ok()
        .clone();
    assert_eq!(rail["days"][0]["booked"], true, "{rail}");
    assert_eq!(rail["days"][1]["booked"], false, "{rail}");
    assert_eq!(
        rail["days"][1]["free_from_minutes"], 1080,
        "their own Friday is what a Friday booking replaces, so it does not fill Friday: {rail}"
    );
}

#[tokio::test]
async fn a_seated_guest_sees_both_bookings_and_friday_sets_their_own_plan_aside() {
    let app = harness_at(morning(), one_table_one_sitting()).await;
    let guest = Caller::new("Лёва");
    let (seated, tonight, friday) = seated_tonight_with_a_plan_for_friday(&app, &guest).await;

    let session = seated.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(ids_of(&session["bookings"]), vec![tonight, friday]);
    assert_eq!(session["bookings"][0]["started"], true);
    assert_eq!(
        session["bookings"][0]["rebooking_replaces"],
        serde_json::Value::Null
    );
    assert_eq!(session["bookings"][1]["started"], false);
    assert_eq!(session["bookings"][1]["rebooking_replaces"], "any_evening");

    let state_at_six = |body: &serde_json::Value| {
        body["slots"]
            .as_array()
            .expect("slots")
            .iter()
            .find(|slot| slot["start_minutes"] == 1080)
            .map(|slot| slot["state"].clone())
    };
    let theirs = seated
        .get(
            "/api/availability?service_date=2026-07-31&party_size=2",
            &guest,
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        state_at_six(&theirs),
        Some(serde_json::json!("free")),
        "{theirs}"
    );
    let anybody = seated
        .get(
            "/api/availability?service_date=2026-07-31&party_size=2",
            &Caller::new("Глеб"),
        )
        .await
        .expect_ok()
        .clone();
    assert_eq!(state_at_six(&anybody), Some(serde_json::json!("taken")));
}

#[tokio::test]
async fn on_the_night_the_clocks_go_forward_tonight_is_saturday_until_its_last_sitting_is_over() {
    // Saturday 28 March 2026 closes at 03:00, and at 02:00 the clocks jump to 03:00. The last
    // sitting, arriving at 01:00, holds its table two real hours: to 04:00 on the wall. Until then
    // tonight is Saturday; from then on it is Sunday, and the guest's plan for Sunday at 10:00 is
    // what a booking tonight replaces.
    let mut config = config_with(vec![table(1, 2, "Бар")]);
    config.week = pustol_domain::WeekSchedule::uniform(pustol_domain::DayHours {
        open_minutes: 600,
        close_minutes: 1620,
        closed: false,
    });
    let app = harness_at(utc(2026, 3, 28, 10, 0), config).await;
    let guest = Caller::new("Тимур");
    let saturday = app
        .post("/api/booking", &guest, book_on("2026-03-28", 1500))
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    staff_mark(
        &app.at(utc(2026, 3, 29, 0, 0)),
        &Caller::manager(),
        &saturday,
        "arrived",
    )
    .await;
    let sunday = app
        .at(utc(2026, 3, 29, 0, 10))
        .post("/api/booking", &guest, book_on("2026-03-29", 600))
        .await
        .expect_ok()["booking"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let half_past_three = app.at(utc(2026, 3, 29, 1, 30));
    let session = half_past_three
        .get("/api/session", &guest)
        .await
        .expect_ok()
        .clone();
    assert_eq!(
        session["bar"]["today"], "2026-03-28",
        "the last sitting still holds its table: {session}"
    );
    assert_eq!(
        ids_of(&session["bookings"]),
        vec![saturday, sunday.clone()],
        "{session}"
    );

    let four = app.at(utc(2026, 3, 29, 2, 0));
    let session = four.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["bar"]["today"], "2026-03-29", "{session}");
    assert_eq!(ids_of(&session["bookings"]), vec![sunday], "{session}");
    assert_eq!(
        session["today_free_from_minutes"], 600,
        "their own Sunday plan is what a booking tonight replaces: {session}"
    );
    assert_eq!(
        four.get("/api/session", &Caller::new("Глеб"))
            .await
            .expect_ok()["today_free_from_minutes"],
        720,
        "for anybody else the plan holds the table until noon"
    );
}
