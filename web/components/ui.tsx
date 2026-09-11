"use client";

/**
 * The pieces every screen is built from.
 *
 * Two rules hold this file together. Nothing below hard-codes a radius, a spacing value or a
 * colour — they come from `lib/tokens.ts` and from the theme's custom properties. And nothing
 * interactive is a bare `<button>`: everything a finger can hit goes through [`Pressable`], which
 * is the one place a pressed state and a minimum tap target are decided.
 */

import type { CSSProperties, ReactNode } from "react";
import { useEffect, useRef } from "react";

import { guests as guestsLabel, time as clockLabel } from "@/lib/format";
import { haptics } from "@/lib/telegram";
import { RADIUS, SPACE, TAP, TEXT } from "@/lib/tokens";
import { useViewport } from "./ThemeProvider";

// ---- the one interactive primitive -------------------------------------------------------------

/**
 * Anything a finger can hit.
 *
 * `-webkit-tap-highlight-color: transparent` removes the platform's own feedback, so every control
 * has to supply its own or the app feels dead in the hand. Rather than trust forty components to
 * remember, there is one of these and everything uses it.
 *
 * `tone="card"` adds the slight squash a large surface wants; a small chip only fades.
 */
export function Pressable({
  children,
  onClick,
  disabled = false,
  tone = "plain",
  style,
  ariaLabel,
  ariaPressed,
  ariaCurrent,
  role,
  haptic = "tap",
  title,
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  tone?: "plain" | "card";
  style?: CSSProperties;
  ariaLabel?: string;
  ariaPressed?: boolean;
  ariaCurrent?: boolean;
  role?: string;
  haptic?: "tap" | "none";
  title?: string;
}) {
  return (
    <button
      type="button"
      className={tone === "card" ? "pressable-card" : "pressable"}
      disabled={disabled || !onClick}
      onClick={() => {
        if (haptic === "tap") haptics.tap();
        onClick?.();
      }}
      {...(ariaLabel === undefined ? {} : { "aria-label": ariaLabel })}
      {...(ariaPressed === undefined ? {} : { "aria-pressed": ariaPressed })}
      {...(ariaCurrent === undefined ? {} : { "aria-current": ariaCurrent })}
      {...(role === undefined ? {} : { role })}
      {...(title === undefined ? {} : { title })}
      style={{
        minHeight: TAP,
        display: "flex",
        alignItems: "center",
        ...style,
      }}
    >
      {children}
    </button>
  );
}

// ---- surfaces ----------------------------------------------------------------------------------

