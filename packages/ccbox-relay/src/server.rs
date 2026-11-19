use crate::store::StorePaths;
use crate::types::{
    CcboxRegisterPayload, EnvelopeIn, EnvelopeOut, MuxFramePayload, MuxFramePayloadOut,
};
use crate::util::{
    CONTROL_V1_STREAM_ID, REMOTE_PROTOCOL_VERSION, is_allowed_client_origin, now_iso, resolve_guid,
};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub store_paths: StorePaths,
    pub relay: Arc<RelayState>,
    pub rate_limiter: Arc<RateLimiter>,
}

pub struct RelayState {
    ccboxes_by_guid: RwLock<HashMap<String, CcboxConn>>,
    clients_by_session_id: RwLock<HashMap<String, ClientConn>>,
    pending_pairs_by_request_id: RwLock<HashMap<String, oneshot::Sender<Result<(), String>>>>,
}

pub struct RateLimiter {
    buckets: Mutex<HashMap<String, RateBucket>>,
}

struct RateBucket {
    window_start_ms: u128,
    count: u32,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub async fn check(&self, key: String, limit: u32, window_ms: u128) -> bool {
        let now = now_ms();
        let mut buckets = self.buckets.lock().await;

        let allowed = {
            let bucket = buckets.entry(key).or_insert(RateBucket {
                window_start_ms: now,
                count: 0,
            });

            if now.saturating_sub(bucket.window_start_ms) >= window_ms {
                bucket.window_start_ms = now;
                bucket.count = 0;
            }

            bucket.count = bucket.count.saturating_add(1);
            bucket.count <= limit
        };

        if buckets.len() > 10_000 {
            let window = window_ms.saturating_mul(2);
            buckets.retain(|_, b| now.saturating_sub(b.window_start_ms) < window);
        }

        allowed
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayState {
    pub fn new() -> Self {
        Self {
            ccboxes_by_guid: RwLock::new(HashMap::new()),
            clients_by_session_id: RwLock::new(HashMap::new()),
            pending_pairs_by_request_id: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for RelayState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct CcboxConn {
    conn_id: Uuid,
    tx: mpsc::UnboundedSender<Message>,
}

#[derive(Clone)]
struct ClientConn {
    tx: mpsc::UnboundedSender<Message>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionKind {
    Ccbox,
    Client,
}

impl ConnectionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ccbox => "ccbox",
            Self::Client => "client",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GuidQuery {
    pub guid: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PairApproveRequest {
    pub pairing_code: String,
    pub device_id: String,
    pub public_key_b64: String,
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
struct RelayPairApprovePayload {
    request_id: String,
    pairing_code: String,
    device_id: String,
    public_key_b64: String,
    label: Option<String>,
    ip: String,
}

#[derive(Debug, Deserialize)]
struct RelayPairResultPayload {
    request_id: String,
    ok: bool,
    error: Option<String>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/pair", post(pair_approve).options(pair_options))
        .route("/ccbox", get(ws_ccbox))
        .route("/client", get(ws_client))
        .with_state(state)
}

async fn root() -> impl IntoResponse {
    (StatusCode::OK, "ccbox relay server")
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

async fn pair_approve(
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<GuidQuery>,
    State(state): State<AppState>,
    Json(body): Json<PairApproveRequest>,
) -> Response {
    let host_header = headers.get("host").and_then(|h| h.to_str().ok());
    let origin_header = headers.get("origin").and_then(|h| h.to_str().ok());
    let allowed_origin = resolve_allowed_pair_origin(host_header, origin_header);

    let ip = resolve_request_ip(&headers, peer_addr);
    let guid = resolve_guid(host_header, query.guid.as_deref());
    let Some(guid) = guid else {
        let mut res = (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "InvalidGuid" })),
        )
            .into_response();
        if let Some(origin) = allowed_origin.as_deref() {
            apply_pair_cors_headers(&mut res, origin);
        }
        return res;
    };
    let device_id = body.device_id.clone();
    let device_label = body.label.clone();

    if origin_header.is_some() && allowed_origin.is_none() {
        log_event(
            "pair.origin_forbidden",
            serde_json::json!({
                "ip": ip.to_string(),
                "host": host_header.unwrap_or(""),
                "origin": origin_header.unwrap_or(""),
                "guid": guid,
                "device_id": device_id,
            }),
        );
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "ok": false, "error": "OriginNotAllowed" })),
        )
            .into_response();
    }

    if !state
        .rate_limiter
        .check(format!("pair:{ip}"), 20, 60_000)
        .await
    {
        log_event(
            "pair.rate_limited",
            serde_json::json!({
                "ip": ip.to_string(),
                "guid": guid,
                "device_id": device_id,
            }),
        );
        let mut res = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "ok": false, "error": "RateLimited" })),
        )
            .into_response();
        if let Some(origin) = allowed_origin.as_deref() {
            apply_pair_cors_headers(&mut res, origin);
        }
        return res;
    }

    if Uuid::parse_str(body.device_id.trim()).is_err() {
        let mut res = (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "InvalidParams" })),
        )
            .into_response();
        if let Some(origin) = allowed_origin.as_deref() {
            apply_pair_cors_headers(&mut res, origin);
        }
        return res;
    }
    if body.public_key_b64.trim().is_empty() || body.pairing_code.trim().is_empty() {
        let mut res = (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "InvalidParams" })),
        )
            .into_response();
        if let Some(origin) = allowed_origin.as_deref() {
            apply_pair_cors_headers(&mut res, origin);
        }
        return res;
    }

    // Pairing is CCBox-owned; the relay only forwards the request.
    let ccbox = state
        .relay
        .ccboxes_by_guid
        .read()
        .await
        .get(&guid)
        .cloned();
    let Some(ccbox) = ccbox else {
        log_event(
            "pair.ccbox_offline",
            serde_json::json!({
                "ip": ip.to_string(),
                "guid": guid,
                "device_id": device_id,
            }),
        );
        let mut res = (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "CCBoxOffline" })),
        )
            .into_response();
        if let Some(origin) = allowed_origin.as_deref() {
            apply_pair_cors_headers(&mut res, origin);
        }
        return res;
    };

    let request_id = Uuid::new_v4().to_string();
    let (result_tx, result_rx) = oneshot::channel::<Result<(), String>>();
    state
        .relay
        .pending_pairs_by_request_id
        .write()
        .await
        .insert(request_id.clone(), result_tx);

    let approve = EnvelopeOut {
        v: REMOTE_PROTOCOL_VERSION,
        type_: "relay/pair/approve",
        ts: now_iso(),
        payload: RelayPairApprovePayload {
            request_id: request_id.clone(),
            pairing_code: body.pairing_code,
            device_id: body.device_id,
            public_key_b64: body.public_key_b64,
            label: body.label,
            ip: ip.to_string(),
        },
    };
    let Ok(approve_text) = serde_json::to_string(&approve) else {
        let mut res = (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "Error" })),
        )
            .into_response();
        if let Some(origin) = allowed_origin.as_deref() {
            apply_pair_cors_headers(&mut res, origin);
        }
        return res;
    };
    let _ = ccbox.tx.send(Message::Text(approve_text.into()));

    let result = tokio::time::timeout(Duration::from_secs(5), result_rx).await;
    state
        .relay
        .pending_pairs_by_request_id
        .write()
        .await
        .remove(&request_id);

    let mut res = match result {
        Ok(Ok(Ok(()))) => {
            log_event(
                "pair.ok",
                serde_json::json!({
                    "ip": ip.to_string(),
                    "guid": guid,
                    "device_id": device_id,
                    "label": device_label,
                }),
            );
            (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Ok(Ok(Err(code))) => {
            log_event(
                "pair.err",
                serde_json::json!({
                    "ip": ip.to_string(),
                    "guid": guid,
                    "device_id": device_id,
                    "code": code,
                }),
            );
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "ok": false, "error": code })),
            )
                .into_response()
        }
        Ok(Err(_canceled)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "Error" })),
        )
            .into_response(),
        Err(_timeout) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "Error" })),
        )
            .into_response(),
    };
    if let Some(origin) = allowed_origin.as_deref() {
        apply_pair_cors_headers(&mut res, origin);
    }
    res
}

