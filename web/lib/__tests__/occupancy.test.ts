/**
 * One occupancy rule, and the four consumers that have to agree with it.
 *
 * The bug this file exists to prevent: a party marked `Ушли` at 21:20 still drawn as busy until
 * 23:00 on the timeline, while `свободно сейчас` had already counted the table and the walk-in
 * sheet was offering it. Four readings of one fact is four chances to contradict the room.
 */

import { describe, expect, it } from "vitest";

import type { ShiftBooking, ShiftTable, ShiftView } from "../api";
import {
  freeTablesAt,
  heldBy,
  holdsAt,
  holdsDuring,
  hourlyLoad,
  occupancyEnd,
  peakHour,
  seatedGuestsAt,
  shiftTotals,
  walkInOffers,
} from "../occupancy";

function table(id: string, number: number, seats: number): ShiftTable {
  return { id, number, seats, zone: "Зал", blocked_because: null };
}

function booking(overrides: Partial<ShiftBooking> = {}): ShiftBooking {
  return {
    id: "b1",
    table_id: "t1",
    table_number: 1,
    table_zone: "Зал",
    start_minutes: 1_200,
    end_minutes: 1_320,
    released_minutes: null,
    party_size: 2,
    guest_name: "Саша",
    guest_username: null,
    status: "arrived",
    source: "app",
    note: null,
    reachable_by_bot: true,
    ...overrides,
  };
}

function shift(overrides: Partial<ShiftView> = {}): ShiftView {
  return {
    service_date: "2026-09-11",
    hours: { open_minutes: 1_080, close_minutes: 1_560, closed: false },
    tables: [table("t1", 1, 2), table("t2", 2, 6)],
    bookings: [],
    stats: { bookings: 0, guests: 0, free_now: null },
    now_minutes: 1_280,
    largest_party_seatable_now: null,
    days: [],
    guest_horizon_days: 4,
    cancel_reasons: [],
    message_templates: [],
    ...overrides,
  };
}

describe("the occupancy end", () => {
  it("is the promised end while the booking still holds its table", () => {
    expect(occupancyEnd(booking())).toBe(1_320);
  });

  it("is the recorded minute once the table has gone back into the pool", () => {
    expect(occupancyEnd(booking({ released_minutes: 1_280 }))).toBe(1_280);
  });

  it("never falls outside the window the booking was promised", () => {
    expect(occupancyEnd(booking({ released_minutes: 1_100 }))).toBe(1_200);
    expect(occupancyEnd(booking({ released_minutes: 1_500 }))).toBe(1_320);
  });

  it("holds nothing at all when the table went back the minute it was taken", () => {
    expect(heldBy(booking({ released_minutes: 1_200 }))).toBeNull();
    expect(heldBy(booking({ table_id: null }))).toBeNull();
    expect(heldBy(booking({ status: "cancelled" }))).toBeNull();
  });
});

describe("a party that leaves at 21:20", () => {
  const gone = booking({ status: "left", released_minutes: 1_280 });
  const room = shift({ bookings: [gone] });

  it("is holding its table a minute before, and not a minute after", () => {
    expect(holdsAt(gone, 1_279)).toBe(true);
    expect(holdsAt(gone, 1_280)).toBe(false);
  });

  it("makes all four readings of the room agree", () => {
    // Free tables now, the room's seated-guest count, the walk-in offer and the width of the
    // block on the timeline: four consumers, one function, and they move together.
    expect(freeTablesAt(room.tables, room.bookings, 1_279).map((found) => found.number)).toEqual([
      2,
    ]);
    expect(freeTablesAt(room.tables, room.bookings, 1_280).map((found) => found.number)).toEqual([
      1, 2,
    ]);

    expect(seatedGuestsAt(room.bookings, 1_279)).toBe(2);
    expect(seatedGuestsAt(room.bookings, 1_280)).toBe(0);

    // The walk-in sheet would not have offered the small table before they left, and does after.
    const before = shift({ bookings: [booking()], now_minutes: 1_279 });
    expect(walkInOffers(before, 2, 120).map((offer) => offer.table.number)).toEqual([2]);
    expect(
      walkInOffers(shift({ bookings: [gone], now_minutes: 1_280 }), 2, 120).map(
        (offer) => offer.table.number,
      ),
    ).toEqual([1, 2]);

    // And the block on the timeline stops at 21:20 rather than at 22:00.
    expect(occupancyEnd(gone) - gone.start_minutes).toBe(80);
  });

  it("still counts as a party that came, on the shift's own receipt", () => {
    const totals = shiftTotals(room);
    expect(totals.arrived).toBe(1);
    expect(totals.noShow).toBe(0);
    expect(totals.guests).toBe(2);
  });
});

