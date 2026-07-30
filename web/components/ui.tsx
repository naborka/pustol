"use client";

/**
 * The pieces every screen is built from.
 *
 * The design repeats a small set of shapes — a card, a chip, a stepper, a sheet — with the same
 * radii and spacing throughout. Naming them once means a change to the shape of a card is one edit
 * rather than forty, and it keeps the screens readable as descriptions of what is on them rather
 * than as walls of style objects.
 */

import type { CSSProperties, ReactNode } from "react";
import { useEffect, useRef } from "react";

import { guests as guestsLabel, time as clockLabel } from "@/lib/format";
import { haptics } from "@/lib/telegram";

export const RADIUS = {
  card: 16,
  chip: 12,
  small: 11,
  sheet: 18,
  pill: 99,
} as const;

/**
 * The colour a booking's status is drawn in, wherever it is drawn.
 *
 * One definition, so the chip in the guest's card and the block on the timeline cannot end up
 * saying the same thing in two colours.
 */
export function statusColor(status: "confirmed" | "arrived" | "no_show" | "cancelled"): string {
  if (status === "arrived") return "var(--ok)";
  if (status === "no_show") return "var(--dest)";
  return "var(--btn)";
}

/** The one place a section title's look is decided. */
export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <span
      style={{
        fontSize: 13,
        fontWeight: 600,
        letterSpacing: ".06em",
        textTransform: "uppercase",
        color: "var(--hint)",
      }}
    >
      {children}
    </span>
  );
}

export function Card({
  children,
  padding = 16,
  gap = 12,
  style,
}: {
  children: ReactNode;
  padding?: number;
  gap?: number;
  style?: CSSProperties;
}) {
  return (
    <div
      style={{
        background: "var(--sec)",
        borderRadius: RADIUS.card,
        padding,
        display: "flex",
        flexDirection: "column",
        gap,
        ...style,
      }}
    >
      {children}
    </div>
  );
}

export function Separator() {
  return <div style={{ height: 1, background: "var(--sep)" }} />;
}

/** A line of explanation under a control, in the design's smallest legible size. */
export function Note({
  children,
  tone = "hint",
}: {
  children: ReactNode;
  tone?: "hint" | "warn" | "ok";
}) {
  const color = tone === "warn" ? "var(--warn)" : tone === "ok" ? "var(--ok)" : "var(--hint)";
  return (
    <span style={{ fontSize: 12, color, lineHeight: 1.5, textWrap: "pretty" }}>{children}</span>
  );
}

/** A label and a value on one line, as the bar-information card uses. */
export function InfoRow({ label, value }: { label: ReactNode; value: ReactNode }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", gap: 12 }}>
      <span style={{ fontSize: 14, color: "var(--hint)" }}>{label}</span>
      <span style={{ fontSize: 14, fontWeight: 600, color: "var(--txt)", textAlign: "right" }}>
        {value}
      </span>
    </div>
  );
}

/**
 * A choice among a few options, as the party-size row and the time grid use.
 *
 * `state` rather than a pair of booleans: a chip is chosen, or available, or unavailable, and those
 * three are the only possibilities. A `selected` plus `disabled` pair would allow a fourth that has
 * no meaning.
 */
export type ChipState = "chosen" | "available" | "unavailable";

export function Chip({
  label,
  state,
  onClick,
  height = 44,
  fontSize = 15,
  style,
  ariaLabel,
}: {
  label: ReactNode;
  state: ChipState;
  onClick?: () => void;
  height?: number;
  fontSize?: number;
  style?: CSSProperties;
  ariaLabel?: string;
}) {
  const chosen = state === "chosen";
  const unavailable = state === "unavailable";
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      aria-pressed={chosen}
      disabled={unavailable || !onClick}
      onClick={() => {
        haptics.tap();
        onClick?.();
      }}
      style={{
        height,
        borderRadius: RADIUS.small,
        background: chosen ? "var(--btn)" : unavailable ? "transparent" : "var(--chip)",
        color: chosen ? "var(--btn-text)" : unavailable ? "var(--hint)" : "var(--txt)",
        fontSize,
        fontWeight: 600,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        ...style,
      }}
    >
      {label}
    </button>
  );
}

