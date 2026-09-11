"use client";

/**
 * The whole app, one screen deep.
 *
 * A Mini App has no address bar and no history to speak of, so navigation is state rather than
 * routing: three guest screens, three staff panes and a set of sheets. Keeping that state here —
 * and every fetch with it — means the screens stay pure functions of what is loaded, which is what
 * makes them worth testing.
 *
 * This file also holds the one rule that decides how an action feels. Anything reversible happens
 * on one tap and comes back with a way to undo it; anything the guest will feel is confirmed
 * first and gets no undo, because the confirmation was the protection.
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
import { invalidReasons, messageFor, needsRelaunch, type ApiFailure } from "@/lib/errors";
import * as fmt from "@/lib/format";
import { firstReason, differs } from "@/lib/settingsRules";
import { credentials, haptics, openBotChat, webApp } from "@/lib/telegram";
import { TIMING } from "@/lib/tokens";
import { ShiftActions, ShiftScreen, type ShiftPane } from "@/components/AdminShift";
import { AppShell, InsetFrame, type StaffTab } from "@/components/AppChrome";
import {
  BookScreen,
  DoneScreen,
  HomeScreen,
  bookingDecision,
} from "@/components/GuestScreens";
import { SaveBar, SettingsScreen } from "@/components/Settings";
import { useInsets } from "@/components/ThemeProvider";
import {
  BookingSheet,
  ChoiceSheet,
  ConflictSheet,
  DaySheet,
  GuestCancelSheet,
  ManualBookingSheet,
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
  | { kind: "guestCancel" };

/** What a party size defaults to: the commonest booking, and what the home card speaks for. */
const DEFAULT_PARTY = 2;

