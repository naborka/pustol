//! What the bot does with what guests send it, against a stub Telegram.
//!
//! Until this existed the reminder's «Не смогу прийти» button was drawn and nothing listened for
//! it: the guest tapped, Telegram spun, and the table stayed sold.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::routing::post;
use pustol_api::callbacks;
use pustol_api::inbox::Inbox;
use pustol_api::state::Clock;
use pustol_domain::BookingId;
use pustol_telegram::{Bot, BotToken, Update, messages};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

use common::{Caller, Harness, harness, morning, utc};

#[derive(Clone, Default)]
struct Telegram {
    base_url: String,
    calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    updates: Arc<Mutex<Vec<serde_json::Value>>>,
    /// When set, `getUpdates` with nothing to hand over waits as Telegram does, until something
    /// arrives or the wait asked for is over. Otherwise it answers at once, so a test that polls
    /// once more is not held for the whole long poll.
    long_polls: Arc<AtomicBool>,
    /// Wakes a `getUpdates` that is waiting for something to arrive.
    arrived: Arc<Notify>,
    /// When set, an offset past every update Telegram now has is not honoured, as after Telegram
    /// has counted update ids afresh.
    forgets_stale_offsets: Arc<AtomicBool>,
    /// When set, a reply to a guest is held until `release` is notified.
    hold_replies: Arc<AtomicBool>,
    release: Arc<Notify>,
}

impl Telegram {
    /// What `getUpdates` hands over from `offset`.
    async fn pending(&self, offset: Option<i64>) -> Vec<serde_json::Value> {
        let updates = self.updates.lock().await;
        let id = |update: &serde_json::Value| update["update_id"].as_i64().unwrap_or(0);
        let newest = updates.iter().map(id).max();
        let stale = offset.is_some_and(|offset| newest.is_none_or(|newest| offset > newest + 1));
        let from = if stale && self.forgets_stale_offsets.load(Ordering::Relaxed) {
            0
        } else {
            offset.unwrap_or(0)
        };
        updates.iter().filter(|update| id(update) >= from).cloned().collect()
    }

    /// Hands new updates over, waking a long poll that is waiting for them.
    async fn replace_updates(&self, updates: Vec<serde_json::Value>) {
        *self.updates.lock().await = updates;
        self.arrived.notify_waiters();
    }

    async fn calls_to(&self, method: &str) -> Vec<serde_json::Value> {
        self.calls
            .lock()
            .await
            .iter()
            .filter(|(called, _)| called == method)
            .map(|(_, body)| body.clone())
            .collect()
    }

    /// Who was answered, in order.
    async fn answered(&self) -> Vec<serde_json::Value> {
        self.calls_to("sendMessage")
            .await
            .iter()
            .map(|sent| sent["chat_id"].clone())
            .collect()
    }
}

