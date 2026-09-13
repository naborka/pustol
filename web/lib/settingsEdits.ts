/**
 * The changes the settings screen makes to an item already in a list, each naming the item by what
 * it is rather than by where it stands.
 *
 * An edit made while a save is on its way is made again on top of what the save stored, and the
 * server lists staff by username and tables by number, not in the order they were sent: a position
 * taken before the answer names another item after it. Messages and cancel reasons are kept in the
 * order they were sent, so their edits may go by position.
 */

import { usernameKey, type Edit } from "./settingsRules";

export function removeStaff(username: string): Edit {
  return (draft) => {
    const index = draft.staff.findIndex(
      (member) => usernameKey(member.username) === usernameKey(username),
    );
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
