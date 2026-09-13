/**
 * The one way the app reads anything: the session, a shift, the settings, a day rail, a time grid.
 *
 * The rule itself lives in `reads.ts`; this is where it meets React. The question asked is always
 * the question on screen when the read starts, even when the call comes from a retry an older
 * render set up. Every question keeps its own answer, so what is drawn is the answer to the
 * question on screen now, never the last answer that happened to arrive.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { ApiError } from "./api";
import type { ApiFailure } from "./errors";
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
} from "./reads";

/** A read the screen is asking: what it is about, and how to ask it. */
export interface Question<T> {
  key: string;
  ask: () => Promise<T>;
}

/** Where an applied answer belongs. */
export interface Landing {
  key: string;
  /** Its question is the one on screen. */
  onScreen: boolean;
}

/** A failed read, as whoever asked should hear of it. */
export interface Failed {
  /** Worth a word: somebody asked, and nothing newer about the question is on record. */
  tell: boolean;
  /** When the read was asked, to compare with `mark`. */
  number: number;
}

/** A failure as the API described it, or as close as the app can get. */
export function failureOf(error: unknown): ApiFailure {
  return error instanceof ApiError ? error.failure : { code: "internal", message: String(error) };
}

export function useRead<T>(
  question: Question<T> | null,
  onAnswer: (answer: T, landing: Landing) => void,
  onFailure: (failure: ApiFailure, failed: Failed) => void,
  /** How the server orders answers to this question, when it does. */
  newer?: Newer<T>,
) {
  const [ledger, setLedger] = useState<Ledger<T>>(EMPTY_LEDGER);
  const ledgerNow = useRef<Ledger<T>>(EMPTY_LEDGER);
  // The answer last applied while its question was on screen, drawn while a new question loads.
  const [lastOnScreen, setLastOnScreen] = useState<T | null>(null);
  const latest = useRef({ question, onAnswer, onFailure, newer });
  useEffect(() => {
    latest.current = { question, onAnswer, onFailure, newer };
  });

  const actions = useMemo(() => {
    const keyNow = () => latest.current.question?.key ?? null;
    const commit = (next: Ledger<T>) => {
      if (next === ledgerNow.current) return;
      ledgerNow.current = next;
      setLedger(next);
    };
    const land = (key: string, data: T) => {
      const onScreen = key === keyNow();
      if (onScreen) setLastOnScreen(data);
      latest.current.onAnswer(data, { key, onScreen });
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
          const outcome = failed(ledgerNow.current, number, asking.key, failure);
          commit(outcome.ledger);
          const tell = !quiet && outcome.recorded && asking.key === keyNow();
          latest.current.onFailure(failure, { tell, number });
          return;
        }
        const outcome = answered(
          ledgerNow.current,
          number,
          asking.key,
          answer,
          latest.current.newer,
        );
        commit(outcome.ledger);
        if (outcome.apply) land(asking.key, answer);
      },
      /**
       * Puts a write's own answer about `key` on record, made on the value there now. Returns whether
       * it was applied: a question the server orders keeps a newer answer it already has.
       */
      put: (key: string, change: (current: T | undefined) => T | undefined): boolean => {
        const outcome = written(ledgerNow.current, key, change, latest.current.newer);
        commit(outcome.ledger);
        if (outcome.apply && outcome.data !== undefined) land(key, outcome.data);
        return outcome.apply;
      },
      /** A moment every read asked from now on comes after. */
      mark: (): number => {
        const [next, number] = marked(ledgerNow.current);
        commit(next);
        return number;
      },
    };
  }, []);

  const key = question?.key ?? null;
  const value = valueOn(ledger, key);
  return {
    ...actions,
    /** The answer on record for the question on screen, or null. */
    value,
    /** That answer, or while there is none yet, the one shown for the question before. */
    shown: value ?? lastOnScreen,
    /** The failure to show for the question on screen, or null. */
    failure: failureOn(ledger, key),
    /** A read of the question on screen is on its way. */
    pending: pendingOn(ledger, key),
    /** The number of the newest read of the question on screen that answered, to compare with `mark`. */
    answeredUpTo: answeredUpTo(ledger, key),
  };
}
