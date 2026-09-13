/**
 * Names the app gives things before the server has seen them.
 *
 * `crypto.randomUUID` is missing from older iOS webviews and from any page not served over HTTPS, so
 * a version-4 UUID is built from random bytes where it is.
 */

export interface RandomSource {
  randomUUID?: () => string;
  getRandomValues: (array: Uint8Array) => Uint8Array;
}

export function uuid(source: RandomSource = crypto): string {
  if (typeof source.randomUUID === "function") return source.randomUUID();
  const bytes = Array.from(source.getRandomValues(new Uint8Array(16)), (byte, index) =>
    index === 6 ? (byte & 0x0f) | 0x40 : index === 8 ? (byte & 0x3f) | 0x80 : byte,
  );
  const hex = bytes.map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return [hex.slice(0, 8), hex.slice(8, 12), hex.slice(12, 16), hex.slice(16, 20), hex.slice(20)].join(
    "-",
  );
}
