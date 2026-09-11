//! Table allocation: the one place that decides which table a party gets.
//!
//! The architectural commitment of this system is that a booking is only ever recorded against
//! a concrete table. Nothing is ever accepted against an aggregate seat count, so the state
//! "confirmed but unseatable" has no representation — it cannot be reached by any code path,
//! rather than being prevented by a check somebody might forget to call.
//!
//! Every question about availability is answered by this function: the guest's time picker, the
//! staff's manual booking form, the reassignment that follows closing a table and the
//! reconciliation that follows changing the room. One implementation means the picker can never
//! offer a slot the booking endpoint would then refuse.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::config::ValidConfig;
use crate::schedule::{BarTable, TableId};
use crate::service_day::{Interval, ServiceDay};

/// Stable identity of a booking.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct BookingId(pub Uuid);

/// Where a booking stands in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookingStatus {
    /// Taken and expected.
    Confirmed,
    /// The party is at the table.
    Arrived,
    /// The party never came. The booking stays on the shift as a record of the evening, and the
    /// table goes back into the pool at [`Booking::released_at`] — which staff set to the end of
    /// the grace period, not to the moment they pressed the button, so a party that is merely
    /// late does not lose its table to somebody standing at the door.
    NoShow,
    /// The party came, sat, and left before their window was up. The table is free again from
    /// [`Booking::released_at`]; everything before it still belongs to them and still counts as
    /// a served party on the shift's own receipt.
    Left,
    /// Released. Holds nothing, and is not part of the evening any more.
    Cancelled,
}

impl BookingStatus {
    /// Whether this booking is still part of the shift at all.
    ///
    /// Cancelled is the only status that is not: it holds nothing, it is not counted, and it is
    /// not shown. `no_show` and `left` are very much part of the evening — they are how the shift
    /// says what happened — and what they release is a *range*, which is [`Booking::occupancy`].
    #[must_use]
    pub const fn is_live(self) -> bool {
        !matches!(self, Self::Cancelled)
    }
}

/// The allocation-relevant facts about a booking.
///
/// Guest names, usernames and contact details are deliberately absent: allocation must not be
/// able to depend on who the guest is, and a type that cannot express it cannot drift into it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Booking {
    pub id: BookingId,
    /// `None` for an orphan: a live booking with no table, created when the room changed under
    /// it. An orphan holds nothing and blocks nobody; it is a debt the staff must settle.
    pub table_id: Option<TableId>,
    pub service_day: ServiceDay,
    /// The window promised to the guest. Fixed when the booking was taken, and never shortened:
    /// what the bar agreed to is not rewritten by what happened on the night.
    pub window: Interval,
    /// The moment the table went back into the pool, inside `window`. Set when the party left
    /// early or turned out not to be coming; absent for every booking still running its course.
    pub released_at: Option<DateTime<Utc>>,
    pub party_size: i32,
    pub status: BookingStatus,
}

impl Booking {
    /// The range this booking actually holds its table for, or `None` when it holds nothing.
    ///
    /// **The one occupancy rule.** Free tables now, the guest's arrival times, the walk-in offer,
    /// the width of a block on the timeline and the shift's own occupancy figure are all answers
    /// to this one question, so none of them can report a table as busy while another reports it
    /// as free. A booking released at the very minute it began holds an empty range, which is the
    /// honest answer and needs no special case anywhere.
    #[must_use]
    pub fn occupancy(&self) -> Option<Interval> {
        if !self.status.is_live() {
            return None;
        }
        let end = self.released_at.unwrap_or_else(|| self.window.end());
        Interval::new(self.window.start(), end).ok()
    }

    /// Whether this booking holds `table_id` at any moment of `window`.
    #[must_use]
    pub fn holds(&self, table_id: TableId, window: Interval) -> bool {
        self.table_id == Some(table_id)
            && self
                .occupancy()
                .is_some_and(|held| held.overlaps(window))
    }
}

/// A table taken out of service for one shift.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TableBlock {
    pub table_id: TableId,
    pub service_day: ServiceDay,
}

/// The table allocation chose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Assignment {
    pub table_id: TableId,
    /// Carried alongside the id so callers can name the table to staff without another lookup.
    pub number: i32,
}

/// Everything allocation is allowed to look at.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub party_size: i32,
    /// The absolute window the party would occupy. Overlap is only ever judged here, never on
    /// wall-clock minutes, so a clock change cannot make two real overlaps look adjacent.
    pub window: Interval,
    /// The shift being booked — the scope a table block applies to.
    pub service_day: ServiceDay,
    pub tables: &'a [BarTable],
    /// Bookings that could conflict. Callers must include neighbouring shifts, because a
    /// window can outlast midnight; this function does not filter by service day.
    pub bookings: &'a [Booking],
    pub blocks: &'a [TableBlock],
    /// A booking being moved, which must not be treated as blocking its own new place.
    pub ignoring: Option<BookingId>,
}

