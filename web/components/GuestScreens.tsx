"use client";

/**
 * What a guest sees: their booking, the picker, and the confirmation.
 *
 * Nothing here mentions a table. The bar assigns tables and moves them when the room changes, and a
 * number on a guest's screen becomes a number they arrive quoting — so the guest is told when they
 * are expected and how many of them, which is all they need and all that stays true.
 */

import type { Availability, BarView, GuestBooking, Session } from "@/lib/api";
import * as fmt from "@/lib/format";
import {
  Card,
  DaytimeDisclosure,
  InfoRow,
  Note,
  PartySizeRow,
  SectionLabel,
  Separator,
  SlotGrid,
} from "./ui";

/** The dot and the words that say a booking is real. */
function ConfirmedBadge() {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <span
        aria-hidden
        style={{ width: 7, height: 7, borderRadius: 99, background: "var(--ok)" }}
      />
      <span
        style={{
          fontSize: 12,
          fontWeight: 600,
          letterSpacing: ".06em",
          textTransform: "uppercase",
          color: "var(--ok)",
        }}
      >
        Бронь подтверждена
      </span>
    </div>
  );
}

export function BookingCard({
  booking,
  bar,
  onCancel,
}: {
  booking: GuestBooking;
  bar: BarView;
  onCancel: () => void;
}) {
  return (
    <Card padding={18} gap={14}>
      <ConfirmedBadge />
      <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
        <span
          style={{
            fontSize: 30,
            fontWeight: 700,
            color: "var(--txt)",
            letterSpacing: "-.02em",
          }}
        >
          {fmt.whenLabel(booking.service_date, bar.today, booking.start_minutes)}
        </span>
        <span style={{ fontSize: 15, color: "var(--hint)" }}>{fmt.guests(booking.party_size)}</span>
      </div>
      <Separator />
      <span style={{ fontSize: 13, color: "var(--hint)", lineHeight: 1.45, textWrap: "pretty" }}>
        Стол держим {fmt.minutesWord(bar.grace_minutes)} после времени брони. Опаздываете —
        напишите нам в чат.
      </span>
      <button
        type="button"
        onClick={onCancel}
        style={{
          padding: 12,
          borderRadius: 10,
          background: "var(--tint)",
          color: "var(--dest)",
          fontSize: 15,
          fontWeight: 600,
          textAlign: "center",
        }}
      >
        Отменить бронь
      </button>
    </Card>
  );
}

export function ReminderBanner({
  bar,
  onEnable,
  onDismiss,
}: {
  bar: BarView;
  onEnable: () => void;
  onDismiss: () => void;
}) {
  return (
    <Card>
      <span style={{ fontSize: 15, fontWeight: 600, color: "var(--txt)" }}>
        Напомнить о брони?
      </span>
      <span style={{ fontSize: 13, color: "var(--hint)", lineHeight: 1.45, textWrap: "pretty" }}>
        Пришлём напоминание за {fmt.hoursWord(bar.remind_hours)}. Если планы изменятся — отмените
        одной кнопкой прямо из чата.
      </span>
      <div style={{ display: "flex", gap: 8 }}>
        <button
          type="button"
          onClick={onEnable}
          style={{
            flex: 1,
            padding: 11,
            borderRadius: 10,
            background: "var(--btn)",
            color: "var(--btn-text)",
            fontSize: 14,
            fontWeight: 600,
            textAlign: "center",
          }}
        >
          Включить
        </button>
        <button
          type="button"
          onClick={onDismiss}
          style={{
            padding: "11px 16px",
            borderRadius: 10,
            color: "var(--hint)",
            fontSize: 14,
            fontWeight: 500,
          }}
        >
          Не сейчас
        </button>
      </div>
    </Card>
  );
}

