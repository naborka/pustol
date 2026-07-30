"use client";

/**
 * The shift, as the person working it needs to see it.
 *
 * Two views of the same evening. The timeline answers "what does the room look like at nine" and is
 * the one staff live in; the list answers "who is coming next" and is what you read out. Both are
 * drawn from the same payload, so they cannot disagree.
 */

import { useEffect, useRef } from "react";

import type { ShiftBooking, ShiftTable, ShiftView } from "@/lib/api";
import * as fmt from "@/lib/format";
import { haptics } from "@/lib/telegram";
import { Card, CardAction, Note, Segmented, statusColor } from "./ui";

/** Pixels per hour on the timeline. */
const HOUR_WIDTH = 68;
const ROW_HEIGHT = 34;
const LABEL_WIDTH = 50;
const HEADER_HEIGHT = 26;

/** Where a minute sits on the timeline. */
function xOf(minutes: number, openMinutes: number): number {
  return ((minutes - openMinutes) * HOUR_WIDTH) / 60;
}

function statusWord(status: ShiftBooking["status"]): string {
  if (status === "arrived") return "за столом";
  if (status === "no_show") return "не пришли";
  return "ждём";
}

function blockBackground(status: ShiftBooking["status"]): string {
  if (status === "arrived") return "rgba(66,199,103,.18)";
  if (status === "no_show") return "var(--tint)";
  return "rgba(82,136,193,.20)";
}

export function DayNavigator({
  serviceDate,
  today,
  onChange,
}: {
  serviceDate: string;
  today: string;
  onChange: (date: string) => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "12px 12px 8px",
        gap: 6,
      }}
    >
      <button
        type="button"
        aria-label="Предыдущий день"
        onClick={() => {
          haptics.tap();
          onChange(fmt.addDays(serviceDate, -1));
        }}
        style={arrowStyle}
      >
        ‹
      </button>
      <div
        style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          lineHeight: 1.2,
        }}
      >
        <span style={{ fontSize: 15, fontWeight: 600, color: "var(--txt)" }}>
          {fmt.dayName(serviceDate, today)}
        </span>
        <span style={{ fontSize: 11, color: "var(--hint)" }}>{fmt.dayDate(serviceDate)}</span>
      </div>
      <button
        type="button"
        aria-label="Следующий день"
        onClick={() => {
          haptics.tap();
          onChange(fmt.addDays(serviceDate, 1));
        }}
        style={arrowStyle}
      >
        ›
      </button>
    </div>
  );
}

const arrowStyle = {
  width: 34,
  height: 34,
  borderRadius: 9,
  background: "var(--sec)",
  color: "var(--txt)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  fontSize: 16,
} as const;

function Stat({ value, label, tone }: { value: string; label: string; tone?: "alarm" }) {
  return (
    <div
      style={{
        flex: 1,
        background: "var(--sec)",
        borderRadius: 12,
        padding: "10px 12px",
        display: "flex",
        flexDirection: "column",
        gap: 1,
      }}
    >
      <span
        style={{
          fontSize: 20,
          fontWeight: 700,
          color: tone === "alarm" ? "var(--dest)" : "var(--txt)",
          letterSpacing: "-.01em",
        }}
      >
        {value}
      </span>
      <span style={{ fontSize: 11, color: "var(--hint)" }}>{label}</span>
    </div>
  );
}

export function ShiftStats({ shift }: { shift: ShiftView }) {
  const free = shift.stats.free_now;
  return (
    <div style={{ display: "flex", gap: 8, padding: "4px 12px 10px" }}>
      <Stat value={String(shift.stats.bookings)} label="броней" />
      <Stat value={String(shift.stats.guests)} label="гостей" />
      <Stat
        value={free === null ? "—" : String(free)}
        label="свободно сейчас"
        {...(free !== null && free <= 2 ? { tone: "alarm" as const } : {})}
      />
    </div>
  );
}

/**
 * The timeline.
 *
 * Scrolled on open to where the evening actually starts rather than to opening time: a bar that
 * takes its first booking at seven should not open on three empty hours. The scroll is verified
 * against the element and retried, because a write issued before the grid has laid out is clamped to
 * zero and silently lost.
 */
