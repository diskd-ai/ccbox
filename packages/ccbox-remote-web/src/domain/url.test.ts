import { describe, expect, it } from "vitest";
import { toPairUrl, toWsUrl } from "@/domain/url";

describe("toWsUrl", () => {
  it("normalizes https URL to wss client endpoint", () => {
    expect(toWsUrl("https://relay.example.com")).toBe("wss://relay.example.com/client");
  });

  it("uses ws scheme for localhost-like targets", () => {
    expect(toWsUrl("localhost:8787")).toBe("ws://localhost:8787/client");
  });

  it("resolves UUID targets to ccbox relay host", () => {
    expect(toWsUrl("01234567-89ab-cdef-0123-456789abcdef")).toBe(
      "wss://01234567-89ab-cdef-0123-456789abcdef.ccbox.app/client",
    );
  });
});

describe("toPairUrl", () => {
  it("maps websocket endpoint to https /pair", () => {
    expect(toPairUrl("wss://relay.example.com/client")).toBe("https://relay.example.com/pair");
  });
});
