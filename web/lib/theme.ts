/**
 * The palette.
 *
 * A Mini App that ignores Telegram's theme looks like a website somebody embedded. Telegram hands
 * the client a set of colours chosen by whatever theme the user is running, and the design's own
 * variables map onto them one for one — which is what makes honouring them a substitution rather
 * than a redesign.
 *
 * Three colours have no Telegram equivalent: the green that says a booking is confirmed, the amber
 * that says a table is closed, and the wash behind a destructive action. The first two are taken
 * from the design per scheme; the third is derived from Telegram's own destructive colour, so a
 * user's red and the wash behind it can never disagree.
 */

export type ColorScheme = "light" | "dark";

/** Every colour the interface uses. */
export interface Palette {
  bg: string;
  sec: string;
  txt: string;
  hint: string;
  btn: string;
  buttonText: string;
  link: string;
  sep: string;
  dest: string;
  ok: string;
  warn: string;
  /** The wash behind a destructive button. */
  tint: string;
  /** A chip that is available but not chosen. */
  chip: string;
  /** A chip that cannot be chosen at all. */
  chipOff: string;
}

/** Telegram's theme, as much of it as this app reads. */
export interface TelegramThemeParams {
  bg_color?: string;
  secondary_bg_color?: string;
  text_color?: string;
  hint_color?: string;
  button_color?: string;
  button_text_color?: string;
  link_color?: string;
  section_separator_color?: string;
  destructive_text_color?: string;
  section_bg_color?: string;
}

/**
 * The fallbacks, chosen so that every one of them can be read.
 *
 * Each colour that carries meaning clears 4.5:1 against both grounds it is ever drawn on — the
 * page and a card — in its own scheme. `contrast.test.ts` asserts it, so a colour picked for how
 * it looks in a mock-up cannot quietly ship at 3:1 and become the thing nobody can read in a dim
 * bar. The old greys were the worst offenders: `#708499` on a card was under four.
 */
const DARK: Palette = {
  bg: "#17212b",
  sec: "#232e3c",
  txt: "#ffffff",
  hint: "#8fa3b8",
  btn: "#5288c1",
  buttonText: "#ffffff",
  link: "#6ab7ff",
  sep: "rgba(255,255,255,.08)",
  dest: "#f2666e",
  ok: "#42c767",
  warn: "#eaa13a",
  tint: "rgba(242,102,110,.14)",
  chip: "#232e3c",
  chipOff: "#1d2733",
};

const LIGHT: Palette = {
  bg: "#ffffff",
  sec: "#f2f2f7",
  txt: "#000000",
  hint: "#5f6b7a",
  btn: "#2481cc",
  buttonText: "#ffffff",
  link: "#2481cc",
  sep: "rgba(0,0,0,.09)",
  dest: "#c9282f",
  ok: "#207a38",
  warn: "#985b00",
  tint: "rgba(201,40,47,.10)",
  chip: "#f2f2f7",
  chipOff: "#f7f7fa",
};

export function baseline(scheme: ColorScheme): Palette {
  return scheme === "light" ? LIGHT : DARK;
}

/** `#rrggbb` or `#rgb` as its three channels, or null if it is neither. */
function channels(color: string): [number, number, number] | null {
  const hex = color.trim().replace(/^#/, "");
  const expanded =
    hex.length === 3
      ? hex
          .split("")
          .map((digit) => digit + digit)
          .join("")
      : hex;
  if (!/^[0-9a-fA-F]{6}$/.test(expanded)) return null;
  return [
    Number.parseInt(expanded.slice(0, 2), 16),
    Number.parseInt(expanded.slice(2, 4), 16),
    Number.parseInt(expanded.slice(4, 6), 16),
  ];
}

/** Relative luminance, as WCAG defines it. */
function luminance(color: string): number | null {
  const parts = channels(color);
  if (!parts) return null;
  const [red, green, blue] = parts.map((value) => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/**
 * The WCAG contrast ratio between two colours, from 1 to 21, or null for anything not a hex colour.
 *
 * Here rather than in a test file because it is the definition of a rule the palette has to obey,
 * and a rule that lives only in its own test is a rule the next palette will not know about.
 */
export function contrastRatio(foreground: string, background: string): number | null {
  const front = luminance(foreground);
  const back = luminance(background);
  if (front === null || back === null) return null;
  const lighter = Math.max(front, back);
  const darker = Math.min(front, back);
  return (lighter + 0.05) / (darker + 0.05);
}

/** The smallest contrast this app will ship: WCAG AA for ordinary text. */
export const MIN_CONTRAST = 4.5;

/** `#rrggbb` or `#rgb` as an `rgba(...)` with the alpha applied, or null if it is neither. */
export function withAlpha(color: string, alpha: number): string | null {
  const hex = color.trim().replace(/^#/, "");
  const expanded =
    hex.length === 3
      ? hex
          .split("")
          .map((digit) => digit + digit)
          .join("")
      : hex;
  if (!/^[0-9a-fA-F]{6}$/.test(expanded)) return null;
  const red = Number.parseInt(expanded.slice(0, 2), 16);
  const green = Number.parseInt(expanded.slice(2, 4), 16);
  const blue = Number.parseInt(expanded.slice(4, 6), 16);
  return `rgba(${red},${green},${blue},${alpha})`;
}

/**
 * Builds the palette from Telegram's theme, falling back per colour rather than wholesale.
 *
 * Per colour matters: Telegram clients differ in which parameters they send, and an older one that
 * omits `section_separator_color` should still get its own background rather than the design's.
 */
export function paletteFrom(
  scheme: ColorScheme,
  theme: TelegramThemeParams | undefined,
): Palette {
  const base = baseline(scheme);
  if (!theme) return base;

  const dest = theme.destructive_text_color ?? base.dest;
  const tintAlpha = scheme === "light" ? 0.1 : 0.14;
  const sec = theme.secondary_bg_color ?? theme.section_bg_color ?? base.sec;

  return {
    bg: theme.bg_color ?? base.bg,
    sec,
    txt: theme.text_color ?? base.txt,
    hint: theme.hint_color ?? base.hint,
    btn: theme.button_color ?? base.btn,
    buttonText: theme.button_text_color ?? base.buttonText,
    link: theme.link_color ?? base.link,
    sep: theme.section_separator_color ?? base.sep,
    dest,
    // No Telegram parameter says "this went well" or "look at this". Taken from the design so they
    // stay legible against whichever background the user's theme supplies.
    ok: base.ok,
    warn: base.warn,
    tint: withAlpha(dest, tintAlpha) ?? base.tint,
    chip: sec,
    chipOff: base.chipOff,
  };
}

/** The palette as CSS custom properties, so a component can style with `var(--txt)`. */
export function cssVariables(palette: Palette): Record<string, string> {
  return {
    "--bg": palette.bg,
    "--sec": palette.sec,
    "--txt": palette.txt,
    "--hint": palette.hint,
    "--btn": palette.btn,
    "--btn-text": palette.buttonText,
    "--link": palette.link,
    "--sep": palette.sep,
    "--dest": palette.dest,
    "--ok": palette.ok,
    "--warn": palette.warn,
    "--tint": palette.tint,
    "--chip": palette.chip,
    "--chip-off": palette.chipOff,
  };
}