export function Card({
  children,
  padding = SPACE[4],
  gap = SPACE[3],
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
        borderRadius: RADIUS.lg,
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

/** The one place a section title's look is decided. */
export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <span
      style={{
        fontSize: TEXT.md,
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

/** A line of explanation under a control. */
export function Note({
  children,
  tone = "hint",
}: {
  children: ReactNode;
  tone?: "hint" | "warn" | "ok" | "dest";
}) {
  const color =
    tone === "warn"
      ? "var(--warn)"
      : tone === "ok"
        ? "var(--ok)"
        : tone === "dest"
          ? "var(--dest)"
          : "var(--hint)";
  return (
    <span style={{ fontSize: TEXT.sm, color, lineHeight: 1.5, textWrap: "pretty" }}>
      {children}
    </span>
  );
}

/** A label and a value on one line. */
export function InfoRow({ label, value }: { label: ReactNode; value: ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        justifyContent: "space-between",
        alignItems: "baseline",
        gap: SPACE[3],
      }}
    >
      <span style={{ fontSize: TEXT.base, color: "var(--hint)" }}>{label}</span>
      <span
        style={{
          fontSize: TEXT.base,
          fontWeight: 600,
          color: "var(--txt)",
          textAlign: "right",
        }}
      >
        {value}
      </span>
    </div>
  );
}

/** A coloured dot, for a status that also says its name in words beside it. */
export function Dot({ color, size = 7 }: { color: string; size?: number }) {
  return (
    <span
      aria-hidden
      style={{
        width: size,
        height: size,
        borderRadius: RADIUS.pill,
        background: color,
        flex: "none",
      }}
    />
  );
}

// ---- choices -----------------------------------------------------------------------------------

/**
 * A choice among a few options.
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
  height = TAP,
  fontSize = TEXT.lg,
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
    <Pressable
      {...(onClick ? { onClick } : {})}
      ariaPressed={chosen}
      {...(ariaLabel === undefined ? {} : { ariaLabel })}
      style={{
        minHeight: height,
        height,
        borderRadius: RADIUS.sm,
        background: chosen ? "var(--btn)" : unavailable ? "transparent" : "var(--chip)",
        border: unavailable ? "1px solid var(--sep)" : "1px solid transparent",
        color: chosen ? "var(--btn-text)" : unavailable ? "var(--hint)" : "var(--txt)",
        textDecoration: unavailable ? "line-through" : "none",
        fontSize,
        fontWeight: 600,
        justifyContent: "center",
        ...style,
      }}
    >
      {label}
    </Pressable>
  );
}

/**
 * How many are coming.
 *
 * A wrapping grid of six columns rather than one flexible row. At a limit of ten on a 360px screen
 * a row of equal shares gives each chip twenty-six pixels, which is not a target — it is a dare.
 */
export function PartySizeGrid({
  max,
  value,
  onChange,
}: {
  max: number;
  value: number;
  onChange: (size: number) => void;
}) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(6, 1fr)",
        gap: SPACE[2],
      }}
    >
      {Array.from({ length: max }, (_, index) => index + 1).map((size) => (
        <Chip
          key={size}
          label={size}
          height={48}
          fontSize={TEXT.xl}
          state={size === value ? "chosen" : "available"}
          onClick={() => onChange(size)}
          ariaLabel={guestsLabel(size)}
        />
      ))}
    </div>
  );
}

/**
 * A grid of arrival times.
 *
 * Times that have gone are not drawn at all — a grey 18:00 at nine in the evening is not
 * information. Times somebody else has is drawn struck through, because watching the evening fill
 * up *is* information, and tapping one says so rather than doing nothing: a control that ignores a
 * finger reads as a broken app, not as a full evening.
 */
export function SlotGrid({
  slots,
  chosen,
  onPick,
  onTaken,
  height = TAP,
  fontSize = TEXT.lg,
  gap = SPACE[2],
}: {
  slots: { start_minutes: number; state: "free" | "taken" | "past" }[];
  chosen: number | null;
  onPick: (minutes: number) => void;
  onTaken: () => void;
  height?: number;
  fontSize?: number;
  gap?: number;
}) {
  const offered = slots.filter((slot) => slot.state !== "past");
  return (
    <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap }}>
      {offered.map((slot) => {
        const taken = slot.state === "taken";
        return (
          <Chip
            key={slot.start_minutes}
            label={clockLabel(slot.start_minutes)}
            height={height}
            fontSize={fontSize}
            state={
              slot.start_minutes === chosen ? "chosen" : taken ? "unavailable" : "available"
            }
            onClick={taken ? onTaken : () => onPick(slot.start_minutes)}
          />
        );
      })}
    </div>
  );
}

/** A two- or three-way switch. */
export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  background = "var(--sec)",
  label,
}: {
  options: { value: T; label: ReactNode }[];
  value: T;
  onChange: (value: T) => void;
  background?: string;
  label?: string;
}) {
  return (
    <div
      role="tablist"
      {...(label === undefined ? {} : { "aria-label": label })}
      style={{
        display: "flex",
        background,
        borderRadius: RADIUS.sm,
        padding: 2,
        gap: 2,
        width: "100%",
      }}
    >
      {options.map((option) => {
        const chosen = option.value === value;
        return (
          <Pressable
            key={String(option.value)}
            role="tab"
            ariaPressed={chosen}
            onClick={() => onChange(option.value)}
            style={{
              flex: 1,
              minHeight: 40,
              padding: SPACE[2],
              borderRadius: RADIUS.sm - 2,
              justifyContent: "center",
              fontSize: TEXT.md,
              fontWeight: 600,
              background: chosen ? "var(--btn)" : "transparent",
              color: chosen ? "var(--btn-text)" : "var(--hint)",
            }}
          >
            {option.label}
          </Pressable>
        );
      })}
    </div>
  );
}

