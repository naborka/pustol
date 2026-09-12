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
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  ApiError,
  client as makeClient,
  draftOf,
  type Attendance,
  type Availability,
  type DayOffer,
  type GuestBooking,
  type Reconciliation,
  type Session,
  type SettingsDraft,
  type SettingsView,
  type ShiftBooking,
  type ShiftTable,
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
  previousAttendance,
  reconciliationReport,
  strandedLines,
} from "@/lib/outcomes";
import { firstReason, differs } from "@/lib/settingsRules";
import { hasStarted } from "@/lib/status";
import { credentials, haptics, openBotChat, openContact, webApp } from "@/lib/telegram";
import { TIMING } from "@/lib/tokens";
import { ShiftActions, ShiftScreen, type ShiftPane } from "@/components/AdminShift";
import { AppShell, InsetFrame, type StaffTab } from "@/components/AppChrome";
import {
  BookScreen,
  DoneScreen,
  HomeScreen,
  bookingDecision,
} from "@/components/GuestScreens";
import { SaveBar, SettingsScreen, type Section } from "@/components/Settings";
import { useInsets } from "@/components/ThemeProvider";
import {
  BookingSheet,
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

type OpenSheet =
  | { kind: "none" }
  | { kind: "booking"; booking: ShiftBooking }
  | { kind: "templates"; booking: ShiftBooking }
  | { kind: "cancelBooking"; booking: ShiftBooking }
  | { kind: "table"; table: ShiftTable }
  | { kind: "conflict"; reasons: string[] }
  | { kind: "days" }
  | { kind: "walkIn" }
  | { kind: "manual" }
  | { kind: "move"; booking: ShiftBooking }
  | { kind: "guestCancel" };

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

/** A failure as the API described it, or as close as the app can get. */
function failureOf(error: unknown): ApiFailure {
  return error instanceof ApiError ? error.failure : { code: "internal", message: String(error) };
}

/**
 * Fetches, keeps only the answer to the newest question, and says while a newer one is on its way.
 *
 * Tapping 2 then 4 guests fires two requests, and without this the first to come back wins — which
 * on a bad connection is how a guest is shown the times for a party they are no longer bringing.
 * Numbered rather than aborted, because an abort still has to be raced against the state update.
 *
 * The last answer stays until the next arrives, marked pending, rather than being blanked: blanking
 * swapped the grid for a spinner on every tap and made the page jump under the guest's thumb.
 */
function useLatest<T>(
  ask: () => Promise<T | null>,
  keep: (value: T | null) => void,
  onFailure: (error: unknown) => void,
  onSuccess: () => void,
): [load: () => Promise<void>, pending: boolean] {
  const asked = useRef(0);
  const [pending, setPending] = useState(false);
  const load = useCallback(async () => {
    const question = (asked.current += 1);
    setPending(true);
    try {
      const answer = await ask();
      if (question !== asked.current) return;
      keep(answer);
      onSuccess();
    } catch (error) {
      if (question !== asked.current) return;
      // An answer to an older question must not stand in for one that failed.
      keep(null);
      onFailure(error);
    } finally {
      if (question === asked.current) setPending(false);
    }
  }, [ask, keep, onFailure, onSuccess]);
  return [load, pending];
}

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

export default function Page() {
  // Read after mounting, never while rendering. The page is prerendered where there is no Telegram
  // at all, and a first render that decided "no payload" wrote «Откройте приложение из Telegram»
  // into the HTML every guest saw until the scripts had loaded.
  const [token, setToken] = useState<string | null | undefined>(undefined);
  useEffect(() => setToken(credentials()), []);
  const api = useMemo(() => (token ? makeClient(token) : null), [token]);

  const [session, setSession] = useState<Session | null>(null);
  const [fatal, setFatal] = useState<ApiFailure | null>(null);
  const [toast, setToast] = useState<ToastMessage | null>(null);
  const dismissToast = useRef(0);
  const hasSession = useRef(false);

  const [tab, setTab] = useState<Tab>("client");
  const [screen, setScreen] = useState<GuestScreen>("home");
  const [moved, setMoved] = useState(false);

  const [partySize, setPartySize] = useState(DEFAULT_PARTY);
  const [serviceDate, setServiceDate] = useState<string | null>(null);
  const [chosenMinutes, setChosenMinutes] = useState<number | null>(null);
  const [availability, setAvailability] = useState<Availability | null>(null);
  const [dayRail, setDayRail] = useState<DayOffer[] | null>(null);
  const [daysFailed, setDaysFailed] = useState(false);
  const [timesFailed, setTimesFailed] = useState(false);

  const [shiftDate, setShiftDate] = useState<string | null>(null);
  const [pane, setPane] = useState<ShiftPane>("now");
  const [shift, setShift] = useState<ShiftView | null>(null);
  const [shiftFailed, setShiftFailed] = useState(false);

  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [settingsSection, setSettingsSection] = useState<Section | null>(null);
  const [editedWeekday, setEditedWeekday] = useState(1);
  const [saving, setSaving] = useState(false);

  const insets = useInsets();
  const [sheet, setSheet] = useState<OpenSheet>({ kind: "none" });
  const [manual, setManual] = useState({
    name: "",
    partySize: DEFAULT_PARTY,
    minutes: null as number | null,
    table: null as string | null,
  });
  const [move, setMove] = useState({ minutes: null as number | null, table: null as string | null });
  const [staffTimes, setStaffTimes] = useState<Availability | null>(null);
  const [staffTimesFailed, setStaffTimesFailed] = useState(false);
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
    setToast(message);
    dismissToast.current = window.setTimeout(
      () => setToast((current) => (current === message ? null : current)),
      message.undo ? TIMING.undoMs : TIMING.toastMs,
    );
  }, []);

  const tell = useCallback((text: string) => say({ text }), [say]);

  /** Turns a failure into words for whoever is looking at it. */
  const report = useCallback(
    (error: unknown, audience: "guest" | "staff") => {
      const failure = failureOf(error);
      if (needsRelaunch(failure)) {
        setFatal(failure);
        return failure;
      }
      haptics.error();
      tell(messageFor(failure, audience));
      return failure;
    },
    [tell],
  );

  /**
   * A refresh nobody asked for fails silently, unless only relaunching can fix it.
   *
   * A toast every thirty seconds because the bar's Wi-Fi dropped would bury the one that matters.
   */
  const reportQuietly = useCallback((error: unknown) => {
    const failure = failureOf(error);
    if (needsRelaunch(failure)) setFatal(failure);
  }, []);

  /**
   * One action at a time.
   *
   * On a slow connection the button a guest just pressed looks as if nothing happened, and they
   * press it again. For a message or a cancellation that is a second message or a confusing "not
   * found"; for a booking, a second request racing the first.
   */
  const busy = useRef(false);
  const exclusive = useCallback(
    <Args extends unknown[]>(action: (...args: Args) => Promise<void>) =>
      async (...args: Args) => {
        if (busy.current) return;
        busy.current = true;
        try {
          await action(...args);
        } finally {
          busy.current = false;
        }
      },
    [],
  );

  const reload = useCallback(
    async (quiet = false) => {
      if (!api) return;
      try {
        const next = await api.session();
        hasSession.current = true;
        setSession(next);
        setServiceDate((current) => current ?? next.bookable_days[0] ?? next.bar.today);
        setShiftDate((current) => current ?? next.bar.today);
        setFatal(null);
      } catch (error) {
        if (quiet) {
          reportQuietly(error);
          return;
        }
        // Without a first screen there is nothing to fall back on. With one, a failed reread
        // leaves the screen as it was and says so, rather than replacing it with a dead end.
        const failure = report(error, "guest");
        if (!hasSession.current) setFatal(failure);
      }
    },
    [api, report, reportQuietly],
  );

  useEffect(() => {
    void reload();
  }, [reload]);

  // The rail depends only on how many are coming, so it is fetched when that changes and not when
  // a different day on the rail is tapped.
  const [loadDays] = useLatest(
    useCallback(async () => (await api?.days(partySize))?.days ?? null, [api, partySize]),
    setDayRail,
    useCallback(
      (error: unknown) => {
        setDaysFailed(true);
        report(error, "guest");
      },
      [report],
    ),
    useCallback(() => setDaysFailed(false), []),
  );

  useEffect(() => {
    if (screen !== "book") return;
    void loadDays();
  }, [screen, loadDays]);

  // The time grid recomputes whenever the question changes. Every answer comes from the server,
  // which has run the real allocator: a time shown as free is a time with a table behind it.
  const [loadAvailability, timesPending] = useLatest(
    useCallback(
      async () =>
        api && serviceDate !== null ? await api.availability(serviceDate, partySize) : null,
      [api, serviceDate, partySize],
    ),
    setAvailability,
    useCallback(
      (error: unknown) => {
        setTimesFailed(true);
        report(error, "guest");
      },
      [report],
    ),
    useCallback(() => setTimesFailed(false), []),
  );

  useEffect(() => {
    if (screen !== "book") return;
    void loadAvailability();
  }, [screen, loadAvailability]);

  // Numbered like `useLatest`, so a slow answer for the evening just left cannot replace the one
  // just asked for — but not blanked first: a refresh of the same evening keeps it on screen.
  const shiftAsked = useRef(0);
  const loadShift = useCallback(
    async (date: string, quiet = false) => {
      if (!api) return;
      const question = (shiftAsked.current += 1);
      try {
        const next = await api.shift(date);
        if (question !== shiftAsked.current) return;
        setShift(next);
        setShiftFailed(false);
      } catch (error) {
        if (question !== shiftAsked.current) return;
        if (quiet) {
          reportQuietly(error);
          return;
        }
        setShiftFailed(true);
        report(error, "staff");
      }
    },
    [api, report, reportQuietly],
  );

  useEffect(() => {
    if (tab !== "shift" || shiftDate === null) return;
    // Another evening is not a refresh of this one: what is on screen goes, rather than standing
    // under the new day's name while it loads.
    setShift((current) => (current?.service_date === shiftDate ? current : null));
    void loadShift(shiftDate);
  }, [tab, shiftDate, loadShift]);

  useWhileVisible(
    useMemo(
      () => (tab === "shift" && shiftDate !== null ? () => void loadShift(shiftDate, true) : null),
      [tab, shiftDate, loadShift],
    ),
    SHIFT_REFRESH_MS,
  );
  useWhileVisible(
    useMemo(
      () => (tab === "client" && screen === "home" ? () => void reload(true) : null),
      [tab, screen, reload],
    ),
    HOME_REFRESH_MS,
  );

  const loadSettings = useCallback(
    async (date: string) => {
      if (!api) return;
      try {
        const next = await api.settings(date);
        setSettings(next);
        setDraft(draftOf(next));
      } catch (error) {
        report(error, "staff");
      }
    },
    [api, report],
  );

  const settingsDirty = settings !== null && draft !== null && differs(draft, draftOf(settings));

  useEffect(() => {
    // An edit in progress is never replaced by a reread. Switching to the shift and back used to
    // throw a manager's unsaved changes away without a word.
    if (tab !== "settings" || shiftDate === null || settingsDirty) return;
    void loadSettings(shiftDate);
  }, [tab, shiftDate, loadSettings, settingsDirty]);

  // Closing Telegram with unsaved settings asks first, the way switching tabs no longer loses them.
  useEffect(() => {
    const app = webApp();
    if (settingsDirty) app?.enableClosingConfirmation?.();
    else app?.disableClosingConfirmation?.();
  }, [settingsDirty]);

  // Writing a booking down and moving one ask the same question, so there is one of it. A move
  // sets its own booking aside — shifting it half an hour must not mean giving up its table first
  // and hoping — and a booking already under way is not asking at all: its time cannot change.
  const moving = sheet.kind === "move" ? sheet.booking : null;
  const movingTime = moving !== null && !hasStarted(moving, shift?.now_minutes ?? null);
  const asksTimes = sheet.kind === "manual" || movingTime;
  const askParty = moving ? moving.party_size : manual.partySize;
  const askIgnoring = movingTime && moving ? moving.id : undefined;

  const [loadStaffTimes, staffTimesPending] = useLatest(
    useCallback(
      async () =>
        api && shiftDate !== null && asksTimes
          ? await api.staffAvailability(shiftDate, askParty, askIgnoring)
          : null,
      [api, shiftDate, asksTimes, askParty, askIgnoring],
    ),
    setStaffTimes,
    useCallback(
      (error: unknown) => {
        setStaffTimesFailed(true);
        report(error, "staff");
      },
      [report],
    ),
    useCallback(() => setStaffTimesFailed(false), []),
  );

  useEffect(() => {
    if (!asksTimes) return;
    void loadStaffTimes();
  }, [asksTimes, loadStaffTimes]);

  // Telegram's own back button, where there is one, rather than a second one drawn in the page. It
  // steps back through whatever is open, innermost first — on Android the hardware back button is
  // this button, and without it the whole app closed and took the open sheet with it.
  useEffect(() => {
    const back = webApp()?.BackButton;
    if (!back) return undefined;
    const goBack =
      sheet.kind !== "none"
        ? () => setSheet({ kind: "none" })
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
  if (fatal) {
    // Retrying with the proof the server just refused refuses again. Only reopening from Telegram
    // brings a new one, so that is the way out offered.
    const telegram = webApp();
    const relaunch = needsRelaunch(fatal) && telegram !== undefined;
    return (
      <InsetFrame insets={insets}>
        <Failure
          message={messageFor(fatal, "guest")}
          actionLabel={relaunch ? "Закрыть" : "Попробовать снова"}
          onAction={() => (relaunch ? telegram.close() : void reload())}
        />
      </InsetFrame>
    );
  }
  if (!session || !api || serviceDate === null || shiftDate === null) {
    return (
      <InsetFrame insets={insets}>
        <Spinner label="Открываем" />
      </InsetFrame>
    );
  }

  const bar = session.bar;
  const closeSheet = () => setSheet({ kind: "none" });
  const isToday = shiftDate === bar.today;
  // ISO dates compare as strings. An evening that is over is read, not written into.
  const isPast = shiftDate < bar.today;

  // ---- guest actions --------------------------------------------------------------------------

  const openPicker = () => {
    const start = session.booking?.service_date ?? bar.today;
    // The party the home card spoke for, so the picker opens on the promise the card just made.
    setPartySize(session.booking?.party_size ?? session.today_free_for_party);
    setServiceDate(
      session.bookable_days.includes(start) ? start : (session.bookable_days[0] ?? start),
    );
    setChosenMinutes(null);
    setScreen("book");
  };

  const book = exclusive(async () => {
    if (chosenMinutes === null) return;
    try {
      const taken = await api.book(serviceDate, chosenMinutes, partySize);
      haptics.success();
      setMoved(taken.replaced !== null);
      // The answer already says what was booked. Showing it does not wait on rereading the home
      // screen, whose failure must never turn a booking that happened into an error.
      setSession((current) => (current ? { ...current, booking: taken.booking } : current));
      setScreen("done");
      void reload(true);
    } catch (error) {
      report(error, "guest");
      // The refusal is usually "somebody just took it", so the picker is refreshed rather than left
      // showing a time that no longer exists.
      setChosenMinutes(null);
      void loadAvailability();
      void loadDays();
    }
  });

  /** Books the same slot again, for a guest who has just changed their mind about cancelling. */
  const rebook = exclusive(async (was: GuestBooking) => {
    try {
      await api.book(was.service_date, was.start_minutes, was.party_size);
      haptics.success();
      await reload();
      tell("Бронь вернулась.");
    } catch (error) {
      report(error, "guest");
      await reload();
    }
  });

  const cancelMine = exclusive(async () => {
    const was = session.booking;
    if (!was) return;
    try {
      await api.cancelMine();
      haptics.success();
      closeSheet();
      await reload();
      say({
        text: "Бронь отменена. Стол снова свободен.",
        undo: { label: "Вернуть", run: () => void rebook(was) },
      });
    } catch (error) {
      report(error, "guest");
    }
  });

  const enableReminders = exclusive(async () => {
    try {
      await api.optInToReminders();
      await reload();
      openBotChat(process.env.NEXT_PUBLIC_BOT_USERNAME ?? "");
      tell(`Напомним за ${fmt.hoursWord(bar.remind_hours)} до брони.`);
    } catch (error) {
      report(error, "guest");
    }
  });

  const dismissReminders = exclusive(async () => {
    try {
      await api.dismissReminderPrompt();
      await reload();
    } catch (error) {
      report(error, "guest");
    }
  });

  const decision = bookingDecision(
    partySize,
    serviceDate,
    bar.today,
    chosenMinutes,
    session.booking !== null,
  );
  const guestFooter =
    screen === "done" ? (
      <MainButton label="На главную" onClick={() => setScreen("home")} />
    ) : screen === "book" ? (
      <MainButton label={decision.label} enabled={decision.enabled} onClick={() => void book()} />
    ) : session.booking ? null : (
      <MainButton label="Забронировать стол" onClick={openPicker} />
    );

  // ---- staff actions -------------------------------------------------------------------------

  const afterShiftChange = async (message?: ToastMessage) => {
    await loadShift(shiftDate);
    if (message) say(message);
  };

  /**
   * One tap, applied at once, with the way back attached.
   *
   * The undo goes back to the status the booking *had*, read off it before the change, so taking
   * back a mistake restores the room rather than something that resembles it.
   */
  const setAttendance = exclusive(
    async (booking: ShiftBooking, attendance: Attendance, undoable = true) => {
      const before = previousAttendance(booking);
      try {
        const updated = await api.setAttendance(booking.id, attendance);
        haptics.success();
        if (sheet.kind === "booking") setSheet({ kind: "booking", booking: updated });
        await afterShiftChange({
          text: attendanceOutcome(updated, attendance),
          ...(undoable
            ? {
                undo: {
                  label: "Вернуть",
                  run: () => void setAttendance(updated, before, false),
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
      const updated = await api.setNote(booking.id, note);
      if (sheet.kind === "booking") setSheet({ kind: "booking", booking: updated });
      await loadShift(shiftDate);
    } catch (error) {
      report(error, "staff");
    }
  });

  const cancelAsStaff = exclusive(async (booking: ShiftBooking, reason: string) => {
    try {
      const outcome = await api.cancelAsStaff(booking.id, reason);
      const told = outcome.guest_notified
        ? `${booking.guest_name} получил сообщение с причиной.`
        : `${booking.guest_name} записан вручную — предупредите его сами.`;
      closeSheet();
      await afterShiftChange({
        text: [`Бронь отменена. ${told}`, reconciliationReport(outcome.reconciliation)]
          .filter(Boolean)
          .join(" "),
      });
    } catch (error) {
      report(error, "staff");
    }
  });

  const sendTemplate = exclusive(async (booking: ShiftBooking, text: string) => {
    try {
      await api.sendTemplate(booking.id, text);
      closeSheet();
      tell(`Отправлено ${booking.guest_name}: «${text}»`);
    } catch (error) {
      report(error, "staff");
    }
  });

  /**
   * Runs something that rearranges the shift, then reloads and reports what moved — per booking,
   * by name, never as a count of what it hoped to do.
   */
  const rearrange = exclusive(
    async (
      run: () => Promise<Reconciliation>,
      lead: string,
      whenNothingMoved: string,
      orphanLead?: string,
      undo?: ToastMessage["undo"],
    ) => {
      try {
        const summary = reconciliationReport(await run(), orphanLead);
        const text = summary.length > 0 ? `${lead} ${summary}`.trim() : whenNothingMoved;
        await afterShiftChange(undo ? { text, undo } : { text });
      } catch (error) {
        report(error, "staff");
      }
    },
  );

  const blockTables = (tableIds: string[], reason: string, number: number) => {
    closeSheet();
    return rearrange(
      () => api.blockTables(shiftDate, tableIds, reason),
      `Стол ${number} закрыт на вечер.`,
      `Стол ${number} закрыт на вечер. Броней там не было.`,
      "Остались без стола",
      { label: "Вернуть", run: () => void unblockTables(tableIds, number) },
    );
  };

  /**
   * Opening a table back up is as reversible as closing it, so it offers the same way back — with
   * the reason it was closed for, which is the only way re-closing it puts the room where it was.
   */
  const unblockTables = (tableIds: string[], number: number, wasClosedFor?: string) => {
    closeSheet();
    return rearrange(
      () => api.unblockTables(shiftDate, tableIds),
      `Стол ${number} снова в подборе.`,
      `Стол ${number} снова в подборе.`,
      undefined,
      wasClosedFor === undefined
        ? undefined
        : {
            label: "Вернуть",
            run: () => void blockTables(tableIds, wasClosedFor, number),
          },
    );
  };

  const findTables = () => {
    closeSheet();
    return rearrange(
      () => api.reconcileShift(shiftDate),
      "",
      "Свободных столов на это время нет. Откройте закрытый стол или предложите другое время.",
    );
  };

  const seatWalkIn = exclusive(async (tableId: string) => {
    try {
      const created = await api.seatWalkIn(shiftDate, walkInParty, tableId);
      haptics.success();
      closeSheet();
      await afterShiftChange({ text: `Посадили за стол ${created.table_number}.` });
    } catch (error) {
      report(error, "staff");
      await loadShift(shiftDate);
    }
  });

  const createManualBooking = exclusive(async (tableId: string) => {
    if (manual.minutes === null) return;
    try {
      const created = await api.createStaffBooking(
        shiftDate,
        manual.minutes,
        manual.partySize,
        manual.name,
        tableId,
      );
      setManual({ name: "", partySize: DEFAULT_PARTY, minutes: null, table: null });
      closeSheet();
      await afterShiftChange({
        text: `${created.guest_name} записан на ${fmt.time(created.start_minutes)}, стол ${created.table_number}.`,
      });
    } catch (error) {
      report(error, "staff");
      void loadStaffTimes();
    }
  });

  /** The report says whether the guest was told: not knowing means sending a second message. */
  const moveBooking = exclusive(async (booking: ShiftBooking, minutes: number, tableId: string) => {
    try {
      const moved = await api.moveBooking(booking.id, minutes, tableId);
      haptics.success();
      closeSheet();
      const where = `стол ${moved.booking.table_number}`;
      const told = moved.booking.start_minutes === booking.start_minutes
        ? `${moved.booking.guest_name} за ${where}.`
        : `${moved.booking.guest_name}: ${fmt.time(moved.booking.start_minutes)}, ${where}. ` +
          (moved.guest_notified ? "Гостю сообщили." : "Гость не в боте — предупредите сами.");
      await afterShiftChange({
        text: [told, reconciliationReport(moved.reconciliation)].filter(Boolean).join(" "),
      });
    } catch (error) {
      report(error, "staff");
      await loadShift(shiftDate);
    }
  });

  const saveSettings = exclusive(async () => {
    if (!draft) return;
    setSaving(true);
    try {
      const saved = await api.saveSettings(shiftDate, draft);
      setSettings(saved.settings);
      setDraft(draftOf(saved.settings));
      const parts = ["Настройки сохранены.", reconciliationReport(saved.reconciliation)];
      if (saved.above_cap > 0) {
        parts.push(`${fmt.bookings(saved.above_cap)} больше нового лимита — они остаются в силе.`);
      }
      tell(parts.filter(Boolean).join(" "));
      await reload();
    } catch (error) {
      const failure = report(error, "staff");
      if (failure.code === "would_strand_bookings") {
        // Named, with their times. The API has always sent both; showing one general sentence
        // instead left a manager to work out which of thirty evenings was in the way.
        setSheet({ kind: "conflict", reasons: strandedLines(strandedBookings(failure)) });
      } else if (failure.code === "settings_invalid") {
        setSheet({ kind: "conflict", reasons: invalidReasons(failure) });
      }
    } finally {
      setSaving(false);
    }
  });

  // ---- what the shell is given ------------------------------------------------------------------

  const staffFooter =
    tab === "shift" && shift !== null && !shift.hours.closed && !isPast ? (
      <ShiftActions
        isToday={isToday}
        onWalkIn={() => {
          setWalkInParty(DEFAULT_PARTY);
          setWalkInTable(null);
          setSheet({ kind: "walkIn" });
        }}
        onManual={() => setSheet({ kind: "manual" })}
      />
    ) : tab === "settings" && settingsDirty && draft && settings ? (
      <SaveBar
        reason={firstReason(draft, settings.limits)}
        saving={saving}
        onSave={() => void saveSettings()}
        onRevert={() => setDraft(draftOf(settings))}
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
            nowMinutes={shift?.now_minutes ?? null}
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
              setSheet({ kind: "templates", booking: sheet.booking });
            }}
            onOpenCancel={() => {
              if (sheet.kind === "booking") setSheet({ kind: "cancelBooking", booking: sheet.booking });
            }}
            onOpenMove={() => {
              if (sheet.kind !== "booking") return;
              setMove({ minutes: null, table: null });
              setSheet({ kind: "move", booking: sheet.booking });
            }}
            onFindTable={() => void findTables()}
          />

          <ChoiceSheet
            open={sheet.kind === "templates"}
            title="Написать гостю"
            hint={`Уйдёт от бота в чат гостя. ${
              sheet.kind === "templates" ? sheet.booking.guest_name : ""
            } получит его сразу — отменить отправку нельзя.`}
            choices={shift?.message_templates ?? []}
            onClose={closeSheet}
            onChoose={(text) => {
              if (sheet.kind === "templates") void sendTemplate(sheet.booking, text);
            }}
          />

          <ChoiceSheet
            open={sheet.kind === "cancelBooking"}
            title="Причина отмены"
            hint="Гость получит сообщение с этой причиной, и стол сразу освободится. Отменить это нельзя."
            choices={shift?.cancel_reasons ?? []}
            onClose={closeSheet}
            onChoose={(reason) => {
              if (sheet.kind === "cancelBooking") void cancelAsStaff(sheet.booking, reason);
            }}
          />

          <TableSheet
            key={sheet.kind === "table" ? sheet.table.id : "no-table"}
            open={sheet.kind === "table"}
            table={sheet.kind === "table" ? sheet.table : null}
            shift={shift}
            onClose={closeSheet}
            onBlock={(tableIds, reason) => {
              if (sheet.kind === "table") void blockTables(tableIds, reason, sheet.table.number);
            }}
            onUnblock={(tableIds) => {
              if (sheet.kind !== "table") return;
              void unblockTables(
                tableIds,
                sheet.table.number,
                sheet.table.blocked_because ?? undefined,
              );
            }}
          />

          <DaySheet
            open={sheet.kind === "days"}
            days={shift?.days ?? []}
            today={bar.today}
            serviceDate={shiftDate}
            guestHorizonDays={shift?.guest_horizon_days ?? 0}
            onClose={closeSheet}
            onChoose={(date) => {
              closeSheet();
              setShiftDate(date);
            }}
          />

          <WalkInSheet
            open={sheet.kind === "walkIn"}
            shift={shift}
            maxParty={bar.max_party}
            turnMinutes={bar.turn_minutes}
            partySize={walkInParty}
            chosenTableId={walkInTable}
            onClose={closeSheet}
            onPartySize={setWalkInParty}
            onChooseTable={setWalkInTable}
            onSeat={(tableId) => void seatWalkIn(tableId)}
          />

          <ManualBookingSheet
            open={sheet.kind === "manual"}
            shift={shift}
            maxParty={bar.max_party}
            turnMinutes={bar.turn_minutes}
            availability={staffTimes}
            partySize={manual.partySize}
            chosenMinutes={manual.minutes}
            chosenTableId={manual.table}
            guestName={manual.name}
            failedToLoad={staffTimesFailed}
            timesPending={staffTimesPending}
            onClose={closeSheet}
            onPartySize={(size) =>
              setManual((current) => ({ ...current, partySize: size, minutes: null }))
            }
            onPick={(minutes) => setManual((current) => ({ ...current, minutes }))}
            onTakenSlot={() => tell("Это время занято. Свободное — без зачёркивания.")}
            onChooseTable={(table) => setManual((current) => ({ ...current, table }))}
            onGuestName={(name) => setManual((current) => ({ ...current, name }))}
            onRetry={() => void loadStaffTimes()}
            onCreate={(tableId) => void createManualBooking(tableId)}
          />

          <MoveBookingSheet
            open={sheet.kind === "move"}
            booking={moving}
            shift={shift}
            turnMinutes={bar.turn_minutes}
            availability={staffTimes}
            chosenMinutes={move.minutes}
            chosenTableId={move.table}
            failedToLoad={staffTimesFailed}
            timesPending={staffTimesPending}
            onClose={closeSheet}
            onPick={(minutes) => setMove((current) => ({ ...current, minutes }))}
            onTakenSlot={() => tell("Это время занято. Свободное — без зачёркивания.")}
            onChooseTable={(table) => setMove((current) => ({ ...current, table }))}
            onRetry={() => void loadStaffTimes()}
            onMove={(minutes, tableId) => {
              if (moving) void moveBooking(moving, minutes, tableId);
            }}
          />

          <GuestCancelSheet
            open={sheet.kind === "guestCancel"}
            booking={session.booking}
            today={bar.today}
            onClose={closeSheet}
            onConfirm={() => void cancelMine()}
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
          onCancel={() => setSheet({ kind: "guestCancel" })}
          onEnableReminders={() => void enableReminders()}
          onDismissReminders={() => void dismissReminders()}
          onContact={openContact}
        />
      ) : null}

      {tab === "client" && screen === "book" ? (
        <BookScreen
          bar={bar}
          days={dayRail}
          availability={availability}
          partySize={partySize}
          serviceDate={serviceDate}
          chosenMinutes={chosenMinutes}
          daysFailed={daysFailed}
          timesFailed={timesFailed}
          timesPending={timesPending}
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
            void loadAvailability();
          }}
          {...(webApp()?.BackButton ? {} : { onBack: () => setScreen("home") })}
        />
      ) : null}

      {tab === "client" && screen === "done" && session.booking ? (
        <DoneScreen booking={session.booking} bar={bar} moved={moved} />
      ) : null}

      {tab === "shift" ? (
        shift ? (
          <ShiftScreen
            shift={shift}
            today={bar.today}
            graceMinutes={bar.grace_minutes}
            pane={pane}
            onPane={setPane}
            onServiceDate={setShiftDate}
            onOpenDays={() => setSheet({ kind: "days" })}
            onOpenTable={(table) => setSheet({ kind: "table", table })}
            actions={{
              onOpen: (booking) => setSheet({ kind: "booking", booking }),
              onSeat: seat,
              onLeft: markLeft,
              onFindTable: () => void findTables(),
            }}
          />
        ) : shiftFailed ? (
          <Failure
            message="Не удалось прочитать смену."
            actionLabel="Попробовать снова"
            onAction={() => void loadShift(shiftDate)}
          />
        ) : (
          <Spinner label="Читаем смену" />
        )
      ) : null}

      {tab === "settings" ? (
        settings && draft ? (
          <SettingsScreen
            settings={settings}
            draft={draft}
            editedWeekday={editedWeekday}
            onDraft={setDraft}
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
