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

import {
  BookScreen,
  DayRailStrip,
  DoneScreen,
  HomeScreen,
  bookingDecision,
  heldAfter,
  heldOn,
  pickerStart,
} from "../GuestScreens";
import { GuestCancelSheet } from "../Sheets";
import type { GuestBooking } from "@/lib/api";
import { TAP } from "@/lib/tokens";
import { availability, bar, booking, dayOffer, noop, rail, seated, session } from "./fixtures";

afterEach(cleanup);

/** A no-show whose table is still held tonight: only a booking tonight replaces it. */
const heldNoShow: GuestBooking = {
  ...booking,
  id: "b9",
  start_minutes: 1_260,
  end_minutes: 1_380,
  status: "no_show",
  started: true,
  rebooking_replaces: "same_evening",
};

const friday: GuestBooking = { ...booking, id: "b2", service_date: "2026-09-12" };

function home(overrides: Parameters<typeof session>[0] = {}) {
  return render(
    <HomeScreen
      session={session(overrides)}
      onMove={noop}
      onCancel={noop}
      onEnableReminders={noop}
      onDismissReminders={noop}
      onContact={noop}
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
    const { container } = home({ bookings: [booking] });
    expect(screen.getByText("Стол ваш")).toBeDefined();
    expect(screen.getByText("Сегодня в 21:30")).toBeDefined();
    expect(screen.getByText("4 гостя")).toBeDefined();
    expect(screen.queryByText("Столик на вечер")).toBeNull();
    // A table as a *place* — "стол 7 · Стойка", "стол 7." — never appears. "Держим стол 15 минут"
    // is about a duration, which is why the pattern ends where a place would.
    expect(container.textContent).not.toMatch(/стол\s+\d+\s*(·|,|\.|$)/i);
    expect(container.textContent).not.toMatch(/undefined|null|NaN/);
  });

  it("quotes the grace period the bar actually configured", () => {
    home({ bookings: [booking], bar: { ...bar, grace_minutes: 25 } });
    expect(screen.getByText(/Держим стол 25 минут после времени брони/)).toBeDefined();
  });

  it("offers to move and to cancel, and hands back which booking", async () => {
    const onMove = vi.fn();
    const onCancel = vi.fn();
    render(
      <HomeScreen
        session={session({ bookings: [booking] })}
        onMove={onMove}
        onCancel={onCancel}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onContact={noop}
      />,
    );
    await userEvent.click(screen.getByText("Перенести"));
    expect(onMove).toHaveBeenCalledWith(booking);
    await userEvent.click(screen.getByText("Отменить"));
    expect(onCancel).toHaveBeenCalledWith(booking);
  });

  it("offers a move exactly when the server says a new booking would replace this one, and always a cancel", () => {
    // The server says what a new booking would do to each, by its own clock and its own slot grid;
    // the card never works it out again. A held no-show with no arrival time left tonight is `null`.
    const cases: [string, GuestBooking, typeof bar, boolean][] = [
      ["a plan not yet begun", booking, bar, true],
      ["a no-show whose table is still held", heldNoShow, bar, true],
      ["a held no-show after the last arrival, even by a clock that disagrees", heldNoShow, { ...bar, now_minutes: 1_440 }, true],
      ["a held no-show whose evening has no time left", { ...heldNoShow, rebooking_replaces: null }, bar, false],
      ["a party at the table", seated, bar, false],
    ];
    for (const [name, held, clock, movable] of cases) {
      home({ bookings: [held], bar: clock });
      expect(screen.queryByText("Перенести") !== null, name).toBe(movable);
      expect(screen.getByText("Отменить"), name).toBeDefined();
      expect(screen.queryByText("Другой вечер"), name).toBeNull();
      cleanup();
    }
  });

  it("shows a card for every booking the guest holds, each cancelling its own", async () => {
    const onCancel = vi.fn();
    render(
      <HomeScreen
        session={session({ bookings: [seated, friday] })}
        onMove={noop}
        onCancel={onCancel}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onContact={noop}
      />,
    );
    expect(screen.getAllByText("Стол ваш")).toHaveLength(2);
    const card = screen.getByRole("group", { name: "Завтра в 21:30" });
    await userEvent.click(within(card).getByText("Отменить"));
    expect(onCancel).toHaveBeenCalledWith(friday);
  });

  it("asks about reminders exactly once, and never again after «Не нужно»", () => {
    const { unmount } = home({ bookings: [booking] });
    expect(screen.getByText("Напомнить за 3 часа?")).toBeDefined();
    expect(
      screen.getByText(
        "Бот напишет в этот чат. Планы изменятся — отмена одной кнопкой прямо из сообщения.",
      ),
    ).toBeDefined();
    unmount();

    home({ bookings: [booking], reminders: { opted_in: false, deliverable: true, should_ask: false } });
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

  it("marks an evening the guest already holds as theirs, and will not let it be tapped", async () => {
    const onServiceDate = vi.fn();
    railOf([dayOffer({ booked: true }), dayOffer({ service_date: "2026-09-12" })], onServiceDate);
    expect(screen.getByText("ваша бронь")).toBeDefined();
    await userEvent.click(screen.getByText("Сегодня"));
    expect(onServiceDate).not.toHaveBeenCalled();
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
        daysFailure={null}
        timesFailure={null}
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
        daysFailure={{ code: "internal", message: "boom" }}
        timesFailure={{ code: "network", message: "offline" }}
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
    expect(bookingDecision(4, "2026-09-11", bar, 1_290)).toEqual({
      label: "Забронировать · 4 гостя · сегодня в 21:30",
      enabled: true,
    });
    expect(bookingDecision(2, "2026-09-13", bar, 1_320).label).toBe(
      "Забронировать · 2 гостя · вс, 13 сен в 22:00",
    );
  });

  it("asks for the missing half of the decision until it has it", () => {
    expect(bookingDecision(4, "2026-09-11", bar, null)).toEqual({
      label: "Выберите время",
      enabled: false,
    });
  });

  it("will not book an evening the guest already holds, and says why", () => {
    const refused = { label: "На этот вечер у вас уже есть бронь", enabled: false };
    expect(bookingDecision(4, "2026-09-11", bar, 1_290, [seated], true)).toEqual(refused);
    expect(bookingDecision(4, "2026-09-11", bar, null, [seated], true)).toEqual(refused);
  });
});

