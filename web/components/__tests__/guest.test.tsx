/**
 * The guest's three screens.
 *
 * These check the things a reader of the code cannot check by reading it: that a guest is never
 * shown a table number, that a day with nothing left says so before it is tapped, that a time
 * somebody else has cannot be taken by tapping it, and that the one primary action says the whole
 * decision it is about to make.
 */

import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BookScreen, DayRailStrip, DoneScreen, HomeScreen, bookingDecision } from "../GuestScreens";
import { GuestCancelSheet } from "../Sheets";
import { TAP } from "@/lib/tokens";
import { availability, bar, booking, dayOffer, noop, rail, session } from "./fixtures";

afterEach(cleanup);

function home(overrides: Parameters<typeof session>[0] = {}) {
  return render(
    <HomeScreen
      session={session(overrides)}
      onMove={noop}
      onCancel={noop}
      onEnableReminders={noop}
      onDismissReminders={noop}
      onWriteToBar={noop}
    />,
  );
}

describe("the guest's home screen", () => {
  it("says which bar this is, and whether it is open", () => {
    // The name was in the session payload from the first day and drawn nowhere.
    home();
    expect(screen.getByText("Пустол")).toBeDefined();
    expect(screen.getByText("ул. Рубинштейна, 24")).toBeDefined();
    expect(screen.getByText("Открыт до 02:00")).toBeDefined();
  });

  it("says the bar is shut when it is", () => {
    home({ bar: { ...bar, now_minutes: 600 } });
    expect(screen.getByText("Откроется в 18:00")).toBeDefined();
  });

  it("invites a booking by saying what tonight still has", () => {
    home();
    expect(screen.getByText("Столик на вечер")).toBeDefined();
    expect(
      screen.getByText("Сегодня свободно с 21:30. Подтверждение сразу, без звонка и ожидания."),
    ).toBeDefined();
  });

  it("says so plainly when tonight has nothing", () => {
    home({ today_free_from_minutes: null });
    expect(screen.getByText("Сегодня мест нет — посмотрите завтра.")).toBeDefined();
  });

  it("shows a booking without ever naming a table", () => {
    // The bar assigns tables and moves them when the room changes. A number on a guest's screen is
    // a number they arrive quoting.
    const { container } = home({ booking });
    expect(screen.getByText("Стол ваш")).toBeDefined();
    expect(screen.getByText("Сегодня в 21:30")).toBeDefined();
    expect(screen.getByText("4 гостя")).toBeDefined();
    // A table as a *place* — "стол 7 · Стойка", "стол 7." — never appears. "Держим стол 15 минут"
    // is about a duration, which is why the pattern ends where a place would.
    expect(container.textContent).not.toMatch(/стол\s+\d+\s*(·|,|\.|$)/i);
    expect(container.textContent).not.toMatch(/undefined|null|NaN/);
  });

  it("quotes the grace period the bar actually configured", () => {
    home({ booking, bar: { ...bar, grace_minutes: 25 } });
    expect(screen.getByText(/Держим стол 25 минут после времени брони/)).toBeDefined();
  });

  it("offers to move and to cancel, and hands both back to whoever asked", async () => {
    const onMove = vi.fn();
    const onCancel = vi.fn();
    render(
      <HomeScreen
        session={session({ booking })}
        onMove={onMove}
        onCancel={onCancel}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    await userEvent.click(screen.getByText("Перенести"));
    expect(onMove).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByText("Отменить"));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("asks about reminders exactly once, and never again after «Не нужно»", () => {
    const { unmount } = home({ booking });
    expect(screen.getByText("Напомнить за 3 часа?")).toBeDefined();
    expect(
      screen.getByText(
        "Бот напишет в этот чат. Планы изменятся — отмена одной кнопкой прямо из сообщения.",
      ),
    ).toBeDefined();
    unmount();

    home({ booking, reminders: { opted_in: false, deliverable: true, should_ask: false } });
    expect(screen.queryByText("Напомнить за 3 часа?")).toBeNull();
  });

  it("does not repeat the question on the confirmation screen", () => {
    render(<DoneScreen booking={booking} bar={bar} />);
    expect(screen.getByText("Стол забронирован")).toBeDefined();
    expect(screen.getByText("Сегодня в 21:30 · 4 гостя")).toBeDefined();
    expect(screen.queryByText(/Напомнить за/)).toBeNull();
  });
});

describe("the day rail", () => {
  function railOf(days: Parameters<typeof DayRailStrip>[0]["days"], onServiceDate = noop) {
    return render(
      <DayRailStrip
        days={days}
        today="2026-09-11"
        serviceDate="2026-09-11"
        onServiceDate={onServiceDate}
      />,
    );
  }

  it("is exactly as long as the horizon, whatever the horizon is", () => {
    for (const length of [1, 4, 30]) {
      const { unmount } = railOf(rail(length));
      expect(screen.getAllByRole("button")).toHaveLength(length);
      unmount();
    }
  });

  it("names today and tomorrow, then the weekday with its date", () => {
    railOf(rail(4));
    expect(screen.getByText("Сегодня")).toBeDefined();
    expect(screen.getByText("Завтра")).toBeDefined();
    // Two Sundays in a thirty-day rail must not read the same, so the date is on the chip.
    expect(screen.getByText("вс 13")).toBeDefined();
    expect(screen.getByText("пн 14")).toBeDefined();
  });

  it("says what each day holds before anybody taps it", () => {
    railOf([
      dayOffer(),
      dayOffer({ service_date: "2026-09-12", free_from_minutes: null }),
      dayOffer({ service_date: "2026-09-13", closed: true, free_from_minutes: null }),
    ]);
    expect(screen.getByText("с 21:30")).toBeDefined();
    expect(screen.getByText("мест нет")).toBeDefined();
    expect(screen.getByText("выходной")).toBeDefined();
  });

  it("will not let a guest tap into a day with nothing in it", async () => {
    const onServiceDate = vi.fn();
    railOf(
      [
        dayOffer(),
        dayOffer({ service_date: "2026-09-12", free_from_minutes: null }),
        dayOffer({ service_date: "2026-09-13", closed: true, free_from_minutes: null }),
      ],
      onServiceDate,
    );
    await userEvent.click(screen.getByText("Завтра"));
    await userEvent.click(screen.getByText("вс 13"));
    expect(onServiceDate).not.toHaveBeenCalled();

    await userEvent.click(screen.getByText("Сегодня"));
    expect(onServiceDate).toHaveBeenCalledWith("2026-09-11");
  });
});

describe("the picker", () => {
  function picker(overrides: Partial<Parameters<typeof BookScreen>[0]> = {}) {
    return render(
      <BookScreen
        bar={bar}
        days={rail(4)}
        availability={availability()}
        partySize={2}
        serviceDate="2026-09-11"
        chosenMinutes={null}
        failedToLoad={false}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onTakenSlot={noop}
        onRetry={noop}
        {...overrides}
      />,
    );
  }

  it("offers exactly the party sizes the bar accepts, at a size a thumb can hit", () => {
    picker({ bar: { ...bar, max_party: 10 } });
    const ten = screen.getByRole("button", { name: "10 гостей" });
    expect(ten).toBeDefined();
    expect(screen.queryByRole("button", { name: "11 гостей" })).toBeNull();

    // A row of ten equal shares on a 360px screen gives each chip twenty-six pixels. A grid of six
    // wraps instead, and every cell stays a target.
    const grid = ten.parentElement as HTMLElement;
    expect(grid.style.gridTemplateColumns).toBe("repeat(6, 1fr)");
    expect(Number.parseInt(ten.style.height, 10)).toBeGreaterThanOrEqual(TAP);
  });

  it("does not render a time that has already gone", () => {
    picker();
    expect(screen.queryByText("18:00")).toBeNull();
    expect(screen.getByText("21:30")).toBeDefined();
  });

  it("strikes through a time somebody else has, and says so when it is tapped", async () => {
    const onPick = vi.fn();
    const onTakenSlot = vi.fn();
    picker({ onPick, onTakenSlot });

    const taken = screen.getByText("22:00");
    expect(taken.style.textDecoration).toBe("line-through");
    await userEvent.click(taken);
    expect(onPick).not.toHaveBeenCalled();
    expect(onTakenSlot).toHaveBeenCalledOnce();

    await userEvent.click(screen.getByText("21:30"));
    expect(onPick).toHaveBeenCalledWith(1_290);
  });

  it("says how many windows are free and that each has a real table behind it", () => {
    picker();
    expect(
      screen.getByText(
        "Зачёркнутое время занято. Свободных окон: 2 — за каждым уже стоит настоящий стол на 2 гостя.",
      ),
    ).toBeDefined();
  });

  it("has a loading state, and a failure state with a way out", async () => {
    const onRetry = vi.fn();
    const { rerender } = picker({ days: null, availability: null });
    expect(screen.getAllByRole("status").length).toBeGreaterThan(0);

    rerender(
      <BookScreen
        bar={bar}
        days={null}
        availability={null}
        partySize={2}
        serviceDate="2026-09-11"
        chosenMinutes={null}
        failedToLoad
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onTakenSlot={noop}
        onRetry={onRetry}
      />,
    );
    expect(screen.getAllByText("Попробовать снова")).toHaveLength(2);
    await userEvent.click(screen.getAllByText("Попробовать снова")[0] as HTMLElement);
    expect(onRetry).toHaveBeenCalled();
  });

  it("never shows a guest a table", () => {
    const { container } = picker();
    expect(container.textContent).not.toMatch(/стол \d/i);
    expect(container.textContent).not.toMatch(/undefined|null|NaN/);
  });
});

describe("the main button", () => {
  it("carries the whole decision once a time is chosen", () => {
    expect(bookingDecision(4, "2026-09-11", "2026-09-11", 1_290)).toEqual({
      label: "Забронировать · 4 гостя · сегодня в 21:30",
      enabled: true,
    });
    expect(bookingDecision(2, "2026-09-13", "2026-09-11", 1_320).label).toBe(
      "Забронировать · 2 гостя · вс, 13 сен в 22:00",
    );
  });

  it("asks for the missing half of the decision until it has it", () => {
    expect(bookingDecision(4, "2026-09-11", "2026-09-11", null)).toEqual({
      label: "Выберите время",
      enabled: false,
    });
  });
});

describe("cancelling", () => {
  it("restates the booking and takes a second tap", async () => {
    const onConfirm = vi.fn();
    const onClose = vi.fn();
    render(
      <GuestCancelSheet
        open
        booking={booking}
        today="2026-09-11"
        onClose={onClose}
        onConfirm={onConfirm}
      />,
    );
    const sheet = screen.getByRole("dialog");
    expect(within(sheet).getByText("Отменить бронь?")).toBeDefined();
    expect(within(sheet).getByText("Сегодня в 21:30 · 4 гостя")).toBeDefined();
    expect(
      within(sheet).getByText(
        "Стол сразу уйдёт другим гостям. Вернуть его получится, только если он останется свободен.",
      ),
    ).toBeDefined();

    await userEvent.click(within(sheet).getByText("Оставить"));
    expect(onClose).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();

    await userEvent.click(within(sheet).getByText("Отменить бронь"));
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
