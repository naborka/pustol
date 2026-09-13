/**
 * What the app says after it has acted, and where the way back goes.
 */

import { describe, expect, it } from "vitest";

import type { Reconciliation, ShiftBooking } from "../api";
import {
  attendanceOutcome,
  closuresToRestore,
  reconciliationReport,
  strandedLines,
} from "../outcomes";

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
    started: false,
    finished: false,
    ...overrides,
  };
}

const reconciliation = (overrides: Partial<Reconciliation> = {}): Reconciliation => ({
  moved: [],
  orphaned: [],
  ...overrides,
});

describe("what a rearrangement reports", () => {
  it("names who moved and where to", () => {
    expect(
      reconciliationReport(
        reconciliation({
          moved: [{ booking_id: "a", guest_name: "Тимур", to_number: 6 }],
        }),
      ),
    ).toBe("Пересажены: Тимур → стол 6.");
  });

  it("names who it could not place, and what to do about it", () => {
    // The half that used to go missing. A report that says only who moved is a report that hides
    // the one person somebody still has to deal with.
    expect(
      reconciliationReport(reconciliation({ orphaned: [{ booking_id: "b", guest_name: "Глеб" }] })),
    ).toBe(
      "Всё ещё без стола: Глеб — свободных столов на это время нет. Откройте закрытый стол или предложите другое время.",
    );
  });

  it("keeps the two halves separate, and both in one sentence", () => {
    const report = reconciliationReport(
      reconciliation({
        moved: [{ booking_id: "a", guest_name: "Глеб", to_number: 10 }],
        orphaned: [{ booking_id: "b", guest_name: "Тимур" }],
      }),
      "Остались без стола",
    );
    expect(report).toContain("Пересажены: Глеб → стол 10.");
    expect(report).toContain("Остались без стола: Тимур");
  });

  it("says nothing at all when nothing moved", () => {
    expect(reconciliationReport(reconciliation())).toBe("");
  });

  it("never reports a count of what it hoped to do", () => {
    const report = reconciliationReport(
      reconciliation({
        moved: [
          { booking_id: "a", guest_name: "Аня", to_number: 3 },
          { booking_id: "b", guest_name: "Борис", to_number: 4 },
        ],
      }),
    );
    expect(report).toContain("Аня");
    expect(report).toContain("Борис");
    expect(report).not.toMatch(/2 брони|две брони/);
  });
});

describe("what an attendance change says", () => {
  it("names the table when there is one", () => {
    expect(attendanceOutcome(booking({ status: "arrived" }), "arrived")).toBe(
      "Саша за столом 7.",
    );
    expect(attendanceOutcome(booking({ status: "left" }), "left")).toBe("Стол 7 свободен.");
  });

  it("refuses to invent a table number it does not have", () => {
    // `за столом null` is how a screen tells a bartender it has lost the plot.
    const stranded = booking({ table_id: null, table_number: null, table_zone: null });
    for (const attendance of ["arrived", "left", "no_show", "confirmed"] as const) {
      expect(attendanceOutcome(stranded, attendance)).not.toMatch(/null|undefined|NaN/);
    }
  });
});

describe("the bookings a refused save would strand", () => {
  it("lists each by name and time", () => {
    expect(
      strandedLines([
        { guestName: "Саша", startMinutes: 1_260 },
        { guestName: "Тимур", startMinutes: 1_320 },
      ]),
    ).toEqual(["Саша · 21:00", "Тимур · 22:00"]);
  });

  it("still names a booking whose time is missing", () => {
    expect(strandedLines([{ guestName: "Глеб", startMinutes: null }])).toEqual(["Глеб"]);
  });
});

describe("the way back from opening tables", () => {
  it("closes each table the server reopened again, for the reason the server says it had", () => {
    expect(
      closuresToRestore([
        { table_id: "t2", reason: "Дождь" },
        { table_id: "t3", reason: "Частное мероприятие" },
        { table_id: "t4", reason: "Дождь" },
      ]),
    ).toEqual([
      { tableIds: ["t2", "t4"], reason: "Дождь" },
      { tableIds: ["t3"], reason: "Частное мероприятие" },
    ]);
  });

  it("has nothing to close when nothing was reopened", () => {
    expect(closuresToRestore([])).toEqual([]);
  });
});
