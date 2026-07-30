/**
 * The typed client.
 *
 * Every call carries the payload Telegram signed. The server verifies it on every request, so this
 * file holds no session, no token and no notion of being "logged in" — there is nothing here for an
 * attacker to steal and nothing to get out of step with the server.
 */

import type { IsoDate } from "./format";
import type { ApiFailure } from "./errors";

export interface Hours {
  open_minutes: number;
  close_minutes: number;
  closed: boolean;
}

export type BookingStatus = "confirmed" | "arrived" | "no_show" | "cancelled";
export type Attendance = "confirmed" | "arrived" | "no_show";
export type Source = "app" | "staff";
export type SlotState = "free" | "taken" | "past";

export interface GuestBooking {
  id: string;
  service_date: IsoDate;
  start_minutes: number;
  end_minutes: number;
  party_size: number;
  status: BookingStatus;
}

export interface BarView {
  name: string;
  address: string;
  timezone: string;
  max_party: number;
  grace_minutes: number;
  remind_hours: number;
  turn_minutes: number;
  slot_step_minutes: number;
  today: IsoDate;
  today_hours: Hours;
  last_arrival_minutes: number | null;
}

export interface Session {
  user: { id: number; first_name: string; username: string | null };
  is_staff: boolean;
  reminders: { opted_in: boolean; deliverable: boolean; should_ask: boolean };
  bar: BarView;
  booking: GuestBooking | null;
  bookable_days: IsoDate[];
}

export interface Slot {
  start_minutes: number;
  state: SlotState;
  evening: boolean;
}

export interface Availability {
  service_date: IsoDate;
  party_size: number;
  turn_minutes: number;
  slots: Slot[];
  free_count: number;
}

export interface BookingTaken {
  booking: GuestBooking;
  replaced: string | null;
}

export interface ShiftBooking {
  id: string;
  table_id: string | null;
  table_number: number | null;
  table_zone: string | null;
  start_minutes: number;
  end_minutes: number;
  party_size: number;
  guest_name: string;
  guest_username: string | null;
  status: BookingStatus;
  source: Source;
  reachable_by_bot: boolean;
}

export interface ShiftTable {
  id: string;
  number: number;
  seats: number;
  zone: string;
  blocked_because: string | null;
}

export interface ShiftView {
  service_date: IsoDate;
  hours: Hours;
  tables: ShiftTable[];
  bookings: ShiftBooking[];
  stats: { bookings: number; guests: number; free_now: number | null };
  now_minutes: number | null;
  cancel_reasons: string[];
  message_templates: string[];
}

export interface Reconciliation {
  moved: { booking_id: string; guest_name: string; to_number: number }[];
  orphaned: { booking_id: string; guest_name: string }[];
}

export interface CancelledByStaff {
  booking: ShiftBooking;
  reconciliation: Reconciliation;
  guest_notified: boolean;
}

export interface Bounds {
  min: number;
  max: number;
}

export interface Limits {
  open_minutes: Bounds;
  close_minutes: Bounds;
  turn_minutes: Bounds;
  max_party: Bounds;
  horizon_days: Bounds;
  remind_hours: Bounds;
  grace_minutes: Bounds;
  seats: Bounds;
  slot_step_minutes: number[];
}

export interface SettingsTable {
  id: string;
  number: number;
  seats: number;
  zone: string;
  bookings_today: number;
}

export interface SettingsView {
  name: string;
  address: string;
  timezone: string;
  week: Hours[];
  zones: string[];
  tables: SettingsTable[];
  turn_minutes: number;
  slot_step_minutes: number;
  max_party: number;
  horizon_days: number;
  remind_hours: number;
  grace_minutes: number;
  message_templates: string[];
  cancel_reasons: string[];
  staff: { username: string; bound: boolean }[];
  next_table_number: number;
  limits: Limits;
}

export interface SavedSettings {
  settings: SettingsView;
  reconciliation: Reconciliation;
  above_cap: number;
}

/** A table in a proposal: one that exists, or one being added. */
export type TableDraft =
  | { kind: "existing"; id: string; seats: number; zone: string }
  | { kind: "new"; seats: number; zone: string };

export interface SettingsDraft {
  name: string;
  address: string;
  timezone: string;
  week: Hours[];
  zones: string[];
  tables: TableDraft[];
  turn_minutes: number;
  slot_step_minutes: number;
  max_party: number;
  horizon_days: number;
  remind_hours: number;
  grace_minutes: number;
  message_templates: string[];
  cancel_reasons: string[];
  staff: { username: string }[];
}

/** A failure that carries the API's own code, so callers can decide what to say. */
export class ApiError extends Error {
  readonly failure: ApiFailure;
  readonly status: number;

  constructor(status: number, failure: ApiFailure) {
    super(failure.message);
    this.name = "ApiError";
    this.status = status;
    this.failure = failure;
  }
}

