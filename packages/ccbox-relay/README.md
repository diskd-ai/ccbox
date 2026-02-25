ccbox Relay Server (Rust)
========================

This is the public relay/data-plane server for `ccbox` remote orchestration.

It implements:
- `GET /ccbox` (WebSocket): CCBox tunnel endpoint
- `GET /client` (WebSocket): browser/mobile client endpoint
- `POST /pair`: device pairing approval
- `GET /health`

Local dev
---------

Run the relay:
```bash
ccbox-relay serve --port 8787 --data-dir ./data
```

Note: `ccbox serve --relay-url` requires `wss://` (TLS). For local dev, run a TLS-terminating proxy in front of `ccbox-relay` with a cert trusted by your machine.

Create a pairing code:
```bash
ccbox-relay pair:create --data-dir ./data --guid <GUID>
```

Note: `ccbox serve --relay ...` now requests pairing codes over authenticated WebSocket (`ccbox/pairing/create`) and prints them automatically.

Run `ccbox serve` against the relay:
```bash
# First, print the ccbox_id (GUID)
ccbox serve --print-identity

# Then connect using that GUID:
ccbox serve --relay-url "wss://<host>/ccbox?guid=<GUID>"
```

Data directory
--------------

`--data-dir` contains JSON files:
- `trusted_devices.json`
- `ccboxes.json`
- `pairings/<guid>.json` (one active pairing record per GUID)
