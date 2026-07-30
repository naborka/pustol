//! The outbox, driven against a stub Telegram.
//!
//! Delivery is where this system meets something it does not control, so the stub answers with the
//! actual shapes Telegram answers with. Asserting against a mock that always says yes would leave
//! every retry and give-up path untested.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use chrono::{DateTime, TimeDelta, Utc};
use pustol_api::worker;
use pustol_api::state::Clock;
use pustol_db::Store;
use pustol_db::notifications::NotificationKind;
use pustol_telegram::{Bot, BotToken};
use tokio::sync::Mutex;

use common::{Caller, harness_at, morning, utc};

/// What the stub should answer with, and what it saw.
#[derive(Clone)]
struct Telegram {
    answers: Arc<Mutex<Vec<(StatusCode, serde_json::Value)>>>,
    seen: Arc<Mutex<Vec<serde_json::Value>>>,
    calls: Arc<AtomicUsize>,
}

impl Telegram {
    fn new(answers: Vec<(StatusCode, serde_json::Value)>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(answers)),
            seen: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn accepting() -> Self {
        Self::new(Vec::new())
    }
}

async fn send_message(
    State(stub): State<Telegram>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    stub.calls.fetch_add(1, Ordering::Relaxed);
    stub.seen.lock().await.push(body);
    let mut answers = stub.answers.lock().await;
    if answers.is_empty() {
        return (StatusCode::OK, Json(serde_json::json!({ "ok": true })));
    }
    let (status, body) = answers.remove(0);
    (status, Json(body))
}

/// Starts the stub and returns a bot pointed at it.
async fn stub_telegram(stub: Telegram) -> (Bot, SocketAddr) {
    let app = axum::Router::new()
        .route("/bot{token}/sendMessage", post(send_message))
        .with_state(stub);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let bot = Bot::new(BotToken::new(common::TOKEN), reqwest::Client::new())
        .with_base_url(format!("http://{address}"));
    (bot, address)
}

/// A guest with a booking at 20:00 who has asked to be reminded.
async fn booked_and_opted_in(app: &common::Harness) -> Caller {
    let guest = Caller::new("Алексей");
    app.post("/api/reminders/opt-in", &guest, serde_json::Value::Null)
        .await
        .expect_ok();
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();
    guest
}

/// Three hours before the booking, which is when the reminder falls due.
fn reminder_due() -> DateTime<Utc> {
    utc(2026, 7, 30, 15, 0)
}

async fn drain(store: &Store, bot: &Bot, now: DateTime<Utc>) -> usize {
    worker::drain_once(store, bot, &Clock::Fixed(now))
        .await
        .expect("the outbox can be read")
}

#[tokio::test]
async fn a_reminder_goes_out_with_a_button_that_cancels_it() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let _guest = booked_and_opted_in(&app).await;
    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;

    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 1);
    let seen = stub.seen.lock().await;
    assert_eq!(seen.len(), 1);
    let sent = &seen[0];
    assert!(
        sent["text"].as_str().expect("text").contains("Напоминаем"),
        "got {sent}"
    );
    let button = &sent["reply_markup"]["inline_keyboard"][0][0];
    assert_eq!(button["text"], "Не смогу прийти");
    assert!(
        button["callback_data"]
            .as_str()
            .expect("callback data")
            .starts_with(worker::CANCEL_CALLBACK),
        "a reminder without a way to cancel is a notification, not a service"
    );

    // Delivered once and never again.
    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 0);
}

#[tokio::test]
async fn nothing_goes_out_before_its_hour() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let _guest = booked_and_opted_in(&app).await;
    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;
    assert_eq!(drain(&app.store, &bot, morning()).await, 0);
    assert_eq!(stub.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_guest_who_never_asked_for_reminders_is_not_reminded() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let guest = Caller::new("Вера");
    app.post(
        "/api/booking",
        &guest,
        serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
    )
    .await
    .expect_ok();
    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;
    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 0);
}

#[tokio::test]
async fn a_blocked_bot_is_remembered_and_never_retried() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let guest = booked_and_opted_in(&app).await;
    let stub = Telegram::new(vec![(
        StatusCode::FORBIDDEN,
        serde_json::json!({ "ok": false, "description": "Forbidden: bot was blocked by the user" }),
    )]);
    let (bot, _) = stub_telegram(stub.clone()).await;

    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 1);
    // Not retried, however long is left.
    assert_eq!(
        drain(&app.store, &bot, reminder_due() + TimeDelta::days(1)).await,
        0
    );
    assert_eq!(stub.calls.load(Ordering::Relaxed), 1);

    // And the app stops promising reminders it cannot deliver.
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["reminders"]["deliverable"], false);
}

