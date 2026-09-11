"use client";

/**
 * The shift, as the person working it needs to see it.
 *
 * Three views of one evening, and they answer three different questions.
 *
 * **Сейчас** is the default and the one staff live in. It answers *who is next, who is late, and
 * what can I seat right now* — all three on the same screen, because a list that sends a bartender
 * to another view to find out whether four people fit has failed at the only moment it mattered.
 *
 * **Столы** answers the other question: *which window do I put this booking in.* Same data, drawn
 * against the clock.
 *
 * **Итоги** is the shift's own receipt, and it is only honest because walk-ins are recorded.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import type { ShiftBooking, ShiftTable, ShiftView } from "@/lib/api";
import * as fmt from "@/lib/format";
import {
  hourlyLoad,
  occupancyEnd,
  peakHour,
  seatedGuestsAt,
  shiftTotals,
} from "@/lib/occupancy";
import {
  GROUP_ORDER,
  GROUP_TITLE,
  groupOf,
  standingOf,
  statusColor,
  statusLabel,
  statusWash,
  type ShiftGroup,
  type Standing,
} from "@/lib/status";
import { RADIUS, SPACE, TAP, TEXT } from "@/lib/tokens";
import {
  Card,
  CardAction,
  Empty,
  Note,
  Pressable,
  Rail,
  Segmented,
  SectionLabel,
  TextField,
} from "./ui";

export type ShiftPane = "now" | "tables" | "totals";

/** Pixels per hour on the timeline. */
const HOUR_WIDTH = 68;
/** One table's row. Tall enough for a number, its seats, and a finger. */
const ROW_HEIGHT = 48;
const LABEL_WIDTH = 56;
const HEADER_HEIGHT = 26;

/** Where a minute sits on the timeline. */
function xOf(minutes: number, openMinutes: number): number {
  return ((minutes - openMinutes) * HOUR_WIDTH) / 60;
}

// ---- the day ------------------------------------------------------------------------------------

export function DayHeader({
  shift,
  today,
  onServiceDate,
  onOpenDays,
}: {
  shift: ShiftView;
  today: string;
  onServiceDate: (date: string) => void;
  onOpenDays: () => void;
}) {
  const step = (delta: number) => onServiceDate(fmt.addDays(shift.service_date, delta));
  const arrow = {
    flex: "none" as const,
    width: 48,
    minHeight: 48,
    borderRadius: RADIUS.md,
    background: "var(--sec)",
    color: "var(--txt)",
    justifyContent: "center" as const,
    fontSize: TEXT.xl,
  };
  return (
    <div
      role="group"
      aria-label="День смены"
      style={{
        display: "flex",
        alignItems: "stretch",
        gap: SPACE[2],
        padding: `${SPACE[3]}px ${SPACE[3]}px ${SPACE[2]}px`,
      }}
    >
      <Pressable ariaLabel="Предыдущий день" onClick={() => step(-1)} style={arrow}>
        ‹
      </Pressable>
      <Pressable
        ariaLabel="Выбрать день"
        onClick={onOpenDays}
        tone="card"
        style={{
          flex: 1,
          minHeight: 48,
          borderRadius: RADIUS.md,
          background: "var(--chip)",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          gap: 1,
        }}
      >
        <span style={{ fontSize: TEXT.lg, fontWeight: 700, color: "var(--txt)" }}>
          {fmt.dayName(shift.service_date, today)}
        </span>
        <span style={{ fontSize: TEXT.xs, fontWeight: 600, color: "var(--hint)" }}>
          {fmt.dayStamp(shift.service_date)}
        </span>
      </Pressable>
      <Pressable ariaLabel="Следующий день" onClick={() => step(1)} style={arrow}>
        ›
      </Pressable>
    </div>
  );
}

// ---- the pulse ----------------------------------------------------------------------------------

/**
 * Two lines rather than three stat cards.
 *
 * The first says what the room is doing; the second says what it can still take. A count of free
 * tables alone does not tell a bartender whether the four people at the door fit, and sending him
 * to another view to find out is exactly the flaw this list must not have.
 */
