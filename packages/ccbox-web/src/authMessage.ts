export const AUTH_DOMAIN_SEPARATOR = "ccbox-remote-auth:v1" as const;

export type DeviceKind = "client" | "ccbox";

/**
 * Canonical signing input:
 *   utf8(AUTH_DOMAIN_SEPARATOR) || utf8(device_kind) || utf8(device_id) || nonce_bytes
 */
export function buildAuthMessageV1(
  deviceKind: DeviceKind,
  deviceId: string,
  nonceBytes: Uint8Array,
): Uint8Array<ArrayBuffer> {
  const enc = new TextEncoder();
  const a = enc.encode(AUTH_DOMAIN_SEPARATOR);
  const b = enc.encode(deviceKind);
  const c = enc.encode(deviceId);

  const out = new Uint8Array(a.length + b.length + c.length + nonceBytes.length);
  out.set(a, 0);
  out.set(b, a.length);
  out.set(c, a.length + b.length);
  out.set(nonceBytes, a.length + b.length + c.length);
  return out;
}

