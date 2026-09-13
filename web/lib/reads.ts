/**
 * Which answers reach screen, which failures show. One rule for every read.
 *
 * Each read names its question (evening, party and date, session) and gets number. Each question
 * keeps own last value and last failure, so answer about tomorrow never decides news about tonight.
 *
 * Answer replaces value on record when server orders it newer (room carries version), or, when
 * server cannot tell or orders nothing, when asked later. Write's own answer numbered at send.
 * Failure recorded only when asked after every answer heard; shown only while nothing else in flight
 * for that question, beside value on record if any. Any later-asked answer, read or write, applied
 * or not, clears failure or blocks recording it: too-old answer still proves question answerable.
 */

import type { ApiFailure } from "./errors";

/** Above zero: `next` newer. Below zero: older. Zero: server cannot tell. */
export type Order<T> = (next: T, shown: T) => number;

export interface Entry<T> {
  /** `number`: read or write that brought it. */
  readonly value?: { readonly number: number; readonly data: T };
  /** Newest number among answers heard, read or write, applied or not; failure asked before is old news. */
  readonly heard: number;
  readonly failure?: { readonly number: number; readonly failure: ApiFailure };
}

export interface Ledger<T> {
  /** Number given to newest read, write or mark. */
  readonly asked: number;
  readonly inFlight: readonly { readonly number: number; readonly key: string }[];
  readonly entries: Readonly<Record<string, Entry<T>>>;
}

export const EMPTY_LEDGER: Ledger<never> = { asked: 0, inFlight: [], entries: {} };

const NOTHING: Entry<never> = { heard: 0 };

function entryOf<T>(ledger: Ledger<T>, key: string): Entry<T> {
  return ledger.entries[key] ?? NOTHING;
}

function withEntry<T>(ledger: Ledger<T>, key: string, entry: Entry<T>): Ledger<T> {
  return { ...ledger, entries: { ...ledger.entries, [key]: entry } };
}

function landed<T>(ledger: Ledger<T>, number: number): Ledger<T> {
  return { ...ledger, inFlight: ledger.inFlight.filter((read) => read.number !== number) };
}

function replaces<T>(entry: Entry<T>, number: number, data: T, order?: Order<T>): boolean {
  const shown = entry.value;
  if (shown === undefined) return true;
  const said = order ? order(data, shown.data) : 0;
  return said > 0 || (said === 0 && number > shown.number);
}

/** Failure of read asked after `number` stays. */
function heardAt<T>(entry: Entry<T>, number: number): Entry<T> {
  const stale = entry.failure !== undefined && entry.failure.number <= number;
  if (!stale && number <= entry.heard) return entry;
  const next = { ...entry, heard: Math.max(entry.heard, number) };
  if (!stale) return next;
  const { failure: _cleared, ...rest } = next;
  return rest;
}

function applied<T>(entry: Entry<T>, number: number, data: T): Entry<T> {
  return { ...heardAt(entry, number), value: { number, data } };
}

/** Starts read of `key`; returns ledger and read number. */
export function begun<T>(ledger: Ledger<T>, key: string): [Ledger<T>, number] {
  const number = ledger.asked + 1;
  return [{ ...ledger, asked: number, inFlight: [...ledger.inFlight, { number, key }] }, number];
}

/** Number after every read asked so far, before every read asked later. */
export function marked<T>(ledger: Ledger<T>): [Ledger<T>, number] {
  const number = ledger.asked + 1;
  return [{ ...ledger, asked: number }, number];
}

/** Applied only when newer than value on record. */
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
  const kept = apply ? applied(entry, number, data) : heardAt(entry, number);
  return { ledger: withEntry(next, key, kept), apply };
}

/** Recorded only when asked after every answer heard. */
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
 * Write's own answer about `key`, built on value on record, numbered `sent` (mark taken at send).
 * `change` returning undefined brought no answer; `key` left untouched.
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

export function pendingOn<T>(ledger: Ledger<T>, key: string | null): boolean {
  return ledger.inFlight.some((read) => read.key === key);
}

export function valueOn<T>(ledger: Ledger<T>, key: string | null): T | null {
  if (key === null) return null;
  return entryOf(ledger, key).value?.data ?? null;
}

/** Latest failure for `key`, only once nothing else in flight. */
export function failureOn<T>(ledger: Ledger<T>, key: string | null): ApiFailure | null {
  if (key === null || pendingOn(ledger, key)) return null;
  return entryOf(ledger, key).failure?.failure ?? null;
}

function lastHeard<T>(entry: Entry<T>): number {
  return Math.max(entry.heard, entry.failure?.number ?? 0);
}

/** Drops every question except one on screen, ones in flight, and `keep` most recently heard of rest. */
export function pruned<T>(ledger: Ledger<T>, onScreen: string | null, keep: number): Ledger<T> {
  const idle = Object.entries(ledger.entries).filter(
    ([key]) => key !== onScreen && !pendingOn(ledger, key),
  );
  if (idle.length <= keep) return ledger;
  const dropped = new Set(
    idle
      .sort(([, left], [, right]) => lastHeard(right) - lastHeard(left))
      .slice(keep)
      .map(([key]) => key),
  );
  return {
    ...ledger,
    entries: Object.fromEntries(Object.entries(ledger.entries).filter(([key]) => !dropped.has(key))),
  };
}
