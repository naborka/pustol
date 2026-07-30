//! A proposed change to the bar's configuration, before identities have been settled.
//!
//! The settings screen edits a list of tables in which some rows exist and some have only just
//! been tapped into being. Letting the client mint identities for the new ones would let it name
//! a table that already exists somewhere else, so a proposal says only *which existing table* it
//! refers to and leaves the rest to be resolved here.
//!
//! Resolution is also where "removed" is turned into "retired". A table the proposal does not
//! mention keeps its number, its seats and its history, and simply stops being part of the live
//! room — because bookings that already happened at it must keep resolving to it, and its printed
//! number must never be handed to a different table.

use chrono_tz::Tz;
use serde::Deserialize;
use uuid::Uuid;

use crate::config::{BarConfig, DayHours, StaffMember, WeekSchedule};
use crate::schedule::{BarTable, TableId, Zone, ZoneError, next_table_number};

/// A table in a proposal: one that exists, or one that does not yet.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableDraft {
    /// A table already in the room, possibly resized or moved to another zone.
    Existing {
        id: Uuid,
        seats: i32,
        zone: String,
    },
    /// A table the proposal is adding. Its identity and printed number are assigned here.
    New { seats: i32, zone: String },
}

/// One weekday's proposed hours.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct DayHoursDraft {
    pub open_minutes: i32,
    pub close_minutes: i32,
    pub closed: bool,
}

/// Somebody the proposal wants on the admin list.
#[derive(Clone, Debug, Deserialize)]
pub struct StaffDraft {
    pub username: String,
}

/// The whole settings screen as it stands when Save is pressed.
///
/// `Deserialize` only. Nothing here is ever sent back out: responses are built from explicit
/// projections, so a field added to this struct cannot accidentally appear in a guest's payload.
#[derive(Clone, Debug, Deserialize)]
pub struct Draft {
    pub name: String,
    pub address: String,
    pub timezone: String,
    /// Indexed from Sunday, matching `chrono`'s day numbering.
    pub week: [DayHoursDraft; 7],
    pub zones: Vec<String>,
    pub tables: Vec<TableDraft>,
    pub turn_minutes: i32,
    pub slot_step_minutes: i32,
    pub max_party: i32,
    pub horizon_days: i32,
    pub remind_hours: i32,
    pub grace_minutes: i32,
    pub message_templates: Vec<String>,
    pub cancel_reasons: Vec<String>,
    pub staff: Vec<StaffDraft>,
}

/// A proposal that cannot even be understood, as distinct from one that is understood and
/// illegal — the latter is [`crate::config::ConfigError`].
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum DraftError {
    #[error("the proposal refers to table {id}, which this bar does not have")]
    UnknownTable { id: Uuid },
    #[error("the proposal refers to table {id} twice")]
    RepeatedTable { id: Uuid },
    #[error("{name} is not a timezone this system can compute in")]
    UnknownTimezone { name: String },
    #[error(transparent)]
    Zone(#[from] ZoneError),
}

impl Draft {
    /// Settles the proposal against the room as it is now.
    ///
    /// `new_id` supplies identities for added tables. It is injected rather than called directly
    /// so that nothing in this crate needs a source of randomness, which is what lets the
    /// resolution be asserted exactly in a test.
    ///
    /// The result is a [`BarConfig`] — a proposal, not yet in force. Whether it is *legal* is a
    /// separate question, asked by [`crate::config::ValidConfig::new`], so that the two failure
    /// modes stay distinguishable to whoever has to fix them.
    pub fn resolve(
        &self,
        current: &BarConfig,
        mut new_id: impl FnMut() -> Uuid,
    ) -> Result<BarConfig, DraftError> {
        let timezone: Tz = self
            .timezone
            .parse()
            .map_err(|_| DraftError::UnknownTimezone {
                name: self.timezone.clone(),
            })?;

        let tables = self.resolve_tables(current, &mut new_id)?;
        let zones = self
            .zones
            .iter()
            .map(|name| Zone::new(name.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(BarConfig {
            name: self.name.trim().to_owned(),
            address: self.address.trim().to_owned(),
            timezone,
            week: self.resolve_week(),
            zones,
            tables,
            turn_minutes: self.turn_minutes,
            slot_step_minutes: self.slot_step_minutes,
            max_party: self.max_party,
            horizon_days: self.horizon_days,
            remind_hours: self.remind_hours,
            grace_minutes: self.grace_minutes,
            message_templates: self
                .message_templates
                .iter()
                .map(|text| text.trim().to_owned())
                .collect(),
            cancel_reasons: self
                .cancel_reasons
                .iter()
                .map(|text| text.trim().to_owned())
                .collect(),
            staff: self.resolve_staff(current),
        })
    }

    /// Settles which tables the room has, and which have left it.
    fn resolve_tables(
        &self,
        current: &BarConfig,
        new_id: &mut impl FnMut() -> Uuid,
    ) -> Result<Vec<BarTable>, DraftError> {
        let mut tables = Vec::with_capacity(self.tables.len());
        let mut mentioned = Vec::new();
        // Numbers are drawn against every table the bar has ever had, the retired ones included, so
        // a number is never reused. Carried as a running value rather than recomputed per addition,
        // which would rebuild and clone the whole room once for every table added.
        let mut next_number = next_table_number(&current.tables);
        for proposed in &self.tables {
            match proposed {
                TableDraft::Existing { id, seats, zone } => {
                    if mentioned.contains(id) {
                        return Err(DraftError::RepeatedTable { id: *id });
                    }
                    let existing = current
                        .tables
                        .iter()
                        .find(|table| table.id == TableId(*id))
                        .ok_or(DraftError::UnknownTable { id: *id })?;
                    mentioned.push(*id);
                    tables.push(BarTable {
                        seats: *seats,
                        zone: Zone::new(zone.clone())?,
                        retired: false,
                        ..existing.clone()
                    });
                }
                TableDraft::New { seats, zone } => {
                    tables.push(BarTable {
                        id: TableId(new_id()),
                        number: next_number,
                        seats: *seats,
                        zone: Zone::new(zone.clone())?,
                        retired: false,
                    });
                    next_number += 1;
                }
            }
        }

        // Whatever the proposal leaves out is retired, not deleted: bookings that already
        // happened at it must keep resolving to it, and its number must never be reissued.
        for existing in &current.tables {
            if !mentioned.contains(&existing.id.0) {
                tables.push(BarTable {
                    retired: true,
                    ..existing.clone()
                });
            }
        }
        tables.sort_unstable_by_key(|table| table.number);
        Ok(tables)
    }

    /// Carries every binding already made across to the new roster.
    ///
    /// The settings screen sends usernames only. Letting a proposal clear a numeric id would
    /// quietly downgrade authorisation back to something a username squatter could take over.
    fn resolve_staff(&self, current: &BarConfig) -> Vec<StaffMember> {
        self.staff
            .iter()
            .map(|proposed| StaffMember {
                telegram_user_id: current
                    .staff
                    .iter()
                    .find(|member| member.username.eq_ignore_ascii_case(&proposed.username))
                    .and_then(|member| member.telegram_user_id),
                username: proposed.username.clone(),
            })
            .collect()
    }

    fn resolve_week(&self) -> WeekSchedule {
        WeekSchedule::new(std::array::from_fn(|index| DayHours {
            open_minutes: self.week[index].open_minutes,
            close_minutes: self.week[index].close_minutes,
            closed: self.week[index].closed,
        }))
    }
}
