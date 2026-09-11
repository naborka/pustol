/**
 * Telegram surface bootstrap: expanding, colours, insets, and the height the layout uses.
 *
 * A fake WebApp is passed in. The unit under test is not mocked.
 */

import { describe, expect, it, vi } from "vitest";

import {
  bootstrapTelegram,
  combinedInsets,
  ZERO_INSETS,
  type Insets,
  type WebApp,
} from "../telegram";

function callOrder(fn: unknown): number {
  const order = (fn as { mock: { invocationCallOrder: number[] } }).mock.invocationCallOrder[0];
  if (order === undefined) throw new Error("not called");
  return order;
}

function fakeWebApp(overrides: Partial<WebApp> = {}): WebApp {
  const listeners = new Map<string, Set<() => void>>();
  return {
    initData: "",
    colorScheme: "dark",
    themeParams: {},
    version: "8.0",
    isExpanded: false,
    viewportStableHeight: 700,
    ready: vi.fn(),
    expand: vi.fn(),
    disableVerticalSwipes: vi.fn(),
    close: vi.fn(),
    onEvent: vi.fn((event: string, handler: () => void) => {
      const set = listeners.get(event) ?? new Set();
      set.add(handler);
      listeners.set(event, set);
    }),
    offEvent: vi.fn((event: string, handler: () => void) => {
      listeners.get(event)?.delete(handler);
    }),
    openTelegramLink: vi.fn(),
    requestFullscreen: vi.fn(),
    setHeaderColor: vi.fn(),
    setBackgroundColor: vi.fn(),
    safeAreaInset: { top: 0, right: 0, bottom: 0, left: 0 },
    contentSafeAreaInset: { top: 0, right: 0, bottom: 0, left: 0 },
    ...overrides,
    emit(event: string) {
      for (const handler of listeners.get(event) ?? []) handler();
    },
  } as WebApp & { emit?: (event: string) => void };
}

describe("combined insets", () => {
  it("adds device safe area to Telegram content safe area", () => {
    const safe: Insets = { top: 10, right: 2, bottom: 20, left: 1 };
    const content: Insets = { top: 40, right: 0, bottom: 8, left: 0 };
    expect(combinedInsets(safe, content)).toEqual({
      top: 50,
      right: 2,
      bottom: 28,
      left: 1,
    });
  });

  it("treats a missing inset object as zero, not a crash", () => {
    expect(combinedInsets(undefined, undefined)).toEqual(ZERO_INSETS);
    expect(combinedInsets({ top: 4, right: 0, bottom: 0, left: 0 }, undefined)).toEqual({
      top: 4,
      right: 0,
      bottom: 0,
      left: 0,
    });
  });
});

describe("bootstrapTelegram", () => {
  it("makes itself ready, expands, stops the swipe that collapses it, then paints", () => {
    // The order is the point. A client that has not been told the app is drawn answers some of
    // these with stale values, and a height measured while collapsed is the wrong height.
    const app = fakeWebApp();
    bootstrapTelegram(app, vi.fn());

    expect(app.ready).toHaveBeenCalledOnce();
    expect(app.expand).toHaveBeenCalledOnce();
    expect(app.disableVerticalSwipes).toHaveBeenCalledOnce();
    expect(app.setHeaderColor).toHaveBeenCalledWith("bg_color");
    expect(app.setBackgroundColor).toHaveBeenCalledWith("bg_color");
    expect(callOrder(app.ready)).toBeLessThan(callOrder(app.expand));
    expect(callOrder(app.expand)).toBeLessThan(callOrder(app.disableVerticalSwipes));
    expect(callOrder(app.disableVerticalSwipes)).toBeLessThan(callOrder(app.setHeaderColor));
  });

  it("does not ask for fullscreen, which is the platform's to give", () => {
    const app = fakeWebApp();
    bootstrapTelegram(app, vi.fn());
    expect(app.requestFullscreen).not.toHaveBeenCalled();
  });

  it("still starts on a client too old to stop the swipe", () => {
    const app = fakeWebApp();
    delete (app as { disableVerticalSwipes?: unknown }).disableVerticalSwipes;
    expect(() => bootstrapTelegram(app, vi.fn())).not.toThrow();
    expect(app.ready).toHaveBeenCalledOnce();
    expect(app.expand).toHaveBeenCalledOnce();
  });

  it("publishes the surface and republishes it whenever Telegram moves the ground", () => {
    const app = fakeWebApp({
      safeAreaInset: { top: 10, right: 2, bottom: 20, left: 1 },
      contentSafeAreaInset: { top: 40, right: 0, bottom: 8, left: 0 },
      viewportStableHeight: 700,
    });
    const onSurface = vi.fn();
    const stop = bootstrapTelegram(app, onSurface);
    expect(onSurface).toHaveBeenCalledWith({
      insets: { top: 50, right: 2, bottom: 28, left: 1 },
      stableHeight: 700,
    });

    app.safeAreaInset = { top: 12, right: 2, bottom: 34, left: 1 };
    const emit = (app as WebApp & { emit: (event: string) => void }).emit;
    emit("safeAreaChanged");
    expect(onSurface).toHaveBeenLastCalledWith({
      insets: { top: 52, right: 2, bottom: 42, left: 1 },
      stableHeight: 700,
    });

    stop();
    emit("safeAreaChanged");
    expect(onSurface).toHaveBeenCalledTimes(2);
  });

  it("hears the viewport change, which is what a keyboard opening looks like", () => {
    const app = fakeWebApp({ viewportStableHeight: 700 });
    const onSurface = vi.fn();
    bootstrapTelegram(app, onSurface);

    app.viewportStableHeight = 420;
    (app as WebApp & { emit: (event: string) => void }).emit("viewportChanged");
    expect(onSurface).toHaveBeenLastCalledWith({
      insets: ZERO_INSETS,
      stableHeight: 420,
    });
  });
});
