/**
 * What a call carries to prove who is making it.
 *
 * Telegram's payload is accepted for an hour and never refreshed while the app stays open, so a
 * client that kept sending it locked a bartender out of the shift every hour.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import { client } from "../api";

function reply(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
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
