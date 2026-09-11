//! Shared fixtures: the bar the prototype describes, so the tests argue about behaviour rather
//! than about set-up.
//!
//! Each test binary uses a different subset of these helpers, so unused ones are expected.
#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pustol_domain::allocator::{Booking, BookingId, BookingStatus, TableBlock};
use pustol_domain::config::{BarConfig, DayHours, StaffMember, ValidConfig, WeekSchedule};
use pustol_domain::schedule::{BarTable, TableId, Zone};
use pustol_domain::service_day::{Interval, ServiceDay, resolve};
use uuid::Uuid;

pub const BELGRADE: Tz = chrono_tz::Europe::Belgrade;

/// 10:00 to 02:00, every day — the prototype's default week.
pub const DEFAULT_HOURS: DayHours = DayHours {
    open_minutes: 600,
    close_minutes: 1560,
    closed: false,
};

pub fn zone(name: &str) -> Zone {
    Zone::new(name).expect("fixture zone names are not blank")
}

/// Deterministic identifier from a sequence number.
///
/// Random identifiers would make any assertion about ordering pass or fail by luck, which is
/// worse than no assertion at all: a tie-break bug would show up as an occasional red build
/// nobody can reproduce. Fixtures therefore number their rows, and the tests that care about
/// ordering deliberately number them *against* the order they expect, so an implementation that
/// falls back to identifier order cannot pass.
pub fn id(sequence: u128) -> Uuid {
    Uuid::from_u128(sequence)
}

pub fn table(number: i32, seats: i32, zone_name: &str) -> BarTable {
    BarTable {
        id: TableId(id(u128::try_from(number).expect("table numbers are positive"))),
        number,
        seats,
        zone: zone(zone_name),
        retired: false,
    }
}

/// The fifteen tables of the prototype: four two-tops at the bar, six four-tops and three
/// six-tops in the room, a four-top and a six-top on the terrace.
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

pub fn default_config() -> BarConfig {
    BarConfig {
        name: "Бар «Подвал»".to_owned(),
        address: "Дечанска 12, Белград".to_owned(),
        timezone: BELGRADE,
        week: WeekSchedule::uniform(DEFAULT_HOURS),
        zones: vec![zone("Бар"), zone("Зал"), zone("Терраса")],
        tables: default_tables(),
        turn_minutes: 120,
        slot_step_minutes: 30,
        max_party: 6,
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
        staff: vec![
            StaffMember {
                username: "anna_mgr".to_owned(),
                telegram_user_id: Some(1001),
            },
            StaffMember {
                username: "pavel_bar".to_owned(),
                telegram_user_id: None,
            },
        ],
    }
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

/// Instant of a wall-clock minute offset on `day`, in the bar's timezone.
pub fn at(day: ServiceDay, minutes: i32) -> DateTime<Utc> {
    resolve(day, minutes, BELGRADE).expect("fixture times exist")
}

/// A booking of `party_size` arriving `minutes` into `day` and staying `turn_minutes`.
///
/// `sequence` fixes the identifier so that ordering assertions are reproducible.
pub fn booking(
    sequence: u128,
    day: ServiceDay,
    minutes: i32,
    party_size: i32,
    table: Option<&BarTable>,
    turn_minutes: i32,
) -> Booking {
    Booking {
        id: BookingId(id(sequence)),
        table_id: table.map(|found| found.id),
        service_day: day,
        window: Interval::from_duration(at(day, minutes), turn_minutes)
            .expect("fixture turns are positive"),
        released_at: None,
        party_size,
        status: BookingStatus::Confirmed,
    }
}

pub fn block(day: ServiceDay, table: &BarTable) -> TableBlock {
    TableBlock {
        table_id: table.id,
        service_day: day,
    }
}

/// Table with the given printed number, panicking rather than returning an option: a fixture
/// that asks for a table the fixture room does not have is a broken test, not a case to handle.
pub fn numbered(tables: &[BarTable], number: i32) -> &BarTable {
    tables
        .iter()
        .find(|table| table.number == number)
        .unwrap_or_else(|| panic!("fixture room has no table {number}"))
}

/// The default bar with its configuration put in force.
pub fn in_force() -> ValidConfig {
    ValidConfig::new(default_config()).expect("the fixture bar is legal")
}

/// `config` put in force, panicking with the reasons if it is not legal.
pub fn force(config: BarConfig) -> ValidConfig {
    ValidConfig::new(config).unwrap_or_else(|errors| panic!("fixture config is illegal: {errors:?}"))
}