async fn pair_options(headers: HeaderMap) -> Response {
    let host_header = headers.get("host").and_then(|h| h.to_str().ok());
    let origin_header = headers.get("origin").and_then(|h| h.to_str().ok());
    let Some(origin) = resolve_allowed_pair_origin(host_header, origin_header) else {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    };

    let mut res = StatusCode::NO_CONTENT.into_response();
    apply_pair_cors_headers(&mut res, &origin);
    res.headers_mut().insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    res
}

async fn ws_ccbox(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    Query(query): Query<GuidQuery>,
    State(state): State<AppState>,
) -> Response {
    let ip = resolve_request_ip(&headers, peer_addr);
    if !state
        .rate_limiter
        .check(format!("ws_ccbox:{ip}"), 60, 60_000)
        .await
    {
        log_event(
            "ws.rate_limited",
            serde_json::json!({
                "kind": "ccbox",
                "ip": ip.to_string(),
            }),
        );
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }

    let guid = resolve_guid(
        headers.get("host").and_then(|h| h.to_str().ok()),
        query.guid.as_deref(),
    );
    let Some(guid) = guid else {
        log_event(
            "ws.invalid_guid",
            serde_json::json!({
                "kind": "ccbox",
                "ip": ip.to_string(),
                "host": headers.get("host").and_then(|h| h.to_str().ok()).unwrap_or(""),
            }),
        );
        return (StatusCode::BAD_REQUEST, "invalid guid").into_response();
    };

    log_event(
        "ws.upgrade",
        serde_json::json!({
            "kind": "ccbox",
            "ip": ip.to_string(),
            "guid": guid,
        }),
    );
    ws.on_upgrade(move |socket| handle_socket(socket, ConnectionKind::Ccbox, guid, ip, state))
}

