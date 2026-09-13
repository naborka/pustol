/**
 * Settings: an index that answers its own questions, and a save bar that names its refusal.
 */

import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SECTIONS, SaveBar, SettingsScreen, sectionValue, type Section } from "../Settings";
import { draftOf, type SettingsDraft } from "@/lib/api";
import { edited, firstReason } from "@/lib/settingsRules";
import { LIMITS, noop, settingsView, shift, shiftBooking } from "./fixtures";

afterEach(cleanup);

type ScreenProps = Parameters<typeof SettingsScreen>[0];

/** Stands in for page, which holds open section. */
function Settings(props: Omit<ScreenProps, "section" | "onSection">) {
  const [section, setSection] = useState<Section | null>(null);
  return <SettingsScreen {...props} section={section} onSection={setSection} />;
}

function open(overrides: Partial<Omit<ScreenProps, "section" | "onSection">> = {}) {
  const view = settingsView();
  return render(
    <Settings
      settings={view}
      draft={draftOf(view)}
      room={null}
      editedWeekday={5}
      onEdit={noop}
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
    expect(screen.getByText("@marina · @nastya · @pavel")).toBeDefined();
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
      <Settings
        settings={view}
        room={null}
        draft={current}
        editedWeekday={5}
        onEdit={(change) => {
          current = edited(current, change);
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

  it("adds a message as an empty field to fill in, not as words nobody chose", async () => {
    // Placeholder must never become message text: «Новое сообщение» could reach guest.
    const view = settingsView();
    let current: SettingsDraft = draftOf(view);
    const draw = () => (
      <Settings
        settings={view}
        room={null}
        draft={current}
        editedWeekday={5}
        onEdit={(change) => {
          current = edited(current, change);
          rerender(draw());
        }}
        onEditWeekday={noop}
      />
    );
    const { rerender } = render(draw());

    await userEvent.click(screen.getByText("Сообщения и причины отмены"));
    await userEvent.click(screen.getByText("+ Сообщение"));
    expect(current.message_templates.at(-1)).toBe("");
    expect(screen.queryByDisplayValue("Новое сообщение")).toBeNull();
    expect(firstReason(current, LIMITS)).toBe("Пустое сообщение отправить нельзя.");
  });

  it("counts a table's bookings from the evening on screen, and none until that evening is read", async () => {
    const view = settingsView({
      tables: [{ id: "t1", number: 7, seats: 2, zone: "Стойка" }],
      max_party: 2,
    });
    const evening = shift({
      bookings: [
        shiftBooking({ id: "a" }),
        shiftBooking({ id: "b" }),
        shiftBooking({ id: "c" }),
        shiftBooking({ id: "d", table_id: "t2", table_number: 8 }),
        shiftBooking({ id: "e", table_id: null, table_number: null }),
      ],
    });
    const { rerender } = open({ settings: view, draft: draftOf(view), room: evening });
    await userEvent.click(screen.getByText("Зал"));
    expect(screen.getByText("3 брони")).toBeDefined();

    rerender(
      <Settings
        settings={view}
        draft={draftOf(view)}
        room={null}
        editedWeekday={5}
        onEdit={noop}
        onEditWeekday={noop}
      />,
    );
    expect(screen.getByText("Стол 7")).toBeDefined();
    expect(screen.queryByText("3 брони")).toBeNull();
  });

  it("names a table it adds before saving, so a second save of it is the same table", async () => {
    const view = settingsView();
    let current: SettingsDraft = draftOf(view);
    const draw = () => (
      <Settings
        settings={view}
        room={null}
        draft={current}
        editedWeekday={5}
        onEdit={(change) => {
          current = edited(current, change);
          rerender(draw());
        }}
        onEditWeekday={noop}
      />
    );
    const { rerender } = render(draw());

    await userEvent.click(screen.getByText("Зал"));
    await userEvent.click(screen.getByText("+ Добавить стол"));
    const added = current.tables.at(-1);
    expect(added?.id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(added).toMatchObject({ seats: 4, zone: "Зал" });
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

describe("a list at its bound", () => {
  const disabled = (label: string) =>
    (screen.getByText(label).closest("button") as HTMLButtonElement).disabled;

  async function visit(section: string, check: () => Promise<void> | void) {
    await userEvent.click(screen.getByText(section));
    await check();
    await userEvent.click(screen.getByRole("button", { name: "Назад" }));
  }

  it("will not add one more, and adds freely below it", async () => {
    // Fixture: two tables, four messages, four reasons, three staff.
    const full = { ...LIMITS, lists: { zones: 3, tables: 2, message_templates: 4, cancel_reasons: 4, staff: 3 } };
    for (const [limits, blocked] of [
      [full, true],
      [LIMITS, false],
    ] as const) {
      const view = settingsView({ limits });
      const { unmount } = open({ settings: view, draft: draftOf(view) });
      await visit("Зал", () => expect(disabled("+ Добавить стол")).toBe(blocked));
      await visit("Сообщения и причины отмены", () => {
        expect(disabled("+ Сообщение")).toBe(blocked);
        expect(disabled("+ Причина")).toBe(blocked);
      });
      await visit("Персонал", async () => {
        await userEvent.type(screen.getByPlaceholderText("@username"), "@aaron_bar");
        expect(disabled("Добавить")).toBe(blocked);
      });
      unmount();
    }
  });
});

describe("the reason a list cannot be saved", () => {
  it("names the list and its bound", () => {
    const draft = draftOf(settingsView());
    draft.cancel_reasons = Array.from({ length: LIMITS.lists.cancel_reasons + 1 }, (_, index) => `Причина ${index}`);
    expect(firstReason(draft, LIMITS)).toBe("Причин отмены больше 20 быть не может.");
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
        onWhy={null}
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
    render(<SaveBar reason={null} saving={false} onSave={onSave} onRevert={noop} onWhy={null} />);
    await userEvent.click(screen.getByText("Сохранить"));
    expect(onSave).toHaveBeenCalledOnce();
    expect(screen.queryByText("Не сохранено.")).toBeNull();
  });

  it("keeps a refused save's reasons one tap away, and lets it be saved again", async () => {
    const onWhy = vi.fn();
    const onSave = vi.fn();
    render(<SaveBar reason={null} saving={false} onSave={onSave} onRevert={noop} onWhy={onWhy} />);
    expect(screen.getByText("Не сохранено.")).toBeDefined();
    await userEvent.click(screen.getByText("Почему"));
    expect(onWhy).toHaveBeenCalledOnce();
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
