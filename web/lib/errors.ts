/**
 * What a failure says to the person who caused it.
 *
 * The API answers with a stable code and an English sentence meant for a log. The words a guest
 * reads live here, on the screen that shows them: a backend that shipped Russian copy would need a
 * deploy to fix a comma, and would have to guess which of the two audiences is reading.
 */

export interface ApiFailure {
  code: string;
  message: string;
  detail?: Record<string, unknown>;
}

/** What each code means to a guest. */
const GUEST: Record<string, string> = {
  no_credentials: "Откройте приложение из Telegram — так мы поймём, чья это бронь.",
  not_telegram: "Не удалось подтвердить, что запрос из Telegram. Откройте приложение заново.",
  session_expired: "Сессия устарела. Закройте и откройте приложение — всё сохранится.",
  forbidden: "Этот раздел только для сотрудников бара.",
  not_found: "Бронь не найдена — возможно, её уже отменили.",
  not_an_arrival_time: "В это время бар не принимает. Выберите другое.",
  in_the_past: "Это время уже прошло. Посмотрите, что свободно дальше.",
  no_table_free: "Это время только что заняли. Выберите другое.",
  party_too_large: "Такую компанию через приложение не принимаем — напишите бару.",
  shift_not_bookable: "На этот день брони пока нет. Выберите другой.",
  impossible_time: "Такого времени в этот день не существует.",
  internal: "Что-то сломалось у нас. Попробуйте ещё раз через минуту.",
};

/** What each code means to somebody working the shift. */
const STAFF: Record<string, string> = {
  ...GUEST,
  no_table_free: "На это время нет свободного стола для такой компании.",
  chosen_table_not_free: "Этот стол только что заняли или закрыли. Выберите другой.",
  booking_started: "Бронь уже началась — время не перенести. Стол поменять можно.",
  booking_finished: "Этот вечер уже закончился — переносить нечего.",
  not_an_arrival_time: "Бар в это время не работает.",
  party_too_large: "Компания больше лимита. Поднимите лимит в настройках или посадите вручную.",
  shift_not_bookable: "В этот день бар закрыт.",
  blank_guest_name: "Впишите имя гостя — иначе его не позвать.",
  missing_block_reason: "Укажите, почему стол закрыт.",
  unknown_cancel_reason: "Такой причины нет в списке. Обновите настройки.",
  unknown_message: "Такого сообщения нет в списке. Обновите настройки.",
  no_bot_chat: "Гость записан вручную — у бота нет с ним чата. Позвоните или откройте чат.",
  settings_invalid: "Так сохранить нельзя — проверьте отмеченные значения.",
  settings_unreadable: "Настройки не сходятся с текущим залом. Обновите страницу и попробуйте снова.",
  would_strand_bookings:
    "Эти брони уже приняты по действующим правилам. Сначала перенесите или отмените их.",
  unknown_timezone: "Такого часового пояса нет.",
};

const FALLBACK = "Не получилось. Попробуйте ещё раз.";

export type Audience = "guest" | "staff";

/** The sentence to show for a failure. */
export function messageFor(failure: ApiFailure | null, audience: Audience): string {
  if (!failure) return FALLBACK;
  const table = audience === "staff" ? STAFF : GUEST;
  return table[failure.code] ?? FALLBACK;
}

/** Whether the app should tell the guest to relaunch rather than to try again. */
export function needsRelaunch(failure: ApiFailure | null): boolean {
  return (
    failure?.code === "session_expired" ||
    failure?.code === "not_telegram" ||
    failure?.code === "no_credentials"
  );
}

/** One booking a refused settings save would have stranded. */
export interface StrandedBooking {
  guestName: string;
  startMinutes: number | null;
}

/**
 * The bookings a refused settings save names, so the sheet can list them rather than gesture at
 * them.
 *
 * The API has always sent the names and the times; the screen used to throw them away and show one
 * sentence, which left a manager to work out for themselves which of thirty evenings was in the
 * way.
 */
export function strandedBookings(failure: ApiFailure | null): StrandedBooking[] {
  if (failure?.code !== "would_strand_bookings") return [];
  const conflicts = failure.detail?.["conflicts"];
  if (!Array.isArray(conflicts)) return [];
  return conflicts.flatMap((conflict) => {
    if (typeof conflict !== "object" || conflict === null) return [];
    const row = conflict as Record<string, unknown>;
    const guestName = typeof row["guest_name"] === "string" ? row["guest_name"] : "";
    if (guestName.length === 0) return [];
    const start = row["start_minutes"];
    return [{ guestName, startMinutes: typeof start === "number" ? start : null }];
  });
}

/** The reasons a settings save was refused, already worded by the domain. */
export function invalidReasons(failure: ApiFailure | null): string[] {
  if (failure?.code !== "settings_invalid") return [];
  const reasons = failure.detail?.["reasons"];
  if (!Array.isArray(reasons)) return [];
  return reasons.filter((reason): reason is string => typeof reason === "string");
}
