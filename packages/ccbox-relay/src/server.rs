use crate::pairing::ensure_pairing_record;
use crate::store::{
    StorePaths, delete_pairing, load_ccboxes, load_pairing, load_trusted_devices, save_ccboxes,
    save_pairing, save_trusted_devices,
};
use crate::types::{
    AuthChallengePayload, AuthErrPayload, AuthHelloPayload, AuthOkPayload, AuthResponsePayload,
    CcboxDevice, CcboxPairingCreatePayload, CcboxPairingErrPayload, CcboxPairingOkPayload,
    CcboxRegisterPayload, EnvelopeIn, EnvelopeOut, MuxFramePayload, MuxFramePayloadOut,
    PairingRecord, TrustedDevice,
};
use crate::util::{
    CONTROL_V1_STREAM_ID, REMOTE_PROTOCOL_VERSION, build_auth_message, is_allowed_client_origin,
    now_iso, random_nonce32, resolve_guid,
};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::{Mutex, RwLock, mpsc};
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

#[derive(Debug)]
enum AuthState {
    AwaitHello,
    AwaitResponse {
        device_id: String,
        device_kind: ConnectionKind,
        nonce: [u8; 32],
        expires_at_ms: u128,
    },
    Authenticated {},
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

#[derive(Debug, Error)]
enum AuthError {
    #[error("{0}")]
    Code(&'static str),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
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

    let store_paths = state.store_paths.clone();
    let guid_for_blocking = guid.clone();
    let result = tokio::task::spawn_blocking(move || {
        pair_approve_blocking(&store_paths, &guid_for_blocking, body)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|res| res.map_err(|error| error.to_string()));

    let mut res = match result {
        Ok(()) => {
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
        Err(code) => {
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

fn pair_approve_blocking(
    paths: &StorePaths,
    guid: &str,
    body: PairApproveRequest,
) -> Result<(), String> {
    if Uuid::parse_str(body.device_id.trim()).is_err() {
        return Err("InvalidParams".to_string());
    }
    if body.public_key_b64.trim().is_empty() || body.pairing_code.trim().is_empty() {
        return Err("InvalidParams".to_string());
    }

    let record = load_pairing(paths, guid).map_err(|e| e.to_string())?;
    let Some(record) = record else {
        return Err("PairingExpired".to_string());
    };

    let expires_at = OffsetDateTime::parse(&record.expires_at, &Rfc3339)
        .map_err(|_| "PairingExpired".to_string())?;
    if expires_at < OffsetDateTime::now_utc() {
        return Err("PairingExpired".to_string());
    }
    if record.attempts_remaining == 0 {
        return Err("PairingLocked".to_string());
    }

    if record.code_base32 != body.pairing_code {
        let next = PairingRecord {
            attempts_remaining: record.attempts_remaining.saturating_sub(1),
            ..record
        };
        save_pairing(paths, guid, &next).map_err(|e| e.to_string())?;
        return Err("PairingInvalid".to_string());
    }

    let mut trusted = load_trusted_devices(paths).map_err(|e| e.to_string())?;
    trusted
        .trusted_devices
        .retain(|d| d.device_id != body.device_id);
    trusted.trusted_devices.push(TrustedDevice {
        device_id: body.device_id,
        public_key_b64: body.public_key_b64,
        created_at: now_iso(),
        last_seen_at: None,
        revoked: false,
        label: body.label,
    });
    save_trusted_devices(paths, &trusted).map_err(|e| e.to_string())?;
    delete_pairing(paths, guid).map_err(|e| e.to_string())?;
    Ok(())
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

    let mut auth_state = AuthState::AwaitHello;

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut session_id: Option<String> = None;
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

        let env: EnvelopeIn = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if env.v != REMOTE_PROTOCOL_VERSION {
            continue;
        }

        match &auth_state {
            AuthState::AwaitHello => {
                if env.type_ != "auth/hello" {
                    continue;
                }
                let hello: AuthHelloPayload = match serde_json::from_value(env.payload) {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                if hello.device_kind != kind.as_str() {
                    log_event(
                        "auth.err",
                        serde_json::json!({
                            "kind": kind.as_str(),
                            "ip": ip.to_string(),
                            "guid": guid,
                            "conn_id": conn_id.to_string(),
                            "code": "DeviceKindMismatch",
                        }),
                    );
                    send_auth_err(&tx, "DeviceKindMismatch");
                    break;
                }
                if Uuid::parse_str(&hello.device_id).is_err() {
                    log_event(
                        "auth.err",
                        serde_json::json!({
                            "kind": kind.as_str(),
                            "ip": ip.to_string(),
                            "guid": guid,
                            "conn_id": conn_id.to_string(),
                            "code": "InvalidDeviceId",
                        }),
                    );
                    send_auth_err(&tx, "InvalidDeviceId");
                    break;
                }
                if kind == ConnectionKind::Ccbox && hello.device_id.to_lowercase() != guid {
                    log_event(
                        "auth.err",
                        serde_json::json!({
                            "kind": kind.as_str(),
                            "ip": ip.to_string(),
                            "guid": guid,
                            "conn_id": conn_id.to_string(),
                            "device_id": hello.device_id,
                            "code": "GuidMismatch",
                        }),
                    );
                    send_auth_err(&tx, "GuidMismatch");
                    break;
                }

                log_event(
                    "auth.hello",
                    serde_json::json!({
                        "kind": kind.as_str(),
                        "ip": ip.to_string(),
                        "guid": guid,
                        "conn_id": conn_id.to_string(),
                        "device_id": hello.device_id,
                    }),
                );

                let nonce = random_nonce32();
                let expires_at_ms = now_ms() + 10_000;
                auth_state = AuthState::AwaitResponse {
                    device_id: hello.device_id,
                    device_kind: kind,
                    nonce,
                    expires_at_ms,
                };
                let payload = AuthChallengePayload {
                    nonce_b64: base64::engine::general_purpose::STANDARD.encode(nonce),
                    expires_in_ms: 10_000,
                };
                send_envelope(&tx, "auth/challenge", payload);
            }
            AuthState::AwaitResponse {
                device_id,
                device_kind,
                nonce,
                expires_at_ms,
            } => {
                let device_id = device_id.clone();
                let device_kind = *device_kind;
                let nonce = *nonce;
                let expires_at_ms = *expires_at_ms;

                if env.type_ != "auth/response" {
                    continue;
                }

                if now_ms() > expires_at_ms {
                    log_event(
                        "auth.err",
                        serde_json::json!({
                            "kind": kind.as_str(),
                            "ip": ip.to_string(),
                            "guid": guid,
                            "conn_id": conn_id.to_string(),
                            "device_id": device_id,
                            "code": "ChallengeExpired",
                        }),
                    );
                    send_auth_err(&tx, "ChallengeExpired");
                    break;
                }

                let response: AuthResponsePayload = match serde_json::from_value(env.payload) {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                let signature_bytes = match base64::engine::general_purpose::STANDARD
                    .decode(response.signature_b64.trim())
                {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        log_event(
                            "auth.err",
                            serde_json::json!({
                                "kind": kind.as_str(),
                                "ip": ip.to_string(),
                                "guid": guid,
                                "conn_id": conn_id.to_string(),
                                "device_id": device_id,
                                "code": "BadSignature",
                            }),
                        );
                        send_auth_err(&tx, "BadSignature");
                        break;
                    }
                };
                let signature = match Signature::from_slice(&signature_bytes) {
                    Ok(sig) => sig,
                    Err(_) => {
                        log_event(
                            "auth.err",
                            serde_json::json!({
                                "kind": kind.as_str(),
                                "ip": ip.to_string(),
                                "guid": guid,
                                "conn_id": conn_id.to_string(),
                                "device_id": device_id,
                                "code": "BadSignature",
                            }),
                        );
                        send_auth_err(&tx, "BadSignature");
                        break;
                    }
                };

                let auth_message = build_auth_message(device_kind.as_str(), &device_id, &nonce);
                let verify = verify_device_signature(
                    &state.store_paths,
                    device_kind,
                    &device_id,
                    response.public_key_b64.as_deref(),
                    &auth_message,
                    &signature,
                )
                .await;

                match verify {
                    Ok(()) => {
                        auth_state = AuthState::Authenticated {};
                        send_envelope(
                            &tx,
                            "auth/ok",
                            AuthOkPayload {
                                device_id: device_id.clone(),
                            },
                        );
                        log_event(
                            "auth.ok",
                            serde_json::json!({
                                "kind": kind.as_str(),
                                "ip": ip.to_string(),
                                "guid": guid,
                                "conn_id": conn_id.to_string(),
                                "device_id": device_id,
                            }),
                        );

                        if kind == ConnectionKind::Client {
                            let sid = Uuid::new_v4().to_string();
                            session_id = Some(sid.clone());
                            state
                                .relay
                                .clients_by_session_id
                                .write()
                                .await
                                .insert(sid.clone(), ClientConn { tx: tx.clone() });
                            log_event(
                                "client.session",
                                serde_json::json!({
                                    "ip": ip.to_string(),
                                    "guid": guid,
                                    "conn_id": conn_id.to_string(),
                                    "session_id": sid,
                                }),
                            );
                        }
                    }
                    Err(AuthError::Code(code)) => {
                        log_event(
                            "auth.err",
                            serde_json::json!({
                                "kind": kind.as_str(),
                                "ip": ip.to_string(),
                                "guid": guid,
                                "conn_id": conn_id.to_string(),
                                "device_id": device_id,
                                "code": code,
                            }),
                        );
                        send_auth_err(&tx, code);
                        break;
                    }
                    Err(AuthError::Io(_)) => {
                        log_event(
                            "auth.err",
                            serde_json::json!({
                                "kind": kind.as_str(),
                                "ip": ip.to_string(),
                                "guid": guid,
                                "conn_id": conn_id.to_string(),
                                "device_id": device_id,
                                "code": "Error",
                            }),
                        );
                        send_auth_err(&tx, "Error");
                        break;
                    }
                }
            }
            AuthState::Authenticated { .. } => {
                if kind == ConnectionKind::Ccbox {
                    if env.type_ == "ccbox/pairing/create" {
                        let req: CcboxPairingCreatePayload = serde_json::from_value(env.payload)
                            .unwrap_or(CcboxPairingCreatePayload { ttl_seconds: None });
                        let ttl_seconds = req.ttl_seconds.unwrap_or(120).clamp(10, 3600);
                        let store_paths = state.store_paths.clone();
                        let guid_for_blocking = guid.clone();

                        let result = tokio::task::spawn_blocking(move || {
                            ensure_pairing_record(
                                &store_paths,
                                &guid_for_blocking,
                                ttl_seconds as i64,
                                5,
                            )
                            .map_err(|error| error.to_string())
                        })
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|value| value);

                        match result {
                            Ok(pairing) => {
                                log_event(
                                    "pair.create.ok",
                                    serde_json::json!({
                                        "ip": ip.to_string(),
                                        "guid": guid,
                                        "conn_id": conn_id.to_string(),
                                        "reused": pairing.reused,
                                        "expires_at": pairing.record.expires_at,
                                    }),
                                );
                                send_envelope(
                                    &tx,
                                    "ccbox/pairing/ok",
                                    CcboxPairingOkPayload {
                                        pairing_code: pairing.record.code_base32,
                                        expires_at: pairing.record.expires_at,
                                        attempts_remaining: pairing.record.attempts_remaining,
                                        reused: pairing.reused,
                                    },
                                );
                            }
                            Err(code) => {
                                log_event(
                                    "pair.create.err",
                                    serde_json::json!({
                                        "ip": ip.to_string(),
                                        "guid": guid,
                                        "conn_id": conn_id.to_string(),
                                        "code": code,
                                    }),
                                );
                                send_envelope(
                                    &tx,
                                    "ccbox/pairing/err",
                                    CcboxPairingErrPayload { code },
                                );
                            }
                        }
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
                        state.relay.ccboxes_by_guid.write().await.insert(
                            guid.clone(),
                            CcboxConn {
                                conn_id,
                                tx: tx.clone(),
                            },
                        );
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

                    if env.type_ == "mux/frame" {
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
                        continue;
                    }

                    continue;
                }

                // client
                let Some(session_id) = session_id.clone() else {
                    continue;
                };

                let orch = state.relay.ccboxes_by_guid.read().await.get(&guid).cloned();
                let Some(orch) = orch else {
                    if env.type_ == "rpc/request" {
                        if let Some(id) = env
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
                    }
                    continue;
                };

                if env.type_ == "rpc/request" {
                    let id = env
                        .payload
                        .as_object()
                        .and_then(|obj| obj.get("id"))
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    let method = env
                        .payload
                        .as_object()
                        .and_then(|obj| obj.get("method"))
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    if !id.is_empty() {
                        log_event(
                            "rpc.forward",
                            serde_json::json!({
                                "ip": ip.to_string(),
                                "guid": guid,
                                "session_id": session_id,
                                "id": id,
                                "method": method,
                            }),
                        );
                    }
                }

                let inner_bytes = text.as_bytes();
                let frame = EnvelopeOut {
                    v: REMOTE_PROTOCOL_VERSION,
                    type_: "mux/frame",
                    ts: now_iso(),
                    payload: MuxFramePayloadOut {
                        session_id,
                        stream_id: CONTROL_V1_STREAM_ID,
                        payload_b64: base64::engine::general_purpose::STANDARD.encode(inner_bytes),
                    },
                };
                let Ok(frame_text) = serde_json::to_string(&frame) else {
                    continue;
                };
                let _ = orch.tx.send(Message::Text(frame_text.into()));
            }
        }
    }

    if kind == ConnectionKind::Client {
        if let Some(session_id) = session_id {
            state
                .relay
                .clients_by_session_id
                .write()
                .await
                .remove(&session_id);
        }
    }

    if kind == ConnectionKind::Ccbox && registered_ccbox {
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
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            if let Ok(ip) = first.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if let Ok(ip) = xri.trim().parse::<IpAddr>() {
            return ip;
        }
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

fn send_auth_err(tx: &mpsc::UnboundedSender<Message>, code: &str) {
    send_envelope(
        tx,
        "auth/err",
        AuthErrPayload {
            code: code.to_string(),
        },
    );
}

fn send_envelope<P: serde::Serialize>(
    tx: &mpsc::UnboundedSender<Message>,
    type_: &'static str,
    payload: P,
) {
    let env = EnvelopeOut {
        v: REMOTE_PROTOCOL_VERSION,
        type_,
        ts: now_iso(),
        payload,
    };
    if let Ok(text) = serde_json::to_string(&env) {
        let _ = tx.send(Message::Text(text.into()));
    }
}

async fn verify_device_signature(
    paths: &StorePaths,
    kind: ConnectionKind,
    device_id: &str,
    public_key_b64_override: Option<&str>,
    message: &[u8],
    signature: &Signature,
) -> Result<(), AuthError> {
    let paths = paths.clone();
    let device_id = device_id.to_string();
    let public_key_b64_override = public_key_b64_override.map(|s| s.to_string());
    let message = message.to_vec();
    let signature = *signature;

    tokio::task::spawn_blocking(move || {
        if kind == ConnectionKind::Client {
            let mut trusted = load_trusted_devices(&paths)?;
            let entry = trusted
                .trusted_devices
                .iter_mut()
                .find(|d| d.device_id == device_id)
                .ok_or(AuthError::Code("DeviceUnknown"))?;
            if entry.revoked {
                return Err(AuthError::Code("DeviceRevoked"));
            }
            let public_key_bytes = decode_public_key(&entry.public_key_b64)
                .map_err(|_| AuthError::Code("BadSignature"))?;
            verify_signature(&public_key_bytes, &message, &signature)
                .map_err(|_| AuthError::Code("BadSignature"))?;
            entry.last_seen_at = Some(now_iso());
            save_trusted_devices(&paths, &trusted)?;
            return Ok(());
        }

        // ccbox
        let mut ccboxes = load_ccboxes(&paths)?;
        let existing = ccboxes.ccboxes.iter_mut().find(|c| c.ccbox_id == device_id);

        if let Some(existing) = existing {
            if existing.revoked {
                return Err(AuthError::Code("DeviceRevoked"));
            }
            let public_key_bytes = decode_public_key(&existing.public_key_b64)
                .map_err(|_| AuthError::Code("BadSignature"))?;
            verify_signature(&public_key_bytes, &message, &signature)
                .map_err(|_| AuthError::Code("BadSignature"))?;
            existing.last_seen_at = Some(now_iso());
            save_ccboxes(&paths, &ccboxes)?;
            return Ok(());
        }

        let Some(public_key_b64) = public_key_b64_override else {
            return Err(AuthError::Code("DeviceUnknown"));
        };
        let public_key_bytes =
            decode_public_key(&public_key_b64).map_err(|_| AuthError::Code("BadSignature"))?;
        verify_signature(&public_key_bytes, &message, &signature)
            .map_err(|_| AuthError::Code("BadSignature"))?;

        ccboxes.ccboxes.push(CcboxDevice {
            ccbox_id: device_id,
            public_key_b64,
            created_at: now_iso(),
            last_seen_at: Some(now_iso()),
            revoked: false,
            label: None,
        });
        save_ccboxes(&paths, &ccboxes)?;
        Ok(())
    })
    .await
    .map_err(|error| AuthError::Io(std::io::Error::other(error)))?
}

fn decode_public_key(public_key_b64: &str) -> Result<[u8; 32], ()> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64.trim())
        .map_err(|_| ())?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| ())?;
    Ok(bytes)
}

fn verify_signature(
    public_key_bytes: &[u8; 32],
    message: &[u8],
    signature: &Signature,
) -> Result<(), ()> {
    let key = VerifyingKey::from_bytes(public_key_bytes).map_err(|_| ())?;
    key.verify_strict(message, signature).map_err(|_| ())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{load_pairing, make_store_paths, save_trusted_devices};
    use crate::types::{TrustedDevice, TrustedDevicesFile};
    use ed25519_dalek::{Signer as _, SigningKey};
    use rand_core::{OsRng, RngCore as _};
    use serde_json::Value;
    use tempfile::tempdir;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    type WsStream = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    fn random_signing_key() -> SigningKey {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    async fn ws_authenticate(
        ws: &mut WsStream,
        device_kind: &str,
        device_id: &str,
        signing_key: &SigningKey,
        public_key_b64_override: Option<&str>,
    ) {
        let hello = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "auth/hello",
            "payload": { "device_id": device_id, "device_kind": device_kind },
        });
        ws.send(ClientMessage::Text(hello.to_string().into()))
            .await
            .expect("send auth/hello");

        let nonce_b64 = loop {
            let Some(msg) = ws.next().await else {
                panic!("socket closed before auth/challenge");
            };
            let msg = msg.expect("ws message");
            if let ClientMessage::Text(text) = msg {
                let env: Value = serde_json::from_str(&text).expect("env json");
                if env.get("type").and_then(Value::as_str) == Some("auth/challenge") {
                    let payload = env.get("payload").expect("payload");
                    break payload
                        .get("nonce_b64")
                        .and_then(Value::as_str)
                        .expect("nonce_b64")
                        .to_string();
                }
            }
        };

        let nonce_bytes = base64::engine::general_purpose::STANDARD
            .decode(nonce_b64)
            .expect("nonce base64 decodes");
        let message = build_auth_message(device_kind, device_id, &nonce_bytes);
        let signature = signing_key.sign(&message).to_bytes();
        let signature_b64 = base64::engine::general_purpose::STANDARD.encode(signature);

        let mut response_payload = serde_json::json!({ "signature_b64": signature_b64 });
        if let Some(public_key_b64) = public_key_b64_override {
            response_payload
                .as_object_mut()
                .expect("payload object")
                .insert(
                    "public_key_b64".to_string(),
                    Value::String(public_key_b64.to_string()),
                );
        }

        let response = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "auth/response",
            "payload": response_payload,
        });
        ws.send(ClientMessage::Text(response.to_string().into()))
            .await
            .expect("send auth/response");

        loop {
            let Some(msg) = ws.next().await else {
                panic!("socket closed before auth/ok");
            };
            let msg = msg.expect("ws message");
            if let ClientMessage::Text(text) = msg {
                let env: Value = serde_json::from_str(&text).expect("env json");
                match env.get("type").and_then(Value::as_str) {
                    Some("auth/ok") => return,
                    Some("auth/err") => {
                        panic!("auth failed: {text}");
                    }
                    _ => {}
                }
            }
        }
    }

    #[tokio::test]
    async fn smoke_projects_list_round_trips_over_mux() {
        let dir = tempdir().expect("tempdir");
        let store_paths = make_store_paths(dir.path());

        let client_key = random_signing_key();
        let client_device_id = Uuid::new_v4().to_string();
        let client_public_key_b64 =
            base64::engine::general_purpose::STANDARD.encode(client_key.verifying_key().to_bytes());

        let trusted = TrustedDevicesFile {
            trusted_devices: vec![TrustedDevice {
                device_id: client_device_id.clone(),
                public_key_b64: client_public_key_b64,
                created_at: now_iso(),
                last_seen_at: None,
                revoked: false,
                label: None,
            }],
        };
        save_trusted_devices(&store_paths, &trusted).expect("trusted devices saved");

        let state = AppState {
            store_paths: store_paths.clone(),
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
        let ccbox_key = random_signing_key();
        let ccbox_public_key_b64 =
            base64::engine::general_purpose::STANDARD.encode(ccbox_key.verifying_key().to_bytes());

        let ccbox_url = format!("ws://{addr}/ccbox?guid={guid}");
        let (mut ccbox_ws, _) = connect_async(ccbox_url).await.expect("ccbox connect");
        ws_authenticate(
            &mut ccbox_ws,
            "ccbox",
            &guid,
            &ccbox_key,
            Some(&ccbox_public_key_b64),
        )
        .await;
        let register = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "ccbox/register",
            "payload": { "ccbox_id": guid },
        });
        ccbox_ws
            .send(ClientMessage::Text(register.to_string().into()))
            .await
            .expect("ccbox/register");

        let (mut ccbox_write, mut ccbox_read) = ccbox_ws.split();
        let ccbox_task = tokio::spawn(async move {
            while let Some(msg) = ccbox_read.next().await {
                let msg = msg.expect("ccbox ws message");
                match msg {
                    ClientMessage::Ping(bytes) => {
                        ccbox_write
                            .send(ClientMessage::Pong(bytes))
                            .await
                            .expect("pong");
                    }
                    ClientMessage::Text(text) => {
                        let env: EnvelopeIn = serde_json::from_str(&text).expect("ccbox env");
                        if env.type_ != "mux/frame" {
                            continue;
                        }
                        let mux: MuxFramePayload =
                            serde_json::from_value(env.payload).expect("mux payload");
                        if mux.stream_id != CONTROL_V1_STREAM_ID {
                            continue;
                        }

                        let inner_bytes = base64::engine::general_purpose::STANDARD
                            .decode(mux.payload_b64)
                            .expect("inner base64");
                        let inner_text = String::from_utf8(inner_bytes).expect("inner utf8");
                        let inner_env: EnvelopeIn =
                            serde_json::from_str(&inner_text).expect("inner env");
                        if inner_env.type_ != "rpc/request" {
                            continue;
                        }
                        let id = inner_env
                            .payload
                            .get("id")
                            .and_then(|v| v.as_str())
                            .expect("rpc id");

                        let response_inner = serde_json::json!({
                            "v": REMOTE_PROTOCOL_VERSION,
                            "type": "rpc/response",
                            "ts": now_iso(),
                            "payload": {
                                "id": id,
                                "ok": true,
                                "result": { "projects": [] }
                            }
                        });
                        let response_inner_text =
                            serde_json::to_string(&response_inner).expect("response json");

                        let response_env = EnvelopeOut {
                            v: REMOTE_PROTOCOL_VERSION,
                            type_: "mux/frame",
                            ts: now_iso(),
                            payload: MuxFramePayloadOut {
                                session_id: mux.session_id,
                                stream_id: CONTROL_V1_STREAM_ID,
                                payload_b64: base64::engine::general_purpose::STANDARD
                                    .encode(response_inner_text.as_bytes()),
                            },
                        };
                        let out_text =
                            serde_json::to_string(&response_env).expect("mux response json");
                        ccbox_write
                            .send(ClientMessage::Text(out_text.into()))
                            .await
                            .expect("send mux response");
                        return;
                    }
                    _ => {}
                }
            }
        });

        let client_url = format!("ws://{addr}/client?guid={guid}");
        let (mut client_ws, _) = connect_async(client_url).await.expect("client connect");
        ws_authenticate(
            &mut client_ws,
            "client",
            &client_device_id,
            &client_key,
            None,
        )
        .await;

        let req_id = Uuid::new_v4().to_string();
        let request = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "rpc/request",
            "payload": {
                "id": req_id,
                "method": "projects.list",
                "params": {}
            }
        });
        client_ws
            .send(ClientMessage::Text(request.to_string().into()))
            .await
            .expect("rpc/request");

