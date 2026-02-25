use crate::domain::{
    CONTROL_V1_STREAM_ID, DEVICE_KIND_CCBOX, REMOTE_PROTOCOL_VERSION, build_auth_message,
};
use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use futures_util::{SinkExt as _, StreamExt as _};
use rand_core::OsRng;
use rand_core::RngCore as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::io::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;
use url::Url;
use uuid::Uuid;

mod control;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServeOptions {
    pub label: Option<String>,
    pub relay_domain: String,
    pub relay_url: Option<String>,
    pub relay_base_url: Option<String>,
    pub pairing_code: Option<String>,
    pub enable_shell: bool,
    pub print_identity: bool,
    pub no_relay: bool,
    pub listen_addr: Option<String>,
}

#[derive(Debug, Error)]
pub enum ServeError {
    #[error(transparent)]
    ResolveStateDir(#[from] crate::infra::ResolveCcboxStateDirError),

    #[error(transparent)]
    ResolveSessionsDir(#[from] crate::infra::ResolveSessionsDirError),

    #[error(transparent)]
    Identity(#[from] IdentityError),

    #[error("failed to build tokio runtime: {0}")]
    Runtime(String),

    #[error("invalid relay url: {0}")]
    RelayUrl(String),

    #[error("invalid listen addr: {0}")]
    ListenAddr(String),

    #[error("local relay error: {0}")]
    LocalRelay(String),

    #[error("websocket connect failed: {0}")]
    WsConnect(String),

    #[error("websocket error: {0}")]
    Ws(String),

    #[error("auth failed: {0}")]
    AuthFailed(String),

    #[error("pairing bootstrap failed: {0}")]
    PairingBootstrap(String),

    #[error("invalid base64 payload: {0}")]
    Base64(String),

    #[error("process manager error: {0}")]
    ProcessManager(String),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct CcboxIdentity {
    pub ccbox_id: Uuid,
    pub public_key_b64: String,
    signing_key: SigningKey,
}

impl CcboxIdentity {
    pub fn sign_challenge_for_device(
        &self,
        device_id: &str,
        nonce_bytes: &[u8],
    ) -> Result<String, IdentityError> {
        let message = build_auth_message(DEVICE_KIND_CCBOX, device_id, nonce_bytes);
        let signature = self.signing_key.sign(&message).to_bytes();
        Ok(base64::engine::general_purpose::STANDARD.encode(signature))
    }
}

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("failed to create remote state dir {path}: {source}")]
    CreateRemoteDir { path: String, source: io::Error },

    #[error("failed to read ccbox_id: {0}")]
    ReadCcboxId(#[from] io::Error),

    #[error("invalid ccbox_id in {path}: {value}")]
    InvalidCcboxId { path: String, value: String },

    #[error("failed to write ccbox_id: {0}")]
    WriteCcboxId(io::Error),

    #[error("failed to read ccbox key: {0}")]
    ReadKey(io::Error),

    #[error("invalid ccbox key encoding in {path}")]
    InvalidKeyEncoding { path: String },

    #[error("invalid ccbox key length in {path}: expected 32 bytes, got {len}")]
    InvalidKeyLength { path: String, len: usize },

    #[error("failed to write ccbox key: {0}")]
    WriteKey(io::Error),
}

pub fn load_or_create_ccbox_identity(state_dir: &Path) -> Result<CcboxIdentity, IdentityError> {
    let remote_dir = state_dir.join("remote");
    fs::create_dir_all(&remote_dir).map_err(|error| IdentityError::CreateRemoteDir {
        path: remote_dir.display().to_string(),
        source: error,
    })?;

    let ccbox_id_path = remote_dir.join("ccbox_id");
    let ccbox_id = if ccbox_id_path.exists() {
        let raw = fs::read_to_string(&ccbox_id_path)?;
        let trimmed = raw.trim();
        Uuid::parse_str(trimmed).map_err(|_| IdentityError::InvalidCcboxId {
            path: ccbox_id_path.display().to_string(),
            value: trimmed.to_string(),
        })?
    } else {
        let id = Uuid::new_v4();
        fs::write(&ccbox_id_path, format!("{id}\n")).map_err(IdentityError::WriteCcboxId)?;
        id
    };

    let key_path = remote_dir.join("ccbox_ed25519");
    let signing_key = if key_path.exists() {
        let raw = fs::read_to_string(&key_path).map_err(IdentityError::ReadKey)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw.trim())
            .map_err(|_| IdentityError::InvalidKeyEncoding {
                path: key_path.display().to_string(),
            })?;
        let bytes: [u8; 32] =
            bytes
                .try_into()
                .map_err(|bytes: Vec<u8>| IdentityError::InvalidKeyLength {
                    path: key_path.display().to_string(),
                    len: bytes.len(),
                })?;
        SigningKey::from_bytes(&bytes)
    } else {
        let key = SigningKey::generate(&mut OsRng);
        write_secret_file(
            &key_path,
            &base64::engine::general_purpose::STANDARD.encode(key.to_bytes()),
        )
        .map_err(IdentityError::WriteKey)?;
        key
    };

    let public_key_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes());

    Ok(CcboxIdentity {
        ccbox_id,
        public_key_b64,
        signing_key,
    })
}

pub fn run_serve(opts: ServeOptions) -> Result<(), ServeError> {
    let state_dir = crate::infra::resolve_ccbox_state_dir()?;
    let sessions_dir = crate::infra::resolve_sessions_dir()?;
    let identity = load_or_create_ccbox_identity(&state_dir)?;
    let connection_guid = Uuid::new_v4();
    let connection_guid_text = connection_guid.to_string();

    if opts.print_identity {
        let mut out = io::stdout().lock();
        let _ = writeln!(out, "ccbox_id={}", identity.ccbox_id);
        let _ = writeln!(out, "public_key_b64={}", identity.public_key_b64);
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| ServeError::Runtime(error.to_string()))?;

    let mut out = io::stdout().lock();
    let _ = writeln!(out, "ccbox serve");
    let _ = writeln!(out, "ccbox_id={}", identity.ccbox_id);
    let _ = writeln!(out, "guid={connection_guid_text}");
    if opts.no_relay {
        let bind_addr = parse_listen_addr(opts.listen_addr.as_deref())?;
        let connect_addr = connect_addr_for_bind(bind_addr);
        let _ = writeln!(out, "mode=local-no-relay");
        let _ = writeln!(out, "bind_addr={bind_addr}");
        let _ = writeln!(
            out,
            "client_ws_url=ws://{connect_addr}/client?guid={connection_guid_text}"
        );
        let _ = writeln!(
            out,
            "pair_url=http://{connect_addr}/pair?guid={connection_guid_text}"
        );
        let _ = writeln!(
            out,
            "ccbox_ws_url=ws://{connect_addr}/ccbox?guid={connection_guid_text}"
        );

        let relay_data_dir = state_dir.join("remote").join("relay");
        let store_paths = ccbox_relay::store::make_store_paths(&relay_data_dir);
        match ensure_pairing_code(&store_paths, &connection_guid_text, 120) {
            Ok(Some(pairing)) => {
                let _ = writeln!(out, "pairing_code={}", pairing.code_base32);
                let _ = writeln!(out, "pairing_expires_at={}", pairing.expires_at);
            }
            Ok(None) => {}
            Err(error) => {
                let _ = writeln!(out, "pairing_error={error}");
            }
        }
    } else {
        let relay_ws_url = resolve_ccbox_ws_url(&opts, connection_guid)?.to_string();
        let client_ws_url = resolve_client_ws_url(&opts, connection_guid)?.to_string();
        let pair_url = resolve_pair_url(&client_ws_url)
            .map_err(ServeError::RelayUrl)?
            .to_string();
        let mut pairing_code_for_web = opts
            .pairing_code
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let mut pairing_expires_at = None::<String>;

        if pairing_code_for_web.is_none() {
            match runtime.block_on(fetch_or_create_remote_pairing_code(
                &opts,
                &identity,
                connection_guid,
            )) {
                Ok(pairing) => {
                    pairing_code_for_web = Some(pairing.pairing_code);
                    pairing_expires_at = Some(pairing.expires_at);
                }
                Err(error) => {
                    let _ = writeln!(out, "pairing_error={error}");
                }
            }
        }

        if let Some(pairing_code) = pairing_code_for_web.as_deref() {
            let _ = writeln!(out, "pairing_code={pairing_code}");
        }
        if let Some(pairing_expires_at) = pairing_expires_at.as_deref() {
            let _ = writeln!(out, "pairing_expires_at={pairing_expires_at}");
        }

        let web_url = resolve_web_url(
            &opts,
            connection_guid,
            &client_ws_url,
            pairing_code_for_web.as_deref(),
        )
        .map_err(ServeError::RelayUrl)?
        .to_string();
        let _ = writeln!(out, "mode=relay");
        let _ = writeln!(out, "web_url={web_url}");
        let _ = writeln!(out, "connect_url={web_url}");
        print_qr_to_stderr(&web_url);
        let _ = writeln!(out, "pair_url={pair_url}");
        let _ = writeln!(out, "client_ws_url={client_ws_url}");
        let _ = writeln!(out, "relay_ws_url={relay_ws_url}");
    }

    if opts.no_relay {
        runtime.block_on(async move {
            serve_loop_local(opts, identity, sessions_dir, state_dir, connection_guid).await
        })?;
    } else {
        runtime.block_on(async move {
            serve_loop(opts, identity, sessions_dir, connection_guid).await
        })?;
    }
    Ok(())
}

fn print_qr_to_stderr(url: &str) {
    if !io::stderr().is_terminal() {
        return;
    }

    let Ok(code) = qrcode::QrCode::new(url.as_bytes()) else {
        let _ = writeln!(io::stderr().lock(), "qr_error=encode_failed");
        return;
    };

    let qr = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build();

    let mut err = io::stderr().lock();
    let _ = writeln!(err);
    let _ = writeln!(err, "scan_qr_url={url}");
    let _ = writeln!(err, "{qr}");
    let _ = writeln!(err);
}

fn resolve_pair_url(client_ws_url: &str) -> Result<Url, String> {
    let mut url = Url::parse(client_ws_url).map_err(|error| error.to_string())?;
    match url.scheme() {
        "wss" => url
            .set_scheme("https")
            .map_err(|_| "invalid url scheme".to_string())?,
        "ws" => url
            .set_scheme("http")
            .map_err(|_| "invalid url scheme".to_string())?,
        other => return Err(format!("unexpected websocket scheme: {other}")),
    }
    url.set_path("/pair");
    url.set_fragment(None);
    Ok(url)
}

fn resolve_web_url(
    opts: &ServeOptions,
    ccbox_id: Uuid,
    client_ws_url: &str,
    pairing_code: Option<&str>,
) -> Result<Url, String> {
    let mut url = if opts.relay_url.is_none() && opts.relay_base_url.is_none() {
        let raw = format!("https://{}.{}", ccbox_id, opts.relay_domain);
        Url::parse(&raw).map_err(|error| error.to_string())?
    } else {
        let mut url = Url::parse(client_ws_url).map_err(|error| error.to_string())?;
        match url.scheme() {
            "wss" => url
                .set_scheme("https")
                .map_err(|_| "invalid url scheme".to_string())?,
            "ws" => url
                .set_scheme("http")
                .map_err(|_| "invalid url scheme".to_string())?,
            other => return Err(format!("unexpected websocket scheme: {other}")),
        }
        url.set_path("/");
        url.set_query(None);
        url.query_pairs_mut().append_pair("connect", client_ws_url);
        url.set_fragment(None);
        url
    };

    if let Some(pairing_code) = pairing_code {
        let pairing_code = pairing_code.trim();
        if !pairing_code.is_empty() {
            url.query_pairs_mut()
                .append_pair("pairing_code", pairing_code);
        }
    }
    url.query_pairs_mut().append_pair("autoconnect", "1");

    Ok(url)
}

#[derive(Debug)]
struct RemotePairingCode {
    pairing_code: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
struct CcboxPairingCreatePayload {
    ttl_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct CcboxPairingOkPayload {
    pairing_code: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct CcboxPairingErrPayload {
    code: String,
}

async fn fetch_or_create_remote_pairing_code(
    opts: &ServeOptions,
    identity: &CcboxIdentity,
    connection_guid: Uuid,
) -> Result<RemotePairingCode, ServeError> {
    let connection_guid_text = connection_guid.to_string();
    let url = resolve_ccbox_ws_url(opts, connection_guid)?;
    let (mut ws, _response) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .map_err(|error| ServeError::WsConnect(error.to_string()))?;

    authenticate_ccbox(&mut ws, identity, &connection_guid_text).await?;
    let request = EnvelopeOut {
        v: REMOTE_PROTOCOL_VERSION,
        type_: "ccbox/pairing/create",
        ts: now_iso(),
        payload: CcboxPairingCreatePayload { ttl_seconds: 120 },
    };
    send_json(&mut ws, &request).await?;

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Some(msg) = ws.next().await else {
                return Err(ServeError::PairingBootstrap(
                    "socket closed before pairing response".to_string(),
                ));
            };
            let msg = msg.map_err(|error| ServeError::Ws(error.to_string()))?;

            match msg {
                Message::Ping(bytes) => {
                    ws.send(Message::Pong(bytes))
                        .await
                        .map_err(|error| ServeError::Ws(error.to_string()))?;
                }
                Message::Text(text) => {
                    let env: EnvelopeIn = match serde_json::from_str(&text) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if env.v != REMOTE_PROTOCOL_VERSION {
                        continue;
                    }
                    match env.type_.as_str() {
                        "ccbox/pairing/ok" => {
                            let payload: CcboxPairingOkPayload =
                                serde_json::from_value(env.payload)?;
                            if payload.pairing_code.trim().is_empty() {
                                return Err(ServeError::PairingBootstrap(
                                    "relay returned empty pairing code".to_string(),
                                ));
                            }
                            return Ok(RemotePairingCode {
                                pairing_code: payload.pairing_code,
                                expires_at: payload.expires_at,
                            });
                        }
                        "ccbox/pairing/err" => {
                            let payload: CcboxPairingErrPayload =
                                serde_json::from_value(env.payload)?;
                            return Err(ServeError::PairingBootstrap(payload.code));
                        }
                        _ => {}
                    }
                }
                Message::Close(_) => {
                    return Err(ServeError::PairingBootstrap(
                        "socket closed before pairing response".to_string(),
                    ));
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| ServeError::PairingBootstrap("pairing request timeout".to_string()))??;

    let _ = ws.send(Message::Close(None)).await;
    Ok(result)
}

fn parse_listen_addr(raw: Option<&str>) -> Result<SocketAddr, ServeError> {
    let raw = raw.unwrap_or("127.0.0.1:8787").trim();
    raw.parse::<SocketAddr>()
        .map_err(|error| ServeError::ListenAddr(format!("{raw}: {error}")))
}

fn connect_addr_for_bind(bind_addr: SocketAddr) -> SocketAddr {
    let port = bind_addr.port();
    match bind_addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port)
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port)
        }
        _ => bind_addr,
    }
}

fn ensure_pairing_code(
    store_paths: &ccbox_relay::store::StorePaths,
    guid: &str,
    ttl_seconds: i64,
) -> Result<Option<ccbox_relay::types::PairingRecord>, String> {
    ccbox_relay::pairing::ensure_pairing_record(store_paths, guid, ttl_seconds, 5)
        .map(|result| Some(result.record))
        .map_err(|error| error.to_string())
}

async fn serve_loop_local(
    opts: ServeOptions,
    identity: CcboxIdentity,
    sessions_dir: std::path::PathBuf,
    state_dir: std::path::PathBuf,
    connection_guid: Uuid,
) -> Result<(), ServeError> {
    let bind_addr = parse_listen_addr(opts.listen_addr.as_deref())?;
    let connect_addr = connect_addr_for_bind(bind_addr);
    let connection_guid_text = connection_guid.to_string();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(true);
    });

    let relay_data_dir = state_dir.join("remote").join("relay");
    let store_paths = ccbox_relay::store::make_store_paths(&relay_data_dir);

    let relay_task = tokio::spawn({
        let store_paths = store_paths.clone();
        async move { ccbox_relay::server::run_http_server_on(bind_addr, store_paths).await }
    });

    let url = Url::parse(&format!("ws://{connect_addr}/ccbox?guid={connection_guid}"))
        .map_err(|error| ServeError::RelayUrl(error.to_string()))?;

    let mut control = control::ControlPlane::new(sessions_dir.clone())?;
    let mut backoff_ms: u64 = 250;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        if relay_task.is_finished() {
            match relay_task.await {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => return Err(ServeError::LocalRelay(error)),
                Err(error) => return Err(ServeError::LocalRelay(error.to_string())),
            }
        }

        let run = connect_and_run_once(
            &url,
            &opts,
            &identity,
            &connection_guid_text,
            &mut control,
            shutdown_rx.clone(),
        )
        .await;
        match run {
            Ok(()) => break,
            Err(ServeError::AuthFailed(code)) => return Err(ServeError::AuthFailed(code)),
            Err(error) => {
                eprintln!("ccbox serve (local): {error}");
            }
        }

        let sleep_ms = backoff_ms.saturating_add(jitter_ms(backoff_ms / 4).await);
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
    }

    relay_task.abort();
    Ok(())
}

