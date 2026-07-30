//! Reading and writing what the bar has decided.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use pustol_domain::config::{BarConfig, DayHours, StaffMember, ValidConfig, WeekSchedule};
use pustol_domain::draft::Draft;
use pustol_domain::schedule::{BarTable, TableId, Zone, next_table_number};
use pustol_domain::{parties_above_cap, schedule_conflicts};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::ids::BarId;
use crate::{Store, bookings, lock_bar};

/// Names the bookings a refused settings change would have stranded.
fn stranded(
    conflicts: &[pustol_domain::ScheduleConflict],
    loaded: &[crate::records::BookingRecord],
) -> Vec<StrandedBooking> {
    conflicts
        .iter()
        .map(|conflict| {
            let record = loaded
                .iter()
                .find(|record| record.booking.id == conflict.booking());
            StrandedBooking {
                conflict: *conflict,
                guest_name: record.map(|record| record.guest_name.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

/// A booking a proposed change cannot honour, with enough about it to tell staff which one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StrandedBooking {
    pub conflict: pustol_domain::ScheduleConflict,
    pub guest_name: String,
}

/// What a settings save actually did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SavedSettings {
    pub config: ValidConfig,
    /// Bookings the change forced to move, and any it could not place.
    pub reconciliation: crate::bookings::Reseated,
    /// Live bookings for parties above the new cap. Not a refusal — they already have a table and
    /// will be served — but staff are told, so a cap lowered by accident is visible at once.
    pub above_cap: usize,
    /// The number a table added next would be given, so the settings screen can label a row it
    /// has only just created.
    pub next_table_number: i32,
}

impl Store {
    /// The configuration in force at one bar.
    pub async fn config(&self, bar: BarId) -> Result<ValidConfig> {
        let mut connection = self.pool().acquire().await?;
        load_config(&mut connection, bar).await
    }

    /// The number a newly added table would be given.
    pub async fn next_table_number(&self, bar: BarId) -> Result<i32> {
        let mut connection = self.pool().acquire().await?;
        let tables = load_tables(&mut connection, bar).await?;
        Ok(next_table_number(&tables))
    }

    /// Applies everything the settings screen sent, or refuses the lot.
    ///
    /// Refusal comes in three flavours, deliberately distinguishable: the proposal makes no sense
    /// (an unknown table, an unknown timezone), the proposal is illegal (a party cap no table can
    /// seat), or the proposal would strand bookings the bar has already promised. Only the last
    /// needs a human to move bookings first, and only the last is worth interrupting them for.
    ///
    /// Inventory changes are applied and the bookings reconciled rather than refused: the screen
    /// describes the room as it now physically is, and a system that will not record that just
    /// gets worked around.
    pub async fn save_settings(
        &self,
        bar: BarId,
        draft: &Draft,
        now: DateTime<Utc>,
    ) -> Result<SavedSettings> {
        let mut transaction = self.pool().begin().await?;
        lock_bar(&mut transaction, bar).await?;

        let current = load_config(&mut transaction, bar).await?;
        let proposed = draft
            .resolve(&current, Uuid::new_v4)
            .map_err(|error| Error::UnusableProposal(error.to_string()))?;
        let proposed = ValidConfig::new(proposed).map_err(Error::ProposedConfigInvalid)?;

        let unfinished = bookings::load_unfinished(&mut transaction, bar, now).await?;
        let live = crate::records::bookings_of(&unfinished);
        let conflicts = schedule_conflicts(&proposed, &live, now);
        if !conflicts.is_empty() {
            // Named, so the screen can say which bookings and not merely that some exist.
            return Err(Error::WouldStrandBookings(stranded(&conflicts, &unfinished)));
        }
        let above_cap = parties_above_cap(&proposed, &live, now).len();

        write_bar(&mut transaction, bar, &proposed).await?;
        write_week(&mut transaction, bar, &proposed).await?;
        write_tables(&mut transaction, bar, &proposed).await?;
        write_staff(&mut transaction, bar, &proposed).await?;

        // The room may have shrunk. Anything that no longer fits goes through the same allocator
        // that seated it, and anything unseatable becomes an orphan for staff to settle.
        let reconciliation =
            bookings::reconcile_from(&mut transaction, bar, &proposed, now).await?;

        let next_table_number = next_table_number(&proposed.tables);
        transaction.commit().await?;

        Ok(SavedSettings {
            config: proposed,
            reconciliation,
            above_cap,
            next_table_number,
        })
    }
}

/// Loads the configuration in force, refusing to hand back one the domain would not accept.
///
/// A stored row that fails validation means either that somebody wrote around the API or that a
/// limit was tightened without a data migration. Both deserve a loud failure rather than a bar
/// that quietly runs on rules the rest of the system does not believe in.
pub(crate) async fn load_config(
    connection: &mut PgConnection,
    bar: BarId,
) -> Result<ValidConfig> {
    let row = sqlx::query(
        "select name, address, timezone, turn_minutes, slot_step_minutes, max_party,
                horizon_days, remind_hours, grace_minutes, zones, message_templates, cancel_reasons
         from bar where id = $1",
    )
    .bind(bar)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(Error::NotFound { entity: "bar" })?;

    let timezone_name: String = row.try_get("timezone")?;
    let timezone: Tz = timezone_name
        .parse()
        .map_err(|_| Error::UnknownTimezone(timezone_name))?;

    let week = load_week(&mut *connection, bar).await?;
    let tables = load_tables(&mut *connection, bar).await?;
    let staff = load_staff(&mut *connection, bar).await?;

    let zones = row
        .try_get::<Vec<String>, _>("zones")?
        .into_iter()
        .map(Zone::new)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| Error::CorruptRow {
            entity: "bar.zones",
            detail: error.to_string(),
        })?;

    let config = BarConfig {
        name: row.try_get("name")?,
        address: row.try_get("address")?,
        timezone,
        week,
        zones,
        tables,
        turn_minutes: row.try_get("turn_minutes")?,
        slot_step_minutes: row.try_get("slot_step_minutes")?,
        max_party: row.try_get("max_party")?,
        horizon_days: row.try_get("horizon_days")?,
        remind_hours: row.try_get("remind_hours")?,
        grace_minutes: row.try_get("grace_minutes")?,
        message_templates: row.try_get("message_templates")?,
        cancel_reasons: row.try_get("cancel_reasons")?,
        staff,
    };
    ValidConfig::new(config).map_err(Error::StoredConfigInvalid)
}

async fn load_week(connection: &mut PgConnection, bar: BarId) -> Result<WeekSchedule> {
    let rows = sqlx::query(
        "select weekday, open_minutes, close_minutes, closed from bar_hours where bar_id = $1",
    )
    .bind(bar)
    .fetch_all(connection)
    .await?;

    // The schema guarantees seven rows through a deferred constraint trigger, so a missing day
    // here would mean the guarantee was lost — say so rather than silently inventing hours.
    if rows.len() != 7 {
        return Err(Error::CorruptRow {
            entity: "bar_hours",
            detail: format!("{} weekdays stored, not 7", rows.len()),
        });
    }
    let mut days = [DayHours {
        open_minutes: 0,
        close_minutes: 0,
        closed: true,
    }; 7];
    for row in rows {
        let weekday: i16 = row.try_get("weekday")?;
        let index = usize::try_from(weekday).unwrap_or(usize::MAX);
        *days.get_mut(index).ok_or_else(|| Error::CorruptRow {
            entity: "bar_hours.weekday",
            detail: format!("{weekday} is not a weekday"),
        })? = DayHours {
            open_minutes: row.try_get("open_minutes")?,
            close_minutes: row.try_get("close_minutes")?,
            closed: row.try_get("closed")?,
        };
    }
    Ok(WeekSchedule::new(days))
}

/// Every table the bar has ever had, retired ones included: they still own their numbers, and
/// bookings that happened at them still point to them.
pub(crate) async fn load_tables(
    connection: &mut PgConnection,
    bar: BarId,
) -> Result<Vec<BarTable>> {
    let rows = sqlx::query(
        "select id, number, seats, zone, retired_at from bar_table
         where bar_id = $1 order by number",
    )
    .bind(bar)
    .fetch_all(connection)
    .await?;

    rows.into_iter()
        .map(|row| {
            let zone: String = row.try_get("zone")?;
            Ok(BarTable {
                id: TableId(row.try_get("id")?),
                number: row.try_get("number")?,
                seats: row.try_get("seats")?,
                zone: Zone::new(zone).map_err(|error| Error::CorruptRow {
                    entity: "bar_table.zone",
                    detail: error.to_string(),
                })?,
                retired: row
                    .try_get::<Option<DateTime<Utc>>, _>("retired_at")?
                    .is_some(),
            })
        })
        .collect()
}

async fn load_staff(connection: &mut PgConnection, bar: BarId) -> Result<Vec<StaffMember>> {
    let rows = sqlx::query(
        "select username, telegram_user_id from bar_staff where bar_id = $1 order by username_lower",
    )
    .bind(bar)
    .fetch_all(connection)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(StaffMember {
                username: row.try_get("username")?,
                telegram_user_id: row.try_get("telegram_user_id")?,
            })
        })
        .collect()
}

