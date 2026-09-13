/**
 * Names the app gives things before the server has seen them.
 */

import { describe, expect, it } from "vitest";

import { uuid } from "../ids";

const V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

describe("a new id", () => {
  it("is the platform's own UUID where it has one", () => {
    const source = {
      randomUUID: () => "0b5c7c9e-58a4-4f3e-9a55-1d2c3b4a5f60",
      getRandomValues: (array: Uint8Array) => array,
    };
    expect(uuid(source)).toBe("0b5c7c9e-58a4-4f3e-9a55-1d2c3b4a5f60");
  });

  it("is a version-4 UUID made from random bytes where randomUUID is missing, as in older iOS webviews", () => {
    expect(uuid({ getRandomValues: (array: Uint8Array) => array.fill(0xff) })).toBe(
      "ffffffff-ffff-4fff-bfff-ffffffffffff",
    );
    expect(uuid({ getRandomValues: (array: Uint8Array) => array.fill(0) })).toBe(
      "00000000-0000-4000-8000-000000000000",
    );
    expect(uuid({ getRandomValues: (array: Uint8Array) => crypto.getRandomValues(array) })).toMatch(V4);
  });

  it("differs every time", () => {
    expect(uuid()).toMatch(V4);
    expect(uuid()).not.toBe(uuid());
  });
});