async fn serve_loop(
    opts: ServeOptions,
    identity: CcboxIdentity,
    sessions_dir: std::path::PathBuf,
    connection_guid: Uuid,
) -> Result<(), ServeError> {
    let connection_guid_text = connection_guid.to_string();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(true);
    });

    let mut control = control::ControlPlane::new(sessions_dir.clone())?;

    let mut backoff_ms: u64 = 250;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        let url = resolve_ccbox_ws_url(&opts, connection_guid)?;
        let run = connect_and_run_once(
            &url,
            &opts,
            &identity,
            &connection_guid_text,
            &mut control,
            shutdown_rx.clone(),
        )
        .await;
        match run {
            Ok(()) => break,
            Err(ServeError::AuthFailed(code)) => return Err(ServeError::AuthFailed(code)),
            Err(error) => {
                eprintln!("ccbox serve: {error}");
            }
        }

        let sleep_ms = backoff_ms.saturating_add(jitter_ms(backoff_ms / 4).await);
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
    }

    Ok(())
}

fn resolve_ccbox_ws_url(opts: &ServeOptions, ccbox_id: Uuid) -> Result<Url, ServeError> {
    if let Some(raw) = &opts.relay_url {
        let url = Url::parse(raw).map_err(|error| ServeError::RelayUrl(error.to_string()))?;
        if url.scheme() != "wss" {
            return Err(ServeError::RelayUrl(
                "relay url must use wss://".to_string(),
            ));
        }
        return Ok(url);
    }

    if let Some(raw) = &opts.relay_base_url {
        let mut url = Url::parse(raw).map_err(|error| ServeError::RelayUrl(error.to_string()))?;
        if url.scheme() != "wss" {
            return Err(ServeError::RelayUrl(
                "relay base url must use wss://".to_string(),
            ));
        }
        url.set_path("/ccbox");
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair("guid", &ccbox_id.to_string());
        return Ok(url);
    }

    let raw = format!("wss://{}.{}{}", ccbox_id, opts.relay_domain, "/ccbox");
    Url::parse(&raw).map_err(|error| ServeError::RelayUrl(error.to_string()))
}