export function Pulse({ shift }: { shift: ShiftView }) {
  const running = shift.now_minutes !== null;
  // The free count comes from the server, which answers it from the same occupancy rule against
  // the whole room. Recomputing it here would be a second implementation of a number the two are
  // required to agree on, and the only way two implementations stay in step is by not existing.
  const free = shift.stats.free_now;
  const seated = running ? seatedGuestsAt(shift.bookings, shift.now_minutes ?? 0) : null;
  const fits = shift.largest_party_seatable_now;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: SPACE[1],
        padding: `0 ${SPACE[3]}px ${SPACE[2] + 2}px`,
      }}
    >
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "baseline",
          gap: SPACE[3],
        }}
      >
        <span style={{ fontSize: TEXT.lg, fontWeight: 600, color: "var(--txt)" }}>
          {running
            ? `${fmt.time(shift.now_minutes ?? 0)} · ${fmt.guests(seated ?? 0)} за столами`
            : `${fmt.bookings(shift.stats.bookings)} · ${fmt.guests(shift.stats.guests)}`}
        </span>
        {free === null ? null : (
          <span
            style={{
              fontSize: TEXT.lg,
              fontWeight: 600,
              color: free <= 2 ? "var(--dest)" : "var(--txt)",
              whiteSpace: "nowrap",
            }}
          >
            {fmt.tables(free)} свободно
          </span>
        )}
      </div>
      {running ? (
        <Note tone={fits === null ? "warn" : "hint"}>
          {fits === null
            ? "Посадить сейчас некуда: все подходящие столы заняты"
            : `Сейчас можно посадить компанию до ${fmt.guests(fits)}`}
        </Note>
      ) : null}
    </div>
  );
}

// ---- one row ------------------------------------------------------------------------------------

/** Where a booking is, in the words that row uses. */
function placeLine(booking: ShiftBooking): { text: string; warn: boolean } {
  if (booking.table_id === null) return { text: "Без стола", warn: true };
  const zone = booking.table_zone ? ` · ${booking.table_zone}` : "";
  return { text: `стол ${booking.table_number}${zone}`, warn: false };
}

/** What the row's status line says, which for a party with no table is about the table. */
function standingLine(booking: ShiftBooking, standing: Standing): string {
  if (booking.table_id === null) {
    return standing.kind === "seated"
      ? "Сидят, но стол за ними не закреплён"
      : "Посадить некуда";
  }
  return statusLabel(standing);
}

export interface RowActions {
  onOpen: (booking: ShiftBooking) => void;
  onSeat: (booking: ShiftBooking) => void;
  onLeft: (booking: ShiftBooking) => void;
  onFindTable: () => void;
}

