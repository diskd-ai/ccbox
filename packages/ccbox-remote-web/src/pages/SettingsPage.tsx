import { useEffect, useMemo, useState } from "react";
import { Copy, Loader2 } from "lucide-react";
import { useConnection } from "@/hooks/useConnection";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { getOrCreateIdentity, resetIdentity } from "@/domain/identity";
import { pairDevice } from "@/domain/pairing";
import { toPairUrl } from "@/domain/url";
import type { DeviceIdentity } from "@/domain/types";

function formatConnectionState(state: string): string {
  switch (state) {
    case "disconnected":
      return "Disconnected";
    case "connecting":
      return "Connecting";
    case "awaiting_challenge":
      return "Awaiting challenge";
    case "authenticating":
      return "Authenticating";
    case "online":
      return "Online";
    case "reconnecting":
      return "Reconnecting";
    case "auth_failed":
      return "Authentication failed";
    default:
      return state;
  }
}

const SettingsPage = () => {
  const { state, url, connect, disconnect, error } = useConnection();
  const { theme, setTheme } = usePreferencesStore();

  const [urlInput, setUrlInput] = useState(url);
  const [pairingCode, setPairingCode] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [pairing, setPairing] = useState(false);
  const [pairStatus, setPairStatus] = useState<string | null>(null);
  const [identity, setIdentity] = useState<DeviceIdentity | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    void getOrCreateIdentity().then(setIdentity);
  }, []);

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
        setPairStatus("Paired");
      } else {
        setPairStatus(result.message);
      }
    } finally {
      setPairing(false);
    }
  };

  const handleResetIdentity = async () => {
    if (!confirm("This will generate a new device identity. You will need to re-pair with your CCBox. Continue?")) return;
    const nextIdentity = await resetIdentity();
    setIdentity(nextIdentity);
    setPairStatus("Identity reset. Pair again before connecting.");
  };

  const copyToClipboard = (text: string, key: string) => {
    void navigator.clipboard.writeText(text);
    setCopied(key);
    setTimeout(() => setCopied(null), 1500);
  };

  const isOnline = state === "online";

  return (
    <div className="p-4 lg:p-6 max-w-[800px] mx-auto space-y-6">
      <h1 className="text-xl font-sans font-semibold text-foreground">Settings</h1>

      <div className="border border-border bg-surface rounded-xl shadow-card overflow-hidden">
        <div className="px-4 py-2.5 border-b border-border text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">
          Device Identity
        </div>
        <div className="px-4 py-4 space-y-4">
          {identity ? (
            <>
              <div className="flex items-center justify-between gap-4">
                <div className="min-w-0">
                  <div className="text-xs text-text-secondary mb-0.5">Device ID</div>
                  <div className="text-sm text-foreground font-mono truncate">{identity.device_id}</div>
                </div>
                <button
                  onClick={() => copyToClipboard(identity.device_id, "device_id")}
                  className="text-text-secondary hover:text-foreground transition-colors shrink-0"
                >
                  {copied === "device_id" ? (
                    <span className="text-xs text-status-online font-medium">Copied</span>
                  ) : (
                    <Copy className="w-3.5 h-3.5" />
                  )}
                </button>
              </div>
              <div className="flex items-center justify-between gap-4">
                <div className="min-w-0">
                  <div className="text-xs text-text-secondary mb-0.5">Public Key (Base64)</div>
                  <div className="text-sm text-foreground font-mono truncate">{identity.public_key_b64}</div>
                </div>
                <button
                  onClick={() => copyToClipboard(identity.public_key_b64, "public_key")}
                  className="text-text-secondary hover:text-foreground transition-colors shrink-0"
                >
                  {copied === "public_key" ? (
                    <span className="text-xs text-status-online font-medium">Copied</span>
                  ) : (
                    <Copy className="w-3.5 h-3.5" />
                  )}
                </button>
              </div>
            </>
          ) : (
            <div className="text-sm text-text-muted">Loading identity...</div>
          )}

          <button
            onClick={handleResetIdentity}
            className="px-3 py-1.5 text-xs font-sans font-medium border border-status-error text-status-error rounded-lg hover:bg-status-error hover:text-primary-foreground transition-colors"
          >
            Reset Identity
          </button>
        </div>
      </div>

      <div className="border border-border bg-surface rounded-xl shadow-card overflow-hidden">
        <div className="px-4 py-2.5 border-b border-border text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">
          Connection and Pairing
        </div>
        <div className="px-4 py-4 space-y-4">
          <div className="space-y-1.5">
            <div className="text-xs text-text-secondary">CCBox URL or UUID</div>
            <div className="flex gap-2">
              <input
                type="text"
                placeholder="wss://... or UUID"
                value={urlInput}
                onChange={(e) => setUrlInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && handleConnect()}
                className="flex-1 bg-background border border-border text-foreground px-3 py-2 text-sm rounded-lg focus:outline-none focus:border-ring focus:shadow-elevated placeholder:text-text-muted transition-shadow"
              />
              {!isOnline ? (
                <button
                  onClick={handleConnect}
                  disabled={connecting || !urlInput.trim()}
                  className="px-4 py-2 text-sm font-sans font-medium bg-primary text-primary-foreground rounded-lg hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center gap-1.5 shadow-card"
                >
                  {connecting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                  Connect
                </button>
              ) : (
                <button
                  onClick={disconnect}
                  className="px-4 py-2 text-sm font-sans border border-border text-text-secondary rounded-lg hover:text-foreground hover:bg-bg-hover transition-colors"
                >
                  Disconnect
                </button>
              )}
            </div>
          </div>

          <div className="space-y-1.5">
            <div className="text-xs text-text-secondary">Pairing Code</div>
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
                disabled={pairing || !pairingCode.trim() || !urlInput.trim() || !identity}
                className="px-4 py-2 text-sm font-sans font-medium bg-status-online text-primary-foreground rounded-lg hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center gap-1.5 shadow-card"
              >
                {pairing && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                Pair
              </button>
            </div>
            {pairEndpoint && (
              <div className="text-xs text-text-muted font-mono truncate">{pairEndpoint}</div>
            )}
          </div>

          {pairStatus && (
            <div className="text-xs text-status-warning bg-status-warning/10 px-3 py-2 rounded-lg">{pairStatus}</div>
          )}

          {error && (
            <div className="text-xs text-status-error bg-status-error/10 px-3 py-2 rounded-lg">{error}</div>
          )}

          <div className="flex items-center gap-2 text-sm">
            <span className={`w-2 h-2 rounded-full ${isOnline ? "bg-status-online glow-online" : "bg-text-muted"}`} />
            <span className={isOnline ? "text-status-online font-medium" : "text-text-secondary"}>
              {formatConnectionState(state)}
            </span>
          </div>
        </div>
      </div>

      <div className="border border-border bg-surface rounded-xl shadow-card overflow-hidden">
        <div className="px-4 py-2.5 border-b border-border text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">
          Appearance
        </div>
        <div className="px-4 py-4">
          <div className="flex items-center gap-2">
            {(["dark", "light", "system"] as const).map((t) => (
              <button
                key={t}
                onClick={() => setTheme(t)}
                className={`px-4 py-1.5 text-sm font-sans font-medium border transition-all rounded-lg capitalize ${
                  theme === t
                    ? "border-primary bg-primary text-primary-foreground shadow-card"
                    : "border-border text-text-secondary hover:text-foreground hover:bg-bg-hover"
                }`}
              >
                {t}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="text-xs text-text-muted">
        Press <kbd className="bg-secondary px-1.5 py-0.5 font-mono rounded-md">?</kbd> to view keyboard shortcuts
      </div>
    </div>
  );
};

export default SettingsPage;
