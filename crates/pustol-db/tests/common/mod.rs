//! A real `PostgreSQL`, because the invariant this system rests on is enforced by an exclusion
//! constraint and a fake would test nothing.
//!
//! Every test gets its own database.
//!
//! Bar-scoped rows would have been isolated well enough by giving each test its own bar, but the
//! outbox is not bar-scoped: a worker drains everything that is due, which is correct in
//! production and means parallel tests would claim each other's messages and increment each
//! other's attempt counts. Rather than distort the queue API with a filter that only tests want, an
//! entire database is cheap enough to hand out per test.
//!
//! Each test also owns its own connection pool. A pool shared through a process-wide cell would be
//! created inside whichever test ran first, and `#[tokio::test]` gives every test its own runtime:
//! as soon as that first runtime shut down, the pool's background tasks would die and every later
//! test would fail with "a Tokio context was found, but it is being shutdown". Per-test pools have
//! no such lifetime to get wrong.

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pustol_db::ids::{BarId, TelegramUserId};
use pustol_db::bookings::{Channel, NewBooking};
use pustol_db::identity::TelegramAccount;
use pustol_db::Store;
use pustol_domain::config::{BarConfig, DayHours, StaffMember, ValidConfig, WeekSchedule};
use pustol_domain::draft::{DayHoursDraft, Draft, StaffDraft, TableDraft};
use pustol_domain::schedule::{BarTable, TableId, Zone};
use pustol_domain::service_day::{Interval, ServiceDay};
use uuid::Uuid;

pub const BELGRADE: Tz = chrono_tz::Europe::Belgrade;

/// 10:00 to 02:00, every day.
pub const DEFAULT_HOURS: DayHours = DayHours {
    open_minutes: 600,
    close_minutes: 1560,
    closed: false,
};

static NEXT_ACCOUNT: AtomicI64 = AtomicI64::new(1);
static NEXT_DATABASE: AtomicI64 = AtomicI64::new(1);

fn cluster_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/pustol".to_owned())
}

/// The same connection string pointed at another database on the same cluster.
fn pointing_at(url: &str, database: &str) -> String {
    let (base, query) = url.split_once('?').map_or((url, ""), |(base, query)| (base, query));
    let stem = base.rsplit_once('/').map_or(base, |(stem, _)| stem);
    if query.is_empty() {
        format!("{stem}/{database}")
    } else {
        format!("{stem}/{database}?{query}")
    }
}

/// A migrated database of this test's own, with a pool belonging to this test's runtime.
///
/// Databases are named after the process so a crashed run leaves droppings that
/// `scripts/pg.sh start` clears, rather than droppings that collide with the next run.
pub async fn store() -> Store {
    let cluster = cluster_url();
    let name = format!(
        "pustol_t{}_{}",
        std::process::id(),
        NEXT_DATABASE.fetch_add(1, Ordering::Relaxed)
    );

    let maintenance = pointing_at(&cluster, "postgres");
    let admin = sqlx::PgPool::connect(&maintenance).await.unwrap_or_else(|error| {
        panic!("no cluster at {maintenance}: {error}\nrun scripts/pg.sh start")
    });
    // `create database` takes no bind parameters, so the name has to be interpolated. It is built
    // here from a process id and a counter and never from anything a caller supplies.
    sqlx::query(sqlx::AssertSqlSafe(format!("create database \"{name}\"")))
        .execute(&admin)
        .await
        .unwrap_or_else(|error| panic!("cannot create {name}: {error}"));
    admin.close().await;

    let store = Store::connect(&pointing_at(&cluster, &name), 12)
        .await
        .expect("the database just created accepts connections");
    store.migrate().await.expect("migrations apply");
    store
}

/// A Telegram account number no other test will use.
pub fn fresh_account(name: &str) -> TelegramAccount {
    let id = NEXT_ACCOUNT.fetch_add(1, Ordering::Relaxed);
    TelegramAccount {
        id: TelegramUserId(1_000_000 + id),
        username: Some(format!("guest_{id:06}")),
        first_name: name.to_owned(),
        last_name: None,
        language_code: Some("ru".to_owned()),
    }
}

pub fn zone(name: &str) -> Zone {
    Zone::new(name).expect("fixture zone names are not blank")
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

/// The fifteen tables of the prototype.
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

/// A bar with the given room.
///
/// The party cap follows the largest table rather than being fixed, because a cap no table can
/// seat is illegal by construction — the fixture would be asserting against a configuration the
/// bar could never run on.
pub fn config_with(tables: Vec<BarTable>, staff: &str) -> BarConfig {
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
        week: WeekSchedule::uniform(DEFAULT_HOURS),
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
            username: staff.to_owned(),
            telegram_user_id: None,
        }],
    }
}

