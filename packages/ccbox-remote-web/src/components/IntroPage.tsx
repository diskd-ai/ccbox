import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Loader2, Settings } from "lucide-react";
import { useConnection } from "@/hooks/useConnection";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { getOrCreateIdentity } from "@/domain/identity";
import { pairDevice } from "@/domain/pairing";
import { toPairUrl } from "@/domain/url";
import type { ConnectionState, DeviceIdentity } from "@/domain/types";

const STATE_HINT: Partial<Record<ConnectionState, string>> = {
  connecting: "Opening connection...",
  awaiting_challenge: "Awaiting challenge...",
  authenticating: "Authenticating...",
  auth_failed: "Authentication failed",
};

function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
}

function deriveGuidOrConnectValue(connectParam: string): string {
  const raw = connectParam.trim();
  if (!raw) return "";
  if (isUuid(raw)) return raw;

  try {
    const url = new URL(raw);
    const guid = url.searchParams.get("guid")?.trim() ?? "";
    if (isUuid(guid)) return guid;

    // Support links like: wss://<guid>.ccbox.app/client
    const host = url.hostname.trim().toLowerCase();
    const sub = host.split(".")[0]?.trim() ?? "";
    if (isUuid(sub)) return sub;
  } catch {
    // Fall back to raw connect input (handled by toWsUrl).
  }

  return raw;
}

