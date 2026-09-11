/**
 * Every colour that carries meaning can be read.
 *
 * A palette is picked in a mock-up on a bright laptop and used in a dim bar on a phone at arm's
 * length. This is the arithmetic that tells the difference: WCAG AA for ordinary text, 4.5:1, for
 * every colour that says something against both of the grounds it is ever drawn on.
 *
 * Only the fallbacks are checked, and deliberately. Telegram's own theme can supply any colours it
 * likes and this app honours them; what it must never do is ship a default nobody can read.
 */

import { describe, expect, it } from "vitest";

import { MIN_CONTRAST, baseline, contrastRatio, type ColorScheme } from "../theme";

const MEANINGFUL = ["hint", "txt", "ok", "warn", "dest"] as const;
const GROUNDS = ["bg", "sec"] as const;

describe("the fallback palette", () => {
  for (const scheme of ["dark", "light"] as ColorScheme[]) {
    for (const colour of MEANINGFUL) {
      for (const ground of GROUNDS) {
        it(`${scheme}: ${colour} can be read on ${ground}`, () => {
          const palette = baseline(scheme);
          const ratio = contrastRatio(palette[colour], palette[ground]);
          expect(ratio, `${colour} on ${ground} is not a colour`).not.toBeNull();
          expect(ratio ?? 0).toBeGreaterThanOrEqual(MIN_CONTRAST);
        });
      }
    }
  }

  it("puts the button's own text on the button", () => {
    for (const scheme of ["dark", "light"] as ColorScheme[]) {
      const palette = baseline(scheme);
      expect(contrastRatio(palette.buttonText, palette.btn) ?? 0).toBeGreaterThanOrEqual(3);
    }
  });
});

describe("the contrast arithmetic", () => {
  it("agrees with the two ends of the scale", () => {
    expect(contrastRatio("#ffffff", "#000000")).toBeCloseTo(21, 5);
    expect(contrastRatio("#777777", "#777777")).toBeCloseTo(1, 5);
  });

  it("does not care which way round the pair is given", () => {
    expect(contrastRatio("#123456", "#fedcba")).toBeCloseTo(
      contrastRatio("#fedcba", "#123456") ?? 0,
      10,
    );
  });

  it("reports nothing for what is not a hex colour", () => {
    expect(contrastRatio("rgba(0,0,0,.5)", "#ffffff")).toBeNull();
    expect(contrastRatio("#fff", "var(--bg)")).toBeNull();
  });
});
