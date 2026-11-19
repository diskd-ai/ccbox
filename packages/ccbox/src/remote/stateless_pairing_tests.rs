use super::*;
use ccbox_relay::server::{AppState as RelayAppState, RateLimiter, RelayState, build_router};
use ccbox_relay::store::make_store_paths;
use ccbox_relay::types::TrustedDevicesFile;
use ed25519_dalek::SigningKey;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

fn random_signing_key() -> SigningKey {
    let seed = rand32();
    SigningKey::from_bytes(&seed)
}

fn rand32() -> [u8; 32] {
    use rand_core::RngCore as _;
    let mut out = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut out);
    out
}

async fn ws_authenticate_client(
    ws: &mut WsStream,
    device_id: &str,
    signing_key: &SigningKey,
) -> Result<(), String> {
    let hello = serde_json::json!({
        "v": REMOTE_PROTOCOL_VERSION,
        "type": "auth/hello",
        "ts": now_iso(),
        "payload": { "device_id": device_id, "device_kind": "client" },
    });
    ws.send(WsMessage::Text(hello.to_string().into()))
        .await
        .expect("send auth/hello");

    let nonce_b64 = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let Some(msg) = ws.next().await else {
                return Err("socket closed before auth/challenge".to_string());
            };
            let msg = msg.map_err(|e| e.to_string())?;
            if let WsMessage::Text(text) = msg {
                let env: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                match env.get("type").and_then(Value::as_str) {
                    Some("auth/challenge") => {
                        let payload = env.get("payload").ok_or("missing payload")?;
                        let nonce_b64 = payload
                            .get("nonce_b64")
                            .and_then(Value::as_str)
                            .ok_or("missing nonce_b64")?;
                        return Ok(nonce_b64.to_string());
                    }
                    Some("auth/err") => {
                        let code = env
                            .get("payload")
                            .and_then(|p| p.get("code"))
                            .and_then(Value::as_str)
                            .unwrap_or("Error");
                        return Err(code.to_string());
                    }
                    _ => {}
                }
            }
        }
    })
    .await
    .map_err(|_| "timeout waiting for auth/challenge".to_string())??;

    let nonce_bytes = base64::engine::general_purpose::STANDARD
        .decode(nonce_b64)
        .map_err(|e| e.to_string())?;
    let message = build_auth_message("client", device_id, &nonce_bytes);
    let signature = signing_key.sign(&message).to_bytes();
    let signature_b64 = base64::engine::general_purpose::STANDARD.encode(signature);

    let response = serde_json::json!({
        "v": REMOTE_PROTOCOL_VERSION,
        "type": "auth/response",
        "ts": now_iso(),
        "payload": { "signature_b64": signature_b64 },
    });
    ws.send(WsMessage::Text(response.to_string().into()))
        .await
        .expect("send auth/response");

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let Some(msg) = ws.next().await else {
                return Err("socket closed before auth result".to_string());
            };
            let msg = msg.map_err(|e| e.to_string())?;
            if let WsMessage::Text(text) = msg {
                let env: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                match env.get("type").and_then(Value::as_str) {
                    Some("auth/ok") => return Ok(()),
                    Some("auth/err") => {
                        let code = env
                            .get("payload")
                            .and_then(|p| p.get("code"))
                            .and_then(Value::as_str)
                            .unwrap_or("Error");
                        return Err(code.to_string());
                    }
                    _ => {}
                }
            }
        }
    })
    .await
    .map_err(|_| "timeout waiting for auth result".to_string())?
}

async fn ws_rpc(
    ws: &mut WsStream,
    method: &str,
    params: Value,
) -> Result<Value, (String, String)> {
    let id = Uuid::new_v4().to_string();
    let request = serde_json::json!({
        "v": REMOTE_PROTOCOL_VERSION,
        "type": "rpc/request",
        "ts": now_iso(),
        "payload": { "id": id, "method": method, "params": params }
    });
    ws.send(WsMessage::Text(request.to_string().into()))
        .await
        .expect("send rpc/request");

    let response = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let Some(msg) = ws.next().await else {
                return None;
            };
            let msg = msg.ok()?;
            let WsMessage::Text(text) = msg else { continue };
            let env: Value = serde_json::from_str(&text).ok()?;
            if env.get("type").and_then(Value::as_str) != Some("rpc/response") {
                continue;
            }
            let payload = env.get("payload")?;
            if payload.get("id").and_then(Value::as_str) != Some(&id) {
                continue;
            }
            return Some(payload.clone());
        }
    })
    .await
    .expect("rpc/response within 2s")
    .expect("rpc/response payload");

    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    } else {
        let err = response.get("error").and_then(Value::as_object);
        let code = err
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("Error")
            .to_string();
        let message = err
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Err((code, message))
    }
}

async fn start_relay_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let dir = tempdir().expect("relay tempdir");
    let store_paths = make_store_paths(dir.path());

    let state = RelayAppState {
        store_paths,
        relay: Arc::new(RelayState::new()),
        rate_limiter: Arc::new(RateLimiter::new()),
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind relay");
    let addr = listener.local_addr().expect("relay local addr");

    let task = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    (addr, task)
}

