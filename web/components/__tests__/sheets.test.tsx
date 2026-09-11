/**
 * The sheets, and the rule that decides which of them exists.
 *
 * Reversible things happen on one tap and offer a way back. Irreversible things — a message a
 * guest receives, a cancellation they are told about — are chosen from the bar's own list and get
 * no undo, because the choosing was the protection.
 */

import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  BookingSheet,
  ChoiceSheet,
  DaySheet,
  TableSheet,
  WalkInSheet,
  chooseWalkInTable,
} from "../Sheets";
import { noop, shift, shiftBooking, shiftTable } from "./fixtures";

afterEach(cleanup);

function bookingSheet(booking = shiftBooking(), handlers: Record<string, () => void> = {}) {
  const props = {
    onAttendance: vi.fn(),
    onNote: vi.fn(),
    onOpenTemplates: vi.fn(),
    onOpenCancel: vi.fn(),
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
  it("names the table it would use before anybody commits", () => {
    render(
      <WalkInSheet
        open
        shift={shift()}
        maxParty={6}
        turnMinutes={120}
        partySize={2}
        onClose={noop}
        onPartySize={noop}
        onSeat={noop}
      />,
    );
    // Table 7 seats two and is busy; table 8 seats four and is the smallest that is free.
    expect(screen.getByText("Стол 8 · Зал")).toBeDefined();
    expect(
      screen.getByText(
        "4 места, занят до 23:20. Самый маленький подходящий — большие остаются для больших компаний.",
      ),
    ).toBeDefined();
    expect(screen.getByText("Посадить за стол 8")).toBeDefined();
  });

  it("keeps the large tables for the large parties", () => {
    expect(chooseWalkInTable(shift({ bookings: [] }), 2, 120)?.number).toBe(7);
    expect(chooseWalkInTable(shift({ bookings: [] }), 3, 120)?.number).toBe(8);
    expect(chooseWalkInTable(shift({ bookings: [] }), 7, 120)?.number).toBe(10);
  });

  it("agrees with the shift's own line about who fits", () => {
    // The pulse says "до 8 гостей" and the sheet then seats a party of eight. Two answers from one
    // rule, which is the only way they can never contradict each other.
    const view = shift();
    expect(view.largest_party_seatable_now).toBe(8);
    expect(chooseWalkInTable(view, 8, 120)?.number).toBe(10);
    expect(chooseWalkInTable(view, 9, 120)).toBeNull();
  });

  it("refuses plainly when nothing fits, with the button inert", () => {
    const full = shift({
      largest_party_seatable_now: null,
      bookings: [
        shiftBooking({ id: "a", table_id: "t1", table_number: 7 }),
        shiftBooking({ id: "b", table_id: "t2", table_number: 8 }),
        shiftBooking({ id: "c", table_id: "t3", table_number: 10 }),
      ],
    });
    render(
      <WalkInSheet
        open
        shift={full}
        maxParty={6}
        turnMinutes={120}
        partySize={2}
        onClose={noop}
        onPartySize={noop}
        onSeat={noop}
      />,
    );
    expect(screen.getByText("Свободного стола нет")).toBeDefined();
    expect(
      screen.getByText("Все подходящие столы заняты. Освободите стол или предложите подождать."),
    ).toBeDefined();
    expect(screen.getByText("Посадить некуда").closest("button")?.disabled).toBe(true);
  });

  it("does not exist at all on an evening that is not tonight", () => {
    const { container } = render(
      <WalkInSheet
        open
        shift={shift({ now_minutes: null })}
        maxParty={6}
        turnMinutes={120}
        partySize={2}
        onClose={noop}
        onPartySize={noop}
        onSeat={noop}
      />,
    );
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
    expect(chooseWalkInTable(view, 2, 120)?.number).toBe(7);
  });
});
