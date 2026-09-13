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

pub mod database;

use std::sync::atomic::{AtomicI64, Ordering};

use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use pustol_db::Store;
use pustol_db::bookings::{Channel, NewBooking};
use pustol_db::identity::{Signature, TelegramAccount};
use pustol_db::ids::{BarId, TelegramUserId};
use pustol_domain::config::{BarConfig, DayHours, StaffMember, ValidConfig, WeekSchedule};
use pustol_domain::draft::{DayHoursDraft, Draft, StaffDraft, TableDraft};
use pustol_domain::schedule::{BarTable, TableId, Zone};
use pustol_domain::service_day::{Interval, ServiceDay};
use pustol_domain::slots::{Slot, slot_list};
use pustol_domain::{BlockReason, BookingId, GuestName};
use uuid::Uuid;

pub const BELGRADE: Tz = chrono_tz::Europe::Belgrade;

/// 10:00 to 02:00, every day.
pub const DEFAULT_HOURS: DayHours = DayHours {
    open_minutes: 600,
    close_minutes: 1560,
    closed: false,
};

static NEXT_ACCOUNT: AtomicI64 = AtomicI64::new(1);

/// Arrival times, as the picker asks them of the room.
pub struct Offered {
    pub slots: Vec<Slot>,
}

/// The picker's read: the room on a shift, asked for a party's arrival times.
#[allow(async_fn_in_trait)]
pub trait Availability {
    async fn availability(
        &self,
        bar: BarId,
        day: ServiceDay,
        party_size: i32,
        now: DateTime<Utc>,
        ignoring: &[BookingId],
    ) -> pustol_db::Result<Offered>;
}

impl Availability for Store {
    async fn availability(
        &self,
        bar: BarId,
        day: ServiceDay,
        party_size: i32,
        now: DateTime<Utc>,
        ignoring: &[BookingId],
    ) -> pustol_db::Result<Offered> {
        let room = self.room(bar, day).await?;
        Ok(Offered {
            slots: slot_list(&room.query(party_size, now, ignoring)),
        })
    }
}

/// A reason to close a table, known not to be blank.
pub fn reason(text: &str) -> BlockReason {
    BlockReason::new(text).expect("fixture reasons are not blank")
}

/// A migrated database of this test's own, with a pool belonging to this test's runtime.
pub async fn store() -> Store {
    database::fresh_store().await
}

/// A payload Telegram stamped at `at`, on a clock taken to agree with this one.
///
/// For tests about something other than the clock. The ones about it say how far apart the two
/// clocks may be.
pub fn signed(at: DateTime<Utc>) -> Signature {
    Signature {
        stamped_at: at,
        clock_skew: TimeDelta::zero(),
    }
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
        contact: None,
    }
}

pub fn default_config() -> BarConfig {
    config_with(default_tables(), "anna_mgr")
}

/// Creates a bar from `config`, panicking if the fixture itself is illegal.
pub async fn bar_with(store: &Store, config: BarConfig) -> (BarId, ValidConfig) {
    let config = ValidConfig::new(config)
        .unwrap_or_else(|errors| panic!("fixture config is illegal: {errors:?}"));
    let bar = store
        .create_bar(&config, morning())
        .await
        .expect("bar is created");
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

/// A proposal that changes nothing, made from the settings as they now stand, ready to be edited.
pub async fn draft_of(store: &Store, bar: BarId) -> Draft {
    let settings = store.settings(bar).await.expect("the settings load");
    let config = &settings.config;
    Draft {
        version: settings.version,
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
            .map(|table| TableDraft {
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
        contact: config.contact.clone().unwrap_or_default(),
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
            replacing: Vec::new(),
        },
        reminder: Some(reminder_wording),
    }
}

/// A guest's `request` saying it replaces exactly `ids`, as the guest's app would have said.
pub fn replacing(mut request: NewBooking, ids: &[pustol_domain::BookingId]) -> NewBooking {
    let Channel::Guest { replacing, .. } = &mut request.channel else {
        panic!("only a guest's booking replaces anything");
    };
    *replacing = ids.to_vec();
    request
}

/// A booking staff took by telephone or at the door.
pub fn staff_booking(bar: BarId, name: &str, minutes: i32, party_size: i32) -> NewBooking {
    NewBooking {
        bar,
        service_day: thursday(),
        start_minutes: minutes,
        party_size,
        channel: Channel::Staff {
            guest_name: GuestName::new(name).expect("fixture names are not blank"),
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
    format!(
        "{}: бронь {} отменена — {reason}",
        config.name, record.guest_name
    )
}

pub fn move_words() -> pustol_db::bookings::MoveWords {
    pustol_db::bookings::MoveWords {
        notice: |config, was, now| {
            format!(
                "{}: бронь {} перенесена на {}",
                config.name,
                was.guest_name,
                now.booking.window.start()
            )
        },
        reminder: reminder_wording,
    }
}
