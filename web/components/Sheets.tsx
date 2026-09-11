"use client";

/**
 * The sheets: a booking, a table, an evening, a message, a reason, a party at the door.
 *
 * Each is a decision with consequences, which is why it gets a sheet rather than an inline
 * control: the screen underneath stops accepting taps while somebody chooses.
 *
 * **The reversibility rule.** Anything that can be taken back — seating a party, marking them
 * gone, closing a table — happens on one tap and offers an undo. Anything that cannot — a message
 * to a guest, a cancellation they are told about — is confirmed by choosing from the bar's own
 * list, and gets no undo, because the confirmation *is* the protection.
 */

import { useState } from "react";

import type {
  Availability,
  GuestBooking,
  ShiftBooking,
  ShiftDay,
  ShiftTable,
  ShiftView,
} from "@/lib/api";
import * as fmt from "@/lib/format";
import { holdsDuring } from "@/lib/occupancy";
import { standingOf, statusLabel } from "@/lib/status";
import { openChatWith } from "@/lib/telegram";
import { RADIUS, SPACE, TEXT } from "@/lib/tokens";
import {
  Card,
  CardAction,
  Chip,
  Note,
  PartySizeGrid,
  Pressable,
  SectionLabel,
  Separator,
  Sheet,
  SheetChoice,
  SheetTitle,
  SlotGrid,
  Spinner,
  TextField,
} from "./ui";

/** The notes staff actually write. A fixed list, because a keyboard at the pass is a lost minute. */
export const NOTE_CHIPS = [
  "День рождения",
  "Постоянный гость",
  "Аллергия",
  "У окна",
  "Компания шумная",
] as const;

/** The reasons a table goes out of service. Staff-facing, so they live with the screen. */
export const BLOCK_REASONS = [
  "Сломан / залит",
  "Дождь",
  "Частное мероприятие",
  "Держим для своих",
] as const;

const SOURCE_LABEL: Record<ShiftBooking["source"], string> = {
  app: "Из приложения",
  staff: "Записан персоналом",
  walk: "Гости с улицы",
};

function Fact({ label, value, tone }: { label: string; value: string; tone?: "warn" }) {
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
          color: tone === "warn" ? "var(--warn)" : "var(--txt)",
          textAlign: "right",
        }}
      >
        {value}
      </span>
    </div>
  );
}

// ---- one booking --------------------------------------------------------------------------------

