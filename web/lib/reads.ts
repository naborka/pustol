/**
 * Which answers reach the screen, and which failures it admits to — one rule for every read.
 *
 * Every read names its question — the evening, the party and the date, the session — and gets a
 * number. Each question keeps its own last value and its own last failure, so an answer about
 * tomorrow never decides whether an answer about tonight is news: one number shared by every
 * question dropped a good answer whenever another question had answered in between.
 *
 * An answer replaces the value on record when the server orders it after that value (a room carries
 * a version), or, when the server cannot tell the two apart or orders nothing, when it was asked
 * later. A write's own answer is numbered when the write was sent. A failure is recorded only when
 * it was asked after every answer heard, and is shown only while nothing else is on its way for
 * that question — beside the value on record, if there is one. Any answer asked after it clears it,
 * or keeps it from being recorded, a read's or a write's, applied or not: an answer too old to show
 * still proves the question can be answered.
 */

import type { ApiFailure } from "./errors";

/**
 * How the server orders two answers to one question: above zero when `next` is newer, below when it
 * is older, zero when the server cannot tell them apart.
 */
export type Order<T> = (next: T, shown: T) => number;

export interface Entry<T> {
  /** The value on record, and the number of the read or write that brought it. */
  readonly value?: { readonly number: number; readonly data: T };
  /**
   * The newest number among the answers heard, a read's or a write's, applied or not: a failure
   * asked before it is older news.
   */
  readonly heard: number;
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

const NOTHING: Entry<never> = { heard: 0, answered: 0 };

function entryOf<T>(ledger: Ledger<T>, key: string): Entry<T> {
  return ledger.entries[key] ?? NOTHING;
}

function withEntry<T>(ledger: Ledger<T>, key: string, entry: Entry<T>): Ledger<T> {
  return { ...ledger, entries: { ...ledger.entries, [key]: entry } };
}

function landed<T>(ledger: Ledger<T>, number: number): Ledger<T> {
  return { ...ledger, inFlight: ledger.inFlight.filter((read) => read.number !== number) };
}

/** Whether `data`, asked as `number`, replaces what `entry` has on record. */
function replaces<T>(entry: Entry<T>, number: number, data: T, order?: Order<T>): boolean {
  const shown = entry.value;
  if (shown === undefined) return true;
  const said = order ? order(data, shown.data) : 0;
  return said > 0 || (said === 0 && number > shown.number);
}

/** `entry` once an answer asked as `number` came back: a failure of a read asked after it stays. */
function heardAt<T>(entry: Entry<T>, number: number): Entry<T> {
  const stale = entry.failure !== undefined && entry.failure.number <= number;
  if (!stale && number <= entry.heard) return entry;
  const next = { ...entry, heard: Math.max(entry.heard, number) };
  if (!stale) return next;
  const { failure: _cleared, ...rest } = next;
  return rest;
}

/** `entry` with `data`, asked as `number`, on record. */
function applied<T>(entry: Entry<T>, number: number, data: T): Entry<T> {
  return { ...heardAt(entry, number), value: { number, data } };
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
  order?: Order<T>,
): { ledger: Ledger<T>; apply: boolean } {
  const next = landed(ledger, number);
  const entry = entryOf(next, key);
  const apply = replaces(entry, number, data, order);
  const answeredUpTo = Math.max(entry.answered, number);
  const kept = apply ? applied(entry, number, data) : heardAt(entry, number);
  return { ledger: withEntry(next, key, { ...kept, answered: answeredUpTo }), apply };
}

/** A read of `key` failed: recorded only when it was asked after every answer heard. */
export function failed<T>(
  ledger: Ledger<T>,
  number: number,
  key: string,
  failure: ApiFailure,
): { ledger: Ledger<T>; recorded: boolean } {
  const next = landed(ledger, number);
  const entry = entryOf(next, key);
  const recorded = number > entry.heard;
  if (!recorded || (entry.failure && entry.failure.number > number)) {
    return { ledger: next, recorded };
  }
  return { ledger: withEntry(next, key, { ...entry, failure: { number, failure } }), recorded };
}

/**
 * A write's own answer about `key`, made on the value on record and numbered `sent`, the mark taken
 * when the write was sent. A change with nothing to put there brought no answer, so `key` is left
 * as it was.
 */
export function written<T>(
  ledger: Ledger<T>,
  key: string,
  sent: number,
  change: (current: T | undefined) => T | undefined,
  order?: Order<T>,
): { ledger: Ledger<T>; apply: boolean; data: T | undefined } {
  const entry = entryOf(ledger, key);
  const data = change(entry.value?.data);
  if (data === undefined) return { ledger, apply: false, data };
  const apply = replaces(entry, sent, data, order);
  const kept = apply ? applied(entry, sent, data) : heardAt(entry, sent);
  return { ledger: kept === entry ? ledger : withEntry(ledger, key, kept), apply, data };
}

/** Whether a read of `key` is on its way. */
export function pendingOn<T>(ledger: Ledger<T>, key: string | null): boolean {
  return ledger.inFlight.some((read) => read.key === key);
}

/** The number of the newest read of `key` on its way; 0 when none is. */
export function pendingUpTo<T>(ledger: Ledger<T>, key: string | null): number {
  return ledger.inFlight.reduce(
    (newest, read) => (read.key === key ? Math.max(newest, read.number) : newest),
    0,
  );
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
