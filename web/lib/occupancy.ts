/**
 * When a booking holds its table — the screen's half of the one occupancy rule.
 *
 * The server answers the same question for free-now, for the guest's arrival times and for the
 * walk-in offer, from the same interval. Everything drawn on a staff screen derives from
 * [`occupancyEnd`] and from nothing else, so a block on the timeline, the free-table count in the
 * pulse and the shift's own occupancy figure cannot disagree about a party that went home at 21:20.
 *
 * The bug this exists to remove: a booking marked `left` still drawn as busy until 23:00, while
 * `свободно сейчас` already counted the table as free and the walk-in sheet was offering it.
 */

import type { ShiftBooking, ShiftTable, ShiftView } from "./api";

/** A booking's occupancy, in wall-clock minutes of its shift. */
export interface Held {
  from: number;
  to: number;
}

/**
 * The minute a booking gives its table back.
 *
 * The promised end, unless the party left or turned out not to be coming, in which case it is the
 * minute recorded for that. Clamped to the promised window because that is the only range a
 * booking can hold — the server clamps it too, and saying so twice costs nothing and rules out a
 * negative-width block for good.
 */
export function occupancyEnd(
  booking: Pick<ShiftBooking, "start_minutes" | "end_minutes" | "released_minutes">,
): number {
  const released = booking.released_minutes ?? booking.end_minutes;
  return Math.min(Math.max(released, booking.start_minutes), booking.end_minutes);
}

/** The range a booking holds its table for, or null when it holds nothing. */
export function heldBy(booking: ShiftBooking): Held | null {
  if (booking.table_id === null || booking.status === "cancelled") return null;
  const to = occupancyEnd(booking);
  return to > booking.start_minutes ? { from: booking.start_minutes, to } : null;
}

/** Whether a booking is holding its table at this minute. */
export function holdsAt(booking: ShiftBooking, minute: number): boolean {
  const held = heldBy(booking);
  return held !== null && held.from <= minute && minute < held.to;
}

/** Whether a booking holds its table at any point of `[from, to)`. */
export function holdsDuring(booking: ShiftBooking, from: number, to: number): boolean {
  const held = heldBy(booking);
  return held !== null && held.from < to && from < held.to;
}

/**
 * Tables free for the whole of `[from, to)`, smallest first, ties by printed number.
 *
 * The same order the allocator uses, so the table this offers is the table the server then gives:
 * smallest that fits, because a couple at a six-top is how a Friday runs out of six-tops.
 *
 * Deliberately *not* what the pulse line reads: the free count comes from the server, which answers
 * it from the same rule against the whole room.
 */
export function freeTablesDuring(
  tables: ShiftTable[],
  bookings: ShiftBooking[],
  from: number,
  to: number,
): ShiftTable[] {
  return [...tables]
    .filter(
      (table) =>
        table.blocked_because === null &&
        !bookings.some(
          (booking) => booking.table_id === table.id && holdsDuring(booking, from, to),
        ),
    )
    .sort((left, right) => left.seats - right.seats || left.number - right.number);
}

/** Tables with nobody at them and nothing closing them, at this minute. */
export function freeTablesAt(
  tables: ShiftTable[],
  bookings: ShiftBooking[],
  minute: number,
): ShiftTable[] {
  return freeTablesDuring(tables, bookings, minute, minute + 1);
}

/**
 * How many guests are sitting in the room at this minute.
 *
 * A party that has arrived and one that has since left are the same party: whether they were at the
 * table at a given minute is a question about the range they held, which is the one occupancy rule
 * above. What separates them from a booking that is merely expected — or from a no-show whose table
 * is still being kept — is that somebody actually sat down.
 */
export function seatedGuestsAt(bookings: ShiftBooking[], minute: number): number {
  return bookings
    .filter(
      (booking) =>
        (booking.status === "arrived" || booking.status === "left") && holdsAt(booking, minute),
    )
    .reduce((total, booking) => total + booking.party_size, 0);
}

/** The shift's own receipt: six numbers, all of them derived rather than reported. */
export interface ShiftTotals {
  bookings: number;
  guests: number;
  arrived: number;
  noShow: number;
  walkIns: number;
  /** Booked table-hours over open table-hours, 0 to 1. */
  occupancy: number;
}

export function shiftTotals(shift: ShiftView): ShiftTotals {
  const live = shift.bookings.filter((booking) => booking.status !== "cancelled");
  const open = shift.hours.closed
    ? 0
    : (shift.hours.close_minutes - shift.hours.open_minutes) * shift.tables.length;
  const held = live.reduce((total, booking) => {
    const range = heldBy(booking);
    if (!range) return total;
    // Clipped to opening hours: a turn that runs to the minute the lights go off cannot count for
    // more table-time than the room was open for, or the figure exceeds one.
    const from = Math.max(range.from, shift.hours.open_minutes);
    const to = Math.min(range.to, shift.hours.close_minutes);
    return total + Math.max(0, to - from);
  }, 0);

  return {
    bookings: live.filter((booking) => booking.source !== "walk").length,
    guests: live.reduce((total, booking) => total + booking.party_size, 0),
    arrived: live.filter((booking) => booking.status === "arrived" || booking.status === "left")
      .length,
    noShow: live.filter((booking) => booking.status === "no_show").length,
    walkIns: live.filter((booking) => booking.source === "walk").length,
    occupancy: open > 0 ? held / open : 0,
  };
}

/** One bar of the occupancy chart: how many tables were held during that hour. */
export interface HourLoad {
  /** Wall-clock minute the hour starts at. May exceed 1440 on a shift running past midnight. */
  minute: number;
  tables: number;
}

/**
 * Tables in use per opening hour.
 *
 * Counted at the start of each hour rather than averaged across it: the question the owner asks is
 * "how busy did it get", and a peak smoothed into its neighbours stops answering it.
 */
export function hourlyLoad(shift: ShiftView): HourLoad[] {
  if (shift.hours.closed) return [];
  const hours: HourLoad[] = [];
  for (
    let minute = shift.hours.open_minutes;
    minute < shift.hours.close_minutes;
    minute += 60
  ) {
    hours.push({
      minute,
      tables: shift.tables.filter((table) =>
        shift.bookings.some(
          (booking) => booking.table_id === table.id && holdsDuring(booking, minute, minute + 60),
        ),
      ).length,
    });
  }
  return hours;
}

/** The busiest hour, or null on a shift where nothing was booked. */
export function peakHour(load: HourLoad[]): HourLoad | null {
  return load.reduce<HourLoad | null>(
    (peak, hour) => (hour.tables > 0 && (peak === null || hour.tables > peak.tables) ? hour : peak),
    null,
  );
}
