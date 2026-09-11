/**
 * The sheets, and the rule that decides which of them exists.
 *
 * Reversible things happen on one tap and offer a way back. Irreversible things — a message a
 * guest receives, a cancellation they are told about — are chosen from the bar's own list and get
 * no undo, because the choosing was the protection.
 */

import { useState } from "react";

import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ShiftView } from "@/lib/api";

import {
  BookingSheet,
  ChoiceSheet,
  DaySheet,
  ManualBookingSheet,
  MoveBookingSheet,
  TableSheet,
  WalkInSheet,
} from "../Sheets";
import { Sheet, TextField } from "../ui";
import { availability, noop, shift, shiftBooking, shiftTable } from "./fixtures";

afterEach(cleanup);

function bookingSheet(booking = shiftBooking(), handlers: Record<string, () => void> = {}) {
  const props = {
    onAttendance: vi.fn(),
    onNote: vi.fn(),
    onOpenTemplates: vi.fn(),
    onOpenCancel: vi.fn(),
    onOpenMove: vi.fn(),
    onFindTable: vi.fn(),
    ...handlers,
  };
  render(
    <BookingSheet
      open
      booking={booking}
      nowMinutes={1_280}
      graceMinutes={15}
      onClose={noop}
      onAttendance={props.onAttendance as never}
      onNote={props.onNote as never}
      onOpenTemplates={props.onOpenTemplates}
      onOpenCancel={props.onOpenCancel}
      onOpenMove={props.onOpenMove}
      onFindTable={props.onFindTable}
    />,
  );
  return props;
}

describe("one booking", () => {
  it("states when, where and how it arrived", () => {
    bookingSheet();
    const sheet = screen.getByRole("dialog");
    expect(within(sheet).getByText("21:00 — 23:00 · 2 гостя")).toBeDefined();
    expect(within(sheet).getByText("стол 7 · Стойка")).toBeDefined();
    expect(within(sheet).getByText("Из приложения")).toBeDefined();
  });

  it("calls a party with no booking what they are", () => {
    bookingSheet(shiftBooking({ guest_name: "Без брони", source: "walk", status: "arrived" }));
    expect(screen.getAllByText("Гости без брони").length).toBeGreaterThan(0);
    expect(screen.getByText("Гости с улицы")).toBeDefined();
  });

  it("asks the one question a shift asks, and offers «Ушли» only when there is somebody to leave", async () => {
    const props = bookingSheet(shiftBooking({ status: "arrived" }));
    for (const label of ["Ждём", "За столом", "Не пришли"]) {
      expect(screen.getByText(label), label).toBeDefined();
    }
    await userEvent.click(screen.getByText("Ушли"));
    expect(props.onAttendance).toHaveBeenCalledWith("left");

    cleanup();
    bookingSheet(shiftBooking({ status: "confirmed" }));
    expect(screen.queryByText("Ушли")).toBeNull();
  });

  it("saves a note the moment a chip is tapped, and takes it off again", async () => {
    const props = bookingSheet();
    await userEvent.click(screen.getByText("День рождения"));
    expect(props.onNote).toHaveBeenCalledWith("День рождения");

    cleanup();
    const already = bookingSheet(shiftBooking({ note: "У окна" }));
    await userEvent.click(screen.getByText("У окна"));
    expect(already.onNote).toHaveBeenCalledWith(null);
  });

  it("says why the guest cannot be written to, rather than offering and failing", () => {
    bookingSheet(shiftBooking({ reachable_by_bot: false }));
    const button = screen.getByText("Гость без Telegram — написать нельзя");
    expect(button.closest("button")?.disabled).toBe(true);
    expect(screen.queryByText("Написать гостю")).toBeNull();
  });

  it("repeats the warning and the action for a party with no table", async () => {
    const props = bookingSheet(
      shiftBooking({ table_id: null, table_number: null, table_zone: null }),
    );
    expect(
      screen.getByText(
        "Стол под ними закрыли или уменьшили. Гостям об этом не сообщают — их бронь всё ещё выглядит подтверждённой.",
      ),
    ).toBeDefined();
    await userEvent.click(screen.getByText("Найти стол"));
    expect(props.onFindTable).toHaveBeenCalledOnce();
  });

  it("will not seat a party the room has no table for, from here either", async () => {
    // The row already refuses. The sheet used to offer it anyway, and the toast then read
    // "Глеб за столом null".
    const props = bookingSheet(
      shiftBooking({ guest_name: "Глеб", table_id: null, table_number: null, table_zone: null }),
    );
    await userEvent.click(screen.getByText("За столом"));
    expect(props.onAttendance).not.toHaveBeenCalled();

    await userEvent.click(screen.getByText("Не пришли"));
    expect(props.onAttendance).toHaveBeenCalledWith("no_show");
  });
});

