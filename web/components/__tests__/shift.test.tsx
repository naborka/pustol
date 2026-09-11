/**
 * The shift: the list that runs the evening, the timeline, and the receipt.
 *
 * What these check is the thing the whole rebuild turns on — that `Сейчас` answers *who is next*,
 * *who is late* and *what can I seat right now* on its own, without sending anybody to another
 * view; and that a party with no table is impossible to seat by accident.
 */

import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NowPane, Pulse, ShiftActions, ShiftScreen, TablesPane, TotalsPane } from "../AdminShift";
import { noop, shift, shiftBooking, shiftTable } from "./fixtures";

afterEach(cleanup);

function actions(overrides: Partial<Parameters<typeof NowPane>[0]["actions"]> = {}) {
  return {
    onOpen: vi.fn(),
    onSeat: vi.fn(),
    onLeft: vi.fn(),
    onFindTable: vi.fn(),
    ...overrides,
  };
}

function now(
  view = shift(),
  handlers = actions(),
  search = "",
  onSearch: (value: string) => void = noop,
) {
  render(
    <NowPane
      shift={view}
      graceMinutes={15}
      search={search}
      onSearch={onSearch}
      actions={handlers}
    />,
  );
  return handlers;
}

describe("the pulse", () => {
  it("says the time, the room and what still fits, on two lines", () => {
    render(<Pulse shift={shift({ bookings: [shiftBooking({ status: "arrived" })] })} />);
    expect(screen.getByText("21:20 · 2 гостя за столами")).toBeDefined();
    expect(screen.getByText("2 стола свободно")).toBeDefined();
    expect(screen.getByText("Сейчас можно посадить компанию до 8 гостей")).toBeDefined();
  });

  it("says plainly when nothing fits", () => {
    render(<Pulse shift={shift({ largest_party_seatable_now: null })} />);
    expect(
      screen.getByText("Посадить сейчас некуда: все подходящие столы заняты"),
    ).toBeDefined();
  });

  it("leaves out the free count on an evening that is not tonight, rather than inventing one", () => {
    // The server sends no free count on any evening but the one running, because "free now" has no
    // meaning on next Tuesday.
    const { container } = render(
      <Pulse
        shift={shift({
          now_minutes: null,
          largest_party_seatable_now: null,
          stats: { bookings: 1, guests: 2, free_now: null },
        })}
      />,
    );
    expect(container.textContent).not.toMatch(/свободно/);
    expect(container.textContent).not.toMatch(/—/);
    expect(screen.getByText("1 бронь · 2 гостя")).toBeDefined();
  });
});

