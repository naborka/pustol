//! What happens to the bookings when the room changes underneath them.

mod common;

use pustol_domain::allocator::TableBlock;
use pustol_domain::reconcile::{Request, reconcile};
use pustol_domain::{BookingStatus, seating_is_sound};

use common::{block, booking, default_config, numbered, thursday, utc};

fn morning() -> chrono::DateTime<chrono::Utc> {
    utc(2026, 7, 30, 6, 0)
}

#[test]
fn a_sound_room_moves_nobody() {
    let config = default_config();
    let tables = config.tables.clone();
    let bookings = vec![
        booking(1, thursday(), 1200, 2, Some(numbered(&tables, 1)), 120),
        booking(2, thursday(), 1200, 6, Some(numbered(&tables, 11)), 120),
    ];
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: &bookings,
        blocks: &[],
        now: morning(),
    });
    assert!(outcome.is_empty());
}

#[test]
fn shrinking_a_table_moves_the_party_that_no_longer_fits() {
    let config = default_config();
    let mut tables = config.tables.clone();
    let six_top = numbered(&tables, 11).clone();
    let seated = booking(1, thursday(), 1200, 6, Some(&six_top), 120);

    for table in &mut tables {
        if table.id == six_top.id {
            table.seats = 4;
        }
    }

    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: std::slice::from_ref(&seated),
        blocks: &[],
        now: morning(),
    });
    assert_eq!(outcome.orphaned, Vec::new());
    assert_eq!(outcome.moved.len(), 1);
    let moved = outcome.moved[0];
    assert_eq!(moved.booking, seated.id);
    assert_eq!(moved.from, Some(six_top.id));
    assert_eq!(moved.to_number, 12, "the next six top by number");
}

#[test]
fn retiring_a_table_moves_the_party_sitting_at_it() {
    let config = default_config();
    let mut tables = config.tables.clone();
    let two_top = numbered(&tables, 1).clone();
    let seated = booking(1, thursday(), 1200, 2, Some(&two_top), 120);
    for table in &mut tables {
        if table.id == two_top.id {
            table.retired = true;
        }
    }
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: std::slice::from_ref(&seated),
        blocks: &[],
        now: morning(),
    });
    assert_eq!(outcome.moved.len(), 1);
    assert_eq!(outcome.moved[0].to_number, 2);
}

#[test]
fn closing_a_table_moves_its_bookings_through_the_same_allocator() {
    let config = default_config();
    let tables = config.tables.clone();
    let two_top = numbered(&tables, 1);
    let seated = booking(1, thursday(), 1200, 2, Some(two_top), 120);
    let blocks = vec![block(thursday(), two_top)];

    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: std::slice::from_ref(&seated),
        blocks: &blocks,
        now: morning(),
    });
    assert_eq!(outcome.moved.len(), 1);
    assert_eq!(outcome.moved[0].to_number, 2);
}

#[test]
fn closing_a_whole_zone_reseats_what_it_can_and_orphans_the_rest() {
    let config = default_config();
    let tables = config.tables.clone();
    // Fill every six-top in the room, then close the terrace, which holds one of them.
    let six_tops: Vec<_> = tables.iter().filter(|table| table.seats == 6).collect();
    assert_eq!(six_tops.len(), 4, "three in the room, one on the terrace");
    let bookings: Vec<_> = six_tops
        .iter()
        .enumerate()
        .map(|(index, table)| {
            booking(
                u128::try_from(index).expect("small"),
                thursday(),
                1200,
                6,
                Some(table),
                120,
            )
        })
        .collect();

    let terrace: Vec<TableBlock> = tables
        .iter()
        .filter(|table| table.zone.as_str() == "Терраса")
        .map(|table| block(thursday(), table))
        .collect();
    let stranded = bookings
        .iter()
        .find(|candidate| {
            terrace
                .iter()
                .any(|blocked| Some(blocked.table_id) == candidate.table_id)
        })
        .expect("one six top is on the terrace");

    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: &bookings,
        blocks: &terrace,
        now: morning(),
    });
    assert_eq!(outcome.moved, Vec::new(), "every other six top is taken");
    assert_eq!(outcome.orphaned, vec![stranded.id]);
}

#[test]
fn an_orphan_is_seated_again_as_soon_as_the_room_can_take_it() {
    // The guest was never told their table went away — their booking still reads "confirmed".
    // Leaving them without a table when one is free would keep a promise the bar can honour
    // in a state the bar cannot serve.
    let config = default_config();
    let tables = config.tables.clone();
    let orphan = booking(1, thursday(), 1200, 6, None, 120);
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: std::slice::from_ref(&orphan),
        blocks: &[],
        now: morning(),
    });
    assert_eq!(outcome.moved.len(), 1);
    assert_eq!(outcome.moved[0].booking, orphan.id);
    assert_eq!(outcome.moved[0].from, None);
    assert_eq!(outcome.moved[0].to_number, 11);
}