/**
 * A horizontal rail of chips.
 *
 * `touch-action: pan-x` and `overscroll-behavior-x: contain` together are what stop a sideways drag
 * reaching Telegram's own gesture handler, which would collapse the app mid-swipe.
 */
export function Rail({
  children,
  label,
  style,
}: {
  children: ReactNode;
  label?: string;
  style?: CSSProperties;
}) {
  return (
    <div
      {...(label === undefined ? {} : { role: "group", "aria-label": label })}
      style={{
        display: "flex",
        gap: SPACE[2],
        overflowX: "auto",
        overflowY: "hidden",
        touchAction: "pan-x",
        overscrollBehaviorX: "contain",
        WebkitOverflowScrolling: "touch",
        paddingBottom: 2,
        ...style,
      }}
    >
      {children}
    </div>
  );
}

// ---- steppers ----------------------------------------------------------------------------------

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
        padding: `${SPACE[2]}px ${SPACE[2]}px ${SPACE[2]}px ${SPACE[4]}px`,
        background: "var(--sec)",
        borderRadius: RADIUS.md,
        gap: SPACE[2],
      }}
    >
      <span style={{ fontSize: TEXT.lg, color: "var(--txt)" }}>{label}</span>
      <div style={{ display: "flex", alignItems: "center", gap: SPACE[1] }}>
        <StepperButton sign="−" enabled={canDecrease} onClick={onDecrease} label="меньше" />
        <span
          style={{
            fontSize: TEXT.lg,
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
  size = TAP,
}: {
  sign: string;
  enabled: boolean;
  onClick: () => void;
  label: string;
  size?: number;
}) {
  return (
    <Pressable
      ariaLabel={label}
      disabled={!enabled}
      onClick={onClick}
      style={{
        width: size,
        minHeight: size,
        height: size,
        borderRadius: RADIUS.sm,
        background: "var(--bg)",
        color: enabled ? "var(--link)" : "var(--hint)",
        justifyContent: "center",
        fontSize: 19,
      }}
    >
      {sign}
    </Pressable>
  );
}

// ---- actions -----------------------------------------------------------------------------------

/** The wide button along the bottom of a screen: the one decision it is asking for. */
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
    <Pressable
      disabled={!enabled}
      onClick={onClick}
      tone="card"
      style={{
        width: "100%",
        minHeight: 50,
        padding: SPACE[4],
        borderRadius: RADIUS.md,
        background,
        color,
        fontSize: 16,
        fontWeight: 600,
        justifyContent: "center",
        textAlign: "center",
      }}
    >
      {label}
    </Pressable>
  );
}

/** A full-width action inside a card. */
export function CardAction({
  label,
  onClick,
  tone = "link",
  disabled = false,
}: {
  label: ReactNode;
  onClick: () => void;
  tone?: "link" | "destructive" | "primary" | "warn";
  disabled?: boolean;
}) {
  const color =
    tone === "destructive"
      ? "var(--dest)"
      : tone === "primary"
        ? "var(--btn-text)"
        : tone === "warn"
          ? "var(--warn)"
          : "var(--link)";
  const background =
    tone === "destructive"
      ? "var(--tint)"
      : tone === "primary"
        ? "var(--btn)"
        : tone === "warn"
          ? "var(--warn-wash)"
          : "var(--sec)";
  return (
    <Pressable
      onClick={onClick}
      disabled={disabled}
      style={{
        padding: SPACE[3],
        minHeight: TAP,
        borderRadius: RADIUS.md,
        background: disabled ? "var(--chip)" : background,
        color: disabled ? "var(--hint)" : color,
        fontSize: TEXT.lg,
        fontWeight: 600,
        justifyContent: "center",
        textAlign: "center",
      }}
    >
      {label}
    </Pressable>
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
        padding: `${SPACE[3]}px ${SPACE[4]}px`,
        minHeight: TAP,
        borderRadius: RADIUS.md,
        background: "var(--sec)",
        border: "none",
        color: "var(--txt)",
        fontSize: TEXT.lg,
        width: "100%",
        ...style,
      }}
    />
  );
}

