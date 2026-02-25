const DB_NAME = "ccbox_remote";
const DB_VERSION = 1;
const STORE = "kv";

type KvRow = { k: string; v: unknown };

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onerror = () => reject(req.error ?? new Error("IndexedDB open failed"));
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) {
        db.createObjectStore(STORE, { keyPath: "k" });
      }
    };
    req.onsuccess = () => resolve(req.result);
  });
}

export async function kvGet(key: string): Promise<unknown | null> {
  const db = await openDb();
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, "readonly");
      const store = tx.objectStore(STORE);
      const req = store.get(key);
      req.onerror = () => reject(req.error ?? new Error("IndexedDB get failed"));
      req.onsuccess = () => {
        const row = req.result as KvRow | undefined;
        resolve(row ? row.v : null);
      };
    });
  } finally {
    db.close();
  }
}

export async function kvSet(key: string, value: unknown): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.onabort = () => reject(tx.error ?? new Error("IndexedDB set aborted"));
      tx.onerror = () => reject(tx.error ?? new Error("IndexedDB set failed"));
      const store = tx.objectStore(STORE);
      store.put({ k: key, v: value } satisfies KvRow);
      tx.oncomplete = () => resolve();
    });
  } finally {
    db.close();
  }
}

export async function kvDel(key: string): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.onabort = () => reject(tx.error ?? new Error("IndexedDB delete aborted"));
      tx.onerror = () => reject(tx.error ?? new Error("IndexedDB delete failed"));
      const store = tx.objectStore(STORE);
      store.delete(key);
      tx.oncomplete = () => resolve();
    });
  } finally {
    db.close();
  }
}
