"use client";

/**
 * What a guest sees: the bar, their booking, the picker, the confirmation.
 *
 * Nothing here mentions a table. The bar assigns tables and moves them when the room changes, and a
 * number on a guest's screen becomes a number they arrive quoting — so the guest is told when they
 * are expected and how many of them, which is all they need and all that stays true.
 *
 * The picker is three steps in one screen: how many, which evening, what time. The evening is a
 * rail the length of the bar's own booking horizon, and every chip on it says what it holds before
 * it is tapped, so no guest ever taps into a day with nothing in it.
 */

import type { BarView, DayOffer, GuestBooking, Session } from "@/lib/api";
import * as fmt from "@/lib/format";
import type { Availability } from "@/lib/api";
import { RADIUS, SPACE, TAP, TEXT } from "@/lib/tokens";
import {
  Card,
  CardAction,
  Dot,
  InfoRow,
  Note,
  PartySizeGrid,
  Pressable,
  Rail,
  SectionLabel,
  Separator,
  SlotGrid,
  Spinner,
} from "./ui";

// ---- home --------------------------------------------------------------------------------------

/** Who the bar is, and whether it is open. The name was in the payload and drawn nowhere. */
export function BarHeader({ bar }: { bar: BarView }) {
  const open = fmt.isOpenNow(bar.today_hours, bar.now_minutes);
  return (
    <div
      style={{
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "space-between",
        gap: SPACE[3],
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
        <span
          style={{
            fontSize: TEXT.h2,
            fontWeight: 700,
            color: "var(--txt)",
            letterSpacing: "-.01em",
          }}
        >
          {bar.name}
        </span>
        <span style={{ fontSize: TEXT.md, color: "var(--hint)" }}>{bar.address}</span>
      </div>
      <div
        style={{
          flex: "none",
          display: "flex",
          alignItems: "center",
          gap: SPACE[1] + 2,
          padding: `${SPACE[1] + 2}px ${SPACE[2] + 2}px`,
          borderRadius: RADIUS.pill,
          background: "var(--sec)",
        }}
      >
        <Dot color={open ? "var(--ok)" : "var(--hint)"} size={6} />
        <span style={{ fontSize: TEXT.sm, fontWeight: 600, color: "var(--txt)" }}>
          {fmt.openLabel(bar.today_hours, bar.now_minutes)}
        </span>
      </div>
    </div>
  );
}

export function BookingCard({
  booking,
  bar,
  onMove,
  onCancel,
}: {
  booking: GuestBooking;
  bar: BarView;
  onMove: () => void;
  onCancel: () => void;
}) {
  return (
    <Card padding={SPACE[4] + 2} gap={SPACE[3] + 2}>
      <div style={{ display: "flex", alignItems: "center", gap: SPACE[2] }}>
        <Dot color="var(--ok)" />
        <span
          style={{
            fontSize: TEXT.sm,
            fontWeight: 600,
            letterSpacing: ".06em",
            textTransform: "uppercase",
            color: "var(--ok)",
          }}
        >
          Стол ваш
        </span>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
        <span
          style={{
            fontSize: TEXT.h1,
            fontWeight: 700,
            color: "var(--txt)",
            letterSpacing: "-.02em",
          }}
        >
          {fmt.whenLabel(booking.service_date, bar.today, booking.start_minutes)}
        </span>
        <span style={{ fontSize: TEXT.lg, color: "var(--hint)" }}>
          {fmt.guests(booking.party_size)}
        </span>
      </div>
      <Separator />
      <Note>
        Держим стол {fmt.minutesWord(bar.grace_minutes)} после времени брони. Опаздываете —
        напишите нам, стол дождётся.
      </Note>
      <div style={{ display: "flex", gap: SPACE[2] }}>
        <div style={{ flex: 1 }}>
          <CardAction tone="primary" label="Перенести" onClick={onMove} />
        </div>
        <div style={{ flex: 1 }}>
          <CardAction tone="destructive" label="Отменить" onClick={onCancel} />
        </div>
      </div>
    </Card>
  );
}

/** The one place in the whole app that asks about reminders. */
export function ReminderCard({
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
      <span style={{ fontSize: TEXT.lg, fontWeight: 600, color: "var(--txt)" }}>
        Напомнить за {fmt.hoursWord(bar.remind_hours)}?
      </span>
      <Note>
        Бот напишет в этот чат. Планы изменятся — отмена одной кнопкой прямо из сообщения.
      </Note>
      <div style={{ display: "flex", gap: SPACE[2] }}>
        <div style={{ flex: 1 }}>
          <CardAction tone="primary" label="Напомнить" onClick={onEnable} />
        </div>
        <Pressable
          onClick={onDismiss}
          style={{
            padding: `0 ${SPACE[4]}px`,
            borderRadius: RADIUS.md,
            color: "var(--hint)",
            fontSize: TEXT.base,
            fontWeight: 600,
            justifyContent: "center",
          }}
        >
          Не нужно
        </Pressable>
      </div>
    </Card>
  );
}

/** No booking yet: a quiet statement of what tonight has, not a second button. */
function Invitation({ session }: { session: Session }) {
  const free = session.today_free_from_minutes;
  return (
    <Card padding={SPACE[4] + 2} gap={SPACE[2]}>
      <span style={{ fontSize: TEXT.h2, fontWeight: 700, color: "var(--txt)" }}>
        Столик на вечер
      </span>
      <Note>
        {free === null
          ? "Сегодня мест нет — посмотрите завтра."
          : `Сегодня свободно с ${fmt.time(free)}. Подтверждение сразу, без звонка и ожидания.`}
      </Note>
    </Card>
  );
}

export function HomeScreen({
  session,
  onMove,
  onCancel,
  onEnableReminders,
  onDismissReminders,
  onWriteToBar,
}: {
  session: Session;
  onMove: () => void;
  onCancel: () => void;
  onEnableReminders: () => void;
  onDismissReminders: () => void;
  onWriteToBar: () => void;
}) {
  const { bar, booking } = session;
  return (
    <div
      style={{
        padding: `${SPACE[4]}px ${SPACE[4]}px ${SPACE[6]}px`,
        display: "flex",
        flexDirection: "column",
        gap: SPACE[3] + 2,
      }}
    >
      <BarHeader bar={bar} />

      {booking ? (
        <BookingCard booking={booking} bar={bar} onMove={onMove} onCancel={onCancel} />
      ) : (
        <Invitation session={session} />
      )}

      {/*
        The one place in the app that asks. And only once there is an evening to be reminded
        about: "планы изменятся — отмена одной кнопкой" is a sentence about a booking, and asking
        somebody who has not made one yet is a question with no subject.
      */}
      {booking && session.reminders.should_ask ? (
        <ReminderCard bar={bar} onEnable={onEnableReminders} onDismiss={onDismissReminders} />
      ) : null}

      <Card>
        <InfoRow label="Сегодня" value={fmt.hoursLabel(bar.today_hours)} />
        <Separator />
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            gap: SPACE[3],
          }}
        >
          <span style={{ fontSize: TEXT.base, color: "var(--hint)" }}>
            Компания больше {bar.max_party}
          </span>
          <Pressable
            onClick={onWriteToBar}
            style={{
              fontSize: TEXT.base,
              fontWeight: 600,
              color: "var(--link)",
              padding: `0 0 0 ${SPACE[2]}px`,
            }}
          >
            Написать бару
          </Pressable>
        </div>
      </Card>
    </div>
  );
}

