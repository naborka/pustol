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
pub fn cancellation(
    bar_name: &str,
    starts_at: DateTime<Utc>,
    timezone: Tz,
    reason: &str,
) -> String {
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
    plural(count, "гость", "гостя", "гостей")
}

/// Russian plural: `one` after 1, 21, 31; `few` after 2 to 4, 22 to 24; else `many`, teens too.
fn plural(count: i32, one: &str, few: &str, many: &str) -> String {
    let noun = match (count % 100, count % 10) {
        (11..=19, _) => many,
        (_, 1) => one,
        (_, 2..=4) => few,
        _ => many,
    };
    format!("{count} {noun}")
}

const IN_THE_APP: &str = "Забронировать, перенести или отменить стол можно в приложении.";

/// Reply to «Не смогу прийти» once table freed.
pub const CANCELLED_FROM_REMINDER: &str =
    "Бронь отменена. Спасибо, что предупредили — стол ушёл другим гостям.";

/// Reply to button whose booking is gone, started, or never this guest's.
pub const NO_LONGER_ACTIVE: &str = "Эта бронь уже не действует.";

/// Cancel failed on server side. Button stays so guest can retry.
pub const COULD_NOT_CANCEL: &str =
    "Не получилось отменить. Попробуйте ещё раз или отмените в приложении.";

/// Reply to bot start from app reminder prompt.
#[must_use]
pub fn reminders_on(remind_hours: i32) -> String {
    format!(
        "Готово: напоминание о брони придёт сюда за {}.",
        hours(remind_hours)
    )
}

/// Reply to bot start any other way.
#[must_use]
pub fn welcome(bar_name: &str, contact: Option<&str>) -> String {
    format!("Это бот бара «{bar_name}». {IN_THE_APP}{}", reach(contact))
}

/// Reply to typed message. Nobody reads chat; silence feels like being ignored, so name contact.
#[must_use]
pub fn nobody_reads_this(bar_name: &str, contact: Option<&str>) -> String {
    format!(
        "Это бот бара «{bar_name}», сообщения здесь никто не читает. {IN_THE_APP}{}",
        reach(contact)
    )
}

fn reach(contact: Option<&str>) -> String {
    contact.map_or_else(String::new, |contact| {
        format!(" Связаться с баром: {contact}.")
    })
}

fn hours(count: i32) -> String {
    plural(count, "час", "часа", "часов")
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
