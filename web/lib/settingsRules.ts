/**
 * Whether a settings proposal is legal — asked here so a control can grey out the instant it is
 * pressed.
 *
 * This mirrors `BarConfig::validate` in `pustol-domain`, and that is a deliberate second
 * implementation rather than an oversight. The alternatives were each worse:
 *
 * * ask the server on every keystroke — a stepper that greys out 200ms after a tap, on a phone in a
 *   basement, is a stepper people stop trusting;
 * * compile the Rust domain to WebAssembly and call it — one implementation, no latency, but it puts
 *   a wasm toolchain between a contributor and `npm run dev`.
 *
 * What makes the duplication safe is that this copy is *advisory* and the server's is *authoritative*.
 * The server validates every save with the real rules and refuses with named reasons, which this
 * screen renders. So the worst that drift can do is let a control look live and have the save come
 * back refused with an explanation — a degraded moment, never a wrong booking. Nothing here decides
 * anything; it only decides what to grey out.
 */

import type { Bounds, Limits, SettingsDraft } from "./api";

/** A reason a proposal cannot be saved, named so the screen can put it next to the control. */
export type Reason =
  | { kind: "blank_name" }
  | { kind: "blank_address" }
  | { kind: "open_out_of_range"; weekday: number }
  | { kind: "close_out_of_range"; weekday: number }
  | { kind: "every_day_closed" }
  | { kind: "shift_shorter_than_turn"; weekday: number }
  | { kind: "setting_out_of_range"; setting: NumericSetting }
  | { kind: "slot_step_not_offered" }
  | { kind: "no_zones" }
  | { kind: "duplicate_zone"; zone: string }
  | { kind: "no_tables" }
  | { kind: "seats_out_of_range"; index: number }
  | { kind: "unknown_zone"; index: number }
  | { kind: "max_party_exceeds_largest_table"; largest: number }
  | { kind: "no_message_templates" }
  | { kind: "blank_message_template" }
  | { kind: "no_cancel_reasons" }
  | { kind: "blank_cancel_reason" }
  | { kind: "no_staff" }
  | { kind: "malformed_staff_username"; username: string }
  | { kind: "duplicate_staff_username"; username: string };

export type NumericSetting =
  | "turn_minutes"
  | "max_party"
  | "horizon_days"
  | "remind_hours"
  | "grace_minutes";

function within(value: number, bounds: Bounds): boolean {
  return value >= bounds.min && value <= bounds.max;
}

/** Telegram's published rule: five to thirty-two characters, starting with a letter. */
export function isTelegramUsername(candidate: string): boolean {
  return /^[A-Za-z][A-Za-z0-9_]{4,31}$/.test(candidate);
}

export function largestTable(draft: SettingsDraft): number {
  return draft.tables.reduce((largest, table) => Math.max(largest, table.seats), 0);
}