async fn write_bar(
    connection: &mut PgConnection,
    bar: BarId,
    config: &ValidConfig,
) -> Result<()> {
    sqlx::query(
        "update bar set name = $2, address = $3, timezone = $4, turn_minutes = $5,
                slot_step_minutes = $6, max_party = $7, horizon_days = $8, remind_hours = $9,
                grace_minutes = $10, zones = $11, message_templates = $12, cancel_reasons = $13
         where id = $1",
    )
    .bind(bar)
    .bind(&config.name)
    .bind(&config.address)
    .bind(config.timezone.name())
    .bind(config.turn_minutes)
    .bind(config.slot_step_minutes)
    .bind(config.max_party)
    .bind(config.horizon_days)
    .bind(config.remind_hours)
    .bind(config.grace_minutes)
    .bind(
        config
            .zones
            .iter()
            .map(|zone| zone.as_str().to_owned())
            .collect::<Vec<_>>(),
    )
    .bind(&config.message_templates)
    .bind(&config.cancel_reasons)
    .execute(connection)
    .await?;
    Ok(())
}

async fn write_week(
    connection: &mut PgConnection,
    bar: BarId,
    config: &ValidConfig,
) -> Result<()> {
    // Upserted rather than replaced, so the deferred "a week has seven days" trigger is never
    // presented with a bar that momentarily has none.
    let days = config.week.all();
    let weekdays: Vec<i16> = (0..7).collect();
    let open: Vec<i32> = days.iter().map(|hours| hours.open_minutes).collect();
    let close: Vec<i32> = days.iter().map(|hours| hours.close_minutes).collect();
    let closed: Vec<bool> = days.iter().map(|hours| hours.closed).collect();

    sqlx::query(
        "insert into bar_hours (bar_id, weekday, open_minutes, close_minutes, closed)
         select $1, day.weekday, day.open_minutes, day.close_minutes, day.closed
         from unnest($2::smallint[], $3::int[], $4::int[], $5::bool[])
              as day(weekday, open_minutes, close_minutes, closed)
         on conflict (bar_id, weekday) do update set
            open_minutes = excluded.open_minutes,
            close_minutes = excluded.close_minutes,
            closed = excluded.closed",
    )
    .bind(bar)
    .bind(weekdays)
    .bind(open)
    .bind(close)
    .bind(closed)
    .execute(connection)
    .await?;
    Ok(())
}

