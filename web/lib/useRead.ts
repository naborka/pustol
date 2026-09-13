/**
 * The one way the app reads anything: the session, a shift, the settings, a day rail, a time grid.
 *
 * The rule itself lives in `reads.ts`; this is where it meets React. The question asked is always
 * the question on screen when the read starts, even when the call comes from a retry an older
 * render set up, and an answer is weighed against the question on screen when it lands.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { ApiError } from "./api";
import type { ApiFailure } from "./errors";
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
} from "./reads";

/** A read the screen is asking: what it is about, and how to ask it. */
export interface Question<T> {
  key: string;
  ask: () => Promise<T>;
}

/** A failure as the API described it, or as close as the app can get. */
export function failureOf(error: unknown): ApiFailure {
  return error instanceof ApiError ? error.failure : { code: "internal", message: String(error) };
}

export function useRead<T>(
  question: Question<T> | null,
  onAnswer: (answer: T, key: string) => void,
  onFailure: (failure: ApiFailure, tell: boolean) => void,
) {
  const [ledger, setLedger] = useState<Ledger>(EMPTY_LEDGER);
  const ledgerNow = useRef<Ledger>(EMPTY_LEDGER);
  const latest = useRef({ question, onAnswer, onFailure });
  useEffect(() => {
    latest.current = { question, onAnswer, onFailure };
  });

  const actions = useMemo(() => {
    const keyNow = () => latest.current.question?.key ?? null;
    const commit = (next: Ledger) => {
      if (next === ledgerNow.current) return;
      ledgerNow.current = next;
      setLedger(next);
    };
    return {
      /** Asks the question on screen now. `quiet` is a refresh nobody asked for. */
      load: async (quiet = false) => {
        const asking = latest.current.question;
        if (!asking) return;
        const [started, number] = begun(ledgerNow.current, asking.key);
        commit(started);
        let answer: T;
        try {
          answer = await asking.ask();
        } catch (error) {
          const failure = failureOf(error);
          const tell = shouldTell(ledgerNow.current, number, asking.key, keyNow(), quiet);
          commit(failed(ledgerNow.current, number, asking.key, failure));
          latest.current.onFailure(failure, tell);
          return;
        }
        const outcome = answered(ledgerNow.current, number, asking.key, keyNow());
        commit(outcome.ledger);
        if (outcome.apply) latest.current.onAnswer(answer, asking.key);
      },
      /**
       * Puts a write's own answer about `key` on screen, if the screen still asks about `key`: `show`
       * runs only then. Every read asked before it is older from here on.
       */
      put: (key: string, show: () => void): boolean => {
        const outcome = written(ledgerNow.current, key, keyNow());
        commit(outcome.ledger);
        if (outcome.apply) show();
        return outcome.apply;
      },
      /** Records that `key` is broken as of now, whatever read is on its way; returns when. */
      failNow: (key: string, failure: ApiFailure): number => {
        const [next, number] = failedNow(ledgerNow.current, key, failure);
        commit(next);
        return number;
      },
    };
  }, []);

  const key = question?.key ?? null;
  return {
    ...actions,
    /** The failure to show for the question on screen, or null. */
    failure: failureOn(ledger, key),
    /** A read of the question on screen is on its way. */
    pending: pendingOn(ledger, key),
    /** The number of the answer on screen, for comparing with a failure recorded by `failNow`. */
    applied: ledger.applied,
  };
}