fn resolve_client_ws_url(opts: &ServeOptions, ccbox_id: Uuid) -> Result<Url, ServeError> {
    let expected_host = format!("{}.{}", ccbox_id, opts.relay_domain).to_ascii_lowercase();

    if let Some(raw) = &opts.relay_url {
        let mut url = Url::parse(raw).map_err(|error| ServeError::RelayUrl(error.to_string()))?;
        if url.scheme() != "wss" {
            return Err(ServeError::RelayUrl(
                "relay url must use wss://".to_string(),
            ));
        }
        url.set_path("/client");
        url.set_query(None);
        url.set_fragment(None);

        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        if host != expected_host {
            url.query_pairs_mut()
                .append_pair("guid", &ccbox_id.to_string());
        }
        return Ok(url);
    }

    if let Some(raw) = &opts.relay_base_url {
        let mut url = Url::parse(raw).map_err(|error| ServeError::RelayUrl(error.to_string()))?;
        if url.scheme() != "wss" {
            return Err(ServeError::RelayUrl(
                "relay base url must use wss://".to_string(),
            ));
        }
        url.set_path("/client");
        url.set_query(None);
        url.set_fragment(None);

        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        if host != expected_host {
            url.query_pairs_mut()
                .append_pair("guid", &ccbox_id.to_string());
        }
        return Ok(url);
    }

    let raw = format!("wss://{}.{}{}", ccbox_id, opts.relay_domain, "/client");
    Url::parse(&raw).map_err(|error| ServeError::RelayUrl(error.to_string()))
}