fn spawn_ccbox_for_test(
    relay_addr: SocketAddr,
    guid: Uuid,
    state_dir: std::path::PathBuf,
    sessions_dir: std::path::PathBuf,
    pairing_code_override: Option<String>,
) -> (tokio::sync::watch::Sender<bool>, tokio::task::JoinHandle<Result<(), ServeError>>) {
    let opts = ServeOptions {
        label: None,
        relay_domain: "ccbox.app".to_string(),
        relay_url: None,
        relay_base_url: None,
        pairing_code: pairing_code_override,
        enable_shell: false,
        print_identity: false,
        no_relay: false,
        listen_addr: None,
    };

    let url = Url::parse(&format!("ws://{relay_addr}/ccbox?guid={guid}")).expect("ccbox ws url");
    let guid_text = guid.to_string();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let task = tokio::task::spawn_local(async move {
        let mut control = control::ControlPlane::new(sessions_dir).expect("control plane");
        connect_and_run_once(
            &url,
            &opts,
            &state_dir,
            &guid_text,
            &mut control,
            shutdown_rx,
        )
        .await
    });

    (shutdown_tx, task)
}

#[tokio::test(flavor = "current_thread")]
async fn unpaired_device_gets_device_unknown() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let (relay_addr, relay_task) = start_relay_server().await;

            let guid = Uuid::new_v4();
            let state_dir = tempdir().expect("ccbox state dir");
            let sessions_dir = tempdir().expect("ccbox sessions dir");
            let (shutdown_tx, ccbox_task) = spawn_ccbox_for_test(
                relay_addr,
                guid,
                state_dir.path().to_path_buf(),
                sessions_dir.path().to_path_buf(),
                Some("AAAAAAAAAA".to_string()),
            );

            let device_id = Uuid::new_v4().to_string();
            let signing_key = random_signing_key();
            let client_url = format!("ws://{relay_addr}/client?guid={guid}");
            let (mut client_ws, _) = connect_async(client_url).await.expect("client connect");

            let err = ws_authenticate_client(&mut client_ws, &device_id, &signing_key)
                .await
                .expect_err("expected auth failure");
            assert_eq!(err, "DeviceUnknown");

            let _ = shutdown_tx.send(true);
            ccbox_task.abort();
            relay_task.abort();
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn pairing_allows_auth_and_rpc_and_persists_trust() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let (relay_addr, relay_task) = start_relay_server().await;

            let guid = Uuid::new_v4();
            let state_dir = tempdir().expect("ccbox state dir");
            let sessions_dir = tempdir().expect("ccbox sessions dir");
            let pairing_code = "AAAAAAAAAA".to_string();
            let (shutdown_tx, ccbox_task) = spawn_ccbox_for_test(
                relay_addr,
                guid,
                state_dir.path().to_path_buf(),
                sessions_dir.path().to_path_buf(),
                Some(pairing_code.clone()),
            );

            let signing_key = random_signing_key();
            let device_id = Uuid::new_v4().to_string();
            let public_key_b64 = base64::engine::general_purpose::STANDARD
                .encode(signing_key.verifying_key().to_bytes());

            let pair_url = format!("http://{relay_addr}/pair?guid={guid}");
            let mut pair_res = tokio::task::spawn_blocking({
                let payload = serde_json::json!({
                    "pairing_code": pairing_code,
                    "device_id": device_id,
                    "public_key_b64": public_key_b64,
                    "label": null,
                });
                move || {
                    ureq::post(&pair_url)
                        .config()
                        .http_status_as_error(false)
                        .build()
                        .send_json(payload)
                }
            })
            .await
            .expect("spawn_blocking")
            .expect("pair response");

            assert_eq!(pair_res.status().as_u16(), 200);
            let pair_body: Value = pair_res.body_mut().read_json().expect("pair json");
            assert_eq!(pair_body.get("ok").and_then(Value::as_bool), Some(true));

            // Auth + RPC.
            let client_url = format!("ws://{relay_addr}/client?guid={guid}");
            let (mut client_ws, _) = connect_async(client_url).await.expect("client connect");
            ws_authenticate_client(&mut client_ws, &device_id, &signing_key)
                .await
                .expect("auth ok");

            let info = ws_rpc(&mut client_ws, "ccbox.getInfo", serde_json::json!({}))
                .await
                .expect("ccbox.getInfo ok");
            assert!(info.is_object(), "expected object result");

            let sessions_all = ws_rpc(&mut client_ws, "sessions.listAll", serde_json::json!({}))
                .await
                .expect("sessions.listAll ok");
            assert!(
                sessions_all
                    .get("sessions")
                    .and_then(Value::as_array)
                    .is_some(),
                "expected sessions array"
            );

            // Trusted device persisted on CCBox side.
            let trusted_path = state_dir.path().join("remote").join("trusted_devices.json");
            let trusted_text = std::fs::read_to_string(&trusted_path).expect("trusted_devices.json");
            let trusted: TrustedDevicesFile =
                serde_json::from_str(&trusted_text).expect("trusted json");
            assert!(
                trusted
                    .trusted_devices
                    .iter()
                    .any(|d| d.device_id == device_id),
                "expected device_id to be persisted"
            );

            let _ = shutdown_tx.send(true);
            ccbox_task.abort();
            relay_task.abort();
        })
        .await;
}