describe("what cannot be undone", () => {
  it("makes a message a choice from the bar's own list, and says it cannot be taken back", () => {
    render(
      <ChoiceSheet
        open
        title="Написать гостю"
        hint="Уйдёт от бота в чат гостя. Саша получит его сразу — отменить отправку нельзя."
        choices={["Ваш стол готов"]}
        onClose={noop}
        onChoose={noop}
      />,
    );
    expect(screen.getByText(/отменить отправку нельзя/)).toBeDefined();
    expect(screen.getByText("Ваш стол готов")).toBeDefined();
  });

  it("says so when the bar has left the list empty, rather than showing an empty sheet", () => {
    render(
      <ChoiceSheet
        open
        title="Причина отмены"
        hint="…"
        choices={[]}
        onClose={noop}
        onChoose={noop}
      />,
    );
    expect(screen.getByText("Список пуст — заполните его в настройках.")).toBeDefined();
  });
});

describe("a table", () => {
  it("opens on what it is, not on closing it", async () => {
    const onBlock = vi.fn();
    render(
      <TableSheet
        open
        table={shiftTable()}
        shift={shift()}
        onClose={noop}
        onBlock={onBlock}
        onUnblock={noop}
      />,
    );
    expect(screen.getByText("Мест")).toBeDefined();
    expect(screen.getByText("Стойка")).toBeDefined();
    expect(screen.getByText("Саша")).toBeDefined();

    // Closing it takes a second tap and a reason.
    await userEvent.click(screen.getByText("Закрыть стол на вечер"));
    expect(onBlock).not.toHaveBeenCalled();
    await userEvent.click(screen.getByText("Дождь"));
    expect(onBlock).toHaveBeenCalledWith(["t1"], "Дождь");
  });

  it("offers to open a table that is shut", async () => {
    const onUnblock = vi.fn();
    render(
      <TableSheet
        open
        table={shiftTable({ blocked_because: "Дождь" })}
        shift={shift()}
        onClose={noop}
        onBlock={noop}
        onUnblock={onUnblock}
      />,
    );
    await userEvent.click(screen.getByText("Открыть стол снова"));
    expect(onUnblock).toHaveBeenCalledWith(["t1"]);
  });
});

describe("the day sheet", () => {
  it("lists the evenings with what is on, and says where the guest's horizon ends", async () => {
    const onChoose = vi.fn();
    render(
      <DaySheet
        open
        days={shift().days}
        today="2026-09-11"
        serviceDate="2026-09-11"
        guestHorizonDays={4}
        onClose={noop}
        onChoose={onChoose}
      />,
    );
    expect(screen.getByText("Гости бронируют на 4 дня вперёд. Персонал — на любой из этих.")).toBeDefined();
    expect(screen.getByText("1 бронь")).toBeDefined();
    expect(screen.getByText("пусто")).toBeDefined();
    expect(screen.getByText("выходной")).toBeDefined();

    await userEvent.click(screen.getByText("Завтра · сб, 12 сен"));
    expect(onChoose).toHaveBeenCalledWith("2026-09-12");
  });
});

