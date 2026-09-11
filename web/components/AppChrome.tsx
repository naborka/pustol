"use client";

/**
 * The shell: one fixed-height box, one scrolling element, and the bars that never move.
 *
 * This is the fix for the complaint that the app misbehaves inside Telegram while looking right in
 * a desktop browser. Three rules, and all three are load-bearing.
 *
 * *The shell is exactly `--tg-vh` tall.* Not `100vh`, not `100dvh`, not `window.innerHeight`: in
 * the Telegram webview the expanded viewport is taller than the part the user can see, so a layout
 * sized from any of those pushes its bottom bar under Telegram's own chrome.
 *
 * *Exactly one element scrolls.* The `<main>`, with `overscroll-behavior: contain` so a flick at
 * the end of a list stops there instead of travelling to the page and, on Android, to Telegram's
 * dismiss gesture.
 *
 * *The bars are siblings, not overlays.* Header, action bar and tab bar are `flex: none` and sit
 * outside the scroller, so nothing can scroll under them and nothing has to be padded around them.
 */

import type { ReactNode } from "react";

import type { Insets } from "@/lib/telegram";
import { RADIUS, SPACE, TAP, TEXT } from "@/lib/tokens";
import { Pressable } from "./ui";

export type StaffTab = "client" | "shift" | "settings";

/**
 * The tabs, by name.
 *
 * No glyphs. Android's WebView substitutes a font per device for characters like `◍`, so the same
 * build drew three different marks on three phones — and one of them drew boxes. A word is the same
 * word everywhere, and the bar above the active one says which is which without relying on colour.
 */
const TABS: [StaffTab, string][] = [
  ["shift", "Смена"],
  ["client", "Моя бронь"],
  ["settings", "Настройки"],
];

export function StaffMenu({
  tab,
  onTab,
  bottomInset,
}: {
  tab: StaffTab;
  onTab: (tab: StaffTab) => void;
  bottomInset: number;
}) {
  return (
    <nav
      aria-label="Разделы"
      style={{
        flex: "none",
        display: "flex",
        borderTop: "1px solid var(--sep)",
        background: "var(--sec)",
        paddingBottom: bottomInset,
      }}
    >
      {TABS.map(([value, label]) => {
        const current = tab === value;
        return (
          <Pressable
            key={value}
            ariaCurrent={current}
            onClick={() => onTab(value)}
            style={{
              flex: 1,
              minHeight: TAP + 8,
              padding: `${SPACE[1]}px 0 ${SPACE[2]}px`,
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              gap: SPACE[1],
              color: current ? "var(--btn)" : "var(--hint)",
            }}
          >
            <span
              aria-hidden
              style={{
                width: 18,
                height: 2,
                borderRadius: RADIUS.pill,
                background: current ? "var(--btn)" : "transparent",
              }}
            />
            <span style={{ fontSize: TEXT.sm, fontWeight: 600 }}>{label}</span>
          </Pressable>
        );
      })}
    </nav>
  );
}

/** The shell's own height, with the browser's dynamic viewport as the fallback outside Telegram. */
const SHELL_HEIGHT = "var(--tg-vh, 100dvh)";

/** Pads a full-screen state (loading, dead end) by Telegram's insets. */
export function InsetFrame({ insets, children }: { insets: Insets; children: ReactNode }) {
  return (
    <div
      style={{
        height: SHELL_HEIGHT,
        overflow: "hidden",
        background: "var(--bg)",
        paddingTop: insets.top,
        paddingRight: insets.right,
        paddingBottom: insets.bottom,
        paddingLeft: insets.left,
      }}
    >
      {children}
    </div>
  );
}

export function AppShell({
  staff,
  tab,
  onTab,
  insets,
  children,
  header,
  footer,
  toast,
  sheet,
}: {
  staff: boolean;
  tab: StaffTab;
  onTab: (tab: StaffTab) => void;
  insets: Insets;
  children: ReactNode;
  /** Above the scroller and never scrolling with it. */
  header?: ReactNode;
  /** The one primary action of the screen. */
  footer?: ReactNode;
  toast?: ReactNode;
  /** Rendered inside the shell, so a panel can never position itself off the visible box. */
  sheet?: ReactNode;
}) {
  return (
    <div
      style={{
        position: "relative",
        height: SHELL_HEIGHT,
        overflow: "hidden",
        background: "var(--bg)",
        display: "flex",
        flexDirection: "column",
        paddingTop: insets.top,
        paddingLeft: insets.left,
        paddingRight: insets.right,
      }}
    >
      {header ? <div style={{ flex: "none" }}>{header}</div> : null}

      <main
        style={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overflowX: "hidden",
          overscrollBehavior: "contain",
          WebkitOverflowScrolling: "touch",
        }}
      >
        {children}
      </main>

      {/*
        A slot of no height at all, directly above whichever bars this screen has. The toast is
        absolutely positioned inside it, so the layout decides where it sits and no component has
        to know how tall the tab bar is.
      */}
      <div style={{ flex: "none", position: "relative", height: 0 }}>{toast}</div>

      {footer ? (
        <div
          style={{
            flex: "none",
            padding: staff
              ? `${SPACE[2]}px ${SPACE[3]}px`
              : `${SPACE[2]}px ${SPACE[3]}px ${SPACE[2] + insets.bottom}px`,
            background: "var(--bg)",
          }}
        >
          {footer}
        </div>
      ) : null}

      {staff ? <StaffMenu tab={tab} onTab={onTab} bottomInset={insets.bottom} /> : null}

      {sheet}
    </div>
  );
}