describe("the evening the picker opens on", () => {
  it("is today without a booking, and a plan's own evening when moving it", () => {
    expect(pickerStart(session())).toBe("2026-09-11");
    expect(pickerStart(session({ bookings: [booking] }), booking)).toBe("2026-09-11");
    expect(pickerStart(session({ bookings: [friday] }), friday)).toBe("2026-09-12");
  });

  it("is never an evening the guest already holds while another is open", () => {
    // A guest at the table tonight tapping «Забронировать стол» is booking another evening.
    expect(pickerStart(session({ bookings: [seated] }))).toBe("2026-09-12");
  });

  it("is that evening when it is the only one the bar takes, where the picker then says so", () => {
    expect(pickerStart(session({ bookings: [seated], bookable_days: ["2026-09-11"] }))).toBe(
      "2026-09-11",
    );
  });

  it("is a held no-show's own evening, which is the only one that replaces it", () => {
    const session_ = session({ bookings: [heldNoShow], bookable_days: ["2026-09-12"] });
    expect(pickerStart(session_, heldNoShow)).toBe("2026-09-11");
  });

  it("falls back to the first open evening when a plan's own is out of reach", () => {
    const far = { ...booking, service_date: "2026-09-20" };
    expect(pickerStart(session({ bookings: [far] }), far)).toBe("2026-09-11");
  });
});

describe("an evening the guest already holds", () => {
  it("is the server's word on each booking, never worked out from what a new booking would replace", () => {
    // A no-show on an evening guests can no longer book: nothing the guest can do replaces it, and
    // nothing about it stops a booking on its evening.
    const outOfReach: GuestBooking = { ...heldNoShow, rebooking_replaces: null, holds_evening: false };
    expect(heldOn([outOfReach], "2026-09-11")).toBe(false);
    expect(heldOn([seated], "2026-09-11")).toBe(true);
    expect(heldOn([seated], "2026-09-12")).toBe(false);
    expect(pickerStart(session({ bookings: [outOfReach] }))).toBe("2026-09-11");
  });
});

describe("the bookings a guest holds after a write", () => {
  it("lose what a booking replaced, gain what it took, and read soonest first", () => {
    const later = { ...booking, id: "b5", start_minutes: 1_350 };
    expect(heldAfter([friday, booking], [booking.id], later)).toEqual([later, friday]);
    expect(heldAfter([seated], [], friday)).toEqual([seated, friday]);
  });

  it("lose a cancelled booking and nothing else", () => {
    expect(heldAfter([seated, friday], [friday.id], null)).toEqual([seated]);
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
        "Стол сразу уйдёт другим гостям — вернуть эту бронь не получится.",
      ),
    ).toBeDefined();

    await userEvent.click(within(sheet).getByText("Оставить"));
    expect(onClose).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();

    await userEvent.click(within(sheet).getByText("Отменить бронь"));
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});

