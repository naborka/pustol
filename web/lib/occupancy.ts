/**
 * When booking holds its table: screen half of the one occupancy rule.
 *
 * Server answers free-now, arrival times, free tables per time and walk-in offer from same interval.
 * What staff screen still derives from bookings goes only through [`occupancyEnd`], so timeline
 * block and shift occupancy figures never disagree about party that left at 21:20.
 */

import type { ShiftBooking, ShiftTable, ShiftView, StaffSlot } from "./api";

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

/** Whether a booking holds its table at any point of `[from, to)`. */
export function holdsDuring(booking: ShiftBooking, from: number, to: number): boolean {
  const held = heldBy(booking);
  return held !== null && held.from < to && from < held.to;
}

/** A table standing empty, and whether the party at the door actually fits at it. */
export interface TableOffer {
  table: ShiftTable;
  fits: boolean;
}

/**
 * Server-named free tables as offers: fitting first, each half smallest first, ties by number.
 *
 * Allocator order, so top is table room would pick: smallest that fits, since couple at six-top is
 * how Friday runs out of six-tops. Server rechecks in transaction, has last word.
 *
 * Too-small tables drawn, not dropped: empty room under «свободного стола нет» reads as broken app.
 */
export function offersOf(
  shift: ShiftView,
  freeIds: readonly string[],
  partySize: number,
): TableOffer[] {
  const free = new Set(freeIds);
  const offers = shift.tables
    .filter((table) => free.has(table.id))
    .sort((left, right) => left.seats - right.seats || left.number - right.number)
    .map((table) => ({ table, fits: table.seats >= partySize }));
  return [...offers.filter((offer) => offer.fits), ...offers.filter((offer) => !offer.fits)];
}

/** Tables server names free for walk-in window now. Never computed here: wall minutes wrong on clock-change night. */
export function walkInOffers(shift: ShiftView, partySize: number): TableOffer[] {
  return offersOf(shift, shift.walk_in_free_table_ids, partySize);
}

/** Tables server names free for slot at `startMinutes`; none when answer lacks that slot. */
export function slotOffers(
  shift: ShiftView,
  slots: readonly StaffSlot[],
  startMinutes: number,
  partySize: number,
): TableOffer[] {
  const slot = slots.find((each) => each.start_minutes === startMinutes);
  return slot ? offersOf(shift, slot.free_table_ids, partySize) : [];
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
