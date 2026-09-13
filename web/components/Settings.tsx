"use client";

/**
 * The settings screen: an index of six rows, each pushing a section.
 *
 * The draft-and-save model underneath is unchanged and was always right — one proposal, edited
 * locally, saved or reverted as a whole. What was wrong was the shape: six groups of controls laid
 * out flat behind a row of tabs, so the answer to "what are the hours" was four taps and a scroll.
 * Now every row on the index shows its own current value, and the section is one tap away.
 *
 * Every control still asks the same question before it lets itself be pressed: would the bar still
 * be legal if this change were made? A control that looks live and then refuses is worse than one
 * that is plainly unavailable, and the notes underneath say *why* rather than leaving staff to
 * guess.
 */

import { useState, type ReactNode } from "react";

import type { Limits, SettingsDraft, SettingsView, ShiftView } from "@/lib/api";
import * as fmt from "@/lib/format";
import { uuid } from "@/lib/ids";
import { moveTableTo, removeStaff, removeTable, resizeTable } from "@/lib/settingsEdits";
import {
  largestTable,
  lastArrivalMinutes,
  roomFor,
  shortestShiftMinutes,
  wouldBeLegal,
  type Edit,
} from "@/lib/settingsRules";
import { RADIUS, SPACE, TAP, TEXT } from "@/lib/tokens";
import {
  Card,
  CardAction,
  Note,
  Pressable,
  SectionLabel,
  Segmented,
  Separator,
  Stepper,
  StepperButton,
  TextField,
} from "./ui";

/** Monday first, the way a week is read, over an array indexed from Sunday. */
const WEEK_ORDER = [1, 2, 3, 4, 5, 6, 0] as const;

export const SECTIONS = [
  { id: "bar", label: "Бар" },
  { id: "room", label: "Зал" },
  { id: "hours", label: "Часы работы" },
  { id: "rules", label: "Правила бронирования" },
  { id: "texts", label: "Сообщения и причины отмены" },
  { id: "staff", label: "Персонал" },
] as const;

export type Section = (typeof SECTIONS)[number]["id"];

/** Makes a change to the proposal. */
type Apply = (change: Edit) => void;
/** Asks whether a change would leave the bar legal. */
type Ask = (change: Edit) => boolean;

interface Context {
  draft: SettingsDraft;
  settings: SettingsView;
  /** Evening booking count per table id; null until read. */
  bookingsByTable: ReadonlyMap<string, number> | null;
  limits: Limits;
  editedWeekday: number;
  onEditWeekday: (weekday: number) => void;
  edit: Apply;
  allowed: Ask;
}

/** What each row of the index says it holds, so the value is readable without opening it. */
export function sectionValue(section: Section, draft: SettingsDraft, weekday: number): string {
  switch (section) {
    case "bar":
      return [draft.name, draft.address, draft.contact.trim()].filter(Boolean).join(" · ");
    case "room":
      return `${fmt.tables(draft.tables.length)} · ${draft.zones.join(", ")}`;
    case "hours": {
      const hours = draft.week[weekday];
      const name = fmt.weekdayShortByIndex(weekday);
      if (!hours) return name;
      return hours.closed ? `${name} выходной` : `${name} ${fmt.hoursLabel(hours)}`;
    }
    case "rules":
      return `бронь ${fmt.hours(draft.turn_minutes)} · до ${draft.max_party} гостей · шаг ${draft.slot_step_minutes} мин`;
    case "texts":
      return `${draft.message_templates.length} и ${draft.cancel_reasons.length} — персонал выбирает только из них`;
    case "staff":
      return draft.staff.map((member) => `@${member.username}`).join(" · ");
  }
}

function bookingsByTableOf(room: ShiftView): Map<string, number> {
  const counts = new Map<string, number>();
  for (const booking of room.bookings) {
    if (booking.table_id !== null) {
      counts.set(booking.table_id, (counts.get(booking.table_id) ?? 0) + 1);
    }
  }
  return counts;
}

