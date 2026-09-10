"use client";

/**
 * Persistent Mini App chrome: staff tabs, insets, no view-name header.
 */

import type { ReactNode } from "react";

import { haptics, type Insets } from "@/lib/telegram";

export type StaffTab = "client" | "shift" | "settings";

const TABS = [
  ["client", "◍", "Моя бронь"],
  ["shift", "▤", "Смена"],
  ["settings", "⚙", "Настройки"],
] as const;

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
      {TABS.map(([value, glyph, label]) => (
        <button
          key={value}
          type="button"
          aria-current={tab === value}
          onClick={() => {
            haptics.tap();
            onTab(value);
          }}
          style={{
            flex: 1,
            minHeight: 44,
            padding: "9px 0 10px",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: 3,
            color: tab === value ? "var(--btn)" : "var(--hint)",
          }}
        >
          <span aria-hidden style={{ fontSize: 17, lineHeight: 1 }}>
            {glyph}
          </span>
          <span style={{ fontSize: 10, fontWeight: 600 }}>{label}</span>
        </button>
      ))}
    </nav>
  );
}

/** Pads a full-screen state (load, fatal) by Telegram insets. */
export function InsetFrame({
  insets,
  children,
}: {
  insets: Insets;
  children: ReactNode;
}) {
  return (
    <div
      style={{
        minHeight: "100vh",
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
  footer,
}: {
  staff: boolean;
  tab: StaffTab;
  onTab: (tab: StaffTab) => void;
  insets: Insets;
  children: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <div
      style={{
        minHeight: "100vh",
        background: "var(--bg)",
        display: "flex",
        flexDirection: "column",
        paddingTop: insets.top,
        paddingLeft: insets.left,
        paddingRight: insets.right,
      }}
    >
      <main style={{ flex: 1, overflowY: "auto", overflowX: "hidden", position: "relative" }}>
        {children}
      </main>
      {footer ? (
        <div
          style={{
            flex: "none",
            padding: staff ? "8px 12px" : `8px 12px ${8 + insets.bottom}px`,
            background: "var(--bg)",
          }}
        >
          {footer}
        </div>
      ) : null}
      {staff ? <StaffMenu tab={tab} onTab={onTab} bottomInset={insets.bottom} /> : null}
    </div>
  );
}