async fn connect_and_run_once(
    url: &Url,
    opts: &ServeOptions,
    identity: &CcboxIdentity,
    connection_guid: &str,
    control: &mut control::ControlPlane,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), ServeError> {
    let (mut ws, _response) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .map_err(|error| ServeError::WsConnect(error.to_string()))?;

    authenticate_ccbox(&mut ws, identity, connection_guid).await?;
    send_ccbox_register(&mut ws, opts, connection_guid).await?;
    control.on_connected();

    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
    ping_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut maintenance_interval = tokio::time::interval(Duration::from_millis(250));
    maintenance_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            Ok(()) = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    let _ = ws.send(Message::Close(None)).await;
                    return Ok(());
                }
            }
            _ = ping_interval.tick() => {
                ws.send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|error| ServeError::Ws(error.to_string()))?;
            }
            _ = maintenance_interval.tick() => {
                control.tick(&mut ws).await?;
            }
            msg = ws.next() => {
                let Some(msg) = msg else {
                    return Ok(());
                };
                let msg = msg.map_err(|error| ServeError::Ws(error.to_string()))?;
                if !handle_ws_message_control(
                    &mut ws,
                    msg,
                    opts,
                    connection_guid,
                    control,
                )
                .await?
                {
                    return Ok(());
                }
            }
        }
    }
}

