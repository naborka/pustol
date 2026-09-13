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
  session,
  settingsView,
  shift,
  shiftBooking,
} from "@/components/__tests__/fixtures";
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
  const calls: { method: string; path: string }[] = [];
  const fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const url = new URL(input, "http://app.test");
    const method = init?.method ?? "GET";
    calls.push({ method, path: url.pathname });
    const handler = routes[`${method} ${url.pathname}`];
    const { status = 200, body } = handler
      ? await handler({ body: init?.body ? JSON.parse(String(init.body)) : undefined, url })
      : { status: 404, body: { error: { code: "not_found", message: url.pathname } } };
    return new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  });
  vi.stubGlobal("fetch", fetch);
  return {
    count: (method: string, path: string) =>
      calls.filter((call) => call.method === method && call.path === path).length,
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

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  delete window.Telegram;
});

describe("the first paint", () => {
  it("does not tell a guest to open the app from Telegram before it has even looked", () => {
    // The page is prerendered where there is no Telegram at all, and that HTML is what a guest sees
    // until the scripts have loaded.
    const html = renderToString(<Page />);
    expect(html).not.toContain("Откройте приложение из Telegram");
  });

  it("opens even when a refresh fires while the app is first being read", async () => {
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
    await first.open();

    expect(await screen.findByText("Столик на вечер")).toBeDefined();
    expect(server.count("GET", "/api/session")).toBe(1);
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
        return { body: shiftBooking({ note: "У окна" }) };
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
      "PATCH /api/admin/bookings/b1/attendance": ({ body }) => ({
        body: shiftBooking({ status: (body as { attendance: "arrived" }).attendance }),
      }),
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
    const seated = gate();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        return { body: date === "2026-09-11" ? shift() : shift({ service_date: date, bookings: [] }) };
      },
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seated.opened;
        return { body: shiftBooking({ status: "arrived" }) };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    await screen.findByText("сб, 12 сен");

    await seated.open();
    await screen.findByText("Саша за столом 7.");
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(3));
    await settle();
    expect(screen.getByText("сб, 12 сен")).toBeDefined();
    expect(screen.queryByText("Саша")).toBeNull();
  });

  it("does nothing, and keeps the table open, when a table is closed while another action runs", async () => {
    fakeTelegram();
    const seated = gate();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seated.opened;
        return { body: shiftBooking({ status: "arrived" }) };
      },
      "POST /api/admin/blocks": () => ({ body: nothingMoved }),
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
    await seated.open();
  });

  it("finishes reading the shift when a refresh fires while it is first being read", async () => {
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
    await first.open();

    expect(await screen.findByText("Саша")).toBeDefined();
    expect(server.count("GET", "/api/admin/shift")).toBe(1);
  });

  it("says it is reading another evening, not that the last one failed to reread", async () => {
    fakeTelegram();
    const nextDay = gate();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async ({ url }) => {
        reads += 1;
        if (reads === 2) return failed;
        if (reads === 3) await nextDay.opened;
        return { body: shift({ service_date: url.searchParams.get("service_date") ?? "" }) };
      },
      "PATCH /api/admin/bookings/b1/attendance": () => ({
        body: shiftBooking({ status: "arrived" }),
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    await screen.findByText("Саша за столом 7.");
    await user.click(screen.getByRole("button", { name: "Следующий день" }));

    expect(await screen.findByText("Читаем смену")).toBeDefined();
    expect(screen.queryByText("Не удалось прочитать смену.")).toBeNull();
    await nextDay.open();
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
          release = () => resolve({ body: { booking, replaced: null } });
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
      "POST /api/booking": () => ({ body: { booking, replaced: null } }),
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
      "GET /api/session": () => (sessions++ === 0 ? { body: session({ booking }) } : failed),
      "DELETE /api/booking": () => ({ body: booking }),
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
        if (sessions === 1) return { body: session({ booking }) };
        return sessions === 2 ? { body: session() } : failed;
      },
      "DELETE /api/booking": () => ({ body: booking }),
      "POST /api/booking": () => ({ body: { booking, replaced: null } }),
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
        if (sessions === 1) return { body: session({ booking }) };
        return sessions === 2 ? { body: session() } : failed;
      },
      "DELETE /api/booking": () => ({ body: booking }),
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
      "GET /api/session": () => (sessions++ === 0 ? { body: session({ booking }) } : failed),
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
        return { body: read <= 2 ? session({ booking }) : session() };
      },
      "DELETE /api/booking": () => ({ body: booking }),
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
