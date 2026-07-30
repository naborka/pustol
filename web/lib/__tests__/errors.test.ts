import { describe, expect, it } from "vitest";

import {
  invalidReasons,
  messageFor,
  needsRelaunch,
  strandedBookingIds,
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
    ]) {
      expect(messageFor(failure(code), "staff"), code).not.toBe(
        messageFor(failure("something_unmapped"), "staff"),
      );
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
  it("lists the bookings a change would strand", () => {
    expect(
      strandedBookingIds(
        failure("would_strand_bookings", {
          conflicts: [{ booking_id: "one" }, { booking_id: "two" }],
        }),
      ),
    ).toEqual(["one", "two"]);
  });

  it("survives a detail that is not the shape it expected", () => {
    expect(strandedBookingIds(failure("would_strand_bookings"))).toEqual([]);
    expect(strandedBookingIds(failure("would_strand_bookings", { conflicts: "none" }))).toEqual([]);
    expect(
      strandedBookingIds(failure("would_strand_bookings", { conflicts: [null, 7, {}] })),
    ).toEqual([]);
    expect(strandedBookingIds(failure("no_table_free"))).toEqual([]);
  });

  it("lists the reasons a proposal was illegal", () => {
    expect(
      invalidReasons(failure("settings_invalid", { reasons: ["первая", "вторая"] })),
    ).toEqual(["первая", "вторая"]);
    expect(invalidReasons(failure("settings_invalid"))).toEqual([]);
    expect(invalidReasons(failure("no_table_free", { reasons: ["x"] }))).toEqual([]);
  });
});