export function SettingsScreen({
  settings,
  draft,
  room,
  editedWeekday,
  onEdit,
  onEditWeekday,
  section,
  onSection,
}: {
  settings: SettingsView;
  draft: SettingsDraft;
  /** Shown evening, source of per-table counts; null until read. */
  room: ShiftView | null;
  editedWeekday: number;
  onEdit: (change: Edit) => void;
  onEditWeekday: (weekday: number) => void;
  /**
   * Open section, or index. Page holds it so shift detour returns to section and Telegram back button
   * can close it.
   */
  section: Section | null;
  onSection: (section: Section | null) => void;
}) {
  const limits = settings.limits;

  // One edit shape for every section: change described once, then both asked about and applied.
  const context: Context = {
    draft,
    settings,
    bookingsByTable: room === null ? null : bookingsByTableOf(room),
    limits,
    editedWeekday,
    onEditWeekday,
    edit: onEdit,
    allowed: (change) => wouldBeLegal(draft, change, limits),
  };

  if (section === null) {
    return (
      <div
        style={{
          padding: `${SPACE[3]}px ${SPACE[4]}px ${SPACE[5]}px`,
          display: "flex",
          flexDirection: "column",
          gap: SPACE[2],
        }}
      >
        {SECTIONS.map((item) => (
          <Pressable
            key={item.id}
            onClick={() => onSection(item.id)}
            tone="card"
            style={{
              minHeight: 62,
              padding: `${SPACE[2] + 2}px ${SPACE[3] + 2}px`,
              borderRadius: RADIUS.md,
              background: "var(--sec)",
              justifyContent: "space-between",
              gap: SPACE[3],
            }}
          >
            <span
              style={{
                display: "flex",
                flexDirection: "column",
                gap: 2,
                minWidth: 0,
                textAlign: "left",
              }}
            >
              <span style={{ fontSize: TEXT.lg, fontWeight: 600, color: "var(--txt)" }}>
                {item.label}
              </span>
              <span
                style={{
                  fontSize: TEXT.sm,
                  color: "var(--hint)",
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                }}
              >
                {sectionValue(item.id, draft, editedWeekday)}
              </span>
            </span>
            <span aria-hidden style={{ flex: "none", fontSize: TEXT.xl, color: "var(--hint)" }}>
              ›
            </span>
          </Pressable>
        ))}
      </div>
    );
  }

  const current = SECTIONS.find((item) => item.id === section);
  return (
    <div
      style={{
        padding: `${SPACE[3]}px ${SPACE[4]}px ${SPACE[5]}px`,
        display: "flex",
        flexDirection: "column",
        gap: SPACE[4],
      }}
    >
      <Pressable
        ariaLabel="Назад"
        onClick={() => onSection(null)}
        style={{
          alignSelf: "flex-start",
          fontSize: TEXT.base,
          color: "var(--link)",
          fontWeight: 600,
          padding: `0 ${SPACE[2]}px 0 0`,
        }}
      >
        ‹ Настройки
      </Pressable>
      <span style={{ fontSize: TEXT.h2, fontWeight: 700, color: "var(--txt)" }}>
        {current?.label}
      </span>
      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[5] }}>
        {sectionBody(section, context)}
      </div>
    </div>
  );
}

function sectionBody(section: Section, ctx: Context): ReactNode {
  switch (section) {
    case "bar":
      return (
        <section style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
          <TextField
            value={ctx.draft.name}
            placeholder="Название"
            onChange={(value) =>
              ctx.edit((next) => {
                next.name = value;
              })
            }
          />
          <TextField
            value={ctx.draft.address}
            placeholder="Адрес"
            onChange={(value) =>
              ctx.edit((next) => {
                next.address = value;
              })
            }
          />
          <TextField
            value={ctx.draft.contact}
            placeholder="Телефон или @ник для гостей"
            onChange={(value) =>
              ctx.edit((next) => {
                next.contact = value;
              })
            }
          />
          <Note>
            Контакт видят гости, чья компания больше предела, и бот называет его в ответ на сообщения.
            Пусто — значит некуда.
          </Note>
          <Note>
            Часовой пояс — {ctx.draft.timezone}. Все времена в приложении показаны по нему.
          </Note>
        </section>
      );
    case "room":
      return <RoomSection ctx={ctx} />;
    case "hours":
      return <HoursSection ctx={ctx} />;
    case "rules":
      return <RulesSection ctx={ctx} />;
    case "texts":
      return (
        <>
          <ListSection
            title="Сообщения гостю"
            addLabel="+ Сообщение"
            items={ctx.draft.message_templates}
            canAdd={roomFor(ctx.draft, ctx.limits, "message_templates", 1)}
            placeholder="Новое сообщение"
            onChange={(items) =>
              ctx.edit((next) => {
                next.message_templates = items;
              })
            }
          />
          <ListSection
            title="Причины отмены"
            addLabel="+ Причина"
            items={ctx.draft.cancel_reasons}
            canAdd={roomFor(ctx.draft, ctx.limits, "cancel_reasons", 1)}
            placeholder="Новая причина"
            onChange={(items) =>
              ctx.edit((next) => {
                next.cancel_reasons = items;
              })
            }
          />
          <Note>Персонал выбирает только из этих списков — свободного текста гостю не уходит.</Note>
        </>
      );
    case "staff":
      return <StaffSection ctx={ctx} />;
  }
}