        let mut got_response = false;
        while let Some(msg) = client_ws.next().await {
            let msg = msg.expect("client ws message");
            if let ClientMessage::Text(text) = msg {
                let env: EnvelopeIn = serde_json::from_str(&text).expect("client env");
                if env.type_ != "rpc/response" {
                    continue;
                }
                let ok = env
                    .payload
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                assert!(ok, "expected ok rpc response");
                let id = env.payload.get("id").and_then(Value::as_str).unwrap_or("");
                assert_eq!(id, req_id);
                got_response = true;
                break;
            }
        }
        assert!(got_response, "expected a rpc/response message");

        let _ = ccbox_task.await;
        server_task.abort();
    }

    #[tokio::test]
    async fn sends_auth_err_for_unknown_device_before_close() {
        let dir = tempdir().expect("tempdir");
        let store_paths = make_store_paths(dir.path());

        let state = AppState {
            store_paths: store_paths.clone(),
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
        let signing_key = random_signing_key();
        let client_url = format!("ws://{addr}/client?guid={guid}");
        let (mut client_ws, _) = connect_async(client_url).await.expect("client connect");

        let hello = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "auth/hello",
            "payload": { "device_id": device_id, "device_kind": "client" },
        });
        client_ws
            .send(ClientMessage::Text(hello.to_string().into()))
            .await
            .expect("send auth/hello");