function EmptyState() {
  return (
    <Card padding={26} gap={8} style={{ alignItems: "center", textAlign: "center" }}>
      <span
        style={{ fontSize: 20, fontWeight: 700, color: "var(--txt)", letterSpacing: "-.01em" }}
      >
        Столик на вечер
      </span>
      <span
        style={{
          fontSize: 14,
          color: "var(--hint)",
          lineHeight: 1.45,
          maxWidth: 250,
          textWrap: "pretty",
        }}
      >
        Три касания — и стол ваш. Подтверждение сразу, без звонков.
      </span>
    </Card>
  );
}

export function HomeScreen({
  session,
  onCancel,
  onEnableReminders,
  onDismissReminders,
  onWriteToBar,
}: {
  session: Session;
  onCancel: () => void;
  onEnableReminders: () => void;
  onDismissReminders: () => void;
  onWriteToBar: () => void;
}) {
  const { bar, booking } = session;
  return (
    <div
      style={{
        padding: "16px 16px 24px",
        display: "flex",
        flexDirection: "column",
        gap: 14,
      }}
    >
      {booking ? (
        <>
          <BookingCard booking={booking} bar={bar} onCancel={onCancel} />
          {session.reminders.should_ask ? (
            <ReminderBanner
              bar={bar}
              onEnable={onEnableReminders}
              onDismiss={onDismissReminders}
            />
          ) : null}
        </>
      ) : (
        <EmptyState />
      )}

      <Card>
        <InfoRow label="Сегодня открыт" value={fmt.hoursLabel(bar.today_hours)} />
        <Separator />
        <InfoRow label="Адрес" value={bar.address} />
        <Separator />
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "baseline",
            gap: 12,
          }}
        >
          <span style={{ fontSize: 14, color: "var(--hint)" }}>
            Компания больше {bar.max_party}
          </span>
          <button
            type="button"
            onClick={onWriteToBar}
            style={{ fontSize: 14, fontWeight: 600, color: "var(--link)" }}
          >
            Написать бару
          </button>
        </div>
      </Card>
    </div>
  );
}

export function BookScreen({
  bar,
  bookableDays,
  availability,
  partySize,
  serviceDate,
  chosenMinutes,
  daytimeShown,
  onPartySize,
  onServiceDate,
  onPick,
  onShowDaytime,
  onBack,
}: {
  bar: BarView;
  bookableDays: string[];
  availability: Availability | null;
  partySize: number;
  serviceDate: string;
  chosenMinutes: number | null;
  daytimeShown: boolean;
  onPartySize: (size: number) => void;
  onServiceDate: (date: string) => void;
  onPick: (minutes: number) => void;
  onShowDaytime: () => void;
  onBack?: () => void;
}) {
  const slots = availability?.slots ?? [];
  const daytime = slots.filter((slot) => !slot.evening);
  const evening = slots.filter((slot) => slot.evening);
  return (
    <div
      style={{ padding: "14px 16px 20px", display: "flex", flexDirection: "column", gap: 22 }}
    >
      {onBack ? (
        <button
          type="button"
          aria-label="Назад"
          onClick={onBack}
          style={{
            fontSize: 14,
            color: "var(--link)",
            fontWeight: 500,
            padding: "4px 0 0",
            alignSelf: "flex-start",
          }}
        >
          ‹ Назад
        </button>
      ) : null}
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <SectionLabel>Сколько гостей</SectionLabel>
        <PartySizeRow max={bar.max_party} value={partySize} onChange={onPartySize} />
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <SectionLabel>Когда</SectionLabel>
        <div style={{ display: "flex", gap: 8, overflowX: "auto", paddingBottom: 2 }}>
          {bookableDays.map((day) => {
            const chosen = day === serviceDate;
            return (
              <button
                key={day}
                type="button"
                aria-pressed={chosen}
                onClick={() => onServiceDate(day)}
                style={{
                  flex: "none",
                  minWidth: 80,
                  padding: "11px 8px",
                  borderRadius: 12,
                  background: chosen ? "var(--btn)" : "var(--chip)",
                  color: chosen ? "var(--btn-text)" : "var(--txt)",
                  display: "flex",
                  flexDirection: "column",
                  alignItems: "center",
                  gap: 2,
                }}
              >
                <span style={{ fontSize: 14, fontWeight: 600 }}>
                  {fmt.dayName(day, bar.today)}
                </span>
                <span style={{ fontSize: 11, opacity: 0.7 }}>{fmt.dayDate(day)}</span>
              </button>
            );
          })}
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div
          style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline" }}
        >
          <SectionLabel>Во сколько</SectionLabel>
          <span style={{ fontSize: 12, color: "var(--hint)" }}>
            стол на {fmt.hours(availability?.turn_minutes ?? bar.turn_minutes)}
          </span>
        </div>

        {daytime.length > 0 && !daytimeShown ? (
          <DaytimeDisclosure
            from={daytime[0]?.start_minutes ?? 0}
            to={daytime[daytime.length - 1]?.start_minutes ?? 0}
            onShow={onShowDaytime}
          />
        ) : null}

        {daytime.length > 0 && daytimeShown ? (
          <>
            <SlotGrid slots={daytime} chosen={chosenMinutes} onPick={onPick} />
            <Separator />
          </>
        ) : null}

        <SlotGrid slots={evening} chosen={chosenMinutes} onPick={onPick} />

        <Note>
          {availability === null
            ? "Считаем свободные окна…"
            : availability.free_count === 0
              ? "На этот день нет ни одного окна для такой компании. Попробуйте другой день."
              : `Свободно окон: ${availability.free_count}. Показываем только то, куда действительно сможем посадить ${fmt.guests(partySize)}.`}
        </Note>
      </div>
    </div>
  );
}