describe("a party at the door", () => {
  function walkInSheet(
    view: ShiftView,
    partySize: number,
    chosenTableId: string | null = null,
    handlers: { onChooseTable?: (tableId: string) => void; onSeat?: (tableId: string) => void } = {},
  ) {
    return render(
      <WalkInSheet
        open
        shift={view}
        maxParty={6}
        turnMinutes={120}
        partySize={partySize}
        chosenTableId={chosenTableId}
        onClose={noop}
        onPartySize={noop}
        onChooseTable={handlers.onChooseTable ?? noop}
        onSeat={handlers.onSeat ?? noop}
      />,
    );
  }

  it("lists every table staff could put them at, the room's own pick first", () => {
    // Table 7 seats two and is busy. Eight and ten are free, and both are offered: which of them
    // a couple gets is a decision about the room, and only the person in it can make it.
    walkInSheet(shift(), 2);
    const sheet = screen.getByRole("dialog");
    expect(within(sheet).getByText("Стол 8 · Зал")).toBeDefined();
    expect(within(sheet).getByText("Стол 10 · Зал")).toBeDefined();
    expect(within(sheet).queryByText("Стол 7 · Стойка")).toBeNull();
    expect(
      within(sheet).getByText(
        "Сверху — самый маленький подходящий: большие столы остаются для больших компаний. Стол будет занят до 23:20.",
      ),
    ).toBeDefined();
    expect(screen.getByText("Посадить за стол 8")).toBeDefined();
  });

  it("seats them at the table staff chose rather than at the one it suggested", async () => {
    const onChooseTable = vi.fn();
    walkInSheet(shift(), 2, null, { onChooseTable });
    await userEvent.click(screen.getByText("Стол 10 · Зал"));
    expect(onChooseTable).toHaveBeenCalledWith("t3");

    cleanup();
    const onSeat = vi.fn();
    walkInSheet(shift(), 2, "t3", { onSeat });
    await userEvent.click(screen.getByText("Посадить за стол 10"));
    expect(onSeat).toHaveBeenCalledWith("t3");
  });

  it("falls back to the room's own pick when the chosen table stops fitting", async () => {
    // Staff picked the four-top, then said the party is six. The button names the table it will
    // actually use, and asks for that one: the choice is read off the list in one place, so the
    // label and the request cannot come apart.
    const onSeat = vi.fn();
    walkInSheet(shift(), 6, "t2", { onSeat });
    await userEvent.click(screen.getByText("Посадить за стол 10"));
    expect(onSeat).toHaveBeenCalledWith("t3");
  });

  it("shows a free table the party is too large for, and says why it is not on offer", () => {
    walkInSheet(shift(), 6, null);
    const tooSmall = screen.getByText("Стол 8 · Зал").closest("button");
    expect(within(tooSmall as HTMLElement).getByText("4 места · мало мест")).toBeDefined();
    expect((tooSmall as HTMLButtonElement).disabled).toBe(true);
  });

  it("agrees with the shift's own line about who fits", () => {
    // The pulse says "до 8 гостей" and the sheet then seats a party of eight. Two answers from one
    // rule, which is the only way they can never contradict each other.
    const view = shift();
    expect(view.largest_party_seatable_now).toBe(8);
    walkInSheet(view, 8);
    expect(screen.getByText("Посадить за стол 10")).toBeDefined();

    cleanup();
    walkInSheet(view, 9);
    expect(screen.getByText("Посадить некуда").closest("button")?.disabled).toBe(true);
  });

  it("refuses plainly when every table is taken, with the button inert", () => {
    const full = shift({
      largest_party_seatable_now: null,
      bookings: [
        shiftBooking({ id: "a", table_id: "t1", table_number: 7 }),
        shiftBooking({ id: "b", table_id: "t2", table_number: 8 }),
        shiftBooking({ id: "c", table_id: "t3", table_number: 10 }),
      ],
    });
    walkInSheet(full, 2);
    expect(screen.getByText("Свободного стола нет")).toBeDefined();
    expect(
      screen.getByText("Все подходящие столы заняты. Освободите стол или предложите подождать."),
    ).toBeDefined();
    expect(screen.getByText("Посадить некуда").closest("button")?.disabled).toBe(true);
  });

  it("says the free tables are too small rather than that there are none", () => {
    const onlySmall = shift({
      largest_party_seatable_now: 2,
      bookings: [
        shiftBooking({ id: "b", table_id: "t2", table_number: 8 }),
        shiftBooking({ id: "c", table_id: "t3", table_number: 10 }),
      ],
    });
    walkInSheet(onlySmall, 6);
    expect(screen.getByText("Свободного стола нет")).toBeDefined();
    expect(
      screen.getByText(
        "Свободные столы малы для такой компании. Освободите стол побольше или предложите подождать.",
      ),
    ).toBeDefined();
    expect(screen.getByText("Стол 7 · Стойка")).toBeDefined();
  });

  it("does not exist at all on an evening that is not tonight", () => {
    const { container } = walkInSheet(shift({ now_minutes: null }), 2);
    expect(container.firstChild).toBeNull();
  });

  it("offers a table freed mid-step, which a slot grid would have hidden", () => {
    // Somebody leaves at 21:20 — not on the half-hour. Flooring the walk-in onto the grid would
    // look for 21:30 and, in a room with nothing else free, find nothing.
    const view = shift({
      now_minutes: 1_280,
      tables: [shiftTable()],
      bookings: [shiftBooking({ status: "left", released_minutes: 1_280 })],
    });
    walkInSheet(view, 2);
    expect(screen.getByText("Посадить за стол 7")).toBeDefined();
  });
});

