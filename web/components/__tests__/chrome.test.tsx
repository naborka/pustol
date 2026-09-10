/**
 * App chrome: staff menu always on, no view-name header, insets, guest back.
 */

import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppShell } from "../AppChrome";
import { BookScreen } from "../GuestScreens";
import type { Availability, BarView } from "@/lib/api";
import { ZERO_INSETS } from "@/lib/telegram";

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

const availability: Availability = {
  service_date: "2026-07-30",
  party_size: 2,
  turn_minutes: 120,
  slots: [{ start_minutes: 1_200, state: "free", evening: true }],
  free_count: 1,
};

const noop = () => {};

describe("staff chrome", () => {
  it("keeps the tab menu on every staff tab and never draws the old view header", async () => {
    const onTab = vi.fn();
    const { rerender } = render(
      <AppShell staff tab="client" onTab={onTab} insets={ZERO_INSETS}>
        бронь
      </AppShell>,
    );

    const menu = () => screen.getByRole("navigation", { name: "Разделы" });
    expect(menu()).toBeDefined();
    expect(screen.getByRole("button", { name: "Моя бронь" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Смена" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Настройки" })).toBeDefined();
    expect(screen.queryByText("админ")).toBeNull();
    expect(screen.queryByText("бар и правила")).toBeNull();
    expect(screen.queryByRole("banner")).toBeNull();

    rerender(
      <AppShell staff tab="shift" onTab={onTab} insets={ZERO_INSETS}>
        смена
      </AppShell>,
    );
    expect(menu()).toBeDefined();
    expect(screen.getByRole("button", { name: "Смена" }).getAttribute("aria-current")).toBe(
      "true",
    );

    rerender(
      <AppShell staff tab="settings" onTab={onTab} insets={ZERO_INSETS}>
        настройки
      </AppShell>,
    );
    expect(menu()).toBeDefined();
    expect(screen.queryByText("админ")).toBeNull();
    expect(screen.queryByText("бар и правила")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Смена" }));
    expect(onTab).toHaveBeenCalledWith("shift");
  });

  it("does not invent a guest tab bar", () => {
    render(
      <AppShell staff={false} tab="client" onTab={noop} insets={ZERO_INSETS}>
        гость
      </AppShell>,
    );
    expect(screen.queryByRole("navigation", { name: "Разделы" })).toBeNull();
    expect(screen.queryByText("Моя бронь")).toBeNull();
  });

  it("insets the shell top and the staff menu bottom so Telegram chrome is not covered", () => {
    const { container } = render(
      <AppShell
        staff
        tab="shift"
        onTab={noop}
        insets={{ top: 47, right: 0, bottom: 34, left: 0 }}
      >
        смена
      </AppShell>,
    );
    const shell = container.firstElementChild as HTMLElement;
    expect(shell.style.paddingTop).toBe("47px");
    const menu = screen.getByRole("navigation", { name: "Разделы" });
    expect(menu.style.paddingBottom).toBe("34px");
  });
});

describe("book to home without the view header", () => {
  it("goes home from an in-content back when Telegram has no BackButton", async () => {
    const onBack = vi.fn();
    render(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30"]}
        availability={availability}
        partySize={2}
        serviceDate="2026-07-30"
        chosenMinutes={null}
        daytimeShown={false}
        onPartySize={noop}
        onServiceDate={noop}
        onPick={noop}
        onShowDaytime={noop}
        onBack={onBack}
      />,
    );
    expect(screen.queryByText("админ")).toBeNull();
    expect(screen.queryByRole("banner")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Назад" }));
    expect(onBack).toHaveBeenCalledOnce();
  });

  it("does not draw a second back when Telegram already has one", () => {
    render(
      <BookScreen
        bar={bar}
        bookableDays={["2026-07-30"]}
        availability={availability}
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
    expect(screen.queryByRole("button", { name: "Назад" })).toBeNull();
  });
});
