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
  type Attendance,
  type Availability,
  type DayOffer,
  type GuestBooking,
  type Reconciliation,
  type Session,
  type SettingsView,
  type ShiftBooking,
  type ShiftView,
} from "@/lib/api";
import {
  invalidReasons,
  messageFor,
  needsRelaunch,
  strandedBookings,
  type ApiFailure,
} from "@/lib/errors";
import * as fmt from "@/lib/format";
import {
  attendanceOutcome,
  closuresToRestore,
  reconciliationReport,
  strandedLines,
  type Closure,
} from "@/lib/outcomes";
import { edited, firstReason, type Edit } from "@/lib/settingsRules";
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
import { TIMING } from "@/lib/tokens";
import { failureOf, useRead } from "@/lib/useRead";
import { ShiftActions, ShiftScreen, type ShiftPane } from "@/components/AdminShift";
import { AppShell, InsetFrame, type StaffTab } from "@/components/AppChrome";
import {
  BookScreen,
  DoneScreen,
  HomeScreen,
  bookingDecision,
  heldAfter,
  heldOn,
  pickerStart,
} from "@/components/GuestScreens";
import { SaveBar, SettingsScreen, type Section } from "@/components/Settings";
import { useInsets } from "@/components/ThemeProvider";
import {
  BookingSheet,
  CancelReasonSheet,
  ChoiceSheet,
  ConflictSheet,
  DaySheet,
  GuestCancelSheet,
  ManualBookingSheet,
  MoveBookingSheet,
  TableSheet,
  WalkInSheet,
} from "@/components/Sheets";
import { Failure, MainButton, Spinner, Toast, type ToastMessage } from "@/components/ui";

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
  // Read after mounting, never while rendering. The page is prerendered where there is no Telegram
  // at all, and a first render that decided "no payload" wrote «Откройте приложение из Telegram»
  // into the HTML every guest saw until the scripts had loaded.
  const [token, setToken] = useState<string | null | undefined>(undefined);
  useEffect(() => setToken(credentials()), []);
  const api = useMemo(() => (token ? makeClient(token) : null), [token]);

  const [toast, setToast] = useState<ToastMessage | null>(null);
  const toastNow = useRef<ToastMessage | null>(null);
  const dismissToast = useRef(0);

  const [tab, setTab] = useState<Tab>("client");
  const [screen, setScreen] = useState<GuestScreen>("home");
  const [taken, setTaken] = useState<{ booking: GuestBooking; moved: boolean } | null>(null);

  const [partySize, setPartySize] = useState(DEFAULT_PARTY);
  const [serviceDate, setServiceDate] = useState<string | null>(null);
  const [chosenMinutes, setChosenMinutes] = useState<number | null>(null);
  const [availability, setAvailability] = useState<Availability | null>(null);
  const [dayRail, setDayRail] = useState<DayOffer[] | null>(null);

  const [shiftDate, setShiftDate] = useState<string | null>(null);
  const [pane, setPane] = useState<ShiftPane>("now");
  const [shift, setShift] = useState<ShiftView | null>(null);

  const [session, changeSession, sessionNow] = useSynced<Session | null>(null);
  // When the session was last found dead: only a session read answered after that brings it back.
  const [deadAt, setDeadAt] = useState(0);

  const [pair, changePair, pairNow] = useSynced<SettingsPair | null>(null);
  // The edits made while a save is on its way, to be made again on top of what it stored.
  const editsDuringSave = useRef<Edit[] | null>(null);
  const [settingsSection, setSettingsSection] = useState<Section | null>(null);
  const [editedWeekday, setEditedWeekday] = useState(1);
  const [saving, setSaving] = useState(false);

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
  const [staffTimes, setStaffTimes] = useState<Availability | null>(null);
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

  // ---- reads ------------------------------------------------------------------------------------

  /**
   * Turns a failure into words for whoever is looking at it, when it is worth any.
   *
   * A failure only relaunching can fix is always recorded, however quietly it came: the session is
   * dead, and the screen says so until a session read answers after it.
   */
  const reportRef = useRef<(failure: ApiFailure, audience: "guest" | "staff", tell?: boolean) => void>(
    () => {},
  );
  const report = useCallback(
    (failure: ApiFailure, audience: "guest" | "staff", tellIt = true) =>
      reportRef.current(failure, audience, tellIt),
    [],
  );

  const showSession = (next: Session) => {
    changeSession(() => next);
    setSheet((current) => refreshedGuestSheet(current, next.bookings));
  };
  const sessionRead = useRead<Session>(
    api ? { key: SESSION, ask: () => api.session() } : null,
    (next) => {
      showSession(next);
      setServiceDate((current) => current ?? next.bookable_days[0] ?? next.bar.today);
      setShiftDate((current) => current ?? next.bar.today);
    },
    (failure, tellIt) => report(failure, "guest", tellIt),
  );
  const { load: loadSession, put: putSession, failNow: sessionFailed } = sessionRead;

  useEffect(() => {
    reportRef.current = (failure, audience, tellIt = true) => {
      if (needsRelaunch(failure)) {
        setDeadAt(sessionFailed(SESSION, failure));
        return;
      }
      if (!tellIt) return;
      haptics.error();
      tell(messageFor(failure, audience));
    };
  }, [sessionFailed, tell]);

  /** Puts what a guest write answered on screen at once; a reread afterwards only freshens it. */
  const amendSession = (change: (current: Session) => Session) =>
    putSession(SESSION, () => {
      const current = sessionNow.current;
      if (current) showSession(change(current));
    });

  useEffect(() => {
    if (!api) return;
    void loadSession();
  }, [api, loadSession]);

  // The rail depends only on how many are coming, so it is fetched when that changes and not when
  // a different day on the rail is tapped.
  const daysRead = useRead<DayOffer[]>(
    api ? { key: String(partySize), ask: async () => (await api.days(partySize)).days } : null,
    setDayRail,
    (failure, tellIt) => report(failure, "guest", tellIt),
  );
  const loadDays = daysRead.load;

  useEffect(() => {
    if (screen !== "book") return;
    void loadDays();
  }, [screen, partySize, api, loadDays]);

  // The time grid recomputes whenever the question changes. Every answer comes from the server,
  // which has run the real allocator: a time shown as free is a time with a table behind it.
  const timesRead = useRead<Availability>(
    api && serviceDate !== null
      ? {
          key: `${serviceDate}|${partySize}`,
          ask: () => api.availability(serviceDate, partySize),
        }
      : null,
    setAvailability,
    (failure, tellIt) => report(failure, "guest", tellIt),
  );
  const loadTimes = timesRead.load;

  useEffect(() => {
    if (screen !== "book") return;
    void loadTimes();
  }, [screen, serviceDate, partySize, api, loadTimes]);

  /** A shift the server answered, and every sheet brought up to date with it. */
  const showShift = (next: ShiftView) => {
    setShift(next);
    setSheet((current) => refreshedSheet(current, next));
  };
  const shiftRead = useRead<ShiftView>(
    api && shiftDate !== null ? { key: shiftDate, ask: () => api.shift(shiftDate) } : null,
    showShift,
    (failure, tellIt) => report(failure, "staff", tellIt),
  );
  const { load: loadShift, put: putShift } = shiftRead;

  useEffect(() => {
    if (tab !== "shift" || shiftDate === null) return;
    void loadShift();
  }, [tab, shiftDate, api, loadShift]);

  useWhileVisible(
    useMemo(
      () => (tab === "shift" && shiftDate !== null ? () => void loadShift(true) : null),
      [tab, shiftDate, loadShift],
    ),
    SHIFT_REFRESH_MS,
  );
  useWhileVisible(
    useMemo(
      () => (tab === "client" && screen === "home" ? () => void loadSession(true) : null),
      [tab, screen, loadSession],
    ),
    HOME_REFRESH_MS,
  );

  // Folded into the edit at the moment it lands, so nothing typed while it loaded is lost.
  const settingsRead = useRead<SettingsView>(
    api && shiftDate !== null ? { key: shiftDate, ask: () => api.settings(shiftDate) } : null,
    (next, date) => {
      const outcome = received(pairNow.current, date, next);
      changePair(() => outcome.pair);
      if (outcome.notice) tell(outcome.notice);
    },
    (failure, tellIt) => report(failure, "staff", tellIt),
  );
  const { load: loadSettings, put: putSettings } = settingsRead;

  const settingsDirty = pair !== null && isDirty(pair);

  useEffect(() => {
    if (tab !== "settings" || shiftDate === null) return;
    void loadSettings();
  }, [tab, shiftDate, api, loadSettings]);

  // Closing Telegram with unsaved settings asks first, the way switching tabs no longer loses them.
  useEffect(() => {
    const app = webApp();
    if (settingsDirty) app?.enableClosingConfirmation?.();
    else app?.disableClosingConfirmation?.();
  }, [settingsDirty]);

  const editDraft = useCallback(
    (change: Edit) => {
      editsDuringSave.current?.push(change);
      changePair((current) => current && { ...current, draft: edited(current.draft, change) });
    },
    [changePair],
  );

  const revertDraft = useCallback(() => {
    if (editsDuringSave.current) editsDuringSave.current = [];
    changePair((current) => current && { ...current, draft: current.base });
  }, [changePair]);

  // Writing a booking down and moving one ask the same question, so there is one of it. A move
  // sets its own booking aside — shifting it half an hour must not mean giving up its table first
  // and hoping — and a booking already under way is not asking at all: its time cannot change.
  const moving = sheet.kind === "move" ? sheet.booking : null;
  const movingTime = moving !== null && !moving.started;
  const asksTimes = sheet.kind === "manual" || movingTime;
  const askParty = moving ? (move.party ?? moving.party_size) : manual.partySize;
  const askIgnoring = movingTime && moving ? moving.id : undefined;
  const staffTimesKey =
    shiftDate !== null && asksTimes ? `${shiftDate}|${askParty}|${askIgnoring ?? ""}` : null;

  const staffTimesRead = useRead<Availability>(
    api && shiftDate !== null && staffTimesKey !== null
      ? {
          key: staffTimesKey,
          ask: () => api.staffAvailability(shiftDate, askParty, askIgnoring),
        }
      : null,
    setStaffTimes,
    (failure, tellIt) => report(failure, "staff", tellIt),
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
  const dead = deadAt > sessionRead.applied;
  const blocking = dead || session === null ? sessionRead.failure : null;
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
  if (dead || !session || !api || serviceDate === null || shiftDate === null) {
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
  // Only the evening asked for is shown: another evening is not a refresh of this one.
  const shiftOnScreen = shift !== null && shift.service_date === shiftDate ? shift : null;
  // The server's day, not the one this phone read when it opened: a shift left open overnight.
  const today = shiftOnScreen?.today ?? bar.today;
  const isToday = shiftDate === today;
  // ISO dates compare as strings. An evening that is over is read, not written into.
  const isPast = shiftDate < today;
  // An answer to another question must not stand in for one that failed.
  const days = daysRead.failure ? null : dayRail;
  const times = timesRead.failure ? null : availability;
  const shownStaffTimes = staffTimesRead.failure ? null : staffTimes;

  // ---- guest actions --------------------------------------------------------------------------

  const openPicker = (booking: GuestBooking | null = null) => {
    // The party the home card spoke for, so the picker opens on the promise the card just made.
    setPartySize(booking?.party_size ?? session.today_free_for_party);
    setServiceDate(pickerStart(session, booking));
    setChosenMinutes(null);
    setScreen("book");
  };

  const book = exclusive(async () => {
    if (chosenMinutes === null) return;
    try {
      const answer = await api.book(serviceDate, chosenMinutes, partySize);
      haptics.success();
      setTaken({ booking: answer.booking, moved: answer.replaced.length > 0 });
      // The answer already says what was booked. Showing it does not wait on rereading the home
      // screen, whose failure must never turn a booking that happened into an error.
      amendSession((current) => ({
        ...current,
        bookings: heldAfter(current.bookings, answer.replaced, answer.booking),
      }));
      setScreen("done");
      void loadSession(true);
    } catch (error) {
      report(failureOf(error), "guest");
      // The refusal is usually "somebody just took it", so the picker is refreshed rather than left
      // showing a time that no longer exists.
      setChosenMinutes(null);
      void loadTimes();
      void loadDays();
    }
  });

  /** Books the same slot again, for a guest who has just changed their mind about cancelling. */
  const rebook = exclusive(async (was: GuestBooking) => {
    try {
      const answer = await api.book(was.service_date, was.start_minutes, was.party_size);
      haptics.success();
      amendSession((current) => ({
        ...current,
        bookings: heldAfter(current.bookings, answer.replaced, answer.booking),
      }));
      tell("Бронь вернулась.");
    } catch (error) {
      report(failureOf(error), "guest");
    }
    void loadSession(true);
  });

  const cancelMine = exclusive(async (from: OpenSheet, was: GuestBooking) => {
    try {
      await api.cancelMine(was.id);
      haptics.success();
      closeIfStill(from);
      amendSession((current) => ({
        ...current,
        bookings: heldAfter(current.bookings, [was.id], null),
      }));
      say({
        text: "Бронь отменена. Стол снова свободен.",
        undo: { label: "Вернуть", run: () => void rebook(was) },
      });
      void loadSession(true);
    } catch (error) {
      report(failureOf(error), "guest");
    }
  });

  const enableReminders = exclusive(async () => {
    try {
      const reminders = await api.optInToReminders();
      amendSession((current) => ({ ...current, reminders }));
      void loadSession(true);
      openBotChat(process.env.NEXT_PUBLIC_BOT_USERNAME ?? "");
      tell(`Напомним за ${fmt.hoursWord(bar.remind_hours)} до брони.`);
    } catch (error) {
      report(failureOf(error), "guest");
    }
  });

  const dismissReminders = exclusive(async () => {
    try {
      const reminders = await api.dismissReminderPrompt();
      amendSession((current) => ({ ...current, reminders }));
      void loadSession(true);
    } catch (error) {
      report(failureOf(error), "guest");
    }
  });

  const offer = dayRail?.find((day) => day.service_date === serviceDate);
  const decision = bookingDecision(
    partySize,
    serviceDate,
    bar,
    chosenMinutes,
    session.bookings,
    offer ? offer.booked : heldOn(session.bookings, serviceDate),
  );
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
   * Puts the evening a staff action answered with on screen, if it is still the evening on screen.
   *
   * The answer is the room after the write, every booking in it, so a party the server reseated is
   * shown where it now sits, and a sheet on a booking that is gone closes.
   */
  const showAnswered = (room: ShiftView) => {
    putShift(room.service_date, () => showShift(room));
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
        const answer = await api.setAttendance(booking.id, attendance);
        haptics.success();
        showAnswered(answer.shift);
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
        report(failureOf(error), "staff");
      }
    },
  );

  const seat = (booking: ShiftBooking) => void setAttendance(booking, "arrived");
  const markLeft = (booking: ShiftBooking) => void setAttendance(booking, "left");

  const setNote = exclusive(async (booking: ShiftBooking, note: string | null) => {
    try {
      showAnswered((await api.setNote(booking.id, note)).shift);
    } catch (error) {
      report(failureOf(error), "staff");
    }
  });

  const cancelAsStaff = exclusive(
    async (from: OpenSheet, booking: ShiftBooking, reason: string) => {
      try {
        const answer = await api.cancelAsStaff(booking.id, reason);
        const told = answer.guest_notified
          ? `${booking.guest_name} получил сообщение с причиной.`
          : `${booking.guest_name} не получит сообщения — предупредите его сами.`;
        closeIfStill(from);
        showAnswered(answer.shift);
        say({
          text: [`Бронь отменена. ${told}`, reconciliationReport(answer.reconciliation)]
            .filter(Boolean)
            .join(" "),
        });
      } catch (error) {
        report(failureOf(error), "staff");
      }
    },
  );

  const sendTemplate = exclusive(async (from: OpenSheet, booking: ShiftBooking, text: string) => {
    try {
      await api.sendTemplate(booking.id, text);
      closeIfStill(from);
      tell(`Отправлено ${booking.guest_name}: «${text}»`);
    } catch (error) {
      report(failureOf(error), "staff");
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
        const answer = await api.blockTables(shiftDate, closure.tableIds, closure.reason);
        showAnswered(answer.shift);
        closed.push(...answer.closed);
        moved.moved.push(...answer.reconciliation.moved);
        moved.orphaned.push(...answer.reconciliation.orphaned);
      }
    } catch (error) {
      report(failureOf(error), "staff");
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
      const answer = await api.unblockTables(shiftDate, tableIds);
      showAnswered(answer.shift);
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
      report(failureOf(error), "staff");
    }
  });

  const findTables = exclusive(async (from: OpenSheet) => {
    closeIfStill(from);
    try {
      const answer = await api.reconcileShift(shiftDate);
      showAnswered(answer.shift);
      tell(
        rearranged(
          answer.reconciliation,
          "",
          "Свободных столов на это время нет. Откройте закрытый стол или предложите другое время.",
        ),
      );
    } catch (error) {
      report(failureOf(error), "staff");
    }
  });

  const seatWalkIn = exclusive(async (from: OpenSheet, tableId: string) => {
    try {
      const answer = await api.seatWalkIn(shiftDate, walkInParty, tableId);
      haptics.success();
      closeIfStill(from);
      showAnswered(answer.shift);
      tell(`Посадили за стол ${answer.booking.table_number}.`);
    } catch (error) {
      report(failureOf(error), "staff");
      // A refusal carries no room, and it usually means the room on screen is behind.
      void loadShift(true);
    }
  });

  const createManualBooking = exclusive(async (from: OpenSheet, tableId: string) => {
    const sent = manual;
    if (sent.minutes === null) return;
    try {
      const answer = await api.createStaffBooking(
        shiftDate,
        sent.minutes,
        sent.partySize,
        sent.name,
        tableId,
      );
      // A form already holding the next guest is not this one's to clear.
      setManual((current) =>
        JSON.stringify(current) === JSON.stringify(sent) ? EMPTY_MANUAL : current,
      );
      closeIfStill(from);
      showAnswered(answer.shift);
      const created = answer.booking;
      tell(
        `${created.guest_name} записан на ${fmt.time(created.start_minutes)}, стол ${created.table_number}.`,
      );
    } catch (error) {
      report(failureOf(error), "staff");
      void loadStaffTimes();
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
        const answer = await api.moveBooking(booking.id, minutes, tableId, party);
        haptics.success();
        closeIfStill(from);
        showAnswered(answer.shift);
        const now = answer.booking;
        const where = `стол ${now.table_number}`;
        const told =
          now.start_minutes === booking.start_minutes
            ? now.party_size === booking.party_size
              ? `${now.guest_name} за ${where}.`
              : `${now.guest_name}: ${fmt.guests(now.party_size)}, ${where}.`
            : `${now.guest_name}: ${fmt.time(now.start_minutes)}, ${where}. ` +
              (answer.guest_notified ? "Гостю сообщили." : "Гость не в боте — предупредите сами.");
        say({
          text: [told, reconciliationReport(answer.reconciliation)].filter(Boolean).join(" "),
        });
      } catch (error) {
        report(failureOf(error), "staff");
        void loadShift(true);
      }
    },
  );

  const saveSettings = exclusive(async () => {
    const sent = pairNow.current?.draft;
    if (!sent) return;
    const date = shiftDate;
    const openedBefore = openings.current;
    setSaving(true);
    editsDuringSave.current = [];
    try {
      const saved = await api.saveSettings(date, sent);
      const meanwhile = editsDuringSave.current ?? [];
      const shown = putSettings(date, () =>
        changePair((current) => savedInto(current, date, sent, saved.settings, meanwhile)),
      );
      // Saved for an evening no longer on screen: its per-table counts are that evening's, so the
      // one on screen is read instead.
      if (!shown) void loadSettings(true);
      const parts = ["Настройки сохранены.", reconciliationReport(saved.reconciliation)];
      if (saved.above_cap > 0) {
        parts.push(`${fmt.bookings(saved.above_cap)} больше нового лимита — они остаются в силе.`);
      }
      tell(parts.filter(Boolean).join(" "));
      void loadSession(true);
    } catch (error) {
      const failure = failureOf(error);
      report(failure, "staff");
      // A sheet opened while the save was on its way is somebody's next decision; the toast alone
      // reports the refusal rather than a sheet thrown over theirs.
      const undisturbed = openings.current === openedBefore;
      if (failure.code === "would_strand_bookings") {
        // Named, with their times. The API has always sent both; showing one general sentence
        // instead left a manager to work out which of thirty evenings was in the way.
        if (undisturbed) {
          openSheet({ kind: "conflict", reasons: strandedLines(strandedBookings(failure)) });
        }
      } else if (failure.code === "settings_invalid") {
        if (undisturbed) openSheet({ kind: "conflict", reasons: invalidReasons(failure) });
      } else if (failure.code === "settings_changed") {
        void loadSettings();
      }
    } finally {
      editsDuringSave.current = null;
      setSaving(false);
    }
  });

  // ---- what the shell is given ------------------------------------------------------------------

  const staffFooter =
    tab === "shift" && shiftOnScreen !== null && !shiftOnScreen.hours.closed && !isPast ? (
      <ShiftActions
        isToday={isToday}
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

          <ChoiceSheet
            open={sheet.kind === "templates"}
            title="Написать гостю"
            hint={`Уйдёт от бота в чат гостя. ${
              sheet.kind === "templates" ? sheet.booking.guest_name : ""
            } получит его сразу — отменить отправку нельзя.`}
            choices={shiftOnScreen?.message_templates ?? []}
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
            turnMinutes={bar.turn_minutes}
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
            turnMinutes={bar.turn_minutes}
            availability={shownStaffTimes}
            partySize={manual.partySize}
            chosenMinutes={manual.minutes}
            chosenTableId={manual.table}
            guestName={manual.name}
            failedToLoad={staffTimesRead.failure !== null}
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
            availability={shownStaffTimes}
            chosenMinutes={move.minutes}
            chosenTableId={move.table}
            failedToLoad={staffTimesRead.failure !== null}
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
            reasons={sheet.kind === "conflict" ? sheet.reasons : []}
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
          days={days}
          availability={times}
          partySize={partySize}
          serviceDate={serviceDate}
          chosenMinutes={chosenMinutes}
          daysFailed={daysRead.failure !== null}
          timesFailed={timesRead.failure !== null}
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
        shiftOnScreen ? (
          <ShiftScreen
            shift={shiftOnScreen}
            today={shiftOnScreen.today}
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
        ) : shiftRead.failure ? (
          <Failure
            message="Не удалось прочитать смену."
            actionLabel="Попробовать снова"
            onAction={() => void loadShift()}
          />
        ) : (
          <Spinner label="Читаем смену" />
        )
      ) : null}

      {tab === "settings" ? (
        pair ? (
          <SettingsScreen
            settings={pair.settings}
            draft={pair.draft}
            editedWeekday={editedWeekday}
            onEdit={editDraft}
            onEditWeekday={setEditedWeekday}
            section={settingsSection}
            onSection={setSettingsSection}
          />
        ) : (
          <Spinner label="Читаем настройки" />
        )
      ) : null}
    </AppShell>
  );
}
