"use client";

/**
 * The whole app, one screen deep.
 *
 * A Mini App has no address bar and no history to speak of, so navigation is state rather than
 * routing: three guest screens, three staff panes and a set of sheets. Keeping that state here —
 * and every fetch with it — means the screens stay pure functions of what is loaded, which is what
 * makes them worth testing.
 *
 * This file also holds the rules for how an action feels. Anything reversible happens on one tap
 * and comes back with a way to undo it; anything the guest will feel is confirmed first and gets no
 * undo, because the confirmation was the protection. One action runs at a time, so a second tap on
 * a slow connection is not a second booking or a second message. And what is on screen keeps up by
 * itself: a shift left open on the bar is the normal case, not the exception.
 *
 * Everything read goes through one read model (`useRead`), and every staff write answers with the
 * evening as the server has it after the write, which goes on screen through that same model. The
 * phone never patches its own copy of the room.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  client as makeClient,
  draftOf,
  type Attendance,
  type DayOffer,
  type GuestAvailability,
  type GuestBooking,
  type Reconciliation,
  type Session,
  type SessionEnd,
  type SettingsView,
  type ShiftBooking,
  type ShiftView,
  type StaffAvailability,
} from "@/lib/api";
import { failureOf, messageFor, needsRelaunch, type Audience } from "@/lib/errors";
import * as fmt from "@/lib/format";
import {
  attendanceOutcome,
  closuresToRestore,
  reconciliationReport,
  refusalOf,
  type Closure,
  type Refusal,
} from "@/lib/outcomes";
import { edited, firstReason, same, type Edit } from "@/lib/settingsRules";
import type { Order } from "@/lib/reads";
import { isDirty, received, savedInto, type SettingsPair } from "@/lib/settingsSync";
import {
  NO_SHEET,
  closedIfStill,
  refreshedGuestSheet,
  refreshedSheet,
  type OpenSheet,
  type SheetContent,
} from "@/lib/sheet";
import { credentials, haptics, openBotChat, openContact, webApp } from "@/lib/telegram";
import { SPACE, TIMING } from "@/lib/tokens";
import { useRead } from "@/lib/useRead";
import { ShiftActions, ShiftScreen, type ShiftPane } from "@/components/AdminShift";
import { AppShell, InsetFrame, type StaffTab } from "@/components/AppChrome";
import {
  BookScreen,
  DoneScreen,
  HomeScreen,
  bookingDecision,
  chosenTime,
  heldAfter,
  pickerStart,
} from "@/components/GuestScreens";
import { SaveBar, SettingsScreen, type Section } from "@/components/Settings";
import { useInsets } from "@/components/ThemeProvider";
import {
  BookingSheet,
  CancelReasonSheet,
  ConflictSheet,
  DaySheet,
  GuestCancelSheet,
  ManualBookingSheet,
  MessageSheet,
  MoveBookingSheet,
  TableSheet,
  WalkInSheet,
} from "@/components/Sheets";
import {
  Card,
  Failure,
  MainButton,
  Note,
  ReadView,
  Spinner,
  Toast,
  type ToastMessage,
} from "@/components/ui";

type Tab = StaffTab;
type GuestScreen = "home" | "book" | "done";

/**
 * What a staff-side party size starts at, and the placeholder the picker holds until it is opened.
 *
 * The guest's own default is not this: it is `session.today_free_for_party`, the party the server's
 * home-screen sentence spoke for, so the card and the picker cannot promise different evenings.
 */
const DEFAULT_PARTY = 2;

/**
 * How often an open shift asks what has changed.
 *
 * A guest books from the app, a colleague seats somebody from another phone, a party runs late —
 * and none of it reached a shift screen that nobody touched. Half a minute is well inside the grace
 * a late party is given, and a request that small costs the server nothing.
 */
const SHIFT_REFRESH_MS = 30_000;

/** How often the guest's home screen does: the open-until line and a booking the bar has closed. */
const HOME_REFRESH_MS = 60_000;

/** The session is one question, whoever asks it. */
const SESSION = "session";

/** So are the settings: they are the bar's, whichever evening is on screen. */
const SETTINGS = "settings";

/**
 * Rooms are ordered by the server's version, not by when they arrived. The version does not move
 * with the clock or with whether the bot can reach a guest, so two rooms of one version go by when
 * they were asked.
 */
const roomOrder: Order<ShiftView> = (next, shown) => next.version - shown.version;

/** Settings are ordered by the bar's count of saves, so an older reread never lands over a save. */
const settingsOrder: Order<SettingsView> = (next, shown) => next.version - shown.version;

/** A read whose answers are only drawn, never folded into anything. */
const nothing = () => {};

const EMPTY_MANUAL = {
  name: "",
  partySize: DEFAULT_PARTY,
  minutes: null as number | null,
  table: null as string | null,
};

/**
 * Calls `refresh` every `intervalMs` while the app is on screen, and at once when it comes back.
 *
 * Telegram keeps a minimised Mini App alive and tells it when it is shown again; a phone locked on
 * the bar hides the page. Neither is a reason to keep polling, and both are a reason to catch up
 * the moment somebody looks.
 */
function useWhileVisible(refresh: (() => void) | null, intervalMs: number) {
  useEffect(() => {
    if (!refresh) return undefined;
    const tick = () => {
      if (document.visibilityState === "visible") refresh();
    };
    const interval = window.setInterval(tick, intervalMs);
    document.addEventListener("visibilitychange", tick);
    const app = webApp();
    app?.onEvent("activated", tick);
    return () => {
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", tick);
      app?.offEvent("activated", tick);
    };
  }, [refresh, intervalMs]);
}

