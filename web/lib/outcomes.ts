/**
 * What the app says after it has done something, and what the way back is.
 *
 * These were closures inside the page, which meant the two things most worth pinning down — that a
 * report names every booking it moved *and* every one it could not, and that an undo goes back to
 * the status the booking actually had — could not be tested at all.
 */

import type { Attendance, Reconciliation, ShiftBooking } from "./api";
import { time } from "./format";
import type { StrandedBooking } from "./errors";

/** Something the app just did, and — where the act is reversible — the way back. */
export interface Outcome {
  text: string;
  undo?: { label: string; run: () => void };
}

/**
 * What a rearrangement of the room actually did, per booking, by name.
 *
 * Never a count of what it hoped to do: "пересадили 2 брони" leaves staff to work out which two,
 * and says nothing at all about the third it could not place. The two halves are worded separately
 * because they are read at different moments — one is good news, the other is a job still to do.
 */
export function reconciliationReport(
  outcome: Reconciliation,
  orphanLead = "Всё ещё без стола",
): string {
  const parts: string[] = [];
  if (outcome.moved.length > 0) {
    parts.push(
      `Пересажены: ${outcome.moved
        .map((entry) => `${entry.guest_name} → стол ${entry.to_number}`)
        .join(", ")}.`,
    );
  }
  if (outcome.orphaned.length > 0) {
    const names = outcome.orphaned.map((entry) => entry.guest_name).join(", ");
    parts.push(
      `${orphanLead}: ${names} — свободных столов на это время нет. Откройте закрытый стол или предложите другое время.`,
    );
  }
  return parts.join(" ");
}

/** The status an attendance change goes back to, so undo restores what was there rather than a guess. */
export function previousAttendance(booking: ShiftBooking): Attendance {
  switch (booking.status) {
    case "arrived":
    case "left":
      return "arrived";
    case "cancelled":
    case "confirmed":
    case "no_show":
      return "confirmed";
  }
}

/**
 * What to say after seating a party, marking them gone, or marking them absent.
 *
 * A booking with no table has no table number to name, and a sentence reading "за столом null" is
 * how a screen tells a bartender it has lost the plot. There is no such state to report, because
 * the screen does not offer the action — but the wording refuses to invent one either way.
 */
export function attendanceOutcome(updated: ShiftBooking, attendance: Attendance): string {
  const table = updated.table_number;
  switch (attendance) {
    case "arrived":
      return table === null
        ? `${updated.guest_name} за столом.`
        : `${updated.guest_name} за столом ${table}.`;
    case "left":
      return table === null ? "Стол свободен." : `Стол ${table} свободен.`;
    case "no_show":
      return `${updated.guest_name}: не пришли, стол свободен.`;
    case "confirmed":
      return `${updated.guest_name}: снова ждём.`;
  }
}

/** The bookings a refused settings save would have stranded, each named with its time. */
export function strandedLines(stranded: StrandedBooking[]): string[] {
  return stranded.map((booking) =>
    booking.startMinutes === null
      ? booking.guestName
      : `${booking.guestName} · ${time(booking.startMinutes)}`,
  );
}
