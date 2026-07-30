import { describe, expect, it } from "vitest";

import type { Limits, SettingsDraft } from "../api";
import {
  copyDraft,
  differs,
  isLegal,
  isTelegramUsername,
  lastArrivalMinutes,
  reasonsAgainst,
  shortestShiftMinutes,
  wouldBeLegal,
} from "../settingsRules";

const LIMITS: Limits = {
  open_minutes: { min: 480, max: 1_080 },
  close_minutes: { min: 1_200, max: 1_680 },
  turn_minutes: { min: 60, max: 240 },
  max_party: { min: 2, max: 10 },
  horizon_days: { min: 1, max: 30 },
  remind_hours: { min: 1, max: 12 },
  grace_minutes: { min: 5, max: 60 },
  seats: { min: 1, max: 12 },
  slot_step_minutes: [15, 30, 60],
};

function draft(overrides: Partial<SettingsDraft> = {}): SettingsDraft {
  return {
    name: "Бар «Подвал»",
    address: "Дечанска 12, Белград",
    timezone: "Europe/Belgrade",
    week: Array.from({ length: 7 }, () => ({
      open_minutes: 600,
      close_minutes: 1_560,
      closed: false,
    })),
    zones: ["Бар", "Зал", "Терраса"],
    tables: [
      { kind: "existing", id: "t1", seats: 2, zone: "Бар" },
      { kind: "existing", id: "t2", seats: 4, zone: "Зал" },
      { kind: "existing", id: "t3", seats: 6, zone: "Терраса" },
    ],
    turn_minutes: 120,
    slot_step_minutes: 30,
    max_party: 6,
    horizon_days: 4,
    remind_hours: 3,
    grace_minutes: 15,
    message_templates: ["Ваш стол готов, ждём вас!"],
    cancel_reasons: ["Частное мероприятие"],
    staff: [{ username: "anna_mgr" }],
    ...overrides,
  };
}

const kinds = (proposal: SettingsDraft) =>
  reasonsAgainst(proposal, LIMITS).map((reason) => reason.kind);

describe("what makes a proposal legal", () => {
  it("accepts the bar as it stands", () => {
    expect(reasonsAgainst(draft(), LIMITS)).toEqual([]);
    expect(isLegal(draft(), LIMITS)).toBe(true);
  });

  it("needs a name and an address", () => {
    expect(kinds(draft({ name: "   " }))).toContain("blank_name");
    expect(kinds(draft({ address: "" }))).toContain("blank_address");
  });

  it("keeps opening and closing times inside what the bar offers", () => {
    const early = draft();
    early.week[3] = { open_minutes: 60, close_minutes: 1_560, closed: false };
    expect(kinds(early)).toContain("open_out_of_range");

    const late = draft();
    late.week[3] = { open_minutes: 600, close_minutes: 1_800, closed: false };
    expect(kinds(late)).toContain("close_out_of_range");
  });

  it("refuses a week with no open day at all", () => {
    const shut = draft();
    shut.week = shut.week.map((hours) => ({ ...hours, closed: true }));
    expect(kinds(shut)).toContain("every_day_closed");
  });

  it("refuses a shift too short to hold one booking", () => {
    const proposal = draft({ turn_minutes: 240 });
    proposal.week[1] = { open_minutes: 1_020, close_minutes: 1_200, closed: false };
    expect(kinds(proposal)).toContain("shift_shorter_than_turn");
  });

  it("accepts a shift exactly one booking long", () => {
    const proposal = draft({ turn_minutes: 240 });
    proposal.week = proposal.week.map(() => ({
      open_minutes: 1_020,
      close_minutes: 1_260,
      closed: false,
    }));
    expect(reasonsAgainst(proposal, LIMITS)).toEqual([]);
  });

  it("leaves a day off out of the turn-length question", () => {
    const proposal = draft({ turn_minutes: 240 });
    proposal.week[1] = { open_minutes: 1_020, close_minutes: 1_200, closed: true };
    expect(kinds(proposal)).not.toContain("shift_shorter_than_turn");
  });

  it("bounds every numeric rule", () => {
    expect(kinds(draft({ turn_minutes: 30 }))).toContain("setting_out_of_range");
    expect(kinds(draft({ horizon_days: 0 }))).toContain("setting_out_of_range");
    expect(kinds(draft({ remind_hours: 13 }))).toContain("setting_out_of_range");
    expect(kinds(draft({ grace_minutes: 1 }))).toContain("setting_out_of_range");
  });

  it("only offers the time steps the bar supports", () => {
    expect(kinds(draft({ slot_step_minutes: 20 }))).toContain("slot_step_not_offered");
    for (const step of LIMITS.slot_step_minutes) {
      expect(reasonsAgainst(draft({ slot_step_minutes: step }), LIMITS)).toEqual([]);
    }
  });

  it("refuses a party cap no table can seat", () => {
    // Otherwise the guest is offered a size for which every slot is grey, with nothing on screen
    // to explain why.
    expect(kinds(draft({ max_party: 8 }))).toContain("max_party_exceeds_largest_table");
  });

  it("refuses an empty room and an unreasonable table", () => {
    expect(kinds(draft({ tables: [] }))).toContain("no_tables");
    const huge = draft();
    huge.tables[0] = { kind: "existing", id: "t1", seats: 20, zone: "Бар" };
    expect(kinds(huge)).toContain("seats_out_of_range");
  });

  it("refuses a table standing in a zone the bar does not have", () => {
    const stray = draft();
    stray.tables[0] = { kind: "existing", id: "t1", seats: 2, zone: "Подвал" };
    expect(kinds(stray)).toContain("unknown_zone");
  });

  it("refuses zones that are missing or repeated", () => {
    expect(kinds(draft({ zones: [] }))).toContain("no_zones");
    expect(kinds(draft({ zones: ["Зал", "Зал"] }))).toContain("duplicate_zone");
  });

  it("refuses an empty or blank list of things to say to a guest", () => {
    expect(kinds(draft({ message_templates: [] }))).toContain("no_message_templates");
    expect(kinds(draft({ message_templates: ["  "] }))).toContain("blank_message_template");
    expect(kinds(draft({ cancel_reasons: [] }))).toContain("no_cancel_reasons");
    expect(kinds(draft({ cancel_reasons: [""] }))).toContain("blank_cancel_reason");
  });

  it("refuses to lock everybody out of the admin side", () => {
    expect(kinds(draft({ staff: [] }))).toContain("no_staff");
  });

  it("insists an admin username could actually be a Telegram one", () => {
    expect(kinds(draft({ staff: [{ username: "9lives" }] }))).toContain(
      "malformed_staff_username",
    );
    expect(
      kinds(draft({ staff: [{ username: "anna_mgr" }, { username: "ANNA_MGR" }] })),
    ).toContain("duplicate_staff_username");
  });

  it("reports every reason at once rather than the first", () => {
    const broken = draft({ name: "", staff: [], max_party: 9 });
    const reported = kinds(broken);
    expect(reported).toContain("blank_name");
    expect(reported).toContain("no_staff");
    expect(reported.length).toBeGreaterThanOrEqual(3);
  });
});