export function DoneScreen({
  booking,
  session,
  onEnableReminders,
}: {
  booking: GuestBooking;
  session: Session;
  onEnableReminders: () => void;
}) {
  const { bar } = session;
  return (
    <div
      style={{
        padding: "52px 22px",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 10,
        textAlign: "center",
      }}
    >
      <div
        aria-hidden
        style={{
          width: 74,
          height: 74,
          borderRadius: 99,
          background: "var(--ok)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontSize: 36,
          color: "#fff",
          fontWeight: 700,
          animation: "pop .28s cubic-bezier(.2,.9,.3,1.2) both",
        }}
      >
        ✓
      </div>
      <span
        style={{
          fontSize: 24,
          fontWeight: 700,
          color: "var(--txt)",
          marginTop: 8,
          letterSpacing: "-.01em",
        }}
      >
        Стол забронирован
      </span>
      <span style={{ fontSize: 16, color: "var(--hint)" }}>
        {fmt.whenLabel(booking.service_date, bar.today, booking.start_minutes)} ·{" "}
        {fmt.guests(booking.party_size)}
      </span>
      <div style={{ height: 8 }} />

      {session.reminders.should_ask ? (
        <Card style={{ width: "100%" }}>
          <span style={{ fontSize: 15, fontWeight: 600, color: "var(--txt)" }}>
            Напомнить за {fmt.hoursWord(bar.remind_hours)}?
          </span>
          <span
            style={{ fontSize: 13, color: "var(--hint)", lineHeight: 1.45, textWrap: "pretty" }}
          >
            Чтобы мы могли написать вам в этот чат — разрешите сообщения от бота.
          </span>
          <button
            type="button"
            onClick={onEnableReminders}
            style={{
              padding: 12,
              borderRadius: 10,
              background: "var(--btn)",
              color: "var(--btn-text)",
              fontSize: 15,
              fontWeight: 600,
              textAlign: "center",
            }}
          >
            Разрешить сообщения
          </button>
        </Card>
      ) : null}

      {session.reminders.opted_in ? (
        <span style={{ fontSize: 13, color: "var(--ok)", fontWeight: 500 }}>
          Напомним за {fmt.hoursWord(bar.remind_hours)} до брони
        </span>
      ) : null}
    </div>
  );
}
