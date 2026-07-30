//! The room: the tables that exist, how many they seat and which zone they belong to.

use std::fmt;

use uuid::Uuid;

/// Stable identity of a table.
///
/// Bookings reference this, never the printed number, so renumbering or retiring a table can
/// never make an existing booking point at a different piece of furniture.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TableId(pub Uuid);

impl fmt::Display for TableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A named part of the room that can be opened or closed as a unit — the terrace when it
/// rains, the back room for a private party.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Zone(String);

impl Zone {
    /// Zones are compared and displayed by their name, so a blank one is not a zone.
    pub fn new(name: impl Into<String>) -> Result<Self, ZoneError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ZoneError::Blank);
        }
        Ok(Self(name))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ZoneError {
    #[error("a zone name cannot be blank")]
    Blank,
}

/// A table as the room currently is.
///
/// `retired` rather than deleted: bookings that already happened at this table must keep
/// resolving to it for the shift history to stay truthful, and the printed number must never
/// be handed to a different table. Retirement removes a table from allocation and from the
/// settings list while leaving both of those intact.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BarTable {
    pub id: TableId,
    /// The number staff and guests say out loud. Unique per bar and never reused.
    pub number: i32,
    pub seats: i32,
    pub zone: Zone,
    pub retired: bool,
}

impl BarTable {
    /// A table can take a party only if it physically seats them.
    #[must_use]
    pub fn seats_party(&self, party_size: i32) -> bool {
        self.seats >= party_size
    }

    /// Whether the table is part of the live room at all.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.retired
    }
}

/// The next printed number for a new table.
///
/// Derived from every table the bar has ever had, retired ones included, so that retiring
/// table 15 and adding another does not produce a second table 15 — staff would read the same
/// number on two different tables in the same shift history.
#[must_use]
pub fn next_table_number(tables: &[BarTable]) -> i32 {
    tables.iter().map(|t| t.number).max().unwrap_or(0) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(number: i32, seats: i32, retired: bool) -> BarTable {
        BarTable {
            id: TableId(Uuid::new_v4()),
            number,
            seats,
            zone: Zone::new("Зал").expect("non blank"),
            retired,
        }
    }

    #[test]
    fn a_zone_name_must_not_be_blank() {
        assert_eq!(Zone::new("   "), Err(ZoneError::Blank));
        assert_eq!(Zone::new("Терраса").unwrap().as_str(), "Терраса");
    }

    #[test]
    fn table_numbers_are_never_reused_after_retirement() {
        let tables = vec![table(1, 2, false), table(2, 4, false), table(3, 6, true)];
        assert_eq!(next_table_number(&tables), 4);
    }

    #[test]
    fn the_first_table_of_a_new_bar_is_number_one() {
        assert_eq!(next_table_number(&[]), 1);
    }

    #[test]
    fn a_table_seats_a_party_no_larger_than_its_seats() {
        let four_top = table(5, 4, false);
        assert!(four_top.seats_party(4));
        assert!(!four_top.seats_party(5));
    }
}
