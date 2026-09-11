/**
 * The measurements. Every one of them, once.
 *
 * A screen built from literals drifts: a card gets a 14px radius because somebody typed 14, and
 * the one beside it keeps 16, and nobody notices until both are on a phone at the same time. These
 * are the only sizes this app has, so "does this match?" is answered by which name was used rather
 * than by comparing numbers across forty files.
 *
 * Deliberately small. Three radii rather than six, one spacing scale rather than a free number:
 * a system you can hold in your head is a system people use correctly.
 */

/** Corner radii. `pill` is for anything fully rounded. */
export const RADIUS = { sm: 10, md: 12, lg: 16, pill: 999 } as const;

/** The spacing scale, in 4px steps. Indexed, so `SPACE[4]` is 16px. */
export const SPACE = [0, 4, 8, 12, 16, 20, 24, 32] as const;

/**
 * The smallest an interactive element may be, in CSS pixels.
 *
 * Not a suggestion: below this, a thumb on a moving tram misses. Anything a finger can hit is at
 * least this tall and this wide, padding included.
 */
export const TAP = 44;

/** Type sizes, from the smallest legible label to the one number a screen is about. */
export const TEXT = {
  xs: 11,
  sm: 12,
  md: 13,
  base: 14,
  lg: 15,
  xl: 17,
  h2: 22,
  h1: 28,
} as const;

/** The one place a duration is turned into a number of milliseconds. */
export const TIMING = {
  /** How long an undo stays on screen. Six seconds: long enough to read, short enough to trust. */
  undoMs: 6000,
  /** A message with nothing to undo. */
  toastMs: 4200,
} as const;
