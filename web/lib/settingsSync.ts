/**
 * Settings changing under manager edits.
 *
 * Screen holds server settings and manager proposal (`draft`). Edit measured against untouched
 * proposal of those settings, never side copy, so own save never mistaken for someone else's.
 * Anything arriving (reread, reread after stale save refusal, save answer) folds in on arrival
 * against draft as it is then, so edit typed during load never lost. Read model orders settings by
 * version, so nothing older than screen arrives here.
 */

import { draftOf, type SettingsDraft, type SettingsView } from "./api";
import { copyDraft, differs, edited, same, trimmed, usernameKey, type Edit } from "./settingsRules";

export interface SettingsPair {
  settings: SettingsView;
  draft: SettingsDraft;
}

type DraftField = Exclude<keyof SettingsDraft, "version">;

/** Manager-facing name per field. New field without name is type error. */
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

export function fresh(settings: SettingsView): SettingsPair {
  return { settings, draft: draftOf(settings) };
}

export function isDirty(pair: SettingsPair): boolean {
  return differs(pair.draft, draftOf(pair.settings));
}

/**
 * Proposal as server stores and reads it back: `Draft::resolve` in `pustol-domain`, then storage
 * order. Advisory like `settingsRules`: only decides whether two proposals mean same, so own save
 * trimmed or reordered by server never taken for someone else's.
 *
 * Tables by number: known id keeps number, others take next in proposal order. Staff by username,
 * case-insensitive.
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
        const [a, b] = [usernameKey(left.username), usernameKey(right.username)];
        return a < b ? -1 : a > b ? 1 : 0;
      }),
  };
}

/**
 * Three-way per top-level field, compared as stored. Only mine changed: mine. Only theirs, or both
 * same: theirs. Both differently: mine, named as conflict.
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

/** Same version as screen means same settings, whatever order server lists them in. */
export function received(
  current: SettingsPair | null,
  next: SettingsView,
): { pair: SettingsPair; notice: string | null } {
  if (current === null) return { pair: fresh(next), notice: null };
  if (next.version === current.settings.version) return { pair: current, notice: null };
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

/** Save answer. Edits made while in flight replay on what it stored, server's own form of what was sent. */
export function savedInto(
  current: SettingsPair | null,
  stored: SettingsView,
  meanwhile: Edit[],
): SettingsPair {
  if (current === null) return fresh(stored);
  return { settings: stored, draft: meanwhile.reduce(edited, draftOf(stored)) };
}