async fn authenticate_ccbox(
    ws: &mut WsStream,
    identity: &CcboxIdentity,
    connection_guid: &str,
) -> Result<(), ServeError> {
    let device_id = connection_guid.to_string();
    let hello = EnvelopeOut {
        v: REMOTE_PROTOCOL_VERSION,
        type_: "auth/hello",
        ts: now_iso(),
        payload: AuthHelloPayload {
            device_id,
            device_kind: "ccbox",
        },
    };
    send_json(ws, &hello).await?;

    loop {
        let Some(msg) = ws.next().await else {
            return Err(ServeError::Ws("socket closed during auth".to_string()));
        };
        let msg = msg.map_err(|error| ServeError::Ws(error.to_string()))?;
        match msg {
            Message::Text(text) => {
                let env: EnvelopeIn = serde_json::from_str(&text)?;
                if env.v != REMOTE_PROTOCOL_VERSION {
                    continue;
                }
                match env.type_.as_str() {
                    "auth/challenge" => {
                        let payload: AuthChallengePayload = serde_json::from_value(env.payload)?;
                        let nonce_bytes = base64::engine::general_purpose::STANDARD
                            .decode(payload.nonce_b64)
                            .map_err(|error| ServeError::Base64(error.to_string()))?;
                        let signature_b64 = identity
                            .sign_challenge_for_device(connection_guid, &nonce_bytes)
                            .map_err(ServeError::Identity)?;
                        let response = EnvelopeOut {
                            v: REMOTE_PROTOCOL_VERSION,
                            type_: "auth/response",
                            ts: now_iso(),
                            payload: AuthResponsePayload {
                                signature_b64,
                                public_key_b64: Some(identity.public_key_b64.clone()),
                            },
                        };
                        send_json(ws, &response).await?;
                    }
                    "auth/ok" => {
                        return Ok(());
                    }
                    "auth/err" => {
                        let payload: AuthErrPayload = serde_json::from_value(env.payload)?;
                        return Err(ServeError::AuthFailed(payload.code));
                    }
                    _ => {}
                }
            }
            Message::Ping(bytes) => {
                ws.send(Message::Pong(bytes))
                    .await
                    .map_err(|error| ServeError::Ws(error.to_string()))?;
            }
            Message::Close(_) => {
                return Err(ServeError::Ws("socket closed during auth".to_string()));
            }
            _ => {}
        }
    }
}

