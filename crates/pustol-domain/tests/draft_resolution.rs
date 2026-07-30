//! Turning what the settings screen sends into a configuration with settled identities.

mod common;

use pustol_domain::config::StaffMember;
use pustol_domain::draft::{DayHoursDraft, Draft, DraftError, StaffDraft, TableDraft};
use pustol_domain::{BarConfig, TableId, Zone};
use uuid::Uuid;

use common::{default_config, id};

/// Identities for added tables, drawn in order so the assertions can name them.
fn minter(from: u128) -> impl FnMut() -> Uuid {
    let mut next = from;
    move || {
        next += 1;
        id(next)
    }
}

fn draft_of(config: &BarConfig) -> Draft {
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

#[test]
fn a_proposal_that_changes_nothing_resolves_to_what_is_already_in_force() {
    let current = default_config();
    let resolved = draft_of(&current)
        .resolve(&current, minter(1000))
        .expect("resolvable");
    assert_eq!(resolved, current);
}

#[test]
fn an_added_table_is_given_the_next_number_that_has_never_been_used() {
    let mut current = default_config();
    // Table 15 was retired at some point; the room shows fourteen tables.
    current
        .tables
        .iter_mut()
        .find(|table| table.number == 15)
        .expect("fixture has fifteen tables")
        .retired = true;

    let mut draft = draft_of(&current);
    draft.tables.push(TableDraft::New {
        seats: 4,
        zone: "Зал".to_owned(),
    });

    let resolved = draft.resolve(&current, minter(1000)).expect("resolvable");
    let added = resolved
        .tables
        .iter()
        .find(|table| table.id == TableId(id(1001)))
        .expect("the new table got the first minted identity");
    assert_eq!(
        added.number, 16,
        "the retired fifteenth still owns its number"
    );
    assert_eq!(added.seats, 4);
}

#[test]
fn several_added_tables_take_consecutive_numbers() {
    let current = default_config();
    let mut draft = draft_of(&current);
    for _ in 0..3 {
        draft.tables.push(TableDraft::New {
            seats: 2,
            zone: "Бар".to_owned(),
        });
    }
    let resolved = draft.resolve(&current, minter(2000)).expect("resolvable");
    let added: Vec<i32> = resolved
        .tables
        .iter()
        .filter(|table| table.number > 15)
        .map(|table| table.number)
        .collect();
    assert_eq!(added, vec![16, 17, 18]);
}

#[test]
fn a_table_the_proposal_leaves_out_is_retired_with_its_number_and_history_intact() {
    let current = default_config();
    let removed = current
        .tables
        .iter()
        .find(|table| table.number == 7)
        .expect("fixture table")
        .clone();

    let mut draft = draft_of(&current);
    draft
        .tables
        .retain(|table| !matches!(table, TableDraft::Existing { id, .. } if *id == removed.id.0));

    let resolved = draft.resolve(&current, minter(1000)).expect("resolvable");
    let still_there = resolved
        .tables
        .iter()
        .find(|table| table.id == removed.id)
        .expect("a removed table is retired, not deleted");
    assert!(still_there.retired);
    assert_eq!(still_there.number, 7);
    assert_eq!(still_there.seats, removed.seats);
    assert_eq!(
        resolved.active_tables().count(),
        14,
        "the live room lost one table"
    );
}

#[test]
fn a_table_already_retired_stays_retired() {
    let mut current = default_config();
    current.tables[0].retired = true;
    let resolved = draft_of(&current)
        .resolve(&current, minter(1000))
        .expect("resolvable");
    assert!(resolved.tables.iter().any(|table| table.retired));
}

#[test]
fn resizing_and_moving_a_table_keeps_its_identity_and_number() {
    let current = default_config();
    let mut draft = draft_of(&current);
    let target = current.tables[0].id.0;
    for table in &mut draft.tables {
        if let TableDraft::Existing { id, seats, zone } = table
            && *id == target
        {
            *seats = 6;
            *zone = "Терраса".to_owned();
        }
    }
    let resolved = draft.resolve(&current, minter(1000)).expect("resolvable");
    let moved = resolved
        .tables
        .iter()
        .find(|table| table.id == TableId(target))
        .expect("still present");
    assert_eq!(moved.seats, 6);
    assert_eq!(moved.zone, Zone::new("Терраса").unwrap());
    assert_eq!(moved.number, current.tables[0].number);
}

#[test]
fn a_proposal_naming_a_table_this_bar_does_not_have_is_refused() {
    let current = default_config();
    let mut draft = draft_of(&current);
    let stranger = id(9999);
    draft.tables.push(TableDraft::Existing {
        id: stranger,
        seats: 4,
        zone: "Зал".to_owned(),
    });
    assert_eq!(
        draft.resolve(&current, minter(1000)),
        Err(DraftError::UnknownTable { id: stranger })
    );
}

#[test]
fn a_proposal_naming_the_same_table_twice_is_refused() {
    let current = default_config();
    let mut draft = draft_of(&current);
    let repeated = current.tables[0].id.0;
    draft.tables.push(TableDraft::Existing {
        id: repeated,
        seats: 2,
        zone: "Бар".to_owned(),
    });
    assert_eq!(
        draft.resolve(&current, minter(1000)),
        Err(DraftError::RepeatedTable { id: repeated })
    );
}

#[test]
fn a_timezone_the_system_cannot_compute_in_is_refused() {
    let current = default_config();
    let mut draft = draft_of(&current);
    draft.timezone = "Europe/Belgrad".to_owned();
    assert_eq!(
        draft.resolve(&current, minter(1000)),
        Err(DraftError::UnknownTimezone {
            name: "Europe/Belgrad".to_owned()
        })
    );
}

#[test]
fn an_existing_binding_survives_a_round_trip_through_the_settings_screen() {
    // The screen sends usernames only. Losing the numeric id would quietly downgrade
    // authorisation to something a username squatter could take over.
    let current = default_config();
    let bound = current
        .staff
        .iter()
        .find(|member| member.telegram_user_id.is_some())
        .expect("fixture has a bound member")
        .clone();

    let resolved = draft_of(&current)
        .resolve(&current, minter(1000))
        .expect("resolvable");
    assert_eq!(
        resolved
            .staff
            .iter()
            .find(|member| member.username == bound.username)
            .and_then(|member| member.telegram_user_id),
        bound.telegram_user_id
    );
}

#[test]
fn a_newly_invited_member_starts_unbound() {
    let current = default_config();
    let mut draft = draft_of(&current);
    draft.staff.push(StaffDraft {
        username: "marko_bg".to_owned(),
    });
    let resolved = draft.resolve(&current, minter(1000)).expect("resolvable");
    assert!(resolved.staff.contains(&StaffMember {
        username: "marko_bg".to_owned(),
        telegram_user_id: None,
    }));
}

#[test]
fn a_member_reinvited_under_a_different_case_keeps_their_binding() {
    let current = default_config();
    let bound = current
        .staff
        .iter()
        .find(|member| member.telegram_user_id.is_some())
        .expect("fixture has a bound member")
        .clone();
    let mut draft = draft_of(&current);
    draft.staff[0].username = bound.username.to_uppercase();

    let resolved = draft.resolve(&current, minter(1000)).expect("resolvable");
    assert_eq!(resolved.staff[0].telegram_user_id, bound.telegram_user_id);
}

#[test]
fn surrounding_whitespace_is_stripped_rather_than_stored() {
    let current = default_config();
    let mut draft = draft_of(&current);
    draft.name = "  Бар «Подвал»  ".to_owned();
    draft.message_templates = vec!["  Ваш стол готов  ".to_owned()];
    let resolved = draft.resolve(&current, minter(1000)).expect("resolvable");
    assert_eq!(resolved.name, "Бар «Подвал»");
    assert_eq!(resolved.message_templates, vec!["Ваш стол готов"]);
}

#[test]
fn a_blank_zone_name_is_refused_before_anything_else_is_considered() {
    let current = default_config();
    let mut draft = draft_of(&current);
    draft.zones.push("   ".to_owned());
    assert!(matches!(
        draft.resolve(&current, minter(1000)),
        Err(DraftError::Zone(_))
    ));
}