/// Picks the table for a party, or `None` when the room cannot take them.
///
/// Smallest table that fits, ties broken by printed number. Best-fit is what keeps the large
/// tables available for the large parties that have nowhere else to go; seating a couple at a
/// six-top because it happened to be first in the list is how a Friday runs out of six-tops.
/// The tie-break makes the choice deterministic, which is what lets the same inputs be
/// replayed in a test and in a support conversation.
#[must_use]
pub fn assign(request: &Request<'_>) -> Option<Assignment> {
    let mut candidates: Vec<&BarTable> = request
        .tables
        .iter()
        .filter(|table| table.is_active())
        .filter(|table| table.seats_party(request.party_size))
        .filter(|table| !is_blocked(table.id, request.service_day, request.blocks))
        .collect();
    candidates.sort_unstable_by_key(|table| (table.seats, table.number));

    candidates
        .into_iter()
        .find(|table| free_during(table.id, request))
        .map(|table| Assignment {
            table_id: table.id,
            number: table.number,
        })
}

fn is_blocked(table_id: TableId, service_day: ServiceDay, blocks: &[TableBlock]) -> bool {
    blocks
        .iter()
        .any(|block| block.table_id == table_id && block.service_day == service_day)
}

fn free_during(table_id: TableId, request: &Request<'_>) -> bool {
    !request.bookings.iter().any(|booking| {
        Some(booking.id) != request.ignoring && booking.holds(table_id, request.window)
    })
}

/// The largest party the room could seat right now, or `None` when nothing fits.
///
/// The second line of the shift's pulse, and the same run the walk-in sheet makes. Counting free
/// tables does not answer the question a bartender is actually being asked at the door — four
/// people fit or they do not — and two separate implementations of "who fits" would eventually
/// offer a table the walk-in endpoint then refused.
///
/// Searched downwards from the bar's own cap because a table that seats a party of `n` seats every
/// smaller party too: the first size that fits is therefore the largest one that does.
#[must_use]
pub fn largest_party_seatable(
    config: &ValidConfig,
    service_day: ServiceDay,
    window: Interval,
    bookings: &[Booking],
    blocks: &[TableBlock],
) -> Option<i32> {
    (1..=config.max_party).rev().find(|party_size| {
        assign(&Request {
            party_size: *party_size,
            window,
            service_day,
            tables: &config.tables,
            bookings,
            blocks,
            ignoring: None,
        })
        .is_some()
    })
}