// ---- the sheet ---------------------------------------------------------------------------------

/**
 * The panel that slides up from the bottom.
 *
 * Rendered *inside* the shell, absolutely rather than fixed. A fixed panel positions itself against
 * the visual viewport, which in the Telegram webview is not the box the app is drawn in: the panel
 * ends up under Telegram's chrome on iOS and under the keyboard on Android. Absolute inside a shell
 * that is exactly `--tg-vh` tall cannot be anywhere but on screen.
 *
 * When the keyboard opens, Telegram reports a shorter viewport, the shell shrinks, and this panel —
 * being a percentage of it — shrinks with it. The focused field is then scrolled into view *inside
 * the panel*, never by moving the page, which is what used to make the sheet jump.
 */
export function Sheet({
  open,
  onClose,
  title,
  children,
  footer,
}: {
  open: boolean;
  onClose: () => void;
  title?: string;
  children: ReactNode;
  /**
   * The sheet's own decision, pinned below the scrolling body.
   *
   * A sheet with a text field in it loses most of its height to the keyboard, and an action at the
   * bottom of the scroller goes with it — so the one button the sheet exists for would be the one
   * thing the guest cannot reach. Outside the scroller it cannot move.
   */
  footer?: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);

  // Once, on opening. Kept apart from the key listener, which has to track the current `onClose`
  // — sharing an effect meant every keystroke stole the focus and shut the phone keyboard.
  useEffect(() => {
    if (open) panel.current?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return undefined;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // The keyboard has just changed the height of everything. Keep whatever is being typed into in
  // sight, inside the panel's own scroller.
  //
  // Driven by Telegram's own `viewportChanged`, which `useViewport` publishes: the iOS keyboard
  // moves the visual viewport without firing a window resize, so a resize listener would miss the
  // one case this exists for. The listener stays for the browsers outside Telegram that only have
  // that signal.
  const { stableHeight } = useViewport();
  useEffect(() => {
    if (!open) return undefined;
    const keepFocusVisible = () => {
      const focused = document.activeElement;
      // The panel itself takes focus when it opens, and scrolling a scroller into itself is not
      // what this is for: only something being typed into needs keeping in sight.
      if (
        focused instanceof HTMLElement &&
        focused !== panel.current &&
        panel.current?.contains(focused)
      ) {
        focused.scrollIntoView({ block: "nearest" });
      }
    };
    keepFocusVisible();
    window.addEventListener("resize", keepFocusVisible);
    return () => window.removeEventListener("resize", keepFocusVisible);
  }, [open, stableHeight]);

  if (!open) return null;
  return (
    <>
      <div
        onClick={onClose}
        aria-hidden
        style={{
          position: "absolute",
          inset: 0,
          // The one colour in this file that is not from the theme, and deliberately: a dim is
          // darkness. Deriving it from the palette would make it white in a light scheme, which
          // is a fog rather than a dim.
          background: "rgba(0,0,0,.5)",
          zIndex: 10,
          animation: "fadeIn .16s ease both",
        }}
      />
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        {...(title === undefined ? {} : { "aria-label": title })}
        tabIndex={-1}
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          bottom: 0,
          zIndex: 11,
          background: "var(--bg)",
          borderRadius: `${RADIUS.lg}px ${RADIUS.lg}px 0 0`,
          padding: `${SPACE[2]}px ${SPACE[4]}px calc(${SPACE[5]}px + var(--inset-bottom, 0px))`,
          maxHeight: "88%",
          display: "flex",
          flexDirection: "column",
          animation: "sheetUp .24s cubic-bezier(.2,.8,.3,1) both",
        }}
      >
        <div
          aria-hidden
          style={{
            flex: "none",
            width: 36,
            height: 4,
            borderRadius: RADIUS.pill,
            background: "var(--sep)",
            margin: `0 auto ${SPACE[3]}px`,
          }}
        />
        <div
          style={{
            flex: "1 1 auto",
            minHeight: 0,
            overflowY: "auto",
            overscrollBehavior: "contain",
            WebkitOverflowScrolling: "touch",
          }}
        >
          {children}
        </div>
        {footer ? (
          <div style={{ flex: "none", paddingTop: SPACE[3] }}>{footer}</div>
        ) : null}
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
    <Pressable
      onClick={onClick}
      tone="card"
      style={{
        padding: SPACE[3],
        background: "var(--sec)",
        borderRadius: RADIUS.md,
        fontSize: TEXT.base,
        color: "var(--txt)",
        lineHeight: 1.4,
      }}
    >
      {text}
    </Pressable>
  );
}