/**
 * A grid of arrival times.
 *
 * Shared by the guest's picker and the staff manual-booking sheet, so the rule that decides whether
 * a time can be tapped exists once. Two copies would eventually disagree about what a taken slot
 * looks like, on the two screens most likely to be compared.
 */
export function SlotGrid({
  slots,
  chosen,
  onPick,
  height = 44,
  fontSize = 15,
  gap = 8,
}: {
  slots: { start_minutes: number; state: "free" | "taken" | "past" }[];
  chosen: number | null;
  onPick: (minutes: number) => void;
  height?: number;
  fontSize?: number;
  gap?: number;
}) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap }}>
      {slots.map((slot) => (
        <Chip
          key={slot.start_minutes}
          label={clockLabel(slot.start_minutes)}
          height={height}
          fontSize={fontSize}
          state={
            slot.start_minutes === chosen
              ? "chosen"
              : slot.state === "free"
                ? "available"
                : "unavailable"
          }
          {...(slot.state === "free" ? { onClick: () => onPick(slot.start_minutes) } : {})}
        />
      ))}
    </div>
  );
}

/** How many are coming. */
export function PartySizeRow({
  max,
  value,
  onChange,
  height = 50,
  fontSize = 17,
  gap = 8,
}: {
  max: number;
  value: number;
  onChange: (size: number) => void;
  height?: number;
  fontSize?: number;
  gap?: number;
}) {
  return (
    <div style={{ display: "flex", gap }}>
      {Array.from({ length: max }, (_, index) => index + 1).map((size) => (
        <Chip
          key={size}
          label={size}
          height={height}
          fontSize={fontSize}
          style={{ flex: 1 }}
          state={size === value ? "chosen" : "available"}
          onClick={() => onChange(size)}
          ariaLabel={guestsLabel(size)}
        />
      ))}
    </div>
  );
}

/** The disclosure that unfolds the daytime half of the picker. */
export function DaytimeDisclosure({
  from,
  to,
  onShow,
  compact = false,
}: {
  from: number;
  to: number;
  onShow: () => void;
  compact?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onShow}
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: compact ? "11px 13px" : "12px 14px",
        borderRadius: compact ? 10 : RADIUS.small,
        background: "var(--sec)",
      }}
    >
      <span style={{ fontSize: compact ? 13 : 14, color: "var(--txt)" }}>Днём</span>
      <span style={{ fontSize: compact ? 12 : 13, color: "var(--hint)" }}>
        {clockLabel(from)} — {clockLabel(to)} ›
      </span>
    </button>
  );
}

