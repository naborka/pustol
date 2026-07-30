"use client";

/**
 * The sheets: the guest card, the messages, the reasons, closing a table, and a booking by hand.
 *
 * Each is a decision with consequences, which is why it gets a sheet rather than an inline control:
 * the screen underneath stops accepting taps while somebody chooses whether to cancel an evening.
 */

import { useState } from "react";

import type { Availability, ShiftBooking, ShiftTable } from "@/lib/api";
import * as fmt from "@/lib/format";
import { openChatWith } from "@/lib/telegram";
import {
  CardAction,
  DaytimeDisclosure,
  Note,
  PartySizeRow,
  RADIUS,
  Segmented,
  SectionLabel,
  Sheet,
  SheetChoice,
  SheetTitle,
  SlotGrid,
  TextField,
  statusColor,
} from "./ui";

/** The guest's card: who they are, whether they came, and how to reach them. */
export function BookingSheet({
  open,
  booking,
  onClose,
  onAttendance,
  onOpenTemplates,
  onOpenCancel,
  onFindTable,
}: {
  open: boolean;
  booking: ShiftBooking | null;
  onClose: () => void;
  onAttendance: (attendance: "confirmed" | "arrived" | "no_show") => void;
  onOpenTemplates: () => void;
  onOpenCancel: () => void;
  onFindTable: () => void;
}) {
  if (!booking) return null;
  const seated = booking.table_id !== null;
  return (
    <Sheet open={open} onClose={onClose} title={booking.guest_name}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span
            style={{
              fontSize: 22,
              fontWeight: 700,
              color: "var(--txt)",
              letterSpacing: "-.01em",
            }}
          >
            {booking.guest_name}
          </span>
          <span style={{ fontSize: 14, color: "var(--hint)" }}>
            {fmt.time(booking.start_minutes)}–{fmt.time(booking.end_minutes)} ·{" "}
            {fmt.guests(booking.party_size)} ·{" "}
            {seated
              ? `стол ${booking.table_number}${booking.table_zone ? ` (${booking.table_zone})` : ""}`
              : "стол не назначен"}
          </span>
        </div>

        {!seated ? (
          <div
            style={{
              padding: 14,
              borderRadius: RADIUS.chip,
              background: "rgba(234,161,58,.16)",
              display: "flex",
              flexDirection: "column",
              gap: 11,
            }}
          >
            <span
              style={{
                fontSize: 13,
                color: "var(--warn)",
                lineHeight: 1.45,
                textWrap: "pretty",
              }}
            >
              Стол закрыли, а свободного на это время не нашлось. Гость об этом ещё не знает.
            </span>
            <button
              type="button"
              onClick={onFindTable}
              style={{
                padding: 12,
                borderRadius: 10,
                background: "var(--warn)",
                color: "#1a1205",
                fontSize: 14,
                fontWeight: 700,
                textAlign: "center",
              }}
            >
              Найти стол
            </button>
          </div>
        ) : null}

        <div
          style={{
            display: "flex",
            background: "var(--sec)",
            borderRadius: RADIUS.small,
            padding: 3,
            gap: 3,
          }}
        >
          {(
            [
              ["confirmed", "Ждём"],
              ["arrived", "Пришли"],
              ["no_show", "Не пришли"],
            ] as const
          ).map(([value, label]) => {
            const chosen = booking.status === value;
            return (
              <button
                key={value}
                type="button"
                aria-pressed={chosen}
                onClick={() => onAttendance(value)}
                style={{
                  flex: 1,
                  padding: 9,
                  borderRadius: 9,
                  textAlign: "center",
                  fontSize: 13,
                  fontWeight: 600,
                  background: chosen ? statusColor(value) : "transparent",
                  color: chosen ? "#fff" : "var(--hint)",
                }}
              >
                {label}
              </button>
            );
          })}
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <SectionLabel>Связаться</SectionLabel>
          <button
            type="button"
            onClick={onOpenTemplates}
            style={contactRowStyle}
          >
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              <span style={{ fontSize: 15, fontWeight: 600, color: "var(--txt)" }}>
                Написать в бот
              </span>
              <span
                style={{
                  fontSize: 12,
                  color: booking.reachable_by_bot ? "var(--ok)" : "var(--warn)",
                }}
              >
                {booking.reachable_by_bot
                  ? "Гость бронировал через приложение"
                  : "Гость бронировал не через приложение"}
              </span>
            </div>
            <span aria-hidden style={{ fontSize: 16, color: "var(--hint)" }}>
              ›
            </span>
          </button>

          <button
            type="button"
            disabled={!booking.guest_username}
            onClick={() => {
              if (booking.guest_username) openChatWith(booking.guest_username);
            }}
            style={{ ...contactRowStyle, opacity: booking.guest_username ? 1 : 0.5 }}
          >
            <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              <span style={{ fontSize: 15, fontWeight: 600, color: "var(--txt)" }}>
                Открыть чат с гостем
              </span>
              <span
                style={{
                  fontSize: 12,
                  color: booking.guest_username ? "var(--hint)" : "var(--warn)",
                }}
              >
                {booking.guest_username
                  ? `@${booking.guest_username}`
                  : "Username скрыт — чат недоступен"}
              </span>
            </div>
            <span aria-hidden style={{ fontSize: 16, color: "var(--hint)" }}>
              ›
            </span>
          </button>
        </div>

        <button
          type="button"
          onClick={onOpenCancel}
          style={{
            padding: 14,
            borderRadius: RADIUS.chip,
            background: "var(--tint)",
            color: "var(--dest)",
            fontSize: 15,
            fontWeight: 600,
            textAlign: "center",
          }}
        >
          Отменить бронь
        </button>
      </div>
    </Sheet>
  );
}

