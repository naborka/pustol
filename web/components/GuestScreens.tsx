"use client";

/**
 * What a guest sees: the bar, their bookings, the picker, the confirmation.
 *
 * Nothing here mentions a table. The bar assigns tables and moves them when the room changes, and a
 * number on a guest's screen becomes a number they arrive quoting — so the guest is told when they
 * are expected and how many of them, which is all they need and all that stays true.
 *
 * The picker is three steps in one screen: how many, which evening, what time. The evening is a
 * rail the length of the bar's own booking horizon, and every chip on it says what it holds before
 * it is tapped, so no guest ever taps into a day with nothing in it.
 *
 * What a new booking does to the ones a guest already holds is the server's rule, reported on each
 * booking and each evening. The screens read it; they never work it out again from a clock.
 */

import type { BarView, DayOffer, GuestBooking, Session } from "@/lib/api";
import type { ApiFailure } from "@/lib/errors";
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
  ReadFailed,
  SectionLabel,
  Separator,
  SlotGrid,
  Spinner,
  StaleNotice,
} from "./ui";

// ---- what a new booking would do ---------------------------------------------------------------

/** Whether a new booking on `serviceDate` would replace `held`. */
export function replacedBy(held: GuestBooking, serviceDate: fmt.IsoDate): boolean {
  return (
    held.rebooking_replaces === "any_evening" ||
    (held.rebooking_replaces === "same_evening" && held.service_date === serviceDate)
  );
}

/**
 * The time the guest chose, while the times on screen still have it free. Read off the times rather
 * than kept: one that passed or was taken since is simply no longer chosen, with nothing to reset.
 */
export function chosenTime(times: Availability | null, chosen: number | null): number | null {
  const free = times?.slots.some((slot) => slot.start_minutes === chosen && slot.state === "free");
  return free ? chosen : null;
}

/**
 * The bookings a guest holds once a write answered: without the ones it removed, with the one it
 * took, soonest first — the order the server sends them in.
 */
export function heldAfter(
  bookings: GuestBooking[],
  removed: string[],
  added: GuestBooking | null,
): GuestBooking[] {
  const kept = bookings.filter((held) => !removed.includes(held.id) && held.id !== added?.id);
  return [...kept, ...(added ? [added] : [])].sort(
    (left, right) =>
      left.service_date.localeCompare(right.service_date) ||
      left.start_minutes - right.start_minutes,
  );
}

/**
 * Whether the server would refuse a booking on `serviceDate` because of one the guest holds. Never
 * read off `rebooking_replaces`: a booking nothing can replace does not always hold its evening.
 */
export function heldOn(bookings: GuestBooking[], serviceDate: fmt.IsoDate): boolean {
  return bookings.some((held) => held.service_date === serviceDate && held.holds_evening);
}

// ---- home --------------------------------------------------------------------------------------

/** Who the bar is, and whether it is open. The name was in the payload and drawn nowhere. */
export function BarHeader({ bar }: { bar: BarView }) {
  const open = bar.open_now;
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
          {fmt.openLabel(bar.today_hours, bar.now_minutes, open)}
        </span>
      </div>
    </div>
  );
}

/**
 * The evening the picker opens on.
 *
 * Moving a held no-show opens on its own evening, the only one a booking replaces it from. Moving a
 * plan opens on its own evening while the bar still takes it. A new booking opens on tonight. Either
 * way never on an evening the guest already holds while another is open — a guest at the table
 * tonight is booking another night — and, when there is no other, on that one, where the picker
 * says why it cannot be booked.
 */
export function pickerStart(session: Session, moving: GuestBooking | null = null): fmt.IsoDate {
  if (moving?.rebooking_replaces === "same_evening") return moving.service_date;
  const { bookings, bar, bookable_days: days } = session;
  const start = moving?.service_date ?? bar.today;
  const open = days.filter((day) => !heldOn(bookings, day));
  return open.includes(start) ? start : (open[0] ?? days[0] ?? start);
}

