/**
 * The whole app, against a fake Telegram and a fake server.
 *
 * The screens are tested on their own elsewhere. What only shows up here is the wiring between
 * them: when the app fetches, what it does with two taps, what a failure after a success leaves
 * on screen, and what Telegram's own buttons do. Everything in this file was broken in a way no
 * screen test could see.
 */

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderToString } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import Page from "../page";
import {
  availability,
  booking,
  rail,
  seated,
  session,
  settingsView,
  shift,
  shiftBooking,
  shiftTable,
} from "@/components/__tests__/fixtures";
import type { Attendance, GuestBooking, SettingsDraft, ShiftBooking } from "@/lib/api";
import type { WebApp } from "@/lib/telegram";

/** An answer the test lets through when it chooses. */
function gate() {
  let open: () => void = () => {};
  const opened = new Promise<void>((resolve) => {
    open = resolve;
  });
  return { opened, open: () => act(async () => open()) };
}

/** Long enough for every answer already let through to reach the screen. */
const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 50)));

const nothingMoved = { moved: [], orphaned: [] };

interface Answer {
  status?: number;
  body: unknown;
}

type Handler = (request: { body: unknown; url: URL }) => Answer | Promise<Answer>;

const failed: Answer = { status: 500, body: { error: { code: "internal", message: "boom" } } };

function fakeServer(routes: Record<string, Handler>) {
  const calls: { method: string; path: string; body: unknown }[] = [];
  const fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const url = new URL(input, "http://app.test");
    const method = init?.method ?? "GET";
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ method, path: url.pathname, body });
    const handler = routes[`${method} ${url.pathname}`];
    const { status = 200, body: answer } = handler
      ? await handler({ body, url })
      : { status: 404, body: { error: { code: "not_found", message: url.pathname } } };
    return new Response(JSON.stringify(answer), {
      status,
      headers: { "content-type": "application/json" },
    });
  });
  vi.stubGlobal("fetch", fetch);
  return {
    count: (method: string, path: string) =>
      calls.filter((call) => call.method === method && call.path === path).length,
    bodies: (method: string, path: string) =>
      calls.filter((call) => call.method === method && call.path === path).map((call) => call.body),
  };
}

function fakeTelegram() {
  const listeners = new Map<string, Set<() => void>>();
  const back = new Set<() => void>();
  const app = {
    initData: "signed-by-telegram",
    colorScheme: "dark",
    themeParams: {},
    version: "8.0",
    isExpanded: true,
    viewportStableHeight: 700,
    ready: vi.fn(),
    expand: vi.fn(),
    close: vi.fn(),
    onEvent: (event: string, handler: () => void) => {
      const set = listeners.get(event) ?? new Set();
      set.add(handler);
      listeners.set(event, set);
    },
    offEvent: (event: string, handler: () => void) => listeners.get(event)?.delete(handler),
    openTelegramLink: vi.fn(),
    enableClosingConfirmation: vi.fn(),
    disableClosingConfirmation: vi.fn(),
    HapticFeedback: {
      impactOccurred: vi.fn(),
      notificationOccurred: vi.fn(),
      selectionChanged: vi.fn(),
    },
    BackButton: {
      show: vi.fn(),
      hide: vi.fn(),
      onClick: (handler: () => void) => back.add(handler),
      offClick: (handler: () => void) => back.delete(handler),
    },
  };
  window.Telegram = { WebApp: app as unknown as WebApp };
  return {
    app,
    emit: (event: string) => listeners.get(event)?.forEach((handler) => handler()),
    pressBack: () => [...back].forEach((handler) => handler()),
  };
}

const staffSession = () => ({ body: session({ is_staff: true }) });

/**
 * What the server answers an attendance change with: the booking now, what it was just before, and
 * the evening as it stands after.
 */
const attended = (status: Attendance, previous: Attendance, released: number | null = null) => {
  const now = shiftBooking({ status, released_minutes: released });
  return { body: { booking: now, previous, shift: shift({ bookings: [now] }) } };
};

/** A staff cancellation of Саша, and the evening left with `remaining`. */
const cancelled = (remaining: ShiftBooking[] = []) => ({
  body: {
    booking: shiftBooking({ status: "cancelled" }),
    reconciliation: nothingMoved,
    guest_notified: true,
    shift: shift({ bookings: remaining }),
  },
});

const timur = shiftBooking({
  id: "b2",
  guest_name: "Тимур",
  table_id: "t2",
  table_number: 8,
  table_zone: "Зал",
  start_minutes: 1_320,
  end_minutes: 1_440,
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  delete window.Telegram;
});

describe("the first paint", () => {
  it("does not tell a guest to open the app from Telegram before it has even looked", () => {
    // The page is prerendered where there is no Telegram at all, and that HTML is what a guest sees
    // until the scripts have loaded.
    const html = renderToString(<Page />);
    expect(html).not.toContain("Откройте приложение из Telegram");
  });

  it("opens on the first read when a refresh that overtook it fails", async () => {
    // The refresh's failure is older news than the first read's answer, however late that lands.
    const telegram = fakeTelegram();
    const first = gate();
    let sessions = 0;
    const server = fakeServer({
      "GET /api/session": async () => {
        sessions += 1;
        if (sessions > 1) return failed;
        await first.opened;
        return { body: session() };
      },
    });
    render(<Page />);

    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(1));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(2));
    await settle();
    expect(screen.queryByText("Попробовать снова")).toBeNull();

    await first.open();
    expect(await screen.findByText("Столик на вечер")).toBeDefined();
    await settle();
    expect(screen.queryByText("Попробовать снова")).toBeNull();
  });

  it("says the app could not open, rather than spinning, once every read has failed", async () => {
    const telegram = fakeTelegram();
    const first = gate();
    let sessions = 0;
    const server = fakeServer({
      "GET /api/session": async () => {
        sessions += 1;
        if (sessions === 1) await first.opened;
        return failed;
      },
    });
    render(<Page />);

    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(1));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(2));
    await first.open();
    expect(await screen.findByText("Попробовать снова")).toBeDefined();
  });

  it("shows what a retry answered outside Telegram, not the failure it retried", async () => {
    vi.stubEnv("NEXT_PUBLIC_DEV_INIT_DATA", "signed-for-development");
    let sessions = 0;
    fakeServer({
      "GET /api/session": () => {
        sessions += 1;
        if (sessions === 1) {
          return { status: 401, body: { error: { code: "session_expired", message: "old" } } };
        }
        if (sessions === 2) {
          return { status: 503, body: { error: { code: "network", message: "offline" } } };
        }
        return { body: session() };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    const expired = "Сессия устарела. Закройте и откройте приложение — всё сохранится.";
    expect(await screen.findByText(expired)).toBeDefined();
    await user.click(screen.getByText("Попробовать снова"));
    expect(await screen.findByText("Нет связи. Проверьте интернет и попробуйте ещё раз.")).toBeDefined();
    expect(screen.queryByText(expired)).toBeNull();

    await user.click(screen.getByText("Попробовать снова"));
    expect(await screen.findByText("Столик на вечер")).toBeDefined();
  });

  it("catches up when the app comes back while an older read is still on its way", async () => {
    const telegram = fakeTelegram();
    const late = gate();
    let sessions = 0;
    const server = fakeServer({
      "GET /api/session": async () => {
        const read = (sessions += 1);
        if (read === 2) await late.opened;
        return { body: read <= 2 ? session({ bookings: [booking] }) : session() };
      },
    });
    render(<Page />);

    await screen.findByText("Стол ваш");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(2));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(3));
    await waitFor(() => expect(screen.queryByText("Стол ваш")).toBeNull());

    await late.open();
    await settle();
    expect(screen.queryByText("Стол ваш")).toBeNull();
  });
});

