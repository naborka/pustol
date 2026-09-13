use std::time::{Duration, Instant};

use pustol_telegram::{Bot, BotToken, SendError};

const TOKEN: &str = "123456:AAHsecretPartThatMustNeverBeStored-00000";

#[tokio::test]
async fn a_network_failure_does_not_carry_the_token() {
    // Nothing listens on port 1: connection refused at once.
    let bot = Bot::new(BotToken::new(TOKEN)).with_base_url("http://127.0.0.1:1");

    let failure = bot
        .send_message(1, "hello", &[])
        .await
        .expect_err("nothing listens there");

    assert!(
        matches!(failure, SendError::Transient(_)),
        "got {failure:?}"
    );
    let written = failure.to_string();
    assert!(
        !written.contains("secretPart"),
        "the outbox stores this text in last_error: {written}"
    );
}

#[tokio::test]
async fn a_telegram_that_never_answers_is_a_transient_failure_rather_than_a_hang() {
    // Accepts, then silent, like stuck proxy.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let bot = Bot::new(BotToken::new(TOKEN))
        .with_base_url(format!("http://{address}"))
        .with_timeout(Duration::from_millis(200));

    let started = Instant::now();
    let failure = bot
        .send_message(1, "hello", &[])
        .await
        .expect_err("no answer");
    assert!(
        matches!(failure, SendError::Transient(_)),
        "got {failure:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn an_update_with_a_part_nobody_can_read_still_carries_its_id() {
    // Batch confirmed by highest id; unparseable update would be refetched every poll.
    let update: pustol_telegram::Update = serde_json::from_value(serde_json::json!({
        "update_id": 5,
        "message": { "surprise": true }
    }))
    .expect("the id is always readable");
    assert_eq!(update.update_id, 5);
    assert!(update.message.is_none());
}