export function BookingCard({
  booking,
  bar,
  onMove,
  onCancel,
}: {
  booking: GuestBooking;
  bar: BarView;
  /** Null when no booking the guest can make now would replace this one. */
  onMove: (() => void) | null;
  onCancel: () => void;
}) {
  const when = fmt.whenLabel(booking.service_date, bar.today, booking.start_minutes);
  return (
    <section role="group" aria-label={when}>
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
            {when}
          </span>
          <span style={{ fontSize: TEXT.lg, color: "var(--hint)" }}>
            {fmt.guests(booking.party_size)}
          </span>
        </div>
        <Separator />
        <Note>
          Держим стол {fmt.minutesAccusative(bar.grace_minutes)} после времени брони — дальше он
          может уйти другим гостям.
        </Note>
        <div style={{ display: "flex", gap: SPACE[2] }}>
          {onMove === null ? null : (
            <div style={{ flex: 1 }}>
              <CardAction tone="primary" label="Перенести" onClick={onMove} />
            </div>
          )}
          <div style={{ flex: 1 }}>
            <CardAction tone="destructive" label="Отменить" onClick={onCancel} />
          </div>
        </div>
      </Card>
    </section>
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
  onContact,
}: {
  session: Session;
  onMove: (booking: GuestBooking) => void;
  onCancel: (booking: GuestBooking) => void;
  onEnableReminders: () => void;
  onDismissReminders: () => void;
  onContact: (url: string) => void;
}) {
  const { bar, bookings } = session;
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

      {bookings.length === 0 ? (
        <Invitation session={session} />
      ) : (
        bookings.map((held) => (
          <BookingCard
            key={held.id}
            booking={held}
            bar={bar}
            onMove={held.rebooking_replaces === null ? null : () => onMove(held)}
            onCancel={() => onCancel(held)}
          />
        ))
      )}

      {/*
        The one place in the app that asks. And only once there is an evening to be reminded
        about: "планы изменятся — отмена одной кнопкой" is a sentence about a booking, and asking
        somebody who has not made one yet is a question with no subject.
      */}
      {bookings.length > 0 && session.reminders.should_ask ? (
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
            {bar.contact
              ? `Компания больше ${bar.max_party}`
              : `Компания больше ${bar.max_party} — только по договорённости с баром`}
          </span>
          {bar.contact ? (
            <Pressable
              onClick={() => onContact(bar.contact?.url ?? "")}
              style={{
                fontSize: TEXT.base,
                fontWeight: 600,
                color: "var(--link)",
                padding: `0 0 0 ${SPACE[2]}px`,
              }}
            >
              {`Связаться: ${bar.contact.label}`}
            </Pressable>
          ) : null}
        </div>
      </Card>
    </div>
  );
}

// ---- the picker --------------------------------------------------------------------------------

/** What a day chip says about itself, and whether it can be tapped at all. */
function chipDetail(offer: DayOffer): { text: string; tone: "hint" | "warn"; open: boolean } {
  if (offer.booked) return { text: "ваша бронь", tone: "hint", open: false };
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
  daysFailure,
  timesFailure,
  timesPending = false,
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
  /**
   * The newest read of the evenings failed: a card when there are none to show, a notice under the
   * ones shown. Kept apart from `timesFailure`: one succeeding must not hide that the other failed.
   */
  daysFailure: ApiFailure | null;
  timesFailure: ApiFailure | null;
  /** The times on screen answer a question the guest has since changed. */
  timesPending?: boolean;
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
          daysFailure ? (
            <Card gap={SPACE[2]}>
              <ReadFailed
                failure={daysFailure}
                audience="guest"
                generic="Не удалось прочитать свободные вечера."
                onRetry={onRetry}
              />
            </Card>
          ) : (
            <Spinner label="Смотрим вечера" />
          )
        ) : (
          <>
            {days.length === 0 ? (
              <Note tone="warn">Бар пока не принимает брони.</Note>
            ) : (
              <DayRailStrip
                days={days}
                today={bar.today}
                serviceDate={serviceDate}
                onServiceDate={onServiceDate}
              />
            )}
            {daysFailure ? (
              <StaleNotice failure={daysFailure} audience="guest" onRetry={onRetry} />
            ) : null}
          </>
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
          timesFailure ? (
            <Card gap={SPACE[2]}>
              <ReadFailed
                failure={timesFailure}
                audience="guest"
                generic="Не удалось прочитать свободные окна."
                onRetry={onRetry}
              />
            </Card>
          ) : (
            <Spinner label="Считаем свободные окна" />
          )
        ) : (
          <>
            {offered.length === 0 ? (
              <Note tone="warn">В этот вечер не осталось ни одного времени. Выберите другой.</Note>
            ) : (
              <>
                <SlotGrid
                  slots={offered}
                  chosen={chosenMinutes}
                  onPick={onPick}
                  onTaken={onTakenSlot}
                  stale={timesPending}
                />
                <Note>
                  Зачёркнутое время занято. Свободных окон: {availability.free_count} — за каждым
                  уже стоит настоящий стол на {fmt.guests(partySize)}.
                </Note>
              </>
            )}
            {timesFailure ? (
              <StaleNotice failure={timesFailure} audience="guest" onRetry={onRetry} />
            ) : null}
          </>
        )}
      </div>
    </div>
  );
}

/**
 * What the main button says at the bottom of the picker: the whole decision, in one line.
 *
 * `booked` is the rail's word that the guest already holds this evening, where the server would
 * refuse the booking. A guest whose booking would be replaced is moving it, and «Забронировать»
 * would make them wonder whether they are about to hold two.
 */
export function bookingDecision(
  partySize: number,
  serviceDate: string,
  bar: Pick<BarView, "today">,
  chosenMinutes: number | null,
  bookings: GuestBooking[] = [],
  booked = false,
): { label: string; enabled: boolean } {
  if (booked) return { label: "На этот вечер у вас уже есть бронь", enabled: false };
  if (chosenMinutes === null) return { label: "Выберите время", enabled: false };
  const when = `${fmt.dayFull(serviceDate, bar.today).toLowerCase()} в ${fmt.time(chosenMinutes)}`;
  const verb = bookings.some((held) => replacedBy(held, serviceDate)) ? "Перенести" : "Забронировать";
  return { label: `${verb} · ${fmt.guests(partySize)} · ${when}`, enabled: true };
}

// ---- the confirmation --------------------------------------------------------------------------

export function DoneScreen({
  booking,
  bar,
  moved = false,
}: {
  booking: GuestBooking;
  bar: BarView;
  /** The booking replaced an earlier one, so this is a move rather than a new table. */
  moved?: boolean;
}) {
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
        {moved ? "Бронь перенесена" : "Стол забронирован"}
      </span>
      <span style={{ fontSize: 16, color: "var(--hint)" }}>
        {fmt.whenLabel(booking.service_date, bar.today, booking.start_minutes)} ·{" "}
        {fmt.guests(booking.party_size)}
      </span>
      <div style={{ maxWidth: 300, marginTop: SPACE[2] }}>
        <Note>
          Держим стол {fmt.minutesAccusative(bar.grace_minutes)} после времени брони — дальше он может
          уйти другим гостям.
        </Note>
      </div>
    </div>
  );
}

/** The height a tap target must clear, re-exported so screens can assert it in a test. */
export const MIN_TAP = TAP;
