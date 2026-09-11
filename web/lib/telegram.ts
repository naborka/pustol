/**
 * The Telegram Mini App surface, and what to do when there isn't one.
 *
 * Everything the app knows about who is using it comes from `initData`, which only the real client
 * can produce. Outside Telegram — a browser tab during development — there is no payload and the API
 * refuses every request, which is exactly right: a development mode that let the client assert its
 * own identity would be a way in.
 *
 * `NEXT_PUBLIC_DEV_INIT_DATA` exists for local work and is not an exception to that. It has to be a
 * payload genuinely signed with the bot's token, so only somebody who already holds the token can
 * make one, and the server checks it the same way it checks a real client's.
 */

export interface WebAppUser {
  id: number;
  first_name: string;
  last_name?: string;
  username?: string;
  language_code?: string;
}

/** Device or Telegram content inset, in CSS pixels. */
export interface Insets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export const ZERO_INSETS: Insets = { top: 0, right: 0, bottom: 0, left: 0 };

/**
 * Every event that can move the ground under the layout.
 *
 * `viewportChanged` is the one that matters most and the one that was missing: the Android
 * keyboard resizes the webview, and a shell sized from a height read once at start-up puts its
 * main button under the keyboard and leaves it there.
 */
const SURFACE_EVENTS = [
  "viewportChanged",
  "safeAreaChanged",
  "contentSafeAreaChanged",
  "fullscreenChanged",
  "fullscreenFailed",
] as const;

export interface WebApp {
  initData: string;
  initDataUnsafe?: { user?: WebAppUser };
  colorScheme: "light" | "dark";
  themeParams: Record<string, string>;
  version: string;
  isExpanded: boolean;
  isFullscreen?: boolean;
  /**
   * The height that stays put — the expanded viewport minus whatever Telegram might yet cover.
   *
   * The only height this app lays out against. `viewportHeight` grows and shrinks as the client
   * animates, and `window.innerHeight` in the Telegram webview is taller than the part the user
   * can actually see, which is what puts the main button under Telegram's own chrome.
   */
  viewportStableHeight: number;
  viewportHeight?: number;
  ready: () => void;
  expand: () => void;
  /** Bot API 7.7 and later. Absent on older clients, where a vertical drag still collapses. */
  disableVerticalSwipes?: () => void;
  close: () => void;
  onEvent: (event: string, handler: () => void) => void;
  offEvent: (event: string, handler: () => void) => void;
  openTelegramLink: (url: string) => void;
  /**
   * Declared but never called, and a test holds it that way.
   *
   * Fullscreen is the platform's to give: the user asks for it from Telegram's own menu, and this
   * app follows along through `fullscreenChanged`. Requesting it on start-up made the app taller
   * than the room it was given and was part of why its bottom bar ended up under Telegram's chrome.
   */
  requestFullscreen?: () => void;
  setHeaderColor?: (color: string) => void;
  setBackgroundColor?: (color: string) => void;
  safeAreaInset?: Insets;
  contentSafeAreaInset?: Insets;
  HapticFeedback?: {
    impactOccurred: (style: "light" | "medium" | "heavy") => void;
    notificationOccurred: (type: "error" | "success" | "warning") => void;
    selectionChanged: () => void;
  };
  BackButton?: {
    show: () => void;
    hide: () => void;
    onClick: (handler: () => void) => void;
    offClick: (handler: () => void) => void;
  };
}

declare global {
  interface Window {
    Telegram?: { WebApp?: WebApp };
  }
}

export function webApp(): WebApp | undefined {
  if (typeof window === "undefined") return undefined;
  return window.Telegram?.WebApp;
}

/** The signed payload, or the development one, or nothing. */
export function credentials(): string | null {
  const live = webApp()?.initData;
  if (live) return live;
  const development = process.env.NEXT_PUBLIC_DEV_INIT_DATA;
  return development && development.length > 0 ? development : null;
}

/** Nudges, where the client offers them. Silent everywhere else rather than a special case. */
export const haptics = {
  tap(): void {
    webApp()?.HapticFeedback?.selectionChanged();
  },
  success(): void {
    webApp()?.HapticFeedback?.notificationOccurred("success");
  },
  warning(): void {
    webApp()?.HapticFeedback?.notificationOccurred("warning");
  },
  error(): void {
    webApp()?.HapticFeedback?.notificationOccurred("error");
  },
};

/** Opens the bot's own chat, which is how a guest starts one so the bot may write to them. */
export function openBotChat(botUsername: string, startParam = "reminders"): void {
  const app = webApp();
  const url = `https://t.me/${botUsername}?start=${encodeURIComponent(startParam)}`;
  if (app) {
    app.openTelegramLink(url);
  } else if (typeof window !== "undefined") {
    window.open(url, "_blank", "noopener");
  }
}

/** Opens a chat with a guest, from their username. */
export function openChatWith(username: string): void {
  const app = webApp();
  const url = `https://t.me/${username}`;
  if (app) {
    app.openTelegramLink(url);
  } else if (typeof window !== "undefined") {
    window.open(url, "_blank", "noopener");
  }
}

/** Device safe area plus Telegram's remaining chrome. Both are zero on old clients. */
export function combinedInsets(safe?: Insets, content?: Insets): Insets {
  const device = safe ?? ZERO_INSETS;
  const telegram = content ?? ZERO_INSETS;
  return {
    top: device.top + telegram.top,
    right: device.right + telegram.right,
    bottom: device.bottom + telegram.bottom,
    left: device.left + telegram.left,
  };
}

function readInsets(app: Pick<WebApp, "safeAreaInset" | "contentSafeAreaInset">): Insets {
  return combinedInsets(app.safeAreaInset, app.contentSafeAreaInset);
}

/** Everything about the surface the layout has to fit inside. */
export interface Surface {
  insets: Insets;
  /** The height that stays put, in CSS pixels. Zero before Telegram has answered. */
  stableHeight: number;
}

export const NO_SURFACE: Surface = { insets: ZERO_INSETS, stableHeight: 0 };

/** The surface as Telegram currently reports it. */
export function surfaceOf(app: WebApp): Surface {
  return {
    insets: readInsets(app),
    stableHeight: Math.round(app.viewportStableHeight) || 0,
  };
}

/**
 * Ready, expand, stop vertical swipes collapsing us, paint the chrome, publish the surface.
 *
 * The order matters. `ready` before anything else, because a client that has not been told the app
 * is drawn answers some of these with stale values. `expand` before measuring, because the height
 * measured while collapsed is the wrong one. `disableVerticalSwipes` is feature-detected rather
 * than version-sniffed: an older client simply does not have it, and a drag down a long list will
 * collapse the app there — which is the platform's behaviour, not a bug we can paper over.
 */
export function bootstrapTelegram(app: WebApp, onSurface: (surface: Surface) => void): () => void {
  app.ready();
  app.expand();
  app.disableVerticalSwipes?.();
  app.setHeaderColor?.("bg_color");
  app.setBackgroundColor?.("bg_color");

  const publish = () => onSurface(surfaceOf(app));
  publish();
  for (const event of SURFACE_EVENTS) {
    app.onEvent(event, publish);
  }
  return () => {
    for (const event of SURFACE_EVENTS) {
      app.offEvent(event, publish);
    }
  };
}