export default function Page() {
  const token = useMemo(() => credentials(), []);
  const api = useMemo(() => (token === null ? null : makeClient(token)), [token]);

  const [session, setSession] = useState<Session | null>(null);
  const [fatal, setFatal] = useState<ApiFailure | null>(null);
  const [toast, setToast] = useState<ToastMessage | null>(null);
  const dismissToast = useRef(0);

  const [tab, setTab] = useState<Tab>("client");
  const [screen, setScreen] = useState<GuestScreen>("home");

  const [partySize, setPartySize] = useState(DEFAULT_PARTY);
  const [serviceDate, setServiceDate] = useState<string | null>(null);
  const [chosenMinutes, setChosenMinutes] = useState<number | null>(null);
  const [availability, setAvailability] = useState<Availability | null>(null);
  const [dayRail, setDayRail] = useState<DayOffer[] | null>(null);
  const [pickerFailed, setPickerFailed] = useState(false);

  const [shiftDate, setShiftDate] = useState<string | null>(null);
  const [pane, setPane] = useState<ShiftPane>("now");
  const [shift, setShift] = useState<ShiftView | null>(null);
  const [shiftFailed, setShiftFailed] = useState(false);

  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [editedWeekday, setEditedWeekday] = useState(1);
  const [saving, setSaving] = useState(false);

  const insets = useInsets();
  const [sheet, setSheet] = useState<OpenSheet>({ kind: "none" });
  const [manual, setManual] = useState({ name: "", partySize: DEFAULT_PARTY, minutes: null as number | null });
  const [manualAvailability, setManualAvailability] = useState<Availability | null>(null);
  const [manualFailed, setManualFailed] = useState(false);
  const [walkInParty, setWalkInParty] = useState(DEFAULT_PARTY);

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
      const failure =
        error instanceof ApiError ? error.failure : { code: "internal", message: String(error) };
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

  const reload = useCallback(async () => {
    if (!api) return;
    try {
      const next = await api.session();
      setSession(next);
      setServiceDate((current) => current ?? next.bookable_days[0] ?? next.bar.today);
      setShiftDate((current) => current ?? next.bar.today);
      setFatal(null);
    } catch (error) {
      // Whatever it was, the first screen cannot be drawn: `report` has already settled the
      // relaunch case, and anything else becomes a dead end with a retry.
      setFatal(report(error, "guest"));
    }
  }, [api, report]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Answers are numbered so a slow one cannot land on top of a newer question. Tapping 2 then 4
  // guests fires two requests, and without this the first to come back wins — which on a bad
  // connection is how a guest is shown the times for a party they are no longer bringing.
  const answer = useRef({ days: 0, availability: 0, manual: 0 });

  // The rail depends only on how many are coming, so it is fetched when that changes and not when
  // a different day on the rail is tapped.
  const loadDays = useCallback(async () => {
    if (!api) return;
    const asked = (answer.current.days += 1);
    setDayRail(null);
    try {
      const rail = (await api.days(partySize)).days;
      if (asked !== answer.current.days) return;
      setDayRail(rail);
      setPickerFailed(false);
    } catch (error) {
      if (asked !== answer.current.days) return;
      setPickerFailed(true);
      report(error, "guest");
    }
  }, [api, partySize, report]);

  useEffect(() => {
    if (screen !== "book") return;
    void loadDays();
  }, [screen, loadDays]);

  // The time grid recomputes whenever the question changes. Every answer comes from the server,
  // which has run the real allocator: a time shown as free is a time with a table behind it.
  const loadAvailability = useCallback(async () => {
    if (!api || serviceDate === null) return;
    const asked = (answer.current.availability += 1);
    setAvailability(null);
    try {
      const next = await api.availability(serviceDate, partySize);
      if (asked !== answer.current.availability) return;
      setAvailability(next);
      setPickerFailed(false);
    } catch (error) {
      if (asked !== answer.current.availability) return;
      setPickerFailed(true);
      report(error, "guest");
    }
  }, [api, serviceDate, partySize, report]);

  useEffect(() => {
    if (screen !== "book") return;
    void loadAvailability();
  }, [screen, loadAvailability]);

  const loadShift = useCallback(
    async (date: string) => {
      if (!api) return;
      try {
        setShift(await api.shift(date));
        setShiftFailed(false);
      } catch (error) {
        setShiftFailed(true);
        report(error, "staff");
      }
    },
    [api, report],
  );

  useEffect(() => {
    if (tab !== "shift" || shiftDate === null) return;
    void loadShift(shiftDate);
  }, [tab, shiftDate, loadShift]);

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

  useEffect(() => {
    if (tab !== "settings" || shiftDate === null) return;
    void loadSettings(shiftDate);
  }, [tab, shiftDate, loadSettings]);

  // The manual-booking sheet asks the same question the guest picker does, for the shift on screen.
  const loadManualAvailability = useCallback(async () => {
    if (!api || shiftDate === null) return;
    const asked = (answer.current.manual += 1);
    setManualAvailability(null);
    try {
      const next = await api.staffAvailability(shiftDate, manual.partySize);
      if (asked !== answer.current.manual) return;
      setManualAvailability(next);
      setManualFailed(false);
    } catch (error) {
      if (asked !== answer.current.manual) return;
      setManualFailed(true);
      report(error, "staff");
    }
  }, [api, shiftDate, manual.partySize, report]);

  useEffect(() => {
    if (sheet.kind !== "manual") return;
    void loadManualAvailability();
  }, [sheet.kind, loadManualAvailability]);

  // Telegram's own back button, where there is one, rather than a second one drawn in the page.
  useEffect(() => {
    const back = webApp()?.BackButton;
    if (!back) return undefined;
    const goBack = () => setScreen("home");
    if (tab === "client" && screen === "book") {
      back.onClick(goBack);
      back.show();
    } else {
      back.hide();
    }
    return () => back.offClick(goBack);
  }, [tab, screen]);

  if (token === null) {
    return (
      <InsetFrame insets={insets}>
        <Failure message={messageFor({ code: "no_credentials", message: "" }, "guest")} />
      </InsetFrame>
    );
  }
  if (fatal) {
    return (
      <InsetFrame insets={insets}>
        <Failure
          message={messageFor(fatal, "guest")}
          actionLabel="Попробовать снова"
          onAction={() => void reload()}
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

  // ---- guest actions --------------------------------------------------------------------------

  const openPicker = () => {
    const start = session.booking?.service_date ?? bar.today;
    setPartySize(session.booking?.party_size ?? DEFAULT_PARTY);
    setServiceDate(
      session.bookable_days.includes(start) ? start : (session.bookable_days[0] ?? start),
    );
    setChosenMinutes(null);
    setScreen("book");
  };

  const book = async () => {
    if (chosenMinutes === null) return;
    try {
      await api.book(serviceDate, chosenMinutes, partySize);
      haptics.success();
      await reload();
      setScreen("done");
    } catch (error) {
      report(error, "guest");
      // The refusal is usually "somebody just took it", so the picker is refreshed rather than left
      // showing a time that no longer exists.
      setChosenMinutes(null);
      void loadAvailability();
      void loadDays();
    }
  };

  /** Books the same slot again, for a guest who has just changed their mind about cancelling. */
  const rebook = async (was: GuestBooking) => {
    try {
      await api.book(was.service_date, was.start_minutes, was.party_size);
      haptics.success();
      await reload();
      tell("Бронь вернулась.");
    } catch (error) {
      report(error, "guest");
      await reload();
    }
  };

  const cancelMine = async () => {
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
  };

  const enableReminders = async () => {
    try {
      await api.optInToReminders();
      await reload();
      openBotChat(process.env.NEXT_PUBLIC_BOT_USERNAME ?? "");
      tell(`Напомним за ${fmt.hoursWord(bar.remind_hours)} до брони.`);
    } catch (error) {
      report(error, "guest");
    }
  };

  const dismissReminders = async () => {
    try {
      await api.dismissReminderPrompt();
      await reload();
    } catch (error) {
      report(error, "guest");
    }
  };

  const decision = bookingDecision(partySize, serviceDate, bar.today, chosenMinutes);
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

  const describe = (outcome: Reconciliation) => {
    const parts: string[] = [];
    if (outcome.moved.length > 0) {
      parts.push(
        `Пересажены: ${outcome.moved
          .map((entry) => `${entry.guest_name} → стол ${entry.to_number}`)
          .join(", ")}.`,
      );
    }
    if (outcome.orphaned.length > 0) {
      parts.push(
        `Всё ещё без стола: ${outcome.orphaned.map((entry) => entry.guest_name).join(", ")}.`,
      );
    }
    return parts.join(" ");
  };

  /** One tap, applied at once, with the way back attached. */
  const setAttendance = async (
    booking: ShiftBooking,
    attendance: Attendance,
    outcome: (updated: ShiftBooking) => ToastMessage,
  ) => {
    try {
      const updated = await api.setAttendance(booking.id, attendance);
      haptics.success();
      if (sheet.kind === "booking") setSheet({ kind: "booking", booking: updated });
      await afterShiftChange(outcome(updated));
    } catch (error) {
      report(error, "staff");
    }
  };

  const undoAttendance = (booking: ShiftBooking, back: Attendance) => ({
    label: "Вернуть",
    run: () =>
      void setAttendance(booking, back, (updated) => ({
        text: `${updated.guest_name}: вернули как было.`,
      })),
  });

  const seat = (booking: ShiftBooking) =>
    void setAttendance(booking, "arrived", (updated) => ({
      text: `${updated.guest_name} за столом ${updated.table_number}.`,
      undo: undoAttendance(booking, "confirmed"),
    }));

  const markLeft = (booking: ShiftBooking) =>
    void setAttendance(booking, "left", (updated) => ({
      text: `Стол ${updated.table_number} свободен.`,
      undo: undoAttendance(booking, "arrived"),
    }));

  const markNoShow = (booking: ShiftBooking) =>
    void setAttendance(booking, "no_show", (updated) => ({
      text: `${updated.guest_name}: не пришли, стол свободен.`,
      undo: undoAttendance(booking, "confirmed"),
    }));

  const setNote = async (booking: ShiftBooking, note: string | null) => {
    try {
      const updated = await api.setNote(booking.id, note);
      if (sheet.kind === "booking") setSheet({ kind: "booking", booking: updated });
      await loadShift(shiftDate);
    } catch (error) {
      report(error, "staff");
    }
  };

  const cancelAsStaff = async (booking: ShiftBooking, reason: string) => {
    try {
      const outcome = await api.cancelAsStaff(booking.id, reason);
      const told = outcome.guest_notified
        ? `${booking.guest_name} получил сообщение с причиной.`
        : `${booking.guest_name} записан вручную — предупредите его сами.`;
      closeSheet();
      await afterShiftChange({
        text: [`Бронь отменена. ${told}`, describe(outcome.reconciliation)]
          .filter(Boolean)
          .join(" "),
      });
    } catch (error) {
      report(error, "staff");
    }
  };

  const sendTemplate = async (booking: ShiftBooking, text: string) => {
    try {
      await api.sendTemplate(booking.id, text);
      closeSheet();
      tell(`Отправлено ${booking.guest_name}: «${text}»`);
    } catch (error) {
      report(error, "staff");
    }
  };

  /**
   * Runs something that rearranges the shift, then reloads and reports what moved — per booking,
   * by name, never as a count of what it hoped to do.
   */
  const rearrange = async (
    run: () => Promise<Reconciliation>,
    lead: string,
    whenNothingMoved: string,
    undo?: ToastMessage["undo"],
  ) => {
    try {
      const summary = describe(await run());
      const text = summary.length > 0 ? `${lead} ${summary}`.trim() : whenNothingMoved;
      await afterShiftChange(undo ? { text, undo } : { text });
    } catch (error) {
      report(error, "staff");
    }
  };

  const blockTables = (tableIds: string[], reason: string, number: number) => {
    closeSheet();
    return rearrange(
      () => api.blockTables(shiftDate, tableIds, reason),
      `Стол ${number} закрыт на вечер.`,
      `Стол ${number} закрыт на вечер. Броней там не было.`,
      { label: "Вернуть", run: () => void unblockTables(tableIds, number) },
    );
  };

  const unblockTables = (tableIds: string[], number: number) => {
    closeSheet();
    return rearrange(
      () => api.unblockTables(shiftDate, tableIds),
      `Стол ${number} снова в подборе.`,
      `Стол ${number} снова в подборе.`,
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

  const seatWalkIn = async () => {
    try {
      const created = await api.seatWalkIn(shiftDate, walkInParty);
      haptics.success();
      closeSheet();
      await afterShiftChange({ text: `Посадили за стол ${created.table_number}.` });
    } catch (error) {
      report(error, "staff");
      await loadShift(shiftDate);
    }
  };

  const createManualBooking = async () => {
    if (manual.minutes === null) return;
    try {
      const created = await api.createStaffBooking(
        shiftDate,
        manual.minutes,
        manual.partySize,
        manual.name,
      );
      setManual({ name: "", partySize: DEFAULT_PARTY, minutes: null });
      closeSheet();
      await afterShiftChange({
        text: `${created.guest_name} записан на ${fmt.time(created.start_minutes)}, стол ${created.table_number}.`,
      });
    } catch (error) {
      report(error, "staff");
      void loadManualAvailability();
    }
  };

  const saveSettings = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      const saved = await api.saveSettings(shiftDate, draft);
      setSettings(saved.settings);
      setDraft(draftOf(saved.settings));
      const parts = ["Настройки сохранены.", describe(saved.reconciliation)];
      if (saved.above_cap > 0) {
        parts.push(`${fmt.bookings(saved.above_cap)} больше нового лимита — они остаются в силе.`);
      }
      tell(parts.filter(Boolean).join(" "));
      await reload();
    } catch (error) {
      const failure = report(error, "staff");
      if (failure.code === "would_strand_bookings") {
        setSheet({ kind: "conflict", reasons: [messageFor(failure, "staff")] });
      } else if (failure.code === "settings_invalid") {
        setSheet({ kind: "conflict", reasons: invalidReasons(failure) });
      }
    } finally {
      setSaving(false);
    }
  };

  // ---- what the shell is given ------------------------------------------------------------------

  const settingsDirty = settings !== null && draft !== null && differs(draft, draftOf(settings));
  const staffFooter =
    tab === "shift" && shift !== null && !shift.hours.closed ? (
      <ShiftActions
        isToday={isToday}
        onWalkIn={() => {
          setWalkInParty(DEFAULT_PARTY);
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
              if (attendance === "arrived") seat(sheet.booking);
              else if (attendance === "left") markLeft(sheet.booking);
              else if (attendance === "no_show") markNoShow(sheet.booking);
              else
                void setAttendance(sheet.booking, "confirmed", (updated) => ({
                  text: `${updated.guest_name}: снова ждём.`,
                }));
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
              if (sheet.kind === "table") void unblockTables(tableIds, sheet.table.number);
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
            onClose={closeSheet}
            onPartySize={setWalkInParty}
            onSeat={() => void seatWalkIn()}
          />

          <ManualBookingSheet
            open={sheet.kind === "manual"}
            maxParty={bar.max_party}
            availability={manualAvailability}
            partySize={manual.partySize}
            chosenMinutes={manual.minutes}
            guestName={manual.name}
            failedToLoad={manualFailed}
            onClose={closeSheet}
            onPartySize={(size) =>
              setManual((current) => ({ ...current, partySize: size, minutes: null }))
            }
            onPick={(minutes) => setManual((current) => ({ ...current, minutes }))}
            onTakenSlot={() => tell("Это время занято. Свободное — без зачёркивания.")}
            onGuestName={(name) => setManual((current) => ({ ...current, name }))}
            onRetry={() => void loadManualAvailability()}
            onCreate={() => void createManualBooking()}
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
          onWriteToBar={() => openBotChat(process.env.NEXT_PUBLIC_BOT_USERNAME ?? "", "hello")}
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
          failedToLoad={pickerFailed}
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
        <DoneScreen booking={session.booking} bar={bar} />
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
          />
        ) : (
          <Spinner label="Читаем настройки" />
        )
      ) : null}
    </AppShell>
  );
}