export function BookingSheet({
  open,
  booking,
  nowMinutes,
  graceMinutes,
  onClose,
  onAttendance,
  onNote,
  onOpenTemplates,
  onOpenCancel,
  onFindTable,
}: {
  open: boolean;
  booking: ShiftBooking | null;
  nowMinutes: number | null;
  graceMinutes: number;
  onClose: () => void;
  onAttendance: (attendance: "confirmed" | "arrived" | "no_show" | "left") => void;
  onNote: (note: string | null) => void;
  onOpenTemplates: () => void;
  onOpenCancel: () => void;
  onFindTable: () => void;
}) {
  if (!booking) return null;
  const seated = booking.table_id !== null;
  const standing = standingOf(booking, nowMinutes, graceMinutes);
  const title = booking.source === "walk" ? "Гости без брони" : booking.guest_name;

  return (
    <Sheet open={open} onClose={onClose} title={title}>
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[4] }}>
        <SheetTitle>{title}</SheetTitle>

        <Card gap={SPACE[2] + 2}>
          <Fact
            label="Когда"
            value={`${fmt.time(booking.start_minutes)} — ${fmt.time(booking.end_minutes)} · ${fmt.guests(
              booking.party_size,
            )}`}
          />
          <Separator />
          <Fact
            label="Где"
            value={
              seated
                ? `стол ${booking.table_number}${booking.table_zone ? ` · ${booking.table_zone}` : ""}`
                : "Без стола"
            }
            {...(seated ? {} : { tone: "warn" as const })}
          />
          <Separator />
          <Fact label="Откуда" value={SOURCE_LABEL[booking.source]} />
        </Card>

        {!seated ? (
          <div
            style={{
              padding: SPACE[3] + 2,
              borderRadius: RADIUS.md,
              background: "rgba(234,161,58,.16)",
              display: "flex",
              flexDirection: "column",
              gap: SPACE[2] + 2,
            }}
          >
            <Note tone="warn">
              Стол под ними закрыли или уменьшили. Гостям об этом не сообщают — их бронь всё ещё
              выглядит подтверждённой.
            </Note>
            <CardAction tone="warn" label="Найти стол" onClick={onFindTable} />
          </div>
        ) : null}

        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
          <SectionLabel>Как прошло</SectionLabel>
          {/*
            Three chips for the one question a shift asks about a booking — did they come? Going
            home early is a different question, and it is asked where it is answered: by the row's
            own one-tap `Ушли`, repeated here only while there is somebody at the table to leave.
          */}
          <div style={{ display: "flex", gap: SPACE[2] }}>
            {(
              [
                ["confirmed", "Ждём"],
                ["arrived", "За столом"],
                ["no_show", "Не пришли"],
              ] as const
            ).map(([value, label]) => (
              <Chip
                key={value}
                label={label}
                state={booking.status === value ? "chosen" : "available"}
                style={{ flex: 1 }}
                fontSize={TEXT.md}
                onClick={() => onAttendance(value)}
              />
            ))}
          </div>
          {/* Only where it adds something the chips do not already say. */}
          {standing.kind === "waiting" || standing.kind === "seated" ? null : (
            <Note tone={standing.kind === "late" ? "dest" : "hint"}>{statusLabel(standing)}</Note>
          )}
          {booking.status === "arrived" ? (
            <CardAction label="Ушли" onClick={() => onAttendance("left")} />
          ) : null}
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
          <SectionLabel>Заметка</SectionLabel>
          <div style={{ display: "flex", flexWrap: "wrap", gap: SPACE[2] }}>
            {NOTE_CHIPS.map((text) => {
              const chosen = booking.note === text;
              return (
                <Chip
                  key={text}
                  label={text}
                  fontSize={TEXT.md}
                  state={chosen ? "chosen" : "available"}
                  style={{ flex: "none", padding: `0 ${SPACE[3]}px` }}
                  onClick={() => onNote(chosen ? null : text)}
                />
              );
            })}
          </div>
        </div>

        <CardAction
          label={
            booking.reachable_by_bot
              ? "Написать гостю"
              : "Гость без Telegram — написать нельзя"
          }
          disabled={!booking.reachable_by_bot}
          onClick={onOpenTemplates}
        />

        {booking.guest_username ? (
          <Pressable
            onClick={() => openChatWith(booking.guest_username ?? "")}
            style={{
              justifyContent: "center",
              color: "var(--link)",
              fontSize: TEXT.base,
              fontWeight: 600,
            }}
          >
            Открыть чат @{booking.guest_username}
          </Pressable>
        ) : null}

        <CardAction tone="destructive" label="Отменить бронь" onClick={onOpenCancel} />
      </div>
    </Sheet>
  );
}

// ---- choose one ---------------------------------------------------------------------------------

/**
 * "Pick one of these, and it happens."
 *
 * The message a guest gets and the reason for a cancellation are the same interaction with
 * different words, and the words belong to the caller: this sheet is the shape, not the copy.
 */
export function ChoiceSheet({
  open,
  title,
  hint,
  choices,
  onClose,
  onChoose,
}: {
  open: boolean;
  title: string;
  hint: string;
  choices: string[];
  onClose: () => void;
  onChoose: (choice: string) => void;
}) {
  return (
    <Sheet open={open} onClose={onClose} title={title}>
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[3] }}>
        <SheetTitle>{title}</SheetTitle>
        <Note>{hint}</Note>
        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
          {choices.length === 0 ? (
            <Note tone="warn">Список пуст — заполните его в настройках.</Note>
          ) : (
            choices.map((choice) => (
              <SheetChoice key={choice} text={choice} onClick={() => onChoose(choice)} />
            ))
          )}
        </div>
      </div>
    </Sheet>
  );
}

