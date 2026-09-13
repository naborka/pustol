/**
 * Edits to existing list items, naming item by identity, not position.
 *
 * Edit made during save replays on what save stored, and server lists staff by username and tables
 * by number, not send order: position taken before answer names other item after. Messages and
 * cancel reasons keep send order, so their edits may use position.
 */

import type { Edit } from "./settingsRules";

/**
 * Matches exact spelling. Rows one case apart both show until save refuses them; server stores sent
 * case, so tapped row goes.
 */
export function removeStaff(username: string): Edit {
  return (draft) => {
    const index = draft.staff.findIndex((member) => member.username === username);
    if (index >= 0) draft.staff.splice(index, 1);
  };
}

export function removeTable(id: string): Edit {
  return (draft) => {
    draft.tables = draft.tables.filter((table) => table.id !== id);
  };
}

export function resizeTable(id: string, delta: number): Edit {
  return (draft) => {
    const table = draft.tables.find((each) => each.id === id);
    if (table) table.seats += delta;
  };
}

export function moveTableTo(id: string, zone: string): Edit {
  return (draft) => {
    const table = draft.tables.find((each) => each.id === id);
    if (table) table.zone = zone;
  };
}
