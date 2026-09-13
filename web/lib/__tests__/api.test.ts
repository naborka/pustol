/**
 * What a call carries to prove who is making it.
 *
 * Telegram's payload is accepted for an hour and never refreshed while the app stays open, so a
 * client that kept sending it locked a bartender out of the shift every hour.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import { REQUEST_TIMEOUT_MS, client } from "../api";

function reply(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function refusal(status: number, code: string): Response {
  return new Response(JSON.stringify({ error: { code, message: code } }), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function authorizationOf(call: unknown[] | undefined): string | undefined {
  const init = call?.[1] as { headers?: Record<string, string> } | undefined;
  return init?.headers?.authorization;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the proof a call carries", () => {
  it("sends Telegram's payload until the first screen hands back a session, and the session after", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(reply({ session_token: "claims.signature" }))
      .mockResolvedValue(reply({ party_size: 2, days: [] }));
    vi.stubGlobal("fetch", fetch);

    const api = client("signed-by-telegram");
    await api.session();
    await api.days(2);

    expect(authorizationOf(fetch.mock.calls[0])).toBe("tma signed-by-telegram");
    expect(authorizationOf(fetch.mock.calls[1])).toBe("session claims.signature");
  });
});

describe("a session that has ended", () => {
  const session = { session_token: "claims.signature" };
  const expired = { code: "session_expired", message: "session_expired" };

  it("ends on any refusal only reopening the app answers, and says so once", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValueOnce(reply(session)).mockResolvedValueOnce(refusal(401, "session_expired")),
    );
    const told = vi.fn();
    const api = client("signed", told);
    await api.session();
    await expect(api.days(2)).rejects.toMatchObject({ failure: expired });
    expect(told.mock.calls).toEqual([[{ failure: expired, retrying: false }]]);
  });

  it("does not end on a failure asking again can answer", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(refusal(500, "internal")));
    const told = vi.fn();
    await expect(client("signed", told).days(2)).rejects.toMatchObject({ failure: { code: "internal" } });
    expect(told).not.toHaveBeenCalled();
  });

  it("refuses every other call without asking the server while it stays ended", async () => {
    const fetch = vi.fn().mockResolvedValue(refusal(401, "session_expired"));
    vi.stubGlobal("fetch", fetch);
    const api = client("signed", vi.fn());
    await expect(api.days(2)).rejects.toMatchObject({ failure: expired });
    await expect(api.shift("2026-09-11")).rejects.toMatchObject({ failure: expired });
    await expect(api.cancelMine("b1")).rejects.toMatchObject({ failure: expired });
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("comes back only when a session read asked after the end answers, and says it is asking", async () => {
    let answerEarly: (response: Response) => void = () => {};
    const fetch = vi
      .fn()
      .mockReturnValueOnce(new Promise<Response>((resolve) => (answerEarly = resolve)))
      .mockResolvedValueOnce(refusal(401, "session_expired"))
      .mockResolvedValueOnce(reply(session))
      .mockResolvedValueOnce(reply({ party_size: 2, days: [] }));
    vi.stubGlobal("fetch", fetch);
    const told = vi.fn();
    const api = client("signed", told);

    const early = api.session();
    await expect(api.days(2)).rejects.toMatchObject({ failure: expired });
    answerEarly(reply(session));
    await early;
    expect(told).toHaveBeenLastCalledWith({ failure: expired, retrying: false });
    await expect(api.days(2)).rejects.toMatchObject({ failure: expired });

    const retry = api.session();
    expect(told).toHaveBeenLastCalledWith({ failure: expired, retrying: true });
    await retry;
    expect(told).toHaveBeenLastCalledWith(null);
    await expect(api.days(2)).resolves.toEqual({ party_size: 2, days: [] });
  });

  it("says why a session read asked after the end failed, and stays ended", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(refusal(401, "session_expired"))
      .mockResolvedValueOnce(refusal(503, "network"));
    vi.stubGlobal("fetch", fetch);
    const told = vi.fn();
    const api = client("signed", told);

    await expect(api.session()).rejects.toMatchObject({ failure: expired });
    await expect(api.session()).rejects.toMatchObject({ failure: { code: "network" } });
    expect(told).toHaveBeenLastCalledWith({
      failure: { code: "network", message: "network" },
      retrying: false,
    });
    await expect(api.days(2)).rejects.toMatchObject({ failure: { code: "network" } });
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});

describe("a network that does not answer", () => {
  it("names a dropped connection as the phone's rather than the bar's", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
    await expect(client("signed").days(2)).rejects.toMatchObject({
      failure: { code: "network" },
    });
  });

  it("gives up on a request that never comes back instead of spinning for ever", async () => {
    vi.useFakeTimers();
    try {
      vi.stubGlobal(
        "fetch",
        vi.fn(
          (_path: string, init: RequestInit) =>
            new Promise((_resolve, reject) => {
              init.signal?.addEventListener("abort", () =>
                reject(new DOMException("aborted", "AbortError")),
              );
            }),
        ),
      );
      const outcome = expect(client("signed").days(2)).rejects.toMatchObject({
        failure: { code: "network" },
      });
      await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);
      await outcome;
    } finally {
      vi.useRealTimers();
    }
  });
});
