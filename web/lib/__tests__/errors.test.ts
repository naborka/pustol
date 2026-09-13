import { describe, expect, it } from "vitest";

import {
  invalidReasons,
  messageFor,
  needsRelaunch,
  strandedBookings,
  type ApiFailure,
} from "../errors";

const failure = (code: string, detail?: Record<string, unknown>): ApiFailure =>
  detail === undefined ? { code, message: code } : { code, message: code, detail };

describe("what a failure says", () => {
  it("speaks to a guest about a lost race the way a guest thinks about it", () => {
    expect(messageFor(failure("no_table_free"), "guest")).toContain("только что заняли");
  });

  it("tells staff something they can act on for the same code", () => {
    // A guest needs to pick another time. Staff need to know whether to seat somebody by hand.
    expect(messageFor(failure("no_table_free"), "staff")).toContain("свободного стола");
    expect(messageFor(failure("no_table_free"), "staff")).not.toBe(
      messageFor(failure("no_table_free"), "guest"),
    );
  });

  it("says something useful for a code it has never seen rather than nothing", () => {
    expect(messageFor(failure("a_code_from_the_future"), "guest")).toBeTruthy();
    expect(messageFor(null, "guest")).toBeTruthy();
  });

  it("tells staff restoring a booking that the guest already holds another tonight", () => {
    expect(messageFor(failure("already_booked_tonight"), "staff")).toBe(
      "У гостя уже есть другая бронь на этот вечер.",
    );
    expect(messageFor(failure("already_booked_tonight"), "guest")).toBe(
      "На этот вечер у вас уже есть бронь.",
    );
  });

  it("says a finished booking can be neither moved nor cancelled, to whoever tried", () => {
    const staff = messageFor(failure("booking_finished"), "staff");
    expect(staff).toMatch(/перенести/);
    expect(staff).toMatch(/отменить/);
    expect(messageFor(failure("booking_finished"), "guest")).not.toBe(
      messageFor(failure("something_unmapped"), "guest"),
    );
  });

  it("tells a manager somebody else saved the settings first, without asking them to type it again", () => {
    // The screen folds the other save into the edit; «повторите» sent managers retyping what was kept.
    const said = messageFor(failure("settings_changed"), "staff");
    expect(said).toMatch(/кто-то/i);
    expect(said).not.toMatch(/повторите/i);
  });

  it("has words for every failure the staff side can produce", () => {
    for (const code of [
      "blank_guest_name",
      "missing_block_reason",
      "unknown_cancel_reason",
      "unknown_message",
      "no_bot_chat",
      "settings_invalid",
      "settings_unreadable",
      "would_strand_bookings",
      "shift_not_bookable",
      "party_too_large",
      "guest_has_another_plan",
      "table_taken",
      "text_invalid",
    ]) {
      expect(messageFor(failure(code), "staff"), code).not.toBe(
        messageFor(failure("something_unmapped"), "staff"),
      );
    }
  });
});

