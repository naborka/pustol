/**
 * The screens, rendered.
 *
 * These check the things a reader of the code cannot check by reading it: that a guest is never
 * shown a table number, that a time already taken cannot be tapped, and that the notes say what the
 * bar has actually configured rather than a number somebody hard-coded.
 */

import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  Availability,
  BarView,
  GuestBooking,
  Limits,
  Session,
  SettingsDraft,
  SettingsView,
  ShiftView,
} from "@/lib/api";
import { draftOf } from "@/lib/api";
import { BookScreen, HomeScreen } from "../GuestScreens";
import { DayNavigator, ShiftList } from "../AdminShift";
import { SettingsScreen } from "../Settings";

afterEach(cleanup);

const bar: BarView = {
  name: "Бар «Подвал»",
  address: "Дечанска 12, Белград",
  timezone: "Europe/Belgrade",
  max_party: 6,
  grace_minutes: 15,
  remind_hours: 3,
  turn_minutes: 120,
  slot_step_minutes: 30,
  today: "2026-07-30",
  today_hours: { open_minutes: 600, close_minutes: 1_560, closed: false },
  last_arrival_minutes: 1_440,
};

const booking: GuestBooking = {
  id: "b1",
  service_date: "2026-07-30",
  start_minutes: 1_200,
  end_minutes: 1_320,
  party_size: 2,
  status: "confirmed",
};

function session(overrides: Partial<Session> = {}): Session {
  return {
    user: { id: 999, first_name: "Алексей", username: "alexey" },
    is_staff: false,
    reminders: { opted_in: false, deliverable: true, should_ask: true },
    bar,
    booking: null,
    bookable_days: ["2026-07-30", "2026-07-31"],
    ...overrides,
  };
}

const noop = () => {};

