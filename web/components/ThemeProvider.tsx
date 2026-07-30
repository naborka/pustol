"use client";

/**
 * Paints the app in the user's own Telegram theme.
 *
 * The variables are written on the root element rather than passed down, so any component can reach
 * a colour with `var(--txt)` without a provider in its way. Telegram can change the theme while the
 * app is open — somebody switching to dark mode mid-evening — so the change is listened for rather
 * than read once at start-up.
 */

import { useEffect, useState } from "react";

import { cssVariables, paletteFrom, type ColorScheme } from "@/lib/theme";
import { webApp } from "@/lib/telegram";

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [scheme, setScheme] = useState<ColorScheme>("dark");

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
    app?.ready();
    app?.expand();
    app?.onEvent("themeChanged", apply);

    // Outside Telegram — a browser during development — follow the operating system instead, so the
    // app is at least legible.
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    media?.addEventListener("change", apply);

    return () => {
      app?.offEvent("themeChanged", apply);
      media?.removeEventListener("change", apply);
    };
  }, []);

  return <div data-color-scheme={scheme}>{children}</div>;
}

function preferredScheme(): ColorScheme {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