function HoursSection({ ctx }: { ctx: Context }) {
  const { draft, editedWeekday, onEditWeekday, edit, allowed } = ctx;
  const hours = draft.week[editedWeekday] ?? {
    open_minutes: 0,
    close_minutes: 0,
    closed: true,
  };
  const toggleClosed: Edit = (next) => {
    const day = next.week[editedWeekday];
    if (day) day.closed = !day.closed;
  };
  const canToggleClosed = allowed(toggleClosed);
  const shiftHour =
    (field: "open_minutes" | "close_minutes", delta: number): Edit =>
    (next) => {
      const day = next.week[editedWeekday];
      if (day) day[field] += delta;
    };
  const lastArrival = lastArrivalMinutes(draft, editedWeekday);

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
      <div style={{ display: "flex", gap: SPACE[1] + 1 }}>
        {WEEK_ORDER.map((weekday) => {
          const day = draft.week[weekday];
          const chosen = weekday === editedWeekday;
          return (
            <Pressable
              key={weekday}
              ariaPressed={chosen}
              ariaLabel={fmt.weekdayLongByIndex(weekday)}
              onClick={() => onEditWeekday(weekday)}
              style={{
                flex: 1,
                minHeight: 48,
                borderRadius: RADIUS.sm,
                background: chosen ? "var(--btn)" : "var(--chip)",
                color: chosen ? "var(--btn-text)" : day?.closed ? "var(--hint)" : "var(--txt)",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                gap: 3,
                fontSize: TEXT.md,
                fontWeight: 600,
              }}
            >
              <span>{fmt.weekdayShortByIndex(weekday)}</span>
              <span
                aria-hidden
                style={{
                  width: 4,
                  height: 4,
                  borderRadius: RADIUS.pill,
                  background: day?.closed ? "var(--dest)" : "transparent",
                }}
              />
            </Pressable>
          );
        })}
      </div>

      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: `${SPACE[2]}px ${SPACE[3] + 2}px`,
          background: "var(--sec)",
          borderRadius: RADIUS.md,
          gap: SPACE[2],
        }}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          <span style={{ fontSize: TEXT.lg, fontWeight: 600, color: "var(--txt)" }}>
            {fmt.weekdayLongByIndex(editedWeekday)}
          </span>
          <span
            style={{ fontSize: TEXT.sm, color: hours.closed ? "var(--dest)" : "var(--ok)" }}
          >
            {hours.closed ? "Выходной" : "Рабочий день"}
          </span>
        </div>
        <Pressable
          disabled={!canToggleClosed}
          onClick={() => edit(toggleClosed)}
          style={{
            fontSize: TEXT.md,
            fontWeight: 600,
            color: canToggleClosed ? "var(--link)" : "var(--hint)",
            padding: `0 ${SPACE[1]}px`,
            justifyContent: "flex-end",
          }}
        >
          {hours.closed ? "Сделать рабочим" : "Сделать выходным"}
        </Pressable>
      </div>

      {!hours.closed
        ? (
            [
              ["open_minutes", "Открытие"] as const,
              ["close_minutes", "Закрытие"] as const,
            ] as const
          ).map(([field, label]) => (
            <Stepper
              key={field}
              label={label}
              value={fmt.time(hours[field])}
              canDecrease={allowed(shiftHour(field, -60))}
              canIncrease={allowed(shiftHour(field, 60))}
              onDecrease={() => edit(shiftHour(field, -60))}
              onIncrease={() => edit(shiftHour(field, 60))}
            />
          ))
        : null}

      <Note>
        Последняя бронь — {lastArrival === null ? "—" : fmt.time(lastArrival)}: закрытие минус
        время стола.
      </Note>

      {!hours.closed ? (
        <CardAction
          label="Применить ко всем дням"
          onClick={() =>
            edit((next) => {
              const source = next.week[editedWeekday];
              if (!source) return;
              next.week = next.week.map(() => ({ ...source }));
            })
          }
        />
      ) : null}
    </section>
  );
}

