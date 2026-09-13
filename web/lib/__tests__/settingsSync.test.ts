/** Settings changing under manager edits: what reaches screen, what is kept. */

import { describe, expect, it } from "vitest";

import { draftOf, type SettingsDraft, type SettingsView } from "../api";
import { removeStaff, removeTable, resizeTable } from "../settingsEdits";
import { edited } from "../settingsRules";
import { asStored, fresh, isDirty, mergeDrafts, received, savedInto, type SettingsPair } from "../settingsSync";
import { settingsView } from "@/components/__tests__/fixtures";

const v1 = settingsView({ version: 1 });
const v2 = settingsView({ version: 2 });

function editing(settings: SettingsView, change: (draft: SettingsDraft) => void): SettingsPair {
  const pair = fresh(settings);
  return { ...pair, draft: edited(pair.draft, change) };
}

describe("a proposal as the server stores it", () => {
  it("trims what the server trims, lists staff and tables in the server's order, and keeps the rest as typed", () => {
    const draft = edited(draftOf(v1), (next) => {
      next.name = " Чердак ";
      next.address = "Невский, 1 ";
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

  it("of the same version change nothing, whatever order they list the staff in", () => {
    // Save answers in sent order, reads sort staff; taking reread must not make untouched roster look edited.
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
      staff: [...shown.staff].sort((left, right) => left.username.localeCompare(right.username)),
    });
    for (const current of [fresh(shown), editing(shown, (draft) => void (draft.name = "Мансарда"))]) {
      expect(received(current, reread)).toEqual({ pair: current, notice: null });
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
      version: 3,
      staff: sorted.staff.filter((member) => member.username !== "pavel"),
    });
    const { pair, notice } = received(current, removed);
    expect(pair.draft.staff).toEqual([{ username: "aaron" }, { username: "nastya" }]);
    expect(pair.draft.name).toBe("Мансарда");
    expect(notice).toBe("Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.");
  });

  it("take a save of one's own the server trimmed as saved, with nothing left to save", () => {
    // Save committed, answer lost: reread holds name as server trimmed it.
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
      tables: [...v1.tables, { ...table, number: 9 }],
    });
    const saved = savedInto(editing(v1, adding), stored, []);
    expect(isDirty(saved)).toBe(false);

    const later = settingsView({ ...stored, name: "Подвал", version: 3 });
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
    // Server sorts staff by username, tables by number: index from before save names someone else after.
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
});