describe("the list that runs the evening", () => {
  it("reads the groups in the order the evening is worked", () => {
    now(
      shift({
        bookings: [
          shiftBooking({ id: "a", guest_name: "Аня", start_minutes: 1_300 }),
          shiftBooking({ id: "b", guest_name: "Борис", status: "arrived", table_id: "t2", table_number: 8 }),
          shiftBooking({
            id: "c",
            guest_name: "Глеб",
            table_id: null,
            table_number: null,
            table_zone: null,
          }),
          shiftBooking({
            id: "d",
            guest_name: "Дина",
            status: "left",
            released_minutes: 1_270,
            table_id: "t3",
            table_number: 10,
          }),
        ],
      }),
    );
    const headings = screen
      .getAllByText(/^(Без стола|Ждём|За столом|Закрыто) · \d$/)
      .map((node) => node.textContent);
    expect(headings).toEqual(["Без стола · 1", "Ждём · 1", "За столом · 1", "Закрыто · 1"]);
  });

  it("describes a late party in words, not by colour alone", () => {
    now(
      shift({
        now_minutes: 1_300,
        bookings: [shiftBooking({ start_minutes: 1_260 })],
      }),
    );
    expect(screen.getByText("Опаздывает 25 мин")).toBeDefined();
  });

  it("seats somebody in one tap, without opening a sheet", async () => {
    const handlers = now();
    await userEvent.click(screen.getByText("Посадить"));
    expect(handlers.onSeat).toHaveBeenCalledOnce();
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });

  it("offers «Ушли» for a party at the table, and nothing for one whose evening is over", () => {
    now(
      shift({
        bookings: [
          shiftBooking({ id: "a", status: "arrived" }),
          shiftBooking({
            id: "b",
            guest_name: "Дина",
            status: "left",
            released_minutes: 1_270,
            table_id: "t2",
            table_number: 8,
          }),
        ],
      }),
    );
    expect(screen.getAllByText("Ушли")).toHaveLength(1);
    expect(screen.queryByText("Посадить")).toBeNull();
    expect(screen.getByText("Ушли в 21:10 · стол свободен")).toBeDefined();
  });

  it("finds a guest by three letters of their name", async () => {
    const view = shift({
      bookings: [
        shiftBooking({ id: "a", guest_name: "Саша" }),
        shiftBooking({ id: "b", guest_name: "Тимур", table_id: "t2", table_number: 8 }),
      ],
    });
    const { rerender } = render(
      <NowPane
        shift={view}
        graceMinutes={15}
        search=""
        onSearch={noop}
        actions={actions()}
      />,
    );
    expect(screen.getByText("Саша")).toBeDefined();

    rerender(
      <NowPane
        shift={view}
        graceMinutes={15}
        search="Тим"
        onSearch={noop}
        actions={actions()}
      />,
    );
    expect(screen.getByText("Тимур")).toBeDefined();
    expect(screen.queryByText("Саша")).toBeNull();
  });

  it("finds a party by the number of the table they are at", () => {
    render(
      <NowPane
        shift={shift({
          bookings: [
            shiftBooking({ id: "a", guest_name: "Саша", table_number: 7 }),
            shiftBooking({ id: "b", guest_name: "Тимур", table_id: "t2", table_number: 8 }),
          ],
        })}
        graceMinutes={15}
        search="8"
        onSearch={noop}
        actions={actions()}
      />,
    );
    expect(screen.getByText("Тимур")).toBeDefined();
    expect(screen.queryByText("Саша")).toBeNull();
  });

  it("says so when a search finds nobody, and when the evening is empty", () => {
    const { unmount } = render(
      <NowPane
        shift={shift()}
        graceMinutes={15}
        search="ктотоещё"
        onSearch={noop}
        actions={actions()}
      />,
    );
    expect(screen.getByText("Никого не нашли")).toBeDefined();
    unmount();

    now(shift({ bookings: [] }));
    expect(screen.getByText("На этот вечер броней нет")).toBeDefined();
  });
});

describe("a booking with no table", () => {
  const stranded = shift({
    bookings: [
      shiftBooking({
        guest_name: "Глеб",
        table_id: null,
        table_number: null,
        table_zone: null,
      }),
    ],
  });

  it("comes first, says why, and says the guest has not been told", () => {
    now(stranded);
    expect(screen.getByText("Без стола · 1")).toBeDefined();
    expect(
      screen.getByText(
        "Стол под ними закрыли или уменьшили. Гостям об этом не сообщают — их бронь всё ещё выглядит подтверждённой.",
      ),
    ).toBeDefined();
  });

  it("is never offered «Посадить», because seating them is not a state that exists", async () => {
    const handlers = now(stranded);
    expect(screen.queryByText("Посадить")).toBeNull();
    const rowAction = screen.getAllByText("Найти стол");
    expect(rowAction.length).toBeGreaterThan(0);
    await userEvent.click(rowAction[rowAction.length - 1] as HTMLElement);
    expect(handlers.onFindTable).toHaveBeenCalled();
    expect(handlers.onSeat).not.toHaveBeenCalled();
  });

  it("says «Без стола» in the warning colour, never in the grey a table number uses", () => {
    now(stranded);
    const place = screen.getByText("Без стола");
    expect(place.style.color).toBe("var(--warn)");
    expect(place.style.fontWeight).toBe("600");
    expect(screen.getByText("Посадить некуда")).toBeDefined();
  });

  it("says something different again when they are sitting at a table nobody owns", () => {
    now(
      shift({
        bookings: [
          shiftBooking({
            status: "arrived",
            table_id: null,
            table_number: null,
            table_zone: null,
          }),
        ],
      }),
    );
    expect(screen.getByText("Сидят, но стол за ними не закреплён")).toBeDefined();
  });
});

