//! The whole stack, driven through HTTP, against a real database and a stub Telegram.
//!
//! Handlers are tested at this level rather than by calling them directly because most of what can
//! go wrong here is in the wiring: the authorisation extractor, the status codes, the shapes on the
//! wire. A test that calls a handler function bypasses every one of those.

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use http_body_util::BodyExt;
use pustol_api::{AppState, Clock, router};
use pustol_db::Store;
use pustol_db::ids::BarId;
use pustol_domain::config::{BarConfig, DayHours, StaffMember, ValidConfig, WeekSchedule};
use pustol_domain::schedule::{BarTable, TableId, Zone};
use pustol_telegram::init_data::{BotToken, sign_for_tests};
use pustol_telegram::Bot;
use tower::ServiceExt;
use uuid::Uuid;

pub const TOKEN: &str = "123456:AAHfakeTokenForTestsOnly-000000000000000";
pub const BELGRADE: chrono_tz::Tz = chrono_tz::Europe::Belgrade;

static NEXT_ACCOUNT: AtomicI64 = AtomicI64::new(1);
static NEXT_DATABASE: AtomicI64 = AtomicI64::new(1);

/// A running app: the router, the state it was built with, and the bar it serves.
pub struct Harness {
    pub app: Router,
    pub bar: BarId,
    pub store: Store,
    pub config: ValidConfig,
    pub now: DateTime<Utc>,
}

fn cluster_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/pustol".to_owned())
}

fn pointing_at(url: &str, database: &str) -> String {
    let (base, query) = url
        .split_once('?')
        .map_or((url, ""), |(base, query)| (base, query));
    let stem = base.rsplit_once('/').map_or(base, |(stem, _)| stem);
    if query.is_empty() {
        format!("{stem}/{database}")
    } else {
        format!("{stem}/{database}?{query}")
    }
}

async fn fresh_store() -> Store {
    let cluster = cluster_url();
    let name = format!(
        "pustol_t{}_api{}",
        std::process::id(),
        NEXT_DATABASE.fetch_add(1, Ordering::Relaxed)
    );
    let maintenance = pointing_at(&cluster, "postgres");
    let admin = sqlx::PgPool::connect(&maintenance)
        .await
        .unwrap_or_else(|error| panic!("no cluster at {maintenance}: {error}"));
    // `create database` takes no bind parameters; the name is built here and never supplied.
    sqlx::query(sqlx::AssertSqlSafe(format!("create database \"{name}\"")))
        .execute(&admin)
        .await
        .unwrap_or_else(|error| panic!("cannot create {name}: {error}"));
    admin.close().await;

    let store = Store::connect(&pointing_at(&cluster, &name), 12)
        .await
        .expect("the new database accepts connections");
    store.migrate().await.expect("migrations apply");
    store
}

/// Thursday 30 July 2026 at 06:00 UTC — early morning in Belgrade, before any fixture booking.
pub fn morning() -> DateTime<Utc> {
    utc(2026, 7, 30, 6, 0)
}

pub fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("valid instant")
}

pub fn thursday() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 30).expect("valid date")
}

pub fn zone(name: &str) -> Zone {
    Zone::new(name).expect("not blank")
}

pub fn table(number: i32, seats: i32, zone_name: &str) -> BarTable {
    BarTable {
        id: TableId(Uuid::new_v4()),
        number,
        seats,
        zone: zone(zone_name),
        retired: false,
    }
}

pub fn default_tables() -> Vec<BarTable> {
    let mut tables = Vec::new();
    for number in 1..=4 {
        tables.push(table(number, 2, "Бар"));
    }
    for number in 5..=10 {
        tables.push(table(number, 4, "Зал"));
    }
    for number in 11..=13 {
        tables.push(table(number, 6, "Зал"));
    }
    tables.push(table(14, 4, "Терраса"));
    tables.push(table(15, 6, "Терраса"));
    tables
}

pub fn config_with(tables: Vec<BarTable>) -> BarConfig {
    let largest = tables
        .iter()
        .filter(|table| table.is_active())
        .map(|table| table.seats)
        .max()
        .unwrap_or(2);
    BarConfig {
        name: "Бар «Подвал»".to_owned(),
        address: "Дечанска 12, Белград".to_owned(),
        timezone: BELGRADE,
        week: WeekSchedule::uniform(DayHours {
            open_minutes: 600,
            close_minutes: 1560,
            closed: false,
        }),
        zones: vec![zone("Бар"), zone("Зал"), zone("Терраса")],
        tables,
        turn_minutes: 120,
        slot_step_minutes: 30,
        max_party: largest.clamp(2, 10),
        horizon_days: 4,
        remind_hours: 3,
        grace_minutes: 15,
        message_templates: vec![
            "Ваш стол готов, ждём вас!".to_owned(),
            "Опаздываете? Держим стол ещё 15 минут.".to_owned(),
        ],
        cancel_reasons: vec![
            "Технические проблемы в баре".to_owned(),
            "Частное мероприятие".to_owned(),
        ],
        staff: vec![StaffMember {
            username: "anna_mgr".to_owned(),
            telegram_user_id: None,
        }],
    }
}