// ---- outcomes ----------------------------------------------------------------------------------

/** Something the app just did, and — where the act is reversible — the way back. */
export interface ToastMessage {
  text: string;
  undo?: { label: string; run: () => void };
}

/**
 * The message that appears over the bottom of the screen and leaves again.
 *
 * Positioned by the layout rather than by a number: it is rendered in a zero-height slot that sits
 * directly above whichever bars a screen has, so it can never cover the main button on a guest
 * screen or hide behind the tab bar on a staff one. The old hard-coded 78px was the staff tab bar's
 * height, and it was wrong on every other screen.
 *
 * Drawn in the theme's own text colour with the page colour on it — inverted, so it stands off the
 * screen in either scheme and its contrast is the `txt`-on-`bg` pair the palette test already
 * guarantees. The old one was a fixed dark panel, which in a light theme was a black box.
 */
export function Toast({ message }: { message: ToastMessage | null }) {
  if (!message) return null;
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: "absolute",
        left: SPACE[3],
        right: SPACE[3],
        bottom: SPACE[2],
        zIndex: 9,
        background: "var(--txt)",
        borderRadius: RADIUS.md,
        padding: `${SPACE[3]}px ${SPACE[3]}px ${SPACE[3]}px ${SPACE[4]}px`,
        display: "flex",
        alignItems: "center",
        gap: SPACE[3],
        animation: "toastIn .22s ease both",
      }}
    >
      <span
        style={{
          flex: 1,
          fontSize: TEXT.md,
          color: "var(--bg)",
          lineHeight: 1.45,
          textWrap: "pretty",
        }}
      >
        {message.text}
      </span>
      {message.undo ? (
        <Pressable
          onClick={message.undo.run}
          style={{
            flex: "none",
            minHeight: TAP,
            padding: `0 ${SPACE[2]}px`,
            color: "var(--bg)",
            fontSize: TEXT.base,
            fontWeight: 700,
            textDecoration: "underline",
          }}
        >
          {message.undo.label}
        </Pressable>
      ) : null}
    </div>
  );
}

/** Something is happening, and a word for what. A spinner on its own is not information. */
export function Spinner({ label = "Загружаем" }: { label?: string }) {
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: SPACE[3],
        padding: SPACE[7],
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
      <span style={{ fontSize: TEXT.md, color: "var(--hint)" }}>{label}</span>
    </div>
  );
}

/** Nothing here, and why that is fine. */
export function Empty({ title, detail }: { title: string; detail?: string }) {
  return (
    <div
      style={{
        padding: `${SPACE[6]}px ${SPACE[4]}px`,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: SPACE[2],
        textAlign: "center",
      }}
    >
      <span style={{ fontSize: TEXT.lg, fontWeight: 600, color: "var(--txt)" }}>{title}</span>
      {detail ? <Note>{detail}</Note> : null}
    </div>
  );
}

/** A dead end with an explanation and, where there is one, a way out. */
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
    <div
      style={{
        padding: `${SPACE[7]}px ${SPACE[5]}px`,
        display: "flex",
        flexDirection: "column",
        gap: SPACE[3],
      }}
    >
      <Card padding={SPACE[4]} gap={SPACE[3]} style={{ alignItems: "center", textAlign: "center" }}>
        <span
          style={{
            fontSize: TEXT.lg,
            color: "var(--txt)",
            lineHeight: 1.45,
            textWrap: "pretty",
          }}
        >
          {message}
        </span>
        {actionLabel && onAction ? (
          <CardAction label={actionLabel} onClick={onAction} />
        ) : null}
      </Card>
    </div>
  );
}