        let nonce_b64 = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let Some(msg) = client_ws.next().await else {
                    panic!("socket closed before auth/challenge");
                };
                let msg = msg.expect("ws message");
                if let ClientMessage::Text(text) = msg {
                    let env: Value = serde_json::from_str(&text).expect("env json");
                    if env.get("type").and_then(Value::as_str) == Some("auth/challenge") {
                        let payload = env.get("payload").expect("payload");
                        break payload
                            .get("nonce_b64")
                            .and_then(Value::as_str)
                            .expect("nonce_b64")
                            .to_string();
                    }
                }
            }
        })
        .await
        .expect("auth/challenge within 1s");

        let nonce_bytes = base64::engine::general_purpose::STANDARD
            .decode(nonce_b64)
            .expect("nonce base64 decodes");
        let message = build_auth_message("client", &device_id, &nonce_bytes);
        let signature = signing_key.sign(&message).to_bytes();
        let signature_b64 = base64::engine::general_purpose::STANDARD.encode(signature);

        let response = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "auth/response",
            "payload": { "signature_b64": signature_b64 },
        });
        client_ws
            .send(ClientMessage::Text(response.to_string().into()))
            .await
            .expect("send auth/response");

        let auth_err_code = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let Some(msg) = client_ws.next().await else {
                    return None;
                };
                let msg = msg.expect("ws message");
                if let ClientMessage::Text(text) = msg {
                    let env: Value = serde_json::from_str(&text).expect("env json");
                    if env.get("type").and_then(Value::as_str) == Some("auth/err") {
                        break env
                            .get("payload")
                            .and_then(|payload| payload.get("code"))
                            .and_then(Value::as_str)
                            .map(|s| s.to_string());
                    }
                }
            }
        })
        .await
        .expect("auth/err within 1s");

        assert_eq!(auth_err_code.as_deref(), Some("DeviceUnknown"));

        server_task.abort();
    }

    #[tokio::test]
    async fn ccbox_pairing_create_fetches_or_reuses_active_code() {
        let dir = tempdir().expect("tempdir");
        let store_paths = make_store_paths(dir.path());

        let state = AppState {
            store_paths: store_paths.clone(),
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
        let ccbox_key = random_signing_key();
        let ccbox_public_key_b64 =
            base64::engine::general_purpose::STANDARD.encode(ccbox_key.verifying_key().to_bytes());

        let ccbox_url = format!("ws://{addr}/ccbox?guid={guid}");
        let (mut ccbox_ws, _) = connect_async(ccbox_url).await.expect("ccbox connect");
        ws_authenticate(
            &mut ccbox_ws,
            "ccbox",
            &guid,
            &ccbox_key,
            Some(&ccbox_public_key_b64),
        )
        .await;

        let request = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "ccbox/pairing/create",
            "payload": { "ttl_seconds": 120 },
        });
        ccbox_ws
            .send(ClientMessage::Text(request.to_string().into()))
            .await
            .expect("pairing create send");

        let mut first_pairing_code = None::<String>;
        while let Some(msg) = ccbox_ws.next().await {
            let msg = msg.expect("ws message");
            if let ClientMessage::Text(text) = msg {
                let env: Value = serde_json::from_str(&text).expect("json");
                if env.get("type").and_then(Value::as_str) != Some("ccbox/pairing/ok") {
                    continue;
                }
                let payload = env.get("payload").expect("payload");
                let pairing_code = payload
                    .get("pairing_code")
                    .and_then(Value::as_str)
                    .expect("pairing_code")
                    .to_string();
                assert_eq!(pairing_code.len(), 10);
                first_pairing_code = Some(pairing_code);
                break;
            }
        }
        let first_pairing_code = first_pairing_code.expect("pairing code");

        let request = serde_json::json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "type": "ccbox/pairing/create",
            "payload": { "ttl_seconds": 120 },
        });
        ccbox_ws
            .send(ClientMessage::Text(request.to_string().into()))
            .await
            .expect("pairing create send");

        let mut second_pairing_code = None::<String>;
        while let Some(msg) = ccbox_ws.next().await {
            let msg = msg.expect("ws message");
            if let ClientMessage::Text(text) = msg {
                let env: Value = serde_json::from_str(&text).expect("json");
                if env.get("type").and_then(Value::as_str) != Some("ccbox/pairing/ok") {
                    continue;
                }
                let payload = env.get("payload").expect("payload");
                let pairing_code = payload
                    .get("pairing_code")
                    .and_then(Value::as_str)
                    .expect("pairing_code")
                    .to_string();
                let reused = payload
                    .get("reused")
                    .and_then(Value::as_bool)
                    .expect("reused");
                assert!(reused);
                second_pairing_code = Some(pairing_code);
                break;
            }
        }
        let second_pairing_code = second_pairing_code.expect("pairing code");
        assert_eq!(first_pairing_code, second_pairing_code);

        let stored_pairing = load_pairing(&store_paths, &guid)
            .expect("load pairing")
            .expect("pairing exists");
        assert_eq!(stored_pairing.code_base32, first_pairing_code);

        server_task.abort();
    }
}