describe("typing in a sheet", () => {
  /** A parent that hands a new `onClose` on every render, which is what a page component does. */
  function NameSheet() {
    const [name, setName] = useState("");
    return (
      <Sheet open onClose={() => {}} title="Записать гостя">
        <TextField value={name} onChange={setName} placeholder="Имя" />
      </Sheet>
    );
  }

  it("leaves the focus in the field instead of pulling it back to the panel", async () => {
    // The bug: the sheet took focus whenever its effect re-ran, and the effect depended on a
    // callback the page rebuilds every render. Every keystroke re-rendered, stole the focus, and
    // shut the phone keyboard — so a name could not be typed at all.
    render(<NameSheet />);
    const field = screen.getByPlaceholderText("Имя");
    await userEvent.type(field, "Глеб");
    expect(document.activeElement).toBe(field);
    expect((field as HTMLInputElement).value).toBe("Глеб");
  });
});

describe("moving a booking", () => {
  const later = shiftBooking({ start_minutes: 1_320, end_minutes: 1_440 });

  function moveSheet(
    booking = later,
    chosen: { minutes?: number | null; table?: string | null } = {},
    handlers: {
      onChooseTable?: (tableId: string) => void;
      onMove?: (minutes: number, tableId: string) => void;
    } = {},
  ) {
    return render(
      <MoveBookingSheet
        open
        booking={booking}
        shift={shift()}
        turnMinutes={120}
        availability={availability()}
        chosenMinutes={chosen.minutes ?? null}
        chosenTableId={chosen.table ?? null}
        failedToLoad={false}
        onClose={noop}
        onPick={noop}
        onTakenSlot={noop}
        onChooseTable={handlers.onChooseTable ?? noop}
        onRetry={noop}
        onMove={handlers.onMove ?? noop}
      />,
    );
  }

  it("starts on where the booking already is, with nothing to do", () => {
    moveSheet();
    expect(screen.getByText("Стол 7 · Стойка")).toBeDefined();
    expect(screen.getByText("Ничего не меняли").closest("button")?.disabled).toBe(true);
  });

  it("does not let the booking block its own time or its own table", () => {
    // Table 7 is held by this very booking. Moving it half an hour must not mean giving the table
    // up first and hoping somebody else has not taken it.
    moveSheet(later, { minutes: 1_350 });
    expect(screen.getByText("Перенести на 22:30, стол 7")).toBeDefined();
  });

  it("sends the table staff tapped, at the time they left alone", async () => {
    const onChooseTable = vi.fn();
    moveSheet(later, {}, { onChooseTable });
    await userEvent.click(screen.getByText("Стол 8 · Зал"));
    expect(onChooseTable).toHaveBeenCalledWith("t2");

    cleanup();
    const onMove = vi.fn();
    moveSheet(later, { table: "t2" }, { onMove });
    await userEvent.click(screen.getByText("Пересадить за стол 8"));
    expect(onMove).toHaveBeenCalledWith(1_320, "t2");
  });

  it("keeps the time of a booking that has started, and still offers the tables", () => {
    // 21:20, and they sat down at 21:00. The window is history; where they sit is not.
    moveSheet(shiftBooking({ status: "arrived" }), { table: "t2" });
    expect(
      screen.getByText("Бронь уже началась — время не меняем. Стол можно поменять в любой момент."),
    ).toBeDefined();
    expect(screen.queryByText("Время")).toBeNull();
    expect(screen.getByText("Пересадить за стол 8")).toBeDefined();
  });
});

describe("writing a booking down", () => {
  function manualSheet(
    chosen: { minutes?: number | null; table?: string | null } = {},
    onCreate: (tableId: string) => void = noop,
  ) {
    return render(
      <ManualBookingSheet
        open
        shift={shift()}
        maxParty={6}
        turnMinutes={120}
        availability={availability()}
        partySize={2}
        chosenMinutes={chosen.minutes ?? null}
        chosenTableId={chosen.table ?? null}
        guestName="Глеб"
        failedToLoad={false}
        onClose={noop}
        onPartySize={noop}
        onPick={noop}
        onTakenSlot={noop}
        onChooseTable={noop}
        onGuestName={noop}
        onRetry={noop}
        onCreate={onCreate}
      />,
    );
  }

  it("offers no tables until there is a time for them to be free at", () => {
    manualSheet();
    expect(screen.queryByText("Стол 8 · Зал")).toBeNull();
    expect(screen.getByText("Имя и время").closest("button")?.disabled).toBe(true);
  });

  it("offers the tables free at the chosen time, and books the one staff picked", async () => {
    const onCreate = vi.fn();
    manualSheet({ minutes: 1_290, table: "t3" }, onCreate);
    // Table 7 is taken from 21:00 to 23:00, so it is not on offer for a 21:30 sitting.
    expect(screen.queryByText("Стол 7 · Стойка")).toBeNull();
    expect(screen.getByText("Стол 8 · Зал")).toBeDefined();

    await userEvent.click(screen.getByText("Записать на 21:30, стол 10"));
    expect(onCreate).toHaveBeenCalledWith("t3");
  });
});