describe("reaching a person at the bar", () => {
  it("offers the bar's own contact for a party the app does not take", async () => {
    const onContact = vi.fn();
    render(
      <HomeScreen
        session={session({
          bar: { ...bar, contact: { label: "@podval_bar", url: "https://t.me/podval_bar" } },
        })}
        onMove={noop}
        onCancel={noop}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onContact={onContact}
      />,
    );
    await userEvent.click(screen.getByText("Связаться: @podval_bar"));
    expect(onContact).toHaveBeenCalledWith("https://t.me/podval_bar");
  });

  it("points nowhere when the bar gave nowhere to point", () => {
    // The bot's chat is read by nobody. A link into it was a promise nobody kept.
    home();
    expect(screen.queryByText(/Написать бару|Связаться/)).toBeNull();
    expect(screen.getByText(/Компания больше 6/)).toBeDefined();
  });
});

describe("moving a booking the guest already holds", () => {
  it("says «Перенести» on the button, so nobody wonders whether they are about to hold two", () => {
    expect(bookingDecision(4, "2026-09-11", bar, 1_290, [booking]).label).toBe(
      "Перенести · 4 гостя · сегодня в 21:30",
    );
    expect(bookingDecision(4, "2026-09-12", bar, 1_290, [booking]).label).toBe(
      "Перенести · 4 гостя · завтра в 21:30",
    );
  });

  it("says «Забронировать» when no booking the guest holds would be replaced", () => {
    expect(bookingDecision(4, "2026-09-12", bar, 1_290, [seated]).label).toBe(
      "Забронировать · 4 гостя · завтра в 21:30",
    );
  });

  it("says «Перенести» for a held no-show only on its own evening", () => {
    expect(bookingDecision(4, "2026-09-11", bar, 1_290, [heldNoShow]).label).toBe(
      "Перенести · 4 гостя · сегодня в 21:30",
    );
    expect(bookingDecision(4, "2026-09-12", bar, 1_290, [heldNoShow]).label).toBe(
      "Забронировать · 4 гостя · завтра в 21:30",
    );
  });

  it("confirms a move as a move", () => {
    render(<DoneScreen booking={booking} bar={bar} moved />);
    expect(screen.getByText("Бронь перенесена")).toBeDefined();
    expect(screen.queryByText("Стол забронирован")).toBeNull();
  });

  it("tells the guest how long the table is held in words that agree with the number", () => {
    home({ bookings: [booking], bar: { ...bar, grace_minutes: 21 } });
    expect(screen.getByText(/Держим стол 21 минуту после времени брони/)).toBeDefined();
    expect(screen.queryByText(/стол дождётся/)).toBeNull();
  });
});

describe("the time grid, for a screen reader and a slow phone", () => {
  it("says a taken time is taken in its name, not only by a line through it", () => {
    render(
      <BookScreen
        bar={bar}
        days={rail(4)}
        availability={availability()}
        partySize={2}
        serviceDate="2026-09-11"
        chosenMinutes={null}
        daysFailure={null}
        timesFailure={null}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onTakenSlot={noop}
        onRetry={noop}
      />,
    );
    expect(screen.getByRole("button", { name: "22:00, занято" })).toBeDefined();
  });

  it("keeps the last answer on screen while it asks again, but will not take a tap on it", async () => {
    // Blanking the grid to a spinner on every change made the page jump; taking a tap on times for
    // the party the guest just stopped bringing would book the wrong question.
    const onPick = vi.fn();
    render(
      <BookScreen
        bar={bar}
        days={rail(4)}
        availability={availability()}
        partySize={4}
        serviceDate="2026-09-11"
        chosenMinutes={null}
        daysFailure={null}
        timesFailure={null}
        timesPending
        onPartySize={noop}
        onServiceDate={noop}
        onPick={onPick}
        onTakenSlot={noop}
        onRetry={noop}
      />,
    );
    const time = screen.getByRole("button", { name: "21:30" });
    expect((time as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(time);
    expect(onPick).not.toHaveBeenCalled();
  });
});
