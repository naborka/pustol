//! Pure reservation domain for Pustol: no I/O, no database, no clock of its own.
//!
//! Everything that decides *whether a table can be given to a guest* lives here, so that the
//! same rules answer the guest-facing picker, the staff-facing manual booking form, the
//! reassignment that follows closing a table and the reconciliation that follows changing the
//! room. A rule implemented twice is a rule that will eventually disagree with itself.
//!
//! Three commitments hold the design together:
//!
//! * **A booking is only ever recorded against a concrete table.** Nothing is accepted against
//!   an aggregate seat count, so "confirmed but unseatable" has no representation at all.
//! * **Overlap is only ever judged on absolute instants.** Opening hours are wall-clock facts
//!   and are compared as wall-clock minutes; whether two parties would share a table is a
//!   question about real time. Answering the second question in wall-clock minutes is how a
//!   table gets sold twice on the night the clocks change.
//! * **Derived facts are derived.** The latest arrival time is closing time minus one turn, and
//!   is computed everywhere it is needed rather than stored somewhere it can drift.
//!
//! Nothing in this crate reads the clock. The current instant is always an argument, which is
//! what makes "this slot is in the past" and "this booking has already finished" testable.

pub mod allocator;
pub mod config;
pub mod draft;
pub mod reconcile;
pub mod schedule;
pub mod service_day;
pub mod slots;

pub use allocator::{
    Assignment, Booking, BookingId, BookingStatus, TableBlock, free_tables,
    largest_party_seatable, seating_is_sound,
};
pub use config::{
    BarConfig, Bounds, ConfigError, DayHours, LIMITS, Limits, ScheduleConflict, Setting,
    StaffMember, ValidConfig, WeekSchedule, is_telegram_username, parties_above_cap,
    schedule_conflicts,
};
pub use draft::{DayHoursDraft, Draft, DraftError, StaffDraft, TableDraft};
pub use reconcile::{Reconciliation, Reseating};
pub use schedule::{BarTable, TableId, Zone, ZoneError, next_table_number};
pub use service_day::{Interval, ServiceDay, TimeError, minutes_within, resolve};
pub use slots::{
    PartOfDay, Slot, SlotAvailability, bookable_days, days_from, first_free_minutes, horizon_days,
};