// ---- the picker --------------------------------------------------------------------------------

/** What a day chip says about itself, and whether it can be tapped at all. */
function chipDetail(offer: DayOffer): { text: string; tone: "hint" | "warn"; open: boolean } {
  if (offer.closed) return { text: "выходной", tone: "hint", open: false };
  if (offer.free_from_minutes === null) return { text: "мест нет", tone: "warn", open: false };
  return { text: `с ${fmt.time(offer.free_from_minutes)}`, tone: "hint", open: true };
}

export function DayRailStrip({
  days,
  today,
  serviceDate,
  onServiceDate,
}: {
  days: DayOffer[];
  today: string;
  serviceDate: string;
  onServiceDate: (date: string) => void;
}) {
  return (
    <Rail label="Вечер">
      {days.map((offer) => {
        const chosen = offer.service_date === serviceDate;
        const detail = chipDetail(offer);
        return (
          <Pressable
            key={offer.service_date}
            ariaPressed={chosen}
            disabled={!detail.open}
            {...(detail.open ? { onClick: () => onServiceDate(offer.service_date) } : {})}
            tone="card"
            style={{
              flex: "none",
              minWidth: 86,
              minHeight: 60,
              padding: `${SPACE[2]}px ${SPACE[2]}px`,
              borderRadius: RADIUS.md,
              background: chosen ? "var(--btn)" : detail.open ? "var(--chip)" : "var(--chip-off)",
              color: chosen ? "var(--btn-text)" : detail.open ? "var(--txt)" : "var(--hint)",
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              gap: 2,
            }}
          >
            <span style={{ fontSize: TEXT.base, fontWeight: 600 }}>
              {fmt.dayChip(offer.service_date, today)}
            </span>
            <span
              style={{
                fontSize: TEXT.xs,
                fontWeight: 600,
                color: chosen
                  ? "var(--btn-text)"
                  : detail.tone === "warn"
                    ? "var(--warn)"
                    : "var(--hint)",
              }}
            >
              {detail.text}
            </span>
          </Pressable>
        );
      })}
    </Rail>
  );
}

