/**
 * Which answers reach the screen, and which failures it admits to.
 */

import { describe, expect, it } from "vitest";

import {
  EMPTY_LEDGER,
  answered,
  begun,
  failed,
  failedNow,
  failureOn,
  pendingOn,
  shouldTell,
  written,
  type Ledger,
} from "../reads";

const boom = { code: "internal", message: "boom" };
const offline = { code: "network", message: "offline" };

function ask(ledger: Ledger, key: string): [Ledger, number] {
  return begun(ledger, key);
}

describe("an answer", () => {
  it("is applied when it is newer than the answer on screen, not only when it is the newest asked", () => {
    let ledger = EMPTY_LEDGER;
    let first: number;
    let tick: number;
    [ledger, first] = ask(ledger, "session");
    [ledger, tick] = ask(ledger, "session");
    ledger = failed(ledger, tick, "session", boom);
    const outcome = answered(ledger, first, "session", "session");
    expect(outcome.apply).toBe(true);
    expect(outcome.ledger.applied).toBe(first);
  });

  it("is dropped when a newer answer is already on screen", () => {
    let ledger = EMPTY_LEDGER;
    let old: number;
    let fresh: number;
    [ledger, old] = ask(ledger, "2026-09-11");
    [ledger, fresh] = ask(ledger, "2026-09-11");
    ledger = answered(ledger, fresh, "2026-09-11", "2026-09-11").ledger;
    const late = answered(ledger, old, "2026-09-11", "2026-09-11");
    expect(late.apply).toBe(false);
    expect(late.ledger.applied).toBe(fresh);
  });

  it("is dropped when the screen no longer asks that question", () => {
    let ledger = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "2026-09-11");
    expect(answered(ledger, read, "2026-09-11", "2026-09-12").apply).toBe(false);
  });
});

describe("a write that puts its own answer on screen", () => {
  it("outranks every read asked before it", () => {
    let ledger = EMPTY_LEDGER;
    let tick: number;
    [ledger, tick] = ask(ledger, "2026-09-11");
    const write = written(ledger, "2026-09-11", "2026-09-11");
    expect(write.apply).toBe(true);
    ledger = write.ledger;
    expect(answered(ledger, tick, "2026-09-11", "2026-09-11").apply).toBe(false);
    ledger = failed(ledger, tick, "2026-09-11", boom);
    expect(failureOn(ledger, "2026-09-11")).toBeNull();
  });

  it("does not reach a screen showing another question", () => {
    const write = written(EMPTY_LEDGER, "2026-09-11", "2026-09-12");
    expect(write.apply).toBe(false);
    expect(write.ledger).toBe(EMPTY_LEDGER);
  });
});

describe("a failure", () => {
  it("is shown for the question it answered, once nothing newer is on its way or on screen", () => {
    let ledger = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "2|2026-09-11");
    expect(pendingOn(ledger, "2|2026-09-11")).toBe(true);
    ledger = failed(ledger, read, "2|2026-09-11", boom);
    expect(pendingOn(ledger, "2|2026-09-11")).toBe(false);
    expect(failureOn(ledger, "2|2026-09-11")).toEqual(boom);
  });

  it("is never shown for another question, even before that question is asked", () => {
    let ledger = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "party 2");
    ledger = failed(ledger, read, "party 2", boom);
    expect(failureOn(ledger, "party 4")).toBeNull();
    expect(failureOn(ledger, null)).toBeNull();
  });

  it("is not shown while a read of the same question is still on its way", () => {
    let ledger = EMPTY_LEDGER;
    let first: number;
    let tick: number;
    [ledger, first] = ask(ledger, "session");
    [ledger, tick] = ask(ledger, "session");
    ledger = failed(ledger, tick, "session", boom);
    expect(failureOn(ledger, "session")).toBeNull();
    ledger = failed(ledger, first, "session", offline);
    expect(failureOn(ledger, "session")).toEqual(boom);
  });

  it("is not shown once a newer answer has landed", () => {
    let ledger = EMPTY_LEDGER;
    let broken: number;
    let fine: number;
    [ledger, broken] = ask(ledger, "k");
    ledger = failed(ledger, broken, "k", boom);
    [ledger, fine] = ask(ledger, "k");
    ledger = answered(ledger, fine, "k", "k").ledger;
    expect(failureOn(ledger, "k")).toBeNull();
  });

  it("is told to whoever asked, unless they did not ask or a newer answer landed", () => {
    let ledger = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "k");
    expect(shouldTell(ledger, read, "k", "k", false)).toBe(true);
    expect(shouldTell(ledger, read, "k", "k", true)).toBe(false);
    expect(shouldTell(ledger, read, "k", "other", false)).toBe(false);
    ledger = written(ledger, "k", "k").ledger;
    expect(shouldTell(ledger, read, "k", "k", false)).toBe(false);
  });

  it("recorded from outside a read is newer than every read asked so far", () => {
    let ledger = EMPTY_LEDGER;
    let read: number;
    [ledger, read] = ask(ledger, "session");
    ledger = answered(ledger, read, "session", "session").ledger;
    const [after, stamp] = failedNow(ledger, "session", offline);
    expect(stamp).toBeGreaterThan(after.applied);
    expect(failureOn(after, "session")).toEqual(offline);
  });
});
