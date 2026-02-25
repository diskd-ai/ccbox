ccbox Remote Web Client (Minimal)
=================================

Minimal Vite + TypeScript client for early end-to-end testing of the remote protocol.

Current scope (v0):
- Connects to:
  - `wss://<guid>.ccbox.app/client` (relay mode), or
  - `ws://<host:port>/client?guid=<guid>` (local `ccbox serve --no-relay` mode)
- Performs device authentication (`auth/*` handshake)
- Calls `projects.list` and renders the result

Local dev:
```bash
cd packages/ccbox-web
pnpm install
pnpm dev
```

Notes:
- The client generates a device identity (UUID + Ed25519 keypair) and stores it in IndexedDB.
- To authenticate successfully, the relay must have the device public key registered for the generated `device_id`.
  - Pairing is done by POSTing to `/pair` on the same host as the relay WebSocket endpoint.
