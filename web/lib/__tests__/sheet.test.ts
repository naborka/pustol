import { describe, expect, it } from "vitest";

import {
  NO_SHEET,
  closedIfStill,
  isStill,
  refreshedGuestSheet,
  refreshedSheet,
  type OpenSheet,
} from "../sheet";
import { booking, shift, shiftBooking, shiftTable } from "@/components/__tests__/fixtures";

describe("an open sheet when the shift is read again", () => {
  const arrived = shiftBooking({ status: "arrived" });
  const fresh = shift({
    bookings: [arrived, shiftBooking({ id: "b2", guest_name: "Тимур" })],
    tables: [shiftTable({ blocked_because: "Дождь" }), shiftTable({ id: "t2", number: 8 })],
  });

  it("shows the booking as the shift now has it, whichever sheet it is open in", () => {
    for (const kind of ["booking", "templates", "cancelBooking", "move"] as const) {
      expect(refreshedSheet({ kind, booking: shiftBooking(), opened: 3 }, fresh)).toEqual({
        kind,
        booking: arrived,
        opened: 3,
      });
    }
  });

  it("shows the table as the shift now has it", () => {
    expect(refreshedSheet({ kind: "table", table: shiftTable(), opened: 4 }, fresh)).toEqual({
      kind: "table",
      table: shiftTable({ blocked_because: "Дождь" }),
      opened: 4,
    });
  });

  it("closes a sheet whose booking or table the shift no longer has", () => {
    // A cancelled booking's sheet left open was a way to seat, move or message a booking that no
    // longer exists.
    for (const kind of ["booking", "templates", "cancelBooking", "move"] as const) {
      const gone: OpenSheet = { kind, booking: shiftBooking({ id: "elsewhere" }), opened: 1 };
      expect(refreshedSheet(gone, fresh)).toEqual(NO_SHEET);
    }
    const noTable: OpenSheet = { kind: "table", table: shiftTable({ id: "t9" }), opened: 1 };
    expect(refreshedSheet(noTable, fresh)).toEqual(NO_SHEET);
  });

  it("leaves a sheet that shows no booking and no table alone", () => {
    const sheets: OpenSheet[] = [
      { kind: "none" },
      { kind: "days", opened: 1 },
      { kind: "walkIn", opened: 1 },
      { kind: "manual", opened: 1 },
      { kind: "guestCancel", booking, opened: 1 },
      { kind: "conflict", refusal: { lead: "", reasons: ["x"] }, opened: 1 },
    ];
    for (const sheet of sheets) expect(refreshedSheet(sheet, fresh)).toBe(sheet);
  });
});

describe("the guest's cancel sheet when their bookings are read again", () => {
  const open: OpenSheet = { kind: "guestCancel", booking, opened: 2 };

  it("stays while the booking it restates is still theirs", () => {
    const moved = { ...booking, start_minutes: 1_320 };
    expect(refreshedGuestSheet(open, [moved])).toEqual({ ...open, booking: moved });
  });

  it("closes once that booking is gone, and leaves any other sheet alone", () => {
    expect(refreshedGuestSheet(open, [{ ...booking, id: "b2" }])).toEqual(NO_SHEET);
    const days: OpenSheet = { kind: "days", opened: 3 };
    expect(refreshedGuestSheet(days, [])).toBe(days);
  });
});

describe("closing the sheet an action came from", () => {
  const from: OpenSheet = { kind: "templates", booking: shiftBooking(), opened: 5 };

  it("closes it when it is still open, however its copy has been refreshed since", () => {
    const refreshed = refreshedSheet(from, shift({ bookings: [shiftBooking({ status: "arrived" })] }));
    expect(isStill(refreshed, from)).toBe(true);
    expect(closedIfStill(refreshed, from)).toEqual({ kind: "none" });
  });

  it("leaves another sheet open, even one on the same booking or of the same kind", () => {
    const others: OpenSheet[] = [
      { kind: "booking", booking: shiftBooking(), opened: 6 },
      { kind: "templates", booking: shiftBooking(), opened: 6 },
      { kind: "manual", opened: 5 },
      { kind: "none" },
    ];
    for (const current of others) {
      expect(isStill(current, from)).toBe(false);
      expect(closedIfStill(current, from)).toBe(current);
    }
  });

  it("opens nothing, and closes nothing new, when it came from no sheet at all", () => {
    const none: OpenSheet = { kind: "none" };
    expect(closedIfStill(none, none)).toEqual({ kind: "none" });
    const opened: OpenSheet = { kind: "days", opened: 1 };
    expect(closedIfStill(opened, none)).toBe(opened);
  });
});