/// Builds an app whose clock is stopped at `now`.
pub async fn harness_at(now: DateTime<Utc>, config: BarConfig) -> Harness {
    let store = fresh_store().await;
    let config = ValidConfig::new(config)
        .unwrap_or_else(|errors| panic!("fixture config is illegal: {errors:?}"));
    let bar = store.create_bar(&config).await.expect("bar created");
    // The bot points at an address nothing listens on: no test here exercises delivery, and a stub
    // that silently accepted sends would make a broken outbox look healthy.
    let bot = Bot::new(BotToken::new(TOKEN), reqwest::Client::new())
        .with_base_url("http://127.0.0.1:1");
    let state = AppState::new(
        store.clone(),
        bot,
        bar,
        BotToken::new(TOKEN),
        Clock::Fixed(now),
    );
    Harness {
        app: router(state),
        bar,
        store,
        config,
        now,
    }
}

pub async fn harness() -> Harness {
    harness_at(morning(), config_with(default_tables())).await
}

/// A Telegram account no other test uses.
#[derive(Clone, Debug)]
pub struct Caller {
    pub id: i64,
    pub first_name: String,
    pub username: Option<String>,
}

impl Caller {
    pub fn new(first_name: &str) -> Self {
        let id = NEXT_ACCOUNT.fetch_add(1, Ordering::Relaxed);
        Self {
            id: 2_000_000 + id,
            first_name: first_name.to_owned(),
            username: Some(format!("guest_{id:06}")),
        }
    }

    /// The invited manager, whose username claims the roster seat on first sight.
    pub fn manager() -> Self {
        let id = NEXT_ACCOUNT.fetch_add(1, Ordering::Relaxed);
        Self {
            id: 3_000_000 + id,
            first_name: "Анна".to_owned(),
            username: Some("anna_mgr".to_owned()),
        }
    }

    /// An account with no username at all, which Telegram permits.
    pub fn anonymous(first_name: &str) -> Self {
        let id = NEXT_ACCOUNT.fetch_add(1, Ordering::Relaxed);
        Self {
            id: 4_000_000 + id,
            first_name: first_name.to_owned(),
            username: None,
        }
    }

    fn user_json(&self) -> String {
        let username = self
            .username
            .as_ref()
            .map_or_else(String::new, |name| format!(r#","username":"{name}""#));
        format!(
            r#"{{"id":{},"first_name":"{}"{username},"language_code":"ru"}}"#,
            self.id, self.first_name
        )
    }

    /// A payload signed the way Telegram signs one, dated `now`.
    pub fn credentials(&self, now: DateTime<Utc>) -> String {
        let auth_date = now.timestamp().to_string();
        let init_data = sign_for_tests(
            &[("auth_date", &auth_date), ("user", &self.user_json())],
            &BotToken::new(TOKEN),
        );
        format!("tma {init_data}")
    }
}

/// One HTTP call, with its status and parsed body.
pub struct Answer {
    pub status: StatusCode,
    pub body: serde_json::Value,
}

impl Answer {
    /// The stable error code, for asserting on a failure without depending on its wording.
    pub fn error_code(&self) -> Option<&str> {
        self.body.get("error")?.get("code")?.as_str()
    }

    pub fn expect_ok(&self) -> &serde_json::Value {
        assert!(
            self.status.is_success(),
            "expected success, got {} {}",
            self.status,
            self.body
        );
        &self.body
    }
}

impl Harness {
    async fn call(&self, request: Request<Body>) -> Answer {
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("a body")
            .to_bytes();
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
            })
        };
        Answer { status, body }
    }

    pub async fn get(&self, path: &str, caller: &Caller) -> Answer {
        self.call(
            Request::builder()
                .uri(path)
                .header(header::AUTHORIZATION, caller.credentials(self.now))
                .body(Body::empty())
                .expect("a request"),
        )
        .await
    }

    pub async fn send(
        &self,
        method: &str,
        path: &str,
        caller: &Caller,
        body: serde_json::Value,
    ) -> Answer {
        self.call(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, caller.credentials(self.now))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("a request"),
        )
        .await
    }

    pub async fn post(&self, path: &str, caller: &Caller, body: serde_json::Value) -> Answer {
        self.send("POST", path, caller, body).await
    }

    /// A call with no credentials at all.
    pub async fn get_anonymously(&self, path: &str) -> Answer {
        self.call(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("a request"),
        )
        .await
    }

    /// A call with the credentials given verbatim, for testing what happens to a stale payload.
    pub async fn send_raw(
        &self,
        method: &str,
        path: &str,
        credentials: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Answer {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(credentials) = credentials {
            request = request.header(header::AUTHORIZATION, credentials);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        };
        self.call(request.expect("a request")).await
    }

    /// A call with a payload signed by somebody else's token.
    pub async fn get_with_forged_credentials(&self, path: &str, caller: &Caller) -> Answer {
        let auth_date = self.now.timestamp().to_string();
        let forged = sign_for_tests(
            &[("auth_date", &auth_date), ("user", &caller.user_json())],
            &BotToken::new("999999:AAHsomebodyElsesToken-0000000000000000"),
        );
        self.call(
            Request::builder()
                .uri(path)
                .header(header::AUTHORIZATION, format!("tma {forged}"))
                .body(Body::empty())
                .expect("a request"),
        )
        .await
    }
}