/** A decision the guest will feel, restated before it is made. */
export function ConfirmSheet({
  open,
  title,
  restated,
  detail,
  confirmLabel,
  onConfirm,
  onClose,
}: {
  open: boolean;
  title: string;
  restated: string;
  detail: string;
  confirmLabel: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Sheet open={open} onClose={onClose} title={title}>
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[3] }}>
        <SheetTitle tone="destructive">{title}</SheetTitle>
        <Card gap={SPACE[1]}>
          <span style={{ fontSize: TEXT.xl, fontWeight: 700, color: "var(--txt)" }}>
            {restated}
          </span>
        </Card>
        <Note>{detail}</Note>
        <CardAction tone="destructive" label={confirmLabel} onClick={onConfirm} />
        <CardAction label="Оставить" onClick={onClose} />
      </div>
    </Sheet>
  );
}

export function ConflictSheet({
  open,
  reasons,
  onClose,
}: {
  open: boolean;
  reasons: string[];
  onClose: () => void;
}) {
  return (
    <Sheet open={open} onClose={onClose} title="Так сохранить нельзя">
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[3] }}>
        <SheetTitle tone="destructive">Так сохранить нельзя</SheetTitle>
        <Note>
          Эти брони уже приняты по действующим правилам. Сначала перенесите или отмените их — тогда
          настройку можно будет сохранить.
        </Note>
        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[1] + 2 }}>
          {reasons.map((reason) => (
            <span
              key={reason}
              style={{
                padding: `${SPACE[3]}px ${SPACE[3]}px`,
                background: "var(--tint)",
                borderRadius: RADIUS.sm,
                fontSize: TEXT.md,
                color: "var(--txt)",
                lineHeight: 1.4,
              }}
            >
              {reason}
            </span>
          ))}
        </div>
        <CardAction label="Понятно" onClick={onClose} />
      </div>
    </Sheet>
  );
}

// ---- the room -----------------------------------------------------------------------------------

/** One table: what it is, what is on it, and how to take it out of service. */
export function TableSheet({
  open,
  table,
  shift,
  onClose,
  onBlock,
  onUnblock,
}: {
  open: boolean;
  table: ShiftTable | null;
  shift: ShiftView | null;
  onClose: () => void;
  onBlock: (tableIds: string[], reason: string) => void;
  onUnblock: (tableIds: string[]) => void;
}) {
  const [closing, setClosing] = useState(false);
  if (!table || !shift) return null;

  const here = shift.bookings.filter((booking) => booking.table_id === table.id);
  const closed = table.blocked_because !== null;
  const title = `Стол ${table.number}`;

  return (
    <Sheet open={open} onClose={onClose} title={title}>
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[4] }}>
        <SheetTitle>{title}</SheetTitle>

        <Card gap={SPACE[2] + 2}>
          <Fact label="Мест" value={String(table.seats)} />
          <Separator />
          <Fact label="Зона" value={table.zone} />
          <Separator />
          <Fact
            label="Сегодня"
            value={closed ? (table.blocked_because ?? "закрыт") : fmt.bookings(here.length)}
            {...(closed ? { tone: "warn" as const } : {})}
          />
        </Card>

        {here.length > 0 ? (
          <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
            <SectionLabel>Кто здесь</SectionLabel>
            {here.map((booking) => (
              <div
                key={booking.id}
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  gap: SPACE[3],
                  padding: `${SPACE[2]}px ${SPACE[3]}px`,
                  background: "var(--sec)",
                  borderRadius: RADIUS.sm,
                  fontSize: TEXT.base,
                  color: "var(--txt)",
                }}
              >
                <span>{booking.guest_name}</span>
                <span style={{ color: "var(--hint)", fontVariantNumeric: "tabular-nums" }}>
                  {fmt.time(booking.start_minutes)} · {booking.party_size}
                </span>
              </div>
            ))}
          </div>
        ) : null}

        {closed ? (
          <CardAction
            tone="primary"
            label="Открыть стол снова"
            onClick={() => onUnblock([table.id])}
          />
        ) : closing ? (
          <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
            <SectionLabel>Почему закрываем</SectionLabel>
            {BLOCK_REASONS.map((reason) => (
              <SheetChoice
                key={reason}
                text={reason}
                onClick={() => onBlock([table.id], reason)}
              />
            ))}
            <CardAction label="Не закрывать" onClick={() => setClosing(false)} />
          </div>
        ) : (
          <CardAction
            tone="destructive"
            label="Закрыть стол на вечер"
            onClick={() => setClosing(true)}
          />
        )}
      </div>
    </Sheet>
  );
}