export function BookScreen({
  bar,
  days,
  availability,
  partySize,
  serviceDate,
  chosenMinutes,
  failedToLoad,
  onPartySize,
  onServiceDate,
  onPick,
  onTakenSlot,
  onRetry,
  onBack,
}: {
  bar: BarView;
  days: DayOffer[] | null;
  availability: Availability | null;
  partySize: number;
  serviceDate: string;
  chosenMinutes: number | null;
  failedToLoad: boolean;
  onPartySize: (size: number) => void;
  onServiceDate: (date: string) => void;
  onPick: (minutes: number) => void;
  onTakenSlot: () => void;
  onRetry: () => void;
  onBack?: () => void;
}) {
  const slots = availability?.slots ?? [];
  const offered = slots.filter((slot) => slot.state !== "past");

  return (
    <div
      style={{
        padding: `${SPACE[3] + 2}px ${SPACE[4]}px ${SPACE[5]}px`,
        display: "flex",
        flexDirection: "column",
        gap: SPACE[5],
      }}
    >
      {onBack ? (
        <Pressable
          ariaLabel="Назад"
          onClick={onBack}
          style={{
            alignSelf: "flex-start",
            fontSize: TEXT.base,
            color: "var(--link)",
            fontWeight: 600,
            padding: `0 ${SPACE[2]}px 0 0`,
          }}
        >
          ‹ Назад
        </Pressable>
      ) : null}

      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
        <SectionLabel>Сколько гостей</SectionLabel>
        <PartySizeGrid max={bar.max_party} value={partySize} onChange={onPartySize} />
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
        <SectionLabel>Какой вечер</SectionLabel>
        {days === null ? (
          failedToLoad ? (
            <Card gap={SPACE[2]}>
              <Note tone="warn">Не удалось прочитать свободные вечера.</Note>
              <CardAction label="Попробовать снова" onClick={onRetry} />
            </Card>
          ) : (
            <Spinner label="Смотрим вечера" />
          )
        ) : days.length === 0 ? (
          <Note tone="warn">Бар пока не принимает брони.</Note>
        ) : (
          <DayRailStrip
            days={days}
            today={bar.today}
            serviceDate={serviceDate}
            onServiceDate={onServiceDate}
          />
        )}
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: SPACE[2] + 2 }}>
        <div
          style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline" }}
        >
          <SectionLabel>Во сколько</SectionLabel>
          <span style={{ fontSize: TEXT.sm, color: "var(--hint)" }}>
            стол на {fmt.hours(availability?.turn_minutes ?? bar.turn_minutes)}
          </span>
        </div>

        {availability === null ? (
          failedToLoad ? (
            <Card gap={SPACE[2]}>
              <Note tone="warn">Не удалось прочитать свободные окна.</Note>
              <CardAction label="Попробовать снова" onClick={onRetry} />
            </Card>
          ) : (
            <Spinner label="Считаем свободные окна" />
          )
        ) : offered.length === 0 ? (
          <Note tone="warn">В этот вечер не осталось ни одного времени. Выберите другой.</Note>
        ) : (
          <>
            <SlotGrid
              slots={offered}
              chosen={chosenMinutes}
              onPick={onPick}
              onTaken={onTakenSlot}
            />
            <Note>
              Зачёркнутое время занято. Свободных окон: {availability.free_count} — за каждым уже
              стоит настоящий стол на {fmt.guests(partySize)}.
            </Note>
          </>
        )}
      </div>
    </div>
  );
}

/** What the main button says at the bottom of the picker: the whole decision, in one line. */
export function bookingDecision(
  partySize: number,
  serviceDate: string,
  today: string,
  chosenMinutes: number | null,
): { label: string; enabled: boolean } {
  if (chosenMinutes === null) return { label: "Выберите время", enabled: false };
  const when = `${fmt.dayFull(serviceDate, today).toLowerCase()} в ${fmt.time(chosenMinutes)}`;
  return { label: `Забронировать · ${fmt.guests(partySize)} · ${when}`, enabled: true };
}

// ---- the confirmation --------------------------------------------------------------------------

export function DoneScreen({ booking, bar }: { booking: GuestBooking; bar: BarView }) {
  return (
    <div
      style={{
        padding: `${SPACE[7] + 20}px ${SPACE[5]}px`,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: SPACE[2] + 2,
        textAlign: "center",
      }}
    >
      <div
        aria-hidden
        style={{
          width: 74,
          height: 74,
          borderRadius: RADIUS.pill,
          background: "var(--ok)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontSize: 36,
          // The page colour, not white: against the dark scheme's green a white tick is 2.3:1,
          // which is under the threshold even for something this large.
          color: "var(--bg)",
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
          marginTop: SPACE[2],
          letterSpacing: "-.01em",
        }}
      >
        Стол забронирован
      </span>
      <span style={{ fontSize: 16, color: "var(--hint)" }}>
        {fmt.whenLabel(booking.service_date, bar.today, booking.start_minutes)} ·{" "}
        {fmt.guests(booking.party_size)}
      </span>
      <div style={{ maxWidth: 300, marginTop: SPACE[2] }}>
        <Note>
          Держим стол {fmt.minutesWord(bar.grace_minutes)} после времени брони. Опаздываете —
          напишите нам, стол дождётся.
        </Note>
      </div>
    </div>
  );
}

/** The height a tap target must clear, re-exported so screens can assert it in a test. */
export const MIN_TAP = TAP;
