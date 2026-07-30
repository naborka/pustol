//! Brings a bar into being.
//!
//! The schema will not let a bar exist without a complete week, a room and at least one admin, so
//! there is no "empty" state to start the app in and no migration that could leave one. This is the
//! one command that creates all of it, in one transaction.
//!
//!     TELEGRAM_ADMIN_USERNAME=anna_mgr cargo run -p pustol-api --bin seed

use anyhow::{Context, Result};
use pustol_db::Store;
use pustol_domain::config::{BarConfig, DayHours, StaffMember, ValidConfig, WeekSchedule};
use pustol_domain::schedule::{BarTable, TableId, Zone};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let admin = std::env::var("TELEGRAM_ADMIN_USERNAME")
        .context("TELEGRAM_ADMIN_USERNAME must be set: without one, nobody can open the admin side")?;

    let store = Store::connect(&database_url, 4).await?;
    store.migrate().await?;

    if let Ok(existing) = store.sole_bar().await {
        println!("a bar already exists: {existing}");
        return Ok(());
    }

    let config = ValidConfig::new(default_bar(&admin))
        .map_err(|errors| anyhow::anyhow!("the seed configuration is not legal: {errors:?}"))?;
    let bar = store.create_bar(&config).await?;
    println!("created bar {bar} with @{admin} as its first admin");
    Ok(())
}

/// The room the prototype describes: fifteen tables across a bar, a room and a terrace.
fn default_bar(admin: &str) -> BarConfig {
    let mut tables = Vec::new();
    let mut add = |number: i32, seats: i32, zone: &str| {
        tables.push(BarTable {
            id: TableId(Uuid::new_v4()),
            number,
            seats,
            zone: Zone::new(zone).expect("zone names here are not blank"),
            retired: false,
        });
    };
    for number in 1..=4 {
        add(number, 2, "Бар");
    }
    for number in 5..=10 {
        add(number, 4, "Зал");
    }
    for number in 11..=13 {
        add(number, 6, "Зал");
    }
    add(14, 4, "Терраса");
    add(15, 6, "Терраса");

    BarConfig {
        name: "Бар «Подвал»".to_owned(),
        address: "Дечанска 12, Белград".to_owned(),
        timezone: chrono_tz::Europe::Belgrade,
        week: WeekSchedule::uniform(DayHours {
            open_minutes: 600,
            close_minutes: 1560,
            closed: false,
        }),
        zones: ["Бар", "Зал", "Терраса"]
            .into_iter()
            .map(|name| Zone::new(name).expect("not blank"))
            .collect(),
        tables,
        turn_minutes: 120,
        slot_step_minutes: 30,
        max_party: 6,
        horizon_days: 4,
        remind_hours: 3,
        grace_minutes: 15,
        message_templates: vec![
            "Ваш стол готов, ждём вас!".to_owned(),
            "Опаздываете? Держим стол ещё 15 минут.".to_owned(),
            "Можем предложить другое время — напишите, какое удобно.".to_owned(),
            "Уточните, пожалуйста, сколько вас будет.".to_owned(),
        ],
        cancel_reasons: vec![
            "Технические проблемы в баре".to_owned(),
            "Частное мероприятие".to_owned(),
            "Нет свободных столов на это время".to_owned(),
            "По просьбе гостя".to_owned(),
        ],
        staff: vec![StaffMember {
            username: admin.trim().trim_start_matches('@').to_owned(),
            telegram_user_id: None,
        }],
    }
}