/**
 * State whose newest value can be read the moment it is written, not only on the next render: an
 * answer folded into it must be folded into what is there now, not into a copy from a render ago.
 */
function useSynced<T>(initial: T): [T, (change: (current: T) => T) => void, { readonly current: T }] {
  const now = useRef(initial);
  const [value, setValue] = useState(initial);
  const change = useCallback((next: (current: T) => T) => {
    now.current = next(now.current);
    setValue(now.current);
  }, []);
  return [value, change, now];
}

export default function Page() {
  // Read after mounting, never while rendering: the page is prerendered where there is no Telegram,
  // and that HTML is what a guest sees until the scripts have loaded.
  const [token, setToken] = useState<string | null | undefined>(undefined);
  useEffect(() => setToken(credentials()), []);
  // Whether the client found the session ended, as it last said.
  const [sessionEnd, changeSessionEnd, sessionEndNow] = useSynced<SessionEnd>(null);
  const api = useMemo(
    () => (token ? makeClient(token, (end) => changeSessionEnd(() => end)) : null),
    [token, changeSessionEnd],
  );

  const [toast, setToast] = useState<ToastMessage | null>(null);
  const toastNow = useRef<ToastMessage | null>(null);
  const dismissToast = useRef(0);

  const [tab, setTab] = useState<Tab>("client");
  const [screen, setScreen] = useState<GuestScreen>("home");
  const [taken, setTaken] = useState<{ booking: GuestBooking; moved: boolean } | null>(null);

  const [partySize, setPartySize] = useState(DEFAULT_PARTY);
  const [serviceDate, setServiceDate] = useState<string | null>(null);
  const [chosenMinutes, setChosenMinutes] = useState<number | null>(null);

  const [shiftDate, setShiftDate] = useState<string | null>(null);
  const [pane, setPane] = useState<ShiftPane>("now");

  const [pair, changePair, pairNow] = useSynced<SettingsPair | null>(null);
  // The edits made while a save is on its way, to be made again on top of what it stored.
  const editsDuringSave = useRef<Edit[] | null>(null);
  const [settingsSection, setSettingsSection] = useState<Section | null>(null);
  const [editedWeekday, setEditedWeekday] = useState(1);
  const [saving, changeSaving, savingNow] = useSynced(false);
  // Whether somebody asked for the settings while a save was on its way, to read once it answered.
  const settingsWanted = useRef(false);
  // Why the last save was refused, until the next edit or save.
  const [refusal, setRefusal] = useState<Refusal | null>(null);
  // What a read folded into the edit, said on the settings screen until the next edit or save.
  const [folded, setFolded] = useState<string | null>(null);

  const insets = useInsets();
  const [sheet, setSheet] = useState<OpenSheet>(NO_SHEET);
  const openings = useRef(0);
  const openSheet = useCallback(
    (content: SheetContent) => setSheet({ ...content, opened: (openings.current += 1) }),
    [],
  );
  const [manual, setManual] = useState(EMPTY_MANUAL);
  const [move, setMove] = useState({
    minutes: null as number | null,
    table: null as string | null,
    party: null as number | null,
  });
  const [walkInParty, setWalkInParty] = useState(DEFAULT_PARTY);
  // A preference, not the decision: the sheet resolves it against the tables actually free.
  const [walkInTable, setWalkInTable] = useState<string | null>(null);

  /**
   * Shows an outcome and clears it.
   *
   * An `undo` gets six seconds rather than four: long enough to read the sentence and decide,
   * short enough that nobody trusts it to still be there later.
   */
  const say = useCallback((message: ToastMessage) => {
    window.clearTimeout(dismissToast.current);
    toastNow.current = message;
    setToast(message);
    dismissToast.current = window.setTimeout(
      () => {
        if (toastNow.current !== message) return;
        toastNow.current = null;
        setToast(null);
      },
      message.undo ? TIMING.undoMs : TIMING.toastMs,
    );
  }, []);

  const tell = useCallback((text: string) => say({ text }), [say]);

  /** Turns a failed action into words for whoever took it. An ended session has a screen of its own. */
  const report = useCallback(
    (error: unknown, audience: Audience) => {
      const failure = failureOf(error);
      if (needsRelaunch(failure)) return;
      haptics.error();
      tell(messageFor(failure, audience));
    },
    [tell],
  );

  // ---- reads ------------------------------------------------------------------------------------

  /**
   * Asks by itself — a refresh, a reread after a write — only while the session stands. Once it has
   * ended only a retry somebody taps asks: anything else spun over the screen that says to reopen
   * the app, and met the same refusal.
   */
  const refresh = useCallback(
    (load: () => Promise<void> | void) => {
      if (sessionEndNow.current === null) void load();
    },
    [sessionEndNow],
  );

  const sessionRead = useRead<Session>(
    api ? { key: SESSION, ask: () => api.session() } : null,
    (next) => {
      setServiceDate((current) => current ?? next.bookable_days[0] ?? next.bar.today);
      setShiftDate((current) => current ?? next.bar.today);
    },
  );
  const { load: loadSession, put: putSession } = sessionRead;
  const session = sessionRead.value;

  // The guest's cancel sheet follows the bookings on screen, whether a read or a write brought them.
  useEffect(() => {
    if (session) setSheet((current) => refreshedGuestSheet(current, session.bookings));
  }, [session]);

  /** Puts what a guest write answered on screen at once; a reread afterwards only freshens it. */
  const amendSession = (change: (current: Session) => Session) =>
    putSession(SESSION, (current) => current && change(current));

  useEffect(() => {
    if (!api) return;
    void loadSession();
  }, [api, loadSession]);

  // The rail depends only on how many are coming, so it is fetched when that changes and not when
  // a different day on the rail is tapped.
  const daysRead = useRead<DayOffer[]>(
    api ? { key: String(partySize), ask: async () => (await api.days(partySize)).days } : null,
    nothing,
  );
  const loadDays = daysRead.load;

  useEffect(() => {
    if (screen !== "book") return;
    void loadDays();
  }, [screen, partySize, api, loadDays]);

  // The time grid recomputes whenever the question changes. Every answer comes from the server,
  // which has run the real allocator: a time shown as free is a time with a table behind it, and
  // what a booking then would replace is the server's word too.
  const timesRead = useRead<GuestAvailability>(
    api && serviceDate !== null
      ? {
          key: `${serviceDate}|${partySize}`,
          ask: () => api.availability(serviceDate, partySize),
        }
      : null,
    nothing,
  );
  const loadTimes = timesRead.load;

  useEffect(() => {
    if (screen !== "book") return;
    void loadTimes();
  }, [screen, serviceDate, partySize, api, loadTimes]);

  const shiftRead = useRead<ShiftView>(
    api && shiftDate !== null ? { key: shiftDate, ask: () => api.shift(shiftDate) } : null,
    nothing,
    roomOrder,
  );
  const { load: loadShift, put: putShift, mark: markShift } = shiftRead;
  // Only the evening asked for is shown: another evening is not a refresh of this one.
  const shiftOnScreen = shiftRead.value;

  // Every sheet open on the evening on screen follows the room it now has, whether a read or a write
  // brought it.
  useEffect(() => {
    if (shiftOnScreen) setSheet((current) => refreshedSheet(current, shiftOnScreen));
  }, [shiftOnScreen]);

  // The settings count each table's bookings from this evening too.
  useEffect(() => {
    if ((tab !== "shift" && tab !== "settings") || shiftDate === null) return;
    void loadShift();
  }, [tab, shiftDate, api, loadShift]);

  useWhileVisible(
    useMemo(
      () => (tab === "shift" && shiftDate !== null ? () => refresh(loadShift) : null),
      [tab, shiftDate, loadShift, refresh],
    ),
    SHIFT_REFRESH_MS,
  );
  useWhileVisible(
    useMemo(
      () => (tab === "client" && screen === "home" ? () => refresh(loadSession) : null),
      [tab, screen, loadSession, refresh],
    ),
    HOME_REFRESH_MS,
  );

  // Folded into the edit at the moment it lands, so nothing typed while it loaded is lost. A read
  // landing while a save is on its way may or may not have seen that save, so it is dropped and
  // asked again once the save has answered. What it did to the edit waits on the settings screen:
  // it may land while the shift is on screen, where a toast would be gone before anybody came back.
  const settingsRead = useRead<SettingsView>(
    api ? { key: SETTINGS, ask: () => api.settings() } : null,
    (next) => {
      if (savingNow.current) {
        settingsWanted.current = true;
        return;
      }
      const outcome = received(pairNow.current, next);
      changePair(() => outcome.pair);
      if (outcome.notice) setFolded(outcome.notice);
    },
    settingsOrder,
  );
  const { load: readSettings, put: putSettings, mark: markSettings } = settingsRead;

  /** Reads the settings now, or once the save on its way has answered. */
  const loadSettings = useCallback(() => {
    if (!savingNow.current) {
      void readSettings();
      return;
    }
    settingsWanted.current = true;
  }, [readSettings, savingNow]);

  const settingsDirty = useMemo(() => pair !== null && isDirty(pair), [pair]);

  useEffect(() => {
    if (tab !== "settings") return;
    loadSettings();
  }, [tab, api, loadSettings]);

  // Closing Telegram with unsaved settings asks first, the way switching tabs no longer loses them.
  useEffect(() => {
    const app = webApp();
    if (settingsDirty) app?.enableClosingConfirmation?.();
    else app?.disableClosingConfirmation?.();
  }, [settingsDirty]);

  const editDraft = useCallback(
    (change: Edit) => {
      editsDuringSave.current?.push(change);
      setRefusal(null);
      setFolded(null);
      changePair((current) => current && { ...current, draft: edited(current.draft, change) });
    },
    [changePair],
  );

  const revertDraft = useCallback(() => {
    if (editsDuringSave.current) editsDuringSave.current = [];
    setRefusal(null);
    setFolded(null);
    changePair((current) => current && { ...current, draft: draftOf(current.settings) });
  }, [changePair]);

  // Writing a booking down and moving one ask the same question, so there is one of it. A move
  // sets its own booking aside — shifting it half an hour must not mean giving up its table first
  // and hoping — and asks even once the booking is under way, when only its tables can change.
  const moving = sheet.kind === "move" ? sheet.booking : null;
  const asksTimes = sheet.kind === "manual" || moving !== null;
  const askParty = moving ? (move.party ?? moving.party_size) : manual.partySize;
  const askIgnoring = moving?.id;
  const staffTimesKey =
    shiftDate !== null && asksTimes ? `${shiftDate}|${askParty}|${askIgnoring ?? ""}` : null;

  const staffTimesRead = useRead<StaffAvailability>(
    api && shiftDate !== null && staffTimesKey !== null
      ? {
          key: staffTimesKey,
          ask: () => api.staffAvailability(shiftDate, askParty, askIgnoring),
        }
      : null,
    nothing,
  );
  const loadStaffTimes = staffTimesRead.load;

  useEffect(() => {
    if (staffTimesKey === null) return;
    void loadStaffTimes();
  }, [staffTimesKey, api, loadStaffTimes]);

  /**
   * One action at a time.
   *
   * On a slow connection the button a guest just pressed looks as if nothing happened, and they
   * press it again. For a message or a cancellation that is a second message or a confusing "not
   * found"; for a booking, a second request racing the first. A tap dropped for that reason says so,
   * or it reads as a button that does not work — except over a way back: replacing «Вернуть» with
   * «Подождите» takes away the undo of the tap before, so then it only buzzes.
   */
  const busy = useRef(false);
  const exclusive = useCallback(
    <Args extends unknown[]>(action: (...args: Args) => Promise<void>) =>
      async (...args: Args) => {
        if (busy.current) {
          haptics.warning();
          if (!toastNow.current?.undo) tell("Подождите — прошлое действие ещё выполняется.");
          return;
        }
        busy.current = true;
        try {
          await action(...args);
        } finally {
          busy.current = false;
        }
      },
    [tell],
  );

  // Telegram's own back button, where there is one, rather than a second one drawn in the page. It
  // steps back through whatever is open, innermost first — on Android the hardware back button is
  // this button, and without it the whole app closed and took the open sheet with it.
  useEffect(() => {
    const back = webApp()?.BackButton;
    if (!back) return undefined;
    const goBack =
      sheet.kind !== "none"
        ? () => setSheet(NO_SHEET)
        : tab === "client" && screen === "book"
          ? () => setScreen("home")
          : tab === "settings" && settingsSection !== null
            ? () => setSettingsSection(null)
            : null;
    if (!goBack) {
      back.hide();
      return undefined;
    }
    back.onClick(goBack);
    back.show();
    return () => back.offClick(goBack);
  }, [tab, screen, sheet.kind, settingsSection]);

  if (token === null) {
    return (
      <InsetFrame insets={insets}>
        <Failure message={messageFor({ code: "no_credentials", message: "" }, "guest")} />
      </InsetFrame>
    );
  }
  // Only a read of the session asked after it ended can bring the app back, so only that read may
  // spin over the screen that says so.
  const blocking = sessionEnd
    ? sessionEnd.retrying
      ? null
      : sessionEnd.failure
    : session === null
      ? sessionRead.failure
      : null;
  if (blocking) {
    // Retrying with the proof the server just refused refuses again. Only reopening from Telegram
    // brings a new one, so that is the way out offered.
    const telegram = webApp();
    const relaunch = needsRelaunch(blocking) && telegram !== undefined;
    return (
      <InsetFrame insets={insets}>
        <Failure
          message={messageFor(blocking, "guest")}
          actionLabel={relaunch ? "Закрыть" : "Попробовать снова"}
          onAction={() => (relaunch ? telegram.close() : void loadSession())}
        />
      </InsetFrame>
    );
  }
  if (sessionEnd || !session || !api || serviceDate === null || shiftDate === null) {
    return (
      <InsetFrame insets={insets}>
        <Spinner label="Открываем" />
      </InsetFrame>
    );
  }

  const bar = session.bar;
  const closeSheet = () => setSheet(NO_SHEET);
  /** Closes the sheet an action was started from, and no sheet opened since. */
  const closeIfStill = (from: OpenSheet) => setSheet((current) => closedIfStill(current, from));
  // The server's day, not the one this phone read when it opened: a shift left open overnight.
  const today = shiftOnScreen?.today ?? bar.today;
  // ISO dates compare as strings. An evening that is over is read, not written into.
  const isPast = shiftDate < today;
  // The time chosen, and what booking it does, come from the answer to the question on screen: the
  // answer drawn while another loads was for another evening or party.
  const timesOnScreen = timesRead.value;
  const chosen = chosenTime(timesOnScreen, chosenMinutes);

  // ---- guest actions --------------------------------------------------------------------------

  const openPicker = (booking: GuestBooking | null = null) => {
    // The party the home card spoke for, so the picker opens on the promise the card just made.
    setPartySize(booking?.party_size ?? session.today_free_for_party);
    setServiceDate(pickerStart(session, booking));
    setChosenMinutes(null);
    setScreen("book");
  };

  const book = exclusive(async () => {
    if (chosen === null || timesOnScreen === null) return;
    try {
      // What the button said this booking replaces. The server refuses rather than replace otherwise.
      const answer = await api.book(serviceDate, chosen, partySize, timesOnScreen.replacing);
      haptics.success();
      setTaken({ booking: answer.booking, moved: answer.replaced.length > 0 });
      // The answer already says what was booked. Showing it does not wait on rereading the home
      // screen, whose failure must never turn a booking that happened into an error.
      amendSession((current) => ({
        ...current,
        bookings: heldAfter(current.bookings, answer.replaced, answer.booking),
      }));
      setScreen("done");
      refresh(loadSession);
    } catch (error) {
      report(error, "guest");
      const { code } = failureOf(error);
      if (code === "booking_changed") {
        // What the guest holds changed since the button was drawn. The picker stays as it is; the
        // button redraws from the times as they stand now, for them to look at and press again.
        refresh(loadSession);
        refresh(loadDays);
        refresh(loadTimes);
        return;
      }
      // Refused for what the guest already holds: the labels were drawn from bookings that changed.
      if (code === "already_booked_tonight") refresh(loadSession);
      // The refusal is usually "somebody just took it", so the picker is refreshed rather than left
      // showing a time that no longer exists.
      setChosenMinutes(null);
      refresh(loadTimes);
      refresh(loadDays);
    }
  });

  // No undo: the confirmation sheet was the protection. Booking the slot again could replace another
  // booking the guest holds, and could never bring back one that had begun.
  const cancelMine = exclusive(async (from: OpenSheet, was: GuestBooking) => {
    try {
      await api.cancelMine(was.id);
      haptics.success();
      closeIfStill(from);
      amendSession((current) => ({
        ...current,
        bookings: heldAfter(current.bookings, [was.id], null),
      }));
      tell("Бронь отменена. Стол снова свободен.");
      refresh(loadSession);
    } catch (error) {
      report(error, "guest");
    }
  });

  const enableReminders = exclusive(async () => {
    try {
      const reminders = await api.optInToReminders();
      amendSession((current) => ({ ...current, reminders }));
      refresh(loadSession);
      openBotChat(process.env.NEXT_PUBLIC_BOT_USERNAME ?? "");
      tell(`Напомним за ${fmt.hoursWord(bar.remind_hours)} до брони.`);
    } catch (error) {
      report(error, "guest");
    }
  });

  const dismissReminders = exclusive(async () => {
    try {
      const reminders = await api.dismissReminderPrompt();
      amendSession((current) => ({ ...current, reminders }));
      refresh(loadSession);
    } catch (error) {
      report(error, "guest");
    }
  });

  const decision = bookingDecision(partySize, serviceDate, bar, chosen, timesOnScreen);
  // A guest holding a plan moves it from its card; one holding only a table tonight, or nothing,
  // books another evening from here.
  const holdsPlan = session.bookings.some((held) => held.rebooking_replaces === "any_evening");
  const guestFooter =
    screen === "done" ? (
      <MainButton label="На главную" onClick={() => setScreen("home")} />
    ) : screen === "book" ? (
      <MainButton label={decision.label} enabled={decision.enabled} onClick={() => void book()} />
    ) : holdsPlan ? null : (
      <MainButton label="Забронировать стол" onClick={() => openPicker()} />
    );

  // ---- staff actions -------------------------------------------------------------------------

  /**
   * Sends a staff write and puts the evening it answered with on record for that evening.
   *
   * The answer is the room after the write, every booking in it, so a party the server reseated is
   * shown where it now sits, and a sheet on a booking that is gone closes. It is numbered when the
   * write is sent: a room of the same version read after that is fresher, one read before is not.
   */
  const writeShift = async <A extends { shift: ShiftView }>(send: () => Promise<A>): Promise<A> => {
    const sent = markShift();
    const answer = await send();
    putShift(answer.shift.service_date, () => answer.shift, sent);
    return answer;
  };

  /**
   * One tap, applied at once, with the way back attached.
   *
   * The undo goes back to the attendance the server says the booking had just before the change,
   * so taking back a mistake restores the room — even when a colleague changed it a moment earlier
   * and this phone had not heard yet.
   */
  const setAttendance = exclusive(
    async (booking: ShiftBooking, attendance: Attendance, undoable = true) => {
      try {
        const answer = await writeShift(() => api.setAttendance(booking.id, attendance));
        haptics.success();
        say({
          text: attendanceOutcome(answer.booking, attendance),
          ...(undoable && answer.previous !== attendance
            ? {
                undo: {
                  label: "Вернуть",
                  run: () => void setAttendance(answer.booking, answer.previous, false),
                },
              }
            : {}),
        });
      } catch (error) {
        report(error, "staff");
      }
    },
  );

  const seat = (booking: ShiftBooking) => void setAttendance(booking, "arrived");
  const markLeft = (booking: ShiftBooking) => void setAttendance(booking, "left");

  const setNote = exclusive(async (booking: ShiftBooking, note: string | null) => {
    try {
      await writeShift(() => api.setNote(booking.id, note));
    } catch (error) {
      report(error, "staff");
    }
  });

  const cancelAsStaff = exclusive(
    async (from: OpenSheet, booking: ShiftBooking, reason: string) => {
      try {
        const answer = await writeShift(() => api.cancelAsStaff(booking.id, reason));
        const told = answer.guest_notified
          ? `${booking.guest_name} получил сообщение с причиной.`
          : `${booking.guest_name} не получит сообщения — предупредите его сами.`;
        closeIfStill(from);
        say({
          text: [`Бронь отменена. ${told}`, reconciliationReport(answer.reconciliation)]
            .filter(Boolean)
            .join(" "),
        });
      } catch (error) {
        report(error, "staff");
      }
    },
  );

  const sendTemplate = exclusive(async (from: OpenSheet, booking: ShiftBooking, text: string) => {
    try {
      await api.sendTemplate(booking.id, text);
      closeIfStill(from);
      tell(`Отправлено ${booking.guest_name}: «${text}»`);
    } catch (error) {
      report(error, "staff");
      // The room on screen said the bot could reach the guest. Read again, it says it cannot.
      if (failureOf(error).code === "no_bot_chat") refresh(loadShift);
    }
  });

  /** What a rearrangement did, per booking, by name — never a count of what it hoped to do. */
  const rearranged = (
    reconciliation: Reconciliation,
    lead: string,
    whenNothingMoved: string,
    orphanLead?: string,
  ) => {
    const summary = reconciliationReport(reconciliation, orphanLead);
    return summary.length > 0 ? `${lead} ${summary}`.trim() : whenNothingMoved;
  };

  /**
   * Closing tables is as reversible as opening them: the way back opens exactly the tables this tap
   * closed, as the server counted them — not one a colleague had already closed.
   */
  const closeTables = exclusive(async (from: OpenSheet, closures: Closure[], number: number) => {
    closeIfStill(from);
    const closed: string[] = [];
    const moved: Reconciliation = { moved: [], orphaned: [] };
    try {
      for (const closure of closures) {
        const answer = await writeShift(() =>
          api.blockTables(shiftDate, closure.tableIds, closure.reason),
        );
        closed.push(...answer.closed);
        moved.moved.push(...answer.reconciliation.moved);
        moved.orphaned.push(...answer.reconciliation.orphaned);
      }
    } catch (error) {
      report(error, "staff");
      return;
    }
    const text = rearranged(
      moved,
      `Стол ${number} закрыт на вечер.`,
      `Стол ${number} закрыт на вечер. Броней там не было.`,
      "Остались без стола",
    );
    say(
      closed.length > 0
        ? { text, undo: { label: "Вернуть", run: () => void openTables(NO_SHEET, closed, number) } }
        : { text },
    );
  });

  /** Opening tables comes back as closures the server removed, each with the reason it had. */
  const openTables = exclusive(async (from: OpenSheet, tableIds: string[], number: number) => {
    closeIfStill(from);
    try {
      const answer = await writeShift(() => api.unblockTables(shiftDate, tableIds));
      const text = rearranged(
        answer.reconciliation,
        `Стол ${number} снова в подборе.`,
        `Стол ${number} снова в подборе.`,
      );
      const restore = closuresToRestore(answer.reopened);
      say(
        restore.length > 0
          ? { text, undo: { label: "Вернуть", run: () => void closeTables(NO_SHEET, restore, number) } }
          : { text },
      );
    } catch (error) {
      report(error, "staff");
    }
  });

  const findTables = exclusive(async (from: OpenSheet) => {
    closeIfStill(from);
    try {
      const answer = await writeShift(() => api.reconcileShift(shiftDate));
      tell(
        rearranged(
          answer.reconciliation,
          "",
          "Свободных столов на это время нет. Откройте закрытый стол или предложите другое время.",
        ),
      );
    } catch (error) {
      report(error, "staff");
    }
  });

  const seatWalkIn = exclusive(async (from: OpenSheet, tableId: string) => {
    try {
      const answer = await writeShift(() => api.seatWalkIn(shiftDate, walkInParty, tableId));
      haptics.success();
      closeIfStill(from);
      tell(`Посадили за стол ${answer.booking.table_number}.`);
    } catch (error) {
      report(error, "staff");
      // A refusal carries no room, and it usually means the room on screen is behind.
      refresh(loadShift);
    }
  });

  const createManualBooking = exclusive(async (from: OpenSheet, tableId: string) => {
    const sent = manual;
    const minutes = sent.minutes;
    if (minutes === null) return;
    try {
      const answer = await writeShift(() =>
        api.createStaffBooking(shiftDate, minutes, sent.partySize, sent.name, tableId),
      );
      // A form already holding the next guest is not this one's to clear.
      setManual((current) => (same(current, sent) ? EMPTY_MANUAL : current));
      closeIfStill(from);
      const created = answer.booking;
      tell(
        `${created.guest_name} записан на ${fmt.time(created.start_minutes)}, стол ${created.table_number}.`,
      );
    } catch (error) {
      report(error, "staff");
      refresh(loadStaffTimes);
    }
  });

  /** The report says whether the guest was told: not knowing means sending a second message. */
  const moveBooking = exclusive(
    async (
      from: OpenSheet,
      booking: ShiftBooking,
      minutes: number,
      tableId: string,
      party: number,
    ) => {
      try {
        const answer = await writeShift(() => api.moveBooking(booking.id, minutes, tableId, party));
        haptics.success();
        closeIfStill(from);
        const now = answer.booking;
        const where = `стол ${now.table_number}`;
        const told =
          now.start_minutes === booking.start_minutes
            ? now.party_size === booking.party_size
              ? `${now.guest_name} за ${where}.`
              : `${now.guest_name}: ${fmt.guests(now.party_size)}, ${where}.`
            : `${now.guest_name}: ${fmt.time(now.start_minutes)}, ${where}. ` +
              (answer.guest_notified
                ? "Гостю сообщили."
                : "Бот не может написать гостю — предупредите сами.");
        say({
          text: [told, reconciliationReport(answer.reconciliation)].filter(Boolean).join(" "),
        });
      } catch (error) {
        report(error, "staff");
        refresh(loadShift);
      }
    },
  );

  const saveSettings = exclusive(async () => {
    const sent = pairNow.current?.draft;
    if (!sent) return;
    const openedBefore = openings.current;
    setRefusal(null);
    setFolded(null);
    changeSaving(() => true);
    editsDuringSave.current = [];
    const asked = markSettings();
    try {
      const saved = await api.saveSettings(sent);
      const meanwhile = editsDuringSave.current ?? [];
      changePair((current) => savedInto(current, saved.settings, meanwhile));
      // On record as an answer too: a reread that failed before this save is no longer news.
      putSettings(SETTINGS, () => saved.settings, asked);
      const parts = ["Настройки сохранены.", reconciliationReport(saved.reconciliation)];
      if (saved.above_cap > 0) {
        parts.push(`${fmt.bookings(saved.above_cap)} больше нового лимита — они остаются в силе.`);
      }
      tell(parts.filter(Boolean).join(" "));
      refresh(loadSession);
    } catch (error) {
      report(error, "staff");
      const failure = failureOf(error);
      const refused = refusalOf(failure);
      if (refused) {
        // Kept on the save bar. A sheet opened while the save was on its way is somebody's next
        // decision, so the reasons wait there rather than being thrown over it.
        setRefusal(refused);
        if (openings.current === openedBefore) openSheet({ kind: "conflict", refusal: refused });
      } else if (failure.code === "settings_changed") {
        refresh(loadSettings);
      }
    } finally {
      editsDuringSave.current = null;
      changeSaving(() => false);
      const wanted = settingsWanted.current;
      settingsWanted.current = false;
      if (wanted) refresh(readSettings);
    }
  });

  // ---- what the shell is given ------------------------------------------------------------------

  const staffFooter =
    tab === "shift" && shiftOnScreen !== null && !shiftOnScreen.hours.closed && !isPast ? (
      <ShiftActions
        seatsNow={shiftOnScreen.walk_in_until_minutes !== null}
        onWalkIn={() => {
          setWalkInParty(DEFAULT_PARTY);
          setWalkInTable(null);
          openSheet({ kind: "walkIn" });
        }}
        onManual={() => openSheet({ kind: "manual" })}
      />
    ) : tab === "settings" && pair !== null && settingsDirty ? (
      <SaveBar
        reason={firstReason(pair.draft, pair.settings.limits)}
        saving={saving}
        onSave={() => void saveSettings()}
        onRevert={revertDraft}
        onWhy={refusal ? () => openSheet({ kind: "conflict", refusal }) : null}
      />
    ) : undefined;

  const footer = tab === "client" ? guestFooter : staffFooter;

  return (
    <AppShell
      staff={session.is_staff}
      tab={tab}
      onTab={setTab}
      insets={insets}
      {...(footer ? { footer } : {})}
      toast={<Toast message={toast} />}
      sheet={
        <>
          <BookingSheet
            open={sheet.kind === "booking"}
            booking={sheet.kind === "booking" ? sheet.booking : null}
            nowMinutes={shiftOnScreen?.now_minutes ?? null}
            graceMinutes={bar.grace_minutes}
            onClose={closeSheet}
            onAttendance={(attendance) => {
              if (sheet.kind !== "booking") return;
              void setAttendance(sheet.booking, attendance);
            }}
            onNote={(note) => {
              if (sheet.kind === "booking") void setNote(sheet.booking, note);
            }}
            onOpenTemplates={() => {
              if (sheet.kind !== "booking") return;
              openSheet({ kind: "templates", booking: sheet.booking });
            }}
            onOpenCancel={() => {
              if (sheet.kind === "booking") openSheet({ kind: "cancelBooking", booking: sheet.booking });
            }}
            onOpenMove={() => {
              if (sheet.kind !== "booking") return;
              setMove({ minutes: null, table: null, party: null });
              openSheet({ kind: "move", booking: sheet.booking });
            }}
            onFindTable={() => void findTables(sheet)}
          />

          <MessageSheet
            open={sheet.kind === "templates"}
            booking={sheet.kind === "templates" ? sheet.booking : null}
            templates={shiftOnScreen?.message_templates ?? []}
            onClose={closeSheet}
            onChoose={(text) => {
              if (sheet.kind === "templates") void sendTemplate(sheet, sheet.booking, text);
            }}
          />

          <CancelReasonSheet
            open={sheet.kind === "cancelBooking"}
            booking={sheet.kind === "cancelBooking" ? sheet.booking : null}
            reasons={shiftOnScreen?.cancel_reasons ?? []}
            onClose={closeSheet}
            onChoose={(reason) => {
              if (sheet.kind === "cancelBooking") void cancelAsStaff(sheet, sheet.booking, reason);
            }}
          />

          <TableSheet
            key={sheet.kind === "table" ? sheet.table.id : "no-table"}
            open={sheet.kind === "table"}
            table={sheet.kind === "table" ? sheet.table : null}
            shift={shiftOnScreen}
            onClose={closeSheet}
            onBlock={(tableIds, reason) => {
              if (sheet.kind === "table") {
                void closeTables(sheet, [{ tableIds, reason }], sheet.table.number);
              }
            }}
            onUnblock={(tableIds) => {
              if (sheet.kind === "table") void openTables(sheet, tableIds, sheet.table.number);
            }}
          />

          <DaySheet
            open={sheet.kind === "days"}
            days={shiftOnScreen?.days ?? []}
            today={today}
            serviceDate={shiftDate}
            guestHorizonDays={shiftOnScreen?.guest_horizon_days ?? 0}
            onClose={closeSheet}
            onChoose={(date) => {
              closeSheet();
              setShiftDate(date);
            }}
          />

          <WalkInSheet
            open={sheet.kind === "walkIn"}
            shift={shiftOnScreen}
            maxParty={bar.max_party}
            partySize={walkInParty}
            chosenTableId={walkInTable}
            onClose={closeSheet}
            onPartySize={setWalkInParty}
            onChooseTable={setWalkInTable}
            onSeat={(tableId) => void seatWalkIn(sheet, tableId)}
          />

          <ManualBookingSheet
            open={sheet.kind === "manual"}
            shift={shiftOnScreen}
            maxParty={bar.max_party}
            availability={staffTimesRead.drawn}
            partySize={manual.partySize}
            chosenMinutes={manual.minutes}
            chosenTableId={manual.table}
            guestName={manual.name}
            loadFailure={staffTimesRead.failure}
            timesPending={staffTimesRead.pending}
            onClose={closeSheet}
            onPartySize={(size) =>
              setManual((current) => ({ ...current, partySize: size, minutes: null }))
            }
            onPick={(minutes) => setManual((current) => ({ ...current, minutes }))}
            onTakenSlot={() => tell("Это время занято. Свободное — без зачёркивания.")}
            onChooseTable={(table) => setManual((current) => ({ ...current, table }))}
            onGuestName={(name) => setManual((current) => ({ ...current, name }))}
            onRetry={() => void loadStaffTimes()}
            onCreate={(tableId) => void createManualBooking(sheet, tableId)}
          />

          <MoveBookingSheet
            open={sheet.kind === "move"}
            booking={moving}
            shift={shiftOnScreen}
            turnMinutes={bar.turn_minutes}
            maxParty={bar.max_party}
            partySize={move.party ?? moving?.party_size ?? DEFAULT_PARTY}
            availability={staffTimesRead.drawn}
            chosenMinutes={move.minutes}
            chosenTableId={move.table}
            loadFailure={staffTimesRead.failure}
            timesPending={staffTimesRead.pending}
            onClose={closeSheet}
            // A table chosen for the old party may not seat the new one, so the choice goes back to
            // the room's own best fit.
            onPartySize={(party) => setMove((current) => ({ ...current, party, table: null }))}
            onPick={(minutes) => setMove((current) => ({ ...current, minutes }))}
            onTakenSlot={() => tell("Это время занято. Свободное — без зачёркивания.")}
            onChooseTable={(table) => setMove((current) => ({ ...current, table }))}
            onRetry={() => void loadStaffTimes()}
            onMove={(minutes, tableId, party) => {
              if (moving) void moveBooking(sheet, moving, minutes, tableId, party);
            }}
          />

          <GuestCancelSheet
            open={sheet.kind === "guestCancel"}
            booking={sheet.kind === "guestCancel" ? sheet.booking : null}
            today={bar.today}
            onClose={closeSheet}
            onConfirm={() => {
              if (sheet.kind === "guestCancel") void cancelMine(sheet, sheet.booking);
            }}
          />

          <ConflictSheet
            open={sheet.kind === "conflict"}
            refusal={sheet.kind === "conflict" ? sheet.refusal : null}
            onClose={closeSheet}
          />
        </>
      }
    >
      {tab === "client" && screen === "home" ? (
        <HomeScreen
          session={session}
          onMove={openPicker}
          onCancel={(booking) => openSheet({ kind: "guestCancel", booking })}
          onEnableReminders={() => void enableReminders()}
          onDismissReminders={() => void dismissReminders()}
          onContact={openContact}
        />
      ) : null}

      {tab === "client" && screen === "book" ? (
        <BookScreen
          bar={bar}
          days={daysRead.drawn}
          availability={timesRead.drawn}
          partySize={partySize}
          serviceDate={serviceDate}
          chosenMinutes={chosen}
          daysFailure={daysRead.failure}
          timesFailure={timesRead.failure}
          timesPending={timesRead.pending}
          onPartySize={(size) => {
            setPartySize(size);
            setChosenMinutes(null);
          }}
          onServiceDate={(date) => {
            setServiceDate(date);
            setChosenMinutes(null);
          }}
          onPick={setChosenMinutes}
          onTakenSlot={() => tell("Это время занято. Свободное — без зачёркивания.")}
          onRetry={() => {
            void loadDays();
            void loadTimes();
          }}
          {...(webApp()?.BackButton ? {} : { onBack: () => setScreen("home") })}
        />
      ) : null}

      {tab === "client" && screen === "done" && taken ? (
        <DoneScreen booking={taken.booking} bar={bar} moved={taken.moved} />
      ) : null}

      {tab === "shift" ? (
        <ReadView
          value={shiftOnScreen}
          failure={shiftRead.failure}
          audience="staff"
          generic="Не удалось прочитать смену."
          loading="Читаем смену"
          onRetry={() => void loadShift()}
          padding={`${SPACE[3]}px ${SPACE[3]}px 0`}
        >
          {(room) => (
            <ShiftScreen
              shift={room}
              today={room.today}
              graceMinutes={bar.grace_minutes}
              pane={pane}
              onPane={setPane}
              onServiceDate={setShiftDate}
              onOpenDays={() => openSheet({ kind: "days" })}
              onOpenTable={(table) => openSheet({ kind: "table", table })}
              actions={{
                onOpen: (booking) => openSheet({ kind: "booking", booking }),
                onSeat: seat,
                onLeft: markLeft,
                onFindTable: () => void findTables(NO_SHEET),
              }}
            />
          )}
        </ReadView>
      ) : null}

      {tab === "settings" ? (
        <ReadView
          value={pair}
          failure={settingsRead.failure}
          audience="staff"
          generic="Не удалось прочитать настройки."
          loading="Читаем настройки"
          onRetry={() => loadSettings()}
          padding={`${SPACE[3]}px ${SPACE[4]}px 0`}
        >
          {(shown) => (
            <>
              {folded !== null && settingsDirty ? (
                <div style={{ padding: `${SPACE[3]}px ${SPACE[4]}px 0` }}>
                  <Card gap={SPACE[2]}>
                    <Note tone="warn">{folded}</Note>
                  </Card>
                </div>
              ) : null}
              <SettingsScreen
                settings={shown.settings}
                draft={shown.draft}
                room={shiftOnScreen}
                editedWeekday={editedWeekday}
                onEdit={editDraft}
                onEditWeekday={setEditedWeekday}
                section={settingsSection}
                onSection={setSettingsSection}
              />
            </>
          )}
        </ReadView>
      ) : null}
    </AppShell>
  );
}
