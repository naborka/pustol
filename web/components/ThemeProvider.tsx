"use client";

/**
 * Paints the app in the user's own Telegram theme, and keeps the layout inside the real viewport.
 *
 * Two jobs, one subscription, because they answer to the same events. Colours are written as
 * custom properties on the root element rather than passed down, so any component can reach one
 * with `var(--txt)` without a provider in its way. The viewport is written the same way —
 * `--tg-vh` and the four insets — so the shell can size itself in CSS and a component that needs
 * the numbers can ask [`useViewport`].
 *
 * Telegram can change any of this while the app is open: somebody switches to dark mode
 * mid-evening, a keyboard opens, a phone turns. All of it is listened for rather than read once.
 */

import { createContext, useContext, useEffect, useState } from "react";

import { cssVariables, paletteFrom, type ColorScheme } from "@/lib/theme";
import {
  NO_SURFACE,
  bootstrapTelegram,
  webApp,
  type Insets,
  type Surface,
} from "@/lib/telegram";

const SurfaceContext = createContext<Surface>(NO_SURFACE);

/** The surface the layout has to fit inside: Telegram's stable height and the safe areas. */
export function useViewport(): Surface {
  return useContext(SurfaceContext);
}

/** Just the insets, for the few places that need padding rather than height. */
export function useInsets(): Insets {
  return useContext(SurfaceContext).insets;
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [scheme, setScheme] = useState<ColorScheme>("dark");
  const [surface, setSurface] = useState<Surface>(NO_SURFACE);

  useEffect(() => {
    const app = webApp();

    const apply = () => {
      const live = webApp();
      const next: ColorScheme = live?.colorScheme ?? preferredScheme();
      setScheme(next);
      const palette = paletteFrom(next, live?.themeParams);
      const root = document.documentElement;
      for (const [name, value] of Object.entries(cssVariables(palette))) {
        root.style.setProperty(name, value);
      }
      root.style.colorScheme = next;
    };

    apply();

    const publish = (next: Surface) => {
      setSurface(next);
      const root = document.documentElement;
      root.style.setProperty("--inset-top", `${next.insets.top}px`);
      root.style.setProperty("--inset-right", `${next.insets.right}px`);
      root.style.setProperty("--inset-bottom", `${next.insets.bottom}px`);
      root.style.setProperty("--inset-left", `${next.insets.left}px`);
      // Only written once Telegram has answered with a real height. Writing a zero would collapse
      // the shell for the frame before the first `viewportChanged`, and the CSS fallback — the
      // browser's own dynamic viewport — is right everywhere this app runs outside Telegram.
      if (next.stableHeight > 0) {
        root.style.setProperty("--tg-vh", `${next.stableHeight}px`);
      }
    };
    const stopSurface = app ? bootstrapTelegram(app, publish) : undefined;
    app?.onEvent("themeChanged", apply);

    // Outside Telegram — a browser during development — follow the operating system instead, so the
    // app is at least legible.
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    media?.addEventListener("change", apply);

    return () => {
      stopSurface?.();
      app?.offEvent("themeChanged", apply);
      media?.removeEventListener("change", apply);
    };
  }, []);

  return (
    <SurfaceContext.Provider value={surface}>
      <div data-color-scheme={scheme}>{children}</div>
    </SurfaceContext.Provider>
  );
}

function preferredScheme(): ColorScheme {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