async fn method(
    State(stub): State<Telegram>,
    Path((_token, method)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    stub.calls.lock().await.push((method.clone(), body.clone()));
    if method == "sendMessage" && stub.hold_replies.load(Ordering::Relaxed) {
        stub.release.notified().await;
    }
    if method == "getUpdates" {
        // A long poll, as Telegram answers one: at once when something is there, otherwise when
        // something arrives or the wait asked for is over.
        let offset = body["offset"].as_i64();
        let wait = if stub.long_polls.load(Ordering::Relaxed) {
            body["timeout"].as_u64().unwrap_or(0)
        } else {
            0
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(wait);
        loop {
            let arrived = stub.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            let result = stub.pending(offset).await;
            if !result.is_empty() || tokio::time::Instant::now() >= deadline {
                return Json(serde_json::json!({ "ok": true, "result": result }));
            }
            let _ = tokio::time::timeout_at(deadline, arrived).await;
        }
    }
    Json(serde_json::json!({ "ok": true, "result": true }))
}

async fn stub_telegram() -> (Bot, Telegram) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address: SocketAddr = listener.local_addr().expect("an address");
    let stub = Telegram {
        base_url: format!("http://{address}"),
        ..Telegram::default()
    };
    let app = axum::Router::new()
        .route("/{token}/{method}", post(method))
        .with_state(stub.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let bot = Bot::new(BotToken::new(common::TOKEN)).with_base_url(stub.base_url.clone());
    (bot, stub)
}

/// An inbox of its own: a process of its own, as far as claiming updates goes.
fn inbox(app: &Harness, bot: Bot) -> Inbox {
    Inbox::new(app.store.clone(), bot, app.bar, Clock::Fixed(app.now))
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

fn update(value: serde_json::Value) -> Update {
    serde_json::from_value(value).expect("a message")
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

    let answered = inbox(&app, bot)
        .handle(tap(guest.id, &callbacks::cancel_booking(booking)))
        .await
        .expect("the store is reachable");

    assert!(answered, "nobody else had taken this tap in hand");
    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(
        session["bookings"],
        serde_json::json!([]),
        "the table went back"
    );
    let answers = stub.calls_to("answerCallbackQuery").await;
    assert_eq!(
        answers.len(),
        1,
        "Telegram shows a spinner until the tap is answered"
    );
    assert_eq!(answers[0]["callback_query_id"], "query-1");
    assert_eq!(answers[0]["text"], messages::CANCELLED_FROM_REMINDER);
    let edits = stub.calls_to("editMessageReplyMarkup").await;
    assert_eq!(
        edits.len(),
        1,
        "a button that has done its job is taken away"
    );
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
        .await
        .expect("the store is reachable");

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(
        session["bookings"].as_array().expect("bookings").len(),
        1,
        "callback data is whatever a client chose to send"
    );
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
        .await
        .expect("the store is reachable");

    let session = evening
        .get("/api/session", &guest)
        .await
        .expect_ok()
        .clone();
    assert_eq!(session["bookings"].as_array().expect("bookings").len(), 1);
    assert_eq!(stub.calls_to("answerCallbackQuery").await.len(), 1);
}

#[tokio::test]
async fn a_button_carrying_nonsense_is_answered_and_ignored() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    inbox(&app, bot)
        .handle(tap(42, "cancel_booking:not-a-booking"))
        .await
        .expect("the store is reachable");
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

    inbox(&app, bot)
        .handle(update(said(3, guest.id, "/start reminders")))
        .await
        .expect("the store is reachable");

    let session = app.get("/api/session", &guest).await.expect_ok().clone();
    assert_eq!(session["reminders"]["deliverable"], true);
    let sent = stub.calls_to("sendMessage").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["chat_id"], guest.id);
    assert_eq!(
        sent[0]["text"],
        messages::reminders_on(app.config.remind_hours)
    );
}

#[tokio::test]
async fn a_message_nobody_will_read_is_answered_with_where_to_go_instead() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    inbox(&app, bot)
        .handle(update(said(4, 77, "Нас будет восемь, можно?")))
        .await
        .expect("the store is reachable");
    let sent = stub.calls_to("sendMessage").await;
    assert_eq!(sent.len(), 1, "silence reads as being ignored");
    assert_eq!(sent[0]["chat_id"], 77);
    assert_eq!(
        sent[0]["text"],
        messages::nobody_reads_this(&app.config.name, None)
    );
}

#[tokio::test]
async fn starting_the_bot_plainly_says_where_to_go_and_who_answers() {
    for contact in [Some("@podval_bar"), None] {
        let mut config = common::config_with(common::default_tables());
        config.contact = contact.map(str::to_owned);
        let app = common::harness_at(common::morning(), config).await;
        let (bot, stub) = stub_telegram().await;
        inbox(&app, bot)
            .handle(update(said(6, 79, "/start")))
            .await
            .expect("the store is reachable");

        let sent = stub.calls_to("sendMessage").await;
        assert_eq!(sent.len(), 1, "one answer, and only one");
        assert_eq!(
            sent[0]["text"],
            messages::welcome(&app.config.name, contact),
            "the welcome, not the answer to a message nobody reads"
        );
    }
}

