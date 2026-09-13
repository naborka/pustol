/** Edits name list item by identity, never position. */

import { describe, expect, it } from "vitest";

import { draftOf } from "../api";
import { moveTableTo, removeStaff, removeTable, resizeTable } from "../settingsEdits";
import { edited } from "../settingsRules";
import { settingsView } from "@/components/__tests__/fixtures";

const base = draftOf(settingsView());

describe("a change to a member of staff", () => {
  it("names the member by the username of the row tapped, as it was typed, wherever the list has them", () => {
    // Rows differing only by case stay two rows until save refuses; tap removes only tapped one.
    const sorted = {
      ...base,
      staff: [{ username: "aaron_bar" }, { username: "marina" }, { username: "pavel" }, { username: "Pavel" }],
    };
    expect(edited(sorted, removeStaff("Pavel")).staff).toEqual([
      { username: "aaron_bar" },
      { username: "marina" },
      { username: "pavel" },
    ]);
    expect(edited(sorted, removeStaff("pavel")).staff).toEqual([
      { username: "aaron_bar" },
      { username: "marina" },
      { username: "Pavel" },
    ]);
    expect(edited(sorted, removeStaff("nobody_here")).staff).toEqual(sorted.staff);
  });
});

describe("a change to a table", () => {
  it("names the table by its id, wherever the list has it", () => {
    const reversed = { ...base, tables: [...base.tables].reverse() };
    expect(edited(reversed, resizeTable("t1", 1)).tables).toEqual([
      { id: "t2", seats: 6, zone: "Зал" },
      { id: "t1", seats: 3, zone: "Стойка" },
    ]);
    expect(edited(reversed, moveTableTo("t1", "Веранда")).tables[1]).toEqual({
      id: "t1",
      seats: 2,
      zone: "Веранда",
    });
    expect(edited(reversed, removeTable("t2")).tables).toEqual([{ id: "t1", seats: 2, zone: "Стойка" }]);
  });

  it("changes nothing when the table is no longer there", () => {
    for (const change of [resizeTable("gone", 1), moveTableTo("gone", "Зал"), removeTable("gone")]) {
      expect(edited(base, change)).toEqual(base);
    }
  });
});
