//! A shift is longer than an hour.
//!
//! Telegram hands the app its signed payload once, when it opens, and the API accepts that payload
//! for an hour. A bartender who opened the shift at six was locked out at seven, and at every hour
//! after. The first screen now exchanges the payload for a session that lasts the day.

mod common;

use axum::http::StatusCode;
use chrono::TimeDelta;

use common::{Caller, harness, morning};

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
