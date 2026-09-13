"use client";

/**
 * The whole app, one screen deep.
 *
 * A Mini App has no address bar and no history to speak of, so navigation is state rather than
 * routing: three guest screens, three staff panes and a set of sheets. Keeping that state here —
 * and every fetch with it — means the screens stay pure functions of what is loaded, which is what
 * makes them worth testing.
 *
 * Action rules: reversible acts happen on one tap with undo; acts guest feels get confirmation, no
 * undo. One action at a time, so second tap on slow connection is not second booking or message.
 * Screen refreshes itself: shift left open on bar is normal case.
 *
 * All reads go through `useRead`. Every staff write answers with room as server has it after, put on
 * screen through same model. Phone never patches own copy of room.
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
 * Open shift poll. Guest bookings, colleague seatings, late parties must reach untouched screen.
 * 30 s sits well inside late-party grace.
 */
const SHIFT_REFRESH_MS = 30_000;

/** Guest home poll: open-until line, bookings bar closed. */
const HOME_REFRESH_MS = 60_000;

/** One read key for session, whoever asks. */
const SESSION = "session";

/** One read key for settings: bar-wide, not per evening. */
const SETTINGS = "settings";

/**
 * Rooms ordered by server version, not arrival. Version ignores clock and bot reachability, so equal
 * versions go by ask order.
 */
const roomOrder: Order<ShiftView> = (next, shown) => next.version - shown.version;

/** Ordered by save count, so older reread never lands over save. */
const settingsOrder: Order<SettingsView> = (next, shown) => next.version - shown.version;

/** For reads whose answers only drawn, never folded. */
const nothing = () => {};

const EMPTY_MANUAL = {
  name: "",
  partySize: DEFAULT_PARTY,
  minutes: null as number | null,
  table: null as string | null,
};

