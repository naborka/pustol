/**
 * Which answers reach the screen, and which failures it admits to — one rule for every read.
 *
 * Every read and every write that puts its own answer on screen gets a number, and every read names
 * its question: the evening, the party and the date, the session. An answer is applied when the
 * screen still asks its question and it is newer than the answer already there — not only when it
 * is the newest asked, which dropped a good answer whenever a background tick overtook it and then
 * failed. A failure is a fact about one question, derived when drawn rather than a flag something
 * must remember to clear.
 */

import type { ApiFailure } from "./errors";

export interface Ledger {
  /** The number the newest read or write was given. */
  readonly asked: number;
  /** The number of the answer on screen; 0 before the first. */
  readonly applied: number;
  readonly inFlight: readonly { readonly number: number; readonly key: string }[];
  /** The newest failure of each question. */
  readonly failures: Readonly<Record<string, { readonly number: number; readonly failure: ApiFailure }>>;
}

export const EMPTY_LEDGER: Ledger = { asked: 0, applied: 0, inFlight: [], failures: {} };

/** A read of `key` starting, and the number it goes by. */
export function begun(ledger: Ledger, key: string): [Ledger, number] {
  const number = ledger.asked + 1;
  return [{ ...ledger, asked: number, inFlight: [...ledger.inFlight, { number, key }] }, number];
}

function landed(ledger: Ledger, number: number): Ledger {
  return { ...ledger, inFlight: ledger.inFlight.filter((read) => read.number !== number) };
}

/** A read answered: applied when the screen still asks `key` and nothing newer is on it. */
export function answered(
  ledger: Ledger,
  number: number,
  key: string,
  keyNow: string | null,
): { ledger: Ledger; apply: boolean } {
  const next = landed(ledger, number);
  const apply = key === keyNow && number > ledger.applied;
  return { ledger: apply ? { ...next, applied: number } : next, apply };
}

function recorded(ledger: Ledger, number: number, key: string, failure: ApiFailure): Ledger {
  const known = ledger.failures[key];
  if (known && known.number > number) return ledger;
  return { ...ledger, failures: { ...ledger.failures, [key]: { number, failure } } };
}

/** A read of `key` failed. */
export function failed(ledger: Ledger, number: number, key: string, failure: ApiFailure): Ledger {
  return recorded(landed(ledger, number), number, key, failure);
}

/** A failure that did not come from a read of `key` but says `key` is broken, as of now. */
export function failedNow(ledger: Ledger, key: string, failure: ApiFailure): [Ledger, number] {
  const number = ledger.asked + 1;
  return [recorded({ ...ledger, asked: number }, number, key, failure), number];
}

/**
 * A write's own answer: on screen at once when the screen asks `key`, newer than every read asked
 * before it — each of those was asked before the write happened.
 */
export function written(
  ledger: Ledger,
  key: string,
  keyNow: string | null,
): { ledger: Ledger; apply: boolean } {
  if (key !== keyNow) return { ledger, apply: false };
  const number = ledger.asked + 1;
  return { ledger: { ...ledger, asked: number, applied: number }, apply: true };
}

/** Whether a read of `key` is on its way. */
export function pendingOn(ledger: Ledger, key: string | null): boolean {
  return ledger.inFlight.some((read) => read.key === key);
}

/** The failure to show for the question on screen, if it is still the latest word on it. */
export function failureOn(ledger: Ledger, key: string | null): ApiFailure | null {
  if (key === null) return null;
  const known = ledger.failures[key];
  if (!known || known.number <= ledger.applied || pendingOn(ledger, key)) return null;
  return known.failure;
}

/**
 * Whether a failed read is worth a word. A refresh nobody asked for never is: a toast every thirty
 * seconds because the bar's Wi-Fi dropped buries the one that matters.
 */
export function shouldTell(
  ledger: Ledger,
  number: number,
  key: string,
  keyNow: string | null,
  quiet: boolean,
): boolean {
  return !quiet && key === keyNow && number > ledger.applied;
}
