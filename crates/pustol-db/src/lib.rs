//! Storage for Pustol.
//!
//! This crate owns the transactions, and therefore the invariants that only a transaction can
//! maintain. The methods on [`Store`] are shaped like the bar's actual operations — take a
//! booking, close the terrace, save the settings — rather than like table rows, because each of
//! those operations is a read, a decision made by [`pustol_domain`], and a write that must all
//! happen as one thing. Exposing row-level helpers instead would push transaction and locking
//! discipline up into the HTTP layer, where forgetting it is invisible until two guests are given
//! the same table.
//!
//! Queries are built at run time rather than through `sqlx`'s compile-time macros. That keeps
//! `cargo build` hermetic — no database and no committed query cache is needed to compile — at the
//! cost of moving column and type mistakes from compile time to the integration suite, which
//! exercises every query in this crate against a freshly migrated database.

pub mod bar;
pub mod bookings;
pub mod error;
pub mod identity;
pub mod ids;
pub mod notifications;
pub mod records;

use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgConnection, PgPool};

pub use error::{Error, Result};
pub use ids::{BarId, TelegramUserId};
pub use records::{BlockRecord, BookingRecord, BookingSource};

/// The schema is applied from here at start-up, so a database and the code that talks to it can
/// never diverge.
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// A handle on the database.
#[derive(Clone, Debug)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Opens a pool. `max_connections` is a deployment decision, so it is asked for rather than
    /// guessed at.
    pub async fn connect(url: &str, max_connections: u32) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    #[must_use]
    pub const fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Brings the database up to date.
    pub async fn migrate(&self) -> Result<()> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }
}

/// Serialises every decision about who gets a table in one bar.
///
/// The allocator has to read the room and then write a booking, and no amount of care in between
/// closes the window in which another transaction does the same. The exclusion constraint on
/// `booking` makes the resulting overlap impossible to store, but a constraint violation is a
/// failed request; this lock makes the ordinary case succeed on the first attempt.
///
/// The scope is the whole bar, not the shift, because a booking window can outlast midnight and
/// so two shifts can compete for one table. Locking per shift would be correct only as long as
/// the gap between closing and opening stayed longer than the longest turn — a coincidence of
/// today's limits, not a property of the design. Different bars hash to different keys and never
/// wait on each other.
pub(crate) async fn lock_bar(connection: &mut PgConnection, bar: BarId) -> Result<()> {
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(bar.0.to_string())
        .execute(connection)
        .await?;
    Ok(())
}