async fn write_tables(
    connection: &mut PgConnection,
    bar: BarId,
    config: &ValidConfig,
) -> Result<()> {
    let ids: Vec<Uuid> = config.tables.iter().map(|table| table.id.0).collect();
    let numbers: Vec<i32> = config.tables.iter().map(|table| table.number).collect();
    let seats: Vec<i32> = config.tables.iter().map(|table| table.seats).collect();
    let zones: Vec<String> = config
        .tables
        .iter()
        .map(|table| table.zone.as_str().to_owned())
        .collect();
    let retired: Vec<bool> = config.tables.iter().map(|table| table.retired).collect();

    // `coalesce(existing.retired_at, now())` keeps the moment a table was first retired, so the
    // shift history can still say when the room changed.
    sqlx::query(
        "insert into bar_table (id, bar_id, number, seats, zone, retired_at)
         select proposed.id, $1, proposed.number, proposed.seats, proposed.zone,
                case when proposed.retired then coalesce(existing.retired_at, now()) end
         from unnest($2::uuid[], $3::int[], $4::int[], $5::text[], $6::bool[])
              as proposed(id, number, seats, zone, retired)
         left join bar_table existing on existing.id = proposed.id
         on conflict (id) do update set
            seats = excluded.seats,
            zone = excluded.zone,
            retired_at = excluded.retired_at",
    )
    .bind(bar)
    .bind(ids)
    .bind(numbers)
    .bind(seats)
    .bind(zones)
    .bind(retired)
    .execute(connection)
    .await?;
    Ok(())
}