#[tokio::test]
async fn start_addressed_to_the_bot_by_name_is_start() {
    // Telegram clients write `/start@PodvalBot` when a command is picked from a list, and a payload
    // follows the name. In a private chat every message is this bot's, whatever name it carries.
    let app = harness().await;
    let welcome = messages::welcome(&app.config.name, None);
    let reminders = messages::reminders_on(app.config.remind_hours);
    let cases = [
        (20, "/start@PodvalBot", welcome.clone()),
        (21, "/start@PodvalBot reminders", reminders.clone()),
        (
            22,
            "/started",
            messages::nobody_reads_this(&app.config.name, None),
        ),
        (23, "/start@", welcome.clone()),
        (24, "/start@SomeOtherName reminders", reminders.clone()),
        (25, "/start\treminders", reminders.clone()),
        (26, "/start\nreminders", reminders.clone()),
        (27, "/start   reminders", reminders.clone()),
        (28, "/start@PodvalBot\u{a0}reminders", reminders.clone()),
    ];
    for (update_id, text, expected) in cases {
        let (bot, stub) = stub_telegram().await;
        inbox(&app, bot)
            .handle(update(said(update_id, 80, text)))
            .await
            .expect("the store is reachable");
        let sent = stub.calls_to("sendMessage").await;
        assert_eq!(sent.len(), 1, "{text}");
        assert_eq!(sent[0]["text"], expected, "{text}");
    }
}

#[tokio::test]
async fn polling_hands_each_update_over_once_and_moves_past_it() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(7, 77, "привет"));
    let inbox = inbox(&app, bot);

    let next = inbox.poll_once(None).await.expect("telegram answered");
    assert_eq!(next, Some(8));
    inbox.poll_once(next).await.expect("telegram answered");

    let polls = stub.calls_to("getUpdates").await;
    assert_eq!(
        polls[1]["offset"], 8,
        "confirming update 7 so Telegram stops sending it"
    );
    assert_eq!(
        stub.calls_to("sendMessage").await.len(),
        1,
        "handled once, not twice"
    );
}

#[tokio::test]
async fn a_second_process_fetching_an_update_the_first_already_answered_does_not_answer_it_again() {
    // A crash, or Telegram out of reach while stopping, leaves the next process fetching from the
    // beginning what the last one already answered.
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(7, 77, "привет"));

    inbox(&app, bot.clone())
        .poll_once(None)
        .await
        .expect("telegram answered");
    let next = inbox(&app, bot)
        .poll_once(None)
        .await
        .expect("telegram answered");

    assert_eq!(
        next,
        Some(8),
        "an update somebody else answered is settled all the same"
    );
    assert_eq!(stub.answered().await, vec![serde_json::json!(77)]);
}

#[tokio::test]
async fn two_processes_handling_one_update_at_once_answer_it_once() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    let first = inbox(&app, bot.clone());
    let second = inbox(&app, bot);
    let arrived = update(said(7, 77, "привет"));

    let (one, other) = tokio::join!(first.handle(arrived.clone()), second.handle(arrived));

    let answered = [one.expect("reachable"), other.expect("reachable")];
    assert_eq!(
        answered.iter().filter(|took| **took).count(),
        1,
        "{answered:?}"
    );
    assert_eq!(stub.answered().await, vec![serde_json::json!(77)]);
}

#[tokio::test]
async fn an_update_is_answered_once_however_many_processes_read_the_inbox() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(7, 77, "привет"));
    inbox(&app, bot.clone())
        .poll_once(None)
        .await
        .expect("telegram answered");

    stub.updates.lock().await.push(said(8, 78, "ещё раз"));
    inbox(&app, bot)
        .poll_once(None)
        .await
        .expect("telegram answered");

    assert_eq!(
        stub.answered().await,
        vec![serde_json::json!(77), serde_json::json!(78)]
    );
}

