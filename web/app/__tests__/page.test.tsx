/**
 * The whole app, against a fake Telegram and a fake server.
 *
 * The screens are tested on their own elsewhere. What only shows up here is the wiring between
 * them: when the app fetches, what it does with two taps, what a failure after a success leaves
 * on screen, and what Telegram's own buttons do. Everything in this file was broken in a way no
 * screen test could see.
 */

import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
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
} from "@/components/__tests__/fixtures";
import type { WebApp } from "@/lib/telegram";

interface Answer {
  status?: number;
  body: unknown;
}

type Handler = (body: unknown) => Answer | Promise<Answer>;

const failed: Answer = { status: 500, body: { error: { code: "internal", message: "boom" } } };

function fakeServer(routes: Record<string, Handler>) {
  const calls: { method: string; path: string }[] = [];
  const fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const url = new URL(input, "http://app.test");
    const method = init?.method ?? "GET";
    calls.push({ method, path: url.pathname });
    const handler = routes[`${method} ${url.pathname}`];
    const { status = 200, body } = handler
      ? await handler(init?.body ? JSON.parse(String(init.body)) : undefined)
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
    await user.click(screen.getByText("Настройки"));
    expect(await screen.findByDisplayValue("Чердак")).toBeDefined();
  });
});
