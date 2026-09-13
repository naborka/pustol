//! Bot button callback data; one module writes and reads it so worker and inbox never disagree.

use pustol_domain::BookingId;
use uuid::Uuid;

const CANCEL_BOOKING: &str = "cancel_booking";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Callback {
    /// «Не смогу прийти», under a reminder.
    CancelBooking(BookingId),
}

/// 51 bytes; Telegram caps callback data at 64.
#[must_use]
pub fn cancel_booking(booking: BookingId) -> String {
    format!("{CANCEL_BOOKING}:{}", booking.0)
}

/// `None` for data this system never wrote.
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