async fn ws_client(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    Query(query): Query<GuidQuery>,
    State(state): State<AppState>,
) -> Response {
    let ip = resolve_request_ip(&headers, peer_addr);
    if !state
        .rate_limiter
        .check(format!("ws_client:{ip}"), 60, 60_000)
        .await
    {
        log_event(
            "ws.rate_limited",
            serde_json::json!({
                "kind": "client",
                "ip": ip.to_string(),
            }),
        );
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }

    let host_header = headers.get("host").and_then(|h| h.to_str().ok());
    if should_enforce_origin(host_header) {
        let origin = headers
            .get("origin")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if !is_allowed_client_origin(origin) {
            log_event(
                "ws.origin_forbidden",
                serde_json::json!({
                    "kind": "client",
                    "ip": ip.to_string(),
                    "host": host_header.unwrap_or(""),
                    "origin": origin,
                }),
            );
            return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
        }
    }

    let guid = resolve_guid(host_header, query.guid.as_deref());
    let Some(guid) = guid else {
        log_event(
            "ws.invalid_guid",
            serde_json::json!({
                "kind": "client",
                "ip": ip.to_string(),
                "host": host_header.unwrap_or(""),
            }),
        );
        return (StatusCode::BAD_REQUEST, "invalid guid").into_response();
    };

    log_event(
        "ws.upgrade",
        serde_json::json!({
            "kind": "client",
            "ip": ip.to_string(),
            "guid": guid,
        }),
    );
    ws.on_upgrade(move |socket| handle_socket(socket, ConnectionKind::Client, guid, ip, state))
}