function RoomSection({ ctx }: { ctx: Context }) {
  const { draft, settings, bookingsByTable, limits, edit, allowed } = ctx;
  const totalSeats = draft.tables.reduce((total, table) => total + table.seats, 0);
  const largest = largestTable(draft);
  /** The number a row that has only just been tapped into being will be given. */
  let provisional = settings.next_table_number;

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
      <Note>
        {fmt.tables(draft.tables.length)} · {fmt.seats(totalSeats)} · самый большой на {largest}
      </Note>

      {draft.tables.map((table) => {
        const existing = settings.tables.find((stored) => stored.id === table.id);
        const number = existing?.number ?? provisional++;
        const bookingsToday = bookingsByTable?.get(table.id) ?? 0;
        const zoneIndex = draft.zones.indexOf(table.zone);
        const nextZone =
          draft.zones[(zoneIndex + 1) % Math.max(1, draft.zones.length)] ?? table.zone;

        const resize = (delta: number) => resizeTable(table.id, delta);
        const remove = removeTable(table.id);
        const canRemove = allowed(remove);

        return (
          <div
            key={table.id}
            style={{
              display: "flex",
              alignItems: "center",
              gap: SPACE[2],
              padding: `${SPACE[1] + 2}px ${SPACE[1] + 2}px ${SPACE[1] + 2}px ${SPACE[3]}px`,
              background: "var(--sec)",
              borderRadius: RADIUS.md,
            }}
          >
            <Pressable
              ariaLabel={`Стол ${number}: сменить зону`}
              onClick={() => edit(moveTableTo(table.id, nextZone))}
              style={{
                flex: 1,
                minWidth: 0,
                flexDirection: "column",
                alignItems: "flex-start",
                justifyContent: "center",
                gap: 1,
              }}
            >
              <span style={{ fontSize: TEXT.base, fontWeight: 600, color: "var(--txt)" }}>
                Стол {number}
              </span>
              <span style={{ fontSize: TEXT.xs, color: "var(--link)" }}>{table.zone}</span>
            </Pressable>

            {bookingsToday > 0 ? (
              <span
                style={{
                  fontSize: TEXT.xs,
                  fontWeight: 600,
                  color: "var(--warn)",
                  whiteSpace: "nowrap",
                }}
              >
                {fmt.bookings(bookingsToday)}
              </span>
            ) : null}

            <div style={{ display: "flex", alignItems: "center", gap: 2 }}>
              <StepperButton
                sign="−"
                label={`Стол ${number}: меньше мест`}
                enabled={table.seats - 1 >= limits.seats.min && allowed(resize(-1))}
                onClick={() => edit(resize(-1))}
              />
              <span
                style={{
                  fontSize: TEXT.base,
                  fontWeight: 600,
                  color: "var(--txt)",
                  minWidth: 26,
                  textAlign: "center",
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {table.seats}
              </span>
              <StepperButton
                sign="+"
                label={`Стол ${number}: больше мест`}
                enabled={table.seats + 1 <= limits.seats.max && allowed(resize(1))}
                onClick={() => edit(resize(1))}
              />
            </div>

            <Pressable
              ariaLabel={`Убрать стол ${number}`}
              disabled={!canRemove}
              onClick={() => edit(remove)}
              style={{
                width: TAP,
                minHeight: TAP,
                borderRadius: RADIUS.sm,
                color: canRemove ? "var(--dest)" : "var(--hint)",
                justifyContent: "center",
                fontSize: TEXT.xl,
              }}
            >
              ×
            </Pressable>
          </div>
        );
      })}

      <CardAction
        label="+ Добавить стол"
        disabled={!roomFor(draft, limits, "tables", 1)}
        onClick={() => {
          // Id outside edit: edit made during save replays on top of stored save and must add same
          // table.
          const id = uuid();
          edit((next) => {
            next.tables.push({ id, seats: 4, zone: next.zones[0] ?? "Зал" });
          });
        }}
      />

      <Note>
        Название стола меняет зону. Убранный стол уходит из подбора, но остаётся в истории смен, и
        его номер больше никому не достанется.
      </Note>
    </section>
  );
}

function RulesSection({ ctx }: { ctx: Context }) {
  const { draft, limits, editedWeekday, edit, allowed } = ctx;
  const rules: {
    field: "turn_minutes" | "max_party" | "grace_minutes" | "horizon_days" | "remind_hours";
    label: string;
    step: number;
    display: string;
  }[] = [
    {
      field: "turn_minutes",
      label: "Бронь занимает",
      step: 30,
      display: fmt.hours(draft.turn_minutes),
    },
    { field: "max_party", label: "Компания до", step: 1, display: `${draft.max_party} чел.` },
    {
      field: "grace_minutes",
      label: "Ждём опоздавших",
      step: 5,
      display: `${draft.grace_minutes} мин`,
    },
    { field: "horizon_days", label: "Бронь вперёд", step: 1, display: `${draft.horizon_days} дн.` },
    {
      field: "remind_hours",
      label: "Напомнить за",
      step: 1,
      display: fmt.hours(draft.remind_hours * 60),
    },
  ];

  // The day being edited, not whatever day it happens to be on the device reading this: the note
  // explains the hours in the section above it.
  const lastArrival = lastArrivalMinutes(draft, editedWeekday);
  const shortest = shortestShiftMinutes(draft);
  const turnBlocked =
    draft.turn_minutes + 30 <= limits.turn_minutes.max &&
    !allowed((next) => {
      next.turn_minutes += 30;
    });

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
      {rules.map((rule) => {
        const by =
          (delta: number): Edit =>
          (next) => {
            next[rule.field] += delta;
          };
        return (
          <Stepper
            key={rule.field}
            label={rule.label}
            value={rule.display}
            canDecrease={allowed(by(-rule.step))}
            canIncrease={allowed(by(rule.step))}
            onDecrease={() => edit(by(-rule.step))}
            onIncrease={() => edit(by(rule.step))}
          />
        );
      })}

      <Card padding={SPACE[3] + 2} gap={SPACE[2] + 1} style={{ background: "var(--sec)" }}>
        <span style={{ fontSize: TEXT.lg, color: "var(--txt)" }}>Шаг времени</span>
        <Segmented
          background="var(--bg)"
          label="Шаг времени"
          options={limits.slot_step_minutes.map((step) => ({
            value: step,
            label: `${step} мин`,
          }))}
          value={draft.slot_step_minutes}
          onChange={(step) =>
            edit((next) => {
              next.slot_step_minutes = step;
            })
          }
        />
      </Card>

      <Note>
        Последняя бронь — {lastArrival === null ? "—" : fmt.time(lastArrival)}: закрытие минус
        время стола.
      </Note>

      {turnBlocked && shortest !== null ? (
        <Note tone="warn">Дольше нельзя: самая короткая смена {fmt.hours(shortest)}.</Note>
      ) : null}
    </section>
  );
}

