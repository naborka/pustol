import { describe, expect, it } from "vitest";

import { refreshedSheet, withBooking, type OpenSheet } from "../sheet";
import { shift, shiftBooking, shiftTable } from "@/components/__tests__/fixtures";

describe("an open sheet when the shift is read again", () => {
  const arrived = shiftBooking({ status: "arrived" });
  const fresh = shift({
    bookings: [arrived, shiftBooking({ id: "b2", guest_name: "Тимур" })],
    tables: [shiftTable({ blocked_because: "Дождь" }), shiftTable({ id: "t2", number: 8 })],
  });

  it("shows the booking as the shift now has it, whichever sheet it is open in", () => {
    for (const kind of ["booking", "templates", "cancelBooking", "move"] as const) {
      expect(refreshedSheet({ kind, booking: shiftBooking() }, fresh)).toEqual({
        kind,
        booking: arrived,
      });
    }
  });

  it("shows the table as the shift now has it", () => {
    expect(refreshedSheet({ kind: "table", table: shiftTable() }, fresh)).toEqual({
      kind: "table",
      table: shiftTable({ blocked_because: "Дождь" }),
    });
  });

  it("leaves a sheet alone when the shift no longer has what it shows", () => {
    const gone: OpenSheet = { kind: "booking", booking: shiftBooking({ id: "elsewhere" }) };
    expect(refreshedSheet(gone, fresh)).toBe(gone);
    const noTable: OpenSheet = { kind: "table", table: shiftTable({ id: "t9" }) };
    expect(refreshedSheet(noTable, fresh)).toBe(noTable);
  });

  it("leaves a sheet that shows no booking and no table alone", () => {
    const sheets: OpenSheet[] = [
      { kind: "none" },
      { kind: "days" },
      { kind: "walkIn" },
      { kind: "manual" },
      { kind: "guestCancel" },
      { kind: "conflict", reasons: ["x"] },
    ];
    for (const sheet of sheets) expect(refreshedSheet(sheet, fresh)).toBe(sheet);
  });
});

describe("an open sheet when one booking comes back changed", () => {
  const updated = shiftBooking({ status: "arrived" });

  it("takes the change only when it still shows that booking", () => {
    expect(withBooking({ kind: "booking", booking: shiftBooking() }, updated)).toEqual({
      kind: "booking",
      booking: updated,
    });
    const other: OpenSheet = { kind: "booking", booking: shiftBooking({ id: "b2" }) };
    expect(withBooking(other, updated)).toBe(other);
  });

  it("does not open a sheet that was closed meanwhile", () => {
    const closed: OpenSheet = { kind: "none" };
    expect(withBooking(closed, updated)).toBe(closed);
  });
});