async fn handle_socket(
    socket: WebSocket,
    kind: ConnectionKind,
    guid: String,
    ip: IpAddr,
    state: AppState,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let conn_id = Uuid::new_v4();

    log_event(
        "ws.open",
        serde_json::json!({
            "kind": kind.as_str(),
            "ip": ip.to_string(),
            "guid": guid,
            "conn_id": conn_id.to_string(),
        }),
    );

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // In the stateless relay model, the relay assigns a session id immediately on connect.
    // Authentication happens end-to-end between the client and the CCBox over mux frames.
    let session_id = match kind {
        ConnectionKind::Client => Some(Uuid::new_v4().to_string()),
        ConnectionKind::Ccbox => None,
    };

    match kind {
        ConnectionKind::Client => {
            if let Some(session_id) = session_id.as_deref() {
                state
                    .relay
                    .clients_by_session_id
                    .write()
                    .await
                    .insert(session_id.to_string(), ClientConn { tx: tx.clone() });
                log_event(
                    "client.session",
                    serde_json::json!({
                        "ip": ip.to_string(),
                        "guid": guid,
                        "conn_id": conn_id.to_string(),
                        "session_id": session_id,
                    }),
                );
            }
        }
        ConnectionKind::Ccbox => {
            // Treat the socket as the CCBox tunnel for `guid` as soon as it is connected.
            state.relay.ccboxes_by_guid.write().await.insert(
                guid.clone(),
                CcboxConn {
                    conn_id,
                    tx: tx.clone(),
                },
            );
        }
    }

    let mut registered_ccbox = false;

    while let Some(msg) = ws_receiver.next().await {
        let Ok(msg) = msg else {
            break;
        };

        match msg {
            Message::Ping(bytes) => {
                let _ = tx.send(Message::Pong(bytes));
                continue;
            }
            Message::Close(_) => break,
            _ => {}
        }

        let text = match msg {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            _ => continue,
        };

        match kind {
            ConnectionKind::Ccbox => {
                let env: EnvelopeIn = match serde_json::from_str(&text) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if env.v != REMOTE_PROTOCOL_VERSION {
                    continue;
                }

                if env.type_ == "ccbox/register" {
                    let reg: CcboxRegisterPayload = match serde_json::from_value(env.payload) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if reg.ccbox_id.to_lowercase() != guid {
                        break;
                    }
                    registered_ccbox = true;
                    log_event(
                        "ccbox.register",
                        serde_json::json!({
                            "ip": ip.to_string(),
                            "guid": guid,
                            "conn_id": conn_id.to_string(),
                            "ccbox_id": reg.ccbox_id,
                        }),
                    );
                    continue;
                }

                if env.type_ == "relay/pair/result" {
                    let payload: RelayPairResultPayload = match serde_json::from_value(env.payload) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let request_id = payload.request_id.trim();
                    if request_id.is_empty() {
                        continue;
                    }

                    let result = if payload.ok {
                        Ok(())
                    } else {
                        Err(payload.error.unwrap_or_else(|| "Error".to_string()))
                    };

                    let pending = state
                        .relay
                        .pending_pairs_by_request_id
                        .write()
                        .await
                        .remove(request_id);
                    if let Some(tx) = pending {
                        let _ = tx.send(result);
                    }
                    continue;
                }

                if env.type_ != "mux/frame" {
                    continue;
                }

                let mux: MuxFramePayload = match serde_json::from_value(env.payload) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if mux.stream_id != CONTROL_V1_STREAM_ID {
                    continue;
                }

                let Some(client) = state
                    .relay
                    .clients_by_session_id
                    .read()
                    .await
                    .get(&mux.session_id)
                    .cloned()
                else {
                    continue;
                };

                let bytes = match base64::engine::general_purpose::STANDARD
                    .decode(mux.payload_b64.trim())
                {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };
                let text = String::from_utf8_lossy(&bytes).to_string();
                let _ = client.tx.send(Message::Text(text.into()));
            }
            ConnectionKind::Client => {
                let Some(session_id) = session_id.as_deref() else {
                    continue;
                };

                let orch = state.relay.ccboxes_by_guid.read().await.get(&guid).cloned();
                let Some(orch) = orch else {
                    // Preserve the existing behavior for rpc requests: respond with CCBoxOffline.
                    let env: EnvelopeIn = match serde_json::from_str(&text) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if env.v != REMOTE_PROTOCOL_VERSION {
                        continue;
                    }

                    if env.type_ == "rpc/request"
                        && let Some(id) = env
                            .payload
                            .as_object()
                            .and_then(|obj| obj.get("id"))
                            .and_then(|v| v.as_str())
                    {
                        let method = env
                            .payload
                            .as_object()
                            .and_then(|obj| obj.get("method"))
                            .and_then(JsonValue::as_str)
                            .unwrap_or("");
                        log_event(
                            "rpc.ccbox_offline",
                            serde_json::json!({
                                "ip": ip.to_string(),
                                "guid": guid,
                                "session_id": session_id,
                                "id": id,
                                "method": method,
                            }),
                        );
                        let response = serde_json::json!({
                            "v": REMOTE_PROTOCOL_VERSION,
                            "type": "rpc/response",
                            "ts": now_iso(),
                            "payload": {
                                "id": id,
                                "ok": false,
                                "error": { "code": "CCBoxOffline", "message": "ccbox offline" }
                            }
                        });
                        let _ = tx.send(Message::Text(response.to_string().into()));
                    }
                    continue;
                };

                let inner_bytes = text.as_bytes();
                let frame = EnvelopeOut {
                    v: REMOTE_PROTOCOL_VERSION,
                    type_: "mux/frame",
                    ts: now_iso(),
                    payload: MuxFramePayloadOut {
                        session_id: session_id.to_string(),
                        stream_id: CONTROL_V1_STREAM_ID,
                        payload_b64: base64::engine::general_purpose::STANDARD
                            .encode(inner_bytes),
                    },
                };
                let Ok(frame_text) = serde_json::to_string(&frame) else {
                    continue;
                };
                let _ = orch.tx.send(Message::Text(frame_text.into()));
            }
        }
    }

    if kind == ConnectionKind::Client
        && let Some(session_id) = session_id
    {
        state
            .relay
            .clients_by_session_id
            .write()
            .await
            .remove(&session_id);
    }

    if kind == ConnectionKind::Ccbox {
        let mut map = state.relay.ccboxes_by_guid.write().await;
        if map.get(&guid).is_some_and(|conn| conn.conn_id == conn_id) {
            map.remove(&guid);
        }
    }

    log_event(
        "ws.close",
        serde_json::json!({
            "kind": kind.as_str(),
            "ip": ip.to_string(),
            "guid": guid,
            "conn_id": conn_id.to_string(),
            "registered": registered_ccbox,
        }),
    );

    drop(tx);
    if tokio::time::timeout(Duration::from_millis(200), &mut send_task)
        .await
        .is_err()
    {
        send_task.abort();
    }
}

