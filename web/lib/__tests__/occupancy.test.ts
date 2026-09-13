/**
 * One occupancy rule; every consumer agrees with it.
 *
 * Party marked `Ушли` at 21:20 must not stay busy to 23:00 on timeline while `свободно сейчас` counts table free.
 */

import { describe, expect, it } from "vitest";

import type { ShiftBooking, ShiftView } from "../api";
import {
  heldBy,
  holdsDuring,
  hourlyLoad,
  occupancyEnd,
  peakHour,
  shiftTotals,
  slotOffers,
  walkInOffers,
} from "../occupancy";
import { availability, shift, shiftBooking, shiftTable } from "@/components/__tests__/fixtures";

/** Party seated at table 1, 20:00 to 22:00. */
function booking(overrides: Partial<ShiftBooking> = {}): ShiftBooking {
  return shiftBooking({
    table_number: 1,
    start_minutes: 1_200,
    end_minutes: 1_320,
    status: "arrived",
    started: true,
    ...overrides,
  });
}

function room(overrides: Partial<ShiftView> = {}): ShiftView {
  return shift({
    tables: [shiftTable({ number: 1 }), shiftTable({ id: "t2", number: 2, seats: 6 })],
    bookings: [],
    walk_in_free_table_ids: [],
    ...overrides,
  });
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
  const evening = room({ bookings: [gone] });

  it("is holding its table a minute before, and not a minute after", () => {
    expect(holdsDuring(gone, 1_279, 1_280)).toBe(true);
    expect(holdsDuring(gone, 1_280, 1_281)).toBe(false);
  });

  it("makes the readings of the room this screen works out agree", () => {
    // Hourly bars and timeline block width share one function.
    expect(hourlyLoad(evening).map((hour) => hour.tables)).toEqual([0, 0, 1, 1, 0, 0, 0, 0]);
    expect(occupancyEnd(gone) - gone.start_minutes).toBe(80);
  });

  it("still counts as a party that came, on the shift's own receipt", () => {
    const totals = shiftTotals(evening);
    expect(totals.arrived).toBe(1);
    expect(totals.noShow).toBe(0);
    expect(totals.guests).toBe(2);
  });
});

describe("what the shift adds up to", () => {
  it("keeps walk-ins out of the booking count and names them on their own", () => {
    const evening = room({
      bookings: [
        booking({ id: "a", source: "app" }),
        booking({ id: "b", table_id: "t2", table_number: 2, source: "walk", party_size: 4 }),
      ],
    });
    const totals = shiftTotals(evening);
    expect(totals.bookings).toBe(1);
    expect(totals.walkIns).toBe(1);
    expect(totals.guests).toBe(6);
  });

  it("measures occupancy as table-hours held over table-hours open", () => {
    // Two tables, eight hours open: sixteen table-hours. One two-hour booking is one eighth.
    const evening = room({ bookings: [booking()] });
    expect(shiftTotals(evening).occupancy).toBeCloseTo(120 / (480 * 2), 6);
  });

  it("never exceeds one, however far a turn runs past closing", () => {
    const evening = room({
      hours: { open_minutes: 1_080, close_minutes: 1_320, closed: false },
      tables: [shiftTable({ number: 1 })],
      bookings: [booking({ start_minutes: 1_080, end_minutes: 1_560 })],
    });
    expect(shiftTotals(evening).occupancy).toBeLessThanOrEqual(1);
  });

  it("reports nothing at all on a day the bar is shut", () => {
    const evening = room({ hours: { open_minutes: 1_080, close_minutes: 1_560, closed: true } });
    expect(shiftTotals(evening).occupancy).toBe(0);
    expect(hourlyLoad(evening)).toEqual([]);
    expect(peakHour(hourlyLoad(evening))).toBeNull();
  });
});

describe("the hourly bars", () => {
  it("count a table in the hours it is actually held for", () => {
    const evening = room({ bookings: [booking({ start_minutes: 1_260, end_minutes: 1_380 })] });
    const load = hourlyLoad(evening);
    expect(load.map((hour) => hour.tables)).toEqual([0, 0, 0, 1, 1, 0, 0, 0]);
    expect(peakHour(load)).toEqual({ minute: 1_260, tables: 1 });
  });

  it("shrink when a party leaves early", () => {
    const evening = room({
      bookings: [
        booking({ start_minutes: 1_260, end_minutes: 1_380, status: "left", released_minutes: 1_300 }),
      ],
    });
    expect(hourlyLoad(evening).map((hour) => hour.tables)).toEqual([0, 0, 0, 1, 0, 0, 0, 0]);
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
  it("lists every table the server names as free, the ones the party fits at first", () => {
    // Free-but-too-small tables are still drawn. A bartender looking at an empty room and reading
    // «свободного стола нет» is being told the app has lost the plot; the reason is the answer.
    const evening = room({
      tables: [
        shiftTable({ number: 1 }),
        shiftTable({ id: "t2", number: 2, seats: 6 }),
        shiftTable({ id: "t3", number: 3, seats: 4 }),
      ],
      walk_in_free_table_ids: ["t2", "t1", "t3"],
    });
    expect(walkInOffers(evening, 3).map((offer) => [offer.table.number, offer.fits])).toEqual([
      [3, true],
      [2, true],
      [1, false],
    ]);
  });

  it("offers no table the server does not name, however free the wall clock says it is", () => {
    // Clocks go back: seated first 02:00, turn ends second 02:00; wall minutes see empty window booking at first 02:30 misses.
    const evening = room({
      tables: [shiftTable({ number: 1 }), shiftTable({ id: "t2", number: 2 })],
      bookings: [
        booking({ status: "confirmed", started: false, start_minutes: 1_590, end_minutes: 1_590 }),
        booking({ id: "b2", table_id: "t2", status: "confirmed", started: false, start_minutes: 1_590, end_minutes: 1_590 }),
      ],
      now_minutes: 1_560,
      walk_in_until_minutes: 1_560,
    });
    expect(walkInOffers(evening, 2)).toEqual([]);
  });

  it("ignores a table it is not drawing", () => {
    expect(walkInOffers(room({ walk_in_free_table_ids: ["gone"] }), 2)).toEqual([]);
  });
});

describe("the tables offered for a booking at a time", () => {
  it("are the ones the server names as free for that time's window, in the walk-in order", () => {
    const { slots } = availability({
      slots: [{ start_minutes: 1_290, state: "free", evening: true, free_table_ids: ["t2", "t1"] }],
    });
    expect(slotOffers(room(), slots, 1_290, 3).map((offer) => [offer.table.number, offer.fits])).toEqual([
      [2, true],
      [1, false],
    ]);
  });

  it("are none at a time the answer has no slot for", () => {
    expect(slotOffers(room(), availability().slots, 1_300, 2)).toEqual([]);
  });
});
