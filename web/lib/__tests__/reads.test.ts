/**
 * Which answers reach the screen, and which failures it admits to.
 */

import { describe, expect, it } from "vitest";

import {
  EMPTY_LEDGER,
  answered,
  answeredUpTo,
  begun,
  failed,
  failureOn,
  marked,
  pendingOn,
  valueOn,
  written,
  type Ledger,
  type Newer,
} from "../reads";

const boom = { code: "internal", message: "boom" };
const offline = { code: "network", message: "offline" };

type Room = { version: number; name: string };
const byVersion: Newer<Room> = (next, shown) => next.version >= shown.version;

function ask<T>(ledger: Ledger<T>, key: string): [Ledger<T>, number] {
  return begun(ledger, key);
}

describe("an answer", () => {
  it("is applied when its question has no answer yet, whatever another question has", () => {
    // One applied number for every question dropped the first answer for tonight whenever tomorrow
    // had answered in between.
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

  it("is applied, and clears the failure, when a later ask of the same question failed first", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let first: number;
    let retry: number;
    [ledger, first] = ask(ledger, "11|2");
    [ledger, retry] = ask(ledger, "11|2");
    ledger = failed(ledger, retry, "11|2", boom).ledger;
    const outcome = answered(ledger, first, "11|2", "times");
    expect(outcome.apply).toBe(true);
    expect(valueOn(outcome.ledger, "11|2")).toBe("times");
    expect(failureOn(outcome.ledger, "11|2")).toBeNull();
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

  it("counts as answered even when it is not applied", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "session");
    ledger = written(ledger, "session", () => "written").ledger;
    ledger = answered(ledger, read, "session", "read").ledger;
    expect(valueOn(ledger, "session")).toBe("written");
    expect(answeredUpTo(ledger, "session")).toBe(read);
  });
});

describe("a write that puts its own answer on screen", () => {
  it("outranks every read asked before it", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let tick: number;
    [ledger, tick] = ask(ledger, "11");
    const write = written(ledger, "11", () => "written");
    expect(write.apply).toBe(true);
    ledger = write.ledger;
    expect(answered(ledger, tick, "11", "tick").apply).toBe(false);
    expect(failed(ledger, tick, "11", boom).recorded).toBe(false);
  });

  it("is made on the value on record for its own question", () => {
    let ledger: Ledger<string[]> = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "session");
    ledger = answered(ledger, read, "session", ["b1", "b2"]).ledger;
    ledger = written(ledger, "session", (held) => held?.filter((id) => id !== "b1")).ledger;
    expect(valueOn(ledger, "session")).toEqual(["b2"]);
    expect(written(EMPTY_LEDGER as Ledger<string>, "other", () => undefined).apply).toBe(false);
  });

  it("is dropped when the room on record is newer than the one it answered with", () => {
    let ledger: Ledger<Room> = EMPTY_LEDGER;
    let tick: number;
    [ledger, tick] = ask(ledger, "11");
    ledger = answered(ledger, tick, "11", { version: 7, name: "colleague" }, byVersion).ledger;
    const write = written(ledger, "11", () => ({ version: 6, name: "mine" }), byVersion);
    expect(write.apply).toBe(false);
    expect(valueOn(write.ledger, "11")?.name).toBe("colleague");
    expect(written(ledger, "11", () => ({ version: 7, name: "mine" }), byVersion).apply).toBe(true);
  });

  it("clears the failure of a read asked before it", () => {
    let ledger: Ledger<string> = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "11");
    ledger = failed(ledger, read, "11", boom).ledger;
    ledger = written(ledger, "11", () => "written").ledger;
    expect(failureOn(ledger, "11")).toBeNull();
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
    ledger = answered(ledger, before, "session", "before").ledger;
    expect(answeredUpTo(ledger, "session")).toBeLessThan(mark);
    ledger = answered(ledger, after, "session", "after").ledger;
    expect(answeredUpTo(ledger, "session")).toBeGreaterThan(mark);
  });
});
