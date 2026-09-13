/**
 * Settings that change under a manager's edits.
 *
 * The screen holds two things: the settings as the server last gave them, and the manager's
 * proposal (`draft`). An edit is measured against the proposal those settings make untouched, never
 * against a copy kept beside them, so the manager's own save is never mistaken for somebody else's.
 * Whatever arrives — a reread, the reread after a save refused as stale, a save's own answer — is
 * folded in at the moment it arrives, against the draft as it is then, so an edit typed while a read
 * was loading is never the thing that gets lost. Nothing older than what is on screen replaces it.
 */

import { draftOf, type SettingsDraft, type SettingsView } from "./api";
import { copyDraft, differs, edited, trimmed, type Edit } from "./settingsRules";

export interface SettingsPair {
  settings: SettingsView;
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
export function fresh(settings: SettingsView): SettingsPair {
  return { settings, draft: draftOf(settings) };
}

export function isDirty(pair: SettingsPair): boolean {
  return differs(pair.draft, draftOf(pair.settings));
}

function same(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function usernameKey(member: { username: string }): string {
  return member.username.toLowerCase();
}

/**
 * A proposal as the server stores it and reads it back: `Draft::resolve` in `pustol-domain`, then
 * the order storage lists it in. Advisory, like `settingsRules`: it only decides whether two
 * proposals mean the same, so the manager's own save, trimmed or reordered by the server, is never
 * taken for somebody else's.
 *
 * Tables go by number: one these settings have keeps its number, any other takes the next in the
 * order the proposal lists it. Staff go by username, whatever its case.
 */
export function asStored(draft: SettingsDraft, settings: SettingsView): SettingsDraft {
  let next = settings.next_table_number;
  const numbered = draft.tables.map((table) => ({
    table,
    number: settings.tables.find((known) => known.id === table.id)?.number ?? next++,
  }));
  return {
    ...copyDraft(draft),
    name: trimmed(draft.name),
    address: trimmed(draft.address),
    contact: trimmed(draft.contact),
    tables: numbered
      .sort((left, right) => left.number - right.number)
      .map(({ table }) => ({ ...table })),
    message_templates: draft.message_templates.map((text) => trimmed(text)),
    cancel_reasons: draft.cancel_reasons.map((text) => trimmed(text)),
    staff: draft.staff
      .map((member) => ({ ...member }))
      .sort((left, right) => {
        const [a, b] = [usernameKey(left), usernameKey(right)];
        return a < b ? -1 : a > b ? 1 : 0;
      }),
  };
}

/**
 * Three-way, one top-level field at a time, each compared as the server stores it: a field only I
 * changed is mine, a field only they changed, or one we both made the same, is theirs, and a field
 * we both changed differently is mine and named as a conflict.
 */
export function mergeDrafts(
  shown: SettingsView,
  mine: SettingsDraft,
  next: SettingsView,
): { draft: SettingsDraft; conflicts: DraftField[] } {
  const theirs = draftOf(next);
  const base = asStored(draftOf(shown), shown);
  const mineStored = asStored(mine, shown);
  const theirsStored = asStored(theirs, next);
  const draft = copyDraft(theirs);
  const kept = copyDraft(mine);
  const conflicts: DraftField[] = [];
  for (const field of FIELDS) {
    if (same(mineStored[field], base[field]) || same(mineStored[field], theirsStored[field])) {
      continue;
    }
    Object.assign(draft, { [field]: kept[field] });
    if (!same(theirsStored[field], base[field])) conflicts.push(field);
  }
  return { draft, conflicts };
}

/**
 * Settings of one version are the same settings whichever evening they were read for, and now the
 * server lists them in one order: only the evening and its per-table counts are news.
 */
function withEveningOf(shown: SettingsView, next: SettingsView): SettingsView {
  return {
    ...shown,
    service_date: next.service_date,
    tables: shown.tables.map((table) => ({
      ...table,
      bookings_today: next.tables.find((each) => each.id === table.id)?.bookings_today ?? 0,
    })),
  };
}

/** Settings read from the server, folded into what is on screen, and what to tell the manager. */
export function received(
  current: SettingsPair | null,
  next: SettingsView,
): { pair: SettingsPair; notice: string | null } {
  if (current === null) return { pair: fresh(next), notice: null };
  const newer = compareVersions(next.version, current.settings.version);
  if (newer < 0) return { pair: current, notice: null };
  if (newer === 0) {
    return { pair: { ...current, settings: withEveningOf(current.settings, next) }, notice: null };
  }
  if (!isDirty(current)) return { pair: fresh(next), notice: null };

  const { draft, conflicts } = mergeDrafts(current.settings, current.draft, next);
  const pair = { settings: next, draft };
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
 * A save's own answer, whatever evening is on screen now: the settings are the bar's, and the counts
 * say which evening they are for. The edits made while it was on its way are made again on top of
 * what it stored, which is the server's own wording of what was sent.
 */
export function savedInto(
  current: SettingsPair | null,
  stored: SettingsView,
  meanwhile: Edit[],
): SettingsPair {
  if (current === null) return fresh(stored);
  if (compareVersions(stored.version, current.settings.version) < 0) return current;
  return { settings: stored, draft: meanwhile.reduce(edited, draftOf(stored)) };
}
