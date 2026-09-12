//! The data behind the bot's buttons.
//!
//! Written and read in one place. The worker draws the button and the inbox answers it, and two
//! spellings of one string are a button that does nothing — which is what the reminder's button
//! was until something read it.

use pustol_domain::BookingId;
use uuid::Uuid;

const CANCEL_BOOKING: &str = "cancel_booking";

/// What a button asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Callback {
    /// «Не смогу прийти», under a reminder.
    CancelBooking(BookingId),
}

/// The data for a button that gives this booking's table back. 51 bytes, within Telegram's 64.
#[must_use]
pub fn cancel_booking(booking: BookingId) -> String {
    format!("{CANCEL_BOOKING}:{}", booking.0)
}

/// What a tap asks for, or `None` for data this system never wrote.
#[must_use]
pub fn parse(data: &str) -> Option<Callback> {
    let (kind, rest) = data.split_once(':')?;
    match kind {
        CANCEL_BOOKING => Uuid::parse_str(rest)
            .ok()
            .map(|id| Callback::CancelBooking(BookingId(id))),
        _ => None,
    }
}