describe("a shift left open on the bar", () => {
  it("keeps up with the room without anybody touching it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const telegram = fakeTelegram();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
    });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    expect(server.count("GET", "/api/admin/shift")).toBe(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));

    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(3));
    expect(screen.getByText("Саша")).toBeDefined();
  });

  it("offers nothing to write into an evening that is already over", async () => {
    fakeTelegram();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": ({ url }) => ({
        body: shift({ service_date: url.searchParams.get("service_date") ?? "" }),
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    expect(await screen.findByText("Посадить сейчас")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Предыдущий день" }));
    await screen.findByText("чт, 10 сен");
    expect(screen.queryByText("Записать гостя")).toBeNull();
    expect(screen.queryByText("Посадить сейчас")).toBeNull();
  });

  it("goes by the server's evening, not the day this phone opened on, for what it offers", async () => {
    // Left open past midnight: the phone's session still says the 11th, the bar is on the 12th.
    fakeTelegram();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        return {
          body: shift({
            service_date: date,
            today: "2026-09-12",
            now_minutes: date === "2026-09-12" ? 1_280 : null,
          }),
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    expect(screen.queryByText("Посадить сейчас")).toBeNull();
    expect(screen.queryByText("Записать гостя")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    expect(await screen.findByText("Посадить сейчас")).toBeDefined();
  });

  it("closes the open sheet when Telegram's back button is pressed", async () => {
    const telegram = fakeTelegram();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    expect(await screen.findByText("Как прошло")).toBeDefined();

    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByText("Как прошло")).toBeNull());
  });

  it("shows an open booking as the room now has it once the shift is read again", async () => {
    const telegram = fakeTelegram();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => {
        reads += 1;
        return {
          body: reads === 1 ? shift() : shift({ bookings: [shiftBooking({ status: "arrived" })] }),
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    const sheet = await screen.findByRole("dialog");
    expect(within(sheet).queryByText("Ушли")).toBeNull();

    act(() => telegram.emit("activated"));
    expect(await within(sheet).findByText("Ушли")).toBeDefined();
  });

  it("does not reopen a closed sheet when its note answers afterwards", async () => {
    const telegram = fakeTelegram();
    const noted = gate();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "PATCH /api/admin/bookings/b1/note": async () => {
        await noted.opened;
        const now = shiftBooking({ note: "У окна" });
        return { body: { booking: now, shift: shift({ bookings: [now] }) } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("У окна"));
    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByText("Как прошло")).toBeNull());

    await noted.open();
    await settle();
    expect(screen.queryByText("Как прошло")).toBeNull();
  });

  it("does not reopen a closed sheet when its undo is tapped", async () => {
    const telegram = fakeTelegram();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "PATCH /api/admin/bookings/b1/attendance": ({ body }) => {
        const attendance = (body as { attendance: Attendance }).attendance;
        return attended(attendance, attendance === "arrived" ? "confirmed" : "arrived");
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("За столом"));
    await screen.findByText("Саша за столом 7.");
    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByText("Как прошло")).toBeNull());

    await user.click(screen.getByText("Вернуть"));
    await screen.findByText("Саша: снова ждём.");
    await settle();
    expect(screen.queryByText("Как прошло")).toBeNull();
  });

  it("keeps the next evening on screen when an action on this one answers after the step", async () => {
    fakeTelegram();
    const seatedGate = gate();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        return { body: date === "2026-09-11" ? shift() : shift({ service_date: date, bookings: [] }) };
      },
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seatedGate.opened;
        return attended("arrived", "confirmed");
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    await screen.findByText("сб, 12 сен");

    await seatedGate.open();
    await screen.findByText("Саша за столом 7.");
    await settle();
    expect(screen.getByText("сб, 12 сен")).toBeDefined();
    expect(screen.queryByText("Саша")).toBeNull();
  });

  it("does nothing, and keeps the table open, when a table is closed while another action runs", async () => {
    fakeTelegram();
    const seatedGate = gate();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seatedGate.opened;
        return attended("arrived", "confirmed");
      },
      "POST /api/admin/blocks": () => ({
        body: { reconciliation: nothingMoved, closed: ["t2"], shift: shift() },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    await user.click(screen.getByText("Столы"));
    await user.click(await screen.findByRole("button", { name: "Стол 8" }));
    const sheet = await screen.findByRole("dialog");
    await user.click(within(sheet).getByText("Закрыть стол на вечер"));
    await user.click(within(sheet).getByText("Дождь"));

    expect(screen.getByRole("dialog")).toBeDefined();
    expect(server.count("POST", "/api/admin/blocks")).toBe(0);
    await seatedGate.open();
  });

  it("says so, with a buzz, when a tap is dropped because another action is still running", async () => {
    const telegram = fakeTelegram();
    const seatedGate = gate();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seatedGate.opened;
        return attended("arrived", "confirmed");
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    await user.click(screen.getByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Не пришли"));

    expect(
      await screen.findByText("Подождите — прошлое действие ещё выполняется."),
    ).toBeDefined();
    expect(telegram.app.HapticFeedback.notificationOccurred).toHaveBeenCalledWith("warning");
    expect(server.count("PATCH", "/api/admin/bookings/b1/attendance")).toBe(1);
    await seatedGate.open();
  });

  it("keeps an undo on screen when a tap is dropped while another action runs", async () => {
    // Replacing «Вернуть» with «Подождите» took away the only way back from the tap before.
    const telegram = fakeTelegram();
    const noting = gate();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "PATCH /api/admin/bookings/b1/attendance": () => attended("arrived", "confirmed"),
      "PATCH /api/admin/bookings/b1/note": async () => {
        await noting.opened;
        const now = shiftBooking({ status: "arrived", note: "У окна" });
        return { body: { booking: now, shift: shift({ bookings: [now] }) } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    await screen.findByText("Саша за столом 7.");
    await user.click(screen.getByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("У окна"));
    await user.click(screen.getByText("Вернуть"));

    expect(telegram.app.HapticFeedback.notificationOccurred).toHaveBeenCalledWith("warning");
    expect(screen.getByText("Саша за столом 7.")).toBeDefined();
    expect(screen.getByText("Вернуть")).toBeDefined();
    expect(screen.queryByText("Подождите — прошлое действие ещё выполняется.")).toBeNull();
    expect(server.count("PATCH", "/api/admin/bookings/b1/attendance")).toBe(1);
    await noting.open();
  });

  it("leaves a sheet opened meanwhile open when a message sent from an earlier one answers", async () => {
    const telegram = fakeTelegram();
    const sent = gate();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "POST /api/admin/bookings/b1/message": async () => {
        await sent.opened;
        return { body: { queued: true } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Написать гостю"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Ваш стол готов"));
    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByText(/отменить отправку нельзя/)).toBeNull());
    await user.click(screen.getByText("Саша"));
    expect(await screen.findByText("Как прошло")).toBeDefined();

    await sent.open();
    await screen.findByText("Отправлено Саша: «Ваш стол готов»");
    await settle();
    expect(screen.getByText("Как прошло")).toBeDefined();
  });

  it("leaves another booking's sheet open when a cancellation from an earlier sheet answers", async () => {
    const telegram = fakeTelegram();
    const cancelling = gate();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift({ bookings: [shiftBooking(), timur] }) }),
      "POST /api/admin/bookings/b1/cancel": async () => {
        await cancelling.opened;
        return cancelled([timur]);
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Дождь"));
    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByText("Причина отмены")).toBeNull());
    await user.click(screen.getByText("Тимур"));
    expect(await screen.findByText("Как прошло")).toBeDefined();

    await cancelling.open();
    await screen.findByText(/^Бронь отменена\./);
    await settle();
    expect(within(screen.getByRole("dialog")).getByText("Как прошло")).toBeDefined();
  });

  it("closes a sheet reopened on the booking a cancellation just removed", async () => {
    const telegram = fakeTelegram();
    const cancelling = gate();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "POST /api/admin/bookings/b1/cancel": async () => {
        await cancelling.opened;
        return cancelled();
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Дождь"));
    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByText("Причина отмены")).toBeNull());
    await user.click(screen.getByText("Саша"));
    expect(await screen.findByText("Как прошло")).toBeDefined();

    await cancelling.open();
    await screen.findByText(/^Бронь отменена\./);
    await waitFor(() => expect(screen.queryByText("Как прошло")).toBeNull());
  });

  it("shows a cancelled booking gone even when the shift cannot be read again", async () => {
    fakeTelegram();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => (reads++ === 0 ? { body: shift() } : failed),
      "POST /api/admin/bookings/b1/cancel": () => cancelled(),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Дождь"));
    await screen.findByText(/^Бронь отменена\./);
    await settle();
    expect(screen.queryByText("Саша")).toBeNull();
  });

  it("shows where the server reseated another party after an attendance change, with no reread", async () => {
    fakeTelegram();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () =>
        reads++ === 0 ? { body: shift({ bookings: [shiftBooking(), timur] }) } : failed,
      "PATCH /api/admin/bookings/b1/attendance": () => {
        const now = shiftBooking({ status: "arrived" });
        const moved = { ...timur, table_id: "t3", table_number: 10 };
        return { body: { booking: now, previous: "confirmed", shift: shift({ bookings: [now, moved] }) } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("стол 8 · Зал");
    await user.click(screen.getAllByText("Посадить")[0] as HTMLElement);
    await screen.findByText("Саша за столом 7.");
    expect(await screen.findByText("стол 10 · Зал")).toBeDefined();
    expect(screen.queryByText("стол 8 · Зал")).toBeNull();
  });

  it("shows what finding a table rearranged even when a refresh after it fails", async () => {
    const telegram = fakeTelegram();
    const gleb = shiftBooking({
      id: "b3",
      guest_name: "Глеб",
      table_id: null,
      table_number: null,
      table_zone: null,
      start_minutes: 1_320,
      end_minutes: 1_440,
    });
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => (reads++ === 0 ? { body: shift({ bookings: [gleb] }) } : failed),
      "POST /api/admin/shift/reconcile": () => ({
        body: {
          reconciliation: { moved: [{ booking_id: "b3", guest_name: "Глеб", to_number: 10 }], orphaned: [] },
          shift: shift({ bookings: [{ ...gleb, table_id: "t3", table_number: 10, table_zone: "Зал" }] }),
        },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click((await screen.findAllByText("Найти стол"))[0] as HTMLElement);
    await screen.findByText("Пересажены: Глеб → стол 10.");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    await settle();
    expect(screen.getByText("стол 10 · Зал")).toBeDefined();
    expect(screen.queryByText("Не удалось прочитать смену.")).toBeNull();
  });

  it("offers no undo for closing a table the server says this tap did not close", async () => {
    // A colleague closed it a moment earlier; «Вернуть» would have opened their table.
    fakeTelegram();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "POST /api/admin/blocks": () => ({
        body: {
          reconciliation: nothingMoved,
          closed: [],
          shift: shift({
            tables: [shiftTable(), shiftTable({ id: "t2", number: 8, seats: 4, zone: "Зал", blocked_because: "Дождь" })],
          }),
        },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Столы"));
    await user.click(await screen.findByRole("button", { name: "Стол 8" }));
    const sheet = await screen.findByRole("dialog");
    await user.click(within(sheet).getByText("Закрыть стол на вечер"));
    await user.click(within(sheet).getByText("Дождь"));
    await screen.findByText("Стол 8 закрыт на вечер. Броней там не было.");
    expect(screen.queryByText("Вернуть")).toBeNull();
  });

  it("undoes opening a table by closing what the server reopened, for the reason it reports", async () => {
    fakeTelegram();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({
        body: shift({
          tables: [shiftTable(), shiftTable({ id: "t2", number: 8, seats: 4, zone: "Зал", blocked_because: "Дождь" })],
        }),
      }),
      "DELETE /api/admin/blocks": () => ({
        body: {
          reconciliation: nothingMoved,
          reopened: [{ table_id: "t2", reason: "Частное мероприятие" }],
          shift: shift(),
        },
      }),
      "POST /api/admin/blocks": () => ({
        body: { reconciliation: nothingMoved, closed: ["t2"], shift: shift() },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Столы"));
    await user.click(await screen.findByRole("button", { name: "Стол 8" }));
    await user.click(within(await screen.findByRole("dialog")).getByText("Открыть стол снова"));
    await screen.findByText("Стол 8 снова в подборе.");
    await user.click(screen.getByText("Вернуть"));

    await waitFor(() => expect(server.count("POST", "/api/admin/blocks")).toBe(1));
    expect(server.bodies("POST", "/api/admin/blocks")[0]).toEqual({
      service_date: "2026-09-11",
      table_ids: ["t2"],
      reason: "Частное мероприятие",
    });
  });

  it("undoes a change back to the status the server says the booking had", async () => {
    fakeTelegram();
    const asked: Attendance[] = [];
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "PATCH /api/admin/bookings/b1/attendance": ({ body }) => {
        const attendance = (body as { attendance: Attendance }).attendance;
        asked.push(attendance);
        // A colleague seated them a moment ago; this phone still shows them expected.
        return asked.length === 1
          ? attended("no_show", "arrived", 1_280)
          : attended(attendance, "no_show");
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Не пришли"));
    await screen.findByText("Саша: не пришли, стол свободен.");
    await user.click(screen.getByText("Вернуть"));

    await waitFor(() => expect(asked).toEqual(["no_show", "arrived"]));
  });

  it("shows the first read of the shift when a refresh that overtook it fails", async () => {
    const telegram = fakeTelegram();
    const first = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        reads += 1;
        if (reads > 1) return failed;
        await first.opened;
        return { body: shift() };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(1));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    await settle();
    expect(screen.queryByText("Не удалось прочитать смену.")).toBeNull();

    await first.open();
    expect(await screen.findByText("Саша")).toBeDefined();
    expect(screen.queryByText("Не удалось прочитать смену.")).toBeNull();
  });

  it("says the shift could not be read, rather than spinning, once every read of it failed", async () => {
    const telegram = fakeTelegram();
    const first = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        reads += 1;
        if (reads === 1) await first.opened;
        return failed;
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(1));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    await first.open();
    expect(await screen.findByText("Не удалось прочитать смену.")).toBeDefined();
  });

  it("catches up when the app comes back while an older read of the shift is still on its way", async () => {
    const telegram = fakeTelegram();
    const late = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        const read = (reads += 1);
        if (read === 2) await late.opened;
        return { body: read <= 2 ? shift() : shift({ bookings: [] }) };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(3));
    await waitFor(() => expect(screen.queryByText("Саша")).toBeNull());

    await late.open();
    await settle();
    expect(screen.queryByText("Саша")).toBeNull();
  });

  it("says it is reading another evening, not that the last one failed to reread", async () => {
    const telegram = fakeTelegram();
    const nextDay = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async ({ url }) => {
        reads += 1;
        if (reads === 2) return failed;
        if (reads === 3) await nextDay.opened;
        return { body: shift({ service_date: url.searchParams.get("service_date") ?? "" }) };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    await settle();
    await user.click(screen.getByRole("button", { name: "Следующий день" }));

    expect(await screen.findByText("Читаем смену")).toBeDefined();
    expect(screen.queryByText("Не удалось прочитать смену.")).toBeNull();
    await nextDay.open();
  });

  it("shows the times of the evening on screen when a late retry answers for the one left", async () => {
    const telegram = fakeTelegram();
    const written = gate();
    const nextDay = gate();
    const asked: string[] = [];
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        return {
          body:
            date === "2026-09-11"
              ? shift()
              : shift({ service_date: date, now_minutes: null, bookings: [] }),
        };
      },
      "GET /api/admin/availability": async ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        asked.push(date);
        if (date === "2026-09-11") return { body: availability() };
        if (asked.filter((each) => each === date).length === 1) await nextDay.opened;
        return {
          body: availability({
            service_date: date,
            slots: [{ start_minutes: 1_140, state: "free", evening: true }],
            free_count: 1,
          }),
        };
      },
      "POST /api/admin/bookings": async () => {
        await written.opened;
        return { status: 409, body: { error: { code: "no_table_free", message: "taken" } } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Записать"));
    await user.type(await screen.findByPlaceholderText("Как записать"), "Глеб");
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText(/^Записать на 21:30/));

    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByPlaceholderText("Как записать")).toBeNull());
    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    await screen.findByText("сб, 12 сен");
    await user.click(await screen.findByText("Записать гостя"));
    await waitFor(() => expect(asked).toContain("2026-09-12"));

    await written.open();
    await nextDay.open();
    await settle();
    const sheet = screen.getByRole("dialog");
    expect(within(sheet).getByRole("button", { name: "19:00" })).toBeDefined();
    expect(within(sheet).queryByRole("button", { name: "21:30" })).toBeNull();
  });

  it("never shows a failed question's failure for the question a colleague's change replaced it with", async () => {
    const telegram = fakeTelegram();
    const four = gate();
    const later = shiftBooking({ start_minutes: 1_320, end_minutes: 1_440 });
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => {
        reads += 1;
        return { body: shift({ bookings: [reads === 1 ? later : { ...later, party_size: 4 }] }) };
      },
      "GET /api/admin/availability": async ({ url }) => {
        if (url.searchParams.get("party_size") === "2") return failed;
        await four.opened;
        return { body: availability({ party_size: 4 }) };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Перенести"));
    expect(await screen.findByText("Не удалось прочитать свободные окна.")).toBeDefined();

    act(() => telegram.emit("activated"));
    await screen.findByText("Саша · 22:00 · 4 гостя");
    expect(screen.queryByText("Не удалось прочитать свободные окна.")).toBeNull();
    await four.open();
    expect(await screen.findByRole("button", { name: "21:30" })).toBeDefined();
  });
});

describe("booking", () => {
  async function pickTheFirstFreeTime(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByText("Забронировать стол"));
    await user.click(await screen.findByText("21:30"));
    return screen.findByText(/^Забронировать · /);
  }

  it("books once however often the button is pressed while the first press is on its way", async () => {
    fakeTelegram();
    let release: () => void = () => {};
    const server = fakeServer({
      "GET /api/session": () => ({ body: session() }),
      "GET /api/days": () => ({ body: { party_size: 2, days: rail(2) } }),
      "GET /api/availability": () => ({ body: availability() }),
      "POST /api/booking": () =>
        new Promise<Answer>((resolve) => {
          release = () => resolve({ body: { booking, replaced: [] } });
        }),
    });
    const user = userEvent.setup();
    render(<Page />);

    const button = await pickTheFirstFreeTime(user);
    await user.click(button);
    await user.click(button);
    expect(server.count("POST", "/api/booking")).toBe(1);
    release();
  });

  it("confirms a booking that went through even when the home screen cannot be read again", async () => {
    fakeTelegram();
    let sessions = 0;
    fakeServer({
      "GET /api/session": () => (sessions++ === 0 ? { body: session() } : failed),
      "GET /api/days": () => ({ body: { party_size: 2, days: rail(2) } }),
      "GET /api/availability": () => ({ body: availability() }),
      "POST /api/booking": () => ({ body: { booking, replaced: [] } }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await pickTheFirstFreeTime(user));
    expect(await screen.findByText("Стол забронирован")).toBeDefined();
  });

  it("offers a retry for the evenings when they failed, even if the times did not", async () => {
    fakeTelegram();
    fakeServer({
      "GET /api/session": () => ({ body: session() }),
      "GET /api/days": () => failed,
      "GET /api/availability": () => ({ body: availability() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    expect(await screen.findByText("Не удалось прочитать свободные вечера.")).toBeDefined();
    await screen.findByText("21:30");
    expect(screen.getByText("Не удалось прочитать свободные вечера.")).toBeDefined();
  });

  it("does not show the failure for a party the guest stopped bringing while the new one loads", async () => {
    fakeTelegram();
    const held = gate();
    fakeServer({
      "GET /api/session": () => ({ body: session() }),
      "GET /api/days": async ({ url }) => {
        if (url.searchParams.get("party_size") === "2") return failed;
        await held.opened;
        return { body: { party_size: 4, days: rail(2) } };
      },
      "GET /api/availability": () => ({ body: availability() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    expect(await screen.findByText("Не удалось прочитать свободные вечера.")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "4 гостя" }));
    await waitFor(() =>
      expect(screen.queryByText("Не удалось прочитать свободные вечера.")).toBeNull(),
    );
    await held.open();
  });
});

describe("a guest changing their mind", () => {
  async function cancelTheBooking(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByText("Отменить"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    return screen.findByText("Бронь отменена. Стол снова свободен.");
  }

  it("sees the booking gone once cancelled, even when the home screen cannot be read again", async () => {
    fakeTelegram();
    let sessions = 0;
    fakeServer({
      "GET /api/session": () =>
        sessions++ === 0 ? { body: session({ bookings: [booking] }) } : failed,
      "DELETE /api/bookings/b1": () => ({ body: booking }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await cancelTheBooking(user);
    expect(screen.queryByText("Стол ваш")).toBeNull();
    expect(screen.getByText("Вернуть")).toBeDefined();
  });

  it("sees the booking back once restored, even when the home screen cannot be read again", async () => {
    fakeTelegram();
    let sessions = 0;
    fakeServer({
      "GET /api/session": () => {
        sessions += 1;
        if (sessions === 1) return { body: session({ bookings: [booking] }) };
        return sessions === 2 ? { body: session() } : failed;
      },
      "DELETE /api/bookings/b1": () => ({ body: booking }),
      "POST /api/booking": () => ({ body: { booking, replaced: [] } }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await cancelTheBooking(user);
    await user.click(screen.getByText("Вернуть"));
    expect(await screen.findByText("Бронь вернулась.")).toBeDefined();
    expect(screen.getByText("Стол ваш")).toBeDefined();
  });

  it("is told why a restore was refused, even when the home screen cannot be read again", async () => {
    fakeTelegram();
    let sessions = 0;
    fakeServer({
      "GET /api/session": () => {
        sessions += 1;
        if (sessions === 1) return { body: session({ bookings: [booking] }) };
        return sessions === 2 ? { body: session() } : failed;
      },
      "DELETE /api/bookings/b1": () => ({ body: booking }),
      "POST /api/booking": () => ({
        status: 409,
        body: { error: { code: "no_table_free", message: "taken" } },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await cancelTheBooking(user);
    await user.click(screen.getByText("Вернуть"));
    expect(await screen.findByText("Это время только что заняли. Выберите другое.")).toBeDefined();
    await settle();
    expect(screen.getByText("Это время только что заняли. Выберите другое.")).toBeDefined();
  });

  it("is not asked about reminders again once answered, even when the home screen cannot be read again", async () => {
    fakeTelegram();
    let sessions = 0;
    const answered = { opted_in: false, deliverable: true, should_ask: false };
    fakeServer({
      "GET /api/session": () =>
        sessions++ === 0 ? { body: session({ bookings: [booking] }) } : failed,
      "POST /api/reminders/dismiss": () => ({ body: answered }),
      "POST /api/reminders/opt-in": () => ({ body: { ...answered, opted_in: true } }),
    });
    const user = userEvent.setup();
    const { unmount } = render(<Page />);

    await user.click(await screen.findByText("Не нужно"));
    await waitFor(() => expect(screen.queryByText("Напомнить за 3 часа?")).toBeNull());
    unmount();

    sessions = 0;
    render(<Page />);
    await user.click(await screen.findByText("Напомнить"));
    expect(await screen.findByText("Напомним за 3 часа до брони.")).toBeDefined();
    expect(screen.queryByText("Напомнить за 3 часа?")).toBeNull();
  });

  it("does not see a cancelled booking come back when an older reread answers late", async () => {
    const telegram = fakeTelegram();
    const late = gate();
    let sessions = 0;
    const server = fakeServer({
      "GET /api/session": async () => {
        const read = (sessions += 1);
        if (read === 2) await late.opened;
        return { body: read <= 2 ? session({ bookings: [booking] }) : session() };
      },
      "DELETE /api/bookings/b1": () => ({ body: booking }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await screen.findByText("Стол ваш");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(2));
    await cancelTheBooking(user);
    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(3));
    await settle();

    await late.open();
    await settle();
    expect(screen.queryByText("Стол ваш")).toBeNull();
  });

  it("books Friday from the footer while at the table, then cancels each booking from its own card", async () => {
    fakeTelegram();
    const friday: GuestBooking = { ...booking, id: "b2", service_date: "2026-09-12", party_size: 2 };
    let held: GuestBooking[] = [seated];
    const asked: string[] = [];
    const server = fakeServer({
      "GET /api/session": () => ({ body: session({ bookings: held }) }),
      "GET /api/days": () => ({
        body: {
          party_size: 2,
          days: rail(2).map((day) =>
            day.service_date === "2026-09-11" ? { ...day, booked: true } : day,
          ),
        },
      }),
      "GET /api/availability": ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        asked.push(date);
        return { body: availability({ service_date: date }) };
      },
      "POST /api/booking": () => {
        held = [seated, friday];
        return { body: { booking: friday, replaced: [] } };
      },
      "DELETE /api/bookings/b2": () => {
        held = [seated];
        return { body: friday };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    expect(await screen.findByText("ваша бронь")).toBeDefined();
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("Забронировать · 2 гостя · завтра в 21:30"));
    expect(asked).toEqual(["2026-09-12"]);
    expect(await screen.findByText("Стол забронирован")).toBeDefined();

    await user.click(screen.getByText("На главную"));
    expect(await screen.findAllByText("Стол ваш")).toHaveLength(2);
    await user.click(within(screen.getByRole("group", { name: "Завтра в 21:30" })).getByText("Отменить"));
    const sheet = await screen.findByRole("dialog");
    expect(within(sheet).getByText("Завтра в 21:30 · 2 гостя")).toBeDefined();
    await user.click(within(sheet).getByText("Отменить бронь"));
    await screen.findByText("Бронь отменена. Стол снова свободен.");

    expect(server.count("DELETE", "/api/bookings/b2")).toBe(1);
    expect(screen.queryByRole("group", { name: "Завтра в 21:30" })).toBeNull();
    expect(screen.getByRole("group", { name: "Сегодня в 21:00" })).toBeDefined();
  });

  it("moves a no-show whose table is still held to another time tonight, and says it moved", async () => {
    fakeTelegram();
    const noShow: GuestBooking = {
      ...booking,
      id: "b9",
      start_minutes: 1_260,
      end_minutes: 1_380,
      status: "no_show",
      started: true,
      rebooking_replaces: "same_evening",
    };
    const asked: string[] = [];
    fakeServer({
      "GET /api/session": () => ({ body: session({ bookings: [noShow] }) }),
      "GET /api/days": () => ({ body: { party_size: 4, days: rail(2) } }),
      "GET /api/availability": ({ url }) => {
        asked.push(url.searchParams.get("service_date") ?? "");
        return { body: availability() };
      },
      "POST /api/booking": () => ({ body: { booking: { ...booking, id: "b3" }, replaced: ["b9"] } }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Перенести"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("Перенести · 4 гостя · сегодня в 21:30"));
    expect(await screen.findByText("Бронь перенесена")).toBeDefined();
    expect(asked).toEqual(["2026-09-11"]);
  });

  it("never lets a guest at the table book tonight again", async () => {
    fakeTelegram();
    const server = fakeServer({
      "GET /api/session": () =>
        ({ body: session({ bookings: [seated], bookable_days: ["2026-09-11"] }) }),
      "GET /api/days": () => ({ body: { party_size: 2, days: [{ ...rail(1)[0], booked: true }] } }),
      "GET /api/availability": () => ({ body: availability() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    expect(await screen.findByText("ваша бронь")).toBeDefined();
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("На этот вечер у вас уже есть бронь"));
    await settle();
    expect(server.count("POST", "/api/booking")).toBe(0);
  });
});

describe("settings", () => {
  it("keep an unsaved change through a look at the shift, and ask before closing meanwhile", async () => {
    const telegram = fakeTelegram();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: settingsView() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await waitFor(() => expect(telegram.app.enableClosingConfirmation).toHaveBeenCalled());

    await user.click(screen.getByText("Смена"));
    await screen.findByText("Саша");
    const last = (confirmation: { mock: { invocationCallOrder: number[] } }) =>
      Math.max(0, ...confirmation.mock.invocationCallOrder);
    expect(last(telegram.app.enableClosingConfirmation)).toBeGreaterThan(
      last(telegram.app.disableClosingConfirmation),
    );

    await user.click(screen.getByText("Настройки"));
    expect(await screen.findByDisplayValue("Чердак")).toBeDefined();
  });

  it("keep an edit typed while the settings are being read again", async () => {
    fakeTelegram();
    const reread = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        reads += 1;
        if (reads > 1) await reread.opened;
        return { body: settingsView() };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    await screen.findByPlaceholderText("Название");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await waitFor(() => expect(server.count("GET", "/api/admin/settings")).toBe(2));

    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await reread.open();
    await settle();
    expect(screen.getByDisplayValue("Чердак")).toBeDefined();
  });

  it("take a reread whole when the change made meanwhile changed nothing", async () => {
    const telegram = fakeTelegram();
    const reread = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        reads += 1;
        if (reads === 1) return { body: settingsView() };
        await reread.opened;
        return { body: settingsView({ name: "Чердак", version: "2026-09-13T09:00:00Z" }) };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Правила бронирования"));
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await waitFor(() => expect(server.count("GET", "/api/admin/settings")).toBe(2));

    await user.click(await screen.findByRole("tab", { name: "30 мин" }));
    await reread.open();
    await settle();
    act(() => telegram.pressBack());
    expect(await screen.findByText("Чердак · ул. Рубинштейна, 24")).toBeDefined();
    expect(screen.queryByText("Сохранить")).toBeNull();
  });

  it("keep what was just saved when an older reread answers after the save", async () => {
    fakeTelegram();
    const reread = gate();
    let reads = 0;
    let stored = settingsView();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        const answer = stored;
        reads += 1;
        if (reads > 1) await reread.opened;
        return { body: answer };
      },
      "PUT /api/admin/settings": () => {
        stored = settingsView({ name: "Чердак" });
        return { body: { settings: stored, reconciliation: nothingMoved, above_cap: 0 } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    await screen.findByPlaceholderText("Название");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));

    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await user.click(await screen.findByText("Сохранить"));
    await screen.findByText("Настройки сохранены.");

    await reread.open();
    await settle();
    expect(screen.getByDisplayValue("Чердак")).toBeDefined();
    expect(screen.queryByText("Сохранить")).toBeNull();
  });

  it("keep a change typed while the save is on its way, on top of what was saved", async () => {
    fakeTelegram();
    const saving = gate();
    const sent: SettingsDraft[] = [];
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: settingsView({ version: "v1" }) }),
      "PUT /api/admin/settings": async ({ body }) => {
        const draft = body as SettingsDraft;
        sent.push(draft);
        if (sent.length === 1) await saving.opened;
        return {
          body: {
            settings: settingsView({
              name: draft.name,
              address: draft.address,
              version: `v${sent.length + 1}`,
            }),
            reconciliation: nothingMoved,
            above_cap: 0,
          },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await user.click(await screen.findByText("Сохранить"));
    await waitFor(() => expect(sent).toHaveLength(1));

    const address = screen.getByPlaceholderText("Адрес");
    await user.clear(address);
    await user.type(address, "Невский, 1");
    await saving.open();
    await screen.findByText("Настройки сохранены.");
    await settle();
    expect(screen.getByDisplayValue("Невский, 1")).toBeDefined();
    expect(screen.getByDisplayValue("Чердак")).toBeDefined();

    await user.click(screen.getByText("Сохранить"));
    await waitFor(() => expect(sent).toHaveLength(2));
    expect(sent[0]).toMatchObject({ name: "Чердак", address: "ул. Рубинштейна, 24", version: "v1" });
    expect(sent[1]).toMatchObject({ name: "Чердак", address: "Невский, 1", version: "v2" });
  });

  it("carry a change to a table added in the save over to the table the save created, not a second one", async () => {
    fakeTelegram();
    const saving = gate();
    const sent: SettingsDraft[] = [];
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: settingsView({ version: "v1" }) }),
      "PUT /api/admin/settings": async ({ body }) => {
        const draft = body as SettingsDraft;
        sent.push(draft);
        if (sent.length === 1) await saving.opened;
        const stored = settingsView();
        return {
          body: {
            settings: settingsView({
              tables: draft.tables.map((table, index) =>
                table.kind === "existing"
                  ? { ...stored.tables.find((each) => each.id === table.id)!, seats: table.seats }
                  : { id: `new${index}`, number: 9, seats: table.seats, zone: table.zone, bookings_today: 0 },
              ),
              next_table_number: 10,
              version: `v${sent.length + 1}`,
            }),
            reconciliation: nothingMoved,
            above_cap: 0,
          },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Зал"));
    await user.click(await screen.findByText("+ Добавить стол"));
    await user.click(await screen.findByText("Сохранить"));
    await waitFor(() => expect(sent).toHaveLength(1));

    await user.click(screen.getByRole("button", { name: "Стол 9: больше мест" }));
    await saving.open();
    await screen.findByText("Настройки сохранены.");
    await user.click(await screen.findByText("Сохранить"));
    await waitFor(() => expect(sent).toHaveLength(2));
    expect(sent[1]?.tables).toEqual([
      { kind: "existing", id: "t1", seats: 2, zone: "Стойка" },
      { kind: "existing", id: "t2", seats: 6, zone: "Зал" },
      { kind: "existing", id: "new2", seats: 5, zone: "Зал" },
    ]);
  });

  it("fold what somebody else saved into the edit a stale save was refused for, and save it on their version", async () => {
    fakeTelegram();
    let reads = 0;
    const sent: SettingsDraft[] = [];
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => {
        reads += 1;
        return {
          body:
            reads === 1
              ? settingsView({ version: "2026-09-13T08:00:00Z" })
              : settingsView({ address: "Невский, 1", version: "2026-09-13T09:00:00Z" }),
        };
      },
      "PUT /api/admin/settings": ({ body }) => {
        const draft = body as SettingsDraft;
        sent.push(draft);
        if (sent.length === 1) {
          return { status: 409, body: { error: { code: "settings_changed", message: "changed" } } };
        }
        return {
          body: {
            settings: settingsView({ name: draft.name, address: draft.address, version: "2026-09-13T10:00:00Z" }),
            reconciliation: nothingMoved,
            above_cap: 0,
          },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await user.click(await screen.findByText("Сохранить"));

    expect(
      await screen.findByText(
        "Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.",
      ),
    ).toBeDefined();
    expect(screen.getByDisplayValue("Чердак")).toBeDefined();
    expect(screen.getByDisplayValue("Невский, 1")).toBeDefined();

    await user.click(screen.getByText("Сохранить"));
    await waitFor(() => expect(sent).toHaveLength(2));
    expect(sent[1]).toMatchObject({
      name: "Чердак",
      address: "Невский, 1",
      version: "2026-09-13T09:00:00Z",
    });
  });

  it("keep what was typed while the reread after a stale save loads, and name the field both changed", async () => {
    fakeTelegram();
    const reread = gate();
    let reads = 0;
    const sent: SettingsDraft[] = [];
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        reads += 1;
        if (reads === 1) return { body: settingsView({ version: "2026-09-13T08:00:00Z" }) };
        await reread.opened;
        return { body: settingsView({ name: "Подвал", version: "2026-09-13T09:00:00Z" }) };
      },
      "PUT /api/admin/settings": ({ body }) => {
        sent.push(body as SettingsDraft);
        return sent.length === 1
          ? { status: 409, body: { error: { code: "settings_changed", message: "changed" } } }
          : {
              body: {
                settings: settingsView({ name: "Мансарда", version: "2026-09-13T10:00:00Z" }),
                reconciliation: nothingMoved,
                above_cap: 0,
              },
            };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await user.click(await screen.findByText("Сохранить"));
    await waitFor(() => expect(server.count("GET", "/api/admin/settings")).toBe(2));

    await user.clear(name);
    await user.type(name, "Мансарда");
    await reread.open();
    expect(
      await screen.findByText(
        "Пока вы редактировали, кто-то изменил настройки: название. Оставили ваши значения — проверьте и сохраните.",
      ),
    ).toBeDefined();
    expect(screen.getByDisplayValue("Мансарда")).toBeDefined();

    await user.click(screen.getByText("Сохранить"));
    await waitFor(() => expect(sent).toHaveLength(2));
    expect(sent[1]).toMatchObject({ name: "Мансарда", version: "2026-09-13T09:00:00Z" });
  });

  it("never put an older save's answer over newer settings read after «Вернуть»", async () => {
    fakeTelegram();
    const saving = gate();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => {
        reads += 1;
        return {
          body:
            reads === 1
              ? settingsView({ version: "2026-09-13T08:00:00Z" })
              : settingsView({ name: "Подвал", version: "2026-09-13T10:00:00Z" }),
        };
      },
      "PUT /api/admin/settings": async () => {
        await saving.opened;
        return {
          body: {
            settings: settingsView({ name: "Чердак", version: "2026-09-13T09:00:00Z" }),
            reconciliation: nothingMoved,
            above_cap: 0,
          },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await user.click(await screen.findByText("Сохранить"));
    await user.click(await screen.findByText("Вернуть"));
    await waitFor(() => expect(screen.queryByText("Вернуть")).toBeNull());

    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    expect(await screen.findByDisplayValue("Подвал")).toBeDefined();

    await saving.open();
    await screen.findByText("Настройки сохранены.");
    await settle();
    expect(screen.getByDisplayValue("Подвал")).toBeDefined();
    expect(screen.queryByText("Сохранить")).toBeNull();
  });

  it("leave the day on screen's table counts when a save made for another day answers after the switch", async () => {
    fakeTelegram();
    const saving = gate();
    let version = "2026-09-13T08:00:00Z";
    let seats = 2;
    const view = (date: string | null) =>
      settingsView({
        version,
        tables: [
          { id: "t1", number: 7, seats, zone: "Стойка", bookings_today: date === "2026-09-12" ? 3 : 0 },
          { id: "t2", number: 8, seats: 6, zone: "Зал", bookings_today: 0 },
        ],
      });
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": ({ url }) => ({
        body: shift({ service_date: url.searchParams.get("service_date") ?? "" }),
      }),
      "GET /api/admin/settings": ({ url }) => ({ body: view(url.searchParams.get("service_date")) }),
      "PUT /api/admin/settings": async ({ body, url }) => {
        await saving.opened;
        seats = (body as SettingsDraft).tables[0]?.seats ?? seats;
        version = "2026-09-13T09:00:00Z";
        return {
          body: {
            settings: view(url.searchParams.get("service_date")),
            reconciliation: nothingMoved,
            above_cap: 0,
          },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Зал"));
    await user.click(await screen.findByRole("button", { name: "Стол 7: больше мест" }));
    await user.click(await screen.findByText("Сохранить"));

    await user.click(screen.getByText("Смена"));
    await user.click(await screen.findByRole("button", { name: "Следующий день" }));
    await screen.findByText("сб, 12 сен");
    await user.click(screen.getByText("Настройки"));
    expect(await screen.findByText("3 брони")).toBeDefined();

    await saving.open();
    await screen.findByText("Настройки сохранены.");
    await settle();
    expect(screen.getByText("3 брони")).toBeDefined();
    expect(screen.queryByText("Сохранить")).toBeNull();
  });

  it("report a refused save in words alone when a sheet was opened while it was on its way", async () => {
    fakeTelegram();
    const saving = gate();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: settingsView() }),
      "PUT /api/admin/settings": async () => {
        await saving.opened;
        return {
          status: 422,
          body: {
            error: {
              code: "settings_invalid",
              message: "invalid",
              detail: { reasons: ["Стол 1: столько мест не бывает."] },
            },
          },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await user.click(await screen.findByText("Сохранить"));

    await user.click(screen.getByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    expect(await screen.findByText("Как прошло")).toBeDefined();

    await saving.open();
    expect(await screen.findByText("Так сохранить нельзя — проверьте отмеченные значения.")).toBeDefined();
    await settle();
    expect(screen.getByText("Как прошло")).toBeDefined();
    expect(screen.queryByText("Стол 1: столько мест не бывает.")).toBeNull();
  });

  it("confirm a save even when the session cannot be read again afterwards", async () => {
    fakeTelegram();
    let sessions = 0;
    fakeServer({
      "GET /api/session": () => (sessions++ === 0 ? staffSession() : failed),
      "GET /api/admin/settings": () => ({ body: settingsView() }),
      "PUT /api/admin/settings": () => ({
        body: {
          settings: settingsView({ name: "Чердак" }),
          reconciliation: nothingMoved,
          above_cap: 0,
        },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак");
    await user.click(await screen.findByText("Сохранить"));
    expect(await screen.findByText("Настройки сохранены.")).toBeDefined();
    await settle();
    expect(screen.getByText("Настройки сохранены.")).toBeDefined();
  });
});
