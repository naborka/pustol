/**
 * Typed client.
 *
 * First call sends Telegram signed payload; server answers with session token, later calls send that.
 * Payload valid one hour only, and Telegram never refreshes it while app stays open. Token lives only
 * in this closure, never storage or URL.
 *
 * Session refused for reason only relaunch fixes ends here for every call: only session read asked
 * after end is sent, until one answers.
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
 * What new booking does to one guest already holds. Server rule, never guessed here.
 *
 * `any_evening`: plan not begun, replaced by booking on any evening, while some bookable evening
 * still has arrival time guest's other bookings do not hold.
 * `same_evening`: no-show with table still held, replaced only by booking on its own evening while
 * that evening still has arrival time by server clock and slot grid.
 * `null`: nothing guest can book now replaces it: party at table, held no-show with no arrival time
 * left, plan with no evening to move to, or evening guests can no longer book.
 * «Перенести» offered exactly when not `null`. Never says whether evening taken: see `holds_evening`.
 */
export type Rebooking = "any_evening" | "same_evening" | null;

export interface GuestBooking {
  id: string;
  service_date: IsoDate;
  start_minutes: number;
  end_minutes: number;
  party_size: number;
  status: BookingStatus;
  /** Window begun, by server clock. */
  started: boolean;
  rebooking_replaces: Rebooking;
  /** New booking on its evening refused because of it. */
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
  /** Today's opening in wall minutes while still ahead by server clock; else null. */
  opens_at_minutes: number | null;
  /** Decided on instants by walk-in opening and closing: wall minutes wrong on clock-change night. */
  open_now: boolean;
  /** Where person at bar answers, already label and link. */
  contact: { label: string; url: string } | null;
}

export interface Session {
  /** Sent instead of Telegram payload from here on; payload valid one hour only. */
  session_token: string;
  user: { id: number; first_name: string; username: string | null };
  is_staff: boolean;
  reminders: { opted_in: boolean; deliverable: boolean; should_ask: boolean };
  bar: BarView;
  /** Every booking still holding table for guest, soonest first. */
  bookings: GuestBooking[];
  bookable_days: IsoDate[];
  /** Earliest time tonight still free for new booking; null when none. */
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
  /** Guest already holds this evening with booking new one would not replace. */
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

export interface GuestAvailability extends Availability {
  /** Bookings a booking this evening replaces; send back with it. */
  replacing: string[];
  /** Guest already holds this evening with booking new one would not replace. */
  booked: boolean;
}

export interface StaffSlot extends Slot {
  /** Tables free for slot window for this party, moved booking set aside. */
  free_table_ids: string[];
}

export interface StaffAvailability extends Availability {
  slots: StaffSlot[];
  /** Tables free for set-aside booking's own stored window; null when nothing set aside. */
  kept_free_table_ids: string[] | null;
}

export interface BookingTaken {
  booking: GuestBooking;
  /** Soonest first. */
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
  /** Guest has Telegram account and bot may write to it. */
  reachable_by_bot: boolean;
  /** Window begun by server clock; its time is history from then. */
  started: boolean;
  /** Table hold over by server clock; cannot be moved or cancelled. */
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
  /** Bar's running service day by server clock; phone left open overnight is not. */
  today: IsoDate;
  /** Bar change counter, same snapshot as rest. Lower version is older room, whenever it arrives. */
  version: number;
  hours: Hours;
  tables: ShiftTable[];
  bookings: ShiftBooking[];
  /** `seated_now` counted by server on instants; null on evening not running. */
  stats: {
    bookings: number;
    guests: number;
    free_now: number | null;
    seated_now: number | null;
  };
  now_minutes: number | null;
  /**
   * When party seated now frees table, in shift wall minutes like `now_minutes`: one turn or closing,
   * whichever first, counted by server across clock change. Null when server takes no walk-in now.
   */
  walk_in_until_minutes: number | null;
  /**
   * Active unblocked tables free for whole window of party seated now, counted by server on instants.
   * Empty when server takes no walk-in now.
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
 * Staff write that can change room answers with evening after write, read in same transaction, so
 * phone never patches own room copy.
 */
interface WithShift {
  shift: ShiftView;
}

export interface CancelledByStaff extends WithShift {
  booking: ShiftBooking;
  reconciliation: Reconciliation;
  /** Notice queued and bot can reach guest. */
  guest_notified: boolean;
}

/** `previous`: attendance just before change, for undo. */
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
  /** Tables this call closed; already closed ones excluded. */
  closed: string[];
}

export interface TablesReopened extends Rearranged {
  /** Closures this call removed, each with its reason. */
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
  /** Max length per text, in characters. */
  text: { name: number; address: number; message: number; reason: number };
  /** Max items per list. */
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
  /** As manager typed; empty when none. */
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
  /** Bar save counter. Save names it; refused if anybody saved since. */
  version: number;
}

export interface SavedSettings {
  settings: SettingsView;
  reconciliation: Reconciliation;
  above_cap: number;
}

/** App names new table itself, so saving same proposal twice updates first save's table, not adds second. */
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
  /** Settings version this proposal made from. */
  version: number;
}

/** Why session ended, and whether session read asked since is in flight; null while not ended. */
export type SessionEnd = { failure: ApiFailure; retrying: boolean } | null;

/**
 * Lost request on weak signal otherwise spins forever. Timer plus `AbortController`, not
 * `AbortSignal.timeout`: older iOS webviews lack it.
 */
export const REQUEST_TIMEOUT_MS = 15_000;

/** No answer: phone connection, not bar server. */
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
      // Non-API body, such as proxy error page, still gets code, so callers never see undefined.
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

/** `onSessionEnd` hears each change to whether session ended. */
export function client(credentials: string, onSessionEnd: (end: SessionEnd) => void = () => {}) {
  let session: string | null = null;
  const authorization = () => (session === null ? `tma ${credentials}` : `session ${session}`);

  // Calls numbered as asked. `at`: when session found ended. `told`: call whose failure is shown.
  // Refusal asked before session last resumed is old news.
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
    /** Sent even after session end: answer asked after end resumes session. */
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

    /** `replacing`: what guest was told this booking replaces; server refuses mismatch. */
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