async fn send_ccbox_register(
    ws: &mut WsStream,
    opts: &ServeOptions,
    connection_guid: &str,
) -> Result<(), ServeError> {
    let mut capabilities = vec!["control-v1".to_string()];
    if opts.enable_shell {
        capabilities.push("shell-v1".to_string());
    }

    let register = EnvelopeOut {
        v: REMOTE_PROTOCOL_VERSION,
        type_: "ccbox/register",
        ts: now_iso(),
        payload: CcboxRegisterPayload {
            ccbox_id: connection_guid.to_string(),
            label: opts.label.clone(),
            capabilities,
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    };
    send_json(ws, &register).await
}

async fn handle_ws_message_control(
    ws: &mut WsStream,
    msg: Message,
    opts: &ServeOptions,
    connection_guid: &str,
    control: &mut control::ControlPlane,
) -> Result<bool, ServeError> {
    match msg {
        Message::Ping(bytes) => {
            ws.send(Message::Pong(bytes))
                .await
                .map_err(|error| ServeError::Ws(error.to_string()))?;
            Ok(true)
        }
        Message::Text(text) => {
            handle_incoming_text(ws, &text, opts, connection_guid, control).await?;
            Ok(true)
        }
        Message::Close(_) => Ok(false),
        _ => Ok(true),
    }
}

async fn handle_incoming_text(
    ws: &mut WsStream,
    text: &str,
    opts: &ServeOptions,
    connection_guid: &str,
    control: &mut control::ControlPlane,
) -> Result<(), ServeError> {
    let env: EnvelopeIn = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if env.v != REMOTE_PROTOCOL_VERSION {
        return Ok(());
    }

    if env.type_ != "mux/frame" {
        return Ok(());
    }

    let mux: MuxFramePayload = match serde_json::from_value(env.payload) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if mux.stream_id != CONTROL_V1_STREAM_ID {
        return Ok(());
    }

    let payload_bytes = base64::engine::general_purpose::STANDARD
        .decode(mux.payload_b64)
        .map_err(|error| ServeError::Base64(error.to_string()))?;
    let payload_text = String::from_utf8_lossy(&payload_bytes);

    let inner: EnvelopeIn = match serde_json::from_str(&payload_text) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if inner.v != REMOTE_PROTOCOL_VERSION {
        return Ok(());
    }
    if inner.type_ != "rpc/request" {
        return Ok(());
    }
    let req: RpcRequestPayload = match serde_json::from_value(inner.payload) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };

    control
        .handle_rpc(ws, mux.session_id, opts, connection_guid, &req)
        .await
}

