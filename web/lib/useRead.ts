/**
 * The one way the app reads anything: the session, a shift, the settings, a day rail, a time grid.
 *
 * The rule itself lives in `reads.ts`; this is where it meets React. The question asked is always
 * the question on screen when the read starts, even when the call comes from a retry an older
 * render set up. Every question keeps its own answer, so what is drawn is the answer to the
 * question on screen now, never the last answer that happened to arrive.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { failureOf } from "./errors";
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
} from "./reads";

/** A read the screen is asking: what it is about, and how to ask it. */
export interface Question<T> {
  key: string;
  ask: () => Promise<T>;
}

/** How many questions neither on screen nor on their way keep what was heard of them. */
const KEPT_ASIDE = 3;

export function useRead<T>(
  question: Question<T> | null,
  /** A read's answer was put on record. A write's own answer is its writer's to act on. */
  onAnswer: (answer: T) => void,
  /** How the server orders answers to this question, when it does. */
  order?: Order<T>,
) {
  const [ledger, setLedger] = useState<Ledger<T>>(EMPTY_LEDGER);
  const ledgerNow = useRef<Ledger<T>>(EMPTY_LEDGER);
  // The answer last applied while its question was on screen, drawn while a new question loads.
  const [lastOnScreen, setLastOnScreen] = useState<T | null>(null);
  const latest = useRef({ question, onAnswer, order });
  useEffect(() => {
    latest.current = { question, onAnswer, order };
  });

  const actions = useMemo(() => {
    const keyNow = () => latest.current.question?.key ?? null;
    const commit = (next: Ledger<T>) => {
      const kept = pruned(next, keyNow(), KEPT_ASIDE);
      if (kept === ledgerNow.current) return;
      ledgerNow.current = kept;
      setLedger(kept);
    };
    const mark = (): number => {
      const [next, number] = marked(ledgerNow.current);
      commit(next);
      return number;
    };
    return {
      /** Asks the question on screen now. */
      load: async () => {
        const asking = latest.current.question;
        if (!asking) return;
        const [started, number] = begun(ledgerNow.current, asking.key);
        commit(started);
        let answer: T;
        try {
          answer = await asking.ask();
        } catch (error) {
          commit(failed(ledgerNow.current, number, asking.key, failureOf(error)).ledger);
          return;
        }
        const outcome = answered(
          ledgerNow.current,
          number,
          asking.key,
          answer,
          latest.current.order,
        );
        commit(outcome.ledger);
        if (!outcome.apply) return;
        if (asking.key === keyNow()) setLastOnScreen(answer);
        latest.current.onAnswer(answer);
      },
      /**
       * Puts a write's own answer about `key` on record, made on the value there now and numbered
       * `sent`, the mark taken when the write was sent; without one, now.
       */
      put: (
        key: string,
        change: (current: T | undefined) => T | undefined,
        sent: number = mark(),
      ): void => {
        commit(written(ledgerNow.current, key, sent, change, latest.current.order).ledger);
      },
      /** A moment every read asked from now on comes after. */
      mark,
    };
  }, []);

  const key = question?.key ?? null;
  const value = valueOn(ledger, key);
  const failure = failureOn(ledger, key);
  return {
    ...actions,
    /** The answer on record for the question on screen, or null. */
    value,
    /**
     * What to draw: that answer, or while there is none yet, the one shown for the question before —
     * never once this question has failed, since another question's answer does not stand in for it.
     */
    drawn: value ?? (failure ? null : lastOnScreen),
    /** The failure of the newest read of the question on screen, once nothing else is on its way. */
    failure,
    /** A read of the question on screen is on its way. */
    pending: pendingOn(ledger, key),
  };
}