export function Timeline({
  shift,
  onOpenBooking,
  onTapTable,
}: {
  shift: ShiftView;
  onOpenBooking: (booking: ShiftBooking) => void;
  onTapTable: (table: ShiftTable) => void;
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

  /** One booking as a block on a row. */
  const Block = ({
    booking,
    background,
    accent,
  }: {
    booking: ShiftBooking;
    background: string;
    accent: string;
  }) => (
    <button
      type="button"
      onClick={() => onOpenBooking(booking)}
      style={{
        position: "absolute",
        left: xOf(booking.start_minutes, open),
        width: Math.max(12, xOf(booking.end_minutes, booking.start_minutes) - 3),
        top: 3,
        bottom: 3,
        borderRadius: 7,
        background,
        borderLeft: `3px solid ${accent}`,
        padding: "0 6px",
        display: "flex",
        alignItems: "center",
        overflow: "hidden",
        zIndex: 1,
      }}
    >
      <span
        style={{
          fontSize: 11,
          fontWeight: 600,
          color: "var(--txt)",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {booking.guest_name} · {booking.party_size}
      </span>
    </button>
  );

  // The hour rules are drawn by the background rather than by a span per hour per row, which would
  // be two hundred and fifty identical elements on a fifteen-table shift.
  const hourRules = {
    backgroundImage: "linear-gradient(to right, var(--sep) 1px, transparent 1px)",
    backgroundSize: `${HOUR_WIDTH}px 100%`,
  } as const;

  return (
    <>
      <div style={{ display: "flex", padding: "0 0 12px" }}>
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
                height: ROW_HEIGHT,
                display: "flex",
                alignItems: "center",
                padding: "0 8px 0 12px",
                borderBottom: "1px solid var(--sep)",
              }}
            >
              <span style={{ fontSize: 13, fontWeight: 700, color: "var(--warn)" }}>!</span>
            </div>
          ) : null}
          {shift.tables.map((table) => (
            <button
              key={table.id}
              type="button"
              onClick={() => {
                haptics.tap();
                onTapTable(table);
              }}
              style={{
                height: ROW_HEIGHT,
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                padding: "0 8px 0 12px",
                width: "100%",
                borderBottom: "1px solid var(--sep)",
              }}
            >
              <span
                style={{
                  fontSize: 13,
                  fontWeight: 600,
                  color: table.blocked_because ? "var(--dest)" : "var(--txt)",
                }}
              >
                {table.number}
              </span>
              <span style={{ fontSize: 10, color: "var(--hint)" }}>{table.seats}</span>
            </button>
          ))}
        </div>

        <div ref={scroller} style={{ flex: 1, overflowX: "auto", overflowY: "hidden" }}>
          <div style={{ width, position: "relative" }}>
            <div style={{ height: HEADER_HEIGHT, position: "relative" }}>
              {hourMarks.map((minute) => (
                <span
                  key={minute}
                  style={{
                    position: "absolute",
                    left: xOf(minute, open),
                    top: 6,
                    fontSize: 10,
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
                  height: ROW_HEIGHT,
                  position: "relative",
                  borderBottom: "1px solid var(--sep)",
                  background: "rgba(234,161,58,.07)",
                }}
              >
                {orphans.map((booking) => (
                  <Block
                    key={booking.id}
                    booking={booking}
                    background="rgba(234,161,58,.20)"
                    accent="var(--warn)"
                  />
                ))}
              </div>
            ) : null}

            {shift.tables.map((table) => {
              const seated = byTable.get(table.id) ?? [];
              return (
                <div
                  key={table.id}
                  style={{
                    height: ROW_HEIGHT,
                    position: "relative",
                    borderBottom: "1px solid var(--sep)",
                    ...hourRules,
                  }}
                >
                  {table.blocked_because ? (
                    <span
                      style={{
                        position: "absolute",
                        left: 0,
                        right: 0,
                        top: 3,
                        bottom: 3,
                        borderRadius: 7,
                        background: "var(--tint)",
                        display: "flex",
                        alignItems: "center",
                        paddingLeft: 10,
                        fontSize: 11,
                        fontWeight: 600,
                        color: "var(--hint)",
                        zIndex: 0,
                      }}
                    >
                      {table.blocked_because}
                    </span>
                  ) : null}
                  {seated.map((booking) => (
                    <Block
                      key={booking.id}
                      booking={booking}
                      background={blockBackground(booking.status)}
                      accent={statusColor(booking.status)}
                    />
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
      <span
        style={{
          padding: "0 16px 10px",
          fontSize: 11,
          color: "var(--hint)",
          lineHeight: 1.5,
          textWrap: "pretty",
        }}
      >
        Нажмите на бронь — карточка гостя. Нажмите на номер стола слева — закрыть стол.
      </span>
    </>
  );
}

export function ShiftList({
  shift,
  onOpenBooking,
}: {
  shift: ShiftView;
  onOpenBooking: (booking: ShiftBooking) => void;
}) {
  const ordered = [...shift.bookings].sort((a, b) => a.start_minutes - b.start_minutes);
  if (ordered.length === 0) {
    return (
      <div style={{ padding: "30px 0", textAlign: "center" }}>
        <span style={{ fontSize: 14, color: "var(--hint)" }}>На этот день броней нет</span>
      </div>
    );
  }
  return (
    <div style={{ padding: "0 12px 12px", display: "flex", flexDirection: "column", gap: 6 }}>
      {ordered.map((booking) => {
        const seated = booking.table_id !== null;
        const dot = seated ? statusColor(booking.status) : "var(--warn)";
        return (
          <button
            key={booking.id}
            type="button"
            onClick={() => onOpenBooking(booking)}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              padding: "11px 12px",
              background: "var(--sec)",
              borderRadius: 12,
              width: "100%",
            }}
          >
            <span
              style={{
                fontSize: 15,
                fontWeight: 700,
                color: "var(--txt)",
                fontVariantNumeric: "tabular-nums",
                minWidth: 44,
              }}
            >
              {fmt.time(booking.start_minutes)}
            </span>
            <span
              aria-hidden
              style={{ width: 8, height: 8, borderRadius: 99, background: dot, flex: "none" }}
            />
            <div
              style={{
                flex: 1,
                display: "flex",
                flexDirection: "column",
                gap: 1,
                minWidth: 0,
                textAlign: "left",
              }}
            >
              <span
                style={{
                  fontSize: 14,
                  fontWeight: 600,
                  color: "var(--txt)",
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                }}
              >
                {booking.guest_name}
              </span>
              <span style={{ fontSize: 11, color: "var(--hint)" }}>
                {fmt.guests(booking.party_size)} ·{" "}
                {seated ? `стол ${booking.table_number}` : "стол не назначен"}
                {booking.source === "staff" ? " · вручную" : ""}
              </span>
            </div>
            <span style={{ fontSize: 11, fontWeight: 600, color: dot }}>
              {seated ? statusWord(booking.status) : "без стола"}
            </span>
          </button>
        );
      })}
    </div>
  );
}

export function ClosedDayCard() {
  return (
    <div style={{ margin: "0 12px 14px" }}>
      <Card padding={22} gap={6} style={{ alignItems: "center", textAlign: "center" }}>
        <span style={{ fontSize: 17, fontWeight: 700, color: "var(--txt)" }}>Выходной</span>
        <Note>
          В этот день бар закрыт, гостям он не предлагается. Часы работы меняются в настройках.
        </Note>
      </Card>
    </div>
  );
}

export function OrphanWarning({ count, onFindTables }: { count: number; onFindTables: () => void }) {
  return (
    <button
      type="button"
      onClick={onFindTables}
      style={{
        margin: "0 12px 10px",
        padding: "11px 13px",
        borderRadius: 12,
        background: "rgba(234,161,58,.16)",
        display: "flex",
        gap: 9,
        alignItems: "flex-start",
        width: "calc(100% - 24px)",
      }}
    >
      <span
        aria-hidden
        style={{ fontSize: 13, fontWeight: 700, color: "var(--warn)", lineHeight: 1.4 }}
      >
        !
      </span>
      <span
        style={{ fontSize: 12, color: "var(--warn)", lineHeight: 1.45, textWrap: "pretty" }}
      >
        {fmt.bookings(count)} без стола — нажмите, чтобы подобрать заново, или откройте карточку
        гостя.
      </span>
    </button>
  );
}

export function ShiftScreen({
  shift,
  today,
  view,
  onView,
  onServiceDate,
  onOpenBooking,
  onTapTable,
  onNewBooking,
  onFindTables,
}: {
  shift: ShiftView;
  today: string;
  view: "timeline" | "list";
  onView: (view: "timeline" | "list") => void;
  onServiceDate: (date: string) => void;
  onOpenBooking: (booking: ShiftBooking) => void;
  onTapTable: (table: ShiftTable) => void;
  onNewBooking: () => void;
  onFindTables: () => void;
}) {
  const orphans = shift.bookings.filter((booking) => booking.table_id === null).length;
  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      <DayNavigator serviceDate={shift.service_date} today={today} onChange={onServiceDate} />
      <ShiftStats shift={shift} />

      {shift.hours.closed ? (
        <ClosedDayCard />
      ) : (
        <>
          <div style={{ display: "flex", padding: "0 12px 10px", gap: 2 }}>
            <Segmented
              options={[
                { value: "timeline" as const, label: "Таймлайн" },
                { value: "list" as const, label: "Список" },
              ]}
              value={view}
              onChange={onView}
            />
          </div>

          {orphans > 0 ? <OrphanWarning count={orphans} onFindTables={onFindTables} /> : null}

          {view === "timeline" ? (
            <Timeline shift={shift} onOpenBooking={onOpenBooking} onTapTable={onTapTable} />
          ) : (
            <ShiftList shift={shift} onOpenBooking={onOpenBooking} />
          )}

          <div style={{ padding: "0 12px 20px" }}>
            <CardAction label="+ Бронь вручную" onClick={onNewBooking} />
          </div>
        </>
      )}
    </div>
  );
}
