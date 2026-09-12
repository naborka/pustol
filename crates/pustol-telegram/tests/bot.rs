//! The send path, against addresses that fail the way networks fail.

use std::time::{Duration, Instant};

use pustol_telegram::{Bot, BotToken, SendError};

const TOKEN: &str = "123456:AAHsecretPartThatMustNeverBeStored-00000";

#[tokio::test]
async fn a_network_failure_does_not_carry_the_token() {
    // Nothing listens on port 1, so the connection is refused before any byte is exchanged.
    let bot = Bot::new(BotToken::new(TOKEN)).with_base_url("http://127.0.0.1:1");

    let failure = bot
        .send_message(1, "hello", &[])
        .await
        .expect_err("nothing listens there");

    assert!(matches!(failure, SendError::Transient(_)), "got {failure:?}");
    let written = failure.to_string();
    assert!(
        !written.contains("secretPart"),
        "the outbox stores this text in last_error: {written}"
    );
}

#[tokio::test]
async fn a_telegram_that_never_answers_is_a_transient_failure_rather_than_a_hang() {
    // Accepts the connection and then says nothing, which is what a stuck proxy does. Without a
    // bound the outbox waits on this for ever, and so does the shutdown that waits on the outbox.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
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
    let failure = bot.send_message(1, "hello", &[]).await.expect_err("no answer");
    assert!(matches!(failure, SendError::Transient(_)), "got {failure:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}
