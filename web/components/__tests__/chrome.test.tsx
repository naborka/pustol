/**
 * The shell: one fixed-height box, one scroller, bars that never move, and a sheet that cannot
 * position itself off the visible part of the app.
 *
 * These are the rules that make the difference between "looks right in Chrome" and "works inside
 * Telegram", and none of them is visible by reading a screen component.
 */

import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppShell, InsetFrame } from "../AppChrome";
import { Pressable, Sheet, Toast } from "../ui";
import { ZERO_INSETS } from "@/lib/telegram";
import { noop } from "./fixtures";

afterEach(cleanup);

function shell(props: Partial<Parameters<typeof AppShell>[0]> = {}) {
  return render(
    <AppShell staff tab="shift" onTab={noop} insets={ZERO_INSETS} {...props}>
      смена
    </AppShell>,
  );
}

describe("the shell", () => {
  it("is exactly as tall as Telegram says the app is, and never scrolls itself", () => {
    // `100vh` and `window.innerHeight` are both taller than the part of a Telegram webview the
    // user can see, which is what used to push the main button under Telegram's own chrome.
    const { container } = shell();
    const root = container.firstElementChild as HTMLElement;
    expect(root.style.height).toBe("var(--tg-vh, 100dvh)");
    expect(root.style.overflow).toBe("hidden");
    expect(root.style.position).toBe("relative");
  });

  it("has exactly one scrolling element, and it stops its own overscroll", () => {
    const { container } = shell();
    const main = container.querySelector("main") as HTMLElement;
    expect(main.style.overflowY).toBe("auto");
    expect(main.style.overscrollBehavior).toBe("contain");
    const scrollers = [...container.querySelectorAll("*")].filter(
      (node) => (node as HTMLElement).style.overflowY === "auto",
    );
    expect(scrollers).toHaveLength(1);
  });

  it("keeps the bars outside the scroller so nothing can slide under them", () => {
    const { container } = shell({
      header: <div>шапка</div>,
      footer: <button type="button">действие</button>,
    });
    const main = container.querySelector("main") as HTMLElement;
    expect(within(main).queryByText("шапка")).toBeNull();
    expect(within(main).queryByText("действие")).toBeNull();
    expect(screen.getByText("шапка")).toBeDefined();
    expect(screen.getByText("действие")).toBeDefined();
  });

  it("insets the top and the tab bar's bottom so Telegram's chrome is not covered", () => {
    const { container } = shell({ insets: { top: 47, right: 0, bottom: 34, left: 0 } });
    const root = container.firstElementChild as HTMLElement;
    expect(root.style.paddingTop).toBe("47px");
    expect(screen.getByRole("navigation", { name: "Разделы" }).style.paddingBottom).toBe("34px");
  });
});

describe("the tab bar", () => {
  it("names its tabs in words and marks the current one with a bar, not a glyph", async () => {
    // Android's WebView substitutes a font per device for marks like `◍`, so the same build drew
    // three different things on three phones — and boxes on one of them.
    const onTab = vi.fn();
    const { container } = shell();
    const menu = screen.getByRole("navigation", { name: "Разделы" });
    expect(within(menu).getByRole("button", { name: "Смена" })).toBeDefined();
    expect(within(menu).getByRole("button", { name: "Моя бронь" })).toBeDefined();
    expect(within(menu).getByRole("button", { name: "Настройки" })).toBeDefined();
    expect(container.textContent).not.toMatch(/[◍▤⚙]/);

    const current = within(menu).getByRole("button", { name: "Смена" });
    expect(current.getAttribute("aria-current")).toBe("true");
    const indicator = current.querySelector("span[aria-hidden]") as HTMLElement;
    expect(indicator.style.width).toBe("18px");
    expect(indicator.style.height).toBe("2px");
    expect(indicator.style.background).toBe("var(--btn)");

    cleanup();
    shell({ onTab });
    await userEvent.click(screen.getByRole("button", { name: "Настройки" }));
    expect(onTab).toHaveBeenCalledWith("settings");
  });

  it("does not invent a tab bar for a guest", () => {
    render(
      <AppShell staff={false} tab="client" onTab={noop} insets={ZERO_INSETS}>
        гость
      </AppShell>,
    );
    expect(screen.queryByRole("navigation", { name: "Разделы" })).toBeNull();
  });
});

