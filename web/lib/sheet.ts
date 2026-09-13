/**
 * Which sheet is open over the app, and what it shows.
 *
 * A sheet holds a copy of the booking or table it was opened on. The shift keeps changing under it —
 * the half-minute refresh, a colleague's phone, the action just taken — and a copy nobody updates is
 * how an undo restores a status the booking stopped having minutes ago. A copy of something the room
 * no longer has is worse, so that sheet closes.
 *
 * Every opening is numbered. An action that answers late closes the sheet it was started from, and
 * only that one: a sheet closed and opened again meanwhile — even on the same booking — is a new
 * decision somebody is in the middle of, not the one the action finished.
 */

import type { GuestBooking, ShiftBooking, ShiftTable, ShiftView } from "./api";

export type SheetContent =
  | { kind: "booking"; booking: ShiftBooking }
  | { kind: "templates"; booking: ShiftBooking }
  | { kind: "cancelBooking"; booking: ShiftBooking }
  | { kind: "table"; table: ShiftTable }
  | { kind: "conflict"; reasons: string[] }
  | { kind: "days" }
  | { kind: "walkIn" }
  | { kind: "manual" }
  | { kind: "move"; booking: ShiftBooking }
  | { kind: "guestCancel"; booking: GuestBooking };

export type OpenSheet = { kind: "none" } | (SheetContent & { opened: number });

export const NO_SHEET: OpenSheet = { kind: "none" };

/** The sheet with `booking` in place of its copy, if it still shows that booking; otherwise as it was. */
export function withBooking(sheet: OpenSheet, booking: ShiftBooking): OpenSheet {
  switch (sheet.kind) {
    case "booking":
    case "templates":
    case "cancelBooking":
    case "move":
      return sheet.booking.id === booking.id ? { ...sheet, booking } : sheet;
    default:
      return sheet;
  }
}

/**
 * The sheet as `shift` now has what it shows, or no sheet once the shift no longer has it: a sheet
 * left open on a cancelled booking was a way to seat, move or message something that is gone.
 */
export function refreshedSheet(sheet: OpenSheet, shift: ShiftView): OpenSheet {
  switch (sheet.kind) {
    case "booking":
    case "templates":
    case "cancelBooking":
    case "move": {
      const current = shift.bookings.find((booking) => booking.id === sheet.booking.id);
      return current ? withBooking(sheet, current) : NO_SHEET;
    }
    case "table": {
      const current = shift.tables.find((table) => table.id === sheet.table.id);
      return current ? { ...sheet, table: current } : NO_SHEET;
    }
    default:
      return sheet;
  }
}

/** The guest's cancel sheet as their bookings now are, or no sheet once its booking is gone. */
export function refreshedGuestSheet(sheet: OpenSheet, bookings: GuestBooking[]): OpenSheet {
  if (sheet.kind !== "guestCancel") return sheet;
  const current = bookings.find((booking) => booking.id === sheet.booking.id);
  return current ? { ...sheet, booking: current } : NO_SHEET;
}

function openingOf(sheet: OpenSheet): number | null {
  return sheet.kind === "none" ? null : sheet.opened;
}

/** Whether `current` is the very opening `from` was, however its copy has been refreshed since. */
export function isStill(current: OpenSheet, from: OpenSheet): boolean {
  return current.kind === from.kind && openingOf(current) === openingOf(from);
}

/** No sheet, if `current` is still the one `from` was; otherwise `current`, untouched. */
export function closedIfStill(current: OpenSheet, from: OpenSheet): OpenSheet {
  return isStill(current, from) ? NO_SHEET : current;
}
