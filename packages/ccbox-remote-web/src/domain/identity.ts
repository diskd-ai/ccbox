import { buildAuthMessageV1 } from "@/domain/authMessage";
import { base64ToBytes, bytesToBase64 } from "@/domain/base64";
import type { DeviceIdentity } from "@/domain/types";

const DB_NAME = "ccbox-identity";
const STORE_NAME = "device";
const STORAGE_KEY = "device_identity_v1";
const LOCAL_STORAGE_KEY = "ccbox-identity-v1";

type StoredIdentity = {
  readonly kind: "webcrypto-ed25519-v1";
  readonly device_id: string;
  readonly created_at: string;
  readonly public_key_b64: string;
  readonly private_key_pkcs8_b64: string;
};

function isObject(value: unknown): value is { [key: string]: unknown } {
  return typeof value === "object" && value !== null;
}

function isStoredIdentity(value: unknown): value is StoredIdentity {
  if (!isObject(value)) return false;
  return (
    value.kind === "webcrypto-ed25519-v1" &&
    typeof value.device_id === "string" &&
    typeof value.created_at === "string" &&
    typeof value.public_key_b64 === "string" &&
    typeof value.private_key_pkcs8_b64 === "string"
  );
}

function isCryptoKeyPair(value: CryptoKey | CryptoKeyPair): value is CryptoKeyPair {
  const pair = value as CryptoKeyPair;
  return typeof pair.publicKey === "object" && typeof pair.privateKey === "object";
}

function generateUuid(): string {
  if (typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }

  const bytes = crypto.getRandomValues(new Uint8Array(16));
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      if (!req.result.objectStoreNames.contains(STORE_NAME)) {
        req.result.createObjectStore(STORE_NAME);
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("Failed to open IndexedDB"));
  });
}

async function loadStoredFromIdb(): Promise<StoredIdentity | null> {
  const db = await openDb();
  try {
    const row = await new Promise<unknown>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readonly");
      const store = tx.objectStore(STORE_NAME);
      const req = store.get(STORAGE_KEY);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error ?? new Error("Failed to read identity"));
    });

    return isStoredIdentity(row) ? row : null;
  } finally {
    db.close();
  }
}

async function saveStoredToIdb(stored: StoredIdentity): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      const store = tx.objectStore(STORE_NAME);
      const req = store.put(stored, STORAGE_KEY);
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error ?? new Error("Failed to persist identity"));
    });
  } finally {
    db.close();
  }
}

async function removeStoredFromIdb(): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      const store = tx.objectStore(STORE_NAME);
      const req = store.delete(STORAGE_KEY);
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error ?? new Error("Failed to delete identity"));
    });
  } finally {
    db.close();
  }
}

function loadStoredFromLocalStorage(): StoredIdentity | null {
  try {
    const raw = localStorage.getItem(LOCAL_STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return isStoredIdentity(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function saveStoredToLocalStorage(stored: StoredIdentity): void {
  localStorage.setItem(LOCAL_STORAGE_KEY, JSON.stringify(stored));
}

function removeStoredFromLocalStorage(): void {
  localStorage.removeItem(LOCAL_STORAGE_KEY);
}

async function importStoredIdentity(stored: StoredIdentity): Promise<DeviceIdentity> {
  const publicKey = await crypto.subtle.importKey(
    "raw",
    base64ToBytes(stored.public_key_b64),
    { name: "Ed25519" },
    true,
    ["verify"],
  );

  const privateKey = await crypto.subtle.importKey(
    "pkcs8",
    base64ToBytes(stored.private_key_pkcs8_b64),
    { name: "Ed25519" },
    true,
    ["sign"],
  );

  return {
    kind: "webcrypto-ed25519-v1",
    device_id: stored.device_id,
    created_at: stored.created_at,
    public_key_b64: stored.public_key_b64,
    public_key: publicKey,
    private_key: privateKey,
  };
}

async function generateNewIdentity(): Promise<{ runtime: DeviceIdentity; stored: StoredIdentity }> {
  let generated: CryptoKey | CryptoKeyPair;
  try {
    generated = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  } catch (error) {
    throw new Error(`WebCrypto Ed25519 unavailable: ${String(error)}`);
  }

  if (!isCryptoKeyPair(generated)) {
    throw new Error("WebCrypto did not return a key pair");
  }

  const publicKeyRaw = new Uint8Array(await crypto.subtle.exportKey("raw", generated.publicKey));
  const privateKeyPkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", generated.privateKey));

  const stored: StoredIdentity = {
    kind: "webcrypto-ed25519-v1",
    device_id: generateUuid(),
    created_at: new Date().toISOString(),
    public_key_b64: bytesToBase64(publicKeyRaw),
    private_key_pkcs8_b64: bytesToBase64(privateKeyPkcs8),
  };

  const runtime = await importStoredIdentity(stored);
  return { runtime, stored };
}

async function persistStoredIdentity(stored: StoredIdentity): Promise<void> {
  try {
    await saveStoredToIdb(stored);
  } catch {
    saveStoredToLocalStorage(stored);
  }
}

export async function getOrCreateIdentity(): Promise<DeviceIdentity> {
  const idbStored = await (async () => {
    try {
      return await loadStoredFromIdb();
    } catch {
      return null;
    }
  })();

  if (idbStored) {
    try {
      return await importStoredIdentity(idbStored);
    } catch {
      removeStoredFromLocalStorage();
    }
  }

  const localStored = loadStoredFromLocalStorage();
  if (localStored) {
    try {
      const runtime = await importStoredIdentity(localStored);
      await persistStoredIdentity(localStored);
      return runtime;
    } catch {
      removeStoredFromLocalStorage();
    }
  }

  const generated = await generateNewIdentity();
  await persistStoredIdentity(generated.stored);
  return generated.runtime;
}

export async function resetIdentity(): Promise<DeviceIdentity> {
  try {
    await removeStoredFromIdb();
  } catch {
    // Ignore and continue local fallback cleanup.
  }
  removeStoredFromLocalStorage();
  return getOrCreateIdentity();
}

export async function signChallenge(identity: DeviceIdentity, nonce_b64: string): Promise<string> {
  const nonceBytes = base64ToBytes(nonce_b64);
  const message = buildAuthMessageV1("client", identity.device_id, nonceBytes);
  const signature = await crypto.subtle.sign({ name: "Ed25519" }, identity.private_key, message);
  return bytesToBase64(new Uint8Array(signature));
}