const contactRowStyle = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: 14,
  background: "var(--sec)",
  borderRadius: RADIUS.chip,
  width: "100%",
} as const;

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
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <SheetTitle>{title}</SheetTitle>
        <span style={{ fontSize: 13, color: "var(--hint)", lineHeight: 1.45 }}>{hint}</span>
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {choices.map((choice) => (
            <SheetChoice key={choice} text={choice} onClick={() => onChoose(choice)} />
          ))}
        </div>
      </div>
    </Sheet>
  );
}

/** The reasons a table goes out of service. Staff-facing, so they live with the screen. */
export const BLOCK_REASONS = [
  "Сломан / залит",
  "Дождь",
  "Частное мероприятие",
  "Держим для своих",
] as const;

export function BlockSheet({
  open,
  table,
  tables,
  bookings,
  onClose,
  onBlock,
  onUnblock,
}: {
  open: boolean;
  table: ShiftTable | null;
  tables: ShiftTable[];
  bookings: ShiftBooking[];
  onClose: () => void;
  onBlock: (tableIds: string[], reason: string) => void;
  onUnblock: (tableIds: string[]) => void;
}) {
  const [scope, setScope] = useState<"table" | "zone">("table");
  if (!table) return null;

  const zoneTables = tables.filter((other) => other.zone === table.zone);
  const chosen = scope === "zone" ? zoneTables : [table];
  const chosenIds = chosen.map((entry) => entry.id);
  const alreadyClosed = chosen.every((entry) => entry.blocked_because !== null);
  const affected = bookings.filter(
    (booking) => booking.table_id !== null && chosenIds.includes(booking.table_id),
  ).length;

  const title =
    scope === "zone" ? `Зона «${table.zone}»` : `Стол ${table.number}`;

  return (
    <Sheet open={open} onClose={onClose} title={title}>
      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <SheetTitle>
            {title}
            {alreadyClosed ? " — закрыто" : ""}
          </SheetTitle>
          <span
            style={{ fontSize: 13, color: "var(--hint)", lineHeight: 1.45, textWrap: "pretty" }}
          >
            {alreadyClosed
              ? "Не участвует в подборе. Откройте, когда снова можно сажать гостей."
              : affected > 0
                ? `Здесь ${fmt.bookings(affected)} — попробуем пересадить их автоматически и покажем, что изменилось.`
                : "Броней здесь нет — закрытие ни на кого не повлияет."}
          </span>
        </div>

        {zoneTables.length > 1 ? (
          <Segmented
            options={[
              { value: "table" as const, label: "Только стол" },
              { value: "zone" as const, label: `Вся зона «${table.zone}»` },
            ]}
            value={scope}
            onChange={setScope}
          />
        ) : null}

        {alreadyClosed ? (
          <CardAction
            tone="primary"
            label="Открыть стол снова"
            onClick={() => onUnblock(chosenIds)}
          />
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            {BLOCK_REASONS.map((reason) => (
              <SheetChoice
                key={reason}
                text={reason}
                onClick={() => onBlock(chosenIds, reason)}
              />
            ))}
          </div>
        )}
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
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <SheetTitle tone="destructive">Так сохранить нельзя</SheetTitle>
        <span
          style={{ fontSize: 13, color: "var(--hint)", lineHeight: 1.45, textWrap: "pretty" }}
        >
          Эти брони уже приняты по действующим правилам. Сначала перенесите или отмените их — тогда
          настройку можно будет сохранить.
        </span>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {reasons.map((reason) => (
            <span
              key={reason}
              style={{
                padding: "12px 13px",
                background: "var(--tint)",
                borderRadius: RADIUS.small,
                fontSize: 13,
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

/** A booking taken over the telephone or at the door. */
export function NewBookingSheet({
  open,
  maxParty,
  availability,
  partySize,
  chosenMinutes,
  guestName,
  daytimeShown,
  onClose,
  onPartySize,
  onPick,
  onGuestName,
  onShowDaytime,
  onCreate,
}: {
  open: boolean;
  maxParty: number;
  availability: Availability | null;
  partySize: number;
  chosenMinutes: number | null;
  guestName: string;
  daytimeShown: boolean;
  onClose: () => void;
  onPartySize: (size: number) => void;
  onPick: (minutes: number) => void;
  onGuestName: (name: string) => void;
  onShowDaytime: () => void;
  onCreate: () => void;
}) {
  const slots = availability?.slots ?? [];
  const daytime = slots.filter((slot) => !slot.evening);
  const evening = slots.filter((slot) => slot.evening);
  const ready = guestName.trim().length > 0 && chosenMinutes !== null;
  const grid = (list: Availability["slots"]) => (
    <SlotGrid slots={list} chosen={chosenMinutes} onPick={onPick} height={40} fontSize={14} gap={6} />
  );

  return (
    <Sheet open={open} onClose={onClose} title="Бронь вручную">
      <div style={{ display: "flex", flexDirection: "column", gap: 18 }}>
        <SheetTitle>Бронь вручную</SheetTitle>

        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <SectionLabel>Имя гостя</SectionLabel>
          <TextField
            value={guestName}
            placeholder="Как записать"
            onChange={onGuestName}
          />
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <SectionLabel>Гостей</SectionLabel>
          <PartySizeRow
            max={maxParty}
            value={partySize}
            onChange={onPartySize}
            height={44}
            fontSize={16}
            gap={6}
          />
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <SectionLabel>Время</SectionLabel>
          {daytime.length > 0 && !daytimeShown ? (
            <DaytimeDisclosure
              compact
              from={daytime[0]?.start_minutes ?? 0}
              to={daytime[daytime.length - 1]?.start_minutes ?? 0}
              onShow={onShowDaytime}
            />
          ) : null}
          {daytime.length > 0 && daytimeShown ? grid(daytime) : null}
          {grid(evening)}
          {availability !== null && availability.free_count === 0 ? (
            <Note tone="warn">На этот день нет окон для такой компании.</Note>
          ) : null}
        </div>

        <button
          type="button"
          disabled={!ready}
          onClick={onCreate}
          style={{
            padding: 15,
            borderRadius: RADIUS.chip,
            background: ready ? "var(--btn)" : "var(--chip)",
            color: ready ? "var(--btn-text)" : "var(--hint)",
            fontSize: 16,
            fontWeight: 600,
            textAlign: "center",
          }}
        >
          {ready && chosenMinutes !== null
            ? `Записать на ${fmt.time(chosenMinutes)}`
            : "Имя и время"}
        </button>
      </div>
    </Sheet>
  );
}