describe("the guest's home screen", () => {
  it("invites a booking when there is none", () => {
    render(
      <HomeScreen
        session={session()}
        onCancel={noop}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    expect(screen.getByText("Столик на вечер")).toBeDefined();
    expect(screen.getByText("10:00 — 02:00")).toBeDefined();
    expect(screen.getByText("Дечанска 12, Белград")).toBeDefined();
    expect(screen.getByText("Компания больше 6")).toBeDefined();
  });

  it("shows a booking without ever naming a table", () => {
    // The bar assigns tables and moves them when the room changes. A number on a guest's screen is a
    // number they arrive quoting.
    const { container } = render(
      <HomeScreen
        session={session({ booking })}
        onCancel={noop}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    expect(screen.getByText("Бронь подтверждена")).toBeDefined();
    expect(screen.getByText("Сегодня в 20:00")).toBeDefined();
    expect(screen.getByText("2 гостя")).toBeDefined();
    expect(container.textContent).not.toMatch(/стол\s*\d/i);
  });

  it("quotes the grace period the bar actually configured", () => {
    render(
      <HomeScreen
        session={session({ booking, bar: { ...bar, grace_minutes: 25 } })}
        onCancel={noop}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    expect(screen.getByText(/25 минут/)).toBeDefined();
  });

  it("offers reminders once and stops offering them", () => {
    const { rerender } = render(
      <HomeScreen
        session={session({ booking })}
        onCancel={noop}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    expect(screen.getByText("Напомнить о брони?")).toBeDefined();

    rerender(
      <HomeScreen
        session={session({
          booking,
          reminders: { opted_in: true, deliverable: true, should_ask: false },
        })}
        onCancel={noop}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    expect(screen.queryByText("Напомнить о брони?")).toBeNull();
  });

  it("does not offer reminders to somebody with nothing booked", () => {
    render(
      <HomeScreen
        session={session()}
        onCancel={noop}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    expect(screen.queryByText("Напомнить о брони?")).toBeNull();
  });

  it("hands the cancellation back to whoever asked for it", async () => {
    const onCancel = vi.fn();
    render(
      <HomeScreen
        session={session({ booking })}
        onCancel={onCancel}
        onEnableReminders={noop}
        onDismissReminders={noop}
        onWriteToBar={noop}
      />,
    );
    await userEvent.click(screen.getByText("Отменить бронь"));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

const availability = (overrides: Partial<Availability> = {}): Availability => ({
  service_date: "2026-07-30",
  party_size: 2,
  turn_minutes: 120,
  slots: [
    { start_minutes: 600, state: "past", evening: false },
    { start_minutes: 990, state: "free", evening: false },
    { start_minutes: 1_200, state: "free", evening: true },
    { start_minutes: 1_230, state: "taken", evening: true },
  ],
  free_count: 2,
  ...overrides,
});

describe("the picker", () => {
  it("folds the daytime away and opens on the evening", () => {
    render(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30", "2026-07-31"]}
        availability={availability()}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown={false}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onShowDaytime={noop}
      />,
    );
    // The evening is on screen; the daytime is behind a disclosure that names its range.
    expect(screen.getByText("20:00")).toBeDefined();
    expect(screen.getByText("Днём")).toBeDefined();
    expect(screen.getByText("10:00 — 16:30 ›")).toBeDefined();
    expect(screen.queryByText("16:30")).toBeNull();
  });

  it("shows the daytime once it is asked for", () => {
    render(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30"]}
        availability={availability()}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onShowDaytime={noop}
      />,
    );
    expect(screen.getByText("16:30")).toBeDefined();
  });

  it("will not let a time that is taken or gone be tapped", async () => {
    const onPick = vi.fn();
    render(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30"]}
        availability={availability()}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown
        onPartySize={noop}
        onServiceDate={noop}
        onPick={onPick}
        onShowDaytime={noop}
      />,
    );
    await userEvent.click(screen.getByText("20:30"));
    expect(onPick).not.toHaveBeenCalled();
    await userEvent.click(screen.getByText("10:00"));
    expect(onPick).not.toHaveBeenCalled();
    await userEvent.click(screen.getByText("20:00"));
    expect(onPick).toHaveBeenCalledWith(1_200);
  });

  it("offers exactly the party sizes the bar accepts", () => {
    render(
      <BookScreen
        bar={{ ...bar, max_party: 4 }}
        bookableDays={["2026-07-30"]}
        availability={availability()}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown={false}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onShowDaytime={noop}
      />,
    );
    expect(screen.getByRole("button", { name: "4 гостя" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "5 гостей" })).toBeNull();
  });

  it("says how many windows are free, and says so plainly when none are", () => {
    const { rerender } = render(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30"]}
        availability={availability()}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown={false}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onShowDaytime={noop}
      />,
    );
    expect(screen.getByText(/Свободно окон: 2/)).toBeDefined();

    rerender(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30"]}
        availability={availability({ free_count: 0 })}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown={false}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onShowDaytime={noop}
      />,
    );
    expect(screen.getByText(/нет ни одного окна/)).toBeDefined();
  });

  it("names the day rather than showing a bare date", () => {
    render(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30", "2026-07-31", "2026-08-01"]}
        availability={availability()}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown={false}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onShowDaytime={noop}
      />,
    );
    expect(screen.getByText("Сегодня")).toBeDefined();
    expect(screen.getByText("Завтра")).toBeDefined();
    expect(screen.getByText("сб")).toBeDefined();
  });
});

const shift: ShiftView = {
  service_date: "2026-07-30",
  hours: { open_minutes: 600, close_minutes: 1_560, closed: false },
  tables: [{ id: "t1", number: 1, seats: 2, zone: "Бар", blocked_because: null }],
  bookings: [
    {
      id: "b1",
      table_id: "t1",
      table_number: 1,
      table_zone: "Бар",
      start_minutes: 1_200,
      end_minutes: 1_320,
      party_size: 2,
      guest_name: "Анна К.",
      guest_username: "anna_k",
      status: "arrived",
      source: "app",
      reachable_by_bot: true,
    },
    {
      id: "b2",
      table_id: null,
      table_number: null,
      table_zone: null,
      start_minutes: 1_140,
      end_minutes: 1_260,
      party_size: 6,
      guest_name: "Тимур",
      guest_username: null,
      status: "confirmed",
      source: "staff",
      reachable_by_bot: false,
    },
  ],
  stats: { bookings: 2, guests: 8, free_now: 0 },
  now_minutes: 1_215,
  cancel_reasons: ["Дождь"],
  message_templates: ["Ваш стол готов"],
};

describe("the shift list", () => {
  it("reads out the evening in arrival order and says who has no table", () => {
    render(<ShiftList shift={shift} onOpenBooking={noop} />);
    const names = screen.getAllByText(/Анна К\.|Тимур/).map((node) => node.textContent);
    expect(names).toEqual(["Тимур", "Анна К."]);
    expect(screen.getByText("без стола")).toBeDefined();
    expect(screen.getByText("за столом")).toBeDefined();
    expect(screen.getByText(/6 гостей · стол не назначен · вручную/)).toBeDefined();
    expect(screen.getByText("2 гостя · стол 1")).toBeDefined();
  });

  it("says so when there is nothing on", () => {
    render(<ShiftList shift={{ ...shift, bookings: [] }} onOpenBooking={noop} />);
    expect(screen.getByText("На этот день броней нет")).toBeDefined();
  });
});

describe("the shift day control", () => {
  it("names prev and next, hits a first-class row, and steps the service date", async () => {
    const onChange = vi.fn();
    render(<DayNavigator serviceDate="2026-07-30" today="2026-07-30" onChange={onChange} />);

    const prev = screen.getByRole("button", { name: "Предыдущий день" });
    const next = screen.getByRole("button", { name: "Следующий день" });
    expect(prev.textContent).toMatch(/29 июл/);
    expect(next.textContent).toMatch(/31 июл/);

    const hit = Number.parseInt(prev.style.minHeight || prev.style.height, 10);
    expect(hit).toBeGreaterThanOrEqual(44);
    expect(prev.style.height).not.toBe("34px");

    await userEvent.click(prev);
    expect(onChange).toHaveBeenCalledWith("2026-07-29");
    await userEvent.click(next);
    expect(onChange).toHaveBeenCalledWith("2026-07-31");
  });

  it("jumps back to today from another day", async () => {
    const onChange = vi.fn();
    render(<DayNavigator serviceDate="2026-08-02" today="2026-07-30" onChange={onChange} />);

    const viewed = screen.getByText("2 авг");
    expect(viewed.textContent).toBe("2 авг");
    expect(viewed.closest("button")).toBeNull();
    await userEvent.click(viewed);
    expect(onChange).not.toHaveBeenCalled();

    const jump = screen.getByRole("button", { name: "Сегодня" });
    expect(jump.textContent).toBe("Сегодня");
    await userEvent.click(jump);
    expect(onChange).toHaveBeenCalledWith("2026-07-30");
  });
});

const SETTINGS_LIMITS: Limits = {
  open_minutes: { min: 480, max: 1_080 },
  close_minutes: { min: 1_200, max: 1_680 },
  turn_minutes: { min: 60, max: 240 },
  max_party: { min: 2, max: 10 },
  horizon_days: { min: 1, max: 30 },
  remind_hours: { min: 1, max: 12 },
  grace_minutes: { min: 5, max: 60 },
  seats: { min: 1, max: 12 },
  slot_step_minutes: [15, 30, 60],
};

function settingsView(): SettingsView {
  return {
    name: "Бар «Подвал»",
    address: "Дечанска 12, Белград",
    timezone: "Europe/Belgrade",
    week: Array.from({ length: 7 }, () => ({
      open_minutes: 600,
      close_minutes: 1_560,
      closed: false,
    })),
    zones: ["Бар", "Зал"],
    tables: [{ id: "t1", number: 1, seats: 2, zone: "Бар", bookings_today: 0 }],
    turn_minutes: 120,
    slot_step_minutes: 30,
    max_party: 6,
    horizon_days: 4,
    remind_hours: 3,
    grace_minutes: 15,
    message_templates: ["Ваш стол готов"],
    cancel_reasons: ["Дождь"],
    staff: [{ username: "anna_mgr", bound: true }],
    next_table_number: 2,
    limits: SETTINGS_LIMITS,
  };
}

describe("settings groups", () => {
  it("shows one group at a time", async () => {
    const view = settingsView();
    render(
      <SettingsScreen
        settings={view}
        draft={draftOf(view)}
        editedWeekday={1}
        onDraft={noop}
        onEditWeekday={noop}
        onSave={noop}
        onRevert={noop}
        saving={false}
      />,
    );

    expect(screen.getByPlaceholderText("Название")).toBeDefined();
    expect(screen.queryByText("Стол 1")).toBeNull();
    expect(screen.queryByText("Бронь занимает")).toBeNull();
    expect(screen.queryByText("@anna_mgr")).toBeNull();

    await userEvent.click(screen.getByRole("tab", { name: "Столы" }));
    expect(screen.getByText("Стол 1")).toBeDefined();
    expect(screen.queryByPlaceholderText("Название")).toBeNull();

    await userEvent.click(screen.getByRole("tab", { name: "Люди" }));
    expect(screen.getByText("@anna_mgr")).toBeDefined();
    expect(screen.queryByText("Стол 1")).toBeNull();
  });

  it("keeps one draft, one save, one revert across groups", async () => {
    const view = settingsView();
    let current: SettingsDraft = draftOf(view);
    const onSave = vi.fn();
    const onRevert = vi.fn();

    const draw = () => (
      <SettingsScreen
        settings={view}
        draft={current}
        editedWeekday={1}
        onDraft={(next) => {
          current = next;
          rerender(draw());
        }}
        onEditWeekday={noop}
        onSave={onSave}
        onRevert={onRevert}
        saving={false}
      />
    );
    const { rerender } = render(draw());

    const name = screen.getByPlaceholderText("Название");
    await userEvent.clear(name);
    await userEvent.type(name, "Новый");
    expect(screen.getByText("Сохранить")).toBeDefined();
    expect(screen.getByText("Отмена")).toBeDefined();

    await userEvent.click(screen.getByRole("tab", { name: "Столы" }));
    expect(screen.queryByPlaceholderText("Название")).toBeNull();
    expect(screen.getByText("Сохранить")).toBeDefined();

    await userEvent.click(screen.getByText("Сохранить"));
    expect(onSave).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByText("Отмена"));
    expect(onRevert).toHaveBeenCalledOnce();
  });
});
