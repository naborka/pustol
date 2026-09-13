/**
 * Which answers reach the screen, and which failures it admits to — one rule for every read.
 *
 * Every read names its question — the evening, the party and the date, the session — and gets a
 * number. Each question keeps its own last value and its own last failure, so an answer about
 * tomorrow never decides whether an answer about tonight is news: one number shared by every
 * question dropped a good answer whenever another question had answered in between.
 *
 * An answer replaces the value on record when it was asked later, or, for a question the server
 * orders itself (a room carries a version), when the server says it is at least as new. A failure
 * is recorded only when it was asked after the value on record, is cleared by any answer applied
 * after it, and is shown only while nothing else is on its way for that question.
 */

import type { ApiFailure } from "./errors";

/** Whether `next` may replace `shown`. Without one, the later ask wins. */
export type Newer<T> = (next: T, shown: T) => boolean;

export interface Entry<T> {
  /** The value on record, and the number of the read or write that brought it. */
  readonly value?: { readonly number: number; readonly data: T };
  readonly failure?: { readonly number: number; readonly failure: ApiFailure };
  /** The newest read of this question that answered, applied or not; 0 before any. */
  readonly answered: number;
}

export interface Ledger<T> {
  /** The number the newest read, write or mark was given. */
  readonly asked: number;
  readonly inFlight: readonly { readonly number: number; readonly key: string }[];
  readonly entries: Readonly<Record<string, Entry<T>>>;
}

export const EMPTY_LEDGER: Ledger<never> = { asked: 0, inFlight: [], entries: {} };

const NOTHING: Entry<never> = { answered: 0 };

function entryOf<T>(ledger: Ledger<T>, key: string): Entry<T> {
  return ledger.entries[key] ?? NOTHING;
}

function withEntry<T>(ledger: Ledger<T>, key: string, entry: Entry<T>): Ledger<T> {
  return { ...ledger, entries: { ...ledger.entries, [key]: entry } };
}

function landed<T>(ledger: Ledger<T>, number: number): Ledger<T> {
  return { ...ledger, inFlight: ledger.inFlight.filter((read) => read.number !== number) };
}

/** A read of `key` starting, and the number it goes by. */
export function begun<T>(ledger: Ledger<T>, key: string): [Ledger<T>, number] {
  const number = ledger.asked + 1;
  return [{ ...ledger, asked: number, inFlight: [...ledger.inFlight, { number, key }] }, number];
}

/** A number later than every read asked so far and earlier than every read asked after. */
export function marked<T>(ledger: Ledger<T>): [Ledger<T>, number] {
  const number = ledger.asked + 1;
  return [{ ...ledger, asked: number }, number];
}

/** A read of `key` answered with `data`: applied when it is newer than the value on record. */
export function answered<T>(
  ledger: Ledger<T>,
  number: number,
  key: string,
  data: T,
  newer?: Newer<T>,
): { ledger: Ledger<T>; apply: boolean } {
  const next = landed(ledger, number);
  const entry = entryOf(next, key);
  const shown = entry.value;
  const apply =
    shown === undefined || (newer ? newer(data, shown.data) : number > shown.number);
  const answeredUpTo = Math.max(entry.answered, number);
  if (!apply) return { ledger: withEntry(next, key, { ...entry, answered: answeredUpTo }), apply };
  return {
    ledger: withEntry(next, key, {
      value: { number: Math.max(number, shown?.number ?? 0), data },
      answered: answeredUpTo,
    }),
    apply,
  };
}

/** A read of `key` failed: recorded only when it was asked after the value on record. */
export function failed<T>(
  ledger: Ledger<T>,
  number: number,
  key: string,
  failure: ApiFailure,
): { ledger: Ledger<T>; recorded: boolean } {
  const next = landed(ledger, number);
  const entry = entryOf(next, key);
  const recorded = number > (entry.value?.number ?? 0);
  if (!recorded || (entry.failure && entry.failure.number > number)) {
    return { ledger: next, recorded };
  }
  return { ledger: withEntry(next, key, { ...entry, failure: { number, failure } }), recorded };
}

/**
 * A write's own answer about `key`, made on the value on record: newer than every read asked before
 * it, unless the question is ordered by the server and the server says otherwise.
 */
export function written<T>(
  ledger: Ledger<T>,
  key: string,
  change: (current: T | undefined) => T | undefined,
  newer?: Newer<T>,
): { ledger: Ledger<T>; apply: boolean; data: T | undefined } {
  const entry = entryOf(ledger, key);
  const data = change(entry.value?.data);
  const apply =
    data !== undefined && (entry.value === undefined || !newer || newer(data, entry.value.data));
  if (!apply) return { ledger, apply, data };
  const [next, number] = marked(ledger);
  return {
    ledger: withEntry(next, key, { value: { number, data }, answered: entry.answered }),
    apply,
    data,
  };
}

/** Whether a read of `key` is on its way. */
export function pendingOn<T>(ledger: Ledger<T>, key: string | null): boolean {
  return ledger.inFlight.some((read) => read.key === key);
}

/** The value on record for `key`, or null. */
export function valueOn<T>(ledger: Ledger<T>, key: string | null): T | null {
  if (key === null) return null;
  return entryOf(ledger, key).value?.data ?? null;
}

/** The failure to show for `key`: the latest word on it, once nothing else is on its way. */
export function failureOn<T>(ledger: Ledger<T>, key: string | null): ApiFailure | null {
  if (key === null || pendingOn(ledger, key)) return null;
  return entryOf(ledger, key).failure?.failure ?? null;
}

/** The number of the newest read of `key` that answered; 0 before any. */
export function answeredUpTo<T>(ledger: Ledger<T>, key: string | null): number {
  return key === null ? 0 : entryOf(ledger, key).answered;
}
