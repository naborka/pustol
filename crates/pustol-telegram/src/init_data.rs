//! Proving that a request really came from Telegram.
//!
//! A Mini App is a web page, and a web page can be opened by anybody with the URL. Everything the
//! app claims about who is using it — the account number that decides whose booking this is and
//! whether they may open the admin side — arrives in `initData`, a query string Telegram signs with
//! a key derived from the bot token. Without this check the entire authorisation model is a
//! suggestion: anyone could paste another person's account number and take over their bookings.
//!
//! The algorithm is Telegram's, transcribed exactly:
//!
//! 1. take every field except `hash`, sort by key, join as `key=value` with newlines;
//! 2. `secret = HMAC-SHA256(key = "WebAppData", message = bot token)`;
//! 3. the payload is genuine when `hex(HMAC-SHA256(key = secret, message = data)) == hash`.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// The constant Telegram derives the signing key with.
const KEY_SALT: &[u8] = b"WebAppData";

/// The field carrying the signature, excluded from the signed text.
const HASH_FIELD: &str = "hash";

/// Telegram's newer Ed25519 signature for third-party verification.
///
/// Present in payloads from recent clients. It is not part of the HMAC check and is not verified
/// here — that check exists for parties who do *not* hold the bot token, and we do.
const SIGNATURE_FIELD: &str = "signature";

/// A Telegram account, as Telegram describes it.
#[derive(Clone, PartialEq, Eq, Debug, serde::Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub first_name: String,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub language_code: Option<String>,
    #[serde(default)]
    pub is_premium: Option<bool>,
    #[serde(default)]
    pub photo_url: Option<String>,
}

/// A verified payload. Constructed only by [`verify`], so holding one is proof of authenticity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InitData {
    pub user: TelegramUser,
    pub auth_date: DateTime<Utc>,
    /// The `start_param` a deep link carried, if any.
    pub start_param: Option<String>,
    pub query_id: Option<String>,
    pub chat_type: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("the payload carries no hash")]
    MissingHash,
    #[error("the payload carries no user")]
    MissingUser,
    #[error("the payload's user field is not the shape Telegram documents: {0}")]
    MalformedUser(String),
    #[error("the payload carries no auth_date")]
    MissingAuthDate,
    #[error("auth_date {0} is not a unix timestamp")]
    MalformedAuthDate(String),
    #[error("the hash does not match: this payload was not signed by the bot's token")]
    BadSignature,
    #[error("the payload was signed {age_seconds}s ago, beyond the {allowed_seconds}s accepted")]
    Stale {
        age_seconds: i64,
        allowed_seconds: i64,
    },
    #[error("the payload was signed in the future, which no clock skew explains")]
    SignedInTheFuture,
}

/// The bot's token, kept in a type that will not print itself.
///
/// A token in a log line is a token in every log aggregator downstream, and it is the only secret
/// standing between the internet and the ability to impersonate any guest.
#[derive(Clone)]
pub struct BotToken {
    token: String,
    /// `HMAC-SHA256("WebAppData", token)`, derived once.
    ///
    /// It is a pure function of the token and every request needs it — twice, when the payload
    /// carries Telegram's newer signature field. Deriving it at construction keeps the verifier's
    /// per-request work to the one HMAC that actually depends on the payload.
    signing_key: [u8; 32],
}

impl BotToken {
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        let mut derive = HmacSha256::new_from_slice(KEY_SALT).expect("hmac accepts any key length");
        derive.update(token.as_bytes());
        Self {
            signing_key: derive.finalize().into_bytes().into(),
            token,
        }
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.token
    }
}

impl std::fmt::Debug for BotToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BotToken(redacted)")
    }
}