/** Thirty evenings, with what is on each. Staff are not bound by the guest's horizon. */
export function DaySheet({
  open,
  days,
  today,
  serviceDate,
  guestHorizonDays,
  onClose,
  onChoose,
}: {
  open: boolean;
  days: ShiftDay[];
  today: string;
  serviceDate: string;
  guestHorizonDays: number;
  onClose: () => void;
  onChoose: (date: string) => void;
}) {
  return (
    <Sheet open={open} onClose={onClose} title="Какой вечер">
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[3] }}>
        <SheetTitle>Какой вечер</SheetTitle>
        <Note>
          Гости бронируют на {fmt.daysWord(guestHorizonDays)} вперёд. Персонал — на любой из
          этих.
        </Note>
        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[1] + 2 }}>
          {days.map((day) => {
            const chosen = day.service_date === serviceDate;
            return (
              <Pressable
                key={day.service_date}
                ariaPressed={chosen}
                onClick={() => onChoose(day.service_date)}
                tone="card"
                style={{
                  justifyContent: "space-between",
                  gap: SPACE[3],
                  padding: `${SPACE[2]}px ${SPACE[3]}px`,
                  borderRadius: RADIUS.sm,
                  background: chosen ? "var(--btn)" : "var(--sec)",
                  color: chosen ? "var(--btn-text)" : "var(--txt)",
                }}
              >
                <span style={{ fontSize: TEXT.base, fontWeight: 600 }}>
                  {fmt.dayName(day.service_date, today)} · {fmt.dayStamp(day.service_date)}
                </span>
                <span
                  style={{
                    fontSize: TEXT.sm,
                    fontWeight: 600,
                    color: chosen
                      ? "var(--btn-text)"
                      : day.closed
                        ? "var(--warn)"
                        : "var(--hint)",
                    whiteSpace: "nowrap",
                  }}
                >
                  {day.closed ? "выходной" : day.bookings === 0 ? "пусто" : fmt.bookings(day.bookings)}
                </span>
              </Pressable>
            );
          })}
        </div>
      </div>
    </Sheet>
  );
}

// ---- at the door --------------------------------------------------------------------------------

/**
 * A party that walked in.
 *
 * The app names the table it would use before anybody commits, because the bartender is about to
 * walk somebody across a room and needs to know where to. It is the same allocator that answers
 * the shift's own "who fits" line, so the two can never offer different tables.
 */
export function WalkInSheet({
  open,
  shift,
  maxParty,
  turnMinutes,
  partySize,
  onClose,
  onPartySize,
  onSeat,
}: {
  open: boolean;
  shift: ShiftView | null;
  maxParty: number;
  turnMinutes: number;
  partySize: number;
  onClose: () => void;
  onPartySize: (size: number) => void;
  onSeat: () => void;
}) {
  if (!shift || shift.now_minutes === null) return null;
  const until = shift.now_minutes + turnMinutes;
  const table = chooseWalkInTable(shift, partySize, turnMinutes);

  return (
    <Sheet
      open={open}
      onClose={onClose}
      title="Посадить сейчас"
      footer={
        <CardAction
          tone="primary"
          disabled={table === null}
          label={table ? `Посадить за стол ${table.number}` : "Посадить некуда"}
          onClick={onSeat}
        />
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[4] }}>
        <SheetTitle>Посадить сейчас</SheetTitle>

        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
          <SectionLabel>Сколько гостей</SectionLabel>
          <PartySizeGrid max={maxParty} value={partySize} onChange={onPartySize} />
        </div>

        {table ? (
          <Card gap={SPACE[1] + 2}>
            <span style={{ fontSize: TEXT.xl, fontWeight: 700, color: "var(--txt)" }}>
              Стол {table.number} · {table.zone}
            </span>
            <Note>
              {fmt.seats(table.seats)}, занят до {fmt.time(until)}. Самый маленький подходящий —
              большие остаются для больших компаний.
            </Note>
          </Card>
        ) : (
          <Card gap={SPACE[1] + 2}>
            <span style={{ fontSize: TEXT.xl, fontWeight: 700, color: "var(--warn)" }}>
              Свободного стола нет
            </span>
            <Note>Все подходящие столы заняты. Освободите стол или предложите подождать.</Note>
          </Card>
        )}
      </div>
    </Sheet>
  );
}

