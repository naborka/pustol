//! Failures the storage layer can report, named by what went wrong rather than by which
//! constraint happened to catch it.

use pustol_domain::config::ConfigError;
use pustol_domain::{ServiceDay, TimeError};

/// Every way a storage operation can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No bar, booking or table with that identity.
    #[error("no {entity} with that identity")]
    NotFound { entity: &'static str },

    /// The requested arrival time is not one this shift offers at all: outside opening hours,
    /// off the time step, on a day off, or beyond the booking horizon.
    #[error("{minutes} minutes into that shift is not an arrival time this bar offers")]
    NotAnArrivalTime { minutes: i32 },

    /// The time has gone. Distinct from "taken" because the guest should be told to look at
    /// tonight rather than to try another table.
    #[error("that time has already passed")]
    InThePast,

    /// Every table that could seat this party is busy or closed. The one case the guest sees as
    /// "somebody just took it".
    #[error("no table is free for a party of {party_size} at that time")]
    NoTableFree { party_size: i32 },

    /// The table staff picked is too small, closed, taken, or gone. Separate from
    /// [`Error::NoTableFree`]: pick another table, rather than find room another way.
    #[error("that table is not free for this party")]
    ChosenTableNotFree,

    /// Moving the time of a booking that has already begun. Its window is history the shift reads
    /// — timeline, receipt, the one-party-per-table rule — so it is not rewritten. The table may
    /// still change.
    #[error("that booking has already started")]
    BookingHasStarted,

    /// Moving or cancelling a booking that no longer holds a table. It is the record of an evening
    /// now.
    #[error("that booking is over")]
    BookingHasFinished,

    /// A party larger than the bar accepts through the app.
    #[error("a party of {party_size} is above this bar's limit of {max_party}")]
    PartyTooLarge { party_size: i32, max_party: i32 },

    /// The exclusion constraint fired: another transaction took the table between this one
    /// reading availability and writing. The caller should re-read and offer the truth.
    #[error("that table was taken by another booking while this one was being written")]
    TableTakenConcurrently,

    /// A shift the bar does not offer to guests: a day off, or beyond the booking horizon.
    #[error("{service_day:?} is not a shift guests may book")]
    ShiftNotBookable { service_day: ServiceDay },

    /// A cancellation reason the bar has not configured.
    ///
    /// Free text reaching a guest is the risk this refusal exists for: a borrowed staff account
    /// would otherwise be a way to send anything to everybody who has ever booked.
    #[error("that is not one of this bar's cancellation reasons")]
    UnknownCancelReason,

    /// A message the bar has not configured. Same reasoning as above.
    #[error("that is not one of this bar's messages")]
    UnknownMessage,

    /// A message for a guest the bot cannot write to: a booking taken at the door, with no account
    /// behind it, or an account the bot has found it cannot reach. Somebody has to call them.
    #[error("the bot has no chat with this guest")]
    NoBotChat,

    /// Closing a table without saying why. Guarded here so the reason can never be optional in
    /// storage, where staff would find rows they cannot explain.
    #[error("closing a table needs a reason")]
    MissingBlockReason,

    /// A note longer than a row in a list can show.
    #[error("a note is at most {limit} characters")]
    NoteTooLong { limit: usize },

    /// Seating somebody "now" on a shift that is not the one running, that is a day off, that has not
    /// opened, or whose closing time the wall has last read.
    ///
    /// There is no now on next Tuesday, nor after tonight's closing. Refused here rather than in a
    /// handler, because the only thing that knows which shift is running is the configuration this
    /// layer reads.
    #[error("{service_day} is not the shift that is running")]
    NotTheRunningShift { service_day: chrono::NaiveDate },

    /// The stored configuration cannot be used. Either somebody wrote around the API, or a
    /// migration left the row in a state the domain refuses.
    #[error("the stored configuration is not usable: {0:?}")]
    StoredConfigInvalid(Vec<ConfigError>),

    /// A proposal that cannot even be interpreted — an unknown table, an unknown timezone — as
    /// distinct from one that is understood and illegal.
    #[error("the proposal cannot be interpreted: {0}")]
    UnusableProposal(String),

    /// A stored row the rest of the system cannot make sense of. Always a bug or a write that
    /// bypassed the API; never something a user did.
    #[error("stored {entity} is unusable: {detail}")]
    CorruptRow {
        entity: &'static str,
        detail: String,
    },

    /// A proposed configuration was refused.
    #[error("the proposed configuration is not legal: {0:?}")]
    ProposedConfigInvalid(Vec<ConfigError>),

    /// A proposal made from settings another save has replaced since. Saving it would quietly undo
    /// that save, so nothing is written.
    #[error("the settings have changed since this proposal was made from them")]
    SettingsChanged,

    /// A proposed configuration would strand bookings that have already been promised.
    #[error("the proposed configuration would strand {} booking(s)", .0.len())]
    WouldStrandBookings(Vec<crate::bar::StrandedBooking>),

    /// The guest already has a confirmed or arrived booking on that evening whose time is still
    /// going, so booking again cannot replace it. Staff restoring a booking are refused the same
    /// way when it would leave the guest two tables held that evening, a no-show inside its grace
    /// period counting as one. Decided by occupancy under the bar's lock, not by a constraint,
    /// because whether a booking still runs depends on the clock.
    #[error("this guest already has a booking on this shift")]
    AlreadyBookedThisShift,

    /// A change by staff would leave the guest with two plans: two bookings not yet begun. Booking
    /// again replaces a plan, so a guest never makes two; staff restoring or moving a booking must
    /// not make them either.
    #[error("this guest already has another plan")]
    GuestHasAnotherPlan,

    /// A guest's booking would replace other bookings than the ones their app said it would.
    ///
    /// The app words its button from what booking again replaces — «Перенести» or «Забронировать» —
    /// and the guest agreed to that. By the time the tap arrives a plan may have begun or staff may
    /// have marked a no-show, and carrying on would give up a booking the guest meant to keep, or
    /// keep one they meant to give up. Nothing is written.
    #[error("what this booking would replace is not what the app said it would")]
    BookingChanged,

    /// A booking set back to a status that holds its table, when that table has been given to
    /// another party since it was released. Pick another table or leave the booking as it is.
    #[error("that table has been given to another party since")]
    TableTaken,

    /// An instant could not be built from a service day and a wall-clock minute.
    #[error(transparent)]
    Time(#[from] TimeError),

    /// A timezone name the bar's own arithmetic cannot interpret.
    #[error("{0} is not a timezone this system can compute in")]
    UnknownTimezone(String),

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
}

/// Names of the database constraints whose violation means something specific to a caller.
///
/// Matching on names rather than on message text keeps the mapping honest: renaming a constraint
/// in a migration breaks the test that asserts the mapping, instead of silently turning a
/// meaningful error into a generic one.
mod constraint {
    pub const TABLE_OVERLAP: &str = "booking_one_party_per_table_at_a_time";
}

impl Error {
    /// Turns a database error into the specific failure it represents, where there is one.
    pub(crate) fn from_write(error: sqlx::Error) -> Self {
        let Some(db) = error.as_database_error() else {
            return Self::Database(error);
        };
        match db.constraint() {
            Some(constraint::TABLE_OVERLAP) => Self::TableTakenConcurrently,
            _ => Self::Database(error),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