/**
 * Calls `refresh` every `intervalMs` while visible, and at once on return. Telegram keeps minimised
 * Mini App alive and fires `activated`; locked phone hides page. Neither should poll; both catch up.
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
 * State readable right after write, not only next render: answers must fold into current value, not
 * stale render copy.
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
  // Read after mount, never in render: page prerenders without Telegram, and guest sees that HTML
  // until scripts load.
  const [token, setToken] = useState<string | null | undefined>(undefined);
  useEffect(() => setToken(credentials()), []);
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
  // Edits made during save, replayed on top of what save stored.
  const editsDuringSave = useRef<Edit[] | null>(null);
  const [settingsSection, setSettingsSection] = useState<Section | null>(null);
  const [editedWeekday, setEditedWeekday] = useState(1);
  const [saving, changeSaving, savingNow] = useSynced(false);
  // Settings asked for during save; read once save answers.
  const settingsWanted = useRef(false);
  // Last save refusal, until next edit or save.
  const [refusal, setRefusal] = useState<Refusal | null>(null);
  // What read folded into edit; shown on settings until next edit or save.
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

  /** Ended session has own screen, so no toast for it. */
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
   * Self-started reads (poll, reread after write) only while session stands. After end only tapped
   * retry asks: anything else spins over reopen screen and meets same refusal.
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

  // Guest cancel sheet follows bookings on screen, from read or write.
  useEffect(() => {
    if (session) setSheet((current) => refreshedGuestSheet(current, session.bookings));
  }, [session]);

  /** Guest write answer goes on screen at once; later reread only freshens. */
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

  // Refetch on question change. Server runs real allocator: time shown free has table behind it, and
  // what booking would replace is server's word too.
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
  // Only evening asked for shown: another evening is not refresh of this one.
  const shiftOnScreen = shiftRead.value;

  // Open sheet follows shown evening's current room, from read or write.
  useEffect(() => {
    if (shiftOnScreen) setSheet((current) => refreshedSheet(current, shiftOnScreen));
  }, [shiftOnScreen]);

  // Settings also count table bookings from this evening.
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

  // Fold into edit on landing, so typing during load survives. Read landing during save may or may
  // not include save: drop, reask after save answers. Fold note waits on settings screen, not toast:
  // may land while shift shown.
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

  /** Read now, or once pending save answers. */
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

  // Closing Telegram with unsaved settings asks first.
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

  // Manual booking and move ask same question. Move ignores own booking: shifting half hour must not
  // first give up its table. Asks even once booking started, when only tables can change.
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
   * One action at a time. Slow connection makes tap look ignored, so people tap again: second message,
   * confusing "not found", racing booking. Dropped tap says so, else button looks broken. Except over
   * undo toast: replacing «Вернуть» with «Подождите» loses undo, so only buzz.
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

  // Telegram BackButton, not drawn one. Steps back innermost first. On Android hardware back is this
  // button; without handler app closes with open sheet.
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
  // Only session read asked after end can restore app, so only it may spin over end screen.
  const blocking = sessionEnd
    ? sessionEnd.retrying
      ? null
      : sessionEnd.failure
    : session === null
      ? sessionRead.failure
      : null;
  if (blocking) {
    // Retry with refused proof refuses again. Only reopening from Telegram brings new proof, so offer
    // that.
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
  /** Closes sheet action started from, never sheet opened since. */
  const closeIfStill = (from: OpenSheet) => setSheet((current) => closedIfStill(current, from));
  // Server's day, not phone's at open: shift may stay open overnight.
  const today = shiftOnScreen?.today ?? bar.today;
  // ISO dates compare as strings. Past evening read-only.
  const isPast = shiftDate < today;
  // Chosen time and its effect come from answer to question on screen: answer drawn while another
  // loads was for other evening or party.
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
      // Replace only what button said. Server refuses otherwise.
      const answer = await api.book(serviceDate, chosen, partySize, timesOnScreen.replacing);
      haptics.success();
      setTaken({ booking: answer.booking, moved: answer.replaced.length > 0 });
      // Answer already says what was booked. Do not wait on home reread: its failure must never turn
      // done booking into error.
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
        // Guest holdings changed since button drawn. Picker stays; button redraws from current times
        // for guest to check and press again.
        refresh(loadSession);
        refresh(loadDays);
        refresh(loadTimes);
        return;
      }
      // Refused for guest holdings: labels drawn from changed bookings.
      if (code === "already_booked_tonight") refresh(loadSession);
      // The refusal is usually "somebody just took it", so the picker is refreshed rather than left
      // showing a time that no longer exists.
      setChosenMinutes(null);
      refresh(loadTimes);
      refresh(loadDays);
    }
  });

  // No undo: confirmation sheet was protection. Rebooking slot could replace another held booking and
  // never restores one already begun.
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
  // Guest with plan moves it from its card; with only tonight's table or nothing, books another
  // evening here.
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
   * Records answered room for its evening. Answer is full room after write: reseated party shows where
   * it sits, sheet on gone booking closes. Numbered at send: same-version room read after is fresher,
   * read before is not.
   */
  const writeShift = async <A extends { shift: ShiftView }>(send: () => Promise<A>): Promise<A> => {
    const sent = markShift();
    const answer = await send();
    putShift(answer.shift.service_date, () => answer.shift, sent);
    return answer;
  };

  /**
   * One tap, applied at once, with undo. Undo restores attendance server says booking had just before,
   * even when colleague changed it unseen by this phone.
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
      // Room on screen said bot reaches guest; server says not, so reread.
      if (failureOf(error).code === "no_bot_chat") refresh(loadShift);
    }
  });

  /** What rearrangement did, per booking by name, never count of intent. */
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
   * Undo opens exactly tables this tap closed, as server counted, not ones colleague already closed.
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

  /** Answer lists closures server removed, each with its reason. */
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
      // Refusal carries no room and usually means shown room is behind.
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
      // Form already holding next guest is not this one's to clear.
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

  /** Report says whether guest was told: not knowing means second message. */
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
      // Record as read answer too: reread failure from before this save is stale.
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
        // Kept on save bar. Sheet opened during save is somebody's next decision; reasons must not
        // cover it.
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
            // Table chosen for old party may not seat new one; reset to room's best fit.
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