function ListSection({
  title,
  addLabel,
  items,
  canAdd,
  placeholder,
  onChange,
}: {
  title: string;
  addLabel: string;
  items: string[];
  canAdd: boolean;
  placeholder: string;
  onChange: (items: string[]) => void;
}) {
  return (
    <section style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
      <SectionLabel>{title}</SectionLabel>
      {items.map((text, index) => (
        <div key={index} style={{ display: "flex", alignItems: "center", gap: SPACE[1] + 2 }}>
          <TextField
            value={text}
            placeholder={placeholder}
            ariaLabel={`${title}: ${index + 1}`}
            onChange={(value) => {
              const next = [...items];
              next[index] = value;
              onChange(next);
            }}
            style={{ flex: 1, minWidth: 0, fontSize: TEXT.base }}
          />
          <Pressable
            ariaLabel="Убрать"
            disabled={items.length <= 1}
            onClick={() => onChange(items.filter((_, position) => position !== index))}
            style={{
              width: TAP,
              minHeight: TAP,
              borderRadius: RADIUS.sm,
              color: items.length > 1 ? "var(--dest)" : "var(--hint)",
              justifyContent: "center",
              fontSize: TEXT.xl,
            }}
          >
            ×
          </Pressable>
        </div>
      ))}
      <CardAction label={addLabel} disabled={!canAdd} onClick={() => onChange([...items, ""])} />
    </section>
  );
}

/**
 * Username plus occurrence among same-spelled rows: roster may hold duplicate until save refuses, and
 * index alone would hand removed row's focus to next member.
 */
function staffRowKey(staff: { username: string }[], index: number): string {
  const username = staff[index]?.username ?? "";
  const before = staff.slice(0, index).filter((other) => other.username === username).length;
  return `${username}#${before}`;
}

