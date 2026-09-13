/** Which answers reach screen, which failures it admits. */

import { describe, expect, it } from "vitest";

import {
  EMPTY_LEDGER,
  answered,
  begun,
  failed,
  failureOn,
  marked,
  pendingOn,
  pruned,
  valueOn,
  written,
  type Ledger,
  type Order,
} from "../reads";

const boom = { code: "internal", message: "boom" };
const offline = { code: "network", message: "offline" };

type Room = { version: number; name: string };
const byVersion: Order<Room> = (next, shown) => next.version - shown.version;

function ask<T>(ledger: Ledger<T>, key: string): [Ledger<T>, number] {
  return begun(ledger, key);
}

/** Write's own answer, numbered at send. */
function write<T>(
  ledger: Ledger<T>,
  key: string,
  sent: number,
  change: (current: T | undefined) => T | undefined,
  order?: Order<T>,
) {
  return written(ledger, key, sent, change, order);
}

describe("an answer", () => {
  it("is applied when its question has no answer yet, whatever another question has", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let tonight: number;
    let tomorrow: number;
    [ledger, tonight] = ask(ledger, "11");
    [ledger, tomorrow] = ask(ledger, "12");
    ledger = answered(ledger, tomorrow, "12", "tomorrow").ledger;
    const late = answered(ledger, tonight, "11", "tonight");
    expect(late.apply).toBe(true);
    expect(valueOn(late.ledger, "11")).toBe("tonight");
    expect(valueOn(late.ledger, "12")).toBe("tomorrow");
  });

  it("is applied, and keeps the failure of a later ask of the same question, which is newer news", () => {
    // Shown value older than failed read; screen admits it.
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let first: number;
    let retry: number;
    [ledger, first] = ask(ledger, "11|2");
    [ledger, retry] = ask(ledger, "11|2");
    ledger = failed(ledger, retry, "11|2", boom).ledger;
    const outcome = answered(ledger, first, "11|2", { version: 1, name: "times" }, byVersion);
    expect(outcome.apply).toBe(true);
    expect(valueOn(outcome.ledger, "11|2")?.name).toBe("times");
    expect(failureOn(outcome.ledger, "11|2")).toEqual(boom);

    let unordered: Ledger<string> = EMPTY_LEDGER;
    [unordered, first] = ask(unordered, "11|2");
    [unordered, retry] = ask(unordered, "11|2");
    unordered = failed(unordered, retry, "11|2", boom).ledger;
    unordered = answered(unordered, first, "11|2", "times").ledger;
    expect(failureOn(unordered, "11|2")).toEqual(boom);
  });

  it("is dropped when a later ask of the same question already answered", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let old: number;
    let fresh: number;
    [ledger, old] = ask(ledger, "session");
    [ledger, fresh] = ask(ledger, "session");
    ledger = answered(ledger, fresh, "session", "fresh").ledger;
    const late = answered(ledger, old, "session", "old");
    expect(late.apply).toBe(false);
    expect(valueOn(late.ledger, "session")).toBe("fresh");
  });

  it("goes by the order it is given instead of by when it was asked, when it has one", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let first: number;
    let second: number;
    [ledger, first] = ask(ledger, "11");
    [ledger, second] = ask(ledger, "11");
    ledger = answered(ledger, second, "11", { version: 4, name: "second" }, byVersion).ledger;
    expect(answered(ledger, first, "11", { version: 5, name: "first" }, byVersion).apply).toBe(true);
    expect(answered(ledger, first, "11", { version: 3, name: "first" }, byVersion).apply).toBe(false);
  });

  it("goes by when it was asked when the order it is given cannot tell two answers apart", () => {
    // Room version ignores clock and bot reachability, so same-version rooms differ; later ask is fresher.
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let older: number;
    let newer: number;
    [ledger, older] = ask(ledger, "11");
    [ledger, newer] = ask(ledger, "11");
    ledger = answered(ledger, newer, "11", { version: 5, name: "21:40" }, byVersion).ledger;
    const late = answered(ledger, older, "11", { version: 5, name: "21:30" }, byVersion);
    expect(late.apply).toBe(false);
    expect(valueOn(late.ledger, "11")?.name).toBe("21:40");

    let reversed: Ledger<Room> = EMPTY_LEDGER;
    [reversed, older] = ask(reversed, "11");
    [reversed, newer] = ask(reversed, "11");
    reversed = answered(reversed, older, "11", { version: 5, name: "21:30" }, byVersion).ledger;
    expect(answered(reversed, newer, "11", { version: 5, name: "21:40" }, byVersion).apply).toBe(true);
  });

  it("breaks a tie by when the answer on record was asked, not by the newest ask that answered", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let first: number;
    let second: number;
    let third: number;
    [ledger, first] = ask(ledger, "11");
    [ledger, second] = ask(ledger, "11");
    [ledger, third] = ask(ledger, "11");
    ledger = answered(ledger, third, "11", { version: 5, name: "third" }, byVersion).ledger;
    ledger = answered(ledger, first, "11", { version: 6, name: "first" }, byVersion).ledger;
    const tie = answered(ledger, second, "11", { version: 6, name: "second" }, byVersion);
    expect(tie.apply).toBe(true);
    expect(valueOn(tie.ledger, "11")?.name).toBe("second");
  });
});

