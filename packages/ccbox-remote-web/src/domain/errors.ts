export interface RpcRejection {
  readonly code: string;
  readonly message: string;
}

export function isRpcRejection(value: unknown): value is RpcRejection {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as { code?: unknown; message?: unknown };
  return typeof candidate.code === "string" && typeof candidate.message === "string";
}

export function formatError(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message || String(reason);
  if (isRpcRejection(reason)) return `${reason.code}: ${reason.message}`;

  try {
    const json = JSON.stringify(reason);
    if (typeof json === "string") return json;
  } catch {
    // fall through
  }

  return String(reason);
}