async function request<T>(
  credentials: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      ...(init?.body ? { "content-type": "application/json" } : {}),
      authorization: `tma ${credentials}`,
      ...init?.headers,
    },
  });

  if (!response.ok) {
    // A failure that is not the API's own shape — a proxy error page, a dropped connection — is
    // still reported with a code, so no caller has to handle "undefined" as a state.
    let failure: ApiFailure = {
      code: "internal",
      message: `HTTP ${response.status}`,
    };
    try {
      const body = (await response.json()) as { error?: ApiFailure };
      if (body.error?.code) failure = body.error;
    } catch {
      // Keep the fallback.
    }
    throw new ApiError(response.status, failure);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

function query(params: Record<string, string | number>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    search.set(key, String(value));
  }
  return search.toString();
}

/** Every call the app can make, bound to one set of credentials. */
export function client(credentials: string) {
  const get = <T>(path: string) => request<T>(credentials, path);
  const send = <T>(method: string, path: string, body?: unknown) =>
    request<T>(credentials, path, {
      method,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });

  return {
    session: () => get<Session>("/api/session"),

    availability: (serviceDate: IsoDate, partySize: number) =>
      get<Availability>(
        `/api/availability?${query({ service_date: serviceDate, party_size: partySize })}`,
      ),

    book: (serviceDate: IsoDate, startMinutes: number, partySize: number) =>
      send<BookingTaken>("POST", "/api/booking", {
        service_date: serviceDate,
        start_minutes: startMinutes,
        party_size: partySize,
      }),

    cancelMine: () => send<GuestBooking>("DELETE", "/api/booking"),

    optInToReminders: () => send<Session["reminders"]>("POST", "/api/reminders/opt-in"),
    dismissReminderPrompt: () => send<Session["reminders"]>("POST", "/api/reminders/dismiss"),

    shift: (serviceDate: IsoDate) =>
      get<ShiftView>(`/api/admin/shift?${query({ service_date: serviceDate })}`),

    staffAvailability: (serviceDate: IsoDate, partySize: number) =>
      get<Availability>(
        `/api/admin/availability?${query({ service_date: serviceDate, party_size: partySize })}`,
      ),

    createStaffBooking: (
      serviceDate: IsoDate,
      startMinutes: number,
      partySize: number,
      guestName: string,
    ) =>
      send<ShiftBooking>("POST", "/api/admin/bookings", {
        service_date: serviceDate,
        start_minutes: startMinutes,
        party_size: partySize,
        guest_name: guestName,
      }),

    setAttendance: (bookingId: string, attendance: Attendance) =>
      send<ShiftBooking>("PATCH", `/api/admin/bookings/${bookingId}/attendance`, {
        attendance,
      }),

    cancelAsStaff: (bookingId: string, reason: string) =>
      send<CancelledByStaff>("POST", `/api/admin/bookings/${bookingId}/cancel`, { reason }),

    sendTemplate: (bookingId: string, text: string) =>
      send<{ queued: boolean }>("POST", `/api/admin/bookings/${bookingId}/message`, { text }),

    blockTables: (serviceDate: IsoDate, tableIds: string[], reason: string) =>
      send<Reconciliation>("POST", "/api/admin/blocks", {
        service_date: serviceDate,
        table_ids: tableIds,
        reason,
      }),

    unblockTables: (serviceDate: IsoDate, tableIds: string[]) =>
      send<Reconciliation>("DELETE", "/api/admin/blocks", {
        service_date: serviceDate,
        table_ids: tableIds,
      }),

    reconcileShift: (serviceDate: IsoDate) =>
      send<Reconciliation>("POST", "/api/admin/shift/reconcile", {
        service_date: serviceDate,
      }),

    settings: (serviceDate: IsoDate) =>
      get<SettingsView>(`/api/admin/settings?${query({ service_date: serviceDate })}`),

    saveSettings: (serviceDate: IsoDate, draft: SettingsDraft) =>
      send<SavedSettings>(
        "PUT",
        `/api/admin/settings?${query({ service_date: serviceDate })}`,
        draft,
      ),
  };
}

export type Client = ReturnType<typeof client>;

/** The proposal a settings screen would send if nothing had been edited. */
export function draftOf(settings: SettingsView): SettingsDraft {
  return {
    name: settings.name,
    address: settings.address,
    timezone: settings.timezone,
    week: settings.week.map((hours) => ({ ...hours })),
    zones: [...settings.zones],
    tables: settings.tables.map((table) => ({
      kind: "existing" as const,
      id: table.id,
      seats: table.seats,
      zone: table.zone,
    })),
    turn_minutes: settings.turn_minutes,
    slot_step_minutes: settings.slot_step_minutes,
    max_party: settings.max_party,
    horizon_days: settings.horizon_days,
    remind_hours: settings.remind_hours,
    grace_minutes: settings.grace_minutes,
    message_templates: [...settings.message_templates],
    cancel_reasons: [...settings.cancel_reasons],
    staff: settings.staff.map((member) => ({ username: member.username })),
  };
}