#[tokio::test]
async fn an_update_claimed_longer_ago_than_telegram_keeps_updates_does_not_silence_a_new_one_with_its_id()
 {
    // After a quiet week Telegram counts update ids afresh from a random number, so a new update can
    // carry the id of one answered long ago.
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(900, 77, "привет"));
    inbox(&app, bot.clone())
        .poll_once(None)
        .await
        .expect("telegram answered");

    let week_later = app.at(morning() + chrono::TimeDelta::days(8));
    {
        let mut updates = stub.updates.lock().await;
        updates.clear();
        updates.push(said(900, 78, "снова мы"));
    }
    inbox(&week_later, bot)
        .poll_once(None)
        .await
        .expect("telegram answered");

    assert_eq!(
        stub.answered().await,
        vec![serde_json::json!(77), serde_json::json!(78)]
    );
}

#[tokio::test]
async fn claims_older_than_telegram_keeps_updates_are_cleared_away() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(900, 77, "привет"));
    inbox(&app, bot.clone())
        .poll_once(None)
        .await
        .expect("telegram answered");

    let two_days_later = app.at(morning() + chrono::TimeDelta::days(2));
    {
        let mut updates = stub.updates.lock().await;
        updates.clear();
        updates.push(said(12, 78, "снова мы"));
    }
    inbox(&two_days_later, bot)
        .poll_once(None)
        .await
        .expect("telegram answered");

    let kept: Vec<i64> = sqlx::query_scalar("select update_id from bot_update")
        .fetch_all(app.store.pool())
        .await
        .expect("read");
    assert_eq!(kept, vec![12]);
}

#[tokio::test]
async fn another_bot_counts_its_own_updates() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(7, 77, "привет"));
    inbox(&app, bot)
        .poll_once(None)
        .await
        .expect("telegram answered");

    let other = Bot::new(BotToken::new(
        "654321:AAHanotherFakeTokenForTests-000000000000",
    ))
    .with_base_url(stub.base_url.clone());
    inbox(&app, other)
        .poll_once(None)
        .await
        .expect("telegram answered");

    assert_eq!(
        stub.answered().await,
        vec![serde_json::json!(77), serde_json::json!(77)],
        "update 7 of one bot is not update 7 of another"
    );
}

