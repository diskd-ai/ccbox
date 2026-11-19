use crate::server::{AppState, RateLimiter, RelayState, build_router};
use crate::store::make_store_paths;
use crate::util::{CONTROL_V1_STREAM_ID, REMOTE_PROTOCOL_VERSION, now_iso};
use base64::Engine as _;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

#[tokio::test]
async fn pair_post_is_forwarded_to_ccbox_and_proxies_ok() {
    let dir = tempdir().expect("tempdir");
    let store_paths = make_store_paths(dir.path());

    let state = AppState {
        store_paths,
        relay: Arc::new(RelayState::new()),
        rate_limiter: Arc::new(RateLimiter::new()),
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    let guid = Uuid::new_v4().to_string();

    // CCBox stub: register without an auth handshake, then approve pairing requests.
    let ccbox_url = format!("ws://{addr}/ccbox?guid={guid}");
    let (mut ccbox_ws, _) = connect_async(ccbox_url).await.expect("ccbox connect");
    let register = serde_json::json!({
        "v": REMOTE_PROTOCOL_VERSION,
        "type": "ccbox/register",
        "ts": now_iso(),
        "payload": {
            "ccbox_id": guid,
            "label": null,
            "capabilities": ["control-v1"],
            "version": "test",
        }
    });
    ccbox_ws
        .send(WsMessage::Text(register.to_string().into()))
        .await
        .expect("send ccbox/register");

    let ccbox_task = tokio::spawn(async move {
        while let Some(msg) = ccbox_ws.next().await {
            let Ok(msg) = msg else { break };
            let WsMessage::Text(text) = msg else { continue };

            let env: Value = match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if env.get("type").and_then(Value::as_str) != Some("relay/pair/approve") {
                continue;
            }

            let request_id = env
                .get("payload")
                .and_then(|p| p.get("request_id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if request_id.is_empty() {
                continue;
            }

            let response = serde_json::json!({
                "v": REMOTE_PROTOCOL_VERSION,
                "type": "relay/pair/result",
                "ts": now_iso(),
                "payload": { "request_id": request_id, "ok": true }
            });
            let _ = ccbox_ws
                .send(WsMessage::Text(response.to_string().into()))
                .await;
        }
    });

    let device_id = Uuid::new_v4().to_string();
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
    let url = format!("http://{addr}/pair?guid={guid}");
    let payload = serde_json::json!({
        "pairing_code": "AAAAAAAAAA",
        "device_id": device_id,
        "public_key_b64": pk_b64,
        "label": null,
    });

    let mut res = tokio::task::spawn_blocking(move || {
        ureq::post(&url)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(payload)
    })
    .await
    .expect("spawn_blocking")
    .expect("http response");

    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.body_mut().read_json().expect("json response");
    assert_eq!(body.get("ok").and_then(Value::as_bool), Some(true));

    ccbox_task.abort();
    server_task.abort();
}

#[tokio::test]
async fn pair_post_returns_ccbox_offline_when_no_ccbox_connected() {
    let dir = tempdir().expect("tempdir");
    let store_paths = make_store_paths(dir.path());

    let state = AppState {
        store_paths,
        relay: Arc::new(RelayState::new()),
        rate_limiter: Arc::new(RateLimiter::new()),
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    let guid = Uuid::new_v4().to_string();
    let device_id = Uuid::new_v4().to_string();
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
    let url = format!("http://{addr}/pair?guid={guid}");
    let payload = serde_json::json!({
        "pairing_code": "AAAAAAAAAA",
        "device_id": device_id,
        "public_key_b64": pk_b64,
        "label": null,
    });

    let res = tokio::task::spawn_blocking(move || {
        ureq::post(&url)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(payload)
    })
    .await
    .expect("spawn_blocking");

    let mut res = res.expect("http response");
    assert_eq!(res.status().as_u16(), 400);
    let body: Value = res.body_mut().read_json().expect("json response");
    assert_eq!(body.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(
        body.get("error").and_then(Value::as_str),
        Some("CCBoxOffline")
    );

    server_task.abort();
}

#[tokio::test]
async fn client_messages_round_trip_via_mux_when_ccbox_connected() {
    let dir = tempdir().expect("tempdir");
    let store_paths = make_store_paths(dir.path());

    let state = AppState {
        store_paths,
        relay: Arc::new(RelayState::new()),
        rate_limiter: Arc::new(RateLimiter::new()),
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    let guid = Uuid::new_v4().to_string();

    // CCBox stub: for any control-v1 mux frame from a client session, return "ok".
    let ccbox_url = format!("ws://{addr}/ccbox?guid={guid}");
    let (mut ccbox_ws, _) = connect_async(ccbox_url).await.expect("ccbox connect");
    let register = serde_json::json!({
        "v": REMOTE_PROTOCOL_VERSION,
        "type": "ccbox/register",
        "ts": now_iso(),
        "payload": {
            "ccbox_id": guid,
            "label": null,
            "capabilities": ["control-v1"],
            "version": "test",
        }
    });
    ccbox_ws
        .send(WsMessage::Text(register.to_string().into()))
        .await
        .expect("send ccbox/register");

    let ccbox_task = tokio::spawn(async move {
        while let Some(msg) = ccbox_ws.next().await {
            let Ok(msg) = msg else { break };
            let WsMessage::Text(text) = msg else { continue };
            let env: Value = match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if env.get("type").and_then(Value::as_str) != Some("mux/frame") {
                continue;
            }
            let payload = env.get("payload");
            let Some(session_id) = payload
                .and_then(|p| p.get("session_id"))
                .and_then(Value::as_str)
                .map(|s| s.to_string())
            else {
                continue;
            };

            let response_text = "ok";
            let response = serde_json::json!({
                "v": REMOTE_PROTOCOL_VERSION,
                "type": "mux/frame",
                "ts": now_iso(),
                "payload": {
                    "session_id": session_id,
                    "stream_id": CONTROL_V1_STREAM_ID,
                    "payload_b64": base64::engine::general_purpose::STANDARD.encode(response_text.as_bytes()),
                }
            });
            let _ = ccbox_ws
                .send(WsMessage::Text(response.to_string().into()))
                .await;
        }
    });

    let client_url = format!("ws://{addr}/client?guid={guid}");
    let (mut client_ws, _) = connect_async(client_url).await.expect("client connect");
    client_ws
        .send(WsMessage::Text("hi".to_string().into()))
        .await
        .expect("send client text");

    let got = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let Some(msg) = client_ws.next().await else {
                return None;
            };
            let msg = msg.ok()?;
            if let WsMessage::Text(text) = msg {
                return Some(text.to_string());
            }
        }
    })
    .await
    .expect("response within 1s")
    .expect("ws text");

    assert_eq!(got, "ok");

    ccbox_task.abort();
    server_task.abort();
}