fn resolve_request_ip(headers: &HeaderMap, peer_addr: SocketAddr) -> IpAddr {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = xff.split(',').next()
        && let Ok(ip) = first.trim().parse::<IpAddr>()
    {
        return ip;
    }
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok())
        && let Ok(ip) = xri.trim().parse::<IpAddr>()
    {
        return ip;
    }
    peer_addr.ip()
}

fn should_enforce_origin(host_header: Option<&str>) -> bool {
    let host = host_header.unwrap_or("");
    let host_no_port = host.split(':').next().unwrap_or("");
    host_no_port == "ccbox.app" || host_no_port.ends_with(".ccbox.app")
}

fn resolve_allowed_pair_origin(
    host_header: Option<&str>,
    origin_header: Option<&str>,
) -> Option<String> {
    let origin = origin_header?.trim();
    if origin.is_empty() {
        return None;
    }
    if origin.eq_ignore_ascii_case("null") {
        return None;
    }

    if should_enforce_origin(host_header) {
        if is_allowed_client_origin(origin) {
            return Some(origin.to_string());
        }
        return None;
    }

    let origin_lc = origin.to_ascii_lowercase();
    if origin_lc.starts_with("http://") || origin_lc.starts_with("https://") {
        return Some(origin.to_string());
    }
    None
}

fn apply_pair_cors_headers(res: &mut Response, origin: &str) {
    let Ok(origin) = HeaderValue::from_str(origin) else {
        return;
    };
    res.headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    res.headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Origin"));
}

fn log_event(event: &'static str, fields: JsonValue) {
    let line = serde_json::json!({
        "ts": now_iso(),
        "event": event,
        "fields": fields,
    });
    use std::io::Write as _;
    let mut out = std::io::stderr().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn now_ms() -> u128 {
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis(),
        Err(_) => 0,
    }
}

pub async fn run_http_server(port: u16, store_paths: StorePaths) -> Result<(), String> {
    run_http_server_on(
        std::net::SocketAddr::from(([0, 0, 0, 0], port)),
        store_paths,
    )
    .await
}

pub async fn run_http_server_on(
    addr: std::net::SocketAddr,
    store_paths: StorePaths,
) -> Result<(), String> {
    let state = AppState {
        store_paths,
        relay: Arc::new(RelayState::new()),
        rate_limiter: Arc::new(RateLimiter::new()),
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| error.to_string())?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|error| error.to_string())
}
