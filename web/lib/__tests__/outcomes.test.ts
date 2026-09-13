/**
 * What the app says after it has acted, and where the way back goes.
 */

import { describe, expect, it } from "vitest";

import type { Reconciliation } from "../api";
import {
  attendanceOutcome,
  closuresToRestore,
  reconciliationReport,
  refusalOf,
  strandedLines,
} from "../outcomes";
import { shiftBooking as booking } from "@/components/__tests__/fixtures";

describe("a refused settings save, kept for «Почему»", () => {
  it("names the bookings a change would strand, and why they stand", () => {
    expect(
      refusalOf({
        code: "would_strand_bookings",
        message: "",
        detail: { conflicts: [{ guest_name: "Саша", start_minutes: 1_260 }, { guest_name: "Глеб" }] },
      }),
    ).toEqual({
      lead: "Эти брони уже приняты по действующим правилам. Сначала перенесите или отмените их — тогда настройку можно будет сохранить.",
      reasons: ["Саша · 21:00", "Глеб"],
    });
  });

  it("lists what the server found wrong with the values, without calling them bookings", () => {
    expect(
      refusalOf({
        code: "settings_invalid",
        message: "",
        detail: { reasons: ["Стол 1: столько мест не бывает."] },
      }),
    ).toEqual({
      lead: "Сервер не принял эти значения. Исправьте их и сохраните снова.",
      reasons: ["Стол 1: столько мест не бывает."],
    });
  });

  it("is nothing for a failure that has no reasons to keep", () => {
    expect(refusalOf({ code: "settings_changed", message: "" })).toBeNull();
    expect(refusalOf({ code: "network", message: "" })).toBeNull();
  });
});

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
