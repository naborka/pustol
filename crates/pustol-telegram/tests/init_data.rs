//! The one piece of security-critical arithmetic in this system, attacked from every angle a
//! forged request could take.

use chrono::{DateTime, TimeDelta, Utc};
use pustol_telegram::init_data::{BotToken, VerifyError, sign_for_tests, verify};

const TOKEN: &str = "123456:AAHfakeTokenForTestsOnly-000000000000000";
const MAX_AGE: TimeDelta = TimeDelta::hours(1);

fn token() -> BotToken {
    BotToken::new(TOKEN)
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_785_000_000, 0).expect("valid instant")
}

fn user_json(id: i64) -> String {
    format!(
        r#"{{"id":{id},"first_name":"Алексей","last_name":"К.","username":"alexey","language_code":"ru","is_premium":true}}"#
    )
}

/// A payload as Telegram would send it, signed a moment ago.
fn genuine(id: i64) -> String {
    let auth_date = now().timestamp().to_string();
    sign_for_tests(
        &[
            ("auth_date", &auth_date),
            ("chat_type", "sender"),
            ("query_id", "AAHdFakeQueryId"),
            ("user", &user_json(id)),
        ],
        &token(),
    )
}

#[test]
fn a_payload_signed_with_the_bots_token_is_accepted_and_read() {
    let verified = verify(&genuine(999), &token(), now(), MAX_AGE).expect("genuine");
    assert_eq!(verified.user.id, 999);
    assert_eq!(verified.user.first_name, "Алексей");
    assert_eq!(verified.user.username.as_deref(), Some("alexey"));
    assert_eq!(verified.user.language_code.as_deref(), Some("ru"));
    assert_eq!(verified.auth_date, now());
    assert_eq!(verified.query_id.as_deref(), Some("AAHdFakeQueryId"));
    assert_eq!(verified.chat_type.as_deref(), Some("sender"));
}