/** Every reason this proposal is illegal. Empty means legal. */
export function reasonsAgainst(draft: SettingsDraft, limits: Limits): Reason[] {
  const reasons: Reason[] = [];

  if (draft.name.trim().length === 0) reasons.push({ kind: "blank_name" });
  if (draft.address.trim().length === 0) reasons.push({ kind: "blank_address" });

  draft.week.forEach((hours, weekday) => {
    if (!within(hours.open_minutes, limits.open_minutes)) {
      reasons.push({ kind: "open_out_of_range", weekday });
    }
    if (!within(hours.close_minutes, limits.close_minutes)) {
      reasons.push({ kind: "close_out_of_range", weekday });
    }
    if (!hours.closed && hours.close_minutes - draft.turn_minutes < hours.open_minutes) {
      reasons.push({ kind: "shift_shorter_than_turn", weekday });
    }
  });
  if (draft.week.every((hours) => hours.closed)) reasons.push({ kind: "every_day_closed" });

  const numeric: [NumericSetting, number, Bounds][] = [
    ["turn_minutes", draft.turn_minutes, limits.turn_minutes],
    ["max_party", draft.max_party, limits.max_party],
    ["horizon_days", draft.horizon_days, limits.horizon_days],
    ["remind_hours", draft.remind_hours, limits.remind_hours],
    ["grace_minutes", draft.grace_minutes, limits.grace_minutes],
  ];
  for (const [setting, value, bounds] of numeric) {
    if (!within(value, bounds)) reasons.push({ kind: "setting_out_of_range", setting });
  }
  if (!limits.slot_step_minutes.includes(draft.slot_step_minutes)) {
    reasons.push({ kind: "slot_step_not_offered" });
  }

  if (draft.zones.length === 0) reasons.push({ kind: "no_zones" });
  draft.zones.forEach((zone, index) => {
    if (draft.zones.slice(0, index).includes(zone)) reasons.push({ kind: "duplicate_zone", zone });
  });

  if (draft.tables.length === 0) reasons.push({ kind: "no_tables" });
  draft.tables.forEach((table, index) => {
    if (!within(table.seats, limits.seats)) reasons.push({ kind: "seats_out_of_range", index });
    if (!draft.zones.includes(table.zone)) reasons.push({ kind: "unknown_zone", index });
  });

  const largest = largestTable(draft);
  if (draft.max_party > largest) {
    reasons.push({ kind: "max_party_exceeds_largest_table", largest });
  }

  if (draft.message_templates.length === 0) reasons.push({ kind: "no_message_templates" });
  if (draft.message_templates.some((text) => text.trim().length === 0)) {
    reasons.push({ kind: "blank_message_template" });
  }
  if (draft.cancel_reasons.length === 0) reasons.push({ kind: "no_cancel_reasons" });
  if (draft.cancel_reasons.some((text) => text.trim().length === 0)) {
    reasons.push({ kind: "blank_cancel_reason" });
  }

  if (draft.staff.length === 0) reasons.push({ kind: "no_staff" });
  draft.staff.forEach((member, index) => {
    if (!isTelegramUsername(member.username)) {
      reasons.push({ kind: "malformed_staff_username", username: member.username });
    }
    if (
      draft.staff
        .slice(0, index)
        .some((other) => other.username.toLowerCase() === member.username.toLowerCase())
    ) {
      reasons.push({ kind: "duplicate_staff_username", username: member.username });
    }
  });

  return reasons;
}

export function isLegal(draft: SettingsDraft, limits: Limits): boolean {
  return reasonsAgainst(draft, limits).length === 0;
}

/**
 * A copy an edit can be tried on without touching the original.
 *
 * `structuredClone` would do the same job and does it about eleven times more slowly, which is
 * noticeable when every control on the screen asks whether its own change would be legal — a
 * settings screen with fifteen tables asks the question dozens of times per render. A proposal is
 * plain JSON by construction (see `SettingsDraft`), so copying it by hand is exact.
 */
export function copyDraft(draft: SettingsDraft): SettingsDraft {
  return {
    ...draft,
    week: draft.week.map((hours) => ({ ...hours })),
    zones: [...draft.zones],
    tables: draft.tables.map((table) => ({ ...table })),
    message_templates: [...draft.message_templates],
    cancel_reasons: [...draft.cancel_reasons],
    staff: draft.staff.map((member) => ({ ...member })),
  };
}

/**
 * Whether a proposed edit would be legal.
 *
 * The one question every control on the settings screen asks. Passing the whole edited proposal —
 * rather than checking the field in isolation — is what makes the cross-field rules work: raising
 * the turn length is refused because of the shortest shift, not because of anything about turns.
 *
 * The mutator has the same shape the screen's own `edit` takes, so one named change can be handed
 * to both: asked about, and then made.
 */
export function wouldBeLegal(
  draft: SettingsDraft,
  change: Edit,
  limits: Limits,
): boolean {
  const next = copyDraft(draft);
  change(next);
  return isLegal(next, limits);
}

/** A change to a proposal, made in place on a copy. */
export type Edit = (draft: SettingsDraft) => void;

/** The latest a party may arrive: closing time less one turn, never stored. */
export function lastArrivalMinutes(draft: SettingsDraft, weekday: number): number | null {
  const hours = draft.week[weekday];
  if (!hours || hours.closed) return null;
  return hours.close_minutes - draft.turn_minutes;
}

/** The shortest open shift of the week, which is what caps the turn length. */
export function shortestShiftMinutes(draft: SettingsDraft): number | null {
  const open = draft.week.filter((hours) => !hours.closed);
  if (open.length === 0) return null;
  return open.reduce(
    (shortest, hours) => Math.min(shortest, hours.close_minutes - hours.open_minutes),
    Number.POSITIVE_INFINITY,
  );
}

/** Whether two proposals differ, which is what makes the Save button live. */
export function differs(left: SettingsDraft, right: SettingsDraft): boolean {
  return JSON.stringify(left) !== JSON.stringify(right);
}
