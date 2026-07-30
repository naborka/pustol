"use client";

/**
 * The settings screen.
 *
 * Every control asks the same question before it lets itself be pressed: would the bar still be
 * legal if this change were made? A control that looks live and then refuses is worse than one that
 * is plainly unavailable, and the notes underneath say *why* rather than leaving staff to guess.
 */

import { useState } from "react";

import { draftOf, type Limits, type SettingsDraft, type SettingsView, type TableDraft } from "@/lib/api";
import * as fmt from "@/lib/format";
import {
  copyDraft,
  differs,
  lastArrivalMinutes,
  largestTable,
  shortestShiftMinutes,
  wouldBeLegal,
  type Edit,
} from "@/lib/settingsRules";
import {
  CardAction,
  Note,
  RADIUS,
  SectionLabel,
  Segmented,
  Stepper,
  StepperButton,
  TextField,
} from "./ui";

/** Monday first, the way a week is read, over an array indexed from Sunday. */
const WEEK_ORDER = [1, 2, 3, 4, 5, 6, 0] as const;

/** Makes a change to the proposal. */
type Apply = (change: Edit) => void;
/** Asks whether a change would leave the bar legal. */
type Ask = (change: Edit) => boolean;

interface Props {
  settings: SettingsView;
  draft: SettingsDraft;
  editedWeekday: number;
  onDraft: (next: SettingsDraft) => void;
  onEditWeekday: (weekday: number) => void;
  onSave: () => void;
  onRevert: () => void;
  saving: boolean;
}