async fn jitter_ms(max_ms: u64) -> u64 {
    if max_ms == 0 {
        return 0;
    }
    let mut rng = OsRng;
    let mut buf = [0u8; 8];
    rng.fill_bytes(&mut buf);
    let n = u64::from_le_bytes(buf);
    n % (max_ms + 1)
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

async fn send_json<P: Serialize>(
    ws: &mut WsStream,
    env: &EnvelopeOut<P>,
) -> Result<(), ServeError> {
    let text = serde_json::to_string(env)?;
    ws.send(Message::Text(text.into()))
        .await
        .map_err(|error| ServeError::Ws(error.to_string()))
}

#[derive(Debug, Deserialize)]
struct EnvelopeIn {
    v: u8,
    #[serde(rename = "type")]
    type_: String,
    payload: Value,
}

#[derive(Debug, Serialize)]
struct EnvelopeOut<P> {
    v: u8,
    #[serde(rename = "type")]
    type_: &'static str,
    ts: String,
    payload: P,
}

#[derive(Debug, Deserialize, Serialize)]
struct AuthHelloPayload<'a> {
    device_id: String,
    device_kind: &'a str,
}

#[derive(Debug, Deserialize)]
struct AuthChallengePayload {
    nonce_b64: String,
}

#[derive(Debug, Serialize)]
struct AuthResponsePayload {
    signature_b64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_key_b64: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthErrPayload {
    code: String,
}

#[derive(Debug, Serialize)]
struct CcboxRegisterPayload {
    ccbox_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    capabilities: Vec<String>,
    version: String,
}

#[derive(Debug, Deserialize)]
struct MuxFramePayload {
    session_id: String,
    stream_id: u64,
    payload_b64: String,
}

#[derive(Debug, Serialize)]
struct MuxFramePayloadOut {
    session_id: String,
    stream_id: u64,
    payload_b64: String,
}

#[derive(Debug, Deserialize)]
struct RpcRequestPayload {
    id: String,
    method: String,
    params: Value,
}

#[cfg(unix)]
fn write_secret_file(path: &Path, contents: &str) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret_file(path: &Path, contents: &str) -> io::Result<()> {
    fs::write(path, format!("{contents}\n"))
}