#[test]
fn a_payload_signed_with_another_token_is_refused() {
    let auth_date = now().timestamp().to_string();
    let forged = sign_for_tests(
        &[("auth_date", &auth_date), ("user", &user_json(999))],
        &BotToken::new("999999:AAHsomeoneElsesToken-00000000000000000"),
    );
    assert_eq!(
        verify(&forged, &token(), now(), MAX_AGE),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn swapping_the_account_number_invalidates_the_signature() {
    // The whole point: without this, taking over another guest's bookings is a text edit.
    let genuine = genuine(999);
    let tampered = genuine.replace("%7B%22id%22%3A999", "%7B%22id%22%3A1000");
    assert_ne!(genuine, tampered, "the substitution must actually apply");
    assert_eq!(
        verify(&tampered, &token(), now(), MAX_AGE),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn changing_any_signed_field_invalidates_the_signature() {
    let genuine = genuine(999);
    for (from, to) in [
        ("chat_type=sender", "chat_type=group"),
        ("query_id=AAHdFakeQueryId", "query_id=AAHdOther"),
    ] {
        let tampered = genuine.replace(from, to);
        assert_ne!(genuine, tampered, "{from} was not present to change");
        assert_eq!(
            verify(&tampered, &token(), now(), MAX_AGE),
            Err(VerifyError::BadSignature),
            "changing {from} went unnoticed"
        );
    }
}

#[test]
fn adding_a_field_to_a_signed_payload_invalidates_it() {
    let smuggled = format!("{}&is_admin=true", genuine(999));
    assert_eq!(
        verify(&smuggled, &token(), now(), MAX_AGE),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn removing_a_signed_field_invalidates_the_payload() {
    let genuine = genuine(999);
    let stripped: String = genuine
        .split('&')
        .filter(|pair| !pair.starts_with("chat_type="))
        .collect::<Vec<_>>()
        .join("&");
    assert_eq!(
        verify(&stripped, &token(), now(), MAX_AGE),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn a_payload_with_no_hash_is_refused_rather_than_trusted() {
    let unsigned = format!("auth_date={}&user={}", now().timestamp(), user_json(999));
    assert_eq!(
        verify(&unsigned, &token(), now(), MAX_AGE),
        Err(VerifyError::MissingHash)
    );
}

#[test]
fn an_empty_payload_is_refused() {
    assert_eq!(
        verify("", &token(), now(), MAX_AGE),
        Err(VerifyError::MissingHash)
    );
}

#[test]
fn a_hash_that_is_not_hexadecimal_is_refused_without_panicking() {
    let broken = format!("auth_date={}&hash=not-hex", now().timestamp());
    assert_eq!(
        verify(&broken, &token(), now(), MAX_AGE),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn a_hash_of_the_wrong_length_is_refused_without_panicking() {
    let broken = format!("auth_date={}&hash=abcd", now().timestamp());
    assert_eq!(
        verify(&broken, &token(), now(), MAX_AGE),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn a_payload_older_than_the_window_is_refused() {
    // Telegram never expires initData, so a payload captured from a screenshot or a proxy log would
    // otherwise authenticate its owner for ever.
    let stale = genuine(999);
    let much_later = now() + TimeDelta::hours(2);
    assert_eq!(
        verify(&stale, &token(), much_later, MAX_AGE),
        Err(VerifyError::Stale {
            age_seconds: 7200,
            allowed_seconds: 3600,
        })
    );
}

#[test]
fn a_payload_at_the_very_edge_of_the_window_is_still_accepted() {
    let edge = now() + MAX_AGE;
    assert!(verify(&genuine(999), &token(), edge, MAX_AGE).is_ok());
    assert!(
        verify(&genuine(999), &token(), edge + TimeDelta::seconds(1), MAX_AGE).is_err(),
        "one second past the window is past the window"
    );
}

#[test]
fn a_payload_signed_in_the_future_is_refused() {
    let earlier = now() - TimeDelta::minutes(5);
    assert_eq!(
        verify(&genuine(999), &token(), earlier, MAX_AGE),
        Err(VerifyError::SignedInTheFuture)
    );
}

#[test]
fn a_correctly_signed_payload_with_no_auth_date_is_refused() {
    let signed = sign_for_tests(&[("user", &user_json(999))], &token());
    assert_eq!(
        verify(&signed, &token(), now(), MAX_AGE),
        Err(VerifyError::MissingAuthDate)
    );
}

#[test]
fn an_auth_date_that_is_not_a_timestamp_is_refused() {
    let signed = sign_for_tests(
        &[("auth_date", "yesterday"), ("user", &user_json(999))],
        &token(),
    );
    assert_eq!(
        verify(&signed, &token(), now(), MAX_AGE),
        Err(VerifyError::MalformedAuthDate("yesterday".to_owned()))
    );
}

#[test]
fn a_correctly_signed_payload_with_no_user_is_refused() {
    // Signature alone does not identify anybody; every request has to say whose it is.
    let auth_date = now().timestamp().to_string();
    let signed = sign_for_tests(&[("auth_date", &auth_date)], &token());
    assert_eq!(
        verify(&signed, &token(), now(), MAX_AGE),
        Err(VerifyError::MissingUser)
    );
}

#[test]
fn a_user_field_that_is_not_the_documented_shape_is_refused() {
    let auth_date = now().timestamp().to_string();
    let signed = sign_for_tests(
        &[("auth_date", &auth_date), ("user", "{\"id\":\"nine\"}")],
        &token(),
    );
    assert!(matches!(
        verify(&signed, &token(), now(), MAX_AGE),
        Err(VerifyError::MalformedUser(_))
    ));
}

#[test]
fn a_user_with_only_the_required_fields_is_accepted() {
    let auth_date = now().timestamp().to_string();
    let signed = sign_for_tests(
        &[
            ("auth_date", &auth_date),
            ("user", r#"{"id":42,"first_name":"Вера"}"#),
        ],
        &token(),
    );
    let verified = verify(&signed, &token(), now(), MAX_AGE).expect("genuine");
    assert_eq!(verified.user.id, 42);
    assert_eq!(verified.user.username, None);
}

#[test]
fn a_payload_carrying_telegrams_ed25519_signature_still_verifies() {
    // Recent clients add a `signature` field for third-party validation. Implementations disagree
    // about whether it belongs in the HMAC's data-check-string, so both readings are accepted;
    // either way an attacker still needs the bot token.
    let auth_date = now().timestamp().to_string();

    let without = sign_for_tests(
        &[("auth_date", &auth_date), ("user", &user_json(999))],
        &token(),
    );
    let with_signature = format!("{without}&signature=aBcDeF0123456789");
    assert!(
        verify(&with_signature, &token(), now(), MAX_AGE).is_ok(),
        "a signature not covered by the hash must not break verification"
    );

    let covered = sign_for_tests(
        &[
            ("auth_date", &auth_date),
            ("signature", "aBcDeF0123456789"),
            ("user", &user_json(999)),
        ],
        &token(),
    );
    assert!(
        verify(&covered, &token(), now(), MAX_AGE).is_ok(),
        "a signature covered by the hash must also verify"
    );
}

#[test]
fn percent_escapes_and_plus_signs_are_decoded_the_way_telegram_encodes_them() {
    // The signed text is built from decoded values, so a decoder that differed from Telegram's by
    // one character would reject every genuine payload.
    let auth_date = now().timestamp().to_string();
    let signed = sign_for_tests(
        &[
            ("auth_date", &auth_date),
            ("start_param", "a b&c=d"),
            ("user", &user_json(999)),
        ],
        &token(),
    );
    let verified = verify(&signed, &token(), now(), MAX_AGE).expect("genuine");
    assert_eq!(verified.start_param.as_deref(), Some("a b&c=d"));
}

#[test]
fn a_bot_token_never_prints_itself() {
    // A token in one log line is a token in every aggregator downstream, and it is the only thing
    // standing between the internet and impersonating any guest.
    let rendered = format!("{:?}", token());
    assert!(!rendered.contains("AAHfake"));
    assert_eq!(rendered, "BotToken(redacted)");
}