export function SettingsScreen({
  settings,
  draft,
  editedWeekday,
  onDraft,
  onEditWeekday,
  onSave,
  onRevert,
  saving,
}: Props) {
  const limits = settings.limits;
  const dirty = differs(draft, draftOf(settings));
  // One notion of "an edit", built here and handed to every section: a change is described once and
  // then both asked about and made, rather than written out twice in two shapes.
  const edit: Apply = (change) => {
    const next = copyDraft(draft);
    change(next);
    onDraft(next);
  };
  const allowed: Ask = (change) => wouldBeLegal(draft, change, limits);

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
  /** Moving one end of the shift by an hour. */
  const shiftHour =
    (field: "open_minutes" | "close_minutes", delta: number): Edit =>
    (next) => {
      const day = next.week[editedWeekday];
      if (day) day[field] += delta;
    };

  return (
    <div
      style={{ padding: "14px 16px 24px", display: "flex", flexDirection: "column", gap: 22 }}
    >
      <section style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <SectionLabel>Бар</SectionLabel>
        <TextField
          value={draft.name}
          placeholder="Название"
          onChange={(value) => edit((next) => {
            next.name = value;
          })}
        />
        <TextField
          value={draft.address}
          placeholder="Адрес"
          onChange={(value) => edit((next) => {
            next.address = value;
          })}
        />
      </section>

      <TablesSection
        draft={draft}
        settings={settings}
        limits={limits}
        edit={edit}
        allowed={allowed}
      />

      <section style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <SectionLabel>Часы работы</SectionLabel>
        <div style={{ display: "flex", gap: 5 }}>
          {WEEK_ORDER.map((weekday) => {
            const day = draft.week[weekday];
            const chosen = weekday === editedWeekday;
            return (
              <button
                key={weekday}
                type="button"
                aria-pressed={chosen}
                onClick={() => onEditWeekday(weekday)}
                style={{
                  flex: 1,
                  height: 48,
                  borderRadius: RADIUS.small,
                  background: chosen ? "var(--btn)" : "var(--chip)",
                  color: chosen
                    ? "var(--btn-text)"
                    : day?.closed
                      ? "var(--hint)"
                      : "var(--txt)",
                  display: "flex",
                  flexDirection: "column",
                  alignItems: "center",
                  justifyContent: "center",
                  gap: 3,
                  fontSize: 13,
                  fontWeight: 600,
                }}
              >
                <span>{fmt.weekdayShortByIndex(weekday)}</span>
                <span
                  aria-hidden
                  style={{
                    width: 4,
                    height: 4,
                    borderRadius: 99,
                    background: day?.closed ? "var(--dest)" : "transparent",
                  }}
                />
              </button>
            );
          })}
        </div>

        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "12px 14px",
            background: "var(--sec)",
            borderRadius: RADIUS.chip,
          }}
        >
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span style={{ fontSize: 15, fontWeight: 600, color: "var(--txt)" }}>
              {fmt.weekdayLongByIndex(editedWeekday)}
            </span>
            <span
              style={{ fontSize: 12, color: hours.closed ? "var(--dest)" : "var(--ok)" }}
            >
              {hours.closed ? "Выходной" : "Рабочий день"}
            </span>
          </div>
          <button
            type="button"
            disabled={!canToggleClosed}
            onClick={() => edit(toggleClosed)}
            style={{
              fontSize: 13,
              fontWeight: 600,
              color: canToggleClosed ? "var(--link)" : "var(--hint)",
              padding: "12px 4px",
            }}
          >
            {hours.closed ? "Сделать рабочим" : "Сделать выходным"}
          </button>
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

      <RulesSection
        draft={draft}
        limits={limits}
        editedWeekday={editedWeekday}
        edit={edit}
        allowed={allowed}
      />

      <ListSection
        title="Сообщения гостю"
        addLabel="+ Сообщение"
        items={draft.message_templates}
        placeholder="Новое сообщение"
        onChange={(items) => edit((next) => {
          next.message_templates = items;
        })}
      />

      <ListSection
        title="Причины отмены"
        addLabel="+ Причина"
        items={draft.cancel_reasons}
        placeholder="Новая причина"
        onChange={(items) => edit((next) => {
          next.cancel_reasons = items;
        })}
      />

      <StaffSection draft={draft} edit={edit} />

      <div style={{ display: "flex", gap: 8 }}>
        <button
          type="button"
          disabled={!dirty || saving}
          onClick={onSave}
          style={{
            flex: 1,
            padding: 15,
            borderRadius: RADIUS.chip,
            background: dirty ? "var(--btn)" : "var(--chip)",
            color: dirty ? "var(--btn-text)" : "var(--hint)",
            fontSize: 16,
            fontWeight: 600,
            textAlign: "center",
          }}
        >
          {saving ? "Сохраняем…" : dirty ? "Сохранить" : "Всё сохранено"}
        </button>
        {dirty ? (
          <button
            type="button"
            onClick={onRevert}
            style={{
              padding: "15px 18px",
              borderRadius: RADIUS.chip,
              background: "var(--sec)",
              color: "var(--hint)",
              fontSize: 15,
              fontWeight: 600,
            }}
          >
            Отмена
          </button>
        ) : null}
      </div>
    </div>
  );
}

