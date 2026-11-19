function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
}

function getRelayBase(): string | null {
  const raw = import.meta.env.VITE_CCBOX_RELAY_BASE;
  if (typeof raw !== "string") return null;
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function getCcboxHostedBase(): string | null {
  if (typeof window === "undefined") return null;

  const host = window.location.hostname.trim().toLowerCase();
  if (host !== "ccbox.app" && !host.endsWith(".ccbox.app")) {
    return null;
  }

  return window.location.host;
}

function buildRelayClientWsUrl(relayBase: string, guid: string): string {
  const base = relayBase.startsWith("wss://") || relayBase.startsWith("ws://") ? relayBase : `wss://${relayBase}`;
  const url = new URL(base);

  if (url.protocol === "https:") url.protocol = "wss:";
  if (url.protocol === "http:") url.protocol = "ws:";

  url.pathname = "/client";
  url.searchParams.set("guid", guid);
  url.hash = "";
  return url.toString();
}

function isLikelyLocalHost(hostPort: string): boolean {
  const value = hostPort.trim().toLowerCase();
  if (value.length === 0) return false;

  const host = (() => {
    if (value.startsWith("[")) {
      const end = value.indexOf("]");
      return end > 0 ? value.slice(1, end) : value;
    }

    const idx = value.indexOf(":");
    return idx >= 0 ? value.slice(0, idx) : value;
  })();

  if (host === "localhost") return true;
  if (host.endsWith(".local")) return true;
  if (host === "::1") return true;
  if (host.startsWith("fe80:")) return true;
  if (host.startsWith("fc") || host.startsWith("fd")) return true;

  const ipv4Parts = host.split(".");
  if (ipv4Parts.length !== 4) return false;
  if (!ipv4Parts.every((part) => /^\d+$/.test(part))) return false;

  const octets = ipv4Parts.map((part) => Number.parseInt(part, 10));
  if (octets.some((n) => Number.isNaN(n) || n < 0 || n > 255)) return false;

  const a = octets[0] ?? -1;
  const b = octets[1] ?? -1;

  if (a === 10 || a === 127) return true;
  if (a === 192 && b === 168) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 169 && b === 254) return true;

  return false;
}

/** Resolve user input to relay websocket endpoint. */
export function toWsUrl(raw: string): string {
  const value = raw.trim();
  if (!value) {
    throw new Error("Missing connect value");
  }

  if (value.startsWith("wss://") || value.startsWith("ws://")) {
    const url = new URL(value);
    if (!url.pathname || url.pathname === "/" || url.pathname.startsWith("/ccbox")) {
      url.pathname = "/client";
    }
    return url.toString();
  }

  if (value.startsWith("https://") || value.startsWith("http://")) {
    const url = new URL(value);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    if (!url.pathname || url.pathname === "/") {
      url.pathname = "/client";
    }
    return url.toString();
  }

  if (isUuid(value)) {
    const relayBase = getRelayBase();
    if (relayBase) {
      return buildRelayClientWsUrl(relayBase, value);
    }

    const hostedBase = getCcboxHostedBase();
    if (hostedBase) {
      return buildRelayClientWsUrl(hostedBase, value);
    }

    return `wss://${value}.ccbox.app/client`;
  }

  const hostPort = value.split(/[/?#]/)[0] ?? "";
  const scheme = isLikelyLocalHost(hostPort) ? "ws" : "wss";

  if (/[/?#]/.test(value)) {
    return `${scheme}://${value}`;
  }
  return `${scheme}://${value}/client`;
}

/** Convert connect input to pairing endpoint. */
export function toPairUrl(raw: string): string {
  const wsUrl = toWsUrl(raw);
  const url = new URL(wsUrl);

  if (url.protocol === "ws:") url.protocol = "http:";
  else if (url.protocol === "wss:") url.protocol = "https:";
  else throw new Error("Unsupported URL scheme");

  url.pathname = "/pair";
  url.hash = "";
  return url.toString();
}
