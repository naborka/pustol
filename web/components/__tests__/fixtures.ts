/**
 * The bar these tests argue about, so they argue about behaviour rather than about set-up.
 */

import type {
  Availability,
  BarView,
  DayOffer,
  GuestBooking,
  Limits,
  Session,
  SettingsView,
  ShiftBooking,
  ShiftTable,
  ShiftView,
} from "@/lib/api";

export const bar: BarView = {
  name: "Пустол",
  address: "ул. Рубинштейна, 24",
  timezone: "Europe/Belgrade",
  max_party: 6,
  grace_minutes: 15,
  remind_hours: 3,
  turn_minutes: 120,
  slot_step_minutes: 30,
  today: "2026-09-11",
  today_hours: { open_minutes: 1_080, close_minutes: 1_560, closed: false },
  last_arrival_minutes: 1_440,
  now_minutes: 1_280,
};

export const booking: GuestBooking = {
  id: "b1",
  service_date: "2026-09-11",
  start_minutes: 1_290,
  end_minutes: 1_410,
  party_size: 4,
  status: "confirmed",
};

export function session(overrides: Partial<Session> = {}): Session {
  return {
    user: { id: 999, first_name: "Алексей", username: "alexey" },
    is_staff: false,
    reminders: { opted_in: false, deliverable: true, should_ask: true },
    bar,
    booking: null,
    bookable_days: ["2026-09-11", "2026-09-12"],
    today_free_from_minutes: 1_290,
    ...overrides,
  };
}

export function dayOffer(overrides: Partial<DayOffer> = {}): DayOffer {
  return {
    service_date: "2026-09-11",
    closed: false,
    free_from_minutes: 1_290,
    ...overrides,
  };
}

/** A rail `length` days long starting on the fixture's today, every day open from 21:30. */
export function rail(length: number): DayOffer[] {
  const days: DayOffer[] = [];
  for (let offset = 0; offset < length; offset += 1) {
    const date = new Date(Date.UTC(2026, 8, 11 + offset)).toISOString().slice(0, 10);
    days.push({ service_date: date, closed: false, free_from_minutes: 1_290 });
  }
  return days;
}

export function availability(overrides: Partial<Availability> = {}): Availability {
  return {
    service_date: "2026-09-11",
    party_size: 2,
    turn_minutes: 120,
    slots: [
      { start_minutes: 1_080, state: "past", evening: true },
      { start_minutes: 1_290, state: "free", evening: true },
      { start_minutes: 1_320, state: "taken", evening: true },
      { start_minutes: 1_350, state: "free", evening: true },
    ],
    free_count: 2,
    ...overrides,
  };
}

export function shiftBooking(overrides: Partial<ShiftBooking> = {}): ShiftBooking {
  return {
    id: "b1",
    table_id: "t1",
    table_number: 7,
    table_zone: "Стойка",
    start_minutes: 1_260,
    end_minutes: 1_380,
    released_minutes: null,
    party_size: 2,
    guest_name: "Саша",
    guest_username: null,
    status: "confirmed",
    source: "app",
    note: null,
    reachable_by_bot: true,
    ...overrides,
  };
}

export function shiftTable(overrides: Partial<ShiftTable> = {}): ShiftTable {
  return { id: "t1", number: 7, seats: 2, zone: "Стойка", blocked_because: null, ...overrides };
}

export function shift(overrides: Partial<ShiftView> = {}): ShiftView {
  return {
    service_date: "2026-09-11",
    hours: { open_minutes: 1_080, close_minutes: 1_560, closed: false },
    tables: [
      shiftTable(),
      shiftTable({ id: "t2", number: 8, seats: 4, zone: "Зал" }),
      shiftTable({ id: "t3", number: 10, seats: 8, zone: "Зал" }),
    ],
    bookings: [shiftBooking()],
    stats: { bookings: 1, guests: 2, free_now: 2 },
    now_minutes: 1_280,
    largest_party_seatable_now: 8,
    days: [
      { service_date: "2026-09-11", closed: false, bookings: 1 },
      { service_date: "2026-09-12", closed: false, bookings: 0 },
      { service_date: "2026-09-13", closed: true, bookings: 0 },
    ],
    guest_horizon_days: 4,
    cancel_reasons: ["Дождь"],
    message_templates: ["Ваш стол готов"],
    ...overrides,
  };
}

export const LIMITS: Limits = {
  open_minutes: { min: 480, max: 1_080 },
  close_minutes: { min: 1_200, max: 1_680 },
  turn_minutes: { min: 60, max: 240 },
  max_party: { min: 2, max: 10 },
  horizon_days: { min: 1, max: 30 },
  remind_hours: { min: 1, max: 12 },
  grace_minutes: { min: 5, max: 60 },
  seats: { min: 1, max: 12 },
  slot_step_minutes: [15, 30, 60],
};

export function settingsView(overrides: Partial<SettingsView> = {}): SettingsView {
  return {
    name: "Пустол",
    address: "ул. Рубинштейна, 24",
    timezone: "Europe/Belgrade",
    week: Array.from({ length: 7 }, () => ({
      open_minutes: 1_080,
      close_minutes: 1_560,
      closed: false,
    })),
    zones: ["Зал", "Стойка", "Веранда"],
    tables: [
      { id: "t1", number: 7, seats: 2, zone: "Стойка", bookings_today: 0 },
      { id: "t2", number: 8, seats: 6, zone: "Зал", bookings_today: 0 },
    ],
    turn_minutes: 120,
    slot_step_minutes: 30,
    max_party: 6,
    horizon_days: 4,
    remind_hours: 3,
    grace_minutes: 15,
    message_templates: ["Ваш стол готов", "Опаздываете?", "Мы рядом", "Ждём вас"],
    cancel_reasons: ["Дождь", "Авария", "Частное мероприятие", "Технические проблемы"],
    staff: [
      { username: "nastya", bound: true },
      { username: "pavel", bound: false },
      { username: "marina", bound: false },
    ],
    next_table_number: 9,
    limits: LIMITS,
    ...overrides,
  };
}

export const noop = () => {};
