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

/// The message for a moved booking. Names both times — the guest is holding the old one in their
/// head.
#[must_use]
pub fn moved(
    bar_name: &str,
    was_at: DateTime<Utc>,
    now_at: DateTime<Utc>,
    timezone: Tz,
    party_size: i32,
) -> String {
    format!(
        "{bar_name}: бронь с {was} перенесена на {now}, {guests}. Не подходит — напишите нам.",
        was = was_at.with_timezone(&timezone).format("%H:%M"),
        now = now_at.with_timezone(&timezone).format("%H:%M"),
        guests = guests(party_size),
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

/// The answer to «Не смогу прийти» when the table went back.
pub const CANCELLED_FROM_REMINDER: &str =
    "Бронь отменена. Спасибо, что предупредили — стол ушёл другим гостям.";

/// The answer to a button whose booking is gone, started, or was never this guest's.
pub const NO_LONGER_ACTIVE: &str = "Эта бронь уже не действует.";

/// The answer when cancelling failed on our side. The button stays, so the guest can try again.
pub const COULD_NOT_CANCEL: &str = "Не получилось отменить. Попробуйте ещё раз или отмените в приложении.";

/// The answer to starting the bot from the app's reminder prompt.
#[must_use]
pub fn reminders_on(remind_hours: i32) -> String {
    format!(
        "Готово: напоминание о брони придёт сюда за {}.",
        hours(remind_hours)
    )
}

/// The answer to starting the bot any other way.
#[must_use]
pub fn welcome(bar_name: &str, contact: Option<&str>) -> String {
    format!(
        "Это бот бара «{bar_name}». Забронировать, перенести или отменить стол можно в приложении.{}",
        reach(contact)
    )
}

/// The answer to a message typed into the bot's chat. Nobody reads it, and saying nothing would
/// read as being ignored — so it says where somebody does.
#[must_use]
pub fn nobody_reads_this(bar_name: &str, contact: Option<&str>) -> String {
    format!(
        "Это бот бара «{bar_name}», сообщения здесь никто не читает. Забронировать, перенести или отменить стол можно в приложении.{}",
        reach(contact)
    )
}

fn reach(contact: Option<&str>) -> String {
    contact.map_or_else(String::new, |contact| {
        format!(" Связаться с баром: {contact}.")
    })
}

fn hours(count: i32) -> String {
    let noun = match (count % 100, count % 10) {
        (11..=19, _) => "часов",
        (_, 1) => "час",
        (_, 2..=4) => "часа",
        _ => "часов",
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
