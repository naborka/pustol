/**
 * Only way app reads: session, shift, settings, day rail, time grid. Rule lives in `reads.ts`; this
 * binds it to React.
 *
 * Question asked is always one on screen when read starts, even from retry set up by older render.
 * Each question keeps own answer, so drawn answer is for question on screen now, never last arrival.
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

export interface Question<T> {
  key: string;
  ask: () => Promise<T>;
}

/** Questions neither on screen nor in flight that keep their record. */
const KEPT_ASIDE = 3;

export function useRead<T>(
  question: Question<T> | null,
  /** Read answer put on record. Write answer is its writer's to act on. */
  onAnswer: (answer: T) => void,
  order?: Order<T>,
) {
  const [ledger, setLedger] = useState<Ledger<T>>(EMPTY_LEDGER);
  const ledgerNow = useRef<Ledger<T>>(EMPTY_LEDGER);
  // Last answer applied while its question on screen; drawn while new question loads.
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
      /** Asks question on screen now, not at render. */
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
      /** Records write answer about `key`, built on value there now, numbered `sent` (mark at send); default now. */
      put: (
        key: string,
        change: (current: T | undefined) => T | undefined,
        sent: number = mark(),
      ): void => {
        commit(written(ledgerNow.current, key, sent, change, latest.current.order).ledger);
      },
      /** Every read asked after this comes later. */
      mark,
    };
  }, []);

  const key = question?.key ?? null;
  const value = valueOn(ledger, key);
  const failure = failureOn(ledger, key);
  return {
    ...actions,
    value,
    /** `value`, or while none, previous question's answer. Never after this question failed. */
    drawn: value ?? (failure ? null : lastOnScreen),
    /** Newest read failure of question on screen, only once nothing else in flight. */
    failure,
    pending: pendingOn(ledger, key),
  };
}
