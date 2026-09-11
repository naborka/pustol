/**
 * What the app says after it has acted, and where the way back goes.
 */

import { describe, expect, it } from "vitest";

import type { Reconciliation, ShiftBooking } from "../api";
import {
  attendanceOutcome,
  previousAttendance,
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

describe("undo", () => {
  it("goes back to the status the booking actually had", () => {
    // A party that was waiting goes back to waiting; one that was at the table goes back to the
    // table. Guessing one of the two would quietly turn a mis-tap into a second mistake.
    expect(previousAttendance(booking({ status: "confirmed" }))).toBe("confirmed");
    expect(previousAttendance(booking({ status: "arrived" }))).toBe("arrived");
    expect(previousAttendance(booking({ status: "left" }))).toBe("arrived");
    expect(previousAttendance(booking({ status: "no_show" }))).toBe("confirmed");
  });

  it("round-trips every reversible change back to where it started", () => {
    for (const status of ["confirmed", "arrived"] as const) {
      const was = booking({ status });
      const before = previousAttendance(was);
      // Whatever happens next — seated, gone, absent — undo returns to `before`.
      for (const next of ["arrived", "left", "no_show"] as const) {
        const after = booking({ status: next });
        expect(previousAttendance({ ...after, status })).toBe(before);
      }
    }
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
