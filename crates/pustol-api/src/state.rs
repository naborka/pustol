//! What every handler needs to hand.

use chrono::{DateTime, Utc};
use pustol_db::{BarId, Store};
use pustol_telegram::{Bot, BotToken};

/// Where "now" comes from.
///
/// Injected rather than read from the system clock at the point of use, because half the rules in
/// this system are about time — is that slot in the past, has that booking finished, is this
/// payload stale — and a test that cannot choose the hour cannot check any of them.
#[derive(Clone, Debug)]
pub enum Clock {
    System,
    /// A fixed instant, for tests.
    Fixed(DateTime<Utc>),
}

impl Clock {
    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        match self {
            Self::System => Utc::now(),
            Self::Fixed(instant) => *instant,
        }
    }
}

/// Everything the handlers share.
#[derive(Clone, Debug)]
pub struct AppState {
    pub store: Store,
    pub bot: Bot,
    /// The bar this deployment serves.
    ///
    /// Resolved once at start-up. The system is deliberately single-tenant: nothing in the schema
    /// prevents a second bar, and every query is already scoped by bar id, but there is no route
    /// that takes one and no need yet for one.
    pub bar: BarId,
    clock: Clock,
    token: BotToken,
}

impl AppState {
    pub fn new(store: Store, bot: Bot, bar: BarId, token: BotToken, clock: Clock) -> Self {
        Self {
            store,
            bot,
            bar,
            clock,
            token,
        }
    }

    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    #[must_use]
    pub const fn bot_token(&self) -> &BotToken {
        &self.token
    }
}