/// Whether a table can hold a booking it is already assigned to.
///
/// The single predicate for "this seating is still sound", used by reconciliation after the
/// room or the schedule changes. Expressing it once means closing a table and shrinking a table
/// cannot diverge into two slightly different notions of unsound.
#[must_use]
pub fn seating_is_sound(booking: &Booking, tables: &[BarTable], blocks: &[TableBlock]) -> bool {
    let Some(table_id) = booking.table_id else {
        return false;
    };
    tables
        .iter()
        .find(|table| table.id == table_id)
        .is_some_and(|table| {
            table.is_active()
                && table.seats_party(booking.party_size)
                && !is_blocked(table_id, booking.service_day, blocks)
        })
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};

    use super::*;
    use crate::config::{BarConfig, DayHours, StaffMember, WeekSchedule};
    use crate::schedule::Zone;

    /// Identifiers derive from the printed number so that no assertion here can pass or fail by
    /// luck: a tie-break that silently fell back to identifier order would be reproducible.
    fn table(number: i32, seats: i32) -> BarTable {
        BarTable {
            id: TableId(Uuid::from_u128(u128::try_from(number).expect("positive"))),
            number,
            seats,
            zone: Zone::new("Зал").expect("non blank"),
            retired: false,
        }
    }

    fn retired(number: i32, seats: i32) -> BarTable {
        BarTable {
            retired: true,
            ..table(number, seats)
        }
    }

    fn day() -> ServiceDay {
        ServiceDay::new(NaiveDate::from_ymd_opt(2026, 7, 30).expect("valid date"))
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 30, hour, minute, 0)
            .single()
            .expect("valid instant")
    }

    fn window(from: (u32, u32), to: (u32, u32)) -> Interval {
        Interval::new(at(from.0, from.1), at(to.0, to.1)).expect("non empty")
    }

    fn booking(table: Option<&BarTable>, window: Interval, party_size: i32) -> Booking {
        Booking {
            id: BookingId(Uuid::new_v4()),
            table_id: table.map(|t| t.id),
            service_day: day(),
            window,
            released_at: None,
            party_size,
            status: BookingStatus::Confirmed,
        }
    }

    struct Room {
        tables: Vec<BarTable>,
        bookings: Vec<Booking>,
        blocks: Vec<TableBlock>,
    }

    impl Room {
        fn new(tables: Vec<BarTable>) -> Self {
            Self {
                tables,
                bookings: Vec::new(),
                blocks: Vec::new(),
            }
        }

        fn request(&self, party_size: i32, window: Interval) -> Request<'_> {
            Request {
                party_size,
                window,
                service_day: day(),
                tables: &self.tables,
                bookings: &self.bookings,
                blocks: &self.blocks,
                ignoring: None,
            }
        }
    }

    #[test]
    fn seats_a_party_at_the_smallest_table_that_fits() {
        let room = Room::new(vec![table(1, 2), table(2, 4), table(3, 6)]);
        let chosen = assign(&room.request(2, window((18, 0), (20, 0)))).expect("room is empty");
        assert_eq!(chosen.number, 1);

        let chosen = assign(&room.request(3, window((18, 0), (20, 0)))).expect("a four top fits");
        assert_eq!(chosen.number, 2);
    }

    #[test]
    fn keeps_the_large_tables_for_the_large_parties() {
        // The six-top is listed first and would win under any first-fit rule. Giving it to a
        // couple is how a Friday runs out of tables for the parties that have nowhere else to sit.
        let room = Room::new(vec![table(1, 6), table(2, 2)]);
        let chosen = assign(&room.request(2, window((18, 0), (20, 0)))).expect("both free");
        assert_eq!(chosen.number, 2);
        assert_eq!(chosen.table_id, room.tables[1].id);
    }

    #[test]
    fn breaks_ties_by_printed_number_so_the_choice_is_reproducible() {
        // Deliberately out of order: the allocator must not depend on input order.
        let room = Room::new(vec![table(7, 4), table(3, 4), table(5, 4)]);
        let chosen = assign(&room.request(4, window((18, 0), (20, 0)))).expect("all free");
        assert_eq!(chosen.number, 3);
    }

    #[test]
    fn refuses_a_table_already_occupied_for_part_of_the_window() {
        let mut room = Room::new(vec![table(1, 2)]);
        room.bookings
            .push(booking(Some(&room.tables[0]), window((19, 0), (21, 0)), 2));
        assert!(assign(&room.request(2, window((18, 0), (20, 0)))).is_none());
    }

    #[test]
    fn accepts_a_table_whose_previous_party_leaves_exactly_as_the_next_arrives() {
        let mut room = Room::new(vec![table(1, 2)]);
        room.bookings
            .push(booking(Some(&room.tables[0]), window((18, 0), (20, 0)), 2));
        let chosen =
            assign(&room.request(2, window((20, 0), (22, 0)))).expect("turns are back to back");
        assert_eq!(chosen.number, 1);
    }

    #[test]
    fn skips_a_table_closed_for_this_shift() {
        let mut room = Room::new(vec![table(1, 2), table(2, 2)]);
        room.blocks.push(TableBlock {
            table_id: room.tables[0].id,
            service_day: day(),
        });
        let chosen = assign(&room.request(2, window((18, 0), (20, 0)))).expect("the other is open");
        assert_eq!(chosen.number, 2);
    }

    #[test]
    fn a_table_closed_on_another_shift_is_still_available_tonight() {
        let mut room = Room::new(vec![table(1, 2)]);
        room.blocks.push(TableBlock {
            table_id: room.tables[0].id,
            service_day: day().checked_add_days(1).expect("in range"),
        });
        assert!(assign(&room.request(2, window((18, 0), (20, 0)))).is_some());
    }

    #[test]
    fn retired_tables_are_not_part_of_the_room() {
        let room = Room::new(vec![retired(1, 2), table(2, 4)]);
        let chosen = assign(&room.request(2, window((18, 0), (20, 0)))).expect("one live table");
        assert_eq!(chosen.number, 2);
    }

    #[test]
    fn a_cancelled_booking_releases_its_table_immediately() {
        let mut room = Room::new(vec![table(1, 2)]);
        let mut cancelled = booking(Some(&room.tables[0]), window((18, 0), (20, 0)), 2);
        cancelled.status = BookingStatus::Cancelled;
        room.bookings.push(cancelled);
        assert!(assign(&room.request(2, window((18, 0), (20, 0)))).is_some());
    }

    #[test]
    fn a_party_that_left_keeps_the_time_it_sat_and_gives_back_the_rest() {
        let mut room = Room::new(vec![table(1, 2)]);
        let mut gone = booking(Some(&room.tables[0]), window((18, 0), (20, 0)), 2);
        gone.status = BookingStatus::Left;
        gone.released_at = Some(at(19, 0));
        room.bookings.push(gone);

        assert!(
            assign(&room.request(2, window((18, 30), (19, 30)))).is_none(),
            "the half hour they were still sitting is theirs"
        );
        assert!(
            assign(&room.request(2, window((19, 0), (20, 0)))).is_some(),
            "from the minute they left, the table is back in the pool"
        );
    }

    #[test]
    fn a_no_show_holds_its_table_only_until_the_moment_it_was_given_up() {
        let mut room = Room::new(vec![table(1, 2)]);
        let mut absent = booking(Some(&room.tables[0]), window((18, 0), (20, 0)), 2);
        absent.status = BookingStatus::NoShow;
        absent.released_at = Some(at(18, 15));
        room.bookings.push(absent);

        assert!(assign(&room.request(2, window((18, 0), (19, 0)))).is_none());
        assert!(assign(&room.request(2, window((18, 15), (20, 0)))).is_some());
    }

    #[test]
    fn a_release_at_the_very_minute_it_began_holds_nothing_at_all() {
        let mut room = Room::new(vec![table(1, 2)]);
        let mut absent = booking(Some(&room.tables[0]), window((18, 0), (20, 0)), 2);
        absent.status = BookingStatus::NoShow;
        absent.released_at = Some(at(18, 0));
        room.bookings.push(absent);
        assert_eq!(room.bookings[0].occupancy(), None);
        assert!(assign(&room.request(2, window((18, 0), (20, 0)))).is_some());
    }

    #[test]
    fn the_largest_party_that_fits_is_the_largest_one_a_table_can_take() {
        let room = Room::new(vec![table(1, 2), table(2, 4), table(3, 8)]);
        let config = config_of(room.tables.clone(), 8);
        assert_eq!(
            largest_party_seatable(
                &config,
                day(),
                window((18, 0), (20, 0)),
                &room.bookings,
                &room.blocks
            ),
            Some(8)
        );
    }

    #[test]
    fn the_largest_party_that_fits_never_exceeds_what_the_bar_accepts() {
        // A ten-top in the room and a cap of six: the answer staff read is the answer they may act
        // on, so it is bounded by the bar's own rule rather than by the furniture.
        let room = Room::new(vec![table(1, 10)]);
        let config = config_of(room.tables.clone(), 6);
        assert_eq!(
            largest_party_seatable(
                &config,
                day(),
                window((18, 0), (20, 0)),
                &room.bookings,
                &room.blocks
            ),
            Some(6)
        );
    }

    #[test]
    fn nobody_fits_when_every_table_is_busy() {
        let mut room = Room::new(vec![table(1, 2), table(2, 4)]);
        let held = window((18, 0), (20, 0));
        room.bookings.push(booking(Some(&room.tables[0]), held, 2));
        room.bookings.push(booking(Some(&room.tables[1]), held, 4));
        let config = config_of(room.tables.clone(), 4);
        assert_eq!(
            largest_party_seatable(&config, day(), held, &room.bookings, &room.blocks),
            None
        );
    }

    #[test]
    fn a_party_leaving_early_grows_the_largest_party_that_fits() {
        let mut room = Room::new(vec![table(1, 2), table(2, 6)]);
        let evening = window((18, 0), (22, 0));
        room.bookings.push(booking(Some(&room.tables[1]), evening, 2));
        let config = config_of(room.tables.clone(), 6);
        let later = window((20, 0), (22, 0));
        assert_eq!(
            largest_party_seatable(&config, day(), later, &room.bookings, &room.blocks),
            Some(2),
            "only the two-top is free while the six-top is sat at"
        );

        room.bookings[0].status = BookingStatus::Left;
        room.bookings[0].released_at = Some(at(20, 0));
        assert_eq!(
            largest_party_seatable(&config, day(), later, &room.bookings, &room.blocks),
            Some(6)
        );
    }

    fn config_of(tables: Vec<BarTable>, max_party: i32) -> ValidConfig {
        ValidConfig::new(BarConfig {
            name: "Пустол".to_owned(),
            address: "ул. Рубинштейна, 24".to_owned(),
            timezone: chrono_tz::Europe::Belgrade,
            week: WeekSchedule::uniform(DayHours {
                open_minutes: 18 * 60,
                close_minutes: 26 * 60,
                closed: false,
            }),
            zones: vec![Zone::new("Зал").expect("non blank")],
            tables,
            turn_minutes: 120,
            slot_step_minutes: 30,
            max_party,
            horizon_days: 4,
            remind_hours: 3,
            grace_minutes: 15,
            message_templates: vec!["Ваш стол готов".to_owned()],
            cancel_reasons: vec!["Дождь".to_owned()],
            staff: vec![StaffMember {
                username: "anna_mgr".to_owned(),
                telegram_user_id: None,
            }],
        })
        .expect("a legal bar")
    }

    #[test]
    fn an_orphan_booking_blocks_nobody() {
        let mut room = Room::new(vec![table(1, 2)]);
        room.bookings
            .push(booking(None, window((18, 0), (20, 0)), 2));
        assert!(assign(&room.request(2, window((18, 0), (20, 0)))).is_some());
    }

    #[test]
    fn reports_no_table_when_the_party_is_larger_than_the_room() {
        let room = Room::new(vec![table(1, 2), table(2, 4)]);
        assert!(assign(&room.request(6, window((18, 0), (20, 0)))).is_none());
    }

    #[test]
    fn reports_no_table_when_every_table_that_fits_is_taken() {
        let mut room = Room::new(vec![table(1, 2), table(2, 2)]);
        let window = window((18, 0), (20, 0));
        room.bookings.push(booking(Some(&room.tables[0]), window, 2));
        room.bookings.push(booking(Some(&room.tables[1]), window, 2));
        assert!(assign(&room.request(2, window)).is_none());
    }

    #[test]
    fn a_booking_being_moved_does_not_block_its_own_reassignment() {
        let mut room = Room::new(vec![table(1, 2)]);
        let existing = booking(Some(&room.tables[0]), window((18, 0), (20, 0)), 2);
        let existing_id = existing.id;
        room.bookings.push(existing);

        let mut request = room.request(2, window((18, 0), (20, 0)));
        assert!(assign(&request).is_none(), "without ignoring, it blocks itself");
        request.ignoring = Some(existing_id);
        assert!(assign(&request).is_some());
    }

    #[test]
    fn a_window_running_past_midnight_still_holds_its_table_on_the_next_shift() {
        // Overlap is absolute, so a late booking taken on one shift is honoured by the
        // allocator when the following shift is being filled. Nothing here filters by day.
        let mut room = Room::new(vec![table(1, 2)]);
        let late = Interval::new(
            at(23, 0),
            Utc.with_ymd_and_hms(2026, 7, 31, 1, 0, 0).single().unwrap(),
        )
        .expect("non empty");
        room.bookings.push(booking(Some(&room.tables[0]), late, 2));

        let next_shift = Interval::new(
            Utc.with_ymd_and_hms(2026, 7, 31, 0, 30, 0).single().unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 31, 2, 30, 0).single().unwrap(),
        )
        .expect("non empty");
        let request = Request {
            party_size: 2,
            window: next_shift,
            service_day: day().checked_add_days(1).expect("in range"),
            tables: &room.tables,
            bookings: &room.bookings,
            blocks: &room.blocks,
            ignoring: None,
        };
        assert!(assign(&request).is_none());
    }

    #[test]
    fn seating_is_unsound_once_its_table_is_retired_shrunk_or_closed() {
        let four_top = table(1, 4);
        let seated = booking(Some(&four_top), window((18, 0), (20, 0)), 4);

        assert!(seating_is_sound(&seated, std::slice::from_ref(&four_top), &[]));

        let shrunk = BarTable {
            seats: 2,
            ..four_top.clone()
        };
        assert!(!seating_is_sound(&seated, std::slice::from_ref(&shrunk), &[]));

        let gone = BarTable {
            retired: true,
            ..four_top.clone()
        };
        assert!(!seating_is_sound(&seated, std::slice::from_ref(&gone), &[]));

        let closed = [TableBlock {
            table_id: four_top.id,
            service_day: day(),
        }];
        assert!(!seating_is_sound(
            &seated,
            std::slice::from_ref(&four_top),
            &closed
        ));

        assert!(
            !seating_is_sound(
                &booking(None, window((18, 0), (20, 0)), 4),
                std::slice::from_ref(&four_top),
                &[]
            ),
            "an orphan is by definition not soundly seated"
        );
    }

    #[test]
    fn seating_is_unsound_when_the_table_no_longer_exists_at_all() {
        let vanished = table(1, 4);
        let seated = booking(Some(&vanished), window((18, 0), (20, 0)), 4);
        assert!(!seating_is_sound(&seated, &[], &[]));
    }
}
