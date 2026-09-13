/**
 * The typed client.
 *
 * The first call carries the payload Telegram signed; the server answers it with a session, and
 * every call after carries that. The payload is accepted for an hour and Telegram never refreshes
 * it while the app stays open. The session is held in this closure only — never in storage, never
 * in a URL.
 *
 * A session the server refused for a reason only reopening the app answers is ended here, for every
 * call: nothing but a read of the session asked after the end is sent until one of those answers.
 */

import type { IsoDate } from "./format";
import { ApiError, needsRelaunch, type ApiFailure } from "./errors";

export interface Hours {
  open_minutes: number;
  close_minutes: number;
  closed: boolean;
}

export type BookingStatus = "confirmed" | "arrived" | "no_show" | "left" | "cancelled";
export type Attendance = "confirmed" | "arrived" | "no_show" | "left";
export type Source = "app" | "staff" | "walk";
export type SlotState = "free" | "taken" | "past";

/**
 * What a new booking would do to one the guest already holds — the server's rule, never guessed here.
 *
 * `any_evening`: a plan not yet begun, replaced by a booking on any evening, while some bookable
 * evening still has an arrival time the guest's other bookings do not hold. `same_evening`: a
 * no-show whose table is still held, replaced only by a booking on its own evening while that evening
 * still has an arrival time by the server's own clock and slot grid. `null`: no booking the guest can
 * make now replaces it — a party at the table, a held no-show with no arrival time left, a plan with
 * no evening left to move to, or a booking on an evening guests can no longer book. «Перенести» is
 * offered exactly when it is not `null`.
 *
 * It never says whether the evening is taken: that is `holds_evening`.
 */
export type Rebooking = "any_evening" | "same_evening" | null;

export interface GuestBooking {
  id: string;
  service_date: IsoDate;
  start_minutes: number;
  end_minutes: number;
  party_size: number;
  status: BookingStatus;
  /** Its window has begun, by the server's clock. */
  started: boolean;
  rebooking_replaces: Rebooking;
  /** A new booking on its evening would be refused because of it. */
  holds_evening: boolean;
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
  /** The bar's own clock, in wall-clock minutes into today's shift. */
  now_minutes: number;
  /** Today's opening in wall-clock minutes while it is still ahead, by the server's clock; else null. */
  opens_at_minutes: number | null;
  /**
   * Whether the bar is serving now, decided on instants by the opening and closing walk-ins go by:
   * on the night the clocks change, wall minutes say the wrong thing.
   */
  open_now: boolean;
  /** Where a person at the bar answers, already turned into a label and a link. */
  contact: { label: string; url: string } | null;
}

export interface Session {
  /** Sent instead of Telegram's payload from here on, which is accepted for an hour only. */
  session_token: string;
  user: { id: number; first_name: string; username: string | null };
  is_staff: boolean;
  reminders: { opted_in: boolean; deliverable: boolean; should_ask: boolean };
  bar: BarView;
  /** Every booking still holding a table for the guest, soonest first. */
  bookings: GuestBooking[];
  bookable_days: IsoDate[];
  /** The earliest time tonight still has for a new booking, or null when it has none. */
  today_free_from_minutes: number | null;
  /** The party size that answer speaks for, and the one the picker opens on. */
  today_free_for_party: number;
}

/** One chip on the guest's day rail. */
export interface DayOffer {
  service_date: IsoDate;
  closed: boolean;
  /** The earliest arrival time still free for this party, null when the day holds none. */
  free_from_minutes: number | null;
  /** The guest already holds this evening with a booking a new one would not replace. */
  booked: boolean;
}