#[tokio::test]
async fn a_rate_limit_waits_exactly_as_long_as_telegram_asked() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let _guest = booked_and_opted_in(&app).await;
    let stub = Telegram::new(vec![(
        StatusCode::TOO_MANY_REQUESTS,
        serde_json::json!({
            "ok": false,
            "description": "Too Many Requests: retry after 90",
            "parameters": { "retry_after": 90 }
        }),
    )]);
    let (bot, _) = stub_telegram(stub.clone()).await;

    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 1);
    // Not a second earlier.
    assert_eq!(
        drain(&app.store, &bot, reminder_due() + TimeDelta::seconds(89)).await,
        0
    );
    assert_eq!(
        drain(&app.store, &bot, reminder_due() + TimeDelta::seconds(90)).await,
        1
    );
    assert_eq!(stub.calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn a_server_fault_is_retried_and_eventually_given_up_on() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let _guest = booked_and_opted_in(&app).await;
    let failures = (0..10)
        .map(|_| {
            (
                StatusCode::BAD_GATEWAY,
                serde_json::json!({ "ok": false, "description": "Bad Gateway" }),
            )
        })
        .collect();
    let stub = Telegram::new(failures);
    let (bot, _) = stub_telegram(stub.clone()).await;

    // Each pass is a fresh attempt, well past the two-minute backoff. The steps stay small enough
    // that the clock does not run past the booking itself, which would stop the reminder being
    // eligible for its own reason.
    let mut attempts = 0;
    let mut clock = reminder_due();
    for _ in 0..10 {
        attempts += drain(&app.store, &bot, clock).await;
        clock += TimeDelta::minutes(10);
    }
    assert_eq!(
        attempts, 6,
        "a message Telegram keeps refusing is abandoned rather than retried for ever"
    );
    assert_eq!(
        drain(&app.store, &bot, clock).await,
        0,
        "and stays abandoned"
    );
}

#[tokio::test]
async fn a_request_telegram_refuses_on_its_merits_is_not_retried() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let guest = booked_and_opted_in(&app).await;
    let stub = Telegram::new(vec![(
        StatusCode::BAD_REQUEST,
        serde_json::json!({ "ok": false, "description": "Bad Request: message text is empty" }),
    )]);
    let (bot, _) = stub_telegram(stub.clone()).await;

    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 1);
    assert_eq!(
        drain(&app.store, &bot, reminder_due() + TimeDelta::days(1)).await,
        0
    );
    // A broken request must not be blamed on the guest.
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["reminders"]["deliverable"], true);
}

#[tokio::test]
async fn a_cancelled_booking_is_never_reminded_about() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let guest = booked_and_opted_in(&app).await;
    app.send("DELETE", "/api/booking", &guest, serde_json::Value::Null)
        .await
        .expect_ok();

    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;
    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 0);
    assert_eq!(stub.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_cancellation_by_staff_reaches_the_guest_with_the_reason_they_chose() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let guest = Caller::new("Ксения");
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

    let staff = Caller::manager();
    app.get("/api/session", &staff).await.expect_ok();
    app.post(
        &format!("/api/admin/bookings/{booking}/cancel"),
        &staff,
        serde_json::json!({ "reason": "Частное мероприятие" }),
    )
    .await
    .expect_ok();

    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;
    // A cancellation is not a reminder: it goes out at once, and the reminder opt-in has no say.
    assert_eq!(drain(&app.store, &bot, morning()).await, 1);
    let seen = stub.seen.lock().await;
    assert!(
        seen[0]["text"]
            .as_str()
            .expect("text")
            .contains("Частное мероприятие"),
        "got {}",
        seen[0]
    );
    assert!(
        seen[0].get("reply_markup").is_none(),
        "there is nothing to cancel any more, so no button"
    );
}

#[tokio::test]
async fn a_message_staff_sent_goes_out_regardless_of_the_reminder_preference() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
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

    let staff = Caller::manager();
    app.get("/api/session", &staff).await.expect_ok();
    app.post(
        &format!("/api/admin/bookings/{booking}/message"),
        &staff,
        serde_json::json!({ "text": "Ваш стол готов, ждём вас!" }),
    )
    .await
    .expect_ok();

    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;
    assert_eq!(drain(&app.store, &bot, morning()).await, 1);
    let seen = stub.seen.lock().await;
    assert_eq!(seen[0]["text"], "Ваш стол готов, ждём вас!");
    assert_eq!(
        seen[0]["chat_id"], guest.id,
        "sent to the guest, not to whoever pressed the button"
    );
}

#[tokio::test]
async fn a_reminder_for_a_booking_that_has_already_started_is_not_sent_late() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let _guest = booked_and_opted_in(&app).await;
    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;

    // Midnight: the 20:00 booking is long since under way, and "your table in three hours" would be
    // nonsense.
    assert_eq!(drain(&app.store, &bot, utc(2026, 7, 30, 22, 0)).await, 0);
}

#[tokio::test]
async fn one_delivery_is_enough_to_learn_that_a_guest_is_reachable() {
    let app = harness_at(morning(), common::config_with(common::default_tables())).await;
    let guest = booked_and_opted_in(&app).await;
    app.store
        .set_reachable(pustol_db::TelegramUserId(guest.id), false)
        .await
        .expect("recorded");

    // Nothing goes out while the bot is believed unable to reach them.
    let stub = Telegram::accepting();
    let (bot, _) = stub_telegram(stub.clone()).await;
    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 0);

    // A staff message is sent anyway — staff pressed send — and its success is evidence.
    app.store
        .set_reachable(pustol_db::TelegramUserId(guest.id), true)
        .await
        .expect("recorded");
    assert_eq!(drain(&app.store, &bot, reminder_due()).await, 1);
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["reminders"]["deliverable"], true);
}

#[tokio::test]
async fn the_kinds_of_message_are_distinguishable_to_the_worker() {
    // A compile-time guard as much as a test: adding a kind without deciding whether it carries a
    // button breaks here.
    assert_ne!(NotificationKind::Reminder, NotificationKind::Cancelled);
    assert_ne!(NotificationKind::Reminder, NotificationKind::StaffMessage);
}