describe("press feedback", () => {
  it("gives every interactive element a pressed state, since the platform's own is off", () => {
    render(
      <>
        <Pressable onClick={noop}>кнопка</Pressable>
        <Pressable onClick={noop} tone="card">
          карточка
        </Pressable>
      </>,
    );
    expect(screen.getByText("кнопка").className).toBe("pressable");
    expect(screen.getByText("карточка").className).toBe("pressable-card");
  });

  it("makes everything a finger can hit at least forty-four pixels tall", () => {
    render(<Pressable onClick={noop}>кнопка</Pressable>);
    expect(Number.parseInt(screen.getByText("кнопка").style.minHeight, 10)).toBeGreaterThanOrEqual(
      44,
    );
  });
});

describe("the toast", () => {
  it("sits in a slot the layout owns, above whichever bars the screen has", () => {
    // The old one was pinned 78px from the bottom — the height of the staff tab bar, and wrong on
    // every screen that did not have one.
    const { container } = shell({ toast: <Toast message={{ text: "готово" }} /> });
    const toast = screen.getByRole("status");
    expect(toast.style.position).toBe("absolute");
    expect(toast.style.bottom).toBe("8px");
    const slot = toast.parentElement as HTMLElement;
    expect(slot.style.height).toBe("0px");
    expect(slot.style.position).toBe("relative");
    // And it comes before the bars in the flow, so it is always above them.
    const bars = container.querySelector("nav") as HTMLElement;
    expect(slot.compareDocumentPosition(bars) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("offers the way back when there is one", async () => {
    const run = vi.fn();
    render(<Toast message={{ text: "Стол 7 свободен.", undo: { label: "Вернуть", run } }} />);
    expect(screen.getByText("Стол 7 свободен.")).toBeDefined();
    await userEvent.click(screen.getByText("Вернуть"));
    expect(run).toHaveBeenCalledOnce();
  });

  it("shows nothing at all when there is nothing to say", () => {
    const { container } = render(<Toast message={null} />);
    expect(container.firstChild).toBeNull();
  });
});

describe("the sheet", () => {
  it("is positioned inside the shell rather than against the visual viewport", async () => {
    // `position: fixed` measures against the visual viewport, which in the Telegram webview is not
    // the box the app is drawn in: the panel ends up under Telegram's chrome, or under a keyboard.
    const onClose = vi.fn();
    shell({
      sheet: (
        <Sheet open onClose={onClose} title="Панель">
          содержимое
        </Sheet>
      ),
    });
    const panel = screen.getByRole("dialog");
    expect(panel.style.position).toBe("absolute");
    expect(panel.style.maxHeight).toBe("88%");
    expect(panel.closest("main")).toBeNull();

    const body = screen.getByText("содержимое");
    expect(body.style.overflowY).toBe("auto");
    expect(body.style.overscrollBehavior).toBe("contain");

    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("pins the sheet's own decision below the scroller, where a keyboard cannot push it", () => {
    render(
      <Sheet
        open
        onClose={noop}
        title="Панель"
        footer={<button type="button">Записать</button>}
      >
        содержимое
      </Sheet>,
    );
    const action = screen.getByText("Записать");
    const body = screen.getByText("содержимое");
    expect(body.contains(action)).toBe(false);
    expect((action.parentElement as HTMLElement).style.flex).toBe("0 0 auto");
  });

  it("dims what is underneath and closes when the dim is tapped", async () => {
    const onClose = vi.fn();
    const { container } = render(
      <Sheet open onClose={onClose} title="Панель">
        содержимое
      </Sheet>,
    );
    const scrim = container.querySelector('[aria-hidden="true"]') as HTMLElement;
    expect(scrim.style.position).toBe("absolute");
    await userEvent.click(scrim);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("is not in the document at all when it is closed", () => {
    const { container } = render(
      <Sheet open={false} onClose={noop} title="Панель">
        содержимое
      </Sheet>,
    );
    expect(container.firstChild).toBeNull();
  });
});

describe("a full-screen state", () => {
  it("fills the same box the shell does", () => {
    const { container } = render(
      <InsetFrame insets={{ top: 47, right: 0, bottom: 34, left: 0 }}>загрузка</InsetFrame>,
    );
    const frame = container.firstElementChild as HTMLElement;
    expect(frame.style.height).toBe("var(--tg-vh, 100dvh)");
    expect(frame.style.paddingTop).toBe("47px");
    expect(frame.style.paddingBottom).toBe("34px");
  });
});
