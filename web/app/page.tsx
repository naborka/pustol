"use client";

/**
 * The whole app, one screen deep.
 *
 * A Mini App has no address bar and no history to speak of, so navigation is state rather than
 * routing: three guest screens, two staff screens and a set of sheets. Keeping that state here — and
 * every fetch with it — means the screens stay pure functions of what is loaded, which is what makes
 * them worth testing.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import {
  ApiError,
  client as makeClient,
  draftOf,
  type Availability,
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
import { credentials, haptics, openBotChat, webApp } from "@/lib/telegram";
import { ShiftScreen } from "@/components/AdminShift";
import { AppShell, InsetFrame, type StaffTab } from "@/components/AppChrome";
import { BookScreen, DoneScreen, HomeScreen } from "@/components/GuestScreens";
import { SettingsScreen } from "@/components/Settings";
import { useInsets } from "@/components/ThemeProvider";
import {
  BlockSheet,
  BookingSheet,
  ChoiceSheet,
  ConflictSheet,
  NewBookingSheet,
} from "@/components/Sheets";
import { Failure, MainButton, Spinner, Toast } from "@/components/ui";

type Tab = StaffTab;
type GuestScreen = "home" | "book" | "done";

type OpenSheet =
  | { kind: "none" }
  | { kind: "booking"; booking: ShiftBooking }
  | { kind: "templates"; booking: ShiftBooking }
  | { kind: "cancel"; booking: ShiftBooking }
  | { kind: "block"; table: ShiftTable }
  | { kind: "conflict"; reasons: string[] }
  | { kind: "new" };

export default function Page() {
  const token = useMemo(() => credentials(), []);
  const api = useMemo(() => (token === null ? null : makeClient(token)), [token]);

  const [session, setSession] = useState<Session | null>(null);
  const [fatal, setFatal] = useState<ApiFailure | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const [tab, setTab] = useState<Tab>("client");
  const [screen, setScreen] = useState<GuestScreen>("home");

  const [partySize, setPartySize] = useState(2);
  const [serviceDate, setServiceDate] = useState<string | null>(null);
  const [chosenMinutes, setChosenMinutes] = useState<number | null>(null);
  const [daytimeShown, setDaytimeShown] = useState(false);
  const [availability, setAvailability] = useState<Availability | null>(null);

  const [shiftDate, setShiftDate] = useState<string | null>(null);
  const [shiftView, setShiftView] = useState<"timeline" | "list">("timeline");
  const [shift, setShift] = useState<ShiftView | null>(null);

  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [editedWeekday, setEditedWeekday] = useState(1);
  const [saving, setSaving] = useState(false);

  const insets = useInsets();
  const [sheet, setSheet] = useState<OpenSheet>({ kind: "none" });
  const [newBooking, setNewBooking] = useState({ name: "", partySize: 2, minutes: null as number | null, daytimeShown: false });
  const [newAvailability, setNewAvailability] = useState<Availability | null>(null);

  /** Shows a message and clears it, so the screen does not accumulate stale outcomes. */
  const say = useCallback((text: string) => {
    setToast(text);
    window.setTimeout(() => setToast((current) => (current === text ? null : current)), 4200);
  }, []);

  /** Turns a failure into words for whoever is looking at it. */
  const report = useCallback(
    (error: unknown, audience: "guest" | "staff") => {
      const failure =
        error instanceof ApiError
          ? error.failure
          : { code: "internal", message: String(error) };
      if (needsRelaunch(failure)) {
        setFatal(failure);
        return failure;
      }
      haptics.error();
      say(messageFor(failure, audience));
      return failure;
    },
    [say],
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

  // The picker recomputes whenever the question changes. Every answer comes from the server, which
  // has run the real allocator: a time shown as free is a time with a table behind it.
  useEffect(() => {
    if (!api || serviceDate === null || screen !== "book") return;
    let current = true;
    setAvailability(null);
    api
      .availability(serviceDate, partySize)
      .then((next) => {
        if (current) setAvailability(next);
      })
      .catch((error: unknown) => {
        if (current) report(error, "guest");
      });
    return () => {
      current = false;
    };
  }, [api, serviceDate, partySize, screen, report]);

  const loadShift = useCallback(
    async (date: string) => {
      if (!api) return;
      try {
        setShift(await api.shift(date));
      } catch (error) {
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
  useEffect(() => {
    if (!api || sheet.kind !== "new" || shiftDate === null) return;
    let current = true;
    setNewAvailability(null);
    api
      .staffAvailability(shiftDate, newBooking.partySize)
      .then((next) => {
        if (current) setNewAvailability(next);
      })
      .catch((error: unknown) => {
        if (current) report(error, "staff");
      });
    return () => {
      current = false;
    };
  }, [api, sheet.kind, shiftDate, newBooking.partySize, report]);

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
        <Spinner />
      </InsetFrame>
    );
  }

  const bar = session.bar;
  const closeSheet = () => setSheet({ kind: "none" });

  // ---- guest actions --------------------------------------------------------------------------

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
      if (serviceDate !== null) {
        api.availability(serviceDate, partySize).then(setAvailability).catch(() => {});
      }
    }
  };

  const cancelMine = async () => {
    try {
      await api.cancelMine();
      haptics.success();
      say("Бронь отменена. Стол снова свободен.");
      await reload();
    } catch (error) {
      report(error, "guest");
    }
  };

  const enableReminders = async () => {
    try {
      await api.optInToReminders();
      await reload();
      openBotChat(process.env.NEXT_PUBLIC_BOT_USERNAME ?? "");
      say(`Напомним за ${fmt.hoursWord(bar.remind_hours)} до брони.`);
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

  const mainAction = () => {
    if (screen === "done") {
      setScreen("home");
      return;
    }
    if (screen === "home") {
      const start = session.booking?.service_date ?? bar.today;
      setPartySize(session.booking?.party_size ?? 2);
      setServiceDate(session.bookable_days.includes(start) ? start : (session.bookable_days[0] ?? start));
      setChosenMinutes(null);
      setDaytimeShown(false);
      setScreen("book");
      return;
    }
    void book();
  };

  const mainLabel =
    screen === "done"
      ? "На главную"
      : screen === "book"
        ? chosenMinutes === null
          ? "Выберите время"
          : `Забронировать на ${fmt.time(chosenMinutes)}`
        : session.booking
          ? "Изменить бронь"
          : "Забронировать стол";

  // ---- staff actions -------------------------------------------------------------------------

  const afterShiftChange = async (message?: string) => {
    closeSheet();
    await loadShift(shiftDate);
    if (message) say(message);
  };

  const describe = (outcome: Reconciliation) => {
    const parts: string[] = [];
    if (outcome.moved.length > 0) {
      parts.push(
        `Пересажены: ${outcome.moved.map((entry) => `${entry.guest_name} → стол ${entry.to_number}`).join(", ")}.`,
      );
    }
    if (outcome.orphaned.length > 0) {
      parts.push(
        `Остались без стола: ${outcome.orphaned.map((entry) => entry.guest_name).join(", ")}.`,
      );
    }
    return parts.join(" ");
  };

  const setAttendance = async (booking: ShiftBooking, attendance: "confirmed" | "arrived" | "no_show") => {
    try {
      const updated = await api.setAttendance(booking.id, attendance);
      setSheet({ kind: "booking", booking: updated });
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
      await afterShiftChange(
        [`Бронь отменена. ${told}`, describe(outcome.reconciliation)].filter(Boolean).join(" "),
      );
    } catch (error) {
      report(error, "staff");
    }
  };

  const sendTemplate = async (booking: ShiftBooking, text: string) => {
    try {
      await api.sendTemplate(booking.id, text);
      closeSheet();
      say(`Отправлено ${booking.guest_name}: «${text}»`);
    } catch (error) {
      report(error, "staff");
    }
  };

  /**
   * Runs something that rearranges the shift, then reloads and reports what moved.
   *
   * The three actions that do this — closing a table, opening one, asking the room to try again —
   * differ only in the call and in what to say when nothing needed moving. Sharing the rest means a
   * fourth cannot forget to reload, or report differently.
   */
  const rearrange = async (
    run: () => Promise<Reconciliation>,
    whenNothingMoved: string,
  ) => {
    try {
      const summary = describe(await run());
      await afterShiftChange(summary.length > 0 ? summary : whenNothingMoved);
    } catch (error) {
      report(error, "staff");
    }
  };

  const blockTables = (tableIds: string[], reason: string) =>
    rearrange(
      () => api.blockTables(shiftDate, tableIds, reason),
      "Закрыто. Броней там не было.",
    );

  const unblockTables = (tableIds: string[]) =>
    rearrange(() => api.unblockTables(shiftDate, tableIds), "Стол снова в подборе.");

  const findTables = () =>
    rearrange(
      () => api.reconcileShift(shiftDate),
      "Свободных столов на это время нет. Откройте закрытый стол или предложите другое время.",
    );

  const createStaffBooking = async () => {
    if (newBooking.minutes === null) return;
    try {
      const created = await api.createStaffBooking(
        shiftDate,
        newBooking.minutes,
        newBooking.partySize,
        newBooking.name,
      );
      setNewBooking({ name: "", partySize: 2, minutes: null, daytimeShown: false });
      await afterShiftChange(
        `${created.guest_name} записан на ${fmt.time(created.start_minutes)}, стол ${created.table_number}.`,
      );
    } catch (error) {
      report(error, "staff");
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
      say(parts.filter(Boolean).join(" "));
      await reload();
    } catch (error) {
      const failure = report(error, "staff");
      if (failure.code === "would_strand_bookings") {
        setSheet({
          kind: "conflict",
          reasons: [messageFor(failure, "staff")],
        });
      } else if (failure.code === "settings_invalid") {
        setSheet({ kind: "conflict", reasons: invalidReasons(failure) });
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <AppShell
        staff={session.is_staff}
        tab={tab}
        onTab={setTab}
        insets={insets}
        footer={
          tab === "client" && sheet.kind === "none" ? (
            <MainButton
              label={mainLabel}
              onClick={mainAction}
              enabled={!(screen === "book" && chosenMinutes === null)}
            />
          ) : undefined
        }
      >
        {tab === "client" && screen === "home" ? (
          <HomeScreen
            session={session}
            onCancel={() => void cancelMine()}
            onEnableReminders={() => void enableReminders()}
            onDismissReminders={() => void dismissReminders()}
            onWriteToBar={() => openBotChat(process.env.NEXT_PUBLIC_BOT_USERNAME ?? "", "hello")}
          />
        ) : null}

        {tab === "client" && screen === "book" ? (
          <BookScreen
            bar={bar}
            bookableDays={session.bookable_days}
            availability={availability}
            partySize={partySize}
            serviceDate={serviceDate}
            chosenMinutes={chosenMinutes}
            daytimeShown={daytimeShown}
            onPartySize={(size) => {
              setPartySize(size);
              setChosenMinutes(null);
            }}
            onServiceDate={(date) => {
              setServiceDate(date);
              setChosenMinutes(null);
            }}
            onPick={setChosenMinutes}
            onShowDaytime={() => setDaytimeShown(true)}
            {...(webApp()?.BackButton ? {} : { onBack: () => setScreen("home") })}
          />
        ) : null}

        {tab === "client" && screen === "done" && session.booking ? (
          <DoneScreen
            booking={session.booking}
            session={session}
            onEnableReminders={() => void enableReminders()}
          />
        ) : null}

        {tab === "shift" ? (
          shift ? (
            <ShiftScreen
              shift={shift}
              today={bar.today}
              view={shiftView}
              onView={setShiftView}
              onServiceDate={setShiftDate}
              onOpenBooking={(booking) => setSheet({ kind: "booking", booking })}
              onTapTable={(table) => setSheet({ kind: "block", table })}
              onNewBooking={() => setSheet({ kind: "new" })}
              onFindTables={() => void findTables()}
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
              onSave={() => void saveSettings()}
              onRevert={() => setDraft(draftOf(settings))}
              saving={saving}
            />
          ) : (
            <Spinner label="Читаем настройки" />
          )
        ) : null}
      </AppShell>

      <BookingSheet
        open={sheet.kind === "booking"}
        booking={sheet.kind === "booking" ? sheet.booking : null}
        onClose={closeSheet}
        onAttendance={(attendance) => {
          if (sheet.kind === "booking") void setAttendance(sheet.booking, attendance);
        }}
        onOpenTemplates={() => {
          if (sheet.kind !== "booking") return;
          if (!sheet.booking.reachable_by_bot) {
            say(messageFor({ code: "no_bot_chat", message: "" }, "staff"));
            return;
          }
          setSheet({ kind: "templates", booking: sheet.booking });
        }}
        onOpenCancel={() => {
          if (sheet.kind === "booking") setSheet({ kind: "cancel", booking: sheet.booking });
        }}
        onFindTable={() => void findTables()}
      />

      <ChoiceSheet
        open={sheet.kind === "templates"}
        title="Сообщение гостю"
        hint={`Уйдёт от бота в чат гостя. ${
          sheet.kind === "templates" ? sheet.booking.guest_name : ""
        } получит его сразу.`}
        choices={shift?.message_templates ?? []}
        onClose={closeSheet}
        onChoose={(text) => {
          if (sheet.kind === "templates") void sendTemplate(sheet.booking, text);
        }}
      />

      <ChoiceSheet
        open={sheet.kind === "cancel"}
        title="Причина отмены"
        hint="Гость получит сообщение с этой причиной. Стол сразу освободится."
        choices={shift?.cancel_reasons ?? []}
        onClose={closeSheet}
        onChoose={(reason) => {
          if (sheet.kind === "cancel") void cancelAsStaff(sheet.booking, reason);
        }}
      />

      <BlockSheet
        // Keyed by table, so opening the sheet for a table in another zone does not inherit the
        // "whole zone" choice made about the last one.
        key={sheet.kind === "block" ? sheet.table.id : "no-table"}
        open={sheet.kind === "block"}
        table={sheet.kind === "block" ? sheet.table : null}
        tables={shift?.tables ?? []}
        bookings={shift?.bookings ?? []}
        onClose={closeSheet}
        onBlock={(tableIds, reason) => void blockTables(tableIds, reason)}
        onUnblock={(tableIds) => void unblockTables(tableIds)}
      />

      <ConflictSheet
        open={sheet.kind === "conflict"}
        reasons={sheet.kind === "conflict" ? sheet.reasons : []}
        onClose={closeSheet}
      />

      <NewBookingSheet
        open={sheet.kind === "new"}
        maxParty={bar.max_party}
        availability={newAvailability}
        partySize={newBooking.partySize}
        chosenMinutes={newBooking.minutes}
        guestName={newBooking.name}
        daytimeShown={newBooking.daytimeShown}
        onClose={closeSheet}
        onPartySize={(size) =>
          setNewBooking((current) => ({ ...current, partySize: size, minutes: null }))
        }
        onPick={(minutes) => setNewBooking((current) => ({ ...current, minutes }))}
        onGuestName={(name) => setNewBooking((current) => ({ ...current, name }))}
        onShowDaytime={() => setNewBooking((current) => ({ ...current, daytimeShown: true }))}
        onCreate={() => void createStaffBooking()}
      />

      <Toast text={toast} />
    </>
  );
}
