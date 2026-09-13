//! A shift is longer than an hour.
//!
//! Telegram hands the app its signed payload once, when it opens, and the API accepts that payload
//! for an hour. A bartender who opened the shift at six was locked out at seven, and at every hour
//! after. The first screen now exchanges the payload for a session that lasts the day.

mod common;

use axum::http::StatusCode;
use chrono::TimeDelta;

use common::{Caller, draft_from, harness, morning};

const SETTINGS: &str = "/api/admin/settings?service_date=2026-07-30";
const SHIFT: &str = "/api/admin/shift?service_date=2026-07-30";

async fn session_of(app: &common::Harness, caller: &Caller) -> String {
    app.get("/api/session", caller).await.expect_ok()["session_token"]
        .as_str()
        .expect("a session token")
        .to_owned()
}

#[tokio::test]
async fn the_session_outlasts_the_payload_it_was_exchanged_for() {
    let app = harness().await;
    let bartender = Caller::new("Паша");
    let session = session_of(&app, &bartender).await;

    let later = app.at(morning() + TimeDelta::hours(6));
    let payload = later
        .send_raw("GET", "/api/session", Some(&bartender.credentials(morning())), None)
        .await;
    assert_eq!(payload.error_code(), Some("session_expired"), "the payload still expires in an hour");

    let answer = later
        .send_raw("GET", "/api/session", Some(&format!("session {session}")), None)
        .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["user"]["id"], bartender.id);
}

#[tokio::test]
async fn a_session_ends_a_day_after_telegram_signed_the_payload() {
    let app = harness().await;
    let bartender = Caller::new("Паша");
    let session = session_of(&app, &bartender).await;

    let next_day = app.at(morning() + TimeDelta::hours(24));
    let answer = next_day
        .send_raw("GET", "/api/session", Some(&format!("session {session}")), None)
        .await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer.error_code(), Some("session_expired"));
}

#[tokio::test]
async fn a_session_is_counted_from_when_telegram_signed_not_from_when_it_was_exchanged() {
    let app = harness().await;
    let bartender = Caller::new("Паша");
    let signed = morning();
    let session = app
        .at(signed + TimeDelta::minutes(50))
        .send_raw("GET", "/api/session", Some(&bartender.credentials(signed)), None)
        .await
        .expect_ok()["session_token"]
        .as_str()
        .expect("a session token")
        .to_owned();
    let credentials = format!("session {session}");

    let last_minute = app
        .at(signed + TimeDelta::hours(23) + TimeDelta::minutes(59))
        .send_raw("GET", "/api/session", Some(&credentials), None)
        .await;
    assert_eq!(last_minute.status, StatusCode::OK, "{}", last_minute.body);

    let over = app
        .at(signed + TimeDelta::hours(24))
        .send_raw("GET", "/api/session", Some(&credentials), None)
        .await;
    assert_eq!(over.error_code(), Some("session_expired"), "not 24 hours after the exchange");
}

#[tokio::test]
async fn a_session_renewed_with_a_session_still_ends_when_the_first_one_did() {
    let app = harness().await;
    let first = session_of(&app, &Caller::new("Паша")).await;
    let renewed = app
        .at(morning() + TimeDelta::hours(2))
        .send_raw("GET", "/api/session", Some(&format!("session {first}")), None)
        .await
        .expect_ok()["session_token"]
        .as_str()
        .expect("a session token")
        .to_owned();

    let over = app
        .at(morning() + TimeDelta::hours(24))
        .send_raw("GET", "/api/session", Some(&format!("session {renewed}")), None)
        .await;
    assert_eq!(over.error_code(), Some("session_expired"));
}

#[tokio::test]
async fn an_edited_session_is_refused_as_not_from_telegram() {
    let app = harness().await;
    let session = session_of(&app, &Caller::new("Паша")).await;
    let mut edited = session.clone();
    let last = edited.pop().expect("not empty");
    edited.push(if last == '0' { '1' } else { '0' });

    let answer = app
        .send_raw("GET", "/api/session", Some(&format!("session {edited}")), None)
        .await;
    assert_eq!(answer.error_code(), Some("not_telegram"));
}

#[tokio::test]
async fn a_username_kept_in_a_session_does_not_claim_a_seat_offered_after_it() {
    let app = harness().await;
    let newcomer = Caller::new("Паша");
    let session = format!("session {}", session_of(&app, &newcomer).await);

    let manager = Caller::manager();
    let settings = app.get(SETTINGS, &manager).await.expect_ok().clone();
    let mut draft = draft_from(&settings);
    draft["staff"]
        .as_array_mut()
        .expect("staff")
        .push(serde_json::json!({ "username": newcomer.username }));
    app.send("PUT", SETTINGS, &manager, draft).await.expect_ok();

    let answer = app.send_raw("GET", SHIFT, Some(&session), None).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", answer.body);
    let staff = app.get(SETTINGS, &manager).await.expect_ok()["staff"].clone();
    let seat = staff
        .as_array()
        .expect("staff")
        .iter()
        .find(|member| member["username"] == serde_json::json!(newcomer.username))
        .expect("invited")
        .clone();
    assert_eq!(seat["bound"], false, "the name may have passed to somebody else since: {seat}");

    app.get(SHIFT, &newcomer).await.expect_ok();
}

#[tokio::test]
async fn a_session_never_writes_back_a_name_the_account_has_since_changed() {
    let app = harness().await;
    let before = Caller::new("Паша");
    let session = format!("session {}", session_of(&app, &before).await);
    let after = Caller {
        username: before.username.as_ref().map(|name| format!("{name}_new")),
        ..before.clone()
    };
    app.get("/api/session", &after).await.expect_ok();

    let answer = app.send_raw("GET", "/api/session", Some(&session), None).await;
    assert_eq!(answer.expect_ok()["user"]["username"], serde_json::json!(after.username));

    app.send_raw(
        "POST",
        "/api/booking",
        Some(&session),
        Some(serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 })),
    )
    .await
    .expect_ok();
    let shift = app.get(SHIFT, &Caller::manager()).await.expect_ok().clone();
    assert_eq!(
        shift["bookings"][0]["guest_username"],
        serde_json::json!(after.username),
        "{shift}"
    );
}

#[tokio::test]
async fn a_session_proves_who_is_asking_and_not_what_they_may_do() {
    let app = harness().await;
    let guest = Caller::new("Гость");
    let session = session_of(&app, &guest).await;
    let answer = app
        .send_raw(
            "GET",
            "/api/admin/shift?service_date=2026-07-30",
            Some(&format!("session {session}")),
            None,
        )
        .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
}
