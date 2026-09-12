//! The session a fresh Telegram payload is exchanged for, attacked the way the payload is.

use chrono::{DateTime, TimeDelta, Utc};
use pustol_telegram::init_data::{BotToken, TelegramUser, VerifyError, verify};
use pustol_telegram::session::{issue, verify_session};

const TOKEN: &str = "123456:AAHfakeTokenForTestsOnly-000000000000000";

fn token() -> BotToken {
    BotToken::new(TOKEN)
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_785_000_000, 0).expect("valid instant")
}

fn user(id: i64) -> TelegramUser {
    TelegramUser {
        id,
        first_name: "Анна".to_owned(),
        last_name: None,
        username: Some("anna_mgr".to_owned()),
        language_code: Some("ru".to_owned()),
        is_premium: None,
        photo_url: None,
    }
}

#[test]
fn a_session_this_bot_issued_names_its_account_until_it_ends() {
    let ends = now() + TimeDelta::hours(24);
    let session = issue(&user(7), ends, &token());

    let verified = verify_session(&session, &token(), now() + TimeDelta::hours(23)).expect("valid");
    assert_eq!(verified.user, user(7));
    assert_eq!(verified.expires_at, ends);

    assert_eq!(
        verify_session(&session, &token(), ends),
        Err(VerifyError::SessionEnded),
        "the end is the end, not one more request"
    );
}

#[test]
fn a_session_another_bot_issued_is_refused() {
    let session = issue(&user(7), now() + TimeDelta::hours(1), &BotToken::new("999:other"));
    assert_eq!(verify_session(&session, &token(), now()), Err(VerifyError::BadSignature));
}

#[test]
fn changing_a_session_in_any_way_invalidates_it() {
    let session = issue(&user(7), now() + TimeDelta::hours(1), &token());
    let forged = issue(&user(8), now() + TimeDelta::hours(1), &token());
    let (_, their_signature) = session.split_once('.').expect("two parts");
    let (their_claims, _) = forged.split_once('.').expect("two parts");
    let swapped = format!("{their_claims}.{their_signature}");
    assert_eq!(verify_session(&swapped, &token(), now()), Err(VerifyError::BadSignature));
}

#[test]
fn a_session_and_a_telegram_payload_cannot_stand_in_for_each_other() {
    // Different keys for different proofs: neither can be replayed as the other.
    let session = issue(&user(7), now() + TimeDelta::hours(1), &token());
    assert!(verify(&session, &token(), now(), TimeDelta::hours(1)).is_err());
    assert!(verify_session("auth_date=1&hash=00", &token(), now()).is_err());
}

#[test]
fn nonsense_is_refused_without_panicking() {
    for nonsense in ["", ".", "zz.zz", "abc", "7b7d.", ".00"] {
        assert!(verify_session(nonsense, &token(), now()).is_err(), "{nonsense:?}");
    }
}