/**
 * The table the walk-in would take: smallest that fits, free for the whole turn from now.
 *
 * The same rule as the server's, so the sheet can name a table before the request is made — a
 * bartender is about to walk somebody across a room and needs to know where to. The server runs it
 * again inside the transaction and has the last word, which is what makes two bartenders tapping
 * at the same moment safe rather than merely unlikely.
 */
export function chooseWalkInTable(
  shift: ShiftView,
  partySize: number,
  turnMinutes: number,
): ShiftTable | null {
  const now = shift.now_minutes;
  if (now === null) return null;
  const until = now + turnMinutes;
  const candidates = [...shift.tables]
    .filter((table) => table.blocked_because === null && table.seats >= partySize)
    .sort((left, right) => left.seats - right.seats || left.number - right.number);
  return (
    candidates.find(
      (table) =>
        !shift.bookings.some(
          (booking) => booking.table_id === table.id && holdsDuring(booking, now, until),
        ),
    ) ?? null
  );
}

/** A booking taken over the telephone or at the door for a later evening. */
export function ManualBookingSheet({
  open,
  maxParty,
  availability,
  partySize,
  chosenMinutes,
  guestName,
  failedToLoad,
  onClose,
  onPartySize,
  onPick,
  onTakenSlot,
  onGuestName,
  onRetry,
  onCreate,
}: {
  open: boolean;
  maxParty: number;
  availability: Availability | null;
  partySize: number;
  chosenMinutes: number | null;
  guestName: string;
  failedToLoad: boolean;
  onClose: () => void;
  onPartySize: (size: number) => void;
  onPick: (minutes: number) => void;
  onTakenSlot: () => void;
  onGuestName: (name: string) => void;
  onRetry: () => void;
  onCreate: () => void;
}) {
  const offered = (availability?.slots ?? []).filter((slot) => slot.state !== "past");
  const ready = guestName.trim().length > 0 && chosenMinutes !== null;

  return (
    <Sheet
      open={open}
      onClose={onClose}
      title="Записать гостя"
      footer={
        <CardAction
          tone="primary"
          disabled={!ready}
          label={
            ready && chosenMinutes !== null
              ? `Записать на ${fmt.time(chosenMinutes)}`
              : "Имя и время"
          }
          onClick={onCreate}
        />
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[4] + 2 }}>
        <SheetTitle>Записать гостя</SheetTitle>

        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
          <SectionLabel>Имя гостя</SectionLabel>
          <TextField value={guestName} placeholder="Как записать" onChange={onGuestName} />
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
          <SectionLabel>Сколько гостей</SectionLabel>
          <PartySizeGrid max={maxParty} value={partySize} onChange={onPartySize} />
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
          <SectionLabel>Время</SectionLabel>
          {availability === null ? (
            failedToLoad ? (
              <>
                <Note tone="warn">Не удалось прочитать свободные окна.</Note>
                <CardAction label="Попробовать снова" onClick={onRetry} />
              </>
            ) : (
              <Spinner label="Считаем свободные окна" />
            )
          ) : offered.length === 0 ? (
            <Note tone="warn">В этот вечер не осталось ни одного времени.</Note>
          ) : (
            <SlotGrid
              slots={offered}
              chosen={chosenMinutes}
              onPick={onPick}
              onTaken={onTakenSlot}
              height={40}
              fontSize={TEXT.base}
              gap={SPACE[1] + 2}
            />
          )}
        </div>
      </div>
    </Sheet>
  );
}

/** The guest's own "are you sure": a cancellation restated before it happens. */
export function GuestCancelSheet({
  open,
  booking,
  today,
  onClose,
  onConfirm,
}: {
  open: boolean;
  booking: GuestBooking | null;
  today: string;
  onClose: () => void;
  onConfirm: () => void;
}) {
  if (!booking) return null;
  return (
    <ConfirmSheet
      open={open}
      title="Отменить бронь?"
      restated={`${fmt.whenLabel(booking.service_date, today, booking.start_minutes)} · ${fmt.guests(
        booking.party_size,
      )}`}
      detail="Стол сразу уйдёт другим гостям. Вернуть его получится, только если он останется свободен."
      confirmLabel="Отменить бронь"
      onConfirm={onConfirm}
      onClose={onClose}
    />
  );
}

