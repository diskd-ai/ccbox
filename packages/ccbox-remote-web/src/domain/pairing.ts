import type { DeviceIdentity } from "@/domain/types";
import { toPairUrl } from "@/domain/url";

export type PairingResult =
  | { readonly tag: "Ok" }
  | { readonly tag: "Err"; readonly message: string };

interface PairResponseBody {
  readonly ok?: boolean;
  readonly error?: string;
}

function isObject(value: unknown): value is { [key: string]: unknown } {
  return typeof value === "object" && value !== null;
}

function parsePairResponseBody(value: unknown): PairResponseBody {
  if (!isObject(value)) return {};

  const ok = typeof value.ok === "boolean" ? value.ok : undefined;
  const error = typeof value.error === "string" ? value.error : undefined;
  return { ok, error };
}

export async function pairDevice(
  connectInput: string,
  pairingCode: string,
  identity: DeviceIdentity,
): Promise<PairingResult> {
  const code = pairingCode.trim();
  if (!code) {
    return { tag: "Err", message: "Missing pairing code" };
  }

  let pairUrl: string;
  try {
    pairUrl = toPairUrl(connectInput);
  } catch (error) {
    return { tag: "Err", message: String(error) };
  }

  try {
    const response = await fetch(pairUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        pairing_code: code,
        device_id: identity.device_id,
        public_key_b64: identity.public_key_b64,
      }),
    });

    const body = parsePairResponseBody(await response.json());

    if (body.ok === true) {
      return { tag: "Ok" };
    }

    if (body.error) {
      return { tag: "Err", message: body.error };
    }

    return { tag: "Err", message: `Pair failed (HTTP ${response.status})` };
  } catch (error) {
    return { tag: "Err", message: `Pair error: ${String(error)}` };
  }
}