describe("a write that puts its own answer on screen", () => {
  it("outranks every read asked before it", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let tick: number;
    let sent: number;
    [ledger, tick] = ask(ledger, "11");
    [ledger, sent] = marked(ledger);
    const outcome = write(ledger, "11", sent, () => "written");
    expect(outcome.apply).toBe(true);
    ledger = outcome.ledger;
    expect(answered(ledger, tick, "11", "tick").apply).toBe(false);
    expect(failed(ledger, tick, "11", boom).recorded).toBe(false);
  });

  it("is made on the value on record for its own question", () => {
    let ledger: Ledger<string[]> = EMPTY_LEDGER;
    let read: number;
    let sent: number;
    [ledger, read] = ask(ledger, "session");
    ledger = answered(ledger, read, "session", ["b1", "b2"]).ledger;
    [ledger, sent] = marked(ledger);
    ledger = write(ledger, "session", sent, (held) => held?.filter((id) => id !== "b1")).ledger;
    expect(valueOn(ledger, "session")).toEqual(["b2"]);
    expect(write(EMPTY_LEDGER as Ledger<string>, "other", 1, () => undefined).apply).toBe(false);
  });

  it("says nothing about a question it brought no answer to: its failure stays, and is still recorded", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let broken: number;
    let sent: number;
    [ledger, broken] = ask(ledger, "session");
    [ledger, sent] = marked(ledger);
    ledger = write(ledger, "session", sent, () => undefined).ledger;
    ledger = failed(ledger, broken, "session", boom).ledger;
    expect(failureOn(ledger, "session")).toEqual(boom);

    [ledger, sent] = marked(ledger);
    ledger = write(ledger, "session", sent, () => undefined).ledger;
    expect(failureOn(ledger, "session")).toEqual(boom);
  });

  it("is dropped when the room on record is newer than the one it answered with", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let tick: number;
    let sent: number;
    [ledger, tick] = ask(ledger, "11");
    ledger = answered(ledger, tick, "11", { version: 7, name: "colleague" }, byVersion).ledger;
    [ledger, sent] = marked(ledger);
    const outcome = write(ledger, "11", sent, () => ({ version: 6, name: "mine" }), byVersion);
    expect(outcome.apply).toBe(false);
    expect(valueOn(outcome.ledger, "11")?.name).toBe("colleague");
    expect(write(ledger, "11", sent, () => ({ version: 7, name: "mine" }), byVersion).apply).toBe(true);
  });

  it("ties with a room of its version by when it was sent: after a read asked before, before a read asked after", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let before: number;
    let sent: number;
    let after: number;
    [ledger, before] = ask(ledger, "11");
    [ledger, sent] = marked(ledger);
    [ledger, after] = ask(ledger, "11");
    ledger = answered(ledger, after, "11", { version: 5, name: "read after" }, byVersion).ledger;
    const mine = write(ledger, "11", sent, () => ({ version: 5, name: "mine" }), byVersion);
    expect(mine.apply).toBe(false);
    expect(valueOn(mine.ledger, "11")?.name).toBe("read after");

    let other: Ledger<Room> = EMPTY_LEDGER;
    [other, before] = ask(other, "11");
    [other, sent] = marked(other);
    other = write(other, "11", sent, () => ({ version: 5, name: "mine" }), byVersion).ledger;
    expect(answered(other, before, "11", { version: 5, name: "read before" }, byVersion).apply).toBe(false);
    expect(valueOn(other, "11")?.name).toBe("mine");
  });

  it("clears the failure of a read asked before it", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let read: number;
    let sent: number;
    [ledger, read] = ask(ledger, "11");
    ledger = failed(ledger, read, "11", boom).ledger;
    [ledger, sent] = marked(ledger);
    ledger = write(ledger, "11", sent, () => "written").ledger;
    expect(failureOn(ledger, "11")).toBeNull();
  });
});