describe("Telegram username shape", () => {
  it("follows the published rule", () => {
    expect(isTelegramUsername("anna_mgr")).toBe(true);
    expect(isTelegramUsername("a1234")).toBe(true);
    expect(isTelegramUsername("anna")).toBe(false);
    expect(isTelegramUsername("a".repeat(33))).toBe(false);
    expect(isTelegramUsername("_anna")).toBe(false);
    expect(isTelegramUsername("anna-mgr")).toBe(false);
    expect(isTelegramUsername("анна_менеджер")).toBe(false);
  });
});

describe("asking whether an edit would be allowed", () => {
  it("answers for the whole proposal, not the field in isolation", () => {
    // Raising the turn length is refused because of the shortest shift, which says nothing about
    // turns on its own.
    const tight = draft({ turn_minutes: 180 });
    tight.week = tight.week.map(() => ({
      open_minutes: 1_020,
      close_minutes: 1_200,
      closed: false,
    }));
    expect(
      wouldBeLegal(tight, (next) => {
        next.turn_minutes += 30;
      }, LIMITS),
    ).toBe(false);
    expect(
      wouldBeLegal(tight, (next) => {
        next.turn_minutes -= 30;
      }, LIMITS),
    ).toBe(true);
  });

  it("leaves the proposal it was asked about untouched", () => {
    const proposal = draft();
    const before = JSON.stringify(proposal);
    wouldBeLegal(proposal, (next) => {
      next.max_party = 99;
    }, LIMITS);
    expect(JSON.stringify(proposal)).toBe(before);
  });

  it("refuses to shrink the last big table under the party cap", () => {
    const proposal = draft();
    expect(
      wouldBeLegal(
        proposal,
        (next) => {
          const table = next.tables[2];
          if (table) table.seats = 4;
        },
        LIMITS,
      ),
    ).toBe(false);
  });

  it("allows shrinking a table the cap does not depend on", () => {
    const proposal = draft();
    expect(
      wouldBeLegal(
        proposal,
        (next) => {
          const table = next.tables[1];
          if (table) table.seats = 3;
        },
        LIMITS,
      ),
    ).toBe(true);
  });
});

describe("derived facts the screen shows", () => {
  it("computes the last arrival rather than storing it", () => {
    expect(lastArrivalMinutes(draft(), 4)).toBe(1_440);
    expect(lastArrivalMinutes(draft({ turn_minutes: 90 }), 4)).toBe(1_470);
    const closed = draft();
    closed.week[4] = { open_minutes: 600, close_minutes: 1_560, closed: true };
    expect(lastArrivalMinutes(closed, 4)).toBeNull();
  });

  it("finds the shortest open shift, which is what caps the turn", () => {
    const proposal = draft();
    proposal.week[2] = { open_minutes: 1_020, close_minutes: 1_260, closed: false };
    expect(shortestShiftMinutes(proposal)).toBe(240);
  });

  it("has no shortest shift when the bar is shut all week", () => {
    const shut = draft();
    shut.week = shut.week.map((hours) => ({ ...hours, closed: true }));
    expect(shortestShiftMinutes(shut)).toBeNull();
  });

  it("notices whether anything has been edited", () => {
    expect(differs(draft(), draft())).toBe(false);
    expect(differs(draft(), draft({ max_party: 4 }))).toBe(true);
  });
});

describe("the copy an edit is tried on", () => {
  it("is deep enough that a trial edit cannot touch the original", () => {
    // A shallow copy would share the week, the tables and the lists, so asking whether a change is
    // legal would quietly make it.
    const original = draft();
    const copy = copyDraft(original);
    copy.week[0]!.closed = true;
    copy.tables[0]!.seats = 12;
    copy.zones.push("Подвал");
    copy.staff[0]!.username = "somebody_else";
    copy.message_templates.push("нет");

    expect(original.week[0]!.closed).toBe(false);
    expect(original.tables[0]!.seats).toBe(2);
    expect(original.zones).toHaveLength(3);
    expect(original.staff[0]!.username).toBe("anna_mgr");
    expect(original.message_templates).toHaveLength(1);
  });

  it("copies everything, so a round trip changes nothing", () => {
    const original = draft();
    expect(copyDraft(original)).toEqual(original);
  });
});