export interface DayRail {
  party_size: number;
  days: DayOffer[];
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

/** The guest's times, with what a booking on that evening would do to the ones they hold. */
export interface GuestAvailability extends Availability {
  /** The bookings a booking on this evening would replace, to be sent back with it. */
  replacing: string[];
  /** The guest already holds this evening with a booking a new one would not replace. */
  booked: boolean;
}

export interface StaffSlot extends Slot {
  /** The tables free for this slot's window for this party, the booking being moved set aside. */
  free_table_ids: string[];
}

export interface StaffAvailability extends Availability {
  slots: StaffSlot[];
  /** Tables free for the set-aside booking's own stored window; null when nothing is set aside. */
  kept_free_table_ids: string[] | null;
}

export interface BookingTaken {
  booking: GuestBooking;
  /** Every booking this one replaced, soonest first. */
  replaced: string[];
}

export interface ShiftBooking {
  id: string;
  table_id: string | null;
  table_number: number | null;
  table_zone: string | null;
  start_minutes: number;
  /** The end of the window promised to the guest, never shortened by what happened on the night. */
  end_minutes: number;
  /** The minute the table went back into the pool, null while the booking still holds it. */
  released_minutes: number | null;
  party_size: number;
  guest_name: string;
  guest_username: string | null;
  status: BookingStatus;
  source: Source;
  /** What staff wrote on this booking. Never shown to the guest and never sent anywhere. */
  note: string | null;
  /** The guest has a Telegram account and the bot may write to it. */
  reachable_by_bot: boolean;
  /** Its window has begun, by the server's clock: its time is history from then on. */
  started: boolean;
  /** Its hold on its table is over, by the server's clock: it can be neither moved nor cancelled. */
  finished: boolean;
}

export interface ShiftTable {
  id: string;
  number: number;
  seats: number;
  zone: string;
  blocked_because: string | null;
}

/** One row of the staff day sheet. */
export interface ShiftDay {
  service_date: IsoDate;
  closed: boolean;
  bookings: number;
}

export interface ShiftView {
  service_date: IsoDate;
  /** The bar's running service day by the server's clock, which a phone left open overnight is not. */
  today: IsoDate;
  /**
   * The bar's counter of changes, read in the same snapshot as the rest. A room with a lower version is
   * older than the one on screen, whenever it arrives.
   */
  version: number;
  hours: Hours;
  tables: ShiftTable[];
  bookings: ShiftBooking[];
  /** `seated_now` is counted by the server in instants; null on an evening that is not running. */
  stats: {
    bookings: number;
    guests: number;
    free_now: number | null;
    seated_now: number | null;
  };
  now_minutes: number | null;
  /**
   * When a party seated now gives its table back, in wall-clock minutes of the shift like
   * `now_minutes`: a turn from now or the closing, whichever comes first, as the server counts it
   * across a clock change. Null when the server takes no party at the door on this evening now.
   */
  walk_in_until_minutes: number | null;
  /**
   * The active, unblocked tables free for the whole of the window a party seated now would get, as the
   * server counts it on instants. Empty when the server takes no party at the door now.
   */
  walk_in_free_table_ids: string[];
  /** The largest party the room could seat this minute; null when none fits or this is not today. */
  largest_party_seatable_now: number | null;
  /** Every evening staff can reach, with what is on. Longer than the guest's horizon. */
  days: ShiftDay[];
  /** How far ahead guests may book, so the day sheet can say where their horizon ends. */
  guest_horizon_days: number;
  cancel_reasons: string[];
  message_templates: string[];
}

export interface Reconciliation {
  moved: { booking_id: string; guest_name: string; to_number: number }[];
  orphaned: { booking_id: string; guest_name: string }[];
}

/**
 * Every staff write that can change the room answers with the evening as it stands after the write,
 * read inside the write's own transaction, so the phone never patches its own copy of the room.
 */
interface WithShift {
  shift: ShiftView;
}

export interface CancelledByStaff extends WithShift {
  booking: ShiftBooking;
  reconciliation: Reconciliation;
  /** A notice was queued and the bot can reach the guest. */
  guest_notified: boolean;
}

/** An attendance change: the booking now, and the attendance it had just before, for an undo. */
export interface AttendanceChange extends WithShift {
  booking: ShiftBooking;
  previous: Attendance;
}

export interface MovedBooking extends WithShift {
  booking: ShiftBooking;
  reconciliation: Reconciliation;
  /** Only a time change is the guest's to hear about. A table number they never saw. */
  guest_notified: boolean;
}

export interface BookingWritten extends WithShift {
  booking: ShiftBooking;
}

export interface Rearranged extends WithShift {
  reconciliation: Reconciliation;
}

export interface TablesClosed extends Rearranged {
  /** The tables this call closed; one that was already closed is not among them. */
  closed: string[];
}

export interface TablesReopened extends Rearranged {
  /** The closures this call removed, each with the reason it had. */
  reopened: { table_id: string; reason: string }[];
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
  /** The longest each text may be, in characters. */
  text: { name: number; address: number; message: number; reason: number };
  /** The most items each list may hold. */
  lists: ListLimits;
}

export interface ListLimits {
  zones: number;
  tables: number;
  message_templates: number;
  cancel_reasons: number;
  staff: number;
}

export interface SettingsTable {
  id: string;
  number: number;
  seats: number;
  zone: string;
}

export interface SettingsView {
  name: string;
  address: string;
  /** As the manager typed it; empty when there is none. */
  contact: string;
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
  /** The bar's count of saves. A save names it, and is refused if anybody saved since. */
  version: number;
}

export interface SavedSettings {
  settings: SettingsView;
  reconciliation: Reconciliation;
  above_cap: number;
}

/**
 * A table in a proposal. The app names a new table itself, so saving the same proposal twice updates
 * the table the first save made rather than adding a second.
 */
export interface TableDraft {
  id: string;
  seats: number;
  zone: string;
}

export interface SettingsDraft {
  name: string;
  address: string;
  contact: string;
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
  /** The version of the settings this proposal was made from. */
  version: number;
}

/** Why the session ended, and whether a read of it asked since is on its way; null while it has not. */
export type SessionEnd = { failure: ApiFailure; retrying: boolean } | null;

/**
 * How long a call may take before the app stops waiting and says so.
 *
 * A request lost on a basement's signal otherwise leaves a spinner turning for ever. Built on a
 * timer and an `AbortController` rather than `AbortSignal.timeout`, which older iOS webviews lack.
 */
export const REQUEST_TIMEOUT_MS = 15_000;

/** The call never got an answer: the phone's connection, not the bar's server. */
function unreachable(): ApiError {
  return new ApiError(0, { code: "network", message: "the request got no answer" });
}

async function request<T>(
  authorization: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), REQUEST_TIMEOUT_MS);
  try {
    let response: Response;
    try {
      response = await fetch(path, {
        ...init,
        signal: abort.signal,
        headers: {
          ...(init?.body ? { "content-type": "application/json" } : {}),
          authorization,
          ...init?.headers,
        },
      });
    } catch {
      throw unreachable();
    }

    if (!response.ok) {
      // A failure that is not the API's own shape — a proxy error page — is still reported with a
      // code, so no caller has to handle "undefined" as a state.
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
    try {
      return (await response.json()) as T;
    } catch (error) {
      if (abort.signal.aborted) throw unreachable();
      throw error;
    }
  } finally {
    clearTimeout(timer);
  }
}

