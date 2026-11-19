import { describe, expect, it } from "vitest";
import { AUTH_DOMAIN_SEPARATOR, buildAuthMessageV1 } from "@/domain/authMessage";

describe("buildAuthMessageV1", () => {
  it("concatenates domain, kind, id, and nonce bytes in order", () => {
    const nonce = new Uint8Array([1, 2, 3]);
    const message = buildAuthMessageV1("client", "device-1", nonce);

    const asArray = Array.from(message);
    const prefix = Array.from(new TextEncoder().encode(`${AUTH_DOMAIN_SEPARATOR}clientdevice-1`));

    expect(asArray.slice(0, prefix.length)).toEqual(prefix);
    expect(asArray.slice(prefix.length)).toEqual([1, 2, 3]);
  });
});
