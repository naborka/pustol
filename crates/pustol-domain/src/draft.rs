//! A proposed change to the bar's configuration, before identities have been settled.
//!
//! The settings screen edits a list of tables in which some rows exist and some have only just
//! been tapped into being. The app names every row, the new ones included, so a save that is sent
//! twice — its answer lost on the way back — names the same tables both times and adds nothing the
//! second time. What an identity means is settled here: one of this bar's tables, live or retired,
//! or a table the room does not have yet. An identity that belongs to another bar is refused where
//! the rooms of every bar can be seen, in storage.
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

/// A table in a proposal, named by the identity the app gave it.
///
/// One shape for a table the bar has and one it is adding, because the app cannot know which a row
/// is by the time a save arrives: the first attempt at a save may already have added it.
///
/// The previous app's shapes are read too, because an app opened before an upgrade keeps sending
/// them: `kind: "existing"` beside an identity means the identity alone, and `kind: "new"` with no
/// identity is a table the server names. That app never named the tables it added, so its save sent
/// twice adds two, as it always did.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "TableShape")]
pub struct TableDraft {
    pub id: Uuid,
    pub seats: i32,
    pub zone: String,
}

/// A table in any shape an app sends one in, before it is settled into a [`TableDraft`].
#[derive(Deserialize)]
struct TableShape {
    #[serde(default)]
    kind: Option<TableKind>,
    #[serde(default)]
    id: Option<Uuid>,
    seats: i32,
    zone: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TableKind {
    Existing,
    New,
}

impl TryFrom<TableShape> for TableDraft {
    type Error = &'static str;

    fn try_from(shape: TableShape) -> Result<Self, Self::Error> {
        let id = match (shape.kind, shape.id) {
            (None | Some(TableKind::Existing), Some(id)) => id,
            (Some(TableKind::New), None) => Uuid::new_v4(),
            (None | Some(TableKind::Existing), None) => {
                return Err("a table needs an id, or to be marked new");
            }
            (Some(TableKind::New), Some(_)) => return Err("a table marked new carries no id"),
        };
        Ok(Self {
            id,
            seats: shape.seats,
            zone: shape.zone,
        })
    }
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
    /// The version of the settings this proposal was made from. Saving it over any other version
    /// would quietly put back whatever changed in between, so that is refused.
    pub version: i64,
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
    /// Empty when the bar gives guests no contact.
    #[serde(default)]
    pub contact: String,
}

/// A proposal that cannot even be understood, as distinct from one that is understood and
/// illegal — the latter is [`crate::config::ConfigError`].
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum DraftError {
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
    /// The result is a [`BarConfig`] — a proposal, not yet in force. Whether it is *legal* is a
    /// separate question, asked by [`crate::config::ValidConfig::new`], so that the two failure
    /// modes stay distinguishable to whoever has to fix them.
    pub fn resolve(&self, current: &BarConfig) -> Result<BarConfig, DraftError> {
        let timezone: Tz = self
            .timezone
            .parse()
            .map_err(|_| DraftError::UnknownTimezone {
                name: self.timezone.clone(),
            })?;

        let tables = self.resolve_tables(current)?;
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
            contact: Some(self.contact.trim())
                .filter(|contact| !contact.is_empty())
                .map(str::to_owned),
        })
    }

    /// Settles which tables the room has, and which have left it.
    ///
    /// A table the bar has keeps its number, and one it had retired comes back under it. Any other
    /// identity is a table being added, under exactly that identity.
    fn resolve_tables(&self, current: &BarConfig) -> Result<Vec<BarTable>, DraftError> {
        let mut tables = Vec::with_capacity(self.tables.len());
        let mut mentioned = Vec::new();
        // Numbers are drawn against every table the bar has ever had, the retired ones included, so
        // a number is never reused. Carried as a running value rather than recomputed per addition,
        // which would rebuild and clone the whole room once for every table added.
        let mut next_number = next_table_number(&current.tables);
        for proposed in &self.tables {
            if mentioned.contains(&proposed.id) {
                return Err(DraftError::RepeatedTable { id: proposed.id });
            }
            mentioned.push(proposed.id);
            let number = if let Some(existing) = current
                .tables
                .iter()
                .find(|table| table.id == TableId(proposed.id))
            {
                existing.number
            } else {
                let added = next_number;
                next_number += 1;
                added
            };
            tables.push(BarTable {
                id: TableId(proposed.id),
                number,
                seats: proposed.seats,
                zone: Zone::new(proposed.zone.clone())?,
                retired: false,
            });
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
