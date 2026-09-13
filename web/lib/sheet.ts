/**
 * Which sheet is open over the app, and what it shows.
 *
 * A sheet holds a copy of the booking or table it was opened on. The shift keeps changing under it —
 * the half-minute refresh, a colleague's phone, the action just taken — and a copy nobody updates is
 * how an undo restores a status the booking stopped having minutes ago.
 */

import type { ShiftBooking, ShiftTable, ShiftView } from "./api";

export type OpenSheet =
  | { kind: "none" }
  | { kind: "booking"; booking: ShiftBooking }
  | { kind: "templates"; booking: ShiftBooking }
  | { kind: "cancelBooking"; booking: ShiftBooking }
  | { kind: "table"; table: ShiftTable }
  | { kind: "conflict"; reasons: string[] }
  | { kind: "days" }
  | { kind: "walkIn" }
  | { kind: "manual" }
  | { kind: "move"; booking: ShiftBooking }
  | { kind: "guestCancel" };

/** The sheet with `booking` in place of its copy, if it still shows that booking; otherwise as it was. */
export function withBooking(sheet: OpenSheet, booking: ShiftBooking): OpenSheet {
  switch (sheet.kind) {
    case "booking":
    case "templates":
    case "cancelBooking":
    case "move":
      return sheet.booking.id === booking.id ? { kind: sheet.kind, booking } : sheet;
    default:
      return sheet;
  }
}

/** The sheet as `shift` now has what it shows; as it was when the shift has none of it. */
export function refreshedSheet(sheet: OpenSheet, shift: ShiftView): OpenSheet {
  switch (sheet.kind) {
    case "booking":
    case "templates":
    case "cancelBooking":
    case "move": {
      const current = shift.bookings.find((booking) => booking.id === sheet.booking.id);
      return current ? withBooking(sheet, current) : sheet;
    }
    case "table": {
      const current = shift.tables.find((table) => table.id === sheet.table.id);
      return current ? { kind: "table", table: current } : sheet;
    }
    default:
      return sheet;
  }
}