export function BookingRow({
  booking,
  standing,
  actions,
}: {
  booking: ShiftBooking;
  standing: Standing;
  actions: RowActions;
}) {
  const place = placeLine(booking);
  const orphan = booking.table_id === null;
  const late = standing.kind === "late";

  const action = orphan
    ? { label: "Найти стол", tone: "warn" as const, run: actions.onFindTable }
    : standing.kind === "waiting" || standing.kind === "late"
      ? { label: "Посадить", tone: "primary" as const, run: () => actions.onSeat(booking) }
      : standing.kind === "seated"
        ? { label: "Ушли", tone: "quiet" as const, run: () => actions.onLeft(booking) }
        : null;

  return (
    <div
      style={{
        display: "flex",
        alignItems: "stretch",
        minHeight: 64,
        background: "var(--sec)",
        borderRadius: RADIUS.md,
        overflow: "hidden",
      }}
    >
      <Pressable
        onClick={() => actions.onOpen(booking)}
        tone="card"
        style={{
          flex: 1,
          minWidth: 0,
          gap: SPACE[3],
          padding: `${SPACE[2] + 2}px ${SPACE[3]}px`,
          alignItems: "center",
        }}
      >
        <div
          style={{
            flex: "none",
            width: 52,
            display: "flex",
            flexDirection: "column",
            gap: 1,
          }}
        >
          <span
            style={{
              fontSize: TEXT.xl,
              fontWeight: 700,
              color: late ? "var(--dest)" : "var(--txt)",
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {fmt.time(booking.start_minutes)}
          </span>
          <span style={{ fontSize: TEXT.xs, color: "var(--hint)" }}>
            {booking.party_size} чел.
          </span>
        </div>

        <div
          style={{
            flex: 1,
            minWidth: 0,
            display: "flex",
            flexDirection: "column",
            gap: 2,
            textAlign: "left",
          }}
        >
          <span
            style={{
              fontSize: TEXT.lg,
              fontWeight: 600,
              color: "var(--txt)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {booking.guest_name}
          </span>
          <span
            style={{
              fontSize: TEXT.sm,
              fontWeight: place.warn ? 600 : 400,
              color: place.warn ? "var(--warn)" : "var(--hint)",
            }}
          >
            {place.text}
          </span>
          {booking.note ? <NoteChip text={booking.note} /> : null}
          <span
            style={{
              fontSize: TEXT.sm,
              fontWeight: 600,
              color: orphan ? "var(--warn)" : statusColor(standing),
            }}
          >
            {standingLine(booking, standing)}
          </span>
        </div>
      </Pressable>

      {action ? (
        <>
          {action.tone === "quiet" ? (
            <div aria-hidden style={{ width: 1, background: "var(--sep)" }} />
          ) : null}
          <Pressable
            onClick={action.run}
            style={{
              flex: "none",
              alignSelf: "center",
              margin: SPACE[2],
              padding: `0 ${SPACE[3]}px`,
              minHeight: TAP,
              borderRadius: RADIUS.sm,
              justifyContent: "center",
              fontSize: TEXT.base,
              fontWeight: 700,
              background:
                action.tone === "primary"
                  ? "var(--btn)"
                  : action.tone === "warn"
                    ? "var(--warn-wash)"
                    : "transparent",
              color:
                action.tone === "primary"
                  ? "var(--btn-text)"
                  : action.tone === "warn"
                    ? "var(--warn)"
                    : "var(--hint)",
              whiteSpace: "nowrap",
            }}
          >
            {action.label}
          </Pressable>
        </>
      ) : null}
    </div>
  );
}

export function NoteChip({ text }: { text: string }) {
  return (
    <span
      style={{
        alignSelf: "flex-start",
        padding: `2px ${SPACE[2]}px`,
        borderRadius: RADIUS.pill,
        background: "var(--chip-off)",
        color: "var(--txt)",
        fontSize: TEXT.xs,
        fontWeight: 600,
      }}
    >
      {text}
    </span>
  );
}

// ---- Сейчас -------------------------------------------------------------------------------------

/**
 * Whether a booking matches what somebody typed: a name, or a table number.
 *
 * Those two and nothing else. A guest at the door says a name; a colleague across the room says a
 * number. Matching notes as well would quietly turn "Аллергия" into a way to lose the guest you
 * were looking for.
 */
export function matchesSearch(booking: ShiftBooking, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return true;
  if (booking.guest_name.toLowerCase().includes(needle)) return true;
  return booking.table_number !== null && String(booking.table_number).includes(needle);
}

export function NowPane({
  shift,
  graceMinutes,
  search,
  onSearch,
  actions,
}: {
  shift: ShiftView;
  graceMinutes: number;
  search: string;
  onSearch: (value: string) => void;
  actions: RowActions;
}) {
  const rows = useMemo(() => {
    const matching = shift.bookings.filter((booking) => matchesSearch(booking, search));
    const grouped = new Map<ShiftGroup, { booking: ShiftBooking; standing: Standing }[]>();
    for (const booking of matching) {
      const standing = standingOf(booking, shift.now_minutes, graceMinutes);
      const group = groupOf(booking, standing);
      const bucket = grouped.get(group);
      if (bucket) bucket.push({ booking, standing });
      else grouped.set(group, [{ booking, standing }]);
    }
    for (const bucket of grouped.values()) {
      bucket.sort((left, right) => left.booking.start_minutes - right.booking.start_minutes);
    }
    return grouped;
  }, [shift.bookings, shift.now_minutes, graceMinutes, search]);

  const anything = [...rows.values()].some((bucket) => bucket.length > 0);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: SPACE[4],
        padding: `0 ${SPACE[3]}px ${SPACE[5]}px`,
      }}
    >
      {/*
        Sticky rather than merely present. A bartender with somebody at the door is usually part
        way down a busy list, and a search box that has scrolled off the top is a search box that
        costs a scroll before it costs a keystroke.
      */}
      <div
        style={{
          position: "sticky",
          top: 0,
          zIndex: 3,
          background: "var(--bg)",
          paddingBottom: SPACE[2],
          display: "flex",
          alignItems: "center",
          gap: SPACE[2],
        }}
      >
        <TextField
          value={search}
          placeholder="Поиск: имя или номер стола"
          ariaLabel="Поиск: имя или номер стола"
          onChange={onSearch}
        />
        {search.length > 0 ? (
          <Pressable
            ariaLabel="Очистить поиск"
            onClick={() => onSearch("")}
            style={{
              flex: "none",
              width: TAP,
              minHeight: TAP,
              borderRadius: RADIUS.sm,
              justifyContent: "center",
              color: "var(--hint)",
              fontSize: TEXT.xl,
            }}
          >
            ×
          </Pressable>
        ) : null}
      </div>

      {!anything ? (
        search.trim().length > 0 ? (
          <Empty title="Никого не нашли" detail="Проверьте имя или номер стола." />
        ) : (
          <Empty
            title="На этот вечер броней нет"
            detail="Гости с улицы и брони по телефону — кнопками внизу."
          />
        )
      ) : null}

      {GROUP_ORDER.map((group) => {
        const bucket = rows.get(group) ?? [];
        if (bucket.length === 0) return null;
        return (
          <section
            key={group}
            style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}
          >
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: SPACE[3],
              }}
            >
              <SectionLabel>
                {GROUP_TITLE[group]} · {bucket.length}
              </SectionLabel>
              {group === "orphan" ? (
                <Pressable
                  onClick={actions.onFindTable}
                  style={{
                    padding: `0 ${SPACE[2]}px`,
                    color: "var(--warn)",
                    fontSize: TEXT.base,
                    fontWeight: 700,
                  }}
                >
                  Найти стол
                </Pressable>
              ) : null}
            </div>
            {group === "orphan" ? (
              <Note tone="warn">
                Стол под ними закрыли или уменьшили. Гостям об этом не сообщают — их бронь всё ещё
                выглядит подтверждённой.
              </Note>
            ) : null}
            {bucket.map(({ booking, standing }) => (
              <BookingRow
                key={booking.id}
                booking={booking}
                standing={standing}
                actions={actions}
              />
            ))}
          </section>
        );
      })}
    </div>
  );
}

