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

export interface WebApp {
  initData: string;
  initDataUnsafe?: { user?: WebAppUser };
  colorScheme: "light" | "dark";
  themeParams: Record<string, string>;
  version: string;
  isExpanded: boolean;
  viewportStableHeight: number;
  ready: () => void;
  expand: () => void;
  close: () => void;
  onEvent: (event: string, handler: () => void) => void;
  offEvent: (event: string, handler: () => void) => void;
  openTelegramLink: (url: string) => void;
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
