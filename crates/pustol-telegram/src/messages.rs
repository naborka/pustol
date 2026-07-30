//! What the bot says.
//!
//! The bot is a presentation surface of its own: it speaks to guests who are not looking at the
//! app, so there is no screen to render this text and it cannot live in the frontend. The bar's own
//! editable messages — the cancellation reasons, the templates staff pick from — are data and live
//! in the database; these are the words the system itself uses.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// The reminder a guest asked for.
///
/// Names the hour rather than "soon", because a reminder whose whole purpose is to prompt a
/// cancellation has to be actionable at a glance.
#[must_use]
pub fn reminder(bar_name: &str, starts_at: DateTime<Utc>, timezone: Tz, party_size: i32) -> String {
    let local = starts_at.with_timezone(&timezone);
    format!(
        "Напоминаем о брони: {bar_name}, {time}, {guests}. Не сможете прийти — отмените, чтобы \
         стол ушёл другим гостям.",
        time = local.format("%H:%M"),
        guests = guests(party_size),
    )
}

/// The message that goes out when staff cancel a booking.
#[must_use]
pub fn cancellation(bar_name: &str, starts_at: DateTime<Utc>, timezone: Tz, reason: &str) -> String {
    let local = starts_at.with_timezone(&timezone);
    format!(
        "{bar_name}: бронь на {time} отменена. Причина: {reason}. Извините за неудобство — \
         напишите нам, если хотите другое время.",
        time = local.format("%H:%M"),
    )
}

/// Russian counts the noun after the number, so a bare "3 гость" reads as broken software.
#[must_use]
pub fn guests(count: i32) -> String {
    let hundreds = count % 100;
    let units = count % 10;
    let noun = if (11..=19).contains(&hundreds) {
        "гостей"
    } else if units == 1 {
        "гость"
    } else if (2..=4).contains(&units) {
        "гостя"
    } else {
        "гостей"
    };
    format!("{count} {noun}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_counts_agree_with_their_number() {
        assert_eq!(guests(1), "1 гость");
        assert_eq!(guests(2), "2 гостя");
        assert_eq!(guests(4), "4 гостя");
        assert_eq!(guests(5), "5 гостей");
        assert_eq!(guests(11), "11 гостей");
        assert_eq!(guests(21), "21 гость");
        assert_eq!(guests(112), "112 гостей");
        assert_eq!(guests(102), "102 гостя");
    }

    #[test]
    fn a_reminder_names_the_local_hour_not_the_stored_instant() {
        let starts_at = DateTime::from_timestamp(1_785_000_000, 0).expect("valid");
        let text = reminder("Бар «Подвал»", starts_at, chrono_tz::Europe::Belgrade, 2);
        let local = starts_at.with_timezone(&chrono_tz::Europe::Belgrade);
        assert!(text.contains(&local.format("%H:%M").to_string()));
        assert!(text.contains("Бар «Подвал»"));
        assert!(text.contains("2 гостя"));
    }

    #[test]
    fn a_cancellation_carries_the_reason_the_guest_was_given() {
        let starts_at = DateTime::from_timestamp(1_785_000_000, 0).expect("valid");
        let text = cancellation(
            "Бар «Подвал»",
            starts_at,
            chrono_tz::Europe::Belgrade,
            "Частное мероприятие",
        );
        assert!(text.contains("Частное мероприятие"));
    }
}