/** A two- or three-way switch, as the timeline/list toggle and the time-step row use. */
export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  background = "var(--sec)",
}: {
  options: { value: T; label: ReactNode }[];
  value: T;
  onChange: (value: T) => void;
  background?: string;
}) {
  return (
    <div
      role="tablist"
      style={{ display: "flex", background, borderRadius: 9, padding: 2, gap: 2, width: "100%" }}
    >
      {options.map((option) => {
        const chosen = option.value === value;
        return (
          <button
            key={String(option.value)}
            type="button"
            role="tab"
            aria-selected={chosen}
            onClick={() => {
              haptics.tap();
              onChange(option.value);
            }}
            style={{
              flex: 1,
              padding: 9,
              borderRadius: 7,
              textAlign: "center",
              fontSize: 13,
              fontWeight: 600,
              background: chosen ? "var(--btn)" : "transparent",
              color: chosen ? "var(--btn-text)" : "var(--hint)",
            }}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * A value with a minus and a plus.
 *
 * Both buttons are told whether the change they would make is legal, and a button that would be
 * refused is inert and grey. The screen never has to guess: it asks the same validator the server
 * would, so a live-looking control that refuses is impossible.
 */
export function Stepper({
  label,
  value,
  canDecrease,
  canIncrease,
  onDecrease,
  onIncrease,
}: {
  label: ReactNode;
  value: ReactNode;
  canDecrease: boolean;
  canIncrease: boolean;
  onDecrease: () => void;
  onIncrease: () => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "8px 8px 8px 14px",
        background: "var(--sec)",
        borderRadius: RADIUS.chip,
        gap: 8,
      }}
    >
      <span style={{ fontSize: 15, color: "var(--txt)" }}>{label}</span>
      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        <StepperButton sign="−" enabled={canDecrease} onClick={onDecrease} label="меньше" />
        <span
          style={{
            fontSize: 15,
            fontWeight: 600,
            color: "var(--txt)",
            minWidth: 56,
            textAlign: "center",
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {value}
        </span>
        <StepperButton sign="+" enabled={canIncrease} onClick={onIncrease} label="больше" />
      </div>
    </div>
  );
}

export function StepperButton({
  sign,
  enabled,
  onClick,
  label,
  size = 44,
}: {
  sign: string;
  enabled: boolean;
  onClick: () => void;
  label: string;
  size?: number;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      disabled={!enabled}
      onClick={() => {
        haptics.tap();
        onClick();
      }}
      style={{
        width: size,
        height: size,
        borderRadius: RADIUS.small,
        background: "var(--bg)",
        color: enabled ? "var(--link)" : "var(--hint)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        fontSize: 19,
      }}
    >
      {sign}
    </button>
  );
}

/** The wide button at the bottom of a guest screen. */
export function MainButton({
  label,
  onClick,
  enabled = true,
  tone = "primary",
}: {
  label: ReactNode;
  onClick: () => void;
  enabled?: boolean;
  tone?: "primary" | "destructive";
}) {
  const background = !enabled
    ? "var(--chip)"
    : tone === "destructive"
      ? "var(--tint)"
      : "var(--btn)";
  const color = !enabled
    ? "var(--hint)"
    : tone === "destructive"
      ? "var(--dest)"
      : "var(--btn-text)";
  return (
    <button
      type="button"
      disabled={!enabled}
      onClick={() => {
        haptics.tap();
        onClick();
      }}
      style={{
        width: "100%",
        padding: 15,
        borderRadius: RADIUS.chip,
        background,
        color,
        fontSize: 16,
        fontWeight: 600,
        textAlign: "center",
      }}
    >
      {label}
    </button>
  );
}

/** A full-width action inside a card: "+ Добавить стол" and its kin. */
export function CardAction({
  label,
  onClick,
  tone = "link",
}: {
  label: ReactNode;
  onClick: () => void;
  tone?: "link" | "destructive" | "primary";
}) {
  const color =
    tone === "destructive" ? "var(--dest)" : tone === "primary" ? "var(--btn-text)" : "var(--link)";
  const background =
    tone === "destructive" ? "var(--tint)" : tone === "primary" ? "var(--btn)" : "var(--sec)";
  return (
    <button
      type="button"
      onClick={() => {
        haptics.tap();
        onClick();
      }}
      style={{
        padding: 13,
        borderRadius: RADIUS.chip,
        background,
        color,
        fontSize: 15,
        fontWeight: 600,
        textAlign: "center",
      }}
    >
      {label}
    </button>
  );
}

export function TextField({
  value,
  onChange,
  placeholder,
  ariaLabel,
  style,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  ariaLabel?: string;
  style?: CSSProperties;
}) {
  return (
    <input
      value={value}
      aria-label={ariaLabel ?? placeholder}
      placeholder={placeholder}
      onChange={(event) => onChange(event.target.value)}
      style={{
        padding: "13px 14px",
        borderRadius: RADIUS.chip,
        background: "var(--sec)",
        border: "none",
        color: "var(--txt)",
        fontSize: 15,
        width: "100%",
        ...style,
      }}
    />
  );
}

/**
 * The sheet that slides up from the bottom.
 *
 * Escape closes it and focus moves inside it, because a sheet that traps a screen reader behind the
 * page underneath is a sheet only some people can use.
 */
export function Sheet({
  open,
  onClose,
  title,
  children,
}: {
  open: boolean;
  onClose: () => void;
  title?: string;
  children: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return undefined;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    panel.current?.focus();
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <>
      <div
        onClick={onClose}
        aria-hidden
        style={{
          position: "fixed",
          inset: 0,
          background: "rgba(0,0,0,.5)",
          zIndex: 10,
          animation: "fadeIn .16s ease both",
        }}
      />
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        style={{
          position: "fixed",
          left: 0,
          right: 0,
          bottom: 0,
          zIndex: 11,
          background: "var(--bg)",
          borderRadius: `${RADIUS.sheet}px ${RADIUS.sheet}px 0 0`,
          padding: "8px 16px 22px",
          maxHeight: "86%",
          overflowY: "auto",
          animation: "sheetUp .24s cubic-bezier(.2,.8,.3,1) both",
        }}
      >
        <div
          aria-hidden
          style={{
            width: 36,
            height: 4,
            borderRadius: 9,
            background: "var(--sep)",
            margin: "0 auto 14px",
          }}
        />
        {children}
      </div>
    </>
  );
}

export function SheetTitle({ children, tone }: { children: ReactNode; tone?: "destructive" }) {
  return (
    <span
      style={{
        fontSize: 20,
        fontWeight: 700,
        color: tone === "destructive" ? "var(--dest)" : "var(--txt)",
      }}
    >
      {children}
    </span>
  );
}

/** A row of choices in a sheet: a message to send, a reason to give. */
export function SheetChoice({ text, onClick }: { text: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={() => {
        haptics.tap();
        onClick();
      }}
      style={{
        padding: 14,
        background: "var(--sec)",
        borderRadius: RADIUS.chip,
        fontSize: 14,
        color: "var(--txt)",
        lineHeight: 1.4,
      }}
    >
      {text}
    </button>
  );
}

/**
 * The message that appears over the bottom of the screen and leaves again.
 *
 * Announced politely so that somebody using a screen reader hears the outcome of what they just did
 * without having it interrupt them mid-sentence.
 */
export function Toast({ text }: { text: string | null }) {
  if (!text) return null;
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: "fixed",
        left: 14,
        right: 14,
        bottom: 78,
        zIndex: 20,
        background: "rgba(20,24,30,.96)",
        border: "1px solid rgba(255,255,255,.1)",
        borderRadius: 13,
        padding: "13px 15px",
        animation: "toastIn .22s ease both",
      }}
    >
      <span style={{ fontSize: 13, color: "#fff", lineHeight: 1.45, textWrap: "pretty" }}>
        {text}
      </span>
    </div>
  );
}

/** Something is happening. Deliberately plain: a spinner is not information. */
export function Spinner({ label = "Загружаем" }: { label?: string }) {
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 12,
        padding: 48,
      }}
    >
      <span
        aria-hidden
        style={{
          width: 22,
          height: 22,
          borderRadius: RADIUS.pill,
          border: "2px solid var(--sep)",
          borderTopColor: "var(--btn)",
          animation: "spin .8s linear infinite",
        }}
      />
      <span style={{ fontSize: 13, color: "var(--hint)" }}>{label}</span>
    </div>
  );
}

/** A dead end with an explanation, which is better than an empty screen. */
export function Failure({
  message,
  actionLabel,
  onAction,
}: {
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  return (
    <div style={{ padding: "40px 22px", display: "flex", flexDirection: "column", gap: 14 }}>
      <Card padding={18} gap={12} style={{ alignItems: "center", textAlign: "center" }}>
        <span style={{ fontSize: 15, color: "var(--txt)", lineHeight: 1.45, textWrap: "pretty" }}>
          {message}
        </span>
        {actionLabel && onAction ? <CardAction label={actionLabel} onClick={onAction} /> : null}
      </Card>
    </div>
  );
}