function TablesSection({
  draft,
  settings,
  limits,
  edit,
  allowed,
}: {
  draft: SettingsDraft;
  settings: SettingsView;
  limits: Limits;
  edit: Apply;
  allowed: Ask;
}) {
  const totalSeats = draft.tables.reduce((total, table) => total + table.seats, 0);
  const largest = largestTable(draft);
  /** The number a row that has only just been tapped into being will be given. */
  let provisional = settings.next_table_number;

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
        <SectionLabel>Столы</SectionLabel>
        <span style={{ fontSize: 12, color: "var(--hint)" }}>
          {fmt.tables(draft.tables.length)} · {fmt.seats(totalSeats)} · самый большой на {largest}
        </span>
      </div>

      {draft.tables.map((table, index) => {
        const existing =
          table.kind === "existing"
            ? settings.tables.find((stored) => stored.id === table.id)
            : undefined;
        const number = existing?.number ?? provisional++;
        const bookingsToday = existing?.bookings_today ?? 0;
        const zoneIndex = draft.zones.indexOf(table.zone);
        const nextZone = draft.zones[(zoneIndex + 1) % Math.max(1, draft.zones.length)] ?? table.zone;

        const resize =
          (delta: number): Edit =>
          (next) => {
            const target = next.tables[index];
            if (target) target.seats += delta;
          };
        const remove: Edit = (next) => {
          next.tables.splice(index, 1);
        };
        const canRemove = allowed(remove);

        return (
          <div
            key={table.kind === "existing" ? table.id : `new-${index}`}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "6px 6px 6px 12px",
              background: "var(--sec)",
              borderRadius: RADIUS.chip,
            }}
          >
            <button
              type="button"
              onClick={() =>
                edit((next) => {
                  const target = next.tables[index];
                  if (target) target.zone = nextZone;
                })
              }
              style={{
                flex: 1,
                minWidth: 0,
                height: 44,
                display: "flex",
                flexDirection: "column",
                justifyContent: "center",
                gap: 1,
              }}
            >
              <span style={{ fontSize: 14, fontWeight: 600, color: "var(--txt)" }}>
                Стол {number}
              </span>
              <span style={{ fontSize: 11, color: "var(--link)" }}>{table.zone}</span>
            </button>

            {bookingsToday > 0 ? (
              <span
                style={{
                  fontSize: 10,
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
                  fontSize: 14,
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

            <button
              type="button"
              aria-label={`Убрать стол ${number}`}
              disabled={!canRemove}
              onClick={() => edit(remove)}
              style={{
                width: 38,
                height: 44,
                borderRadius: RADIUS.small,
                color: canRemove ? "var(--dest)" : "var(--hint)",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                fontSize: 17,
              }}
            >
              ×
            </button>
          </div>
        );
      })}

      <CardAction
        label="+ Добавить стол"
        onClick={() =>
          edit((next) => {
            const zone = next.zones[0] ?? "Зал";
            const added: TableDraft = { kind: "new", seats: 4, zone };
            next.tables.push(added);
          })
        }
      />

      <Note>
        Нажмите на название стола, чтобы сменить зону. Номера не переиспользуются: удалили
        седьмой — восьмой остаётся восьмым, иначе брони начнут ссылаться на чужие столы.
      </Note>

      {draft.max_party >= largest ? (
        <Note tone="warn">
          Компанию больше {largest} принять некуда: это самый большой стол. Лимит брони не может
          быть выше.
        </Note>
      ) : null}
    </section>
  );
}

function RulesSection({
  draft,
  limits,
  editedWeekday,
  edit,
  allowed,
}: {
  draft: SettingsDraft;
  limits: Limits;
  editedWeekday: number;
  edit: Apply;
  allowed: Ask;
}) {
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
    {
      field: "max_party",
      label: "Компания до",
      step: 1,
      display: `${draft.max_party} чел.`,
    },
    {
      field: "grace_minutes",
      label: "Ждём опоздавших",
      step: 5,
      display: `${draft.grace_minutes} мин`,
    },
    {
      field: "horizon_days",
      label: "Бронь вперёд",
      step: 1,
      display: `${draft.horizon_days} дн.`,
    },
    {
      field: "remind_hours",
      label: "Напомнить за",
      step: 1,
      display: fmt.hours(draft.remind_hours * 60),
    },
  ];

  // The day being edited, not whatever day it happens to be on the device reading this: the note
  // explains the hours in the panel above it.
  const lastArrival = lastArrivalMinutes(draft, editedWeekday);
  const shortest = shortestShiftMinutes(draft);
  const turnBlocked =
    draft.turn_minutes + 30 <= limits.turn_minutes.max &&
    !allowed((next) => {
      next.turn_minutes += 30;
    });

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <SectionLabel>Правила брони</SectionLabel>

      {rules.map((rule) => {
        const by = (delta: number): Edit => (next) => {
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

      <div
        style={{
          padding: "12px 14px",
          background: "var(--sec)",
          borderRadius: RADIUS.chip,
          display: "flex",
          flexDirection: "column",
          gap: 9,
        }}
      >
        <span style={{ fontSize: 15, color: "var(--txt)" }}>Шаг времени</span>
        <Segmented
          background="var(--bg)"
          options={limits.slot_step_minutes.map((step) => ({
            value: step,
            label: `${step} мин`,
          }))}
          value={draft.slot_step_minutes}
          onChange={(step) => edit((next) => {
            next.slot_step_minutes = step;
          })}
        />
      </div>

      <Note>
        Последняя бронь — в {lastArrival === null ? "—" : fmt.time(lastArrival)}: это закрытие
        минус время, на которое держим стол. Отдельно не настраивается, чтобы значения не
        разошлись.
      </Note>

      {turnBlocked && shortest !== null ? (
        <Note tone="warn">
          При этих часах держать стол дольше нельзя — самая короткая смена всего{" "}
          {fmt.hours(shortest)}.
        </Note>
      ) : null}
    </section>
  );
}

function ListSection({
  title,
  addLabel,
  items,
  placeholder,
  onChange,
}: {
  title: string;
  addLabel: string;
  items: string[];
  placeholder: string;
  onChange: (items: string[]) => void;
}) {
  return (
    <section style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <SectionLabel>{title}</SectionLabel>
      {items.map((text, index) => (
        <div key={index} style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <TextField
            value={text}
            ariaLabel={`${title}: ${index + 1}`}
            onChange={(value) => {
              const next = [...items];
              next[index] = value;
              onChange(next);
            }}
            style={{ flex: 1, minWidth: 0, fontSize: 14 }}
          />
          <button
            type="button"
            aria-label="Убрать"
            disabled={items.length <= 1}
            onClick={() => onChange(items.filter((_, position) => position !== index))}
            style={{
              width: 38,
              height: 44,
              borderRadius: RADIUS.small,
              color: items.length > 1 ? "var(--dest)" : "var(--hint)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              fontSize: 17,
            }}
          >
            ×
          </button>
        </div>
      ))}
      <CardAction label={addLabel} onClick={() => onChange([...items, placeholder])} />
    </section>
  );
}

function StaffSection({ draft, edit }: { draft: SettingsDraft; edit: Apply }) {
  return (
    <section style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <SectionLabel>Кто работает в админке</SectionLabel>
      {draft.staff.map((member, index) => (
        <div
          key={member.username}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            padding: "0 0 0 14px",
            background: "var(--sec)",
            borderRadius: RADIUS.chip,
          }}
        >
          <span style={{ flex: 1, fontSize: 15, color: "var(--txt)" }}>@{member.username}</span>
          <button
            type="button"
            aria-label={`Убрать @${member.username}`}
            disabled={draft.staff.length <= 1}
            onClick={() =>
              edit((next) => {
                next.staff.splice(index, 1);
              })
            }
            style={{
              width: 44,
              height: 48,
              borderRadius: RADIUS.small,
              color: draft.staff.length > 1 ? "var(--dest)" : "var(--hint)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              fontSize: 17,
            }}
          >
            ×
          </button>
        </div>
      ))}
      <AddStaff
        onAdd={(username) =>
          edit((next) => {
            next.staff.push({ username });
          })
        }
      />
      <Note>
        Любой из списка может менять эти настройки и добавлять других. Последнего убрать нельзя —
        иначе в админку не войдёт никто.
      </Note>
    </section>
  );
}

function AddStaff({ onAdd }: { onAdd: (username: string) => void }) {
  // Half-typed text is kept here rather than in the proposal: a username being spelled out is not
  // yet a change to the roster, and the Save button must not light up because somebody pressed a key.
  const [pending, setPending] = useState("");
  const cleaned = pending.trim().replace(/^@/, "");
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
      <TextField
        value={pending}
        placeholder="@username"
        onChange={setPending}
        style={{ flex: 1, minWidth: 0 }}
      />
      <button
        type="button"
        disabled={cleaned.length === 0}
        onClick={() => {
          onAdd(cleaned);
          setPending("");
        }}
        style={{
          padding: "13px 18px",
          borderRadius: RADIUS.chip,
          background: cleaned.length > 0 ? "var(--btn)" : "var(--chip)",
          color: cleaned.length > 0 ? "var(--btn-text)" : "var(--hint)",
          fontSize: 14,
          fontWeight: 600,
        }}
      >
        Добавить
      </button>
    </div>
  );
}
