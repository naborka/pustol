/**
 * Russian, formatted the way a bar writes it.
 *
 * Every function here is pure and takes the date as a plain `YYYY-MM-DD` string, the way the API
 * sends it. Nothing constructs a `Date` from that string directly: `new Date("2026-07-30")` is
 * parsed as UTC midnight and then read back in the device's timezone, so on any negative offset the
 * weekday comes out a day early. Weekdays are therefore computed on explicit UTC parts.
 */

const WEEKDAY_SHORT = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"] as const;
const WEEKDAY_LONG = [
  "Воскресенье",
  "Понедельник",
  "Вторник",
  "Среда",
  "Четверг",
  "Пятница",
  "Суббота",
] as const;
/** Genitive month names, because a date is read as "30 июля" and never as "30 июль". */
const MONTH_GENITIVE = [
  "янв",
  "фев",
  "мар",
  "апр",
  "мая",
  "июн",
  "июл",
  "авг",
  "сен",
  "окт",
  "ноя",
  "дек",
] as const;

/** A calendar date as the API writes it. */
export type IsoDate = string;

interface DateParts {
  year: number;
  month: number;
  day: number;
}

function parts(date: IsoDate): DateParts {
  const [year, month, day] = date.split("-").map(Number);
  if (
    year === undefined ||
    month === undefined ||
    day === undefined ||
    Number.isNaN(year) ||
    Number.isNaN(month) ||
    Number.isNaN(day)
  ) {
    throw new Error(`not a date: ${date}`);
  }
  return { year, month, day };
}

/** Weekday index, Sunday first, computed without touching the device's timezone. */
export function weekdayIndex(date: IsoDate): number {
  const { year, month, day } = parts(date);
  return new Date(Date.UTC(year, month - 1, day)).getUTCDay();
}

export function weekdayShort(date: IsoDate): string {
  return WEEKDAY_SHORT[weekdayIndex(date)] ?? "";
}

export function weekdayLongByIndex(index: number): string {
  return WEEKDAY_LONG[index] ?? "";
}

export function weekdayShortByIndex(index: number): string {
  return WEEKDAY_SHORT[index] ?? "";
}

/** `date` shifted by whole days, staying in the same string form. */
export function addDays(date: IsoDate, days: number): IsoDate {
  const { year, month, day } = parts(date);
  const shifted = new Date(Date.UTC(year, month - 1, day + days));
  return shifted.toISOString().slice(0, 10);
}

/**
 * Wall-clock minutes as a clock reads them.
 *
 * Minutes past 1440 belong to a shift that runs past midnight, so 1560 is 02:00 rather than 26:00 —
 * the guest is looking at a clock, not at an offset.
 */
export function time(minutes: number): string {
  const hour = Math.floor(minutes / 60) % 24;
  const minute = ((minutes % 60) + 60) % 60;
  return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
}

/** "10:00 — 02:00", or the day off. */
export function hoursLabel(hours: {
  open_minutes: number;
  close_minutes: number;
  closed: boolean;
}): string {
  if (hours.closed) return "Выходной";
  return `${time(hours.open_minutes)} — ${time(hours.close_minutes)}`;
}

/** Hours as a whole number where it is one, so "2 ч" rather than "2.0 ч". */
export function hours(minutes: number): string {
  const value = minutes / 60;
  return Number.isInteger(value) ? `${value} ч` : `${value.toFixed(1)} ч`;
}

/**
 * Picks the right form of a Russian noun for a count.
 *
 * A bare "3 гость" is the mark of software nobody finished, and the rule is not the same as the
 * English one: 21 takes the singular, 11 does not.
 */
export function plural(count: number, one: string, few: string, many: string): string {
  const hundreds = Math.abs(count) % 100;
  const units = Math.abs(count) % 10;
  if (hundreds >= 11 && hundreds <= 19) return many;
  if (units === 1) return one;
  if (units >= 2 && units <= 4) return few;
  return many;
}

export function guests(count: number): string {
  return `${count} ${plural(count, "гость", "гостя", "гостей")}`;
}

export function bookings(count: number): string {
  return `${count} ${plural(count, "бронь", "брони", "броней")}`;
}

export function tables(count: number): string {
  return `${count} ${plural(count, "стол", "стола", "столов")}`;
}

export function seats(count: number): string {
  return `${count} ${plural(count, "место", "места", "мест")}`;
}

export function minutesWord(count: number): string {
  return `${count} ${plural(count, "минута", "минуты", "минут")}`;
}

export function daysWord(count: number): string {
  return `${count} ${plural(count, "день", "дня", "дней")}`;
}

export function hoursWord(count: number): string {
  return `${count} ${plural(count, "час", "часа", "часов")}`;
}

/** "Сегодня", "Завтра", or the weekday. */
export function dayName(date: IsoDate, today: IsoDate): string {
  if (date === today) return "Сегодня";
  if (date === addDays(today, 1)) return "Завтра";
  return weekdayShort(date);
}

/** "30 июл" — the line under the day name. */
export function dayDate(date: IsoDate): string {
  const { month, day } = parts(date);
  return `${day} ${MONTH_GENITIVE[month - 1] ?? ""}`;
}

/**
 * The one-line form: "Сегодня", or "чт, 30 июл" once the day needs naming.
 *
 * Decided on the dates rather than on what `dayName` happened to return. Comparing against the
 * words would make rewording "Сегодня" silently produce "Сегодня, 30 июл".
 */
export function dayFull(date: IsoDate, today: IsoDate): string {
  if (date === today || date === addDays(today, 1)) return dayName(date, today);
  return `${dayName(date, today)}, ${dayDate(date)}`;
}

/** "Сегодня в 20:00" — the headline on a guest's booking card. */
export function whenLabel(date: IsoDate, today: IsoDate, startMinutes: number): string {
  return `${dayFull(date, today)} в ${time(startMinutes)}`;
}

/**
 * What one chip on the day rail is called: "Сегодня", "Завтра", or "сб 12".
 *
 * The day of the month is on the chip rather than only in a subtitle because a thirty-day rail
 * contains four Saturdays, and four chips reading "сб" are four chips a guest cannot tell apart.
 */
export function dayChip(date: IsoDate, today: IsoDate): string {
  if (date === today) return "Сегодня";
  if (date === addDays(today, 1)) return "Завтра";
  const { day } = parts(date);
  return `${weekdayShort(date)} ${day}`;
}

/** "пт, 11 сен" — the line under the day name in the shift header. */
export function dayStamp(date: IsoDate): string {
  return `${weekdayShort(date)}, ${dayDate(date)}`;
}

/** "Открыт до 02:00" or "Закрыт", from the bar's hours and the bar's own clock. */
export function openLabel(
  hours: { open_minutes: number; close_minutes: number; closed: boolean },
  nowMinutes: number,
): string {
  if (hours.closed) return "Закрыт";
  if (nowMinutes < hours.open_minutes) return `Откроется в ${time(hours.open_minutes)}`;
  if (nowMinutes >= hours.close_minutes) return "Закрыт";
  return `Открыт до ${time(hours.close_minutes)}`;
}

/** Whether the bar is serving at this minute — what the dot on the header pill is coloured by. */
export function isOpenNow(
  hours: { open_minutes: number; close_minutes: number; closed: boolean },
  nowMinutes: number,
): boolean {
  return !hours.closed && nowMinutes >= hours.open_minutes && nowMinutes < hours.close_minutes;
}

/** A ratio as a whole percentage: "72 %" reads as a measurement, "72.4 %" as a spreadsheet. */
export function percent(ratio: number): string {
  return `${Math.round(ratio * 100)} %`;
}