function query(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) search.set(key, String(value));
  }
  return search.toString();
}

/**
 * Every call the app can make, bound to one set of credentials. `onSessionEnd` hears each change to
 * whether the session has ended.
 */
export function client(credentials: string, onSessionEnd: (end: SessionEnd) => void = () => {}) {
  let session: string | null = null;
  const authorization = () => (session === null ? `tma ${credentials}` : `session ${session}`);

  // Calls are numbered as they are asked. `at` marks the moment the session was found ended and `told`
  // the call whose failure is said; a refusal asked before the session last came back is old news.
  let asked = 0;
  let end: { at: number; told: number; error: ApiError } | null = null;
  let resumed = 0;
  const reading = new Set<number>();
  let said: SessionEnd = null;

  const tell = () => {
    const now: SessionEnd =
      end === null
        ? null
        : { failure: end.error.failure, retrying: [...reading].some((number) => number > (end?.at ?? 0)) };
    if (now?.failure === said?.failure && now?.retrying === said?.retrying) return;
    said = now;
    onSessionEnd(now);
  };

  const heard = (number: number, error: unknown, sessionRead: boolean) => {
    if (!(error instanceof ApiError)) return;
    const relaunch = needsRelaunch(error.failure);
    if (end === null) {
      if (!relaunch || number <= resumed) return;
      const at = (asked += 1);
      end = { at, told: at, error };
    } else if (number > end.told && (relaunch || sessionRead)) {
      end = { ...end, told: number, error };
    }
  };

  const call = async <T>(path: string, init?: RequestInit): Promise<T> => {
    if (end !== null) throw end.error;
    const number = (asked += 1);
    try {
      return await request<T>(authorization(), path, init);
    } catch (error) {
      heard(number, error, false);
      tell();
      throw error;
    }
  };
  const get = <T>(path: string) => call<T>(path);
  const send = <T>(method: string, path: string, body?: unknown) =>
    call<T>(path, {
      method,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });

  return {
    /** Asked whether or not the session has ended: an answer asked after the end brings it back. */
    session: async () => {
      const number = (asked += 1);
      reading.add(number);
      tell();
      try {
        const answer = await request<Session>(authorization(), "/api/session");
        session = answer.session_token;
        if (end !== null && number > end.at) {
          end = null;
          resumed = number;
        }
        return answer;
      } catch (error) {
        heard(number, error, true);
        throw error;
      } finally {
        reading.delete(number);
        tell();
      }
    },

    availability: (serviceDate: IsoDate, partySize: number) =>
      get<GuestAvailability>(
        `/api/availability?${query({ service_date: serviceDate, party_size: partySize })}`,
      ),

    days: (partySize: number) => get<DayRail>(`/api/days?${query({ party_size: partySize })}`),

    /** `replacing` is what the guest was told this booking replaces; the server refuses otherwise. */
    book: (serviceDate: IsoDate, startMinutes: number, partySize: number, replacing: string[]) =>
      send<BookingTaken>("POST", "/api/booking", {
        service_date: serviceDate,
        start_minutes: startMinutes,
        party_size: partySize,
        replacing,
      }),

    cancelMine: (bookingId: string) =>
      send<GuestBooking>("DELETE", `/api/bookings/${encodeURIComponent(bookingId)}`),

    optInToReminders: () => send<Session["reminders"]>("POST", "/api/reminders/opt-in"),
    dismissReminderPrompt: () => send<Session["reminders"]>("POST", "/api/reminders/dismiss"),

    shift: (serviceDate: IsoDate) =>
      get<ShiftView>(`/api/admin/shift?${query({ service_date: serviceDate })}`),

    /** `ignoring` is a booking being moved, which must not block its own time. */
    staffAvailability: (serviceDate: IsoDate, partySize: number, ignoring?: string) =>
      get<StaffAvailability>(
        `/api/admin/availability?${query({
          service_date: serviceDate,
          party_size: partySize,
          ignoring,
        })}`,
      ),

    createStaffBooking: (
      serviceDate: IsoDate,
      startMinutes: number,
      partySize: number,
      guestName: string,
      tableId: string,
    ) =>
      send<BookingWritten>("POST", "/api/admin/bookings", {
        service_date: serviceDate,
        start_minutes: startMinutes,
        party_size: partySize,
        guest_name: guestName,
        table_id: tableId,
      }),

    moveBooking: (
      bookingId: string,
      startMinutes: number,
      tableId: string | null,
      partySize: number,
    ) =>
      send<MovedBooking>("PATCH", `/api/admin/bookings/${bookingId}/move`, {
        start_minutes: startMinutes,
        table_id: tableId,
        party_size: partySize,
      }),

    /** `tableId` is the table staff chose; `null` asks the room for its own best fit. */
    seatWalkIn: (serviceDate: IsoDate, partySize: number, tableId: string | null) =>
      send<BookingWritten>("POST", "/api/admin/walkins", {
        service_date: serviceDate,
        party_size: partySize,
        table_id: tableId,
      }),

    setAttendance: (bookingId: string, attendance: Attendance) =>
      send<AttendanceChange>("PATCH", `/api/admin/bookings/${bookingId}/attendance`, {
        attendance,
      }),

    setNote: (bookingId: string, note: string | null) =>
      send<BookingWritten>("PATCH", `/api/admin/bookings/${bookingId}/note`, { note }),

    cancelAsStaff: (bookingId: string, reason: string) =>
      send<CancelledByStaff>("POST", `/api/admin/bookings/${bookingId}/cancel`, { reason }),

    sendTemplate: (bookingId: string, text: string) =>
      send<{ queued: boolean }>("POST", `/api/admin/bookings/${bookingId}/message`, { text }),

    blockTables: (serviceDate: IsoDate, tableIds: string[], reason: string) =>
      send<TablesClosed>("POST", "/api/admin/blocks", {
        service_date: serviceDate,
        table_ids: tableIds,
        reason,
      }),

    unblockTables: (serviceDate: IsoDate, tableIds: string[]) =>
      send<TablesReopened>("DELETE", "/api/admin/blocks", {
        service_date: serviceDate,
        table_ids: tableIds,
      }),

    reconcileShift: (serviceDate: IsoDate) =>
      send<Rearranged>("POST", "/api/admin/shift/reconcile", {
        service_date: serviceDate,
      }),

    settings: () => get<SettingsView>("/api/admin/settings"),

    saveSettings: (draft: SettingsDraft) => send<SavedSettings>("PUT", "/api/admin/settings", draft),
  };
}

export type Client = ReturnType<typeof client>;

/** The proposal a settings screen would send if nothing had been edited. */
export function draftOf(settings: SettingsView): SettingsDraft {
  return {
    name: settings.name,
    address: settings.address,
    contact: settings.contact,
    timezone: settings.timezone,
    week: settings.week.map((hours) => ({ ...hours })),
    zones: [...settings.zones],
    tables: settings.tables.map((table) => ({
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
    version: settings.version,
  };
}
