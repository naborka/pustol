/**
 * Settings: an index that answers its own questions, and a save bar that names its refusal.
 */

import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SECTIONS, SaveBar, SettingsScreen, sectionValue } from "../Settings";
import { draftOf, type SettingsDraft } from "@/lib/api";
import { firstReason } from "@/lib/settingsRules";
import { LIMITS, noop, settingsView } from "./fixtures";

afterEach(cleanup);

function open(overrides: Partial<Parameters<typeof SettingsScreen>[0]> = {}) {
  const view = settingsView();
  return render(
    <SettingsScreen
      settings={view}
      draft={draftOf(view)}
      editedWeekday={5}
      onDraft={noop}
      onEditWeekday={noop}
      {...overrides}
    />,
  );
}

describe("the index", () => {
  it("has six rows and each one answers its own question", () => {
    open();
    expect(SECTIONS).toHaveLength(6);
    expect(screen.getByText("Пустол · ул. Рубинштейна, 24")).toBeDefined();
    expect(screen.getByText("2 стола · Зал, Стойка, Веранда")).toBeDefined();
    expect(screen.getByText("пт 18:00 — 02:00")).toBeDefined();
    expect(screen.getByText("бронь 2 ч · до 6 гостей · шаг 30 мин")).toBeDefined();
    expect(
      screen.getByText("4 и 4 — персонал выбирает только из них"),
    ).toBeDefined();
    expect(screen.getByText("@nastya · @pavel · @marina")).toBeDefined();
  });

  it("says a day off is a day off", () => {
    const view = settingsView();
    const draft = draftOf(view);
    const friday = draft.week[5];
    if (friday) friday.closed = true;
    expect(sectionValue("hours", draft, 5)).toBe("пт выходной");
  });

  it("opens one section at a time, and comes back", async () => {
    open();
    expect(screen.queryByPlaceholderText("Название")).toBeNull();

    await userEvent.click(screen.getByText("Бар"));
    expect(screen.getByPlaceholderText("Название")).toBeDefined();
    expect(screen.queryByText("Персонал")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Назад" }));
    expect(screen.getByText("Персонал")).toBeDefined();
    expect(screen.queryByPlaceholderText("Название")).toBeNull();
  });

  it("keeps one draft across sections", async () => {
    const view = settingsView();
    let current: SettingsDraft = draftOf(view);
    const draw = () => (
      <SettingsScreen
        settings={view}
        draft={current}
        editedWeekday={5}
        onDraft={(next) => {
          current = next;
          rerender(draw());
        }}
        onEditWeekday={noop}
      />
    );
    const { rerender } = render(draw());

    await userEvent.click(screen.getByText("Бар"));
    const name = screen.getByPlaceholderText("Название");
    await userEvent.clear(name);
    await userEvent.type(name, "Новый");
    expect(current.name).toBe("Новый");

    await userEvent.click(screen.getByRole("button", { name: "Назад" }));
    expect(screen.getByText("Новый · ул. Рубинштейна, 24")).toBeDefined();
  });

  it("reaches every section it advertises", async () => {
    for (const item of SECTIONS) {
      const { unmount } = open();
      await userEvent.click(screen.getByText(item.label));
      expect(screen.getByRole("button", { name: "Назад" }), item.id).toBeDefined();
      unmount();
    }
  });
});

describe("the save bar", () => {
  it("names the first reason and keeps the button inert until it is fixed", async () => {
    const onSave = vi.fn();
    const onRevert = vi.fn();
    render(
      <SaveBar
        reason="Бронь 3 ч не помещается в самую короткую смену — 2 ч."
        saving={false}
        onSave={onSave}
        onRevert={onRevert}
      />,
    );
    expect(
      screen.getByText(
        "Так сохранить нельзя. Бронь 3 ч не помещается в самую короткую смену — 2 ч.",
      ),
    ).toBeDefined();
    expect(screen.getByText("Сохранить").closest("button")?.disabled).toBe(true);
    await userEvent.click(screen.getByText("Сохранить"));
    expect(onSave).not.toHaveBeenCalled();

    await userEvent.click(screen.getByText("Вернуть"));
    expect(onRevert).toHaveBeenCalledOnce();
  });

  it("saves once the proposal is legal", async () => {
    const onSave = vi.fn();
    render(<SaveBar reason={null} saving={false} onSave={onSave} onRevert={noop} />);
    await userEvent.click(screen.getByText("Сохранить"));
    expect(onSave).toHaveBeenCalledOnce();
  });
});

describe("the reason a save is refused", () => {
  function draftWith(change: (draft: SettingsDraft) => void): SettingsDraft {
    const draft = draftOf(settingsView());
    change(draft);
    return draft;
  }

  it("names the turn that will not fit the shortest shift", () => {
    const draft = draftWith((next) => {
      next.turn_minutes = 180;
      const friday = next.week[5];
      if (friday) {
        friday.open_minutes = 1_080;
        friday.close_minutes = 1_200;
      }
    });
    expect(firstReason(draft, LIMITS)).toBe(
      "Бронь 3 ч не помещается в самую короткую смену — 2 ч.",
    );
  });

  it("names the largest table when the cap outgrows it", () => {
    const draft = draftWith((next) => {
      next.max_party = 8;
    });
    expect(firstReason(draft, LIMITS)).toBe(
      "Компания до 8 не поместится: самый большой стол на 6.",
    );
  });

  it("explains why the last member of staff cannot go", () => {
    const draft = draftWith((next) => {
      next.staff = [];
    });
    expect(firstReason(draft, LIMITS)).toBe(
      "Последнего из списка убрать нельзя — иначе никто не войдёт.",
    );
  });

  it("says nothing at all about a legal proposal", () => {
    expect(firstReason(draftOf(settingsView()), LIMITS)).toBeNull();
  });
});