#[tokio::test]
async fn a_stop_while_answering_still_tells_telegram_what_was_answered() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.hold_replies.store(true, Ordering::Relaxed);
    stub.updates.lock().await.push(said(7, 77, "привет"));
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let running = tokio::spawn(inbox(&app, bot).run(stopped));

    tokio::time::timeout(Duration::from_secs(5), async {
        while stub.calls_to("sendMessage").await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the message is being answered");
    stop.send(true).expect("the inbox is listening");
    stub.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("the inbox stopped")
        .expect("the inbox did not panic");

    let polls = stub.calls_to("getUpdates").await;
    let last = polls.last().expect("polled");
    assert_eq!(
        last["offset"], 8,
        "otherwise the next process fetches update 7 again: {polls:?}"
    );
    assert_eq!(
        last["timeout"], 0,
        "a stop does not wait for more: {polls:?}"
    );
    assert_eq!(stub.answered().await, vec![serde_json::json!(77)]);
}

#[tokio::test]
async fn an_update_that_cannot_be_claimed_is_left_for_the_next_fetch() {
    // The database is gone. Answering without a claim could answer twice, and moving past the update
    // would drop it for good; the inbox does neither and waits.
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.updates.lock().await.push(said(7, 77, "привет"));
    app.store.pool().close().await;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let running = tokio::spawn(inbox(&app, bot).run(stopped));

    tokio::time::timeout(Duration::from_secs(5), async {
        while stub.calls_to("getUpdates").await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the inbox polled");
    tokio::time::sleep(Duration::from_millis(200)).await;
    stop.send(true).expect("the inbox is listening");
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("the inbox stopped")
        .expect("the inbox did not panic");

    let polls = stub.calls_to("getUpdates").await;
    assert!(
        polls.iter().all(|poll| poll["offset"].is_null()),
        "nothing was settled, so nothing is confirmed: {polls:?}"
    );
    assert_eq!(polls.len(), 1, "it waits before asking again: {polls:?}");
    assert!(stub.answered().await.is_empty());
}

#[tokio::test]
async fn after_telegram_counts_update_ids_afresh_the_next_update_is_answered_once_and_confirmed() {
    // After a quiet week Telegram numbers updates from a random start, which can be below the offset
    // the inbox last confirmed. An offset kept at the old high id confirms nothing Telegram now has,
    // and the same update comes back on every poll.
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.forgets_stale_offsets.store(true, Ordering::Relaxed);
    stub.long_polls.store(true, Ordering::Relaxed);
    stub.replace_updates(vec![said(900, 77, "привет")]).await;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let running = tokio::spawn(inbox(&app, bot).run(stopped));

    let answered = |count: usize| {
        let stub = stub.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(5), async {
                while stub.answered().await.len() < count {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("answered");
        }
    };
    answered(1).await;
    stub.replace_updates(vec![said(5, 78, "снова мы")]).await;
    answered(2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let polls = stub.calls_to("getUpdates").await.len();
    stop.send(true).expect("the inbox is listening");
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("the inbox stopped")
        .expect("the inbox did not panic");

    assert!(polls <= 4, "the inbox asked {polls} times in a moment");
    assert_eq!(
        stub.answered().await,
        vec![serde_json::json!(77), serde_json::json!(78)]
    );
    let last = stub.calls_to("getUpdates").await.last().cloned().expect("polled");
    assert_eq!(last["offset"], 6, "update 5 is confirmed: {last}");
}

#[tokio::test]
async fn an_update_taken_in_hand_but_not_answered_is_answered_on_the_next_fetch_exactly_once() {
    // The claim is written, and then the inbox cannot read what the answer needs — or the database
    // wrote the claim and the reply saying so was lost. Nothing has been said to the guest; the claim
    // is this inbox's own, so the next fetch takes it again and answers.
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.replace_updates(vec![said(7, 77, "привет")]).await;
    let first = inbox(&app, bot.clone());
    let set_timezone = |name: &'static str| {
        let pool = app.store.pool().clone();
        let bar = app.bar;
        async move {
            sqlx::query("update bar set timezone = $2 where id = $1")
                .bind(bar)
                .bind(name)
                .execute(&pool)
                .await
                .expect("written");
        }
    };

    set_timezone("Mars/Olympus").await;
    let failed = first.poll_once(None).await;
    assert!(failed.is_err(), "{failed:?}");
    assert!(stub.answered().await.is_empty());

    set_timezone("Europe/Belgrade").await;
    let next = first.poll_once(None).await.expect("answered");
    assert_eq!(next, Some(8));
    assert_eq!(stub.answered().await, vec![serde_json::json!(77)]);

    inbox(&app, bot)
        .poll_once(None)
        .await
        .expect("telegram answered");
    assert_eq!(
        stub.answered().await,
        vec![serde_json::json!(77)],
        "another process still leaves it alone"
    );
}

/// A tap on a button, as `getUpdates` hands it over.
fn tapped(update_id: i64, from: i64, data: &str) -> serde_json::Value {
    serde_json::json!({
        "update_id": update_id,
        "callback_query": {
            "id": format!("query-{update_id}"),
            "from": { "id": from, "is_bot": false, "first_name": "Гость" },
            "message": { "message_id": 55, "date": 0, "chat": { "id": from, "type": "private" } },
            "chat_instance": "1",
            "data": data
        }
    })
}

/// Makes every read of the bar's configuration fail, or work again.
async fn set_timezone(app: &Harness, name: &str) {
    sqlx::query("update bar set timezone = $2 where id = $1")
        .bind(app.bar)
        .bind(name)
        .execute(app.store.pool())
        .await
        .expect("written");
}

async fn eventually(what: &str, mut done: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !done().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

#[tokio::test]
async fn a_stop_after_telegram_counted_ids_afresh_confirms_an_offset_lower_than_the_last_one() {
    // The inbox last confirmed 901. Telegram now numbers from 5: update 5 is answered and update 6
    // cannot be yet. Stopping must hand Telegram 6, or the next process answers 5 again.
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.forgets_stale_offsets.store(true, Ordering::Relaxed);
    stub.long_polls.store(true, Ordering::Relaxed);
    stub.replace_updates(vec![said(900, 77, "привет")]).await;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let running = tokio::spawn(inbox(&app, bot).run(stopped));

    eventually("update 900 is confirmed", async || {
        stub.calls_to("getUpdates")
            .await
            .iter()
            .any(|poll| poll["offset"] == 901)
    })
    .await;
    set_timezone(&app, "Mars/Olympus").await;
    stub.replace_updates(vec![
        tapped(5, 78, "cancel_booking:not-a-booking"),
        said(6, 79, "снова мы"),
    ])
    .await;
    eventually("update 6 is taken in hand", async || {
        sqlx::query_scalar::<_, bool>("select exists (select 1 from bot_update where update_id = 6)")
            .fetch_one(app.store.pool())
            .await
            .expect("read")
    })
    .await;
    stop.send(true).expect("the inbox is listening");
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("the inbox stopped")
        .expect("the inbox did not panic");

    assert_eq!(stub.calls_to("answerCallbackQuery").await.len(), 1, "update 5 was answered");
    let polls = stub.calls_to("getUpdates").await;
    let last = polls.last().expect("polled");
    assert_eq!(last["offset"], 6, "{polls:?}");
    assert_eq!(last["timeout"], 0, "{polls:?}");
}

#[tokio::test]
async fn an_update_that_cannot_be_answered_is_let_go_after_its_retries_and_the_next_is_answered() {
    // The claim is fine; reading what the answer needs fails, every time. Trying for ever would leave
    // every update behind it unanswered for as long as the fault lasts.
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    stub.replace_updates(vec![
        said(7, 77, "привет"),
        tapped(8, 78, "cancel_booking:not-a-booking"),
    ])
    .await;
    set_timezone(&app, "Mars/Olympus").await;
    let inbox = inbox(&app, bot);

    // The first try and every retry but the last fail the poll; the last failure lets update 7 go.
    for attempt in 1..=pustol_api::inbox::ANSWER_RETRIES {
        let failed = inbox.poll_once(None).await;
        assert!(failed.is_err(), "attempt {attempt}: {failed:?}");
    }
    let next = inbox
        .poll_once(None)
        .await
        .expect("update 7 is let go, and update 8 answered");
    assert_eq!(next, Some(9));
    assert_eq!(stub.calls_to("answerCallbackQuery").await.len(), 1);
    assert!(stub.answered().await.is_empty(), "update 7 was never answered");

    assert_eq!(inbox.poll_once(next).await.expect("telegram answered"), Some(9));
    let polls = stub.calls_to("getUpdates").await;
    assert_eq!(polls.last().expect("polled")["offset"], 9, "update 7 is not fetched again");
}

#[tokio::test]
async fn confirming_hands_telegram_the_offset_without_waiting() {
    let app = harness().await;
    let (bot, stub) = stub_telegram().await;
    inbox(&app, bot)
        .confirm(8)
        .await
        .expect("telegram answered");

    let polls = stub.calls_to("getUpdates").await;
    assert_eq!(polls.len(), 1);
    assert_eq!(polls[0]["offset"], 8);
    assert_eq!(polls[0]["timeout"], 0);
}

#[tokio::test]
async fn the_reply_to_an_unread_message_names_who_will_read_one() {
    let mut config = common::config_with(common::default_tables());
    config.contact = Some("@podval_bar".to_owned());
    let app = common::harness_at(common::morning(), config).await;
    let (bot, stub) = stub_telegram().await;
    inbox(&app, bot)
        .handle(update(said(5, 78, "Можно на восьмерых?")))
        .await
        .expect("the store is reachable");
    let sent = stub.calls_to("sendMessage").await;
    assert_eq!(
        sent[0]["text"],
        messages::nobody_reads_this(&app.config.name, Some("@podval_bar"))
    );
}