#[test]
fn bookings_that_have_already_finished_are_left_exactly_as_they_happened() {
    let config = default_config();
    let mut tables = config.tables.clone();
    let two_top = numbered(&tables, 1).clone();
    let last_night = booking(1, thursday(), 1200, 2, Some(&two_top), 120);
    for table in &mut tables {
        if table.id == two_top.id {
            table.retired = true;
        }
    }
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: std::slice::from_ref(&last_night),
        blocks: &[],
        // Long after the booking ended.
        now: utc(2026, 7, 31, 12, 0),
    });
    assert!(
        outcome.is_empty(),
        "history is not rewritten when the room changes"
    );
}

#[test]
fn cancelled_bookings_are_neither_moved_nor_counted() {
    let config = default_config();
    let mut tables = config.tables.clone();
    let two_top = numbered(&tables, 1).clone();
    let mut cancelled = booking(1, thursday(), 1200, 2, Some(&two_top), 120);
    cancelled.status = BookingStatus::Cancelled;
    for table in &mut tables {
        if table.id == two_top.id {
            table.retired = true;
        }
    }
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: std::slice::from_ref(&cancelled),
        blocks: &[],
        now: morning(),
    });
    assert!(outcome.is_empty());
}

#[test]
fn one_table_takes_two_displaced_parties_whose_evenings_do_not_touch() {
    let config = default_config();
    let mut tables = config.tables.clone();
    tables.retain(|table| table.number == 11 || table.number == 12 || table.number == 15);
    // 18:00-20:00 and 20:00-22:00: back to back, so one table can hold both.
    let early = booking(1, thursday(), 1080, 6, Some(numbered(&tables, 11)), 120);
    let late = booking(2, thursday(), 1200, 6, Some(numbered(&tables, 12)), 120);
    for table in &mut tables {
        if table.number == 11 || table.number == 12 {
            table.retired = true;
        }
    }
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: &[early.clone(), late.clone()],
        blocks: &[],
        now: morning(),
    });
    assert_eq!(outcome.orphaned, Vec::new());
    assert_eq!(outcome.moved.len(), 2);
    assert!(
        outcome.moved.iter().all(|moved| moved.to_number == 15),
        "the surviving table takes both sittings: {:?}",
        outcome.moved
    );
}

#[test]
fn the_earlier_arrival_takes_the_last_remaining_table() {
    let config = default_config();
    let mut tables = config.tables.clone();
    tables.retain(|table| table.number == 11 || table.number == 12 || table.number == 15);
    // The later arrival is deliberately given the *lower* identifier, so an implementation that
    // orders candidates by identifier rather than by arrival time cannot pass this test.
    let late = booking(1, thursday(), 1260, 6, Some(numbered(&tables, 12)), 120);
    let early = booking(2, thursday(), 1200, 6, Some(numbered(&tables, 11)), 120);
    for table in &mut tables {
        if table.number == 11 || table.number == 12 {
            table.retired = true;
        }
    }
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: &[late.clone(), early.clone()],
        blocks: &[],
        now: morning(),
    });
    assert_eq!(outcome.moved.len(), 1);
    assert_eq!(outcome.moved[0].booking, early.id);
    assert_eq!(outcome.moved[0].to_number, 15);
    assert_eq!(outcome.orphaned, vec![late.id]);
}

#[test]
fn the_report_is_ordered_by_arrival_however_the_caller_loaded_the_rows() {
    let config = default_config();
    let mut tables = config.tables.clone();
    tables.retain(|table| table.number == 11 || table.number == 12);
    // Again, identifiers run counter to arrival order.
    let late = booking(1, thursday(), 1260, 6, Some(numbered(&tables, 12)), 120);
    let early = booking(2, thursday(), 1200, 6, Some(numbered(&tables, 11)), 120);
    for table in &mut tables {
        table.seats = 4;
    }
    let outcome = reconcile(&Request {
        tables: &tables,
        bookings: &[late.clone(), early.clone()],
        blocks: &[],
        now: morning(),
    });
    assert_eq!(outcome.moved, Vec::new());
    assert_eq!(outcome.orphaned, vec![early.id, late.id]);
}

#[test]
fn soundness_is_the_one_predicate_reconciliation_acts_on() {
    let config = default_config();
    let tables = config.tables.clone();
    let two_top = numbered(&tables, 1);
    let seated = booking(1, thursday(), 1200, 2, Some(two_top), 120);
    assert!(seating_is_sound(&seated, &tables, &[]));

    let blocks = vec![block(thursday(), two_top)];
    assert!(!seating_is_sound(&seated, &tables, &blocks));
}