describe("the questions kept", () => {
  function answeredAll(keys: string[]): Ledger<string> {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    for (const key of keys) {
      let number: number;
      [ledger, number] = ask(ledger, key);
      ledger = answered(ledger, number, key, key).ledger;
    }
    return ledger;
  }

  it("are the one on screen, every one on its way, and the few heard from last of the rest", () => {
    let ledger = answeredAll(["a", "b", "c", "d", "e"]);
    [ledger] = ask(ledger, "a");
    let broken: number;
    [ledger, broken] = ask(ledger, "f");
    ledger = failed(ledger, broken, "f", boom).ledger;
    const kept = pruned(ledger, "b", 2);
    expect(Object.keys(kept.entries).sort()).toEqual(["a", "b", "e", "f"]);
    expect(valueOn(kept, "e")).toBe("e");
    expect(failureOn(kept, "f")).toEqual(boom);
  });

  it("leave the ledger as it is when nothing is past the few kept", () => {
    const ledger = answeredAll(["a", "b", "c"]);
    expect(pruned(ledger, null, 3)).toBe(ledger);
  });
});

describe("a failure", () => {
  it("is shown for the question it answered once nothing is on its way for that question", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "2|2026-09-11");
    expect(pendingOn(ledger, "2|2026-09-11")).toBe(true);
    const outcome = failed(ledger, read, "2|2026-09-11", boom);
    expect(outcome.recorded).toBe(true);
    expect(pendingOn(outcome.ledger, "2|2026-09-11")).toBe(false);
    expect(failureOn(outcome.ledger, "2|2026-09-11")).toEqual(boom);
  });

  it("is never shown for another question", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "party 2");
    ledger = failed(ledger, read, "party 2", boom).ledger;
    expect(failureOn(ledger, "party 4")).toBeNull();
    expect(failureOn(ledger, null)).toBeNull();
  });

  it("is not shown while a read of the same question is on its way, and the newest one is kept", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let first: number;
    let tick: number;
    [ledger, first] = ask(ledger, "session");
    [ledger, tick] = ask(ledger, "session");
    ledger = failed(ledger, tick, "session", boom).ledger;
    expect(failureOn(ledger, "session")).toBeNull();
    ledger = failed(ledger, first, "session", offline).ledger;
    expect(failureOn(ledger, "session")).toEqual(boom);
  });

  it("is not recorded when it is older than the answer on record, even for another question's sake", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let old: number;
    let fresh: number;
    let elsewhere: number;
    [ledger, old] = ask(ledger, "k");
    [ledger, fresh] = ask(ledger, "k");
    [ledger, elsewhere] = ask(ledger, "j");
    ledger = answered(ledger, fresh, "k", "fresh").ledger;
    expect(failed(ledger, old, "k", boom).recorded).toBe(false);
    expect(failed(ledger, elsewhere, "j", boom).recorded).toBe(true);
  });

  it("is not recorded when a read asked after it already answered, even with an answer not applied", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let first: number;
    let second: number;
    let third: number;
    [ledger, first] = ask(ledger, "11");
    [ledger, second] = ask(ledger, "11");
    [ledger, third] = ask(ledger, "11");
    ledger = answered(ledger, first, "11", { version: 6, name: "first" }, byVersion).ledger;
    const lagging = answered(ledger, third, "11", { version: 5, name: "third" }, byVersion);
    expect(lagging.apply).toBe(false);
    const outcome = failed(lagging.ledger, second, "11", boom);
    expect(outcome.recorded).toBe(false);
    expect(failureOn(outcome.ledger, "11")).toBeNull();
  });

  it("is not recorded when a write sent after it already answered, even with a room not applied", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let first: number;
    let broken: number;
    let sent: number;
    [ledger, first] = ask(ledger, "11");
    ledger = answered(ledger, first, "11", { version: 5, name: "colleague" }, byVersion).ledger;
    [ledger, broken] = ask(ledger, "11");
    [ledger, sent] = marked(ledger);
    const mine = write(ledger, "11", sent, () => ({ version: 4, name: "mine" }), byVersion);
    expect(mine.apply).toBe(false);
    const outcome = failed(mine.ledger, broken, "11", boom);
    expect(outcome.recorded).toBe(false);
    expect(failureOn(outcome.ledger, "11")).toBeNull();
  });

  it("is cleared by a newer answer", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let broken: number;
    let fine: number;
    [ledger, broken] = ask(ledger, "k");
    ledger = failed(ledger, broken, "k", boom).ledger;
    [ledger, fine] = ask(ledger, "k");
    ledger = answered(ledger, fine, "k", "fine").ledger;
    expect(failureOn(ledger, "k")).toBeNull();
  });

  it("is cleared by a read asked after it that answered an older room than the one on record", () => {
    // Answer not news, but proves question readable.
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let first: number;
    let sent: number;
    let broken: number;
    let later: number;
    [ledger, first] = ask(ledger, "11");
    ledger = answered(ledger, first, "11", { version: 1, name: "first" }, byVersion).ledger;
    [ledger, sent] = marked(ledger);
    [ledger, broken] = ask(ledger, "11");
    ledger = failed(ledger, broken, "11", boom).ledger;
    ledger = write(ledger, "11", sent, () => ({ version: 3, name: "mine" }), byVersion).ledger;
    expect(failureOn(ledger, "11")).toEqual(boom);

    [ledger, later] = ask(ledger, "11");
    const late = answered(ledger, later, "11", { version: 2, name: "lagging" }, byVersion);
    expect(late.apply).toBe(false);
    expect(valueOn(late.ledger, "11")?.name).toBe("mine");
    expect(failureOn(late.ledger, "11")).toBeNull();
  });

  it("is cleared by a write sent after it, even when the write's room is older than the one on record", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let first: number;
    let broken: number;
    let sent: number;
    [ledger, first] = ask(ledger, "11");
    ledger = answered(ledger, first, "11", { version: 5, name: "colleague" }, byVersion).ledger;
    [ledger, broken] = ask(ledger, "11");
    ledger = failed(ledger, broken, "11", boom).ledger;
    [ledger, sent] = marked(ledger);
    const outcome = write(ledger, "11", sent, () => ({ version: 4, name: "mine" }), byVersion);
    expect(outcome.apply).toBe(false);
    expect(failureOn(outcome.ledger, "11")).toBeNull();
  });

  it("stays when the write that answered was sent before it was asked", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let sent: number;
    let broken: number;
    [ledger, sent] = marked(ledger);
    [ledger, broken] = ask(ledger, "11");
    ledger = failed(ledger, broken, "11", boom).ledger;
    ledger = write(ledger, "11", sent, () => "written").ledger;
    expect(failureOn(ledger, "11")).toEqual(boom);
  });
});

describe("a moment marked from outside every read", () => {
  it("comes after every read asked so far and before every read asked from then on", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let before: number;
    let mark: number;
    let after: number;
    [ledger, before] = ask(ledger, "session");
    [ledger, mark] = marked(ledger);
    [ledger, after] = ask(ledger, "session");
    expect(before).toBeLessThan(mark);
    expect(after).toBeGreaterThan(mark);
    expect(pendingOn(ledger, "session")).toBe(true);
  });
});