describe("what the newest refusals say", () => {
  it("tells a guest their bookings changed under the button, and to look before pressing again", () => {
    expect(messageFor(failure("booking_changed"), "guest")).toBe(
      "Ваши брони изменились — проверьте и нажмите ещё раз.",
    );
  });

  it("tells staff the guest already holds another evening, and what to do about it", () => {
    expect(messageFor(failure("guest_has_another_plan"), "staff")).toBe(
      "У гостя уже есть бронь на другой вечер — сначала перенесите или отмените её.",
    );
  });

  it("names a character no text may hold, to whoever typed it", () => {
    expect(messageFor(failure("text_invalid"), "staff")).toBe("В тексте есть недопустимый символ.");
    expect(messageFor(failure("text_invalid"), "guest")).toBe("В тексте есть недопустимый символ.");
  });

  it("tells whoever holds an app too old for the server to reopen it", () => {
    for (const audience of ["guest", "staff"] as const) {
      expect(messageFor(failure("body_invalid"), audience)).toBe(
        "Приложение устарело — закройте и откройте его заново.",
      );
    }
  });

  it("says a date that does not exist does not exist", () => {
    for (const audience of ["guest", "staff"] as const) {
      expect(messageFor(failure("invalid_date"), audience)).toBe("Такой даты нет.");
    }
  });

  it("tells staff the bot cannot write to a guest without guessing why", () => {
    // A guest from the app who blocked the bot is refused the same way as one written down by hand.
    expect(messageFor(failure("no_bot_chat"), "staff")).toBe(
      "Бот не может написать этому гостю — позвоните или откройте чат.",
    );
  });

  it("tells staff what was not found could be a table as well as a booking", () => {
    const said = messageFor(failure("not_found"), "staff");
    expect(said).toMatch(/брон/);
    expect(said).toMatch(/стол/);
  });

  it("tells staff a table taken since is taken, and where to reseat", () => {
    expect(messageFor(failure("table_taken"), "staff")).toBe(
      "Стол уже заняли — пересадите бронь через «Перенести».",
    );
  });

  it("points a manager at the reasons a refused save keeps, not at marks the screen never draws", () => {
    for (const code of ["settings_invalid", "would_strand_bookings"]) {
      const said = messageFor(failure(code), "staff");
      expect(said, code).toMatch(/«Почему»/);
      expect(said, code).not.toMatch(/отмеченн|^Эти брони/);
    }
  });
});

describe("which failures the app can fix by relaunching", () => {
  it("knows an expired payload is routine and self-healing", () => {
    expect(needsRelaunch(failure("session_expired"))).toBe(true);
    expect(needsRelaunch(failure("no_credentials"))).toBe(true);
    expect(needsRelaunch(failure("not_telegram"))).toBe(true);
  });

  it("does not offer to relaunch for something relaunching will not fix", () => {
    expect(needsRelaunch(failure("no_table_free"))).toBe(false);
    expect(needsRelaunch(failure("forbidden"))).toBe(false);
    expect(needsRelaunch(null)).toBe(false);
  });
});

describe("the detail a refused settings save carries", () => {
  it("names the bookings a change would strand, with their times", () => {
    // "Эти брони уже приняты" on its own leaves a manager to work out which of thirty evenings is
    // in the way. The API has always sent the names; the screen has to use them.
    expect(
      strandedBookings(
        failure("would_strand_bookings", {
          conflicts: [
            { booking_id: "one", guest_name: "Саша", start_minutes: 1260 },
            { booking_id: "two", guest_name: "Тимур", start_minutes: 1320 },
          ],
        }),
      ),
    ).toEqual([
      { guestName: "Саша", startMinutes: 1260 },
      { guestName: "Тимур", startMinutes: 1320 },
    ]);
  });

  it("keeps a booking whose time the detail did not carry", () => {
    expect(
      strandedBookings(
        failure("would_strand_bookings", { conflicts: [{ guest_name: "Глеб" }] }),
      ),
    ).toEqual([{ guestName: "Глеб", startMinutes: null }]);
  });

  it("survives a detail that is not the shape it expected", () => {
    expect(strandedBookings(failure("would_strand_bookings"))).toEqual([]);
    expect(strandedBookings(failure("would_strand_bookings", { conflicts: "none" }))).toEqual([]);
    expect(
      strandedBookings(failure("would_strand_bookings", { conflicts: [null, 7, {}] })),
    ).toEqual([]);
    expect(strandedBookings(failure("no_table_free"))).toEqual([]);
  });

  it("lists the reasons a proposal was illegal", () => {
    expect(
      invalidReasons(failure("settings_invalid", { reasons: ["первая", "вторая"] })),
    ).toEqual(["первая", "вторая"]);
    expect(invalidReasons(failure("settings_invalid"))).toEqual([]);
    expect(invalidReasons(failure("no_table_free", { reasons: ["x"] }))).toEqual([]);
  });
});

describe("a failure that is nobody's fault here", () => {
  it("tells the person holding the phone to check their connection", () => {
    expect(messageFor(failure("network"), "guest")).toContain("интернет");
    expect(messageFor(failure("network"), "staff")).toContain("интернет");
  });
});