async fn write_staff(
    connection: &mut PgConnection,
    bar: BarId,
    config: &ValidConfig,
) -> Result<()> {
    let usernames: Vec<String> = config
        .staff
        .iter()
        .map(|member| member.username.clone())
        .collect();
    let lowered: Vec<String> = usernames
        .iter()
        .map(|username| username.to_lowercase())
        .collect();
    let bound: Vec<Option<i64>> = config
        .staff
        .iter()
        .map(|member| member.telegram_user_id)
        .collect();

    sqlx::query("delete from bar_staff where bar_id = $1 and username_lower <> all($2::text[])")
        .bind(bar)
        .bind(&lowered)
        .execute(&mut *connection)
        .await?;

    // An existing binding is never overwritten from a proposal: the settings screen sends
    // usernames, and letting it clear a numeric id would downgrade authorisation to something a
    // username squatter could take over.
    sqlx::query(
        "insert into bar_staff (bar_id, username, telegram_user_id, bound_at)
         select $1, proposed.username, proposed.telegram_user_id,
                case when proposed.telegram_user_id is not null then now() end
         from unnest($2::text[], $3::bigint[]) as proposed(username, telegram_user_id)
         on conflict (bar_id, username_lower) do update set
            username = excluded.username,
            telegram_user_id = coalesce(bar_staff.telegram_user_id, excluded.telegram_user_id),
            bound_at = case
                when coalesce(bar_staff.telegram_user_id, excluded.telegram_user_id) is null
                then null
                else coalesce(bar_staff.bound_at, now())
            end",
    )
    .bind(bar)
    .bind(&usernames)
    .bind(&bound)
    .execute(connection)
    .await?;
    Ok(())
}

/// Creates a bar with a complete week in one transaction, which is the only way the schema allows
/// a bar to exist at all.
pub(crate) async fn insert_bar(
    connection: &mut PgConnection,
    config: &ValidConfig,
) -> Result<BarId> {
    let bar: BarId = sqlx::query(
        "insert into bar (name, address, timezone, turn_minutes, slot_step_minutes, max_party,
                          horizon_days, remind_hours, grace_minutes, zones, message_templates,
                          cancel_reasons)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) returning id",
    )
    .bind(&config.name)
    .bind(&config.address)
    .bind(config.timezone.name())
    .bind(config.turn_minutes)
    .bind(config.slot_step_minutes)
    .bind(config.max_party)
    .bind(config.horizon_days)
    .bind(config.remind_hours)
    .bind(config.grace_minutes)
    .bind(
        config
            .zones
            .iter()
            .map(|zone| zone.as_str().to_owned())
            .collect::<Vec<_>>(),
    )
    .bind(&config.message_templates)
    .bind(&config.cancel_reasons)
    .fetch_one(&mut *connection)
    .await
    .map(|row| row.get::<Uuid, _>("id"))
    .map(BarId)?;

    write_week(&mut *connection, bar, config).await?;
    write_tables(&mut *connection, bar, config).await?;
    write_staff(&mut *connection, bar, config).await?;
    Ok(bar)
}

impl Store {
    /// Brings a bar into being, hours, room and roster together.
    pub async fn create_bar(&self, config: &ValidConfig) -> Result<BarId> {
        let mut transaction = self.pool().begin().await?;
        let bar = insert_bar(&mut transaction, config).await?;
        transaction.commit().await?;
        Ok(bar)
    }

    /// The only bar this deployment serves, for a single-tenant install.
    pub async fn sole_bar(&self) -> Result<BarId> {
        sqlx::query("select id from bar order by created_at limit 1")
            .fetch_optional(self.pool())
            .await?
            .map(|row| row.get::<Uuid, _>("id"))
            .map(BarId)
            .ok_or(Error::NotFound { entity: "bar" })
    }
}
