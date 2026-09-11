/**
 * One status machine, and the derivation that used to live in four places.
 */

import { describe, expect, it } from "vitest";

import type { ShiftBooking } from "../api";
import {
  GROUP_ORDER,
  hasStarted,
  groupOf,
  isSettled,
  standingOf,
  statusColor,
  statusLabel,
  statusWash,
  statusWord,
  type Standing,
} from "../status";

function booking(overrides: Partial<ShiftBooking> = {}): ShiftBooking {
  return {
    id: "b1",
    table_id: "t1",
    table_number: 7,
    table_zone: "Стойка",
    start_minutes: 1_260,
    end_minutes: 1_380,
    released_minutes: null,
    party_size: 2,
    guest_name: "Саша",
    guest_username: null,
    status: "confirmed",
    source: "app",
    note: null,
    reachable_by_bot: true,
    ...overrides,
  };
}

describe("lateness", () => {
  const grace = 15;

  it("is not late at the grace boundary itself", () => {
    // 21:00 booking, fifteen minutes of grace: at 21:15 the bar is still holding the table, so
    // the party is expected rather than late. One minute either side is the whole question.
    expect(standingOf(booking(), 1_275, grace)).toEqual({ kind: "waiting" });
    expect(standingOf(booking(), 1_274, grace)).toEqual({ kind: "waiting" });
    expect(standingOf(booking(), 1_276, grace)).toEqual({ kind: "late", minutes: 1 });
  });

  it("counts from the end of the grace period, not from the booking", () => {
    expect(standingOf(booking(), 1_295, grace)).toEqual({ kind: "late", minutes: 20 });
  });

  it("is never late on an evening that is not tonight", () => {
    expect(standingOf(booking(), null, grace)).toEqual({ kind: "waiting" });
  });

  it("only applies to a party still expected", () => {
    expect(standingOf(booking({ status: "arrived" }), 1_400, grace)).toEqual({ kind: "seated" });
  });
});

describe("a settled booking", () => {
  it("names the minute the table went back into the pool", () => {
    const gone = booking({ status: "left", released_minutes: 1_280 });
    expect(standingOf(gone, 1_300, 15)).toEqual({ kind: "left", at: 1_280 });
    expect(statusLabel(standingOf(gone, 1_300, 15))).toBe("Ушли в 21:20 · стол свободен");

    const absent = booking({ status: "no_show", released_minutes: 1_275 });
    expect(statusLabel(standingOf(absent, 1_300, 15))).toBe(
      "Не пришли в 21:15 · стол свободен",
    );
  });

  it("falls back to the promised end when nothing was recorded", () => {
    const gone = booking({ status: "left", released_minutes: null });
    expect(standingOf(gone, 1_400, 15)).toEqual({ kind: "left", at: 1_380 });
  });
});

describe("one vocabulary", () => {
  const every: Standing[] = [
    { kind: "waiting" },
    { kind: "late", minutes: 20 },
    { kind: "seated" },
    { kind: "left", at: 1_280 },
    { kind: "no_show", at: 1_275 },
    { kind: "cancelled" },
  ];

  it("names and colours every standing there is", () => {
    for (const standing of every) {
      expect(statusLabel(standing).length).toBeGreaterThan(0);
      expect(statusWord(standing).length).toBeGreaterThan(0);
      expect(statusColor(standing)).toMatch(/^var\(--(btn|dest|ok|hint)\)$/);
      // Never a literal: the wash behind a booking has to be the user's own accent, not a blue
      // somebody expanded by hand next to the block that uses it.
      expect(statusWash(standing)).toMatch(/^var\(--[a-z-]+\)$/);
    }
  });

  it("says lateness in words and not by colour alone", () => {
    expect(statusLabel({ kind: "late", minutes: 20 })).toBe("Опаздывает 20 мин");
    expect(statusColor({ kind: "late", minutes: 20 })).toBe("var(--dest)");
  });

  it("uses one word for a state, not two", () => {
    // The bug this replaced: `ждём` on the list, `Ждём` in the sheet, `без стола` on a badge.
    expect(statusLabel({ kind: "waiting" })).toBe("Ждём");
    expect(statusWord({ kind: "waiting" })).toBe("Ждём");
    expect(statusLabel({ kind: "seated" })).toBe("За столом");
    expect(statusWord({ kind: "seated" })).toBe("За столом");
  });
});

describe("the groups the shift reads in", () => {
  it("puts a party with no table first, whatever time they are due", () => {
    expect(GROUP_ORDER[0]).toBe("orphan");
    const stranded = booking({ table_id: null, table_number: null, table_zone: null });
    expect(groupOf(stranded, standingOf(stranded, 1_300, 15))).toBe("orphan");
  });

  it("puts a party already seated ahead of the ones whose evening is over", () => {
    expect(GROUP_ORDER).toEqual(["orphan", "waiting", "seated", "settled"]);
    const seated = booking({ status: "arrived" });
    expect(groupOf(seated, standingOf(seated, 1_300, 15))).toBe("seated");
    const gone = booking({ status: "left", released_minutes: 1_280 });
    expect(groupOf(gone, standingOf(gone, 1_300, 15))).toBe("settled");
  });

  it("counts a settled booking as settled even when it never had a table", () => {
    const stranded = booking({ table_id: null, status: "no_show", released_minutes: 1_275 });
    expect(groupOf(stranded, standingOf(stranded, 1_300, 15))).toBe("settled");
    expect(isSettled(standingOf(stranded, 1_300, 15))).toBe(true);
  });
});

describe("whether a booking has begun", () => {
  it("is false until the minute it starts, and false on any other evening", () => {
    const at21 = { start_minutes: 1_260 };
    expect(hasStarted(at21, 1_259)).toBe(false);
    expect(hasStarted(at21, 1_260)).toBe(true);
    expect(hasStarted(at21, null)).toBe(false);
  });
});