pub fn default_config() -> BarConfig {
    config_with(default_tables(), "anna_mgr")
}

/// Creates a bar from `config`, panicking if the fixture itself is illegal.
pub async fn bar_with(store: &Store, config: BarConfig) -> (BarId, ValidConfig) {
    let config = ValidConfig::new(config)
        .unwrap_or_else(|errors| panic!("fixture config is illegal: {errors:?}"));
    let bar = store.create_bar(&config).await.expect("bar is created");
    (bar, config)
}

pub async fn default_bar(store: &Store) -> (BarId, ValidConfig) {
    bar_with(store, default_config()).await
}

/// A Thursday in high summer, well away from either clock change.
pub fn thursday() -> ServiceDay {
    ServiceDay::new(NaiveDate::from_ymd_opt(2026, 7, 30).expect("valid date"))
}

pub fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("valid instant")
}

/// Early on the Thursday, before any fixture booking.
pub fn morning() -> DateTime<Utc> {
    utc(2026, 7, 30, 6, 0)
}

/// The instant a wall-clock minute of `day` happens, in the fixture bar's timezone.
pub fn at(day: ServiceDay, minutes: i32) -> DateTime<Utc> {
    pustol_domain::resolve(day, minutes, BELGRADE).expect("fixture times exist")
}

pub fn numbered(tables: &[BarTable], number: i32) -> &BarTable {
    tables
        .iter()
        .find(|table| table.number == number)
        .unwrap_or_else(|| panic!("fixture room has no table {number}"))
}

/// A proposal that changes nothing, ready to be edited by a test.
pub fn draft_of(config: &BarConfig) -> Draft {
    Draft {
        name: config.name.clone(),
        address: config.address.clone(),
        timezone: config.timezone.name().to_owned(),
        week: std::array::from_fn(|index| {
            let hours = config.week.all()[index];
            DayHoursDraft {
                open_minutes: hours.open_minutes,
                close_minutes: hours.close_minutes,
                closed: hours.closed,
            }
        }),
        zones: config.zones.iter().map(ToString::to_string).collect(),
        tables: config
            .active_tables()
            .map(|table| TableDraft::Existing {
                id: table.id.0,
                seats: table.seats,
                zone: table.zone.to_string(),
            })
            .collect(),
        turn_minutes: config.turn_minutes,
        slot_step_minutes: config.slot_step_minutes,
        max_party: config.max_party,
        horizon_days: config.horizon_days,
        remind_hours: config.remind_hours,
        grace_minutes: config.grace_minutes,
        message_templates: config.message_templates.clone(),
        cancel_reasons: config.cancel_reasons.clone(),
        staff: config
            .staff
            .iter()
            .map(|member| StaffDraft {
                username: member.username.clone(),
            })
            .collect(),
    }
}

/// A guest's request for a table on the Thursday shift.
pub fn guest_booking(
    bar: BarId,
    account: &TelegramAccount,
    minutes: i32,
    party_size: i32,
) -> NewBooking {
    NewBooking {
        bar,
        service_day: thursday(),
        start_minutes: minutes,
        party_size,
        channel: Channel::Guest {
            user: account.id,
            name: account.first_name.clone(),
            username: account.username.clone(),
        },
        reminder: Some(reminder_wording),
    }
}

/// A booking staff took by telephone or at the door.
pub fn staff_booking(bar: BarId, name: &str, minutes: i32, party_size: i32) -> NewBooking {
    NewBooking {
        bar,
        service_day: thursday(),
        start_minutes: minutes,
        party_size,
        channel: Channel::Staff {
            guest_name: name.to_owned(),
            table: None,
        },
        // Nobody to remind: a booking taken at the door has no account behind it.
        reminder: None,
    }
}

/// Stands in for the API's own wording, which is not this crate's concern.
pub fn reminder_wording(config: &ValidConfig, window: Interval, party_size: i32) -> String {
    format!(
        "{}: столик на {party_size} в {}",
        config.name,
        window.start()
    )
}

/// Stands in for the API's cancellation copy.
pub fn cancellation_wording(
    config: &ValidConfig,
    record: &pustol_db::records::BookingRecord,
    reason: &str,
) -> String {
    format!("{}: бронь {} отменена — {reason}", config.name, record.guest_name)
}

pub fn move_words() -> pustol_db::bookings::MoveWords {
    pustol_db::bookings::MoveWords {
        notice: |config, record, moved_to| {
            format!(
                "{}: бронь {} перенесена на {}",
                config.name,
                record.guest_name,
                moved_to.start()
            )
        },
        reminder: reminder_wording,
    }
}
