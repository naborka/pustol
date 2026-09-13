/**
 * Sheet holds copy of booking or table it opened on. Shift keeps changing under it (half-minute
 * refresh, colleague phone, action just taken); stale copy is how undo restores status booking lost
 * minutes ago. Copy of something room no longer has is worse, so that sheet closes.
 *
 * Every opening numbered. Late action answer closes only sheet it started from: sheet closed and
 * reopened meanwhile, even on same booking, is new decision in progress.
 */

import type { GuestBooking, ShiftBooking, ShiftTable, ShiftView } from "./api";
import type { Refusal } from "./outcomes";

export type SheetContent =
  | { kind: "booking"; booking: ShiftBooking }
  | { kind: "templates"; booking: ShiftBooking }
  | { kind: "cancelBooking"; booking: ShiftBooking }
  | { kind: "table"; table: ShiftTable }
  | { kind: "conflict"; refusal: Refusal }
  | { kind: "days" }
  | { kind: "walkIn" }
  | { kind: "manual" }
  | { kind: "move"; booking: ShiftBooking }
  | { kind: "guestCancel"; booking: GuestBooking };

export type OpenSheet = { kind: "none" } | (SheetContent & { opened: number });

export const NO_SHEET: OpenSheet = { kind: "none" };

/** No sheet once `shift` lacks its item: sheet on cancelled booking could seat, move or message it. */
export function refreshedSheet(sheet: OpenSheet, shift: ShiftView): OpenSheet {
  switch (sheet.kind) {
    case "booking":
    case "templates":
    case "cancelBooking":
    case "move": {
      const current = shift.bookings.find((booking) => booking.id === sheet.booking.id);
      return current ? { ...sheet, booking: current } : NO_SHEET;
    }
    case "table": {
      const current = shift.tables.find((table) => table.id === sheet.table.id);
      return current ? { ...sheet, table: current } : NO_SHEET;
    }
    default:
      return sheet;
  }
}

export function refreshedGuestSheet(sheet: OpenSheet, bookings: GuestBooking[]): OpenSheet {
  if (sheet.kind !== "guestCancel") return sheet;
  const current = bookings.find((booking) => booking.id === sheet.booking.id);
  return current ? { ...sheet, booking: current } : NO_SHEET;
}

function openingOf(sheet: OpenSheet): number | null {
  return sheet.kind === "none" ? null : sheet.opened;
}

/** Same opening as `from`, however its copy refreshed since. */
export function isStill(current: OpenSheet, from: OpenSheet): boolean {
  return current.kind === from.kind && openingOf(current) === openingOf(from);
}

export function closedIfStill(current: OpenSheet, from: OpenSheet): OpenSheet {
  return isStill(current, from) ? NO_SHEET : current;
}