// ---- Столы --------------------------------------------------------------------------------------

/**
 * The same box model on both columns, written once.
 *
 * The label column and the row grid are laid out independently and must stay in step. A `<button>`
 * gets `border-box` from the user agent and a `<div>` gets whatever the reset says, so a 1px border
 * turns into a row of drift per table and a quarter-row error by table twelve. Both sides take
 * this, and a test asserts that every label lines up with its row.
 */
const ROW_BOX = {
  boxSizing: "border-box" as const,
  height: ROW_HEIGHT,
  borderBottom: "1px solid var(--sep)",
};

export function TablesPane({
  shift,
  graceMinutes,
  onOpenBooking,
  onOpenTable,
}: {
  shift: ShiftView;
  graceMinutes: number;
  onOpenBooking: (booking: ShiftBooking) => void;
  onOpenTable: (table: ShiftTable) => void;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const { open_minutes: open, close_minutes: close } = shift.hours;

  const anchor = (() => {
    if (shift.now_minutes !== null) return shift.now_minutes - 90;
    const earliest = shift.bookings.reduce(
      (soonest, booking) => Math.min(soonest, booking.start_minutes),
      Number.POSITIVE_INFINITY,
    );
    if (Number.isFinite(earliest)) return earliest - 60;
    return Math.max(open, 17 * 60);
  })();

  useEffect(() => {
    const element = scroller.current;
    if (!element) return undefined;
    const target = Math.max(0, xOf(anchor, open));
    let frame = 0;
    let tries = 0;
    const apply = () => {
      element.scrollLeft = target;
      if (Math.round(element.scrollLeft) !== Math.round(target) && tries < 4) {
        tries += 1;
        frame = requestAnimationFrame(apply);
      }
    };
    apply();
    return () => cancelAnimationFrame(frame);
  }, [anchor, open, shift.service_date]);

  const width = xOf(close, open) + 20;
  const hourMarks: number[] = [];
  for (let minute = open; minute <= close; minute += 60) hourMarks.push(minute);

  // Grouped once rather than rescanned per table: with fifteen tables and thirty bookings the
  // nested filter runs the predicate four hundred and fifty times for a list it could sort in one
  // pass. The `null` bucket is exactly the set of parties still owed a table.
  const byTable = new Map<string | null, ShiftBooking[]>();
  for (const booking of shift.bookings) {
    const bucket = byTable.get(booking.table_id);
    if (bucket) bucket.push(booking);
    else byTable.set(booking.table_id, [booking]);
  }
  const orphans = byTable.get(null) ?? [];

  /** One booking, drawn over the hours it actually holds its table for. */
  const Block = ({ booking }: { booking: ShiftBooking }) => {
    const standing = standingOf(booking, shift.now_minutes, graceMinutes);
    const held = occupancyEnd(booking) - booking.start_minutes;
    return (
      <Pressable
        onClick={() => onOpenBooking(booking)}
        ariaLabel={`${booking.guest_name}, ${fmt.time(booking.start_minutes)}`}
        style={{
          position: "absolute",
          left: xOf(booking.start_minutes, open),
          width: Math.max(14, (held * HOUR_WIDTH) / 60 - 3),
          top: 3,
          bottom: 3,
          minHeight: 0,
          height: "auto",
          borderRadius: RADIUS.sm - 3,
          background: statusWash(standing),
          borderLeft: `3px solid ${statusColor(standing)}`,
          padding: `0 ${SPACE[1] + 2}px`,
          overflow: "hidden",
          zIndex: 1,
          flexDirection: "column",
          alignItems: "flex-start",
          justifyContent: "center",
          gap: 1,
        }}
      >
        <span
          style={{
            fontSize: TEXT.xs,
            fontWeight: 700,
            color: "var(--txt)",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            maxWidth: "100%",
          }}
        >
          {booking.guest_name}
        </span>
        <span style={{ fontSize: TEXT.xs, color: "var(--hint)", whiteSpace: "nowrap" }}>
          {fmt.time(booking.start_minutes)} · {booking.party_size}
        </span>
      </Pressable>
    );
  };

  // The hour rules are drawn by the background rather than by a span per hour per row, which would
  // be two hundred and fifty identical elements on a fifteen-table shift.
  const hourRules = {
    backgroundImage: "linear-gradient(to right, var(--sep) 1px, transparent 1px)",
    backgroundSize: `${HOUR_WIDTH}px 100%`,
  } as const;

  return (
    <div style={{ display: "flex", padding: `0 0 ${SPACE[4]}px` }}>
      <div
        style={{
          width: LABEL_WIDTH,
          flex: "none",
          background: "var(--bg)",
          zIndex: 2,
          borderRight: "1px solid var(--sep)",
        }}
      >
        <div style={{ height: HEADER_HEIGHT }} />
        {orphans.length > 0 ? (
          <div
            style={{
              ...ROW_BOX,
              display: "flex",
              alignItems: "center",
              padding: `0 ${SPACE[2]}px`,
            }}
          >
            <span style={{ fontSize: TEXT.xs, fontWeight: 700, color: "var(--warn)" }}>
              без стола
            </span>
          </div>
        ) : null}
        {shift.tables.map((table) => (
          <Pressable
            key={table.id}
            onClick={() => onOpenTable(table)}
            ariaLabel={`Стол ${table.number}`}
            style={{
              ...ROW_BOX,
              minHeight: ROW_HEIGHT,
              width: "100%",
              justifyContent: "space-between",
              padding: `0 ${SPACE[2]}px 0 ${SPACE[3]}px`,
            }}
          >
            <span
              style={{
                fontSize: TEXT.base,
                fontWeight: 700,
                color: table.blocked_because ? "var(--dest)" : "var(--txt)",
              }}
            >
              {table.number}
            </span>
            <span style={{ fontSize: TEXT.xs, color: "var(--hint)" }}>{table.seats}</span>
          </Pressable>
        ))}
      </div>

      <div
        ref={scroller}
        style={{
          flex: 1,
          overflowX: "auto",
          overflowY: "hidden",
          touchAction: "pan-x",
          overscrollBehaviorX: "contain",
          WebkitOverflowScrolling: "touch",
        }}
      >
        <div style={{ width, position: "relative" }}>
          <div style={{ height: HEADER_HEIGHT, position: "relative" }}>
            {hourMarks.map((minute) => (
              <span
                key={minute}
                style={{
                  position: "absolute",
                  left: xOf(minute, open),
                  top: 6,
                  fontSize: TEXT.xs,
                  color: "var(--hint)",
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {fmt.time(minute)}
              </span>
            ))}
          </div>

          {orphans.length > 0 ? (
            <div
              style={{
                ...ROW_BOX,
                position: "relative",
                background: "var(--chip-off)",
              }}
            >
              {orphans.map((booking) => (
                <Block key={booking.id} booking={booking} />
              ))}
            </div>
          ) : null}

          {shift.tables.map((table) => {
            const seated = byTable.get(table.id) ?? [];
            return (
              <div
                key={table.id}
                data-table-row={table.id}
                style={{ ...ROW_BOX, position: "relative", ...hourRules }}
              >
                {table.blocked_because ? (
                  <span
                    style={{
                      position: "absolute",
                      left: 0,
                      right: 0,
                      top: 3,
                      bottom: 3,
                      borderRadius: RADIUS.sm - 3,
                      background: "var(--tint)",
                      display: "flex",
                      alignItems: "center",
                      paddingLeft: SPACE[2] + 2,
                      fontSize: TEXT.xs,
                      fontWeight: 600,
                      color: "var(--hint)",
                      zIndex: 0,
                    }}
                  >
                    {table.blocked_because}
                  </span>
                ) : null}
                {seated.map((booking) => (
                  <Block key={booking.id} booking={booking} />
                ))}
              </div>
            );
          })}

          {shift.now_minutes !== null ? (
            <span
              aria-hidden
              style={{
                position: "absolute",
                left: xOf(shift.now_minutes, open),
                top: 20,
                bottom: 0,
                width: 2,
                background: "var(--dest)",
                opacity: 0.85,
              }}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}

// ---- Итоги --------------------------------------------------------------------------------------

function Figure({ value, label }: { value: string; label: string }) {
  return (
    <div
      style={{
        flex: "1 1 30%",
        minWidth: 92,
        background: "var(--sec)",
        borderRadius: RADIUS.md,
        padding: `${SPACE[2] + 2}px ${SPACE[3]}px`,
        display: "flex",
        flexDirection: "column",
        gap: 1,
      }}
    >
      <span
        style={{
          fontSize: 20,
          fontWeight: 700,
          color: "var(--txt)",
          letterSpacing: "-.01em",
        }}
      >
        {value}
      </span>
      <span style={{ fontSize: TEXT.xs, color: "var(--hint)" }}>{label}</span>
    </div>
  );
}

export function TotalsPane({ shift }: { shift: ShiftView }) {
  const totals = shiftTotals(shift);
  const load = hourlyLoad(shift);
  const peak = peakHour(load);
  const tallest = load.reduce((most, hour) => Math.max(most, hour.tables), 0);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: SPACE[4],
        padding: `0 ${SPACE[3]}px ${SPACE[5]}px`,
      }}
    >
      <div style={{ display: "flex", flexWrap: "wrap", gap: SPACE[2] }}>
        <Figure value={String(totals.bookings)} label="броней" />
        <Figure value={String(totals.guests)} label="гостей" />
        <Figure value={String(totals.arrived)} label="пришли" />
        <Figure value={String(totals.noShow)} label="не пришли" />
        <Figure value={String(totals.walkIns)} label="без брони" />
        <Figure value={fmt.percent(totals.occupancy)} label="занятость столов" />
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
        <SectionLabel>Загрузка по часам</SectionLabel>
        {load.length === 0 ? (
          <Note>В этот день бар закрыт.</Note>
        ) : (
          <>
            <Rail label="Загрузка по часам" style={{ alignItems: "flex-end", gap: SPACE[1] }}>
              {load.map((hour) => (
                <div
                  key={hour.minute}
                  style={{
                    flex: "none",
                    width: 26,
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "center",
                    gap: SPACE[1],
                  }}
                >
                  <span style={{ fontSize: TEXT.xs, color: "var(--hint)" }}>{hour.tables}</span>
                  <span
                    aria-hidden
                    style={{
                      width: "100%",
                      height: Math.max(
                        2,
                        tallest === 0 ? 2 : Math.round((hour.tables / tallest) * 56),
                      ),
                      borderRadius: RADIUS.sm - 6,
                      background: hour.tables === 0 ? "var(--sep)" : "var(--btn)",
                    }}
                  />
                  <span style={{ fontSize: TEXT.xs, color: "var(--hint)" }}>
                    {fmt.time(hour.minute).slice(0, 2)}
                  </span>
                </div>
              ))}
            </Rail>
            <Note>
              {peak === null
                ? "Ни одного занятого стола за вечер."
                : `Пик в ${fmt.time(peak.minute)} · ${fmt.tables(peak.tables)}`}
            </Note>
          </>
        )}
      </div>
    </div>
  );
}

// ---- the screen ---------------------------------------------------------------------------------

export function ClosedDayCard() {
  return (
    <div style={{ margin: `0 ${SPACE[3]}px ${SPACE[3]}px` }}>
      <Card padding={SPACE[5]} gap={SPACE[1] + 2} style={{ alignItems: "center", textAlign: "center" }}>
        <span style={{ fontSize: TEXT.xl, fontWeight: 700, color: "var(--txt)" }}>Выходной</span>
        <Note>Бар закрыт. Часы — в настройках.</Note>
      </Card>
    </div>
  );
}

export function ShiftScreen({
  shift,
  today,
  graceMinutes,
  pane,
  onPane,
  onServiceDate,
  onOpenDays,
  actions,
  onOpenTable,
}: {
  shift: ShiftView;
  today: string;
  graceMinutes: number;
  pane: ShiftPane;
  onPane: (pane: ShiftPane) => void;
  onServiceDate: (date: string) => void;
  onOpenDays: () => void;
  actions: RowActions;
  onOpenTable: (table: ShiftTable) => void;
}) {
  const [search, setSearch] = useState("");

  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      <DayHeader
        shift={shift}
        today={today}
        onServiceDate={onServiceDate}
        onOpenDays={onOpenDays}
      />
      <Pulse shift={shift} />

      {shift.hours.closed ? (
        <ClosedDayCard />
      ) : (
        <>
          <div style={{ display: "flex", padding: `0 ${SPACE[3]}px ${SPACE[2] + 2}px` }}>
            <Segmented
              label="Вид смены"
              options={[
                { value: "now" as const, label: "Сейчас" },
                { value: "tables" as const, label: "Столы" },
                { value: "totals" as const, label: "Итоги" },
              ]}
              value={pane}
              onChange={onPane}
            />
          </div>

          {pane === "now" ? (
            <NowPane
              shift={shift}
              graceMinutes={graceMinutes}
              search={search}
              onSearch={setSearch}
              actions={actions}
            />
          ) : pane === "tables" ? (
            <TablesPane
              shift={shift}
              graceMinutes={graceMinutes}
              onOpenBooking={actions.onOpen}
              onOpenTable={onOpenTable}
            />
          ) : (
            <TotalsPane shift={shift} />
          )}
        </>
      )}
    </div>
  );
}

/** The action bar under the shift: what staff can start from here. */
export function ShiftActions({
  isToday,
  onWalkIn,
  onManual,
}: {
  isToday: boolean;
  onWalkIn: () => void;
  onManual: () => void;
}) {
  if (!isToday) {
    // Seating somebody "now" on a future evening is not a state this app may offer, so the button
    // is not there to be pressed rather than there and refused.
    return <CardAction tone="primary" label="Записать гостя" onClick={onManual} />;
  }
  return (
    <div style={{ display: "flex", gap: SPACE[2] }}>
      <div style={{ flex: 2 }}>
        <CardAction tone="primary" label="Посадить сейчас" onClick={onWalkIn} />
      </div>
      <div style={{ flex: 1 }}>
        <CardAction label="Записать" onClick={onManual} />
      </div>
    </div>
  );
}
