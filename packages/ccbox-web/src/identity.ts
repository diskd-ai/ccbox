import { bytesToBase64 } from "./base64";
import { kvDel, kvGet, kvSet } from "./idbKv";

const IDENTITY_KEY = "device_identity_v1";

export type DeviceIdentity = WebCryptoEd25519Identity;

export type WebCryptoEd25519Identity = {
  readonly kind: "webcrypto-ed25519";
  readonly deviceId: string;
  readonly publicKeyRawB64: string;
  readonly publicKey: CryptoKey;
  readonly privateKey: CryptoKey;
};

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isWebCryptoEd25519Identity(value: unknown): value is WebCryptoEd25519Identity {
  if (!isObject(value)) return false;
  return (
    value.kind === "webcrypto-ed25519" &&
    typeof value.deviceId === "string" &&
    typeof value.publicKeyRawB64 === "string" &&
    typeof value.publicKey === "object" &&
    typeof value.privateKey === "object"
  );
}

function isCryptoKeyPair(value: CryptoKey | CryptoKeyPair): value is CryptoKeyPair {
  return (
    typeof (value as CryptoKeyPair).publicKey === "object" &&
    typeof (value as CryptoKeyPair).privateKey === "object"
  );
}

export async function loadOrCreateIdentity(): Promise<DeviceIdentity> {
  const existing = await kvGet(IDENTITY_KEY);
  if (isWebCryptoEd25519Identity(existing)) return existing;

  const deviceId = crypto.randomUUID();

  let generated: CryptoKey | CryptoKeyPair;
  try {
    generated = await crypto.subtle.generateKey({ name: "Ed25519" }, false, ["sign", "verify"]);
  } catch (err) {
    throw new Error(
      `WebCrypto Ed25519 is not available in this browser. (${String(err)}) Use a browser with Ed25519 WebCrypto support, or add a JS Ed25519 fallback.`,
    );
  }

  if (!isCryptoKeyPair(generated)) {
    throw new Error("WebCrypto returned an unexpected key type (expected key pair).");
  }

  const publicKeyRaw = new Uint8Array(await crypto.subtle.exportKey("raw", generated.publicKey));

  const identity: DeviceIdentity = {
    kind: "webcrypto-ed25519",
    deviceId,
    publicKeyRawB64: bytesToBase64(publicKeyRaw),
    publicKey: generated.publicKey,
    privateKey: generated.privateKey,
  };

  await kvSet(IDENTITY_KEY, identity);
  return identity;
}

export async function resetIdentity(): Promise<void> {
  await kvDel(IDENTITY_KEY);
}

export async function sign(
  identity: DeviceIdentity,
  message: Uint8Array<ArrayBuffer>,
): Promise<Uint8Array<ArrayBuffer>> {
  if (identity.kind !== "webcrypto-ed25519") {
    throw new Error(`Unsupported identity kind: ${identity.kind}`);
  }

  const signature = await crypto.subtle.sign({ name: "Ed25519" }, identity.privateKey, message);
  return new Uint8Array(signature);
}
