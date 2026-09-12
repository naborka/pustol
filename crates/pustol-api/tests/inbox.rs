//! What the bot does with what guests send it, against a stub Telegram.
//!
//! Until this existed the reminder's «Не смогу прийти» button was drawn and nothing listened for
//! it: the guest tapped, Telegram spun, and the table stayed sold.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::routing::post;
use pustol_api::callbacks;
use pustol_api::inbox::Inbox;
use pustol_api::state::Clock;
use pustol_domain::BookingId;
use pustol_telegram::{Bot, BotToken, Update};
use tokio::sync::Mutex;
use uuid::Uuid;

use common::{Caller, Harness, harness, utc};

#[derive(Clone, Default)]
struct Telegram {
    calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    updates: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl Telegram {
    async fn calls_to(&self, method: &str) -> Vec<serde_json::Value> {
        self.calls
            .lock()
            .await
            .iter()
            .filter(|(called, _)| called == method)
            .map(|(_, body)| body.clone())
            .collect()
    }
}

async fn method(
    State(stub): State<Telegram>,
    Path((_token, method)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    stub.calls.lock().await.push((method.clone(), body.clone()));
    if method == "getUpdates" {
        let offset = body["offset"].as_i64().unwrap_or(0);
        let result: Vec<serde_json::Value> = stub
            .updates
            .lock()
            .await
            .iter()
            .filter(|update| update["update_id"].as_i64().unwrap_or(0) >= offset)
            .cloned()
            .collect();
        return Json(serde_json::json!({ "ok": true, "result": result }));
    }
    Json(serde_json::json!({ "ok": true, "result": true }))
}

async fn stub_telegram() -> (Bot, Telegram) {
    let stub = Telegram::default();
    let app = axum::Router::new()
        .route("/{token}/{method}", post(method))
        .with_state(stub.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address: SocketAddr = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let bot = Bot::new(BotToken::new(common::TOKEN)).with_base_url(format!("http://{address}"));
    (bot, stub)
}

fn inbox(app: &Harness, bot: Bot) -> Inbox {
    Inbox {
        store: app.store.clone(),
        bot,
        bar: app.bar,
        clock: Clock::Fixed(app.now),
    }
}

/// A tap on a button under one of the bot's messages, in the shape Telegram sends it.
fn tap(from: i64, data: &str) -> Update {
    serde_json::from_value(serde_json::json!({
        "update_id": 1,
        "callback_query": {
            "id": "query-1",
            "from": { "id": from, "is_bot": false, "first_name": "Гость" },
            "message": { "message_id": 55, "date": 0, "chat": { "id": from, "type": "private" } },
            "chat_instance": "1",
            "data": data
        }
    }))
    .expect("the shape Telegram sends")
}

/// A message typed into the bot's chat.
fn said(update_id: i64, from: i64, text: &str) -> serde_json::Value {
    serde_json::json!({
        "update_id": update_id,
        "message": {
            "message_id": 9,
            "date": 1,
            "chat": { "id": from, "type": "private" },
            "from": { "id": from, "is_bot": false, "first_name": "Гость" },
            "text": text
        }
    })
}

async fn book(app: &Harness, guest: &Caller) -> BookingId {
    let body = app
        .post(
            "/api/booking",
            guest,
            serde_json::json!({ "service_date": "2026-07-30", "start_minutes": 1200, "party_size": 2 }),
        )
        .await
        .expect_ok()
        .clone();
    BookingId(
        body["booking"]["id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .expect("an id"),
    )
}

#[tokio::test]
async fn the_reminder_button_gives_the_table_back_and_says_so() {
    let app = harness().await;
    let guest = Caller::new("Алексей");
    let booking = book(&app, &guest).await;
    let (bot, stub) = stub_telegram().await;

    inbox(&app, bot)
        .handle(tap(guest.id, &callbacks::cancel_booking(booking)))
        .await;

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["booking"], serde_json::Value::Null, "the table went back");
    let answers = stub.calls_to("answerCallbackQuery").await;
    assert_eq!(answers.len(), 1, "Telegram shows a spinner until the tap is answered");
    assert_eq!(answers[0]["callback_query_id"], "query-1");
    assert!(answers[0]["text"].as_str().expect("text").contains("отменена"), "{}", answers[0]);
    let edits = stub.calls_to("editMessageReplyMarkup").await;
    assert_eq!(edits.len(), 1, "a button that has done its job is taken away");
    assert_eq!(edits[0]["message_id"], 55);
}

#[tokio::test]
async fn a_tap_from_somebody_else_cancels_nothing() {
    let app = harness().await;
    let guest = Caller::new("Вера");
    let booking = book(&app, &guest).await;
    let (bot, stub) = stub_telegram().await;

    inbox(&app, bot)
        .handle(tap(guest.id + 1_000, &callbacks::cancel_booking(booking)))
        .await;

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert!(!session["booking"].is_null(), "callback data is whatever a client chose to send");
    assert_eq!(stub.calls_to("answerCallbackQuery").await.len(), 1);
}

#[tokio::test]
async fn a_tap_after_the_party_sat_down_does_not_release_their_table() {
    let app = harness().await;
    let guest = Caller::new("Марк");
    let booking = book(&app, &guest).await;
    let evening = app.at(utc(2026, 7, 30, 18, 10));
    evening
        .send(
            "PATCH",
            &format!("/api/admin/bookings/{}/attendance", booking.0),
            &Caller::manager(),
            serde_json::json!({ "attendance": "arrived" }),
        )
        .await
        .expect_ok();
    let (bot, stub) = stub_telegram().await;

    inbox(&evening, bot)
        .handle(tap(guest.id, &callbacks::cancel_booking(booking)))
        .await;

    let session = evening.get("/api/session", &guest).await.expect_ok().clone();
    assert!(!session["booking"].is_null());
    assert_eq!(stub.calls_to("answerCallbackQuery").await.len(), 1);
}

#[tokio::test]
async fn a_button_carrying_nonsense_is_answered_and_ignored() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    inbox(&app, bot).handle(tap(42, "cancel_booking:not-a-booking")).await;
    assert_eq!(stub.calls_to("answerCallbackQuery").await.len(), 1);
}

#[tokio::test]
async fn starting_the_bot_is_answered_and_proves_it_can_reach_the_guest_again() {
    let app = harness().await;
    let guest = Caller::new("Настя");
    book(&app, &guest).await;
    app.store
        .set_reachable(pustol_db::TelegramUserId(guest.id), false)
        .await
        .expect("recorded");
    let (bot, stub) = stub_telegram().await;

    let update: Update =
        serde_json::from_value(said(3, guest.id, "/start reminders")).expect("a message");
    inbox(&app, bot).handle(update).await;

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["reminders"]["deliverable"], true);
    let sent = stub.calls_to("sendMessage").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["chat_id"], guest.id);
    assert!(sent[0]["text"].as_str().expect("text").contains("напомин"), "{}", sent[0]);
}

#[tokio::test]
async fn a_message_nobody_will_read_is_answered_with_where_to_go_instead() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    let update: Update = serde_json::from_value(said(4, 77, "Нас будет восемь, можно?")).expect("a message");
    inbox(&app, bot).handle(update).await;
    let sent = stub.calls_to("sendMessage").await;
    assert_eq!(sent.len(), 1, "silence reads as being ignored");
    assert_eq!(sent[0]["chat_id"], 77);
}

#[tokio::test]
async fn polling_hands_each_update_over_once_and_moves_past_it() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(7, 77, "привет"));
    let inbox = inbox(&app, bot);

    let next = inbox.poll_once(None).await.expect("telegram answered");
    assert_eq!(next, Some(8));
    let again = inbox.poll_once(next).await.expect("telegram answered");
    assert_eq!(again, Some(8));

    let polls = stub.calls_to("getUpdates").await;
    assert_eq!(polls[1]["offset"], 8, "confirming update 7 so Telegram stops sending it");
    assert_eq!(stub.calls_to("sendMessage").await.len(), 1, "handled once, not twice");
}
