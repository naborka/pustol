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
  const pair = fresh("2026-09-11", settings);
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
    expect(received(null, "2026-09-11", next)).toEqual({ pair: fresh("2026-09-11", next), notice: null });
    expect(received(fresh("2026-09-11", v1), "2026-09-11", next).pair).toEqual(
      fresh("2026-09-11", next),
    );
  });

  it("never replace newer settings with older ones", () => {
    const current = fresh("2026-09-11", settingsView({ name: "Подвал", version: v2.version }));
    expect(received(current, "2026-09-11", v1)).toEqual({ pair: current, notice: null });
  });

  it("of the same version bring another day's counts without touching the edit", () => {
    const current = editing(v1, (draft) => {
      draft.name = "Чердак";
    });
    const tomorrow = settingsView({
      version: v1.version,
      tables: v1.tables.map((table) => ({ ...table, bookings_today: 3 })),
    });
    const { pair, notice } = received(current, "2026-09-12", tomorrow);
    expect(pair).toEqual({ ...current, date: "2026-09-12", settings: tomorrow });
    expect(notice).toBeNull();
  });

  it("merge into an edit, and say the edit was kept", () => {
    const current = editing(v1, (draft) => {
      draft.name = "Мансарда";
    });
    const next = settingsView({ address: "Невский, 1", version: v2.version });
    const { pair, notice } = received(current, "2026-09-11", next);
    expect(pair.draft).toMatchObject({ name: "Мансарда", address: "Невский, 1", version: v2.version });
    expect(pair.base).toEqual(draftOf(next));
    expect(pair.settings).toBe(next);
    expect(notice).toBe("Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.");
  });

  it("name every field both changed", () => {
    const current = editing(v1, (draft) => {
      draft.name = "Мансарда";
      draft.cancel_reasons = ["Потоп"];
    });
    const next = settingsView({ name: "Подвал", cancel_reasons: ["Ремонт"], version: v2.version });
    const { pair, notice } = received(current, "2026-09-11", next);
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
    const { pair, notice } = received(current, "2026-09-11", next);
    expect(isDirty(pair)).toBe(false);
    expect(notice).toBeNull();
  });
});

describe("a save that answers", () => {
  it("replaces the edit it stored, and replays on top what was typed while it was on its way", () => {
    const sent = edited(draftOf(v1), (draft) => {
      draft.name = "Чердак";
    });
    const stored = settingsView({ name: "Чердак", version: v2.version });
    const untouched = { ...fresh("2026-09-11", v1), draft: sent };
    expect(savedInto(untouched, "2026-09-11", sent, stored, [])).toEqual(fresh("2026-09-11", stored));

    const typing = (draft: SettingsDraft) => {
      draft.address = "Невский, 1";
    };
    const meanwhile = { ...untouched, draft: edited(sent, typing) };
    const pair = savedInto(meanwhile, "2026-09-11", sent, stored, [typing]);
    expect(pair.draft).toMatchObject({ name: "Чердак", address: "Невский, 1", version: v2.version });
    expect(pair.base).toEqual(draftOf(stored));
  });

  it("never puts older settings over newer ones that arrived meanwhile", () => {
    const newer = fresh("2026-09-11", settingsView({ name: "Подвал", version: "2026-09-13T10:00:00Z" }));
    const sent = draftOf(v1);
    expect(savedInto(newer, "2026-09-11", sent, v2, [])).toBe(newer);
  });
});
