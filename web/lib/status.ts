/**
 * What a booking is doing, in one place.
 *
 * Before this, the same booking was `ждём` on the list, `Ждём` in the sheet, a blue block on the
 * timeline and `без стола` on a badge, and each of those was written where it was drawn. Four
 * vocabularies for one fact is how a bartender ends up asking which screen is telling the truth.
 *
 * So: one function decides what a booking's standing is, one names it in Russian, one colours it.
 * Every surface reads all three.
 */

import type { ShiftBooking } from "./api";
import { time } from "./format";
import { occupancyEnd } from "./occupancy";

/**
 * Where a booking stands right now.
 *
 * Lateness is derived, never stored: a party is late because the clock has passed their time plus
 * the bar's grace, and a stored flag would be wrong for the whole minute between the two.
 */
export type Standing =
  | { kind: "waiting" }
  | { kind: "late"; minutes: number }
  | { kind: "seated" }
  | { kind: "left"; at: number }
  | { kind: "no_show"; at: number }
  | { kind: "cancelled" };

/**
 * The standing of a booking at `nowMinutes`, or its plain status when the shift is not running.
 *
 * `nowMinutes` is null on any evening but tonight, and nothing on a future evening is late.
 */
export function standingOf(
  booking: Pick<ShiftBooking, "status" | "start_minutes" | "end_minutes" | "released_minutes">,
  nowMinutes: number | null,
  graceMinutes: number,
): Standing {
  switch (booking.status) {
    case "arrived":
      return { kind: "seated" };
    case "left":
      return { kind: "left", at: occupancyEnd(booking) };
    case "no_show":
      return { kind: "no_show", at: occupancyEnd(booking) };
    case "cancelled":
      return { kind: "cancelled" };
    case "confirmed": {
      const late = nowMinutes === null ? 0 : nowMinutes - (booking.start_minutes + graceMinutes);
      return late > 0 ? { kind: "late", minutes: late } : { kind: "waiting" };
    }
  }
}

/** The words on screen. Russian, and the same words wherever this standing is shown. */
export function statusLabel(standing: Standing): string {
  switch (standing.kind) {
    case "waiting":
      return "Ждём";
    case "late":
      return `Опаздывает ${standing.minutes} мин`;
    case "seated":
      return "За столом";
    case "left":
      return `Ушли в ${time(standing.at)} · стол свободен`;
    case "no_show":
      return `Не пришли в ${time(standing.at)} · стол свободен`;
    case "cancelled":
      return "Отменена";
  }
}

/** The short form, for a chip with no room for a sentence. */
export function statusWord(standing: Standing): string {
  switch (standing.kind) {
    case "waiting":
      return "Ждём";
    case "late":
      return `Опаздывает ${standing.minutes} мин`;
    case "seated":
      return "За столом";
    case "left":
      return "Ушли";
    case "no_show":
      return "Не пришли";
    case "cancelled":
      return "Отменена";
  }
}

/** The colour, as a custom property the theme fills in. */
export function statusColor(standing: Standing): string {
  switch (standing.kind) {
    case "waiting":
      return "var(--btn)";
    case "late":
      return "var(--dest)";
    case "seated":
      return "var(--ok)";
    case "left":
    case "no_show":
    case "cancelled":
      return "var(--hint)";
  }
}

/** The wash behind a block on the timeline, in the same colour the label is in. */
export function statusWash(standing: Standing): string {
  switch (standing.kind) {
    case "waiting":
      return "rgba(82,136,193,.22)";
    case "late":
      return "var(--tint)";
    case "seated":
      return "rgba(66,199,103,.20)";
    case "left":
    case "no_show":
    case "cancelled":
      return "rgba(128,128,128,.16)";
  }
}

/** Whether this booking's evening is over — the `Закрыто` group on the shift list. */
export function isSettled(standing: Standing): boolean {
  return standing.kind === "left" || standing.kind === "no_show" || standing.kind === "cancelled";
}

/** Which group on the shift list a booking belongs to, and in what order the groups read. */
export type ShiftGroup = "orphan" | "waiting" | "seated" | "settled";

export const GROUP_ORDER: ShiftGroup[] = ["orphan", "waiting", "seated", "settled"];

export const GROUP_TITLE: Record<ShiftGroup, string> = {
  orphan: "Без стола",
  waiting: "Ждём",
  seated: "За столом",
  settled: "Закрыто",
};

/**
 * The group a booking belongs to.
 *
 * A party with no table comes first however far off their time is: they are the one thing on the
 * shift that somebody has to do something about.
 */
export function groupOf(booking: ShiftBooking, standing: Standing): ShiftGroup {
  if (isSettled(standing)) return "settled";
  if (booking.table_id === null) return "orphan";
  return standing.kind === "seated" ? "seated" : "waiting";
}