describe("what the shift adds up to", () => {
  it("keeps walk-ins out of the booking count and names them on their own", () => {
    const room = shift({
      bookings: [
        booking({ id: "a", source: "app" }),
        booking({ id: "b", table_id: "t2", table_number: 2, source: "walk", party_size: 4 }),
      ],
    });
    const totals = shiftTotals(room);
    expect(totals.bookings).toBe(1);
    expect(totals.walkIns).toBe(1);
    expect(totals.guests).toBe(6);
  });

  it("measures occupancy as table-hours held over table-hours open", () => {
    // Two tables, eight hours open: sixteen table-hours. One two-hour booking is one eighth.
    const room = shift({ bookings: [booking()] });
    expect(shiftTotals(room).occupancy).toBeCloseTo(120 / (480 * 2), 6);
  });

  it("never exceeds one, however far a turn runs past closing", () => {
    const room = shift({
      hours: { open_minutes: 1_080, close_minutes: 1_320, closed: false },
      tables: [table("t1", 1, 2)],
      bookings: [booking({ start_minutes: 1_080, end_minutes: 1_560 })],
    });
    expect(shiftTotals(room).occupancy).toBeLessThanOrEqual(1);
  });

  it("reports nothing at all on a day the bar is shut", () => {
    const room = shift({
      hours: { open_minutes: 1_080, close_minutes: 1_560, closed: true },
      bookings: [],
    });
    expect(shiftTotals(room).occupancy).toBe(0);
    expect(hourlyLoad(room)).toEqual([]);
    expect(peakHour(hourlyLoad(room))).toBeNull();
  });
});

describe("the hourly bars", () => {
  it("count a table in the hours it is actually held for", () => {
    const room = shift({
      bookings: [booking({ start_minutes: 1_260, end_minutes: 1_380 })],
    });
    const load = hourlyLoad(room);
    expect(load.map((hour) => hour.tables)).toEqual([0, 0, 0, 1, 1, 0, 0, 0]);
    expect(peakHour(load)).toEqual({ minute: 1_260, tables: 1 });
  });

  it("shrink when a party leaves early", () => {
    const room = shift({
      bookings: [
        booking({ start_minutes: 1_260, end_minutes: 1_380, status: "left", released_minutes: 1_300 }),
      ],
    });
    expect(hourlyLoad(room).map((hour) => hour.tables)).toEqual([0, 0, 0, 1, 0, 0, 0, 0]);
  });
});

describe("holding a table over a stretch", () => {
  it("is half-open, so back-to-back bookings do not collide", () => {
    const first = booking({ start_minutes: 1_200, end_minutes: 1_320 });
    expect(holdsDuring(first, 1_320, 1_440)).toBe(false);
    expect(holdsDuring(first, 1_319, 1_440)).toBe(true);
  });
});

describe("the tables offered to a party at the door", () => {
  it("lists every free table, the ones the party fits at first", () => {
    // Free-but-too-small tables are still drawn. A bartender looking at an empty room and reading
    // «свободного стола нет» is being told the app has lost the plot; the reason is the answer.
    const room = shift({
      tables: [table("t1", 1, 2), table("t2", 2, 6), table("t3", 3, 4)],
      now_minutes: 1_280,
    });
    expect(walkInOffers(room, 3, 120).map((offer) => [offer.table.number, offer.fits])).toEqual([
      [3, true],
      [2, true],
      [1, false],
    ]);
  });

  it("leaves out what nobody can be put at: taken, or closed for the evening", () => {
    const room = shift({
      tables: [
        table("t1", 1, 2),
        { ...table("t2", 2, 6), blocked_because: "Дождь" },
        table("t3", 3, 4),
      ],
      bookings: [
        booking({ table_id: "t3", table_number: 3, start_minutes: 1_300, end_minutes: 1_420 }),
      ],
      now_minutes: 1_280,
    });
    expect(walkInOffers(room, 2, 120).map((offer) => offer.table.number)).toEqual([1]);
  });

  it("has nothing to offer on an evening that is not running", () => {
    expect(walkInOffers(shift({ now_minutes: null }), 2, 120)).toEqual([]);
  });
});