export function IntroPage() {
  const { state, url, connect, error } = useConnection();
  const { theme, setTheme } = usePreferencesStore();

  const [urlInput, setUrlInput] = useState(url);
  const [pairingCode, setPairingCode] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [pairing, setPairing] = useState(false);
  const [pairStatus, setPairStatus] = useState<string | null>(null);
  const [identity, setIdentity] = useState<DeviceIdentity | null>(null);
  const [showPairing, setShowPairing] = useState(false);

  useEffect(() => {
    void getOrCreateIdentity().then(setIdentity);
  }, []);

  // QR scan deep links include these query params:
  //   ?connect=wss://...&pairing_code=...&autoconnect=1
  useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const connectParam = params.get("connect")?.trim() ?? "";
    const pairingParam = params.get("pairing_code")?.trim() ?? "";
    const autoconnectParam = params.get("autoconnect")?.trim() ?? "";

    const derivedConnect = connectParam ? deriveGuidOrConnectValue(connectParam) : "";
    if (derivedConnect) {
      setUrlInput(derivedConnect);
    }
    if (pairingParam) {
      setPairingCode(pairingParam);
      setShowPairing(true);
    }

    if (autoconnectParam === "1" && derivedConnect) {
      void connect(derivedConnect);
    }
  }, [connect]);

  const pairEndpoint = useMemo(() => {
    if (!urlInput.trim()) return null;
    try {
      return toPairUrl(urlInput);
    } catch {
      return null;
    }
  }, [urlInput]);

  const handleConnect = async () => {
    if (!urlInput.trim()) return;
    setConnecting(true);
    try {
      await connect(urlInput.trim());
    } finally {
      setConnecting(false);
    }
  };

  const handlePair = async () => {
    if (!identity) {
      setPairStatus("Identity not loaded");
      return;
    }
    setPairing(true);
    setPairStatus("Pairing...");
    try {
      const result = await pairDevice(urlInput, pairingCode, identity);
      if (result.tag === "Ok") {
        setPairStatus("Paired successfully");
        // Most users pair because the current connection attempt returned DeviceUnknown.
        // After successful pairing, reconnect automatically so the app loads immediately.
        if (urlInput.trim()) {
          void connect(urlInput.trim());
        }
      } else {
        setPairStatus(result.message);
      }
    } finally {
      setPairing(false);
    }
  };

  const isBusy =
    state === "connecting" ||
    state === "awaiting_challenge" ||
    state === "authenticating";

  const stateHint = STATE_HINT[state] ?? null;

  return (
    <div className="flex flex-col h-dvh overflow-hidden bg-background">
      {/* Minimal top bar */}
      <header className="flex items-center justify-between px-4 h-11 border-b border-border bg-surface shadow-card shrink-0">
        <span className="font-mono text-md font-bold text-foreground tracking-tight">
          ccbox
        </span>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1">
            {(["dark", "light", "system"] as const).map((t) => (
              <button
                key={t}
                onClick={() => setTheme(t)}
                className={`px-2 py-0.5 text-xs font-sans font-medium border transition-all rounded-md capitalize ${
                  theme === t
                    ? "border-primary bg-primary text-primary-foreground"
                    : "border-transparent text-text-muted hover:text-text-secondary"
                }`}
              >
                {t}
              </button>
            ))}
          </div>
          <Link
            to="/settings"
            className="text-text-secondary hover:text-foreground transition-colors"
          >
            <Settings className="w-3.5 h-3.5" />
          </Link>
        </div>
      </header>

      {/* Center content */}
      <main className="flex-1 flex items-center justify-center p-4 overflow-auto">
        <div className="w-full max-w-[420px] space-y-6">
          {/* Logo + tagline */}
          <div className="text-center space-y-2">
            <h1 className="font-mono text-2xl font-bold text-foreground tracking-tight">
              ccbox<span className="text-text-muted font-normal"> remote</span>
            </h1>
            <p className="text-sm text-text-secondary leading-relaxed">
              Connect to your ccbox instance to browse sessions,
              manage projects, and monitor coding agents.
            </p>
          </div>

          {/* Connection card */}
          <div className="border border-border bg-surface rounded-xl shadow-card overflow-hidden">
            <div className="px-4 py-2.5 border-b border-border text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">
              Connect
            </div>
            <div className="px-4 py-4 space-y-4">
              <div className="space-y-1.5">
                <div className="text-xs text-text-secondary">
                  CCBox URL or UUID
                </div>
                <div className="flex gap-2">
                  <input
                    type="text"
                    placeholder="wss://... or UUID"
                    value={urlInput}
                    onChange={(e) => setUrlInput(e.target.value)}
                    onKeyDown={(e) =>
                      e.key === "Enter" && !isBusy && handleConnect()
                    }
                    disabled={isBusy}
                    className="flex-1 bg-background border border-border text-foreground px-3 py-2 text-sm rounded-lg focus:outline-none focus:border-ring focus:shadow-elevated placeholder:text-text-muted transition-shadow disabled:opacity-60"
                  />
                  <button
                    onClick={handleConnect}
                    disabled={isBusy || connecting || !urlInput.trim()}
                    className="px-4 py-2 text-sm font-sans font-medium bg-primary text-primary-foreground rounded-lg hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center gap-1.5 shadow-card"
                  >
                    {isBusy && (
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                    )}
                    Connect
                  </button>
                </div>
              </div>

              {/* State hint */}
              {stateHint && (
                <div className="flex items-center gap-2 text-xs text-status-warning">
                  <div className="w-1.5 h-1.5 rounded-full bg-status-warning glow-warning animate-pulse-dot" />
                  {stateHint}
                </div>
              )}

              {/* Error */}
              {error && (
                <div className="text-xs text-status-error bg-status-error/10 px-3 py-2 rounded-lg">
                  {error}
                </div>
              )}

              {/* Pairing toggle */}
              {!showPairing ? (
                <button
                  onClick={() => setShowPairing(true)}
                  className="text-xs text-text-muted hover:text-text-secondary transition-colors"
                >
                  Need to pair this device?
                </button>
              ) : (
                <div className="space-y-3 pt-1 border-t border-border">
                  <div className="space-y-1.5 pt-3">
                    <div className="text-xs text-text-secondary">
                      Pairing Code
                    </div>
                    <div className="flex gap-2">
                      <input
                        type="text"
                        placeholder="Enter pairing code"
                        value={pairingCode}
                        onChange={(e) => setPairingCode(e.target.value)}
                        className="flex-1 bg-background border border-border text-foreground px-3 py-2 text-sm rounded-lg focus:outline-none focus:border-ring focus:shadow-elevated placeholder:text-text-muted transition-shadow"
                      />
                      <button
                        onClick={handlePair}
                        disabled={
                          pairing ||
                          !pairingCode.trim() ||
                          !urlInput.trim() ||
                          !identity
                        }
                        className="px-4 py-2 text-sm font-sans font-medium bg-status-online text-primary-foreground rounded-lg hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center gap-1.5 shadow-card"
                      >
                        {pairing && (
                          <Loader2 className="w-3.5 h-3.5 animate-spin" />
                        )}
                        Pair
                      </button>
                    </div>
                    {pairEndpoint && (
                      <div className="text-xs text-text-muted font-mono truncate">
                        {pairEndpoint}
                      </div>
                    )}
                  </div>

                  {pairStatus && (
                    <div className="text-xs text-status-warning bg-status-warning/10 px-3 py-2 rounded-lg">
                      {pairStatus}
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>

          {/* Supported engines */}
          <div className="flex items-center justify-center gap-3">
            {[
              { label: "Codex", color: "bg-engine-codex" },
              { label: "Claude", color: "bg-engine-claude" },
              { label: "Gemini", color: "bg-engine-gemini" },
              { label: "OpenCode", color: "bg-engine-opencode" },
            ].map(({ label, color }) => (
              <div
                key={label}
                className="flex items-center gap-1.5 text-xs text-text-muted"
              >
                <div className={`w-1.5 h-1.5 rounded-full ${color}`} />
                {label}
              </div>
            ))}
          </div>
        </div>
      </main>
    </div>
  );
}