describe("the timeline", () => {
  /**
   * The label column and the row grid are laid out independently, so they must carry the same box
   * model. jsdom does no layout, which is exactly why this asserts the cause rather than the
   * symptom: a `<button>` given `border-box` by the user agent and a `<div>` left to the reset is
   * how a 1px border becomes a quarter-row of drift by table twelve.
   */
  it("gives the labels and the rows the same box, explicitly", () => {
    const view = shift();
    const { container } = render(
      <TablesPane shift={view} graceMinutes={15} onOpenBooking={noop} onOpenTable={noop} />,
    );

    const labels = view.tables.map((table) =>
      screen.getByRole("button", { name: `Стол ${table.number}` }),
    );
    const rows = view.tables.map((table) => {
      const row = container.querySelector(`[data-table-row="${table.id}"]`);
      expect(row, `row for table ${table.number}`).not.toBeNull();
      return row as HTMLElement;
    });

    for (const [index, label] of labels.entries()) {
      const row = rows[index] as HTMLElement;
      expect(label.style.boxSizing).toBe("border-box");
      expect(row.style.boxSizing).toBe("border-box");
      expect(label.style.height).toBe(row.style.height);
      expect(label.style.borderBottom).toBe(row.style.borderBottom);
    }
  });

  it("gives a party with no table a row of its own, above the tables", () => {
    render(
      <TablesPane
        shift={shift({
          bookings: [
            shiftBooking({ table_id: null, table_number: null, table_zone: null, guest_name: "Глеб" }),
          ],
        })}
        graceMinutes={15}
        onOpenBooking={noop}
        onOpenTable={noop}
      />,
    );
    expect(screen.getByText("без стола")).toBeDefined();
    expect(screen.getByRole("button", { name: "Глеб, 21:00" })).toBeDefined();
  });

  it("draws a block only as wide as the table is actually held", () => {
    const gone = shiftBooking({ status: "left", released_minutes: 1_320 });
    const { container } = render(
      <TablesPane
        shift={shift({ bookings: [gone] })}
        graceMinutes={15}
        onOpenBooking={noop}
        onOpenTable={noop}
      />,
    );
    const block = screen.getByRole("button", { name: "Саша, 21:00" });
    // An hour held at sixty-eight pixels an hour, less the three-pixel gutter.
    expect(block.style.width).toBe("65px");
    expect(container.textContent).not.toMatch(/undefined|NaN/);
  });

  it("opens the table rather than closing it when its number is tapped", async () => {
    const onOpenTable = vi.fn();
    render(
      <TablesPane shift={shift()} graceMinutes={15} onOpenBooking={noop} onOpenTable={onOpenTable} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Стол 7" }));
    expect(onOpenTable).toHaveBeenCalledOnce();
  });

  it("no longer needs a caption to explain what a tap does", () => {
    const { container } = render(
      <TablesPane shift={shift()} graceMinutes={15} onOpenBooking={noop} onOpenTable={noop} />,
    );
    expect(container.textContent).not.toMatch(/Номер стола — закрыть стол/);
  });
});

describe("the shift's own receipt", () => {
  it("shows six numbers, a bar per hour, and the peak", () => {
    render(
      <TotalsPane
        shift={shift({
          hours: { open_minutes: 1_080, close_minutes: 1_320, closed: false },
          bookings: [
            shiftBooking({ id: "a", status: "arrived" }),
            shiftBooking({ id: "b", status: "no_show", released_minutes: 1_275, table_id: "t2", table_number: 8 }),
            shiftBooking({
              id: "c",
              source: "walk",
              status: "arrived",
              table_id: "t3",
              table_number: 10,
              party_size: 4,
            }),
          ],
        })}
      />,
    );
    for (const label of ["броней", "гостей", "пришли", "не пришли", "без брони", "занятость столов"]) {
      expect(screen.getByText(label), label).toBeDefined();
    }
    expect(screen.getByText(/^Пик в /)).toBeDefined();
  });

  it("says plainly when the day is shut rather than drawing an empty chart", () => {
    render(
      <TotalsPane
        shift={shift({ hours: { open_minutes: 1_080, close_minutes: 1_560, closed: true } })}
      />,
    );
    expect(screen.getByText("В этот день бар закрыт.")).toBeDefined();
  });
});

describe("the action bar", () => {
  it("offers to seat somebody now only on tonight's shift", async () => {
    const onWalkIn = vi.fn();
    const onManual = vi.fn();
    const { rerender } = render(
      <ShiftActions isToday onWalkIn={onWalkIn} onManual={onManual} />,
    );
    expect(screen.getByText("Посадить сейчас")).toBeDefined();
    expect(screen.getByText("Записать")).toBeDefined();

    rerender(<ShiftActions isToday={false} onWalkIn={onWalkIn} onManual={onManual} />);
    expect(screen.queryByText("Посадить сейчас")).toBeNull();
    await userEvent.click(screen.getByText("Записать гостя"));
    expect(onManual).toHaveBeenCalledOnce();
    expect(onWalkIn).not.toHaveBeenCalled();
  });
});

describe("the shift screen", () => {
  it("opens on «Сейчас» and moves between the three views", async () => {
    render(
      <ShiftScreen
        shift={shift()}
        today="2026-09-11"
        graceMinutes={15}
        pane="now"
        onPane={noop}
        onServiceDate={noop}
        onOpenDays={noop}
        onOpenTable={noop}
        actions={actions()}
      />,
    );
    expect(screen.getByRole("tab", { name: "Сейчас" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByPlaceholderText("Поиск: имя или номер стола")).toBeDefined();
    expect(screen.getByRole("tab", { name: "Столы" })).toBeDefined();
    expect(screen.getByRole("tab", { name: "Итоги" })).toBeDefined();
  });

  it("steps the day, and opens a sheet of them when the middle is tapped", async () => {
    const onServiceDate = vi.fn();
    const onOpenDays = vi.fn();
    render(
      <ShiftScreen
        shift={shift()}
        today="2026-09-11"
        graceMinutes={15}
        pane="now"
        onPane={noop}
        onServiceDate={onServiceDate}
        onOpenDays={onOpenDays}
        onOpenTable={noop}
        actions={actions()}
      />,
    );
    const header = screen.getByRole("group", { name: "День смены" });
    expect(within(header).getByText("Сегодня")).toBeDefined();
    expect(within(header).getByText("пт, 11 сен")).toBeDefined();

    for (const control of ["Предыдущий день", "Следующий день", "Выбрать день"]) {
      const hit = screen.getByRole("button", { name: control });
      expect(Number.parseInt(hit.style.minHeight, 10), control).toBeGreaterThanOrEqual(48);
    }

    await userEvent.click(screen.getByRole("button", { name: "Предыдущий день" }));
    expect(onServiceDate).toHaveBeenCalledWith("2026-09-10");
    await userEvent.click(screen.getByRole("button", { name: "Следующий день" }));
    expect(onServiceDate).toHaveBeenCalledWith("2026-09-12");
    await userEvent.click(screen.getByRole("button", { name: "Выбрать день" }));
    expect(onOpenDays).toHaveBeenCalledOnce();
  });

  it("says the bar is shut rather than drawing an empty evening", () => {
    render(
      <ShiftScreen
        shift={shift({ hours: { open_minutes: 1_080, close_minutes: 1_560, closed: true } })}
        today="2026-09-11"
        graceMinutes={15}
        pane="now"
        onPane={noop}
        onServiceDate={noop}
        onOpenDays={noop}
        onOpenTable={noop}
        actions={actions()}
      />,
    );
    expect(screen.getByText("Выходной")).toBeDefined();
    expect(screen.queryByRole("tab", { name: "Столы" })).toBeNull();
  });
});

describe("nothing on a staff screen falls back to a number it does not have", () => {
  it("never prints undefined, null or NaN", () => {
    const { container } = render(
      <ShiftScreen
        shift={shift({
          bookings: [
            shiftBooking({ table_id: null, table_number: null, table_zone: null }),
            shiftBooking({ id: "b", guest_name: "Без брони", source: "walk", status: "arrived" }),
          ],
        })}
        today="2026-09-11"
        graceMinutes={15}
        pane="now"
        onPane={noop}
        onServiceDate={noop}
        onOpenDays={noop}
        onOpenTable={noop}
        actions={actions()}
      />,
    );
    expect(container.textContent).not.toMatch(/undefined|null|NaN/);
    expect(container.textContent).not.toMatch(/стол null|стол undefined/);
  });
});

describe("a table that has been shut", () => {
  it("is drawn in the destructive colour with its reason on the row", () => {
    render(
      <TablesPane
        shift={shift({
          tables: [shiftTable({ blocked_because: "Дождь" })],
          bookings: [],
        })}
        graceMinutes={15}
        onOpenBooking={noop}
        onOpenTable={noop}
      />,
    );
    expect(screen.getByText("Дождь")).toBeDefined();
    expect(screen.getByText("7").style.color).toBe("var(--dest)");
  });
});
