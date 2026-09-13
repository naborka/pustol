/**
 * Settings that change under a manager's edits.
 *
 * The screen holds three things: the settings as the server last gave them, the proposal those
 * settings make untouched (`base`), and the manager's proposal (`draft`). Whatever arrives — a
 * reread, the reread after a save refused as stale, a save's own answer — is folded in at the moment
 * it arrives, against the draft as it is then, so an edit typed while a read was loading is never
 * the thing that gets lost. Nothing older than what is on screen ever replaces it.
 */

import { draftOf, type SettingsDraft, type SettingsView } from "./api";
import type { IsoDate } from "./format";
import { copyDraft, differs, edited, type Edit } from "./settingsRules";

export interface SettingsPair {
  /** The evening the settings were read for: only the per-table counts depend on it. */
  date: IsoDate;
  settings: SettingsView;
  base: SettingsDraft;
  draft: SettingsDraft;
}

type DraftField = Exclude<keyof SettingsDraft, "version">;

/** Every field of a proposal, in the words a manager knows it by. A new field without words is a type error. */
const FIELD_NAME: Record<DraftField, string> = {
  name: "название",
  address: "адрес",
  contact: "контакт",
  timezone: "часовой пояс",
  week: "часы работы",
  zones: "зоны",
  tables: "столы",
  turn_minutes: "время стола",
  slot_step_minutes: "шаг времени",
  max_party: "размер компании",
  horizon_days: "горизонт брони",
  remind_hours: "напоминание",
  grace_minutes: "ожидание опоздавших",
  message_templates: "сообщения гостю",
  cancel_reasons: "причины отмены",
  staff: "персонал",
};

const FIELDS = Object.keys(FIELD_NAME) as DraftField[];

const STAMP = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d+))?(Z|[+-]\d{2}:\d{2})$/;

function instant(version: string): { ms: number; fraction: string } | null {
  const match = STAMP.exec(version);
  if (!match) return null;
  const ms = Date.parse(`${match[1]}${match[3]}`);
  return Number.isNaN(ms) ? null : { ms, fraction: (match[2] ?? "").replace(/0+$/, "") };
}

function sign(value: number): number {
  return value < 0 ? -1 : value > 0 ? 1 : 0;
}

/**
 * Orders two settings versions. The server sends a UTC timestamp whose fraction runs to whatever
 * digits it has, so neither a string comparison nor `Date.parse` alone, which stops at milliseconds,
 * can tell two quick saves apart.
 */
export function compareVersions(left: string, right: string): number {
  const a = instant(left);
  const b = instant(right);
  if (!a || !b) return left < right ? -1 : left > right ? 1 : 0;
  if (a.ms !== b.ms) return sign(a.ms - b.ms);
  const digits = Math.max(a.fraction.length, b.fraction.length);
  const x = a.fraction.padEnd(digits, "0");
  const y = b.fraction.padEnd(digits, "0");
  return x < y ? -1 : x > y ? 1 : 0;
}

/** The settings as they came, with nothing edited. */
export function fresh(date: IsoDate, settings: SettingsView): SettingsPair {
  return { date, settings, base: draftOf(settings), draft: draftOf(settings) };
}

export function isDirty(pair: SettingsPair): boolean {
  return differs(pair.draft, pair.base);
}

function same(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/**
 * Three-way, one top-level field at a time: a field only I changed is mine, a field only they
 * changed is theirs, and a field we both changed differently is mine and named as a conflict.
 */
export function mergeDrafts(
  base: SettingsDraft,
  mine: SettingsDraft,
  theirs: SettingsDraft,
): { draft: SettingsDraft; conflicts: DraftField[] } {
  const draft = copyDraft(theirs);
  const kept = copyDraft(mine);
  const conflicts: DraftField[] = [];
  for (const field of FIELDS) {
    if (same(mine[field], base[field])) continue;
    Object.assign(draft, { [field]: kept[field] });
    if (!same(theirs[field], base[field]) && !same(mine[field], theirs[field])) {
      conflicts.push(field);
    }
  }
  return { draft, conflicts };
}

/** Settings read from the server, folded into what is on screen, and what to tell the manager. */
export function received(
  current: SettingsPair | null,
  date: IsoDate,
  next: SettingsView,
): { pair: SettingsPair; notice: string | null } {
  if (current === null) return { pair: fresh(date, next), notice: null };
  const newer = compareVersions(next.version, current.settings.version);
  if (newer < 0) return { pair: current, notice: null };
  if (!isDirty(current)) return { pair: fresh(date, next), notice: null };
  if (newer === 0) return { pair: { ...current, date, settings: next }, notice: null };

  const { draft, conflicts } = mergeDrafts(current.base, current.draft, draftOf(next));
  const pair = { date, settings: next, base: draftOf(next), draft };
  if (conflicts.length > 0) {
    const fields = conflicts.map((field) => FIELD_NAME[field]).join(", ");
    return {
      pair,
      notice: `Пока вы редактировали, кто-то изменил настройки: ${fields}. Оставили ваши значения — проверьте и сохраните.`,
    };
  }
  return {
    pair,
    notice: isDirty(pair)
      ? "Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните."
      : null,
  };
}

/**
 * A save's own answer. The edits made while it was on its way are made again on top of what it
 * stored, so a table the save has just given an id is changed rather than added a second time.
 */
export function savedInto(
  current: SettingsPair | null,
  date: IsoDate,
  sent: SettingsDraft,
  stored: SettingsView,
  meanwhile: Edit[],
): SettingsPair {
  if (current === null) return fresh(date, stored);
  if (compareVersions(stored.version, current.settings.version) < 0) return current;
  const untouched = !differs(current.draft, sent);
  return {
    date,
    settings: stored,
    base: draftOf(stored),
    draft: untouched ? draftOf(stored) : meanwhile.reduce(edited, draftOf(stored)),
  };
}
