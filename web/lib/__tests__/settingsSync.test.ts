/**
 * Settings that change under a manager's edits: what reaches the screen, and what is kept.
 */

import { describe, expect, it } from "vitest";

import { draftOf, type SettingsDraft, type SettingsView } from "../api";
import { removeStaff, removeTable, resizeTable } from "../settingsEdits";
import { edited } from "../settingsRules";
import {
  asStored,
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

describe("a proposal as the server stores it", () => {
  it("trims what the server trims, lists staff and tables in the server's order, and keeps the rest as typed", () => {
    const draft = edited(draftOf(v1), (next) => {
      next.name = " Чердак ";
      next.address = "Невский, 1 ";
      next.contact = " @podval_bar ";
      next.zones = [" Зал", "Стойка"];
      next.message_templates = [" Ждём вас "];
      next.cancel_reasons = ["Дождь\n"];
      next.staff = [{ username: "pavel" }, { username: "Aaron" }, { username: "marina" }];
      next.tables = [
        { id: "new-1", seats: 2, zone: "Стойка" },
        { id: "t2", seats: 6, zone: "Зал" },
        { id: "t1", seats: 2, zone: "Стойка" },
        { id: "new-2", seats: 4, zone: "Зал" },
      ];
    });
    expect(asStored(draft, v1)).toEqual({
      ...draft,
      name: "Чердак",
      address: "Невский, 1",
      contact: "@podval_bar",
      message_templates: ["Ждём вас"],
      cancel_reasons: ["Дождь"],
      staff: [{ username: "Aaron" }, { username: "marina" }, { username: "pavel" }],
      tables: [
        { id: "t1", seats: 2, zone: "Стойка" },
        { id: "t2", seats: 6, zone: "Зал" },
        { id: "new-1", seats: 2, zone: "Стойка" },
        { id: "new-2", seats: 4, zone: "Зал" },
      ],
    });
  });
});

describe("merging somebody else's save into an edit", () => {
  it("takes their field where mine is untouched, keeps mine where theirs is, and takes their version", () => {
    const mine = { ...draftOf(v1), name: "Чердак" };
    const theirs = settingsView({ address: "Невский, 1", version: v2.version });
    const merged = mergeDrafts(v1, mine, theirs);
    expect(merged.draft).toMatchObject({ name: "Чердак", address: "Невский, 1", version: v2.version });
    expect(merged.conflicts).toEqual([]);
  });

  it("keeps mine and names the field when both changed it differently", () => {
    const mine = { ...draftOf(v1), name: "Чердак", turn_minutes: 150 };
    const theirs = settingsView({ name: "Подвал", turn_minutes: 150, version: v2.version });
    const merged = mergeDrafts(v1, mine, theirs);
    expect(merged.draft.name).toBe("Чердак");
    expect(merged.conflicts).toEqual(["name"]);
  });

  it("takes theirs, naming no conflict, for a field that is mine once the server has stored it", () => {
    const mine = {
      ...draftOf(v1),
      name: "Чердак ",
      message_templates: v1.message_templates.map((text) => ` ${text}`),
      staff: [...v1.staff].reverse().map(({ username }) => ({ username })),
    };
    const theirs = settingsView({ name: "Чердак", version: v2.version });
    const merged = mergeDrafts(v1, mine, theirs);
    expect(merged.draft).toEqual(draftOf(theirs));
    expect(merged.conflicts).toEqual([]);
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

  it("of the same version change nothing but the evening's counts, whatever order they list the staff in", () => {
    // A save answered in the order it was sent while reads listed the staff sorted: the reread made
    // the manager's untouched roster look like an edit, and the next save put back a member somebody
    // else had removed.
    const shown = settingsView({
      version: v2.version,
      staff: [
        { username: "nastya", bound: true },
        { username: "pavel", bound: false },
        { username: "aaron", bound: false },
      ],
    });
    const reread = settingsView({
      version: v2.version,
      service_date: "2026-09-12",
      staff: [...shown.staff].sort((left, right) => left.username.localeCompare(right.username)),
      tables: shown.tables.map((table) => ({ ...table, bookings_today: 3 })),
    });
    for (const current of [fresh(shown), editing(shown, (draft) => void (draft.name = "Мансарда"))]) {
      const { pair, notice } = received(current, reread);
      expect(pair.draft).toBe(current.draft);
      expect(pair.settings).toEqual({ ...shown, service_date: "2026-09-12", tables: reread.tables });
      expect(notice).toBeNull();
    }
  });

  it("never bring back a member somebody else removed because the edit lists the staff in another order", () => {
    const sorted = settingsView({
      version: v2.version,
      staff: [
        { username: "aaron", bound: false },
        { username: "nastya", bound: true },
        { username: "pavel", bound: false },
      ],
    });
    const current: SettingsPair = {
      settings: sorted,
      draft: {
        ...draftOf(sorted),
        name: "Мансарда",
        staff: [{ username: "nastya" }, { username: "pavel" }, { username: "aaron" }],
      },
    };
    const removed = settingsView({
      version: "2026-09-13T09:00:00Z",
      staff: sorted.staff.filter((member) => member.username !== "pavel"),
    });
    const { pair, notice } = received(current, removed);
    expect(pair.draft.staff).toEqual([{ username: "aaron" }, { username: "nastya" }]);
    expect(pair.draft.name).toBe("Мансарда");
    expect(notice).toBe("Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.");
  });

  it("take a save of one's own the server trimmed as saved, with nothing left to save", () => {
    // The save committed and its answer was lost: the reread holds the name as the server trimmed it.
    const current = editing(v1, (draft) => {
      draft.name = "Чердак ";
    });
    const { pair, notice } = received(current, settingsView({ name: "Чердак", version: v2.version }));
    expect(pair.draft.name).toBe("Чердак");
    expect(isDirty(pair)).toBe(false);
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

  it("makes an edit typed while it was on its way on the item that edit named, whatever order it lists them in", () => {
    // The server lists staff by username and tables by number: a position taken before the save
    // named somebody else after it.
    const sent: SettingsDraft = {
      ...draftOf(v1),
      staff: [{ username: "marina" }, { username: "nastya" }, { username: "pavel" }, { username: "aaron_bar" }],
      tables: [...draftOf(v1).tables].reverse(),
    };
    const stored = settingsView({
      version: v2.version,
      staff: [
        { username: "aaron_bar", bound: false },
        { username: "marina", bound: false },
        { username: "nastya", bound: true },
        { username: "pavel", bound: false },
      ],
    });
    const pair = savedInto({ settings: v1, draft: sent }, stored, [
      removeStaff("pavel"),
      resizeTable("t2", -1),
      removeTable("t1"),
    ]);
    expect(pair.draft.staff).toEqual([{ username: "aaron_bar" }, { username: "marina" }, { username: "nastya" }]);
    expect(pair.draft.tables).toEqual([{ id: "t2", seats: 5, zone: "Зал" }]);
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
