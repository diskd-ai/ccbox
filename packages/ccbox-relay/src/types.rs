use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type IsoTimestamp = String;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TrustedDevice {
    pub device_id: String,
    pub public_key_b64: String,
    pub created_at: IsoTimestamp,
    pub last_seen_at: Option<IsoTimestamp>,
    pub revoked: bool,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CcboxDevice {
    pub ccbox_id: String,
    pub public_key_b64: String,
    pub created_at: IsoTimestamp,
    pub last_seen_at: Option<IsoTimestamp>,
    pub revoked: bool,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PairingRecord {
    pub code_base32: String,
    pub created_at: IsoTimestamp,
    pub expires_at: IsoTimestamp,
    pub attempts_remaining: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TrustedDevicesFile {
    pub trusted_devices: Vec<TrustedDevice>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CcboxesFile {
    pub ccboxes: Vec<CcboxDevice>,
}

#[derive(Debug, Deserialize)]
pub struct EnvelopeIn {
    pub v: u8,
    #[serde(rename = "type")]
    pub type_: String,
    pub payload: Value,
}

#[derive(Debug, Serialize)]
pub struct EnvelopeOut<P> {
    pub v: u8,
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub ts: String,
    pub payload: P,
}

#[derive(Debug, Deserialize)]
pub struct AuthHelloPayload {
    pub device_id: String,
    pub device_kind: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthResponsePayload {
    pub signature_b64: String,
    pub public_key_b64: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthChallengePayload {
    pub nonce_b64: String,
    pub expires_in_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct AuthOkPayload {
    pub device_id: String,
}

#[derive(Debug, Serialize)]
pub struct AuthErrPayload {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct CcboxRegisterPayload {
    pub ccbox_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CcboxPairingCreatePayload {
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CcboxPairingOkPayload {
    pub pairing_code: String,
    pub expires_at: IsoTimestamp,
    pub attempts_remaining: u32,
    pub reused: bool,
}

#[derive(Debug, Serialize)]
pub struct CcboxPairingErrPayload {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct MuxFramePayload {
    pub session_id: String,
    pub stream_id: u64,
    pub payload_b64: String,
}

#[derive(Debug, Serialize)]
pub struct MuxFramePayloadOut {
    pub session_id: String,
    pub stream_id: u64,
    pub payload_b64: String,
}