function StaffSection({ ctx }: { ctx: Context }) {
  const { draft, edit } = ctx;
  return (
    <section style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
      {draft.staff.map((member, index) => (
        <div
          key={staffRowKey(draft.staff, index)}
          style={{
            display: "flex",
            alignItems: "center",
            gap: SPACE[1] + 2,
            padding: `0 0 0 ${SPACE[3] + 2}px`,
            background: "var(--sec)",
            borderRadius: RADIUS.md,
          }}
        >
          <span style={{ flex: 1, fontSize: TEXT.lg, color: "var(--txt)" }}>
            @{member.username}
          </span>
          <Pressable
            ariaLabel={`Убрать @${member.username}`}
            disabled={draft.staff.length <= 1}
            onClick={() => edit(removeStaff(member.username))}
            style={{
              width: TAP,
              minHeight: 48,
              borderRadius: RADIUS.sm,
              color: draft.staff.length > 1 ? "var(--dest)" : "var(--hint)",
              justifyContent: "center",
              fontSize: TEXT.xl,
            }}
          >
            ×
          </Pressable>
        </div>
      ))}
      <AddStaff
        canAdd={roomFor(draft, ctx.limits, "staff", 1)}
        onAdd={(username) =>
          edit((next) => {
            next.staff.push({ username });
          })
        }
      />
      <Note>
        Доступ привязывается к аккаунту при первом входе. Последнего убрать нельзя — иначе никто не
        войдёт.
      </Note>
    </section>
  );
}

function AddStaff({
  canAdd,
  onAdd,
}: {
  canAdd: boolean;
  onAdd: (username: string) => void;
}) {
  // Half-typed text is kept here rather than in the proposal: a username being spelled out is not
  // yet a change to the roster, and the Save button must not light up because somebody pressed a key.
  const [pending, setPending] = useState("");
  const cleaned = pending.trim().replace(/^@/, "");
  const ready = canAdd && cleaned.length > 0;
  return (
    <div style={{ display: "flex", alignItems: "center", gap: SPACE[1] + 2 }}>
      <TextField
        value={pending}
        placeholder="@username"
        onChange={setPending}
        style={{ flex: 1, minWidth: 0 }}
      />
      <Pressable
        disabled={!ready}
        onClick={() => {
          onAdd(cleaned);
          setPending("");
        }}
        style={{
          padding: `0 ${SPACE[4] + 2}px`,
          minHeight: TAP,
          borderRadius: RADIUS.md,
          background: ready ? "var(--btn)" : "var(--chip)",
          color: ready ? "var(--btn-text)" : "var(--hint)",
          fontSize: TEXT.base,
          fontWeight: 600,
          justifyContent: "center",
        }}
      >
        Добавить
      </Pressable>
    </div>
  );
}

/**
 * The bar that appears when the draft differs from what is saved.
 *
 * Only then: a save button that is always there and usually inert teaches people to ignore it. When
 * the draft is illegal the button is inert and the line beside it names the first reason, so the
 * refusal arrives before the request rather than after it. Server refusal stays here until next edit
 * or save, reasons one tap away, whatever sheet came and went.
 */
export function SaveBar({
  reason,
  saving,
  onSave,
  onRevert,
  onWhy,
}: {
  reason: string | null;
  saving: boolean;
  onSave: () => void;
  onRevert: () => void;
  /** Opens last save refusal; null when none. */
  onWhy: (() => void) | null;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] }}>
      {reason ? <Note tone="dest">Так сохранить нельзя. {reason}</Note> : null}
      {onWhy ? (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: SPACE[2],
          }}
        >
          <Note tone="dest">Не сохранено.</Note>
          <Pressable
            onClick={onWhy}
            style={{
              fontSize: TEXT.base,
              fontWeight: 600,
              color: "var(--link)",
              padding: `0 ${SPACE[1]}px`,
            }}
          >
            Почему
          </Pressable>
        </div>
      ) : null}
      <div style={{ display: "flex", gap: SPACE[2] }}>
        <Pressable
          onClick={onRevert}
          style={{
            padding: `0 ${SPACE[4] + 2}px`,
            minHeight: 50,
            borderRadius: RADIUS.md,
            background: "var(--sec)",
            color: "var(--hint)",
            fontSize: TEXT.lg,
            fontWeight: 600,
            justifyContent: "center",
          }}
        >
          Вернуть
        </Pressable>
        <div style={{ flex: 1 }}>
          <Pressable
            disabled={reason !== null || saving}
            onClick={onSave}
            tone="card"
            style={{
              width: "100%",
              minHeight: 50,
              borderRadius: RADIUS.md,
              background: reason === null ? "var(--btn)" : "var(--chip)",
              color: reason === null ? "var(--btn-text)" : "var(--hint)",
              fontSize: 16,
              fontWeight: 600,
              justifyContent: "center",
            }}
          >
            {saving ? "Сохраняем…" : "Сохранить"}
          </Pressable>
        </div>
      </div>
    </div>
  );
}

