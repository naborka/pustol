import { describe, expect, it } from "vitest";

import { baseline, cssVariables, paletteFrom, withAlpha } from "../theme";

describe("the palette", () => {
  it("uses the design's colours when Telegram sends none", () => {
    expect(paletteFrom("dark", undefined)).toEqual(baseline("dark"));
    expect(paletteFrom("light", undefined)).toEqual(baseline("light"));
  });

  it("takes the user's own colours where Telegram provides them", () => {
    const palette = paletteFrom("dark", {
      bg_color: "#101010",
      secondary_bg_color: "#202020",
      text_color: "#fafafa",
      hint_color: "#909090",
      button_color: "#00a3ff",
      button_text_color: "#001018",
      link_color: "#66ccff",
      section_separator_color: "rgba(255,255,255,.2)",
      destructive_text_color: "#ff3b30",
    });
    expect(palette.bg).toBe("#101010");
    expect(palette.sec).toBe("#202020");
    expect(palette.txt).toBe("#fafafa");
    expect(palette.btn).toBe("#00a3ff");
    expect(palette.buttonText).toBe("#001018");
    expect(palette.link).toBe("#66ccff");
    expect(palette.sep).toBe("rgba(255,255,255,.2)");
    expect(palette.dest).toBe("#ff3b30");
  });

  it("falls back one colour at a time, not all or nothing", () => {
    // An older client sends a background and nothing else. It should still get its own background.
    const palette = paletteFrom("dark", { bg_color: "#000000" });
    expect(palette.bg).toBe("#000000");
    expect(palette.sep).toBe(baseline("dark").sep);
    expect(palette.hint).toBe(baseline("dark").hint);
  });

  it("derives the destructive wash from the user's own red", () => {
    // A red button on a wash of a different red is the sort of detail that makes an app feel wrong
    // without anybody being able to say why.
    const palette = paletteFrom("dark", { destructive_text_color: "#ff0000" });
    expect(palette.tint).toBe("rgba(255,0,0,0.14)");
    const lighter = paletteFrom("light", { destructive_text_color: "#ff0000" });
    expect(lighter.tint).toBe("rgba(255,0,0,0.1)");
  });

  it("keeps the design's wash when the red is not a colour it can read", () => {
    const palette = paletteFrom("dark", { destructive_text_color: "var(--something)" });
    expect(palette.tint).toBe(baseline("dark").tint);
  });

  it("takes the section background when there is no secondary one", () => {
    const palette = paletteFrom("dark", { section_bg_color: "#123456" });
    expect(palette.sec).toBe("#123456");
    expect(palette.chip).toBe("#123456");
  });

  it("keeps its own colours for outcomes Telegram has no word for", () => {
    const palette = paletteFrom("dark", { bg_color: "#000" });
    expect(palette.ok).toBe(baseline("dark").ok);
    expect(palette.warn).toBe(baseline("dark").warn);
  });
});

describe("alpha", () => {
  it("expands both hex forms", () => {
    expect(withAlpha("#ff0000", 0.5)).toBe("rgba(255,0,0,0.5)");
    expect(withAlpha("#f00", 0.5)).toBe("rgba(255,0,0,0.5)");
    expect(withAlpha("FF8800", 1)).toBe("rgba(255,136,0,1)");
  });

  it("returns nothing for what is not a hex colour", () => {
    expect(withAlpha("rgba(1,2,3,.4)", 0.5)).toBeNull();
    expect(withAlpha("", 0.5)).toBeNull();
    expect(withAlpha("#12345", 0.5)).toBeNull();
  });
});

describe("custom properties", () => {
  it("names every colour a component can ask for", () => {
    const variables = cssVariables(baseline("dark"));
    for (const name of [
      "--bg",
      "--sec",
      "--txt",
      "--hint",
      "--btn",
      "--btn-text",
      "--link",
      "--sep",
      "--dest",
      "--ok",
      "--warn",
      "--tint",
      "--chip",
      "--chip-off",
    ]) {
      expect(variables[name], name).toBeTruthy();
    }
  });
});
