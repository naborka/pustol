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
  heldNoShow,
  rail,
  seated,
  session,
  settingsView,
  shift,
  shiftBooking,
  shiftTable,
} from "@/components/__tests__/fixtures";
import type {
  Attendance,
  GuestBooking,
  SettingsDraft,
  SettingsView,
  ShiftBooking,
} from "@/lib/api";
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

const expired: Answer = { status: 401, body: { error: { code: "session_expired", message: "old" } } };

const EXPIRED = "Сессия устарела. Закройте и откройте приложение — всё сохранится.";

/** What a failed read said as a toast, which no read may say any more. */
const BROKEN = "Что-то сломалось у нас. Попробуйте ещё раз через минуту.";

const STALE = "Не удалось обновить — показано прежнее.";

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

/**
 * A bar's settings as the server keeps them: a save made from settings somebody has saved since is
 * refused, every save moves the version on, and a table is found by the id the app gave it or made
 * with that id.
 */
function settingsServer() {
  let stored = settingsView();
  const read = () => stored;
  return {
    get current() {
      return stored;
    },
    read,
    /** Somebody else's save. */
    change(overrides: Partial<SettingsView>) {
      stored = { ...stored, ...overrides, version: stored.version + 1 };
    },
    /** A save, answered with what it stored, listed as a read lists it. */
    save(draft: SettingsDraft): Answer {
      if (draft.version !== stored.version) {
        return { status: 409, body: { error: { code: "settings_changed", message: "changed" } } };
      }
      let number = stored.next_table_number;
      const tables = draft.tables
        .map((table) => {
          const known = stored.tables.find((each) => each.id === table.id);
          return {
            id: table.id,
            number: known?.number ?? number++,
            seats: table.seats,
            zone: table.zone,
          };
        })
        .sort((left, right) => left.number - right.number);
      const lower = (username: string) => username.toLowerCase();
      const staff = draft.staff
        .map(({ username }) => ({
          username,
          bound: stored.staff.some((member) => lower(member.username) === lower(username) && member.bound),
        }))
        .sort((left, right) => lower(left.username).localeCompare(lower(right.username), "en"));
      stored = {
        ...stored,
        name: draft.name.trim(),
        address: draft.address.trim(),
        contact: draft.contact.trim(),
        timezone: draft.timezone,
        week: draft.week,
        zones: draft.zones,
        tables,
        turn_minutes: draft.turn_minutes,
        slot_step_minutes: draft.slot_step_minutes,
        max_party: draft.max_party,
        horizon_days: draft.horizon_days,
        remind_hours: draft.remind_hours,
        grace_minutes: draft.grace_minutes,
        message_templates: draft.message_templates.map((text) => text.trim()),
        cancel_reasons: draft.cancel_reasons.map((text) => text.trim()),
        staff,
        next_table_number: number,
        version: stored.version + 1,
      };
      return { body: { settings: read(), reconciliation: nothingMoved, above_cap: 0 } };
    },
  };
}

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

  it("stays shut once the session has ended, even when a cancellation sent before answers", async () => {
    const telegram = fakeTelegram();
    const cancelling = gate();
    const reread = gate();
    let sessions = 0;
    fakeServer({
      "GET /api/session": async () => {
        sessions += 1;
        if (sessions === 1) return { body: session({ bookings: [booking] }) };
        if (sessions > 2) await reread.opened;
        return expired;
      },
      "DELETE /api/bookings/b1": async () => {
        await cancelling.opened;
        return { body: booking };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Отменить"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    act(() => telegram.emit("activated"));
    expect(await screen.findByText(EXPIRED)).toBeDefined();

    await cancelling.open();
    await settle();
    expect(screen.queryByText("Столик на вечер")).toBeNull();
    expect(screen.queryByText("Стол ваш")).toBeNull();
    await reread.open();
    expect(await screen.findByText(EXPIRED)).toBeDefined();
  });

  it("shuts at once when the session ends while a read of it asked before is still on its way", async () => {
    // Only a read asked after the end can bring the app back, so only such a read may spin over it.
    const telegram = fakeTelegram();
    const hanging = gate();
    let sessions = 0;
    fakeServer({
      "GET /api/session": async () => {
        sessions += 1;
        if (sessions === 2) await hanging.opened;
        return { body: session({ bookings: [booking] }) };
      },
      "DELETE /api/bookings/b1": () => expired,
    });
    const user = userEvent.setup();
    render(<Page />);

    await screen.findByText("Стол ваш");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(sessions).toBe(2));
    await user.click(screen.getByText("Отменить"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    expect(await screen.findByText(EXPIRED)).toBeDefined();

    await hanging.open();
    await settle();
    expect(screen.getByText(EXPIRED)).toBeDefined();
  });

  it("asks nothing by itself once the session has ended, and keeps saying so", async () => {
    // A quiet refresh started after the end spun over the screen that says to reopen the app.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const telegram = fakeTelegram();
    const server = fakeServer({
      "GET /api/session": () => ({ body: session({ bookings: [booking] }) }),
      "DELETE /api/bookings/b1": () => expired,
    });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<Page />);

    await user.click(await screen.findByText("Отменить"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    expect(await screen.findByText(EXPIRED)).toBeDefined();
    const asked = server.count("GET", "/api/session");

    act(() => telegram.emit("activated"));
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    await settle();
    expect(server.count("GET", "/api/session")).toBe(asked);
    expect(screen.getByText(EXPIRED)).toBeDefined();
    expect(screen.getByText("Закрыть")).toBeDefined();
    expect(screen.queryByText("Открываем")).toBeNull();
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
        const running = date === "2026-09-12";
        return {
          body: shift({
            service_date: date,
            today: "2026-09-12",
            now_minutes: running ? 1_280 : null,
            walk_in_until_minutes: running ? 1_400 : null,
            walk_in_free_table_ids: running ? ["t2", "t3"] : [],
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

  it("offers no seat-now on tonight's evening while the server takes no party at the door", async () => {
    // Before the doors open, or between the close and the next shift.
    fakeTelegram();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({
        body: shift({ walk_in_until_minutes: null, walk_in_free_table_ids: [] }),
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    expect(await screen.findByText("Записать гостя")).toBeDefined();
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

  it("promises no message to a guest the bot cannot reach, before cancelling or after", async () => {
    fakeTelegram();
    const unreachable = shiftBooking({ reachable_by_bot: false });
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift({ bookings: [unreachable] }) }),
      "POST /api/admin/bookings/b1/cancel": () => ({
        body: {
          booking: { ...unreachable, status: "cancelled" },
          reconciliation: nothingMoved,
          guest_notified: false,
          shift: shift({ bookings: [], version: 2 }),
        },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    expect(within(await screen.findByRole("dialog")).getByText("Бот не может написать гостю")).toBeDefined();
    await user.click(within(screen.getByRole("dialog")).getByText("Отменить бронь"));
    const sheet = await screen.findByRole("dialog");
    expect(
      within(sheet).getByText(
        "Боту некуда написать гостю — предупредите его сами. Стол сразу освободится. Отменить это нельзя.",
      ),
    ).toBeDefined();
    await user.click(within(sheet).getByText("Дождь"));
    expect(
      await screen.findByText("Бронь отменена. Саша не получит сообщения — предупредите его сами."),
    ).toBeDefined();
    expect(screen.queryByText(/получит сообщение/)).toBeNull();
  });

  it("says a guest the bot cannot reach was not told their new time", async () => {
    fakeTelegram();
    const unreachable = shiftBooking({ reachable_by_bot: false });
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift({ bookings: [unreachable] }) }),
      "GET /api/admin/availability": () => ({ body: availability() }),
      "PATCH /api/admin/bookings/b1/move": () => {
        const moved = { ...unreachable, start_minutes: 1_290, end_minutes: 1_410 };
        return {
          body: {
            booking: moved,
            reconciliation: nothingMoved,
            guest_notified: false,
            shift: shift({ bookings: [moved], version: 2 }),
          },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Перенести"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("Перенести на 21:30, стол 7"));
    expect(
      await screen.findByText("Саша: 21:30, стол 7. Бот не может написать гостю — предупредите сами."),
    ).toBeDefined();
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
    expect(screen.getByText(STALE)).toBeDefined();
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
        return { body: read <= 2 ? shift() : shift({ bookings: [], version: 2 }) };
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

  it("shows a refresh that answers after a failed reread of the same evening", async () => {
    const telegram = fakeTelegram();
    const tick = gate();
    const asked: string[] = [];
    const tomorrow = () => asked.filter((date) => date === "2026-09-12").length;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        asked.push(date);
        if (date === "2026-09-11") return { body: shift() };
        const read = tomorrow();
        if (read === 1) return { body: shift({ service_date: date, bookings: [] }) };
        if (read === 2) {
          await tick.opened;
          return { body: shift({ service_date: date, bookings: [timur], version: 2 }) };
        }
        return failed;
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    await screen.findByText("сб, 12 сен");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(tomorrow()).toBe(2));
    await user.click(screen.getByRole("button", { name: "Предыдущий день" }));
    await screen.findByText("Саша");
    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    await waitFor(() => expect(tomorrow()).toBe(3));
    await settle();

    await tick.open();
    expect(await screen.findByText("Тимур")).toBeDefined();
    expect(screen.queryByText("Не удалось прочитать смену.")).toBeNull();
  });

  it("shows a colleague's booking a refresh brings in a newer room than a write's, and drops an older room", async () => {
    const telegram = fakeTelegram();
    const newer = gate();
    const older = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        reads += 1;
        if (reads === 2) {
          await newer.opened;
          return { body: shift({ version: 3, bookings: [shiftBooking({ status: "arrived" }), timur] }) };
        }
        if (reads === 3) {
          await older.opened;
          return { body: shift({ version: 1 }) };
        }
        return { body: shift() };
      },
      "PATCH /api/admin/bookings/b1/attendance": () => {
        const now = shiftBooking({ status: "arrived" });
        return {
          body: { booking: now, previous: "confirmed", shift: shift({ version: 2, bookings: [now] }) },
        };
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
    await user.click(screen.getByText("Посадить"));
    await screen.findByText("Саша за столом 7.");

    await newer.open();
    expect(await screen.findByText("Тимур")).toBeDefined();
    await older.open();
    await settle();
    expect(screen.getByText("Тимур")).toBeDefined();
  });

  it("says on the screen, never in a toast, when an evening cannot be read or read again", async () => {
    const telegram = fakeTelegram();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => (reads++ === 0 ? { body: shift() } : failed),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    act(() => telegram.emit("activated"));
    expect(await screen.findByText(STALE)).toBeDefined();
    expect(screen.getByText("Саша")).toBeDefined();
    expect(screen.queryByText(BROKEN)).toBeNull();

    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    expect(await screen.findByText("Не удалось прочитать смену.")).toBeDefined();
    expect(screen.queryByText(STALE)).toBeNull();
    expect(screen.queryByText(BROKEN)).toBeNull();
  });

  it("says nothing once a read asked after a failed one answers, even with a room older than a write's own", async () => {
    const telegram = fakeTelegram();
    const seating = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => {
        reads += 1;
        if (reads === 2) return failed;
        return {
          body: reads === 1 ? shift() : shift({ version: 2, bookings: [shiftBooking({ status: "arrived" })] }),
        };
      },
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seating.opened;
        const now = shiftBooking({ status: "arrived" });
        return { body: { booking: now, previous: "confirmed", shift: shift({ version: 3, bookings: [now] }) } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    act(() => telegram.emit("activated"));
    expect(await screen.findByText(STALE)).toBeDefined();
    await seating.open();
    await screen.findByText("Саша за столом 7.");

    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(3));
    await settle();
    expect(screen.queryByText(STALE)).toBeNull();
  });

  it("says in its own words that somebody taken off the staff list cannot read the shift, and offers no retry", async () => {
    const telegram = fakeTelegram();
    const forbidden: Answer = { status: 403, body: { error: { code: "forbidden", message: "not staff" } } };
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => (reads++ === 0 ? { body: shift() } : forbidden),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    act(() => telegram.emit("activated"));
    expect(await screen.findByText("Этот раздел только для сотрудников бара.")).toBeDefined();
    expect(screen.getByText("Саша")).toBeDefined();
    expect(screen.queryByText(STALE)).toBeNull();
    expect(screen.queryByText("Повторить")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    await waitFor(() => expect(screen.queryByText("Саша")).toBeNull());
    expect(await screen.findByText("Этот раздел только для сотрудников бара.")).toBeDefined();
    expect(screen.queryByText("Не удалось прочитать смену.")).toBeNull();
    expect(screen.queryByText("Попробовать снова")).toBeNull();
  });

  it("offers to read the evening on screen again when a refresh of it failed, and shows what that brings", async () => {
    const telegram = fakeTelegram();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => {
        reads += 1;
        if (reads === 2) return failed;
        return { body: reads === 1 ? shift() : shift({ version: 2, bookings: [shiftBooking(), timur] }) };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    act(() => telegram.emit("activated"));
    await screen.findByText(STALE);
    await user.click(screen.getByText("Повторить"));
    expect(await screen.findByText("Тимур")).toBeDefined();
    expect(screen.queryByText(STALE)).toBeNull();
  });

  it("says nothing when a read of an evening fails while a newer read of it is on its way, and nothing once that lands", async () => {
    const telegram = fakeTelegram();
    const failing = gate();
    const loading = gate();
    let saturdays = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        if (date === "2026-09-11") return { body: shift() };
        saturdays += 1;
        if (saturdays === 1) {
          await failing.opened;
          return failed;
        }
        await loading.opened;
        return { body: shift({ service_date: date, bookings: [timur] }) };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    await user.click(screen.getByRole("button", { name: "Следующий день" }));
    await waitFor(() => expect(saturdays).toBe(1));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(saturdays).toBe(2));

    await failing.open();
    await settle();
    expect(screen.queryByText(BROKEN)).toBeNull();
    expect(screen.queryByText(STALE)).toBeNull();
    expect(screen.getByText("Читаем смену")).toBeDefined();

    await loading.open();
    expect(await screen.findByText("Тимур")).toBeDefined();
    await settle();
    expect(screen.queryByText(BROKEN)).toBeNull();
    expect(screen.queryByText(STALE)).toBeNull();
  });

  it("says nothing on the settings when the shift it was asked for fails after the switch", async () => {
    fakeTelegram();
    const reading = gate();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        await reading.opened;
        return failed;
      },
      "GET /api/admin/settings": () => ({ body: settingsView() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Читаем смену");
    await user.click(screen.getByText("Настройки"));
    await screen.findByText("Правила бронирования");
    await reading.open();
    await settle();
    expect(screen.queryByText(BROKEN)).toBeNull();
    expect(screen.queryByText(STALE)).toBeNull();
  });

  it("keeps the room asked later when two refreshes of one version answer in the wrong order", async () => {
    // A room's version does not move with the clock or with whether the bot can reach a guest.
    const telegram = fakeTelegram();
    const older = gate();
    const newer = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        const read = (reads += 1);
        if (read === 2) {
          await older.opened;
          return { body: shift({ version: 5, now_minutes: 1_290 }) };
        }
        if (read === 3) {
          await newer.opened;
          return {
            body: shift({
              version: 5,
              now_minutes: 1_300,
              bookings: [shiftBooking({ reachable_by_bot: false })],
            }),
          };
        }
        return { body: shift({ version: 4 }) };
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

    await newer.open();
    expect(await screen.findByText(/^21:40 ·/)).toBeDefined();
    await older.open();
    await settle();
    expect(screen.getByText(/^21:40 ·/)).toBeDefined();
    await user.click(screen.getByText("Саша"));
    expect(within(await screen.findByRole("dialog")).getByText("Бот не может написать гостю")).toBeDefined();
  });

  it("keeps a refresh asked after a write was sent over the write's own room of the same version", async () => {
    const telegram = fakeTelegram();
    const seating = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => {
        reads += 1;
        const arrived = shiftBooking({ status: "arrived" });
        return { body: reads === 1 ? shift() : shift({ version: 2, now_minutes: 1_300, bookings: [arrived] }) };
      },
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seating.opened;
        const now = shiftBooking({ status: "arrived" });
        return { body: { booking: now, previous: "confirmed", shift: shift({ version: 2, bookings: [now] }) } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    expect(await screen.findByText(/^21:40 ·/)).toBeDefined();

    await seating.open();
    await screen.findByText("Саша за столом 7.");
    await settle();
    expect(screen.getByText(/^21:40 ·/)).toBeDefined();
  });

  it("admits no failure of a refresh asked before one that answered, even with a room too old to show", async () => {
    const telegram = fakeTelegram();
    const seating = gate();
    const second = gate();
    const third = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        const read = (reads += 1);
        if (read === 2) {
          await second.opened;
          return failed;
        }
        if (read === 3) {
          await third.opened;
          return { body: shift({ version: 2 }) };
        }
        return { body: shift() };
      },
      "PATCH /api/admin/bookings/b1/attendance": async () => {
        await seating.opened;
        const now = shiftBooking({ status: "arrived" });
        return { body: { booking: now, previous: "confirmed", shift: shift({ version: 3, bookings: [now] }) } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Посадить"));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(3));

    await seating.open();
    await screen.findByText("Саша за столом 7.");
    await third.open();
    await settle();
    await second.open();
    await settle();
    expect(screen.queryByText(STALE)).toBeNull();
  });

  it("keeps a write's own room over a refresh of the same version asked before the write was sent", async () => {
    const telegram = fakeTelegram();
    const refresh = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async () => {
        reads += 1;
        if (reads === 1) return { body: shift() };
        await refresh.opened;
        return { body: shift({ version: 2, now_minutes: 1_290, bookings: [shiftBooking({ status: "arrived" })] }) };
      },
      "PATCH /api/admin/bookings/b1/attendance": () => {
        const now = shiftBooking({ status: "arrived" });
        return {
          body: { booking: now, previous: "confirmed", shift: shift({ version: 2, now_minutes: 1_300, bookings: [now] }) },
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await screen.findByText("Саша");
    act(() => telegram.emit("activated"));
    await waitFor(() => expect(server.count("GET", "/api/admin/shift")).toBe(2));
    await user.click(screen.getByText("Посадить"));
    expect(await screen.findByText(/^21:40 ·/)).toBeDefined();

    await refresh.open();
    await settle();
    expect(screen.getByText(/^21:40 ·/)).toBeDefined();
  });

  it("offers no message to a guest the bot can no longer reach, and says why a refused one was not sent", async () => {
    fakeTelegram();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => {
        reads += 1;
        return { body: reads === 1 ? shift() : shift({ bookings: [shiftBooking({ reachable_by_bot: false })] }) };
      },
      "POST /api/admin/bookings/b1/message": () => ({
        status: 400,
        body: { error: { code: "no_bot_chat", message: "the bot cannot write to this guest" } },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Написать гостю"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Ваш стол готов"));
    expect(
      await screen.findByText("Бот не может написать этому гостю — позвоните или откройте чат."),
    ).toBeDefined();
    const sheet = screen.getByRole("dialog");
    await waitFor(() => expect(within(sheet).queryByText("Ваш стол готов")).toBeNull());
    expect(within(sheet).getByText("Бот не может написать гостю — позвоните или откройте чат.")).toBeDefined();
    expect(server.count("POST", "/api/admin/bookings/b1/message")).toBe(1);
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
              : shift({
                  service_date: date,
                  now_minutes: null,
                  walk_in_until_minutes: null,
                  walk_in_free_table_ids: [],
                  bookings: [],
                }),
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
            slots: [{ start_minutes: 1_140, state: "free", evening: true, free_table_ids: ["t1", "t2", "t3"] }],
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
  it("offers a booking under way the tables the server names free from its start, asking with it set aside", async () => {
    fakeTelegram();
    const ignoring: (string | null)[] = [];
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({
        body: shift({ bookings: [shiftBooking({ status: "arrived", started: true })] }),
      }),
      "GET /api/admin/availability": ({ url }) => {
        ignoring.push(url.searchParams.get("ignoring"));
        return {
          body: availability({
            slots: [{ start_minutes: 1_260, state: "past", evening: true, free_table_ids: ["t1", "t3"] }],
          }),
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Смена"));
    await user.click(await screen.findByText("Саша"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Перенести"));
    const sheet = await screen.findByRole("dialog");
    expect(await within(sheet).findByText("Стол 10 · Зал")).toBeDefined();
    expect(within(sheet).queryByText("Стол 8 · Зал")).toBeNull();
    expect(ignoring.length).toBeGreaterThan(0);
    expect(ignoring.every((id) => id === "b1")).toBe(true);
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

  it("shows the times first asked for tonight when a later read of tonight failed and tomorrow answered between", async () => {
    fakeTelegram();
    const first = gate();
    const asked: string[] = [];
    fakeServer({
      "GET /api/session": () => ({ body: session() }),
      "GET /api/days": () => ({ body: { party_size: 2, days: rail(2) } }),
      "GET /api/availability": async ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        asked.push(date);
        if (date === "2026-09-12") {
          return {
            body: availability({
              service_date: date,
              slots: [{ start_minutes: 1_140, state: "free", evening: true, free_table_ids: ["t1", "t2", "t3"] }],
              free_count: 1,
            }),
          };
        }
        if (asked.filter((each) => each === date).length > 1) return failed;
        await first.opened;
        return { body: availability() };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    await waitFor(() => expect(asked).toEqual(["2026-09-11"]));
    await user.click(await screen.findByText("Завтра"));
    expect(await screen.findByRole("button", { name: "19:00" })).toBeDefined();
    await user.click(screen.getByText("Сегодня"));
    await waitFor(() => expect(asked).toHaveLength(3));
    await settle();

    await first.open();
    expect(await screen.findByRole("button", { name: "21:30" })).toBeDefined();
    expect(screen.queryByText("Не удалось прочитать свободные окна.")).toBeNull();
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

describe("a guest's picker read again", () => {
  it("says nothing on the home screen when the times fail after the guest left the picker", async () => {
    const telegram = fakeTelegram();
    const reading = gate();
    fakeServer({
      "GET /api/session": () => ({ body: session() }),
      "GET /api/days": () => ({ body: { party_size: 2, days: rail(2) } }),
      "GET /api/availability": async () => {
        await reading.opened;
        return failed;
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    await screen.findByText("Считаем свободные окна");
    act(() => telegram.pressBack());
    await screen.findByText("Столик на вечер");
    await reading.open();
    await settle();
    expect(screen.queryByText(BROKEN)).toBeNull();
  });

  it("keeps the evenings and times on screen when reading them again fails, and reads them again when asked", async () => {
    fakeTelegram();
    let twos = 0;
    const days: string[] = [];
    fakeServer({
      "GET /api/session": () => ({ body: session() }),
      "GET /api/days": ({ url }) => {
        const party = url.searchParams.get("party_size");
        days.push(party ?? "");
        if (party === "2" && days.filter((each) => each === "2").length === 2) return failed;
        return { body: { party_size: Number(party), days: rail(2) } };
      },
      "GET /api/availability": ({ url }) => {
        if (url.searchParams.get("party_size") === "2" && (twos += 1) === 2) return failed;
        return { body: availability() };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    await screen.findByRole("button", { name: "21:30" });
    await user.click(screen.getByRole("button", { name: "4 гостя" }));
    await waitFor(() => expect(days).toEqual(["2", "4"]));
    await user.click(screen.getByRole("button", { name: "2 гостя" }));

    await waitFor(() => expect(screen.getAllByText(STALE)).toHaveLength(2));
    expect(screen.getByRole("button", { name: "21:30" })).toBeDefined();
    expect(screen.getByText("Сегодня")).toBeDefined();
    expect(screen.queryByText(BROKEN)).toBeNull();

    await user.click(screen.getAllByText("Повторить")[0] as HTMLElement);
    await waitFor(() => expect(screen.queryByText(STALE)).toBeNull());
    expect(screen.getByRole("button", { name: "21:30" })).toBeDefined();
  });
});

describe("a guest changing their mind", () => {
  async function cancelTheBooking(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByText("Отменить"));
    await user.click(within(await screen.findByRole("dialog")).getByText("Отменить бронь"));
    return screen.findByText("Бронь отменена. Стол снова свободен.");
  }

  it("sees the booking gone once cancelled, with no way back offered, even when the home screen cannot be read again", async () => {
    // The confirmation was the protection. A rebooking undo could cancel another booking the guest
    // held, and could never bring back one that had begun.
    fakeTelegram();
    let sessions = 0;
    const server = fakeServer({
      "GET /api/session": () =>
        sessions++ === 0 ? { body: session({ bookings: [booking] }) } : failed,
      "DELETE /api/bookings/b1": () => ({ body: booking }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await cancelTheBooking(user);
    expect(screen.queryByText("Стол ваш")).toBeNull();
    expect(screen.queryByText("Вернуть")).toBeNull();
    await settle();
    expect(server.count("POST", "/api/booking")).toBe(0);
  });

  it("sends what the button promised to replace, and when the bookings changed says so, reads them again and keeps the picker", async () => {
    fakeTelegram();
    let sessions = 0;
    let changed = false;
    const server = fakeServer({
      "GET /api/session": () => {
        sessions += 1;
        return { body: session({ bookings: sessions === 1 ? [booking] : [] }) };
      },
      "GET /api/days": () => ({ body: { party_size: 4, days: rail(2) } }),
      "GET /api/availability": () => ({ body: availability({ replacing: changed ? [] : ["b1"] }) }),
      "POST /api/booking": () => {
        changed = true;
        return { status: 409, body: { error: { code: "booking_changed", message: "changed" } } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Перенести"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("Перенести · 4 гостя · сегодня в 21:30"));
    expect(
      await screen.findByText("Ваши брони изменились — проверьте и нажмите ещё раз."),
    ).toBeDefined();
    expect(server.bodies("POST", "/api/booking")).toEqual([
      { service_date: "2026-09-11", start_minutes: 1_290, party_size: 4, replacing: ["b1"] },
    ]);
    expect(await screen.findByText("Забронировать · 4 гостя · сегодня в 21:30")).toBeDefined();
    expect(server.count("GET", "/api/session")).toBe(2);
  });

  it("reads what the guest holds again when a booking is refused because the evening is already theirs", async () => {
    fakeTelegram();
    const server = fakeServer({
      "GET /api/session": () => ({ body: session({ bookings: [booking] }) }),
      "GET /api/days": () => ({ body: { party_size: 4, days: rail(2) } }),
      "GET /api/availability": () => ({ body: availability({ replacing: ["b1"] }) }),
      "POST /api/booking": () => ({
        status: 409,
        body: { error: { code: "already_booked_tonight", message: "held" } },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Перенести"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("Перенести · 4 гостя · сегодня в 21:30"));
    await waitFor(() => expect(server.count("GET", "/api/session")).toBe(2));
  });

  it("reads the times again when the bookings changed, and no longer offers a time that passed meanwhile", async () => {
    fakeTelegram();
    let sessions = 0;
    let refused = false;
    fakeServer({
      "GET /api/session": () => {
        sessions += 1;
        return { body: session({ bookings: sessions === 1 ? [booking] : [] }) };
      },
      "GET /api/days": () => ({ body: { party_size: 4, days: rail(2) } }),
      "GET /api/availability": () => ({
        body: refused
          ? availability({
              slots: [
                { start_minutes: 1_290, state: "past", evening: true, free_table_ids: [] },
                { start_minutes: 1_350, state: "free", evening: true, free_table_ids: [] },
              ],
              free_count: 1,
            })
          : availability({ replacing: ["b1"] }),
      }),
      "POST /api/booking": () => {
        refused = true;
        return { status: 409, body: { error: { code: "booking_changed", message: "changed" } } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Перенести"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("Перенести · 4 гостя · сегодня в 21:30"));
    await screen.findByText("Ваши брони изменились — проверьте и нажмите ещё раз.");
    await waitFor(() => expect(screen.queryByRole("button", { name: "21:30" })).toBeNull());
    expect(screen.getByRole("button", { name: "22:30" })).toBeDefined();
    expect(screen.queryByText(/сегодня в 21:30/)).toBeNull();
    expect(screen.getByText("Выберите время")).toBeDefined();
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
    const asked: string[] = [];
    const server = fakeServer({
      "GET /api/session": () => ({ body: session({ bookings: [heldNoShow] }) }),
      "GET /api/days": () => ({ body: { party_size: 4, days: rail(2) } }),
      "GET /api/availability": ({ url }) => {
        asked.push(url.searchParams.get("service_date") ?? "");
        return { body: availability({ replacing: [heldNoShow.id] }) };
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
    expect(server.bodies("POST", "/api/booking")[0]).toMatchObject({ replacing: ["b9"] });
  });

  it("never says tonight is already the guest's for a booking the server says does not hold it, before the rail is read", async () => {
    fakeTelegram();
    const railRead = gate();
    const noShow: GuestBooking = { ...heldNoShow, rebooking_replaces: null, holds_evening: false };
    fakeServer({
      "GET /api/session": () => ({ body: session({ bookings: [noShow], bookable_days: ["2026-09-11"] }) }),
      "GET /api/days": async () => {
        await railRead.opened;
        return { body: { party_size: 2, days: rail(1) } };
      },
      "GET /api/availability": () => ({ body: availability() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Забронировать стол"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    expect(await screen.findByText("Забронировать · 2 гостя · сегодня в 21:30")).toBeDefined();
    expect(screen.queryByText("На этот вечер у вас уже есть бронь")).toBeNull();
    await railRead.open();
  });

  it("never lets a guest at the table book tonight again", async () => {
    fakeTelegram();
    const server = fakeServer({
      "GET /api/session": () =>
        ({ body: session({ bookings: [seated], bookable_days: ["2026-09-11"] }) }),
      "GET /api/days": () => ({ body: { party_size: 2, days: [{ ...rail(1)[0], booked: true }] } }),
      "GET /api/availability": () => ({ body: availability({ booked: true }) }),
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
  it("takes the button's word and what it sends from the times the server answered, not from the bookings on the home screen", async () => {
    fakeTelegram();
    const server = fakeServer({
      "GET /api/session": () => ({ body: session({ bookings: [booking] }) }),
      "GET /api/days": () => ({ body: { party_size: 4, days: rail(2) } }),
      "GET /api/availability": ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        return { body: availability({ service_date: date, replacing: date === "2026-09-12" ? ["b1"] : [] }) };
      },
      "POST /api/booking": () => ({
        body: { booking: { ...booking, id: "b3", service_date: "2026-09-12" }, replaced: ["b1"] },
      }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Перенести"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    expect(await screen.findByText("Забронировать · 4 гостя · сегодня в 21:30")).toBeDefined();

    await user.click(screen.getByText("Завтра"));
    await user.click(await screen.findByRole("button", { name: "21:30" }));
    await user.click(await screen.findByText("Перенести · 4 гостя · завтра в 21:30"));
    expect(await screen.findByText("Бронь перенесена")).toBeDefined();
    expect(server.bodies("POST", "/api/booking")).toEqual([
      { service_date: "2026-09-12", start_minutes: 1_290, party_size: 4, replacing: ["b1"] },
    ]);
  });
});

describe("settings", () => {
  it("say they could not be read, with a way to try again, rather than spinning", async () => {
    fakeTelegram();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/settings": () => (reads++ === 0 ? failed : { body: settingsView() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    expect(await screen.findByText("Не удалось прочитать настройки.")).toBeDefined();
    await user.click(screen.getByText("Попробовать снова"));
    expect(await screen.findByText("Правила бронирования")).toBeDefined();
  });

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
        return { body: settingsView({ name: "Чердак", version: 2 }) };
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
        stored = settingsView({ name: "Чердак", version: 2 });
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
      "GET /api/admin/settings": () => ({ body: settingsView({ version: 1 }) }),
      "PUT /api/admin/settings": async ({ body }) => {
        const draft = body as SettingsDraft;
        sent.push(draft);
        if (sent.length === 1) await saving.opened;
        return {
          body: {
            settings: settingsView({
              name: draft.name,
              address: draft.address,
              version: sent.length + 1,
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
    expect(sent[0]).toMatchObject({ name: "Чердак", address: "ул. Рубинштейна, 24", version: 1 });
    expect(sent[1]).toMatchObject({ name: "Чердак", address: "Невский, 1", version: 2 });
  });

  it("carry a change to a table added in the save over to the table the save created, not a second one", async () => {
    fakeTelegram();
    const saving = gate();
    const sent: SettingsDraft[] = [];
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: settingsView({ version: 1 }) }),
      "PUT /api/admin/settings": async ({ body }) => {
        const draft = body as SettingsDraft;
        sent.push(draft);
        if (sent.length === 1) await saving.opened;
        const stored = settingsView();
        return {
          body: {
            settings: settingsView({
              tables: draft.tables.map((table) => {
                const known = stored.tables.find((each) => each.id === table.id);
                return known
                  ? { ...known, seats: table.seats }
                  : { id: table.id, number: 9, seats: table.seats, zone: table.zone };
              }),
              next_table_number: 10,
              version: sent.length + 1,
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
    const added = sent[0]?.tables[2]?.id;
    expect(added).toMatch(/^[0-9a-f-]{36}$/);
    expect(sent[1]?.tables).toEqual([
      { id: "t1", seats: 2, zone: "Стойка" },
      { id: "t2", seats: 6, zone: "Зал" },
      { id: added, seats: 5, zone: "Зал" },
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
              ? settingsView({ version: 1 })
              : settingsView({ address: "Невский, 1", version: 2 }),
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
            settings: settingsView({ name: draft.name, address: draft.address, version: 3 }),
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
      version: 2,
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
        if (reads === 1) return { body: settingsView({ version: 1 }) };
        await reread.opened;
        return { body: settingsView({ name: "Подвал", version: 2 }) };
      },
      "PUT /api/admin/settings": ({ body }) => {
        sent.push(body as SettingsDraft);
        return sent.length === 1
          ? { status: 409, body: { error: { code: "settings_changed", message: "changed" } } }
          : {
              body: {
                settings: settingsView({ name: "Мансарда", version: 3 }),
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
    expect(sent[1]).toMatchObject({ name: "Мансарда", version: 2 });
  });

  it("read the settings again only once a save has answered, and take the newest", async () => {
    fakeTelegram();
    const saving = gate();
    let reads = 0;
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => {
        reads += 1;
        return {
          body:
            reads === 1
              ? settingsView({ version: 1 })
              : settingsView({ name: "Подвал", version: 3 }),
        };
      },
      "PUT /api/admin/settings": async () => {
        await saving.opened;
        return {
          body: {
            settings: settingsView({ name: "Чердак", version: 2 }),
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
    await settle();
    expect(server.count("GET", "/api/admin/settings")).toBe(1);

    await saving.open();
    await screen.findByText("Настройки сохранены.");
    expect(await screen.findByDisplayValue("Подвал")).toBeDefined();
    await settle();
    expect(server.count("GET", "/api/admin/settings")).toBe(2);
    expect(screen.queryByText("Сохранить")).toBeNull();
  });

  it("count a table's bookings from the evening on screen, read for them without a look at the shift, and none until it is read", async () => {
    fakeTelegram();
    const tomorrow = gate();
    const server = fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": async ({ url }) => {
        const date = url.searchParams.get("service_date") ?? "";
        if (date === "2026-09-11") {
          return { body: shift({ bookings: [shiftBooking(), shiftBooking({ id: "b2" })] }) };
        }
        await tomorrow.opened;
        const three = ["b3", "b4", "b5"].map((id) => shiftBooking({ id }));
        return { body: shift({ service_date: date, bookings: three }) };
      },
      "GET /api/admin/settings": () => ({ body: settingsView() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Зал"));
    expect(await screen.findByText("2 брони")).toBeDefined();
    expect(server.count("GET", "/api/admin/shift")).toBe(1);

    await user.click(screen.getByText("Смена"));
    await user.click(await screen.findByRole("button", { name: "Следующий день" }));
    await user.click(screen.getByText("Настройки"));
    await settle();
    expect(screen.getByText("Стол 7")).toBeDefined();
    expect(screen.queryByText("2 брони")).toBeNull();

    await tomorrow.open();
    expect(await screen.findByText("3 брони")).toBeDefined();
  });

  it("take a save made on one evening as saved when it answers after a switch to another, with no second table", async () => {
    fakeTelegram();
    const saving = gate();
    const bar = settingsServer();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": ({ url }) => ({
        body: shift({ service_date: url.searchParams.get("service_date") ?? "" }),
      }),
      "GET /api/admin/settings": () => ({ body: bar.read() }),
      "PUT /api/admin/settings": async ({ body }) => {
        const answer = bar.save(body as SettingsDraft);
        await saving.opened;
        return answer;
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Зал"));
    await user.click(await screen.findByText("+ Добавить стол"));
    await user.click(await screen.findByText("Сохранить"));
    await user.click(screen.getByText("Смена"));
    await user.click(await screen.findByRole("button", { name: "Следующий день" }));
    await screen.findByText("сб, 12 сен");
    await user.click(screen.getByText("Настройки"));

    await saving.open();
    await screen.findByText("Настройки сохранены.");
    await settle();
    expect(screen.queryByText(/кто-то изменил настройки/)).toBeNull();
    expect(screen.queryByText("Сохранить")).toBeNull();
    expect(screen.getAllByText("Стол 9")).toHaveLength(1);
    expect(bar.current.tables).toHaveLength(3);
  });

  it("say nothing about somebody else when a reread lands while one's own save is on its way", async () => {
    fakeTelegram();
    const reread = gate();
    const saving = gate();
    const bar = settingsServer();
    let reads = 0;
    let saves = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        reads += 1;
        if (reads === 2) await reread.opened;
        return { body: bar.read() };
      },
      "PUT /api/admin/settings": async ({ body }) => {
        saves += 1;
        const answer = bar.save(body as SettingsDraft);
        await saving.opened;
        return answer;
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Чердак ");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await waitFor(() => expect(reads).toBe(2));
    await user.click(await screen.findByText("Сохранить"));
    await waitFor(() => expect(saves).toBe(1));

    await reread.open();
    await settle();
    expect(screen.queryByText(/кто-то изменил настройки/)).toBeNull();
    await saving.open();
    await screen.findByText("Настройки сохранены.");
    await settle();
    expect(screen.queryByText(/кто-то изменил настройки/)).toBeNull();
    expect(screen.getByDisplayValue("Чердак")).toBeDefined();
    expect(screen.queryByText("Сохранить")).toBeNull();
  });

  it("fold somebody else's later save into an edit made after one's own, with no second table", async () => {
    fakeTelegram();
    const later = gate();
    const bar = settingsServer();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        reads += 1;
        if (reads === 2) await later.opened;
        return { body: bar.read() };
      },
      "PUT /api/admin/settings": ({ body }) =>
        bar.save(body as SettingsDraft),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Зал"));
    await user.click(await screen.findByText("+ Добавить стол"));
    await user.click(await screen.findByText("Сохранить"));
    await screen.findByText("Настройки сохранены.");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await waitFor(() => expect(reads).toBe(2));
    await user.click(await screen.findByRole("button", { name: "Назад" }));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Мансарда");
    bar.change({ address: "Невский, 1" });

    await later.open();
    expect(
      await screen.findByText(
        "Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.",
      ),
    ).toBeDefined();
    expect(screen.getByDisplayValue("Невский, 1")).toBeDefined();
    await user.click(screen.getByText("Сохранить"));
    await waitFor(() => expect(bar.current.name).toBe("Мансарда"));
    expect(bar.current.address).toBe("Невский, 1");
    expect(bar.current.tables).toHaveLength(3);
  });

  it("never add a table twice when a save's answer was lost and the retry was refused as stale", async () => {
    fakeTelegram();
    const bar = settingsServer();
    let puts = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: bar.read() }),
      "PUT /api/admin/settings": ({ body }) => {
        puts += 1;
        const answer = bar.save(body as SettingsDraft);
        if (puts === 1) throw new TypeError("the answer never came back");
        return answer;
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Зал"));
    await user.click(await screen.findByText("+ Добавить стол"));
    await user.click(await screen.findByText("Сохранить"));
    expect(await screen.findByText("Нет связи. Проверьте интернет и попробуйте ещё раз.")).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Назад" }));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Мансарда");
    await user.click(screen.getByText("Сохранить"));
    expect(
      await screen.findByText(
        "Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.",
      ),
    ).toBeDefined();

    await user.click(screen.getByText("Сохранить"));
    await waitFor(() => expect(bar.current.name).toBe("Мансарда"));
    expect(bar.current.tables).toHaveLength(3);
    expect(new Set(bar.current.tables.map((table) => table.id)).size).toBe(3);
  });

  it("say what a reread folded into an edit on the settings screen, not in a toast the shift would hide", async () => {
    fakeTelegram();
    const later = gate();
    const bar = settingsServer();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        reads += 1;
        if (reads === 2) await later.opened;
        return { body: bar.read() };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Бар"));
    await user.clear(await screen.findByPlaceholderText("Название"));
    await user.type(screen.getByPlaceholderText("Название"), "Мансарда");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await waitFor(() => expect(reads).toBe(2));
    bar.change({ name: "Подвал" });
    await user.click(screen.getByText("Смена"));
    await screen.findByText("Саша");

    await later.open();
    await settle();
    expect(screen.queryByText(/кто-то изменил настройки/)).toBeNull();

    await user.click(screen.getByText("Настройки"));
    const conflict =
      "Пока вы редактировали, кто-то изменил настройки: название. Оставили ваши значения — проверьте и сохраните.";
    expect(await screen.findByText(conflict)).toBeDefined();
    await settle();
    expect(screen.getByText(conflict)).toBeDefined();
    expect(screen.getByDisplayValue("Мансарда")).toBeDefined();

    await user.type(screen.getByPlaceholderText("Название"), "!");
    expect(screen.queryByText(conflict)).toBeNull();
  });

  it("say nothing when a read fails while a newer read of them is on its way, and show what that brings", async () => {
    fakeTelegram();
    const failing = gate();
    const loading = gate();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": async () => {
        reads += 1;
        if (reads === 1) {
          await failing.opened;
          return failed;
        }
        await loading.opened;
        return { body: settingsView() };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await waitFor(() => expect(reads).toBe(1));
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await waitFor(() => expect(reads).toBe(2));

    await failing.open();
    await settle();
    expect(screen.queryByText(BROKEN)).toBeNull();
    await loading.open();
    expect(await screen.findByText("Правила бронирования")).toBeDefined();
    expect(screen.queryByText(STALE)).toBeNull();
  });

  it("keep what is on screen when reading them again fails, and read them again when asked", async () => {
    fakeTelegram();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => {
        reads += 1;
        if (reads === 2) return failed;
        return {
          body: reads === 1 ? settingsView() : settingsView({ name: "Чердак", version: 2 }),
        };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await screen.findByText("Пустол · ул. Рубинштейна, 24");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    expect(await screen.findByText(STALE)).toBeDefined();
    expect(screen.getByText("Пустол · ул. Рубинштейна, 24")).toBeDefined();
    expect(screen.queryByText(BROKEN)).toBeNull();

    await user.click(screen.getByText("Повторить"));
    expect(await screen.findByText("Чердак · ул. Рубинштейна, 24")).toBeDefined();
    expect(screen.queryByText(STALE)).toBeNull();
  });

  it("never bring back a member somebody else removed, whatever order a save answered the staff in", async () => {
    fakeTelegram();
    const bar = settingsServer();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: bar.read() }),
      "PUT /api/admin/settings": ({ body }) => {
        const draft = body as SettingsDraft;
        const answer = bar.save(draft);
        if (answer.status) return answer;
        // In the order the save was sent, where every read lists the staff sorted.
        const saved = answer.body as { settings: SettingsView };
        const staff = draft.staff.map(
          ({ username }) =>
            saved.settings.staff.find((member) => member.username === username) ?? { username, bound: false },
        );
        return { body: { ...saved, settings: { ...saved.settings, staff } } };
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Персонал"));
    await user.type(await screen.findByPlaceholderText("@username"), "@aaron");
    await user.click(screen.getByText("Добавить"));
    await user.click(await screen.findByText("Сохранить"));
    await screen.findByText("Настройки сохранены.");

    await user.click(screen.getByRole("button", { name: "Назад" }));
    await user.click(await screen.findByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Мансарда");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await settle();

    bar.change({ staff: bar.current.staff.filter((member) => member.username !== "pavel") });
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    await screen.findByText(
      "Пока вы редактировали, настройки обновились. Ваши правки на месте — проверьте и сохраните.",
    );
    expect(screen.queryByText(/кто-то изменил настройки/)).toBeNull();

    await user.click(screen.getByText("Сохранить"));
    await waitFor(() => expect(bar.current.name).toBe("Мансарда"));
    expect(bar.current.staff.map((member) => member.username)).toEqual(["aaron", "marina", "nastya"]);
  });

  for (const [path, reread] of [
    ["a reread", async (user: ReturnType<typeof userEvent.setup>) => {
      await user.click(screen.getByText("Смена"));
      await user.click(screen.getByText("Настройки"));
    }],
    ["a retry refused as stale", async (user: ReturnType<typeof userEvent.setup>) => {
      await user.click(screen.getByText("Сохранить"));
    }],
  ] as const) {
    it(`take a trimmed save whose answer was lost as saved, after ${path}, with no conflict and no save bar`, async () => {
      fakeTelegram();
      const bar = settingsServer();
      let puts = 0;
      fakeServer({
        "GET /api/session": staffSession,
        "GET /api/admin/shift": () => ({ body: shift() }),
        "GET /api/admin/settings": () => ({ body: bar.read() }),
        "PUT /api/admin/settings": ({ body }) => {
          puts += 1;
          const answer = bar.save(body as SettingsDraft);
          if (puts === 1) throw new TypeError("the answer never came back");
          return answer;
        },
      });
      const user = userEvent.setup();
      render(<Page />);

      await user.click(await screen.findByText("Настройки"));
      await user.click(await screen.findByText("Бар"));
      const name = await screen.findByPlaceholderText("Название");
      await user.clear(name);
      await user.type(name, "Чердак ");
      await user.click(await screen.findByText("Сохранить"));
      await screen.findByText("Нет связи. Проверьте интернет и попробуйте ещё раз.");
      expect(bar.current.name).toBe("Чердак");

      await reread(user);
      expect(await screen.findByDisplayValue("Чердак")).toBeDefined();
      await settle();
      expect(screen.queryByText(/кто-то изменил настройки/)).toBeNull();
      expect(screen.queryByText("Сохранить")).toBeNull();
    });
  }

  it("keep a refused save's reasons one tap away on the save bar when a sheet was opened while it was on its way", async () => {
    const telegram = fakeTelegram();
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
    expect(
      await screen.findByText("Не сохранено: сервер не принял значения. Причины — по кнопке «Почему»."),
    ).toBeDefined();
    await settle();
    expect(screen.getByText("Как прошло")).toBeDefined();
    expect(screen.queryByText("Стол 1: столько мест не бывает.")).toBeNull();

    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByText("Как прошло")).toBeNull());
    await user.click(screen.getByText("Настройки"));
    expect(await screen.findByText("Не сохранено.")).toBeDefined();
    await user.click(screen.getByText("Почему"));
    expect(
      within(await screen.findByRole("dialog")).getByText("Стол 1: столько мест не бывает."),
    ).toBeDefined();
    act(() => telegram.pressBack());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await user.type(screen.getByPlaceholderText("Название"), "!");
    expect(screen.queryByText("Не сохранено.")).toBeNull();
  });

  it("stop saying they could not be read again once a save of them has answered", async () => {
    fakeTelegram();
    const bar = settingsServer();
    let reads = 0;
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => {
        reads += 1;
        return reads === 2 ? failed : { body: bar.read() };
      },
      "PUT /api/admin/settings": ({ body }) =>
        bar.save(body as SettingsDraft),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await screen.findByText("Правила бронирования");
    await user.click(screen.getByText("Смена"));
    await user.click(screen.getByText("Настройки"));
    expect(await screen.findByText(STALE)).toBeDefined();

    await user.click(screen.getByText("Бар"));
    const name = await screen.findByPlaceholderText("Название");
    await user.clear(name);
    await user.type(name, "Мансарда");
    await user.click(screen.getByText("Сохранить"));
    await screen.findByText("Настройки сохранены.");
    await settle();
    expect(screen.queryByText(STALE)).toBeNull();
    expect(reads).toBe(2);
  });

  it("remove the member whose row was tapped, not one spelled the same in another case", async () => {
    fakeTelegram();
    const bar = settingsServer();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: bar.read() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Персонал"));
    await user.type(await screen.findByPlaceholderText("@username"), "@Pavel");
    await user.click(screen.getByText("Добавить"));
    expect(await screen.findByText("@Pavel")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Убрать @Pavel" }));

    expect(screen.getByText("@pavel")).toBeDefined();
    expect(screen.queryByText("@Pavel")).toBeNull();
    expect(screen.queryByText("Сохранить")).toBeNull();
  });

  it("draw two rows typed the same as two rows, and remove one of them", async () => {
    // Keyed by the username alone, the second row was the same child as the first to React.
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    fakeTelegram();
    const bar = settingsServer();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: bar.read() }),
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Персонал"));
    await user.type(await screen.findByPlaceholderText("@username"), "@pavel");
    await user.click(screen.getByText("Добавить"));
    expect(screen.getAllByText("@pavel")).toHaveLength(2);
    await user.click(screen.getAllByRole("button", { name: "Убрать @pavel" })[1] as HTMLElement);
    expect(screen.getAllByText("@pavel")).toHaveLength(1);
    expect(screen.getAllByText(/^@/).map((node) => node.textContent)).toEqual(["@marina", "@nastya", "@pavel"]);
    expect(screen.queryByText("Сохранить")).toBeNull();
    const logged = errors.mock.calls.flat().join(" ");
    errors.mockRestore();
    expect(logged).not.toMatch(/same key/);
  });

  for (const [who, remaining] of [
    ["@pavel", ["@aaron_bar", "@marina", "@nastya"]],
    ["@aaron_bar", ["@marina", "@nastya", "@pavel"]],
  ] as const) {
    it(`remove ${who}, tapped while a save that added @aaron_bar is on its way, and nobody the answer lists in that place`, async () => {
      // The save answers with the staff sorted, where the tap was made on the list as it was sent.
      fakeTelegram();
      const saving = gate();
      const bar = settingsServer();
      fakeServer({
        "GET /api/session": staffSession,
        "GET /api/admin/shift": () => ({ body: shift() }),
        "GET /api/admin/settings": () => ({ body: bar.read() }),
        "PUT /api/admin/settings": async ({ body }) => {
          const answer = bar.save(body as SettingsDraft);
          await saving.opened;
          return answer;
        },
      });
      const user = userEvent.setup();
      render(<Page />);

      await user.click(await screen.findByText("Настройки"));
      await user.click(await screen.findByText("Персонал"));
      await user.type(await screen.findByPlaceholderText("@username"), "@aaron_bar");
      await user.click(screen.getByText("Добавить"));
      await user.click(await screen.findByText("Сохранить"));
      await user.click(screen.getByRole("button", { name: `Убрать ${who}` }));

      await saving.open();
      await screen.findByText("Настройки сохранены.");
      await settle();
      expect(screen.queryByText(who)).toBeNull();
      for (const member of remaining) expect(screen.getByText(member)).toBeDefined();

      await user.click(screen.getByText("Сохранить"));
      await waitFor(() =>
        expect(bar.current.staff.map((member) => `@${member.username}`)).toEqual(remaining),
      );
    });
  }

  it("change the tables a tap named while a save that added a table is on its way", async () => {
    fakeTelegram();
    const saving = gate();
    const bar = settingsServer();
    fakeServer({
      "GET /api/session": staffSession,
      "GET /api/admin/shift": () => ({ body: shift() }),
      "GET /api/admin/settings": () => ({ body: bar.read() }),
      "PUT /api/admin/settings": async ({ body }) => {
        const answer = bar.save(body as SettingsDraft);
        await saving.opened;
        return answer;
      },
    });
    const user = userEvent.setup();
    render(<Page />);

    await user.click(await screen.findByText("Настройки"));
    await user.click(await screen.findByText("Зал"));
    await user.click(await screen.findByText("+ Добавить стол"));
    await user.click(await screen.findByText("Сохранить"));
    await user.click(screen.getByRole("button", { name: "Убрать стол 7" }));
    await user.click(screen.getByRole("button", { name: "Стол 8: больше мест" }));

    await saving.open();
    await screen.findByText("Настройки сохранены.");
    await user.click(await screen.findByText("Сохранить"));
    await waitFor(() => expect(bar.current.tables).toHaveLength(2));
    expect(bar.current.tables.map((table) => [table.number, table.seats])).toEqual([
      [8, 7],
      [9, 4],
    ]);
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
