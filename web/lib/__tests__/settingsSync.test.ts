/**
 * Settings that change under a manager's edits: what reaches the screen, and what is kept.
 */

import { describe, expect, it } from "vitest";

import { draftOf, type SettingsDraft, type SettingsView } from "../api";
import { edited } from "../settingsRules";
import {
  compareVersions,
  fresh,
  isDirty,
  mergeDrafts,
  received,
  savedInto,
  type SettingsPair,
} from "../settingsSync";
import { settingsView } from "@/components/__tests__/fixtures";

const v1 = settingsView({ version: "2026-09-13T08:00:00Z" });
const v2 = settingsView({ version: "2026-09-13T08:00:00.000001Z" });

function editing(settings: SettingsView, change: (draft: SettingsDraft) => void): SettingsPair {
  const pair = fresh(settings);
  return { ...pair, draft: edited(pair.draft, change) };
}

describe("which settings are newer", () => {
  it("reads the server's timestamps to the last digit it sends", () => {
    expect(compareVersions("2026-09-13T08:00:00Z", "2026-09-13T08:00:00.5Z")).toBeLessThan(0);
    expect(compareVersions("2026-09-13T08:00:00.123456Z", "2026-09-13T08:00:00.1235Z")).toBeLessThan(0);
    expect(compareVersions("2026-09-13T08:00:00.10Z", "2026-09-13T08:00:00.1Z")).toBe(0);
    expect(compareVersions("2026-09-13T09:00:00+01:00", "2026-09-13T08:00:00Z")).toBe(0);
    expect(compareVersions("2026-09-14T00:00:00Z", "2026-09-13T23:59:59.999999Z")).toBeGreaterThan(0);
  });

  it("still orders versions that are not timestamps", () => {
    expect(compareVersions("v1", "v2")).toBeLessThan(0);
    expect(compareVersions("v2", "v2")).toBe(0);
  });
});

describe("merging somebody else's save into an edit", () => {
  const base = draftOf(v1);

  it("takes their field where mine is untouched, keeps mine where theirs is, and takes their version", () => {
    const mine = { ...base, name: "Чердак" };
    const theirs = { ...draftOf(v2), address: "Невский, 1" };
    const merged = mergeDrafts(base, mine, theirs);
    expect(merged.draft).toMatchObject({ name: "Чердак", address: "Невский, 1", version: v2.version });
    expect(merged.conflicts).toEqual([]);
  });

  it("keeps mine and names the field when both changed it differently", () => {
    const mine = { ...base, name: "Чердак", turn_minutes: 150 };
    const theirs = { ...draftOf(v2), name: "Подвал", turn_minutes: 150 };
    const merged = mergeDrafts(base, mine, theirs);
    expect(merged.draft.name).toBe("Чердак");
    expect(merged.conflicts).toEqual(["name"]);
  });
});

describe("settings that arrive", () => {
  it("are taken whole when nothing is being edited", () => {
    const next = settingsView({ name: "Подвал", version: v2.version });
    expect(received(null, next)).toEqual({ pair: fresh(next), notice: null });
    expect(received(fresh(v1), next).pair).toEqual(fresh(next));
  });

  it("never replace newer settings with older ones", () => {
    const current = fresh(settingsView({ name: "Подвал", version: v2.version }));
    expect(received(current, v1)).toEqual({ pair: current, notice: null });
  });

  it("of the same version bring another evening's counts without touching the edit", () => {
    const current = editing(v1, (draft) => {
      draft.name = "Чердак";
    });
    const tomorrow = settingsView({
      version: v1.version,
      service_date: "2026-09-12",
      tables: v1.tables.map((table) => ({ ...table, bookings_today: 3 })),
    });
    const { pair, notice } = received(current, tomorrow);
    expect(pair).toEqual({ ...current, settings: tomorrow });
    expect(notice).toBeNull();
  });

  it("merge into an edit, and say the edit was kept", () => {
    const current = editing(v1, (draft) => {
      draft.name = "Мансарда";
    });
    const next = settingsView({ address: "Невский, 1", version: v2.version });
    const { pair, notice } = received(current, next);
    expect(pair.draft).toMatchObject({ name: "Мансарда", address: "Невский, 1", version: v2.version });
    expect(pair.settings).toBe(next);
    expect(notice).toBe("Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.");
  });

  it("name every field both changed", () => {
    const current = editing(v1, (draft) => {
      draft.name = "Мансарда";
      draft.cancel_reasons = ["Потоп"];
    });
    const next = settingsView({ name: "Подвал", cancel_reasons: ["Ремонт"], version: v2.version });
    const { pair, notice } = received(current, next);
    expect(pair.draft).toMatchObject({ name: "Мансарда", cancel_reasons: ["Потоп"] });
    expect(notice).toBe(
      "Пока вы редактировали, кто-то изменил настройки: название, причины отмены. Оставили ваши значения — проверьте и сохраните.",
    );
  });

  it("say nothing when the edit turns out to be what was saved", () => {
    const current = editing(v1, (draft) => {
      draft.name = "Чердак";
    });
    const next = settingsView({ name: "Чердак", version: v2.version });
    const { pair, notice } = received(current, next);
    expect(isDirty(pair)).toBe(false);
    expect(notice).toBeNull();
  });

  it("are measured against the settings last applied, so a save of one's own is nobody else's", () => {
    const table = { id: "4d1e6a0c-7b0f-4c55-8f7e-9f0a1b2c3d4e", seats: 4, zone: "Зал" };
    const adding = (draft: SettingsDraft) => {
      draft.tables.push(table);
    };
    const typing = (draft: SettingsDraft) => {
      draft.address = "Невский, 1";
    };
    const stored = settingsView({
      version: v2.version,
      tables: [...v1.tables, { ...table, number: 9, bookings_today: 0 }],
    });
    const saved = savedInto(editing(v1, adding), stored, []);
    expect(isDirty(saved)).toBe(false);

    const later = settingsView({ ...stored, name: "Подвал", version: "2026-09-13T09:00:00Z" });
    const { pair, notice } = received({ ...saved, draft: edited(saved.draft, typing) }, later);
    expect(pair.draft.tables).toEqual(draftOf(stored).tables);
    expect(pair.draft).toMatchObject({ name: "Подвал", address: "Невский, 1" });
    expect(notice).toBe("Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.");
  });
});

describe("a save that answers", () => {
  it("replaces the edit it stored, and replays on top what was typed while it was on its way", () => {
    const sent = edited(draftOf(v1), (draft) => {
      draft.name = "Чердак";
    });
    const stored = settingsView({ name: "Чердак", version: v2.version });
    const untouched = { ...fresh(v1), draft: sent };
    expect(savedInto(untouched, stored, [])).toEqual(fresh(stored));

    const typing = (draft: SettingsDraft) => {
      draft.address = "Невский, 1";
    };
    const meanwhile = { ...untouched, draft: edited(sent, typing) };
    const pair = savedInto(meanwhile, stored, [typing]);
    expect(pair.draft).toMatchObject({ name: "Чердак", address: "Невский, 1", version: v2.version });
    expect(pair.settings).toBe(stored);
  });

  it("is applied whatever evening its counts are for", () => {
    const stored = settingsView({ name: "Чердак", version: v2.version, service_date: "2026-09-12" });
    expect(savedInto(fresh(v1), stored, [])).toEqual(fresh(stored));
  });

  it("never puts older settings over newer ones that arrived meanwhile", () => {
    const newer = fresh(settingsView({ name: "Подвал", version: "2026-09-13T10:00:00Z" }));
    expect(savedInto(newer, v2, [])).toBe(newer);
  });
});
