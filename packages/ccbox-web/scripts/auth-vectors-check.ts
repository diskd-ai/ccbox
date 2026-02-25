import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  AUTH_DOMAIN_SEPARATOR,
  buildAuthMessageV1,
  type DeviceKind,
} from "../src/authMessage.ts";

type Vector = {
  readonly name: string;
  readonly device_kind: string;
  readonly device_id: string;
  readonly nonce_b64: string;
  readonly expected_message_b64: string;
};

type VectorsFile = {
  readonly v: number;
  readonly auth_domain_separator: string;
  readonly vectors: readonly Vector[];
};

function parseDeviceKind(value: string): DeviceKind {
  if (value === "client") return "client";
  if (value === "ccbox") return "ccbox";
  throw new Error(`Invalid device_kind in vectors file: ${value}`);
}

const here = resolve(fileURLToPath(import.meta.url), "..");
const vectorsPath = resolve(here, "../../../.agents/docs/REMOTE_AUTH_V1_VECTORS.json");

const text = await readFile(vectorsPath, "utf-8");
const file = JSON.parse(text) as VectorsFile;

if (file.v !== 1) {
  throw new Error(`Unexpected vectors version: ${String(file.v)}`);
}
if (file.auth_domain_separator !== AUTH_DOMAIN_SEPARATOR) {
  throw new Error(
    `auth_domain_separator mismatch: expected ${AUTH_DOMAIN_SEPARATOR}, got ${file.auth_domain_separator}`,
  );
}

for (const vector of file.vectors) {
  const nonceBytes = new Uint8Array(Buffer.from(vector.nonce_b64, "base64"));
  const deviceKind = parseDeviceKind(vector.device_kind);
  const message = buildAuthMessageV1(deviceKind, vector.device_id, nonceBytes);
  const got = Buffer.from(message).toString("base64");
  if (got !== vector.expected_message_b64) {
    throw new Error(`auth message mismatch for vector ${vector.name}`);
  }
}

console.log("auth vectors: OK");