/// Verifies an `initData` query string and returns what it says.
///
/// `max_age` bounds how long a payload stays usable. Telegram does not expire it, so without a
/// bound a payload captured once — from a shared screenshot, a proxy log, a browser history —
/// authenticates its owner for ever.
pub fn verify(
    init_data: &str,
    token: &BotToken,
    now: DateTime<Utc>,
    max_age: TimeDelta,
) -> Result<InitData, VerifyError> {
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in form_urlencoded_pairs(init_data) {
        fields.insert(key, value);
    }

    let hash = fields.remove(HASH_FIELD).ok_or(VerifyError::MissingHash)?;
    let signature = fields.remove(SIGNATURE_FIELD);

    // Telegram documents the data-check-string as every received field except `hash`. Payloads from
    // recent clients also carry `signature`, and implementations disagree about whether it belongs
    // in the string, so both readings are accepted. Neither widens what an attacker can do: each
    // still requires producing an HMAC under the bot's token.
    let matches = signature.is_some_and(|signature| {
        let mut with_signature = fields.clone();
        with_signature.insert(SIGNATURE_FIELD.to_owned(), signature);
        hmac_matches(&data_check_string(&with_signature), token, &hash)
    }) || hmac_matches(&data_check_string(&fields), token, &hash);

    if !matches {
        return Err(VerifyError::BadSignature);
    }

    let auth_date = fields
        .get("auth_date")
        .ok_or(VerifyError::MissingAuthDate)?;
    let auth_date = auth_date
        .parse::<i64>()
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .ok_or_else(|| VerifyError::MalformedAuthDate(auth_date.clone()))?;

    let age = now - auth_date;
    if age < TimeDelta::zero() {
        return Err(VerifyError::SignedInTheFuture);
    }
    if age > max_age {
        return Err(VerifyError::Stale {
            age_seconds: age.num_seconds(),
            allowed_seconds: max_age.num_seconds(),
        });
    }

    let user = fields.get("user").ok_or(VerifyError::MissingUser)?;
    let user: TelegramUser = serde_json::from_str(user)
        .map_err(|error| VerifyError::MalformedUser(error.to_string()))?;

    Ok(InitData {
        user,
        auth_date,
        start_param: fields.get("start_param").cloned(),
        query_id: fields.get("query_id").cloned(),
        chat_type: fields.get("chat_type").cloned(),
    })
}

fn data_check_string(fields: &BTreeMap<String, String>) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compares in constant time.
///
/// A byte-by-byte comparison that returns early leaks, through timing, how much of a guess was
/// right — which is enough to find a valid hash one byte at a time.
fn hmac_matches(data: &str, token: &BotToken, expected_hex: &str) -> bool {
    let Ok(expected) = hex::decode(expected_hex) else {
        return false;
    };
    let mut mac =
        HmacSha256::new_from_slice(&token.signing_key).expect("hmac accepts any key length");
    mac.update(data.as_bytes());
    let computed = mac.finalize().into_bytes();

    computed.ct_eq(expected.as_slice()).into()
}

/// Decodes an `application/x-www-form-urlencoded` string.
///
/// Written out rather than pulled from a dependency because the signed text is built from the
/// *decoded* values: a decoder that differed from Telegram's by one character — over `+`, over a
/// malformed escape — would reject every genuine payload, and one that differed the other way
/// would accept forged ones.
fn form_urlencoded_pairs(input: &str) -> impl Iterator<Item = (String, String)> + '_ {
    input.split('&').filter(|pair| !pair.is_empty()).map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(key), percent_decode(value))
    })
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let Some(byte) = hex_pair(bytes[index + 1], bytes[index + 2]) {
                    out.push(byte);
                    index += 3;
                } else {
                    // A stray percent that is not an escape stands for itself, which is what
                    // browsers and Telegram's own encoder do.
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(hex_digit(high)? << 4 | hex_digit(low)?)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Signs a payload the way Telegram would.
///
/// Test-only, and the only honest way to test the verifier: fixtures copied from a real client
/// cannot be regenerated when a field is added, and a verifier tested against a hash it computed
/// itself proves nothing unless the signing side is written independently of the checking side.
#[cfg(any(test, feature = "test-signing"))]
pub fn sign_for_tests(fields: &[(&str, &str)], token: &BotToken) -> String {
    let sorted: BTreeMap<String, String> = fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    let data = data_check_string(&sorted);

    let mut secret = HmacSha256::new_from_slice(KEY_SALT).expect("hmac accepts any key length");
    secret.update(token.expose().as_bytes());
    let secret = secret.finalize().into_bytes();
    let mut mac = HmacSha256::new_from_slice(&secret).expect("hmac accepts any key length");
    mac.update(data.as_bytes());
    let hash = hex::encode(mac.finalize().into_bytes());

    let mut query: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("{}={}", key, percent_encode(value)))
        .collect();
    query.push(format!("hash={hash}"));
    query.join("&")
}

#[cfg(any(test, feature = "test-signing"))]
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
